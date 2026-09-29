# Architecture of the current slice

`crates/app`: compiled shared Slint components, stable row model, UI-owned state,
weak callback handoffs, single worker with a one-item replaceable request slot.
`crates/core`: typed identifiers, short-lived redacted media addresses, operation
cancellation/generation, privacy defaults and structured provider failures.
`crates/youtube`: isolated yt-dlp invocation, bounded pipe drains/process supervision,
normalized genuine search/stream results and promoted metadata filtering.
`crates/media`: audited minimal C ABI, one in-process libmpv engine, async commands,
coalesced wakeups, observed properties and context-bound persistent GL targets.
`crates/storage`: local SQLite collections/preferences, migration and bounded queries;
not yet connected to the browsing UI.

No Slint, graphics handles, credentials or SQL enter core types. No decoded pixels
enter application messages. Worker results cross `Weak::upgrade_in_event_loop`.
The media update callback only signals; rendering occurs in BeforeRendering with
the window's context current. No permanent frame timer requests redraws.

Progress requests run at 250 ms only while playing with controls visible and the
window not occluded. Elapsed labels change at displayed-second boundaries. The
catalog model is assigned once; result replacement is an intentional reset,
pagination appends bounded rows, and row updates emit one targeted notification.
Pointer positions are not global state. GPU drawing can still cover the whole
window; these rules do not promise partial GPU redraw.

Account integration remains a separate milestone, described concretely in
account-provider.md. No cookie parsing success is presented as account identity.
