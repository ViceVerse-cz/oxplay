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

## Video card menu

Right-clicking a video card (Home, search results, public channel/playlist pages
and the watch page's related list), or pressing the Menu key or Shift+F10 on a
focused card, opens a shared Slint context menu: **Open in new tab**, **Copy
link**, **Copy title**, **Copy thumbnail**, **Open channel**, **Add to local
playlist** and **Download**. Channel and playlist cards have no menu. Menu item titles are their
accessible labels. `crates/app/src/card_menu.rs` handles every command; it
re-reads the card's public catalog item by surface and index at activation and
reports "That video is no longer shown" if the page changed.

- **Copy link** writes `https://www.youtube.com/watch?v=VIDEO_ID`, built by the
  same `VideoId::watch_url` as the Share popup.
- **Copy title** writes the displayed title text.
- **Copy thumbnail** writes the card's already-decoded pixels (at most 320×180,
  as displayed) as an image. It never fetches a larger image. If the artwork is
  not decoded yet or the platform rejects the image, it writes the unsigned
  public address `https://i.ytimg.com/vi/VIDEO_ID/hqdefault.jpg`, derived only
  from the ID, and says so. Provider thumbnail URLs are never copied.
- **Open channel** is enabled only when the provider supplied a channel ID. It
  opens the public channel page with the same admission as a channel card.
- **Add to local playlist** opens the existing Save dialog for that card's
  video. See [local playlist save](local-playlist-save.md).
- **Download** queues an explicit guest download of that video at the default
  quality ceiling. See [downloads](downloads.md).

Unlike the popup's `LineEdit.copy()`, these writes go through `arboard`, which
returns the platform's result. A short toast ("Link copied", "Thumbnail
copied", "Image unavailable — copied the thumbnail link", or a "Couldn't copy"
failure) appears above the page for about two seconds. It is a polite
accessible live region. The success message means the platform accepted the
write; it is not a paste verification. The clipboard instance stays alive for
the session so X11/Wayland pastes can be served. Copied artwork is an explicit
export; for account recommendation rows, whose artwork is otherwise memory-only,
it becomes visible to other applications and OS clipboard history.

Unit tests cover image-first copying, the address fallback and its message,
failure reporting, and pixel-buffer size validation with a fake clipboard. A
controlled-widget test covers menu routing for both surfaces, the disabled
Open channel entry, the Menu key and Shift+F10, Save-dialog opening, and the
toast's live region. No native clipboard write, paste into another application
or screen reader has been validated.
