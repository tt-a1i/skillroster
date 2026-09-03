use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions, Permissions};
use sha2::{Digest, Sha256};

use crate::copy_metadata::{CopyDestination, CopyMetadata};
use crate::durable_fs::DirectorySync;
use crate::package_fingerprint::{
    MAX_SKILL_PACKAGE_DEPTH, PackageHashBuilder, ignored_package_entry_name,
};

#[cfg(test)]
thread_local! {
    static AFTER_SYMLINK_SOURCE_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_STAGING_FILE_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static BEFORE_APPROVED_ROOT_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_APPROVED_ROOT_CANONICALIZE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_CREATED_DIRECTORY_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_CREATED_SYMLINK_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_CREATED_SYMLINK_FIRST_CHECK_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_CREATED_ENTRY_FIRST_CHECK_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_RENAMED_ENTRY_FIRST_CHECK_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static BEFORE_RENAME_NOREPLACE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static FORCE_ATOMIC_NOREPLACE_UNSUPPORTED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_after_symlink_source_open_hook(hook: impl FnOnce() + 'static) {
    AFTER_SYMLINK_SOURCE_OPEN_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
pub(crate) fn set_after_staging_file_open_hook(hook: impl FnOnce() + 'static) {
    AFTER_STAGING_FILE_OPEN_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(crate) fn set_before_approved_root_open_hook(hook: impl FnOnce() + 'static) {
    BEFORE_APPROVED_ROOT_OPEN_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_approved_root_canonicalize_hook(hook: impl FnOnce() + 'static) {
    AFTER_APPROVED_ROOT_CANONICALIZE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_created_directory_open_hook(hook: impl FnOnce() + 'static) {
    AFTER_CREATED_DIRECTORY_OPEN_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_created_symlink_hook(hook: impl FnOnce() + 'static) {
    AFTER_CREATED_SYMLINK_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_created_symlink_first_check_hook(hook: impl FnOnce() + 'static) {
    AFTER_CREATED_SYMLINK_FIRST_CHECK_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_created_entry_first_check_hook(hook: impl FnOnce() + 'static) {
    AFTER_CREATED_ENTRY_FIRST_CHECK_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_renamed_entry_first_check_hook(hook: impl FnOnce() + 'static) {
    AFTER_RENAMED_ENTRY_FIRST_CHECK_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, any(unix, windows)))]
pub(crate) fn set_before_rename_noreplace_hook(hook: impl FnOnce() + 'static) {
    BEFORE_RENAME_NOREPLACE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
pub(crate) fn force_atomic_noreplace_unsupported_once() {
    FORCE_ATOMIC_NOREPLACE_UNSUPPORTED.with(|forced| forced.set(true));
}

#[cfg(test)]
fn run_after_symlink_source_open_hook() {
    AFTER_SYMLINK_SOURCE_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_symlink_source_open_hook() {}

#[cfg(test)]
fn run_after_staging_file_open_hook() {
    AFTER_STAGING_FILE_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_staging_file_open_hook() {}

#[cfg(test)]
fn run_before_approved_root_open_hook() {
    BEFORE_APPROVED_ROOT_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_approved_root_open_hook() {}

#[cfg(test)]
fn run_after_approved_root_canonicalize_hook() {
    AFTER_APPROVED_ROOT_CANONICALIZE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_approved_root_canonicalize_hook() {}

#[cfg(test)]
fn run_after_created_directory_open_hook() {
    AFTER_CREATED_DIRECTORY_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_created_directory_open_hook() {}

#[cfg(test)]
fn run_after_created_symlink_hook() {
    AFTER_CREATED_SYMLINK_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_created_symlink_hook() {}

#[cfg(test)]
fn run_after_created_symlink_first_check_hook() {
    AFTER_CREATED_SYMLINK_FIRST_CHECK_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_created_symlink_first_check_hook() {}

#[cfg(test)]
fn run_after_created_entry_first_check_hook() {
    AFTER_CREATED_ENTRY_FIRST_CHECK_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_created_entry_first_check_hook() {}

#[cfg(test)]
fn run_after_renamed_entry_first_check_hook() {
    AFTER_RENAMED_ENTRY_FIRST_CHECK_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_after_renamed_entry_first_check_hook() {}

#[cfg(test)]
fn run_before_rename_noreplace_hook() {
    BEFORE_RENAME_NOREPLACE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_rename_noreplace_hook() {}

#[cfg(test)]
fn take_forced_atomic_noreplace_unsupported() -> bool {
    FORCE_ATOMIC_NOREPLACE_UNSUPPORTED.with(|forced| forced.replace(false))
}

#[cfg(not(test))]
fn take_forced_atomic_noreplace_unsupported() -> bool {
    false
}

struct Anchor {
    path: PathBuf,
    dir: Dir,
}

#[cfg(unix)]
struct EntryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
struct EntryIdentity {
    handle: fs::File,
}

#[cfg(unix)]
fn entry_identity_at(anchor: &Anchor, relative: &Path) -> io::Result<EntryIdentity> {
    use cap_std::fs::MetadataExt as _;

    let metadata = anchor.dir.symlink_metadata(relative)?;
    Ok(EntryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
fn require_entry_identity_at(
    anchor: &Anchor,
    relative: &Path,
    expected: &EntryIdentity,
) -> io::Result<()> {
    use cap_std::fs::MetadataExt as _;

    let metadata = anchor.dir.symlink_metadata(relative)?;
    if metadata.dev() == expected.device && metadata.ino() == expected.inode {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "renamed entry changed while its identity was retained",
        ))
    }
}

fn parent_dir_and_name(anchor: &Anchor, relative: &Path) -> io::Result<(Dir, OsString)> {
    let name = relative.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "rename target has no file name",
        )
    })?;
    let parent = relative
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok((anchor.dir.open_dir(parent)?, name.to_os_string()))
}

fn unsupported_atomic_noreplace_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable; mutation requires Linux or WSL2 with RENAME_NOREPLACE support",
    )
}

#[cfg(target_os = "linux")]
fn classify_atomic_noreplace_probe(error: io::Error) -> io::Result<()> {
    match error.raw_os_error() {
        Some(libc::ENOENT) => Ok(()),
        Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP) => {
            Err(unsupported_atomic_noreplace_error())
        }
        _ => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(
    from_anchor: &Anchor,
    from: &Path,
    to_anchor: &Anchor,
    to: &Path,
    _retained: &EntryIdentity,
) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::ffi::OsStrExt as _;

    let (from_parent, from_name) = parent_dir_and_name(from_anchor, from)?;
    let (to_parent, to_name) = parent_dir_and_name(to_anchor, to)?;
    let from_name = std::ffi::CString::new(from_name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "rename source contains NUL"))?;
    let to_name = std::ffi::CString::new(to_name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "rename target contains NUL"))?;
    let result = unsafe {
        libc::renameat2(
            from_parent.as_raw_fd(),
            from_name.as_ptr(),
            to_parent.as_raw_fd(),
            to_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace(
    from_anchor: &Anchor,
    from: &Path,
    to_anchor: &Anchor,
    to: &Path,
    _retained: &EntryIdentity,
) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::ffi::OsStrExt as _;

    let (from_parent, from_name) = parent_dir_and_name(from_anchor, from)?;
    let (to_parent, to_name) = parent_dir_and_name(to_anchor, to)?;
    let from_name = std::ffi::CString::new(from_name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "rename source contains NUL"))?;
    let to_name = std::ffi::CString::new(to_name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "rename target contains NUL"))?;
    let result = unsafe {
        libc::renameatx_np(
            from_parent.as_raw_fd(),
            from_name.as_ptr(),
            to_parent.as_raw_fd(),
            to_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_noreplace(
    _from_anchor: &Anchor,
    _from: &Path,
    _to_anchor: &Anchor,
    _to: &Path,
    _retained: &EntryIdentity,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn rename_noreplace(
    _from_anchor: &Anchor,
    _from: &Path,
    to_anchor: &Anchor,
    to: &Path,
    retained: &EntryIdentity,
) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Wdk::Storage::FileSystem::{FileRenameInformation, NtSetInformationFile};
    use windows_sys::Win32::Foundation::RtlNtStatusToDosError;
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    const MAX_NAME_CHARS: usize = 32_767;

    #[repr(C)]
    struct RenameInfo {
        flags: u32,
        root_directory: windows_sys::Win32::Foundation::HANDLE,
        file_name_length: u32,
        file_name: [u16; MAX_NAME_CHARS],
    }

    let (to_parent, to_name) = parent_dir_and_name(to_anchor, to)?;
    let name = to_name.encode_wide().collect::<Vec<_>>();
    if name.len() > MAX_NAME_CHARS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "rename target name exceeds the Windows limit",
        ));
    }
    let mut info = Box::new(RenameInfo {
        flags: 0,
        root_directory: to_parent.as_raw_handle(),
        file_name_length: (name.len() * 2) as u32,
        file_name: [0; MAX_NAME_CHARS],
    });
    info.file_name[..name.len()].copy_from_slice(&name);
    let size = std::mem::offset_of!(RenameInfo, file_name) + name.len() * 2;
    let mut status_block = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtSetInformationFile(
            retained.handle.as_raw_handle(),
            &mut status_block,
            (&raw const *info).cast(),
            size as u32,
            FileRenameInformation,
        )
    };
    if status >= 0 {
        Ok(())
    } else {
        let error = unsafe { RtlNtStatusToDosError(status) };
        Err(io::Error::from_raw_os_error(error as i32))
    }
}

#[cfg(not(any(unix, windows)))]
fn rename_noreplace(
    _from_anchor: &Anchor,
    _from: &Path,
    _to_anchor: &Anchor,
    _to: &Path,
    _retained: &EntryIdentity,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn entry_identity_at(anchor: &Anchor, relative: &Path) -> io::Result<EntryIdentity> {
    use cap_std::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = OpenOptions::new();
    options
        .access_mode(FILE_READ_ATTRIBUTES | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    Ok(EntryIdentity {
        handle: anchor.dir.open_with(relative, &options)?.into_std(),
    })
}

#[cfg(windows)]
fn require_entry_identity_at(
    anchor: &Anchor,
    relative: &Path,
    expected: &EntryIdentity,
) -> io::Result<()> {
    let actual = entry_identity_at(anchor, relative)?;
    if same_file_identity(&expected.handle, &actual.handle)? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "renamed entry changed while its handle was retained",
        ))
    }
}

#[cfg(not(any(unix, windows)))]
struct EntryIdentity {
    fingerprint: String,
}

#[cfg(not(any(unix, windows)))]
fn entry_identity_at(anchor: &Anchor, relative: &Path) -> io::Result<EntryIdentity> {
    Ok(EntryIdentity {
        fingerprint: AnchoredFs::fingerprint_relative(anchor, relative)?,
    })
}

#[cfg(not(any(unix, windows)))]
fn require_entry_identity_at(
    anchor: &Anchor,
    relative: &Path,
    expected: &EntryIdentity,
) -> io::Result<()> {
    if AnchoredFs::fingerprint_relative(anchor, relative)? == expected.fingerprint {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "renamed entry changed while its identity was retained",
        ))
    }
}

impl Anchor {
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            path: self.path.clone(),
            dir: self.dir.try_clone()?,
        })
    }
}

/// Filesystem authority bound to already-open approved directory handles.
///
/// Ambient paths are accepted only to select an anchor and produce diagnostics.
/// Every read, write, link, copy, and rename is then resolved relative to the
/// retained handle, so a concurrent ancestor rename or symlink swap cannot
/// redirect the operation outside that authority.
pub(crate) struct AnchoredFs<'a> {
    anchors: Vec<Anchor>,
    state_path: PathBuf,
    directory_sync: &'a dyn DirectorySync,
}

impl<'a> AnchoredFs<'a> {
    pub(crate) fn state_root(&self) -> &Path {
        &self.state_path
    }

    pub(crate) fn matches_state_directory(&self, path: &Path) -> io::Result<bool> {
        let retained = self
            .anchors
            .iter()
            .find(|anchor| anchor.path == self.state_path)
            .ok_or_else(|| io::Error::other("state capability is missing"))?;
        let candidate_path = fs::canonicalize(path)?;
        let candidate = open_anchor(candidate_path, false)?;
        let retained = retained.dir.try_clone()?.into_std_file();
        let candidate = candidate.dir.into_std_file();
        same_file_identity(&retained, &candidate)
    }

    /// Refuse mutation before the first write when the filesystem cannot
    /// provide an atomic descriptor-relative no-replace rename. An empty
    /// source name makes this probe mutation-free: supported kernels return
    /// ENOENT, while WSL1 and unsupported filesystems reject the flag.
    #[cfg(target_os = "linux")]
    pub(crate) fn require_atomic_noreplace_rename(&self) -> io::Result<()> {
        use std::os::fd::AsRawFd as _;

        if take_forced_atomic_noreplace_unsupported() {
            return Err(unsupported_atomic_noreplace_error());
        }

        let state = self
            .anchors
            .iter()
            .find(|anchor| anchor.path == self.state_path)
            .ok_or_else(|| io::Error::other("state capability is missing"))?;
        let empty = c"";
        let result = unsafe {
            libc::renameat2(
                state.dir.as_raw_fd(),
                empty.as_ptr(),
                state.dir.as_raw_fd(),
                empty.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            return Err(io::Error::other(
                "atomic no-replace rename probe unexpectedly mutated an empty path",
            ));
        }
        classify_atomic_noreplace_probe(io::Error::last_os_error())
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn require_atomic_noreplace_rename(&self) -> io::Result<()> {
        if take_forced_atomic_noreplace_unsupported() {
            Err(unsupported_atomic_noreplace_error())
        } else {
            Ok(())
        }
    }

    pub(crate) fn open(
        approved_roots: &[PathBuf],
        state_dir: &Path,
        directory_sync: &'a dyn DirectorySync,
    ) -> io::Result<Self> {
        let mut paths = approved_roots
            .iter()
            .filter(|path| path.is_dir() && !path.starts_with(state_dir))
            .cloned()
            .collect::<Vec<_>>();
        paths.push(state_dir.to_path_buf());
        paths.sort();
        paths.dedup();

        let mut anchors = paths
            .into_iter()
            .map(|path| {
                let approved_root = path != state_dir;
                if approved_root {
                    run_before_approved_root_open_hook();
                }
                open_anchor(path, approved_root)
            })
            .collect::<io::Result<Vec<_>>>()?;
        anchors.sort_by(|left, right| {
            right
                .path
                .components()
                .count()
                .cmp(&left.path.components().count())
        });
        Ok(Self {
            anchors,
            state_path: state_dir.to_path_buf(),
            directory_sync,
        })
    }

    pub(crate) fn open_with_retained_state(
        approved_roots: &[PathBuf],
        state_dir: &Path,
        retained: &AnchoredFs<'_>,
        directory_sync: &'a dyn DirectorySync,
    ) -> io::Result<Self> {
        if !retained.matches_state_directory(state_dir)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "retained capability does not own the requested state directory",
            ));
        }
        let mut retained_state = retained
            .anchors
            .iter()
            .find(|anchor| anchor.path == retained.state_path)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "retained capability does not own the requested state directory",
                )
            })?
            .try_clone()?;
        retained_state.path = state_dir.to_path_buf();
        let mut paths = approved_roots
            .iter()
            .filter(|path| path.is_dir() && !path.starts_with(state_dir))
            .cloned()
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        let mut anchors = paths
            .into_iter()
            .map(|path| {
                run_before_approved_root_open_hook();
                open_anchor(path, true)
            })
            .collect::<io::Result<Vec<_>>>()?;
        anchors.push(retained_state);
        anchors.sort_by(|left, right| {
            right
                .path
                .components()
                .count()
                .cmp(&left.path.components().count())
        });
        Ok(Self {
            anchors,
            state_path: state_dir.to_path_buf(),
            directory_sync,
        })
    }

    pub(crate) fn fingerprint(&self, path: &Path) -> io::Result<String> {
        let (anchor, relative) = self.resolve(path)?;
        match Self::fingerprint_relative(anchor, &relative) {
            Ok(value) => Ok(value),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok("missing".into()),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn package_fingerprint(&self, path: &Path) -> io::Result<String> {
        let (anchor, relative) = self.resolve(path)?;
        let metadata = match anchor.dir.symlink_metadata(&relative) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok("missing".into());
            }
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Skill package path is not a directory",
            ));
        }
        let mut hashes = PackageHashBuilder::new();
        Self::hash_package_relative(anchor, &relative, Path::new(""), 0, &mut hashes)?;
        Ok(format!("package:sha256:{}", hashes.finish().digest))
    }

    fn hash_package_relative(
        anchor: &Anchor,
        directory: &Path,
        package_relative: &Path,
        depth: usize,
        hashes: &mut PackageHashBuilder,
    ) -> io::Result<()> {
        if depth > MAX_SKILL_PACKAGE_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Skill package fingerprint exceeds depth {MAX_SKILL_PACKAGE_DEPTH}"),
            ));
        }
        let mut entries = anchor
            .dir
            .read_dir(directory)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort();
        for name in entries {
            if ignored_package_entry_name(&name) {
                continue;
            }
            let child = directory.join(&name);
            let relative_child = package_relative.join(&name);
            let relative_text = relative_child.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "non-Unicode path cannot participate in stable identity",
                )
            })?;
            let metadata = anchor.dir.symlink_metadata(&child)?;
            if metadata.is_dir() {
                Self::hash_package_relative(anchor, &child, &relative_child, depth + 1, hashes)?;
            } else if metadata.is_symlink() {
                let target = anchor.dir.read_link_contents(&child)?;
                let target = target.to_str().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "non-Unicode path cannot participate in stable identity",
                    )
                })?;
                hashes.add_symlink(relative_text, target);
            } else if metadata.is_file() {
                let mut bytes = Vec::new();
                anchor
                    .dir
                    .open(&child)?
                    .take(hashes.remaining_bytes().saturating_add(1))
                    .read_to_end(&mut bytes)?;
                hashes.add_regular_file(relative_text, &bytes)?;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Skill package contains an unsupported file type",
                ));
            }
        }
        Ok(())
    }

    fn fingerprint_relative(anchor: &Anchor, relative: &Path) -> io::Result<String> {
        let metadata = anchor.dir.symlink_metadata(relative)?;
        if metadata.is_symlink() {
            let target = anchor.dir.read_link_contents(relative)?;
            return Ok(format!(
                "symlink:sha256:{}",
                hex::encode(Sha256::digest(target.as_os_str().as_encoded_bytes()))
            ));
        }
        if metadata.is_file() {
            let mut file = anchor.dir.open(relative)?;
            let mut hash = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hash.update(&buffer[..read]);
            }
            return Ok(format!("file:sha256:{}", hex::encode(hash.finalize())));
        }
        if metadata.is_dir() {
            let mut entries = anchor
                .dir
                .read_dir(relative)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<OsString>>>()?;
            entries.sort();
            let mut hash = Sha256::new();
            for name in entries {
                hash.update(name.as_encoded_bytes());
                hash.update(Self::fingerprint_relative(anchor, &relative.join(&name))?.as_bytes());
            }
            return Ok(format!("directory:sha256:{}", hex::encode(hash.finalize())));
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a file, directory, or symlink",
        ))
    }

    pub(crate) fn create_dir(&self, path: &Path) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        anchor.dir.create_dir(&relative)?;
        let opened = anchor.dir.open_dir(&relative)?.into_std_file();
        run_after_created_directory_open_hook();
        self.require_opened_directory_at(anchor, &relative, &opened)?;
        run_after_created_entry_first_check_hook();
        self.sync_parent_preserving_identity(anchor, &relative, || {
            self.require_opened_directory_at(anchor, &relative, &opened)
        })
    }

    pub(crate) fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        let mut current = PathBuf::new();
        for component in relative.components() {
            current.push(component.as_os_str());
            match anchor.dir.create_dir(&current) {
                Ok(()) => {
                    let opened = anchor.dir.open_dir(&current)?.into_std_file();
                    self.require_opened_directory_at(anchor, &current, &opened)?;
                    self.sync_parent_preserving_identity(anchor, &current, || {
                        self.require_opened_directory_at(anchor, &current, &opened)
                    })?;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = anchor.dir.symlink_metadata(&current)?;
                    if metadata.is_symlink() || !metadata.is_dir() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "existing path component is not a directory",
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn secure_private_directory(&self, path: &Path) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let directory = open_directory_for_sync(&anchor.dir, &relative)?;
            directory.set_permissions(fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(not(unix))]
        {
            let _ = (anchor, relative);
        }
        Ok(())
    }

    pub(crate) fn open_private_lock(&self, path: &Path) -> io::Result<fs::File> {
        let (anchor, relative) = self.resolve(path)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;

            options.mode(0o600);
        }
        let file = anchor.dir.open_with(&relative, &options)?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt as _;

            file.set_permissions(Permissions::from_mode(0o600))?;
        }
        self.sync_parent(anchor, &relative)?;
        Ok(file.into_std())
    }

    pub(crate) fn read_directory_names(&self, path: &Path) -> io::Result<Vec<OsString>> {
        let (anchor, relative) = self.resolve(path)?;
        let mut names = anchor
            .dir
            .read_dir(relative)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }

    pub(crate) fn read_regular_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        let (anchor, relative) = self.resolve(path)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
        let mut file = anchor.dir.open_with(relative, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path is not a regular file",
            ));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(crate) fn create_file(&self, path: &Path, content: &[u8]) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = anchor.dir.open_with(&relative, &options)?;
        run_after_staging_file_open_hook();
        file.write_all(content)?;
        file.sync_all()?;
        self.require_opened_file_at(anchor, &relative, &file)?;
        run_after_created_entry_first_check_hook();
        self.sync_parent_preserving_identity(anchor, &relative, || {
            self.require_opened_file_at(anchor, &relative, &file)
        })
    }

    pub(crate) fn write_private_file_atomic(
        &self,
        path: &Path,
        temp: &Path,
        content: &[u8],
    ) -> io::Result<()> {
        let (path_anchor, path_relative) = self.resolve(path)?;
        let (temp_anchor, temp_relative) = self.resolve(temp)?;
        if !std::ptr::eq(path_anchor, temp_anchor) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "private staging path must share the target anchor",
            ));
        }
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create(true)
            .truncate(true)
            ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;

            options.mode(0o600);
        }
        let mut file = temp_anchor.dir.open_with(&temp_relative, &options)?;
        run_after_staging_file_open_hook();
        file.write_all(content)?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt as _;

            file.set_permissions(Permissions::from_mode(0o600))?;
        }
        file.sync_all()?;
        self.require_opened_file_at(temp_anchor, &temp_relative, &file)?;
        path_anchor
            .dir
            .rename(&temp_relative, &path_anchor.dir, &path_relative)?;
        self.require_opened_file_at(path_anchor, &path_relative, &file)?;
        self.sync_parent_preserving_identity(path_anchor, &path_relative, || {
            self.require_opened_file_at(path_anchor, &path_relative, &file)
        })
    }

    pub(crate) fn replace_file(
        &self,
        target: &Path,
        temp: &Path,
        content: &[u8],
    ) -> io::Result<()> {
        let (target_anchor, target_relative) = self.resolve(target)?;
        let (temp_anchor, temp_relative) = self.resolve(temp)?;
        if !std::ptr::eq(target_anchor, temp_anchor) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "replacement staging path must share the target anchor",
            ));
        }
        let original = target_anchor.dir.open(&target_relative)?;
        let original_handle = original.try_clone()?.into_std();
        let metadata = CopyMetadata::read(&original_handle)?;
        let staged =
            self.create_file_with_metadata(temp_anchor, &temp_relative, content, &metadata)?;
        metadata.verify(&original_handle)?;
        self.require_opened_file_at(target_anchor, &target_relative, &original)?;
        self.remove_regular_file(target_anchor, &target_relative)?;
        // Windows delete disposition remains pending until our retained source
        // handles close. Keep them through validation/removal, but release them
        // before syncing the removal and publishing the replacement name.
        drop(original_handle);
        drop(original);
        self.sync_parent(target_anchor, &target_relative)?;
        self.rename(temp, target)?;
        self.require_opened_file_at(target_anchor, &target_relative, &staged)?;
        Ok(())
    }

    pub(crate) fn create_symlink(&self, source: &Path, target: &Path) -> io::Result<()> {
        let source_handle = self.open_object_handle(source)?;
        run_after_symlink_source_open_hook();
        if !same_opened_path(&source_handle, source, false)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "symlink source entry changed after its capability was opened",
            ));
        }
        let (target_anchor, target_relative) = self.resolve(target)?;
        #[cfg(unix)]
        let link_source = source.to_path_buf();
        #[cfg(windows)]
        let (link_source, source_is_dir) = {
            let target_parent = target.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "symlink target has no parent")
            })?;
            let resolved_source = if source.is_absolute() {
                source.to_path_buf()
            } else {
                target_parent.join(source)
            };
            let (source_anchor, source_relative) = self.resolve(&resolved_source)?;
            let link_source = if source.is_absolute() {
                relative_path(target_parent, source).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "cannot create a handle-bound symlink across Windows volumes",
                    )
                })?
            } else {
                source.to_path_buf()
            };
            (
                link_source,
                source_anchor.dir.metadata(source_relative)?.is_dir(),
            )
        };
        #[cfg(unix)]
        target_anchor
            .dir
            .symlink_contents(&link_source, &target_relative)?;
        #[cfg(windows)]
        {
            if source_is_dir {
                target_anchor
                    .dir
                    .symlink_dir(&link_source, &target_relative)?;
            } else {
                target_anchor
                    .dir
                    .symlink_file(&link_source, &target_relative)?;
            }
        }
        run_after_created_symlink_hook();
        self.require_symlink_contents_at(target_anchor, &target_relative, &link_source)?;
        run_after_created_symlink_first_check_hook();
        if let Err(error) = self.sync_parent(target_anchor, &target_relative) {
            let identity =
                self.require_symlink_contents_at(target_anchor, &target_relative, &link_source);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                match identity {
                    Ok(()) => format!(
                        "created symlink durability is uncertain after directory sync failed: {error}"
                    ),
                    Err(identity_error) => format!(
                        "created symlink identity and durability are uncertain: {identity_error}; {error}"
                    ),
                },
            ));
        }
        self.require_symlink_contents_at(target_anchor, &target_relative, &link_source)?;
        if same_opened_path(&source_handle, source, false)?
            && same_opened_path(&source_handle, target, true)?
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "published symlink does not resolve to the retained source object",
            ))
        }
    }

    /// Restores previously recorded link contents without reinterpreting them as
    /// fresh source authority. The target entry is still created and synced
    /// relative to its retained approved-root handle.
    #[cfg(unix)]
    pub(crate) fn restore_symlink_contents(&self, source: &Path, target: &Path) -> io::Result<()> {
        let (target_anchor, target_relative) = self.resolve(target)?;
        target_anchor
            .dir
            .symlink_contents(source, &target_relative)?;
        self.sync_parent(target_anchor, &target_relative)
    }

    #[cfg(windows)]
    pub(crate) fn restore_symlink_contents(&self, source: &Path, target: &Path) -> io::Result<()> {
        if source.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "legacy absolute symlink contents cannot be restored handle-bound on Windows",
            ));
        }
        let target_parent = target.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "symlink target has no parent")
        })?;
        let resolved_source = target_parent.join(source);
        let source_handle = self.open_object_handle(&resolved_source)?;
        run_after_symlink_source_open_hook();
        if !same_opened_path(&source_handle, &resolved_source, false)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "legacy symlink source entry changed after its capability was opened",
            ));
        }
        let (target_anchor, target_relative) = self.resolve(target)?;
        if source_handle.metadata()?.is_dir() {
            target_anchor.dir.symlink_dir(source, &target_relative)?;
        } else {
            target_anchor.dir.symlink_file(source, &target_relative)?;
        }
        self.require_symlink_contents_at(target_anchor, &target_relative, source)?;
        if !same_opened_path(&source_handle, &resolved_source, false)?
            || !same_opened_path(&source_handle, target, true)?
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "legacy symlink restoration did not publish the retained source object",
            ));
        }
        self.sync_parent(target_anchor, &target_relative)?;
        self.require_symlink_contents_at(target_anchor, &target_relative, source)?;
        if same_opened_path(&source_handle, &resolved_source, false)?
            && same_opened_path(&source_handle, target, true)?
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "legacy symlink source changed before restoration completed",
            ))
        }
    }

    pub(crate) fn remove_file(&self, path: &Path) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        self.remove_regular_file(anchor, &relative)?;
        self.sync_parent(anchor, &relative)
    }

    pub(crate) fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let (from_anchor, from_relative) = self.resolve(from)?;
        let (to_anchor, to_relative) = self.resolve(to)?;
        let retained = entry_identity_at(from_anchor, &from_relative)?;
        require_entry_identity_at(from_anchor, &from_relative, &retained)?;
        run_before_rename_noreplace_hook();
        if let Err(error) = rename_noreplace(
            from_anchor,
            &from_relative,
            to_anchor,
            &to_relative,
            &retained,
        ) {
            return Err(if error.kind() == io::ErrorKind::AlreadyExists {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "rename destination appeared after it was validated as missing",
                )
            } else {
                error
            });
        }
        require_entry_identity_at(to_anchor, &to_relative, &retained)?;
        run_after_renamed_entry_first_check_hook();
        self.sync_parent_preserving_identity(from_anchor, &from_relative, || {
            require_entry_identity_at(to_anchor, &to_relative, &retained)
        })?;
        if std::ptr::eq(from_anchor, to_anchor) && from_relative.parent() == to_relative.parent() {
            return require_entry_identity_at(to_anchor, &to_relative, &retained);
        }
        self.sync_parent_preserving_identity(to_anchor, &to_relative, || {
            require_entry_identity_at(to_anchor, &to_relative, &retained)
        })
    }

    pub(crate) fn original_windows_security(&self, source: &Path) -> io::Result<Option<String>> {
        let (anchor, relative) = self.resolve(source)?;
        CopyMetadata::read(&anchor.dir.open(relative)?.into_std())
            .map(|metadata| metadata.windows_security())
    }

    pub(crate) fn copy_private_backup(&self, source: &Path, target: &Path) -> io::Result<u64> {
        self.copy_file_to(source, target, CopyDestination::PrivateBackup)
    }

    pub(crate) fn restore_private_backup(
        &self,
        source: &Path,
        target: &Path,
        original_security: Option<&str>,
    ) -> io::Result<u64> {
        self.copy_file_to(source, target, CopyDestination::Restore(original_security))
    }

    pub(crate) fn verify_restoration_metadata(
        &self,
        backup: &Path,
        target: &Path,
        original_security: Option<&str>,
    ) -> io::Result<()> {
        let (backup_anchor, backup_relative) = self.resolve(backup)?;
        let backup = backup_anchor.dir.open(backup_relative)?.into_std();
        let (target_anchor, target_relative) = self.resolve(target)?;
        let target = target_anchor.dir.open(target_relative)?.into_std();
        CopyMetadata::read(&backup)?
            .for_destination(&target, CopyDestination::Restore(original_security))?
            .verify(&target)
    }

    fn copy_file_to(
        &self,
        source: &Path,
        target: &Path,
        purpose: CopyDestination<'_>,
    ) -> io::Result<u64> {
        let (source_anchor, source_relative) = self.resolve(source)?;
        let (target_anchor, target_relative) = self.resolve(target)?;
        self.copy_file_relative(
            source_anchor,
            &source_relative,
            target_anchor,
            &target_relative,
            purpose,
        )
    }

    pub(crate) fn copy_tree(&self, source: &Path, target: &Path) -> io::Result<()> {
        let (source_anchor, source_relative) = self.resolve(source)?;
        let (target_anchor, target_relative) = self.resolve(target)?;
        self.copy_tree_relative(
            source_anchor,
            &source_relative,
            target_anchor,
            &target_relative,
        )
    }

    fn copy_tree_relative(
        &self,
        source_anchor: &Anchor,
        source: &Path,
        target_anchor: &Anchor,
        target: &Path,
    ) -> io::Result<()> {
        let metadata = source_anchor.dir.symlink_metadata(source)?;
        if metadata.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to recursively copy a symlink",
            ));
        }
        if metadata.is_file() {
            self.copy_file_relative(
                source_anchor,
                source,
                target_anchor,
                target,
                CopyDestination::Preserve,
            )?;
            return Ok(());
        }
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a supported file type",
            ));
        }
        let source_directory = open_readable_directory(&source_anchor.dir, source)?;
        let copy_metadata = CopyMetadata::read(&source_directory)?;
        #[cfg(unix)]
        {
            use cap_std::fs::DirBuilderExt;
            let mut builder = cap_std::fs::DirBuilder::new();
            builder.mode(0o700);
            target_anchor.dir.create_dir_with(target, &builder)?;
        }
        #[cfg(not(unix))]
        target_anchor.dir.create_dir(target)?;
        let opened = open_directory_for_sync(&target_anchor.dir, target)?;
        copy_metadata.validate_destination(&opened)?;
        self.require_opened_directory_at(target_anchor, target, &opened)?;
        run_after_created_entry_first_check_hook();
        self.sync_parent_preserving_identity(target_anchor, target, || {
            self.require_opened_directory_at(target_anchor, target, &opened)
        })?;
        let entries = source_anchor
            .dir
            .read_dir(source)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        for name in entries {
            self.copy_tree_relative(
                source_anchor,
                &source.join(&name),
                target_anchor,
                &target.join(name),
            )?;
        }
        copy_metadata.verify(&source_directory)?;
        copy_metadata.apply_to(&opened)?;
        opened.sync_all()?;
        Ok(())
    }

    fn create_file_with_metadata(
        &self,
        anchor: &Anchor,
        relative: &Path,
        content: &[u8],
        metadata: &CopyMetadata,
    ) -> io::Result<cap_std::fs::File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::{OpenOptionsExt as _, PermissionsExt as _};

            options.mode(metadata.permissions().mode() & 0o7777);
        }
        let mut file = anchor.dir.open_with(relative, &options)?;
        run_after_staging_file_open_hook();
        let handle = file.try_clone()?.into_std();
        metadata.apply_to(&handle)?;
        file.write_all(content)?;
        metadata.apply_to(&handle)?;
        file.sync_all()?;
        self.require_opened_file_at(anchor, relative, &file)?;
        run_after_created_entry_first_check_hook();
        self.sync_parent_preserving_identity(anchor, relative, || {
            self.require_opened_file_at(anchor, relative, &file)
        })?;
        Ok(file)
    }

    fn copy_file_relative(
        &self,
        source_anchor: &Anchor,
        source: &Path,
        target_anchor: &Anchor,
        target: &Path,
        purpose: CopyDestination<'_>,
    ) -> io::Result<u64> {
        let mut source_file = source_anchor.dir.open(source)?;
        let source_handle = source_file.try_clone()?.into_std();
        let metadata = CopyMetadata::read(&source_handle)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::{OpenOptionsExt as _, PermissionsExt as _};

            options.mode(metadata.permissions().mode() & 0o7777);
        }
        let mut target_file = target_anchor.dir.open_with(target, &options)?;
        run_after_staging_file_open_hook();
        let target_handle = target_file.try_clone()?.into_std();
        let destination_metadata = metadata.for_destination(&target_handle, purpose)?;
        destination_metadata.apply_to(&target_handle)?;
        let copied = io::copy(&mut source_file, &mut target_file)?;
        metadata.verify(&source_handle)?;
        destination_metadata.apply_to(&target_handle)?;
        self.require_opened_file_at(source_anchor, source, &source_file)?;
        target_file.sync_all()?;
        self.require_opened_file_at(target_anchor, target, &target_file)?;
        run_after_created_entry_first_check_hook();
        self.sync_parent_preserving_identity(target_anchor, target, || {
            self.require_opened_file_at(target_anchor, target, &target_file)
        })?;
        Ok(copied)
    }

    fn require_opened_file_at(
        &self,
        anchor: &Anchor,
        relative: &Path,
        opened: &cap_std::fs::File,
    ) -> io::Result<()> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
        let current = anchor.dir.open_with(relative, &options)?.into_std();
        let opened = opened.try_clone()?.into_std();
        if same_file_identity(&opened, &current)? {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "created file entry changed while its handle was retained",
            ))
        }
    }

    fn require_opened_directory_at(
        &self,
        anchor: &Anchor,
        relative: &Path,
        opened: &fs::File,
    ) -> io::Result<()> {
        let current = anchor.dir.open_dir(relative)?.into_std_file();
        if same_file_identity(opened, &current)? {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "created directory entry changed while its handle was retained",
            ))
        }
    }

    fn require_symlink_contents_at(
        &self,
        anchor: &Anchor,
        relative: &Path,
        expected: &Path,
    ) -> io::Result<()> {
        let metadata = anchor.dir.symlink_metadata(relative)?;
        if !metadata.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "created symlink entry changed before publication completed",
            ));
        }
        let actual = anchor.dir.read_link_contents(relative)?;
        if actual == expected {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "created symlink entry changed before publication completed",
            ))
        }
    }

    fn sync_parent(&self, anchor: &Anchor, relative: &Path) -> io::Result<()> {
        let parent = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let directory = open_directory_for_sync(&anchor.dir, parent)?;
        self.directory_sync
            .sync_directory_handle(&directory, &anchor.path.join(parent))
    }

    fn sync_parent_preserving_identity(
        &self,
        anchor: &Anchor,
        relative: &Path,
        verify: impl Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        match self.sync_parent(anchor, relative) {
            Ok(()) => verify(),
            Err(sync_error) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                match verify() {
                    Ok(()) => format!(
                        "published entry durability is uncertain after directory sync failed: {sync_error}"
                    ),
                    Err(identity_error) => format!(
                        "published entry identity and durability are uncertain: {identity_error}; {sync_error}"
                    ),
                },
            )),
        }
    }

    fn open_object_handle(&self, path: &Path) -> io::Result<fs::File> {
        let (anchor, relative) = self.resolve(path)?;
        let metadata = anchor.dir.symlink_metadata(&relative)?;
        if metadata.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a non-symlink source object",
            ));
        }
        if metadata.is_dir() {
            Ok(anchor.dir.open_dir(relative)?.into_std_file())
        } else if metadata.is_file() {
            Ok(anchor.dir.open(relative)?.into_std())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a file or directory",
            ))
        }
    }

    #[cfg(not(windows))]
    fn remove_regular_file(&self, anchor: &Anchor, relative: &Path) -> io::Result<()> {
        anchor.dir.remove_file(relative)
    }

    #[cfg(windows)]
    fn remove_regular_file(&self, anchor: &Anchor, relative: &Path) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle as _;
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
            FILE_DISPOSITION_INFO_EX, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfoEx, SetFileInformationByHandle,
        };

        use cap_std::fs::OpenOptionsExt as _;

        let mut options = OpenOptions::new();
        options
            .access_mode(DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        let file = anchor.dir.open_with(relative, &options)?.into_std();
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path is not a regular file",
            ));
        }
        let disposition = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        };
        let removed = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfoEx,
                (&raw const disposition).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        };
        if removed == 0 {
            return Err(io::Error::last_os_error());
        }
        drop(file);
        Ok(())
    }

    fn resolve<'b>(&'b self, path: &Path) -> io::Result<(&'b Anchor, PathBuf)> {
        for anchor in &self.anchors {
            if let Ok(relative) = path.strip_prefix(&anchor.path) {
                let relative = if relative.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    relative.to_path_buf()
                };
                if relative.is_absolute()
                    || relative.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::Prefix(_)
                                | std::path::Component::RootDir
                        )
                    })
                {
                    break;
                }
                return Ok((anchor, relative));
            }
        }
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("path has no approved directory handle: {}", path.display()),
        ))
    }
}

