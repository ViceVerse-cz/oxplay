# Local persistence boundary

`LocalStore` contains a synchronous SQLite connection. Create and use it on an
owned database worker, not the UI thread. It has no background thread, polling,
network calls or remote account operations. The separate `vault` module is the
only credential persistence API. Library IDs cannot
be confused with a YouTube playlist ID. Thumbnails and signed URLs are not stored.

Every list uses keyset pagination and accepts 1–100 rows; retain the `next`
cursor only for that same collection/query. Keep application page caches bounded
independently. Saves/follows are idempotent and preserve row identity. Local
preferences have private defaults; persisting an opt-in does not implement the
corresponding feature.

Schema migrations are atomic and reject newer databases without downgrading.
`backup_to` creates a consistent database snapshot in a new destination and
refuses overwrite. Before a future destructive migration, integrate a confirmed
backup/restore flow; do not copy a live database file as a backup. Restore with
all connections closed by selecting the snapshot database. A failed migration
rolls back; an original damaged file should be kept for recovery, never silently
deleted or recreated. Deletion is logical, not forensic erasure.

The caller supplies the application-private database directory. Unix backup
files are created mode 0600. Regular library files use SQLite/OS default creation
permissions and require the caller's private directory; they contain browsing
interests despite holding no account secrets. Packaged Windows ACL and macOS/
Linux directory creation require integration tests.

Schema v3 adds explicitly opted-in local history with a 30-day retention default.
Retention is enforced on open, history reads/writes and setting changes, without
a permanent timer. Disabling history deletes all history rows and index entries
transactionally. There are no separately persisted history-derived recommendations.
History supports bounded keyset reads and per-video/all deletion. Signed and
thumbnail URLs remain excluded. Deleting database rows is not forensic erasure.

Explicit local library JSON import/export uses version 1 and includes only local
playlists/videos and follows. It excludes account material, history and preferences.
Imports validate the whole document before an atomic transaction; playlists are
created locally and subscriptions merge by channel ID. Unknown fields, duplicate
IDs inside a playlist, unsupported versions and invalid identifiers are rejected.
Read imports with a 16 MiB bound before allocation. Both directions also cap at
1,000 playlists, 50,000 videos and 10,000 follows. Exports hold a read transaction
for a consistent snapshot. Exported titles and collections disclose local interests.

## Protected session storage

`vault::ProtectedSessionStore` is a separate synchronous worker-owned boundary.
`SessionProfile` is an application-local random ID; it is never an account name.
The provider supplies explicitly imported, filtered session bytes only after
identity verification. The store never reads browser profiles, parses cookies,
contacts YouTube or determines that the user is logged in.

The macOS adapter keeps a 48-byte key-ID/encryption-key record in the local
Keychain under `org.serein.desktop.session-key.v1`, explicitly excluding iCloud
synchronization. Atomic key creation refuses to replace an existing item.
XChaCha20Poly1305 encrypts each ≤4 MiB session with a fresh OS-random 24-byte nonce.
The envelope header/version, random key ID and local profile ID are authenticated.
Files are atomically replaced in a private 0700 directory with 0600 permissions;
only ciphertext reaches temporary files. Reads reject symlinks, nonregular files,
hardlinks, foreign ownership and permissions granting other users access. A file
lock serializes each directory's operations across processes. Encryption keys
and returned secrets zeroize their owned byte buffers on drop; this does not
promise forensic memory/SSD/swap erasure.

`new` does not read credentials. `save`, `load`, and `delete` access only the
selected local profile. The account controller must invalidate its generation,
cancel authenticated work/playback and discard memory secrets before deletion;
the storage adapter cannot invalidate provider/UI tasks by itself. Deletion
removes the key before ciphertext and reports keychain denial even while trying
to remove the file. It does not revoke Google's browser session. No silent
plaintext persistence or automatic fallback exists. Offer explicit memory-only
connection if secure storage is unavailable. Linux/Windows currently fail closed
as unsupported adapters; they are not validated secure-storage platforms.

Default tests inject private in-memory keys and use synthetic data. The ignored
`macos_keychain_synthetic_roundtrip` test requires explicit local invocation,
uses one random app-owned key, refuses collisions and deletes it on completion/
unwind. It neither uses real cookies nor scans browser or existing keychain items.
Runtime Keychain qualification must be recorded from that actual test and the
signed packaged application, not inferred from compiling the adapter.

The explicit `rusqlite` bundled feature compiles its pinned SQLite source so a
system SQLite package is not an undeclared requirement. Audit both crate/native
source licenses and versions in the final SBOM. Test with
`cargo test -p serein-storage --locked` after adding the crate to the workspace
and resolving the committed lockfile.
