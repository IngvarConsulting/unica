# File And Artifact Workflows

- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

The intended future applied role of `dump` is to bring database changes into
Git-visible files. Currently preview only its arguments; it does not change the
worktree, and no runtime evidence or diff is produced.

For a dump preview, use `dryRun=true`; select an extension with matching
`sourceSet` and `extension` names.

On Windows, macOS, and Linux, verified transactional publication describes the
synchronous full dump (`mode=full`) for a DESIGNER `CONFIGURATION` or
`EXTENSION` source-set. It is currently preview-only because post-run
validation/publication has no proved receipt bound. Unica independently resolves an exact 8.3.27
installation, redirects the selected source-set to a private stage, validates
the required owner and every XML version-bearing root as the raw literal 2.20,
then publishes the complete tree with preimage checks and rollback. ADR-0016
owns this publication contract;
`INV-SOURCE-BOUND-PREIMAGES` and `INV-SOURCE-ROLLBACK-VISIBLE` describe its
verified transaction behavior, while OS-specific mechanics stay behind
`INV-PLATFORM-OS-BEHIND-FACADE`.

All applied dump modes remain fail-closed. Async full dumps and dumps for
external source-sets remain preview-only. `mode=incremental` and `mode=partial` are also available only as read-only
previews with `dryRun=true`; they need shadow/staging publication with exact
path/hash receipts.
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

On Windows, Unica verifies a local system installation through no-follow
handles: its trusted owner and DACL must prevent the invoking non-elevated user
from mutating the install tree, while the ancestry must prevent deletion,
replacement, or retargeting of path components. On macOS and Linux, Unica
verifies physical DESIGNER markers, probes the exact sibling `ibcmd --version`,
and requires a root-owned, link-free install tree without group/world write or
ACLs. Secret-bearing effective configuration stays outside retained recovery.
User-owned or otherwise mutable installs are rejected before `ibcmd` or
`v8-runner` starts; other Unix hosts fail closed.

`convert` is repository-aware and does not require an infobase, but applied
conversion remains fail-closed because it can publish Designer XML outside the
verified dump boundary. Use `dryRun=true`.

Preview `make` with `dryRun=true` for `.cf`, `.cfe`, `.epf`, or `.erf`
artifacts. Applied publication is fail-closed until the runner exposes a
bounded rollback contract. Provide `output`; add `sourceSet` or `extension`
when the target is not the default source. For external processors/reports,
`output` is a publish directory, not a single `.epf`/`.erf` filename.

Preview `load` with `dryRun=true` for `.cf` or `.cfe` artifacts; applied load is
not currently admitted. Supported argument modes are `load` and `merge`;
`merge` requires `settings`, and `update` is not supported. v8-runner rejects
`.epf` and `.erf` for `load`; external processors/reports are handled through
external source-sets with preview-only `build`, `dump`, and `make`.
