// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional local guest-video artwork, never a network client. Call on a worker.
//! Inputs are already normalized by the image worker; this module verifies the
//! bounded PNG envelope/CRC, while the image worker still validates decompression.
//! Unix operations stay relative to a retained private directory descriptor. An
//! exclusive lock excludes concurrent application instances; unsupported systems
//! fail closed without disabling ordinary session browsing.
use oxplay_core::VideoId;
use std::{fmt, path::Path};

pub const MAX_FILE_BYTES: usize = 512 * 1024;
pub const MAX_ENTRIES: usize = 4096;
// Keep directory/entry metadata and one atomic staging write inside the selected
// budget rather than allowing the payload alone to fill it. Files are charged by
// max(logical bytes, allocated blocks), not just their compressed PNG length.
const METADATA_RESERVE: u64 = 4 * 1024 * 1024;
const STAGING_RESERVE: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CacheLimit {
    Off,
    Mib32,
    Mib128,
    #[default]
    Mib256,
}
impl CacheLimit {
    pub fn from_mib(mib: u16) -> Option<Self> {
        match mib {
            0 => Some(Self::Off),
            32 => Some(Self::Mib32),
            128 => Some(Self::Mib128),
            256 => Some(Self::Mib256),
            _ => None,
        }
    }
    pub fn mib(self) -> u16 {
        match self {
            Self::Off => 0,
            Self::Mib32 => 32,
            Self::Mib128 => 128,
            Self::Mib256 => 256,
        }
    }
    fn payload_bytes(self) -> u64 {
        (u64::from(self.mib()) * 1024 * 1024).saturating_sub(METADATA_RESERVE + STAGING_RESERVE)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheError {
    Unavailable,
    UnsafePath,
    Corrupt,
    InvalidImage,
    Unsupported,
}
impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Local artwork cache is unavailable.",
            Self::UnsafePath => "Local artwork cache permissions or files are unsafe.",
            Self::Corrupt => "Cached artwork is damaged and must be fetched again.",
            Self::InvalidImage => "Artwork is not a supported normalized PNG.",
            Self::Unsupported => "Protected artwork caching is unavailable on this platform.",
        })
    }
}
impl std::error::Error for CacheError {}
type Result<T> = std::result::Result<T, CacheError>;

pub struct ArtworkCache {
    #[cfg(unix)]
    directory: unix::Directory,
    limit: CacheLimit,
    records: Vec<Record>,
    #[cfg(unix)]
    clock: u64,
}
struct Record {
    #[cfg(unix)]
    id: VideoId,
    bytes: u64,
    used: u64,
}
impl ArtworkCache {
    /// A disabled cache has no writer; callers can skip PNG encoding entirely.
    pub fn enabled(&self) -> bool {
        self.limit != CacheLimit::Off
    }

