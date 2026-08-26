//! Filesystem durability barriers for published directory entries.
//!
//! File `sync_all` does not by itself make a create, remove, or rename durable.
//! The containing directory must also be flushed before the mutation can be
//! treated as recoverable after a crash.

use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) trait DirectorySync {
    fn sync_directory(&self, path: &Path) -> io::Result<()>;

    /// Flush an already-open directory capability. The diagnostic path is
    /// supplied separately so tests can inject failures without making
    /// production durability depend on ambient path resolution.
    fn sync_directory_handle(&self, directory: &File, path: &Path) -> io::Result<()> {
        let _ = directory;
        self.sync_directory(path)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemDirectorySync;

impl DirectorySync for SystemDirectorySync {
    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        sync_directory(path)
    }

    fn sync_directory_handle(&self, directory: &File, _path: &Path) -> io::Result<()> {
        directory.sync_all()
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
