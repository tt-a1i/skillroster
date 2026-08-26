//! Private-by-default storage for SkillRoster control state.
//!
//! The state root protects every descendant, including recovery objects whose
//! original metadata must survive Undo. Control files are additionally narrowed
//! to owner-only access so copied or inspected artifacts keep the same boundary.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(unix)]
const STATE_DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
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

#[cfg(unix)]
pub(crate) fn private_file_options() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .mode(STATE_FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    options
}

#[cfg(windows)]
pub(crate) fn private_file_options() -> OpenOptions {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    let mut options = OpenOptions::new();
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    options
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn private_file_options() -> OpenOptions {
    OpenOptions::new()
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

pub(crate) fn prepare_private_file(path: &Path) -> io::Result<()> {
    let mut options = private_file_options();
    match options.read(true).write(true).create_new(true).open(path) {
        Ok(file) => secure_opened_file(&file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => secure_file(path),
        Err(error) => Err(error),
    }
}

pub(crate) fn open_private_file_for_replace(path: &Path) -> io::Result<File> {
    let mut options = private_file_options();
    let file = options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    secure_opened_file(&file)?;
    file.set_len(0)?;
    Ok(file)
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

pub(crate) fn secure_control_directory(
    path: &Path,
    mut validate: impl FnMut(&OsStr, &mut File) -> io::Result<bool>,
) -> io::Result<()> {
    use cap_primitives::fs::FollowSymlinks;
    use cap_std::fs::{Dir, OpenOptions as CapOpenOptions};

    let directory = match open_directory(path) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    validate_opened_path(&directory, true)?;
    let dir = Dir::from_std_file(directory.try_clone()?);
    let mut entries = dir.read_dir(".")?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut owned_files = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.file_name();
        let mut options = CapOpenOptions::new();
        options.read(true)._cap_fs_ext_follow(FollowSymlinks::No);
        let mut file = entry.open_with(&options)?.into_std();
        validate_opened_path(&file, false)?;
        if !validate(&name, &mut file)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("unrecognized control file: {}", name.to_string_lossy()),
            ));
        }
        owned_files.push((name, opened_path_identity(&file)?));
    }

    secure_opened_path(&directory, true)?;
    for (name, expected_identity) in owned_files {
        let mut options = CapOpenOptions::new();
        options.read(true)._cap_fs_ext_follow(FollowSymlinks::No);
        let mut file = dir.open_with(&name, &options)?.into_std();
        validate_opened_path(&file, false)?;
        if opened_path_identity(&file)? != expected_identity {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "control file identity changed during validation: {}",
                    name.to_string_lossy()
                ),
            ));
        }
        if !validate(&name, &mut file)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "control file changed during validation: {}",
                    name.to_string_lossy()
                ),
            ));
        }
        secure_opened_file(&file)?;
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenedPathIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn opened_path_identity(file: &File) -> io::Result<OpenedPathIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(OpenedPathIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenedPathIdentity {
    volume: u64,
    file_id: [u8; 16],
}

#[cfg(windows)]
fn opened_path_identity(file: &File) -> io::Result<OpenedPathIdentity> {
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };

    // SAFETY: info is valid writable storage for the requested information class.
    let mut info: FILE_ID_INFO = unsafe { zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle().cast(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(OpenedPathIdentity {
        volume: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenedPathIdentity;

#[cfg(not(any(unix, windows)))]
fn opened_path_identity(_file: &File) -> io::Result<OpenedPathIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "state file identity is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn open_directory(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
}

#[cfg(windows)]
fn open_directory(path: &Path) -> io::Result<File> {
    open_windows_path(path)
}

#[cfg(not(any(unix, windows)))]
fn open_directory(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private state permissions are unsupported on this platform",
    ))
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
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    if directory {
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY);
    }
    let file = options.open(path)?;
    secure_opened_path(&file, directory)
}

#[cfg(unix)]
pub(crate) fn secure_opened_file(file: &File) -> io::Result<()> {
    secure_opened_path(file, false)
}

#[cfg(unix)]
fn secure_opened_path(file: &File, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    validate_opened_path(file, directory)?;
    let mode = if directory {
        STATE_DIRECTORY_MODE
    } else {
        STATE_FILE_MODE
    };
    file.set_permissions(fs::Permissions::from_mode(mode))
}

#[cfg(unix)]
fn validate_opened_path(file: &File, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    let valid_kind = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !valid_kind {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "opened state path has an unsafe file type",
        ));
    }
    // SAFETY: geteuid has no preconditions and does not retain borrowed data.
    let current_uid = unsafe { libc::geteuid() };
    if metadata.uid() != current_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "opened state path is not owned by the current user",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn secure_path(path: &Path, directory: bool) -> io::Result<()> {
    let file = open_windows_path(path)?;
    secure_opened_path(&file, directory)
}

