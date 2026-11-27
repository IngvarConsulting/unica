use super::*;
use std::fs;
use std::sync::Mutex;
use unica_process::runner::{ProcessStreamFailure, ProcessStreamOutput};

const NONCE: &str = "fb9824d8-6c77-4219-86b6-54f935a83c8e";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    /// Выгрузка готова, отказ от публикации принят.
    Exports,
    PreparesMerge,
    DumpInfoIsDirectory,
    OnlyDumpInfo,
    CancelsAfterReady,
    CancelsAfterTerminal,
    ChecksSnapshotLock,
    /// Раннер отказал до выгрузки.
    RefusesBeforeReady,
    RefusesAndBlocksStageCleanup,
    RefusesAndBlocksRetirement,
    LateOwnerPublicationFailure,
    /// Итог утверждает публикацию, хотя её отклонили.
    ClaimsPublication,
}

/// Раннер, который ведёт протокол внешней полной выгрузки и запоминает ответы
/// Unica.
struct ProtocolRunner {
    script: Script,
    replies: Mutex<Vec<Value>>,
    args: Mutex<Vec<String>>,
    preview: Option<Value>,
    help: Option<&'static str>,
    merges: Mutex<Vec<Value>>,
    merge_base_generation: Option<String>,
}

impl ProtocolRunner {
    fn new(script: Script) -> Self {
        Self {
            script,
            replies: Mutex::new(Vec::new()),
            args: Mutex::new(Vec::new()),
            preview: None,
            help: Some(if script == Script::PreparesMerge {
                "--snapshot-only --snapshot-platform-from-sources --prepare-merge-baseline --check-merge-baseline --accept-merge-baseline"
            } else {
                "--snapshot-only --snapshot-platform-from-sources"
            }),
            merges: Mutex::new(Vec::new()),
            merge_base_generation: None,
        }
    }

    fn decisions(&self) -> Vec<String> {
        self.replies
            .lock()
            .unwrap()
            .iter()
            .map(|reply| {
                format!(
                    "{}:{}",
                    reply["phase"].as_str().unwrap(),
                    reply["decision"].as_str().unwrap()
                )
            })
            .collect()
    }
}

fn frame(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

fn refused(message: &str) -> Value {
    json!({"command": "pull", "ok": false, "error": {"code": "validation", "message": message}})
}

impl ProcessRunner for ProtocolRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        let help = command.args.iter().any(|arg| arg == "--help");
        // Keep the arguments of an operation, separately from CLI capability discovery.
        if !help {
            *self.args.lock().unwrap() = command.args.clone();
        }
        let stdout = if help {
            self.help.ok_or("cannot inspect the runner")?.to_owned()
        } else if command
            .args
            .iter()
            .any(|arg| arg == "--accept-merge-baseline" || arg == "--check-merge-baseline")
        {
            self.merges
                .lock()
                .unwrap()
                .pop()
                .ok_or("unexpected merge call")?
                .to_string()
        } else {
            self.preview
                .clone()
                .ok_or("unexpected plain call")?
                .to_string()
        };
        Ok(ProcessOutput {
            status_success: true,
            status: "exit status: 0".into(),
            stdout,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        })
    }

    fn run_controlled_streaming_staged(
        &self,
        command: &ProcessCommand,
        _max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
    ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
        *self.args.lock().unwrap() = command.args.clone();
        let position = command
            .args
            .iter()
            .position(|arg| arg == "--publication-stdio")
            .unwrap();
        let stage = PathBuf::from(&command.args[position + 1]);
        let set = command.args[position - 2].clone();
        let target = command.cwd.join(&set);
        let extension = command
            .args
            .windows(2)
            .find(|pair| pair[0] == "--extension")
            .map(|pair| &pair[1]);
        let mut answer = |line: Vec<u8>| -> Option<Value> {
            match on_line(1, &line) {
                ProcessLineReply::Reply(bytes) => {
                    let reply: Value = serde_json::from_slice(&bytes).unwrap();
                    self.replies.lock().unwrap().push(reply.clone());
                    Some(reply)
                }
                _ => None,
            }
        };
        let selected = answer(frame(json!({
            "phase": "selected", "version": 1, "nonce": NONCE,
            "provider": {"selected": "designer"},
            "executor": {"kind": "local", "executable": "1cv8", "endpoint": null},
        })));
        let continued = selected.is_some_and(|reply| reply["decision"] == "continue");
        if self.script == Script::RefusesAndBlocksStageCleanup {
            make_directory_link(stage.parent().unwrap(), &stage.join("unexpected-link"));
        }
        if self.script == Script::RefusesAndBlocksRetirement {
            let id = stage.file_name().unwrap().to_str().unwrap();
            let records = stage.parent().unwrap().join(".fetch-cleanup");
            assert!(records.join(format!("{id}.preparing")).is_file());
            fs::write(stage.join("partial.txt"), "unfinished export").unwrap();
            fs::create_dir(records.join(format!("{id}.retiring"))).unwrap();
        }
        let refused_before_ready = matches!(
            self.script,
            Script::RefusesBeforeReady
                | Script::RefusesAndBlocksStageCleanup
                | Script::RefusesAndBlocksRetirement
        );
        let (publication, result) = if refused_before_ready || !continued {
            ("not_published", refused("the platform refused"))
        } else {
            // Раннер забирает новую опись платформы до `ready`, но может
            // сохранить старую опись из исходников.
            if self.script != Script::OnlyDumpInfo {
                let dump = if extension.is_some() {
                    "<extension-from-ib/>"
                } else {
                    "<base/>"
                };
                fs::write(stage.join("Configuration.xml"), dump).unwrap();
            }
            let dump_info = stage.join("ConfigDumpInfo.xml");
            if self.script == Script::DumpInfoIsDirectory {
                fs::create_dir(dump_info).unwrap();
            } else {
                fs::write(dump_info, "<versions/>").unwrap();
            }
            let mut ready = json!({
                "phase": "ready", "version": 1, "nonce": NONCE, "command": "pull",
                "requested_mode": "FULL", "mode": "FULL", "extension": extension,
                "mode_reason": null, "provider_dispatched": true, "source_set": set,
                "target_path": target, "stage_path": stage, "snapshot": "prepared",
                "snapshot_warning": null,
            });
            if self.script == Script::PreparesMerge {
                ready["merge_baseline"] = json!({
                    "id": NONCE,
                    "generation": "sha256:g1",
                    "provider": "designer",
                    "base_generation": self.merge_base_generation,
                });
            }
            answer(frame(ready));
            if self.script == Script::ChecksSnapshotLock {
                verify_ready_snapshot_cannot_be_dropped(&command.cwd, &stage);
            }
            let publication = if self.script == Script::ClaimsPublication {
                "published"
            } else {
                "not_published"
            };
            (
                publication,
                refused("full-pull publication was rejected; baseline unchanged"),
            )
        };
        drop(answer);
        if self.script == Script::CancelsAfterReady {
            command.cancellation.cancel();
        } else {
            on_line(
                1,
                &frame(json!({
                    "phase": "terminal", "version": 1, "nonce": NONCE,
                    "publication": publication, "memory": "unchanged", "result": result,
                })),
            );
        }
        if self.script == Script::LateOwnerPublicationFailure {
            unica_workspace::test_support::inject_post_rename_sync_failure_for_test();
        }
        let cancelled = command.cancellation.is_cancelled();
        if self.script == Script::CancelsAfterTerminal {
            command.cancellation.cancel();
        }
        Ok(ProcessStreamOutput {
            status_success: false,
            status: "exit status: 2".into(),
            stderr: String::new(),
            timed_out: false,
            cancelled,
            stopped_by_consumer: false,
            line_error: None,
        })
    }
}

