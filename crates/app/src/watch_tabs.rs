// SPDX-License-Identifier: GPL-3.0-or-later
//! Session-only watch tabs: several public guest videos kept ready in a bounded
//! strip while the one in-process player presents only the active tab.
//! Background tabs hold typed identity, display metadata and a resume point;
//! no stream address, decoder or second player exists for them. Switching
//! submits the ordinary latest-request guest resolve at the remembered
//! position, so existing worker and watch-loading generations reject stale
//! results exactly as for any other selection.
use crate::{App, TabsUi, UiState, WatchTab};
use oxplay_core::{CatalogItem, VideoId};
use slint::{ComponentHandle, Model, Timer, TimerMode, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub const MAX_TABS: usize = 12;
/// Upper bound on waiting for the leaving tab's exact native position.
const CAPTURE_DEADLINE: Duration = Duration::from_millis(600);
const NOTICE_DURATION: Duration = Duration::from_millis(2600);
/// Leaving this close to the end restarts the video instead of resuming at EOF.
const END_MARGIN: f64 = 3.;

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    /// Stable identity of the tab slot; survives index shifts from closing.
    pub key: u64,
    pub id: VideoId,
    pub title: String,
    pub channel: String,
    /// Remembered resume point in whole seconds.
    pub position: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    Added(usize),
    Existing(usize),
}
#[derive(Debug, PartialEq, Eq)]
pub struct Full;
#[derive(Debug, PartialEq, Eq)]
pub enum Closed {
    /// A background tab closed; the presented video is unchanged.
    Background,
    /// The active tab closed; present this neighbour next.
    Activate(usize),
    /// The active tab was the last one.
    Empty,
}

