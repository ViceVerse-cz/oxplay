// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit downloads of public videos. A small versioned JSON manifest in the
//! app-owned `downloads` directory indexes finished files; a bounded queue runs
//! at most two supervised helper jobs; one coordinator thread owns every
//! filesystem change. Nothing here runs until the user opens Downloads or asks
//! for a download. The Slint adapter is `downloads_ui.rs`.
use crate::resolver::SharedResolver;
use oxplay_core::{CancellationToken, OperationContext, ProviderError, VideoId, VideoSummary};
use oxplay_youtube::download::{self as runner, MEDIA_EXTENSIONS, Progress};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

pub const MANIFEST_VERSION: u32 = 1;
/// At most this many helper downloads run at once; the rest wait.
pub const MAX_ACTIVE: usize = 2;
const MAX_WAITING: usize = 50;
const MAX_ENTRIES: usize = 5000;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MANIFEST: &str = "manifest.json";
const THUMBNAILS: &str = "thumbnails";
const STAGING: &str = ".staging";
const MAX_THUMBNAIL_SOURCE: u64 = 8 * 1024 * 1024;
/// Artwork decoded for the list, newest first; older rows show a placeholder.
const LIST_THUMBNAILS: usize = 200;

/// One finished download. File names are relative to the downloads directory
/// and always `<video id>.<extension>`; nothing else is ever opened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub channel: String,
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    pub size_bytes: u64,
    /// Unix seconds when the download finished (or the file's mtime if recovered).
    pub completed_at: u64,
    /// Video height actually obtained, when the helper reported it.
    #[serde(default)]
    pub height: Option<u32>,
    /// A single progressive file was used because no merger was available.
    #[serde(default)]
    pub limited: bool,
    pub file: String,
    /// `thumbnails/<video id>.jpg`, when artwork was saved.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ManifestFile {
    version: u32,
    #[serde(default)]
    items: Vec<Entry>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Loaded {
    pub entries: Vec<Entry>,
    /// The validated list differs from the file and should be written back.
    pub rewrite: bool,
    /// Written by a newer version, or unreadable: never modify it.
    pub read_only: bool,
    pub notice: Option<&'static str>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `<id>.<ext>` with a supported container, or nothing.
fn media_file_id(name: &str) -> Option<VideoId> {
    let (stem, ext) = name.rsplit_once('.')?;
    MEDIA_EXTENSIONS
        .contains(&ext)
        .then(|| VideoId::new(stem).ok())
        .flatten()
}
fn thumbnail_name(id: &str) -> String {
    format!("{id}.jpg")
}
fn regular(path: &Path) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.is_file())
}

/// Read and validate the manifest. Missing files are dropped, unindexed media
/// files are recovered, a damaged manifest is set aside, and a newer version
/// is shown without ever being rewritten.
pub fn load(dir: &Path) -> Loaded {
    let path = dir.join(MANIFEST);
    let mut loaded = Loaded::default();
    let mut items = Vec::new();
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // First use, or the manifest was deleted: recover present files.
        }
        Err(_) => {
            loaded.read_only = true;
            loaded.notice = Some("The downloads list couldn't be read.");
            return loaded;
        }
        Ok(meta) => {
            let parsed = (meta.is_file() && meta.len() <= MAX_MANIFEST_BYTES)
                .then(|| std::fs::read(&path).ok())
                .flatten()
                .and_then(|bytes| serde_json::from_slice::<ManifestFile>(&bytes).ok());
            match parsed {
                Some(file) if file.version > MANIFEST_VERSION => {
                    loaded.read_only = true;
                    loaded.notice = Some(
                        "These downloads were saved by a newer version of Oxplay. They are shown read-only.",
                    );
                    items = file.items;
                }
                Some(file) if file.version == 0 => loaded.rewrite = true,
                Some(file) => items = file.items,
                None => {
                    // Keep the damaged file for inspection; never parse it again.
                    let _ = std::fs::rename(&path, dir.join(format!("{MANIFEST}.damaged")));
                    loaded.rewrite = true;
                    loaded.notice = Some(
                        "The downloads list was damaged. Videos still in the folder were recovered.",
                    );
                }
            }
        }
    }
    let mut seen = HashSet::new();
    for mut entry in items {
        if seen.len() >= MAX_ENTRIES {
            loaded.rewrite = true;
            break;
        }
        let valid_name = media_file_id(&entry.file).is_some_and(|id| id.as_str() == entry.id);
        let Some(meta) = valid_name
            .then(|| regular(&dir.join(&entry.file)))
            .flatten()
        else {
            // Deleted outside Oxplay, or not a name this version would write.
            loaded.rewrite = true;
            continue;
        };
        if !seen.insert(entry.id.clone()) {
            loaded.rewrite = true;
            continue;
        }
        if meta.len() != entry.size_bytes {
            entry.size_bytes = meta.len();
            loaded.rewrite = true;
        }
        let expected = thumbnail_name(&entry.id);
        if entry.thumbnail.is_some()
            && (entry.thumbnail.as_deref() != Some(expected.as_str())
                || regular(&dir.join(THUMBNAILS).join(&expected)).is_none())
        {
            entry.thumbnail = None;
            loaded.rewrite = true;
        }
        entry.title = entry
            .title
            .chars()
            .filter(|c| !c.is_control())
            .take(500)
            .collect();
        entry.channel = entry
            .channel
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        loaded.entries.push(entry);
    }
    if !loaded.read_only
        && let Ok(listing) = std::fs::read_dir(dir)
    {
        for file in listing.flatten() {
            let name = file.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = media_file_id(name) else {
                continue;
            };
            if seen.len() >= MAX_ENTRIES || seen.contains(id.as_str()) {
                continue;
            }
            let Some(meta) = regular(&file.path()) else {
                continue;
            };
            seen.insert(id.as_str().to_owned());
            let thumbnail = thumbnail_name(id.as_str());
            let thumbnail = regular(&dir.join(THUMBNAILS).join(&thumbnail)).map(|_| thumbnail);
            loaded.entries.push(Entry {
                id: id.as_str().to_owned(),
                title: format!("Recovered video {}", id.as_str()),
                channel: String::new(),
                duration_seconds: None,
                size_bytes: meta.len(),
                completed_at: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs()),
                height: None,
                limited: false,
                file: name.to_owned(),
                thumbnail,
            });
            loaded.rewrite = true;
        }
    }
    loaded
        .entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.completed_at));
    loaded
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Atomically replace `target` with `bytes` via a temporary file and rename.
fn replace_file(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(std::io::ErrorKind::InvalidInput)?;
    let temporary = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result =
        write_private(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, target));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Write the manifest atomically (temporary file, fsync, rename).
