// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct MemoryKeys(Arc<Mutex<BTreeMap<String, Zeroizing<Vec<u8>>>>>);
impl KeyStore for MemoryKeys {
    fn read(&self, profile: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Ok(self.0.lock().unwrap().get(profile).cloned())
    }
    fn create(&self, profile: &str, record: &[u8]) -> Result<()> {
        let mut keys = self.0.lock().unwrap();
        if keys.contains_key(profile) {
            return Err(VaultError::InUse);
        }
        keys.insert(profile.to_owned(), Zeroizing::new(record.to_vec()));
        Ok(())
    }
    fn delete(&self, profile: &str) -> Result<()> {
        self.0.lock().unwrap().remove(profile);
        Ok(())
    }
}
fn store(directory: &Path, keys: MemoryKeys) -> ProtectedSessionStore {
    #[cfg(unix)]
    if directory.exists() {
        use std::os::unix::fs::PermissionsExt;
        // tempfile directories honor the environment's umask; the vault contract
        // deliberately requires an explicit private directory instead.
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    ProtectedSessionStore {
        directory: directory.to_owned(),
        profile: SessionProfile::random().unwrap(),
        keys: Box::new(keys),
    }
}

#[test]
fn encrypted_roundtrip_nonce_rotation_and_key_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let keys = MemoryKeys::default();
    let store = store(directory.path(), keys.clone());
    let synthetic = b"OXPLAY_SYNTHETIC_COOKIE_NOT_A_REAL_CREDENTIAL";
    assert!(store.load().unwrap().is_none());
    store.save(synthetic).unwrap();
    let before = std::fs::read(store.envelope_path()).unwrap();
    assert!(
        !before
            .windows(synthetic.len())
            .any(|bytes| bytes == synthetic)
    );
    assert_eq!(store.load().unwrap().unwrap().expose(), synthetic);
    store.save(synthetic).unwrap();
    let after = std::fs::read(store.envelope_path()).unwrap();
    assert_ne!(&before[24..HEADER_BYTES], &after[24..HEADER_BYTES]);
    assert_eq!(&before[8..24], &after[8..24]);
    assert_eq!(keys.0.lock().unwrap().len(), 1);
    store.delete().unwrap();
    assert!(keys.0.lock().unwrap().is_empty());
    assert!(store.load().unwrap().is_none());
    // Even a restored old ciphertext cannot resurrect a disconnected session.
    std::fs::write(store.envelope_path(), before).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            store.envelope_path(),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    assert!(matches!(store.load(), Err(VaultError::KeychainUnavailable)));
    store.delete().unwrap();
}

#[test]
fn tampering_truncation_and_cross_profile_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let keys = MemoryKeys::default();
    let first = store(directory.path(), keys.clone());
    first.save(b"synthetic session").unwrap();
    let original = std::fs::read(first.envelope_path()).unwrap();
    for index in [0, 7, 8, 24, HEADER_BYTES, original.len() - 1] {
        let mut tampered = original.clone();
        tampered[index] ^= 1;
        std::fs::write(first.envelope_path(), tampered).unwrap();
        assert!(matches!(first.load(), Err(VaultError::InvalidEnvelope)));
    }
    std::fs::write(first.envelope_path(), &original[..HEADER_BYTES]).unwrap();
    assert!(matches!(first.load(), Err(VaultError::InvalidEnvelope)));
    let second = store(directory.path(), keys.clone());
    let record = keys.read(first.profile.as_str()).unwrap().unwrap();
    keys.create(second.profile.as_str(), &record).unwrap();
    std::fs::write(second.envelope_path(), original).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            second.envelope_path(),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    assert!(matches!(second.load(), Err(VaultError::InvalidEnvelope)));
}

#[test]
fn bounds_and_profile_names_are_enforced() {
    for profile in [
        "",
        "../outside",
        "0000000000000000000000000000000g",
        "000000000000000000000000000000000",
    ] {
        assert!(SessionProfile::parse(profile).is_err());
    }
    let profile = SessionProfile::random().unwrap();
    assert_eq!(SessionProfile::parse(profile.as_str()).unwrap(), profile);
    assert!(!format!("{profile:?}").contains(profile.as_str()));
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), MemoryKeys::default());
    assert_eq!(store.save(&[]), Err(VaultError::InvalidInput));
    assert_eq!(
        store.save(&vec![0; MAX_SESSION_BYTES + 1]),
        Err(VaultError::InvalidInput)
    );
    assert!(!store.envelope_path().exists());
    let full = vec![b'x'; MAX_SESSION_BYTES];
    store.save(&full).unwrap();
    assert_eq!(
        store.load().unwrap().unwrap().expose().len(),
        MAX_SESSION_BYTES
    );
}

