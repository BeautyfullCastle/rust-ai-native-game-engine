//! Bounded supervision shared by fixed authoring verifiers; not a code sandbox.
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
// Monotonic authoring-process supervision, never simulation time.
#[allow(clippy::disallowed_types)]
use std::time::Instant;
const CAP: usize = 64 * 1024;
fn drain(mut stream: impl Read + Send + 'static) -> thread::JoinHandle<Result<Vec<u8>, String>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        stream
            .by_ref()
            .take((CAP + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    })
}
// This clock bounds a trusted external authoring verifier, not deterministic game simulation.
#[allow(clippy::disallowed_types)]
pub(super) fn execute(mut command: Command, timeout: Duration) -> Result<Vec<u8>, String> {
    command
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("trusted Node could not start: {e}"))?;
    let stdout = drain(child.stdout.take().ok_or("Node stdout unavailable")?);
    let stderr = drain(child.stderr.take().ok_or("Node stderr unavailable")?);
    let start = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("Node sampler timed out".to_owned());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    };
    let out = stdout.join().map_err(|_| "Node stdout reader panicked")??;
    let err = stderr.join().map_err(|_| "Node stderr reader panicked")??;
    let status = outcome?;
    if out.len() > CAP || err.len() > CAP {
        return Err("Node output exceeds bound".into());
    }
    if !status.success() {
        return Err(format!(
            "Node sampler failed: {}",
            String::from_utf8_lossy(&err)
        ));
    }
    Ok(out)
}
