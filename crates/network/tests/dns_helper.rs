// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg(target_os = "macos")]
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::fd::AsRawFd,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
const HELPER: &str = env!("CARGO_BIN_EXE_serein-dns");

struct OwnedHelper(Child);
impl Drop for OwnedHelper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn read_closed_pipe(pipe: impl Read + AsRawFd, limit: u64) -> Vec<u8> {
    // The leader has exited. If an unexpected descendant still owns a writer,
    // fail instead of turning this bounded diagnostic into a blocking read.
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let mut bytes = Vec::new();
    pipe.take(limit).read_to_end(&mut bytes).unwrap();
    bytes
}

/// Exercises the real first-party helper, its watchdog and native resolver,
/// rather than only the in-process DNS-SD primitive. Parent-side supervisor
/// cancellation/protocol rejection has separate deterministic unit coverage.
#[test]
#[ignore = "Explicit production DNS helper smoke: resolves example.com with the system resolver; no HTTP/account"]
fn live_system_dns_helper_returns_bounded_public_protocol() {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    let mut helper = OwnedHelper(
        Command::new(HELPER)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    helper
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(b"example.com\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = helper.0.try_wait().unwrap() {
            assert!(
                status.success(),
                "production DNS helper did not resolve the explicit public fixture"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "production DNS helper exceeded the test deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let bytes = read_closed_pipe(helper.0.stdout.take().unwrap(), 2049);
    let errors = read_closed_pipe(helper.0.stderr.take().unwrap(), 1025);
    assert!(errors.is_empty(), "DNS helper must remain silent");
    assert!((5..=1093).contains(&bytes.len()));
    assert_eq!(&bytes[..4], b"SDN1");
    assert!((1..=64).contains(&bytes[4]));
    let mut remaining = &bytes[5..];
    let mut addresses = Vec::new();
    for _ in 0..bytes[4] {
        let (&family, data) = remaining.split_first().expect("truncated DNS record");
        let (address, length) = match family {
            4 if data.len() >= 4 => (
                IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(&data[..4]).unwrap())),
                4,
            ),
            6 if data.len() >= 16 => (
                IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&data[..16]).unwrap())),
                16,
            ),
            _ => panic!("invalid DNS protocol family/length"),
        };
        assert!(!address.is_loopback() && !address.is_unspecified() && !address.is_multicast());
        if let IpAddr::V4(address) = address {
            assert!(!address.is_private() && !address.is_link_local());
        }
        assert!(!addresses.contains(&address), "duplicate DNS answer");
        addresses.push(address);
        remaining = &data[length..];
    }
    assert!(remaining.is_empty(), "unexpected DNS protocol suffix");
    // Do not log host answers, even though this is an explicit public fixture.
}

#[test]
fn malformed_request_is_offline_and_silent() {
    let mut child = Command::new(HELPER)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"bad hostname\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty() && result.stderr.is_empty());
}

#[test]
fn watchdog_bounds_blocked_initial_input() {
    let mut child = Command::new(HELPER)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _hold_input = child.stdin.take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(3));
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("watchdog did not bound startup");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn parent_death_exits_helper_while_input_remains_open_elsewhere() {
    let command = format!(
        "'{}' <&0 & printf '%s\\n' \"$!\"; wait",
        HELPER.replace('\'', "'\\''")
    );
    let mut parent = Command::new("/bin/sh")
        .args(["-c", &command])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _hold_input = parent.stdin.take().unwrap();
    let mut output = BufReader::new(parent.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let pid: i32 = line.trim().parse().unwrap();
    parent.kill().unwrap();
    parent.wait().unwrap();
    // Keep the input writer alive: EOF cannot explain this helper's exit.
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(pid, 0) } == 0 {
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            panic!("orphaned DNS helper survived parent exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
