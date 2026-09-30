# Watch tabs

Several public videos can be kept ready in session-only, browser-like tabs.
Opening a video normally still replaces the current tab's video. To keep one
ready without interrupting playback, middle-click it, Cmd/Ctrl+click it, choose
**Open in new tab** from its right-click menu, or press Cmd/Ctrl+Return on a
focused card. This works on the shared video cards: Home, search results,
public channel/playlist pages and the related list. Channel and playlist cards
are not tabs. The new tab is added at the right end in the background and shows
the card's title. A video that is already open is not duplicated.

## Strip

A slim 36px strip sits directly below the custom header, so the 48px header,
its AppKit title-bar integration and the centred search field are unchanged.
Chips are compact and thumbnail-less: an elided title, a close ×, and a raised,
bold active chip with a small red play mark while its video is presented. The
full title and creator appear as a tooltip. Chips shrink to 96–220px and the
strip scrolls sideways when full, keeping the active chip in view. Middle-click
closes a chip. Empty strip space drags the window like the header.

The strip is shown for two or more tabs, or for one tab the player is not
showing (after Close player with other tabs open, or during account/local
playback). A single presented tab would only repeat Now playing and the
mini-player, so it reserves no space. Fullscreen and picture in picture hide the
strip. There is no "+" button: new tabs come from video cards.

## Playback and positions

There is one libmpv player, load and presentation context. A background tab
holds only its typed video ID, title, creator and a whole-second resume point;
no stream address is kept and nothing is pre-resolved. The catalog worker has
one latest-request slot, so pre-resolving would cancel foreground work, and
signed stream addresses expire anyway. Switching tabs submits the ordinary
guest resolve at the remembered point (`ResolveAt`), then follows the usual
opening path: immediate watch page with the tab's title, stop barrier, loading
skeletons, retry/recovery and focus intent.

The leaving tab's position is read with the player's tokened resume-position
query, because routine progress polling pauses while controls are hidden. Stop
would invalidate that reply, so the switch waits for it, at most 600ms, and then
uses the last observed position. Leaving within three seconds of the end resumes
from the start. A tab left while it was still loading keeps its previous
point. A switch requested while one is pending retargets it.

Rapid switching uses existing ownership: each switch replaces the worker
request and watch-loading generation, so a late result for an earlier tab
cannot publish its metadata or media. Accepted metadata fills the active chip.

## Closing and keys

Closing the active tab opens its right neighbour, or the left one at the end.
Closing the last tab stops playback like Close player, cancels any in-flight
resolve and returns from the watch page to the page it was entered from:
retained browse results, the library, Settings or account. Close player in the
mini-player dismisses only the presented tab and leaves the others in the
background without starting one.

Ctrl+Tab and Ctrl+Shift+Tab cycle through the tabs (Control on macOS as well;
Cmd+Tab belongs to the system). Cmd/Ctrl+W closes the active tab. These chords
were unused and also work while typing, because editors do not consume them.
The strip allows at most 12 tabs. Further opens show a short "Tab limit
reached" notice in the strip. With 12 tabs and none active, a normal selection
reuses the rightmost tab rather than failing.

Account and local-file playback are not tabs. Starting either leaves the
current tab in the background with its last observed position (best effort,
without the exact query). Clicking the tab returns to guest playback. Tabs are
not persisted. Clear local data removes them.

## Validation and limits

Pure-Rust tests cover replacement, background opens, deduplication, the bound,
neighbour selection, cycling, detaching, metadata and resume points. A compiled
Slint regression uses the mock backend. It checks strip visibility and header
space, chip activate/close/middle-close, Ctrl+Tab/Ctrl+Shift+Tab/Ctrl+W
routing, including while an editor has focus and not in fullscreen, and card
middle-click, Ctrl+click and context-menu opens. It also checks that a plain
click still selects and that channel cards never open tabs. No native media
switch, AppKit rendering, screen reader or performance check was run. Tab chips
are not Tab-key focus stops; keyboard users have Ctrl+Tab and Cmd/Ctrl+W.
Library rows (local playlists/history) do not offer Open in new tab. A switched
tab resumes playing even if it was paused when left.
