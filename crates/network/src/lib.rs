// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded compressed HTTP ranges for an in-process media stream adapter.
//! Guest or explicitly leased signed media; no cookies enter this boundary.
//! Neither this module nor its diagnostics expose signed addresses.
#[cfg(target_os = "macos")]
mod dns_macos;
#[cfg(target_os = "macos")]
mod dns_process;
#[cfg(target_os = "macos")]
pub use dns_process::run_helper;
#[cfg(test)]
mod access_tests;
mod policy;
mod timing;
use reqwest::{
    Client, StatusCode,
    header::{
        CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, HeaderMap, LAST_MODIFIED, LOCATION,
    },
};
use oxplay_core::MediaTrack;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use url::Url;

#[derive(Clone)]
pub struct NetworkConfig {
    #[cfg(target_os = "macos")]
    dns_helper: Arc<std::path::PathBuf>,
}
impl NetworkConfig {
    /// Pure path validation only. The host validates the executable before UI startup.
    pub fn new(dns_helper: Option<std::path::PathBuf>) -> Result<Self> {
        if dns_helper.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err(Error::Policy);
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                dns_helper: Arc::new(dns_helper.ok_or(Error::Policy)?),
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self {})
        }
    }
}

const RANGE_BYTES: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(20);
type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Policy,
    DnsConfiguration,
    HttpStatus(u16),
    Cancelled,
    AccessRevoked,
    Timeout,
    Transport,
    InvalidResponse,
    Unsupported,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::HttpStatus(status) = self {
            return write!(
                f,
                "Media server returned HTTP {status}; a valid partial range is required."
            );
        }
        f.write_str(match self {
            Self::HttpStatus(_) => unreachable!(),
            Self::Policy => "Media request was rejected by network policy.",
            Self::DnsConfiguration => {
                "The system DNS configuration could not initialize the media resolver."
            }
            Self::Cancelled => "Media request cancelled.",
            Self::AccessRevoked => "Media access was revoked; reconnect or select another video.",
            Self::Timeout => "Media request timed out.",
            Self::Transport => "Media connection failed.",
            Self::InvalidResponse => "Media server returned an inconsistent range.",
            Self::Unsupported => "Media server does not support the required range transport.",
        })
    }
}
impl std::error::Error for Error {}

