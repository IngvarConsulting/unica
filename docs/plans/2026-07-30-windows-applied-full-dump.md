# Windows Applied Full Dump Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable verified applied full dumps for `CONFIGURATION` and `EXTENSION` source sets on Windows without weakening private staging, path-binding, no-clobber publication, or rollback guarantees.

**Architecture:** Extend the existing platform filesystem facade with Win32 directory-handle, descriptor-relative inspection and cleanup, explicit-DACL, immutable-platform-attestation, and no-clobber rename primitives, then implement the current `DirectoryAnchor` and publication transaction for Windows. The public MCP routes and transaction state machine remain unchanged; only the deliberate Windows pre-flight guard is removed after real Windows filesystem tests prove parity.

**Tech Stack:** Rust 2021, `windows-sys 0.59`, Win32 Security and FileSystem APIs, existing `unica-coder` platform adapters, Rust integration/unit tests, Python contract tests.

## Global Constraints

- Follow `docs/design/2026-07-30-windows-applied-full-dump-design.md`.
- Keep OS-dependent production code under `crates/unica-coder/src/infrastructure/platform/` (`INV-PLATFORM-OS-BEHIND-FACADE`, ADR-0009).
- Keep platform tests beside the adapters or under `crates/unica-coder/tests/platform/` (`INV-PLATFORM-COLOCATED-TESTS`).
- Do not add a bypass flag, unsafe publication mode, or new public MCP argument/result field.
- Preserve the existing platform `8.3.27`, export format `2.20` write gate (ADR-0016).
- Create private Windows objects with their final protected DACL; do not create permissively and tighten afterward.
- After the initial absolute anchor capture, every security-sensitive child
  create, open, remove, and rename on Windows must be relative to a retained
  parent handle; path-based check/use bracketing is insufficient.
- Windows no-clobber directory rename uses `NtSetInformationFile` with a
  destination `RootDirectory` handle and `ReplaceIfExists = false`.
- Every production change starts with a test observed failing for the intended missing behavior.
- Local commits require the user to configure `git user.name` and `git user.email`; until then, keep completed task changes staged by exact path without changing repository identity.

---

## File structure

- Modify `Cargo.toml` only to expose the Win32 authorization APIs already supplied by `windows-sys`.
- Modify `crates/unica-coder/src/infrastructure/platform/filesystem.rs` for reusable Windows handles, identity, protected-DACL creation, handle-relative opens, enumeration and removal, ACL/trust verification, and atomic no-replace rename.
- Modify `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs` for Windows `DirectoryAnchor` operations and transaction wiring.
- Keep real Windows filesystem boundary tests in the existing
  `full_dump_publication.rs` test module so private transaction failpoints remain
  inaccessible to release builds.
- Modify `tests/ci/test_unica_skills.py` and the four runtime documentation files that currently declare Windows fail-closed.

### Task 1: Owner-only Win32 filesystem primitives

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs:1-285`
- Test: `crates/unica-coder/src/infrastructure/platform/filesystem.rs`

**Interfaces:**
- Consumes: absolute `Path` values inside an already validated workspace.
- Produces:

```rust
#[cfg(windows)]
pub(crate) fn open_directory_nofollow(path: &Path) -> io::Result<fs::File>;

#[cfg(windows)]
pub(crate) fn create_owner_only_directory(path: &Path) -> io::Result<fs::File>;

#[cfg(windows)]
pub(crate) fn create_owner_only_file(path: &Path) -> io::Result<fs::File>;

#[cfg(windows)]
pub(crate) fn verify_owner_only_acl(file: &fs::File) -> io::Result<()>;
```

- The returned handles must support `file_identity` and remain valid while callers bracket path mutations.

- [ ] **Step 1: Enable only the required Win32 API feature**

Add `Win32_Security_Authorization` beside the existing Windows features:

```toml
windows-sys = { version = "0.59", features = [
  "Win32_Foundation",
  "Win32_Security",
  "Win32_Security_Authorization",
  "Win32_Storage_FileSystem",
  "Win32_System_Threading",
] }
```

- [ ] **Step 2: Write failing Windows tests for private creation**

Add tests guarded by `#[cfg(windows)]`:

