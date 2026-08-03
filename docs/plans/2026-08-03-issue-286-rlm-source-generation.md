# Issue #286 RLM Source Generation Binding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent every RLM-backed tool from reading an index that predates the current source generation, even when `rlm-bsl-index info` reports `fresh`.

**Architecture:** Reuse the workspace service's bounded source-generation fingerprint as an orchestrator-owned proof attached to `bsl_index_status.json`. Gate `IndexReadiness::Ready` on both RLM freshness and a matching persisted generation, and let the existing background update/build state machine publish the generation it actually attempted.

**Tech Stack:** Rust 2021, serde/serde_json, existing `WorkspaceIndexService`, existing workspace-service source generation, Cargo unit and integration tests, Python contract tests.

## Global Constraints

- Keep the public MCP boundary unchanged: one server named `unica` with the existing `unica.*` tools and schemas.
- Apply the fix once at the RLM readiness boundary for every remaining RLM-backed consumer; do not add tool-specific stale checks.
- Keep `unica.code.outline` on its ADR-0020 current-file path and out of RLM readiness.
- Do not read or depend on the private RLM SQLite schema.
- Do not change `plugins/unica/third-party/tools.lock.json`, the bundled RLM version, or its CLI contract.
- Preserve active-lock priority, background maintenance, cancellation, terminal/retryable failure classification, and one-shot `stale (content)` recovery.
- Preserve cache and service isolation by normalized `workspaceRoot + sourceRoot` identity.
- Treat a legacy ready marker without `source_generation` as unverified and update it once before allowing RLM-backed reads.
- Capture the generation before the maintenance command; never stamp a post-command generation that might include a concurrent edit the command did not index.
- The design record is `docs/design/2026-08-03-issue-286-rlm-source-generation-design.md`; this implementation does not introduce a new ADR or registry rule.

---

## File Structure

- Modify `crates/unica-coder/src/infrastructure/source_roots.rs`
  - Own the shared, bounded `source_generation(&Path) -> u64` helper used by both provider-session invalidation and RLM-index readiness.
  - Test source changes and `.build` exclusion at the source boundary.
- Modify `crates/unica-coder/src/infrastructure/workspace_services.rs`
  - Import the shared generation helper instead of defining a second copy.
  - Preserve the existing analyzer and RLM session invalidation behavior.
- Modify `crates/unica-coder/src/infrastructure/workspace_index.rs`
  - Persist the optional source generation in `BslIndexStatus`.
  - Bind externally fresh RLM readiness to a matching ready marker.
  - Carry the captured generation through `IndexBackgroundJob` and publish it only on successful ready status.
  - Add schema, readiness, race, worker, and compatibility regressions.
- Modify `docs/plans/2026-08-03-issue-286-rlm-source-generation.md`
  - Check off completed steps and record final verification results.

No public contract, package manifest, RLM adapter query, or tool-specific implementation file should change.

---

### Task 1: Establish the Shared Generation and Status Schema

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:2824-2865`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:57-74`
- Test: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Test: `crates/unica-coder/src/infrastructure/workspace_index.rs`

**Interfaces:**
- Produces: `pub(crate) fn source_generation(source_root: &Path) -> u64`
- Produces: `BslIndexStatus::source_generation: Option<u64>`
- Produces: `BslIndexStatus::with_source_generation(self, generation: u64) -> Self`
- Consumes: existing bounded traversal policy: depth at most 8, at most 20,000 sorted entries, `.build` excluded, and only directories plus `bsl|xml|yaml|yml` files.

- [ ] **Step 1: Write failing tests for the shared helper and backward-compatible status field**

Add to `source_roots.rs` tests:

```rust
#[test]
fn source_generation_ignores_build_cache_and_tracks_bsl_changes() {
    let context = fixture(&[("main", "CONFIGURATION", "src")]);
    let source_root = context.workspace_root.join("src");
    let module = source_root.join("CommonModules/SmokeModule.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Процедура Тест() Экспорт\nКонецПроцедуры\n").unwrap();
    let baseline = source_generation(&source_root);

    let generated = source_root.join(".build/bsl-graph.db");
    fs::create_dir_all(generated.parent().unwrap()).unwrap();
    fs::write(&generated, "generated cache").unwrap();
    assert_eq!(source_generation(&source_root), baseline);

    fs::write(
        &module,
        "Процедура Тест(НовыйПараметр = Неопределено) Экспорт\nКонецПроцедуры\n",
    )
    .unwrap();
    assert_ne!(source_generation(&source_root), baseline);
    cleanup(&context);
}
```

