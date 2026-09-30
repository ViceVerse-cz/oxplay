// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows private caption cache with the Unix module's ownership model: one
//! registry lock serializes instance creation/retirement, each live instance
//! holds its own lock file, and startup reaps only unlocked instances whose
//! complete bounded contents are recognized caption files.
//!
//! Directories are created with an owner-only protected DACL. Windows offers
//! no std directory handles, so operations are path based: every directory
//! and file is re-checked to be a plain directory/regular file (never a
//! reparse point such as a symlink or junction) before it is used or removed.
use super::CreatedFile;
use oxplay_storage::windows_private::{
    create_private_directory, ensure_private_directory, is_plain_directory,
};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::Write,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};
const MAX_INSTANCES: usize = 128;
const MAX_FILES: usize = 8;
const LOCK: &str = ".instance-lock";
const REGISTRY: &str = ".registry-lock";
/// FILE_FLAG_OPEN_REPARSE_POINT and FILE_ATTRIBUTE_REPARSE_POINT.
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE_POINT: u32 = 0x400;

pub(super) struct Directory {
    path: PathBuf,
    root: PathBuf,
    lock: Mutex<Option<File>>,
}

fn open_regular(path: &Path, create_new: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(OPEN_REPARSE_POINT);
    if create_new {
        options.write(true).create_new(true);
    } else {
        options.write(true).create(true);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(file)
}
fn plain_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| is_plain_directory(&metadata))
}
fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.file_attributes() & REPARSE_POINT == 0)
}
fn lock(directory: &Path, name: &str, nonblocking: bool) -> Result<Option<File>, &'static str> {
    if !plain_directory(directory) {
        return Err("Unsafe private caption directory.");
    }
    let file = open_regular(&directory.join(name), false)
        .map_err(|_| "Private caption lock unavailable.")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) if nonblocking => return Ok(None),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(_)) => return Err("Private caption lock failed."),
        }
        if std::time::Instant::now() >= deadline {
            return Err("Private caption cache is busy; try again later.");
        }
        // Only startup/cleanup workers wait here, with a fixed deadline.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
