# Issue #185 RLM Stale-Content Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recover an RLM index with mtime drift through one full-build fallback and report terminal failures accurately instead of returning endless `rlm index building`.

**Architecture:** Keep orchestration in `workspace_index.rs`: retain the exact stale status, make an update background job optionally carry a build recovery command, and run that command once under the existing lock after post-update `stale (content)`. Marker state remains the source of terminal failure diagnostics, while workspace-service serialization and adapters preserve the distinction between real `Building`, exact `Stale`, and `Failed`.

**Tech Stack:** Rust 2021, serde/serde_json, existing `WorkspaceIndexService`, existing platform-managed child process abstraction, Cargo unit tests.

## Global Constraints

- Keep the public MCP boundary as one server named `unica` with `unica.*` tools.
- Do not change the bundled `rlm-tools-bsl` 1.26.0 command-line contract.
- Do not disable or override `RLM_INDEX_SAMPLE_SIZE`.
- Only `stale (content)` after a successful update triggers a full build.
- Run at most one recovery build per update job.
- Return `rlm index building` only while an active Unica index lock exists.
- Preserve the original `stale (content)` cause in success diagnostics and terminal failure messages.
- A fresh `index info` result supersedes a failed marker; other non-ready results do not.
- Compare marker and requested source roots through normalized path identity.

---

## File Structure

- Modify `crates/unica-coder/src/infrastructure/workspace_index.rs`
  - Preserve exact RLM stale status.
  - Honor terminal failed markers.
  - Carry and execute the one-shot recovery build.
  - Record recovery diagnostics.
  - Add index-state and worker regression tests.
- Modify `crates/unica-coder/src/infrastructure/workspace_services.rs`
  - Preserve exact stale detail and failed messages across the workspace-service wire format.
  - Add serialization round-trip tests.
- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`
  - Stop mapping inactive stale state to `rlm index building`.
  - Add adapter warning tests.
- Modify `docs/superpowers/plans/2026-07-23-issue-185-rlm-stale-content-recovery.md`
  - Check off completed steps during execution.

No new production module is warranted: index parsing, marker policy, lock ownership,
and worker orchestration already form one cohesive boundary in `workspace_index.rs`.

---

### Task 1: Preserve Exact Stale State and Honor Failed Markers

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:29-36`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:158-297`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:777-844`
- Test: `crates/unica-coder/src/infrastructure/workspace_index.rs:1150-1510`

**Interfaces:**
- Produces: `IndexReadiness::Stale { status: String }`
- Produces: `IndexReadiness::stale_status(&self) -> Option<&str>`
- Produces: `failed_status_for_source(context: &WorkspaceContext, source_root: &Path) -> Option<String>`
- Consumes: existing `stored_path_matches(stored: Option<&str>, current: &Path) -> bool`

- [ ] **Step 1: Write failing parser tests for exact stale values**

Add beside the existing `cancellation_prefix_is_stable_for_cancelled_index_output` test:

```rust
#[test]
fn info_parser_preserves_exact_stale_status() {
    for status in [
        "stale (content)",
        "stale (age)",
        "stale (structure changed)",
    ] {
        let readiness = readiness_from_info(&IndexOutput::success(format!(
            "Index: /tmp/bsl_index.db\n  Status:   {status}\n"
        )));
        assert_eq!(
            readiness,
            IndexReadiness::Stale {
                status: status.to_string()
            }
        );
    }
}

#[test]
fn only_stale_content_is_recovery_eligible() {
    assert!(IndexReadiness::Stale {
        status: "stale (content)".to_string()
    }
    .is_stale_content());
    assert!(!IndexReadiness::Stale {
        status: "stale (age)".to_string()
    }
    .is_stale_content());
}
```

- [ ] **Step 2: Run the parser tests and verify they fail**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_index::tests::info_parser_preserves_exact_stale_status
cargo test -p unica-coder infrastructure::workspace_index::tests::only_stale_content_is_recovery_eligible
```

Expected: compilation fails because `IndexReadiness::Stale` has no `status`
field and `is_stale_content` does not exist.

- [ ] **Step 3: Implement exact stale parsing**

Replace the unit variant and add its helpers:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexReadiness {
    Ready { db_path: PathBuf },
    Missing,
    Stale { status: String },
    Building,
    Failed(String),
    Unavailable(String),
}

impl IndexReadiness {
    pub fn stale_status(&self) -> Option<&str> {
        match self {
            Self::Stale { status } => Some(status),
            _ => None,
        }
    }

    fn is_stale_content(&self) -> bool {
        self.stale_status() == Some("stale (content)")
    }
}
```