```rust
#[test]
fn owner_only_directory_is_opened_without_following_reparse_points() {
    let root = unique_temp_root("owner-only-directory");
    fs::create_dir_all(&root).unwrap();
    let private = root.join("private");

    let handle = create_owner_only_directory(&private).unwrap();

    verify_owner_only_acl(&handle).unwrap();
    assert_eq!(file_identity(&handle).unwrap(), file_identity(&open_directory_nofollow(&private).unwrap()).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_open_rejects_a_reparse_point() {
    let root = unique_temp_root("directory-reparse");
    let real = root.join("real");
    let link = root.join("link");
    fs::create_dir_all(&real).unwrap();
    std::os::windows::fs::symlink_dir(&real, &link).unwrap();

    let error = open_directory_nofollow(&link).unwrap_err();

    assert!(matches!(error.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn owner_only_file_has_the_final_acl_at_creation() {
    let root = unique_temp_root("owner-only-file");
    fs::create_dir_all(&root).unwrap();
    let private = root.join("effective.yaml");

    let handle = create_owner_only_file(&private).unwrap();

    verify_owner_only_acl(&handle).unwrap();
    fs::remove_dir_all(root).unwrap();
}
```

The production mutation caught by these tests is replacing explicit protected-DACL creation with inherited/default creation, or opening a directory while following a reparse point.

- [ ] **Step 3: Run the focused tests and verify RED**

Run:

```powershell
cargo test -p unica-coder infrastructure::platform::filesystem::tests::windows -- --nocapture
```

Expected: compilation fails because `create_owner_only_directory`,
`create_owner_only_file`, `open_directory_nofollow`, and
`verify_owner_only_acl` do not exist.

- [ ] **Step 4: Implement the minimal Win32 security owner**

Add a private RAII owner that:

1. opens the current process token with `TOKEN_QUERY`;
2. reads `TokenUser`;
3. converts the SID to its string form;
4. creates SDDL `D:P(A;;FA;;;<current-user-sid>)`;
5. calls `ConvertStringSecurityDescriptorToSecurityDescriptorW`;
6. exposes a `SECURITY_ATTRIBUTES` whose lifetime is bounded by the owner;
7. releases token, SID string, and security descriptor with their documented
   Win32 release functions.

Use the owner only inside the create calls:

```rust
let security = OwnerOnlySecurityAttributes::current_user()?;
let created = unsafe {
    CreateDirectoryW(path.as_ptr(), security.as_ptr())
};
```

Open directories with:

```rust
CreateFileW(
    path.as_ptr(),
    FILE_READ_ATTRIBUTES | READ_CONTROL,
    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    std::ptr::null(),
    OPEN_EXISTING,
    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    std::ptr::null_mut(),
)
```

Reject a final `FILE_ATTRIBUTE_REPARSE_POINT` from
`GetFileInformationByHandle`. Create files with `CREATE_NEW`, the explicit
security attributes, `FILE_ATTRIBUTE_NORMAL`, and no sharing broader than the
existing transaction requires.

`verify_owner_only_acl` must read the DACL through `GetSecurityInfo`, require a
protected DACL, enumerate its ACEs, and accept exactly one allow ACE for the
current token user with no deny or inherited ACEs.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run:

```powershell
cargo test -p unica-coder infrastructure::platform::filesystem::tests::windows -- --nocapture
cargo check -p unica-coder --all-targets
```

Expected: all Windows filesystem tests pass and `cargo check` has no warnings.

- [ ] **Step 6: Commit or stage the task**

```powershell
git add -- Cargo.toml Cargo.lock crates/unica-coder/src/infrastructure/platform/filesystem.rs
git commit -m "feat(windows): add private directory handles"
```

If Git identity remains unavailable, do not alter it; leave only these paths staged.

### Task 2: Windows DirectoryAnchor parity

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs:1500-2055`
- Test: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs`

**Interfaces:**
- Consumes Task 1:

```rust
open_directory_nofollow(path: &Path) -> io::Result<File>
create_owner_only_directory(path: &Path) -> io::Result<File>
create_owner_only_file(path: &Path) -> io::Result<File>
verify_owner_only_acl(file: &File) -> io::Result<()>
```

- Produces the existing platform-neutral API:

```rust
impl DirectoryAnchor {
    fn capture_exact(path: &Path) -> Result<Self, String>;
    fn try_clone(&self) -> Result<Self, String>;
    fn create_child(&self, name: &OsStr, display_path: &Path) -> Result<Self, String>;
    fn verify_path_binding(&self) -> Result<(), String>;
}
```

- Also produces Windows platform primitives:

```rust
#[cfg(windows)]
pub(crate) fn open_directory_child_nofollow(
    parent: &fs::File,
    name: &OsStr,
) -> io::Result<fs::File>;

#[cfg(windows)]
pub(crate) fn create_owner_only_directory_child(
    parent: &fs::File,
    name: &OsStr,
) -> io::Result<fs::File>;

#[cfg(windows)]
pub(crate) fn create_owner_only_file_child(
    parent: &fs::File,
    name: &OsStr,
) -> io::Result<fs::File>;
```

