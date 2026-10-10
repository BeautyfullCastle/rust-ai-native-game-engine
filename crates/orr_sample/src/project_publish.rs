//! Shared Linux atomic no-replace publication; never falls back to replacing rename.
use std::path::Path;

pub(crate) fn publish_no_replace(stage: &Path, output: &Path) -> Result<(), String> {
    publish_with(stage, output, |stage, output| {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            stage,
            rustix::fs::CWD,
            output,
            rustix::fs::RenameFlags::NOREPLACE,
        )
    })
}
pub(crate) fn publish_with(
    stage: &Path,
    output: &Path,
    operation: impl FnOnce(&Path, &Path) -> Result<(), rustix::io::Errno>,
) -> Result<(), String> {
    operation(stage, output).map_err(|e| format!("atomic no-replace publication failed: {e}"))
}
