//! Ephemeral jars (Unix and Windows) for explicit authenticated extractor jobs.
//! Files are never used for normal persistence; deletion is not forensic erasure.
use super::{AccountError, SessionCookies};
use std::path::{Path, PathBuf};
pub(super) struct CookieFile {
    directory: PathBuf,
    path: PathBuf,
}
impl CookieFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn close(self) -> Result<(), AccountError> {
        std::fs::remove_file(&self.path).map_err(|_| AccountError::SecureTemporaryStorage)?;
        std::fs::remove_dir(&self.directory).map_err(|_| AccountError::SecureTemporaryStorage)?;
        Ok(())
    }
}
#[cfg(unix)]
impl CookieFile {
    pub fn new(cookies: &SessionCookies) -> Result<Self, AccountError> {
        use std::{
            fs::{self, DirBuilder, OpenOptions},
            io::Write,
            os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        };
        let root = private_root()?;
        let mut random = [0u8; 16];
        // getentropy accepts <=256 bytes and fills the complete live stack buffer.
        if unsafe { libc::getentropy(random.as_mut_ptr().cast(), random.len()) } != 0 {
            return Err(AccountError::SecureTemporaryStorage);
        }
        let name = format!(
            "job-{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let directory = root.join(name);
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| AccountError::SecureTemporaryStorage)?;
        let jar = Self {
            path: directory.join("cookies.txt"),
            directory,
        };
        let bytes = cookies.export_for_vault();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&jar.path)
            .map_err(|_| AccountError::SecureTemporaryStorage)?;
        file.write_all(&bytes)
            .map_err(|_| AccountError::SecureTemporaryStorage)?;
        file.sync_all()
            .map_err(|_| AccountError::SecureTemporaryStorage)?;
        let metadata =
            fs::symlink_metadata(&jar.path).map_err(|_| AccountError::SecureTemporaryStorage)?;
        if !metadata.is_file() {
            return Err(AccountError::SecureTemporaryStorage);
        }
        Ok(jar)
    }
}
#[cfg(not(any(unix, windows)))]
impl CookieFile {
    pub fn new(_: &SessionCookies) -> Result<Self, AccountError> {
        Err(AccountError::SecureTemporaryStorage)
    }
}
impl Drop for CookieFile {
    fn drop(&mut self) {
        // Unlink only known files; never recursively traverse unknown contents.
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
#[cfg(unix)]
fn private_root() -> Result<PathBuf, AccountError> {
    use std::{
        fs::{self, DirBuilder},
        os::unix::fs::{DirBuilderExt, MetadataExt},
    };
    let uid = unsafe { libc::geteuid() };
    let root = std::env::temp_dir().join(format!("oxplay-auth-{uid}"));
    match DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(AccountError::SecureTemporaryStorage),
    }
    let metadata = fs::symlink_metadata(&root).map_err(|_| AccountError::SecureTemporaryStorage)?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(AccountError::SecureTemporaryStorage);
    }
    Ok(root)
}
/// Removes only known, app-owned directories older than an hour. Normal jobs are
/// bounded to45s. No file contents are read. Active jobs and unknown files survive.
#[cfg(unix)]
pub(super) fn cleanup_stale() -> Result<(), AccountError> {
    use std::{
        fs,
        os::unix::fs::MetadataExt,
        time::{Duration, SystemTime},
    };
    let root = private_root()?;
    let uid = unsafe { libc::geteuid() };
    for entry in fs::read_dir(&root)
        .map_err(|_| AccountError::SecureTemporaryStorage)?
        .take(1000)
    {
        let entry = entry.map_err(|_| AccountError::SecureTemporaryStorage)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.len() != 36
            || !name.starts_with("job-")
            || !name[4..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            continue;
        }
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| AccountError::SecureTemporaryStorage)?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            continue;
        }
        let old = metadata
            .modified()
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > Duration::from_secs(3600));
        if !old {
            continue;
        }
        let jar = entry.path().join("cookies.txt");
        if let Ok(meta) = fs::symlink_metadata(&jar) {
            if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
                continue;
            }
            fs::remove_file(&jar).map_err(|_| AccountError::SecureTemporaryStorage)?;
        }
        // Unknown extras cause removal to fail safely; no recursive deletion.
        let _ = fs::remove_dir(entry.path());
    }
    Ok(())
}
#[cfg(not(any(unix, windows)))]
pub(super) fn cleanup_stale() -> Result<(), AccountError> {
    Ok(())
}

/// Windows jars live under the per-user `%TEMP%` (user-private by default
/// ACL inheritance, unlike a shared Unix `/tmp`), in `oxplay-auth\job-*`.
/// Every directory and file must be a plain entry, never a reparse point.
#[cfg(windows)]
mod windows {
    use super::{AccountError, CookieFile, SessionCookies};
    use std::{
        fs,
        io::Write,
        os::windows::fs::{MetadataExt, OpenOptionsExt},
        path::{Path, PathBuf},
        time::{Duration, SystemTime},
    };
    /// FILE_FLAG_OPEN_REPARSE_POINT and FILE_ATTRIBUTE_REPARSE_POINT.
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const REPARSE_POINT: u32 = 0x400;
    const PREFIX: &str = "job-";
    const RANDOM: usize = 16;