- [ ] **Step 1: Write failing Windows anchor tests**

Add `#[cfg(windows)]` tests that exercise the real anchor:

```rust
#[test]
fn windows_anchor_detects_parent_name_replacement() {
    let root = unique_temp_root("anchor-replacement");
    let parent = root.join("parent");
    fs::create_dir_all(&parent).unwrap();
    let anchor = DirectoryAnchor::capture_exact(&parent).unwrap();
    let moved = root.join("moved");
    fs::rename(&parent, &moved).unwrap();
    fs::create_dir(&parent).unwrap();

    let error = anchor.verify_path_binding().unwrap_err();

    assert!(error.contains("identity"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn windows_anchor_creates_a_private_bound_child() {
    let root = unique_temp_root("anchor-child");
    fs::create_dir_all(&root).unwrap();
    let anchor = DirectoryAnchor::capture_exact(&root).unwrap();
    let child_path = root.join("child");

    let child = anchor.create_child(OsStr::new("child"), &child_path).unwrap();

    child.verify_path_binding().unwrap();
    verify_owner_only_acl(&child.directory).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn windows_anchor_rejects_a_display_path_outside_the_parent() {
    let root = unique_temp_root("anchor-containment");
    let parent = root.join("parent");
    let outside = root.join("outside");
    fs::create_dir_all(&parent).unwrap();
    let anchor = DirectoryAnchor::capture_exact(&parent).unwrap();

    let error = anchor.create_child(OsStr::new("child"), &outside).unwrap_err();

    assert!(error.contains("does not match anchored child"));
    assert!(!outside.exists());
    fs::remove_dir_all(root).unwrap();
}
```

The production mutation caught is accepting a same-name replacement after the
anchor was captured, or creating a stage child with inherited permissions.

- [ ] **Step 2: Run the tests and verify RED**

Run:

```powershell
cargo test -p unica-coder windows_anchor -- --nocapture
```

Expected: the current `#[cfg(not(unix))]` stubs return
`secure directory anchors are unavailable on this host`.

- [ ] **Step 3: Implement Windows anchor operations**

Split the current fallback cfgs so Windows receives real implementations and
only unsupported hosts retain stubs:

```rust
#[cfg(windows)]
fn capture_exact(path: &Path) -> Result<Self, String> {
    let directory = open_directory_nofollow(path)
        .map_err(|error| format!("failed to open directory anchor {}: {error}", path.display()))?;
    let identity = file_identity(&directory)
        .map_err(|error| format!("failed to identify directory anchor {}: {error}", path.display()))?;
    Ok(Self { path: path.to_path_buf(), directory, identity })
}
```

Declare the documented user-mode `NtCreateFile` entrypoint from `ntdll`.
Construct `UNICODE_STRING`, `OBJECT_ATTRIBUTES`, and `IO_STATUS_BLOCK` with
`OBJECT_ATTRIBUTES.RootDirectory = parent.as_raw_handle()`. Accept exactly one
relative child component. Use `FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT`
for directories and `FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT` for
files. Create with `FILE_CREATE`, so an existing name is never opened.

`verify_path_binding` reopens `self.path` without following reparse points and
requires equality with `self.identity`. `create_child` first requires
`display_path == self.path.join(name)`, creates and reopens the child relative
to `self.directory`, verifies the returned handle identity and owner-only ACL,
then returns the child anchor. It must not use `display_path` for the create or
reopen operation.

Replace `create_regular_child_owner_only`'s Windows fallback with
`create_owner_only_file_child(parent, name)`, preserving create-new behavior
without using its display path.

- [ ] **Step 4: Run anchor and existing transaction tests**

Run:

```powershell
cargo test -p unica-coder windows_anchor -- --nocapture
cargo test -p unica-coder infrastructure::platform::full_dump_publication -- --test-threads=1
```

Expected: the Windows tests pass; existing platform-neutral transaction tests remain green.

- [ ] **Step 5: Commit or stage the task**

```powershell
git add -- crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs
git commit -m "feat(windows): bind full dump directories to handles"
```

If Git identity remains unavailable, keep the path staged without changing Git configuration.

### Task 3: Atomic no-clobber Windows directory publication

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs:200-285`
- Modify: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs:3170-3932`
- Test: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs`

**Interfaces:**
- Consumes: Task 2 `DirectoryAnchor` and existing `FileIdentity`.
- Produces:

```rust
#[cfg(windows)]
fn rename_prechecked_directory_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    expected_identity: FileIdentity,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String>;

#[cfg(windows)]
fn rename_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String>;
```

- [ ] **Step 1: Add a real-filesystem no-clobber test**

Add this `#[cfg(windows)]` test to the existing
`full_dump_publication.rs` test module:

```rust
#[test]
fn no_clobber_directory_move_preserves_an_existing_destination() {
    let root = unique_temp_root("no-clobber");
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("new.txt"), b"new").unwrap();
    fs::write(destination.join("old.txt"), b"old").unwrap();

    let source_parent = DirectoryAnchor::capture_exact(&root).unwrap();
    let destination_parent = source_parent.try_clone().unwrap();
    let error = rename_child_no_replace(
        &source_parent,
        OsStr::new("source"),
        &destination_parent,
        OsStr::new("destination"),
    )
    .unwrap_err();

    assert!(destination.join("old.txt").is_file());
    assert!(!destination.join("new.txt").exists());
    assert!(source.join("new.txt").is_file());
    assert!(!error.is_empty());
    fs::remove_dir_all(root).unwrap();
}
```

The mutation caught is adding `MOVEFILE_REPLACE_EXISTING` or deleting the
destination before moving.

- [ ] **Step 2: Add transaction race tests**

Add Windows-enabled variants of the existing publication failpoint tests:

```rust
#[test]
fn windows_destination_race_after_backup_survives_rollback() {
    let (root, context, target) = workspace("windows-destination-race");
    let runner = DumpRunner::valid();

    let result = with_publication_hook(PublicationCheckpoint::BeforeStageInstall, {
        let target = target.clone();
        move || {
            fs::create_dir_all(&target).unwrap();
            fs::write(target.join("racer.txt"), b"racer").unwrap();
        }
    }, || invoke(&runner, &platform(), FullDumpInvocation::BuildDump, &context));

    assert!(result.is_err());
    assert_eq!(fs::read(target.join("racer.txt")).unwrap(), b"racer");
    fs::remove_dir_all(root).unwrap();
}
```

- [ ] **Step 3: Run the tests and verify RED**

Run:

```powershell
cargo test -p unica-coder no_clobber_directory_move_preserves_an_existing_destination -- --nocapture
cargo test -p unica-coder windows_destination_race_after_backup_survives_rollback -- --nocapture
```

Expected: tests fail because the full-dump Windows rename functions still return
`atomic no-clobber directory rename is unavailable on this host`.

- [ ] **Step 4: Implement the Windows rename path**

Open the source child relative to `source_parent.directory`, retaining the
source handle and identity. Declare the user-mode `NtSetInformationFile`
entrypoint from `ntdll` and build `FILE_RENAME_INFORMATION` with
`RootDirectory = destination_parent.directory.as_raw_handle()`,
`ReplaceIfExists = false`, and a single relative destination name.

For `rename_prechecked_directory_child_no_replace`:

```rust
let source = open_directory_child_nofollow(&source_parent.directory, source_name)
    .map_err(|error| format!("failed to open staged child: {error}"))?;
if file_identity(&source).map_err(|error| error.to_string())? != expected_identity {
    return Err("staged directory identity changed".to_string());
}
rename_child_no_replace(source_parent, source_name, destination_parent, destination_name)
```

For `rename_child_no_replace`, use only the source child handle and destination
parent handle for the mutation. Convert `NTSTATUS` failures through
`RtlNtStatusToDosError` into `io::Error`. Reopen the destination relative to
the destination parent and require its identity to equal the pre-move source
identity. Handle-relative disposition/removal must be used for cleanup and
quarantine; do not reconstruct an absolute child path for a mutation.

- [ ] **Step 5: Run publication and rollback tests and verify GREEN**

Run:

```powershell
cargo test -p unica-coder no_clobber_directory_move_preserves_an_existing_destination -- --nocapture
cargo test -p unica-coder infrastructure::platform::full_dump_publication -- --test-threads=1
```

Expected: no-clobber, race, rollback, quarantine, and existing transaction tests pass.

- [ ] **Step 6: Commit or stage the task**

```powershell
git add -- crates/unica-coder/src/infrastructure/platform/filesystem.rs crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs
git commit -m "feat(windows): publish full dumps without clobber"
```

If Git identity remains unavailable, keep only these paths staged.

### Task 4: Windows handle-relative reads and descendant inspection

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs:1539-2270`
- Test: both modified platform modules

**Interfaces:**
- Consumes: Task 2 retained directory handles and `FileIdentity`.
- Produces Windows implementations for:

```rust
open_regular_child_nofollow(parent: &File, name: &OsStr) -> io::Result<File>;
open_any_child_nofollow(parent: &File, name: &OsStr) -> io::Result<File>;
read_directory_names(directory: &File) -> io::Result<Vec<OsString>>;
secure_read_regular_file_snapshot(path: &Path, role: &str)
    -> Result<SecureFileSnapshot, String>;
