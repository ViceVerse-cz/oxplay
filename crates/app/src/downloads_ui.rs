// SPDX-License-Identifier: GPL-3.0-or-later
//! Slint adapter for explicit downloads (`downloads.rs`): the Download card
//! command and watch-page button, the Downloads page lists, and offline play
//! through the local-file path. Row models are stable and updated per row.
use crate::{
    App, DownloadRow, DownloadsUi, UiState,
    downloads::{self, Entry, JobState, JobView, Merger},
};
use oxplay_core::{QualityCeiling, VideoId, VideoSummary};
use oxplay_youtube::download::Stage;
use slint::{ComponentHandle, Model, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::Rc,
};

pub struct State {
    manager: RefCell<Option<downloads::Manager>>,
    active: Rc<VecModel<DownloadRow>>,
    completed: Rc<VecModel<DownloadRow>>,
    entries: RefCell<Vec<Entry>>,
    thumbnails: RefCell<HashMap<String, slint::Image>>,
    jobs_revision: Cell<u64>,
    entries_revision: Cell<u64>,
    merger: Cell<Merger>,
}
impl State {
    /// `manager` is `None` when downloads are unavailable in this run.
    pub fn new(manager: Option<downloads::Manager>) -> Self {
        Self {
            manager: RefCell::new(manager),
            active: Rc::new(VecModel::default()),
            completed: Rc::new(VecModel::default()),
            entries: RefCell::new(Vec::new()),
            thumbnails: RefCell::new(HashMap::new()),
            jobs_revision: Cell::new(0),
            entries_revision: Cell::new(0),
            merger: Cell::new(Merger::Unknown),
        }
    }
    /// Cancel running helpers, reap them and remove partial files.
    pub fn shutdown(&self) {
        drop(self.manager.borrow_mut().take());
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn remaining(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} s left")
    } else if seconds < 3600 {
        format!("{} min left", seconds.div_ceil(60))
    } else {
        format!("{} h {} min left", seconds / 3600, seconds % 3600 / 60)
    }
}

/// One status line for a queued or running download.
pub fn job_status(state: &JobState, merger: bool) -> (String, f32) {
    match state {
        JobState::Queued => ("Waiting to start".into(), -1.0),
        JobState::Starting => ("Starting…".into(), -1.0),
        JobState::Cancelling => ("Cancelling…".into(), -1.0),
        JobState::Running(progress) if progress.stage == Stage::Finishing => (
            if merger {
                "Merging video and audio…".into()
            } else {
                "Finishing…".into()
            },
            1.0,
        ),
        JobState::Running(progress) => {
            let mut parts = Vec::new();
            if let Some(fraction) = progress.fraction {
                parts.push(format!("{}%", (fraction * 100.0).floor() as u32));
            }
            parts.push(match progress.total {
                Some(total) => format!(
                    "{} of {}",
                    format_bytes(progress.downloaded),
                    format_bytes(total)
                ),
                None => format_bytes(progress.downloaded),
            });
            if let Some(speed) = progress.speed.filter(|s| *s >= 1.0) {
                parts.push(format!("{}/s", format_bytes(speed as u64)));
            }
            if let Some(eta) = progress.eta {
                parts.push(remaining(eta));
            }
            (parts.join(" · "), progress.fraction.unwrap_or(-1.0))
        }
    }
}

/// Size, date, quality and length of a finished download.
pub fn entry_detail(entry: &Entry) -> String {
    let mut parts = vec![
        format_bytes(entry.size_bytes),
        crate::display_format::timestamp(entry.completed_at.min(i64::MAX as u64) as i64),
    ];
    if let Some(height) = entry.height {
        parts.push(format!("{height}p"));
    }
    if let Some(seconds) = entry.duration_seconds {
        parts.push(crate::clock_text(std::time::Duration::from_secs(seconds)));
    }
    if entry.limited {
        parts.push("Limited quality (no FFmpeg)".into());
    }
    parts.join(" · ")
}

