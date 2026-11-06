//! Bounded supervision for one explicitly trusted copied executable, not a sandbox.
use crate::authoring_io::error;
use std::{
    io::Read,
    os::{fd::AsFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};
// Wall time bounds a trusted executable query; it never enters simulation state.
#[allow(
    clippy::disallowed_types,
    reason = "monotonic process-query deadline enforcement outside deterministic simulation"
)]
mod query_clock {
    use std::time::{Duration, Instant};

    pub(super) struct Deadline(Instant);
    impl Deadline {
        pub(super) fn now() -> Self {
            Self(Instant::now())
        }
        pub(super) fn elapsed(&self) -> Duration {
            self.0.elapsed()
        }
    }
}

struct ChildGuard(Child, bool);
impl ChildGuard {
    fn pid(&self) -> Result<rustix::process::Pid, String> {
        let raw = i32::try_from(self.0.id()).map_err(error)?;
        rustix::process::Pid::from_raw(raw).ok_or("invalid owned child PID".into())
    }
    fn terminate_group(&self) -> Result<(), String> {
        match rustix::process::kill_process_group(self.pid()?, rustix::process::Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(e) => Err(format!(
                "could not terminate owned smoke process group: {e}"
            )),
        }
    }
    fn finish(&mut self) -> Result<std::process::ExitStatus, String> {
        // The child has not been reaped, so its PID/group ID cannot be reused.
        self.terminate_group()?;
        let status = self.0.wait().map_err(error)?;
        self.1 = false;
        Ok(status)
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.1 {
            return;
        }
        let _ = self.terminate_group();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn nonblocking(fd: impl AsFd) -> Result<(), String> {
    let flags = rustix::fs::fcntl_getfl(&fd).map_err(error)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(error)
}
fn drain(stream: &mut impl Read, bytes: &mut Vec<u8>, remaining: usize) -> Result<bool, String> {
    let mut buffer = [0u8; 8192];
    // A single bounded read per poll prevents an endless producer starving the timeout.
    match stream.read(&mut buffer) {
        Ok(0) => Ok(true),
        Ok(n) if n > remaining => Err("trusted runtime smoke output exceeds limit".into()),
        Ok(n) => {
            bytes.extend_from_slice(&buffer[..n]);
            Ok(false)
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(error(e)),
    }
}
pub(super) fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    max_output: usize,
) -> Result<Vec<u8>, String> {
    let mut child = ChildGuard(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("trusted runtime smoke launch failed: {e}"))?,
        true,
    );
    let mut stdout = child.0.stdout.take().ok_or("smoke stdout unavailable")?;
    let mut stderr = child.0.stderr.take().ok_or("smoke stderr unavailable")?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut status = None;
    let started = query_clock::Deadline::now();
    loop {
        let remaining = max_output.saturating_sub(out.len() + err.len());
        let out_done = drain(&mut stdout, &mut out, remaining)?;
        let remaining = max_output.saturating_sub(out.len() + err.len());
        let err_done = drain(&mut stderr, &mut err, remaining)?;
        if status.is_none() {
            // Observe exit without reaping. Cleanup also covers descendants
            // that redirected their output and outlive a completed launcher.
            use rustix::process::{waitid, WaitId, WaitIdOptions};
            let exited = waitid(
                WaitId::Pid(child.pid()?),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .map_err(error)?
            .is_some();
            if exited {
                status = Some(child.finish()?);
            }
        }
        if let Some(status) = status {
            if out_done && err_done {
                if !status.success() {
                    return Err(format!(
                        "trusted runtime smoke failed ({status}): {}",
                        String::from_utf8_lossy(&err)
                    ));
                }
                return Ok(out);
            }
        }
        if started.elapsed() >= timeout {
            return Err("trusted runtime smoke timed out".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
}
