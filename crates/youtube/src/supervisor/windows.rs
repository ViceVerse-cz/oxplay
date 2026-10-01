// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows helper supervision with one Job Object per run.
//!
//! The helper is created suspended, assigned to a fresh job with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and only then resumed, so every
//! descendant it can create is born inside the job. Timeout, cancellation,
//! bounded-output failure, success and even an Oxplay crash (the kernel closes
//! the last job handle) all terminate the complete process tree. This matches
//! the Unix process-group supervisor: bounded capture, a finite deadline and
//! cooperative cancellation checked at least every 20 ms.
use super::{LineSplitter, Output, StreamLimits, Streamed};
use oxplay_core::{OperationContext, ProviderError};
use std::{
    ffi::OsString,
    io::{ErrorKind, Read},
    os::windows::{fs::MetadataExt, io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_BROKEN_PIPE, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Pipes::PeekNamedPipe,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

/// An owned job handle. Closing the last handle kills every process in it.
struct Job(HANDLE);
impl Job {
    fn new() -> Result<Self, ProviderError> {
        // SAFETY: plain Win32 calls; the returned handle is owned by `Job`.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(ProviderError::ExtractorFailed);
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                return Err(ProviderError::ExtractorFailed);
            }
            Ok(job)
        }
    }
}
impl Job {
    /// Processes still alive in the job; `None` if the query failed.
    fn active_processes(&self) -> Option<u32> {
        // SAFETY: the owned handle is valid; the struct is plain data.
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            (QueryInformationJobObject(
                self.0,
                JobObjectBasicAccountingInformation,
                std::ptr::from_mut(&mut info).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            ) != 0)
                .then_some(info.ActiveProcesses)
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and closed exactly once.
        unsafe {
            TerminateJobObject(self.0, 1);
            CloseHandle(self.0);
        }
    }
}

/// Terminates the complete job before reaping the direct child, whether the
/// run succeeded, failed, timed out or was cancelled. Fields drop in order:
/// the job handle closes, then the run's private TEMP directory is removed.
struct OwnedProcess {
    child: Child,
    job: Job,
    _temporary: tempfile::TempDir,
}
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        // SAFETY: the job handle stays valid until `Job::drop` after this body.
        unsafe {
            TerminateJobObject(self.job.0, 1);
        }
        // Covers a direct child that never entered the job (assignment failure).
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Termination is asynchronous for descendants. Wait, bounded, until
        // the job is empty so no helper still holds files in its TEMP.
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.job.active_processes().is_some_and(|count| count > 0)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A fresh per-run TEMP below the user's temporary directory. PyInstaller
/// one-file helpers (the standalone yt-dlp.exe) unpack into TEMP and only
/// clean up when their bootloader exits normally; a terminated run would
/// otherwise leak its `_MEI*` directory into the user's TEMP.
fn run_temporary() -> Result<tempfile::TempDir, ProviderError> {
    let root = std::env::temp_dir().join("oxplay-helpers");
    match std::fs::create_dir(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(_) => return Err(ProviderError::ExtractorFailed),
    }
    let metadata = std::fs::symlink_metadata(&root).map_err(|_| ProviderError::ExtractorFailed)?;
    // FILE_ATTRIBUTE_REPARSE_POINT: never follow a planted link or junction.
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(ProviderError::ExtractorFailed);
    }
    tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&root)
        .map_err(|_| ProviderError::ExtractorFailed)
}

/// Resume the initial thread of a process created with `CREATE_SUSPENDED`.
/// Standard library children do not expose their thread handle, so find it by
/// owner process; the held process handle prevents PID reuse meanwhile.
fn resume_suspended(process_id: u32) -> bool {
    // SAFETY: the snapshot and thread handles are closed on every path.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        let mut more = Thread32First(snapshot, &mut entry) != 0;
        while more {
            if entry.th32OwnerProcessID == process_id {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    resumed |= ResumeThread(thread) != u32::MAX;
                    CloseHandle(thread);
                }
            }
            more = Thread32Next(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        resumed
    }
}

