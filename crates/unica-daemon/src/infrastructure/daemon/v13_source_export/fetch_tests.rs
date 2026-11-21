use super::*;
use std::fs;
use std::sync::Mutex;
use unica_process::runner::{ProcessStreamFailure, ProcessStreamOutput};

const NONCE: &str = "fb9824d8-6c77-4219-86b6-54f935a83c8e";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    /// Выгрузка готова, отказ от публикации принят.
    Exports,
    DumpInfoIsDirectory,
    OnlyDumpInfo,
    CancelsAfterReady,
    /// Раннер отказал до выгрузки.
    RefusesBeforeReady,
    RefusesAndLeavesNonDirectoryStage,
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
}

impl ProtocolRunner {
    fn new(script: Script) -> Self {
        Self {
            script,
            replies: Mutex::new(Vec::new()),
            args: Mutex::new(Vec::new()),
            preview: None,
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
        *self.args.lock().unwrap() = command.args.clone();
        let envelope = self.preview.clone().ok_or("unexpected plain call")?;
        Ok(ProcessOutput {
            status_success: true,
            status: "exit status: 0".into(),
            stdout: envelope.to_string(),
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
        if self.script == Script::RefusesAndLeavesNonDirectoryStage {
            fs::remove_dir(&stage).unwrap();
            fs::write(&stage, "unexpected file in place of the stage").unwrap();
        }
        let refused_before_ready = matches!(
            self.script,
            Script::RefusesBeforeReady | Script::RefusesAndLeavesNonDirectoryStage
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
            answer(frame(json!({
                "phase": "ready", "version": 1, "nonce": NONCE, "command": "pull",
                "requested_mode": "FULL", "mode": "FULL", "extension": extension,
                "mode_reason": null, "provider_dispatched": true, "source_set": set,
                "target_path": target, "stage_path": stage, "snapshot": "prepared",
                "snapshot_warning": null,
            })));
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
        Ok(ProcessStreamOutput {
            status_success: false,
            status: "exit status: 2".into(),
            stderr: String::new(),
            timed_out: false,
            cancelled: command.cancellation.is_cancelled(),
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
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
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
fn fetch_names_the_stage_when_refusal_cleanup_fails() {
    let workspace = workspace();
    let root = workspace.path();
    let before = tree(&root.join("main"));
    let runner = ProtocolRunner::new(Script::RefusesAndLeavesNonDirectoryStage);

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
    assert!(root.join(path).is_file(), "{warning:?}");
    assert_eq!(tree(&root.join("main")), before);
}

/// rule: INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY
#[test]
fn fetch_exports_the_named_extension_to_a_separate_snapshot() {
    let workspace = workspace();
    let root = workspace.path();
    let config_path = root.join(CONFIG_NAME);
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str("  - name: Sales\n    type: EXTENSION\n    path: Sales\n");
    fs::write(config_path, config).unwrap();
    let extension = root.join("Sales");
    fs::create_dir(&extension).unwrap();
    fs::write(extension.join("Configuration.xml"), "<local-extension/>").unwrap();
    let before_base = tree(&root.join("main"));
    let before_extension = tree(&extension);
    let runner = ProtocolRunner::new(Script::Exports);

    let missing_name = prepared_with_args(root, json!({"sourceSet": "Sales"}), false);
    let rejected = run(root, &missing_name, &runner);
    assert!(!rejected.ok, "{rejected:?}");
    assert!(runner.args.lock().unwrap().is_empty());
    assert!(snapshots(root).is_empty());

    let prepared = prepared_with_args(
        root,
        json!({"sourceSet": "Sales", "extension": "Sales"}),
        false,
    );
    let result = run(root, &prepared, &runner);

    assert!(result.ok, "{result:?}");
    assert_eq!(runner.decisions(), ["selected:continue", "ready:rejected"]);
    let args = runner.args.lock().unwrap().join(" ");
    assert!(
        args.contains("pull Sales --force --publication-stdio"),
        "{args}"
    );
    assert!(args.ends_with("--extension Sales"), "{args}");
    let data = result.data.unwrap();
    assert_eq!(data["extension"], "Sales");
    assert_eq!(data["sourceSet"], "Sales");
    assert_eq!(data["target"], "Sales");
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
    for name in ["a", "b", "c", "d", "e"] {
        fs::create_dir_all(root.join(".build/unica/fetch").join(name)).unwrap();
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
    let mut expected = vec!["c".to_string(), "d".to_string(), "e".to_string(), id];
    expected.sort();
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
    assert!(args.ends_with("pull main --force --dry-run"), "{args}");
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
    for name in [id, other] {
        let snapshot = root.join(".build/unica/fetch").join(name);
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("Configuration.xml"), "<base/>").unwrap();
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
