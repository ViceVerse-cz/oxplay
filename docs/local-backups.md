# Local SQLite backup publication

The backup operation now stages a complete SQLite snapshot in a new private
sibling directory before creating the user-selected final filename. The former
implementation created that final filename before copying pages, so interruption
could leave an empty or incomplete file appearing to be a finished backup.
This hardening implements the recoverable-backup requirement in SPEC section 11.

SQLite's bounded-step backup loop retains its 30-second cooperative deadline.
After copying, the destination switches to DELETE journal mode, closes its SQLite
connection and syncs the snapshot file. Publication uses a same-filesystem hard
link that refuses any existing destination, including one created during the
copy. There is no overwrite or non-atomic copy fallback. Unix also syncs the
destination directory before returning success. No application UI or user data
is read on the UI thread for this operation.

On Unix the staging directory explicitly requests 0700 at creation through
locked tempfile 3.27.0's `Builder::permissions`, and the snapshot requests 0600.
The process umask can further restrict these permissions. Tempfile's directory
default is 0777 before umask; relying on that default produced 0755 in the first
test run and was corrected. No permissive creation followed by chmod is used.
Temporary SQLite sidecars remain inside that directory. Ordinary success
and error paths remove the staging directory through its owner; process crashes
or filesystem cleanup failures may leave the private sibling directory. They do
not expose a partially copied final backup filename. No startup scan or deletion
of arbitrary neighboring files was added.

Publication or directory-sync errors preserve any completed final backup rather
than deleting a file that might belong to the user. A directory-sync failure can
therefore report an error with a complete final file already present; its durable
directory entry is then unconfirmed. Storage media, operating-system sync
semantics and unexpected power loss remain external limits. Filesystems without
hard-link support fail explicitly. Windows and other filesystem behavior still
require their own platform validation; this is not a portable packaging claim.

Five new synthetic tests cover abandoned staging, a destination created during
copy, a WAL source restored without sidecars, private file modes and a racing
symlink. The earlier consistent-snapshot/no-overwrite test is retained.
After the measurement hold, `cargo fmt --package serein-storage` succeeded and
`cargo test --locked -p serein-storage` passed all 26 enabled tests, including
the five new backup tests; the explicit Keychain test remained ignored. These
checks include actual macOS staging and published-file permission assertions.
`cargo clippy --locked -p serein-storage --all-targets -- -D warnings` also passed.
Workspace integration checks remain separately coordinated. No real user
library backup or restore was performed.