Change `readiness_from_info`:

```rust
Some(value) if value.starts_with("stale") => IndexReadiness::Stale {
    status: value.to_string(),
},
```

Mechanically update exhaustive matches in `workspace_index.rs`,
`workspace_services.rs`, and `internal_adapters.rs` from
`IndexReadiness::Stale` to `IndexReadiness::Stale { .. }`. Do not change their
behavior yet.

- [ ] **Step 4: Run the parser tests and verify they pass**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_index::tests::info_parser_preserves_exact_stale_status
cargo test -p unica-coder infrastructure::workspace_index::tests::only_stale_content_is_recovery_eligible
```

Expected: both tests pass.

- [ ] **Step 5: Write failing tests for failed-marker precedence**

Add these tests near `ready_info_writes_ready_status_and_does_not_start_background_job`:

```rust
#[test]
fn failed_marker_blocks_automatic_restart_for_same_source() {
    let context = test_context("failed-marker");
    fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
    write_status(
        &context,
        BslIndexStatus::failed(
            "update left stale (content); recovery build failed",
            Some(&context.workspace_root.join("src")),
        ),
    )
    .unwrap();
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   stale (content)\n",
        )]),
        ..Default::default()
    };
    let service = WorkspaceIndexService::with_runner(&runner);

    let report = service.start_for_workspace(&context, &Map::new(), false);

    assert!(runner.backgrounds.borrow().is_empty());
    assert_eq!(
        report.warnings,
        vec![
            "rlm index unavailable: update left stale (content); recovery build failed"
                .to_string()
        ]
    );
    cleanup(&context);
}

#[test]
fn ready_index_returns_matching_failed_marker_message() {
    let context = test_context("failed-readiness");
    fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
    write_status(
        &context,
        BslIndexStatus::failed(
            "update left stale (content); recovery build failed",
            Some(&context.workspace_root.join("src")),
        ),
    )
    .unwrap();
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   stale (content)\n",
        )]),
        ..Default::default()
    };

    let readiness =
        WorkspaceIndexService::with_runner(&runner).ready_index(&context, &Map::new());

    assert_eq!(
        readiness,
        IndexReadiness::Failed(
            "update left stale (content); recovery build failed".to_string()
        )
    );
    cleanup(&context);
}

#[test]
fn fresh_info_replaces_matching_failed_marker() {
    let context = test_context("failed-then-fresh");
    fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
    let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();
    write_status(
        &context,
        BslIndexStatus::failed(
            "old recovery failure",
            Some(&context.workspace_root.join("src")),
        ),
    )
    .unwrap();
    let runner = RecordingIndexRunner {
        outputs: RefCell::new(vec![IndexOutput::success(format!(
            "Index: {}\n  Status:   fresh\n",
            db_path.display()
        ))]),
        ..Default::default()
    };

    WorkspaceIndexService::with_runner(&runner)
        .start_for_workspace(&context, &Map::new(), false);

    assert_eq!(read_bsl_index_status(&context).unwrap().status, "ready");
    cleanup(&context);
}
```

- [ ] **Step 6: Run the marker tests and verify they fail**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_index::tests::failed_marker
cargo test -p unica-coder infrastructure::workspace_index::tests::ready_index_returns_matching_failed_marker_message
cargo test -p unica-coder infrastructure::workspace_index::tests::fresh_info_replaces_matching_failed_marker
```

Expected: the first two tests fail because startup/readiness ignore the failed
marker. The fresh-info test should already pass and protects the reset rule.

- [ ] **Step 7: Implement matching failed-marker lookup and precedence**

Add:

```rust
fn failed_status_for_source(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Option<String> {
    let status = read_bsl_index_status(context)?;
    if status.status != "failed"
        || !stored_path_matches(status.source_root.as_deref(), source_root)
    {
        return None;
    }
    status.message
}
```

In `start_for_workspace_cancellable`, keep the existing `Ready` branch first.
Before starting maintenance for any other readiness, return:

