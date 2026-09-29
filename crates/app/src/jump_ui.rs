// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit time navigation pinned to the current native load and authority.
use crate::{App, JumpUi, UiState, account_playback};
use slint::ComponentHandle;
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
pub struct State {
    target: Cell<Option<(u64, Option<u64>)>>,
}
fn context(
    app: &App,
    state: &UiState,
    snapshot: &serein_media::Snapshot,
) -> Option<(u64, Option<u64>)> {
    if !app.get_loaded()
        || app.get_page() != 2
        || app.get_busy()
        || app.get_native_video_child()
        || state.caption_cache.active()
        || snapshot.stop_pending
        || snapshot.failed_load_request_id == Some(snapshot.load_request_id)
        || matches!(
            snapshot.state,
            serein_media::PlaybackState::Idle | serein_media::PlaybackState::Failed
        )
        || snapshot.load_request_id == 0
        || snapshot.load_request_id != snapshot.active_load_request_id
        || !state.player.current_load_is_active()
        || state.player.clock_identity().is_none()
        || !snapshot.duration.is_finite()
        || snapshot.duration <= 0.
    {
        return None;
    }
    let lease = account_playback::authorization(state);
    if lease.as_ref().is_some_and(|lease| {
        !lease.is_valid() || lease.generation() != state.account_ui.session_generation()
    }) {
        return None;
    }
    Some((
        snapshot.load_request_id,
        lease.map(|lease| lease.generation()),
    ))
}
pub fn observe(app: &App, state: &UiState, snapshot: &serein_media::Snapshot) {
    let current = context(app, state, snapshot);
    let ui = app.global::<JumpUi>();
    if ui.get_available() != current.is_some() {
        ui.set_available(current.is_some());
    }
    if state
        .jump_ui
        .target
        .get()
        .is_some_and(|target| Some(target) != current)
    {
        state.jump_ui.target.set(None);
        app.invoke_close_jump();
    }
}
pub fn connect(app: &App, state: &Rc<UiState>) {
    let state_weak = Rc::downgrade(state);
    app.global::<JumpUi>().on_cancel(move || {
        if let Some(state) = state_weak.upgrade() {
            state.jump_ui.target.set(None);
        }
    });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.global::<JumpUi>().on_begin(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return false;
        };
        let Some(target) = context(&app, &state, &state.player.snapshot()) else {
            return false;
        };
        state.jump_ui.target.set(Some(target));
        let position = state.player.snapshot().position.max(0.).floor() as u64;
        let ui = app.global::<JumpUi>();
        ui.set_draft(format!("{}:{:02}", position / 60, position % 60).into());
        ui.set_status("Enter seconds, minutes:seconds, or hours:minutes:seconds.".into());
        true
    });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.global::<JumpUi>().on_submit(move |input| {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return false;
        };
        let ui = app.global::<JumpUi>();
        if state.jump_ui.target.get().is_none()
            || state.jump_ui.target.get() != context(&app, &state, &state.player.snapshot())
        {
            state.jump_ui.target.set(None);
            ui.set_status("Playback changed. Close this dialog and try again.".into());
            return false;
        }
        let Some(position) = parse_time(&input) else {
            ui.set_status("Use a nonnegative time such as 90, 1:30, or 1:02:30.5.".into());
            return false;
        };
        if position > state.player.snapshot().duration {
            ui.set_status("That time is beyond the end of this video.".into());
            return false;
        }
        match state.player.seek(position) {
            Ok(()) => {
                state.jump_ui.target.set(None);
                let _ = state.player.request_progress();
                true
            }
            Err(error) => {
                ui.set_status(error.to_string().into());
                false
            }
        }
    });
}
fn parse_time(input: &str) -> Option<f64> {
    let input = input.trim();
    if input.is_empty() || input.len() > 32 || !input.is_ascii() {
        return None;
    }
    let parts: Vec<_> = input.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut total = 0.;
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        let mut decimal = false;
        if part.is_empty()
            || !part.as_bytes()[0].is_ascii_digit()
            || !part.as_bytes()[part.len() - 1].is_ascii_digit()
        {
            return None;
        }
        for byte in part.bytes() {
            if byte == b'.' && last && !decimal {
                decimal = true;
            } else if !byte.is_ascii_digit() {
                return None;
            }
        }
        let value: f64 = part.parse().ok()?;
        if index > 0 && value >= 60. {
            return None;
        }
        total = total * 60. + value;
    }
    (total.is_finite() && total <= u32::MAX as f64).then_some(total)
}
#[cfg(test)]
mod tests {
    use super::parse_time;
    #[test]
    fn accepts_clock_not_expression_or_ambiguous_fields() {
        for (input, value) in [
            ("90", 90.),
            ("1:30", 90.),
            ("1:02:30.5", 3750.5),
            (" 0 ", 0.),
            ("90:00", 5400.),
        ] {
            assert_eq!(parse_time(input), Some(value));
        }
        for input in [
            "",
            "-1",
            "+1",
            "1e2",
            "NaN",
            "1:60",
            "1:2:60",
            "1.2:03",
            "1:",
            ":2",
            "1:2:3:4",
            ".5",
            "1.",
            "1..2",
            "4294967296",
        ] {
            assert_eq!(parse_time(input), None, "{input}");
        }
    }
}