fn note(merger: Merger, notice: Option<&str>) -> String {
    let mut lines = Vec::new();
    if cfg!(not(unix)) {
        lines.push("Downloads aren't supported on this platform yet.");
    }
    if let Some(notice) = notice {
        lines.push(notice);
    }
    if merger == Merger::Missing {
        lines.push(
            "FFmpeg wasn't found, so downloads use a single file that already includes sound — on YouTube usually 360p. Your quality setting applies when FFmpeg is available.",
        );
    }
    lines.join("\n")
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Show in folder"
    }
}

/// Ask the platform file manager to show the file. The launcher exits
/// quickly; a short-lived thread reaps it.
fn reveal(path: &Path) -> bool {
    use std::process::{Command, Stdio};
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-R").arg(path);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(format!("/select,{}", path.display()));
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let mut command = Command::new("/usr/bin/xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    };
    match command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let _ = std::thread::Builder::new()
                .name("reveal-download".into())
                .spawn(move || {
                    let _ = child.wait();
                });
            true
        }
        Err(_) => false,
    }
}

fn job_row(job: &JobView, merger: bool) -> DownloadRow {
    let (status, progress) = job_status(&job.state, merger);
    DownloadRow {
        id: job.id.as_str().into(),
        title: job.title.as_str().into(),
        channel: job.channel.as_str().into(),
        status: status.into(),
        progress,
        cancelling: job.state == JobState::Cancelling,
        ..DownloadRow::default()
    }
}

fn entry_row(entry: &Entry, thumbnails: &HashMap<String, slint::Image>) -> DownloadRow {
    let thumbnail = thumbnails.get(&entry.id);
    DownloadRow {
        id: entry.id.as_str().into(),
        title: entry.title.as_str().into(),
        channel: entry.channel.as_str().into(),
        detail: entry_detail(entry).into(),
        progress: 1.0,
        thumbnail_ready: thumbnail.is_some(),
        thumbnail: thumbnail.cloned().unwrap_or_default(),
        ..DownloadRow::default()
    }
}

/// Apply the latest coordinator state. Called on the UI thread after a wake.
fn refresh(app: &App, state: &UiState) {
    let ui = &state.downloads;
    let guard = ui.manager.borrow();
    let Some(manager) = guard.as_ref() else {
        return;
    };
    manager.acknowledge();
    let update = manager.with_view(|view| {
        let jobs = (view.jobs_revision != ui.jobs_revision.get()).then(|| view.jobs.clone());
        let entries =
            (view.entries_revision != ui.entries_revision.get()).then(|| view.entries.clone());
        ui.jobs_revision.set(view.jobs_revision);
        ui.entries_revision.set(view.entries_revision);
        (
            view.loaded,
            view.read_only,
            view.merger,
            view.notice.clone(),
            jobs,
            entries,
            std::mem::take(&mut view.thumbnails),
            std::mem::take(&mut view.messages),
        )
    });
    drop(guard);
    let Some((loaded, read_only, merger, notice, jobs, entries, thumbnails, messages)) = update
    else {
        return;
    };
    let global = app.global::<DownloadsUi>();
    if global.get_loaded() != loaded {
        global.set_loaded(loaded);
    }
    if global.get_read_only() != read_only {
        global.set_read_only(read_only);
    }
    ui.merger.set(merger);
    let text = note(merger, notice.as_deref());
    if global.get_note().as_str() != text {
        global.set_note(text.into());
    }
    let merging = merger == Merger::Available;
    if let Some(jobs) = jobs {
        let same_rows = jobs.len() == ui.active.row_count()
            && jobs
                .iter()
                .zip(ui.active.iter())
                .all(|(job, row)| row.id.as_str() == job.id.as_str());
        if same_rows {
            for (index, job) in jobs.iter().enumerate() {
                let row = job_row(job, merging);
                if ui.active.row_data(index).as_ref() != Some(&row) {
                    ui.active.set_row_data(index, row);
                }
            }
        } else {
            ui.active.set_vec(
                jobs.iter()
                    .map(|job| job_row(job, merging))
                    .collect::<Vec<_>>(),
            );
        }
    }
    if !thumbnails.is_empty() {
        let mut cache = ui.thumbnails.borrow_mut();
        for (id, pixels) in thumbnails {
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
            );
            cache.insert(id, slint::Image::from_rgba8(buffer));
        }
    }
    if let Some(entries) = entries {
        let mut cache = ui.thumbnails.borrow_mut();
        cache.retain(|id, _| entries.iter().any(|entry| &entry.id == id));
        ui.completed.set_vec(
            entries
                .iter()
                .map(|entry| entry_row(entry, &cache))
                .collect::<Vec<_>>(),
        );
        *ui.entries.borrow_mut() = entries;
    } else {
        // Artwork that arrived after its row: update only those rows.
        let cache = ui.thumbnails.borrow();
        for index in 0..ui.completed.row_count() {
            let Some(row) = ui.completed.row_data(index) else {
                continue;
            };
            if !row.thumbnail_ready
                && let Some(image) = cache.get(row.id.as_str())
            {
                ui.completed.set_row_data(
                    index,
                    DownloadRow {
                        thumbnail: image.clone(),
                        thumbnail_ready: true,
                        ..row
                    },
                );
            }
        }
    }
    if let Some(message) = messages.last() {
        crate::card_menu::toast(app, state, message);
    }
}