/// Nonsecret authorization for already-resolved signed CDN media. Implementors
/// carry no cookies, headers or addresses. Invalidity must be permanent for each
/// lease; a new authorization requires a new lease/source.
pub trait AccessLease: Send + Sync {
    /// Cheap, nonblocking validity check, including known session expiry.
    fn is_valid(&self) -> bool;
    /// Level-triggered, event-driven revocation notification. Register the wake
    /// listener BEFORE checking validity; an already-invalid lease must complete
    /// even if invalidation preceded construction/first polling of this future.
    /// Known expiry must wake this future without a polling/background service.
    fn revoked(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

#[derive(Default)]
pub struct Cancellation {
    cancelled: AtomicBool,
    wake: Notify,
}
impl Cancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        // One reader owns this handle. notify_one stores a permit if its
        // current request has not yet polled the cancellation future.
        self.wake.notify_one();
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
pub struct HttpSource {
    url: Url,
    headers: HeaderMap,
    timing: Option<Arc<timing::Stats>>,
    config: NetworkConfig,
    access: Option<Arc<dyn AccessLease>>,
}
impl fmt::Debug for HttpSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HttpSource([redacted])")
    }
}
impl HttpSource {
    pub fn guest(track: &MediaTrack, config: &NetworkConfig) -> Result<Self> {
        Self::with_access(track, config, None)
    }
    /// Explicitly authorized signed CDN URL only. This applies exactly the guest
    /// URL/header/redirect/SSRF policy; it never forwards account cookies.
    pub fn authorized(
        track: &MediaTrack,
        config: &NetworkConfig,
        access: Arc<dyn AccessLease>,
    ) -> Result<Self> {
        Self::with_access(track, config, Some(access))
    }
    fn with_access(
        track: &MediaTrack,
        config: &NetworkConfig,
        access: Option<Arc<dyn AccessLease>>,
    ) -> Result<Self> {
        let url = policy::media_url(track.url.expose_url())?;
        let headers = policy::headers(&url, &track.headers)?;
        let source = Self {
            url,
            headers,
            timing: timing::Stats::configured(),
            config: config.clone(),
            access,
        };
        source.check_access()?;
        Ok(source)
    }
    fn check_access(&self) -> Result<()> {
        if self.access.as_ref().is_some_and(|lease| !lease.is_valid()) {
            Err(Error::AccessRevoked)
        } else {
            Ok(())
        }
    }
    async fn revoked(&self) {
        match &self.access {
            Some(access) => access.revoked().await,
            None => std::future::pending().await,
        }
    }
    /// Preferred adapter entry point: reject a revoked lease before admitting a
    /// new engine reader. Both checks are local; no DNS/socket/runtime work.
    pub fn open_checked(&self) -> Result<(HttpReader, Arc<Cancellation>)> {
        self.check_access()?;
        let opened = self.open();
        self.check_access()?;
        Ok(opened)
    }
    /// Allocation only. No DNS, file, socket, or runtime initialization occurs.
    pub fn open(&self) -> (HttpReader, Arc<Cancellation>) {
        let cancel = Arc::new(Cancellation::default());
        let mut reader = HttpReader {
            source: self.clone(),
            cancel: cancel.clone(),
            session: None,
            position: 0,
            total: None,
            identity: None,
            buffer: Vec::new(),
            buffer_start: 0,
            access_revoked: false,
        };
        // Compatibility API still produces no usable reader for revoked access.
        let _ = reader.check();
        (reader, cancel)
    }
}

