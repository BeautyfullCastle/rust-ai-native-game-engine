//! Shared Linux x86-64 supervision of explicitly trusted, already-built code.
//!
//! ELF checks and successful execution do not authenticate code or establish its
//! source/capabilities. Process groups, deadlines and output limits are cleanup
//! mechanisms, not a sandbox. Callers remain responsible for trusting the command
//! and for validating its arguments, working directory and produced output.
use std::{
    io::Read,
    os::{fd::AsFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Atomically publish a staged path without replacing an existing destination.
/// The caller owns staging, path admission, durability and cleanup policy.
pub use crate::project_publish::publish_no_replace;

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Check only the existing little-endian Linux x86-64 ELF header contract.
/// This is neither full executable validation nor code authentication.
pub fn validate_elf(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 64
        || &bytes[..4] != b"\x7fELF"
        || bytes[4..7] != [2, 1, 1]
        || !matches!(u16::from_le_bytes([bytes[16], bytes[17]]), 2 | 3)
        || u16::from_le_bytes([bytes[18], bytes[19]]) != 62
        || u16::from_le_bytes([bytes[52], bytes[53]]) != 64
    {
        return Err(
            "runtime must be a Linux-compatible little-endian x86-64 ELF executable".into(),
        );
    }
    Ok(())
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
/// Execute an explicitly trusted command with bounded combined stdout/stderr,
/// a deadline and owned process-group cleanup. Returns stdout on successful exit.
/// This does not sandbox the child or authenticate the supplied executable.
pub fn run_bounded(
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
    let started = Instant::now();
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