/// Pure tab model. At most one tab is active, meaning the player owns (or is
/// loading) its video; account/local playback leaves every tab in background.
#[derive(Default)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_key: u64,
}
impl Tabs {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }
    pub fn active(&self) -> Option<usize> {
        self.active
    }
    pub fn get(&self, index: usize) -> Option<&Tab> {
        self.tabs.get(index)
    }
    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|index| self.tabs.get(index))
    }
    pub fn index_of(&self, key: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.key == key)
    }
    fn tab(&mut self, id: VideoId, title: &str, channel: &str) -> Tab {
        self.next_key += 1;
        Tab {
            key: self.next_key,
            id,
            title: title.to_owned(),
            channel: channel.to_owned(),
            position: 0,
        }
    }
    /// A normal selection replaces the active tab's video (the long-standing
    /// single-video behaviour) or creates the first tab. Reselecting the
    /// active video keeps its resume point. With no active tab and a full
    /// strip, the rightmost tab is reused so a normal selection never fails.
    pub fn show(&mut self, id: VideoId, title: &str, channel: &str) -> usize {
        if let Some(index) = self.active {
            let tab = &mut self.tabs[index];
            if tab.id == id {
                fill(tab, title, channel);
            } else {
                *tab = Tab {
                    key: tab.key,
                    id,
                    title: title.to_owned(),
                    channel: channel.to_owned(),
                    position: 0,
                };
            }
            return index;
        }
        let tab = self.tab(id, title, channel);
        let index = if self.tabs.len() < MAX_TABS {
            self.tabs.push(tab);
            self.tabs.len() - 1
        } else {
            let last = self.tabs.len() - 1;
            self.tabs[last] = tab;
            last
        };
        self.active = Some(index);
        index
    }
    /// Keep a video ready without touching the presented one. A video that is
    /// already open is not duplicated.
    pub fn open_background(
        &mut self,
        id: VideoId,
        title: &str,
        channel: &str,
    ) -> Result<Opened, Full> {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
            return Ok(Opened::Existing(index));
        }
        if self.tabs.len() >= MAX_TABS {
            return Err(Full);
        }
        let tab = self.tab(id, title, channel);
        self.tabs.push(tab);
        Ok(Opened::Added(self.tabs.len() - 1))
    }
    /// Record the resume point of the tab the player is leaving.
    pub fn remember(&mut self, position: u32) {
        if let Some(index) = self.active {
            self.tabs[index].position = position;
        }
    }
    pub fn activate(&mut self, index: usize) -> Option<&Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        self.active = Some(index);
        self.tabs.get(index)
    }
    /// Ctrl+Tab order from `from` (the active or pending tab), wrapping.
    pub fn neighbour(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let len = self.tabs.len();
        match from.filter(|index| *index < len) {
            _ if len == 0 => None,
            Some(_) if len == 1 => None,
            Some(index) if forward => Some((index + 1) % len),
            Some(index) => Some((index + len - 1) % len),
            None if forward => Some(0),
            None => Some(len - 1),
        }
    }
    /// Closing the active tab selects its right neighbour, else its left one.
    pub fn close(&mut self, index: usize) -> Option<Closed> {
        if index >= self.tabs.len() {
            return None;
        }
        self.tabs.remove(index);
        Some(match self.active {
            Some(active) if active == index => {
                if self.tabs.is_empty() {
                    self.active = None;
                    Closed::Empty
                } else {
                    let next = index.min(self.tabs.len() - 1);
                    self.active = Some(next);
                    Closed::Activate(next)
                }
            }
            Some(active) if active > index => {
                self.active = Some(active - 1);
                Closed::Background
            }
            _ => Closed::Background,
        })
    }
    /// The player now presents something that is not a tab (account or local).
    pub fn detach(&mut self) -> Option<usize> {
        self.active.take()
    }
    /// Accepted metadata for the active video fills its chip.
    pub fn update(&mut self, id: &VideoId, title: &str, channel: &str) {
        if let Some(index) = self.active
            && self.tabs[index].id == *id
        {
            fill(&mut self.tabs[index], title, channel);
        }
    }
    pub fn clear(&mut self) {
        self.tabs.clear();
        self.active = None;
    }
    /// A single presented tab adds nothing to Now playing/the mini-player; the
    /// strip appears for two or more tabs, or for a tab the player is not showing.
    pub fn strip_visible(&self) -> bool {
        self.tabs.len() >= 2 || (self.tabs.len() == 1 && self.active.is_none())
    }
    fn rows(&self) -> Vec<WatchTab> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| WatchTab {
                title: tab.title.as_str().into(),
                channel: tab.channel.as_str().into(),
                active: self.active == Some(index),
            })
            .collect()
    }
}
fn fill(tab: &mut Tab, title: &str, channel: &str) {
    if !title.is_empty() {
        tab.title = title.to_owned();
    }
    if !channel.is_empty() {
        tab.channel = channel.to_owned();
    }
}

/// Whole-second resume point; the end of a video restarts from zero.
pub fn resume_seconds(position: f64, duration: f64) -> u32 {
    if !position.is_finite() || position <= 0. {
        return 0;
    }
    if duration.is_finite() && duration > 0. && position >= duration - END_MARGIN {
        return 0;
    }
    position.floor().min(f64::from(u32::MAX)) as u32
}

struct Capture {
    token: u64,
    target: u64,
}
pub struct State {
    tabs: RefCell<Tabs>,
    model: Rc<VecModel<WatchTab>>,
    /// Pending exact position of the leaving tab, then the tab to present.
    capture: RefCell<Option<Capture>>,
    capture_deadline: Timer,
    notice: Timer,
    /// Where closing the last tab returns when the watch page is showing.
    return_page: Cell<i32>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            tabs: RefCell::default(),
            model: Rc::new(VecModel::default()),
            capture: RefCell::default(),
            capture_deadline: Timer::default(),
            notice: Timer::default(),
            return_page: Cell::new(0),
        }
    }
}

fn sync(app: &App, state: &UiState) {
    let tabs = state.watch_tabs.tabs.borrow();
    let rows = tabs.rows();
    let model = &state.watch_tabs.model;
    if model.row_count() == rows.len() {
        for (index, row) in rows.into_iter().enumerate() {
            if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        }
    } else {
        model.set_vec(rows);
    }
    let ui = app.global::<TabsUi>();
    ui.set_active_index(tabs.active().map_or(-1, |index| index as i32));
    ui.set_strip_visible(tabs.strip_visible());
}