fn open_anchor(path: PathBuf, approved_root: bool) -> io::Result<Anchor> {
    let handle_path = fs::canonicalize(&path)?;
    if approved_root && handle_path != path {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("approved root path is not canonical: {}", path.display()),
        ));
    }
    if approved_root {
        run_after_approved_root_canonicalize_hook();
    }
    let dir = open_directory_handle_bound(&handle_path)?;
    Ok(Anchor { path, dir })
}

#[cfg(unix)]
fn open_directory_handle_bound(path: &Path) -> io::Result<Dir> {
    use cap_std::fs::OpenOptionsExt as _;

    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "approved root must be absolute",
        ));
    }
    let mut current = Dir::open_ambient_dir(Path::new("/"), ambient_authority())?;
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW);
        let next = current.open_with(Path::new(name), &options)?.into_std();
        current = Dir::from_std_file(next);
    }
    Ok(current)
}

#[cfg(windows)]
fn open_directory_handle_bound(path: &Path) -> io::Result<Dir> {
    use cap_std::fs::OpenOptionsExt as _;
    use std::os::windows::fs::MetadataExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };

    let mut components = path.components();
    let prefix = match components.next() {
        Some(std::path::Component::Prefix(prefix)) => prefix.as_os_str(),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "approved root must include a Windows volume prefix",
            ));
        }
    };
    if !matches!(components.next(), Some(std::path::Component::RootDir)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "approved root must be absolute",
        ));
    }
    let mut volume_root = PathBuf::from(prefix);
    volume_root.push("\\");
    let mut current = Dir::open_ambient_dir(volume_root, ambient_authority())?;
    for component in components {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "approved root contains an unsupported component",
            ));
        };
        let mut options = OpenOptions::new();
        options
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let next = current.open_with(Path::new(name), &options)?.into_std();
        let attributes = next.metadata()?.file_attributes();
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || attributes & FILE_ATTRIBUTE_DIRECTORY == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "approved root traversal encountered a reparse point or non-directory",
            ));
        }
        current = Dir::from_std_file(next);
    }
    Ok(current)
}

