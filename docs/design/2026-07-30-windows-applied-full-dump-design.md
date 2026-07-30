# Windows applied full dump

- Date: `2026-07-30`
- Status: `approved`
- Decision: `none` — no architectural contract changed

## Context

Issue #268 reports that an applied full dump of a `CONFIGURATION` or
`EXTENSION` source set is unavailable on Windows. The request reaches
`PreparedDump::prepare`, but the Windows implementation returns a deliberate
fail-closed error before resolving the platform or launching `v8-runner`.

The guard was introduced because the verified full-dump transaction relies on
two guarantees that were implemented only for Unix:

1. the private staging tree and its effective configuration are inaccessible to
   unrelated users from the instant they are created;
2. directory publication is bound to captured parent identities and cannot
   overwrite an entry created concurrently at the destination name.

The public operation, dump format, validation profile, and publication
transaction already exist. Supporting Windows implements the platform half of
the existing contract under ADR-0009; it does not change the public MCP surface,
the writable format selected by ADR-0016, or the ownership of workspace state.

## Goals

- Permit applied `mode=full` dump for `CONFIGURATION` and `EXTENSION` source
  sets on Windows.
- Preserve the existing validation, conflict detection, rollback, quarantine,
  and cleanup semantics.
- Create private staging directories and files with an explicit owner-only
  Windows DACL without a permissive inheritance window.
- Bind security-sensitive path operations to live directory handles and reject
  reparse-point substitution.
- Publish directories with an atomic no-replace operation.
- Keep all OS-dependent implementation under
  `crates/unica-coder/src/infrastructure/platform/`.

## Non-goals

- No bypass flag or caller-confirmed unsafe publication mode.
- No support for incremental dump without the receipts required by the existing
  source synchronization guard.
- No change to asynchronous runtime-job support.
- No change to the public arguments or result payloads of `unica.runtime.*` or
  `unica.build.dump`.
- No general Windows filesystem abstraction beyond the primitives required by
  the verified full-dump transaction.

## Chosen approach

Extend the existing `DirectoryAnchor` transaction model on Windows instead of
creating a second publisher.

### Windows directory anchors

A Windows directory anchor owns a directory handle opened by `CreateFileW`
with directory and reparse-point semantics. Its captured identity consists of
the volume serial number and file index returned by
`GetFileInformationByHandle`.

`capture_exact`, child creation, path-binding verification, and identity checks
use the same platform-neutral `DirectoryAnchor` interface as Unix. Child opens
and creates use `NtCreateFile` with the retained parent handle in
`OBJECT_ATTRIBUTES.RootDirectory` and a single relative child name.
`FILE_OPEN_REPARSE_POINT` prevents final-component traversal. Relative names
must contain exactly one non-empty component and no separator, `.` or `..`.

Path-based Win32 operations bracketed by identity checks are not sufficient:
an intermediate component can be replaced inside the check/use window. Such
operations are permitted only to capture the initial workspace anchor. Once an
anchor exists, every security-sensitive child operation is handle-relative.

### Owner-only staging

Private stage directories and files are created through relative `NtCreateFile`
calls whose `OBJECT_ATTRIBUTES.SecurityDescriptor` points to the explicit
protected security descriptor. The descriptor contains a protected DACL that
grants the required access to the current process token's user SID and does not
inherit ambient directory permissions.

Private regular files are created with the corresponding explicit security
attributes. Existing post-creation Unix permission tightening remains unchanged;
Windows must not rely on a post-creation ACL rewrite because that would expose a
race window.

After creation, Unica opens the object without following reparse points, checks
its identity and DACL, and only then exposes it to the rest of the transaction.
Failure to create or prove the private ACL aborts before `v8-runner` starts.

### No-clobber directory publication

The Windows platform facade opens the source child relative to its retained
parent and renames that open object with `NtSetInformationFile` using
`FILE_RENAME_INFORMATION.RootDirectory` for the retained destination parent.
`ReplaceIfExists` is false, so an occupied destination name is an atomic error.

The full-dump publisher performs the existing sequence:

1. verify source and destination parent anchors;
2. verify the source child identity captured during staging;
3. execute the handle-relative no-replace rename;
4. verify both parents again;
5. open the published child without following reparse points and verify that
   its identity equals the staged identity.

The same primitive is used for backup, stage installation, rollback, and
quarantine. An entry concurrently installed under any destination name is never
replaced.

### Handle-relative reads and tree inspection

The verified transaction also reads configuration preimages, captures target
and staged-tree snapshots, and walks workspace descendants. Windows implements
these operations through retained handles rather than reopening child paths.

Regular files and arbitrary children are opened with `NtCreateFile`,
`OBJECT_ATTRIBUTES.RootDirectory`, one validated relative component, and
`FILE_OPEN_REPARSE_POINT`. Directory descendants are traversed one component at
a time. Directory names are enumerated from the retained handle with
`GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`; enumeration never
reconstructs a path for a child open.

Secure file snapshots retain the file handle while hashing, compare size and
last-write metadata before and after the read, require one hard link, then
reopen the same name relative to the retained parent and compare identities.
Tree snapshots apply the same rule recursively to every directory and regular
file. Reparse points and unsupported entry kinds fail closed. An absence check
opens the name relative to the retained parent and treats only the native
not-found statuses as absent; a reparse point is present, not absent.

### Handle-relative cleanup