fn workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join(CONFIG_NAME),
        "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n",
    )
    .unwrap();
    fs::create_dir_all(root.path().join("main")).unwrap();
    fs::write(root.path().join("main/Configuration.xml"), "<local/>").unwrap();
    fs::write(root.path().join("main/Local.bsl"), "uncommitted").unwrap();
    root
}

fn prepared(root: &Path, dry_run: bool) -> PreparedSourceExport {
    prepared_with_args(root, json!({"sourceSet": "main"}), dry_run)
}

fn prepared_with_args(root: &Path, args: Value, dry_run: bool) -> PreparedSourceExport {
    let request = InvocationRequest::new(
        ToolIdentity::Run,
        json!({"op": "fetch", "args": args, "dryRun": dry_run}),
        root.display().to_string(),
        7_000,
    )
    .unwrap();
    PreparedSourceExport::parse_fetch(&request).unwrap()
}

fn run(root: &Path, prepared: &PreparedSourceExport, runner: &ProtocolRunner) -> DomainResult {
    let tool = BundledTool {
        program: root.join("v8-runner"),
        warnings: Vec::new(),
        missing: None,
    };
    execute_with_tool(prepared, runner, CancellationToken::new(), &tool)
}

fn snapshots(root: &Path) -> Vec<String> {
    let mut names = fs::read_dir(root.join(".build/unica/fetch"))
        .map(|entries| {
            entries
                .map(Result::unwrap)
                .filter(|entry| {
                    entry.file_type().unwrap().is_dir() && entry.file_name() != ".fetch-cleanup"
                })
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn tree(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn fetch_returns_a_snapshot_and_leaves_the_sources_as_they_were() {
    let workspace = workspace();
    let root = workspace.path();
    let before = tree(&root.join("main"));
    let fingerprint = source_fingerprint(
        &normalize_path_identity(root).unwrap(),
        &root.join("main"),
        &CancellationToken::new(),
    )
    .unwrap();
    let runner = ProtocolRunner::new(Script::Exports);
    let prepared = prepared_with_args(root, json!({}), false);
    assert!(!prepared.holds_source_writes());

    let result = run(root, &prepared, &runner);

    assert!(result.ok, "{result:?}");
    assert_eq!(runner.decisions(), ["selected:continue", "ready:rejected"]);
    let args = runner.args.lock().unwrap().join(" ");
    assert!(
        args.contains("pull main --force --publication-stdio"),
        "{args}"
    );
    assert_eq!(tree(&root.join("main")), before);
    let data = result.data.clone().unwrap();
    let id = data["snapshotId"].as_str().unwrap();
    assert_eq!(data["snapshot"], format!(".build/unica/fetch/{id}"));
    assert_eq!(data["target"], "main");
    assert_eq!(data["performedMode"], "full");
    assert_eq!(data["sourceFingerprint"], fingerprint);
    assert_eq!(data["generation"], Value::Null);
    assert!(data["generationReason"].is_string());
    let snapshot = root.join(".build/unica/fetch").join(id);
    assert_eq!(
        tree(&snapshot),
        [("Configuration.xml".to_string(), b"<base/>".to_vec())]
    );
    assert_eq!(
        result.changed,
        [json!({"path": data["snapshot"], "kind": "created"})]
    );
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_refused_or_unconfirmed_fetch_leaves_no_snapshot() {
    for script in [Script::RefusesBeforeReady, Script::ClaimsPublication] {
        let workspace = workspace();
        let root = workspace.path();
        let before = tree(&root.join("main"));

        let result = run(root, &prepared(root, false), &ProtocolRunner::new(script));

        assert!(!result.ok, "{result:?}");
        assert_eq!(result.at.as_deref(), Some(FETCH));
        if script == Script::ClaimsPublication {
            assert!(result.summary.contains("did not confirm"), "{result:?}");
        }
        assert!(snapshots(root).is_empty(), "{:?}", snapshots(root));
        assert_eq!(tree(&root.join("main")), before);
    }
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn cancellation_after_ready_removes_the_snapshot_and_preserves_sources() {
    let workspace = workspace();
    let root = workspace.path();
    let before = tree(&root.join("main"));
    let runner = ProtocolRunner::new(Script::CancelsAfterReady);

    let result = run(root, &prepared(root, false), &runner);

    assert_eq!(runner.decisions(), ["selected:continue", "ready:rejected"]);
    assert!(!result.ok, "{result:?}");
    assert_eq!(result.at.as_deref(), Some(FETCH));
    assert!(result.summary.contains("fetch was cancelled"), "{result:?}");
    assert!(result.changed.is_empty(), "{result:?}");
    assert!(snapshots(root).is_empty(), "{:?}", snapshots(root));
    assert_eq!(tree(&root.join("main")), before);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn cancellation_after_the_final_runner_receipt_reclaims_the_unpublished_snapshot() {
    let workspace = workspace();
    let root = workspace.path();
    let runner = ProtocolRunner::new(Script::CancelsAfterTerminal);
    let result = run(root, &prepared(root, false), &runner);
    assert_eq!(result.diagnostics[0]["code"], "cancelled", "{result:?}");
    assert!(snapshots(root).is_empty());
    assert_eq!(
        fs::read(root.join("main/Local.bsl")).unwrap(),
        b"uncommitted"
    );
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn fetch_names_the_stage_when_refusal_cleanup_fails() {
    let workspace = workspace();
    let root = workspace.path();
    let before = tree(&root.join("main"));
    let runner = ProtocolRunner::new(Script::RefusesAndBlocksStageCleanup);

    let result = run(root, &prepared(root, false), &runner);

    assert!(!result.ok, "{result:?}");
    assert!(result.changed.is_empty(), "{result:?}");
    let warning = result
        .warnings
        .iter()
        .find(|warning| warning["code"] == SNAPSHOT_KEPT_WARNING)
        .expect("the refusal names the stage it could not clean");
    let path = warning["path"].as_str().unwrap();
    assert!(path.starts_with(".build/unica/fetch/"), "{warning:?}");
    assert!(root.join(path).exists(), "{warning:?}");
    assert_eq!(tree(&root.join("main")), before);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn fetch_exports_the_named_extension_to_a_separate_snapshot() {
    let name = "ДемонстрационноеРасширениеПримера1";
    let undeclared = "ДемонстрационноеРасширениеПримера2";
    assert_eq!(name.len(), 67);
    assert_eq!(undeclared.len(), name.len());
    let workspace = workspace();
    let root = workspace.path();
    let config_path = root.join(CONFIG_NAME);
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str(&format!(
        "  - name: {name}\n    type: EXTENSION\n    path: {name}\n"
    ));
    fs::write(&config_path, &config).unwrap();
    let extension = root.join(name);
    fs::create_dir(&extension).unwrap();
    fs::write(extension.join("Configuration.xml"), "<local-extension/>").unwrap();
    let before_base = tree(&root.join("main"));
    let before_extension = tree(&extension);
    let runner = ProtocolRunner::new(Script::Exports);

    let missing_extension = prepared_with_args(root, json!({"sourceSet": name}), false);
    let rejected = run(root, &missing_extension, &runner);
    assert!(!rejected.ok, "{rejected:?}");

    let unknown_set = prepared_with_args(
        root,
        json!({"sourceSet": undeclared, "extension": undeclared}),
        false,
    );
    let rejected = run(root, &unknown_set, &runner);
    assert!(!rejected.ok, "{rejected:?}");
    assert_eq!(rejected.diagnostics[0]["code"], "bad_value");
    assert!(
        rejected
            .summary
            .contains(&format!("`{undeclared}` is not declared in v8project.yaml")),
        "{rejected:?}"
    );
    assert!(runner.args.lock().unwrap().is_empty());
    assert!(runner.decisions().is_empty());
    assert!(snapshots(root).is_empty());
    assert_eq!(tree(&root.join("main")), before_base);
    assert_eq!(tree(&extension), before_extension);
    assert_eq!(fs::read_to_string(&config_path).unwrap(), config);

    let prepared = prepared_with_args(root, json!({"sourceSet": name, "extension": name}), false);
    let result = run(root, &prepared, &runner);

    assert!(result.ok, "{result:?}");
    assert_eq!(runner.decisions(), ["selected:continue", "ready:rejected"]);
    let args = runner.args.lock().unwrap().join(" ");
    assert!(
        args.contains(&format!("pull {name} --force --publication-stdio")),
        "{args}"
    );
    assert!(
        args.ends_with(&format!(
            "--extension {name} --snapshot-only --snapshot-platform-from-sources"
        )),
        "{args}"
    );
    let data = result.data.unwrap();
    assert_eq!(data["extension"], name);
    assert_eq!(data["sourceSet"], name);
    assert_eq!(data["target"], name);
    assert_eq!(
        tree(&root.join(data["snapshot"].as_str().unwrap())),
        [(
            "Configuration.xml".to_string(),
            b"<extension-from-ib/>".to_vec()
        )]
    );
    assert_eq!(tree(&root.join("main")), before_base);
    assert_eq!(tree(&extension), before_extension);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn fetch_rejects_a_stage_without_a_clean_export() {
    for (script, message) in [
        (
            Script::DumpInfoIsDirectory,
            "could not remove ConfigDumpInfo.xml",
        ),
        (Script::OnlyDumpInfo, "prepared an empty fetch snapshot"),
    ] {
        let workspace = workspace();
        let root = workspace.path();
        fs::write(root.join("main/ConfigDumpInfo.xml"), "<local-versions/>").unwrap();
        let before = tree(&root.join("main"));
        let runner = ProtocolRunner::new(script);

        let result = run(root, &prepared(root, false), &runner);

        assert_eq!(runner.decisions(), ["selected:continue", "ready:rejected"]);
        assert!(!result.ok, "{result:?}");
        assert!(result.summary.contains(message), "{result:?}");
        assert!(result.changed.is_empty(), "{result:?}");
        assert!(snapshots(root).is_empty(), "{:?}", snapshots(root));
        assert_eq!(tree(&root.join("main")), before);
    }
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_fetch_keeps_the_newest_snapshots_only() {
    let workspace = workspace();
    let root = workspace.path();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    let unrelated_id = "00000000000000000000000000000000";
    let unrelated = root.join(".build/unica/fetch").join(unrelated_id);
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("keep.txt"), "outside fetch ownership").unwrap();
    let original_ids: Vec<_> = ["a", "b", "c", "d", "e"]
        .map(|letter| letter.repeat(32))
        .into();
    for id in &original_ids {
        store_snapshot_fixture(&storage, id);
        std::thread::sleep(std::time::Duration::from_millis(30));
    }

    let result = run(
        root,
        &prepared(root, false),
        &ProtocolRunner::new(Script::Exports),
    );

    assert!(result.ok, "{result:?}");
    let id = result.data.unwrap()["snapshotId"]
        .as_str()
        .unwrap()
        .to_string();
    let mut expected = original_ids[2..].to_vec();
    expected.extend([unrelated_id.to_string(), id]);
    expected.sort();
    assert_eq!(snapshots(root), expected);
    assert_eq!(
        fs::read(unrelated.join("keep.txt")).unwrap(),
        b"outside fetch ownership"
    );

    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": unrelated_id}),
        false,
    ))
    .unwrap();
    let result = drop.execute(CancellationToken::new());
    assert!(result.ok, "{result:?}");
    assert_eq!(result.data.unwrap()["removed"], false);
    assert_eq!(snapshots(root), expected);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_preview_names_the_plan_and_creates_no_snapshot() {
    let workspace = workspace();
    let root = workspace.path();
    let mut runner = ProtocolRunner::new(Script::Exports);
    runner.preview = Some(json!({
        "command": "pull", "ok": true,
        "data": {
            "provider_dispatched": false, "requested_mode": "FULL", "mode": "FULL",
            "extension": null, "source_set": "main",
            "target_path": normalize_path_identity(root).unwrap().join("main"),
        },
    }));

    let result = run(root, &prepared(root, true), &runner);

    assert!(result.ok, "{result:?}");
    let args = runner.args.lock().unwrap().join(" ");
    assert!(
        args.ends_with(
            "pull main --force --dry-run --snapshot-only --snapshot-platform-from-sources"
        ),
        "{args}"
    );
    let data = result.data.unwrap();
    assert_eq!(data["op"], FETCH);
    assert_eq!(data["plan"]["target"], "main");
    assert_eq!(data["plan"]["replacesLocalSources"], false);
    assert!(!root.join(".build/unica/fetch").exists());
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn the_fingerprint_follows_the_content_and_skips_git_and_the_cache() {
    let workspace = tempfile::tempdir().unwrap();
    let root = normalize_path_identity(workspace.path()).unwrap();
    let none = CancellationToken::new();
    let empty = format!("sha256:{:x}", Sha256::digest(b""));
    assert_eq!(
        source_fingerprint(&root, &root.join("absent"), &none).unwrap(),
        empty
    );

    fs::create_dir_all(root.join("set/Sub")).unwrap();
    fs::write(root.join("set/Sub/b.bsl"), "b").unwrap();
    fs::write(root.join("set/a.xml"), "a").unwrap();
    let mut expected = Sha256::new();
    for (path, content) in [("Sub/b.bsl", "b"), ("a.xml", "a")] {
        expected.update(path.as_bytes());
        expected.update([0]);
        expected.update(format!("{:x}", Sha256::digest(content.as_bytes())).as_bytes());
        expected.update(b"\n");
    }
    let first = source_fingerprint(&root, &root.join("set"), &none).unwrap();
    assert_eq!(first, format!("sha256:{:x}", expected.finalize()));

    fs::create_dir_all(root.join("set/.git")).unwrap();
    fs::write(root.join("set/.git/HEAD"), "ref").unwrap();
    assert_eq!(
        source_fingerprint(&root, &root.join("set"), &none).unwrap(),
        first
    );
    // Набор — корень рабочего пространства: кеш Unica в отпечаток не входит.
    fs::create_dir_all(root.join(".build/unica")).unwrap();
    fs::write(root.join(".build/unica/x"), "cache").unwrap();
    let whole = source_fingerprint(&root, &root, &none).unwrap();
    fs::write(root.join(".build/unica/y"), "more cache").unwrap();
    assert_eq!(source_fingerprint(&root, &root, &none).unwrap(), whole);

    fs::write(root.join("set/a.xml"), "changed").unwrap();
    assert_ne!(
        source_fingerprint(&root, &root.join("set"), &none).unwrap(),
        first
    );
}

fn drop_request(root: &Path, args: Value, dry_run: bool) -> InvocationRequest {
    InvocationRequest::new(
        ToolIdentity::Run,
        json!({"op": "fetch.drop", "args": args, "dryRun": dry_run}),
        root.display().to_string(),
        7_000,
    )
    .unwrap()
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn fetch_drop_deletes_one_snapshot_and_treats_an_absent_one_as_deleted() {
    let workspace = workspace();
    let root = workspace.path();
    let id = "0123456789abcdef0123456789abcdef";
    let other = "fedcba9876543210fedcba9876543210";
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    for name in [id, other] {
        store_snapshot_fixture(&storage, name);
    }

    let preview = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        true,
    ))
    .unwrap();
    assert!(!preview.holds_source_writes());
    let planned = preview.execute(CancellationToken::new());
    assert!(planned.ok, "{planned:?}");
    assert_eq!(planned.data.unwrap()["plan"]["exists"], true);
    assert_eq!(snapshots(root).len(), 2);

    let apply = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();
    let dropped = apply.execute(CancellationToken::new());
    assert!(dropped.ok, "{dropped:?}");
    assert_eq!(dropped.data.as_ref().unwrap()["removed"], true);
    assert_eq!(snapshots(root), [other]);
    assert_eq!(tree(&root.join("main")).len(), 2);

    let again = apply.execute(CancellationToken::new());
    assert!(again.ok, "{again:?}");
    assert_eq!(again.data.unwrap()["removed"], false);
    assert_eq!(snapshots(root), [other]);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_rebound_stage_after_owner_publication_cannot_be_reported_ready() {
    let workspace = workspace();
    let root = workspace.path();
    let workspace_path = root.to_path_buf();
    let displaced = root.join("displaced-stage");
    let displaced_path = displaced.clone();
    let runner = ProtocolRunner::new(Script::Exports);
    storage::AFTER_OWNER_PUBLICATION.with(|hook| {
        hook.set(Some(Box::new(move || {
            let ids = snapshots(&workspace_path);
            let [id] = ids.as_slice() else {
                panic!("expected one stage, got {ids:?}");
            };
            let fetch = workspace_path.join(".build/unica/fetch");
            let path = fetch.join(id);
            assert!(fetch
                .join(".fetch-cleanup")
                .join(format!("{id}.owner"))
                .is_file());
            fs::rename(&path, displaced_path).unwrap();
            fs::create_dir(&path).unwrap();
            fs::write(path.join("foreign.txt"), "foreign content").unwrap();
        })));
    });

    let result = run(root, &prepared(root, false), &runner);

    assert!(!result.ok, "{result:?}");
    assert_eq!(result.diagnostics[0]["code"], "rollback_incomplete");
    assert!(result.data.is_none());
    assert!(result.changed.is_empty());
    let warning = &result.warnings[0];
    assert!(warning.get("path").is_none(), "{warning}");
    let id = warning["snapshotId"].as_str().unwrap();
    let fetch = root.join(".build/unica/fetch");
    assert_eq!(
        fs::read_to_string(fetch.join(id).join("foreign.txt")).unwrap(),
        "foreign content"
    );
    assert!(displaced.join("Configuration.xml").is_file());
    assert!(!fetch
        .join(".fetch-cleanup")
        .join(format!("{id}.owner"))
        .exists());
    let storage = SnapshotStorage::open(root, false).unwrap().unwrap();
    assert!(storage.owned_snapshots().unwrap().is_empty());
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn failure_after_removing_preparing_keeps_the_authority_to_drop_the_stage() {
    let workspace = workspace();
    let root = workspace.path();
    let workspace_path = root.to_path_buf();
    let displaced = root.join("displaced-stage");
    let displaced_path = displaced.clone();
    storage::AFTER_PREPARING_REMOVAL.with(|hook| {
        hook.set(Some(Box::new(move || {
            let ids = snapshots(&workspace_path);
            let [id] = ids.as_slice() else {
                panic!("expected one stage, got {ids:?}");
            };
            let fetch = workspace_path.join(".build/unica/fetch");
            let records = fetch.join(".fetch-cleanup");
            assert!(records.join(format!("{id}.owner")).is_file());
            assert!(!records.join(format!("{id}.preparing")).exists());
            fs::rename(fetch.join(id), displaced_path).unwrap();
        })));
    });

    let runner = ProtocolRunner::new(Script::Exports);
    let result = run(root, &prepared(root, false), &runner);

    assert!(!result.ok, "{result:?}");
    assert_eq!(result.diagnostics[0]["code"], "rollback_incomplete");
    let warning = &result.warnings[0];
    assert!(warning.get("path").is_none(), "{warning}");
    let id = warning["snapshotId"].as_str().unwrap();
    let fetch = root.join(".build/unica/fetch");
    let records = fetch.join(".fetch-cleanup");
    assert!(records.join(format!("{id}.preparing")).is_file());
    assert!(!records.join(format!("{id}.owner")).exists());
    fs::rename(displaced, fetch.join(id)).unwrap();

    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();
    let result = drop.execute(CancellationToken::new());

    assert!(result.ok, "{result:?}");
    assert_eq!(result.data.unwrap()["removed"], true);
    assert!(!fetch.join(id).exists());
    assert_eq!(fs::read_dir(records).unwrap().count(), 0);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_late_owner_publication_failure_rolls_back_the_exact_record_and_stage() {
    let workspace = workspace();
    let root = workspace.path();
    let runner = ProtocolRunner::new(Script::LateOwnerPublicationFailure);

    let result = run(root, &prepared(root, false), &runner);

    assert!(!result.ok, "{result:?}");
    assert!(result.summary.contains("sync"), "{result:?}");
    let storage = SnapshotStorage::open(root, false).unwrap().unwrap();
    assert!(storage.owned_snapshots().unwrap().is_empty());
    assert!(snapshots(root).is_empty());
    let entries = fs::read_dir(root.join(".build/unica/fetch")).unwrap();
    assert!(entries.map(Result::unwrap).all(|entry| {
        let entry = entry.file_name();
        entry == ".fetch-lock" || entry == ".fetch-cleanup"
    }));
    assert_eq!(
        fs::read_dir(root.join(".build/unica/fetch/.fetch-cleanup"))
            .unwrap()
            .count(),
        0
    );
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn drop_can_resume_an_unfinished_fetch_without_a_retiring_record() {
    let workspace = workspace();
    let root = workspace.path();
    let runner = ProtocolRunner::new(Script::RefusesAndBlocksRetirement);

    let result = run(root, &prepared(root, false), &runner);

    assert!(!result.ok, "{result:?}");
    assert_eq!(result.diagnostics[0]["code"], "rollback_incomplete");
    let warning = &result.warnings[0];
    assert_eq!(warning["code"], SNAPSHOT_KEPT_WARNING);
    let id = warning["snapshotId"].as_str().unwrap();
    let public = root.join(warning["path"].as_str().unwrap());
    assert_eq!(public.file_name().unwrap(), id);
    assert_eq!(
        fs::read(public.join("partial.txt")).unwrap(),
        b"unfinished export"
    );
    let records = public.parent().unwrap().join(".fetch-cleanup");
    let preparing = records.join(format!("{id}.preparing"));
    assert!(preparing.is_file());
    assert!(!records.join(format!("{id}.owner")).exists());
    fs::remove_dir(records.join(format!("{id}.retiring"))).unwrap();
    let storage = SnapshotStorage::open(root, false).unwrap().unwrap();
    assert!(storage.owned_snapshots().unwrap().is_empty());
    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();

    let displaced = root.join("unfinished-stage");
    fs::rename(&public, &displaced).unwrap();
    fs::create_dir(&public).unwrap();
    fs::write(public.join("foreign.txt"), "foreign content").unwrap();
    let refused = drop.execute(CancellationToken::new());
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "rollback_incomplete");
    assert_eq!(
        fs::read(public.join("foreign.txt")).unwrap(),
        b"foreign content"
    );
    assert!(preparing.is_file());
    let foreign = root.join("foreign-stage");
    fs::rename(&public, &foreign).unwrap();
    fs::rename(&displaced, &public).unwrap();

    let removed = drop.execute(CancellationToken::new());

    assert!(removed.ok, "{removed:?}");
    assert_eq!(removed.data.unwrap()["removed"], true);
    assert!(!public.exists());
    assert!(!preparing.exists());
    assert_eq!(
        fs::read(foreign.join("foreign.txt")).unwrap(),
        b"foreign content"
    );
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn drop_clears_absent_preparing_stage_records_without_reporting_a_tree_removal() {
    let workspace = workspace();
    let root = workspace.path();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let fetch = root.join(".build/unica/fetch");
    let records = fetch.join(".fetch-cleanup");
    let stage = storage.create_stage(id).unwrap();
    storage.prepare(id, &stage).unwrap();
    drop(stage);
    let preparing = records.join(format!("{id}.preparing"));
    let owner = records.join(format!("{id}.owner"));
    fs::copy(&preparing, &owner).unwrap();
    fs::write(fetch.join(id).join("partial.txt"), "unfinished export").unwrap();
    let displaced_stage = root.join("stage-outside-fetch");
    fs::rename(fetch.join(id), &displaced_stage).unwrap();
    let args = json!({"snapshotId": id});
    let preview =
        PreparedSourceExport::parse_fetch_drop(&drop_request(root, args.clone(), true)).unwrap();

    let plan = preview.execute(CancellationToken::new());

    assert!(plan.ok, "{plan:?}");
    assert_eq!(plan.data.unwrap()["plan"]["exists"], false);
    assert!(plan.summary.contains("unfinished snapshot record"));
    assert!(plan.summary.contains("under the snapshot lock"));
    assert!(!fetch.join(".fetch-lock").exists());
    assert!(preparing.is_file());
    assert!(owner.is_file());
    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(root, args, false)).unwrap();

    let quarantined = records.join(id);
    fs::create_dir(&quarantined).unwrap();
    fs::write(quarantined.join("foreign.txt"), "keep").unwrap();
    let refused = drop.execute(CancellationToken::new());
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "rollback_incomplete");
    assert!(preparing.is_file());
    assert!(owner.is_file());
    assert!(records.join(format!("{id}.retiring")).is_file());
    let planned_retry = preview.execute(CancellationToken::new());
    assert!(planned_retry.ok, "{planned_retry:?}");
    assert_eq!(planned_retry.data.unwrap()["plan"]["exists"], false);
    assert!(planned_retry.summary.contains("unfinished snapshot record"));
    let displaced = root.join("foreign-quarantine");
    fs::rename(quarantined, &displaced).unwrap();

    let result = drop.execute(CancellationToken::new());

    assert!(result.ok, "{result:?}");
    assert_eq!(result.data.unwrap()["removed"], false);
    assert!(result.changed.is_empty());
    assert!(result.summary.contains("no snapshot directory remained"));
    assert_eq!(fs::read_dir(records).unwrap().count(), 0);
    assert_eq!(fs::read(displaced.join("foreign.txt")).unwrap(), b"keep");
    assert_eq!(
        fs::read(displaced_stage.join("partial.txt")).unwrap(),
        b"unfinished export"
    );
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_changed_owner_record_prevents_retirement_without_modifying_the_snapshot() {
    let workspace = workspace();
    let root = workspace.path();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    store_snapshot_fixture(&storage, id);
    let snapshots = root.join(".build/unica/fetch");
    let records = snapshots.join(".fetch-cleanup");
    let owner_path = records.join(format!("{id}.owner"));
    let owner = fs::read(&owner_path).unwrap();
    let mut retiring = owner.clone();
    retiring.extend([0; 16]);
    fs::write(records.join(format!("{id}.retiring")), retiring).unwrap();
    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();

    let result = drop.execute(CancellationToken::new());

    assert!(!result.ok, "{result:?}");
    assert_eq!(result.diagnostics[0]["code"], "rollback_incomplete");
    assert_eq!(fs::read(owner_path).unwrap(), owner);
    assert_eq!(
        fs::read(snapshots.join(id).join("Configuration.xml")).unwrap(),
        b"<base/>"
    );
    assert!(storage.owned_snapshots().unwrap().is_empty());
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_record_in_the_public_directory_does_not_prove_snapshot_ownership() {
    let workspace = workspace();
    let root = workspace.path();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let stage = storage.create_stage(id).unwrap();
    let file = stage.path().join("foreign.txt");
    fs::write(&file, b"keep").unwrap();
    for name in [format!("{id}.owner"), format!("{id}.retiring")] {
        fs::write(stage.path().parent().unwrap().join(name), b"unrelated file").unwrap();
    }
    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();

    let result = drop.execute(CancellationToken::new());

    assert!(result.ok, "{result:?}");
    assert_eq!(result.data.unwrap()["removed"], false);
    assert_eq!(fs::read(file).unwrap(), b"keep");
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_quarantined_snapshot_can_be_cleaned_by_repeating_drop_with_its_id() {
    let workspace = workspace();
    let root = workspace.path();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    store_snapshot_fixture(&storage, id);
    let public = root.join(".build/unica/fetch").join(id);
    let external = tempfile::tempdir().unwrap();
    let external_file = external.path().join("outside.txt");
    fs::write(&external_file, b"keep").unwrap();
    make_directory_link(external.path(), &public.join("foreign"));
    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();

    let result = drop.execute(CancellationToken::new());

    assert!(!result.ok, "{result:?}");
    assert!(!public.exists());
    assert!(storage.owned_snapshots().unwrap().is_empty());
    let warning = &result.warnings[0];
    assert_eq!(warning["code"], SNAPSHOT_KEPT_WARNING);
    assert_eq!(warning["snapshotId"], id);
    let kept = root.join(warning["path"].as_str().unwrap());
    assert_eq!(
        fs::read(kept.join("Configuration.xml")).unwrap(),
        b"<base/>"
    );
    unica_workspace::test_support::remove_dir_symlink_for_test(kept.join("foreign")).unwrap();

    let retry = drop.execute(CancellationToken::new());

    assert!(retry.ok, "{retry:?}");
    assert_eq!(retry.data.unwrap()["removed"], true);
    assert!(!kept.exists());
    assert_eq!(fs::read(external_file).unwrap(), b"keep");
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn an_unreadable_source_set_does_not_prune_ready_snapshots() {
    let workspace = workspace();
    let root = workspace.path();
    fs::rename(root.join("main"), root.join("saved-source")).unwrap();
    fs::write(root.join("main"), "a file in place of the source directory").unwrap();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    let ids = ["1", "2", "3", "4"].map(|digit| digit.repeat(32));
    for id in &ids {
        store_snapshot_fixture(&storage, id);
    }
    let runner = ProtocolRunner::new(Script::Exports);

    let result = run(root, &prepared(root, false), &runner);

    assert!(!result.ok, "{result:?}");
    assert!(
        result
            .summary
            .contains("fetch could not read main of the source set"),
        "{result:?}"
    );
    assert!(runner.args.lock().unwrap().is_empty());
    assert_eq!(snapshots(root), ids);
    assert_eq!(storage.owned_snapshots().unwrap().len(), ids.len());
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_rejected_fetch_keeps_the_earlier_cleanup_warning() {
    let workspace = workspace();
    let root = workspace.path();
    let external = tempfile::tempdir().unwrap();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    for digit in ["1", "2", "3", "4"] {
        let id = digit.repeat(32);
        store_snapshot_fixture(&storage, &id);
        let path = root.join(".build/unica/fetch").join(id).join("foreign");
        make_directory_link(external.path(), &path);
    }

    let result = run(
        root,
        &prepared(root, false),
        &ProtocolRunner::new(Script::RefusesBeforeReady),
    );

    assert!(!result.ok, "{result:?}");
    let warning = result
        .warnings
        .iter()
        .find(|warning| {
            warning["code"] == SNAPSHOT_KEPT_WARNING
                && warning["message"]
                    .as_str()
                    .unwrap()
                    .contains("older snapshot")
        })
        .expect("the refusal preserves the earlier cleanup warning");
    assert!(root.join(warning["path"].as_str().unwrap()).exists());
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn retention_failure_before_retirement_refuses_to_reserve_a_fifth_snapshot() {
    let workspace = workspace();
    let root = workspace.path();
    let storage = SnapshotStorage::open(root, true).unwrap().unwrap();
    for digit in ["1", "2", "3", "4"] {
        store_snapshot_fixture(&storage, &digit.repeat(32));
    }
    let result = prune_snapshots(root, &storage, SNAPSHOTS_KEPT - 1, |_| {
        Err(CleanupFailure::Ownership(
            std::io::Error::other("cannot write retirement record"),
            None,
        ))
    })
    .unwrap_err();
    assert!(result.summary.contains("could not retire"), "{result:?}");
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.warnings[0]["code"], SNAPSHOT_KEPT_WARNING);
    assert_eq!(storage.owned_snapshots().unwrap().len(), SNAPSHOTS_KEPT);
    assert!(
        prune_snapshots(root, &storage, SNAPSHOTS_KEPT - 1, |snapshot| storage
            .remove(snapshot))
        .unwrap()
        .is_empty()
    );
    assert_eq!(storage.owned_snapshots().unwrap().len(), SNAPSHOTS_KEPT - 1);
}

fn make_directory_link(target: &Path, link: &Path) {
    unica_workspace::test_support::create_directory_link_or_junction_for_test(target, link)
        .unwrap();
}

fn store_snapshot_fixture(storage: &SnapshotStorage, id: &str) {
    let snapshot = storage.create_stage(id).unwrap();
    fs::write(snapshot.path().join("Configuration.xml"), "<base/>").unwrap();
    let preparing = storage.prepare(id, &snapshot).unwrap();
    storage.remember(id, &snapshot, &preparing).unwrap();
}

fn verify_ready_snapshot_cannot_be_dropped(root: &Path, stage: &Path) {
    let storage = SnapshotStorage::open(root, false).unwrap().unwrap();
    let lock = storage.open_lock().unwrap();
    let contended = lock
        .try_lock_exclusive()
        .expect_err("fetch still holds the snapshot lock");
    assert!(unica_workspace::state_fs::is_file_lock_contended(
        &contended
    ));

    let id = stage.file_name().unwrap().to_str().unwrap();
    let args = json!({"snapshotId": id});
    let preview =
        PreparedSourceExport::parse_fetch_drop(&drop_request(root, args.clone(), true)).unwrap();
    let planned = preview.execute(CancellationToken::new());
    assert!(planned.ok, "{planned:?}");
    assert_eq!(planned.data.unwrap()["plan"]["exists"], false);
    assert!(planned.summary.contains("unfinished snapshot record"));

    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(root, args, false)).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let stopped = drop.execute(cancellation);
    assert!(!stopped.ok, "{stopped:?}");
    assert!(
        stopped
            .summary
            .contains("cancelled while waiting for the snapshot directory"),
        "{stopped:?}"
    );
    assert_eq!(
        fs::read(stage.join("Configuration.xml")).unwrap(),
        b"<base/>"
    );
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn ready_stage_is_not_a_snapshot_and_keeps_its_lock_until_terminal() {
    let workspace = workspace();
    let root = workspace.path();
    let runner = ProtocolRunner::new(Script::ChecksSnapshotLock);

    let result = run(root, &prepared(root, false), &runner);

    assert!(result.ok, "{result:?}");
    assert_eq!(snapshots(root).len(), 1);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn linked_snapshot_ancestor_refuses_fetch_and_drop() {
    let workspace = workspace();
    let root = workspace.path();
    let external = tempfile::tempdir().unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let outside_snapshot = external.path().join("unica/fetch").join(id);
    fs::create_dir_all(&outside_snapshot).unwrap();
    fs::write(
        outside_snapshot.join("keep.txt"),
        "outside the working copy",
    )
    .unwrap();
    let link = root.join(".build");
    make_directory_link(external.path(), &link);

    let runner = ProtocolRunner::new(Script::Exports);
    let export = run(root, &prepared(root, false), &runner);
    assert!(!export.ok, "{export:?}");
    assert!(runner.args.lock().unwrap().is_empty());
    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();
    let result = drop.execute(CancellationToken::new());
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        fs::read(outside_snapshot.join("keep.txt")).unwrap(),
        b"outside the working copy"
    );
    assert_eq!(
        fs::read_dir(outside_snapshot.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn fetch_drop_accepts_only_a_snapshot_identifier() {
    let workspace = workspace();
    let root = workspace.path();
    for args in [
        json!({"snapshotId": "../main"}),
        json!({"snapshotId": "0123456789ABCDEF0123456789ABCDEF"}),
        json!({}),
        json!({"snapshotId": "0123456789abcdef0123456789abcdef", "sourceSet": "main"}),
    ] {
        let refusal =
            PreparedSourceExport::parse_fetch_drop(&drop_request(root, args.clone(), false))
                .unwrap_err();
        assert_eq!(refusal.at.as_deref(), Some(FETCH_DROP), "{args}");
    }
}

#[test]
fn fetch_accepts_only_its_closed_arguments() {
    let workspace = workspace();
    let request = InvocationRequest::new(
        ToolIdentity::Run,
        json!({"op": "fetch", "args": {"force": true}, "dryRun": false}),
        workspace.path().display().to_string(),
        7_000,
    )
    .unwrap();
    let refusal = PreparedSourceExport::parse_fetch(&request).unwrap_err();
    assert_eq!(refusal.at.as_deref(), Some(FETCH));
    assert!(
        refusal.summary.contains("sourceSet and extension"),
        "{refusal:?}"
    );
}

#[test]
fn fetch_accept_is_dispatched_as_a_source_export() {
    let workspace = workspace();
    let request = InvocationRequest::new(
        ToolIdentity::Run,
        json!({"op": "fetch.accept", "args": {"snapshotId": NONCE.replace('-', "")}, "dryRun": false}),
        workspace.path().display().to_string(),
        7_000,
    )
    .unwrap();
    assert!(matches!(
        crate::infrastructure::daemon::v13_source_export::prepare(&request),
        crate::infrastructure::daemon::v13_source_export::Preparation::Ready(_)
    ));
}

fn accept_request(root: &Path, id: &str, dry_run: bool) -> InvocationRequest {
    InvocationRequest::new(
        ToolIdentity::Run,
        json!({"op": "fetch.accept", "args": {"snapshotId": id}, "dryRun": dry_run}),
        root.display().to_string(),
        7_000,
    )
    .unwrap()
}

fn accept(root: &Path, id: &str, dry_run: bool, runner: &ProtocolRunner) -> DomainResult {
    use crate::infrastructure::daemon::v13_source_export::{prepare, Preparation};
    let Preparation::Ready(prepared) = prepare(&accept_request(root, id, dry_run)) else {
        panic!("accept request was not dispatched");
    };
    run(root, &prepared, runner)
}

fn prepared_merge(root: &Path, runner: &ProtocolRunner) -> Value {
    let result = run(root, &prepared(root, false), runner);
    assert!(result.ok, "{result:?}");
    let data = result.data.unwrap();
    assert_eq!(data["generation"], "sha256:g1");
    data
}

fn runner_merge_reply(baseline: &str, generation: &str) -> Value {
    json!({"command": "pull", "ok": true, "data": {
        "source_set": "main",
        "generation": generation,
        "provider": "designer",
        "baseline": baseline,
    }})
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn an_accepted_merge_keeps_the_raw_base_and_leaves_locally_merged_files() {
    let workspace = workspace();
    let root = workspace.path();
    let runner = ProtocolRunner::new(Script::PreparesMerge);
    let received = prepared_merge(root, &runner);
    assert!(received["baseSnapshot"].is_null());
    let id = received["snapshotId"].as_str().unwrap();
    fs::write(root.join("main/Configuration.xml"), "<merged/>").unwrap();
    let local = tree(&root.join("main"));

    runner
        .merges
        .lock()
        .unwrap()
        .push(runner_merge_reply("ready", "sha256:g1"));
    let checked = accept(root, id, true, &runner);
    assert!(checked.ok, "{checked:?}");
    assert_eq!(checked.data.unwrap()["ready"], true);

    runner
        .merges
        .lock()
        .unwrap()
        .push(runner_merge_reply("accepted", "sha256:g1"));
    let accepted = accept(root, id, false, &runner);
    assert!(accepted.ok, "{accepted:?}");
    assert_eq!(accepted.data.unwrap()["accepted"], true);
    assert_eq!(tree(&root.join("main")), local);

    let mut runner = ProtocolRunner::new(Script::PreparesMerge);
    runner.merge_base_generation = Some("sha256:g1".into());
    let next = prepared_merge(root, &runner);
    assert_eq!(next["baseSnapshot"], received["snapshot"]);
    let drop = PreparedSourceExport::parse_fetch_drop(&drop_request(
        root,
        json!({"snapshotId": id}),
        false,
    ))
    .unwrap();
    let dropped = drop_snapshot(&drop, id, CancellationToken::new());
    assert_eq!(dropped.data.unwrap()["removed"], false);
    assert_eq!(
        fs::read_to_string(
            root.join(received["snapshot"].as_str().unwrap())
                .join("Configuration.xml")
        )
        .unwrap(),
        "<base/>"
    );

    runner.merge_base_generation = Some("sha256:g2".into());
    let next = prepared_merge(root, &runner);
    assert!(next["baseSnapshot"].is_null());
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn fetch_accept_rejects_unowned_modified_or_unconfirmed_snapshots() {
    let workspace = workspace();
    let root = workspace.path();
    let runner = ProtocolRunner::new(Script::PreparesMerge);
    let received = prepared_merge(root, &runner);
    let id = received["snapshotId"].as_str().unwrap();

    let unknown = accept(root, &"a".repeat(32), false, &runner);
    assert!(!unknown.ok, "{unknown:?}");

    runner
        .merges
        .lock()
        .unwrap()
        .push(runner_merge_reply("accepted", "unexpected"));
    let unconfirmed = accept(root, id, false, &runner);
    assert!(!unconfirmed.ok, "{unconfirmed:?}");

    let raw = root
        .join(received["snapshot"].as_str().unwrap())
        .join("Configuration.xml");
    fs::write(raw, "<edited snapshot/>").unwrap();
    runner
        .merges
        .lock()
        .unwrap()
        .push(runner_merge_reply("accepted", "sha256:g1"));
    let modified = accept(root, id, false, &runner);
    assert!(!modified.ok, "{modified:?}");
    assert_eq!(runner.merges.lock().unwrap().len(), 1);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_runner_without_safe_snapshot_support_cannot_prepare_or_export_a_project() {
    for help in [
        None,
        Some("--prepare-merge-baseline --snapshot-platform-from-sources"),
        Some("--snapshot-only"),
    ] {
        let workspace = workspace();
        let root = workspace.path();
        let sources = tree(&root.join("main"));
        let recipe = fs::read(root.join(CONFIG_NAME)).unwrap();
        let mut runner = ProtocolRunner::new(Script::Exports);
        runner.help = help;
        for dry_run in [true, false] {
            let result = run(root, &prepared(root, dry_run), &runner);
            assert!(!result.ok, "{result:?}");
            assert_eq!(result.diagnostics[0]["code"], "unsupported_operation");
            assert!(runner.args.lock().unwrap().is_empty());
            assert!(runner.decisions().is_empty());
            assert_eq!(tree(&root.join("main")), sources);
            assert_eq!(fs::read(root.join(CONFIG_NAME)).unwrap(), recipe);
            assert!(!root.join(LOCAL_CONFIG_NAME).exists());
            assert!(!root.join(".build").exists());
        }
    }
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn a_snapshot_does_not_pin_platform_in_the_project_local_recipe() {
    let workspace = workspace();
    let root = workspace.path();
    let tool = BundledTool {
        program: root.join("v8-runner"),
        warnings: Vec::new(),
        missing: None,
    };
    let mut runner = ProtocolRunner::new(Script::PreparesMerge);
    runner.preview = Some(json!({
        "command": "pull", "ok": true,
        "data": {
            "provider_dispatched": false, "requested_mode": "FULL", "mode": "FULL",
            "extension": null, "source_set": "main",
            "target_path": normalize_path_identity(root).unwrap().join("main"),
        },
    }));
    let run = |prepared: &PreparedSourceExport| {
        execute_with_tool(prepared, &runner, CancellationToken::new(), &tool)
    };

    let preview = run(&prepared_with_args(root, json!({}), true));
    assert!(preview.ok, "{preview:?}");
    assert!(runner
        .args
        .lock()
        .unwrap()
        .join(" ")
        .ends_with("pull main --force --dry-run --snapshot-only --snapshot-platform-from-sources"));
    assert!(!root.join(".build/unica/fetch").exists());

    let fetched = run(&prepared(root, false));
    assert!(fetched.ok, "{fetched:?}");
    assert!(runner
        .args
        .lock()
        .unwrap()
        .iter()
        .any(|arg| arg == "--snapshot-platform-from-sources"));
    let data = fetched.data.unwrap();
    let id = data["snapshotId"].as_str().unwrap();
    let check = PreparedSourceExport::parse_fetch_accept(&accept_request(root, id, true)).unwrap();
    runner
        .merges
        .lock()
        .unwrap()
        .push(runner_merge_reply("ready", "sha256:g1"));
    let checked = run(&check);
    assert!(checked.ok, "{checked:?}");
    assert!(runner
        .args
        .lock()
        .unwrap()
        .iter()
        .any(|arg| arg == "--snapshot-platform-from-sources"));

    assert!(
        !root.join(LOCAL_CONFIG_NAME).exists(),
        "fetch wrote a local project recipe"
    );
}
