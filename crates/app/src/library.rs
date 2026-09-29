// SPDX-License-Identifier: GPL-3.0-or-later
//! SQLite stays on one dedicated worker. Queues are bounded and teardown closes
//! the result receiver before joining, including when the UI no longer drains it.
use serein_core::{ChannelId, VideoId, VideoSummary};
use serein_storage::{
    HistoryCursor, HistoryEntry, ImportSummary, LocalPlaylist, LocalPlaylistId, LocalPreferences,
    LocalStore, LocalSubscription, MAX_PAGE_SIZE, MAX_TRANSFER_BYTES, Page, PageCursor,
    StorageError, vault::SessionProfile,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreferenceWrite {
    pub id: u64,
    pub value: LocalPreferences,
}
pub enum SaveDestination {
    Existing(LocalPlaylistId),
    New(String),
}
pub struct VideoSave {
    pub serial: u64,
    pub destination: SaveDestination,
    pub video: VideoSummary,
}
/// Authoritative bounded page reads carry an application ticket through every
/// success and failure, including database initialization failure.
pub enum PageQuery {
    Collections(Option<PageCursor>),
    Videos(LocalPlaylistId, Option<PageCursor>),
    Subscriptions(Option<PageCursor>),
    History(Option<HistoryCursor>),
}
pub enum PageResult {
    Collections(Page<LocalPlaylist>),
    Videos(LocalPlaylistId, Page<VideoSummary>),
    Subscriptions(Page<LocalSubscription>),
    History {
        entries: Vec<HistoryEntry>,
        next: Option<HistoryCursor>,
        retention_days: u16,
    },
}
pub enum Request {
    Load,
    Create(String),
    DeleteEmpty(LocalPlaylistId),
    Save(VideoSave),
    Preferences(PreferenceWrite),
    ReadPage {
        ticket: u64,
        page: PageQuery,
    },
    /// Separate ticket namespace from the local-library page controller.
    Home {
        ticket: u64,
        after: Option<PageCursor>,
    },
    Rename(LocalPlaylistId, String),
    /// Caller must obtain explicit confirmation for a nonempty collection.
    Delete(LocalPlaylistId),
    Remove(LocalPlaylistId, VideoId),
    Follow(ChannelId, String),
    Unfollow(ChannelId),
    RecordHistory(VideoSummary, Duration),
    HistoryRetention(u16),
    DeleteHistory(VideoId),
    ClearHistory,
    /// Only user-selected files; neither operation discovers files automatically.
    Import(PathBuf),
    Export {
        path: PathBuf,
        overwrite: bool,
    },
    Backup(PathBuf),
    /// Explicit deletion of local data; has no account or credential side effects.
    ClearLocalData,
}
pub enum Response {
    Library(
        Vec<LocalPlaylist>,
        LocalPreferences,
        Option<LocalPlaylistId>,
    ),
    Saved,
    VideoSaved(u64),
    VideoSaveFailed(u64, String),
    PreferencesSaved(PreferenceWrite),
    PreferencesFailed(PreferenceWrite, String),
    Error(String),
    BackgroundError(String),
    PageRead {
        ticket: u64,
        result: Result<PageResult, String>,
    },
    Home {
        ticket: u64,
        result: Result<Page<VideoSummary>, String>,
    },
    HistoryRecorded(bool),
    Imported(ImportSummary),
    Exported,
    BackedUp,
    Cleared(LocalPreferences),
    ClearFailed(String),
}
pub struct Worker {
    command: Option<mpsc::SyncSender<Request>>,
    results: Option<mpsc::Receiver<Response>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new(path: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        let (command, commands) = mpsc::sync_channel(32);
        let (results_tx, results) = mpsc::sync_channel(32);
        let thread = thread::spawn(move || {
            let store = (|| {
                let parent = path
                    .parent()
                    .ok_or(serein_storage::StorageError::Unavailable)?;
                prepare_library_directory(parent)?;
                LocalStore::open(&path)
            })();
            // Initialization failure still answers each accepted operation with
            // its own response kind; a generic startup error cannot acknowledge
            // a later destructive operation.
            let mut store = store;
            while let Ok(request) = commands.recv() {
                let preferences = if let Request::Preferences(prefs) = &request {
                    Some(*prefs)
                } else {
                    None
                };
                let background = matches!(&request, Request::RecordHistory(_, _));
                let clearing = matches!(&request, Request::ClearLocalData);
                let saving = if let Request::Save(save) = &request {
                    Some(save.serial)
                } else {
                    None
                };
                let reading = if let Request::ReadPage { ticket, .. } = &request {
                    Some(*ticket)
                } else {
                    None
                };
                let home = if let Request::Home { ticket, .. } = &request {
                    Some(*ticket)
                } else {
                    None
                };
                let response = match &mut store {
                    Ok(store) => handle(store, request),
                    Err(error) => Err(*error),
                }
                .unwrap_or_else(|e| {
                    if let Some(ticket) = home {
                        Response::Home {
                            ticket,
                            result: Err(e.to_string()),
                        }
                    } else if let Some(ticket) = reading {
                        Response::PageRead {
                            ticket,
                            result: Err(e.to_string()),
                        }
                    } else if let Some(serial) = saving {
                        Response::VideoSaveFailed(serial, e.to_string())
                    } else if clearing {
                        Response::ClearFailed(e.to_string())
                    } else if let Some(prefs) = preferences {
                        Response::PreferencesFailed(prefs, e.to_string())
                    } else if background {
                        Response::BackgroundError(e.to_string())
                    } else {
                        Response::Error(e.to_string())
                    }
                });
                if results_tx.send(response).is_ok() {
                    wake();
                }
                // Closing the UI releases a blocked result send, but does not
                // revoke already accepted local writes. Drain the bounded
                // command queue until its sender closes during Drop.
            }
        });
        Self {
            command: Some(command),
            results: Some(results),
            thread: Some(thread),
        }
    }
    pub fn submit(&self, request: Request) -> bool {
        self.command
            .as_ref()
            .is_some_and(|tx| tx.try_send(request).is_ok())
    }
    pub fn take(&self) -> Option<Response> {
        self.results.as_ref()?.try_recv().ok()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.results.take();
        self.command.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn summary(
    store: &LocalStore,
    selected: Option<LocalPlaylistId>,
) -> serein_storage::Result<Response> {
    Ok(Response::Library(
        store.playlists(None, 100)?.items,
        store.preferences()?,
        selected,
    ))
}
fn read_page(store: &LocalStore, page: PageQuery) -> serein_storage::Result<PageResult> {
    Ok(match page {
        PageQuery::Collections(after) => {
            PageResult::Collections(store.playlists(after, MAX_PAGE_SIZE)?)
        }
        PageQuery::Videos(id, after) => {
            PageResult::Videos(id, store.playlist_videos(id, after, MAX_PAGE_SIZE)?)
        }
        PageQuery::Subscriptions(after) => {
            PageResult::Subscriptions(store.subscriptions(after, MAX_PAGE_SIZE)?)
        }
        PageQuery::History(after) => {
            let (entries, next) =
                store.history(after.as_ref(), MAX_PAGE_SIZE, SystemTime::now())?;
            PageResult::History {
                entries,
                next,
                retention_days: store.history_retention_days()?,
            }
        }
    })
}
fn handle(store: &mut LocalStore, request: Request) -> serein_storage::Result<Response> {
    match request {
        Request::Load => summary(store, None),
        Request::Create(name) => {
            let item = store.create_playlist(&name)?;
            summary(store, Some(item))
        }
        Request::DeleteEmpty(id) => {
            if !store.playlist_videos(id, None, 1)?.items.is_empty() {
                return Err(serein_storage::StorageError::InvalidInput);
            }
            store.delete_playlist(id)?;
            summary(store, None)
        }
        Request::Save(save) => {
            match save.destination {
                SaveDestination::Existing(id) => store.save_video(id, &save.video)?,
                SaveDestination::New(name) => {
                    store.create_playlist_with_video(&name, &save.video)?;
                }
            }
            Ok(Response::VideoSaved(save.serial))
        }
        Request::Preferences(prefs) => {
            store.set_preferences(prefs.value)?;
            Ok(Response::PreferencesSaved(prefs))
        }
        Request::ReadPage { ticket, page } => Ok(Response::PageRead {
            ticket,
            result: read_page(store, page).map_err(|error| error.to_string()),
        }),
        Request::Home { ticket, after } => Ok(Response::Home {
            ticket,
            result: store
                .recently_saved_videos(after, MAX_PAGE_SIZE)
                .map_err(|error| error.to_string()),
        }),
        Request::Rename(id, name) => {
            store.rename_playlist(id, &name)?;
            Ok(Response::Saved)
        }
        Request::Delete(id) => {
            store.delete_playlist(id)?;
            Ok(Response::Saved)
        }
        Request::Remove(id, video) => {
            store.remove_video(id, &video)?;
            Ok(Response::Saved)
        }
        Request::Follow(id, name) => {
            store.follow_channel(&id, &name)?;
            Ok(Response::Saved)
        }
        Request::Unfollow(id) => {
            store.unfollow_channel(&id)?;
            Ok(Response::Saved)
        }
        Request::RecordHistory(video, position) => Ok(Response::HistoryRecorded(
            store.record_history(&video, position, SystemTime::now())?,
        )),
        Request::HistoryRetention(days) => {
            store.set_history_retention_days(days, SystemTime::now())?;
            Ok(Response::Saved)
        }
        Request::DeleteHistory(id) => {
            store.delete_history_video(&id)?;
            Ok(Response::Saved)
        }
        Request::ClearHistory => {
            store.clear_history()?;
            Ok(Response::Saved)
        }
        Request::Import(path) => {
            let bytes = read_selected_import(&path)?;
            Ok(Response::Imported(store.import_library_json(&bytes)?))
        }
        Request::Export { path, overwrite } => {
            let bytes = store.export_library_json()?;
            atomic_export(&path, &bytes, overwrite)?;
            Ok(Response::Exported)
        }
        Request::Backup(path) => {
            if !path.is_absolute() {
                return Err(StorageError::InvalidInput);
            }
            store.backup_to(&path)?;
            Ok(Response::BackedUp)
        }
        Request::ClearLocalData => {
            store.clear_local_data()?;
            Ok(Response::Cleared(LocalPreferences::default()))
        }
    }
}
pub(crate) fn data_path() -> serein_storage::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    base.filter(|p| p.is_absolute())
        .map(|p| p.join("Serein/library.sqlite3"))
        .ok_or(serein_storage::StorageError::Unavailable)
}

fn prepare_library_directory(directory: &Path) -> serein_storage::Result<()> {
    std::fs::create_dir_all(directory).map_err(|_| StorageError::Unavailable)?;
    let metadata = std::fs::symlink_metadata(directory).map_err(|_| StorageError::Unavailable)?;
    if !metadata.is_dir() {
        return Err(StorageError::InvalidInput);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(StorageError::Unavailable);
        }
        // This is only the app-owned Serein directory, never an arbitrary user directory.
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| StorageError::Unavailable)?;
    }
    Ok(())
}
fn read_selected_import(path: &Path) -> serein_storage::Result<Vec<u8>> {
    let canonical = path.canonicalize().map_err(|_| StorageError::Unavailable)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options
        .open(canonical)
        .map_err(|_| StorageError::Unavailable)?;
    let metadata = file.metadata().map_err(|_| StorageError::Unavailable)?;
    if !metadata.is_file() || metadata.len() > MAX_TRANSFER_BYTES as u64 {
        return Err(StorageError::InvalidInput);
    }
    let mut bytes = Vec::new();
    file.take(MAX_TRANSFER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| StorageError::Unavailable)?;
    if bytes.len() > MAX_TRANSFER_BYTES {
        return Err(StorageError::InvalidInput);
    }
    Ok(bytes)
}
/// Commit a complete private export. No-clobber uses an atomic same-filesystem
/// link; replacing an existing file requires the caller's explicit authorization.
fn atomic_export(path: &Path, bytes: &[u8], overwrite: bool) -> serein_storage::Result<()> {
    if !path.is_absolute() || bytes.len() > MAX_TRANSFER_BYTES {
        return Err(StorageError::InvalidInput);
    }
    let parent = path
        .parent()
        .ok_or(StorageError::InvalidInput)?
        .canonicalize()
        .map_err(|_| StorageError::Unavailable)?;
    let name = path.file_name().ok_or(StorageError::InvalidInput)?;
    let destination = parent.join(name);
    if let Ok(metadata) = std::fs::symlink_metadata(&destination) {
        if !overwrite {
            return Err(StorageError::BackupExists);
        }
        if !metadata.is_file() {
            return Err(StorageError::InvalidInput);
        }
    }
    let nonce = SessionProfile::random().map_err(|_| StorageError::Unavailable)?;
    let temporary = parent.join(format!(".serein-export-{}", nonce.as_str()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| StorageError::Unavailable)?;
        file.write_all(bytes)
            .map_err(|_| StorageError::Unavailable)?;
        file.sync_all().map_err(|_| StorageError::Unavailable)?;
        drop(file);
        if overwrite {
            std::fs::rename(&temporary, &destination).map_err(|_| StorageError::Unavailable)?;
        } else {
            std::fs::hard_link(&temporary, &destination).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    StorageError::BackupExists
                } else {
                    StorageError::Unavailable
                }
            })?;
            std::fs::remove_file(&temporary).map_err(|_| StorageError::Unavailable)?;
        }
        #[cfg(unix)]
        File::open(&parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| StorageError::Unavailable)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "serein-library-synthetic-{}",
                SessionProfile::random().unwrap().as_str()
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn video(index: u32) -> VideoSummary {
        VideoSummary {
            id: VideoId::new(&format!("{index:011}")).unwrap(),
            title: format!("Synthetic fixture {index}"),
            channel: "Synthetic local channel".into(),
            channel_id: None,
            duration: Some(Duration::from_secs(90)),
            thumbnail_url: None,
        }
    }
    #[test]
    fn shutdown_finishes_accepted_writes_without_a_result_consumer() {
        let directory = TestDirectory::new();
        let path = directory.0.join("library.sqlite3");
        let worker = Worker::new(path.clone(), || {});
        let mut expected = None;
        for volume in 0..100 {
            let prefs = LocalPreferences {
                volume_percent: volume,
                ..Default::default()
            };
            if worker.submit(Request::Preferences(PreferenceWrite {
                id: u64::from(volume),
                value: prefs,
            })) {
                expected = Some(prefs);
            } else {
                break;
            }
        }
        assert!(expected.is_some());
        drop(worker);
        let store = LocalStore::open(&path).unwrap();
        assert_eq!(store.preferences().unwrap(), expected.unwrap());
    }
    #[test]
    fn worker_pages_and_mutations_preserve_local_scope() {
        let mut store = LocalStore::in_memory().unwrap();
        let id = store.create_playlist("Synthetic local playlist").unwrap();
        for i in 0..205 {
            handle(
                &mut store,
                Request::Save(VideoSave {
                    serial: u64::from(i),
                    destination: SaveDestination::Existing(id),
                    video: video(i),
                }),
            )
            .unwrap();
        }
        let Response::PageRead {
            ticket: 10,
            result: Ok(PageResult::Videos(_, first)),
        } = handle(
            &mut store,
            Request::ReadPage {
                ticket: 10,
                page: PageQuery::Videos(id, None),
            },
        )
        .unwrap()
        else {
            panic!("video page expected")
        };
        assert_eq!(first.items.len(), 100);
        handle(&mut store, Request::Remove(id, video(0).id)).unwrap();
        let Response::PageRead {
            ticket: 11,
            result: Ok(PageResult::Videos(_, second)),
        } = handle(
            &mut store,
            Request::ReadPage {
                ticket: 11,
                page: PageQuery::Videos(id, first.next),
            },
        )
        .unwrap()
        else {
            panic!("video page expected")
        };
        assert_eq!(second.items.len(), 100);
        assert_eq!(second.items[0].id.as_str(), "00000000100");
        let Response::PageRead {
            ticket: 12,
            result: Ok(PageResult::Videos(_, third)),
        } = handle(
            &mut store,
            Request::ReadPage {
                ticket: 12,
                page: PageQuery::Videos(id, second.next),
            },
        )
        .unwrap()
        else {
            panic!("video page expected")
        };
        assert_eq!(third.items.len(), 5);
        assert!(third.next.is_none());
        assert!(handle(&mut store, Request::DeleteEmpty(id)).is_err());
        handle(
            &mut store,
            Request::Rename(id, "Synthetic renamed collection".into()),
        )
        .unwrap();
        handle(&mut store, Request::Delete(id)).unwrap();
        assert!(store.playlists(None, 100).unwrap().items.is_empty());
    }
    #[test]
    fn worker_create_and_save_has_correlated_failure_and_committed_success() {
        let directory = TestDirectory::new();
        let path = directory.0.join("library.sqlite3");
        let (wake, received) = mpsc::channel();
        let worker = Worker::new(path.clone(), move || {
            let _ = wake.send(());
        });
        assert!(worker.submit(Request::Save(VideoSave {
            serial: 17,
            destination: SaveDestination::New("".into()),
            video: video(7),
        })));
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            worker.take(),
            Some(Response::VideoSaveFailed(17, _))
        ));
        assert!(worker.submit(Request::Save(VideoSave {
            serial: 18,
            destination: SaveDestination::New("Synthetic first playlist".into()),
            video: video(7),
        })));
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(worker.take(), Some(Response::VideoSaved(18))));
        drop(worker);
        let store = LocalStore::open(&path).unwrap();
        let collections = store.playlists(None, 100).unwrap();
        assert_eq!(collections.items.len(), 1);
        assert_eq!(collections.items[0].name, "Synthetic first playlist");
        assert_eq!(
            store
                .playlist_videos(collections.items[0].id, None, 100)
                .unwrap()
                .items[0]
                .id,
            video(7).id
        );
        assert_eq!(store.preferences().unwrap(), LocalPreferences::default());
    }
    #[test]
    fn import_export_and_backup_require_explicit_safe_destinations() {
        let directory = TestDirectory::new();
        let path = directory.0.join("synthetic-library.json");
        let backup = directory.0.join("synthetic-backup.sqlite3");
        let mut source = LocalStore::in_memory().unwrap();
        let id = source
            .create_playlist("Synthetic export collection")
            .unwrap();
        source.save_video(id, &video(7)).unwrap();
        assert!(matches!(
            handle(
                &mut source,
                Request::Export {
                    path: path.clone(),
                    overwrite: false
                }
            )
            .unwrap(),
            Response::Exported
        ));
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            handle(
                &mut source,
                Request::Export {
                    path: path.clone(),
                    overwrite: false
                }
            ),
            Err(StorageError::BackupExists)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut destination = LocalStore::in_memory().unwrap();
        let Response::Imported(summary) =
            handle(&mut destination, Request::Import(path.clone())).unwrap()
        else {
            panic!("import summary expected")
        };
        assert_eq!(summary.playlists_created, 1);
        assert_eq!(summary.videos_saved, 1);
        handle(&mut source, Request::Backup(backup.clone())).unwrap();
        assert!(matches!(
            handle(&mut source, Request::Backup(backup)),
            Err(StorageError::BackupExists)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        }
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 2);
        // Oversized sparse files are rejected before allocating their contents.
        File::create(&path)
            .unwrap()
            .set_len(MAX_TRANSFER_BYTES as u64 + 1)
            .unwrap();
        assert!(matches!(
            handle(&mut destination, Request::Import(path)),
            Err(StorageError::InvalidInput)
        ));
        assert_eq!(destination.playlists(None, 100).unwrap().items.len(), 1);
    }
    #[test]
    fn history_requests_honor_opt_in_and_clear_is_local_only() {
        let mut store = LocalStore::in_memory().unwrap();
        assert!(matches!(
            handle(
                &mut store,
                Request::RecordHistory(video(4), Duration::from_secs(12))
            )
            .unwrap(),
            Response::HistoryRecorded(false)
        ));
        let mut preferences = store.preferences().unwrap();
        preferences.privacy.local_history = true;
        handle(
            &mut store,
            Request::Preferences(PreferenceWrite {
                id: 1,
                value: preferences,
            }),
        )
        .unwrap();
        handle(&mut store, Request::HistoryRetention(7)).unwrap();
        assert!(matches!(
            handle(
                &mut store,
                Request::RecordHistory(video(4), Duration::from_secs(12))
            )
            .unwrap(),
            Response::HistoryRecorded(true)
        ));
        let Response::PageRead {
            ticket: 20,
            result:
                Ok(PageResult::History {
                    entries,
                    retention_days,
                    ..
                }),
        } = handle(
            &mut store,
            Request::ReadPage {
                ticket: 20,
                page: PageQuery::History(None),
            },
        )
        .unwrap()
        else {
            panic!("history page expected")
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(retention_days, 7);
        assert_eq!(entries[0].position, Duration::from_secs(12));
        let Response::Cleared(committed) = handle(&mut store, Request::ClearLocalData).unwrap()
        else {
            panic!("typed clear response required")
        };
        assert_eq!(committed, store.preferences().unwrap());
        assert_eq!(store.preferences().unwrap(), LocalPreferences::default());
        assert!(
            store
                .history(None, 100, SystemTime::now())
                .unwrap()
                .0
                .is_empty()
        );
    }
    #[test]
    fn failed_initialization_answers_clear_with_its_own_terminal_response() {
        let directory = TestDirectory::new();
        let non_directory = directory.0.join("file");
        std::fs::write(&non_directory, b"not a directory").unwrap();
        let (wake, received) = mpsc::channel();
        let worker = Worker::new(non_directory.join("library.sqlite3"), move || {
            let _ = wake.send(());
        });
        assert!(worker.submit(Request::Load));
        assert!(worker.submit(Request::ClearLocalData));
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(worker.take(), Some(Response::Error(_))));
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(worker.take(), Some(Response::ClearFailed(_))));
    }
    #[test]
    fn failed_initialization_preserves_each_page_read_ticket() {
        let directory = TestDirectory::new();
        let non_directory = directory.0.join("file");
        std::fs::write(&non_directory, b"not a directory").unwrap();
        let (wake, received) = mpsc::channel();
        let worker = Worker::new(non_directory.join("library.sqlite3"), move || {
            let _ = wake.send(());
        });
        assert!(worker.submit(Request::ReadPage {
            ticket: 41,
            page: PageQuery::Collections(None)
        }));
        assert!(worker.submit(Request::ReadPage {
            ticket: 42,
            page: PageQuery::Subscriptions(None)
        }));
        for expected in [41, 42] {
            received.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(
                matches!(worker.take(),Some(Response::PageRead { ticket,result:Err(_) }) if ticket==expected)
            );
        }
        assert!(worker.take().is_none());
    }

    #[test]
    fn home_pages_are_bounded_deduplicated_and_independently_correlated() {
        let mut store = LocalStore::in_memory().unwrap();
        for name in ["Synthetic first", "Synthetic second"] {
            let playlist = store.create_playlist(name).unwrap();
            for index in 0..105 {
                store.save_video(playlist, &video(index)).unwrap();
            }
        }
        let Response::Home {
            ticket: 41,
            result: Ok(first),
        } = handle(
            &mut store,
            Request::Home {
                ticket: 41,
                after: None,
            },
        )
        .unwrap()
        else {
            panic!("correlated Home page expected")
        };
        assert_eq!(first.items.len(), 100);
        assert_eq!(first.items[0].id, video(104).id);
        assert_eq!(first.items[99].id, video(5).id);
        let Response::Home {
            ticket: 42,
            result: Ok(second),
        } = handle(
            &mut store,
            Request::Home {
                ticket: 42,
                after: first.next,
            },
        )
        .unwrap()
        else {
            panic!("correlated Home continuation expected")
        };
        assert_eq!(second.items.len(), 5);
        assert_eq!(second.items[4].id, video(0).id);
        assert!(second.next.is_none());
        assert!(
            first
                .items
                .iter()
                .chain(&second.items)
                .all(|v| v.thumbnail_url.is_none())
        );
        // Identical serials in the independent library controller stay distinct.
        assert!(matches!(
            handle(
                &mut store,
                Request::ReadPage {
                    ticket: 42,
                    page: PageQuery::Collections(None),
                }
            )
            .unwrap(),
            Response::PageRead { ticket: 42, .. }
        ));
    }

    #[test]
    fn home_initialization_errors_retain_each_ticket_and_response_namespace() {
        let directory = TestDirectory::new();
        let non_directory = directory.0.join("file");
        std::fs::write(&non_directory, b"not a directory").unwrap();
        let (wake, received) = mpsc::channel();
        let worker = Worker::new(non_directory.join("library.sqlite3"), move || {
            let _ = wake.send(());
        });
        for ticket in [41, 42] {
            assert!(worker.submit(Request::Home {
                ticket,
                after: None
            }));
        }
        assert!(worker.submit(Request::ReadPage {
            ticket: 42,
            page: PageQuery::Collections(None)
        }));
        for expected in [41, 42] {
            received.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(
                matches!(worker.take(), Some(Response::Home { ticket, result: Err(_) }) if ticket==expected)
            );
        }
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            worker.take(),
            Some(Response::PageRead {
                ticket: 42,
                result: Err(_)
            })
        ));
        assert!(worker.take().is_none());
    }
}
