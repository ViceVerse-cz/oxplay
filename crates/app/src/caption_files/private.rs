// SPDX-License-Identifier: GPL-3.0-or-later
//! Unix private cache, with cross-process ownership and confined deletion.
use super::CreatedFile;
use std::{
    ffi::{CStr, CString, OsStr, OsString},
    fs::File,
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
};
const MAX_INSTANCES: usize = 128;
const MAX_FILES: usize = 8;
const LOCK: &str = ".instance-lock";
const REGISTRY: &str = ".registry-lock";
pub(super) struct Directory {
    path: PathBuf,
    name: OsString,
    root: File,
    directory: File,
    _lock: File,
}
fn cstring(name: &OsStr) -> Result<CString, &'static str> {
    CString::new(name.as_bytes()).map_err(|_| "Invalid private caption cache name.")
}
fn open_at(parent: &File, name: &OsStr, flags: i32) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn private_file(file: &File, directory: bool) -> Result<(), &'static str> {
    let meta = file
        .metadata()
        .map_err(|_| "Caption cache metadata unavailable.")?;
    if meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || if directory {
            !meta.is_dir()
        } else {
            !meta.is_file() || meta.nlink() != 1
        }
    {
        return Err("Caption cache ownership or permissions are unsafe.");
    }
    Ok(())
}
fn lock(parent: &File, name: &str, nonblocking: bool) -> Result<Option<File>, &'static str> {
    let file = open_at(parent, OsStr::new(name), libc::O_RDWR | libc::O_CREAT)
        .map_err(|_| "Private caption lock unavailable.")?;
    private_file(&file, false)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(file));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::WouldBlock {
            return Err("Private caption lock failed.");
        }
        if nonblocking {
            return Ok(None);
        }
        if std::time::Instant::now() >= deadline {
            return Err("Private caption cache is busy; try again later.");
        }
        // Only startup/cleanup workers wait here, with a fixed deadline.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