Add to `workspace_index.rs` tests:

```rust
#[test]
fn legacy_status_without_source_generation_remains_readable() {
    let status: BslIndexStatus = serde_json::from_str(
        r#"{
            "status":"ready",
            "source_root":"C:/workspace/src",
            "db_path":"C:/cache/bsl_index.db",
            "message":null,
            "updated_at":1
        }"#,
    )
    .unwrap();

    assert_eq!(status.source_generation, None);
}

#[test]
fn ready_status_can_carry_a_source_generation() {
    let status = BslIndexStatus::ready(Path::new("src"), Path::new("index.db"))
        .with_source_generation(42);

    assert_eq!(status.source_generation, Some(42));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```powershell
cargo test -p unica-coder --lib infrastructure::source_roots::tests::source_generation_ignores_build_cache_and_tracks_bsl_changes
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::legacy_status_without_source_generation_remains_readable
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::ready_status_can_carry_a_source_generation
```

Expected: compilation fails because `source_roots::source_generation`, `BslIndexStatus::source_generation`, and `with_source_generation` do not exist.

- [ ] **Step 3: Move the existing generation algorithm to `source_roots.rs`**

Add the required imports and move the existing functions without changing their limits or file filter:

```rust
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub(crate) fn source_generation(source_root: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_source_path(&mut hasher, source_root, 0);
    hasher.finish()
}

fn hash_source_path(hasher: &mut DefaultHasher, path: &Path, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(metadata) = path.metadata() else {
        0_u8.hash(hasher);
        return;
    };
    path.display().to_string().hash(hasher);
    if !metadata.is_dir() {
        metadata.len().hash(hasher);
        if let Ok(modified) = metadata.modified() {
            if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                duration.as_secs().hash(hasher);
                duration.subsec_nanos().hash(hasher);
            }
        }
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_none_or(|name| name != ".build")
                && (path.is_dir()
                    || matches!(
                        path.extension().and_then(|value| value.to_str()),
                        Some("bsl" | "xml" | "yaml" | "yml")
                    ))
        })
        .collect::<Vec<_>>();
    paths.sort();
    for child in paths.into_iter().take(20_000) {
        hash_source_path(hasher, &child, depth + 1);
    }
}
```

In `workspace_services.rs`, import `source_generation` from `source_roots` and remove only the local `source_generation` and `hash_source_path` definitions. Keep `DefaultHasher`, `Hash`, and `Hasher` imports if their other call sites still use them.

- [ ] **Step 4: Add the optional generation to `BslIndexStatus`**

Add a backward-compatible field and builder:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_generation: Option<u64>,
```

Initialize it to `None` in every `BslIndexStatus` constructor, then add:

```rust
fn with_source_generation(mut self, generation: u64) -> Self {
    self.source_generation = Some(generation);
    self
}
```

Do not add the field to `BslIndexRunMetrics`; generation describes the indexed source snapshot, not command telemetry.

- [ ] **Step 5: Run the focused tests and verify GREEN**

Run the three commands from Step 2 plus:

```powershell
cargo test -p unica-coder --lib infrastructure::workspace_services::tests::source_generation_ignores_generated_build_cache_but_tracks_bsl_source
```

Expected: all pass; the existing workspace-service regression still proves session invalidation uses the same generation semantics.

- [ ] **Step 6: Commit the shared primitive**

```powershell
git add crates/unica-coder/src/infrastructure/source_roots.rs crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/infrastructure/workspace_index.rs
git commit -m "refactor(cache): share source generation fingerprint"
```

---

### Task 2: Reject Externally Fresh Indexes Without Matching Generations

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:198-389`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:1416-1455`
- Test: `crates/unica-coder/src/infrastructure/workspace_index.rs`

