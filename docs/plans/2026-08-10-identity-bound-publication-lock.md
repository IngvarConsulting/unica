# Identity-bound Publication Lock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure every identity-bound recovery cleanup, retry, finalize, and rollback mutation runs under the publication lock that owns its target, including cleanup performed after a failed locked commit.

**Architecture:** Reacquire the original target and guard lock set before deferred error cleanup, then thread the live `PublicationLockToken` through destructive publication and recovery APIs. Bind every `CleanupArtifact` to the publication identity that authorized its creation so an unrelated lock token cannot authorize later removal. Keep descriptor-relative Unix operations and handle-relative Windows operations behind the existing platform facade.

**Tech Stack:** Rust 2021, `fs2` advisory file locks, platform filesystem facade, Rust unit tests, Python 3.12 CI contract tests, GitHub Actions.

## Global Constraints

- Work only in the existing PR #396 head branch `fix/issue-356-identity-safe-cleanup-v012` from `korolevpavel/unica`; do not create a child PR.
- Start every defect fix with a test that fails on the current PR head for the intended reason.
- Preserve the single public MCP server `unica` and the existing `unica.*` surface.
- Preserve the existing result, cleanup-warning, and rollback-diagnostic shapes.
- On Unix, the guarantee covers replaced lexical routes and cooperating Unica writers; it is not a security boundary against a hostile process under the same Unix UID.
- On Windows, keep deletion and rename bound to the verified open child handle.
- Unsupported platforms continue to fail closed.
- No new ADR is required: this plan enforces the existing publication-lock and rollback-visibility contracts.
- Do not modify format specifications, package manifests, version numbers, or release files.

---

### Task 1: Reacquire publication locks before failed-commit cleanup

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:930-1040`
- Test: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:3990-4210`
- Test: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:4740-5335`

**Interfaces:**
- Consumes: `with_publication_locks_mode_and_guard_targets(targets, guard_targets, tree_mode, action)` and the existing `targets`, `guard_targets`, `tree_lock_mode`, and `PublishState` values in `commit_with_classified_post_validation`.
- Produces: `cleanup_after_failed_commit(lock, state) -> Vec<String>`; the callback always runs while the reacquired publication lock set is live.

- [ ] **Step 1: Add a test-only hook at the failed-commit cleanup boundary**

Add the hook beside `TEST_BEFORE_CLEANUP_RETRY_HOOK`:

```rust
#[cfg(test)]
type BeforeFailedCommitCleanupHook = Box<dyn FnOnce()>;

#[cfg(test)]
type BeforeFailedCommitCleanupRelockHook = Box<dyn FnOnce()>;

