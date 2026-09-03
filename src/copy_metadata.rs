//! Metadata boundary for copy-based mutations. Unsupported observable metadata
//! is refused, not silently discarded. All inspection uses retained handles.

use std::fs::{File, Permissions};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CopyMetadata {
    permissions: Permissions,
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
    #[cfg(target_os = "macos")]
    provenance: Option<Vec<u8>>,
    #[cfg(windows)]
    security: windows::Security,
}

fn unsupported(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("unsupported copy metadata: {message}"),
    )
}

pub(crate) enum CopyDestination<'a> {
    Preserve,
    PrivateBackup,
    Restore(Option<&'a str>),
}

impl CopyMetadata {
    pub(crate) fn windows_security(&self) -> Option<String> {
        #[cfg(windows)]
        {
            Some(self.security.0.clone())
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    pub(crate) fn for_destination(
        &self,
        file: &File,
        purpose: CopyDestination<'_>,
    ) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let mut result = self.clone();
            match purpose {
                CopyDestination::Preserve => {}
                CopyDestination::PrivateBackup => {
                    crate::state_security::secure_opened_file(file)?;
                    result.security = windows::Security::read(file)?;
                }
                CopyDestination::Restore(security) => {
                    result.security = windows::Security(security.ok_or_else(|| unsupported(
                        "legacy Windows Receipt has no original owner/group/DACL evidence"))?.to_owned());
                }
            }
            result.validate_destination(file)?;
            Ok(result)
        }
        #[cfg(not(windows))]
        {
            if let CopyDestination::Restore(security) = purpose {
                let _ = security;
            }
            self.validate_destination(file)?;
            Ok(self.clone())
        }
    }

    pub(crate) fn validate_destination(&self, file: &File) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            validate_unix_extensions(file)?;
            if file.metadata()?.uid() != self.uid {
                return Err(unsupported("copy would change file ownership"));
            }
        }
        #[cfg(windows)]
        {
            windows::validate_extensions(file)?;
            self.security.apply_to(file)?;
        }
        Ok(())
    }

    pub(crate) fn read(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            validate_unix_extensions(file)?;
            // A non-owner may not be able to enumerate protected metadata.
            // Do not treat its absence from a listing as permission to discard it.
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err(unsupported("source is not owned by the current user"));
            }
            Ok(Self {
                permissions: metadata.permissions(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                #[cfg(target_os = "macos")]
                provenance: macos_provenance(file)?,
            })
        }
        #[cfg(windows)]
        {
            windows::validate_extensions(file)?;
            Ok(Self {
                permissions: metadata.permissions(),
                security: windows::Security::read(file)?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = metadata;
            Err(unsupported("platform inspection is unavailable"))
        }
    }

    pub(crate) fn permissions(&self) -> cap_std::fs::Permissions {
        cap_std::fs::Permissions::from_std(self.permissions.clone())
    }

    pub(crate) fn apply_to(&self, file: &File) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::MetadataExt;
            validate_unix_extensions(file)?;
            let current = file.metadata()?;
            if current.uid() != self.uid {
                return Err(unsupported("copy would change file ownership"));
            }
            if current.gid() != self.gid {
                // SAFETY: retained writable destination handle; uid=-1 leaves
                // its owner unchanged. Failure precedes destructive publication.
                if unsafe { libc::fchown(file.as_raw_fd(), !0, self.gid) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        #[cfg(windows)]
        self.security.apply_to(file)?;
        file.set_permissions(self.permissions.clone())?;
        self.verify(file)
    }

    pub(crate) fn verify(&self, file: &File) -> io::Result<()> {
        if Self::read(file)? == *self {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "copy metadata changed while its handle was retained",
            ))
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_unix_extensions(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: size zero queries the list length without dereferencing a buffer.
    #[cfg(target_os = "linux")]
    let length = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    #[cfg(target_os = "macos")]
    let length = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0) };
    if length < 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(target_os = "linux")]
    if length != 0 {
        return Err(unsupported(
            "extended attributes or POSIX ACLs require a metadata-preserving external workflow",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        // Current macOS automatically attaches provenance to ordinary new
        // files. Permit only this OS attribute, and compare its exact bytes in
        // CopyMetadata::verify; never remove it or overwrite it through a path.
        if length != 0 {
            let mut names = [0_u8; 64];
            let read = unsafe {
                libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len(), 0)
            };
            if read < 0 {
                return Err(io::Error::last_os_error());
            }
            if &names[..read as usize] != b"com.apple.provenance\0" {
                return Err(unsupported(
                    "extended attributes require a metadata-preserving external workflow",
                ));
            }
        }
        use std::ffi::c_void;
        use std::os::macos::fs::MetadataExt;
        unsafe extern "C" {
            fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_int) -> *mut c_void;
            fn acl_get_entry(
                acl: *mut c_void,
                entry_id: libc::c_int,
                entry: *mut *mut c_void,
            ) -> libc::c_int;
            fn acl_free(acl: *mut c_void) -> libc::c_int;
        }
        // macOS extended ACLs are separate from listxattr. A successful first
        // entry returns zero; an empty ACL returns -1/EINVAL (Darwin contract).
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
        if acl.is_null() {
            // Darwin filesec_get_property(FILESEC_ACL) returns ENOENT when
            // this retained object has no ACL property (Libc/acl_file.c).
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ENOENT) {
                return Err(error);
            }
        } else {
            let mut entry = std::ptr::null_mut();
            let result = unsafe { acl_get_entry(acl, 0, &mut entry) };
            let error = io::Error::last_os_error();
            unsafe { acl_free(acl) };
            if result == 0 {
                return Err(unsupported("macOS extended ACL"));
            }
            if error.raw_os_error() != Some(libc::EINVAL) {
                return Err(error);
            }
        }
        if file.metadata()?.st_flags() != 0 {
            return Err(unsupported("macOS file flags"));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_provenance(file: &File) -> io::Result<Option<Vec<u8>>> {
    use std::os::fd::AsRawFd;
    let mut bytes = [0_u8; 4096];
    // SAFETY: the retained descriptor and bounded writable buffer are valid.
    let length = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            c"com.apple.provenance".as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            0,
            0,
        )
    };
    if length < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOATTR) {
            return Ok(None);
        }
        return Err(error);
    }
    Ok(Some(bytes[..length as usize].to_vec()))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn validate_unix_extensions(_file: &File) -> io::Result<()> {
    Err(unsupported(
        "extended metadata inspection is unavailable on this Unix platform",
    ))
}

