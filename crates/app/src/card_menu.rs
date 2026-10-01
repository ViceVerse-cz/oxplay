// SPDX-License-Identifier: GPL-3.0-or-later
//! Video-card context-menu commands. Every command is an explicit user action
//! on one visible card: copy its canonical link, title or already-decoded
//! thumbnail, open its creator's public channel, save it into a local
//! playlist, or queue an explicit download (see downloads_ui.rs). Nothing
//! here makes a network request or reads a media URL.
use crate::{App, CardAction, CardMenuUi, UiState, VideoRow};
use oxplay_core::{CatalogItem, VideoId, VideoSummary};
use slint::{ComponentHandle, Model, Timer, TimerMode};
use std::{borrow::Cow, cell::RefCell, rc::Rc, time::Duration};

const TOAST_DURATION: Duration = Duration::from_millis(2200);

/// Platform clipboard writes. `true` means the platform accepted the write.
trait Clipboard {
    fn set_text(&mut self, text: &str) -> bool;
    fn set_image(&mut self, width: usize, height: usize, rgba: &[u8]) -> bool;
}

/// The system clipboard, created on first use and kept alive afterwards:
/// on X11/Wayland the owning instance serves pastes from other applications.
#[derive(Default)]
struct SystemClipboard(Option<arboard::Clipboard>);
impl SystemClipboard {
    fn get(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.0.is_none() {
            self.0 = arboard::Clipboard::new().ok();
        }
        self.0.as_mut()
    }
}
impl Clipboard for SystemClipboard {
    fn set_text(&mut self, text: &str) -> bool {
        self.get()
            .is_some_and(|clipboard| clipboard.set_text(text).is_ok())
    }
    fn set_image(&mut self, width: usize, height: usize, rgba: &[u8]) -> bool {
        if width == 0
            || height == 0
            || width.checked_mul(height).and_then(|n| n.checked_mul(4)) != Some(rgba.len())
        {
            return false;
        }
        self.get().is_some_and(|clipboard| {
            clipboard
                .set_image(arboard::ImageData {
                    width,
                    height,
                    bytes: Cow::Borrowed(rgba),
                })
                .is_ok()
        })
    }
}

#[derive(Default)]
pub struct State {
    clipboard: RefCell<SystemClipboard>,
    toast: Timer,
}

/// A stable, unsigned public thumbnail address derived only from the video ID.
/// Provider thumbnail URLs can carry query parameters; they are never copied.
fn thumbnail_address(id: &VideoId) -> String {
    format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id.as_str())
}

#[derive(Debug, PartialEq, Eq)]
enum ThumbnailCopy {
    Image,
    Address,
    Failed,
}

/// Prefer the displayed pixels; fall back to the public image address.
fn copy_thumbnail(
    clipboard: &mut impl Clipboard,
    pixels: Option<(usize, usize, &[u8])>,
    id: &VideoId,
) -> ThumbnailCopy {
    if let Some((width, height, rgba)) = pixels
        && clipboard.set_image(width, height, rgba)
    {
        return ThumbnailCopy::Image;
    }
    if clipboard.set_text(&thumbnail_address(id)) {
        ThumbnailCopy::Address
    } else {
        ThumbnailCopy::Failed
    }
}

fn thumbnail_message(result: ThumbnailCopy) -> &'static str {
    match result {
        ThumbnailCopy::Image => "Thumbnail copied",
        ThumbnailCopy::Address => "Image unavailable — copied the thumbnail link",
        ThumbnailCopy::Failed => "Couldn't copy the thumbnail",
    }
}

fn copy_text(
    clipboard: &mut impl Clipboard,
    text: &str,
    done: &'static str,
    failed: &'static str,
) -> &'static str {
    if clipboard.set_text(text) {
        done
    } else {
        failed
    }
}

pub fn toast(app: &App, state: &UiState, text: &str) {
    app.global::<CardMenuUi>().set_toast(text.into());
    let weak = app.as_weak();
    state
        .card_menu
        .toast
        .start(TimerMode::SingleShot, TOAST_DURATION, move || {
            if let Some(app) = weak.upgrade() {
                app.global::<CardMenuUi>().set_toast("".into());
            }
        });
}

/// The public video behind a visible card, with its displayed row when the
/// row still shows that same video. Surfaces follow `TabsUi.open`.
fn target(
    app: &App,
    state: &UiState,
    surface: i32,
    index: i32,
) -> Option<(VideoSummary, Option<VideoRow>)> {
    let index = usize::try_from(index).ok()?;
    let (item, row) = match surface {
        0 if app.get_page() == 0 => (state.guest_ui.item(index), state.model.row_data(index)),
        1 if app.get_page() == 2 => (
            state.watch_context.item(index),
            state.watch_context.model.row_data(index),
        ),
        _ => return None,
    };
    let Some(CatalogItem::Video(video)) = item else {
        return None;
    };
    let row = row.filter(|row| row.id.as_str() == video.id.as_str());
    Some((video, row))
}

