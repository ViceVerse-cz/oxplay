// SPDX-License-Identifier: GPL-3.0-or-later
//! Short-lived DNS-SD isolation. No native DNS calls execute in the app process.
use crate::policy;
use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr},
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
const MAX_REPLY: usize = 2048;
const DEADLINE: Duration = Duration::from_secs(3);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
pub(crate) struct PublicDns(pub Arc<PathBuf>);
impl reqwest::dns::Resolve for PublicDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let helper = self.0.clone();
        let hostname = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = resolve(&helper, hostname).await?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
fn failure(kind: io::ErrorKind, text: &'static str) -> io::Error {
    io::Error::new(kind, text)
}
fn hostname_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
}
struct Permit;
impl Permit {
    fn acquire() -> io::Result<Self> {
        ACTIVE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .map(|_| Self)
            .map_err(|_| {
                failure(
                    io::ErrorKind::WouldBlock,
                    "Media DNS helper capacity is busy",
                )
            })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Cancel(UnixStream);
impl Drop for Cancel {
    fn drop(&mut self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}
async fn resolve(helper: &Path, hostname: String) -> io::Result<Vec<SocketAddr>> {
    if !helper.is_absolute() || !hostname_valid(&hostname) {
        return Err(failure(
            io::ErrorKind::InvalidInput,
            "Invalid media DNS configuration",
        ));
    }
    let permit = Permit::acquire()?;
    let (cancel, receiver) = UnixStream::pair()
        .map_err(|_| failure(io::ErrorKind::Other, "DNS cancellation channel failed"))?;
    let cancel = Cancel(cancel);
    let helper = helper.to_owned();
    let (sender, reply) = tokio::sync::oneshot::channel();
    // The owning supervisor survives cancellation/drop of a current-thread
    // Tokio runtime. At most four bounded threads/children exist, with no queue.
    std::thread::Builder::new()
        .name("serein-dns-owner".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let result = supervise(&helper, &hostname, receiver, DEADLINE);
            drop(permit); // supervise has already killed/reaped its child
            let _ = sender.send(result);
        })
        .map_err(|_| failure(io::ErrorKind::Other, "DNS supervisor unavailable"))?;
    let result = reply
        .await
        .map_err(|_| failure(io::ErrorKind::Other, "DNS supervisor stopped"))?;
    drop(cancel);
    result
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // Always reap the exact owned child, including protocol errors/panics.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn supervise(
    helper: &Path,
    hostname: &str,
    cancel: UnixStream,
    budget: Duration,
) -> io::Result<Vec<SocketAddr>> {
    let deadline = Instant::now() + budget;
    check_cancelled(&cancel)?;
    let child = Command::new(helper)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| failure(io::ErrorKind::Other, "DNS helper could not start"))?;
    let mut child = OwnedChild(child);
    check_cancelled(&cancel)?;
    let mut input = child
        .0
        .stdin
        .take()
        .ok_or_else(|| failure(io::ErrorKind::Other, "DNS input unavailable"))?;
    // <=254 bytes fits an empty pipe; never includes a URL or credentials.
    input
        .write_all(hostname.as_bytes())
        .and_then(|()| input.write_all(b"\n"))
        .map_err(|_| failure(io::ErrorKind::Other, "DNS input failed"))?;
    drop(input);
    let mut output = child
        .0
        .stdout
        .take()
        .ok_or_else(|| failure(io::ErrorKind::Other, "DNS output unavailable"))?;
    let fd = output.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(failure(io::ErrorKind::Other, "DNS pipe setup failed"));
    }
    let mut bytes = Vec::with_capacity(MAX_REPLY);
    let mut eof = false;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| failure(io::ErrorKind::TimedOut, "Media DNS helper timed out"))?;
        let mut fds = [
            libc::pollfd {
                fd: cancel.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if eof { -1 } else { fd },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ms = remaining
            .as_millis()
            .max(1)
            .min(if eof { 10 } else { i32::MAX as u128 }) as i32;
        let polled = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
        if polled < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(failure(io::ErrorKind::Other, "DNS readiness failed"));
        }
        if fds[0].revents != 0 {
            return Err(failure(io::ErrorKind::Interrupted, "Media DNS cancelled"));
        }
        if fds[1].revents != 0 {
            let mut buffer = [0; 256];
            loop {
                match output.read(&mut buffer) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(n) if bytes.len() + n <= MAX_REPLY => bytes.extend_from_slice(&buffer[..n]),
                    Ok(_) => {
                        return Err(failure(
                            io::ErrorKind::InvalidData,
                            "DNS reply limit exceeded",
                        ));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(failure(io::ErrorKind::Other, "DNS output failed")),
                }
            }
        }
        if eof
            && let Some(status) = child
                .0
                .try_wait()
                .map_err(|_| failure(io::ErrorKind::Other, "DNS child status failed"))?
        {
            if !status.success() {
                return Err(failure(io::ErrorKind::Other, "Native DNS lookup failed"));
            }
            return decode(&bytes);
        }
    }
}
fn check_cancelled(cancel: &UnixStream) -> io::Result<()> {
    let mut fd = libc::pollfd {
        fd: cancel.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut fd, 1, 0) };
        if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if result < 0 {
            return Err(failure(
                io::ErrorKind::Other,
                "DNS cancellation readiness failed",
            ));
        }
        if fd.revents != 0 {
            return Err(failure(io::ErrorKind::Interrupted, "Media DNS cancelled"));
        }
        return Ok(());
    }
}
fn encode(addresses: &[SocketAddr]) -> io::Result<Vec<u8>> {
    if addresses.is_empty() || addresses.len() > 64 {
        return Err(failure(
            io::ErrorKind::InvalidData,
            "DNS address count invalid",
        ));
    }
    let mut bytes = b"SDN1".to_vec();
    bytes.push(addresses.len() as u8);
    for address in addresses {
        if !policy::public_ip(address.ip()) {
            return Err(failure(io::ErrorKind::InvalidData, "DNS address rejected"));
        }
        match address.ip() {
            IpAddr::V4(ip) => {
                bytes.push(4);
                bytes.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                bytes.push(6);
                bytes.extend_from_slice(&ip.octets());
            }
        }
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> io::Result<Vec<SocketAddr>> {
    let invalid = || failure(io::ErrorKind::InvalidData, "Invalid DNS helper reply");
    if bytes.len() < 5 || &bytes[..4] != b"SDN1" || !(1..=64).contains(&bytes[4]) {
        return Err(invalid());
    }
    let mut remaining = &bytes[5..];
    let mut addresses = Vec::with_capacity(bytes[4] as usize);
    for _ in 0..bytes[4] {
        let Some((&kind, data)) = remaining.split_first() else {
            return Err(invalid());
        };
        let (ip, length) = match kind {
            4 if data.len() >= 4 => (
                IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(&data[..4]).unwrap())),
                4,
            ),
            6 if data.len() >= 16 => (
                IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&data[..16]).unwrap())),
                16,
            ),
            _ => return Err(invalid()),
        };
        if !policy::public_ip(ip) {
            return Err(invalid());
        }
        let address = SocketAddr::new(ip, 0);
        if addresses.contains(&address) {
            return Err(invalid());
        }
        addresses.push(address);
        remaining = &data[length..];
    }
    if !remaining.is_empty() {
        return Err(invalid());
    }
    Ok(addresses)
}

