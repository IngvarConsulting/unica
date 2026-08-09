# Identity-bound cleanup under the publication lock

- Date: `2026-08-10`
- Status: `approved`
- Decision: `none` — no architectural contract changed

## Context

PR #396 carries filesystem identity through transaction recovery, publication
cleanup warnings, retry, finalize, and rollback. Review found that the Unix
facade checks the retained child identity and then calls `unlinkat` or
`renameat2`/`renameatx_np` by a descriptor-relative child name. The check and
the namespace mutation are separate system calls.

POSIX offers no portable compare-identity-and-unlink or
compare-identity-and-rename operation for an already opened regular file.
Directory-relative system calls protect the parent route, but still resolve the
final child name when the mutation runs. Windows can instead mutate through an
opened child handle.

The repository contract is narrower than protection against a hostile process
running under the same Unix account. Cooperating Unica writers serialize source
publication through the shared publication locks. Issue #356 additionally
requires route and identity changes observed between recovery phases to fail
closed without deleting a same-name replacement.

The implementation mostly follows that boundary, but its low-level comments
assume the publication lock is held while some error cleanup is retried after
the lock-owning closure has returned. This creates an in-contract gap: another
Unica writer may start before the retained cleanup state is finished.

## Considered approaches

### Reacquire the publication lock for deferred cleanup

Retain the existing identity-bound artifact and directory handles, but execute
every production cleanup, finalize, and rollback mutation in a publication-lock
scope. Error handling that currently runs after the first scope closes
reacquires the same target and guard lock set before retrying retained cleanup.
Production helper signatures carry a lock-scope proof so a future unguarded
call is rejected by the compiler.

This is the selected approach. It closes the gap against cooperating writers,
matches the existing contract, and leaves the OS-specific facade behind the
existing platform boundary.

### Claim entries into another private quarantine

An entry can first be moved to a fresh private namespace, verified after the
move, and only then deleted or restored. This reduces exposure to changes of an
untrusted final name, but substantially enlarges the recovery state machine and
still cannot protect against a hostile process with the same Unix identity
mutating the private namespace after verification.

### Use different handle-based publication mechanisms per OS

Windows already supports handle-based deletion and rename. Linux could use a
mixture of `O_TMPFILE`, `linkat(AT_EMPTY_PATH)`, and filesystem-dependent
fallbacks; macOS would require different clone or deprecated object-reference
APIs. These mechanisms do not provide one portable cleanup contract and would
turn a focused recovery fix into a new publication architecture.

## Selected design

### Lock scope

`CompileTransaction` keeps the complete target, guard-target, and tree-lock
mode inputs used for the first commit attempt. If the locked commit returns an
error with retained cleanup state, it reacquires that same lock set before
calling recovery retry and created-directory cleanup that can mutate source
namespace entries.

Rollback and success finalization already occur inside `commit_locked`; their
signatures will make that requirement explicit instead of relying on a comment.
Single-file publication cleanup follows the same rule.

Lock reacquisition failure is not downgraded to successful cleanup. The primary
publication error remains primary and the lock/cleanup failure is appended to
the existing cleanup diagnostics, leaving retained artifacts named for manual
recovery.

### Compile-time proof

Production functions that can call identity-bound removal or rollback rename
accept the active `PublicationLockToken` (or a smaller capability borrowing
it). The capability is threaded through cleanup, finalize, rollback, and retry;
it is not stored beyond the lock lifetime.

Pure inspection functions remain callable without the token. Platform facade
functions remain OS-oriented primitives, while the native publication layer
owns the rule that a destructive call requires a publication scope.

### Filesystem semantics

On Unix, retained parent handles prevent a replaced lexical parent route from
redirecting a mutation. The final child identity is rechecked immediately
before the descriptor-relative operation while the cooperative publication
scope is exclusive for that target. This is not presented as an atomic
inode-compare system call or as a security boundary against a hostile process
under the same Unix account.

On Windows, deletion and rename continue through the verified child handle.
Unsupported hosts continue to fail closed.

### Recovery state and diagnostics

The identity-bound cleanup token remains the durable authority for an artifact.
A failed retry keeps the token and the retained handles. Partial progress still
advances file cleanup before directory cleanup, and any failure continues to
name the retained recovery or quarantine paths.

No new success or warning shape is introduced. Existing rollback failures stay
hard errors and debris-only cleanup failures stay warnings according to the
current architecture registry.

## Test strategy

Implementation starts with failing tests that prove:

1. deferred artifact cleanup cannot run while a second Unica publisher holds or
   is acquiring the same publication lock;
2. recovery-directory cleanup uses the same reacquired lock scope;
3. rollback and quarantine restoration receive a live lock capability;
4. failure to reacquire a publication lock preserves cleanup state and reports
   the affected recovery paths;
5. existing parent-route and same-name replacement tests continue to fail
   closed on Unix and Windows.

Focused Rust tests run red before production changes and green afterward. The
final gate is the complete workspace Rust suite, CI and development Python
suites, formatting, clippy with warnings denied, architecture sync, the
platform-boundary guard, and the GitHub matrix on the updated PR head.

## Architectural assessment

The change enforces the already accepted cooperative publication-lock and
rollback-visibility contracts. It does not change the public `unica.*` surface,
wire payloads, format specifications, cache ownership, packaging, or layer
boundary. Therefore it does not require a new decision record.