fn unlink(parent: &File, name: &OsStr, directory: bool) -> Result<(), &'static str> {
    let name = cstring(name)?;
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } == 0 {
        Ok(())
    } else {
        Err("Private caption cleanup was incomplete.")
    }
}
fn instance_name(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 32
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}
fn caption_name(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 36 && bytes.ends_with(b".vtt") && instance_name(OsStr::from_bytes(&bytes[..32]))
}
struct Entries(*mut libc::DIR);
impl Drop for Entries {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
fn names(directory: &File, limit: usize) -> Result<Vec<OsString>, &'static str> {
    let descriptor = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if descriptor < 0 {
        return Err("Private caption directory cannot be duplicated.");
    }
    let stream = unsafe { libc::fdopendir(descriptor) };
    if stream.is_null() {
        unsafe {
            libc::close(descriptor);
        }
        return Err("Private caption directory cannot be listed.");
    }
    let entries = Entries(stream);
    // dup shares the directory offset. Reset explicitly for deterministic
    // repeated enumeration through this descriptor, never reopen by path.
    unsafe {
        libc::rewinddir(entries.0);
    }
    let mut result = Vec::new();
    loop {
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = 0;
        }
        #[cfg(target_os = "linux")]
        unsafe {
            *libc::__errno_location() = 0;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err("Caption directory enumeration is not validated on this platform.");
        let entry = unsafe { libc::readdir(entries.0) };
        if entry.is_null() {
            if std::io::Error::last_os_error().raw_os_error() == Some(0) {
                break;
            }
            return Err("Private caption cache entry unavailable.");
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if result.len() >= limit {
            return Err("Private caption cache exceeded its recovery limit.");
        }
        result.push(OsString::from_vec(bytes.to_vec()));
    }
    Ok(result)
}
fn reap(root: &File) -> Result<usize, &'static str> {
    let mut live = 0;
    for name in names(root, MAX_INSTANCES + 1)? {
        if name == REGISTRY {
            continue;
        }
        if !instance_name(&name) {
            return Err("Unknown entry in private caption cache; recovery stopped.");
        }
        let directory = open_at(root, &name, libc::O_RDONLY | libc::O_DIRECTORY)
            .map_err(|_| "Unsafe caption cache directory; recovery stopped.")?;
        private_file(&directory, true)?;
        // The registry lock serializes creation and retirement, so an unlocked
        // child belongs to a dead instance. Never remove a live instance.
        let Some(_owner) = lock(&directory, LOCK, true)? else {
            live += 1;
            continue;
        };
        let entries = names(&directory, MAX_FILES + 1)?;
        // Validate the complete bounded set before any deletion. All mutation
        // uses the already opened directory descriptor, never follows symlinks.
        for entry in &entries {
            if entry == LOCK {
                continue;
            }
            if !caption_name(entry) {
                return Err("Unknown caption cache file; recovery stopped.");
            }
            let file = open_at(&directory, entry, libc::O_RDONLY)
                .map_err(|_| "Unsafe caption cache file; recovery stopped.")?;
            private_file(&file, false)?;
        }
        for entry in entries {
            unlink(&directory, &entry, false)?;
        }
        unlink(root, &name, true)?;
    }
    Ok(live)
}
pub(super) fn random_name() -> Result<String, &'static str> {
    let mut bytes = [0u8; 16];
    if unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return Err("Caption file entropy unavailable.");
    }
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
        // Only the caller's app-owned profile/cache descendants are created;
        // existing ancestors are never chmodded or scanned.
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|_| "Private caption profile unavailable.")?;
        let parent_file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent)
            .map_err(|_| "Unsafe caption profile directory.")?;
        private_file(&parent_file, true)?;
        let leaf = path
            .file_name()
            .ok_or("Caption cache has no directory name.")?;
        let name = cstring(leaf)?;
        if unsafe { libc::mkdirat(parent_file.as_raw_fd(), name.as_ptr(), 0o700) } != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err("Private caption cache unavailable.");
        }
        let root = open_at(&parent_file, leaf, libc::O_RDONLY | libc::O_DIRECTORY)
            .map_err(|_| "Unsafe private caption cache.")?;
        private_file(&root, true)?;
        let _registry =
            lock(&root, REGISTRY, false)?.ok_or("Caption registry lock unavailable.")?;
        if reap(&root)? >= MAX_INSTANCES {
            return Err("Too many live caption cache instances.");
        }
        let name = OsString::from(random_name()?);
        let c_name = cstring(&name)?;
        if unsafe { libc::mkdirat(root.as_raw_fd(), c_name.as_ptr(), 0o700) } != 0 {
            return Err("Private caption instance unavailable.");
        }
        let directory = open_at(&root, &name, libc::O_RDONLY | libc::O_DIRECTORY)
            .map_err(|_| "Private caption instance unavailable.")?;
        private_file(&directory, true)?;
        let owner = lock(&directory, LOCK, true)?.ok_or("Caption instance unexpectedly locked.")?;
        Ok(Self {
            path: path.join(&name),
            name,
            root,
            directory,
            _lock: owner,
        })
    }
    pub(super) fn create_file(&self, bytes: &[u8]) -> Result<CreatedFile, &'static str> {
        self.create_file_with(bytes, |file, bytes| file.write_all(bytes))
    }
    pub(super) fn create_file_with(
        &self,
        bytes: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
    ) -> Result<CreatedFile, &'static str> {
        let name = OsString::from(format!("{}.vtt", random_name()?));
        let mut file = open_at(
            &self.directory,
            &name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )
        .map_err(|_| "Private caption file unavailable.")?;
        let error = write(&mut file, bytes)
            .err()
            .map(|_| "Caption file write failed.");
        // Once created, every path returns to the owning worker even on write
        // failure. Its tracked released record bounds and reports deletion.
        Ok(CreatedFile {
            path: self.path.join(name),
            error,
        })
    }
    pub(super) fn remove_file(&self, path: &Path) -> Result<(), &'static str> {
        if path.parent() != Some(self.path.as_path()) {
            return Err("Caption cleanup path mismatch.");
        }
        let name = path.file_name().ok_or("Caption cleanup name missing.")?;
        if !caption_name(name) {
            return Err("Caption cleanup name mismatch.");
        }
        let name = CString::new(name.as_bytes()).map_err(|_| "Caption cleanup name invalid.")?;
        // Capture errno immediately; a known generated name already absent is
        // cleared successfully, without broadening the confined deletion scope.
        if unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) } == 0
            || std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            Ok(())
        } else {
            Err("Private caption deletion failed.")
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
        unlink(&self.directory, OsStr::new(LOCK), false)?;
        unlink(&self.root, &self.name, true)
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
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
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
        first.finish().unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn unlocked_crash_files_are_reaped_only_inside_private_cache() {
        let profile = Profile::new();
        let crashed = Directory::create(profile.cache()).unwrap();
        let stale = crashed.create_file(b"public caption").unwrap().path;
        let child = crashed.path.clone();
        drop(crashed); // Simulate OS closing lock descriptors without cleanup.
        let outside = profile.0.join("unrelated.vtt");
        std::fs::write(&outside, b"untouched").unwrap();
        let replacement = Directory::create(profile.cache()).unwrap();
        assert!(!stale.exists());
        assert!(!child.exists());
        assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
        replacement.finish().unwrap();
    }
    #[test]
    fn symlinks_and_unrecognized_entries_are_preserved_and_rejected() {
        let profile = Profile::new();
        let crashed = Directory::create(profile.cache()).unwrap();
        let outside = profile.0.join("outside.vtt");
        std::fs::write(&outside, b"must survive").unwrap();
        let symlink = crashed.path.join(format!("{}.vtt", random_name().unwrap()));
        std::os::unix::fs::symlink(&outside, &symlink).unwrap();
        drop(crashed);
        assert!(Directory::create(profile.cache()).is_err());
        assert!(
            std::fs::symlink_metadata(&symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"must survive");
        std::fs::remove_file(symlink).unwrap();
        std::fs::write(profile.cache().join("unknown-user-file"), b"preserve").unwrap();
        assert!(Directory::create(profile.cache()).is_err());
        assert_eq!(
            std::fs::read(profile.cache().join("unknown-user-file")).unwrap(),
            b"preserve"
        );
    }
    #[test]
    fn recovery_is_bounded_and_rejects_directory_symlinks() {
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
        let second_profile = Profile::new();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(second_profile.cache())
            .unwrap();
        let linked = second_profile.cache().join(random_name().unwrap());
        std::os::unix::fs::symlink(&profile.0, &linked).unwrap();
        assert!(Directory::create(second_profile.cache()).is_err());
        assert!(files.iter().all(|path| path.exists()));
    }
    #[test]
    fn cold_profile_is_private_and_enumeration_remains_bound_to_open_directory() {
        let profile = Profile::new();
        let parent = profile.0.join("Oxplay");
        let directory = Directory::create(parent.join("captions")).unwrap();
        assert_eq!(std::fs::metadata(&parent).unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            std::fs::metadata(parent.join("captions")).unwrap().mode() & 0o777,
            0o700
        );
        let own = directory.create_file(b"owned").unwrap().path;
        let own_name = own.file_name().unwrap().to_owned();
        let detached = profile.0.join("detached-instance");
        std::fs::rename(&directory.path, &detached).unwrap();
        let other = profile.0.join("other");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("foreign"), b"untouched").unwrap();
        std::os::unix::fs::symlink(&other, &directory.path).unwrap();
        let listed = names(&directory.directory, MAX_FILES + 1).unwrap();
        assert!(listed.contains(&own_name));
        assert!(!listed.contains(&OsString::from("foreign")));
        directory.remove_file(&own).unwrap();
        assert!(!detached.join(own_name).exists());
        assert_eq!(std::fs::read(other.join("foreign")).unwrap(), b"untouched");
        assert!(
            directory.finish().is_err(),
            "a substituted directory must not be followed"
        );
    }
}
