# Local library integration

The shared `ui/library.slint` page and `src/library_ui.rs` adapter expose the SQLite worker's local collection, follow, and history operations. These are device-local features. They do not perform YouTube account mutations.

- Collection and video pages contain at most 100 records. Previous-page navigation retains at most 1,024 opaque cursors; each displayed page replaces the preceding page rather than accumulating the entire database. The shared Slint ListView virtualizes visible rows. Collection rename, deletion, video removal, and saving the current resolved video use typed IDs.
- **Search this playlist** filters saved video titles and channel names with
  Apply/Enter and Clear. It searches the selected local playlist on the SQLite
  worker, including entries beyond the current page; it makes no network request.
  Search text is limited to 256 UTF-8 bytes and matched literally, so `%`, `_`
  and quotes are not SQL patterns. SQLite folds ASCII letter case; other Unicode
  text currently matches exactly. Results remain bounded to 100 and use the
  existing keyset cursor. A new filter gets a fresh first page; selection, tab or
  browsing-context changes clear it. The typed draft is separate from the
  acknowledged-results label and playlist Create/Rename draft. Failed queue
  admission or a failed filter read preserves the preceding rows, filter and
  cursors. The focused synthetic storage regression was added but has not been
  executed in this implementation pass; native keyboard/visual qualification of
  these new controls remains open. Historical validation below predates them.
- Local follows retain channel IDs and names. Opening a follow validates the stored canonical channel ID and opens that public channel through the guest catalog; the display name is never substituted as a search query. Malformed stored identities fail closed. The channel page can save a local follow using its genuine returned header metadata. These actions never subscribe the YouTube account.
- History is disabled initially. The UI's opt-in becomes active only after the storage worker acknowledges persistence, and SQLite rechecks the setting when recording. Playing-media notifications record at most once per video/30-second position bucket; this feature adds no periodic timer. The default retention is 30 days, with 7/30/90-day controls. Users can delete individual entries or clear history.
- Collection deletion, history clearing, and local-data clearing require a confirmation in the shared UI. Other library controls are disabled while that confirmation is visible, and collection deletion names the selected collection.
- Import/export/backup use explicit asynchronous native file pickers; selected paths remain PathBuf values without lossy conversion. File reading, serialization, SQLite, and file writing run on the storage worker. Export and backup refuse existing destinations. Import/export transfer only the local interchange format, never account credentials; SQLite backup contains local preferences/history as well as collections.
- Clearing local data does not disconnect the YouTube account or erase protected session storage. Account disconnection does not erase local collections.
- **Duplicate playlist** opens a separate name editor and copies saved metadata
  into a new local collection in one transaction. SQLite performs the copy without
  loading the whole playlist into the UI or downloading media. Only a committed
  new ID is revealed through the existing bounded collection window; failures
  leave neither an empty duplicate nor a partial collection.
- Each saved-video row offers **Copy / Move…**. The organizer pins the source
  playlist and video identity, shows their names, and offers independently paged
  destination playlists (at most 100 per read). The source is excluded and no
  destination is selected implicitly. Copy and Move require their own explicit
  button after choosing a destination. The UI names both playlists and explains
  that Move removes the source entry. A transaction copies/upserts destination
  metadata before deleting the source membership; any failure rolls both back.
  An already-saved destination video is updated, not duplicated. The current
  filtered source page refreshes only after commit, with no optimistic removal.
  Leaving/reopening the local view retires the organizer draft; accepted writes
  still complete, but stale destination responses cannot revive a closed form.
  These operations change local collections only, never account collections.

The duplication/transfer source pass added focused synthetic storage regressions
for membership isolation, conflict updates, missing identities and rollback at
both insertion and deletion. They were authored but not executed during this
pass. Native keyboard/visual validation of the organizer is still open; older
functional evidence does not qualify these newly added controls.

- Volume changes from the shared slider/keyboard controls persist after 250 ms
  without another change. Closing the event loop flushes a pending debounce.
  Accepted local writes drain during shutdown even after the result receiver
  closes; a regression test fills the bounded queue and checks the final saved
  value after reopening the database. SQLite work remains on its worker.

The surrounding controls can scroll at the minimum window size; the nested row ListView keeps its own bounded viewport. Native visual/keyboard validation of the new page remains to be recorded separately from compilation and worker tests. There is no claim that these source-level bounds alone prove the large-library performance gate.

Validation on the available macOS host: `cargo check --locked -p serein` passed; `cargo test --locked -p serein library` passed all four focused tests (worker paging/mutations, history opt-in/local clear, explicit safe transfer destinations, bounded backward navigation); `cargo clippy --locked -p serein --all-targets -- -D warnings` passed. These tests use synthetic local data and do not connect a YouTube account.
