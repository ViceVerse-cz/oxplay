// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish a completed SQLite snapshot without exposing a partial final backup.
use super::{Connection, Duration, LocalStore, Result, StorageError};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

struct StagedBackup {
    // All staging files, including any SQLite sidecars, stay under this private
    // sibling directory. Drop cleanup is best effort; a process crash can leave
    // the private directory, but never a partially copied final backup name.
    _directory: tempfile::TempDir,
    path: PathBuf,
    destination: PathBuf,
    _parent: PathBuf,
}

impl StagedBackup {
    fn publish(self) -> Result<()> {
        // Same filesystem by construction. Atomic creation of the final name
        // refuses an existing file/symlink, including one created during copying.
        // Do not fall back to copying or delete the destination on any failure.
        fs::hard_link(&self.path, &self.destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StorageError::BackupExists
            } else {
                StorageError::Unavailable
            }
        })?;
        #[cfg(unix)]
        fs::File::open(&self._parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| StorageError::Unavailable)?;
        Ok(())
    }
}

impl LocalStore {
    /// Creates a completed, synced SQLite snapshot before atomically publishing
    /// its final name. Never overwrites a destination or exposes a partial copy.
    /// Filesystems without same-directory hard links fail without a copy fallback.
    /// Restore only while the store is closed. Call on a worker thread.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<()> {
        self.prepare_backup(destination.as_ref())?.publish()
    }

    fn prepare_backup(&self, destination: &Path) -> Result<StagedBackup> {
        if destination.file_name().is_none() {
            return Err(StorageError::InvalidInput);
        }
        match fs::symlink_metadata(destination) {
            Ok(_) => return Err(StorageError::BackupExists),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StorageError::Unavailable),
        }
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut builder = tempfile::Builder::new();
        builder.prefix(".serein-backup-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // tempfile defaults to 0777 for directories. Set the creation mode
            // explicitly so no permissive interval precedes a later chmod.
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let directory = builder
            .tempdir_in(parent)
            .map_err(|_| StorageError::Unavailable)?;
        let path = directory.path().join("snapshot.sqlite3");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(&path).map_err(|_| StorageError::Unavailable)?);
        let mut target = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        {
            let backup = rusqlite::backup::Backup::new(&self.connection, &mut target)?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if Instant::now() >= deadline {
                    return Err(StorageError::Unavailable);
                }
                match backup.step(128)? {
                    rusqlite::backup::StepResult::Done => break,
                    rusqlite::backup::StepResult::More => {}
                    rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    _ => return Err(StorageError::Unavailable),
                }
            }
        }
        // A backup must be self-contained even if its source uses WAL. Finish
        // SQLite writes and close its handle before syncing/publishing one file.
        let journal: String =
            target.query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))?;
        if !journal.eq_ignore_ascii_case("delete") {
            return Err(StorageError::Unavailable);
        }
        target.close().map_err(|_| StorageError::Unavailable)?;
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .and_then(|file| file.sync_all())
            .map_err(|_| StorageError::Unavailable)?;
        Ok(StagedBackup {
            _directory: directory,
            path,
            destination: destination.to_owned(),
            _parent: parent.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_staging_never_exposes_final_backup_or_leaves_normal_stage_files() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("backup.sqlite3");
        let store = LocalStore::in_memory().unwrap();
        store
            .create_playlist("Synthetic complete snapshot")
            .unwrap();
        let staged = store.prepare_backup(&destination).unwrap();
        assert!(!destination.exists());
        let snapshot = LocalStore::open(&staged.path).unwrap();
        assert_eq!(snapshot.playlists(None, 100).unwrap().items.len(), 1);
        drop(snapshot);
        drop(staged); // Deliberate interruption before publication.
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn racing_destination_is_preserved_and_private_snapshot_is_cleaned() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("backup.sqlite3");
        let store = LocalStore::in_memory().unwrap();
        let staged = store.prepare_backup(&destination).unwrap();
        fs::write(&destination, b"Synthetic preexisting user file").unwrap();
        assert_eq!(staged.publish(), Err(StorageError::BackupExists));
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"Synthetic preexisting user file"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn publication_is_self_contained_for_an_uncheckpointed_wal_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.sqlite3");
        let destination = root.path().join("backup.sqlite3");
        let store = LocalStore::open(&source).unwrap();
        store
            .connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        store.create_playlist("Synthetic WAL data").unwrap();
        store.backup_to(&destination).unwrap();
        let restored = LocalStore::open(&destination).unwrap();
        assert_eq!(
            restored.playlists(None, 100).unwrap().items[0].name,
            "Synthetic WAL data"
        );
        assert!(!destination.with_extension("sqlite3-wal").exists());
        assert!(!destination.with_extension("sqlite3-shm").exists());
        assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".serein-backup-")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_and_published_snapshot_keep_private_modes() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("backup.sqlite3");
        let store = LocalStore::in_memory().unwrap();
        let staged = store.prepare_backup(&destination).unwrap();
        assert_eq!(
            fs::metadata(staged.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&staged.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        staged.publish().unwrap();
        assert_eq!(
            fs::metadata(destination).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn racing_symlink_is_not_followed_or_replaced() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("backup.sqlite3");
        let target = root.path().join("user-file");
        fs::write(&target, b"Synthetic untouched file").unwrap();
        let store = LocalStore::in_memory().unwrap();
        let staged = store.prepare_backup(&destination).unwrap();
        std::os::unix::fs::symlink(&target, &destination).unwrap();
        assert_eq!(staged.publish(), Err(StorageError::BackupExists));
        assert!(fs::symlink_metadata(&destination).unwrap().is_symlink());
        assert_eq!(fs::read(target).unwrap(), b"Synthetic untouched file");
    }
}
