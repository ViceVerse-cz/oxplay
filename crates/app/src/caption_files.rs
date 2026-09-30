// SPDX-License-Identifier: GPL-3.0-or-later
//! Private caption files. Creation runs on the catalog worker; the final lease
//! queues deletion by notification only. Filesystem cleanup never runs on Slint.
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[cfg(unix)]
mod private;
#[cfg(unix)]
use private::Directory;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Directory;
#[cfg(not(any(unix, windows)))]
struct Directory;
#[cfg(not(any(unix, windows)))]
impl Directory {
    fn create(_: PathBuf) -> Result<Self, &'static str> {
        Err("Protected caption files are not validated on this platform.")
    }
    fn create_file(&self, _: &[u8]) -> Result<CreatedFile, &'static str> {
        Err("Protected caption files are not validated on this platform.")
    }
    fn remove_file(&self, _: &Path) -> Result<(), &'static str> {
        Err("Protected caption files are not validated on this platform.")
    }
    fn purge_stale(&self) -> Result<(), &'static str> {
        Err("Protected caption files are not validated on this platform.")
    }
    fn finish(&self) -> Result<(), &'static str> {
        Ok(())
    }
}
const MAX_FILES: usize = 8;
const MAX_BYTES: usize = 2 * 1024 * 1024;
struct CreatedFile {
    path: PathBuf,
    error: Option<&'static str>,
}
struct Record {
    path: PathBuf,
    released: AtomicBool,
    deletion_attempted: AtomicBool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PurgeId(u64);
struct Purge {
    id: PurgeId,
    running: bool,
    result: Option<Result<(), &'static str>>,
}
type Wake = Arc<Mutex<Box<dyn Fn() + Send>>>;
struct State {
    ready: Option<Result<Arc<Directory>, &'static str>>,
    files: Vec<Arc<Record>>,
    creating: usize,
    epoch: u64,
    purge: Option<Purge>,
    callback: Option<Wake>,
    stop: bool,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}
#[derive(Clone)]
pub struct Client(Arc<Shared>);
pub struct Lease {
    record: Arc<Record>,
    shared: Arc<Shared>,
}
impl Lease {
    pub fn path(&self) -> &Path {
        &self.record.path
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // The worker never holds this lock while doing filesystem I/O. Holding
        // it for the state transition prevents a lost Condvar notification.
        let _state = self.shared.state.lock().unwrap();
        self.record.released.store(true, Ordering::Release);
        self.shared.wake.notify_one();
    }
}
pub struct Worker {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<Result<(), &'static str>>>,
}
impl Worker {
    /// Declare this owner before App/Player and finish only after they are
    /// destroyed. Media can retain leases until its command replies/termination.
    pub fn new(cache_root: PathBuf) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                ready: None,
                files: Vec::new(),
                creating: 0,
                epoch: 0,
                purge: None,
                callback: None,
                stop: false,
            }),
            wake: Condvar::new(),
        });
        let work = shared.clone();
        let thread = std::thread::Builder::new()
            .name("caption-cleanup".into())
            .spawn(move || cleanup(work, cache_root))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
    pub fn client(&self) -> Client {
        Client(self.shared.clone())
    }
    pub fn finish(mut self) -> Result<(), &'static str> {
        self.stop()
    }
    fn stop(&mut self) -> Result<(), &'static str> {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.stop = true;
            self.shared.wake.notify_one();
        }
        self.thread
            .take()
            .map(|thread| {
                thread
                    .join()
                    .unwrap_or(Err("caption cleanup worker failed"))
            })
            .unwrap_or(Ok(()))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.stop().is_err() {
            eprintln!("private caption cleanup failed");
        }
    }
}
impl Client {
    pub fn on_purge_complete(&self, wake: impl Fn() + Send + 'static) {
        self.0.state.lock().unwrap().callback = Some(Arc::new(Mutex::new(Box::new(wake))));
    }
    /// UI-thread admission barrier only: no filesystem operation or wait.
    pub fn begin_purge(&self) -> Result<PurgeId, &'static str> {
        let mut state = self.0.state.lock().unwrap();
        if state.stop {
            return Err("Caption file worker stopped.");
        }
        if state.purge.is_some() {
            return Err("Caption cache clearing is already pending.");
        }
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or("Caption operation identifiers exhausted.")?;
        let id = PurgeId(state.epoch);
        state.purge = Some(Purge {
            id,
            running: false,
            result: None,
        });
        // A previous filesystem failure receives one explicit retry, not a loop.
        for record in &state.files {
            record.deletion_attempted.store(false, Ordering::Release);
        }
        self.0.wake.notify_all();
        Ok(id)
    }
    pub fn purge_result(&self, id: PurgeId) -> Option<Result<(), &'static str>> {
        self.0
            .state
            .lock()
            .unwrap()
            .purge
            .as_ref()
            .filter(|purge| purge.id == id)?
            .result
    }
    /// End success/failure/timeout explicitly. Epoch invalidation still rejects
    /// creators that started before this purge, even after admission resumes.
    pub fn end_purge(&self, id: PurgeId) {
        let mut state = self.0.state.lock().unwrap();
        if state.purge.as_ref().is_some_and(|purge| purge.id == id) {
            state.purge = None;
        }
        self.0.wake.notify_all();
    }
    /// Worker-thread only: reserves one of eight slots before any file write.
    pub fn create(&self, bytes: &[u8]) -> Result<Arc<Lease>, &'static str> {
        let (directory, epoch) = self.reserve(bytes.len())?;
        let created = directory.create_file(bytes);
        self.publish(epoch, created)
    }
    fn reserve(&self, length: usize) -> Result<(Arc<Directory>, u64), &'static str> {
        if length > MAX_BYTES {
            return Err("Caption exceeded the file size limit.");
        }
        let mut state = self.0.state.lock().unwrap();
        while state.ready.is_none() && !state.stop {
            state = self.0.wake.wait(state).unwrap();
        }
        if state.stop {
            return Err("Caption file worker stopped.");
        }
        if state.purge.is_some() {
            return Err("Caption cache clearing is pending.");
        }
        if state.files.len() + state.creating >= MAX_FILES {
            return Err(
                "Eight caption tracks are already cached for this playback. Open the video again to clear them.",
            );
        }
        let directory = state
            .ready
            .as_ref()
            .ok_or("Caption directory unavailable.")?
            .clone()?;
        state.creating += 1;
        Ok((directory, state.epoch))
    }
    fn publish(
        &self,
        epoch: u64,
        created: Result<CreatedFile, &'static str>,
    ) -> Result<Arc<Lease>, &'static str> {
        let mut state = self.0.state.lock().unwrap();
        state.creating -= 1;
        let stale = state.stop || epoch != state.epoch;
        let result = created.and_then(|created| {
            let record = Arc::new(Record {
                path: created.path,
                released: AtomicBool::new(stale || created.error.is_some()),
                deletion_attempted: AtomicBool::new(false),
            });
            state.files.push(record.clone());
            if stale {
                Err("Caption creation cancelled by local-data clearing.")
            } else if let Some(error) = created.error {
                Err(error)
            } else {
                Ok(Arc::new(Lease {
                    record,
                    shared: self.0.clone(),
                }))
            }
        });
        self.0.wake.notify_all();
        result
    }
}

