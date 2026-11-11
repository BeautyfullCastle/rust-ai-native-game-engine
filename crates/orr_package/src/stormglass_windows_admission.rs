//! Content-only prerequisite for a future Windows Stormglass folder publisher.
//!
//! This module accepts already captured, trusted local byte snapshots. It does
//! not read paths, execute binaries, write receipts or publish folders. A digest
//! is integrity, not authentication, and PE header checks are not a sandbox or
//! a guarantee that Windows can load the executable. A publisher must separately
//! reserve/recheck no-reparse inputs and staging, run bounded production smoke,
//! and publish with no replacement. These checks do not constitute that workflow.
use orr_games::coop_defense_game::DefenseConfig;
use orr_games::coop_defense_scene::{parse_config, MAX_SOURCE_BYTES};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const MAX_RUNTIME_BYTES: usize = 256 * 1024 * 1024;
pub const RUNTIME_NAME: &str = "coop_defense.exe";
pub const SCENE_NAME: &str = "game.scene.yaml";
pub const LAUNCHER_NAME: &str = "run-game.ps1";
pub const LAUNCHER: &[u8] = include_bytes!("../../../tools/stormglass_windows/run-game.ps1");

/// Immutable content evidence, deliberately not a publication/smoke receipt.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileDigest {
    pub name: &'static str,
    pub bytes: usize,
    pub sha256: String,
}

/// Borrows the exact admitted snapshots so safe callers cannot mutate them while
/// using this result. The caller still owns all filesystem and execution checks.
#[derive(Debug)]
pub struct AdmittedContent<'a> {
    runtime: &'a [u8],
    scene: &'a [u8],
    config: DefenseConfig,
    files: [FileDigest; 3],
}

impl<'a> AdmittedContent<'a> {
    pub fn runtime(&self) -> &'a [u8] {
        self.runtime
    }
    pub fn scene(&self) -> &'a [u8] {
        self.scene
    }
    pub fn config(&self) -> &DefenseConfig {
        &self.config
    }
    pub fn files(&self) -> &[FileDigest; 3] {
        &self.files
    }
}

/// Admit a pinned x64 Windows executable and the shared strict Stormglass scene.
/// The supplied hash must come from a trusted source; this API cannot establish
/// that trust. It does not accept user-selected filenames or optional payloads.
pub fn admit<'a>(
    runtime: &'a [u8],
    expected_sha256: &str,
    scene: &'a [u8],
) -> Result<AdmittedContent<'a>, String> {
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("runtime SHA256 must be exactly 64 hexadecimal characters".into());
    }
    if runtime.len() > MAX_RUNTIME_BYTES {
        return Err("runtime exceeds 256 MiB".into());
    }
    if scene.len() > MAX_SOURCE_BYTES {
        return Err("scene exceeds 64 KiB".into());
    }
    let runtime_digest = digest(RUNTIME_NAME, runtime);
    if !runtime_digest.sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err("runtime SHA256 mismatch".into());
    }
    validate_pe(runtime)?;
    let text = std::str::from_utf8(scene).map_err(|_| "scene must be UTF-8")?;
    let config = parse_config(text)?;
    Ok(AdmittedContent {
        runtime,
        scene,
        config,
        files: [
            runtime_digest,
            digest(SCENE_NAME, scene),
            digest(LAUNCHER_NAME, LAUNCHER),
        ],
    })
}

fn digest(name: &'static str, bytes: &[u8]) -> FileDigest {
    FileDigest {
        name,
        bytes: bytes.len(),
        sha256: Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    }
}

fn word(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes.get(offset..offset + 2).ok_or("truncated PE header")?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}
fn dword(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes.get(offset..offset + 4).ok_or("truncated PE header")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

// Narrow PE32+ header admission, not a second Windows executable loader.
fn validate_pe(bytes: &[u8]) -> Result<(), String> {
    if bytes.get(..2) != Some(b"MZ") {
        return Err("runtime needs a DOS MZ header".into());
    }
    let pe = dword(bytes, 0x3c)? as usize;
    // Bound the offset before addition, including on 32-bit hosts.
    if pe < 64 || pe > bytes.len().saturating_sub(24) || bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("runtime needs a bounded PE signature".into());
    }
    let sections = word(bytes, pe + 6)? as usize;
    let optional_size = word(bytes, pe + 20)? as usize;
    let flags = word(bytes, pe + 22)?;
    if word(bytes, pe + 4)? != 0x8664 || flags & 2 == 0 || flags & 0x2000 != 0 {
        return Err("runtime must be an x64 executable, not a DLL".into());
    }
    let optional = pe + 24;
    let table = optional + optional_size;
    if !(1..=96).contains(&sections)
        || optional_size < 112
        || table + sections * 40 > bytes.len()
        || word(bytes, optional)? != 0x20b
        || !matches!(word(bytes, optional + 68)?, 2 | 3)
    {
        return Err("runtime needs bounded PE32+ Windows application headers".into());
    }
    let entry = dword(bytes, optional + 16)? as u64;
    let headers = dword(bytes, optional + 60)? as usize;
    if headers < table + sections * 40 || headers > bytes.len() {
        return Err("runtime PE header size is invalid".into());
    }
    let mut executable_entry = false;
    for index in 0..sections {
        let section = table + index * 40;
        let virtual_address = dword(bytes, section + 12)? as u64;
        let raw_size = dword(bytes, section + 16)? as u64;
        let raw_start = dword(bytes, section + 20)? as u64;
        if raw_size != 0
            && (raw_start < headers as u64 || raw_start + raw_size > bytes.len() as u64)
        {
            return Err("runtime PE section exceeds snapshot bounds".into());
        }
        if entry >= virtual_address
            && entry < virtual_address + raw_size
            && dword(bytes, section + 36)? & 0x2000_0000 != 0
        {
            executable_entry = true;
        }
    }
    if !executable_entry {
        return Err("runtime PE entry point needs an executable file-backed section".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "stormglass_windows_admission_tests.rs"]
mod tests;