    /// An unavailable protected cache permits local clearing only when there is
    /// no cache to delete. Existing symlink/reparse ancestors are never followed;
    /// an existing cache requires the ordinary locked purge, not this check.
    pub fn clear_if_absent(path: &Path) -> Result<()> {
        use std::path::{Component, PathBuf};
        let parts: Vec<_> = path.components().collect();
        if !path.is_absolute()
            || parts.len() > 65
            || path.as_os_str().as_encoded_bytes().len() > 8192
            || parts
                .iter()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        {
            return Err(CacheError::UnsafePath);
        }
        let mut current = PathBuf::new();
        for part in parts {
            current.push(part.as_os_str());
            if matches!(part, Component::Prefix(_)) {
                continue;
            }
            let meta = match std::fs::symlink_metadata(&current) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => return Err(CacheError::Unavailable),
            };
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(CacheError::UnsafePath);
                }
            }
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(CacheError::UnsafePath);
            }
        }
        Err(CacheError::Unavailable)
    }
    /// `path` must be absolute, below an existing private application directory.
    /// Only its final cache directory is created. Existing unsafe files are not
    /// adopted or chmodded. A second instance returns Unavailable, never waits.
    pub fn open(path: &Path, limit: CacheLimit) -> Result<Self> {
        #[cfg(unix)]
        {
            let directory = unix::Directory::open(path)?;
            let mut cache = Self {
                directory,
                limit,
                records: Vec::new(),
                clock: 0,
            };
            cache.scan()?;
            cache.trim(0, false)?;
            Ok(cache)
        }
        #[cfg(not(unix))]
        {
            let _ = (path, limit);
            Err(CacheError::Unsupported)
        }
    }
    /// Cache-only: no URL is accepted and a miss never starts a request.
    pub fn get(&mut self, id: &VideoId) -> Result<Option<Vec<u8>>> {
        if self.limit == CacheLimit::Off {
            return Ok(None);
        }
        #[cfg(unix)]
        {
            let Some(index) = self.records.iter().position(|r| &r.id == id) else {
                return Ok(None);
            };
            let Some(mut file) = self.directory.file(&filename(id), false)? else {
                self.records.remove(index);
                return Ok(None);
            };
            let size = file.metadata().map_err(|_| CacheError::Unavailable)?.len();
            let mut bytes = Vec::new();
            if size <= MAX_FILE_BYTES as u64 {
                use std::io::Read;
                (&mut file)
                    .take(MAX_FILE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| CacheError::Unavailable)?;
            }
            if size == 0 || bytes.len() as u64 != size || !normalized_png(&bytes) {
                self.remove(index)?;
                return Err(CacheError::Corrupt);
            }
            let used = self.tick();
            self.records[index].used = used;
            Ok(Some(bytes))
        }
        #[cfg(not(unix))]
        {
            let _ = id;
            Err(CacheError::Unsupported)
        }
    }
    /// Atomic normalized PNG publication. No account artwork, URLs, provider
    /// responses or arbitrary filenames can be supplied through this API.
    pub fn put(&mut self, id: &VideoId, png: &[u8]) -> Result<()> {
        if !normalized_png(png) {
            return Err(CacheError::InvalidImage);
        }
        if self.limit == CacheLimit::Off {
            return Ok(());
        }
        #[cfg(unix)]
        {
            // Reserve worst-case staging space before any bytes are written.
            self.trim(MAX_FILE_BYTES as u64, true)?;
            let mut file = self.directory.stage()?;
            use std::io::Write;
            let staged = (|| {
                file.write_all(png).map_err(|_| CacheError::Unavailable)?;
                file.sync_all().map_err(|_| CacheError::Unavailable)?;
                let charge = unix::charge(&file.metadata().map_err(|_| CacheError::Unavailable)?);
                if charge > STAGING_RESERVE {
                    return Err(CacheError::Unavailable);
                }
                self.trim(charge, true)?;
                // Do not silently replace an unsafe existing file.
                self.directory.file(&filename(id), false)?;
                self.directory.publish(&filename(id))?;
                self.records.retain(|r| &r.id != id);
                let used = self.tick();
                self.records.push(Record {
                    id: id.clone(),
                    bytes: charge,
                    used,
                });
                self.directory.sync()
            })();
            drop(file);
            // Retain any failed cleanup as a known staging name; next put/open/
            // clear rechecks it before proceeding. Never acknowledge clear early.
            let cleanup = self.directory.remove(unix::STAGE);
            staged.and(cleanup)
        }
        #[cfg(not(unix))]
        {
            let _ = (id, png);
            Err(CacheError::Unsupported)
        }
    }
    pub fn set_limit(&mut self, limit: CacheLimit) -> Result<()> {
        self.limit = limit;
        if limit == CacheLimit::Off {
            self.clear()
        } else {
            self.trim(0, false)
        }
    }
    /// Deletes only recognized cache files. Unknown/symlink/hardlink entries
    /// fail closed and are preserved. Keeps the zero-byte instance lock.
    pub fn clear(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            self.scan()?;
            while !self.records.is_empty() {
                self.remove(self.records.len() - 1)?;
            }
            self.directory.remove(unix::STAGE)?;
            self.directory.sync()
        }
        #[cfg(not(unix))]
        {
            Err(CacheError::Unsupported)
        }
    }
    #[cfg(unix)]
    fn tick(&mut self) -> u64 {
        if self.clock == u64::MAX {
            self.records.sort_by_key(|r| r.used);
            for (index, record) in self.records.iter_mut().enumerate() {
                record.used = index as u64;
            }
            self.clock = self.records.len() as u64;
        }
        self.clock += 1;
        self.clock
    }
    fn trim(&mut self, extra: u64, adding: bool) -> Result<()> {
        while !self.records.is_empty()
            && (self
                .records
                .iter()
                .map(|r| r.bytes)
                .sum::<u64>()
                .saturating_add(extra)
                > self.limit.payload_bytes()
                || (adding && self.records.len() >= MAX_ENTRIES))
        {
            let oldest = self
                .records
                .iter()
                .enumerate()
                .min_by_key(|(_, r)| r.used)
                .map(|(i, _)| i)
                .expect("nonempty cache");
            self.remove(oldest)?;
        }
        Ok(())
    }
    fn remove(&mut self, index: usize) -> Result<()> {
        #[cfg(unix)]
        {
            self.directory.remove(&filename(&self.records[index].id))?;
            self.records.remove(index);
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = index;
            Err(CacheError::Unsupported)
        }
    }
    #[cfg(unix)]
    fn scan(&mut self) -> Result<()> {
        let mut records = Vec::new();
        self.directory.remove(unix::STAGE)?;
        for name in self.directory.names()? {
            if name == unix::LOCK {
                continue;
            }
            let id = name
                .strip_suffix(".png")
                .and_then(|id| VideoId::new(id).ok())
                .ok_or(CacheError::UnsafePath)?;
            let Some(file) = self.directory.file(&name, false)? else {
                continue;
            };
            let meta = file.metadata().map_err(|_| CacheError::Unavailable)?;
            if meta.len() > MAX_FILE_BYTES as u64 || meta.len() == 0 {
                self.directory.remove(&name)?;
                continue;
            }
            records.push((
                meta.modified().map_err(|_| CacheError::Unavailable)?,
                id,
                unix::charge(&meta),
            ));
        }
        // The persisted fallback order is last write time; access LRU is bounded
        // in memory and does not write metadata on every cached thumbnail read.
        records.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_str().cmp(b.1.as_str())));
        self.records = records
            .into_iter()
            .enumerate()
            .map(|(index, (_, id, bytes))| Record {
                id,
                bytes,
                used: index as u64,
            })
            .collect();
        self.clock = self.records.len() as u64;
        Ok(())
    }
}
#[cfg(unix)]
fn filename(id: &VideoId) -> String {
    format!("{}.png", id.as_str())
}

