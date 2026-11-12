//! A filesystem socket fixture for tests of rejection before opening a path.
//!
//! These tests inspect a socket inode, not a listening network endpoint. On
//! Linux, mknod creates that exact file type without requiring socket syscalls.
//! Other Unix platforms retain the bound-listener fixture. Neither path skips
//! a test or falls back to a different file type when creation fails.

use std::{fs, os::unix::fs::FileTypeExt, path::Path};

pub struct SocketFixture {
    _listener: Option<std::os::unix::net::UnixListener>,
}

impl SocketFixture {
    pub fn create(path: &Path) -> Self {
        #[cfg(target_os = "linux")]
        let fixture = {
            // Python 3 is part of the Linux-local validation toolchain. Passing
            // the path as an argument preserves non-UTF-8 names and avoids shell
            // interpretation. mknod must succeed; permission errors remain fatal.
            let status = std::process::Command::new("python3")
                .args([
                    "-c",
                    "import os, stat, sys; os.mknod(sys.argv[1], stat.S_IFSOCK | 0o600)",
                ])
                .arg(path)
                .status()
                .expect("Python 3 is required to create the Linux socket inode fixture");
            assert!(status.success(), "socket inode fixture creation failed");
            Self { _listener: None }
        };
        #[cfg(not(target_os = "linux"))]
        let fixture = Self {
            _listener: Some(
                std::os::unix::net::UnixListener::bind(path)
                    .expect("socket fixture creation unavailable"),
            ),
        };
        assert!(
            fs::symlink_metadata(path).unwrap().file_type().is_socket(),
            "fixture must be an actual socket inode, not a regular file or symlink"
        );
        fixture
    }
}
