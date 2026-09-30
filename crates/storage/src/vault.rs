// SPDX-License-Identifier: GPL-3.0-or-later
//! OS-protected keys plus authenticated encrypted session envelopes.
//! All calls are synchronous worker-thread operations. Constructing a store
//! does not read a keychain or connect an account. Only explicitly authorized,
//! identity-verified imported material should reach `save`.
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
#[cfg(unix)]
use std::fs::OpenOptions;
use std::{
    fmt,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub const MAX_SESSION_BYTES: usize = 4 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"OXPLAY\0\x01";
const HEADER_BYTES: usize = 8 + 16 + 24;
const MAX_ENVELOPE_BYTES: usize = HEADER_BYTES + MAX_SESSION_BYTES + 16;
const KEY_RECORD_BYTES: usize = 16 + 32;
const SERVICE: &str = "cz.viceverse.oxplay.session-key.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultError {
    UnsupportedPlatform,
    InvalidInput,
    KeychainUnavailable,
    InvalidEnvelope,
    UnsafePath,
    StorageUnavailable,
    InUse,
    RandomUnavailable,
}
impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedPlatform => "Protected session storage is not implemented on this platform. Use an explicit session-only connection.",
            Self::InvalidInput => "The session or local profile identifier is invalid.",
            Self::KeychainUnavailable => "The operating-system credential store is unavailable or access was denied. No plaintext session was saved.",
            Self::InvalidEnvelope => "The stored session failed validation. Reimport the session explicitly.",
            Self::UnsafePath => "The session directory or file is not private and safe to use.",
            Self::StorageUnavailable => "Protected session storage could not be updated.",
            Self::InUse => "Another operation is using protected session storage. Retry after it finishes.",
            Self::RandomUnavailable => "Secure random generation failed. The session was not saved.",
        })
    }
}
impl std::error::Error for VaultError {}
type Result<T> = std::result::Result<T, VaultError>;

/// A random application-local identifier, not a Google account name or channel ID.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionProfile(String);
impl SessionProfile {
    pub fn random() -> Result<Self> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| VaultError::RandomUnavailable)?;
        Ok(Self(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(VaultError::InvalidInput);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SessionProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionProfile([redacted])")
    }
}

