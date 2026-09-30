// SPDX-License-Identifier: GPL-3.0-or-later
//! Header search suggestions: submitted plain-text searches (persisted only
//! with the local-history opt-in, otherwise kept for this session) plus
//! optional anonymous YouTube completions for the text being typed.
//!
//! Remote requests are debounced, only made while the field is focused, and
//! superseded requests are dropped (which cancels their HTTP exchange). Query
//! text is never logged or included in status/diagnostic output.
use crate::{App, SearchSuggestion, SearchSuggestionsUi, UiState, library};
use serein_core::ProviderError;
use serein_storage::{MAX_SEARCH_HISTORY, normalize_search_query, search_query_key};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const MAX_ROWS: usize = 10;
/// History rows shown above remote completions while typing.
const HISTORY_WHILE_TYPING: usize = 4;
const DEBOUNCE: Duration = Duration::from_millis(220);
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);

/// Submitted searches, newest first, bounded and case-insensitively unique.
#[derive(Default)]
pub struct QueryHistory {
    entries: Vec<String>,
}
impl QueryHistory {
    /// Returns the normalized query when it is admissible plain text.
    pub fn record(&mut self, query: &str) -> Option<String> {
        let query = normalize_search_query(query)?;
        let key = search_query_key(&query);
        self.entries
            .retain(|existing| search_query_key(existing) != key);
        self.entries.insert(0, query.clone());
        self.entries.truncate(MAX_SEARCH_HISTORY);
        Some(query)
    }
    pub fn remove(&mut self, query: &str) -> bool {
        let Some(query) = normalize_search_query(query) else {
            return false;
        };
        let key = search_query_key(&query);
        let before = self.entries.len();
        self.entries
            .retain(|existing| search_query_key(existing) != key);
        before != self.entries.len()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    /// `recent` (newest first) wins over `stored` for duplicate identities.
    pub fn replace(&mut self, recent: &[String], stored: Vec<String>) {
        self.entries.clear();
        for query in stored.iter().rev().chain(recent.iter().rev()) {
            self.record(query);
        }
    }
    pub fn entries(&self) -> &[String] {
        &self.entries
    }
    fn matching(&self, typed: &str, limit: usize) -> impl Iterator<Item = &String> {
        let key = typed.trim_start().to_lowercase();
        self.entries
            .iter()
            .filter(move |entry| search_query_key(entry).starts_with(&key))
            .take(limit)
    }
}

/// Splits `text` after the case-insensitively matching typed prefix.
fn split(text: &str, typed: &str) -> (String, String) {
    let typed = typed.trim_start();
    let mut end = 0;
    let mut expected = typed.chars();
    for (index, actual) in text.char_indices() {
        let Some(wanted) = expected.next() else {
            end = index;
            break;
        };
        if !actual.to_lowercase().eq(wanted.to_lowercase()) {
            return (String::new(), text.to_owned());
        }
        end = index + actual.len_utf8();
    }
    if expected.next().is_some() {
        return (String::new(), text.to_owned());
    }
    // Boundary spaces start the bold run: a trailing space may not be measured.
    let end = text[..end].trim_end().len();
    (text[..end].to_owned(), text[end..].to_owned())
}

/// History rows first, then remote completions that are not already shown.
/// With an empty field only recent history is suggested, in normal weight.
pub fn rows(typed: &str, history: &QueryHistory, remote: &[String]) -> Vec<SearchSuggestion> {
    if typed.trim().is_empty() {
        return history
            .entries()
            .iter()
            .take(MAX_ROWS)
            .map(|text| SearchSuggestion {
                text: text.as_str().into(),
                prefix: text.as_str().into(),
                rest: "".into(),
                history: true,
            })
            .collect();
    }
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    let local = history
        .matching(typed, HISTORY_WHILE_TYPING)
        .map(|t| (t, true));
    let remote = remote.iter().map(|t| (t, false));
    for (text, is_history) in local.chain(remote) {
        if rows.len() == MAX_ROWS {
            break;
        }
        if !seen.insert(text.to_lowercase()) {
            continue;
        }
        let (prefix, rest) = split(text, typed);
        rows.push(SearchSuggestion {
            text: text.as_str().into(),
            prefix: prefix.into(),
            rest: rest.into(),
            history: is_history,
        });
    }
    rows
}

struct Query {
    serial: u64,
    text: String,
}
pub struct Completion {
    serial: u64,
    query: String,
    result: Result<Vec<String>, ProviderError>,
}

/// One background thread with a current-thread runtime. Only the newest query
/// is kept; replacing or clearing it drops the in-flight request future.
pub struct Worker {
    requests: Option<watch::Sender<Option<Query>>>,
    results: Arc<Mutex<Option<Completion>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    /// A disabled worker (offline diagnostics/fixtures) never starts a thread.
    pub fn new(enabled: bool, wake: impl Fn() + Send + 'static) -> Self {
        let results = Arc::new(Mutex::new(None));
        if !enabled {
            return Self {
                requests: None,
                results,
                thread: None,
            };
        }
        let (requests, mut receiver) = watch::channel::<Option<Query>>(None);
        let output = results.clone();
        let thread = thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let Ok(client) = serein_youtube::suggestions::client() else {
                    return;
                };
                let mut cooldown: Option<Instant> = None;
                let mut current: Option<(u64, String)> = None;
                loop {
                    let Some((serial, text)) = current.take() else {
                        if receiver.changed().await.is_err() {
                            break;
                        }
                        current = receiver
                            .borrow_and_update()
                            .as_ref()
                            .map(|query| (query.serial, query.text.clone()));
                        continue;
                    };
                    let result = if cooldown.is_some_and(|until| until > Instant::now()) {
                        Err(ProviderError::RateLimited)
                    } else {
                        tokio::select! {
                            biased;
                            changed = receiver.changed() => {
                                if changed.is_err() { break; }
                                // Superseded or cancelled: drop the request.
                                current = receiver.borrow_and_update().as_ref().map(|query| (query.serial, query.text.clone()));
                                continue;
                            }
                            result = serein_youtube::suggestions::fetch(&client, &text) => result,
                        }
                    };
                    if result == Err(ProviderError::RateLimited) && cooldown.is_none_or(|until| until <= Instant::now()) {
                        cooldown = Some(Instant::now() + RATE_LIMIT_COOLDOWN);
                    }
                    if let Ok(mut slot) = output.lock() {
                        *slot = Some(Completion {
                            serial,
                            query: text,
                            result,
                        });
                    }
                    wake();
                }
            });
        });
        Self {
            requests: Some(requests),
            results,
            thread: Some(thread),
        }
    }
    fn submit(&self, serial: u64, text: String) {
        if let Some(requests) = &self.requests {
            requests.send_replace(Some(Query { serial, text }));
        }
    }
    fn cancel(&self) {
        if let Some(requests) = &self.requests
            && requests.borrow().is_some()
        {
            requests.send_replace(None);
        }
    }
    fn take(&self) -> Option<Completion> {
        self.results.lock().ok()?.take()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the channel ends the loop and drops any in-flight request.
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct State {
    worker: Worker,
    history: RefCell<QueryHistory>,
    rows: Rc<slint::VecModel<SearchSuggestion>>,
    /// Remote completions and the exact text they answered.
    remote: RefCell<(String, Vec<String>)>,
    typed: RefCell<String>,
    serial: Cell<u64>,
    debounce: slint::Timer,
    /// Last committed local-history opt-in observed by this feature.
    history_enabled: Cell<Option<bool>>,
    load_serial: Cell<u64>,
    pending_load: Cell<Option<u64>>,
    /// Searches recorded while a stored-history read is outstanding.
    recent_during_load: RefCell<Vec<String>>,
}
impl State {
    pub fn new(worker: Worker) -> Self {
        Self {
            worker,
            history: RefCell::default(),
            rows: Rc::new(slint::VecModel::default()),
            remote: RefCell::default(),
            typed: RefCell::default(),
            serial: Cell::new(0),
            debounce: slint::Timer::default(),
            history_enabled: Cell::new(None),
            load_serial: Cell::new(0),
            pending_load: Cell::new(None),
            recent_during_load: RefCell::default(),
        }
    }
    fn supersede(&self) -> u64 {
        self.debounce.stop();
        let serial = self.serial.get().wrapping_add(1);
        self.serial.set(serial);
        serial
    }
}

fn remote_enabled(app: &App, state: &UiState) -> bool {
    crate::library_ui::desired_preferences(state).search_suggestions
        && !app.get_native_video_child()
}

fn publish(app: &App, state: &UiState) {
    let s = &state.search_suggestions;
    let typed = s.typed.borrow().clone();
    let remote = {
        let (answered, remote) = &*s.remote.borrow();
        if !remote_enabled(app, state) || typed.trim().is_empty() {
            Vec::new()
        } else if answered == typed.trim() {
            remote.clone()
        } else {
            // Keep only still-matching completions until the new answer arrives.
            remote
                .iter()
                .filter(|text| !split(text, &typed).0.is_empty())
                .cloned()
                .collect()
        }
    };
    let rows = rows(&typed, &s.history.borrow(), &remote);
    let ui = app.global::<SearchSuggestionsUi>();
    if ui.get_highlighted() >= rows.len() as i32 {
        ui.set_highlighted(-1);
    }
    s.rows.set_vec(rows);
}

/// Edited or focused field text.
fn query(app: &App, state: &Rc<UiState>, text: &str) {
    let s = &state.search_suggestions;
    *s.typed.borrow_mut() = text.to_owned();
    let serial = s.supersede();
    publish(app, state);
    let trimmed = text.trim().to_owned();
    if !remote_enabled(app, state)
        || !app.get_search_active()
        || serein_youtube::suggestions::request_url(&trimmed).is_none()
    {
        s.worker.cancel();
        return;
    }
    if s.remote.borrow().0 == trimmed {
        // Already answered (e.g. typed back); drop any other in-flight query.
        s.worker.cancel();
        return;
    }
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    s.debounce
        .start(slint::TimerMode::SingleShot, DEBOUNCE, move || {
            let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
                return;
            };
            let s = &state.search_suggestions;
            if s.serial.get() != serial
                || !app.get_search_active()
                || !remote_enabled(&app, &state)
                || s.typed.borrow().trim() != trimmed
            {
                return;
            }
            s.worker.submit(serial, trimmed.clone());
        });
}