```rust
if let Some(message) = failed_status_for_source(context, &source_root) {
    return IndexStartReport {
        warnings: vec![format!("rlm index unavailable: {message}")],
    };
}
```

In `ready_index_cancellable`, keep the existing `Ready` write first. For every
other readiness, return a matching marker failure when present:

```rust
other => failed_status_for_source(context, &source_root)
    .map(IndexReadiness::Failed)
    .unwrap_or(other),
```

- [ ] **Step 8: Run the workspace-index tests**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_index::tests
```

Expected: all `workspace_index` tests pass.

- [ ] **Step 9: Commit exact-state and marker behavior**

```powershell
git add crates/unica-coder/src/infrastructure/workspace_index.rs crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "fix: preserve terminal RLM index state"
```

---

### Task 2: Run One Full-Build Recovery Under the Existing Lock

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:44-117`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:210-418`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs:620-725`
- Test: `crates/unica-coder/src/infrastructure/workspace_index.rs:1500-1760`

**Interfaces:**
- Consumes: `IndexReadiness::is_stale_content() -> bool` from Task 1
- Produces: `IndexBackgroundJob::recovery_build: Option<IndexCommand>`
- Produces: `BslIndexRunMetrics::recovery_reason: Option<String>`
- Produces: `run_background_job_with<F>(job: IndexBackgroundJob, run: F)` where `F: FnMut(&IndexCommand, &mut IndexLockLease) -> Result<IndexOutput, String>`

- [ ] **Step 1: Write failing job-construction tests**

Extend `stale_index_starts_background_update` and
`first_non_dry_run_starts_background_build_when_index_is_missing`:

```rust
let backgrounds = runner.backgrounds.borrow();
assert_eq!(backgrounds[0].primary.args[0..2], ["index", "update"]);
assert_eq!(
    backgrounds[0]
        .recovery_build
        .as_ref()
        .expect("update should carry a recovery build")
        .args[0..2],
    ["index", "build"]
);
```

For the missing-index build test add:

```rust
assert!(backgrounds[0].recovery_build.is_none());
```

- [ ] **Step 2: Run the construction tests and verify they fail**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_index::tests::stale_index_starts_background_update
cargo test -p unica-coder infrastructure::workspace_index::tests::first_non_dry_run_starts_background_build_when_index_is_missing
```

Expected: compilation fails because `recovery_build` does not exist.

- [ ] **Step 3: Carry the recovery build only on update jobs**

Add to `IndexBackgroundJob`:

```rust
pub recovery_build: Option<IndexCommand>,
```

Add a `recovery_build: Option<IndexCommand>` parameter to
`WorkspaceIndexService::start_background` and copy it into the job.

Pass `None` from the missing-index build branch. Pass
`Some(commands.build)` from the stale-index update branch:

```rust
IndexReadiness::Stale { .. } => self.start_background(
    context,
    "update",
    source_root,
    commands.update,
    commands.info,
    Some(commands.build),
    "rlm index building",
),
```

Update direct test construction of `IndexBackgroundJob` with
`recovery_build: None`.

- [ ] **Step 4: Run the construction tests and verify they pass**

Run the two commands from Step 2.

Expected: both tests pass.

- [ ] **Step 5: Write failing scripted-worker recovery tests**

First extract a testable runner boundary. Then add:

```rust
#[test]
fn update_falls_back_to_one_build_after_stale_content() {
    let context = test_context("stale-content-recovery");
    let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();
    let mut job = test_background_job(&context, "update");
    job.recovery_build = Some(job.primary.clone());
    let mut outputs = vec![
        IndexOutput::success("Updated in 0.1s"),
        IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   stale (content)\n",
        ),
        IndexOutput::success(
            "Index built in 1.2s\n  Index: v14\n  Modules: 24\n",
        ),
        IndexOutput::success(format!(
            "Index: {}\n  Status:   fresh\n",
            db_path.display()
        )),
    ]
    .into_iter();
    let mut commands = Vec::new();

    run_background_job_with(job, |command, _lease| {
        commands.push(command.args[0..2].to_vec());
        Ok(outputs.next().expect("scripted output"))
    });

    assert_eq!(
        commands,
        vec![
            vec!["index".to_string(), "update".to_string()],
            vec!["index".to_string(), "info".to_string()],
            vec!["index".to_string(), "build".to_string()],
            vec!["index".to_string(), "info".to_string()],
        ]
    );
    let status = read_bsl_index_status(&context).unwrap();
    assert_eq!(status.status, "ready");
    let metrics = status.last_run.unwrap();
    assert_eq!(metrics.action, "update->build");
    assert_eq!(
        metrics.recovery_reason.as_deref(),
        Some("stale (content) after update")
    );
    cleanup(&context);
}

