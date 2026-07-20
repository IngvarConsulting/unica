# Safe Single-File Publisher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace PR #163's unsafe shared BOM-writer change with one typed single-file publisher used by `CompileTransaction` and `cf-edit`.

**Architecture:** `single_file_publisher.rs` owns locks, target policy, exact-preimage checks, staging, atomic create/replace, and cleanup reporting. Host-specific metadata stays in `platform/filesystem.rs`. `CompileTransaction` reuses prepared publications while retaining multi-file validation and rollback; `cf-edit` uses the one-file API with its exact raw preimage.

**Tech Stack:** Rust 2021, `std::fs`, `fs2`, `sha2`, `windows-sys`, existing Unica unit/application/platform tests.

---

## Preconditions and file map

Baseline at `dc941e0`:

- `cargo build -p unica-coder` passed.
- `cargo test -p unica-coder` passed: 598 unit + 3 integration, 2 ignored, 0 failed.
- Branch: `feat/issue-74-writer-mutation-contract-pr`.

Files:

- Create `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`.
- Modify `crates/unica-coder/src/infrastructure/native_operations.rs`.
- Modify `crates/unica-coder/src/infrastructure/platform/filesystem.rs`.
- Modify `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`.
- Modify `crates/unica-coder/src/infrastructure/native_operations/common.rs`.
- Modify `crates/unica-coder/src/infrastructure/native_operations/cf.rs`.
- Modify `crates/unica-coder/src/application/mod.rs`.
- Modify `AGENTS.md` and the approved design status.

