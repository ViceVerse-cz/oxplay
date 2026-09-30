use oxplay_core::{OperationContext, ProviderError};
use std::{path::Path, time::Duration};
pub(crate) struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[cfg(test)]
pub(crate) fn run(
    binary: &Path,
    args: &[String],
    operation: &OperationContext,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<Output, ProviderError> {
    run_guarded(
        binary,
        args,
        operation,
        timeout,
        stdout_limit,
        stderr_limit,
        &|| false,
    )
}

#[cfg(not(unix))]
pub(crate) fn run_guarded(
    _: &Path,
    _: &[String],
    _: &OperationContext,
    _: Duration,
    _: usize,
    _: usize,
    _: &dyn Fn() -> bool,
) -> Result<Output, ProviderError> {
    // Fail closed until a Windows job-object supervisor is implemented and tested.
    Err(ProviderError::UnsupportedPlatform)
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_guarded(
    binary: &Path,
    args: &[String],
    operation: &OperationContext,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Output, ProviderError> {
    use std::{
        io::{ErrorKind, Read},
        os::{fd::AsRawFd, unix::process::CommandExt},
        process::{Child, Command, Stdio},
        time::Instant,
    };
    struct OwnedProcess(Child);
    impl Drop for OwnedProcess {
        fn drop(&mut self) {
            // process_group(0) makes this child the leader of an application-owned group.
            // Terminate descendants even if the leader already exited; then reap our child.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }
    fn drain(
        reader: &mut impl Read,
        output: &mut Vec<u8>,
        limit: usize,
    ) -> Result<bool, ProviderError> {
        let mut buffer = [0u8; 8192];
        // A finite drain batch ensures a chatty child cannot starve cancellation checks.
        for _ in 0..16 {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(true),
                Ok(n) => {
                    if output.len() + n > limit {
                        return Err(ProviderError::OutputTooLarge);
                    }
                    output.extend_from_slice(&buffer[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(ProviderError::ExtractorFailed),
            }
        }
        Ok(false)
    }
    if operation.cancel.is_cancelled() || cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let child = Command::new(binary)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "en_US.UTF-8")
        .env("PYTHONNOUSERSITE", "1")
        // Deno 2.9.7 cli/tools/upgrade.rs checks this exact variable before
        // spawning its background version-check task, including non-TTY runs.
        .env("DENO_NO_UPDATE_CHECK", "1")
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|_| ProviderError::HelperUnavailable)?;
    let mut process = OwnedProcess(child);
    let mut stdout = process
        .0
        .stdout
        .take()
        .ok_or(ProviderError::ExtractorFailed)?;
    let mut stderr = process
        .0
        .stderr
        .take()
        .ok_or(ProviderError::ExtractorFailed)?;
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        // Owned pipe descriptors remain alive until this function exits.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(ProviderError::ExtractorFailed);
        }
    }
    let deadline = Instant::now() + timeout;
    let mut output = Output {
        success: false,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let mut status = None;
    loop {
        if operation.cancel.is_cancelled() || cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(ProviderError::Timeout);
        }
        let out_done = drain(&mut stdout, &mut output.stdout, stdout_limit)?;
        let err_done = drain(&mut stderr, &mut output.stderr, stderr_limit)?;
        if status.is_none() {
            status = process
                .0
                .try_wait()
                .map_err(|_| ProviderError::ExtractorFailed)?;
        }
        if let Some(exit) = status
            && out_done
            && err_done
        {
            output.success = exit.success();
            return Ok(output);
        }
        let mut polls = [
            libc::pollfd {
                fd: if out_done { -1 } else { stdout.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if err_done { -1 } else { stderr.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // Wait only while the operation owns live helper work, never while the app is idle.
        unsafe {
            libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 20);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use oxplay_core::CancellationToken;
    fn op() -> OperationContext {
        OperationContext {
            request_id: 1,
            session_generation: 0,
            cancel: CancellationToken::default(),
        }
    }
    fn shell(
        script: &str,
        context: &OperationContext,
        timeout: Duration,
        limit: usize,
    ) -> Result<Output, ProviderError> {
        run(
            Path::new("/bin/sh"),
            &["-c".into(), script.into()],
            context,
            timeout,
            limit,
            limit,
        )
    }
    #[test]
    fn helper_environment_disables_deno_update_checks() {
        let result = shell(
            "test \"$DENO_NO_UPDATE_CHECK\" = 1 && test \"$PYTHONNOUSERSITE\" = 1",
            &op(),
            Duration::from_secs(2),
            100,
        )
        .unwrap();
        assert!(result.success);
    }
    #[test]
    fn bounded_capture_and_exit() {
        let result = shell(
            "printf '{\"ok\":true}'; printf 'diagnostic' >&2",
            &op(),
            Duration::from_secs(2),
            1024,
        )
        .unwrap();
        assert!(result.success);
        assert_eq!(result.stdout, b"{\"ok\":true}");
        assert_eq!(result.stderr, b"diagnostic");
        assert!(matches!(
            shell(
                "while :; do printf 1234567890; done",
                &op(),
                Duration::from_secs(2),
                100
            ),
            Err(ProviderError::OutputTooLarge)
        ));
    }
    #[test]
    fn timeout_and_cancellation_stop_descendant_pipe_owners() {
        let started = std::time::Instant::now();
        assert!(matches!(
            shell("sleep 30 & wait", &op(), Duration::from_millis(60), 100),
            Err(ProviderError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        let context = op();
        let cancellation = context.cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            cancellation.cancel();
        });
        assert!(matches!(
            shell("sleep 30 & wait", &context, Duration::from_secs(2), 100),
            Err(ProviderError::Cancelled)
        ));
        thread.join().unwrap();
    }
    #[test]
    fn successful_parent_cannot_leave_a_background_helper_running() {
        let result = shell(
            "(exec >/dev/null 2>&1; sleep 30) & printf '%s' $!",
            &op(),
            Duration::from_secs(2),
            100,
        )
        .unwrap();
        let pid: i32 = String::from_utf8(result.stdout).unwrap().parse().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(pid, 0) } == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "background helper survived group cleanup"
        );
    }

    #[test]
    fn session_generation_change_stops_authenticated_helper() {
        let generation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let shared = generation.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            shared.store(1, std::sync::atomic::Ordering::Release);
        });
        let result = run_guarded(
            Path::new("/bin/sh"),
            &["-c".into(), "sleep 30 & wait".into()],
            &op(),
            Duration::from_secs(2),
            100,
            100,
            &|| generation.load(std::sync::atomic::Ordering::Acquire) != 0,
        );
        assert!(matches!(result, Err(ProviderError::Cancelled)));
        thread.join().unwrap();
    }
    #[test]
    fn cancelled_operation_never_spawns() {
        let context = op();
        context.cancel.cancel();
        assert!(matches!(
            shell("exit 99", &context, Duration::from_secs(2), 100),
            Err(ProviderError::Cancelled)
        ));
    }
}