fn close(state: &UiState) {
    let s = &state.search_suggestions;
    s.supersede();
    s.worker.cancel();
}

fn completed(app: &App, state: &UiState) {
    let s = &state.search_suggestions;
    while let Some(completion) = s.worker.take() {
        let ui = app.global::<SearchSuggestionsUi>();
        // Stale, blurred, disabled or while the user is choosing with arrows.
        if completion.serial != s.serial.get()
            || !app.get_search_active()
            || !remote_enabled(app, state)
            || s.typed.borrow().trim() != completion.query
            || ui.get_highlighted() >= 0
        {
            continue;
        }
        // Failures are silent: local history remains available.
        if let Ok(remote) = completion.result {
            *s.remote.borrow_mut() = (completion.query, remote);
            publish(app, state);
        }
    }
}

/// Call for every submitted search input. Only plain text is remembered;
/// storage additionally requires the committed local-history opt-in.
pub fn record(app: &App, state: &UiState, query: &str) {
    let s = &state.search_suggestions;
    let Some(query) = s.history.borrow_mut().record(query) else {
        return;
    };
    if s.pending_load.get().is_some() {
        s.recent_during_load.borrow_mut().insert(0, query.clone());
    }
    if state.preferences.get().privacy.local_history && !state.caption_cache.active() {
        // Best effort; SQLite rechecks the opt-in inside the insert.
        let _ = state.library.submit(library::Request::RecordSearch(query));
    }
    close(state);
    publish(app, state);
}