Cleanup enumerates the retained private directory handle, opens each entry
relative to that handle with delete access and
`FILE_OPEN_REPARSE_POINT`, and removes the opened object with
`NtSetInformationFile(FileDispositionInformation)`. Directories are emptied
recursively before disposition. Reparse points are removed as entries and are
never followed.

Before removing an expected file or directory, cleanup compares the opened
identity with the identity captured by the transaction. An identity mismatch
leaves the replacement untouched and reports an integrity failure. Display
paths remain diagnostic only.

Windows delete disposition does not make a directory name disappear while a
retained handle to that directory remains open. Private cleanup therefore owns
takeable child and root anchors: it empties a child through the retained handle,
closes that handle, performs the identity-bound disposition through the parent,
and only then verifies the parent. The private root anchor is likewise closed
before root deletion and final absence verification. A delete-pending name is
not treated as an absent or successfully verified replacement.

### Immutable Windows platform attestation

The production platform resolver must not execute a mutable `1cv8.exe` or
`ibcmd.exe`. Windows therefore receives a platform trust attestation equivalent
to the existing Unix policy rather than using the test-fixture bypass.

The attestation rejects an elevated effective caller. It inspects the current
thread token first and falls back to the process token only when the thread is
not impersonating. It also requires a local-volume proof from the retained
installation handle; UNC, device, mapped-remote, and other network filesystems
are not trusted to attest their own owner, DACL, identity, or link-count
metadata.

The attestation walks the local installation ancestry and inventory through
no-follow handles, requires files to have one hard link, and captures each
entry's identity and security descriptor. The owner must be a trusted Windows
installation principal (TrustedInstaller, LocalSystem, or the built-in
Administrators group).

Ancestry and installation contents use different mutation profiles. For
ancestors above the installation directory, DACL inspection rejects rights that
can delete, replace, or retarget an existing path component, including
`DELETE`, `FILE_DELETE_CHILD`, `WRITE_DAC`, and `WRITE_OWNER`. Creating an
unrelated sibling under a protected ancestor is not by itself a way to replace
the retained component and is permitted. For the installation directory and
its complete inventory, inspection rejects the full mutation set: file or
directory creation, write/append, attribute or extended-attribute writes,
delete/delete-child, DACL changes, and ownership changes. Read and execute
access for ordinary users remains permitted.

The security-descriptor digest is part of the immutable entry snapshot.
Executable and probe digests, the complete installation inventory, and trust
metadata are captured before execution and compared again before publication.
An unreadable, unsupported, reparse-point, or ambiguously protected entry fails
closed.

### Failure handling

- A changed parent identity or reparse-point substitution fails closed.
- An occupied destination fails without altering the occupant.
- If failure occurs after moving the old target to backup, the existing rollback
  state machine attempts a no-clobber restoration.
- If the destination is occupied during rollback, the occupant survives and
  the owned tree is quarantined according to the existing transaction rules.
- ACL inspection or identity inspection failure is an integrity failure, not a
  warning.
- Cleanup after a committed publication remains best-effort only where the
  current transaction already classifies it as such.

## Code structure

- `crates/unica-coder/src/infrastructure/platform/filesystem.rs`
  owns reusable Win32 handle, identity, relative `NtCreateFile`,
  handle enumeration, `NtSetInformationFile`, explicit-DACL creation, ACL and
  immutable-trust verification, and handle-relative removal primitives.
- `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs`
  keeps the platform-neutral transaction and implements Windows
  `DirectoryAnchor` operations by calling the platform facade.
- `crates/unica-coder/tests/platform/` contains Windows integration tests that
  exercise real filesystem behavior.
- Existing Unix implementations and tests remain the reference for transaction
  semantics.

No OS-dependent code may move into application or domain modules
(`INV-PLATFORM-OS-BEHIND-FACADE`), and platform tests remain colocated with the
adapters (`INV-PLATFORM-COLOCATED-TESTS`).

## Testing

The implementation follows test-driven development.

1. A Windows regression proves that applied full dump reaches the runner and
   commits a validated configuration tree instead of returning the current
   pre-flight guard.
2. Platform tests prove that a private directory and file are created with a
   protected owner-only DACL.
3. No-clobber tests prove that an existing destination survives unchanged.
4. Reparse substitution and parent-identity replacement tests prove that the
   transaction fails closed.
5. Secure-read and tree-snapshot tests prove that file and directory name
   replacement is detected and that reparse points are rejected.
6. Cleanup tests prove that an expected tree is removed through retained
   handles while an identity-replaced child survives.
7. Windows platform-attestation tests accept a protected installation fixture
   owned by a trusted principal and reject an elevated caller, an untrusted
   owner, or a DACL granting mutation to ordinary users.
8. Rollback tests cover a destination race after backup and after stage
   installation.
9. Existing Unix full-dump and cross-platform compilation tests must remain
   green.

Windows-specific tests run on a Windows host. Platform-neutral transaction tests
continue to run in the ordinary Rust workspace suite.

## Documentation impact

After the Windows regression is green, remove statements that applied full dump
is intentionally unavailable on Windows from:

- `plugins/unica/skills/v8-runner/SKILL.md`;
- `plugins/unica/skills/v8-runner/references/file-and-artifact-workflows.md`;
- `plugins/unica/references/tooling/v8project.md`.

The replacement text states that applied full dump uses verified transactional
publication on all supported hosts. It references the owning invariants instead
of restating them as a new architectural rule.