pub fn save(dir: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&ManifestFile {
        version: MANIFEST_VERSION,
        items: entries.to_vec(),
    })
    .map_err(std::io::Error::other)?;
    replace_file(&dir.join(MANIFEST), &bytes)
}

/// Create the private downloads directory and its fixed subdirectories.
pub fn prepare(dir: &Path) -> std::io::Result<()> {
    for path in [dir.to_owned(), dir.join(THUMBNAILS), dir.join(STAGING)] {
        std::fs::create_dir_all(&path)?;
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_dir() {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// What to download: the card or watch-page video and the quality ceiling.
#[derive(Clone)]
pub struct Job {
    pub video: VideoSummary,
    pub max_height: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rejected {
    Duplicate,
    Full,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Cancelled {
    Waiting,
    Active,
    Unknown,
}

/// FIFO queue with a fixed number of running slots. Pure bookkeeping.
pub struct Queue {
    waiting: VecDeque<Job>,
    active: Vec<VideoId>,
    limit: usize,
}
impl Queue {
    pub fn new(limit: usize) -> Self {
        Self {
            waiting: VecDeque::new(),
            active: Vec::new(),
            limit: limit.max(1),
        }
    }
    pub fn contains(&self, id: &VideoId) -> bool {
        self.active.contains(id) || self.waiting.iter().any(|job| &job.video.id == id)
    }
    pub fn push(&mut self, job: Job) -> Result<(), Rejected> {
        if self.contains(&job.video.id) {
            return Err(Rejected::Duplicate);
        }
        if self.waiting.len() >= MAX_WAITING {
            return Err(Rejected::Full);
        }
        self.waiting.push_back(job);
        Ok(())
    }
    /// The next job to start, if a slot is free.
    pub fn next(&mut self) -> Option<Job> {
        if self.active.len() >= self.limit {
            return None;
        }
        let job = self.waiting.pop_front()?;
        self.active.push(job.video.id.clone());
        Some(job)
    }
    pub fn finish(&mut self, id: &VideoId) -> bool {
        let before = self.active.len();
        self.active.retain(|active| active != id);
        before != self.active.len()
    }
    /// Waiting jobs are removed at once; running jobs stay until they stop.
    pub fn cancel(&mut self, id: &VideoId) -> Cancelled {
        if self.active.contains(id) {
            return Cancelled::Active;
        }
        let before = self.waiting.len();
        self.waiting.retain(|job| &job.video.id != id);
        if before != self.waiting.len() {
            Cancelled::Waiting
        } else {
            Cancelled::Unknown
        }
    }
    #[cfg(test)]
    pub fn active(&self) -> usize {
        self.active.len()
    }
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Merger {
    #[default]
    Unknown,
    Available,
    Missing,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Queued,
    Starting,
    Running(Progress),
    Cancelling,
}
#[derive(Clone, Debug)]
pub struct JobView {
    pub id: VideoId,
    pub title: String,
    pub channel: String,
    pub state: JobState,
}

/// Latest state for the UI. Progress replaces older progress; messages and
/// newly decoded artwork are drained by the UI.
#[derive(Default)]
pub struct View {
    pub loaded: bool,
    pub read_only: bool,
    pub merger: Merger,
    pub notice: Option<String>,
    pub jobs: Vec<JobView>,
    pub jobs_revision: u64,
    pub entries: Vec<Entry>,
    pub entries_revision: u64,
    pub thumbnails: Vec<(String, image::RgbaImage)>,
    pub messages: Vec<String>,
}

struct Shared {
    view: Mutex<View>,
    wake: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}
impl Shared {
    /// Change the view and wake the UI once until it acknowledges.
    fn publish(&self, change: impl FnOnce(&mut View)) {
        if let Ok(mut view) = self.view.lock() {
            change(&mut view);
        }
        if !self.wake.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }
}

pub struct Config {
    pub dir: PathBuf,
    pub resolver: SharedResolver,
    /// Candidate FFmpeg path; checked on the coordinator, never searched for.
    pub merger: Option<PathBuf>,
}

enum JobError {
    Provider(ProviderError),
    Storage,
}
enum Command {
    Open,
    Enqueue(Job),
    Cancel(VideoId),
    Delete(VideoId),
    Finished(VideoId, Result<runner::Outcome, JobError>),
    Shutdown,
}

pub struct Manager {
    shared: Arc<Shared>,
    dir: PathBuf,
    commands: Option<mpsc::Sender<Command>>,
    thread: Option<JoinHandle<()>>,
}
impl Manager {
    /// Starts only the idle coordinator thread; no filesystem access yet.
    pub fn new(config: Config, notify: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            view: Mutex::new(View::default()),
            wake: AtomicBool::new(false),
            notify: Box::new(notify),
        });
        let (commands, input) = mpsc::channel();
        let dir = config.dir.clone();
        let coordinator = Coordinator {
            dir: config.dir,
            resolver: config.resolver,
            merger_path: config.merger,
            merger: None,
            shared: shared.clone(),
            commands: commands.clone(),
            queue: Queue::new(MAX_ACTIVE),
            entries: Vec::new(),
            loaded: false,
            read_only: false,
            running: HashMap::new(),
        };
        let thread = std::thread::Builder::new()
            .name("downloads".into())
            .spawn(move || coordinator.run(input))?;
        Ok(Self {
            shared,
            dir,
            commands: Some(commands),
            thread: Some(thread),
        })
    }
    fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }
    /// Load the manifest (once) so the Downloads page can show it.
    pub fn open(&self) {
        self.send(Command::Open);
    }
    /// Queue an explicit download. Obvious duplicates are refused here so the
    /// caller can say so; the coordinator checks again.
    pub fn enqueue(&self, job: Job) -> Result<(), &'static str> {
        let view = self
            .shared
            .view
            .lock()
            .map_err(|_| "Downloads are unavailable.")?;
        if view.read_only {
            return Err("Downloads are read-only in this version of Oxplay.");
        }
        let id = job.video.id.as_str();
        if view.jobs.iter().any(|j| j.id.as_str() == id) {
            return Err("This video is already in Downloads.");
        }
        if view.entries.iter().any(|entry| entry.id == id) {
            return Err("This video is already downloaded.");
        }
        if view.jobs.len() >= MAX_ACTIVE + MAX_WAITING {
            return Err("Too many downloads are waiting. Try again later.");
        }
        drop(view);
        self.send(Command::Enqueue(job));
        Ok(())
    }
    pub fn cancel(&self, id: VideoId) {
        self.send(Command::Cancel(id));
    }
    pub fn delete(&self, id: VideoId) {
        self.send(Command::Delete(id));
    }
    /// Call on the UI thread before reading the view after a wake.
    pub fn acknowledge(&self) {
        self.shared.wake.store(false, Ordering::Release);
    }
    pub fn with_view<R>(&self, read: impl FnOnce(&mut View) -> R) -> Option<R> {
        self.shared.view.lock().ok().map(|mut view| read(&mut view))
    }
    /// Absolute path of a finished download's media file (not checked here).
    pub fn media_path(&self, entry: &Entry) -> Option<PathBuf> {
        media_file_id(&entry.file)
            .filter(|id| id.as_str() == entry.id)
            .map(|_| self.dir.join(&entry.file))
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        // Cancel running helpers, reap them and remove partial files.
        self.send(Command::Shutdown);
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Running {
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
    title: String,
    channel: String,
    job: Job,
}

struct Coordinator {
    dir: PathBuf,
    resolver: SharedResolver,
    merger_path: Option<PathBuf>,
    merger: Option<Option<PathBuf>>,
    shared: Arc<Shared>,
    commands: mpsc::Sender<Command>,
    queue: Queue,
    entries: Vec<Entry>,
    loaded: bool,
    read_only: bool,
    running: HashMap<String, Running>,
}

fn executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !path.is_absolute() || !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

fn list_thumbnail(dir: &Path, entry: &Entry) -> Option<image::RgbaImage> {
    let name = entry.thumbnail.as_deref()?;
    let bytes = read_bounded(&dir.join(THUMBNAILS).join(name), 1024 * 1024)?;
    crate::thumbnails::decode_sized(&bytes, 160, 90)
}

/// User-facing failure text; never raw helper output.
fn failure_message(error: &JobError, merger: bool) -> &'static str {
    match error {
        JobError::Storage => "Couldn't write to the downloads folder.",
        JobError::Provider(ProviderError::UnsupportedFormat) if !merger => {
            "No single file with sound is available at or below your quality setting. FFmpeg is needed for other qualities."
        }
        JobError::Provider(ProviderError::AuthenticationRequired) => {
            "YouTube asks to sign in for this video. Downloads use guest access only."
        }
        JobError::Provider(ProviderError::Unavailable) => {
            "This video can't be downloaded. It may be live, private or restricted."
        }
        JobError::Provider(ProviderError::Timeout) => "The download stalled and was stopped.",
        JobError::Provider(ProviderError::UnsupportedPlatform) => {
            "Downloads aren't supported on this platform yet."
        }
        JobError::Provider(ProviderError::RateLimited) => {
            "YouTube is limiting requests. Wait a minute before downloading again."
        }
        JobError::Provider(ProviderError::Offline) => {
            "Can't reach YouTube. Check your connection and try again."
        }
        JobError::Provider(ProviderError::HelperUnavailable) => "The yt-dlp helper is unavailable.",
        JobError::Provider(_) => "The download failed. Try again later.",
    }
}

impl Coordinator {
    fn run(mut self, input: mpsc::Receiver<Command>) {
        while let Ok(command) = input.recv() {
            match command {
                Command::Open => self.ensure_loaded(),
                Command::Enqueue(job) => {
                    self.ensure_loaded();
                    self.enqueue(job);
                    self.start_ready();
                }
                Command::Cancel(id) => self.cancel(&id),
                Command::Delete(id) => {
                    self.ensure_loaded();
                    self.delete(&id);
                }
                Command::Finished(id, result) => {
                    self.finish(&id, result);
                    self.start_ready();
                }
                Command::Shutdown => break,
            }
        }
        for running in self.running.values() {
            running.cancel.cancel();
        }
        for (_, mut running) in self.running.drain() {
            if let Some(thread) = running.thread.take() {
                let _ = thread.join();
            }
        }
        if self.loaded {
            let _ = std::fs::remove_dir_all(self.dir.join(STAGING));
        }
    }

    fn merger(&mut self) -> Option<PathBuf> {
        self.merger
            .get_or_insert_with(|| self.merger_path.clone().filter(|path| executable(path)))
            .clone()
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        // Leftovers from an interrupted run; no job is active yet.
        let _ = std::fs::remove_dir_all(self.dir.join(STAGING));
        let loaded = load(&self.dir);
        self.read_only = loaded.read_only;
        if loaded.rewrite && !loaded.read_only && self.dir.is_dir() {
            let _ = save(&self.dir, &loaded.entries);
        }
        let thumbnails: Vec<_> = loaded
            .entries
            .iter()
            .take(LIST_THUMBNAILS)
            .filter_map(|entry| Some((entry.id.clone(), list_thumbnail(&self.dir, entry)?)))
            .collect();
        self.entries = loaded.entries;
        let merger = if self.merger().is_some() {
            Merger::Available
        } else {
            Merger::Missing
        };
        let entries = self.entries.clone();
        self.shared.publish(|view| {
            view.loaded = true;
            view.read_only = loaded.read_only;
            view.merger = merger;
            view.notice = loaded.notice.map(str::to_owned);
            view.entries = entries;
            view.entries_revision += 1;
            view.thumbnails.extend(thumbnails);
        });
    }

    fn message(&self, text: String) {
        self.shared.publish(|view| view.messages.push(text));
    }

    fn enqueue(&mut self, job: Job) {
        if self.read_only {
            self.message("Downloads are read-only in this version of Oxplay.".into());
            return;
        }
        if self.entries.iter().any(|e| e.id == job.video.id.as_str()) {
            self.message("This video is already downloaded.".into());
            return;
        }
        let view = JobView {
            id: job.video.id.clone(),
            title: job.video.title.clone(),
            channel: job.video.channel.clone(),
            state: JobState::Queued,
        };
        match self.queue.push(job) {
            Ok(()) => self.shared.publish(|v| {
                v.jobs.push(view);
                v.jobs_revision += 1;
            }),
            Err(Rejected::Duplicate) => self.message("This video is already in Downloads.".into()),
            Err(Rejected::Full) => {
                self.message("Too many downloads are waiting. Try again later.".into())
            }
        }
    }

    fn set_state(&self, id: &VideoId, state: JobState) {
        let id = id.clone();
        self.shared.publish(move |view| {
            if let Some(job) = view.jobs.iter_mut().find(|job| job.id == id) {
                job.state = state;
                view.jobs_revision += 1;
            }
        });
    }

    fn start_ready(&mut self) {
        while let Some(job) = self.queue.next() {
            let id = job.video.id.clone();
            let cancel = CancellationToken::default();
            let staging = self.dir.join(STAGING).join(id.as_str());
            let merger = self.merger();
            let resolver = self.resolver.clone();
            let shared = self.shared.clone();
            let commands = self.commands.clone();
            let dir = self.dir.clone();
            let operation = OperationContext {
                request_id: 0,
                session_generation: 0,
                cancel: cancel.clone(),
            };
            let thread_id = id.clone();
            let max_height = job.max_height;
            self.set_state(&id, JobState::Starting);
            let spawned = std::thread::Builder::new()
                .name("download-job".into())
                .spawn(move || {
                    let id = thread_id;
                    let result = (|| {
                        prepare(&dir).map_err(|_| JobError::Storage)?;
                        let _ = std::fs::remove_dir_all(&staging);
                        std::fs::create_dir(&staging).map_err(|_| JobError::Storage)?;
                        let provider = resolver.get_on_worker().map_err(JobError::Provider)?;
                        provider
                            .download(
                                &runner::Request {
                                    id: &id,
                                    max_height,
                                    merger: merger.as_deref(),
                                    staging: &staging,
                                },
                                &operation,
                                &mut |progress| {
                                    shared.publish(|view| {
                                        if let Some(job) = view.jobs.iter_mut().find(|j| j.id == id)
                                            && job.state != JobState::Cancelling
                                        {
                                            job.state = JobState::Running(progress);
                                            view.jobs_revision += 1;
                                        }
                                    });
                                },
                            )
                            .map_err(JobError::Provider)
                    })();
                    let _ = commands.send(Command::Finished(id, result));
                });
            match spawned {
                Ok(thread) => {
                    self.running.insert(
                        id.as_str().to_owned(),
                        Running {
                            cancel,
                            thread: Some(thread),
                            title: job.video.title.clone(),
                            channel: job.video.channel.clone(),
                            job,
                        },
                    );
                }
                Err(_) => {
                    self.queue.finish(&id);
                    self.remove_job(&id);
                    self.message("Couldn't start the download.".into());
                }
            }
        }
    }

    fn remove_job(&self, id: &VideoId) {
        let id = id.clone();
        self.shared.publish(move |view| {
            view.jobs.retain(|job| job.id != id);
            view.jobs_revision += 1;
        });
    }

    fn cancel(&mut self, id: &VideoId) {
        match self.queue.cancel(id) {
            Cancelled::Waiting => {
                self.remove_job(id);
                self.message("Download cancelled.".into());
            }
            Cancelled::Active => {
                if let Some(running) = self.running.get(id.as_str()) {
                    running.cancel.cancel();
                }
                self.set_state(id, JobState::Cancelling);
            }
            Cancelled::Unknown => {}
        }
    }

    fn finish(&mut self, id: &VideoId, result: Result<runner::Outcome, JobError>) {
        self.queue.finish(id);
        let running = self.running.remove(id.as_str());
        let staging = self.dir.join(STAGING).join(id.as_str());
        if let Some(mut running) = running {
            if let Some(thread) = running.thread.take() {
                let _ = thread.join();
            }
            let merger = self.merger().is_some();
            match result {
                Ok(outcome) => match self.finalize(&running, outcome) {
                    Some((entry, thumbnail)) => {
                        let title = entry.title.clone();
                        self.entries.retain(|existing| existing.id != entry.id);
                        self.entries.insert(0, entry);
                        let saved = save(&self.dir, &self.entries).is_ok();
                        let entries = self.entries.clone();
                        let key = id.as_str().to_owned();
                        self.shared.publish(move |view| {
                            view.entries = entries;
                            view.entries_revision += 1;
                            if let Some(thumbnail) = thumbnail {
                                view.thumbnails.push((key, thumbnail));
                            }
                            view.messages.push(if saved {
                                format!("Downloaded “{title}”")
                            } else {
                                format!("Downloaded “{title}”, but the downloads list couldn't be saved.")
                            });
                        });
                    }
                    None => self.message(format!(
                        "Couldn't save “{}”. {}",
                        running.title,
                        failure_message(&JobError::Storage, merger)
                    )),
                },
                Err(JobError::Provider(ProviderError::Cancelled)) => {
                    self.message("Download cancelled.".into())
                }
                Err(error) => self.message(format!(
                    "Couldn't download “{}”. {}",
                    running.title,
                    failure_message(&error, merger)
                )),
            }
        }
        let _ = std::fs::remove_dir_all(&staging);
        self.remove_job(id);
    }

    /// Move the staged media into place, save normalized artwork and build the
    /// manifest entry. Runs on the coordinator, never on the UI thread.
    fn finalize(
        &mut self,
        running: &Running,
        outcome: runner::Outcome,
    ) -> Option<(Entry, Option<image::RgbaImage>)> {
        let id = running.job.video.id.as_str();
        let ext = outcome
            .media
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| MEDIA_EXTENSIONS.contains(ext))?
            .to_owned();
        let file = format!("{id}.{ext}");
        let target = self.dir.join(&file);
        std::fs::rename(&outcome.media, &target).ok()?;
        let size_bytes = regular(&target)?.len();
        let mut list_image = None;
        let thumbnail = outcome.thumbnail.as_deref().and_then(|source| {
            let bytes = read_bounded(source, MAX_THUMBNAIL_SOURCE)?;
            let pixels = crate::thumbnails::decode_sized(&bytes, 320, 180)?;
            let mut jpeg = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(pixels.clone())
                .to_rgb8()
                .write_to(&mut jpeg, image::ImageFormat::Jpeg)
                .ok()?;
            let name = thumbnail_name(id);
            replace_file(&self.dir.join(THUMBNAILS).join(&name), jpeg.get_ref()).ok()?;
            list_image = Some(
                image::DynamicImage::ImageRgba8(pixels)
                    .thumbnail(160, 90)
                    .into_rgba8(),
            );
            Some(name)
        });
        let metadata = outcome.metadata;
        let video = &running.job.video;
        Some((
            Entry {
                id: id.to_owned(),
                title: metadata.title.unwrap_or_else(|| running.title.clone()),
                channel: metadata.channel.unwrap_or_else(|| running.channel.clone()),
                duration_seconds: metadata
                    .duration
                    .or(video.duration)
                    .map(|duration| duration.as_secs()),
                size_bytes,
                completed_at: now(),
                height: metadata.height,
                limited: !outcome.merged && self.merger().is_none(),
                file,
                thumbnail,
            },
            list_image,
        ))
    }

    fn delete(&mut self, id: &VideoId) {
        if self.read_only {
            self.message("Downloads are read-only in this version of Oxplay.".into());
            return;
        }
        let Some(index) = self.entries.iter().position(|e| e.id == id.as_str()) else {
            return;
        };
        let entry = self.entries.remove(index);
        let removed = match std::fs::remove_file(self.dir.join(&entry.file)) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
        if !removed {
            self.entries.insert(index, entry);
            self.message("Couldn't delete the video file.".into());
            return;
        }
        let _ = std::fs::remove_file(self.dir.join(THUMBNAILS).join(thumbnail_name(id.as_str())));
        let saved = save(&self.dir, &self.entries).is_ok();
        let entries = self.entries.clone();
        self.shared.publish(move |view| {
            view.entries = entries;
            view.entries_revision += 1;
            view.messages.push(if saved {
                "Download deleted.".into()
            } else {
                "Download deleted, but the downloads list couldn't be saved.".into()
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct Temp(PathBuf);
    impl Temp {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "oxplay-downloads-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn entry(id: &str, ext: &str, at: u64) -> Entry {
        Entry {
            id: id.into(),
            title: format!("Title {id}"),
            channel: "Channel".into(),
            duration_seconds: Some(61),
            size_bytes: 5,
            completed_at: at,
            height: Some(720),
            limited: false,
            file: format!("{id}.{ext}"),
            thumbnail: None,
        }
    }

    #[test]
    fn manifest_round_trips_atomically_with_private_permissions() {
        let temp = Temp::new("roundtrip");
        prepare(&temp.0).unwrap();
        std::fs::write(temp.0.join("aaaaaaaaaaa.mp4"), b"media").unwrap();
        std::fs::write(temp.0.join("bbbbbbbbbbb.mkv"), b"media").unwrap();
        std::fs::write(temp.0.join(THUMBNAILS).join("bbbbbbbbbbb.jpg"), b"art").unwrap();
        let mut second = entry("bbbbbbbbbbb", "mkv", 20);
        second.thumbnail = Some("bbbbbbbbbbb.jpg".into());
        let entries = vec![second, entry("aaaaaaaaaaa", "mp4", 10)];
        save(&temp.0, &entries).unwrap();
        let loaded = load(&temp.0);
        assert_eq!(loaded.entries, entries);
        assert!(!loaded.rewrite && !loaded.read_only && loaded.notice.is_none());
        let text = std::fs::read_to_string(temp.0.join(MANIFEST)).unwrap();
        assert!(text.contains("\"version\": 1"));
        // No temporary file is left behind after the rename.
        let leftovers: Vec<_> = std::fs::read_dir(&temp.0)
            .unwrap()
            .flatten()
            .filter(|f| f.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&temp.0.join(MANIFEST)), 0o600);
            assert_eq!(mode(&temp.0), 0o700);
        }
    }

    #[test]
    fn missing_files_are_dropped_and_unindexed_media_is_recovered() {
        let temp = Temp::new("recover");
        prepare(&temp.0).unwrap();
        std::fs::write(temp.0.join("aaaaaaaaaaa.mp4"), b"media!").unwrap();
        std::fs::write(temp.0.join("ccccccccccc.webm"), b"orphan").unwrap();
        // Not ours: wrong names, unsupported types, directories.
        std::fs::write(temp.0.join("notes.txt"), b"x").unwrap();
        std::fs::write(temp.0.join("short.mp4"), b"x").unwrap();
        std::fs::create_dir(temp.0.join("ddddddddddd.mp4")).unwrap();
        let mut stale_art = entry("aaaaaaaaaaa", "mp4", 30);
        stale_art.thumbnail = Some("aaaaaaaaaaa.jpg".into());
        let mut traversal = entry("eeeeeeeeeee", "mp4", 40);
        traversal.file = "../eeeeeeeeeee.mp4".into();
        save(
            &temp.0,
            &[
                stale_art,
                entry("bbbbbbbbbbb", "mp4", 20),
                traversal,
                entry("aaaaaaaaaaa", "mp4", 5),
            ],
        )
        .unwrap();
        let loaded = load(&temp.0);
        assert!(loaded.rewrite);
        let ids: Vec<_> = loaded.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"aaaaaaaaaaa") && ids.contains(&"ccccccccccc"));
        let kept = loaded
            .entries
            .iter()
            .find(|e| e.id == "aaaaaaaaaaa")
            .unwrap();
        // Size refreshed from disk, missing artwork forgotten.
        assert_eq!(kept.size_bytes, 6);
        assert_eq!(kept.thumbnail, None);
        let recovered = loaded
            .entries
            .iter()
            .find(|e| e.id == "ccccccccccc")
            .unwrap();
        assert!(recovered.title.contains("ccccccccccc"));
        assert_eq!(recovered.file, "ccccccccccc.webm");
        // Writing back makes the next load clean.
        save(&temp.0, &loaded.entries).unwrap();
        assert!(!load(&temp.0).rewrite);
    }

    #[test]
    fn damaged_manifest_is_set_aside_and_newer_versions_stay_read_only() {
        let temp = Temp::new("damaged");
        prepare(&temp.0).unwrap();
        std::fs::write(temp.0.join("aaaaaaaaaaa.mp4"), b"media").unwrap();
        std::fs::write(temp.0.join(MANIFEST), b"{\"version\": 1, \"items\": [tru").unwrap();
        let loaded = load(&temp.0);
        assert!(loaded.rewrite && !loaded.read_only);
        assert!(loaded.notice.unwrap().contains("damaged"));
        assert_eq!(loaded.entries.len(), 1);
        assert!(temp.0.join(format!("{MANIFEST}.damaged")).exists());

        let newer =
            br#"{"version": 99, "items": [{"id": "aaaaaaaaaaa", "title": "T", "channel": "C",
            "size_bytes": 5, "completed_at": 1, "file": "aaaaaaaaaaa.mp4", "future": true}]}"#;
        std::fs::write(temp.0.join(MANIFEST), newer).unwrap();
        std::fs::write(temp.0.join("bbbbbbbbbbb.mp4"), b"orphan").unwrap();
        let loaded = load(&temp.0);
        assert!(loaded.read_only);
        assert!(loaded.notice.unwrap().contains("newer"));
        // Shown, but orphans are not added and the file is never rewritten.
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(std::fs::read(temp.0.join(MANIFEST)).unwrap(), newer);

        // No directory at all: an empty, writable list without errors.
        let missing = load(&temp.0.join("absent"));
        assert_eq!(missing, Loaded::default());
    }

    fn job(id: &str) -> Job {
        Job {
            video: VideoSummary {
                id: VideoId::new(id).unwrap(),
                title: format!("Title {id}"),
                channel: "Channel".into(),
                channel_id: None,
                duration: Some(Duration::from_secs(61)),
                thumbnail_url: None,
                metadata: None,
            },
            max_height: 1080,
        }
    }

    #[test]
    fn queue_runs_two_at_a_time_in_order_and_cancels_waiting_jobs() {
        let mut queue = Queue::new(MAX_ACTIVE);
        for id in ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc", "ddddddddddd"] {
            queue.push(job(id)).unwrap();
        }
        assert_eq!(queue.push(job("aaaaaaaaaaa")), Err(Rejected::Duplicate));
        let first = queue.next().unwrap();
        let second = queue.next().unwrap();
        assert_eq!(first.video.id.as_str(), "aaaaaaaaaaa");
        assert_eq!(second.video.id.as_str(), "bbbbbbbbbbb");
        assert!(queue.next().is_none(), "a third job must wait");
        assert_eq!((queue.active(), queue.waiting()), (2, 2));
        // Active jobs are cancelled by their token and stay until they stop.
        assert_eq!(queue.cancel(&first.video.id), Cancelled::Active);
        assert_eq!(queue.push(job("aaaaaaaaaaa")), Err(Rejected::Duplicate));
        let third = VideoId::new("ccccccccccc").unwrap();
        assert_eq!(queue.cancel(&third), Cancelled::Waiting);
        assert_eq!(queue.cancel(&third), Cancelled::Unknown);
        assert!(queue.finish(&first.video.id));
        assert!(!queue.finish(&first.video.id));
        assert_eq!(queue.next().unwrap().video.id.as_str(), "ddddddddddd");
        assert!(queue.next().is_none());
        for index in 0..MAX_WAITING {
            queue.push(job(&format!("w{index:010}"))).unwrap();
        }
        assert_eq!(queue.push(job("zzzzzzzzzzz")), Err(Rejected::Full));
    }

    #[test]
    fn failures_are_explained_without_helper_output() {
        let text = failure_message(&JobError::Provider(ProviderError::UnsupportedFormat), false);
        assert!(text.contains("FFmpeg"));
        assert!(
            !failure_message(&JobError::Provider(ProviderError::UnsupportedFormat), true)
                .contains("FFmpeg")
        );
        assert!(
            failure_message(
                &JobError::Provider(ProviderError::AuthenticationRequired),
                true
            )
            .contains("guest")
        );
    }

    /// The manager with a synthetic helper: bounded concurrency, queue and
    /// active cancellation, finalization and deletion. No network access.
    #[cfg(unix)]
    #[test]
    fn manager_limits_concurrency_cancels_and_finalizes_with_a_synthetic_helper() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new("manager");
        let helper = temp.0.join("yt-dlp");
        let gate = temp.0.join("release");
        let started = temp.0.join("started");
        std::fs::create_dir(&started).unwrap();
        std::fs::write(
            &helper,
            format!(
                r#"#!/bin/sh
while [ $# -gt 0 ]; do
  case "$1" in --paths) dir="$2";; https://*) url="$1";; esac
  shift
done
id="${{url##*=}}"
: > "{started}/$id"
echo "OXPPLAN 18 10"
echo "OXPPROG downloading 5 10 NA 5.0 1 18"
while [ ! -e "{gate}" ]; do sleep 0.02; done
printf 'media' > "$dir/$id.mp4"
echo "OXPPROG finished 10 10 NA NA NA 18"
echo "OXPDONE {{\"id\": \"$id\", \"title\": \"Done $id\", \"height\": 360, \"ext\": \"mp4\"}}"
"#,
                started = started.display(),
                gate = gate.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir = temp.0.join("downloads");
        let (wake, woken) = mpsc::channel();
        let manager = Manager::new(
            Config {
                dir: dir.clone(),
                resolver: SharedResolver::new(helper, temp.0.join("unused-deno")),
                merger: Some(temp.0.join("missing-ffmpeg")),
            },
            move || {
                let _ = wake.send(());
            },
        )
        .unwrap();
        let wait_for = |what: &str, done: &dyn Fn(&View) -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                manager.acknowledge();
                if manager.with_view(|view| done(view)).unwrap() {
                    return;
                }
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                let _ = woken.recv_timeout(left.min(Duration::from_millis(100)));
            }
        };
        manager.open();
        wait_for("load", &|view| view.loaded);
        assert_eq!(manager.with_view(|v| v.merger).unwrap(), Merger::Missing);
        // Opening alone never creates the directory.
        assert!(!dir.exists());
        for id in ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"] {
            manager.enqueue(job(id)).unwrap();
        }
        wait_for("two running", &|view| {
            view.jobs.len() == 3
                && view
                    .jobs
                    .iter()
                    .filter(|j| matches!(j.state, JobState::Running(_)))
                    .count()
                    == 2
        });
        assert_eq!(
            manager.with_view(|v| v.jobs[2].state.clone()).unwrap(),
            JobState::Queued
        );
        assert_eq!(std::fs::read_dir(&started).unwrap().count(), 2);
        assert_eq!(
            manager.enqueue(job("aaaaaaaaaaa")),
            Err("This video is already in Downloads.")
        );
        // Cancel the waiting job and one running job.
        manager.cancel(VideoId::new("ccccccccccc").unwrap());
        manager.cancel(VideoId::new("aaaaaaaaaaa").unwrap());
        wait_for("cancellations", &|view| {
            view.jobs.len() == 1 && view.jobs[0].id.as_str() == "bbbbbbbbbbb"
        });
        assert!(
            !started.join("ccccccccccc").exists(),
            "queued job never ran"
        );
        assert!(!dir.join(STAGING).join("aaaaaaaaaaa").exists());
        std::fs::write(&gate, b"").unwrap();
        wait_for("completion", &|view| {
            view.jobs.is_empty() && view.entries.len() == 1
        });
        let entry = manager.with_view(|v| v.entries[0].clone()).unwrap();
        assert_eq!(entry.title, "Done bbbbbbbbbbb");
        assert_eq!(entry.height, Some(360));
        assert!(entry.limited, "no merger was available");
        let media = manager.media_path(&entry).unwrap();
        assert_eq!(std::fs::read(&media).unwrap(), b"media");
        assert_eq!(load(&dir).entries, vec![entry.clone()]);
        let messages = manager
            .with_view(|v| std::mem::take(&mut v.messages))
            .unwrap();
        assert!(messages.iter().any(|m| m == "Download cancelled."));
        assert!(messages.iter().any(|m| m.starts_with("Downloaded")));
        assert_eq!(
            manager.enqueue(job("bbbbbbbbbbb")),
            Err("This video is already downloaded.")
        );
        manager.delete(VideoId::new("bbbbbbbbbbb").unwrap());
        wait_for("deletion", &|view| view.entries.is_empty());
        assert!(!media.exists());
        assert!(load(&dir).entries.is_empty());
        drop(manager);
        assert!(!dir.join(STAGING).exists());
    }
}