#[cfg(test)]
thread_local! {
    static TEST_BEFORE_FAILED_COMMIT_CLEANUP_HOOK:
        RefCell<Option<BeforeFailedCommitCleanupHook>> = const { RefCell::new(None) };
    static TEST_BEFORE_FAILED_COMMIT_CLEANUP_RELOCK_HOOK:
        RefCell<Option<BeforeFailedCommitCleanupRelockHook>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn with_before_failed_commit_cleanup_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeFailedCommitCleanupHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_FAILED_COMMIT_CLEANUP_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_BEFORE_FAILED_COMMIT_CLEANUP_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn run_before_failed_commit_cleanup() {
    #[cfg(test)]
    if let Some(hook) = TEST_BEFORE_FAILED_COMMIT_CLEANUP_HOOK
        .with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(test)]
fn with_before_failed_commit_cleanup_relock_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeFailedCommitCleanupRelockHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_FAILED_COMMIT_CLEANUP_RELOCK_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_BEFORE_FAILED_COMMIT_CLEANUP_RELOCK_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn run_before_failed_commit_cleanup_relock() {
    #[cfg(test)]
    if let Some(hook) = TEST_BEFORE_FAILED_COMMIT_CLEANUP_RELOCK_HOOK
        .with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}
```

Call `run_before_failed_commit_cleanup_relock()` and then
`run_before_failed_commit_cleanup()` immediately before the current outer call
to `retry_and_finish_recovery_cleanups`. At this RED stage, do not reacquire
locks yet.

- [ ] **Step 2: Write the failing contention regression**

Add a test named `failed_commit_cleanup_reacquires_the_publication_lock` using the existing publication contention helpers:

```rust
#[test]
fn failed_commit_cleanup_reacquires_the_publication_lock() {
    let root = temp_root("failed-commit-cleanup-lock");
    let config = root.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");

    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader")
        .expect("registration must plan");

    let competing_target = config.clone();
    let competing_thread = Arc::new(Mutex::new(None));
    let competing_thread_for_hook = Arc::clone(&competing_thread);

    let error = with_before_failed_commit_cleanup_hook(
        move || {
            let (contended_sender, contended_receiver) = mpsc::channel();
            let thread = thread::spawn(move || {
                with_publication_lock_contention_signal(contended_sender, || {
                    with_publication_locks(&[competing_target], |_| ())
                })
                .expect("competing writer must eventually acquire its lock");
            });
            contended_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("failed-commit cleanup must still own the target lock");
            *competing_thread_for_hook.lock().unwrap() = Some(thread);
        },
        || {
            with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                transaction.commit()
            })
        },
    )
    .expect_err("post-write validation must fail");

    if let Some(thread) = competing_thread.lock().unwrap().take() {
        thread.join().expect("competing writer must not panic");
    }
    assert!(error.contains("post-write validation"), "{error}");
    fs::remove_dir_all(root).expect("temporary root must be removed");
}
```

Add a second regression that makes lock reacquisition impossible after retained
cleanup debris exists:

```rust
#[test]
fn failed_commit_cleanup_reports_paths_when_lock_reacquisition_fails() {
    let root = temp_root("failed-commit-cleanup-relock-failure");
    let active = root.join("active");
    let preserved = root.join("preserved");
    fs::create_dir(&active).expect("active parent must be created");
    let config = active.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");

    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader")
        .expect("registration must plan");
    let active_for_hook = active.clone();
    let preserved_for_hook = preserved.clone();

    let error = with_before_failed_commit_cleanup_relock_hook(
        move || {
            fs::rename(&active_for_hook, &preserved_for_hook)
                .expect("the locked parent must be displaced before reacquisition");
        },
        || {
            with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
                with_commit_failpoint(CommitFailpoint::AfterRegistrationBackup, || {
                    transaction.commit()
                })
            })
        },
    )
    .expect_err("registration-backup failpoint must abort publication");

    assert!(
        error.contains("failed to reacquire publication locks for deferred cleanup"),
        "{error}"
    );
    assert!(
        error.contains(".unica-stage-") || error.contains(".unica-recovery-"),
        "retained cleanup path is missing: {error}"
    );
    assert!(
        fs::read_dir(&preserved).unwrap().any(|entry| {
            entry.unwrap().file_name().to_string_lossy().contains(".unica-")
        }),
        "failed reacquisition must preserve transaction debris"
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}
```

- [ ] **Step 3: Run the new test and verify RED**

Run:

```bash
cargo test -p unica-coder failed_commit_cleanup_reacquires_the_publication_lock -- --nocapture
cargo test -p unica-coder failed_commit_cleanup_reports_paths_when_lock_reacquisition_fails -- --nocapture
```

Expected: the contention test fails after the two-second timeout with
`failed-commit cleanup must still own the target lock`; the route-swap test
fails because the current code never reports a lock-reacquisition failure.

- [ ] **Step 4: Move failed-commit cleanup into a reacquired lock scope**

Extract the mutation sequence:

```rust
fn cleanup_after_failed_commit(
    _lock: &PublicationLockToken<'_>,
    state: &mut PublishState,
) -> Vec<String> {
    run_before_failed_commit_cleanup();
    let mut errors = retry_and_finish_recovery_cleanups(state);
    errors.extend(cleanup_created_directories(&mut state.created_dirs));
    errors
}
```

Replace the outer error arm with lock reacquisition over the same inputs:

```rust
Err(error) => {
    let primary = adapt_publish_error(&error, PublicationRole::Transaction);
    record_publish_error_cleanup(&mut state, &error);
    run_before_failed_commit_cleanup_relock();
    let cleanup = with_publication_locks_mode_and_guard_targets(
        &targets,
        &guard_targets,
        tree_lock_mode,
        |lock| cleanup_after_failed_commit(lock, &mut state),
    );
    let mut cleanup_errors = match cleanup {
        Ok(errors) => errors,
        Err(lock_error) => vec![format!(
            "failed to reacquire publication locks for deferred cleanup: {lock_error}"
        )],
    };
    cleanup_errors.extend(std::mem::take(&mut state.cleanup_warnings));
    Err(with_cleanup_diagnostics(primary, cleanup_errors))
}
```

Do not retry destructive cleanup without a token when lock reacquisition fails.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run:

```bash
cargo test -p unica-coder failed_commit_cleanup_reacquires_the_publication_lock -- --nocapture
cargo test -p unica-coder failed_commit_cleanup_reports_paths_when_lock_reacquisition_fails -- --nocapture
cargo test -p unica-coder infrastructure::native_operations::compile_transaction::tests -- --test-threads=1
```

Expected: both new tests pass; reacquisition failure preserves debris and names
its path; all compile-transaction tests pass with zero failures.

- [ ] **Step 6: Commit Task 1**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs
git commit -m "fix(publication): reacquire locks for deferred cleanup"
```