fn notice(app: &App, state: &UiState, text: &str) {
    app.global::<TabsUi>().set_notice(text.into());
    let weak = app.as_weak();
    state
        .watch_tabs
        .notice
        .start(TimerMode::SingleShot, NOTICE_DURATION, move || {
            if let Some(app) = weak.upgrade() {
                app.global::<TabsUi>().set_notice("".into());
            }
        });
}

fn remember_page(app: &App, state: &UiState) {
    if app.get_page() != 2 {
        state.watch_tabs.return_page.set(app.get_page());
    }
}

fn cancel_capture(state: &UiState) {
    state.watch_tabs.capture_deadline.stop();
    if let Some(capture) = state.watch_tabs.capture.borrow_mut().take() {
        state.player.cancel_resume_position(capture.token);
    }
}

/// The player presents the active tab's own accepted guest file right now.
fn leaving_live(app: &App, state: &UiState) -> bool {
    let tabs = state.watch_tabs.tabs.borrow();
    let Some(tab) = tabs.active_tab() else {
        return false;
    };
    app.get_loaded()
        && app.get_remote_video()
        && !app.get_watch_loading()
        && crate::account_playback::authorization(state).is_none()
        && state
            .current_video
            .borrow()
            .as_ref()
            .is_some_and(|video| video.id == tab.id)
        && state.player.current_load_is_active()
}

/// Last observed position; may lag while progress polling is suspended.
fn observed_position(app: &App, state: &UiState) -> Option<u32> {
    leaving_live(app, state).then(|| {
        let snapshot = state.player.snapshot();
        resume_seconds(snapshot.position, snapshot.duration)
    })
}

fn admitted(app: &App, state: &UiState) -> bool {
    if app.get_native_video_child() || state.caption_cache.active() {
        return false;
    }
    // Same admission as a card selection: never replace unrelated busy work.
    if app.get_busy() && !app.get_watch_loading() {
        notice(
            app,
            state,
            "Finishing the current request… Try again shortly.",
        );
        return false;
    }
    crate::playback_preferences::admit_search(app, state)
}

/// Present tab `index` through the ordinary guest selection path.
fn start(app: &App, state: &Rc<UiState>, index: usize) {
    let tab = state.watch_tabs.tabs.borrow_mut().activate(index).cloned();
    let Some(tab) = tab else {
        sync(app, state);
        return;
    };
    remember_page(app, state);
    crate::watch_loading::prepare_guest(app, state);
    let quality = crate::library_ui::desired_preferences(state)
        .playback
        .quality;
    state
        .worker
        .borrow_mut()
        .submit(crate::catalog::Request::ResolveAt(
            tab.id.clone(),
            quality,
            oxplay_core::VideoStart::from_seconds(tab.position),
        ));
    let generation = state.worker.borrow().generation();
    crate::watch_loading::guest_begin(app, state, generation, &tab.id);
    // Browsing may no longer hold this video; the tab still knows its identity.
    if app.get_video_title().is_empty() {
        app.set_video_title(tab.title.as_str().into());
    }
    if app.get_video_channel().is_empty() {
        app.set_video_channel(tab.channel.as_str().into());
    }
    state.focus_intent.arm(
        crate::focus_intent::Scope::Guest(generation),
        app.get_search_active(),
    );
    app.set_busy(true);
    app.set_status("Resolving the selected tab's stream…".into());
    sync(app, state);
}

fn finish(app: &App, state: &Rc<UiState>, target: u64, leaving: Option<u32>) {
    let index = {
        let mut tabs = state.watch_tabs.tabs.borrow_mut();
        if let Some(position) = leaving {
            tabs.remember(position);
        }
        tabs.index_of(target)
    };
    match index {
        Some(index) if !app.get_native_video_child() && !state.caption_cache.active() => {
            start(app, state, index)
        }
        _ => sync(app, state),
    }
}

