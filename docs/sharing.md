# Sharing a video link

The watch page's Share action opens a shared Slint popup with a selectable,
read-only address, Copy video link, and Copy at current time. Opening or copying
does not make a network request or open a browser.

Links are built from the normalized `VideoId` as
`https://www.youtube.com/watch?v=VIDEO_ID`. They never contain a resolved media
URL, session material, video title, or account identity. Sharing an account video
requires the same explicit action; a public address does not grant access to a
private or restricted video. Local clips cannot be shared through this action.

The popup pins the video ID, active native load ID, and account session generation
at opening. Copy requests check the same selection and valid account lease.
Changing the video clears the popup synchronously; account expiry/disconnection
clears any pinned account link before media metadata is removed. Media state
notifications also retire stale load/session pins. Sharing is unavailable while
loading, stopping, or showing stale metadata.

Timestamp copying uses the player's observed position only on the user's copy
request. It requires a stable playback clock and finite position/duration with a
known positive duration. Seconds are rounded down and clamped to zero, duration,
and the unsigned 32-bit limit. Seeking in progress and unknown-duration
media produce a useful unavailable-time message; the ordinary link remains
available. Live/DVR timestamp semantics remain unqualified. No new clock or
polling timer was added.

The pinned Slint source exposes `LineEdit.select-all()` and `LineEdit.copy()`;
the latter requests a platform clipboard write without returning success or an
error. The popup therefore says **Copy requested**, and offers the selectable
address as a fallback, instead of claiming verified clipboard success. The
clipboard may be visible to other applications or an OS clipboard history.

Popup focus uses Slint's `PopupWindow.forward-focus` contract with its internal
LineEdit. Explicit Close returns focus to the stable video focus scope. The
popup participates in the existing native-overlay and control-visibility rules.
Native clipboard behavior, keyboard/focus interaction, and screen-reader
qualification remain pending; compilation is not evidence for those gates.