secure_path_is_absent(path: &Path) -> Result<bool, String>;
DirectoryAnchor::capture_descendant(...);
DirectoryAnchor::verify_descendant_identity(...);
DirectoryAnchor::capture_child_root_identity(...);
```

- [ ] **Step 1: Add failing Windows inspection regressions**

Add Windows tests that prove:

1. secure config reads reject a final reparse point and detect a child-name
   identity replacement at the existing secure-read hook;
2. an absence check returns `false` for a reparse point and `true` only for a
   genuinely missing relative child;
3. `capture_descendant` walks two or more components below the retained
   workspace anchor and rejects a replaced component;
4. handle enumeration returns names from the retained directory even if its
   original path is moved and replaced.

The production mutation caught is falling back to `fs::read`,
`Path::exists`, `read_dir(display_path)`, or another child-path operation after
the parent handle has been captured.

- [ ] **Step 2: Run focused tests and verify RED**

```powershell
cargo test -p unica-coder windows_secure_read -- --nocapture
cargo test -p unica-coder windows_descendant_anchor -- --nocapture
cargo test -p unica-coder windows_handle_enumeration -- --nocapture
```

Expected: current `#[cfg(not(unix))]` functions return their fail-closed
unavailable errors.

- [ ] **Step 3: Implement the minimal handle-relative facade**

Extend the existing `NtCreateFile` wrapper:

- a regular-file open requests `GENERIC_READ | FILE_READ_ATTRIBUTES |
  SYNCHRONIZE` with `FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT`;
- an arbitrary-child probe requests `FILE_READ_ATTRIBUTES | SYNCHRONIZE` with
  `FILE_OPEN_REPARSE_POINT` and no directory-type option;
- both reject a final reparse-point attribute;
- directory enumeration uses
  `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)` on the retained
  handle, validates record offsets/name lengths, skips `.` and `..`, and
  returns sorted names.

Implement descendant traversal one component at a time with
`open_directory_child_nofollow`. Implement secure reads by holding the parent
and child handles for the whole read, requiring one hard link, comparing
metadata before/after, reopening the same relative name, and comparing
identities. Implement absence by probing the relative name and treating only
native not-found errors as absent.

- [ ] **Step 4: Run inspection and existing anchor tests**

```powershell
cargo test -p unica-coder windows_secure_read -- --nocapture
cargo test -p unica-coder windows_descendant_anchor -- --nocapture
cargo test -p unica-coder windows_handle_enumeration -- --nocapture
cargo test -p unica-coder windows_anchor -- --nocapture
cargo check -p unica-coder --all-targets
```

- [ ] **Step 5: Commit**

```powershell
git add -- crates/unica-coder/src/infrastructure/platform/filesystem.rs crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs
git commit -m "feat(windows): inspect dump inputs through handles"
```

### Task 5: Windows tree snapshots and handle-relative cleanup

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs:2510-3100`
- Test: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs`

**Interfaces:**
- Consumes: Task 4 handle-relative open and enumeration primitives.
- Produces Windows implementations for `capture_tree_target_nofollow`,
  `capture_tree_child_nofollow`, `capture_directory_snapshot_recursive`,
  `unlink_bound_regular_child`, `remove_directory_contents_nofollow`, and
  `remove_bound_directory_child`.

- [ ] **Step 1: Add failing Windows tree and cleanup tests**

Add Windows real-filesystem tests that prove:

1. a nested regular tree receives a stable snapshot containing identities,
   sizes, and SHA-256 digests;
2. a reparse point anywhere in the target or staged tree fails snapshotting
   without traversal;
3. changing a file name at the existing snapshot hook is detected even when
   the replacement has identical bytes;
4. recursive cleanup removes an expected retained tree, including an
   unsupported reparse entry as an entry rather than following it;
5. replacing an expected child before cleanup leaves the replacement untouched
   and returns an identity error.

- [ ] **Step 2: Run focused tests and verify RED**

```powershell
cargo test -p unica-coder windows_tree_snapshot -- --nocapture
cargo test -p unica-coder windows_handle_cleanup -- --nocapture
```

Expected: the current Windows fallbacks report secure snapshot/cleanup as
unavailable.

- [ ] **Step 3: Implement descriptor-relative snapshots**

Use Task 4 enumeration and relative opens recursively. For each directory,
capture identity, enumerate and inspect children, reopen each relative child,
and compare identity. For each file, require one hard link, hash through the
open handle, compare size and last-write metadata before/after, then reopen and
compare identity. Treat reparse points and unsupported entry kinds as errors.
Use an initial absolute parent capture only at `capture_tree_target_nofollow`;
all child operations are handle-relative.