### Task 2: Bind cleanup artifacts and destructive recovery calls to a live lock token

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs:72-116`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs:481-805`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs:975-1027`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs:1440-1665`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:1015-1335`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:2090-2320`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:2840-2980`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:3140-3855`
- Modify: `crates/unica-coder/src/infrastructure/platform/filesystem.rs:2896-3155`
- Test: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs:1650-2240`
- Test: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs:5250-5705`

**Interfaces:**
- Consumes: `PublicationLockToken.allowed_identities`, `publication_identity(target)`, and the lock token supplied to `prepare`/`commit_locked`.
- Produces: `CleanupArtifact.lock_identity: String`; `cleanup_publication_artifact(lock, artifact)`; lock-requiring recovery cleanup, rollback, and finalize functions.

- [ ] **Step 1: Write a failing wrong-lock cleanup test**

Create an identity-bound warning with the cleanup failpoint, then try to clean it while holding a different target lock:

```rust
#[test]
fn cleanup_artifact_rejects_an_unrelated_publication_lock() {
    let root = unique_temp_root("cleanup-wrong-lock");
    let target = root.join("created.bin");
    let unrelated = root.join("unrelated.bin");
    let report = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
        publish(PublishRequest {
            target: &target,
            replacement: b"published bytes",
            mode: PublishMode::CreateOnly,
        })
    })
    .expect("committed create must retain its cleanup warning");
    let warning = report.cleanup_warnings.into_iter().next().unwrap();

    let cleanup_error = with_publication_locks(&[unrelated], |lock| {
        cleanup_publication_artifact(lock, &warning.artifact)
    })
    .expect("unrelated lock acquisition itself must succeed")
    .expect_err("an unrelated lock must not authorize cleanup");

    assert!(
        cleanup_error.message.contains("publication lock identity"),
        "{cleanup_error}"
    );
    assert!(warning.path.exists(), "the owned artifact must remain retained");

    with_publication_locks(&[target], |lock| {
        cleanup_publication_artifact(lock, &warning.artifact)
    })
    .expect("correct lock acquisition must succeed")
    .expect("the matching lock must authorize cleanup");
    fs::remove_dir_all(root).unwrap();
}
```

- [ ] **Step 2: Run the wrong-lock test and verify RED**

Run:

```bash
cargo test -p unica-coder cleanup_artifact_rejects_an_unrelated_publication_lock -- --nocapture
```

Expected: compilation fails because `cleanup_publication_artifact` does not yet accept a lock token, or the test fails because any acquired lock currently permits removal.

- [ ] **Step 3: Bind artifacts to their authorizing publication identity**

Extend the token and artifact contracts:

```rust
pub(crate) struct CleanupArtifact {
    path: PathBuf,
    lock_identity: String,
    file_identity: FileIdentity,
    directory_identity: FileIdentity,
    file: Arc<Mutex<Option<File>>>,
    directory: Arc<File>,
}

impl PublicationLockToken<'_> {
    fn authorizes(&self, identity: &str) -> bool {
        self.allowed_identities.contains(identity)
    }
}
```

Include `lock_identity` in `CleanupArtifact::eq`. Change artifact creation to receive both the lock and the logical target:

```rust
fn create_cleanup_artifact(
    lock: &PublicationLockToken<'_>,
    lock_target: &Path,
    path: &Path,
) -> Result<CleanupArtifact, PublishError> {
    let lock_identity = publication_identity(lock_target)?;
    if !lock.authorizes(&lock_identity) {
        return Err(PublishError::new(PublishErrorKind::InvalidTarget {
            target: lock_target.to_path_buf(),
        }));
    }
    let absolute_path = absolute_lexical_path(path)?;
    let lexical_parent = absolute_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = fs::canonicalize(lexical_parent)
        .map_err(|source| PublishError::io(PublishPhase::Inspect, lexical_parent, source))?;
    let directory = open_directory_nofollow(&canonical_parent)
        .map_err(|source| PublishError::io(PublishPhase::Inspect, &canonical_parent, source))?;
    let directory_identity = file_identity(&directory)
        .map_err(|source| PublishError::io(PublishPhase::Inspect, &canonical_parent, source))?;
    create_cleanup_artifact_in_directory(
        absolute_path,
        Arc::new(directory),
        directory_identity,
        lock_identity,
    )
}
```

