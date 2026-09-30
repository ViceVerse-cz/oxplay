// SPDX-License-Identifier: GPL-3.0-or-later
//! Guest caption selection with bounded playback-scoped file leases.
use crate::{
    App, CaptionsUi, UiState, caption_files,
    catalog::{CaptionError, Request},
};
use oxplay_core::{ResolvedPlayback, SubtitleTrack, VideoId};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
};
type Key = (String, bool);
fn key(track: &SubtitleTrack) -> Key {
    (track.language.clone(), track.automatic)
}
struct Cached {
    lease: Arc<caption_files::Lease>,
    track: SubtitleTrack,
}
pub struct State {
    files: caption_files::Client,
    labels: Rc<slint::VecModel<slint::SharedString>>,
    video: RefCell<Option<VideoId>>,
    tracks: RefCell<Vec<SubtitleTrack>>,
    cache: RefCell<HashMap<Key, Cached>>,
    retired: RefCell<Vec<Arc<caption_files::Lease>>>,
    retire_after: Cell<Option<u64>>,
    pending: Cell<Option<(u64, usize)>>,
    desired: RefCell<Option<Key>>,
    selected: RefCell<Option<Key>>,
    awaiting: RefCell<Option<(Key, u64)>>,
    awaiting_off: Cell<Option<(u64, u64)>>,
    reload_after: Cell<Option<u64>>,
    load_baseline: Cell<(u64, u64)>,
}
impl State {
    pub fn new(files: caption_files::Client) -> Self {
        Self {
            files,
            labels: Rc::new(slint::VecModel::from(vec!["Off".into()])),
            video: RefCell::new(None),
            tracks: RefCell::new(Vec::new()),
            cache: RefCell::new(HashMap::new()),
            retired: RefCell::new(Vec::new()),
            retire_after: Cell::new(None),
            pending: Cell::new(None),
            desired: RefCell::new(None),
            selected: RefCell::new(None),
            awaiting: RefCell::new(None),
            awaiting_off: Cell::new(None),
            reload_after: Cell::new(None),
            load_baseline: Cell::new((0, 0)),
        }
    }
    fn selected_index(&self) -> i32 {
        self.selected
            .borrow()
            .as_ref()
            .and_then(|selected| {
                self.tracks
                    .borrow()
                    .iter()
                    .position(|track| key(track) == *selected)
            })
            .map_or(0, |index| index as i32 + 1)
    }
}
pub fn metadata(app: &App, state: &UiState, item: &ResolvedPlayback, quality: bool) {
    let ui = app.global::<CaptionsUi>();
    let s = &state.caption_ui;
    let same = s.video.borrow().as_ref() == Some(&item.video.id);
    // No native events are drained between the accepted load command and this
    // UI-thread metadata handoff, so these still describe the previous file.
    let snapshot = state.player.snapshot();
    s.load_baseline
        .set((snapshot.file_starts, snapshot.load_request_id));
    s.pending.set(None);
    s.awaiting.borrow_mut().take();
    s.awaiting_off.set(None);
    // A replacement has no acknowledged caption selection yet. Preserve the
    // desired language for reattachment, but publish it only after observation.
    s.selected.borrow_mut().take();
    if !quality || !same {
        s.retired
            .borrow_mut()
            .extend(s.cache.borrow_mut().drain().map(|(_, cached)| cached.lease));
        if !s.retired.borrow().is_empty() {
            s.retire_after.set(Some(state.player.snapshot().file_loads));
        }
        s.desired.borrow_mut().take();
    }
    // Resolution precedes the native FILE_LOADED event. Prevent sub-add from
    // targeting the previous file while the replacement is still opening.
    s.reload_after.set(Some(state.player.snapshot().file_loads));
    *s.video.borrow_mut() = Some(item.video.id.clone());
    let mut tracks = item.subtitles.clone();
    if let Some(desired) = s.desired.borrow().as_ref()
        && !tracks.iter().any(|track| key(track) == *desired)
        && let Some(cached) = s.cache.borrow().get(desired)
    {
        if tracks.len() >= 64 {
            tracks.pop();
        }
        tracks.push(cached.track.clone());
    }
    let mut labels = vec![slint::SharedString::from("Off")];
    labels.extend(tracks.iter().map(|track| {
        format!(
            "{} — {}{}",
            track.label,
            track.language,
            if track.automatic {
                " (automatic / translated)"
            } else {
                ""
            }
        )
        .into()
    }));
    *s.tracks.borrow_mut() = tracks;
    s.labels.set_vec(labels);
    ui.set_selected(s.selected_index());
    ui.set_busy(true);
    ui.set_status(if s.tracks.borrow().is_empty() { "No supported guest VTT tracks were returned. Some captions may require unavailable provider capabilities." } else if item.subtitles_truncated { "First 64 guest tracks shown. Caption files load only when selected." } else { "Guest captions load only when selected. Up to 8 tracks cached per playback." }.into());
}
fn apply(app: &App, state: &UiState, track: &SubtitleTrack, lease: Arc<caption_files::Lease>) {
    let s = &state.caption_ui;
    let before = state.player.snapshot().subtitle_updates;
    let title: String = track.label.chars().take(64).collect();
    match state
        .player
        .add_subtitle(lease.path(), &title, &track.language, lease.clone())
    {
        Ok(()) => {
            let identity = key(track);
            *s.desired.borrow_mut() = Some(identity.clone());
            *s.awaiting.borrow_mut() = Some((identity, before));
            let ui = app.global::<CaptionsUi>();
            ui.set_busy(true);
            ui.set_status("Turning captions on…".into());
        }
        Err(error) => {
            *s.desired.borrow_mut() = s.selected.borrow().clone();
            let ui = app.global::<CaptionsUi>();
            ui.set_selected(s.selected_index());
            ui.set_status(error.to_string().into());
        }
    }
}
pub fn receive(
    app: &App,
    state: &UiState,
    video: VideoId,
    index: usize,
    result: Result<Arc<caption_files::Lease>, CaptionError>,
) {
    let s = &state.caption_ui;
    if s.video.borrow().as_ref() != Some(&video)
        || state.current_video.borrow().as_ref().map(|v| &v.id) != Some(&video)
        || s.pending.get() != Some((state.worker.borrow().generation(), index))
    {
        return;
    }
    s.pending.set(None);
    app.global::<CaptionsUi>().set_busy(false);
    match result {
        Ok(lease) => {
            let Some(track) = s.tracks.borrow().get(index).cloned() else {
                return;
            };
            s.cache.borrow_mut().insert(
                key(&track),
                Cached {
                    lease: lease.clone(),
                    track: track.clone(),
                },
            );
            apply(app, state, &track, lease);
        }
        Err(error) => app
            .global::<CaptionsUi>()
            .set_status(error.to_string().into()),
    }
}
pub fn observe(app: &App, state: &UiState, snapshot: &oxplay_media::Snapshot) {
    let s = &state.caption_ui;
    let ended = matches!(
        snapshot.state,
        oxplay_media::PlaybackState::Ended
            | oxplay_media::PlaybackState::Failed
            | oxplay_media::PlaybackState::Idle
    );
    if s.retire_after
        .get()
        .is_some_and(|old| snapshot.file_loads > old || ended)
    {
        s.retire_after.set(None);
        s.retired.borrow_mut().clear();
    }
    let selection_failed = (s.reload_after.get().is_some()
        || s.awaiting.borrow().is_some()
        || s.pending.get().is_some())
        && selection_load_failed(s.load_baseline.get(), snapshot);
    let off_failed =
        s.awaiting_off.get().is_some() && off_load_failed(s.load_baseline.get(), snapshot);
    if selection_failed || off_failed {
        s.reload_after.set(None);
        s.awaiting.borrow_mut().take();
        s.awaiting_off.set(None);
        s.desired.borrow_mut().take();
        s.selected.borrow_mut().take();
        if let Some((generation, _)) = s.pending.take()
            && state.worker.borrow_mut().cancel_generation(generation)
        {
            app.set_busy(false);
        }
        let ui = app.global::<CaptionsUi>();
        ui.set_busy(false);
        ui.set_selected(0);
        ui.set_status(
            snapshot
                .error
                .as_deref()
                .unwrap_or("Playback ended before caption selection completed.")
                .into(),
        );
        return;
    }
    if s.reload_after
        .get()
        .is_some_and(|old| snapshot.file_loads > old)
        && snapshot.active_load_request_id == s.load_baseline.get().1
        && matches!(
            snapshot.state,
            oxplay_media::PlaybackState::Playing | oxplay_media::PlaybackState::Paused
        )
    {
        s.reload_after.set(None);
        app.global::<CaptionsUi>()
            .set_busy(s.awaiting_off.get().is_some());
        let cached = s.desired.borrow().as_ref().and_then(|key| {
            s.cache
                .borrow()
                .get(key)
                .map(|cached| (cached.track.clone(), cached.lease.clone()))
        });
        if let Some((track, lease)) = cached {
            apply(app, state, &track, lease);
        }
    }
    if let Some(request) = s.awaiting_off.get()
        && let Some(succeeded) = off_completion(request, snapshot)
    {
        s.awaiting_off.set(None);
        let ui = app.global::<CaptionsUi>();
        ui.set_busy(s.reload_after.get().is_some());
        if succeeded {
            s.selected.borrow_mut().take();
            ui.set_selected(0);
            ui.set_status("Captions off".into());
        } else {
            ui.set_selected(s.selected_index());
            ui.set_status("Could not turn captions off. Try again.".into());
        }
    }
    let pending = s.awaiting.borrow().clone();
    if let Some((identity, previous)) = pending {
        if let Some(error) = &snapshot.error {
            s.awaiting.borrow_mut().take();
            *s.desired.borrow_mut() = s.selected.borrow().clone();
            let ui = app.global::<CaptionsUi>();
            ui.set_busy(false);
            ui.set_selected(s.selected_index());
            ui.set_status(error.clone().into());
        } else if snapshot.subtitle_selection_observed
            && snapshot.subtitle_id.is_some()
            && snapshot.subtitle_updates > previous
            && s.cache
                .borrow()
                .get(&identity)
                .is_some_and(|cached| state.player.subtitle_matches(cached.lease.path()))
        {
            *s.selected.borrow_mut() = Some(identity.clone());
            s.awaiting.borrow_mut().take();
            let ui = app.global::<CaptionsUi>();
            ui.set_busy(false);
            ui.set_selected(s.selected_index());
            let label = s
                .cache
                .borrow()
                .get(&identity)
                .map(|cached| cached.track.label.chars().take(80).collect::<String>())
                .unwrap_or_default();
            ui.set_status(if label.is_empty() {
                "Captions on".into()
            } else {
                format!("{label} captions on").into()
            });
        }
    }
}
// Only the media barrier can acknowledge Off: it drains older sub-adds,
// waits for the exact Off command and queries fresh native sid. An earlier
// observed Off is insufficient, even if the property has not changed.
fn off_completion(request: (u64, u64), snapshot: &oxplay_media::Snapshot) -> Option<bool> {
    let (load, token) = request;
    if load == 0
        || snapshot.load_request_id != load
        || snapshot.active_load_request_id != load
        || snapshot.stop_pending
    {
        return None;
    }
    snapshot
        .subtitle_off_reply
        .filter(|(reply, _)| *reply == token)
        .map(|(_, succeeded)| succeeded)
}
fn off_load_failed(baseline: (u64, u64), snapshot: &oxplay_media::Snapshot) -> bool {
    // keep-open EOF retains the actual native entry and accepts Off. An
    // END_FILE unload clears playback_restarted; keep that terminal path and
    // all failed/replaced/stopped loads subject to the ordinary retirement rule.
    let retained_eof = baseline.1 != 0
        && snapshot.load_request_id == baseline.1
        && snapshot.active_load_request_id == baseline.1
        && snapshot.failed_load_request_id != Some(baseline.1)
        && !snapshot.stop_pending
        && snapshot.state == oxplay_media::PlaybackState::Ended
        && snapshot.playback_restarted;
    selection_load_failed(baseline, snapshot) && !retained_eof
}
fn selection_load_failed(baseline: (u64, u64), snapshot: &oxplay_media::Snapshot) -> bool {
    snapshot.failed_load_request_id == Some(baseline.1)
        || snapshot.load_request_id != baseline.1
        || (snapshot.active_load_request_id == baseline.1
            && snapshot.file_starts > baseline.0
            && matches!(
                snapshot.state,
                oxplay_media::PlaybackState::Ended
                    | oxplay_media::PlaybackState::Failed
                    | oxplay_media::PlaybackState::Idle
            ))
}