fn run(app: &App, state: &Rc<UiState>, surface: i32, index: i32, action: CardAction) -> bool {
    if app.get_native_video_child() {
        return false;
    }
    let Some((video, row)) = target(app, state, surface, index) else {
        toast(app, state, "That video is no longer shown. Try again.");
        return false;
    };
    let message = match action {
        CardAction::CopyLink => copy_text(
            &mut *state.card_menu.clipboard.borrow_mut(),
            &video.id.watch_url(),
            "Link copied",
            "Couldn't copy the link",
        ),
        CardAction::CopyTitle => copy_text(
            &mut *state.card_menu.clipboard.borrow_mut(),
            &video.title,
            "Title copied",
            "Couldn't copy the title",
        ),
        CardAction::CopyThumbnail => {
            // Only already-decoded, displayed pixels (at most 320×180); this
            // never fetches a larger image.
            let pixels = row
                .filter(|row| row.thumbnail_ready)
                .and_then(|row| row.thumbnail.to_rgba8());
            let pixels = pixels.as_ref().map(|buffer| {
                (
                    buffer.width() as usize,
                    buffer.height() as usize,
                    buffer.as_bytes(),
                )
            });
            thumbnail_message(copy_thumbnail(
                &mut *state.card_menu.clipboard.borrow_mut(),
                pixels,
                &video.id,
            ))
        }
        CardAction::OpenChannel => match video.channel_id {
            Some(id) => {
                if crate::guest_ui::open_channel(app, state, id) {
                    return false;
                }
                "Can't open the channel right now. Try again shortly."
            }
            None => "This video's channel isn't known",
        },
        CardAction::AddToPlaylist => match crate::library_ui::begin_card_save(app, state, video) {
            Ok(()) => return true,
            Err(message) => message,
        },
        CardAction::Download => crate::downloads_ui::enqueue(app, state, video),
    };
    toast(app, state, message);
    false
}

pub fn connect(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let s = Rc::downgrade(state);
    app.global::<CardMenuUi>()
        .on_action(move |surface, index, action| {
            let (Some(app), Some(s)) = (weak.upgrade(), s.upgrade()) else {
                return false;
            };
            run(&app, &s, surface, index, action)
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        text_ok: bool,
        image_ok: bool,
        text: Option<String>,
        image: Option<(usize, usize, usize)>,
    }
    impl Clipboard for Fake {
        fn set_text(&mut self, text: &str) -> bool {
            if self.text_ok {
                self.text = Some(text.into());
            }
            self.text_ok
        }
        fn set_image(&mut self, width: usize, height: usize, rgba: &[u8]) -> bool {
            if self.image_ok {
                self.image = Some((width, height, rgba.len()));
            }
            self.image_ok
        }
    }
    fn id() -> VideoId {
        VideoId::new("abcdefghijk").unwrap()
    }

    #[test]
    fn thumbnail_copies_pixels_first_and_never_also_writes_text() {
        let mut clipboard = Fake {
            text_ok: true,
            image_ok: true,
            ..Fake::default()
        };
        let pixels = vec![0u8; 4 * 2 * 3];
        let result = copy_thumbnail(&mut clipboard, Some((2, 3, &pixels)), &id());
        assert_eq!(result, ThumbnailCopy::Image);
        assert_eq!(clipboard.image, Some((2, 3, 24)));
        assert_eq!(clipboard.text, None);
        assert_eq!(thumbnail_message(result), "Thumbnail copied");
    }

    #[test]
    fn thumbnail_falls_back_to_an_unsigned_address_and_reports_it() {
        // Image write rejected by the platform.
        let mut clipboard = Fake {
            text_ok: true,
            ..Fake::default()
        };
        let pixels = vec![0u8; 4];
        assert_eq!(
            copy_thumbnail(&mut clipboard, Some((1, 1, &pixels)), &id()),
            ThumbnailCopy::Address
        );
        assert_eq!(
            clipboard.text.as_deref(),
            Some("https://i.ytimg.com/vi/abcdefghijk/hqdefault.jpg")
        );
        // Artwork not decoded (or evicted): no pixels, same address fallback.
        let mut clipboard = Fake {
            text_ok: true,
            image_ok: true,
            ..Fake::default()
        };
        assert_eq!(
            copy_thumbnail(&mut clipboard, None, &id()),
            ThumbnailCopy::Address
        );
        assert_eq!(clipboard.image, None);
        assert!(thumbnail_message(ThumbnailCopy::Address).contains("link"));
        // Nothing accepted: an explicit failure, never a success message.
        let mut clipboard = Fake::default();
        assert_eq!(
            copy_thumbnail(&mut clipboard, None, &id()),
            ThumbnailCopy::Failed
        );
        assert_eq!(
            thumbnail_message(ThumbnailCopy::Failed),
            "Couldn't copy the thumbnail"
        );
    }

    #[test]
    fn text_copies_report_the_platform_result() {
        let mut clipboard = Fake {
            text_ok: true,
            ..Fake::default()
        };
        assert_eq!(
            copy_text(&mut clipboard, &id().watch_url(), "Link copied", "no"),
            "Link copied"
        );
        assert_eq!(
            clipboard.text.as_deref(),
            Some("https://www.youtube.com/watch?v=abcdefghijk")
        );
        assert_eq!(
            copy_text(
                &mut Fake::default(),
                "Title",
                "Title copied",
                "Couldn't copy the title"
            ),
            "Couldn't copy the title"
        );
    }

    #[test]
    fn system_clipboard_rejects_mismatched_pixel_buffers_without_a_platform_call() {
        // Size validation happens before the clipboard is even created.
        let mut clipboard = SystemClipboard::default();
        assert!(!clipboard.set_image(0, 1, &[]));
        assert!(!clipboard.set_image(2, 2, &[0; 15]));
        assert!(!clipboard.set_image(usize::MAX, 2, &[0; 4]));
        assert!(clipboard.0.is_none());
    }
}