fn remove(app: &App, state: &UiState, query: &str) {
    let s = &state.search_suggestions;
    s.recent_during_load
        .borrow_mut()
        .retain(|recent| search_query_key(recent) != search_query_key(query));
    if s.history.borrow_mut().remove(query)
        && s.history_enabled.get() == Some(true)
        && !state.caption_cache.active()
        && !state
            .library
            .submit(library::Request::DeleteSearch(query.to_owned()))
    {
        app.set_status("The library is busy. The search may reappear after restarting.".into());
    }
    app.global::<SearchSuggestionsUi>().set_highlighted(-1);
    publish(app, state);
}

fn forget(app: &App, state: &UiState) {
    let s = &state.search_suggestions;
    s.history.borrow_mut().clear();
    s.recent_during_load.borrow_mut().clear();
    s.pending_load.set(None);
    publish(app, state);
}

fn load(state: &UiState) {
    let s = &state.search_suggestions;
    let ticket = s.load_serial.get().wrapping_add(1);
    s.load_serial.set(ticket);
    s.recent_during_load.borrow_mut().clear();
    if state
        .library
        .submit(library::Request::SearchHistory { ticket })
    {
        s.pending_load.set(Some(ticket));
    }
}

/// A stored-history read replaces the list; newer session searches win.
pub fn loaded(app: &App, state: &UiState, ticket: u64, result: Result<Vec<String>, String>) {
    let s = &state.search_suggestions;
    if s.pending_load.get() != Some(ticket) {
        return;
    }
    s.pending_load.set(None);
    let recent = std::mem::take(&mut *s.recent_during_load.borrow_mut());
    match result {
        Ok(stored) if s.history_enabled.get() == Some(true) => {
            s.history.borrow_mut().replace(&recent, stored);
        }
        Ok(_) => {}
        Err(error) => app.set_status(format!("{error} Search history is unavailable.").into()),
    }
    publish(app, state);
}