**Interfaces:**
- Consumes: `source_generation(source_root: &Path) -> u64` from Task 1.
- Consumes: `BslIndexStatus::source_generation: Option<u64>` from Task 1.
- Produces: `const SOURCE_GENERATION_STALE_STATUS: &str = "stale (source generation)"`.
- Produces: `bind_readiness_to_source_generation(context, source_root, generation, readiness) -> IndexReadiness`.

- [ ] **Step 1: Add failing regressions for matching, legacy, and changed generations**

Add a test helper:

```rust
fn write_ready_status_for_current_source(
    context: &WorkspaceContext,
    source_root: &Path,
    db_path: &Path,
) {
    write_status(
        context,
        BslIndexStatus::ready(source_root, db_path)
            .with_source_generation(source_generation(source_root)),
    )
    .unwrap();
}
```

Add these tests beside the current ready/stale readiness tests:

```rust
#[test]
fn fresh_info_with_matching_generation_is_ready() {
    let context = test_context("fresh-matching-generation");
    let source_root = context.workspace_root.join("src");
    let module = source_root.join("CommonModules/SmokeModule.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Процедура Smoke()\nКонецПроцедуры\n").unwrap();
    let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();
    write_ready_status_for_current_source(&context, &source_root, &db_path);
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(format!(
            "Index: {}\n  Status:   fresh\n",
            db_path.display()
        ))]),
        ..Default::default()
    };

    let readiness = WorkspaceIndexService::with_runner(&runner)
        .ready_index(&context, &Map::new());

    assert_eq!(readiness, IndexReadiness::Ready { db_path });
    cleanup(&context);
}

#[test]
fn fresh_info_with_legacy_ready_marker_starts_update() {
    let context = test_context("fresh-legacy-generation");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(source_root.join("CommonModules")).unwrap();
    let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();
    write_status(&context, BslIndexStatus::ready(&source_root, &db_path)).unwrap();
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(format!(
            "Index: {}\n  Status:   fresh\n",
            db_path.display()
        ))]),
        ..Default::default()
    };

    let report = WorkspaceIndexService::with_runner(&runner)
        .start_for_workspace(&context, &Map::new(), false);

    assert_eq!(report.warnings, vec!["rlm index building".to_string()]);
    assert_eq!(runner.backgrounds.borrow().len(), 1);
    assert_eq!(runner.backgrounds.borrow()[0].action, "update");
    cleanup(&context);
}

#[test]
fn changed_bsl_rejects_fresh_info_after_service_recreation() {
    let context = test_context("fresh-changed-generation");
    let source_root = context.workspace_root.join("src");
    let module = source_root.join("CommonModules/SmokeModule.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Процедура Smoke(А, Б, В)\nКонецПроцедуры\n").unwrap();
    let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();
    write_ready_status_for_current_source(&context, &source_root, &db_path);
    fs::write(
        &module,
        "Процедура Smoke(А, Б, В, Г = Неопределено)\nКонецПроцедуры\n",
    )
    .unwrap();
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(format!(
            "Index: {}\n  Status:   fresh\n",
            db_path.display()
        ))]),
        ..Default::default()
    };

    let recreated_service = WorkspaceIndexService::with_runner(&runner);
    let readiness = recreated_service.ready_index(&context, &Map::new());

    assert_eq!(
        readiness,
        IndexReadiness::Stale {
            status: SOURCE_GENERATION_STALE_STATUS.to_string()
        }
    );
    cleanup(&context);
}
```

- [ ] **Step 2: Run the new tests and verify RED**

Run:

```powershell
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::fresh_info_with_matching_generation_is_ready
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::fresh_info_with_legacy_ready_marker_starts_update
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::changed_bsl_rejects_fresh_info_after_service_recreation
```

Expected: the first test passes only after test fixtures use the new field; the legacy and changed-source tests fail because current code trusts RLM `fresh` and rewrites the ready marker.

- [ ] **Step 3: Implement generation-bound readiness**

Import `source_generation` and add:

```rust
const SOURCE_GENERATION_STALE_STATUS: &str = "stale (source generation)";

fn bind_readiness_to_source_generation(
    context: &WorkspaceContext,
    source_root: &Path,
    generation: u64,
    readiness: IndexReadiness,
) -> IndexReadiness {
    let IndexReadiness::Ready { db_path } = readiness else {
        return readiness;
    };
    let matches = read_bsl_index_status(context).is_some_and(|status| {
        status.status == "ready"
            && status.source_generation == Some(generation)
            && stored_path_matches(status.source_root.as_deref(), source_root)
            && stored_path_matches(status.db_path.as_deref(), &db_path)
    });
    if matches {
        IndexReadiness::Ready { db_path }
    } else {
        IndexReadiness::Stale {
            status: SOURCE_GENERATION_STALE_STATUS.to_string(),
        }
    }
}
```

In both `start_for_workspace_cancellable` and `ready_index_cancellable`, keep the active-lock recheck immediately after `index info`, then compute `source_generation(&source_root)` and pass `readiness_from_info` through this helper before matching it.

Delete the calls that create or rewrite a ready marker merely because `index info` said fresh. Remove `ready_status_preserving_last_run`; only the background worker may publish a new ready proof after Task 3.

- [ ] **Step 4: Update existing fresh-index fixtures without weakening them**

For every existing workspace-index test whose intended state is Ready, write a matching ready marker with `write_ready_status_for_current_source` before returning scripted `fresh` info. Do not add a matching marker to tests for legacy migration, changed sources, missing indexes, stale indexes, active locks, or failures.

Update `ready_info_preserves_existing_last_run_metrics` to attach the current generation to its existing metric-bearing ready marker. Its assertion should prove the marker and `last_run` remain unchanged after a fresh readiness probe, rather than being rewritten by that probe.

- [ ] **Step 5: Run readiness tests and verify GREEN**

Run:

```powershell
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::fresh_info_with_matching_generation_is_ready
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::fresh_info_with_legacy_ready_marker_starts_update
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::changed_bsl_rejects_fresh_info_after_service_recreation
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::ready_info_preserves_existing_last_run_metrics
cargo test -p unica-coder --lib infrastructure::workspace_index::tests
```

Expected: all workspace-index tests pass; a matching marker is Ready, while legacy and changed generations never expose the DB path.

- [ ] **Step 6: Commit the readiness gate**

```powershell
git add crates/unica-coder/src/infrastructure/workspace_index.rs
git commit -m "fix(cache): reject stale RLM source generations"
```

---

### Task 3: Publish the Attempted Generation from Background Maintenance

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:117-136`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:407-493`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:798-950`
- Test: `crates/unica-coder/src/infrastructure/workspace_index.rs`

**Interfaces:**
- Consumes: `BslIndexStatus::with_source_generation(u64)` from Task 1.
- Produces: `IndexBackgroundJob::source_generation: u64`.
- Produces: successful normal and recovery ready markers bound to the captured generation.
- Preserves: failed/building/unavailable markers do not claim a ready generation.

- [ ] **Step 1: Add failing worker tests for success, concurrent change, and failure**

Extend `successful_background_job_records_last_run_metrics_in_status` to construct the job with `source_generation: 42` and assert:

```rust
assert_eq!(value["source_generation"], 42);
```

Add:

```rust
#[test]
fn background_job_records_the_generation_captured_before_a_source_change() {
    let context = test_context("captured-generation");
    let source_root = context.workspace_root.join("src");
    let module = source_root.join("CommonModules/SmokeModule.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Процедура Smoke()\nКонецПроцедуры\n").unwrap();
    let captured = source_generation(&source_root);
    let mut job = test_background_job(&context, "build");
    job.source_generation = captured;
    let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();

    run_background_job_with(job, |command, _lease| {
        if command.args.get(1).is_some_and(|arg| arg == "build") {
            fs::write(
                &module,
                "Процедура Smoke(НовыйПараметр)\nКонецПроцедуры\n",
            )
            .unwrap();
            Ok(IndexOutput::success("Index built"))
        } else {
            Ok(IndexOutput::success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            )))
        }
    });

    let status = read_bsl_index_status(&context).unwrap();
    assert_eq!(status.source_generation, Some(captured));
    assert_ne!(status.source_generation, Some(source_generation(&source_root)));
    cleanup(&context);
}
```

Extend `cancelled_background_job_records_failure_and_releases_lock` with:

```rust
assert_eq!(current_status.source_generation, None);
```

- [ ] **Step 2: Run the worker tests and verify RED**

Run:

```powershell
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::successful_background_job_records_last_run_metrics_in_status
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::background_job_records_the_generation_captured_before_a_source_change
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::cancelled_background_job_records_failure_and_releases_lock
```

Expected: compilation fails because `IndexBackgroundJob::source_generation` does not exist, or the success assertion fails because ready markers do not publish it.

- [ ] **Step 3: Capture the generation when maintenance starts**

Add to `IndexBackgroundJob`:

```rust
pub source_generation: u64,
```

In `start_background`, after acquiring the lock and before constructing the job, capture:

```rust
let source_generation = source_generation(&source_root);
```

Pass it into `IndexBackgroundJob`. Add an explicit `source_generation` to every direct test constructor and to `test_background_job`; use the source root's current generation unless the test needs a named sentinel such as `42`.

- [ ] **Step 4: Publish generation only on successful ready states**

Change both successful ready writes in `run_background_job_with`:

```rust
BslIndexStatus::ready(&job.source_root, &db_path)
    .with_source_generation(job.source_generation)
    .with_last_run(primary_metrics)