/// Dedicated binary entry point. No configuration, UI, account, or media paths.
pub fn run_helper() -> i32 {
    if start_watchdog().is_err() {
        return 2;
    }
    let mut request = Vec::with_capacity(254);
    if std::io::stdin()
        .take(255)
        .read_to_end(&mut request)
        .is_err()
        || request.last() != Some(&b'\n')
    {
        return 2;
    }
    request.pop();
    let Ok(hostname) = std::str::from_utf8(&request) else {
        return 2;
    };
    if !hostname_valid(hostname) {
        return 2;
    }
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return 2;
    };
    let Ok(addresses) = runtime.block_on(crate::dns_macos::resolve_host(hostname)) else {
        return 2;
    };
    let Ok(reply) = encode(&addresses) else {
        return 2;
    };
    if std::io::stdout().write_all(&reply).is_err() {
        return 2;
    }
    0
}
fn start_watchdog() -> io::Result<()> {
    let parent = unsafe { libc::getppid() };
    if parent <= 1 {
        return Err(failure(io::ErrorKind::Other, "DNS parent unavailable"));
    }
    let queue = unsafe { libc::kqueue() };
    if queue < 0 {
        return Err(io::Error::last_os_error());
    }
    let change = libc::kevent {
        ident: parent as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    let result =
        unsafe { libc::kevent(queue, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if result < 0 || unsafe { libc::getppid() } != parent {
        unsafe {
            libc::close(queue);
        }
        return Err(failure(io::ErrorKind::Other, "DNS parent watch failed"));
    }
    std::thread::Builder::new()
        .name("serein-dns-watch".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(2800);
            loop {
                let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                    unsafe { libc::_exit(3) }
                };
                let timeout = libc::timespec {
                    tv_sec: left.as_secs() as libc::time_t,
                    tv_nsec: left.subsec_nanos() as libc::c_long,
                };
                let mut event: libc::kevent = unsafe { std::mem::zeroed() };
                let result =
                    unsafe { libc::kevent(queue, std::ptr::null(), 0, &mut event, 1, &timeout) };
                if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                // Exit entire isolated process even if native initialization is blocked.
                unsafe { libc::_exit(3) }
            }
        })
        .map_err(|_| {
            unsafe {
                libc::close(queue);
            }
            failure(io::ErrorKind::Other, "DNS watchdog unavailable")
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, sync::Mutex};
    static SERIAL: Mutex<()> = Mutex::new(());
    struct Fixture(PathBuf);
    impl Fixture {
        fn new(body: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "serein-dns-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&path).unwrap();
            let pid = path.join("pid").to_string_lossy().replace('\'', "'\\''");
            std::fs::write(
                path.join("helper"),
                format!("#!/bin/sh\nprintf '%s\\n' \"$$\" > '{pid}'\n{body}\n"),
            )
            .unwrap();
            std::fs::set_permissions(path.join("helper"), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            Self(path)
        }
        fn helper(&self) -> PathBuf {
            self.0.join("helper")
        }
        fn published_pid(&self) -> Option<i32> {
            let file = std::fs::File::open(self.0.join("pid")).ok()?;
            let mut contents = String::new();
            file.take(17).read_to_string(&mut contents).ok()?;
            // Opening/truncating the file is not readiness. A partial decimal
            // prefix could even parse as another live PID; the immutable final
            // newline is the fixture's publication boundary.
            let digits = contents.strip_suffix('\n')?;
            if digits.is_empty() || digits.len() > 10 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            digits.parse::<i32>().ok().filter(|pid| *pid > 1)
        }
        fn assert_reaped(&self) {
            let pid = self
                .published_pid()
                .expect("fixture must publish its complete PID before cancellation/reply");
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn timeout_and_malformed_oversized_replies_kill_and_reap_owned_children() {
        let _serial = SERIAL.lock().unwrap();
        for (body, expected) in [
            ("exec /bin/sleep 30", io::ErrorKind::TimedOut),
            ("printf synthetic-private-name", io::ErrorKind::InvalidData),
            ("exec /usr/bin/yes", io::ErrorKind::InvalidData),
        ] {
            let fixture = Fixture::new(body);
            let (_hold, cancel) = UnixStream::pair().unwrap();
            let now = Instant::now();
            let error = supervise(
                &fixture.helper(),
                "example.com",
                cancel,
                Duration::from_secs(1),
            )
            .unwrap_err();
            assert_eq!(error.kind(), expected);
            assert!(now.elapsed() < Duration::from_secs(3));
            assert!(!error.to_string().contains("synthetic-private"));
            fixture.assert_reaped();
        }
    }
    #[test]
    fn cancelled_future_reaps_even_after_its_tokio_runtime_is_dropped() {
        let _serial = SERIAL.lock().unwrap();
        let fixture = Fixture::new("exec /bin/sleep 30");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let helper = fixture.helper();
            let request = resolve(&helper, "example.com".to_owned());
            tokio::pin!(request);
            tokio::select! {
                _ = &mut request => panic!("fixture should wait"),
                _ = async {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while fixture.published_pid().is_none() {
                        assert!(Instant::now() < deadline, "fixture did not publish a complete PID");
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                } => {}
            }
        });
        drop(runtime);
        let deadline = Instant::now() + Duration::from_secs(2);
        while ACTIVE.load(Ordering::Acquire) != 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        fixture.assert_reaped();
    }
    #[test]
    fn fixture_readiness_requires_complete_pid_not_existence_or_decimal_prefix() {
        let fixture = Fixture::new("exit 0");
        assert!(fixture.published_pid().is_none());
        for incomplete in [
            "",
            "12",
            "12345",
            "0\n",
            "1\n",
            "12\n34\n",
            "999999999999999999999\n",
        ] {
            std::fs::write(fixture.0.join("pid"), incomplete).unwrap();
            assert!(
                fixture.published_pid().is_none(),
                "invalid fixture readiness accepted"
            );
        }
        std::fs::write(fixture.0.join("pid"), b"12345\n").unwrap();
        assert_eq!(fixture.published_pid(), Some(12345));
    }
    #[test]
    fn admission_is_bounded_without_queue_and_protocol_is_strict() {
        let _serial = SERIAL.lock().unwrap();
        let permits: Vec<_> = (0..4).map(|_| Permit::acquire().unwrap()).collect();
        assert!(
            matches!(Permit::acquire(), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
        );
        drop(permits);
        assert_eq!(ACTIVE.load(Ordering::Acquire), 0);
        let addresses = [
            "8.8.8.8:0".parse().unwrap(),
            "[2606:4700:4700::1111]:0".parse().unwrap(),
        ];
        assert_eq!(decode(&encode(&addresses).unwrap()).unwrap(), addresses);
        for bytes in [
            b"SDN1\0".as_slice(),
            b"SDN1\x01\x04\x7f\0\0\x01",
            b"SDN1\x01\x04\x08\x08\x08\x08extra",
        ] {
            assert!(decode(bytes).is_err());
        }
        assert!(encode(&vec![addresses[0]; 65]).is_err());
        assert!(!hostname_valid("https://name/path?secret"));
    }
    #[test]
    fn cancellation_before_supervisor_starts_does_not_spawn_a_helper() {
        let fixture = Fixture::new("exec /bin/sleep 30");
        let (sender, cancel) = UnixStream::pair().unwrap();
        drop(sender);
        let result = supervise(&fixture.helper(), "example.com", cancel, DEADLINE);
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::Interrupted));
        assert!(!fixture.0.join("pid").exists());
    }
}
