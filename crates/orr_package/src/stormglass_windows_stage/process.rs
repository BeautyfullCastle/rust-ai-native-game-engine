//! Bounded supervisor for an explicitly trusted copied executable. Not a sandbox.
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path, ptr, time::Duration};

// This clock only supervises external processes; it never enters simulation state.
#[allow(
    clippy::disallowed_types,
    reason = "monotonic process timeout outside deterministic simulation"
)]
pub(super) mod query_clock {
    use std::time::{Duration, Instant};
    pub(crate) struct Clock(Instant);
    impl Clock {
        pub(crate) fn now() -> Self {
            Self(Instant::now())
        }
        pub(crate) fn elapsed(&self) -> Duration {
            self.0.elapsed()
        }
    }
}
use query_clock::Clock;
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::ReadFile,
    System::{JobObjects::*, Pipes::*, SystemInformation::GetWindowsDirectoryW, Threading::*},
};

fn error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
fn pipe() -> Result<(Handle, Handle), String> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: correctly sized output and security-attributes structures.
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(error("CreatePipe"));
    }
    Ok((Handle(read), Handle(write)))
}
fn not_inherited(handle: &Handle) -> Result<(), String> {
    // SAFETY: live owned handle; only its inheritance flag is cleared.
    if unsafe { SetHandleInformation(handle.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(error("SetHandleInformation"));
    }
    Ok(())
}
struct Attributes(Vec<usize>);
impl Attributes {
    fn new(handles: &[HANDLE; 3]) -> Result<Self, String> {
        let mut bytes = 0;
        // SAFETY: documented size query; no output buffer supplied.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes);
        }
        if bytes == 0 || bytes > 65536 {
            return Err("invalid attribute-list size".into());
        }
        let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        // SAFETY: usize-aligned buffer large enough for the queried size.
        if unsafe {
            InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 1, 0, &mut bytes)
        } == 0
        {
            return Err(error("InitializeProcThreadAttributeList"));
        }
        let mut result = Self(buffer);
        // SAFETY: three live inheritable pipe handles remain valid through process creation.
        if unsafe {
            UpdateProcThreadAttribute(
                result.0.as_mut_ptr().cast(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(error("UpdateProcThreadAttribute"));
        }
        Ok(result)
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}
fn job() -> Result<Handle, String> {
    // SAFETY: unnamed non-inheritable job with default process-local security.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(error("CreateJobObjectW"));
    }
    let handle = Handle(raw);
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: exact documented structure and size. No breakaway flag is enabled.
    if unsafe {
        SetInformationJobObject(
            handle.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(error("SetInformationJobObject"));
    }
    Ok(handle)
}
struct Process {
    process: Handle,
    thread: Handle,
    job: Handle,
    pid: u32,
    cleaned: bool,
}
impl Process {
    fn cleanup(&mut self) -> Result<(), String> {
        let deadline = Clock::now();
        let mut attempts = Vec::new();
        // SAFETY: all handles belong solely to this newly created child/job.
        // Attempt both routes even if one fails. A terminating process can reject
        // TerminateProcess; success below depends on observed completion.
        unsafe {
            if TerminateJobObject(self.job.0, 1) == 0 {
                attempts.push(error("TerminateJobObject"));
            }
            let state = WaitForSingleObject(self.process.0, 0);
            if state != WAIT_OBJECT_0 && TerminateProcess(self.process.0, 1) == 0 {
                attempts.push(error("TerminateProcess"));
            }
        }
        loop {
            let state = unsafe { WaitForSingleObject(self.process.0, 0) };
            if state != WAIT_OBJECT_0 && state != WAIT_TIMEOUT {
                return Err(error("cleanup WaitForSingleObject"));
            }
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            // SAFETY: live job and exact documented accounting buffer.
            if unsafe {
                QueryInformationJobObject(
                    self.job.0,
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of_val(&accounting) as u32,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(error("QueryInformationJobObject"));
            }
            if state == WAIT_OBJECT_0 && accounting.ActiveProcesses == 0 {
                break;
            }
            if deadline.elapsed() >= Duration::from_secs(5) {
                return Err(format!(
                    "owned process/job cleanup not confirmed within five seconds: {attempts:?}"
                ));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.cleaned = true;
        Ok(())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if !self.cleaned {
            // Best-effort fallback; normal paths report cleanup failure explicitly.
            unsafe {
                TerminateJobObject(self.job.0, 1);
                TerminateProcess(self.process.0, 1);
            }
        }
    }
}
fn environment() -> Result<Vec<u16>, String> {
    let mut buffer = [0u16; 32768];
    // SAFETY: valid bounded writable UTF-16 buffer.
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(error("GetWindowsDirectoryW"));
    }
    let mut result = Vec::new();
    for name in ["SystemRoot=", "WINDIR="] {
        result.extend(name.encode_utf16());
        result.extend_from_slice(&buffer[..length]);
        result.push(0);
    }
    result.push(0);
    Ok(result)
}
fn drain(
    handle: &Handle,
    bytes: &mut Vec<u8>,
    used: &mut usize,
    limit: usize,
) -> Result<bool, String> {
    let mut available = 0;
    // SAFETY: live anonymous pipe read end; peeking does not block for bytes.
    if unsafe {
        PeekNamedPipe(
            handle.0,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut available,
            ptr::null_mut(),
        )
    } == 0
    {
        let e = std::io::Error::last_os_error();
        return if matches!(e.raw_os_error(), Some(109 | 232)) {
            Ok(true)
        } else {
            Err(format!("PeekNamedPipe: {e}"))
        };
    }
    if available == 0 {
        return Ok(false);
    }
    let mut buffer = [0u8; 4096];
    let count = buffer
        .len()
        .min(available as usize)
        .min(limit.saturating_sub(*used) + 1);
    let mut read = 0;
    // SAFETY: request does not exceed available bytes or buffer length. This is the only reader.
    if unsafe {
        ReadFile(
            handle.0,
            buffer.as_mut_ptr(),
            count as u32,
            &mut read,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(error("ReadFile"));
    }
    *used += read as usize;
    if *used > limit {
        return Err("combined smoke output exceeds limit".into());
    }
    bytes.extend_from_slice(&buffer[..read as usize]);
    Ok(false)
}
pub struct Output {
    pub exit_code: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
pub fn run(root: &Path, timeout: Duration, limit: usize) -> Result<Output, String> {
    run_inner(root, timeout, limit, false)
}
#[cfg(test)]
pub(super) fn abort_suspended(root: &Path) -> Result<Output, String> {
    run_inner(root, Duration::from_secs(15), 65536, true)
}
fn run_inner(
    root: &Path,
    timeout: Duration,
    limit: usize,
    abort_before_resume: bool,
) -> Result<Output, String> {
    let application = root.join("coop_defense.exe");
    let scene = root.join("game.scene.yaml");
    let application = application.to_str().ok_or("non-Unicode executable")?;
    let scene = scene.to_str().ok_or("non-Unicode scene")?;
    // Fixed names follow validated stage components; no quote, NUL or trailing slash.
    if [application, scene]
        .iter()
        .any(|p| p.contains(['\"', '\0']) || p.ends_with('\\'))
    {
        return Err("invalid fixed path".into());
    }
    let command = format!("\"{application}\" --headless --ticks 60 --scene \"{scene}\"");
    let mut command = wide(OsStr::new(&command));
    if command.len() > 32767 {
        return Err("command line too long".into());
    }
    let application = wide(OsStr::new(application));
    let cwd = wide(root.as_os_str());
    let environment = environment()?;
    let (stdin_read, stdin_write) = pipe()?;
    drop(stdin_write); // Child stdin is immediate EOF.
    let (stdout_read, stdout_write) = pipe()?;
    let (stderr_read, stderr_write) = pipe()?;
    not_inherited(&stdout_read)?;
    not_inherited(&stderr_read)?;
    let inherited = [stdin_read.0, stdout_write.0, stderr_write.0];
    let mut attributes = Attributes::new(&inherited)?;
    let job = job()?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin_read.0;
    startup.StartupInfo.hStdOutput = stdout_write.0;
    startup.StartupInfo.hStdError = stderr_write.0;
    startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: all UTF-16 strings/blocks, attributes and pipe handles outlive this synchronous call.
    // The explicit application name avoids PATH lookup; the child cannot execute before assignment.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut information,
        )
    } == 0
    {
        return Err(error("CreateProcessW"));
    }
    let mut child = Process {
        process: Handle(information.hProcess),
        thread: Handle(information.hThread),
        job,
        pid: information.dwProcessId,
        cleaned: false,
    };
    drop(attributes);
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr_write);
    let result = (|| {
        // SAFETY: owned suspended child and non-breakaway kill-on-close job.
        if unsafe { AssignProcessToJobObject(child.job.0, child.process.0) } == 0 {
            return Err(error("AssignProcessToJobObject"));
        }
        if abort_before_resume {
            return Err("injected failure before resume".into());
        }
        let started = Clock::now();
        if unsafe { ResumeThread(child.thread.0) } != 1 {
            return Err(error("ResumeThread"));
        }
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let mut used = 0;
        let mut exit_code = None;
        loop {
            if started.elapsed() >= timeout {
                return Err("trusted runtime smoke timed out".into());
            }
            let out_done = drain(&stdout_read, &mut stdout, &mut used, limit)?;
            let err_done = drain(&stderr_read, &mut stderr, &mut used, limit)?;
            if exit_code.is_none() {
                match unsafe { WaitForSingleObject(child.process.0, 0) } {
                    WAIT_OBJECT_0 => {
                        let mut code = 0;
                        if unsafe { GetExitCodeProcess(child.process.0, &mut code) } == 0 {
                            return Err(error("GetExitCodeProcess"));
                        }
                        exit_code = Some(code);
                        // Stop any descendants before draining final pipe bytes.
                        if unsafe { TerminateJobObject(child.job.0, 1) } == 0 {
                            return Err(error("TerminateJobObject after exit"));
                        }
                    }
                    WAIT_TIMEOUT => {}
                    _ => return Err(error("WaitForSingleObject")),
                }
            }
            if let Some(code) = exit_code.filter(|_| out_done && err_done) {
                return Ok(Output {
                    exit_code: code,
                    stdout,
                    stderr,
                });
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    })();
    let cleanup = child.cleanup();

    match (result, cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(e), Ok(())) => Err(format!(
            "{e}; owned child {} and job cleanup confirmed",
            child.pid
        )),
        (result, Err(e)) => Err(format!(
            "smoke result: {}; cleanup failure: {e}",
            result.err().unwrap_or_else(|| "completed".into())
        )),
    }
}
