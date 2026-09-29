# Related-video keyboard focus

The working-tree implementation extends the existing `feed_focus.rs` owner to
the watch page's bounded related-video `ListView`. It is authored after frozen
release `a45ff676` / app SHA-256
`0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9`.
The final working-tree debug build passed the finite native check below. The
corrected app tests passed 139/139, and strict full-workspace all-target Clippy
passed before the final Slint-only parking-enabled correction. That final
correction compiled and passed all 16 native checkpoints. Complete final
checks and release-build qualification remain separate.
That earlier release's observations do not qualify this change.

Home and related rows share the existing catalog, with a maximum of 120 items.
They also share one focus owner and one weak one-shot delivery timer because
only one surface is active. Each handoff includes its surface, kind, provider
ID, index and serial. A callback from the other surface cannot claim a target,
even if its row has the same video ID. Final keyboard/accessibility focus belongs
to the actual card, not an invisible proxy representing the whole list.

The related list uses one-column navigation at its existing 106px row height.
Tab/Shift-Tab leave at list boundaries; arrows, Home/End and PageUp/PageDown
request bounded rows. A persistent parking scope holds focus while a requested
virtual row is constructed. Its ready callback schedules the same weak 1ms
one-shot used by the feed; focus is not assigned during component initialization.
There is no polling loop or playback-clock input to this owner.

Compact watch layouts have two scrolling ancestors. Keyboard reveal first
brings the related viewport into the outer watch viewport, then reveals the row
inside the existing related list. It saves the resulting watch offset for
fullscreen/resize restoration. This moves the same persistent video host; no
second image, player, or presenter is introduced. Fullscreen entry cancels
pending related handoffs because the related list is hidden there.

The pinned Slint source's `WindowInner::move_focus` checks
`is_visible_or_clipped_by_flickable` for Tab navigation
(`internal/core/window.rs`); `internal/core/item_tree.rs` deliberately allows
focusable items clipped by scrollable ancestors. That establishes the source
contract for reaching instantiated offscreen controls, not a native focus pass.
The application handoff remains necessary for rows not yet instantiated.
Slint's ordinary Tab reveal occurs after the card's focus-gained callback and
changes the Flickable offset without emitting the application's `scrolled`
callback. Normal related-card focus therefore uses the same weak one-shot timer
to reveal/save its offset after that stack unwinds, only if the exact identity
is still focused. This prevents subsequent resize/fullscreen restoration from
using an older watch offset; the finite native lifecycle check below passed.

IME input, unrelated keys, pointer presses/wheels and window deactivation cancel
a pending handoff before Slint dispatch. Search/editors retain their existing
input-first handling. An unrelated pointer movement or media clock update has no
focus-owner handler. Page and explicit catalog replacement reset the owner;
thumbnail updates and appends retain identity. The focus owner only reads the
catalog and never emits model notifications. Revealing a new viewport may still
request thumbnails and produce their legitimate incremental row updates.

New authored regressions cover all 120 related rows and Tab boundaries without
focus-induced catalog notifications, cross-surface stale callbacks with identical
video IDs, and identity stability across metadata/thumbnail changes. Existing
feed handoff/cancellation/regroup tests remain. The native check covers the two
watch widths, End/Home/rapid arrows/Tab, search cancellation and fullscreen
restoration. Real screen-reader/IME and popup-focus validation remain separate.
No accessibility, performance, or additional-platform gate is claimed here.

The explicit `--related-focus-check --local <fixture> --demo-related --data-root
<new-absolute-root>` diagnostic admits only the previously published synthetic
MP4 digest through a private, verified copy; it rejects online content, helper
overrides, other diagnostics and an existing profile. Thirty labeled related
rows receive distinct synthetic identities. It overrides online/account callbacks
and checks that no remote thumbnails or account connection occurred. This is
application admission, not an operating-system egress sandbox.

Sixteen finite one-shot checkpoints exercise 1320×860 and 760×600 layouts,
End/rapid arrows, Tab exit/reentry, ordinary Tab from the player through actual
shared controls, fullscreen scroll restoration, search cancellation, and a
fullscreen-retired pending delivery. The 42-second watchdog cannot be extended
by another CLI value. Completion requires every stage; early exit/failure cannot
pass. Visible-card assertions describe logical viewport intersection, not actual
compositor or screen-reader observations. No permanent timer is added to normal
application startup.