fn switch(app: &App, state: &Rc<UiState>, index: usize) {
    let (target, active) = {
        let tabs = state.watch_tabs.tabs.borrow();
        let Some(tab) = tabs.get(index) else { return };
        (tab.key, tabs.active() == Some(index))
    };
    // A later request retargets a pending switch; its position request stays.
    // Choosing the still-active tab abandons the switch altogether.
    if active {
        cancel_capture(state);
    } else if let Some(capture) = state.watch_tabs.capture.borrow_mut().as_mut() {
        capture.target = target;
        return;
    }
    if active && (app.get_loaded() || app.get_watch_loading()) {
        if app.get_page() != 2 {
            app.invoke_navigate(2);
        }
        return;
    }
    if !admitted(app, state) {
        return;
    }
    if active {
        // A failed or cancelled load restarts from the remembered point.
        start(app, state, index);
        return;
    }
    // Progress polling pauses with hidden controls, so ask the player for the
    // exact position first. Stop would invalidate this reply; switch after it.
    let token = leaving_live(app, state)
        .then(|| state.player.request_resume_position().ok())
        .flatten();
    let Some(token) = token else {
        let leaving = observed_position(app, state);
        finish(app, state, target, leaving);
        return;
    };
    *state.watch_tabs.capture.borrow_mut() = Some(Capture { token, target });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .watch_tabs
        .capture_deadline
        .start(TimerMode::SingleShot, CAPTURE_DEADLINE, move || {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            let capture = state.watch_tabs.capture.borrow_mut().take();
            if let Some(capture) = capture {
                state.player.cancel_resume_position(capture.token);
                let leaving = observed_position(&app, &state);
                finish(&app, &state, capture.target, leaving);
            }
        });
}

/// Called from the media update with each drained snapshot.
pub fn observe(app: &App, state: &Rc<UiState>, snapshot: &oxplay_media::Snapshot) {
    let Some((token, position)) = snapshot.resume_position_reply else {
        return;
    };
    let target = state
        .watch_tabs
        .capture
        .borrow()
        .as_ref()
        .filter(|capture| capture.token == token)
        .map(|capture| capture.target);
    let Some(target) = target else { return };
    state.watch_tabs.capture.borrow_mut().take();
    state.watch_tabs.capture_deadline.stop();
    let leaving = match position {
        Some(position) if leaving_live(app, state) => {
            Some(resume_seconds(position, snapshot.duration))
        }
        _ => observed_position(app, state),
    };
    finish(app, state, target, leaving);
}

fn close(app: &App, state: &Rc<UiState>, index: usize) {
    let activates_neighbour = {
        let tabs = state.watch_tabs.tabs.borrow();
        tabs.active() == Some(index) && tabs.tabs.len() > 1
    };
    if activates_neighbour && !admitted(app, state) {
        return;
    }
    let closed = state.watch_tabs.tabs.borrow_mut().close(index);
    match closed {
        None => {}
        Some(Closed::Background) => sync(app, state),
        Some(Closed::Activate(next)) => {
            // The closed tab's position is gone with it; no capture is needed.
            cancel_capture(state);
            start(app, state, next);
        }
        Some(Closed::Empty) => {
            cancel_capture(state);
            // Retire an in-flight resolve for the closed tab, then stop.
            state.worker.borrow_mut().cancel();
            app.set_busy(false);
            crate::close_player(app, state);
            if app.get_page() == 2 {
                match state.watch_tabs.return_page.get() {
                    // Browsing keeps its retained results rather than reloading Home.
                    0 => {
                        app.set_page(0);
                        app.invoke_refresh_visible();
                    }
                    page => app.invoke_navigate(page),
                }
            }
            sync(app, state);
        }
    }
}

