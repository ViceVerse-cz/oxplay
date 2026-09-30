// SPDX-License-Identifier: GPL-3.0-or-later
//! Watch-page like/dislike pill. The public like count comes from the current
//! video's details and is shown to everyone. A connected identity's own rating
//! is read without a write, and changed only by an explicit click that sends
//! one reconciled mutation. YouTube publishes no dislike count.
use crate::{App, UiState, display_format};
use oxplay_core::VideoId;
use oxplay_youtube::account::{AccountMutation, MutationOutcome, VideoRating};
use std::cell::{Cell, RefCell};

#[derive(Clone)]
struct Write {
    video: VideoId,
    /// Last confirmed rating, restored when the write definitely failed.
    previous: Option<VideoRating>,
    desired: VideoRating,
}

#[derive(Default)]
pub struct State {
    /// Public like count from the current video's details.
    count: Cell<Option<u64>>,
    /// The first rating read for this video. The public count already
    /// includes it, so only later changes adjust the displayed count.
    baseline: Cell<Option<VideoRating>>,
    /// Confirmed, or optimistically requested, rating. `None` is unknown.
    current: Cell<Option<VideoRating>>,
    write: RefCell<Option<Write>>,
}
impl State {
    /// Unknown rating and no unresolved rating write.
    pub fn wants_read(&self) -> bool {
        self.current.get().is_none() && self.write.borrow().is_none()
    }
}

/// What a click asks for: the same button again removes that rating.
fn desired(current: Option<VideoRating>, like: bool) -> VideoRating {
    let clicked = if like {
        VideoRating::Like
    } else {
        VideoRating::Dislike
    };
    if current == Some(clicked) {
        VideoRating::None
    } else {
        clicked
    }
}

fn displayed_count(
    count: Option<u64>,
    baseline: Option<VideoRating>,
    current: Option<VideoRating>,
) -> Option<u64> {
    let count = count?;
    let (Some(baseline), Some(current)) = (baseline, current) else {
        return Some(count);
    };
    Some(
        match (baseline == VideoRating::Like, current == VideoRating::Like) {
            (false, true) => count.saturating_add(1),
            (true, false) => count.saturating_sub(1),
            _ => count,
        },
    )
}

fn present(app: &App, state: &UiState) {
    let s = &state.account_ui.rating;
    let current = s.current.get();
    app.set_rating_known(current.is_some());
    app.set_account_liked(current == Some(VideoRating::Like));
    app.set_account_disliked(current == Some(VideoRating::Dislike));
    app.set_watch_like_count(
        displayed_count(s.count.get(), s.baseline.get(), current)
            .map(display_format::compact_count)
            .unwrap_or_default()
            .into(),
    );
}

/// A different video, playback mode or identity: forget the account rating.
/// The public count is owned by the details presentation.
pub fn clear(app: &App, state: &UiState) {
    let s = &state.account_ui.rating;
    s.baseline.set(None);
    s.current.set(None);
    s.write.borrow_mut().take();
    state.account_ui.forget_rating_read();
    present(app, state);
}

pub fn set_count(app: &App, state: &UiState, count: Option<u64>) {
    state.account_ui.rating.count.set(count);
    present(app, state);
}

/// A rating read already matched to the current video and account generation.
pub fn observed(app: &App, state: &UiState, rating: VideoRating) {
    let s = &state.account_ui.rating;
    if s.write.borrow().is_some() {
        return;
    }
    if s.baseline.get().is_none() {
        s.baseline.set(Some(rating));
    }
    s.current.set(Some(rating));
    present(app, state);
}

/// Optimistically show the requested state and return the one write to submit.
/// The caller disables both buttons while the account worker is busy.
pub fn request(app: &App, state: &UiState, video: &VideoId, like: bool) -> AccountMutation {
    let s = &state.account_ui.rating;
    let previous = s.current.get();
    let rating = desired(previous, like);
    *s.write.borrow_mut() = Some(Write {
        video: video.clone(),
        previous,
        desired: rating,
    });
    s.current.set(Some(rating));
    present(app, state);
    AccountMutation::Rating {
        video_id: video.clone(),
        rating,
    }
}

/// Whether a rating write is awaiting its (or its reconciliation's) outcome.
pub fn write_pending(state: &UiState) -> bool {
    state.account_ui.rating.write.borrow().is_some()
}

/// Apply a mutation or reconciliation outcome for the pending rating write.
/// `Err(true)` means the outcome is unknown; `Err(false)` means it failed.
pub fn finished(app: &App, state: &UiState, outcome: Result<MutationOutcome, bool>) {
    let s = &state.account_ui.rating;
    let Some(write) = s.write.borrow().clone() else {
        return;
    };
    let current = state.current_video.borrow().as_ref().map(|v| v.id.clone());
    if current.as_ref() != Some(&write.video) {
        s.write.borrow_mut().take();
        return;
    }
    match outcome {
        Ok(MutationOutcome::Verified) => {
            s.write.borrow_mut().take();
            s.current.set(Some(write.desired));
        }
        // Keep the write for explicit reconciliation, but do not claim a state.
        Ok(MutationOutcome::NeedsReconciliation) | Err(true) => s.current.set(None),
        Err(false) => {
            s.write.borrow_mut().take();
            s.current.set(write.previous);
        }
    }
    if s.baseline.get().is_none() {
        // A write without a prior read has no reliable count baseline.
        s.baseline.set(s.current.get());
    }
    present(app, state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use VideoRating::{Dislike, Like, None as Neutral};
    #[test]
    fn a_click_toggles_its_own_rating_and_switches_the_other() {
        assert_eq!(desired(Some(Neutral), true), Like);
        assert_eq!(desired(Some(Like), true), Neutral);
        assert_eq!(desired(Some(Dislike), true), Like);
        assert_eq!(desired(Some(Neutral), false), Dislike);
        assert_eq!(desired(Some(Dislike), false), Neutral);
        assert_eq!(desired(Some(Like), false), Dislike);
        // Unknown state never guesses a removal.
        assert_eq!(desired(None, true), Like);
        assert_eq!(desired(None, false), Dislike);
    }
    #[test]
    fn displayed_count_moves_by_one_relative_to_the_read_baseline() {
        assert_eq!(displayed_count(None, Some(Neutral), Some(Like)), None);
        assert_eq!(displayed_count(Some(10), None, None), Some(10));
        assert_eq!(
            displayed_count(Some(10), Some(Neutral), Some(Neutral)),
            Some(10)
        );
        assert_eq!(
            displayed_count(Some(10), Some(Neutral), Some(Like)),
            Some(11)
        );
        assert_eq!(
            displayed_count(Some(10), Some(Dislike), Some(Like)),
            Some(11)
        );
        assert_eq!(displayed_count(Some(10), Some(Like), Some(Like)), Some(10));
        assert_eq!(
            displayed_count(Some(10), Some(Like), Some(Neutral)),
            Some(9)
        );
        assert_eq!(
            displayed_count(Some(10), Some(Like), Some(Dislike)),
            Some(9)
        );
        // An unknown outcome shows the public count unadjusted.
        assert_eq!(displayed_count(Some(10), Some(Neutral), None), Some(10));
        assert_eq!(displayed_count(Some(0), Some(Like), Some(Neutral)), Some(0));
        assert_eq!(
            displayed_count(Some(u64::MAX), Some(Neutral), Some(Like)),
            Some(u64::MAX)
        );
    }
}