Pass `lock_identity` through `create_cleanup_artifact_in_directory`. For stage files, `lock_target` is the requested publication target. For registration recovery files, it is `registration.path` or `published.target`, never the `.unica-recovery-*` path.

- [ ] **Step 4: Require the matching lock for cleanup**

Change the public cleanup entry point:

```rust
pub(crate) fn cleanup_publication_artifact(
    lock: &PublicationLockToken<'_>,
    artifact: &CleanupArtifact,
) -> Result<(), CleanupWarning> {
    if !lock.authorizes(&artifact.lock_identity) {
        return Err(CleanupWarning {
            path: artifact.path.clone(),
            artifact: artifact.clone(),
            message: "publication lock identity is not held; artifact left untouched".into(),
        });
    }
    remove_bound_publication_artifact(artifact).map_err(|error| CleanupWarning {
        path: artifact.path.clone(),
        artifact: artifact.clone(),
        message: error.to_string(),
    })
}
```

Keep `verify_publication_artifact` lock-free because it performs no namespace mutation.

- [ ] **Step 5: Carry the token through stage cleanup and Drop**

Make `StageGuard` borrow its authorizing token:

```rust
struct StageGuard<'lock, 'scope> {
    lock: &'lock PublicationLockToken<'scope>,
    path: PathBuf,
    artifact: CleanupArtifact,
    armed: bool,
}

impl Drop for StageGuard<'_, '_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = cleanup_publication_artifact(self.lock, &self.artifact);
        }
    }
}
```

Update `PreparedCreate`, `PreparedReplace`, `create_stage`,
`initialize_new_exact_file`, `attach_stage_cleanup`,
`write_exact_new_file`, and `write_exact_new_file_in_directory` with matching
`'lock`/`'scope` parameters. Remove redundant lock references only if the
borrowed `StageGuard` already enforces the same lifetime.

- [ ] **Step 6: Require the token throughout transaction mutation helpers**

Thread `lock: &PublicationLockToken<'_>` through these functions and all their callers:

```rust
fn record_recovery_cleanup(lock: &PublicationLockToken<'_>, state: &mut PublishState, recovery: &mut PendingRecovery)
fn retry_warned_artifacts(lock: &PublicationLockToken<'_>, state: &mut PublishState) -> Vec<String>
fn finish_pending_recovery_cleanups(lock: &PublicationLockToken<'_>, state: &mut PublishState) -> Vec<String>
fn finish_retried_registration_recoveries(lock: &PublicationLockToken<'_>, registrations: &mut [PublishedRegistration]) -> Vec<String>
fn retry_and_finish_recovery_cleanups(lock: &PublicationLockToken<'_>, state: &mut PublishState) -> Vec<String>
fn rollback(lock: &PublicationLockToken<'_>, state: &mut PublishState) -> Vec<String>
fn rollback_registration(lock: &PublicationLockToken<'_>, published: &mut PublishedRegistration, errors: &mut Vec<String>, cleanup_warnings: &mut Vec<CleanupWarning>)
fn preserve_rollback_recovery_copy(lock: &PublicationLockToken<'_>, published: &mut PublishedRegistration, diagnostics: &mut Vec<String>, cleanup_warnings: &mut Vec<CleanupWarning>)
fn finalize_success(lock: &PublicationLockToken<'_>, state: &mut PublishState)
```

Likewise change `RecoveryCleanup::cleanup_file`,
`RecoveryCleanup::cleanup_directory`, and `PendingRecovery::cleanup` to require
the token. `commit_locked` passes its existing `lock`; Task 1's
`cleanup_after_failed_commit` passes the reacquired token.

Update direct retry tests to acquire the original target lock before calling
`retry_warned_artifacts`. A test must never manufacture `PublicationLockToken`.

- [ ] **Step 7: Correct Unix facade comments without weakening behavior**

In `crates/unica-coder/src/infrastructure/platform/filesystem.rs`, replace the
claim that the final child recheck itself makes the operation identity-atomic
with wording that states:

```rust
/// The retained parent prevents lexical-parent redirection. The native
/// publication layer serializes cooperating writers and rechecks the final
/// child identity before this descriptor-relative name mutation. POSIX does
/// not provide compare-inode-and-unlink/rename as one system call.
```