#[cfg(windows)]
fn open_windows_path(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
        WRITE_DAC,
    };

    let mut options = OpenOptions::new();
    options
        .access_mode(READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    options.open(path)
}

#[cfg(windows)]
pub(crate) fn secure_opened_file(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, READ_CONTROL, ReOpenFile, WRITE_DAC,
    };

    // SAFETY: the original handle remains live; a successful ReOpenFile result is newly owned.
    let handle = unsafe {
        ReOpenFile(
            file.as_raw_handle().cast(),
            READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = WindowsHandle(handle as HANDLE);
    secure_windows_handle(handle.0, false)
}

#[cfg(windows)]
fn validate_opened_path(file: &File, directory: bool) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    validate_windows_handle(file.as_raw_handle().cast(), directory).map(|_| ())
}

#[cfg(windows)]
fn secure_opened_path(file: &File, directory: bool) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    secure_windows_handle(file.as_raw_handle().cast(), directory)
}

#[cfg(windows)]
struct WindowsHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns a handle returned by OpenProcessToken or ReOpenFile.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
fn secure_windows_handle(
    handle: windows_sys::Win32::Foundation::HANDLE,
    directory: bool,
) -> io::Result<()> {
    use std::mem::size_of;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::GENERIC_ALL;
    use windows_sys::Win32::Security::Authorization::{SE_FILE_OBJECT, SetSecurityInfo};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE,
        DACL_SECURITY_INFORMATION, InitializeAcl, OBJECT_INHERIT_ACE,
        PROTECTED_DACL_SECURITY_INFORMATION,
    };

    let current_sid = validate_windows_handle(handle, directory)?;

    let acl_length =
        size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + current_sid.len();
    let acl_length =
        u32::try_from(acl_length).map_err(|_| io::Error::other("state ACL is too large"))?;
    let word_count = (acl_length as usize).div_ceil(size_of::<u32>());
    let mut acl_buffer = vec![0_u32; word_count];
    let acl = acl_buffer.as_mut_ptr().cast::<ACL>();
    // SAFETY: acl_buffer is aligned and writable for at least acl_length bytes.
    if unsafe { InitializeAcl(acl, acl_length, ACL_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let inheritance = if directory {
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
    } else {
        0
    };
    // SAFETY: the ACL is initialized and current_sid remains live during the call.
    if unsafe {
        AddAccessAllowedAceEx(
            acl,
            ACL_REVISION,
            inheritance,
            GENERIC_ALL,
            current_sid.as_ptr().cast_mut().cast(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: handle names the validated object; the ACL remains live during the call.
    let status = unsafe {
        SetSecurityInfo(
            handle,
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

#[cfg(windows)]
fn validate_windows_handle(
    handle: windows_sys::Win32::Foundation::HANDLE,
    directory: bool,
) -> io::Result<Vec<u8>> {
    use std::mem::{size_of, zeroed};
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        CheckTokenMembership, GetLengthSid, OWNER_SECURITY_INFORMATION, PSID,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FileAttributeTagInfo, GetFileInformationByHandleEx,
    };

    // SAFETY: info is valid writable storage for the requested information class.
    let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "state path is a reparse point",
        ));
    }
    if (info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "opened state path has an unsafe file type",
        ));
    }

    let mut owner: PSID = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: output pointers are valid; the returned descriptor is released below.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalSecurityDescriptor(descriptor);
    let current_sid = current_user_sid()?;
    // SAFETY: owner belongs to the live security descriptor returned by GetSecurityInfo.
    let owner_length = unsafe { GetLengthSid(owner) } as usize;
    let owner_matches = owner_length == current_sid.len()
        // SAFETY: both SIDs were returned by Windows and are live for this comparison.
        && unsafe { std::slice::from_raw_parts(owner.cast::<u8>(), owner_length) }
            == current_sid.as_slice();
    // Windows may assign a newly created object to an enabled owner group such
    // as Administrators rather than directly to the token's user SID. Treat
    // that as locally controlled, while rejecting owners outside the effective
    // token. The replacement DACL below still grants access only to the user.
    let owner_is_controlled = if owner_matches {
        true
    } else {
        let mut is_member = 0;
        // SAFETY: owner belongs to the live descriptor and a null token asks
        // Windows to check the effective token for the current thread/process.
        if unsafe { CheckTokenMembership(null_mut(), owner, &mut is_member) } == 0 {
            return Err(io::Error::last_os_error());
        }
        is_member != 0
    };
    drop(descriptor);
    if !owner_is_controlled {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "state path is not controlled by the current user",
        ))
    } else {
        Ok(current_sid)
    }
}