pub fn files(state: &UiState) -> caption_files::Client {
    state.caption_ui.files.clone()
}
/// Media owns additional playback/command leases until exact unload/reply.
/// Releasing UI caches here never deletes a file still owned by the engine.
pub fn clear_local(app: &App, state: &UiState) {
    let s = &state.caption_ui;
    s.pending.set(None);
    s.awaiting.borrow_mut().take();
    s.awaiting_off.set(None);
    s.desired.borrow_mut().take();
    s.selected.borrow_mut().take();
    s.reload_after.set(None);
    s.retire_after.set(None);
    s.video.borrow_mut().take();
    s.tracks.borrow_mut().clear();
    s.cache.borrow_mut().clear();
    s.retired.borrow_mut().clear();
    s.labels.set_vec(vec!["Off".into()]);
    let ui = app.global::<CaptionsUi>();
    ui.set_selected(0);
    ui.set_busy(false);
    ui.set_status("Clearing caption files…".into());
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    app.global::<CaptionsUi>()
        .set_tracks(state.caption_ui.labels.clone().into());
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .worker
        .borrow_mut()
        .on_generation_changed(move |generation| {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            let s = &state.caption_ui;
            if s.pending
                .get()
                .is_some_and(|(pending, _)| pending != generation)
            {
                s.pending.set(None);
                let ui = app.global::<CaptionsUi>();
                ui.set_busy(false);
                ui.set_selected(s.selected_index());
                ui.set_status("Caption download cancelled by another action.".into());
            }
        });
    let weak = app.as_weak();
    let state = state.clone();
    app.global::<CaptionsUi>().on_select(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let s = &state.caption_ui;
        let ui = app.global::<CaptionsUi>();
        ui.set_selected(s.selected_index());
        if index == 0 {
            if let Some((generation, _)) = s.pending.take()
                && state.worker.borrow_mut().cancel_generation(generation)
            {
                app.set_busy(false);
            }
            s.desired.borrow_mut().take();
            s.awaiting.borrow_mut().take();
            s.awaiting_off.set(None);
            ui.set_busy(s.reload_after.get().is_some());
            // Keep the last observed track selected until native Off is known;
            // queue admission is not acknowledgement. Off still cancels pending
            // download/sub-add intent even when the native command fails.
            match state.player.disable_subtitles() {
                Ok(token) => {
                    s.awaiting_off
                        .set(Some((state.player.snapshot().load_request_id, token)));
                    ui.set_busy(true);
                    ui.set_status("Turning captions off…".into());
                }
                Err(error) => ui.set_status(error.to_string().into()),
            };
            return;
        }
        if app.get_busy() || ui.get_busy() || s.reload_after.get().is_some() {
            return;
        }
        let Ok(index) = usize::try_from(index - 1) else {
            return;
        };
        let Some(track) = s.tracks.borrow().get(index).cloned() else {
            return;
        };
        let Some(video) = s.video.borrow().clone() else {
            return;
        };
        if state.current_video.borrow().as_ref().map(|v| &v.id) != Some(&video) {
            return;
        }
        if s.selected.borrow().as_ref() == Some(&key(&track)) {
            return;
        }
        let cached = s
            .cache
            .borrow()
            .get(&key(&track))
            .map(|cached| cached.lease.clone());
        if let Some(lease) = cached {
            apply(&app, &state, &track, lease);
            return;
        }
        if s.cache.borrow().len() >= 8 {
            ui.set_status(
                "Eight tracks cached. Open the video again to clear this playback's caption cache."
                    .into(),
            );
            return;
        }
        state.worker.borrow_mut().submit(Request::Caption(
            video,
            index,
            Box::new(track),
            s.files.clone(),
        ));
        s.pending
            .set(Some((state.worker.borrow().generation(), index)));
        app.set_busy(true);
        ui.set_busy(true);
        ui.set_status("Downloading selected guest caption track…".into());
    });
}

