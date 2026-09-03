use super::{File, io, unsupported};
use std::mem::{size_of, zeroed};
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Security(pub(super) String);

impl Security {
    pub(super) fn read(file: &File) -> io::Result<Self> {
        let flags =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut descriptor = null_mut();
        // SAFETY: file is live; all requested components are returned in the
        // allocated descriptor, which is released on every subsequent path.
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle().cast(),
                SE_FILE_OBJECT,
                flags,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let mut text = null_mut();
        let mut length = 0;
        let success = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                flags,
                &mut text,
                &mut length,
            )
        };
        let error = io::Error::last_os_error();
        unsafe { LocalFree(descriptor.cast()) };
        if success == 0 {
            return Err(error);
        }
        let result = if length > 128 * 1024 {
            Err(unsupported(
                "Windows security descriptor exceeds inspection bound",
            ))
        } else {
            // SAFETY: conversion allocated length UTF-16 code units.
            let units = unsafe { std::slice::from_raw_parts(text, length as usize) };
            String::from_utf16(units.strip_suffix(&[0]).unwrap_or(units))
                .map(Self)
                .map_err(|_| unsupported("invalid Windows security descriptor encoding"))
        };
        unsafe { LocalFree(text.cast()) };
        result
    }

    pub(super) fn apply_to(&self, file: &File) -> io::Result<()> {
        // Never broaden a private recovery file's DACL to imitate a source.
        // A copy is supported only when its owner/group/DACL (including
        // inheritance control) already exactly match the source snapshot.
        if Self::read(file)? == *self {
            Ok(())
        } else {
            Err(unsupported(
                "destination owner, group or DACL differs from source",
            ))
        }
    }
}

pub(super) fn validate_extensions(file: &File) -> io::Result<()> {
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_EA_INFORMATION, FileEaInformation, NtQueryInformationFile,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_READONLY, FileStreamInfo, GetFileInformationByHandleEx,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let allowed = FILE_ATTRIBUTE_ARCHIVE
        | FILE_ATTRIBUTE_DIRECTORY
        | FILE_ATTRIBUTE_NORMAL
        | FILE_ATTRIBUTE_READONLY;
    if file.metadata()?.file_attributes() & !allowed != 0 {
        return Err(unsupported(
            "Windows attributes beyond ordinary/readonly file metadata",
        ));
    }
    // SAFETY: both structures are initialized writable output storage; the
    // synchronous retained file handle stays live for the native query.
    let mut status: IO_STATUS_BLOCK = unsafe { zeroed() };
    let mut ea: FILE_EA_INFORMATION = unsafe { zeroed() };
    let result = unsafe {
        NtQueryInformationFile(
            file.as_raw_handle().cast(),
            &mut status,
            (&mut ea as *mut FILE_EA_INFORMATION).cast(),
            size_of::<FILE_EA_INFORMATION>() as u32,
            FileEaInformation,
        )
    };
    if result < 0 {
        return Err(unsupported(&format!(
            "Windows EA inspection failed: NTSTATUS {result:#x}"
        )));
    }
    if ea.EaSize != 0 {
        return Err(unsupported("Windows extended attributes"));
    }
    // One unnamed data stream is the only supported stream layout. The query
    // fails closed if the complete stream list will not fit this fixed bound.
    let mut storage = vec![0_u64; 8192];
    let success = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle().cast(),
            FileStreamInfo,
            storage.as_mut_ptr().cast(),
            (storage.len() * size_of::<u64>()) as u32,
        )
    };
    if success == 0 {
        let error = io::Error::last_os_error();
        // Directories can legitimately have no streams (ERROR_HANDLE_EOF).
        if file.metadata()?.is_dir() && error.raw_os_error() == Some(38) {
            return Ok(());
        }
        return Err(error);
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), storage.len() * 8) };
    let next = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let name_length = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let expected = "::$DATA"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    // FILE_STREAM_INFO.StreamName starts at offset 24, independent of padding.
    if next != 0 || name_length != expected.len() || bytes[24..24 + expected.len()] != expected {
        return Err(unsupported("Windows named data streams"));
    }
    Ok(())
}