fn open_background(app: &App, state: &UiState, item: Option<CatalogItem>) {
    let Some(CatalogItem::Video(video)) = item else {
        return;
    };
    if app.get_native_video_child() || state.caption_cache.active() {
        return;
    }
    let result =
        state
            .watch_tabs
            .tabs
            .borrow_mut()
            .open_background(video.id, &video.title, &video.channel);
    match result {
        Ok(Opened::Added(index)) => {
            app.set_status(format!("Opened in background tab {}.", index + 1).into())
        }
        Ok(Opened::Existing(index)) => {
            notice(app, state, &format!("Already open in tab {}.", index + 1))
        }
        Err(Full) => notice(
            app,
            state,
            &format!("Tab limit reached ({MAX_TABS}). Close a tab to open another."),
        ),
    }
    sync(app, state);
}

/// Hook: a guest selection is replacing the presented video (before its stop).
pub fn selected(app: &App, state: &UiState, id: &VideoId) {
    cancel_capture(state);
    remember_page(app, state);
    let (title, channel) = crate::guest_ui::video_summary(state, id)
        .map(|video| (video.title, video.channel))
        .unwrap_or_default();
    state
        .watch_tabs
        .tabs
        .borrow_mut()
        .show(id.clone(), &title, &channel);
    sync(app, state);
}

/// Hook: accepted guest metadata for the presented video.
pub fn accepted(app: &App, state: &UiState, video: &oxplay_core::VideoSummary) {
    state
        .watch_tabs
        .tabs
        .borrow_mut()
        .update(&video.id, &video.title, &video.channel);
    sync(app, state);
}

/// Hook: account or local playback replaces the presented tab. Its tab stays
/// in the strip with the last observed position (best effort).
pub fn detach(app: &App, state: &UiState) {
    cancel_capture(state);
    let leaving = observed_position(app, state);
    {
        let mut tabs = state.watch_tabs.tabs.borrow_mut();
        if let Some(position) = leaving {
            tabs.remember(position);
        }
        tabs.detach();
    }
    remember_page(app, state);
    sync(app, state);
}

/// Hook: the mini-player's Close player dismisses the presented tab only.
pub fn player_closed(app: &App, state: &UiState) {
    cancel_capture(state);
    {
        let mut tabs = state.watch_tabs.tabs.borrow_mut();
        if let Some(index) = tabs.detach() {
            tabs.close(index);
        }
    }
    sync(app, state);
}