- [ ] **Step 4: Implement descriptor-relative cleanup**

Add a delete-capable arbitrary-child open using `NtCreateFile` with
`FILE_OPEN_REPARSE_POINT`. Normal directories are enumerated and emptied
recursively; regular files and reparse entries are never followed. Mark the
opened object for deletion with
`NtSetInformationFile(FileDispositionInformation)`. Expected-object cleanup
must compare `FileIdentity` before mutation and leave a replacement untouched.

- [ ] **Step 5: Run tree, validation, and cleanup suites**

```powershell
cargo test -p unica-coder windows_tree_snapshot -- --nocapture
cargo test -p unica-coder windows_handle_cleanup -- --nocapture
cargo test -p unica-coder staged_ -- --nocapture
cargo test -p unica-coder windows_anchor -- --nocapture
cargo check -p unica-coder --all-targets
```

- [ ] **Step 6: Commit**

```powershell
git add -- crates/unica-coder/src/infrastructure/platform/filesystem.rs crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs
git commit -m "feat(windows): snapshot and clean dump trees"
```

### Task 6: Windows immutable platform attestation

**Files:**
- Modify: `Cargo.toml` only if an additional `windows-sys` feature is required.
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs:1047-1538`
- Test: both modified platform modules

**Interfaces:**
- Consumes: Task 5 secure inventory traversal.
- Produces a Windows `ImmutablePlatformTrustSnapshot::capture` that proves the
  installation is not mutable by an untrusted principal before executing the
  platform.

- [ ] **Step 1: Add failing trust-policy tests**

Build security descriptors from SDDL in tests and prove that the Windows policy:

1. accepts TrustedInstaller, LocalSystem, or built-in Administrators ownership
   with substitution rights on ancestry and full mutation rights inside the
   installation limited to those trusted principals, while permitting
   read/execute access for ordinary users;
2. rejects an untrusted owner;
3. rejects an effective allow ACE granting ordinary users `DELETE`,
   `FILE_DELETE_CHILD`, `WRITE_DAC`, `WRITE_OWNER`, `GENERIC_WRITE`, or
   `GENERIC_ALL` on an ancestor above the install root;
4. rejects any effective allow ACE granting ordinary users file/directory
   creation, write/append, attribute writes, delete/delete-child, `WRITE_DAC`,
   or `WRITE_OWNER` on the install root or its inventory;
5. does not treat an `INHERIT_ONLY` ACE as a capability on the current object;
6. rejects a null/absent DACL and malformed or unsupported ACEs;
7. rejects an elevated effective thread token, falls back to the process token
   only for `ERROR_NO_TOKEN`, and fails closed for other thread-token errors;
8. rejects UNC/device roots and mapped or remote filesystems using evidence
   queried from the retained installation handle;
9. rejects a reparse point or multiply linked file in an installation
   inventory.

The pure descriptor-policy tests do not require changing machine ownership or
ACLs. At least one integration test must capture real handles and prove that
the descriptor and identity used by the policy belong to the opened object.

- [ ] **Step 2: Run the trust tests and verify RED**

```powershell
cargo test -p unica-coder windows_immutable_platform -- --nocapture
```

Expected: Windows immutable capture still returns
`immutable platform trust verification is unavailable on this host`.

- [ ] **Step 3: Implement Windows trust capture**

Reject an elevated effective token and fail closed if elevation state cannot be
proved. Open the current thread token first and fall back to the process token
only when `OpenThreadToken` reports `ERROR_NO_TOKEN`.

Accept only normal or verbatim local-disk path prefixes. Query native
filesystem device information from the retained installation handle and reject
`FILE_REMOTE_DEVICE`, `FILE_DEVICE_NETWORK_FILE_SYSTEM`, UNC/device roots, and
mapped remote volumes before trusting their metadata.

Walk the volume-root-to-install ancestry one component at a time and the
complete install inventory with retained handles. Reject reparse points and
require one hard link for files.

Read owner and DACL from each opened handle with `GetSecurityInfo`. Require the
owner SID to equal TrustedInstaller, LocalSystem, or built-in Administrators.
Enumerate the DACL. For ancestry above the installation root, reject
substitution-capable effective allow ACEs (`DELETE`, `FILE_DELETE_CHILD`,
`WRITE_DAC`, `WRITE_OWNER`, `GENERIC_WRITE`, or `GENERIC_ALL`) for every
untrusted SID; creation of an unrelated sibling alone is permitted. For the
installation root and inventory, reject the complete mutation mask for every
untrusted SID. Inherited basic allow ACEs are evaluated by the same profile,
while `INHERIT_ONLY` ACEs grant no capability on the current object and deny
ACEs grant none. Object, callback, conditional, or unknown ACE forms that
cannot be evaluated unambiguously fail closed. Hash the canonical self-relative
security descriptor and store the digest beside path, kind, and identity in
`ImmutablePlatformEntry`.

Keep the Unix owner/mode implementation unchanged by making the trust evidence
platform-specific inside the entry.

- [ ] **Step 4: Run attestation and platform resolver tests**

```powershell
cargo test -p unica-coder windows_immutable_platform -- --nocapture
cargo test -p unica-coder platform_attestation -- --nocapture
cargo test -p unica-coder platform_resolver -- --nocapture
cargo check -p unica-coder --all-targets
```

- [ ] **Step 5: Commit**

```powershell
git add -- Cargo.toml Cargo.lock crates/unica-coder/src/infrastructure/platform/filesystem.rs crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs
git commit -m "feat(windows): attest immutable platform installs"
```

### Task 7: Remove the Windows pre-flight guard

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs:485-507`
- Test: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs`

**Interfaces:**
- Consumes: Tasks 1–6 Windows security, inspection, attestation, cleanup, and
  publication primitives.
- Produces: one `PreparedDump::prepare` implementation shared by Unix and Windows; unsupported non-Unix/non-Windows hosts remain fail-closed at unavailable platform primitives.

- [ ] **Step 1: Add a failing applied-dump regression**

Extend the real `DumpRunner` fixture test so Windows must reach the runner and
publish:

```rust
#[cfg(windows)]
#[test]
fn windows_applied_full_dump_reaches_runner_and_commits_validated_tree() {
    let (root, context, target) = workspace("windows-applied-full-dump");
    let runner = DumpRunner::valid();

    let result = invoke(&runner, &platform(), FullDumpInvocation::BuildDump, &context)
        .expect("Windows applied full dump");

    assert!(result.0.ok);
    assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
    assert!(target.join("Configuration.xml").is_file());
    fs::remove_dir_all(root).unwrap();
}
```

The production mutation caught is restoring the Windows pre-flight error or
failing before the platform runner is called.

- [ ] **Step 2: Run the regression and verify RED**

Run:

```powershell
cargo test -p unica-coder windows_applied_full_dump_reaches_runner_and_commits_validated_tree -- --nocapture
```

Expected: failure with `verified applied full dump is fail-closed on Windows`.

- [ ] **Step 3: Remove the dedicated Windows prepare implementation**

Delete the `#[cfg(windows)] PreparedDump::prepare` error function and widen the
real implementation from `#[cfg(not(windows))]` to all supported hosts. Do not
change argument validation, platform attestation, snapshots, stage validation,
or publication ordering.