```

and after one-shot recovery:

```rust
BslIndexStatus::ready(&job.source_root, &db_path)
    .with_source_generation(job.source_generation)
    .with_last_run(recovery_metrics)
```

Do not attach the generation to `building`, `failed`, `terminal_failure`, or `unavailable` writes.

- [ ] **Step 5: Add and run the complete update-cycle regression**

Add a test that runs a scripted update job to fresh, then creates a new `WorkspaceIndexService` and returns scripted `fresh` info:

```rust
#[test]
fn successful_update_makes_the_unchanged_generation_ready_again() {
    let context = test_context("updated-generation-ready");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(source_root.join("CommonModules")).unwrap();
    let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();
    let mut job = test_background_job(&context, "update");
    job.source_generation = source_generation(&source_root);
    run_background_job_with(job, |command, _lease| {
        if command.args.get(1).is_some_and(|arg| arg == "info") {
            Ok(IndexOutput::success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            )))
        } else {
            Ok(IndexOutput::success("Index updated"))
        }
    });
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(format!(
            "Index: {}\n  Status:   fresh\n",
            db_path.display()
        ))]),
        ..Default::default()
    };

    let readiness = WorkspaceIndexService::with_runner(&runner)
        .ready_index(&context, &Map::new());

    assert_eq!(readiness, IndexReadiness::Ready { db_path });
    cleanup(&context);
}
```

Run:

```powershell
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::successful_background_job_records_last_run_metrics_in_status
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::background_job_records_the_generation_captured_before_a_source_change
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::cancelled_background_job_records_failure_and_releases_lock
cargo test -p unica-coder --lib infrastructure::workspace_index::tests::successful_update_makes_the_unchanged_generation_ready_again
cargo test -p unica-coder --lib infrastructure::workspace_index::tests
```

Expected: all pass. Successful normal and recovery jobs publish the captured generation; failure does not; unchanged sources become Ready after update.

- [ ] **Step 6: Commit worker publication**

```powershell
git add crates/unica-coder/src/infrastructure/workspace_index.rs
git commit -m "fix(cache): bind RLM builds to source generation"
```

---

### Task 4: Complete Regression and Contract Verification

**Files:**
- Modify: `docs/plans/2026-08-03-issue-286-rlm-source-generation.md`
- Verify: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Verify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Verify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Verify unchanged: public tool contracts, package manifests, `plugins/unica/third-party/tools.lock.json`

**Interfaces:**
- Consumes: all production and test changes from Tasks 1-3.
- Produces: a formatted, lint-clean, fully tested issue #286 implementation with no public contract drift.

- [x] **Step 1: Run formatting and focused Rust tests**

Run:

```powershell
cargo fmt --all -- --check
cargo test -p unica-coder --lib infrastructure::source_roots::tests
cargo test -p unica-coder --lib infrastructure::workspace_index::tests
cargo test -p unica-coder --lib infrastructure::workspace_services::tests
```

Expected: PASS with no warnings or failures. If formatting differs, run `cargo fmt --all`, inspect the diff, then repeat the check.

- [x] **Step 2: Run the complete crate suite and lints**

Run:

```powershell
cargo test -p unica-coder
cargo clippy -p unica-coder --all-targets -- -D warnings
```

Expected: all supported tests pass and clippy reports no warnings. If a Windows symlink test skips because privilege is unavailable, record the skip; do not weaken the test.

- [x] **Step 3: Run documentation and product-contract checks**

Run with the repository's Python 3.12 interpreter:

```powershell
python3.12 -m unittest tests.ci.test_design_documents
python3.12 -m unittest tests.ci.test_product_contracts
python3.12 scripts/ci/check-architecture-sync.py --base upstream/main --strict
```

Expected: PASS. Architecture sync accepts `Decision: none` because no public surface, layer boundary, cache owner, package contract, or registry rule changed.

- [x] **Step 4: Inspect the final diff and prohibited paths**

Run:

```powershell
git diff --check
git status --short
git diff upstream/main...HEAD -- crates/unica-coder/src/infrastructure/source_roots.rs crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/infrastructure/workspace_index.rs docs/design/2026-08-03-issue-286-rlm-source-generation-design.md docs/plans/2026-08-03-issue-286-rlm-source-generation.md
git diff --exit-code upstream/main...HEAD -- plugins/unica/third-party/tools.lock.json plugins/unica/.mcp.json plugins/unica/.codex-plugin/plugin.json plugins/unica/.claude-plugin/plugin.json crates/unica-coder/src/application/tool_contracts.rs
```

Expected:

- no whitespace errors or unrelated files;
- only the shared generation helper, index status/readiness/worker, tests, design, and plan changed;
- package metadata, public schemas, and bundled tool lock are unchanged;
- no direct RLM SQLite-schema read was added.

- [x] **Step 5: Record results and commit plan completion**

Check off completed plan steps and append exact test counts or any environment-specific skips to Task 4. Then run:

```powershell
git add docs/plans/2026-08-03-issue-286-rlm-source-generation.md
git commit -m "docs: complete issue 286 implementation plan"
```

Expected: clean working tree and an independently reviewable branch based on `upstream/main`.

#### Verification record (2026-08-03)

- `cargo fmt --all -- --check`: passed (exit 0; no formatting changes).
- Focused tests: `source_roots` was 11/12 passed and `workspace_index` was 57/58 passed; each lone failure attempted to create a Windows symlink and received error 1314 (privilege not held). `workspace_services` was initially 89/90 because `workspace_service_work_saturation_preserves_control_path` received connection reset 10054; repeating exactly that test passed 1/1 (1,774 filtered), and the aggregate crate suite did not reproduce it. All focused Rust invocations emitted the same two warnings from unchanged `infrastructure/runtime_jobs.rs`.
- Full crate suite: 1,748 passed, 25 failed, 2 ignored out of 1,775. The 25 failures are pre-existing Windows symlink/reparse environment failures (24 error 1314; one Windows anchor test error 5), not changes in this issue's diff. They are failures, not skips, and were not weakened.
- `cargo clippy -p unica-coder --all-targets -- -D warnings`: failed on the two warnings in unchanged `infrastructure/runtime_jobs.rs` (unused re-export and dead helper `assert_system_cancellation_reaps_process_tree`).
- With the bundled Python 3.12.13 interpreter: `test_design_documents` passed 8/8; `test_product_contracts` was 35/36 because an unchanged assertion expects `/missing/v8-runner` while Windows returns `\\missing\\v8-runner`; strict architecture sync passed and reported an unchanged public MCP surface.
- Diff inspection: `git diff --check` passed; the branch changes exactly the three approved infrastructure files plus the design and this plan. The prohibited package manifests, tool lock, and `tool_contracts.rs` have a zero diff. No SQLite client, SQL, or direct RLM SQLite-schema read was added.

The verification commands are complete and the scoped issue #286 regressions are covered, but this record deliberately does **not** claim a clean crate suite or lint-clean branch: the baseline/environment failures above remain visible and require separate remediation if a globally green Windows run is required.