/// Hook: explicit local-data clearing also forgets session tabs.
pub fn clear(app: &App, state: &UiState) {
    cancel_capture(state);
    state.watch_tabs.tabs.borrow_mut().clear();
    sync(app, state);
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let ui = app.global::<TabsUi>();
    ui.set_tabs(slint::ModelRc::from(state.watch_tabs.model.clone()));
    let weak = app.as_weak();
    let s = Rc::downgrade(state);
    ui.on_activate(move |index| {
        if let (Some(app), Some(s), Ok(index)) =
            (weak.upgrade(), s.upgrade(), usize::try_from(index))
        {
            switch(&app, &s, index);
        }
    });
    let weak = app.as_weak();
    let s = Rc::downgrade(state);
    ui.on_close(move |index| {
        if let (Some(app), Some(s), Ok(index)) =
            (weak.upgrade(), s.upgrade(), usize::try_from(index))
        {
            close(&app, &s, index);
        }
    });
    let weak = app.as_weak();
    let s = Rc::downgrade(state);
    ui.on_cycle(move |forward| {
        let (Some(app), Some(s)) = (weak.upgrade(), s.upgrade()) else {
            return;
        };
        let target = {
            let tabs = s.watch_tabs.tabs.borrow();
            let pending = s
                .watch_tabs
                .capture
                .borrow()
                .as_ref()
                .and_then(|capture| tabs.index_of(capture.target));
            tabs.neighbour(pending.or(tabs.active()), forward)
        };
        if let Some(target) = target {
            switch(&app, &s, target);
        }
    });
    let weak = app.as_weak();
    let s = Rc::downgrade(state);
    ui.on_close_active(move || {
        let (Some(app), Some(s)) = (weak.upgrade(), s.upgrade()) else {
            return;
        };
        let active = s.watch_tabs.tabs.borrow().active();
        if let Some(index) = active {
            close(&app, &s, index);
        }
    });
    let weak = app.as_weak();
    let s = Rc::downgrade(state);
    ui.on_open(move |surface, index| {
        let (Some(app), Some(s), Ok(index)) = (weak.upgrade(), s.upgrade(), usize::try_from(index))
        else {
            return;
        };
        let item = match surface {
            0 if app.get_page() == 0 => s.guest_ui.item(index),
            1 if app.get_page() == 2 => s.watch_context.item(index),
            _ => None,
        };
        open_background(&app, &s, item);
    });
    sync(app, state);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> VideoId {
        VideoId::new(&format!("tabVideo{n:03}")).unwrap()
    }
    fn open(tabs: &mut Tabs, n: u8) -> Result<Opened, Full> {
        tabs.open_background(id(n), &format!("Title {n}"), "Channel")
    }

    #[test]
    fn a_normal_selection_replaces_the_active_tab_and_background_opens_append() {
        let mut tabs = Tabs::default();
        assert!(!tabs.strip_visible());
        assert_eq!(tabs.show(id(1), "One", "A"), 0);
        // One presented tab adds nothing beside Now playing.
        assert!(!tabs.strip_visible());
        tabs.remember(42);
        assert_eq!(open(&mut tabs, 2), Ok(Opened::Added(1)));
        assert!(tabs.strip_visible());
        // Background opens never move the presented tab.
        assert_eq!(tabs.active(), Some(0));
        assert_eq!(tabs.active_tab().unwrap().position, 42);
        // Reselecting the same active video keeps its resume point.
        let key = tabs.get(0).unwrap().key;
        tabs.show(id(1), "", "");
        assert_eq!(tabs.get(0).unwrap().position, 42);
        assert_eq!(tabs.get(0).unwrap().title, "One");
        // A different normal selection replaces the video in the same slot.
        assert_eq!(tabs.show(id(3), "Three", "C"), 0);
        let replaced = tabs.get(0).unwrap();
        assert_eq!((replaced.key, replaced.position), (key, 0));
        assert_eq!(replaced.id, id(3));
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn background_opens_are_deduplicated_and_bounded() {
        let mut tabs = Tabs::default();
        tabs.show(id(0), "Zero", "");
        for n in 1..MAX_TABS as u8 {
            assert_eq!(open(&mut tabs, n), Ok(Opened::Added(n as usize)));
        }
        assert_eq!(tabs.len(), MAX_TABS);
        assert_eq!(open(&mut tabs, 3), Ok(Opened::Existing(3)));
        assert_eq!(open(&mut tabs, 0), Ok(Opened::Existing(0)));
        assert_eq!(open(&mut tabs, 99), Err(Full));
        assert_eq!(tabs.len(), MAX_TABS);
        // With nothing active, a normal selection still succeeds by reusing
        // the rightmost slot instead of growing past the bound.
        tabs.detach();
        assert_eq!(tabs.show(id(99), "New", ""), MAX_TABS - 1);
        assert_eq!(tabs.len(), MAX_TABS);
        assert_eq!(tabs.active_tab().unwrap().id, id(99));
    }

    #[test]
    fn switching_remembers_each_leaving_position_and_resumes_the_target() {
        let mut tabs = Tabs::default();
        tabs.show(id(1), "One", "");
        open(&mut tabs, 2).unwrap();
        open(&mut tabs, 3).unwrap();
        // Leave tab 0 at 95 s for tab 2.
        tabs.remember(95);
        assert_eq!(tabs.activate(2).unwrap().position, 0);
        tabs.remember(12);
        // Back to tab 0: its own point, not the one just left.
        assert_eq!(tabs.activate(0).unwrap().position, 95);
        assert_eq!(tabs.get(2).unwrap().position, 12);
        assert!(tabs.activate(3).is_none());
        assert_eq!(tabs.active(), Some(0));
    }

    #[test]
    fn closing_the_active_tab_prefers_the_right_neighbour_then_the_left() {
        let mut tabs = Tabs::default();
        tabs.show(id(0), "", "");
        for n in 1..4 {
            open(&mut tabs, n).unwrap();
        }
        tabs.activate(1);
        let right = tabs.get(2).unwrap().key;
        assert_eq!(tabs.close(1), Some(Closed::Activate(1)));
        assert_eq!(tabs.active_tab().unwrap().key, right);
        // Rightmost active tab falls back to its left neighbour.
        tabs.activate(2);
        let left = tabs.get(1).unwrap().key;
        assert_eq!(tabs.close(2), Some(Closed::Activate(1)));
        assert_eq!(tabs.active_tab().unwrap().key, left);
        // Closing a tab before the active one keeps the same active tab.
        assert_eq!(tabs.close(0), Some(Closed::Background));
        assert_eq!(tabs.active_tab().unwrap().key, left);
        assert_eq!(tabs.close(5), None);
        assert_eq!(tabs.close(0), Some(Closed::Empty));
        assert_eq!(tabs.active(), None);
        assert!(!tabs.strip_visible());
    }

    #[test]
    fn closing_background_tabs_and_detached_tabs_never_activate_another() {
        let mut tabs = Tabs::default();
        tabs.show(id(0), "", "");
        open(&mut tabs, 1).unwrap();
        open(&mut tabs, 2).unwrap();
        assert_eq!(tabs.close(2), Some(Closed::Background));
        assert_eq!(tabs.active(), Some(0));
        // Account/local playback detaches; the tab remains to return to.
        assert_eq!(tabs.detach(), Some(0));
        assert!(tabs.strip_visible());
        assert_eq!(tabs.close(0), Some(Closed::Background));
        assert_eq!(tabs.active(), None);
        // A single tab the player is not showing keeps the strip visible.
        assert_eq!(tabs.len(), 1);
        assert!(tabs.strip_visible());
    }

    #[test]
    fn cycling_wraps_and_starts_from_either_end_without_an_active_tab() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.neighbour(None, true), None);
        tabs.show(id(0), "", "");
        assert_eq!(tabs.neighbour(Some(0), true), None);
        open(&mut tabs, 1).unwrap();
        open(&mut tabs, 2).unwrap();
        assert_eq!(tabs.neighbour(Some(0), true), Some(1));
        assert_eq!(tabs.neighbour(Some(2), true), Some(0));
        assert_eq!(tabs.neighbour(Some(0), false), Some(2));
        assert_eq!(tabs.neighbour(None, true), Some(0));
        assert_eq!(tabs.neighbour(None, false), Some(2));
        assert_eq!(tabs.neighbour(Some(9), false), Some(2));
    }

    #[test]
    fn metadata_updates_touch_only_the_matching_active_tab() {
        let mut tabs = Tabs::default();
        tabs.show(id(1), "", "");
        open(&mut tabs, 2).unwrap();
        tabs.update(&id(2), "Wrong", "Wrong");
        assert_eq!(tabs.get(1).unwrap().title, "Title 2");
        tabs.update(&id(1), "Accepted", "Creator");
        assert_eq!(tabs.get(0).unwrap().title, "Accepted");
        assert_eq!(tabs.get(0).unwrap().channel, "Creator");
        tabs.update(&id(1), "", "");
        assert_eq!(tabs.get(0).unwrap().title, "Accepted");
        let rows = tabs.rows();
        assert!(rows[0].active && !rows[1].active);
        tabs.clear();
        assert_eq!((tabs.len(), tabs.active()), (0, None));
    }

    #[test]
    fn resume_points_are_whole_seconds_and_the_end_restarts() {
        assert_eq!(resume_seconds(95.9, 600.), 95);
        assert_eq!(resume_seconds(-1., 600.), 0);
        assert_eq!(resume_seconds(f64::NAN, 600.), 0);
        assert_eq!(resume_seconds(598., 600.), 0);
        // Unknown duration (live or not yet observed) keeps the position.
        assert_eq!(resume_seconds(598., 0.), 598);
    }
}