#[cfg(windows)]
struct LocalSecurityDescriptor(windows_sys::Win32::Security::PSECURITY_DESCRIPTOR);

#[cfg(windows)]
impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated this descriptor with LocalAlloc.
        unsafe { windows_sys::Win32::Foundation::LocalFree(self.0.cast()) };
    }
}

#[cfg(windows)]
fn current_user_sid() -> io::Result<Vec<u8>> {
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, GetLastError, HANDLE};
    use windows_sys::Win32::Security::{
        GetLengthSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = null_mut();
    // SAFETY: token points to writable storage and GetCurrentProcess returns a pseudo-handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = WindowsHandle(token);
    let mut token_bytes = 0;
    // SAFETY: the null-buffer call obtains the required size.
    let first = unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut token_bytes) };
    if first != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::last_os_error());
    }
    let mut token_buffer = vec![0_u8; token_bytes as usize];
    // SAFETY: token_buffer is writable for token_bytes.
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
    // SAFETY: the SID belongs to the validated TOKEN_USER buffer.
    let sid_length = unsafe { GetLengthSid(user.User.Sid) } as usize;
    // SAFETY: the SID is live and valid for sid_length bytes.
    Ok(unsafe { std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), sid_length) }.to_vec())
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

#[cfg(not(any(unix, windows)))]
pub(crate) fn secure_opened_file(_file: &File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private state permissions are unsupported on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn validate_opened_path(_file: &File, _directory: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private state permissions are unsupported on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn secure_opened_path(_file: &File, _directory: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private state permissions are unsupported on this platform",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
        for (directory, file) in [
            ("receipts", "receipt.json"),
            ("source-confirmation", "detail.json"),
        ] {
            let path = state.join(directory).join(file);
            fs::write(&path, "fixture").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }

        secure_state_layout(&state).unwrap();
        secure_control_directory(&state.join("receipts"), |_, _| Ok(true)).unwrap();
        secure_control_directory(&state.join("source-confirmation"), |_, _| Ok(true)).unwrap();

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
        for (directory, file) in [
            ("receipts", "receipt.json"),
            ("source-confirmation", "detail.json"),
        ] {
            assert_eq!(
                fs::metadata(state.join(directory).join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600,
                "unexpected permissions for {directory}/{file}"
            );
        }
    }

    #[test]
    fn refuses_control_file_symlinks_without_touching_the_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let temp = TempDir::new().unwrap();
        let state = temp.path().join("state");
        let receipts = state.join("receipts");
        fs::create_dir_all(&receipts).unwrap();
        let outside = temp.path().join("outside.json");
        fs::write(&outside, "outside").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&outside, receipts.join("receipt.json")).unwrap();

        secure_control_directory(&receipts, |_, _| Ok(true)).unwrap_err();

        assert_eq!(
            fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(fs::read_to_string(outside).unwrap(), "outside");
    }

    #[test]
    fn private_file_preparation_refuses_a_symlink_without_creating_its_target() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target = temp.path().join("outside.db");
        let link = temp.path().join("skillroster.db");
        symlink(&target, &link).unwrap();

        assert!(prepare_private_file(&link).is_err());
        assert!(!target.exists());
    }
}