#[cfg(not(any(unix, windows)))]
fn open_directory_handle_bound(path: &Path) -> io::Result<Dir> {
    Dir::open_ambient_dir(path, ambient_authority())
}

#[cfg(unix)]
fn open_directory_for_sync(dir: &Dir, relative: &Path) -> io::Result<fs::File> {
    open_readable_directory(dir, relative)
}

fn open_readable_directory(dir: &Dir, relative: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    }
    let file = dir.open_with(relative, &options)?.into_std();
    if !file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "opened metadata source is not a directory",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn open_directory_for_sync(dir: &Dir, relative: &Path) -> io::Result<fs::File> {
    use cap_std::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    Ok(dir.open_with(relative, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_directory_for_sync(dir: &Dir, relative: &Path) -> io::Result<fs::File> {
    Ok(dir.open_dir(relative)?.into_std_file())
}

#[cfg(windows)]
fn relative_path(from: &Path, to: &Path) -> Option<PathBuf> {
    let from = from.components().collect::<Vec<_>>();
    let to = to.components().collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        return None;
    }

    let mut relative = PathBuf::new();
    for component in &from[common..] {
        if matches!(component, std::path::Component::Normal(_)) {
            relative.push("..");
        }
    }
    for component in &to[common..] {
        relative.push(component.as_os_str());
    }
    Some(relative)
}

#[cfg(unix)]
fn same_opened_path(opened: &fs::File, path: &Path, follow: bool) -> io::Result<bool> {
    let options = if follow {
        fs::File::open(path)
    } else {
        use std::os::unix::fs::OpenOptionsExt as _;

        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }?;
    same_file_identity(opened, &options)
}

#[cfg(unix)]
fn same_file_identity(left: &fs::File, right: &fs::File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt as _;

    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn same_opened_path(opened: &fs::File, path: &Path, follow: bool) -> io::Result<bool> {
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::FromRawHandle as _;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let flags = if follow {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    };
    let entry = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if entry == INVALID_HANDLE_VALUE || entry.is_null() {
        return Err(io::Error::last_os_error());
    }
    let entry = unsafe { fs::File::from_raw_handle(entry.cast()) };
    same_file_identity(opened, &entry)
}

#[cfg(windows)]
fn same_file_identity(left: &fs::File, right: &fs::File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    fn identity(handle: windows_sys::Win32::Foundation::HANDLE) -> io::Result<(u32, u32, u32)> {
        let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            information.dwVolumeSerialNumber,
            information.nFileIndexHigh,
            information.nFileIndexLow,
        ))
    }

    Ok(identity(left.as_raw_handle())? == identity(right.as_raw_handle())?)
}

#[cfg(all(test, target_os = "linux"))]
mod linux_probe_tests {
    use super::classify_atomic_noreplace_probe;
    use std::io;

    #[test]
    fn atomic_noreplace_probe_accepts_missing_source() {
        assert!(
            classify_atomic_noreplace_probe(io::Error::from_raw_os_error(libc::ENOENT)).is_ok()
        );
    }

    #[test]
    fn running_kernel_accepts_the_mutation_free_probe() {
        let empty = c"";
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                empty.as_ptr(),
                libc::AT_FDCWD,
                empty.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        assert_eq!(result, -1);
        classify_atomic_noreplace_probe(io::Error::last_os_error())
            .expect("the test kernel must support atomic no-replace rename");
    }

    #[test]
    fn atomic_noreplace_probe_rejects_unsupported_kernel_or_filesystem() {
        for code in [libc::EINVAL, libc::ENOSYS, libc::EOPNOTSUPP] {
            let error = classify_atomic_noreplace_probe(io::Error::from_raw_os_error(code))
                .expect_err("unsupported no-replace rename must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        }
    }
}