### Task 1: Platform filesystem primitives

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs`
- Test: same file

- [ ] **Step 1: Write failing platform tests**

Add tests with pid+nanos temporary roots:

```rust
#[test]
fn no_clobber_install_never_replaces_an_existing_target() {
    let root = temp_root("no-clobber");
    let staged = root.join("stage");
    let target = root.join("target");
    fs::write(&staged, b"replacement").unwrap();
    fs::write(&target, b"original").unwrap();

    let error = install_file_no_clobber(&staged, &target).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::read(&staged).unwrap(), b"replacement");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hard_link_count_observes_a_second_name() {
    let root = temp_root("links");
    let target = root.join("target");
    let alias = root.join("alias");
    fs::write(&target, b"bytes").unwrap();
    fs::hard_link(&target, &alias).unwrap();
    assert_eq!(hard_link_count(&File::open(&target).unwrap()).unwrap(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn portable_permissions_round_trip_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("permissions");
    let source = root.join("source");
    let staged = root.join("stage");
    fs::write(&source, b"source").unwrap();
    fs::write(&staged, b"stage").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();

    let expected = portable_permissions(&fs::metadata(&source).unwrap());
    let staged_file = OpenOptions::new().read(true).write(true).open(&staged).unwrap();
    expected.apply_to(&staged_file).unwrap();

    assert!(expected.matches(&staged_file.metadata().unwrap()));
    assert_eq!(staged_file.metadata().unwrap().permissions().mode() & 0o7777, 0o600);
    fs::remove_dir_all(root).unwrap();
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder --lib infrastructure::platform::filesystem::tests
```

Expected: compile errors for the missing platform functions/types.

- [ ] **Step 3: Implement the platform API**

Add:

```rust
#[derive(Debug, Clone)]
pub(crate) struct PortablePermissions {
    permissions: fs::Permissions,
    key: u32,
}

impl PortablePermissions {
    pub(crate) fn readonly(&self) -> bool { self.permissions.readonly() }
    pub(crate) fn matches(&self, metadata: &fs::Metadata) -> bool {
        self.key == permission_key(&metadata.permissions())
    }
    pub(crate) fn apply_to(&self, file: &fs::File) -> io::Result<()> {
        file.set_permissions(self.permissions.clone())
    }
}

pub(crate) fn portable_permissions(metadata: &fs::Metadata) -> PortablePermissions;
pub(crate) fn restrict_stage_to_owner(file: &fs::File) -> io::Result<()>;
pub(crate) fn hard_link_count(file: &fs::File) -> io::Result<u64>;
pub(crate) fn install_file_no_clobber(source: &Path, target: &Path) -> io::Result<()> {
    fs::hard_link(source, target)
}
```

Implementations:

- Unix permission key is `mode() & 0o7777`, private stage mode is `0600`, link count is `MetadataExt::nlink()`.
- Windows permission key is readonly; private-stage chmod is a no-op; link count uses `GetFileInformationByHandle` and `nNumberOfLinks` from `windows-sys`.
- Other hosts use readonly as key and no-op restriction; hard-link count returns `io::ErrorKind::Unsupported` so replacement fails closed instead of assuming one link.
- Keep every host cfg in this platform file.

- [ ] **Step 4: Verify GREEN**

```bash
cargo test -p unica-coder --lib infrastructure::platform::filesystem::tests
python3 scripts/ci/check-rust-platform-boundary.py
```

Expected: tests and boundary check exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/platform/filesystem.rs
git commit -m "Добавить файловые примитивы безопасной публикации" -m "Co-Authored-By: codex <codex@openai.com>"
```

### Task 2: Typed publisher, locks, and happy paths

**Files:**
- Create: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`
- Test: new module

- [ ] **Step 1: Register the module and add RED tests**

Add `pub(crate) mod single_file_publisher;`. Test:

```rust
#[test]
fn create_only_publishes_exact_bytes_and_returns_created() {
    let root = temp_root("create");
    let target = root.join("created.xml");
    let report = publish(PublishRequest {
        target: &target,
        replacement: b"complete",
        mode: PublishMode::CreateOnly,
    }).unwrap();
    assert_eq!(report.effect, PublishEffect::Created);
    assert_eq!(fs::read(&target).unwrap(), b"complete");
    assert!(publication_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replace_existing_publishes_exact_bytes_and_returns_replaced() {
    let root = temp_root("replace");
    let target = root.join("Configuration.xml");
    fs::write(&target, b"before").unwrap();
    let report = publish(PublishRequest {
        target: &target,
        replacement: b"after",
        mode: PublishMode::ReplaceExisting { expected_preimage: b"before" },
    }).unwrap();
    assert_eq!(report.effect, PublishEffect::Replaced);
    assert_eq!(fs::read(&target).unwrap(), b"after");
    assert!(publication_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn identical_replacement_returns_unchanged_without_staging() {
    let root = temp_root("unchanged");
    let target = root.join("Configuration.xml");
    fs::write(&target, b"same").unwrap();
    let report = publish(PublishRequest {
        target: &target,
        replacement: b"same",
        mode: PublishMode::ReplaceExisting { expected_preimage: b"same" },
    }).unwrap();
    assert_eq!(report.effect, PublishEffect::Unchanged);
    assert!(publication_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_preimage_is_rejected_before_staging() {
    let root = temp_root("stale");
    let target = root.join("Configuration.xml");
    fs::write(&target, b"current").unwrap();
    let error = publish(PublishRequest {
        target: &target,
        replacement: b"next",
        mode: PublishMode::ReplaceExisting { expected_preimage: b"older" },
    }).unwrap_err();
    assert!(matches!(error.kind(), PublishErrorKind::StalePreimage { .. }));
    assert_eq!(fs::read(&target).unwrap(), b"current");
    assert!(publication_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}
```

- [ ] **Step 2: Verify RED**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::single_file_publisher::tests
```

Expected: typed API is absent.

- [ ] **Step 3: Define exact request/result/error types**

```rust
pub(crate) enum PublishMode<'a> {
    CreateOnly,
    ReplaceExisting { expected_preimage: &'a [u8] },
}
pub(crate) struct PublishRequest<'a> {
    pub(crate) target: &'a Path,
    pub(crate) replacement: &'a [u8],
    pub(crate) mode: PublishMode<'a>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishEffect { Created, Replaced, Unchanged }
pub(crate) struct PublishReport {
    pub(crate) effect: PublishEffect,
    pub(crate) cleanup_warnings: Vec<CleanupWarning>,
}
pub(crate) struct CleanupWarning {
    pub(crate) path: PathBuf,
    pub(crate) message: String,
}
pub(crate) struct PublishError {
    kind: PublishErrorKind,
    cleanup_warnings: Vec<CleanupWarning>,
}
pub(crate) enum PublishErrorKind {
    InvalidTarget { target: PathBuf },
    AlreadyExists { target: PathBuf },
    MissingTarget { target: PathBuf },
    LinkOrReparsePoint { target: PathBuf },
    NonRegular { target: PathBuf },
    ReadOnly { target: PathBuf },
    MultipleHardLinks { target: PathBuf, count: u64 },
    StalePreimage { target: PathBuf },
    MetadataChanged { target: PathBuf },
    StageCollisionsExhausted { target: PathBuf, attempts: usize },
    Io { phase: PublishPhase, path: PathBuf, source: io::Error },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishPhase {
    Inspect, Lock, Stage, Write, Flush, Sync,
    Permissions, Validate, Recheck, Commit, Cleanup,
}
```

Implement typed getters, `Display`, and `Error`. Internal tests match variants, never messages.

- [ ] **Step 4: Move common lock machinery from CompileTransaction**

Expose:

```rust
pub(crate) fn with_publication_locks<T>(
    targets: &[PathBuf],
    action: impl FnOnce(&PublicationLockToken<'_>) -> T,
) -> Result<T, PublishError>;
```

Requirements:

- Identity is canonical existing parent plus target filename; target itself is never canonicalized.
- Sort/deduplicate before acquiring process mutexes and `fs2` lock files.
- Guards remain alive through the callback.
- Keep persistent lock files.
- `PublicationLockToken` stores allowed identities and `prepare` rejects a target outside the token.

- [ ] **Step 5: Implement prepared states and high-level publish**

```rust
pub(crate) enum PreparedPublication<'request, 'lock> {
    Unchanged,
    Create(PreparedCreate<'request, 'lock>),
    Replace(PreparedReplace<'request, 'lock>),
}
pub(crate) fn prepare<'request, 'lock>(
    lock: &'lock PublicationLockToken<'_>,
    request: PublishRequest<'request>,
) -> Result<PreparedPublication<'request, 'lock>, PublishError>;
pub(crate) fn publish(request: PublishRequest<'_>) -> Result<PublishReport, PublishError> {
    with_publication_locks(&[request.target.to_path_buf()], |lock| {
        prepare(lock, request)?.commit()
    })?
}
```

`PreparedCreate` and `PreparedReplace` are non-Clone, borrow the lock token, own their stage guard, and expose consuming `commit(self)` plus `discard(self)`. `PreparedReplace::portable_permissions()` exposes the authoritative snapshot only for transaction recovery.

Stage rules: sibling path, `create_new(true)`, pid + `AtomicU64`, 16 collision attempts. Capture process-default creation permissions, restrict before the first byte, write/flush/sync, then restore captured permissions for CreateOnly or apply the target snapshot for ReplaceExisting, sync again, and perform exact readback. Create commits via no-clobber + unlink; replace via `replace_file_atomically`. Unchanged returns before staging.

- [ ] **Step 6: Verify GREEN and commit**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::single_file_publisher::tests
git add crates/unica-coder/src/infrastructure/native_operations.rs crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs
git commit -m "Добавить типизированный publisher одного файла" -m "Co-Authored-By: codex <codex@openai.com>"
```

### Task 3: Target safety policy

**Files:**
- Modify/Test: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`

- [ ] **Step 1: Add RED tests**

Add:

- `replace_preserves_unix_mode_0600`.
- `read_only_target_is_rejected_unchanged` using mode `0400`.
- `link_or_reparse_target_is_rejected_without_touching_referent`.
- On Windows, the same test exercises a reparse point when the test host permits creating one; an unsupported privilege is reported as a skipped fixture, not a passing mutation.
- `non_regular_target_is_rejected`.
- `multiple_hard_links_are_rejected`.
- `create_only_rejects_every_existing_target_kind`.

Every test asserts target/referent/alias bytes and permissions remain unchanged and no stage debris remains.

- [ ] **Step 2: Verify RED**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::single_file_publisher::tests
```

Expected: policy tests fail.

- [ ] **Step 3: Implement authoritative snapshots**

```rust
struct ReplaceSnapshot {
    bytes: Vec<u8>,
    permissions: PortablePermissions,
    hard_link_count: u64,
}
fn inspect_replace_target(
    target: &Path,
    expected_preimage: &[u8],
    phase: PublishPhase,
) -> Result<ReplaceSnapshot, PublishError>;
```

Inspection order: `symlink_metadata`; reject link/reparse/nonregular; open read-only; reject readonly; read hard-link count from handle and reject count != 1; read and compare exact preimage. Before commit repeat inspection and require permissions/link count to match the first snapshot. Apply snapshot permissions to replacement stage. CreateOnly accepts absence only, both at prepare and pre-commit.

- [ ] **Step 4: Verify GREEN and commit**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::single_file_publisher::tests
git add crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs
git commit -m "Защитить тип и права publish-target" -m "Co-Authored-By: codex <codex@openai.com>"
```

### Task 4: Cleanup, collision, and race proofs

**Files:**
- Modify/Test: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`

- [ ] **Step 1: Add RED lifecycle tests**

Add:

- `target_changed_after_staging_is_rejected`.
- `permission_change_before_commit_is_rejected`.
- `stage_collisions_retry_without_clobbering`.
- `stage_collision_exhaustion_is_typed` with 16 attempts.
- `precommit_failpoints_preserve_target_and_remove_stage` over Write, Flush, Sync, Permissions, Validate, Recheck, Commit.
- `cleanup_failure_is_attached_to_primary_error`.
- `committed_create_with_cleanup_failure_returns_warning`.
- `create_only_detects_target_created_before_commit`.

- [ ] **Step 2: Verify RED**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::single_file_publisher::tests
```

Expected: hook/failpoint/collision behavior is absent.

- [ ] **Step 3: Implement RAII and test controls**

```rust
struct StageGuard { path: PathBuf, armed: bool }
impl StageGuard {
    fn cleanup(&mut self) -> Result<(), CleanupWarning>;
    fn disarm(&mut self);
}
impl Drop for StageGuard {
    fn drop(&mut self) {
        if self.armed { let _ = remove_stage(&self.path); }
    }
}

#[cfg(test)]
pub(crate) enum PublishFailpoint {
    Write, Flush, Sync, Permissions, Validate, Recheck, Commit, Cleanup,
}
#[cfg(test)]
pub(crate) fn with_publish_failpoints<T>(
    failpoints: &[PublishFailpoint],
    action: impl FnOnce() -> T,
) -> T;
#[cfg(test)]
pub(crate) fn with_before_commit_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T;
#[cfg(test)]
pub(crate) fn with_publication_lock_pause<T>(
    acquired: Arc<Barrier>,
    release: Arc<Barrier>,
    action: impl FnOnce() -> T,
) -> T;
#[cfg(test)]
pub(crate) fn with_publication_lock_contention_signal<T>(
    sender: Sender<()>,
    action: impl FnOnce() -> T,
) -> T;
```

`create_stage_with_candidates` accepts a candidate-path closure so collision and exhaustion tests do not alter the global sequence. `remove_stage` uses `prepare_file_for_removal` then `remove_file`; NotFound is success. Pre-commit cleanup warnings attach to `PublishError`. A committed create returns cleanup warning with `Created` rather than false `Err`. Drop is last-resort cleanup only.

- [ ] **Step 4: Verify GREEN and commit**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::single_file_publisher::tests
git add crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs
git commit -m "Гарантировать cleanup staged publication" -m "Co-Authored-By: codex <codex@openai.com>"
```

### Task 5: CompileTransaction integration

**Files:**
- Modify/Test: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`

- [ ] **Step 1: Add RED regressions**

Add:

```rust
#[cfg(unix)]
#[test]
fn post_validation_rollback_restores_bytes_and_unix_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("rollback-mode");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let mut tx = CompileTransaction::new();
    tx.register_canonical_child(&config, "Role", "Reader").unwrap();
    let error = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || tx.commit())
        .unwrap_err();
    assert!(error.contains("post-write validation"));
    assert_eq!(fs::read(&config).unwrap(), original);
    assert_eq!(fs::metadata(&config).unwrap().permissions().mode() & 0o7777, 0o600);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}
```

Also add:

- `registration_target_remains_present_after_backup_preparation` using a pause after backup creation.
- `compile_transaction_rejects_readonly_registration_without_partial_creates`.
- `compile_transaction_rejects_hard_linked_registration_without_mutation`.
- `post_validation_failure_rolls_back_two_registrations_and_one_create`.

Assert original bytes/modes, no created object/directories, no debris.

- [ ] **Step 2: Verify RED**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::compile_transaction::tests
```

Expected: backup-presence and new target-policy tests fail.

- [ ] **Step 3: Replace private locks/stages**

Remove transaction-owned lock registry/helpers, target policy helpers, stage generator/write helpers, `PlannedRegistration.lock_path`/`original_permissions`, and `PublishState.staged_paths`.

Commit flow:

1. semantic preflight;
2. create/track missing parents;
3. collect creates + changed registrations;
4. `with_publication_locks` once for all paths;
5. prepare all publications;
6. commit creates;
7. recovery-copy then commit each replacement;
8. post-validate;
9. finalize or rollback inside the same lock callback.

Locks must remain held through cleanup/rollback.

- [ ] **Step 4: Use recovery copies and atomic rollback**

Use explicit states:

```rust
struct PendingRecovery { path: PathBuf, directory: PathBuf }
struct PublishedRegistration {
    target: PathBuf,
    recovery: PathBuf,
    recovery_directory: PathBuf,
    original: Vec<u8>,
}
```

`PendingRecovery` owns an armed cleanup guard and converts into `PublishedRegistration` only after replacement commits. Publisher cleanup warnings are appended to `CommitReport.cleanup_warnings`.

Write recovery from expected original bytes, apply authoritative permissions exposed by `PreparedReplace`, sync, and only then commit. Never use `fs::copy` or hard-link backup. `AfterRegistrationBackup` fires while original target still exists. Rollback calls:

```rust
replace_file_atomically(&published.recovery, &published.target)
```

Never remove target first. On rollback failure preserve recovery and include its path in error.

- [ ] **Step 5: Verify GREEN and commit**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::compile_transaction::tests
cargo test -p unica-coder --lib application::tests::meta_compile_
cargo test -p unica-coder --lib application::tests::role_compile_
git add crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs
git commit -m "Разделить publisher с CompileTransaction" -m "Co-Authored-By: codex <codex@openai.com>"
```

### Task 6: cf-edit integration without broad writer migration

**Files:**
- Modify/Test: `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- Test: `crates/unica-coder/src/application/mod.rs`

- [ ] **Step 1: Add RED common tests**

```rust
#[test]
fn utf8_bom_bytes_emits_exactly_one_bom() {
    assert_eq!(utf8_bom_bytes("<xml/>"), b"\xef\xbb\xbf<xml/>");
    assert_eq!(utf8_bom_bytes("\u{feff}\u{feff}<xml/>"), b"\xef\xbb\xbf<xml/>");
}
#[test]
fn utf8_snapshot_keeps_raw_preimage_and_decodes_text_without_bom() {
    let root = temp_root("snapshot");
    let path = root.join("Configuration.xml");
    let raw = b"\xef\xbb\xbf<xml/>\r\n";
    fs::write(&path, raw).unwrap();
    let snapshot = read_utf8_sig_snapshot(&path).unwrap();
    assert_eq!(snapshot.raw, raw);
    assert_eq!(snapshot.text, "<xml/>\r\n");
    fs::remove_dir_all(root).unwrap();
}
```

- [ ] **Step 2: Add RED public cf-edit tests**

Near `cf_edit_args` add:

- `cf_edit_preserves_unix_mode_0600`.
- `cf_edit_rejects_readonly_configuration_unchanged`.
- `cf_edit_rejects_symlink_configuration_without_touching_referent`.
- `cf_edit_rejects_hard_linked_configuration_unchanged`.
- `cf_edit_equal_serialized_result_is_a_public_noop`: exact bytes, empty changes/cache events.
- `compile_transaction_and_cf_edit_share_target_lock`: both read same preimage; first commits, second gets stale even if desired bytes equal current.

Use only `modify-property`.

- [ ] **Step 3: Verify RED**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::common::mutation_tests
cargo test -p unica-coder --lib application::tests::cf_edit_
```

Expected: snapshot/encoder and publisher wiring absent.

- [ ] **Step 4: Implement one-read snapshots and restore legacy writer**

```rust
pub(crate) struct Utf8TextSnapshot {
    pub(crate) raw: Vec<u8>,
    pub(crate) text: String,
}
pub(crate) fn read_utf8_sig_snapshot(path: &Path) -> Result<Utf8TextSnapshot, String> {
    let raw = fs::read(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let text = std::str::from_utf8(&raw)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?
        .trim_start_matches('\u{feff}')
        .to_string();
    Ok(Utf8TextSnapshot { raw, text })
}
pub(crate) fn read_utf8_sig(path: &Path) -> Result<String, String> {
    read_utf8_sig_snapshot(path).map(|snapshot| snapshot.text)
}
pub(crate) fn utf8_bom_bytes(content: &str) -> Vec<u8> {
    let content = content.trim_start_matches('\u{feff}');
    let mut bytes = Vec::with_capacity(3 + content.len());
    bytes.extend_from_slice(b"\xef\xbb\xbf");
    bytes.extend_from_slice(content.as_bytes());
    bytes
}
```

Restore `write_utf8_bom` to direct `File::create` + `write_all` of encoder bytes. Delete PR-local `atomic_replace` and its broad test. Other writers retain pre-PR semantics.

- [ ] **Step 5: Map actual publisher effects in cf-edit**

Add:

```rust
struct CfEditRun {
    stdout: String,
    config_path: PathBuf,
    artifacts: Vec<PathBuf>,
    config_updated: bool,
    warnings: Vec<String>,
}
```

Retain raw/text snapshot. Encode replacement and call `publish` with `ReplaceExisting { expected_preimage: &source_raw }`. Map exhaustively:

- `Replaced`: Saved line, `config_updated = true`.
- `Unchanged`: No Configuration.xml changes, `config_updated = false`.
- `Created`: internal invariant error.
- Cleanup warnings go to `AdapterOutcome.warnings`.

Build changes/cache-event-driving state from `config_updated`, not logical `config_changed`.

- [ ] **Step 6: Verify GREEN and commit**

```bash
cargo test -p unica-coder --lib infrastructure::native_operations::common::mutation_tests
cargo test -p unica-coder --lib application::tests::cf_edit_
cargo test -p unica-coder --lib infrastructure::native_operations::compile_transaction::tests
git add crates/unica-coder/src/infrastructure/native_operations/common.rs crates/unica-coder/src/infrastructure/native_operations/cf.rs crates/unica-coder/src/application/mod.rs
git commit -m "Подключить cf-edit к безопасному publisher" -m "Co-Authored-By: codex <codex@openai.com>"
```

### Task 7: PR hygiene, Rust review, and verification

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/specs/2026-07-20-safe-single-file-publisher-design.md`
- Modify Rust files only when review requires a tested correction

- [ ] **Step 1: Mark design approved**

Set:

```markdown
- Status: approved on 2026-07-20; implementation authorized
```

- [ ] **Step 2: Remove only the two unrelated AGENTS lines**

Delete the Superpowers workflow line and the Rust-review workflow line added by commit `91f1fa8`. Do not alter any other instruction.

- [ ] **Step 3: Apply `rust-expert-best-practices-code-review`**

Check exhaustive enums, typed errors with `io::Error` sources, no runtime-input panics, cfg boundary, single-consumption ownership states, stable lock order, post-commit warning semantics, and localized Windows unsafe safety comment. For every behavioral correction, write and run a failing regression first.

- [ ] **Step 4: Run fresh full verification**

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets -- -D warnings
cargo test -p unica-coder
python3 scripts/ci/check-rust-platform-boundary.py
git diff --check
git diff --check a2793c9..HEAD
git diff a2793c9..HEAD -- AGENTS.md
rg -n "atomic_replace" crates/unica-coder/src/infrastructure/native_operations/common.rs
```

Expected: all commands exit 0, 0 test failures, no AGENTS diff, no common `atomic_replace`.

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md docs/superpowers/specs/2026-07-20-safe-single-file-publisher-design.md crates/unica-coder/src
git commit -m "Завершить ревью safe publisher" -m "Co-Authored-By: codex <codex@openai.com>"
```

If review changes no Rust source, stage only AGENTS and design status.

### Task 8: GitHub delivery

**External state:** issue #74, one publisher child issue, PR #163, review replies, branch push.

- [ ] **Step 1: Re-read current state**

```bash
gh issue view 74 --repo IngvarConsulting/unica --json number,title,body,state,labels,url
gh pr view 163 --repo IngvarConsulting/unica --json number,title,body,state,reviewDecision,reviews,comments,commits,url
gh api repos/IngvarConsulting/unica/pulls/163/comments
```

- [ ] **Step 2: Create focused child issue**

Acceptance must match implemented modes, policy, permissions, locks/preimage, cleanup, and the two initial consumers. Deferred section must name EOL/hash/ranges, all-writer migration, ACL/xattr/owner, adversarial TOCTOU, and power-loss durability. Link parent #74 and PR #163.

- [ ] **Step 3: Rewrite #74 as parent epic**

Include problem, writer invariant matrix, child slices, publisher child linked to PR #163, `code.patch` dependency, deferred guarantees, and epic-level completion criteria. PR #163 must not close the epic.

- [ ] **Step 4: Update PR and Igor threads**

PR body: `Refs #74`, `Closes #<child>`, exact guarantees, exact deferrals, consumer list, and verification evidence. Reply to each actionable Igor thread with matching code/test. Do not resolve unimplemented requests.

- [ ] **Step 5: Push and inspect checks**

```bash
git status --short --branch
git push origin feat/issue-74-writer-mutation-contract-pr
gh pr checks 163 --repo IngvarConsulting/unica
```

Report pending checks as pending.

- [ ] **Step 6: Finish branch without merging**

Use `superpowers:finishing-a-development-branch`. Keep PR #163 open; merging requires a separate instruction.