/// Call wherever committed/desired preferences change (same sites as comments).
pub fn sync_preferences(app: &App, state: &UiState) {
    let s = &state.search_suggestions;
    let desired = crate::library_ui::desired_preferences(state);
    let ui = app.global::<SearchSuggestionsUi>();
    ui.set_youtube_enabled(desired.search_suggestions);
    if !desired.search_suggestions {
        s.supersede();
        s.worker.cancel();
        *s.remote.borrow_mut() = Default::default();
    }
    let enabled = state.preferences.get().privacy.local_history;
    match (s.history_enabled.replace(Some(enabled)), enabled) {
        // SQLite deleted stored searches with the disable; forget them too.
        (Some(true), false) => forget(app, state),
        (None | Some(false), true) => {
            forget(app, state);
            load(state);
        }
        _ => publish(app, state),
    }
}

/// History retention changed: stored rows may have been pruned.
pub fn reload(state: &UiState) {
    if state.search_suggestions.history_enabled.get() == Some(true) {
        load(state);
    }
}

/// "Clear history" was accepted by the library worker queue.
pub fn history_cleared(app: &App, state: &UiState) {
    forget(app, state);
}

/// Committed "Clear local data" (history opt-in is reset to off).
pub fn local_data_cleared(app: &App, state: &UiState) {
    close(state);
    *state.search_suggestions.remote.borrow_mut() = Default::default();
    forget(app, state);
    sync_preferences(app, state);
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let ui = app.global::<SearchSuggestionsUi>();
    ui.set_rows(state.search_suggestions.rows.clone().into());
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_query(move |text| {
        if let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) {
            query(&app, &state, &text);
        }
    });
    let owner = Rc::downgrade(state);
    ui.on_closed(move || {
        if let Some(state) = owner.upgrade() {
            close(&state);
        }
    });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_remove(move |text| {
        if let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) {
            remove(&app, &state, &text);
        }
    });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_wake(move || {
        if let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) {
            completed(&app, &state);
        }
    });
    let weak = app.as_weak();
    let owner = Rc::downgrade(state);
    ui.on_set_youtube_enabled(move |enabled| {
        let (Some(app), Some(state)) = (weak.upgrade(), owner.upgrade()) else {
            return;
        };
        if crate::library_ui::set_search_suggestions(&app, &state, enabled) {
            sync_preferences(&app, &state);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(rows: &[SearchSuggestion]) -> Vec<(String, bool)> {
        rows.iter()
            .map(|row| (row.text.to_string(), row.history))
            .collect()
    }

    #[test]
    fn session_history_is_normalized_deduplicated_and_bounded() {
        let mut history = QueryHistory::default();
        assert_eq!(history.record("  rust   lang "), Some("rust lang".into()));
        assert_eq!(history.record("RUST LANG"), Some("RUST LANG".into()));
        assert_eq!(history.entries(), ["RUST LANG"]);
        for rejected in [
            "",
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ&si=token",
            "youtu.be/aqz-KE-bpKQ",
            "bad\u{7}",
        ] {
            assert_eq!(history.record(rejected), None);
        }
        for index in 0..70 {
            history.record(&format!("synthetic {index}"));
        }
        assert_eq!(history.entries().len(), MAX_SEARCH_HISTORY);
        assert_eq!(history.entries()[0], "synthetic 69");
        assert!(history.remove("SYNTHETIC 69"));
        assert!(!history.remove("synthetic 69"));
        assert_eq!(history.entries()[0], "synthetic 68");
        history.clear();
        assert!(history.entries().is_empty());
    }

    #[test]
    fn stored_history_replacement_keeps_newer_session_searches_first() {
        let mut history = QueryHistory::default();
        history.record("stale");
        history.replace(
            &["newest".into(), "Stored B".into()],
            vec!["stored b".into(), "stored a".into()],
        );
        assert_eq!(history.entries(), ["newest", "Stored B", "stored a"]);
        let stored: Vec<String> = (0..80).map(|index| format!("stored {index}")).collect();
        history.replace(&[], stored);
        assert_eq!(history.entries().len(), MAX_SEARCH_HISTORY);
        assert_eq!(history.entries()[0], "stored 0");
    }

    #[test]
    fn rows_put_matching_history_first_and_bold_only_the_completion() {
        let mut history = QueryHistory::default();
        for query in ["rust book", "python", "Rust Lang"] {
            history.record(query);
        }
        let remote: Vec<String> = ["rust lang", "rust language", "Rüst", "rust game"]
            .map(String::from)
            .into();
        let shown = rows("RUST ", &history, &remote);
        assert_eq!(
            texts(&shown),
            [
                ("Rust Lang".into(), true),
                ("rust book".into(), true),
                ("rust language".into(), false),
                ("Rüst".into(), false),
                ("rust game".into(), false),
            ]
        );
        assert_eq!(
            (shown[0].prefix.as_str(), shown[0].rest.as_str()),
            ("Rust", " Lang")
        );
        // A non-matching completion is shown entirely in bold.
        assert_eq!(
            (shown[3].prefix.as_str(), shown[3].rest.as_str()),
            ("", "Rüst")
        );
        let many: Vec<String> = (0..20).map(|index| format!("rust {index}")).collect();
        assert_eq!(rows("rust", &history, &many).len(), MAX_ROWS);
    }

    #[test]
    fn empty_field_suggests_recent_history_in_normal_weight() {
        let mut history = QueryHistory::default();
        assert!(rows("", &history, &["remote".into()]).is_empty());
        for index in 0..15 {
            history.record(&format!("synthetic {index}"));
        }
        let shown = rows("   ", &history, &["remote".into()]);
        assert_eq!(shown.len(), MAX_ROWS);
        assert!(shown.iter().all(|row| row.history && row.rest.is_empty()));
        assert_eq!(shown[0].prefix, "synthetic 14");
    }

    #[test]
    fn prefix_split_is_case_insensitive_and_char_boundary_safe() {
        assert_eq!(
            split("Příliš žluťoučký", "PŘÍ"),
            ("Pří".into(), "liš žluťoučký".into())
        );
        assert_eq!(split("ab", "abc"), (String::new(), "ab".into()));
        assert_eq!(split("ab", "ab"), ("ab".into(), String::new()));
        assert_eq!(split("日本語", "日本"), ("日本".into(), "語".into()));
    }

    #[test]
    fn disabled_worker_never_starts_or_answers() {
        let worker = Worker::new(false, || panic!("offline worker must not wake"));
        worker.submit(1, "synthetic".into());
        worker.cancel();
        assert!(worker.take().is_none());
        assert!(worker.thread.is_none());
    }

    #[test]
    fn superseding_and_cancelling_an_idle_enabled_worker_is_prompt() {
        // No query is submitted, so no network request can be made.
        let worker = Worker::new(true, || {});
        worker.cancel();
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