    fn plain(metadata: &fs::Metadata, directory: bool) -> bool {
        metadata.file_attributes() & REPARSE_POINT == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            }
    }
    fn private_root() -> Result<PathBuf, AccountError> {
        let root = std::env::temp_dir().join("oxplay-auth");
        match fs::create_dir(&root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(AccountError::SecureTemporaryStorage),
        }
        let metadata =
            fs::symlink_metadata(&root).map_err(|_| AccountError::SecureTemporaryStorage)?;
        if !plain(&metadata, true) {
            return Err(AccountError::SecureTemporaryStorage);
        }
        Ok(root)
    }
    fn job_name(name: &str) -> bool {
        name.len() == PREFIX.len() + RANDOM
            && name.starts_with(PREFIX)
            && name[PREFIX.len()..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric())
    }

    pub(in crate::account) fn create(cookies: &SessionCookies) -> Result<CookieFile, AccountError> {
        let root = private_root()?;
        // create_dir semantics: an existing name is never adopted.
        let directory = tempfile::Builder::new()
            .prefix(PREFIX)
            .rand_bytes(RANDOM)
            .tempdir_in(&root)
            .map_err(|_| AccountError::SecureTemporaryStorage)?
            .keep();
        let jar = CookieFile {
            path: directory.join("cookies.txt"),
            directory,
        };
        let bytes = cookies.export_for_vault();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(&jar.path)
            .map_err(|_| AccountError::SecureTemporaryStorage)?;
        file.write_all(&bytes)
            .map_err(|_| AccountError::SecureTemporaryStorage)?;
        file.sync_all()
            .map_err(|_| AccountError::SecureTemporaryStorage)?;
        let metadata =
            fs::symlink_metadata(&jar.path).map_err(|_| AccountError::SecureTemporaryStorage)?;
        if !plain(&metadata, false) {
            return Err(AccountError::SecureTemporaryStorage);
        }
        Ok(jar)
    }

    /// Removes only recognized app-owned job directories older than an hour.
    pub(in crate::account) fn cleanup_stale() -> Result<(), AccountError> {
        let root = private_root()?;
        for entry in fs::read_dir(&root)
            .map_err(|_| AccountError::SecureTemporaryStorage)?
            .take(1000)
        {
            let entry = entry.map_err(|_| AccountError::SecureTemporaryStorage)?;
            let name = entry.file_name();
            if !name.to_str().is_some_and(job_name) {
                continue;
            }
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| AccountError::SecureTemporaryStorage)?;
            let old = metadata
                .modified()
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > Duration::from_secs(3600));
            if !plain(&metadata, true) || !old {
                continue;
            }
            remove_job(&path)?;
        }
        Ok(())
    }
    fn remove_job(directory: &Path) -> Result<(), AccountError> {
        let jar = directory.join("cookies.txt");
        if let Ok(meta) = fs::symlink_metadata(&jar) {
            if !plain(&meta, false) {
                return Ok(());
            }
            fs::remove_file(&jar).map_err(|_| AccountError::SecureTemporaryStorage)?;
        }
        // Unknown extras cause removal to fail safely; no recursive deletion.
        let _ = fs::remove_dir(directory);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn ephemeral_credentials_are_plain_files_and_removed_on_drop() {
            let cookies = SessionCookies::import_netscape(
                zeroize::Zeroizing::new(
                    b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic-only\n"
                        .to_vec(),
                ),
                0,
            )
            .unwrap();
            let file = CookieFile::new(&cookies).unwrap();
            let path = file.path().to_owned();
            let directory = file.directory.clone();
            assert!(job_name(directory.file_name().unwrap().to_str().unwrap()));
            assert!(plain(&fs::symlink_metadata(&path).unwrap(), false));
            assert!(
                fs::read(&path)
                    .unwrap()
                    .windows(b"synthetic-only\n".len())
                    .any(|w| w == b"synthetic-only\n")
            );
            cleanup_stale().unwrap();
            assert!(path.exists(), "a fresh job is never reaped");
            drop(file);
            assert!(!path.exists());
            assert!(!directory.exists());
        }
    }
}
#[cfg(windows)]
impl CookieFile {
    pub fn new(cookies: &SessionCookies) -> Result<Self, AccountError> {
        windows::create(cookies)
    }
}
#[cfg(windows)]
pub(super) use windows::cleanup_stale;

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    #[test]
    fn ephemeral_credentials_are_private_and_removed_on_drop() {
        let cookies=SessionCookies::import_netscape(zeroize::Zeroizing::new(b"# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic-only\n".to_vec()),0).unwrap();
        let file = CookieFile::new(&cookies).unwrap();
        let path = file.path().to_owned();
        let directory = file.directory.clone();
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
        assert!(
            std::fs::read(&path)
                .unwrap()
                .windows(b"synthetic-only\n".len())
                .any(|w| w == b"synthetic-only\n")
        );
        drop(file);
        assert!(!path.exists());
        assert!(!directory.exists());
    }
}
