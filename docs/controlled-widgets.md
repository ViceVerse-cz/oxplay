# Acknowledged shared controls

Slint's fluent ComboBox assigns both `current-index` and `current-value` before
calling `selected`. An ordinary one-way binding can therefore stop following
Rust state after the first user edit; a rejected change can also leave the label
and index disagreeing after deferred change handlers run. `ConfirmedChoice`
keeps an independent acknowledged index/value and restores both after the
request callback. Later acknowledgements remain authoritative.

The caption, stream quality, Appearance, search type and channel content
selectors now use this shared component. Caption selection reflects native
observation, rather than successful file download or command admission. Search
and channel filters follow accepted navigation, including returning to an older
route. The Rust filter callbacks also reject invalid indices, unrelated catalog
scopes, active work, native-child mode and local-data clearing before mutation.
Quality requests no longer change the app's quality property through a
two-way binding before the replacement path accepts them.

Appearance previews only admitted preference writes. A rejected request retains
the prior accepted choice. A matching persistence failure restores the committed
preference; an older response cannot overwrite a newer admitted preview. The
existing single SQLite worker supplies ordered committed outcomes. Volume saves
preserve the stored theme intent; a diagnostic `--ui-theme` override does not
enter SQLite through the volume debounce. Unrelated preference completions do
not erase that session-only override. Local Save
uses a separate per-dialog destination draft over the frozen destination list:
changing it does not save a video, a fresh dialog resets it, and reopening an
in-flight save preserves it while the selector is disabled.

## Regression scope

`cargo test -p oxplay --test controlled_widgets --locked` exercises the actual
compiled shared `App` with Slint's same-revision mock backend. Five cases check
rejected and delayed acknowledgements, later rollback, changing caption labels,
quality state, and Save draft/reset behavior. Callbacks in these tests are
synthetic; they do not qualify remote requests, SQL persistence, native caption
commands, OS keyboard delivery or a screen reader. The existing Rust worker and
media tests cover their separate contracts.

No pixels are rendered by this backend. Development builds include the required
Slint element metadata; release builds omit it and this test target. No testing
backend, extra renderer or interpreter enters the production dependency graph.
These tests do not replace native visual or platform qualification and are not
performance/usage benchmarks.

The integrated local suite passed 443 Rust tests (four explicitly ignored
integrations), all 163 Python tooling tests, formatting, strict Clippy and locked
debug/release workspace builds. The five shared-widget cases run automatically
in the ordinary debug CI test command; no additional renderer features or online
service are required. Native guest-caption and PiP release evidence is recorded
in their separate integration notes, with its narrower platform scope.