enum Drain {
    Open { progressed: bool },
    Closed,
}

/// Read only bytes already buffered in the pipe, so no read can block and
/// starve deadline/cancellation checks. A finite batch bounds each call.
fn read_batch(
    reader: &mut (impl Read + AsRawHandle),
    sink: &mut dyn FnMut(&[u8]) -> Result<(), ProviderError>,
) -> Result<Drain, ProviderError> {
    let mut buffer = [0u8; 8192];
    let mut progressed = false;
    for _ in 0..16 {
        let mut available = 0u32;
        // SAFETY: the owned pipe handle is alive for this borrow.
        let peeked = unsafe {
            PeekNamedPipe(
                reader.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if peeked == 0 {
            return if std::io::Error::last_os_error().raw_os_error()
                == Some(ERROR_BROKEN_PIPE as i32)
            {
                Ok(Drain::Closed)
            } else {
                Err(ProviderError::ExtractorFailed)
            };
        }
        if available == 0 {
            break;
        }
        let wanted = (available as usize).min(buffer.len());
        match reader.read(&mut buffer[..wanted]) {
            Ok(0) => return Ok(Drain::Closed),
            Ok(n) => {
                progressed = true;
                sink(&buffer[..n])?;
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == ErrorKind::BrokenPipe => return Ok(Drain::Closed),
            Err(_) => return Err(ProviderError::ExtractorFailed),
        }
    }
    Ok(Drain::Open { progressed })
}

/// Drain one pipe unless it already closed. Returns whether bytes arrived.
fn pump(
    done: &mut bool,
    reader: &mut (impl Read + AsRawHandle),
    sink: &mut dyn FnMut(&[u8]) -> Result<(), ProviderError>,
) -> Result<bool, ProviderError> {
    if *done {
        return Ok(false);
    }
    Ok(match read_batch(reader, sink)? {
        Drain::Closed => {
            *done = true;
            false
        }
        Drain::Open { progressed } => progressed,
    })
}

/// Anonymous pipes have no readiness wait; poll only while an operation owns
/// live helper work, never while the app is idle.
fn idle_wait(progressed: bool) {
    if !progressed {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The Windows directory: always present, never the caller's working directory.
fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

/// Spawn one isolated helper suspended, place it in a fresh kill-on-close job
/// and only then resume it. Every early return tears the job down.
fn spawn(
    binary: &Path,
    args: &[String],
) -> Result<(OwnedProcess, ChildStdout, ChildStderr), ProviderError> {
    let root = system_root();
    let mut path = OsString::from(root.join("System32"));
    path.push(";");
    path.push(&root);
    let mut command = Command::new(binary);
    command
        .args(args)
        .env_clear()
        // Win32/CRT/Python initialization requires the system root. TEMP/TMP
        // name a private per-run directory, removed after the job is empty.
        .env("SystemRoot", &root)
        .env("windir", &root)
        .env("PATH", path)
        .env("LANG", "en_US.UTF-8")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        // Deno 2.9.7 cli/tools/upgrade.rs checks this exact variable before
        // spawning its background version-check task, including non-TTY runs.
        .env("DENO_NO_UPDATE_CHECK", "1");
    let temporary = run_temporary()?;
    command
        .env("TEMP", temporary.path())
        .env("TMP", temporary.path());
    let job = Job::new()?;
    let child = command
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // No console window for a GUI host; the child cannot run before it
        // belongs to the job.
        .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
        .spawn()
        .map_err(|_| ProviderError::HelperUnavailable)?;
    let mut process = OwnedProcess {
        child,
        job,
        _temporary: temporary,
    };
    // SAFETY: both handles are owned and alive.
    if unsafe { AssignProcessToJobObject(process.job.0, process.child.as_raw_handle()) } == 0 {
        return Err(ProviderError::ExtractorFailed);
    }
    if !resume_suspended(process.child.id()) {
        return Err(ProviderError::ExtractorFailed);
    }
    let stdout = process
        .child
        .stdout
        .take()
        .ok_or(ProviderError::ExtractorFailed)?;
    let stderr = process
        .child
        .stderr
        .take()
        .ok_or(ProviderError::ExtractorFailed)?;
    Ok((process, stdout, stderr))
}

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
    let (mut process, mut stdout, mut stderr) = spawn(binary, args)?;
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
        let mut progressed = pump(
            &mut out_done,
            &mut stdout,
            &mut bounded(&mut output.stdout, stdout_limit),
        )?;
        progressed |= pump(
            &mut err_done,
            &mut stderr,
            &mut bounded(&mut output.stderr, stderr_limit),
        )?;
        if status.is_none() {
            status = process
                .child
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
        idle_wait(progressed);
    }
}

/// Windows twin of the Unix `run_streaming`: each complete stdout line goes
/// to `on_line` as it arrives; only one partial line and the stderr tail are
/// retained. Cancellation, the idle and overall deadlines, an overlong line
/// and every early return terminate the whole job.
pub(crate) fn run_streaming(
    binary: &Path,
    args: &[String],
    operation: &OperationContext,
    limits: StreamLimits,
    cancelled: &dyn Fn() -> bool,
    on_line: &mut dyn FnMut(&str),
) -> Result<Streamed, ProviderError> {
    if operation.cancel.is_cancelled() || cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let (mut process, mut stdout, mut stderr) = spawn(binary, args)?;
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
        let mut progressed = pump(&mut out_done, &mut stdout, &mut |bytes| {
            lines.push(bytes, on_line)
        })?;
        progressed |= pump(&mut err_done, &mut stderr, &mut |bytes| {
            tail.extend_from_slice(bytes);
            if tail.len() > limits.stderr_tail {
                tail.drain(..tail.len() - limits.stderr_tail);
            }
            Ok(())
        })?;
        if progressed {
            last_activity = Instant::now();
        }
        if status.is_none() {
            status = process
                .child
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
        idle_wait(progressed);
    }
}

#[cfg(test)]
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

    /// A private directory holding one synthetic batch helper.
    struct Script(PathBuf);
    impl Script {
        fn new(name: &str, body: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "oxplay-supervisor-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("helper.cmd"), body).unwrap();
            Self(directory)
        }
        fn helper(&self) -> PathBuf {
            self.0.join("helper.cmd")
        }
        fn run(
            &self,
            context: &OperationContext,
            timeout: Duration,
            limit: usize,
        ) -> Result<Output, ProviderError> {
            super::super::run(&self.helper(), &[], context, timeout, limit, limit)
        }
    }
    impl Drop for Script {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn helper_environment_disables_deno_update_checks() {
        let script = Script::new(
            "environment",
            "@echo off\r\nif not \"%DENO_NO_UPDATE_CHECK%\"==\"1\" exit /b 3\r\nif not \"%PYTHONNOUSERSITE%\"==\"1\" exit /b 4\r\nif \"%SystemRoot%\"==\"\" exit /b 5\r\nexit /b 0\r\n",
        );
        let result = script.run(&op(), Duration::from_secs(10), 100).unwrap();
        assert!(result.success);
    }

    #[test]
    fn bounded_capture_and_exit() {
        let script = Script::new(
            "capture",
            "@echo off\r\necho {\"ok\":true}\r\n1>&2 echo diagnostic\r\nexit /b 0\r\n",
        );
        let result = script.run(&op(), Duration::from_secs(10), 1024).unwrap();
        assert!(result.success);
        assert_eq!(result.stdout, b"{\"ok\":true}\r\n");
        assert_eq!(result.stderr, b"diagnostic\r\n");
        let failing = Script::new("failure", "@echo off\r\nexit /b 7\r\n");
        assert!(
            !failing
                .run(&op(), Duration::from_secs(10), 1024)
                .unwrap()
                .success
        );
        let chatty = Script::new(
            "chatty",
            "@echo off\r\n:loop\r\necho 1234567890\r\ngoto loop\r\n",
        );
        assert!(matches!(
            chatty.run(&op(), Duration::from_secs(10), 100),
            Err(ProviderError::OutputTooLarge)
        ));
    }

    #[test]
    fn timeout_and_cancellation_stop_descendant_pipe_owners() {
        let script = Script::new("sleep", "@echo off\r\nping -n 30 127.0.0.1 >nul\r\n");
        let started = Instant::now();
        assert!(matches!(
            script.run(&op(), Duration::from_millis(300), 100),
            Err(ProviderError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        let context = op();
        let cancellation = context.cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            cancellation.cancel();
        });
        let started = Instant::now();
        assert!(matches!(
            script.run(&context, Duration::from_secs(20), 100),
            Err(ProviderError::Cancelled)
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        thread.join().unwrap();
    }

    #[test]
    fn background_descendant_cannot_outlive_its_run() {
        let marker = std::env::temp_dir().join(format!(
            "oxplay-supervisor-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let marker_text = marker.to_str().unwrap();
        assert!(!marker_text.contains([' ', '"', '&', '%']));
        // The direct child exits at once; a detached grandchild would create
        // the marker about two seconds later unless the job terminated it.
        let script = Script::new(
            "background",
            &format!(
                "@echo off\r\nstart \"\" /b cmd /d /c \"ping -n 3 127.0.0.1 >nul & echo escaped>{marker_text}\"\r\nexit /b 0\r\n"
            ),
        );
        let result = script.run(&op(), Duration::from_millis(800), 100);
        assert!(matches!(result, Ok(_) | Err(ProviderError::Timeout)));
        std::thread::sleep(Duration::from_secs(4));
        let escaped = marker.exists();
        let _ = std::fs::remove_file(&marker);
        assert!(!escaped, "background helper survived job termination");
    }

    #[test]
    fn private_run_temp_is_removed_with_its_contents() {
        let script = Script::new(
            "temporary",
            "@echo off\r\necho x> \"%TEMP%\\left-behind.txt\"\r\n<nul set /p =%TEMP%\r\nexit /b 0\r\n",
        );
        let result = script.run(&op(), Duration::from_secs(10), 4096).unwrap();
        assert!(result.success);
        let temporary = PathBuf::from(String::from_utf8(result.stdout).unwrap().trim());
        assert!(temporary.starts_with(std::env::temp_dir().join("oxplay-helpers")));
        assert!(!temporary.exists(), "per-run TEMP survived its run");
    }

    #[test]
    fn session_generation_change_stops_authenticated_helper() {
        let script = Script::new("generation", "@echo off\r\nping -n 30 127.0.0.1 >nul\r\n");
        let generation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let shared = generation.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            shared.store(1, std::sync::atomic::Ordering::Release);
        });
        let result = run_guarded(
            &script.helper(),
            &[],
            &op(),
            Duration::from_secs(20),
            100,
            100,
            &|| generation.load(std::sync::atomic::Ordering::Acquire) != 0,
        );
        assert!(matches!(result, Err(ProviderError::Cancelled)));
        thread.join().unwrap();
    }

    #[test]
    fn cancelled_operation_never_spawns() {
        let script = Script::new("never", "@echo off\r\nexit /b 99\r\n");
        let context = op();
        context.cancel.cancel();
        assert!(matches!(
            script.run(&context, Duration::from_secs(2), 100),
            Err(ProviderError::Cancelled)
        ));
    }

    #[test]
    fn missing_helper_is_unavailable() {
        let script = Script::new("missing", "");
        assert!(matches!(
            super::super::run(
                &script.0.join("absent.exe"),
                &[],
                &op(),
                Duration::from_secs(2),
                100,
                100
            ),
            Err(ProviderError::HelperUnavailable)
        ));
    }

    fn limits(idle: Duration, line: usize) -> StreamLimits {
        StreamLimits {
            overall: Duration::from_secs(20),
            idle,
            line,
            stderr_tail: 8,
        }
    }
    impl Script {
        fn stream(
            &self,
            context: &OperationContext,
            limits: StreamLimits,
            lines: &mut Vec<String>,
        ) -> Result<Streamed, ProviderError> {
            run_streaming(
                &self.helper(),
                &[],
                context,
                limits,
                &|| false,
                &mut |line| lines.push(line.to_owned()),
            )
        }
    }

    #[test]
    fn streaming_delivers_lines_as_they_arrive_and_keeps_only_a_stderr_tail() {
        let script = Script::new(
            "stream-lines",
            "@echo off\r\necho one\r\necho two\r\n1>&2 <nul set /p =early-diagnostic-late\r\n<nul set /p =last\r\nexit /b 0\r\n",
        );
        let mut lines = Vec::new();
        let result = script
            .stream(&op(), limits(Duration::from_secs(10), 64), &mut lines)
            .unwrap();
        assert!(result.success);
        assert_eq!(lines, ["one", "two", "last"]);
        assert_eq!(result.stderr, b"tic-late");
        // Total output far above any per-line bound is fine when lines are short.
        let many = Script::new(
            "stream-many",
            "@echo off\r\nfor /L %%i in (1,1,2000) do echo 0123456789\r\nexit /b 3\r\n",
        );
        let mut lines = Vec::new();
        let result = many
            .stream(&op(), limits(Duration::from_secs(10), 16), &mut lines)
            .unwrap();
        assert!(!result.success);
        assert_eq!(lines.len(), 2000);
        assert!(lines.iter().all(|line| line == "0123456789"));
    }

    #[test]
    fn streaming_rejects_an_unbounded_line_and_stops_a_silent_helper() {
        let unbounded = Script::new(
            "stream-unbounded",
            "@echo off\r\n:loop\r\n<nul set /p =1234567890\r\ngoto loop\r\n",
        );
        let mut lines = Vec::new();
        assert!(matches!(
            unbounded.stream(&op(), limits(Duration::from_secs(10), 100), &mut lines),
            Err(ProviderError::OutputTooLarge)
        ));
        let silent = Script::new(
            "stream-silent",
            "@echo off\r\necho started\r\nping -n 30 127.0.0.1 >nul\r\n",
        );
        let started = Instant::now();
        assert!(matches!(
            silent.stream(&op(), limits(Duration::from_secs(1), 100), &mut lines),
            Err(ProviderError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(6));
        assert_eq!(lines, ["started"]);
    }

    #[test]
    fn streaming_overall_deadline_and_guard_stop_the_job() {
        let chatty = Script::new(
            "stream-chatty",
            "@echo off\r\n:loop\r\necho tick\r\nping -n 2 127.0.0.1 >nul\r\ngoto loop\r\n",
        );
        let mut lines = Vec::new();
        let started = Instant::now();
        let overall = StreamLimits {
            overall: Duration::from_millis(1500),
            ..limits(Duration::from_secs(10), 100)
        };
        assert!(matches!(
            chatty.stream(&op(), overall, &mut lines),
            Err(ProviderError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(!lines.is_empty());
        let generation = std::sync::atomic::AtomicBool::new(false);
        let result = run_streaming(
            &chatty.helper(),
            &[],
            &op(),
            limits(Duration::from_secs(10), 100),
            &|| generation.load(std::sync::atomic::Ordering::Acquire),
            &mut |_| generation.store(true, std::sync::atomic::Ordering::Release),
        );
        assert!(matches!(result, Err(ProviderError::Cancelled)));
    }

    #[test]
    fn streaming_cancellation_kills_the_helper_job() {
        let script = Script::new(
            "stream-cancel",
            "@echo off\r\necho ready\r\nping -n 30 127.0.0.1 >nul\r\n",
        );
        let context = op();
        let cancellation = context.cancel.clone();
        let started = Instant::now();
        let mut seen = 0;
        let result = run_streaming(
            &script.helper(),
            &[],
            &context,
            limits(Duration::from_secs(10), 100),
            &|| false,
            &mut |_| {
                seen += 1;
                cancellation.cancel();
            },
        );
        assert!(matches!(result, Err(ProviderError::Cancelled)));
        assert_eq!(seen, 1);
        assert!(started.elapsed() < Duration::from_secs(6));
    }
}