#[test]
fn failed_recovery_preserves_stale_content_cause() {
    let context = test_context("stale-content-recovery-failed");
    let mut job = test_background_job(&context, "update");
    job.recovery_build = Some(job.primary.clone());
    let mut outputs = vec![
        IndexOutput::success("Updated in 0.1s"),
        IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   stale (content)\n",
        ),
        IndexOutput {
            status_success: false,
            status: "exit status: 1".to_string(),
            stdout: String::new(),
            stderr: "disk full".to_string(),
            timed_out: false,
            cancelled: false,
            duration_ms: 4,
        },
    ]
    .into_iter();

    run_background_job_with(job, |_command, _lease| {
        Ok(outputs.next().expect("scripted output"))
    });

    let status = read_bsl_index_status(&context).unwrap();
    assert_eq!(status.status, "failed");
    let message = status.message.unwrap();
    assert!(message.contains("stale (content) after update"));
    assert!(message.contains("disk full"));
    cleanup(&context);
}

#[test]
fn recovery_does_not_recurse_when_final_info_is_stale() {
    let context = test_context("stale-content-recovery-terminal");
    let mut job = test_background_job(&context, "update");
    job.recovery_build = Some(job.primary.clone());
    let mut outputs = vec![
        IndexOutput::success("Updated in 0.1s"),
        IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   stale (content)\n",
        ),
        IndexOutput::success("Index built in 1.2s"),
        IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   stale (content)\n",
        ),
    ]
    .into_iter();
    let mut calls = 0;

    run_background_job_with(job, |_command, _lease| {
        calls += 1;
        Ok(outputs.next().expect("scripted output"))
    });

    assert_eq!(calls, 4);
    let status = read_bsl_index_status(&context).unwrap();
    assert_eq!(status.status, "failed");
    assert!(status.message.unwrap().contains("still stale (content)"));
    cleanup(&context);
}
```

Add these helpers:

```rust
fn inert_index_command(context: &WorkspaceContext, verb: &str) -> IndexCommand {
    IndexCommand {
        program: PathBuf::from("unused-by-scripted-runner"),
        args: vec![
            "index".to_string(),
            verb.to_string(),
            context.workspace_root.join("src").display().to_string(),
        ],
        cwd: context.workspace_root.clone(),
        env: Vec::new(),
        timeout: Duration::from_secs(5),
        cancellation: CancellationToken::new(),
    }
}