/// Explicit finite native diagnostic; absent from normal application operation.
pub struct Smoke {
    verified: Rc<Cell<bool>>,
    _timers: Vec<slint::Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let verified = Rc::new(Cell::new(false));
        let mut timers = Vec::new();
        let before_quality = Rc::new(Cell::new(0));
        let off_request = Rc::new(Cell::new(None));
        for stage in [12, 25, 30, 35, 65] {
            let weak = app.as_weak();
            let state = state.clone();
            let verified = verified.clone();
            let before_quality = before_quality.clone();
            let off_request = off_request.clone();
            let timer = slint::Timer::default();
            timer.start(slint::TimerMode::SingleShot, std::time::Duration::from_secs(stage), move || {
                let Some(app) = weak.upgrade() else { return };
                let ui = app.global::<CaptionsUi>();
                let s = &state.caption_ui;
                let snapshot = state.player.snapshot();
                assert!(snapshot.error.is_none(), "caption smoke media failure");
                match stage {
                    12 => {
                        assert!(app.get_remote_video() && app.get_loaded() && !app.get_busy(), "caption smoke requires embedded guest playback");
                        assert!(!s.tracks.borrow().is_empty(), "fixture did not return supported guest captions");
                        state.player.set_paused(true).expect("caption smoke pause failed");
                        app.invoke_show_captions();
                        ui.invoke_select(1);
                    }
                    25 => {
                        assert!(!ui.get_busy() && ui.get_selected() == 1 && snapshot.subtitle_id.is_some(), "selected caption was not observed");
                        assert_eq!(s.cache.borrow().len(), 1);
                        ui.invoke_select(0);
                        off_request.set(Some(s.awaiting_off.get().expect("caption Off barrier was not admitted")));
                        assert!(ui.get_busy() && ui.get_selected() == 1, "Off submission falsely acknowledged a selection change");
                    }
                    30 => {
                        assert!(snapshot.subtitle_selection_observed && snapshot.subtitle_id.is_none(), "caption Off was not observed");
                        assert!(!ui.get_busy() && ui.get_selected() == 0 && s.awaiting_off.get().is_none(), "caption Off UI acknowledgement did not settle");
                        assert_eq!(off_completion(off_request.get().expect("caption Off request was not retained"), &snapshot), Some(true), "the exact caption Off barrier did not complete successfully");
                        assert_eq!(s.cache.borrow().len(), 1, "Off discarded reusable file");
                        ui.invoke_select(1);
                        assert!(s.pending.get().is_none(), "cached reselect unexpectedly started a download");
                    }
                    35 => {
                        assert!(!ui.get_busy() && ui.get_selected() == 1 && snapshot.subtitle_id.is_some(), "cached selection was not observed");
                        before_quality.set(snapshot.file_loads);
                        app.invoke_quality(1);
                    }
                    _ => {
                        assert!(!app.get_busy() && !ui.get_busy(), "quality/caption work did not finish");
                        assert!(snapshot.file_loads > before_quality.get(), "quality did not load a new media file");
                        assert_eq!(state.quality_index.get(), 1);
                        assert!(snapshot.paused && snapshot.height <= 720, "quality change lost pause or ceiling");
                        assert!(snapshot.subtitle_id.is_some() && ui.get_selected() == 1, "caption selection was not reattached after quality change");
                        assert_eq!(s.cache.borrow().len(), 1, "quality change duplicated caption cache");
                        verified.set(true);
                    }
                }
                eprintln!("caption smoke stage={stage} selected={} sid={:?} files={} loads={} paused={}", ui.get_selected(), snapshot.subtitle_id, s.cache.borrow().len(), snapshot.file_loads, snapshot.paused);
            });
            timers.push(timer);
        }
        Self {
            verified,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        if self.verified.get() {
            Ok(())
        } else {
            Err("caption smoke ended before its native assertions completed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn off_acknowledgement_requires_exact_completed_barrier_not_old_sid() {
        let mut snapshot = oxplay_media::Snapshot {
            load_request_id: 42,
            active_load_request_id: 42,
            state: oxplay_media::PlaybackState::Paused,
            subtitle_id: None,
            subtitle_selection_observed: true,
            subtitle_updates: 9,
            ..Default::default()
        };
        // A pending sub-add can still select a track despite the old observed
        // Off. Neither it nor another unrelated native error completes Off.
        assert_eq!(off_completion((42, 7), &snapshot), None);
        snapshot.error = Some("unrelated command failure".into());
        assert_eq!(off_completion((42, 7), &snapshot), None);
        snapshot.subtitle_off_reply = Some((6, true));
        assert_eq!(off_completion((42, 7), &snapshot), None);
        snapshot.subtitle_off_reply = Some((7, true));
        assert_eq!(off_completion((42, 7), &snapshot), Some(true));
        // A no-op Off still completes via the barrier's fresh query; it need
        // not increment property-notification counters to release the UI.
        assert_eq!(snapshot.subtitle_updates, 9);
        snapshot.subtitle_off_reply = Some((7, false));
        assert_eq!(off_completion((42, 7), &snapshot), Some(false));
        snapshot.load_request_id = 43;
        assert_eq!(off_completion((42, 7), &snapshot), None);
        assert_eq!(off_completion((43, 7), &snapshot), None);
        snapshot.active_load_request_id = 43;
        snapshot.subtitle_off_reply = None;
        assert_eq!(off_completion((43, 8), &snapshot), None);
        snapshot.subtitle_off_reply = Some((8, true));
        assert_eq!(off_completion((43, 8), &snapshot), Some(true));
        snapshot.stop_pending = true;
        assert_eq!(off_completion((43, 8), &snapshot), None);
        snapshot.stop_pending = false;
        snapshot.load_request_id = 0;
        snapshot.active_load_request_id = 0;
        assert_eq!(off_completion((0, 8), &snapshot), None);
    }
    #[test]
    fn only_off_can_finish_at_retained_eof_but_never_after_native_unload() {
        let baseline = (4, 42);
        let mut snapshot = oxplay_media::Snapshot {
            file_starts: 5,
            load_request_id: 42,
            active_load_request_id: 42,
            state: oxplay_media::PlaybackState::Ended,
            playback_restarted: true,
            ..Default::default()
        };
        assert!(
            selection_load_failed(baseline, &snapshot),
            "On/startup retains its terminal rule"
        );
        assert!(!off_load_failed(baseline, &snapshot));
        assert_eq!(
            off_completion((42, 7), &snapshot),
            None,
            "EOF cannot fabricate Off completion"
        );
        snapshot.subtitle_off_reply = Some((7, true));
        assert_eq!(off_completion((42, 7), &snapshot), Some(true));
        snapshot.playback_restarted = false;
        assert!(
            off_load_failed(baseline, &snapshot),
            "END_FILE retires the retained native entry"
        );
        snapshot.playback_restarted = true;
        snapshot.failed_load_request_id = Some(42);
        assert!(off_load_failed(baseline, &snapshot));
        snapshot.failed_load_request_id = None;
        snapshot.stop_pending = true;
        assert!(off_load_failed(baseline, &snapshot));
        snapshot.stop_pending = false;
        snapshot.load_request_id = 43;
        assert!(off_load_failed(baseline, &snapshot));
    }
    #[test]
    fn terminal_loading_detection_is_bound_to_the_actual_load_request() {
        let mut snapshot = oxplay_media::Snapshot {
            file_starts: 4,
            load_request_id: 42,
            active_load_request_id: 41,
            state: oxplay_media::PlaybackState::Failed,
            error: Some("previous failure".into()),
            ..Default::default()
        };
        assert!(!selection_load_failed((4, 42), &snapshot));
        snapshot.file_starts = 5;
        snapshot.failed_load_request_id = Some(41);
        assert!(
            !selection_load_failed((4, 42), &snapshot),
            "stale request failure must not abort the new selection"
        );
        snapshot.active_load_request_id = 42;
        snapshot.state = oxplay_media::PlaybackState::Buffering;
        assert!(!selection_load_failed((4, 42), &snapshot));
        for terminal in [
            oxplay_media::PlaybackState::Ended,
            oxplay_media::PlaybackState::Failed,
            oxplay_media::PlaybackState::Idle,
        ] {
            snapshot.state = terminal;
            assert!(selection_load_failed((4, 42), &snapshot));
        }
        snapshot.file_starts = 4;
        snapshot.state = oxplay_media::PlaybackState::Playing;
        snapshot.failed_load_request_id = Some(42);
        assert!(
            selection_load_failed((4, 42), &snapshot),
            "command failure need not emit START_FILE"
        );
        snapshot.failed_load_request_id = None;
        snapshot.load_request_id = 0;
        assert!(
            selection_load_failed((4, 42), &snapshot),
            "stop invalidates pending selection"
        );
    }
}