The retained first debug attempt (`artifacts/related-focus/attempt-1`) used app
SHA-256 `268b2271205b9668948f1506eff6c4d9528306611daad06d1de0f7adcfc5cf34`
from the modified working tree. It passed wide deferred End/rapid-arrow focus,
Tab exit/reentry, compact ordinary Tab, fullscreen watch-offset restoration and
search cancellation through stage 28. At stage 30 its assertion incorrectly
expected Slint's `changed fullscreen-active` handler to retire the serial inside
the same callback. The corrected harness checks retirement at the next finite
checkpoint; delivery already independently rejects a fullscreen/inactive surface.
The failed run exited normally with a nonzero diagnostic result; its process
group was absent and private fixture copy removed. It is retained as a harness
failure, not represented as a passing native check or a product performance test.

The second attempt (`artifacts/related-focus/attempt-2`, debug SHA-256
`2dc004027259e98a6b251e3c5427415d2be9cd679f1fb7043d1ae98422d61180`)
matched 138 selected source inputs before/after its build. Its later checkpoint
confirmed serial retirement but exposed a focus-state issue: the disabled
related parking `FocusScope` still reported `has-focus` while fullscreen hid
the list. This property alone did not establish which item the window retained. The implementation now transfers only retiring related card or
parking focus to the existing player scope on fullscreen entry, preserving the
saved outer watch offset. Unrelated search/control focus is left alone. The
finite check now requires observed player focus, retired delivery, and both the
Slint and native-window fullscreen states. The second failed run is retained, with normal failure exit, absent process group
and removed private fixture copy.

The third attempt (`artifacts/related-focus/attempt-3`) still failed the parking
flag assertion after explicit player transfer. Pinned upstream inspection found
why: `internal/core/items/input_items.rs:778` returns `FocusIgnored` whenever a
`FocusScope` is disabled, including for `FocusOut`; its `has-focus` property can
therefore remain stale after focus moves. The related parking scope now remains
enabled like the existing feed parking scope, so it can process `FocusOut`.
It remains excluded from Tab/click entry and every Rust admission checks the
active surface. The explicit player transfer is retained; the diagnostic will
report both player and parking focus. The fourth run validated this correction.
The earlier failure does not establish simultaneous actual keyboard focus on two
items.

The fourth run passed all 16 stages on macOS/Apple M1 with the existing embedded
OpenGL video path: wide deferred End and coalesced arrows, Tab boundary/reentry,
760×600 ordinary Tab into the related viewport, watch offset restored after
fullscreen, search cancellation, and a pending handoff retired on a second
fullscreen transition. At the settled retirement checkpoint it recorded
`player=true parking=false serial=0`; both native-window and shared-UI fullscreen
state were checked. Catalog changes remained zero, the single fixture reset was
not repeated, and the media load count stayed one. Exit was zero; the supervisor
confirmed no surviving owned process group and removal of the private copied
fixture. Total harness elapsed time was 44.857 seconds including external display
assertion, startup and cleanup; this is not a performance measurement.

The debug binary SHA-256 was
`9ab3773a031e5f1ab3affd4320bb942112755235bf0a8648a0066968473f8ca6`.
All 138 selected build-input records matched before/after this build. They are
working-tree records against base commit `e1cc653e`, not a claim that the dirty
binary was built from that commit. Later Save-capture CLI refinements are outside
this related-run scope. No human account, remote playback, screen reader, pixel
inspection, A/V-sync or power measurement was performed.

[The evidence manifest](evidence/2026-09-29-related-focus-export.json) hashes the
sanitized passing log/summary, before/after input inventories and all three
retained failed logs/summaries. The raw harness profile path is deliberately not
exported. [The passing log](evidence/2026-09-29-related-focus-native.log) records
every finite checkpoint; the earlier failures explain the changed assertion and
source-informed focus-state correction.


The frozen release subsequently passed the same 16-stage check: source commit
`d78332c23602b1bfaec87688eccba657a208b9be`, application SHA-256
`e32cfa128282c781a80eeba06a8259207b07f54da946056b77185fa04efb88b9`.
The harness verified the frozen build record against its 138 committed-input
inventory before launch, and rechecked both source records and the binary after
cleanup. All checkpoints passed in order, with exactly one terminal marker and
`player=true parking=false serial=0`. Exit was zero after 43.747 seconds including
startup/display assertion/cleanup; both owned app and wake groups were absent,
and the private copied fixture was removed. No helper overrides, existing user
profile or account connection were used. This is release functional evidence,
not a resource, compositor, screen-reader or other-platform qualification.

[Release evidence manifest](evidence/2026-09-29-related-focus-release-export.json)
hashes the passing log, summary, provenance, frozen binary record, source
inventory and harness. The public harness replaces only the host-specific
default repository path with `Path.cwd()`; the manifest retains both its exported
hash and the exact executed original hash recorded in provenance. The earlier
modified-tree attempts remain preserved separately.
