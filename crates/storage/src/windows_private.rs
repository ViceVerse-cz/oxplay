// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows equivalents of the Unix `0700` directory / `O_NOFOLLOW` checks.
//!
//! New private directories get a protected DACL granting full control only to
//! the current user's SID (no inherited ACEs, so neither other users nor
//! broader profile ACLs apply); files created inside inherit that single ACE.
//! Existing directories must be real directories, never reparse points
//! (symlinks/junctions). Their ACLs are not re-audited: the application data
//! root under `%LOCALAPPDATA%` is user-private by default.
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::MetadataExt, fs::OpenOptionsExt},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    Storage::FileSystem::{
        CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// The current process user's SID in string form, e.g. `S-1-5-21-...`.
fn current_user_sid() -> io::Result<String> {
    // SAFETY: token/buffer/string handles are owned locally and released on
    // every path; the TOKEN_USER view never outlives its aligned buffer.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        struct Token(HANDLE);
        impl Drop for Token {
            fn drop(&mut self) {
                unsafe { CloseHandle(self.0) };
            }
        }
        let token = Token(token);
        let mut length = 0u32;
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut length);
        if length == 0 || length > 4096 {
            return Err(io::Error::last_os_error());
        }
        // u64 storage satisfies TOKEN_USER's pointer alignment.
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        if GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut end = 0;
        while *text.add(end) != 0 {
            end += 1;
        }
        let sid = String::from_utf16(std::slice::from_raw_parts(text, end));
        LocalFree(text.cast());
        sid.map_err(|_| io::Error::other("invalid SID text"))
    }
}

/// Create one directory whose protected DACL admits only the current user.
/// Fails with `AlreadyExists` instead of adopting an existing path.
pub fn create_private_directory(path: &Path) -> io::Result<()> {
    let sddl = format!("D:P(A;OICI;FA;;;{})", current_user_sid()?);
    let sddl = wide(std::ffi::OsStr::new(&sddl));
    let path = wide(path.as_os_str());
    // SAFETY: the descriptor is allocated by the call and freed below; wide
    // strings are NUL-terminated and outlive both calls.
    unsafe {
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let created = CreateDirectoryW(path.as_ptr(), &attributes);
        let error = io::Error::last_os_error();
        LocalFree(descriptor);
        if created == 0 {
            return Err(error);
        }
    }
    Ok(())
}

/// True only for an existing directory that is not a symlink/junction.
pub fn is_plain_directory(metadata: &std::fs::Metadata) -> bool {
    metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

/// Ensure `path` is a private plain directory, creating it (never ancestors).
pub fn ensure_private_directory(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if is_plain_directory(&metadata) => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a plain directory",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_private_directory(path) {
                Ok(()) => Ok(()),
                // A concurrent creator is re-validated like any existing path.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = std::fs::symlink_metadata(path)?;
                    if is_plain_directory(&metadata) {
                        Ok(())
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "not a plain directory",
                        ))
                    }
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// Open a regular file without following a final symlink/junction. `create`
/// also allows creation (inheriting the private directory's DACL).
pub fn open_no_follow(path: &Path, write: bool, create: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(write || create)
        .create(create)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_directory_is_created_once_and_validated() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private");
        ensure_private_directory(&private).unwrap();
        assert!(is_plain_directory(
            &std::fs::symlink_metadata(&private).unwrap()
        ));
        ensure_private_directory(&private).unwrap();
        assert_eq!(
            create_private_directory(&private).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let file = private.join("file");
        std::fs::write(&file, b"synthetic").unwrap();
        assert!(ensure_private_directory(&file).is_err());
        assert!(open_no_follow(&file, false, false).is_ok());
        assert!(open_no_follow(&private, false, false).is_err());
        assert!(current_user_sid().unwrap().starts_with("S-1-"));
    }
}
