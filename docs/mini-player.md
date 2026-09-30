# In-app mini-player and retained watch context

Opening Home, local collections, Settings, account screens or a public channel
keeps accepted playback running in a bottom-right mini-player. Returning to Now
playing restores the full watch view and its retained metadata. Browsing does
not send a pause command or recreate the player. Minimize/hidden-window policies,
account expiry and sign-out retain their existing behavior.

The mini-player is 380×213.75 logical pixels at normal sizes, inset 20px from the
right/bottom, and scales down with available width. It moves the same persistent
Slint video host above browsing content. Play/pause, seeking, mute, settings,
global PiP and Return to watch page remain available. Playback keyboard shortcuts
work only with that video focused, so catalog keys/editors retain their input.
Its progress admission uses the same visibility-scoped 250ms scheduler; hidden
controls stop that scheduler. No new polling loop is added.

Global floating PiP is a different mode: it temporarily compacts the existing
window, removes decorations and requests always-on-top. Entering it from the
mini-player first restores the watch scene; exiting returns to that watch page.
There is one window, media load, decoder and presentation context throughout.
The borrowed texture never crosses windows. The restricted native-child
experiment does not enable the mini-player.

`watch_context.rs` retains up to 20 typed guest catalog items and a stable related
model. These are genuine previously requested catalog entries, not fabricated
recommendations. Browsing updates its own stable model; related selection,
keyboard focus and thumbnail source lookup use the retained typed indices.
Switching surfaces/datasets invalidates thumbnail generations, preventing late
same-index results from reaching another list. Hidden image references are
released while metadata stays available; thumbnail workers/caches remain bounded
and shared. No authenticated/private results are copied into the guest model.

Accepted creator artwork remains one image across navigation. Clicking its
avatar/name explicitly opens the current video's typed public channel ID. It
passes no private video ID, cookies or authenticated metadata into that route.
Missing channel IDs leave the action unavailable. New local/account selections
and explicit local-data clearing retire guest-related context. Sign-out still
clears private watch metadata and stops authenticated playback.

Focused compiled-Slint tests inject pointer input to verify video vs control
activation, PiP drag suppression, mini-player geometry across browsing pages,
retained watch properties/model, settings selection and both creator hit targets.
They use labeled synthetic metadata and mocked commands; no network, native GPU,
media continuity, OS drag or accessibility qualification is implied. Native
functional checks remain outstanding. Performance/usage testing was not run.

## Close, play/pause feedback and buffering (2026-09-30)

The mini-player shows a **Close player** button (top-right, visible with the
controls or while paused). It stops the single media load and retires
watch-scoped captions, chapters, comments, share and metadata state; browsing
and its catalog are untouched, and account playback uses the existing account
teardown. A user play/pause (video click, play button, Space/K) briefly shows
the resulting action's icon in the centre, fading and growing out over 420 ms.
While started playback is stalled on the network (`Buffering`, not paused) a
centre spinner appears; the loading overlay uses the same spinner. Both
animations exist only for those states, so no redraw loop outlives them.
Reduced-motion preferences are not yet observed. `close_player` is covered by a
compiled-Slint test for visibility and dispatch; the stop sequence itself was
not exercised natively because this capture session cannot present video.
