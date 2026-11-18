//! Immutable native run generations in an explicitly selected private directory. The
//! Linux transport works relative to a pinned directory descriptor. The Windows transport
//! pins the folder by file identity (`orr_fsid`) and refuses links and junctions. Both
//! keep one protocol: a bounded folder, one new file per save, nothing replaced.

pub const MAX_SAVE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DIRECTORY_ENTRIES: usize = 256;

/// `run-<32 hex>-<8 hex>-<16 hex>.json`: the only names that are read as generations.
pub(crate) fn valid_name(name: &str) -> bool {
    let Some(inner) = name
        .strip_prefix("run-")
        .and_then(|n| n.strip_suffix(".json"))
    else {
        return false;
    };
    let fields: Vec<_> = inner.split('-').collect();
    fields.len() == 3
        && fields.iter().zip([32, 8, 16]).all(|(f, len)| {
            f.len() == len
                && f.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

#[cfg(target_os = "linux")]
#[path = "defense_run_save_store_linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "defense_run_save_store_windows.rs"]
mod imp;

pub use imp::RunStore;