#[cfg(windows)]
mod windows;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_posix_acl_is_refused_and_retained() {
        use std::os::fd::AsRawFd;
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, "original").unwrap();
        let file = File::open(&path).unwrap();
        // Linux POSIX ACL xattr version 2: owner, named user, group, mask, other.
        let mut acl = 2_u32.to_le_bytes().to_vec();
        for (tag, permissions, id) in [
            (1_u16, 6_u16, u32::MAX),
            (2, 4, 1),
            (4, 4, u32::MAX),
            (16, 4, u32::MAX),
            (32, 0, u32::MAX),
        ] {
            acl.extend_from_slice(&tag.to_le_bytes());
            acl.extend_from_slice(&permissions.to_le_bytes());
            acl.extend_from_slice(&id.to_le_bytes());
        }
        let name = c"system.posix_acl_access";
        // SAFETY: the fixture-owned descriptor, name and value buffer are valid.
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    acl.as_ptr().cast(),
                    acl.len(),
                    0,
                )
            },
            0,
            "{}",
            io::Error::last_os_error()
        );
        assert!(
            CopyMetadata::read(&file)
                .unwrap_err()
                .to_string()
                .contains("ACL")
        );
        let mut retained = [0_u8; 128];
        let length = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                retained.as_mut_ptr().cast(),
                retained.len(),
            )
        };
        assert_eq!(length as usize, acl.len());
        assert_eq!(&retained[..acl.len()], &acl);
    }

    #[cfg(unix)]
    #[test]
    fn different_owner_is_refused_without_changing_destination() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, "original").unwrap();
        let file = File::open(&path).unwrap();
        let original = CopyMetadata::read(&file).unwrap();
        let mut wrong_owner = original.clone();
        wrong_owner.uid = wrong_owner.uid.wrapping_add(1);
        assert!(wrong_owner.apply_to(&file).is_err());
        assert_eq!(CopyMetadata::read(&file).unwrap(), original);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_extended_acl_is_refused_and_retained() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, "original").unwrap();
        assert!(
            std::process::Command::new("chmod")
                .args(["+a", "everyone allow read"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let file = File::open(&path).unwrap();
        let error = CopyMetadata::read(&file).unwrap_err();
        assert!(error.to_string().contains("extended ACL"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        // The refusal does not clear the ACL, so a second read still refuses.
        assert!(CopyMetadata::read(&file).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_named_stream_is_refused_without_removing_it() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, "original").unwrap();
        let stream = path.with_file_name("file:metadata");
        fs::write(&stream, "retained stream").unwrap();
        let error = CopyMetadata::read(&File::open(&path).unwrap()).unwrap_err();
        assert!(error.to_string().contains("named data streams"), "{error}");
        assert_eq!(fs::read_to_string(stream).unwrap(), "retained stream");
    }

    #[cfg(windows)]
    #[test]
    fn windows_custom_dacl_does_not_get_replaced_by_destination_defaults() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, "original").unwrap();
        fs::write(&destination, "").unwrap();
        let grant = format!("{}:(F)", std::env::var("USERNAME").unwrap());
        assert!(
            std::process::Command::new("icacls")
                .arg(&source)
                .args(["/inheritance:r", "/grant:r", &grant])
                .status()
                .unwrap()
                .success()
        );
        let original = CopyMetadata::read(&File::open(&source).unwrap()).unwrap();
        let target = File::options()
            .read(true)
            .write(true)
            .open(&destination)
            .unwrap();
        assert!(original.apply_to(&target).is_err());
        assert_eq!(
            CopyMetadata::read(&File::open(&source).unwrap()).unwrap(),
            original
        );
        assert_eq!(fs::read(destination).unwrap(), b"");
    }
}