- [ ] **Step 4: Close retained private handles before Windows disposition verification**

Add owner-only boundary regressions for synchronous relative file creation,
moving a populated child between live owner-only parents, and delete-pending
cleanup while retained anchors remain open.

Every synchronous `NtCreateFile` access mask must explicitly include
`SYNCHRONIZE`. After owner-only directory creation and validation, reopen the
child relative to its retained parent with ordinary inspection rights, compare
identity, close the DELETE-capable create handle, and return only the
least-privilege handle.

Make private execution, recovery, and root anchors takeable during cleanup.
Empty a child through its retained handle, close that handle, perform the
identity-bound delete through the parent, and then verify the parent. Close the
private root anchor before deleting and verifying the root. Preserve the
existing retry, recovery, and identity-mismatch behavior.

- [ ] **Step 5: Run focused and workspace Rust tests**

Run:

```powershell
cargo test -p unica-coder windows_applied_full_dump_reaches_runner_and_commits_validated_tree -- --nocapture
cargo test -p unica-coder infrastructure::platform::full_dump_publication -- --test-threads=1
cargo test --workspace -- --test-threads=1
```

Expected: the Windows regression and the full workspace pass.

- [ ] **Step 6: Commit or stage the task**

```powershell
git add -- crates/unica-coder/src/infrastructure/platform/filesystem.rs crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs
git commit -m "fix(runtime): enable verified Windows full dump"
```

If Git identity remains unavailable, keep the path staged.

### Task 8: Synchronize user-facing support policy

**Files:**
- Modify: `tests/ci/test_unica_skills.py:1392-1415`
- Modify: `plugins/unica/skills/v8-runner/SKILL.md:290-310`
- Modify: `plugins/unica/skills/v8-runner/references/file-and-artifact-workflows.md:25-45`
- Modify: `plugins/unica/references/tooling/v8project.md:100-117`
- Modify: `plugins/unica/references/tooling/runtime-build.md:128-145`

