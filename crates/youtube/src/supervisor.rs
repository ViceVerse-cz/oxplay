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

/// Finite limits for a long-running helper whose stdout is consumed as lines.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamLimits {
    /// Hard ceiling for the whole helper run.
    pub overall: Duration,
    /// Maximum time without any stdout/stderr bytes before the run is stopped.
    pub idle: Duration,
    /// Longest accepted stdout line; a longer line fails as `OutputTooLarge`.
    pub line: usize,
    /// Only the most recent stderr bytes are retained for classification.
    pub stderr_tail: usize,
}
pub(crate) struct Streamed {
    pub success: bool,
    /// The retained stderr tail. Never log it: it can contain signed URLs.
    pub stderr: Vec<u8>,
}

#[cfg(not(unix))]
pub(crate) fn run_streaming(
    _: &Path,
    _: &[String],
    _: &OperationContext,
    _: StreamLimits,
    _: &dyn Fn() -> bool,
    _: &mut dyn FnMut(&str),
) -> Result<Streamed, ProviderError> {
    // Fail closed until a Windows job-object supervisor is implemented and tested.
    Err(ProviderError::UnsupportedPlatform)
}

#[cfg(unix)]
mod owned {
    use oxplay_core::ProviderError;
    use std::{
        io::{ErrorKind, Read},
        os::{fd::AsRawFd, unix::process::CommandExt},
        path::Path,
        process::{Child, ChildStderr, ChildStdout, Command, Stdio},
    };
    pub(super) struct OwnedProcess(pub Child);
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
    /// Spawn one isolated helper in its own process group with nonblocking pipes.
    pub(super) fn spawn(
        binary: &Path,
        args: &[String],
    ) -> Result<(OwnedProcess, ChildStdout, ChildStderr), ProviderError> {
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
        let stdout = process
            .0
            .stdout
            .take()
            .ok_or(ProviderError::ExtractorFailed)?;
        let stderr = process
            .0
            .stderr
            .take()
            .ok_or(ProviderError::ExtractorFailed)?;
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            // Owned pipe descriptors remain alive until the caller drops them.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(ProviderError::ExtractorFailed);
            }
        }
        Ok((process, stdout, stderr))
    }
    /// Read one finite batch. Returns (end of stream, whether any bytes arrived).
    pub(super) fn read_batch(
        reader: &mut impl Read,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), ProviderError>,
    ) -> Result<(bool, bool), ProviderError> {
        let mut buffer = [0u8; 8192];
        let mut activity = false;
        // A finite drain batch ensures a chatty child cannot starve cancellation checks.
        for _ in 0..16 {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok((true, activity)),
                Ok(n) => {
                    activity = true;
                    sink(&buffer[..n])?;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok((false, activity)),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(ProviderError::ExtractorFailed),
            }
        }
        Ok((false, activity))
    }
    /// Wait only while the operation owns live helper work, never while the app is idle.
    pub(super) fn wait(stdout: Option<&ChildStdout>, stderr: Option<&ChildStderr>) {
        let mut polls = [
            libc::pollfd {
                fd: stdout.map_or(-1, AsRawFd::as_raw_fd),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stderr.map_or(-1, AsRawFd::as_raw_fd),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        unsafe {
            libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 20);
        }
    }
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
    use std::time::Instant;
    fn bounded(
        output: &mut Vec<u8>,
        limit: usize,
    ) -> impl FnMut(&[u8]) -> Result<(), ProviderError> + '_ {
        move |bytes| {
            if output.len() + bytes.len() > limit {
                return Err(ProviderError::OutputTooLarge);
            }
            output.extend_from_slice(bytes);
            Ok(())
        }
    }
    if operation.cancel.is_cancelled() || cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let (mut process, mut stdout, mut stderr) = owned::spawn(binary, args)?;
    let deadline = Instant::now() + timeout;
    let mut output = Output {
        success: false,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let (mut out_done, mut err_done) = (false, false);
    let mut status = None;
    loop {
        if operation.cancel.is_cancelled() || cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(ProviderError::Timeout);
        }
        if !out_done {
            out_done =
                owned::read_batch(&mut stdout, &mut bounded(&mut output.stdout, stdout_limit))?.0;
        }
        if !err_done {
            err_done =
                owned::read_batch(&mut stderr, &mut bounded(&mut output.stderr, stderr_limit))?.0;
        }
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
        owned::wait(
            (!out_done).then_some(&stdout),
            (!err_done).then_some(&stderr),
        );
    }
}

/// Split stdout bytes into bounded lines without accumulating finished ones.
#[cfg(unix)]
struct LineSplitter {
    partial: Vec<u8>,
    limit: usize,
}
#[cfg(unix)]
impl LineSplitter {
    fn push(&mut self, bytes: &[u8], on_line: &mut dyn FnMut(&str)) -> Result<(), ProviderError> {
        for &byte in bytes {
            if byte == b'\n' {
                if self.partial.last() == Some(&b'\r') {
                    self.partial.pop();
                }
                on_line(&String::from_utf8_lossy(&self.partial));
                self.partial.clear();
            } else if self.partial.len() >= self.limit {
                return Err(ProviderError::OutputTooLarge);
            } else {
                self.partial.push(byte);
            }
        }
        Ok(())
    }
}