#[test]
fn locked_or_denied_vault_never_persists_plaintext() {
    struct Denied;
    impl KeyStore for Denied {
        fn read(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
            Err(VaultError::KeychainUnavailable)
        }
        fn create(&self, _: &str, _: &[u8]) -> Result<()> {
            Err(VaultError::KeychainUnavailable)
        }
        fn delete(&self, _: &str) -> Result<()> {
            Err(VaultError::KeychainUnavailable)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut store = store(directory.path(), MemoryKeys::default());
    store.keys = Box::new(Denied);
    assert_eq!(
        store.save(b"synthetic only"),
        Err(VaultError::KeychainUnavailable)
    );
    assert!(!store.envelope_path().exists());
    let guard = store.lock().unwrap();
    assert!(matches!(store.load(), Err(VaultError::InUse)));
    drop(guard);
    assert_eq!(store.delete(), Err(VaultError::KeychainUnavailable));
}

#[cfg(unix)]
#[test]
fn private_permissions_and_symlink_rejection() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let private = directory.path().join("sessions");
    let store = store(&private, MemoryKeys::default());
    store.save(b"synthetic").unwrap();
    assert_eq!(
        std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(store.envelope_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(store.load(), Err(VaultError::UnsafePath)));
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::remove_file(store.envelope_path()).unwrap();
    let external = directory.path().join("outside");
    std::fs::write(&external, b"must remain unchanged").unwrap();
    symlink(&external, store.envelope_path()).unwrap();
    assert!(matches!(store.load(), Err(VaultError::UnsafePath)));
    store.delete().unwrap();
    assert_eq!(std::fs::read(external).unwrap(), b"must remain unchanged");
}

/// Explicit isolated integration test. Only writes one random application-owned
/// synthetic key, never scans/imports browser data. Cleanup is attempted on unwind;
/// OS denial is reported and cannot be represented as guaranteed deletion.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "explicit local macOS Keychain integration; no real credentials"]
fn macos_keychain_synthetic_roundtrip() {
    system_store_synthetic_roundtrip();
}

/// The same synthetic flow against Windows Credential Manager.
#[cfg(windows)]
#[test]
#[ignore = "explicit Windows Credential Manager integration; no real credentials"]
fn windows_credential_manager_synthetic_roundtrip() {
    system_store_synthetic_roundtrip();
}

#[cfg(any(target_os = "macos", windows))]
fn system_store_synthetic_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let profile = SessionProfile::random().unwrap();
    assert!(
        SystemKeyStore.read(profile.as_str()).unwrap().is_none(),
        "Refuse to touch any existing keychain item"
    );
    struct Cleanup(SessionProfile);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if SystemKeyStore.delete(self.0.as_str()).is_err() {
                eprintln!("Synthetic Keychain cleanup failed; no real account was used");
            }
        }
    }
    let _cleanup = Cleanup(profile.clone());
    let store = ProtectedSessionStore::new(directory.path().join("sessions"), profile).unwrap();
    store
        .save(b"OXPLAY_SYNTHETIC_KEYCHAIN_TEST_NO_USER_CREDENTIAL")
        .unwrap();
    assert_eq!(
        store.load().unwrap().unwrap().expose(),
        b"OXPLAY_SYNTHETIC_KEYCHAIN_TEST_NO_USER_CREDENTIAL"
    );
    let before = std::fs::read(store.envelope_path()).unwrap();
    let reopened =
        ProtectedSessionStore::new(directory.path().join("sessions"), store.profile.clone())
            .unwrap();
    assert_eq!(
        reopened.load().unwrap().unwrap().expose(),
        b"OXPLAY_SYNTHETIC_KEYCHAIN_TEST_NO_USER_CREDENTIAL"
    );
    reopened
        .save(b"OXPLAY_SYNTHETIC_REPLACEMENT_SESSION")
        .unwrap();
    let after = std::fs::read(store.envelope_path()).unwrap();
    assert_eq!(
        &before[8..24],
        &after[8..24],
        "rewrite changed the protected key identity"
    );
    assert_ne!(
        &before[24..HEADER_BYTES],
        &after[24..HEADER_BYTES],
        "rewrite reused its encryption nonce"
    );
    assert_eq!(
        store.load().unwrap().unwrap().expose(),
        b"OXPLAY_SYNTHETIC_REPLACEMENT_SESSION"
    );
    store.delete().unwrap();
    assert!(!store.envelope_path().exists());
    assert!(reopened.load().unwrap().is_none());
    assert!(
        SystemKeyStore
            .read(store.profile.as_str())
            .unwrap()
            .is_none()
    );
}
