# File And Artifact Workflows

Use `dump` to bring database changes into Git-visible files. Check the worktree before dump and review the diff after dump.

For an applied dump:

- use synchronous `mode=full` for a DESIGNER `CONFIGURATION` or `EXTENSION`;
- select an extension with matching `sourceSet` and `extension` names.

Unica independently resolves an exact 8.3.27 installation, redirects the
selected source-set to a private stage, validates the required owner and every
XML version-bearing root as the raw literal 2.20, then publishes the complete
tree with preimage checks and rollback. Async full dumps and external
source-sets remain preview-only. `mode=incremental` and `mode=partial` are also
available only as read-only previews with `dryRun=true`; they need
shadow/staging publication with exact path/hash receipts.
Partial preview also requires `object` or `objects`.

The final stage-to-target move is tentative, not a source-identity CAS. Unica
keeps the publication lock, recaptures the complete target, and commits only an
exact match with the sealed stage. A detected replacement is moved into private
quarantine before return; the original target is then restored with no-clobber,
or recovery is retained if restoration cannot prove an unoccupied destination.
The restored tree must also equal the captured backup; a swapped backup name is
quarantined instead of accepted. No unvalidated tree installed by the
invocation remains at the selected source path when the lock is released. A
continuously hostile same-UID process can still race pathname cleanup;
excluding that actor requires a stronger OS trust boundary, such as a separate
identity or immutable parent directory.

On Windows, synchronous applied full dump is fail-closed until owner-only ACL
enforcement and handle-safe no-clobber directory publication are implemented;
preview remains read-only. On the supported POSIX path, Unica verifies physical
DESIGNER markers, probes the exact sibling `ibcmd --version`, and keeps
secret-bearing effective configuration outside retained recovery.
The platform install must be system-owned and immutable to the invoking
non-root user: every install entry and ancestor is root-owned, not group/world
writable, link-free, and ACL-free. User-owned or otherwise mutable installs are
rejected before `ibcmd` or `v8-runner` starts. ACL proof is implemented on
macOS and Linux; other Unix hosts fail closed.

`convert` is repository-aware and does not require an infobase, but applied
conversion is currently fail-closed because it can publish Designer XML outside
the verified dump boundary. Use `dryRun=true`.

Use `make` for `.cf`, `.cfe`, `.epf`, or `.erf` artifacts. Provide `output`; add `sourceSet` or `extension` when the target is not the default source. For external processors/reports, `output` is a publish directory, not a single `.epf`/`.erf` filename.

Use `load` for applying `.cf` or `.cfe` artifacts. Supported modes are `load` and `merge`; `merge` requires `settings`, and `update` is not a supported load mode. v8-runner rejects `.epf` and `.erf` for `load`; external processors/reports are handled through external source-sets with `build`, `dump`, and `make`.
