// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthetic authorization only. No real cookies, accounts, or external requests.
use super::*;
use crate::tests::{fixture_policy, fixture_server, partial, runtime};
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Lease {
    invalid: AtomicBool,
    wake: Notify,
    revoke_on_check: AtomicUsize,
}
impl Lease {
    fn revoke(&self) {
        self.invalid.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
}
impl AccessLease for Lease {
    fn is_valid(&self) -> bool {
        if self
            .revoke_on_check
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            == Ok(1)
        {
            self.revoke();
        }
        !self.invalid.load(Ordering::Acquire)
    }
    fn revoked(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let wake = self.wake.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            if self.is_valid() {
                wake.await;
            }
        })
    }
}
fn source(lease: Arc<Lease>) -> HttpSource {
    HttpSource {
        url: policy::media_url(
            "https://synthetic.googlevideo.com/videoplayback?token=synthetic-secret",
        )
        .unwrap(),
        headers: HeaderMap::new(),
        timing: None,
        config: NetworkConfig::new(Some("/synthetic/serein-dns".into())).unwrap(),
        access: Some(lease),
    }
}
fn buffered(lease: Arc<Lease>) -> HttpReader {
    let (mut reader, _) = source(lease).open_checked().unwrap();
    reader.total = Some(16);
    reader.buffer = vec![42; 16];
    reader
}
#[test]
fn revoked_access_blocks_open_cached_read_seek_and_size_without_io() {
    let lease = Arc::new(Lease::default());
    let mut reader = buffered(lease.clone());
    let source = reader.source.clone();
    lease.revoke();
    assert!(matches!(source.open_checked(), Err(Error::AccessRevoked)));
    let (mut late, cancel) = source.open();
    assert!(cancel.is_cancelled());
    assert_eq!(late.seek(0), Err(Error::AccessRevoked));
    assert_eq!(reader.read(&mut [0; 8]), Err(Error::AccessRevoked));
    assert_eq!(reader.seek(0), Err(Error::AccessRevoked));
    assert_eq!(reader.size(), Err(Error::AccessRevoked));
    assert!(reader.buffer.is_empty());
    assert!(reader.session.is_none());
    assert!(!format!("{source:?} {}", Error::AccessRevoked).contains("synthetic-secret"));
}
#[test]
fn revocation_detected_after_cached_copy_wipes_output_and_returns_no_bytes() {
    let lease = Arc::new(Lease::default());
    let mut reader = buffered(lease.clone());
    // Admission, immediately before copy, and immediately after copy.
    lease.revoke_on_check.store(3, Ordering::Release);
    let mut destination = [99; 8];
    assert_eq!(reader.read(&mut destination), Err(Error::AccessRevoked));
    assert_eq!(destination, [0; 8]);
    assert_eq!(reader.position, 0);
    assert!(reader.buffer.is_empty());
}
#[test]
fn authorized_constructor_preserves_no_credentials_and_origin_policies() {
    let lease = Arc::new(Lease::default());
    let source = source(lease.clone());
    let mut track = MediaTrack {
        url: serein_core::MediaUrl::parse(source.url.as_str()).unwrap(),
        codec: None,
        width: None,
        height: None,
        fps: None,
        contains_audio: true,
        headers: serein_core::OriginHeaders {
            origin: source.url.origin().ascii_serialization(),
            fields: vec![],
        },
    };
    assert!(HttpSource::authorized(&track, &source.config, lease.clone()).is_ok());
    for field in ["Cookie", "Authorization", "Proxy-Authorization", "Host"] {
        track.headers.fields = vec![(field.into(), "synthetic".into())];
        assert!(matches!(
            HttpSource::authorized(&track, &source.config, lease.clone()),
            Err(Error::Policy)
        ));
    }
    track.headers.fields.clear();
    lease.revoke();
    assert!(matches!(
        HttpSource::authorized(&track, &source.config, lease),
        Err(Error::AccessRevoked)
    ));
    assert!(HttpSource::guest(&track, &source.config).is_ok());
}
#[test]
fn pre_poll_revocation_is_level_triggered_and_cannot_start_http() {
    let lease = Arc::new(Lease::default());
    let source = source(lease.clone());
    let client = Client::builder().no_proxy().build().unwrap();
    let request = fetch_range(&client, &source, 0, None, None);
    lease.revoke();
    assert!(matches!(
        runtime().block_on(request),
        Err(Error::AccessRevoked)
    ));
}
#[test]
fn revocation_interrupts_a_blocked_response_and_drops_the_session() {
    let lease = Arc::new(Lease::default());
    let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let (url, server) = fixture_server(move |_| {
        accepted_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
        partial(b"\0\0\0\x10ftypisom1234", "bytes 0-15/16", "")
    });
    let mut source = source(lease.clone());
    source.url = url;
    let (mut reader, _) = source.open_checked().unwrap();
    reader.session = Some(Session {
        client: Client::builder().no_proxy().build().unwrap(),
        runtime: runtime(),
    });
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = reader.read(&mut [0; 8]);
        assert!(reader.session.is_none());
        assert!(reader.buffer.is_empty());
        done_tx.send(result).unwrap();
    });
    accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    lease.revoke();
    assert_eq!(
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        Err(Error::AccessRevoked)
    );
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    server.join().unwrap();
}
#[test]
fn revocation_before_redirect_never_contacts_the_next_origin() {
    let lease = Arc::new(Lease::default());
    let destination = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let address = destination.local_addr().unwrap();
    let invalidate = lease.clone();
    let (url, server) = fixture_server(move |_| {
        invalidate.revoke();
        format!("HTTP/1.1 302 Found\r\nLocation: http://{address}/media\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
    });
    let mut source = source(lease);
    source.url = url;
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    assert!(matches!(
        runtime().block_on(fetch_range_with_policy(
            &client,
            &source,
            0,
            None,
            None,
            fixture_policy
        )),
        Err(Error::AccessRevoked)
    ));
    server.join().unwrap();
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
struct DnsDrop(Arc<AtomicBool>);
impl Drop for DnsDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct PendingDns {
    started: std::sync::mpsc::SyncSender<()>,
    dropped: Arc<AtomicBool>,
}
impl reqwest::dns::Resolve for PendingDns {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let started = self.started.clone();
        let dropped = self.dropped.clone();
        Box::pin(async move {
            let _drop = DnsDrop(dropped);
            started.send(()).unwrap();
            std::future::pending().await
        })
    }
}
#[test]
fn revocation_cancels_pending_connector_dns_before_reader_returns() {
    let lease = Arc::new(Lease::default());
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let dropped = Arc::new(AtomicBool::new(false));
    let dns = PendingDns {
        started: started_tx,
        dropped: dropped.clone(),
    };
    let (mut reader, _) = source(lease.clone()).open_checked().unwrap();
    reader.session = Some(Session {
        client: Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(dns))
            .build()
            .unwrap(),
        runtime: runtime(),
    });
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        done_tx.send(reader.read(&mut [0; 8])).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    lease.revoke();
    assert_eq!(
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        Err(Error::AccessRevoked)
    );
    assert!(dropped.load(Ordering::Acquire));
    worker.join().unwrap();
}
