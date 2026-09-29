# Local library integration

The shared `ui/library.slint` page and `src/library_ui.rs` adapter expose the SQLite worker's local collection, follow, and history operations. These are device-local features. They do not perform YouTube account mutations.

- Collection and video pages contain at most 100 records. Previous-page navigation retains at most 1,024 opaque cursors; each displayed page replaces the preceding page rather than accumulating the entire database. The shared Slint ListView virtualizes visible rows. Collection rename, deletion, video removal, and saving the current resolved video use typed IDs.
- Local follows retain channel IDs and names. Opening a follow validates the stored canonical channel ID and opens that public channel through the guest catalog; the display name is never substituted as a search query. Malformed stored identities fail closed. The channel page can save a local follow using its genuine returned header metadata. These actions never subscribe the YouTube account.
- History is disabled initially. The UI's opt-in becomes active only after the storage worker acknowledges persistence, and SQLite rechecks the setting when recording. Playing-media notifications record at most once per video/30-second position bucket; this feature adds no periodic timer. The default retention is 30 days, with 7/30/90-day controls. Users can delete individual entries or clear history.
- Collection deletion, history clearing, and local-data clearing require a confirmation in the shared UI. Other library controls are disabled while that confirmation is visible, and collection deletion names the selected collection.
- Import/export/backup use explicit asynchronous native file pickers; selected paths remain PathBuf values without lossy conversion. File reading, serialization, SQLite, and file writing run on the storage worker. Export and backup refuse existing destinations. Import/export transfer only the local interchange format, never account credentials; SQLite backup contains local preferences/history as well as collections.
- Clearing local data does not disconnect the YouTube account or erase protected session storage. Account disconnection does not erase local collections.
- Volume changes from the shared slider/keyboard controls persist after 250 ms
  without another change. Closing the event loop flushes a pending debounce.
  Accepted local writes drain during shutdown even after the result receiver
  closes; a regression test fills the bounded queue and checks the final saved
  value after reopening the database. SQLite work remains on its worker.

The surrounding controls can scroll at the minimum window size; the nested row ListView keeps its own bounded viewport. Native visual/keyboard validation of the new page remains to be recorded separately from compilation and worker tests. There is no claim that these source-level bounds alone prove the large-library performance gate.

Validation on the available macOS host: `cargo check --locked -p serein` passed; `cargo test --locked -p serein library` passed all four focused tests (worker paging/mutations, history opt-in/local clear, explicit safe transfer destinations, bounded backward navigation); `cargo clippy --locked -p serein --all-targets -- -D warnings` passed. These tests use synthetic local data and do not connect a YouTube account.