struct Session {
    client: Client,
    runtime: tokio::runtime::Runtime,
}
#[cfg(not(target_os = "macos"))]
struct PublicDns(hickory_resolver::TokioResolver);
#[cfg(not(target_os = "macos"))]
impl reqwest::dns::Resolve for PublicDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = self.0.clone();
        let name = name.as_str().to_owned();
        Box::pin(async move {
            // Absolute DNS name avoids resolver search-domain expansion.
            let lookup = resolver.lookup_ip(format!("{name}.")).await?;
            let addresses: Vec<_> = lookup
                .iter()
                .map(|ip| std::net::SocketAddr::new(ip, 0))
                .collect();
            if addresses.is_empty() || addresses.iter().any(|a| !policy::public_ip(a.ip())) {
                return Err(std::io::Error::other("Media DNS policy rejected the answer").into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
impl Session {
    fn new(config: &NetworkConfig) -> Result<Self> {
        #[cfg(not(target_os = "macos"))]
        let _ = config;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .map_err(|_| Error::Transport)?;
        let client = {
            let _entered = runtime.enter();
            // Read the host's DNS configuration on the demux thread. No public
            // resolver is substituted if system configuration is unavailable.
            #[cfg(not(target_os = "macos"))]
            let resolver = {
                let mut builder = hickory_resolver::TokioResolver::builder_tokio()
                    .map_err(|_| Error::DnsConfiguration)?;
                let options = builder.options_mut();
                options.timeout = Duration::from_secs(3);
                options.attempts = 1;
                options.cache_size = 32;
                options.max_active_requests = 4;
                options.num_concurrent_reqs = 1;
                options.ip_strategy = hickory_resolver::config::LookupIpStrategy::Ipv4AndIpv6;
                let resolver = builder.build().map_err(|_| Error::DnsConfiguration)?;
                PublicDns(resolver)
            };
            #[cfg(target_os = "macos")]
            let resolver = dns_process::PublicDns(config.dns_helper.clone());
            Client::builder()
                .dns_resolver(Arc::new(resolver))
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(8))
                .timeout(DEADLINE)
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(15))
                .user_agent("Oxplay/0.1")
                .build()
                .map_err(|_| Error::Transport)?
        };
        Ok(Self { client, runtime })
    }
}

pub struct HttpReader {
    source: HttpSource,
    cancel: Arc<Cancellation>,
    session: Option<Session>,
    position: u64,
    total: Option<u64>,
    identity: Option<Identity>,
    buffer: Vec<u8>,
    buffer_start: u64,
    access_revoked: bool,
}
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    // Validators belong to the exact representation, not the redirect chain.
    // This may contain a signed address; Identity deliberately has no Debug.
    resource: Url,
    etag: Option<String>,
    modified: Option<String>,
}
struct Range {
    bytes: Vec<u8>,
    total: u64,
    identity: Identity,
}
impl HttpReader {
    fn invalidate(&mut self, revoked: bool) {
        self.access_revoked |= revoked;
        self.cancel.cancel();
        self.buffer.clear();
        self.total = None;
        self.identity = None;
        // Dropping the current-thread runtime cancels any connector/DNS tasks
        // still held by reqwest after its request future was discarded. macOS
        // DNS futures signal their independently supervised helper to kill/reap.
        self.session.take();
    }
    fn check(&mut self) -> Result<()> {
        if self.access_revoked || self.source.check_access().is_err() {
            self.invalidate(true);
            return Err(Error::AccessRevoked);
        }
        if self.cancel.is_cancelled() {
            self.invalidate(false);
            return Err(Error::Cancelled);
        }
        Ok(())
    }
    /// Local cursor change, including mpv's initial seek(0) before it installs
    /// the cancellation callback. Range/length validation happens on read.
    pub fn seek(&mut self, absolute: u64) -> Result<u64> {
        self.check()?;
        if absolute > i64::MAX as u64 {
            return Err(Error::InvalidResponse);
        }
        self.position = absolute;
        self.check()?;
        Ok(absolute)
    }
    pub fn size(&mut self) -> Result<u64> {
        self.check()?;
        if self.total.is_none() {
            self.fetch(0, timing::Trigger::Size)?;
        }
        self.check()?;
        self.total.ok_or(Error::InvalidResponse)
    }
    pub fn read(&mut self, destination: &mut [u8]) -> Result<usize> {
        self.check()?;
        if destination.is_empty() {
            self.check()?;
            return Ok(0);
        }
        if self.total.is_none() {
            self.fetch(0, timing::Trigger::Read)?;
        }
        if self.total.is_some_and(|total| self.position >= total) {
            self.check()?;
            return Ok(0);
        }
        let in_buffer = self
            .position
            .checked_sub(self.buffer_start)
            .and_then(|offset| usize::try_from(offset).ok())
            .filter(|offset| *offset < self.buffer.len());
        let offset = if let Some(offset) = in_buffer {
            offset
        } else {
            self.fetch(self.position, timing::Trigger::Read)?;
            0
        };
        let count = destination
            .len()
            .min(64 * 1024)
            .min(self.buffer.len() - offset);
        if count == 0 {
            return Err(Error::InvalidResponse);
        }
        self.check()?;
        destination[..count].copy_from_slice(&self.buffer[offset..offset + count]);
        // The final validity read is the admission linearization point. A
        // revocation detected during copying returns no bytes and clears output.
        // Bytes already returned to a caller cannot be recalled by this trait.
        if let Err(error) = self.check() {
            destination[..count].fill(0);
            return Err(error);
        }
        self.position += count as u64;
        Ok(count)
    }
    fn fetch(&mut self, start: u64, trigger: timing::Trigger) -> Result<()> {
        // No clocks, counter updates, locks or allocations when disabled.
        if let Some(stats) = &self.source.timing {
            let attempt = stats.begin(trigger);
            let result = self.fetch_inner(start);
            attempt.finish(result.map(|()| self.buffer.len()));
            result
        } else {
            self.fetch_inner(start)
        }
    }
    fn fetch_inner(&mut self, start: u64) -> Result<()> {
        self.check()?;
        if self.session.is_none() {
            self.session = Some(Session::new(&self.source.config)?);
        }
        self.check()?;
        let session = self.session.as_ref().ok_or(Error::Transport)?;
        let request = fetch_range(
            &session.client,
            &self.source,
            start,
            self.total,
            self.identity.as_ref(),
        );
        let outcome = session.runtime.block_on(async {
            let cancelled = self.cancel.wake.notified();
            tokio::pin!(cancelled);
            cancelled.as_mut().enable();
            self.cancel.check()?;
            tokio::select! {
                biased;
                _ = &mut cancelled => Err(Error::Cancelled),
                result = tokio::time::timeout(DEADLINE, request) => result.map_err(|_| Error::Timeout)?,
            }
        });
        if matches!(outcome, Err(Error::AccessRevoked)) {
            self.invalidate(true);
            return Err(Error::AccessRevoked);
        }
        self.check()?;
        let range = outcome?;
        if start == 0 {
            // Only known progressive container signatures enter the demuxer.
            // Manifest/playlist bodies cannot acquire libmpv URL-opening power.
            let mp4 = range.bytes.get(4..8) == Some(b"ftyp");
            let webm = range.bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]);
            if !mp4 && !webm {
                return Err(Error::Unsupported);
            }
        }
        self.total = Some(range.total);
        self.identity = Some(range.identity);
        self.buffer_start = start;
        self.buffer = range.bytes;
        self.check()?;
        Ok(())
    }
}

