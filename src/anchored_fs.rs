use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};

struct Anchor {
    path: PathBuf,
    dir: Dir,
}

/// Filesystem authority bound to already-open approved directory handles.
///
/// Ambient paths are accepted only to select an anchor and produce diagnostics.
/// Every read, write, link, copy, and rename is then resolved relative to the
/// retained handle, so a concurrent ancestor rename or symlink swap cannot
/// redirect the operation outside that authority.
pub(crate) struct AnchoredFs {
    anchors: Vec<Anchor>,
}

impl AnchoredFs {
    pub(crate) fn open(approved_roots: &[PathBuf], state_dir: &Path) -> io::Result<Self> {
        let mut paths = approved_roots
            .iter()
            .filter(|path| path.is_dir() && !path.starts_with(state_dir))
            .cloned()
            .collect::<Vec<_>>();
        paths.push(state_dir.to_path_buf());
        paths.sort();
        paths.dedup();

        let mut anchors = Vec::with_capacity(paths.len());
        for path in paths {
            let dir = Dir::open_ambient_dir(&path, ambient_authority())?;
            let opened = dir.try_clone()?.into_std_file();
            let entry = fs::symlink_metadata(&path)?;
            if entry.file_type().is_symlink() || !same_directory(&opened, &path)? {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "approved root entry changed while its directory handle was opened: {}",
                        path.display()
                    ),
                ));
            }
            anchors.push(Anchor { path, dir });
        }
        anchors.sort_by(|left, right| {
            right
                .path
                .components()
                .count()
                .cmp(&left.path.components().count())
        });
        Ok(Self { anchors })
    }

    pub(crate) fn fingerprint(&self, path: &Path) -> io::Result<String> {
        let (anchor, relative) = self.resolve(path)?;
        match Self::fingerprint_relative(anchor, &relative) {
            Ok(value) => Ok(value),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok("missing".into()),
            Err(error) => Err(error),
        }
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
        anchor.dir.create_dir(relative)
    }

    pub(crate) fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        anchor.dir.create_dir_all(relative)
    }

    pub(crate) fn create_file(&self, path: &Path, content: &[u8]) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = anchor.dir.open_with(relative, &options)?;
        file.write_all(content)?;
        file.sync_all()
    }

    pub(crate) fn replace_file(
        &self,
        target: &Path,
        temp: &Path,
        content: &[u8],
    ) -> io::Result<()> {
        self.create_file(temp, content)?;
        self.remove_file(target)?;
        self.rename(temp, target)
    }

    pub(crate) fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        let (anchor, relative) = self.resolve(path)?;
        anchor.dir.read_link_contents(relative)
    }

    pub(crate) fn create_symlink(&self, source: &Path, target: &Path) -> io::Result<()> {
        let (target_anchor, target_relative) = self.resolve(target)?;
        #[cfg(unix)]
        {
            target_anchor.dir.symlink_contents(source, target_relative)
        }
        #[cfg(windows)]
        {
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
            if source_anchor.dir.metadata(source_relative)?.is_dir() {
                target_anchor.dir.symlink_dir(link_source, target_relative)
            } else {
                target_anchor.dir.symlink_file(link_source, target_relative)
            }
        }
    }

    pub(crate) fn remove_file(&self, path: &Path) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        anchor.dir.remove_file(relative)
    }

    pub(crate) fn remove_symlink(&self, path: &Path) -> io::Result<()> {
        let (anchor, relative) = self.resolve(path)?;
        #[cfg(unix)]
        {
            anchor.dir.remove_file(relative)
        }
        #[cfg(windows)]
        {
            match anchor.dir.remove_file(&relative) {
                Ok(()) => Ok(()),
                Err(file_error) => anchor.dir.remove_dir(relative).or(Err(file_error)),
            }
        }
    }

    pub(crate) fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let (from_anchor, from_relative) = self.resolve(from)?;
        let (to_anchor, to_relative) = self.resolve(to)?;
        from_anchor
            .dir
            .rename(from_relative, &to_anchor.dir, to_relative)
    }

    pub(crate) fn copy_file(&self, source: &Path, target: &Path) -> io::Result<u64> {
        let (source_anchor, source_relative) = self.resolve(source)?;
        let (target_anchor, target_relative) = self.resolve(target)?;
        source_anchor
            .dir
            .copy(source_relative, &target_anchor.dir, target_relative)
    }

    pub(crate) fn copy_tree(&self, source: &Path, target: &Path) -> io::Result<()> {
        let (source_anchor, source_relative) = self.resolve(source)?;
        let (target_anchor, target_relative) = self.resolve(target)?;
        Self::copy_tree_relative(
            source_anchor,
            &source_relative,
            target_anchor,
            &target_relative,
        )
    }

    fn copy_tree_relative(
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
            source_anchor.dir.copy(source, &target_anchor.dir, target)?;
            return Ok(());
        }
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a supported file type",
            ));
        }
        target_anchor.dir.create_dir(target)?;
        let entries = source_anchor
            .dir
            .read_dir(source)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        for name in entries {
            Self::copy_tree_relative(
                source_anchor,
                &source.join(&name),
                target_anchor,
                &target.join(name),
            )?;
        }
        Ok(())
    }

    fn resolve<'a>(&'a self, path: &Path) -> io::Result<(&'a Anchor, PathBuf)> {
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
fn same_directory(opened: &fs::File, path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt as _;

    let opened = opened.metadata()?;
    let entry = fs::symlink_metadata(path)?;
    Ok(opened.dev() == entry.dev() && opened.ino() == entry.ino())
}

#[cfg(windows)]
fn same_directory(opened: &fs::File, path: &Path) -> io::Result<bool> {
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
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

    let opened_identity = identity(opened.as_raw_handle())?;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let entry = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if entry == INVALID_HANDLE_VALUE || entry.is_null() {
        return Err(io::Error::last_os_error());
    }
    let entry_identity = identity(entry);
    unsafe { CloseHandle(entry) };
    Ok(opened_identity == entry_identity?)
}