Do not move syscalls or platform branches out of the facade.

- [ ] **Step 8: Run focused tests and verify GREEN**

Run:

```bash
cargo test -p unica-coder infrastructure::native_operations::single_file_publisher::tests -- --test-threads=1
cargo test -p unica-coder infrastructure::native_operations::compile_transaction::tests -- --test-threads=1
cargo test -p unica-coder infrastructure::platform::filesystem::tests -- --test-threads=1
```

Expected: all focused suites pass; wrong-lock cleanup preserves the artifact;
matching-lock cleanup succeeds; all existing same-name decoy tests stay green.

- [ ] **Step 9: Commit Task 2**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs crates/unica-coder/src/infrastructure/platform/filesystem.rs
git commit -m "fix(publication): require lock proof for identity cleanup"
```

### Task 3: Integrate current main, verify, publish, and merge PR #396

**Files:**
- Verify: all files changed by `origin/main...HEAD`
- Update only if required by merge: files touched by current `origin/main`
- GitHub: `IngvarConsulting/unica#396`

**Interfaces:**
- Consumes: the two implementation commits, current `origin/main`, GitHub check runs, and review-thread state.
- Produces: a pushed PR head with green required checks, zero unresolved review threads, and a merged PR whose commit is reachable from `origin/main`.

- [ ] **Step 1: Refresh live GitHub and base state**

Run:

```bash
git fetch origin main +refs/pull/396/head:refs/remotes/origin/pr-396
gh pr view 396 --repo IngvarConsulting/unica --json state,isDraft,headRefOid,baseRefOid,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup
git rev-list --left-right --count origin/main...HEAD
```

Expected: PR remains open; local HEAD contains the live PR head plus the new commits. Stop if the remote head changed independently and reconcile before pushing.

- [ ] **Step 2: Merge current `origin/main` into the PR head without rewriting reviewed history**

Run:

```bash
git merge --no-edit origin/main
```

Expected: a clean merge, or conflicts limited to files changed by both branches. Resolve conflicts by preserving both the current-main behavior and the lock-bound cleanup contract; run the focused suites again after any resolution.

- [ ] **Step 3: Run the complete local verification gate**

Run each command and require exit code 0:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
python3.12 -m unittest discover -s tests/ci --durations 20
python3.12 -m unittest discover -s tests/dev --durations 20
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
python3.12 scripts/ci/check-rust-platform-boundary.py
git diff --check origin/main...HEAD
```

Expected: zero failures; skipped tests are reported but do not hide any failed command.

- [ ] **Step 4: Push to the existing fork head branch**

Run:

```bash
git push pr250fork HEAD:fix/issue-356-identity-safe-cleanup-v012
```

Expected: a fast-forward update of the existing PR #396 head. Do not create another PR.

- [ ] **Step 5: Wait for GitHub checks and review threads**

Poll the live PR until all required checks finish:

```bash
gh pr checks 396 --repo IngvarConsulting/unica --watch --interval 20
gh api graphql -f owner=IngvarConsulting -f name=unica -F number=396 -f query='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:100){nodes{id isResolved isOutdated comments(first:20){nodes{path line body url}}}}}}}'
gh pr view 396 --repo IngvarConsulting/unica --json state,headRefOid,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup
```

Expected: required checks succeed and unresolved review-thread count is zero. If CI or review reports a defect, return to RED→GREEN for that defect and push another commit to the same branch.

- [ ] **Step 6: Squash-merge and verify persisted state**

Run only after the previous step is green:

```bash
gh pr merge 396 --repo IngvarConsulting/unica --squash
git fetch origin main
gh pr view 396 --repo IngvarConsulting/unica --json state,mergedAt,mergeCommit,url
gh issue view 356 --repo IngvarConsulting/unica --json state,closedAt,url
pr396_merge_sha="$(gh pr view 396 --repo IngvarConsulting/unica --json mergeCommit --jq '.mergeCommit.oid')"
test -n "$pr396_merge_sha"
git merge-base --is-ancestor "$pr396_merge_sha" origin/main
```

Expected: PR state is `MERGED`, issue #356 is closed, and the squash merge commit is an ancestor of refreshed `origin/main`.

---

## Completion evidence

Record in the final handoff:

- RED failure messages for both new regression tests;
- focused and full local test counts;
- pushed head SHA;
- GitHub Actions run URL and final conclusions;
- review-thread count;
- merge commit SHA and issue #356 persisted state;
- any platform limitation that remains explicitly outside the cooperative-writer contract.