fn cleanup(shared: Arc<Shared>, cache_root: PathBuf) -> Result<(), &'static str> {
    let directory = Directory::create(cache_root).map(Arc::new);
    {
        shared.state.lock().unwrap().ready = Some(directory.clone());
        shared.wake.notify_all();
    }
    let failed;
    loop {
        let (released, purge, finished) = {
            let mut state = shared.state.lock().unwrap();
            loop {
                let quiescent = state.creating == 0
                    && state.files.iter().all(|record| {
                        record.released.load(Ordering::Acquire)
                            && record.deletion_attempted.load(Ordering::Acquire)
                    });
                let purge_ready = state
                    .purge
                    .as_ref()
                    .is_some_and(|purge| !purge.running && purge.result.is_none())
                    && quiescent;
                if state.files.iter().any(|record| {
                    record.released.load(Ordering::Acquire)
                        && !record.deletion_attempted.load(Ordering::Acquire)
                }) || purge_ready
                    || (state.stop && quiescent)
                {
                    break;
                }
                state = shared.wake.wait(state).unwrap();
            }
            let released: Vec<_> = state
                .files
                .iter()
                .filter(|record| {
                    record.released.load(Ordering::Acquire)
                        && !record.deletion_attempted.swap(true, Ordering::AcqRel)
                })
                .cloned()
                .collect();
            let quiescent = released.is_empty()
                && state.creating == 0
                && state
                    .files
                    .iter()
                    .all(|record| record.released.load(Ordering::Acquire));
            let purge = if quiescent {
                let failed_files = !state.files.is_empty();
                state
                    .purge
                    .as_mut()
                    .filter(|purge| !purge.running && purge.result.is_none())
                    .map(|purge| {
                        purge.running = true;
                        (purge.id, failed_files)
                    })
            } else {
                None
            };
            (released, purge, state.stop && quiescent)
        };
        for record in released {
            if directory
                .as_ref()
                .is_ok_and(|directory| directory.remove_file(&record.path).is_ok())
            {
                let mut state = shared.state.lock().unwrap();
                state
                    .files
                    .retain(|existing| !Arc::ptr_eq(existing, &record));
            }
        }
        if let Some((id, failed_files)) = purge {
            let result = if failed_files {
                Err("Some private caption files could not be deleted.")
            } else {
                directory
                    .as_ref()
                    .map_err(|error| *error)
                    .and_then(|directory| directory.purge_stale())
            };
            let callback = {
                let mut state = shared.state.lock().unwrap();
                if let Some(purge) = state.purge.as_mut().filter(|purge| purge.id == id) {
                    purge.result = Some(result);
                    state.callback.clone()
                } else {
                    None
                }
            };
            shared.wake.notify_all();
            if let Some(callback) = callback {
                callback.lock().unwrap()();
            }
        }
        if finished {
            failed = !shared.state.lock().unwrap().files.is_empty();
            break;
        }
    }
    let finish_failed = directory
        .as_ref()
        .is_ok_and(|directory| directory.finish().is_err());
    if failed || finish_failed {
        Err("Private caption cleanup was incomplete.")
    } else {
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn setup() -> (PathBuf, Worker, Client) {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!(
            "oxplay-caption-purge-{}",
            private::random_name().unwrap()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let worker = Worker::new(path.join("captions")).unwrap();
        let client = worker.client();
        (path, worker, client)
    }
    fn result(client: &Client, id: PurgeId) -> Result<(), &'static str> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut state = client.0.state.lock().unwrap();
        loop {
            if let Some(result) = state
                .purge
                .as_ref()
                .filter(|p| p.id == id)
                .and_then(|p| p.result)
            {
                return result;
            }
            assert!(std::time::Instant::now() < deadline, "purge did not finish");
            state = client
                .0
                .wake
                .wait_timeout(state, std::time::Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }
    #[test]
    fn purge_waits_for_final_media_lease_and_keeps_admission_closed_until_acknowledged() {
        let (profile, worker, client) = setup();
        let lease = client.create(b"WEBVTT\n\n").unwrap();
        let path = lease.path().to_owned();
        let media = lease.clone();
        let id = client.begin_purge().unwrap();
        drop(lease);
        assert!(client.create(b"blocked").is_err());
        assert_eq!(client.purge_result(id), None);
        assert!(path.exists());
        drop(media);
        assert_eq!(result(&client, id), Ok(()));
        assert!(!path.exists());
        assert!(client.create(b"still blocked").is_err());
        client.end_purge(id);
        let next = client.create(b"WEBVTT\n\n").unwrap();
        drop(next);
        worker.finish().unwrap();
        std::fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn timed_out_purge_cannot_publish_old_creation_or_end_new_barrier() {
        let (profile, worker, client) = setup();
        let (directory, epoch) = client.reserve(8).unwrap();
        let created = directory.create_file(b"WEBVTT\n\n").unwrap();
        let path = created.path.clone();
        let first = client.begin_purge().unwrap();
        assert_eq!(client.purge_result(first), None);
        client.end_purge(first); // timeout does not restore old creator authority
        let second = client.begin_purge().unwrap();
        client.end_purge(first);
        assert!(client.create(b"blocked").is_err());
        assert!(client.publish(epoch, Ok(created)).is_err());
        assert_eq!(result(&client, second), Ok(()));
        assert!(!path.exists());
        client.end_purge(second);
        worker.finish().unwrap();
        std::fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn partial_write_with_failed_deletion_stays_tracked_and_requires_explicit_retry() {
        use std::io::Write;
        let (profile, worker, client) = setup();
        let (directory, epoch) = client.reserve(8).unwrap();
        let created = directory
            .create_file_with(b"WEBVTT\n\n", |file, bytes| {
                file.write_all(&bytes[..3])?;
                Err(std::io::Error::other("injected partial write failure"))
            })
            .unwrap();
        let path = created.path.clone();
        let parent = path.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500)).unwrap();
        let id = client.begin_purge().unwrap();
        assert!(client.publish(epoch, Ok(created)).is_err());
        assert!(result(&client, id).is_err());
        assert!(path.exists());
        assert_eq!(client.0.state.lock().unwrap().files.len(), 1);
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        client.end_purge(id);
        let retry = client.begin_purge().unwrap();
        assert_eq!(result(&client, retry), Ok(()));
        assert!(!path.exists());
        client.end_purge(retry);
        worker.finish().unwrap();
        std::fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn already_absent_known_file_is_successfully_cleared() {
        let (profile, worker, client) = setup();
        let lease = client.create(b"WEBVTT\n\n").unwrap();
        std::fs::remove_file(lease.path()).unwrap();
        let id = client.begin_purge().unwrap();
        drop(lease);
        assert_eq!(result(&client, id), Ok(()));
        client.end_purge(id);
        worker.finish().unwrap();
        std::fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn final_lease_controls_private_file_cleanup_and_capacity() {
        use std::os::unix::fs::DirBuilderExt;
        let profile = std::env::temp_dir().join(format!(
            "oxplay-caption-worker-test-{}",
            private::random_name().unwrap()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&profile)
            .unwrap();
        let worker = Worker::new(profile.join("captions")).unwrap();
        let client = worker.client();
        let mut leases = Vec::new();
        for _ in 0..8 {
            leases.push(client.create(b"WEBVTT\n\n").unwrap());
        }
        assert!(client.create(b"extra").is_err());
        assert!(client.create(&vec![b'x'; MAX_BYTES + 1]).is_err());
        let media_lease = leases[0].clone();
        let path = media_lease.path().to_owned();
        let directory = path.parent().unwrap().to_owned();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        drop(leases);
        assert!(
            path.exists(),
            "pending media lease must retain caption file"
        );
        drop(media_lease);
        worker.finish().unwrap();
        assert!(!path.exists());
        assert!(!directory.exists());
        assert!(client.create(b"after shutdown").is_err());
        std::fs::remove_dir_all(profile).unwrap();
    }
}