// PNG framing only, deliberately not another image decoder. Normalized RGBA8
// output has no metadata/profile/text/URL chunks; reject all other encodings.
fn normalized_png(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_FILE_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    let mut offset = 8;
    let mut header = false;
    let mut data = false;
    let mut chunks = 0;
    while offset + 12 <= bytes.len() {
        chunks += 1;
        if chunks > 256 {
            return false;
        }
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let Some(end) = offset
            .checked_add(length)
            .and_then(|n| n.checked_add(12))
            .filter(|&n| n <= bytes.len())
        else {
            return false;
        };
        let kind = &bytes[offset + 4..offset + 8];
        let payload = &bytes[offset + 8..end - 4];
        if crc32(&bytes[offset + 4..end - 4])
            != u32::from_be_bytes(bytes[end - 4..end].try_into().unwrap())
        {
            return false;
        }
        match kind {
            b"IHDR" if !header && offset == 8 && length == 13 => {
                let width = u32::from_be_bytes(payload[..4].try_into().unwrap());
                let height = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                if !(1..=320).contains(&width)
                    || !(1..=180).contains(&height)
                    || payload[8..] != [8, 6, 0, 0, 0]
                {
                    return false;
                }
                header = true;
            }
            b"IDAT" if header && length != 0 => {
                data = true;
            }
            b"IEND" if header && data && length == 0 => {
                return end == bytes.len();
            }
            _ => return false,
        }
        offset = end;
    }
    false
}
fn crc32(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0; 256];
        let mut index = 0;
        while index < 256 {
            let mut crc = index as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = (crc >> 1) ^ (0xedb88320u32.wrapping_mul(crc & 1));
                bit += 1;
            }
            table[index] = crc;
            index += 1;
        }
        table
    };
    !bytes.iter().fold(u32::MAX, |crc, byte| {
        (crc >> 8) ^ TABLE[((crc ^ u32::from(*byte)) & 255) as usize]
    })
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        fs::{File, Metadata},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
        path::Component,
    };
    pub(super) const LOCK: &str = ".lock";
    pub(super) const STAGE: &str = ".pending.png";
    pub(super) struct Directory {
        fd: File,
        _lock: File,
    }
    pub(super) fn charge(meta: &Metadata) -> u64 {
        meta.len().max(meta.blocks().saturating_mul(512))
    }
    fn c(value: &[u8]) -> Result<CString> {
        CString::new(value).map_err(|_| CacheError::UnsafePath)
    }
    fn io() -> CacheError {
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) => CacheError::UnsafePath,
            _ => CacheError::Unavailable,
        }
    }
    fn open_dir(parent: i32, name: &CStr) -> Result<File> {
        // SAFETY: valid C string; successful descriptor is uniquely adopted.
        let fd = unsafe {
            libc::openat(
                parent,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn private_dir(file: &File) -> Result<()> {
        let meta = file.metadata().map_err(|_| CacheError::Unavailable)?;
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(CacheError::UnsafePath);
        }
        Ok(())
    }
    impl Directory {
        pub(super) fn open(path: &Path) -> Result<Self> {
            if !path.is_absolute() {
                return Err(CacheError::UnsafePath);
            }
            let mut components = path.components();
            if components.next() != Some(Component::RootDir) {
                return Err(CacheError::UnsafePath);
            }
            let parts: Vec<_> = components.collect();
            if parts.is_empty()
                || parts.len() > 64
                || parts.iter().any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(CacheError::UnsafePath);
            }
            let mut parent = open_dir(libc::AT_FDCWD, c"/")?;
            for part in &parts[..parts.len() - 1] {
                parent = open_dir(parent.as_raw_fd(), &c(part.as_os_str().as_bytes())?)?;
            }
            private_dir(&parent)?;
            let name = c(parts.last().unwrap().as_os_str().as_bytes())?;
            let made = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
            if made != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                return Err(io());
            }
            let fd = open_dir(parent.as_raw_fd(), &name)?;
            private_dir(&fd)?;
            let lock = open_file(&fd, LOCK, true)?.ok_or(CacheError::Unavailable)?;
            if lock.metadata().map_err(|_| CacheError::Unavailable)?.len() != 0 {
                return Err(CacheError::UnsafePath);
            }
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(CacheError::Unavailable);
            }
            Ok(Self { fd, _lock: lock })
        }
        pub(super) fn file(&self, name: &str, create: bool) -> Result<Option<File>> {
            open_file(&self.fd, name, create)
        }
        pub(super) fn stage(&self) -> Result<File> {
            self.remove(STAGE)?;
            let name = c(STAGE.as_bytes())?;
            let fd = unsafe {
                libc::openat(
                    self.fd.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            validate_file(&file)?;
            Ok(file)
        }
        pub(super) fn remove(&self, name: &str) -> Result<()> {
            if self.file(name, false)?.is_none() {
                return Ok(());
            }
            let name = c(name.as_bytes())?;
            if unsafe { libc::unlinkat(self.fd.as_raw_fd(), name.as_ptr(), 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT)
            {
                return Err(io());
            }
            Ok(())
        }
        pub(super) fn publish(&self, name: &str) -> Result<()> {
            let (from, to) = (c(STAGE.as_bytes())?, c(name.as_bytes())?);
            if unsafe {
                libc::renameat(
                    self.fd.as_raw_fd(),
                    from.as_ptr(),
                    self.fd.as_raw_fd(),
                    to.as_ptr(),
                )
            } != 0
            {
                return Err(io());
            }
            Ok(())
        }
        pub(super) fn sync(&self) -> Result<()> {
            self.fd.sync_all().map_err(|_| CacheError::Unavailable)
        }
        pub(super) fn names(&self) -> Result<Vec<String>> {
            let copy = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
            if copy < 0 {
                return Err(io());
            }
            let dir = unsafe { libc::fdopendir(copy) };
            if dir.is_null() {
                unsafe { libc::close(copy) };
                return Err(io());
            }
            struct Listing(*mut libc::DIR);
            impl Drop for Listing {
                fn drop(&mut self) {
                    unsafe { libc::closedir(self.0) };
                }
            }
            let listing = Listing(dir);
            unsafe { libc::rewinddir(listing.0) };
            let mut names = Vec::new();
            loop {
                // Clear errno so an enumeration failure cannot acknowledge a
                // partial scan/clear as successful.
                #[cfg(target_os = "macos")]
                unsafe {
                    *libc::__error() = 0;
                }
                #[cfg(any(target_os = "linux", target_os = "android"))]
                unsafe {
                    *libc::__errno_location() = 0;
                }
                let entry = unsafe { libc::readdir(listing.0) };
                if entry.is_null() {
                    if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                        return Err(io());
                    }
                    break;
                }
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if name == b"." || name == b".." {
                    continue;
                }
                if names.len() > MAX_ENTRIES {
                    return Err(CacheError::UnsafePath);
                }
                names.push(
                    std::str::from_utf8(name)
                        .map_err(|_| CacheError::UnsafePath)?
                        .to_owned(),
                );
            }
            Ok(names)
        }
    }
    fn validate_file(file: &File) -> Result<()> {
        let meta = file.metadata().map_err(|_| CacheError::Unavailable)?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.nlink() != 1
            || meta.mode() & 0o077 != 0
        {
            return Err(CacheError::UnsafePath);
        }
        Ok(())
    }
    fn open_file(parent: &File, name: &str, create: bool) -> Result<Option<File>> {
        let name = c(name.as_bytes())?;
        let flags = libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if create {
                libc::O_RDWR | libc::O_CREAT
            } else {
                libc::O_RDONLY
            };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            if !create && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                return Ok(None);
            }
            return Err(io());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        validate_file(&file)?;
        Ok(Some(file))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    struct Fixture {
        _root: tempfile::TempDir,
        path: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let path = root.path().canonicalize().unwrap().join("artwork");
            Self { _root: root, path }
        }
        fn cache(&self, limit: CacheLimit) -> ArtworkCache {
            ArtworkCache::open(&self.path, limit).unwrap()
        }
    }
    fn id(index: usize) -> VideoId {
        VideoId::new(&format!("{index:011}")).unwrap()
    }
    fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], bytes: &[u8]) {
        png.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(bytes);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    // Valid, deterministic uncompressed zlib PNG fixture. This is a synthetic
    // image-format test, never public/provider artwork or an application icon.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = width.to_be_bytes().to_vec();
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut png, b"IHDR", &header);
        let raw = vec![0u8; (width as usize * 4 + 1) * height as usize];
        let mut zlib = vec![0x78, 0x01];
        let count = raw.len().div_ceil(65535);
        for (index, block) in raw.chunks(65535).enumerate() {
            zlib.push(u8::from(index + 1 == count));
            let len = block.len() as u16;
            zlib.extend_from_slice(&len.to_le_bytes());
            zlib.extend_from_slice(&(!len).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for byte in raw {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
        chunk(&mut png, b"IDAT", &zlib);
        chunk(&mut png, b"IEND", &[]);
        png
    }
    #[test]
    fn private_roundtrip_reopen_lock_and_clear() {
        let fixture = Fixture::new();
        let image = png(320, 180);
        let mut cache = fixture.cache(CacheLimit::Mib256);
        assert!(matches!(
            ArtworkCache::open(&fixture.path, CacheLimit::Mib32),
            Err(CacheError::Unavailable)
        ));
        cache.put(&id(1), &image).unwrap();
        assert_eq!(cache.get(&id(1)).unwrap(), Some(image.clone()));
        assert_eq!(cache.get(&id(2)).unwrap(), None);
        assert_eq!(
            fs::metadata(&fixture.path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(fixture.path.join(filename(&id(1))))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        drop(cache);
        let mut cache = fixture.cache(CacheLimit::Mib256);
        assert_eq!(cache.get(&id(1)).unwrap(), Some(image));
        cache.clear().unwrap();
        assert!(cache.get(&id(1)).unwrap().is_none());
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1); // zero-byte lock only
    }
    #[test]
    fn png_corruption_is_removed_and_invalid_input_never_publishes() {
        let fixture = Fixture::new();
        let mut cache = fixture.cache(CacheLimit::Mib32);
        let image = png(8, 8);
        cache.put(&id(1), &image).unwrap();
        let mut damaged = image.clone();
        damaged[45] ^= 1;
        fs::write(fixture.path.join(filename(&id(1))), &damaged).unwrap();
        assert_eq!(cache.get(&id(1)), Err(CacheError::Corrupt));
        assert!(cache.get(&id(1)).unwrap().is_none());
        for invalid in [
            damaged,
            vec![0; MAX_FILE_BYTES + 1],
            png(321, 1),
            png(1, 181),
            Vec::new(),
        ] {
            assert_eq!(cache.put(&id(2), &invalid), Err(CacheError::InvalidImage));
            assert!(!fixture.path.join(filename(&id(2))).exists());
        }
        // Recompute a CRC on an unwanted ancillary metadata chunk: even a
        // structurally intact image carrying text/profile metadata is rejected.
        let mut metadata = image[..image.len() - 12].to_vec();
        chunk(&mut metadata, b"tEXt", b"Synthetic\0not stored");
        chunk(&mut metadata, b"IEND", &[]);
        assert_eq!(cache.put(&id(2), &metadata), Err(CacheError::InvalidImage));
    }
    #[test]
    fn allocated_payload_eviction_and_limit_reduction_reserve_staging_headroom() {
        let fixture = Fixture::new();
        let mut cache = fixture.cache(CacheLimit::Mib128);
        let image = png(320, 180);
        for index in 0..150 {
            cache.put(&id(index), &image).unwrap();
        }
        assert_eq!(cache.records.len(), 150);
        cache.get(&id(0)).unwrap(); // retain an old write which was just used
        cache.set_limit(CacheLimit::Mib32).unwrap();
        assert!(cache.records.len() < 150);
        assert!(cache.get(&id(0)).unwrap().is_some());
        assert!(cache.get(&id(1)).unwrap().is_none());
        let bytes = cache.records.iter().map(|r| r.bytes).sum::<u64>();
        assert!(bytes + METADATA_RESERVE + STAGING_RESERVE <= 32 * 1024 * 1024);
        cache.set_limit(CacheLimit::Off).unwrap();
        assert!(cache.records.is_empty());
        cache.put(&id(151), &image).unwrap();
        assert!(cache.get(&id(151)).unwrap().is_none());
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1);
        assert_eq!(CacheLimit::from_mib(64), None);
        for mib in [0, 32, 128, 256] {
            assert_eq!(CacheLimit::from_mib(mib).unwrap().mib(), mib);
        }
    }
    #[test]
    fn symlinks_hardlinks_unknown_files_and_insecure_paths_are_not_adopted() {
        let fixture = Fixture::new();
        let mut cache = fixture.cache(CacheLimit::Mib32);
        let outside = fixture.path.parent().unwrap().join("outside");
        fs::write(&outside, b"Synthetic untouched outside file").unwrap();
        let candidate = fixture.path.join(filename(&id(1)));
        symlink(&outside, &candidate).unwrap();
        assert!(cache.put(&id(1), &png(1, 1)).is_err());
        assert!(cache.clear().is_err());
        assert_eq!(
            fs::read(&outside).unwrap(),
            b"Synthetic untouched outside file"
        );
        fs::remove_file(candidate).unwrap();
        cache.put(&id(1), &png(1, 1)).unwrap();
        let link = fixture.path.parent().unwrap().join("hardlink");
        fs::hard_link(fixture.path.join(filename(&id(1))), &link).unwrap();
        assert_eq!(cache.get(&id(1)), Err(CacheError::UnsafePath));
        assert_eq!(cache.clear(), Err(CacheError::UnsafePath));
        fs::remove_file(link).unwrap();
        cache.clear().unwrap();
        fs::write(fixture.path.join("unrecognized"), b"do not delete").unwrap();
        assert_eq!(cache.clear(), Err(CacheError::UnsafePath));
        assert!(fixture.path.join("unrecognized").exists());
        drop(cache);
        let alias = fixture.path.parent().unwrap().join("alias");
        symlink(&fixture.path, &alias).unwrap();
        assert!(matches!(
            ArtworkCache::open(&alias, CacheLimit::Mib32),
            Err(CacheError::UnsafePath)
        ));
        fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            ArtworkCache::open(&fixture.path, CacheLimit::Mib32),
            Err(CacheError::UnsafePath)
        ));
    }
    #[test]
    fn crash_stage_is_accounted_and_clear_does_not_follow_a_staging_symlink() {
        let fixture = Fixture::new();
        let cache = fixture.cache(CacheLimit::Mib32);
        drop(cache);
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(fixture.path.join(unix::STAGE))
            .unwrap();
        let mut cache = fixture.cache(CacheLimit::Off);
        assert!(!fixture.path.join(unix::STAGE).exists());
        let outside = fixture.path.parent().unwrap().join("outside");
        fs::write(&outside, b"retained").unwrap();
        symlink(&outside, fixture.path.join(unix::STAGE)).unwrap();
        assert!(cache.clear().is_err());
        assert_eq!(fs::read(outside).unwrap(), b"retained");
    }
    #[test]
    fn directory_replacement_cannot_redirect_retained_cache_operations() {
        let fixture = Fixture::new();
        let mut cache = fixture.cache(CacheLimit::Mib32);
        cache.put(&id(1), &png(1, 1)).unwrap();
        let original = fixture.path.with_extension("retained");
        fs::rename(&fixture.path, &original).unwrap();
        let outside = fixture.path.with_extension("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, &fixture.path).unwrap();
        cache.put(&id(2), &png(1, 1)).unwrap();
        cache.clear().unwrap();
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&original).unwrap().count(), 1);
    }
    #[test]
    fn unopened_clear_accepts_only_absence_without_following_ancestors() {
        let fixture = Fixture::new();
        assert_eq!(ArtworkCache::clear_if_absent(&fixture.path), Ok(()));
        assert_eq!(
            ArtworkCache::clear_if_absent(&fixture.path.join("missing")),
            Ok(())
        );
        let cache = fixture.cache(CacheLimit::Off);
        assert_eq!(
            ArtworkCache::clear_if_absent(&fixture.path),
            Err(CacheError::Unavailable)
        );
        drop(cache);
        let alias = fixture.path.with_extension("alias");
        symlink(&fixture.path, &alias).unwrap();
        assert_eq!(
            ArtworkCache::clear_if_absent(&alias.join("missing")),
            Err(CacheError::UnsafePath)
        );
        assert_eq!(
            ArtworkCache::clear_if_absent(Path::new("relative")),
            Err(CacheError::UnsafePath)
        );
    }
}