/// Load the manifest when the Downloads page opens (once per session).
pub fn open(app: &App, state: &UiState) {
    if let Some(manager) = state.downloads.manager.borrow().as_ref() {
        manager.open();
    }
    refresh(app, state);
}

/// Queue an explicit download of a public video at the default quality
/// ceiling. Returns the short confirmation or reason to show.
pub fn enqueue(app: &App, state: &UiState, video: VideoSummary) -> &'static str {
    if !app.global::<DownloadsUi>().get_available() {
        return "Downloads aren't available right now.";
    }
    let manager = state.downloads.manager.borrow();
    let Some(manager) = manager.as_ref() else {
        return "Downloads aren't available right now.";
    };
    let max_height = QualityCeiling::from_index(app.get_default_quality_index())
        .unwrap_or_default()
        .height();
    match manager.enqueue(downloads::Job { video, max_height }) {
        Ok(()) => "Added to Downloads",
        Err(reason) => reason,
    }
}

fn entry(state: &UiState, id: &str) -> Option<Entry> {
    state
        .downloads
        .entries
        .borrow()
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
}

fn upgrade(weak: &slint::Weak<App>, state: &std::rc::Weak<UiState>) -> Option<(App, Rc<UiState>)> {
    Some((weak.upgrade()?, state.upgrade()?))
}

fn media_path(state: &UiState, id: &str) -> Option<(Entry, std::path::PathBuf)> {
    let entry = entry(state, id)?;
    let path = state
        .downloads
        .manager
        .borrow()
        .as_ref()?
        .media_path(&entry)?;
    Some((entry, path))
}