fn instance_name(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.len() == 32
            && name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
fn caption_name(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.len() == 36 && name.ends_with(".vtt") && instance_name(OsStr::new(&name[..32]))
    })
}
fn names(directory: &Path, limit: usize) -> Result<Vec<OsString>, &'static str> {
    if !plain_directory(directory) {
        return Err("Unsafe private caption directory.");
    }
    let mut result = Vec::new();
    for entry in
        std::fs::read_dir(directory).map_err(|_| "Private caption directory cannot be listed.")?
    {
        let entry = entry.map_err(|_| "Private caption cache entry unavailable.")?;
        if result.len() >= limit {
            return Err("Private caption cache exceeded its recovery limit.");
        }
        result.push(entry.file_name());
    }
    Ok(result)
}
fn reap(root: &Path) -> Result<usize, &'static str> {
    let mut live = 0;
    for name in names(root, MAX_INSTANCES + 1)? {
        if name == REGISTRY {
            continue;
        }
        let directory = root.join(&name);
        if !instance_name(&name) || !plain_directory(&directory) {
            return Err("Unknown entry in private caption cache; recovery stopped.");
        }
        // The registry lock serializes creation and retirement, so an unlocked
        // child belongs to a dead instance. Never remove a live instance.
        let Some(owner) = lock(&directory, LOCK, true)? else {
            live += 1;
            continue;
        };
        let entries = names(&directory, MAX_FILES + 1)?;
        // Validate the complete bounded set before any deletion.
        for entry in &entries {
            if entry == LOCK {
                continue;
            }
            if !caption_name(entry) || !regular_file(&directory.join(entry)) {
                return Err("Unknown caption cache file; recovery stopped.");
            }
        }
        for entry in entries.iter().filter(|entry| *entry != LOCK) {
            std::fs::remove_file(directory.join(entry))
                .map_err(|_| "Private caption cleanup was incomplete.")?;
        }
        drop(owner);
        std::fs::remove_file(directory.join(LOCK))
            .map_err(|_| "Private caption cleanup was incomplete.")?;
        std::fs::remove_dir(&directory).map_err(|_| "Private caption cleanup was incomplete.")?;
    }
    Ok(live)
}
pub(super) fn random_name() -> Result<String, &'static str> {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Caption file entropy unavailable.")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
impl Directory {
    pub(super) fn create(path: PathBuf) -> Result<Self, &'static str> {
        if !path.is_absolute() {
            return Err("Caption cache requires an absolute private path.");
        }
        let parent = path
            .parent()
            .ok_or("Caption cache has no private parent.")?;
        // Only the caller's app-owned profile/cache descendants are created.
        if let Some(grandparent) = parent.parent() {
            std::fs::create_dir_all(grandparent)
                .map_err(|_| "Private caption profile unavailable.")?;
        }
        ensure_private_directory(parent).map_err(|_| "Unsafe caption profile directory.")?;
        ensure_private_directory(&path).map_err(|_| "Unsafe private caption cache.")?;
        let _registry =
            lock(&path, REGISTRY, false)?.ok_or("Caption registry lock unavailable.")?;
        if reap(&path)? >= MAX_INSTANCES {
            return Err("Too many live caption cache instances.");
        }
        let directory = path.join(random_name()?);
        create_private_directory(&directory)
            .map_err(|_| "Private caption instance unavailable.")?;
        let owner = lock(&directory, LOCK, true)?.ok_or("Caption instance unexpectedly locked.")?;
        Ok(Self {
            path: directory,
            root: path,
            lock: Mutex::new(Some(owner)),
        })
    }
    pub(super) fn create_file(&self, bytes: &[u8]) -> Result<CreatedFile, &'static str> {
        if !plain_directory(&self.path) {
            return Err("Unsafe private caption directory.");
        }
        let name = format!("{}.vtt", random_name()?);
        let path = self.path.join(&name);
        let mut file =
            open_regular(&path, true).map_err(|_| "Private caption file unavailable.")?;
        let error = file
            .write_all(bytes)
            .err()
            .map(|_| "Caption file write failed.");
        // Once created, every path returns to the owning worker even on write
        // failure. Its tracked released record bounds and reports deletion.
        Ok(CreatedFile { path, error })
    }
    pub(super) fn remove_file(&self, path: &Path) -> Result<(), &'static str> {
        if path.parent() != Some(self.path.as_path()) {
            return Err("Caption cleanup path mismatch.");
        }
        let name = path.file_name().ok_or("Caption cleanup name missing.")?;
        if !caption_name(name) {
            return Err("Caption cleanup name mismatch.");
        }
        if !plain_directory(&self.path) {
            return Err("Unsafe private caption directory.");
        }
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("Private caption deletion failed."),
            Ok(_) if !regular_file(path) => return Err("Private caption deletion failed."),
            Ok(_) => {}
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Private caption deletion failed."),
        }
    }
    pub(super) fn purge_stale(&self) -> Result<(), &'static str> {
        let _registry =
            lock(&self.root, REGISTRY, false)?.ok_or("Caption registry lock unavailable.")?;
        let live = reap(&self.root)?;
        if live > 1 {
            Err(
                "Another running Oxplay instance still owns caption files. Close it and retry clearing local data.",
            )
        } else {
            Ok(())
        }
    }
    pub(super) fn finish(&self) -> Result<(), &'static str> {
        let _registry =
            lock(&self.root, REGISTRY, false)?.ok_or("Caption registry lock unavailable.")?;
        if !plain_directory(&self.path) {
            return Err("Unsafe private caption directory.");
        }
        // Release this instance's lock before deleting its file and directory.
        drop(
            self.lock
                .lock()
                .map_err(|_| "Private caption lock failed.")?
                .take(),
        );
        std::fs::remove_file(self.path.join(LOCK))
            .map_err(|_| "Private caption cleanup was incomplete.")?;
        std::fs::remove_dir(&self.path).map_err(|_| "Private caption cleanup was incomplete.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "oxplay-caption-cache-test-{}",
                random_name().unwrap()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn cache(&self) -> PathBuf {
            self.0.join("captions")
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn live_instances_survive_another_startup_and_own_cleanup() {
        let profile = Profile::new();
        let first = Directory::create(profile.cache()).unwrap();
        let path = first.create_file(b"WEBVTT\n\n").unwrap().path;
        let second = Directory::create(profile.cache()).unwrap();
        assert!(
            path.exists(),
            "another startup must not reap the live owner"
        );
        assert_ne!(first.path, second.path);
        second.finish().unwrap();
        assert!(
            path.exists(),
            "another owner shutdown must not remove this file"
        );
        first.remove_file(&path).unwrap();
        first.remove_file(&path).unwrap();
        first.finish().unwrap();
        assert!(!path.exists());
        assert!(!first.path.exists());
    }
    #[test]
    fn unlocked_crash_files_are_reaped_only_inside_private_cache() {
        let profile = Profile::new();
        let crashed = Directory::create(profile.cache()).unwrap();
        let stale = crashed.create_file(b"public caption").unwrap().path;
        let child = crashed.path.clone();
        drop(crashed); // Simulate the OS closing lock handles without cleanup.
        let outside = profile.0.join("unrelated.vtt");
        std::fs::write(&outside, b"untouched").unwrap();
        let replacement = Directory::create(profile.cache()).unwrap();
        assert!(!stale.exists());
        assert!(!child.exists());
        assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
        replacement.purge_stale().unwrap();
        replacement.finish().unwrap();
    }
    #[test]
    fn unrecognized_or_excess_entries_stop_recovery_before_deletion() {
        let profile = Profile::new();
        let crashed = Directory::create(profile.cache()).unwrap();
        let mut files = Vec::new();
        for _ in 0..9 {
            files.push(crashed.create_file(b"bounded").unwrap().path);
        }
        drop(crashed);
        assert!(Directory::create(profile.cache()).is_err());
        assert!(
            files.iter().all(|path| path.exists()),
            "bounds are checked before deletion"
        );
        let second = Profile::new();
        std::fs::create_dir(second.cache()).unwrap();
        std::fs::write(second.cache().join("unknown-user-file"), b"preserve").unwrap();
        assert!(Directory::create(second.cache()).is_err());
        assert_eq!(
            std::fs::read(second.cache().join("unknown-user-file")).unwrap(),
            b"preserve"
        );
    }
    #[test]
    fn cleanup_rejects_foreign_paths() {
        let profile = Profile::new();
        let directory = Directory::create(profile.cache()).unwrap();
        let outside = profile.0.join(format!("{}.vtt", random_name().unwrap()));
        std::fs::write(&outside, b"must survive").unwrap();
        assert!(directory.remove_file(&outside).is_err());
        assert!(
            directory
                .remove_file(&directory.path.join("unknown.vtt"))
                .is_err()
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"must survive");
        directory.finish().unwrap();
    }
}