fn test_background_job(context: &WorkspaceContext, action: &str) -> IndexBackgroundJob {
    let lock = lock_path(context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let lock_lease = acquire_index_lock(
        &lock,
        action,
        &context.workspace_root.join("src"),
    )
    .unwrap()
    .expect("test background job should acquire lock");
    IndexBackgroundJob {
        action: action.to_string(),
        source_root: context.workspace_root.join("src"),
        primary: inert_index_command(context, action),
        info: inert_index_command(context, "info"),
        recovery_build: None,
        status_path: status_path(context),
        lock_path: lock,
        lock_lease,
    }
}
```

- [ ] **Step 6: Run the scripted-worker tests and verify they fail**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_index::tests::update_falls_back_to_one_build_after_stale_content
cargo test -p unica-coder infrastructure::workspace_index::tests::failed_recovery_preserves_stale_content_cause
cargo test -p unica-coder infrastructure::workspace_index::tests::recovery_does_not_recurse_when_final_info_is_stale
```

Expected: compilation fails because the testable worker boundary and recovery
diagnostics do not exist.

- [ ] **Step 7: Add recovery diagnostics to last-run metrics**

Add:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub recovery_reason: Option<String>,
```

to `BslIndexRunMetrics`. Set it to `None` in `from_output`. Add:

```rust
fn recovered_from(
    mut self,
    action: &str,
    reason: &str,
    started_at: u64,
    finished_at: u64,
    total_duration_ms: u64,
) -> Self {
    self.action = action.to_string();
    self.recovery_reason = Some(reason.to_string());
    self.started_at = started_at;
    self.finished_at = finished_at;
    self.duration_ms = total_duration_ms;
    self
}
```

Update existing literal `BslIndexRunMetrics` values in tests with
`recovery_reason: None`.

- [ ] **Step 8: Extract the worker runner boundary and implement one-shot recovery**

Keep production behavior:

```rust
fn run_background_job(job: IndexBackgroundJob) {
    run_background_job_with(job, |command, lease| {
        run_index_command_with_heartbeat(command, Some(lease))
    });
}
```

Implement:

```rust
fn run_background_job_with<F>(job: IndexBackgroundJob, mut run: F)
where
    F: FnMut(&IndexCommand, &mut IndexLockLease) -> Result<IndexOutput, String>,
{
    let mut job = job;
    let started_at = now_secs();
    let primary = match run(&job.primary, &mut job.lock_lease) {
        Ok(output) => output,
        Err(error) => {
            let _ = write_status_path(
                &job.status_path,
                BslIndexStatus::failed(error.as_str(), Some(&job.source_root)),
            );
            return;
        }
    };
    let primary_finished_at = now_secs();
    let primary_metrics = BslIndexRunMetrics::from_output(
        &job.action,
        started_at,
        primary_finished_at,
        &primary,
    );
    if !primary.status_success || primary.cancelled || primary.timed_out {
        let message = command_failure_message(&job.action, &primary);
        let _ = write_status_path(
            &job.status_path,
            BslIndexStatus::failed(message.as_str(), Some(&job.source_root))
                .with_last_run(primary_metrics),
        );
        return;
    }

    let post_primary = match run(&job.info, &mut job.lock_lease) {
        Ok(info) => readiness_from_info(&info),
        Err(error) => {
            let _ = write_status_path(
                &job.status_path,
                BslIndexStatus::failed(error.as_str(), Some(&job.source_root))
                    .with_last_run(primary_metrics),
            );
            return;
        }
    };
    match post_primary {
        IndexReadiness::Ready { db_path } => {
            let _ = write_status_path(
                &job.status_path,
                BslIndexStatus::ready(&job.source_root, &db_path)
                    .with_last_run(primary_metrics),
            );
        }
        readiness if readiness.is_stale_content() && job.recovery_build.is_some() => {
            let reason = "stale (content) after update";
            let _ = write_status_path(
                &job.status_path,
                BslIndexStatus::building(
                    "build recovery after stale (content)",
                    Some(&job.source_root),
                ),
            );
            let recovery = run(
                job.recovery_build
                    .as_ref()
                    .expect("guarded recovery command"),
                &mut job.lock_lease,
            );
            let recovery = match recovery {
                Ok(output) => output,
                Err(error) => {
                    let message = format!(
                        "rlm index update finished but info is stale (content); recovery build failed: {error}"
                    );
                    let _ = write_status_path(
                        &job.status_path,
                        BslIndexStatus::failed(
                            message.as_str(),
                            Some(&job.source_root),
                        )
                        .with_last_run(primary_metrics),
                    );
                    return;
                }
            };
            if !recovery.status_success || recovery.cancelled || recovery.timed_out {
                let detail = command_failure_message("build", &recovery);
                let message = format!(
                    "rlm index update finished but info is stale (content); recovery build failed: {detail}"
                );
                let _ = write_status_path(
                    &job.status_path,
                    BslIndexStatus::failed(message.as_str(), Some(&job.source_root))
                        .with_last_run(primary_metrics),
                );
                return;
            }

            let finished_at = now_secs();
            let recovery_metrics = BslIndexRunMetrics::from_output(
                "build",
                primary_finished_at,
                finished_at,
                &recovery,
            )
            .recovered_from(
                "update->build",
                reason,
                started_at,
                finished_at,
                primary
                    .duration_ms
                    .saturating_add(recovery.duration_ms),
            );
            let final_readiness = match run(&job.info, &mut job.lock_lease) {
                Ok(info) => readiness_from_info(&info),
                Err(error) => {
                    let message = format!(
                        "rlm index update finished but info is stale (content); recovery build info failed: {error}"
                    );
                    let _ = write_status_path(
                        &job.status_path,
                        BslIndexStatus::failed(
                            message.as_str(),
                            Some(&job.source_root),
                        )
                        .with_last_run(recovery_metrics),
                    );
                    return;
                }
            };
            match final_readiness {
                IndexReadiness::Ready { db_path } => {
                    let _ = write_status_path(
                        &job.status_path,
                        BslIndexStatus::ready(&job.source_root, &db_path)
                            .with_last_run(recovery_metrics),
                    );
                }
                other => {
                    let message = format!(
                        "rlm index update finished but info is stale (content); recovery build finished but info is still {other:?}"
                    );
                    let _ = write_status_path(
                        &job.status_path,
                        BslIndexStatus::failed(
                            message.as_str(),
                            Some(&job.source_root),
                        )
                        .with_last_run(recovery_metrics),
                    );
                }
            }
        }
        other => {
            let message = format!(
                "rlm index {} finished but info is {other:?}",
                job.action
            );
            let _ = write_status_path(
                &job.status_path,
                BslIndexStatus::failed(message.as_str(), Some(&job.source_root))
                    .with_last_run(primary_metrics),
            );
        }
    }
}
```

Extract the existing primary failure formatting into:

```rust
fn command_failure_message(action: &str, output: &IndexOutput) -> String {
    if output.cancelled {
        cancelled_error(format!("rlm index {action} stopped"))
    } else if output.timed_out {
        format!("rlm index {action} timed out")
    } else {
        format!(
            "rlm index {action} failed: {} {}",
            output.status,
            output.stderr.trim()
        )
    }
}
```

This keeps cancellation and timeout wording identical to the existing worker.

- [ ] **Step 9: Run worker and lock tests**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_index::tests
```

Expected: all tests pass, including existing cancellation, timeout, lock
heartbeat, released-lock, and last-run metrics tests.

- [ ] **Step 10: Commit one-shot recovery**

```powershell
git add crates/unica-coder/src/infrastructure/workspace_index.rs
git commit -m "fix: rebuild RLM index after stale content"
```

---

### Task 3: Preserve Status Across Workspace Service and Adapter Output

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:1650-1720`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs:2650-2690`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs` test module
- Test: `crates/unica-coder/src/infrastructure/internal_adapters.rs` test module

**Interfaces:**
- Consumes: `IndexReadiness::Stale { status: String }` from Task 1
- Consumes: `IndexReadiness::Failed(String)` from Task 1 marker policy
- Produces: stable workspace-service round trip for exact stale status
- Produces: `readiness_warning(IndexReadiness) -> String` that maps only `Building` to `rlm index building`

- [ ] **Step 1: Write failing workspace-service round-trip tests**

Add in the `workspace_services.rs` test module:

```rust
#[test]
fn service_response_preserves_exact_stale_status() {
    let response = ServiceResponse::from_readiness(
        IndexReadiness::Stale {
            status: "stale (content)".to_string(),
        },
        Vec::new(),
    );

    assert_eq!(
        response.index_readiness(),
        IndexReadiness::Stale {
            status: "stale (content)".to_string(),
        }
    );
}

#[test]
fn service_response_preserves_failed_index_message() {
    let response = ServiceResponse::from_readiness(
        IndexReadiness::Failed(
            "update left stale (content); recovery build failed".to_string(),
        ),
        Vec::new(),
    );

    assert_eq!(
        response.index_readiness(),
        IndexReadiness::Failed(
            "update left stale (content); recovery build failed".to_string(),
        )
    );
}
```

- [ ] **Step 2: Run the round-trip tests and verify the stale test fails**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_services::tests::service_response_preserves_exact_stale_status
cargo test -p unica-coder infrastructure::workspace_services::tests::service_response_preserves_failed_index_message
```

Expected: the stale test fails because exact detail is discarded; failed
message preservation should pass.

- [ ] **Step 3: Preserve stale detail in the existing response fields**

Use `error` as the existing optional detail carrier without changing the wire
schema:

```rust
IndexReadiness::Stale { status } => Self {
    ok: true,
    index_status: Some("stale".to_string()),
    error: Some(status),
    warnings,
    ..Self::default()
},
```

Deserialize with a backward-compatible default:

```rust
Some("stale") => IndexReadiness::Stale {
    status: self
        .error
        .clone()
        .unwrap_or_else(|| "stale".to_string()),
},
```

- [ ] **Step 4: Run the round-trip tests and verify they pass**

Run the two commands from Step 2.

Expected: both pass.

- [ ] **Step 5: Write failing adapter warning tests**

Add near other internal-adapter helper tests:

```rust
#[test]
fn only_active_building_readiness_reports_index_building() {
    assert_eq!(
        readiness_warning(IndexReadiness::Building),
        "rlm index building"
    );
    assert_eq!(
        readiness_warning(IndexReadiness::Stale {
            status: "stale (content)".to_string(),
        }),
        "rlm index stale: stale (content)"
    );
}

#[test]
fn failed_readiness_reports_original_reason() {
    assert_eq!(
        readiness_warning(IndexReadiness::Failed(
            "update left stale (content); recovery build failed: disk full".to_string(),
        )),
        "rlm index unavailable: update left stale (content); recovery build failed: disk full"
    );
}
```

- [ ] **Step 6: Run the adapter tests and verify the stale test fails**

Run:

```powershell
cargo test -p unica-coder infrastructure::internal_adapters::tests::only_active_building_readiness_reports_index_building
cargo test -p unica-coder infrastructure::internal_adapters::tests::failed_readiness_reports_original_reason
```

Expected: stale is still rendered as `rlm index building`; failed reason test
passes.

- [ ] **Step 7: Restrict the building warning to real Building state**

Change `readiness_warning`:

```rust
match readiness {
    IndexReadiness::Ready { .. } => "rlm index ready".to_string(),
    IndexReadiness::Missing => "rlm index unavailable: index is missing".to_string(),
    IndexReadiness::Stale { status } => format!("rlm index stale: {status}"),
    IndexReadiness::Building => "rlm index building".to_string(),
    IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error)
        if error.starts_with(CANCELLED_PREFIX) =>
    {
        error
    }
    IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => {
        format!("rlm index unavailable: {error}")
    }
}
```

- [ ] **Step 8: Run focused service and adapter tests**

Run:

```powershell
cargo test -p unica-coder infrastructure::workspace_services::tests
cargo test -p unica-coder infrastructure::internal_adapters::tests
```

Expected: all tests pass.

- [ ] **Step 9: Commit status propagation**

```powershell
git add crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "fix: report terminal RLM index failures"
```

---

### Task 4: Complete Regression Verification

**Files:**
- Modify: `docs/superpowers/plans/2026-07-23-issue-185-rlm-stale-content-recovery.md`
- Verify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Verify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Verify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Interfaces:**
- Consumes: all production and test changes from Tasks 1-3
- Produces: verified issue #185 implementation with no public MCP contract change

- [ ] **Step 1: Run formatting**

Run:

```powershell
cargo fmt --all -- --check
```

Expected: PASS. If it reports formatting differences, run `cargo fmt --all`,
inspect the diff, and rerun the check.

- [ ] **Step 2: Run the complete crate test suite**

Run:

```powershell
cargo test -p unica-coder
```

Expected: all tests pass.

- [ ] **Step 3: Run package-contract checks**

Run:

```powershell
python -m pytest tests/ci/test_product_contracts.py
python scripts/ci/check-tool-contracts.py
```

Expected: both commands pass; the public server remains `unica` and no bundled
tool contract changes are detected.

- [ ] **Step 4: Inspect the final diff**

Run:

```powershell
git diff --check
git status --short
git diff HEAD~3 -- crates/unica-coder/src/infrastructure/workspace_index.rs crates/unica-coder/src/infrastructure/workspace_services.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
```

Expected:

- no whitespace errors;
- only issue #185 source, tests, and plan tracking changes;
- no change to `plugins/unica/third-party/tools.lock.json`;
- no `RLM_INDEX_SAMPLE_SIZE=0`;
- no public MCP tool or server rename.

- [ ] **Step 5: Commit plan completion if checkbox updates remain**

```powershell
git add docs/superpowers/plans/2026-07-23-issue-185-rlm-stale-content-recovery.md
git commit -m "docs: complete issue 185 implementation plan"
```