fn with_manager(state: &UiState, id: &str, run: impl FnOnce(&downloads::Manager, VideoId)) {
    if let (Ok(id), Some(manager)) = (VideoId::new(id), state.downloads.manager.borrow().as_ref()) {
        run(manager, id);
    }
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let global = app.global::<DownloadsUi>();
    global.set_active(slint::ModelRc::from(state.downloads.active.clone()));
    global.set_completed(slint::ModelRc::from(state.downloads.completed.clone()));
    global.set_reveal_label(reveal_label().into());
    global.set_available(
        cfg!(unix) && state.downloads.manager.borrow().is_some() && !app.get_native_video_child(),
    );
    let (weak, s) = (app.as_weak(), Rc::downgrade(state));
    global.on_wake(move || {
        if let Some((app, state)) = upgrade(&weak, &s) {
            refresh(&app, &state);
        }
    });
    let (weak, s) = (app.as_weak(), Rc::downgrade(state));
    global.on_download_current(move || {
        let Some((app, state)) = upgrade(&weak, &s) else {
            return;
        };
        let video = state.current_video.borrow().clone();
        let message = match video {
            Some(video)
                if app.get_remote_video()
                    && app.get_loaded()
                    && crate::account_playback::authorization(&state).is_none() =>
            {
                enqueue(&app, &state, video)
            }
            _ => "Only public videos opened without an account can be downloaded.",
        };
        crate::card_menu::toast(&app, &state, message);
    });
    let s = Rc::downgrade(state);
    global.on_cancel(move |id| {
        if let Some(state) = s.upgrade() {
            with_manager(&state, &id, |manager, id| manager.cancel(id));
        }
    });
    let s = Rc::downgrade(state);
    global.on_delete(move |id| {
        if let Some(state) = s.upgrade() {
            with_manager(&state, &id, |manager, id| manager.delete(id));
        }
    });
    let (weak, s) = (app.as_weak(), Rc::downgrade(state));
    global.on_reveal(move |id| {
        let Some((app, state)) = upgrade(&weak, &s) else {
            return;
        };
        if !media_path(&state, &id).is_some_and(|(_, path)| reveal(&path)) {
            crate::card_menu::toast(&app, &state, "Couldn't open the file manager.");
        }
    });
    let (weak, s) = (app.as_weak(), Rc::downgrade(state));
    global.on_play(move |id| {
        let Some((app, state)) = upgrade(&weak, &s) else {
            return;
        };
        let result = match media_path(&state, &id) {
            Some((entry, path)) => {
                crate::local_media_ui::open_download(&app, &state, path, entry.title, entry.channel)
            }
            None => Err("This download can't be opened."),
        };
        if let Err(message) = result {
            crate::card_menu::toast(&app, &state, message);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplay_youtube::download::Progress;

    #[test]
    fn sizes_and_statuses_read_naturally() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(45_600_000), "46 MB");
        assert_eq!(format_bytes(2_340_000_000), "2.3 GB");
        assert_eq!(job_status(&JobState::Queued, true).0, "Waiting to start");
        let running = JobState::Running(Progress {
            fraction: Some(0.426),
            downloaded: 12_300_000,
            total: Some(45_600_000),
            speed: Some(3_100_000.0),
            eta: Some(80),
            stage: Stage::Downloading,
        });
        let (status, progress) = job_status(&running, true);
        assert_eq!(status, "42% · 12 MB of 46 MB · 3.1 MB/s · 2 min left");
        assert!((progress - 0.426).abs() < 1e-6);
        let unknown = JobState::Running(Progress {
            fraction: None,
            downloaded: 5_000,
            total: None,
            speed: None,
            eta: None,
            stage: Stage::Downloading,
        });
        assert_eq!(job_status(&unknown, false), ("5.0 KB".to_owned(), -1.0));
        let finishing = JobState::Running(Progress {
            fraction: Some(1.0),
            downloaded: 1,
            total: Some(1),
            speed: None,
            eta: None,
            stage: Stage::Finishing,
        });
        assert_eq!(job_status(&finishing, true).0, "Merging video and audio…");
        assert_eq!(job_status(&finishing, false).0, "Finishing…");
    }

    #[test]
    fn finished_rows_list_size_date_quality_length_and_limits() {
        let entry = Entry {
            id: "abcdefghijk".into(),
            title: "Title".into(),
            channel: "Channel".into(),
            duration_seconds: Some(3725),
            size_bytes: 120_000_000,
            completed_at: 1_790_000_000,
            height: Some(1080),
            limited: true,
            file: "abcdefghijk.mp4".into(),
            thumbnail: None,
        };
        assert_eq!(
            entry_detail(&entry),
            "120 MB · Sep 21, 2026 · 1080p · 1:02:05 · Limited quality (no FFmpeg)"
        );
        assert!(note(Merger::Missing, None).contains("FFmpeg"));
        assert!(!note(Merger::Available, None).contains("FFmpeg"));
        assert!(note(Merger::Available, Some("Damaged list.")).contains("Damaged"));
    }
}