fn header_text(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Result<Option<String>> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .ok()
                .filter(|value| value.len() <= 1024)
                .map(str::to_owned)
                .ok_or(Error::InvalidResponse)
        })
        .transpose()
}
fn content_range(
    value: &str,
    start: u64,
    requested_end: u64,
    known_total: Option<u64>,
) -> Result<(u64, u64)> {
    let (range, total) = value
        .strip_prefix("bytes ")
        .and_then(|v| v.split_once('/'))
        .ok_or(Error::InvalidResponse)?;
    let (first, last) = range.split_once('-').ok_or(Error::InvalidResponse)?;
    let number = |v: &str| -> Result<u64> {
        if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::InvalidResponse);
        }
        v.parse().map_err(|_| Error::InvalidResponse)
    };
    let (first, last, total) = (number(first)?, number(last)?, number(total)?);
    if first != start
        || last < first
        || last > requested_end
        || total == 0
        || last >= total
        || total > i64::MAX as u64
        || known_total.is_some_and(|known| known != total)
        || last != requested_end.min(total - 1)
    {
        return Err(Error::InvalidResponse);
    }
    Ok((last - first + 1, total))
}
async fn fetch_range(
    client: &Client,
    source: &HttpSource,
    start: u64,
    known_total: Option<u64>,
    known_identity: Option<&Identity>,
) -> Result<Range> {
    fetch_range_with_policy(
        client,
        source,
        start,
        known_total,
        known_identity,
        policy::media_url,
    )
    .await
}
async fn fetch_range_with_policy(
    client: &Client,
    source: &HttpSource,
    start: u64,
    known_total: Option<u64>,
    known_identity: Option<&Identity>,
    validate: fn(&str) -> Result<Url>,
) -> Result<Range> {
    // Poll the level-triggered revocation future before the HTTP future. There
    // is no event-registration/check gap even when revocation and a completed
    // response are simultaneously ready. Guest work uses an allocation-free
    // pending future and the exact same request policy.
    let revoked = source.revoked();
    tokio::pin!(revoked);
    source.check_access()?;
    let work = fetch_authorized_range(client, source, start, known_total, known_identity, validate);
    let outcome = tokio::select! {
        biased;
        _ = &mut revoked => Err(Error::AccessRevoked),
        result = work => result,
    };
    source.check_access()?;
    outcome
}
async fn fetch_authorized_range(
    client: &Client,
    source: &HttpSource,
    start: u64,
    known_total: Option<u64>,
    known_identity: Option<&Identity>,
    validate: fn(&str) -> Result<Url>,
) -> Result<Range> {
    let end = start
        .checked_add(RANGE_BYTES as u64 - 1)
        .ok_or(Error::InvalidResponse)?;
    let mut target = source.url.clone();
    for hop in 0..=3 {
        source.check_access()?;
        let mut request = client
            .get(target.clone())
            .header("Range", format!("bytes={start}-{end}"))
            .header("Accept-Encoding", "identity");
        // Re-evaluate every hop. No original-origin field crosses an origin
        // boundary, including noncredential extractor headers.
        if target.origin() == source.url.origin() {
            request = request.headers(source.headers.clone());
        }
        if let Some(identity) = known_identity.filter(|identity| identity.resource == target) {
            if let Some(etag) = identity.etag.as_ref().filter(|v| !v.starts_with("W/")) {
                request = request.header("If-Range", etag);
            } else if let Some(modified) = &identity.modified {
                request = request.header("If-Range", modified);
            }
        }
        let mut response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                Error::Timeout
            } else {
                Error::Transport
            }
        })?;
        source.check_access()?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop == 3 {
                return Err(Error::Policy);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(Error::InvalidResponse)?;
            let next = target.join(location).map_err(|_| Error::Policy)?;
            target = validate(next.as_str())?;
            source.check_access()?;
            continue;
        }
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
        {
            return Err(Error::InvalidResponse);
        }
        let value = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .ok_or(Error::InvalidResponse)?;
        let (length, total) = content_range(value, start, end, known_total)?;
        if let Some(value) = response.headers().get(CONTENT_LENGTH)
            && value.to_str().ok().and_then(|v| v.parse::<u64>().ok()) != Some(length)
        {
            return Err(Error::InvalidResponse);
        }
        let identity = Identity {
            resource: target,
            etag: header_text(response.headers(), ETAG)?,
            modified: header_text(response.headers(), LAST_MODIFIED)?,
        };
        if known_identity.is_some_and(|known| known != &identity) {
            return Err(Error::InvalidResponse);
        }
        let mut bytes = Vec::with_capacity(length as usize);
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
            source.check_access()?;
            if chunk.len() > length as usize - bytes.len() {
                return Err(Error::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != length as usize {
            return Err(Error::InvalidResponse);
        }
        source.check_access()?;
        return Ok(Range {
            bytes,
            total,
            identity,
        });
    }
    Err(Error::Policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_ranges_never_accept_wrong_offsets_or_changed_representation_size() {
        assert_eq!(
            content_range("bytes 0-99/100", 0, 1023, None),
            Ok((100, 100))
        );
        for value in [
            "bytes 1-99/100",
            "bytes 0-98/100",
            "bytes 0-100/100",
            "bytes 0-99/*",
            "bytes 0-99/0",
            "bytes +0-99/100",
            "items 0-99/100",
        ] {
            assert!(content_range(value, 0, 1023, None).is_err());
        }
        assert!(content_range("bytes 0-99/100", 0, 1023, Some(101)).is_err());
    }
    #[test]
    fn initial_open_and_seek_are_offline_and_cancel_is_permanent() {
        let source = HttpSource {
            url: policy::media_url("https://synthetic.googlevideo.com/videoplayback").unwrap(),
            headers: HeaderMap::new(),
            timing: None,
            access: None,
            config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
        };
        let (mut reader, cancel) = source.open();
        assert!(reader.session.is_none());
        assert_eq!(reader.seek(0), Ok(0));
        assert_eq!(reader.seek(42), Ok(42));
        assert!(reader.session.is_none());
        cancel.cancel();
        assert_eq!(reader.read(&mut [0; 1]), Err(Error::Cancelled));
        assert_eq!(reader.size(), Err(Error::Cancelled));
        assert_eq!(reader.seek(0), Err(Error::Cancelled));
        assert!(reader.session.is_none());
        assert!(!format!("{source:?}").contains("synthetic"));
    }
    pub(super) fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    pub(super) fn fixture_policy(value: &str) -> Result<Url> {
        let url = Url::parse(value).map_err(|_| Error::Policy)?;
        if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
            return Err(Error::Policy);
        }
        Ok(url)
    }
    pub(super) fn fixture_server(
        reply: impl FnOnce(String) -> Vec<u8> + Send + 'static,
    ) -> (Url, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/media", listener.local_addr().unwrap())).unwrap();
        listener.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("local fixture accept failed: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0 && request.len() + count <= 16 * 1024);
                request.extend_from_slice(&buffer[..count]);
            }
            let response = reply(String::from_utf8(request).unwrap());
            let _ = socket.write_all(&response);
        });
        (url, thread)
    }
    pub(super) fn partial(body: &[u8], range: &str, extra: &str) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: {range}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len()).into_bytes();
        response.extend_from_slice(body);
        response
    }
    #[test]
    fn local_ranges_validate_body_and_strip_headers_after_cross_origin_redirect() {
        let (destination, final_thread) = fixture_server(|request| {
            let request = request.to_ascii_lowercase();
            assert!(!request.contains("synthetic-origin-only"));
            assert!(request.contains("range: bytes=0-1048575"));
            partial(b"synthetic bytes", "bytes 0-14/15", "")
        });
        let (origin, first_thread) = fixture_server(move |request| {
            assert!(request.contains("synthetic-origin-only"));
            format!("HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
        });
        let mut headers = HeaderMap::new();
        headers.insert("accept-language", "synthetic-origin-only".parse().unwrap());
        let source = HttpSource {
            url: origin,
            headers,
            timing: None,
            access: None,
            config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
        };
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        let result = runtime()
            .block_on(fetch_range_with_policy(
                &client,
                &source,
                0,
                None,
                None,
                fixture_policy,
            ))
            .unwrap();
        assert_eq!(result.bytes, b"synthetic bytes");
        assert_eq!(result.total, 15);
        first_thread.join().unwrap();
        final_thread.join().unwrap();
    }
    #[test]
    fn local_wrong_range_and_changed_identity_are_errors_not_eof() {
        for (range, changed_identity) in [("bytes 1-14/15", false), ("bytes 0-14/15", true)] {
            let (url, thread) =
                fixture_server(move |_| partial(b"synthetic bytes", range, "ETag: \"after\"\r\n"));
            let known = changed_identity.then(|| Identity {
                resource: url.clone(),
                etag: Some("\"before\"".into()),
                modified: None,
            });
            let source = HttpSource {
                url,
                headers: HeaderMap::new(),
                timing: None,
                access: None,
                config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
            };
            let result = runtime().block_on(fetch_range_with_policy(
                &Client::builder().no_proxy().build().unwrap(),
                &source,
                0,
                None,
                known.as_ref(),
                fixture_policy,
            ));
            assert!(matches!(result, Err(Error::InvalidResponse)));
            thread.join().unwrap();
        }
    }
    #[test]
    fn range_validator_is_sent_only_to_its_exact_final_resource() {
        let (destination, final_thread) = fixture_server(|request| {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("if-range: \"fixture-etag\"")
            );
            partial(
                b"synthetic bytes",
                "bytes 0-14/15",
                "ETag: \"fixture-etag\"\r\n",
            )
        });
        let identity = Identity {
            resource: destination.clone(),
            etag: Some("\"fixture-etag\"".into()),
            modified: None,
        };
        let (origin, first_thread) = fixture_server(move |request| {
            assert!(!request.to_ascii_lowercase().contains("if-range:"));
            format!("HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
        });
        let source = HttpSource {
            url: origin,
            headers: HeaderMap::new(),
            timing: None,
            access: None,
            config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
        };
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let result = runtime().block_on(fetch_range_with_policy(
            &client,
            &source,
            0,
            Some(15),
            Some(&identity),
            fixture_policy,
        ));
        assert!(result.is_ok());
        first_thread.join().unwrap();
        final_thread.join().unwrap();
    }
    #[test]
    fn excess_truncated_encoded_and_html_responses_do_not_become_media() {
        let responses = [
            partial(b"synthetic bytes!", "bytes 0-14/15", ""),
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-14/15\r\nContent-Length: 15\r\nConnection: close\r\n\r\nshort".to_vec(),
            partial(b"synthetic bytes", "bytes 0-14/15", "Content-Encoding: gzip\r\n"),
            partial(b"<html>no</html>", "bytes 0-14/15", ""),
        ];
        for response in responses {
            let (url, thread) = fixture_server(move |_| response);
            let source = HttpSource {
                url,
                headers: HeaderMap::new(),
                timing: None,
                access: None,
                config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
            };
            let (mut reader, _) = source.open();
            reader.session = Some(Session {
                client: Client::builder().no_proxy().build().unwrap(),
                runtime: runtime(),
            });
            assert!(reader.read(&mut [0; 8]).is_err());
            assert_eq!(reader.position, 0);
            thread.join().unwrap();
        }
    }
    #[test]
    fn redirect_to_an_unapproved_origin_stops_before_the_next_request() {
        let (url, thread) = fixture_server(|_| {
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        });
        let source = HttpSource {
            url,
            headers: HeaderMap::new(),
            timing: None,
            access: None,
            config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
        };
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert!(matches!(
            runtime().block_on(fetch_range(&client, &source, 0, None, None)),
            Err(Error::Policy)
        ));
        thread.join().unwrap();
    }
    #[test]
    fn compressed_reads_are_short_and_seeks_reuse_only_the_covered_range() {
        let source = HttpSource {
            url: policy::media_url("https://synthetic.googlevideo.com/videoplayback").unwrap(),
            headers: HeaderMap::new(),
            timing: None,
            access: None,
            config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
        };
        let (mut reader, _) = source.open();
        reader.total = Some(100_000);
        reader.buffer = vec![42; 100_000];
        let mut destination = vec![0; 200_000];
        assert_eq!(reader.read(&mut destination), Ok(64 * 1024));
        assert_eq!(reader.read(&mut destination), Ok(100_000 - 64 * 1024));
        assert_eq!(reader.read(&mut destination), Ok(0));
        reader.seek(16).unwrap();
        assert_eq!(reader.read(&mut destination[..10]), Ok(10));
        assert_eq!(&destination[..10], &[42; 10]);
        assert!(reader.session.is_none());
    }
    #[test]
    fn cancel_interrupts_blocked_response_without_waiting_for_socket_timeout() {
        let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let (url, thread) = fixture_server(move |_| {
            accepted_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
            partial(b"synthetic bytes", "bytes 0-14/15", "")
        });
        let source = HttpSource {
            url,
            headers: HeaderMap::new(),
            timing: None,
            access: None,
            config: NetworkConfig::new(Some("/synthetic/oxplay-dns".into())).unwrap(),
        };
        let (mut reader, cancel) = source.open();
        reader.session = Some(Session {
            client: Client::builder().no_proxy().build().unwrap(),
            runtime: runtime(),
        });
        let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
        let read_thread = std::thread::spawn(move || {
            result_tx.send(reader.read(&mut [0; 8])).unwrap();
            assert_eq!(reader.read(&mut [0; 8]), Err(Error::Cancelled));
        });
        accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        cancel.cancel();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(Error::Cancelled)
        );
        release_tx.send(()).unwrap();
        read_thread.join().unwrap();
        thread.join().unwrap();
    }
}
