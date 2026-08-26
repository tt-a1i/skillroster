//! Filesystem durability barriers for published directory entries.
//!
//! File `sync_all` does not by itself make a create, remove, or rename durable.
//! The containing directory must also be flushed before the mutation can be
//! treated as recoverable after a crash.

use std::io;
use std::path::Path;

#[cfg(unix)]
use std::fs::File;

pub(crate) trait DirectorySync {
    fn sync_directory(&self, path: &Path) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemDirectorySync;

impl DirectorySync for SystemDirectorySync {
    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        sync_directory(path)
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?
        .sync_all()
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "directory durability is unsupported on this platform: {}",
            path.display()
        ),
    ))
}