/// Not Clone, Debug, Display or serializable; secret access is deliberately explicit.
pub struct SecretSession(Zeroizing<Vec<u8>>);
impl SecretSession {
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

/// Private so production cannot accidentally select an in-memory test key store.
trait KeyStore: Send {
    fn read(&self, profile: &str) -> Result<Option<Zeroizing<Vec<u8>>>>;
    fn create(&self, profile: &str, record: &[u8]) -> Result<()>;
    fn delete(&self, profile: &str) -> Result<()>;
}

struct SystemKeyStore;
#[cfg(target_os = "macos")]
impl SystemKeyStore {
    fn options(profile: &str) -> security_framework::passwords::PasswordOptions {
        let mut options =
            security_framework::passwords::PasswordOptions::new_generic_password(SERVICE, profile);
        // Explicitly target only the local login Keychain, never iCloud Keychain.
        options.set_access_synchronized(Some(false));
        options
    }
}
#[cfg(target_os = "macos")]
impl KeyStore for SystemKeyStore {
    fn read(&self, profile: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        match security_framework::passwords::generic_password(Self::options(profile)) {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(None)
            }
            Err(_) => Err(VaultError::KeychainUnavailable),
        }
    }
    fn create(&self, profile: &str, record: &[u8]) -> Result<()> {
        // Classic local Keychain add is atomic and never replaces an existing
        // item, unlike set_generic_password's upsert. No iCloud synchronization.
        let keychain = security_framework::os::macos::keychain::SecKeychain::default()
            .map_err(|_| VaultError::KeychainUnavailable)?;
        keychain
            .add_generic_password(SERVICE, profile, record)
            .map_err(|error| {
                if error.code() == security_framework_sys::base::errSecDuplicateItem {
                    VaultError::InUse
                } else {
                    VaultError::KeychainUnavailable
                }
            })
    }
    fn delete(&self, profile: &str) -> Result<()> {
        match security_framework::passwords::delete_generic_password_options(Self::options(profile))
        {
            Ok(()) => Ok(()),
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(())
            }
            Err(_) => Err(VaultError::KeychainUnavailable),
        }
    }
}
/// Windows Credential Manager: one local-machine-persisted generic credential
/// per random profile, protected by the user's logon credentials (DPAPI). Like
/// the macOS Keychain item it is separate from the ciphertext directory, so a
/// copied envelope cannot be decrypted after disconnect deletes the key.
#[cfg(windows)]
mod credential_manager {
    use super::{Result, SERVICE, VaultError};
    use windows_sys::Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
            CredReadW, CredWriteW,
        },
    };
    use zeroize::Zeroizing;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn target(profile: &str) -> Vec<u16> {
        wide(&format!("{SERVICE}/{profile}"))
    }
    fn not_found() -> bool {
        std::io::Error::last_os_error().raw_os_error() == Some(ERROR_NOT_FOUND as i32)
    }

    pub(super) fn read(profile: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let target = target(profile);
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: CredReadW allocates the credential; it is freed exactly once
        // after its blob is copied into a zeroizing buffer.
        unsafe {
            if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
                return if not_found() {
                    Ok(None)
                } else {
                    Err(VaultError::KeychainUnavailable)
                };
            }
            let record = &*credential;
            let bytes = if record.CredentialBlob.is_null() {
                Zeroizing::new(Vec::new())
            } else {
                Zeroizing::new(
                    std::slice::from_raw_parts(
                        record.CredentialBlob,
                        record.CredentialBlobSize as usize,
                    )
                    .to_vec(),
                )
            };
            CredFree(credential.cast());
            Ok(Some(bytes))
        }
    }

    pub(super) fn create(profile: &str, record: &[u8]) -> Result<()> {
        // CredWriteW is an upsert. The caller holds the vault file lock, and a
        // pre-existing key is reported instead of silently replaced.
        if read(profile)?.is_some() {
            return Err(VaultError::InUse);
        }
        let mut target = target(profile);
        let mut user = wide(profile);
        let mut blob = Zeroizing::new(record.to_vec());
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: every pointer references a live local buffer for this call.
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(VaultError::KeychainUnavailable);
        }
        Ok(())
    }

    pub(super) fn delete(profile: &str) -> Result<()> {
        let target = target(profile);
        // SAFETY: NUL-terminated target name alive for the call.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 && !not_found() {
            return Err(VaultError::KeychainUnavailable);
        }
        Ok(())
    }
}
#[cfg(windows)]
impl KeyStore for SystemKeyStore {
    fn read(&self, profile: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        credential_manager::read(profile)
    }
    fn create(&self, profile: &str, record: &[u8]) -> Result<()> {
        credential_manager::create(profile, record)
    }
    fn delete(&self, profile: &str) -> Result<()> {
        credential_manager::delete(profile)
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
impl KeyStore for SystemKeyStore {
    fn read(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Err(VaultError::UnsupportedPlatform)
    }
    fn create(&self, _: &str, _: &[u8]) -> Result<()> {
        Err(VaultError::UnsupportedPlatform)
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(VaultError::UnsupportedPlatform)
    }
}

pub struct ProtectedSessionStore {
    directory: PathBuf,
    profile: SessionProfile,
    keys: Box<dyn KeyStore>,
}
impl ProtectedSessionStore {
    pub fn platform_supported() -> bool {
        cfg!(any(target_os = "macos", windows))
    }

    /// No automatic import, credential read, or directory creation occurs here.
    pub fn new(directory: impl AsRef<Path>, profile: SessionProfile) -> Result<Self> {
        if !Self::platform_supported() {
            return Err(VaultError::UnsupportedPlatform);
        }
        if !directory.as_ref().is_absolute() {
            return Err(VaultError::UnsafePath);
        }
        Ok(Self {
            directory: directory.as_ref().to_owned(),
            profile,
            keys: Box::new(SystemKeyStore),
        })
    }

    fn envelope_path(&self) -> PathBuf {
        self.directory.join(format!("{}.session", self.profile.0))
    }

    /// Persists only the provided opaque, already validated session. Caller must
    /// check its session generation immediately before calling this method.
    pub fn save(&self, session: &[u8]) -> Result<()> {
        if session.is_empty() || session.len() > MAX_SESSION_BYTES {
            return Err(VaultError::InvalidInput);
        }
        let _guard = self.lock()?;
        let existing = self.keys.read(&self.profile.0)?;
        let new_key = existing.is_none();
        let record = match existing {
            Some(record) if record.len() == KEY_RECORD_BYTES => record,
            Some(_) => return Err(VaultError::InvalidEnvelope),
            None => {
                // Never replace a missing key for an existing envelope silently.
                if self
                    .envelope_path()
                    .try_exists()
                    .map_err(|_| VaultError::StorageUnavailable)?
                {
                    return Err(VaultError::InvalidEnvelope);
                }
                let mut record = Zeroizing::new(vec![0; KEY_RECORD_BYTES]);
                getrandom::fill(&mut record).map_err(|_| VaultError::RandomUnavailable)?;
                self.keys.create(&self.profile.0, &record)?;
                record
            }
        };
        let result = self.write_envelope(session, &record);
        if result.is_err() && new_key {
            // An unsuccessful first save does not leave an app-owned orphan key.
            self.keys.delete(&self.profile.0)?;
        }
        result
    }

    pub fn load(&self) -> Result<Option<SecretSession>> {
        let _guard = self.lock()?;
        let path = self.envelope_path();
        let mut file = match private_open(&path, false) {
            Ok(file) => file,
            Err(VaultError::StorageUnavailable)
                if !path
                    .try_exists()
                    .map_err(|_| VaultError::StorageUnavailable)? =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let size = file
            .metadata()
            .map_err(|_| VaultError::StorageUnavailable)?
            .len();
        if size < (HEADER_BYTES + 16) as u64 || size > MAX_ENVELOPE_BYTES as u64 {
            return Err(VaultError::InvalidEnvelope);
        }
        let mut envelope = Vec::with_capacity(size as usize);
        Read::by_ref(&mut file)
            .take((MAX_ENVELOPE_BYTES + 1) as u64)
            .read_to_end(&mut envelope)
            .map_err(|_| VaultError::StorageUnavailable)?;
        if envelope.len() > MAX_ENVELOPE_BYTES
            || envelope.len() < HEADER_BYTES + 16
            || &envelope[..8] != MAGIC
        {
            return Err(VaultError::InvalidEnvelope);
        }
        let record = self
            .keys
            .read(&self.profile.0)?
            .ok_or(VaultError::KeychainUnavailable)?;
        if record.len() != KEY_RECORD_BYTES || record[..16] != envelope[8..24] {
            return Err(VaultError::InvalidEnvelope);
        }
        let cipher = XChaCha20Poly1305::new_from_slice(&record[16..])
            .map_err(|_| VaultError::InvalidEnvelope)?;
        let aad = self.aad(&envelope[..HEADER_BYTES]);
        let bytes = cipher
            .decrypt(
                XNonce::from_slice(&envelope[24..HEADER_BYTES]),
                Payload {
                    msg: &envelope[HEADER_BYTES..],
                    aad: &aad,
                },
            )
            .map_err(|_| VaultError::InvalidEnvelope)?;
        Ok(Some(SecretSession(Zeroizing::new(bytes))))
    }

    /// Call after invalidating the account generation, stopping authenticated
    /// work/playback, and dropping in-memory secrets. Deleting a connection does
    /// not revoke the browser session. Failure must remain visible to the user.
    pub fn delete(&self) -> Result<()> {
        let _guard = self.lock()?;
        // Remove key first so a stale copied ciphertext cannot be decrypted via
        // this app's keychain item. Still attempt ciphertext deletion on denial.
        let key_result = self.keys.delete(&self.profile.0);
        let file_result = match std::fs::remove_file(self.envelope_path()) {
            Ok(()) => sync_directory(&self.directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(VaultError::StorageUnavailable),
        };
        key_result.and(file_result)
    }

    fn aad(&self, header: &[u8]) -> Vec<u8> {
        let mut aad = Vec::with_capacity(header.len() + SERVICE.len() + 32);
        aad.extend_from_slice(header);
        aad.extend_from_slice(SERVICE.as_bytes());
        aad.extend_from_slice(self.profile.0.as_bytes());
        aad
    }

    fn write_envelope(&self, session: &[u8], record: &[u8]) -> Result<()> {
        let mut header = [0; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8..24].copy_from_slice(&record[..16]);
        getrandom::fill(&mut header[24..]).map_err(|_| VaultError::RandomUnavailable)?;
        let cipher = XChaCha20Poly1305::new_from_slice(&record[16..])
            .map_err(|_| VaultError::InvalidEnvelope)?;
        let aad = self.aad(&header);
        let encrypted = cipher
            .encrypt(
                XNonce::from_slice(&header[24..]),
                Payload {
                    msg: session,
                    aad: &aad,
                },
            )
            .map_err(|_| VaultError::InvalidEnvelope)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".oxplay-envelope-")
            .tempfile_in(&self.directory)
            .map_err(|_| VaultError::StorageUnavailable)?;
        temporary
            .write_all(&header)
            .and_then(|_| temporary.write_all(&encrypted))
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|_| VaultError::StorageUnavailable)?;
        // Atomic same-directory replacement; no plaintext ever enters the file.
        temporary
            .persist(self.envelope_path())
            .map_err(|_| VaultError::StorageUnavailable)?;
        sync_directory(&self.directory)
    }

    fn lock(&self) -> Result<File> {
        ensure_private_directory(&self.directory)?;
        let file = private_open(&self.directory.join(".oxplay-session.lock"), true)?;
        file.try_lock().map_err(|_| VaultError::InUse)?;
        Ok(file)
    }
}

#[cfg(unix)]
fn ensure_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0 =>
        {
            Ok(())
        }
        Ok(_) => Err(VaultError::UnsafePath),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Do not create or change permissions on ancestor directories.
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|_| VaultError::StorageUnavailable)?;
            ensure_private_directory(path)
        }
        Err(_) => Err(VaultError::StorageUnavailable),
    }
}
#[cfg(windows)]
fn ensure_private_directory(path: &Path) -> Result<()> {
    crate::windows_private::ensure_private_directory(path).map_err(windows_error)
}
#[cfg(windows)]
fn windows_error(error: std::io::Error) -> VaultError {
    if error.kind() == std::io::ErrorKind::InvalidInput {
        VaultError::UnsafePath
    } else {
        VaultError::StorageUnavailable
    }
}
#[cfg(not(any(unix, windows)))]
fn ensure_private_directory(_: &Path) -> Result<()> {
    Err(VaultError::UnsupportedPlatform)
}

#[cfg(unix)]
fn private_open(path: &Path, create: bool) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                VaultError::UnsafePath
            } else {
                VaultError::StorageUnavailable
            }
        })?;
    let metadata = file
        .metadata()
        .map_err(|_| VaultError::StorageUnavailable)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(VaultError::UnsafePath);
    }
    Ok(file)
}
#[cfg(windows)]
fn private_open(path: &Path, create: bool) -> Result<File> {
    crate::windows_private::open_no_follow(path, create, create).map_err(windows_error)
}
#[cfg(not(any(unix, windows)))]
fn private_open(_: &Path, _: bool) -> Result<File> {
    Err(VaultError::UnsupportedPlatform)
}
#[cfg(not(windows))]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| VaultError::StorageUnavailable)
}
/// std cannot open a Windows directory handle for flushing. NTFS journals the
/// rename metadata, and the envelope contents were synced before the rename.
#[cfg(windows)]
fn sync_directory(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