**Interfaces:**
- Consumes: Task 7 observed Windows support.
- Produces: documentation that routes applied full dump on Windows through the same verified synchronous operation as macOS and Linux.

- [ ] **Step 1: Replace the old documentation assertion with a behavior contract**

Rename the CI test to
`test_verified_applied_full_dump_documents_supported_hosts_and_verified_publication`.
Assert that the combined documentation:

```python
self.assertRegex(
    combined,
    re.compile(
        r"Windows.{0,240}(?:verified|transactional|no-clobber).{0,240}"
        r"(?:full dump|mode.?=.?full)",
        re.IGNORECASE | re.DOTALL,
    ),
)
self.assertNotRegex(
    combined,
    re.compile(
        r"Windows.{0,240}(?:fail-closed|blocked|unsupported).{0,240}"
        r"(?:full dump|mode.?=.?full)",
        re.IGNORECASE | re.DOTALL,
    ),
)
```

The production documentation change caught is reintroducing the obsolete claim
that Windows applied full dump is unavailable.

- [ ] **Step 2: Run the contract test and verify RED**

Run:

```powershell
python -m unittest discover -s tests/ci -p 'test_unica_skills.py' -k verified_applied_full_dump -v
```

Expected: failure because the current docs explicitly say Windows is fail-closed.

- [ ] **Step 3: Update the four documentation files**

State that synchronous applied `mode=full` for `CONFIGURATION` and `EXTENSION`
uses verified transactional publication on Windows, macOS, and Linux. Preserve
the separate restrictions for:

- incremental/partial dump receipts;
- external source-set applied dump;
- applied conversion;
- asynchronous runtime jobs.

Reference `INV-PLATFORM-OS-BEHIND-FACADE` and the existing publication owner
instead of adding a second normative rule.

- [ ] **Step 4: Run documentation and architecture checks**

Run:

```powershell
python -m unittest discover -s tests/ci -p 'test_unica_skills.py' -v
python -m unittest discover -s tests/ci -p 'test_design_documents.py' -v
python -m unittest discover -s tests/ci -p 'test_architecture_registry.py' -v
python scripts/ci/check-rust-platform-boundary.py
```

Expected: all checks pass.

- [ ] **Step 5: Commit or stage the task**

```powershell
git add -- tests/ci/test_unica_skills.py plugins/unica/skills/v8-runner/SKILL.md plugins/unica/skills/v8-runner/references/file-and-artifact-workflows.md plugins/unica/references/tooling/v8project.md plugins/unica/references/tooling/runtime-build.md
git commit -m "docs(runtime): support verified Windows full dump"
```

If Git identity remains unavailable, keep only these paths staged.

### Task 9: Final verification

**Files:**
- Verify all modified files.

**Interfaces:**
- Consumes: completed Tasks 1–8.
- Produces: evidence that the implementation, platform boundary, docs, and workspace remain green.

- [ ] **Step 1: Format and inspect**

Run:

```powershell
cargo fmt --all
git diff --check
git status --short
```

Expected: no whitespace errors; status contains only files named in this plan.

- [ ] **Step 2: Run Rust quality gates**

Run:

```powershell
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
```

Expected: both commands exit zero with no warnings.

- [ ] **Step 3: Run Python source gates**

Run:

```powershell
python -m unittest discover -s tests/ci --durations 20
python -m unittest discover -s tests/dev --durations 20
python -m py_compile scripts/ci/*.py tests/ci/*.py
python -m py_compile scripts/dev/*.py tests/dev/*.py
python scripts/ci/check-version-contract.py
python scripts/ci/check-rust-platform-boundary.py
```

Expected: all commands exit zero.

- [ ] **Step 4: Run architecture synchronization**

Run:

```powershell
$base = git merge-base HEAD origin/main
python scripts/ci/check-architecture-sync.py --base $base
```

Expected: exit zero; no ADR is required because the design implements ADR-0009
without changing the public surface or architectural rule ownership.

- [ ] **Step 5: Review the final diff against issue #268**

Confirm from the diff and test output:

- no Windows pre-flight guard remains;
- applied full dump reaches the runner;
- private creation uses an explicit protected DACL;
- directory opens reject reparse points;
- publication never replaces an occupied destination;
- rollback preserves a concurrent occupant;
- docs no longer report Windows full dump as unsupported.

- [ ] **Step 6: Create the final commit if identity is configured**

```powershell
git add -- docs/design/2026-07-30-windows-applied-full-dump-design.md docs/plans/2026-07-30-windows-applied-full-dump.md
git commit -m "fix(runtime): support verified Windows full dump"
```

If task commits were already created, amend neither history nor unrelated user
changes; commit only the remaining design/plan artifacts.