/// Run a long helper and hand each complete stdout line to `on_line` as it
/// arrives. Nothing is accumulated except one partial line and the stderr
/// tail, so a long download stays bounded. Cancellation, the idle and overall
/// deadlines, and every early return kill and reap the whole process group.
#[cfg(unix)]
pub(crate) fn run_streaming(
    binary: &Path,
    args: &[String],
    operation: &OperationContext,
    limits: StreamLimits,
    cancelled: &dyn Fn() -> bool,
    on_line: &mut dyn FnMut(&str),
) -> Result<Streamed, ProviderError> {
    use std::time::Instant;
    if operation.cancel.is_cancelled() || cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let (mut process, mut stdout, mut stderr) = owned::spawn(binary, args)?;
    let started = Instant::now();
    let mut last_activity = started;
    let mut lines = LineSplitter {
        partial: Vec::new(),
        limit: limits.line,
    };
    let mut tail = Vec::new();
    let (mut out_done, mut err_done) = (false, false);
    let mut status = None;
    loop {
        if operation.cancel.is_cancelled() || cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let now = Instant::now();
        if now.duration_since(started) >= limits.overall
            || now.duration_since(last_activity) >= limits.idle
        {
            return Err(ProviderError::Timeout);
        }
        if !out_done {
            let (done, activity) =
                owned::read_batch(&mut stdout, &mut |bytes| lines.push(bytes, on_line))?;
            out_done = done;
            if activity {
                last_activity = Instant::now();
            }
        }
        if !err_done {
            let (done, activity) = owned::read_batch(&mut stderr, &mut |bytes| {
                tail.extend_from_slice(bytes);
                if tail.len() > limits.stderr_tail {
                    tail.drain(..tail.len() - limits.stderr_tail);
                }
                Ok(())
            })?;
            err_done = done;
            if activity {
                last_activity = Instant::now();
            }
        }
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
            if !lines.partial.is_empty() {
                on_line(&String::from_utf8_lossy(&lines.partial));
            }
            return Ok(Streamed {
                success: exit.success(),
                stderr: tail,
            });
        }
        owned::wait(
            (!out_done).then_some(&stdout),
            (!err_done).then_some(&stderr),
        );
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

    fn limits(idle: Duration, line: usize) -> StreamLimits {
        StreamLimits {
            overall: Duration::from_secs(5),
            idle,
            line,
            stderr_tail: 8,
        }
    }
    fn stream(
        script: &str,
        context: &OperationContext,
        limits: StreamLimits,
        lines: &mut Vec<String>,
    ) -> Result<Streamed, ProviderError> {
        run_streaming(
            Path::new("/bin/sh"),
            &["-c".into(), script.into()],
            context,
            limits,
            &|| false,
            &mut |line| lines.push(line.to_owned()),
        )
    }

    #[test]
    fn streaming_delivers_lines_as_they_arrive_and_keeps_only_a_stderr_tail() {
        let mut lines = Vec::new();
        let result = stream(
            "printf 'one\\r\\ntwo\\n'; printf 'early-diagnostic-late' >&2; printf 'last'",
            &op(),
            limits(Duration::from_secs(2), 64),
            &mut lines,
        )
        .unwrap();
        assert!(result.success);
        assert_eq!(lines, ["one", "two", "last"]);
        assert_eq!(result.stderr, b"tic-late");
        // Total output far above any per-line bound is fine when lines are short.
        let mut lines = Vec::new();
        let result = stream(
            "i=0; while [ $i -lt 2000 ]; do echo 0123456789; i=$((i+1)); done; exit 3",
            &op(),
            limits(Duration::from_secs(2), 16),
            &mut lines,
        )
        .unwrap();
        assert!(!result.success);
        assert_eq!(lines.len(), 2000);
    }

    #[test]
    fn streaming_rejects_an_unbounded_line_and_stops_a_silent_helper() {
        let mut lines = Vec::new();
        assert!(matches!(
            stream(
                "while :; do printf 1234567890; done",
                &op(),
                limits(Duration::from_secs(2), 100),
                &mut lines
            ),
            Err(ProviderError::OutputTooLarge)
        ));
        let started = std::time::Instant::now();
        assert!(matches!(
            stream(
                "echo started; sleep 30 & wait",
                &op(),
                limits(Duration::from_millis(150), 100),
                &mut lines
            ),
            Err(ProviderError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(lines, ["started"]);
    }

    #[test]
    fn streaming_cancellation_kills_the_helper_group() {
        let context = op();
        let cancellation = context.cancel.clone();
        let started = std::time::Instant::now();
        let mut seen = 0;
        let result = run_streaming(
            Path::new("/bin/sh"),
            &["-c".into(), "echo ready; sleep 30 & wait".into()],
            &context,
            limits(Duration::from_secs(5), 100),
            &|| false,
            &mut |_| {
                seen += 1;
                cancellation.cancel();
            },
        );
        assert!(matches!(result, Err(ProviderError::Cancelled)));
        assert_eq!(seen, 1);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
