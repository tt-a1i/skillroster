//! Private-by-default storage for SkillRoster control state.
//!
//! The state root protects every descendant, including recovery objects whose
//! original metadata must survive Undo. Control files are additionally narrowed
//! to owner-only access so copied or inspected artifacts keep the same boundary.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

const STATE_DIRECTORY_MODE: u32 = 0o700;
const STATE_FILE_MODE: u32 = 0o600;

pub(crate) fn prepare_state_root(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("state root is not a regular directory: {}", path.display()),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_private_dir_all(path)?,
        Err(error) => return Err(error),
    }
    secure_directory(path)
}

pub(crate) fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(STATE_FILE_MODE);
    }
    options
}

pub(crate) fn secure_file(path: &Path) -> io::Result<()> {
    secure_path(path, false)
}

pub(crate) fn secure_directory(path: &Path) -> io::Result<()> {
    secure_path(path, true)
}

pub(crate) fn secure_optional_file(path: &Path) -> io::Result<()> {
    match secure_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub(crate) fn secure_state_layout(state_dir: &Path) -> io::Result<()> {
    prepare_state_root(state_dir)?;
    for name in [
        "receipts",
        "recovery",
        "source-confirmation",
        "plan-backups",
        "library",
    ] {
        let path = state_dir.join(name);
        match secure_directory(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    for name in [
        "skillroster.db",
        "skillroster.db-wal",
        "skillroster.db-shm",
        "write.lock",
    ] {
        secure_optional_file(&state_dir.join(name))?;
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_dir_all(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(STATE_DIRECTORY_MODE);
    builder.create(path)
}

#[cfg(not(unix))]
fn create_private_dir_all(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn secure_path(path: &Path, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    if directory {
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    let valid_kind = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !valid_kind {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("state path has an unsafe file type: {}", path.display()),
        ));
    }
    // SAFETY: geteuid has no preconditions and does not retain borrowed data.
    let current_uid = unsafe { libc::geteuid() };
    if metadata.uid() != current_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "state path is not owned by the current user: {}",
                path.display()
            ),
        ));
    }
    let mode = if directory {
        STATE_DIRECTORY_MODE
    } else {
        STATE_FILE_MODE
    };
    file.set_permissions(fs::Permissions::from_mode(mode))
}

#[cfg(windows)]
fn secure_path(path: &Path, directory: bool) -> io::Result<()> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INSUFFICIENT_BUFFER, GENERIC_ALL, GetLastError, HANDLE,
    };
    use windows_sys::Win32::Security::Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE,
        DACL_SECURITY_INFORMATION, GetLengthSid, GetTokenInformation, InitializeAcl,
        OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle was returned by OpenProcessToken and is owned here.
            unsafe { CloseHandle(self.0) };
        }
    }

    let metadata = fs::symlink_metadata(path)?;
    let valid_kind = !metadata.file_type().is_symlink()
        && if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        };
    if !valid_kind {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("state path has an unsafe file type: {}", path.display()),
        ));
    }

    let mut token = 0;
    // SAFETY: token points to writable storage and GetCurrentProcess returns a pseudo-handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut token_bytes = 0;
    // SAFETY: the null-buffer call obtains the required size.
    let first = unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut token_bytes) };
    if first != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::last_os_error());
    }
    let mut token_buffer = vec![0_u8; token_bytes as usize];
    // SAFETY: token_buffer is writable for token_bytes and remains alive while its SID is used.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            token_buffer.as_mut_ptr().cast::<c_void>(),
            token_bytes,
            &mut token_bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful TokenUser query starts with a TOKEN_USER value.
    let user = unsafe { &*token_buffer.as_ptr().cast::<TOKEN_USER>() };
    let sid: PSID = user.User.Sid;
    // SAFETY: sid belongs to the validated TOKEN_USER buffer.
    let sid_length = unsafe { GetLengthSid(sid) } as usize;
    let acl_length =
        size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid_length;
    let acl_length =
        u32::try_from(acl_length).map_err(|_| io::Error::other("state ACL is too large"))?;
    let mut acl_buffer = vec![0_u8; acl_length as usize];
    let acl = acl_buffer.as_mut_ptr().cast::<ACL>();
    // SAFETY: acl_buffer is writable for acl_length bytes.
    if unsafe { InitializeAcl(acl, acl_length, ACL_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let inheritance = if directory {
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
    } else {
        0
    };
    // SAFETY: acl was initialized and sid remains valid for the duration of this call.
    if unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, inheritance, GENERIC_ALL, sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut wide_path = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide_path.push(0);
    // SAFETY: wide_path is NUL-terminated; acl remains valid for the call.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

#[cfg(not(any(unix, windows)))]
fn secure_path(path: &Path, _directory: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "private state permissions are unsupported on this platform: {}",
            path.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    #[test]
    fn narrows_existing_state_directory_and_file() {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let temp = TempDir::new().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        let file = state.join("skillroster.db");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .open(&file)
            .unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

        prepare_state_root(&state).unwrap();
        secure_file(&file).unwrap();

        assert_eq!(
            fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_state_root() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let temp = TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let state = temp.path().join("state");
        symlink(&outside, &state).unwrap();

        let error = prepare_state_root(&state).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn narrows_existing_control_layout_and_sidecars() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        for name in [
            "receipts",
            "recovery",
            "source-confirmation",
            "plan-backups",
            "library",
        ] {
            let path = state.join(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for name in [
            "skillroster.db",
            "skillroster.db-wal",
            "skillroster.db-shm",
            "write.lock",
        ] {
            let path = state.join(name);
            fs::write(&path, "fixture").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }

        secure_state_layout(&state).unwrap();

        for name in [
            "receipts",
            "recovery",
            "source-confirmation",
            "plan-backups",
            "library",
        ] {
            assert_eq!(
                fs::metadata(state.join(name)).unwrap().permissions().mode() & 0o777,
                0o700,
                "unexpected directory permissions for {name}"
            );
        }
        for name in [
            "skillroster.db",
            "skillroster.db-wal",
            "skillroster.db-shm",
            "write.lock",
        ] {
            assert_eq!(
                fs::metadata(state.join(name)).unwrap().permissions().mode() & 0o777,
                0o600,
                "unexpected file permissions for {name}"
            );
        }
    }
}
