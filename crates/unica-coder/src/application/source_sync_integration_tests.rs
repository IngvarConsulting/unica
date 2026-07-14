use super::ports::ApplicationPorts;
use super::*;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::source_sync::SourceSyncRepository;
use crate::infrastructure::AdapterOutcome;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone)]
enum DumpBehavior {
    Exact(Vec<u8>),
    Divergent(Vec<u8>),
    Metadata(Vec<u8>),
    Fail,
}

struct RoundTripPorts {
    dump_behavior: RefCell<DumpBehavior>,
    runtime_args: RefCell<Vec<Map<String, Value>>>,
    switch_config_before_patch: RefCell<Option<String>>,
    switch_config_during_build: RefCell<Option<String>>,
}

impl RoundTripPorts {
    fn new() -> Self {
        Self {
            dump_behavior: RefCell::new(DumpBehavior::Fail),
            runtime_args: RefCell::new(Vec::new()),
            switch_config_before_patch: RefCell::new(None),
            switch_config_during_build: RefCell::new(None),
        }
    }

    fn set_dump_behavior(&self, behavior: DumpBehavior) {
        *self.dump_behavior.borrow_mut() = behavior;
    }

    fn switch_config_before_patch(&self, source_root: &str) {
        *self.switch_config_before_patch.borrow_mut() = Some(source_root.to_string());
    }

    fn switch_config_during_build(&self, source_root: &str) {
        *self.switch_config_during_build.borrow_mut() = Some(source_root.to_string());
    }
}

impl ApplicationPorts for Rc<RoundTripPorts> {
    fn discover_workspace(&self, cwd: PathBuf) -> Result<WorkspaceContext, String> {
        <super::ports::DefaultApplicationPorts as ApplicationPorts>::discover_workspace(
            &super::ports::DefaultApplicationPorts,
            cwd,
        )
    }

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        if !matches!(spec.handler, ToolHandler::RuntimeAdapter) {
            if spec.name == "unica.code.patch" {
                if let Some(source_root) = self.switch_config_before_patch.borrow_mut().take() {
                    write_project_config(
                        &context.workspace_root.join("v8project.yaml"),
                        &source_root,
                    );
                }
            }
            return <super::ports::DefaultApplicationPorts as ApplicationPorts>::invoke_handler(
                &super::ports::DefaultApplicationPorts,
                spec,
                args,
                context,
                dry_run,
            );
        }

        self.runtime_args.borrow_mut().push(args.clone());
        match args.get("operation").and_then(Value::as_str) {
            Some("build") => {
                if let Some(source_root) = self.switch_config_during_build.borrow_mut().take() {
                    write_project_config(
                        &context.workspace_root.join("v8project.yaml"),
                        &source_root,
                    );
                }
                let mut outcome = AdapterOutcome::ok("runner loaded changed source");
                outcome
                    .changes
                    .push("internal v8-runner runtime adapter executed".to_string());
                outcome.stdout = Some(
                    json!({
                        "ok": true,
                        "command": "build",
                        "data": {
                            "ok": true,
                            "steps": [{
                                "source_set": "main",
                                "mode": {"partial": {"file_count": 1}},
                                "ok": true,
                                "message": "loaded changed source"
                            }]
                        }
                    })
                    .to_string(),
                );
                Ok(outcome)
            }
            Some("dump") => match self.dump_behavior.borrow().clone() {
                DumpBehavior::Fail => {
                    let mut outcome = AdapterOutcome::ok("runner partial dump failed");
                    outcome.ok = false;
                    outcome.errors.push("simulated runner failure".to_string());
                    Ok(outcome)
                }
                DumpBehavior::Exact(bytes) | DumpBehavior::Divergent(bytes) => {
                    let shadow_root = shadow_source_root(args)?;
                    write_shadow_output(
                        &shadow_root,
                        "CommonModules/SampleService.xml",
                        &std::fs::read(
                            context
                                .workspace_root
                                .join("src/CommonModules/SampleService.xml"),
                        )
                        .map_err(|error| {
                            format!("failed to read fake owner descriptor: {error}")
                        })?,
                    )?;
                    write_shadow_output(
                        &shadow_root,
                        "CommonModules/SampleService/Ext/Module.bsl",
                        &bytes,
                    )?;
                    Ok(successful_dump_outcome())
                }
                DumpBehavior::Metadata(bytes) => {
                    let shadow_root = shadow_source_root(args)?;
                    write_shadow_output(&shadow_root, "Catalogs/Items.xml", &bytes)?;
                    Ok(successful_dump_outcome())
                }
            },
            operation => Err(format!("unexpected runtime operation {operation:?}")),
        }
    }

    fn cache_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        _cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        Ok(CacheReport {
            mode: if events.is_empty() {
                "read".to_string()
            } else if dry_run {
                "dry-run".to_string()
            } else {
                "applied".to_string()
            },
            root: context.cache_root.display().to_string(),
            workspace_epoch: context.workspace_epoch,
            events: events
                .iter()
                .map(|event| event.name().to_string())
                .collect(),
            invalidated: Vec::new(),
            refreshed: Vec::new(),
            lazy_rebuilt: Vec::new(),
            stale: Vec::new(),
            fresh: Vec::new(),
        })
    }

    fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
}

#[derive(Default)]
struct PostCommitFailurePorts {
    cache_events: RefCell<Vec<DomainEvent>>,
    notifications: RefCell<Vec<DomainEvent>>,
}

impl ApplicationPorts for Rc<PostCommitFailurePorts> {
    fn discover_workspace(&self, cwd: PathBuf) -> Result<WorkspaceContext, String> {
        <super::ports::DefaultApplicationPorts as ApplicationPorts>::discover_workspace(
            &super::ports::DefaultApplicationPorts,
            cwd,
        )
    }

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        _args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        assert_eq!(spec.name, "unica.cf.edit");
        assert!(!dry_run);

        let target = context.workspace_root.join("src/Configuration.xml");
        let mut bytes = std::fs::read(&target)
            .map_err(|error| format!("failed to read post-commit test target: {error}"))?;
        bytes.extend_from_slice(b"\n<!-- committed-before-finalization -->\n");
        std::fs::write(&target, bytes)
            .map_err(|error| format!("failed to commit post-commit test target: {error}"))?;

        let mut outcome = AdapterOutcome::ok("source changed but finalization failed");
        outcome.ok = false;
        outcome
            .changes
            .push("updated src/Configuration.xml before finalization failure".to_string());
        outcome
            .errors
            .push("injected post-commit finalization failure".to_string());
        Ok(outcome)
    }

    fn cache_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        _cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        self.cache_events.borrow_mut().extend_from_slice(events);
        Ok(CacheReport {
            mode: if events.is_empty() {
                "read".to_string()
            } else if dry_run {
                "dry-run".to_string()
            } else {
                "applied".to_string()
            },
            root: context.cache_root.display().to_string(),
            workspace_epoch: context.workspace_epoch,
            events: events
                .iter()
                .map(|event| event.name().to_string())
                .collect(),
            invalidated: Vec::new(),
            refreshed: Vec::new(),
            lazy_rebuilt: Vec::new(),
            stale: Vec::new(),
            fresh: Vec::new(),
        })
    }

    fn notify_invalidation(&self, _context: &WorkspaceContext, events: &[DomainEvent]) {
        self.notifications.borrow_mut().extend_from_slice(events);
    }
}

fn successful_dump_outcome() -> AdapterOutcome {
    let mut outcome = AdapterOutcome::ok("runner produced partial shadow dump");
    outcome
        .changes
        .push("internal v8-runner runtime adapter executed".to_string());
    outcome
}

fn write_shadow_output(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), String> {
    let output = root.join(relative);
    std::fs::create_dir_all(
        output
            .parent()
            .ok_or_else(|| "shadow output has no parent".to_string())?,
    )
    .map_err(|error| format!("failed to create fake shadow output: {error}"))?;
    std::fs::write(&output, bytes)
        .map_err(|error| format!("failed to write fake shadow output: {error}"))
}

#[test]
fn code_patch_build_and_shadow_dump_preserve_bytes_and_force_is_wrapper_only() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));

    let patched = app
        .call_tool("unica.code.patch", &code_patch_args(&workspace))
        .unwrap();
    assert!(patched.ok, "{}: {:?}", patched.summary, patched.errors);
    let patched_bytes = std::fs::read(&module_path).unwrap();

    let built = app
        .call_tool("unica.runtime.execute", &build_args(&workspace))
        .unwrap();
    assert!(built.ok, "{}: {:?}", built.summary, built.errors);
    assert_eq!(
        built.details.as_ref().unwrap()["requested"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        built.details.as_ref().unwrap()["processed"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .all(|record| !record.is_dirty()));

    ports.set_dump_behavior(DumpBehavior::Exact(patched_bytes.clone()));
    let exact = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, false))
        .unwrap();
    assert!(exact.ok, "{}: {:?}", exact.summary, exact.errors);
    assert_eq!(std::fs::read(&module_path).unwrap(), patched_bytes);
    assert_eq!(
        exact.details.as_ref().unwrap()["processed"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let divergent_bytes = b"\xef\xbb\xbfProcedure FromInfobase()\r\nEndProcedure\r\n".to_vec();
    ports.set_dump_behavior(DumpBehavior::Divergent(divergent_bytes.clone()));
    let conflict = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, false))
        .unwrap();
    assert!(!conflict.ok);
    assert_eq!(
        conflict.details.as_ref().unwrap()["conflicted"][0]["reason"],
        "infobaseDiverged"
    );
    assert!(conflict.diagnostics.is_none());
    assert_eq!(std::fs::read(&module_path).unwrap(), patched_bytes);

    let forced = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, true))
        .unwrap();
    assert!(forced.ok, "{}: {:?}", forced.summary, forced.errors);
    assert_eq!(std::fs::read(&module_path).unwrap(), divergent_bytes);
    assert_eq!(
        forced.details.as_ref().unwrap()["processed"][0]["forced"],
        true
    );
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .all(|record| !record.is_dirty()));

    let dump_calls = ports
        .runtime_args
        .borrow()
        .iter()
        .filter(|args| args.get("operation").and_then(Value::as_str) == Some("dump"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(dump_calls.len(), 3);
    for args in dump_calls {
        assert!(!args.contains_key("force"));
        assert!(args
            .get("config")
            .and_then(Value::as_str)
            .is_some_and(|path| {
                Path::new(path).file_name().and_then(|name| name.to_str()) == Some("v8project.yaml")
                    && path.contains(".build/unica/source-sync/")
                    && path.contains("/transactions/shadow-dump-")
            }));
        assert_eq!(args["objects"], json!(["CommonModule.SampleService"]));
    }
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_shadow_runner_keeps_working_source_and_clean_state() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    assert!(
        app.call_tool("unica.code.patch", &code_patch_args(&workspace))
            .unwrap()
            .ok
    );
    assert!(
        app.call_tool("unica.runtime.execute", &build_args(&workspace))
            .unwrap()
            .ok
    );
    let before = std::fs::read(&module_path).unwrap();

    ports.set_dump_behavior(DumpBehavior::Fail);
    let failed = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, false))
        .unwrap();

    assert!(!failed.ok);
    assert!(failed.diagnostics.is_none());
    assert_eq!(
        failed.details.as_ref().unwrap()["skipped"][0]["reason"],
        "runnerFailed"
    );
    assert_eq!(std::fs::read(&module_path).unwrap(), before);
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .all(|record| !record.is_dirty()));
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn meta_edit_build_and_partial_dump_survive_application_restart() {
    let (root, workspace, _module_path) = round_trip_workspace();
    let catalog_path = workspace.join("src/Catalogs/Items.xml");
    let ports = Rc::new(RoundTripPorts::new());

    let edited = UnicaApplication::with_ports(Box::new(ports.clone()))
        .call_tool("unica.meta.edit", &meta_edit_args(&workspace))
        .unwrap();
    assert!(edited.ok, "{}: {:?}", edited.summary, edited.errors);
    assert_eq!(
        edited.details.as_ref().unwrap()["affectedTargets"][0]["kind"],
        "metadataOwner"
    );
    let edited_bytes = std::fs::read(&catalog_path).unwrap();
    assert!(String::from_utf8_lossy(&edited_bytes).contains("<CodeLength>10</CodeLength>"));
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .any(|record| record.is_dirty()));

    // A fresh application instance has no in-memory mutation session. The
    // ordinary build must recover the durable target and clear it only from
    // the runner's terminal source-set step.
    let restarted = UnicaApplication::with_ports(Box::new(ports.clone()));
    let built = restarted
        .call_tool("unica.runtime.execute", &build_args(&workspace))
        .unwrap();
    assert!(built.ok, "{}: {:?}", built.summary, built.errors);
    assert_eq!(
        built.details.as_ref().unwrap()["processed"][0]["reason"],
        "buildStepSucceeded"
    );

    ports.set_dump_behavior(DumpBehavior::Metadata(edited_bytes.clone()));
    let dumped = restarted
        .call_tool("unica.runtime.execute", &metadata_dump_args(&workspace))
        .unwrap();
    assert!(dumped.ok, "{}: {:?}", dumped.summary, dumped.errors);
    assert_eq!(std::fs::read(&catalog_path).unwrap(), edited_bytes);
    assert_eq!(
        dumped.details.as_ref().unwrap()["processed"][0]["reason"],
        "shadowMatchesSource"
    );
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .all(|record| !record.is_dirty()));
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_state_blocks_typed_full_dump_before_handler() {
    assert_active_state_blocks_unsafe_dump_mode("full");
}

#[test]
fn active_state_blocks_typed_incremental_dump_before_handler() {
    assert_active_state_blocks_unsafe_dump_mode("incremental");
}

#[test]
fn project_root_drift_blocks_build_before_handler() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);
    seed_alternate_source_root(&workspace);
    write_project_config(&workspace.join("v8project.yaml"), "other");

    let blocked = app
        .call_tool("unica.runtime.execute", &build_args(&workspace))
        .expect("source topology drift must be represented as an operation result");

    assert_blocked_with_conflict_reason(&blocked, "sourceTopologyChanged");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn project_root_drift_blocks_partial_dump_before_handler() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);
    seed_alternate_source_root(&workspace);
    write_project_config(&workspace.join("v8project.yaml"), "other");

    let blocked = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, true))
        .expect("source topology drift must be represented as an operation result");

    assert_blocked_with_conflict_reason(&blocked, "sourceTopologyChanged");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn removing_explicit_source_sets_blocks_build_and_partial_dump_as_topology_drift() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);
    std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\n").unwrap();

    let build = app
        .call_tool("unica.runtime.execute", &build_args(&workspace))
        .unwrap();
    let dump = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, true))
        .unwrap();

    assert_blocked_with_conflict_reason(&build, "sourceTopologyChanged");
    assert_blocked_with_conflict_reason(&dump, "sourceTopologyChanged");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn duplicate_runtime_source_root_blocks_persisted_target_reconciliation() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: alias\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();

    let build = app
        .call_tool("unica.runtime.execute", &build_args(&workspace))
        .unwrap();
    let dump = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, true))
        .unwrap();

    assert_blocked_with_conflict_reason(&build, "sourceTopologyChanged");
    assert_blocked_with_conflict_reason(&dump, "sourceTopologyChanged");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn custom_build_config_with_different_root_blocks_before_handler() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);
    seed_alternate_source_root(&workspace);
    let custom_config = workspace.join("custom-v8project.yaml");
    write_project_config(&custom_config, "other");
    let mut args = build_args(&workspace);
    args.insert(
        "config".to_string(),
        json!(custom_config.display().to_string()),
    );

    let blocked = app
        .call_tool("unica.runtime.execute", &args)
        .expect("custom source topology drift must be represented as an operation result");

    assert_blocked_with_conflict_reason(&blocked, "sourceTopologyChanged");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn dirty_partial_dump_blocks_by_default_but_force_reaches_runner() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);

    let blocked = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, false))
        .unwrap();
    assert_blocked_with_conflict_reason(&blocked, "localSourceDiverged");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);

    ports.set_dump_behavior(DumpBehavior::Exact(before.clone()));
    let forced = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, true))
        .unwrap();
    assert!(forced.ok, "{}: {:?}", forced.summary, forced.errors);
    assert_eq!(ports.runtime_args.borrow().len(), 1);
    assert!(!ports.runtime_args.borrow()[0].contains_key("force"));
    assert_eq!(std::fs::read(&module_path).unwrap(), before);
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unknown_partial_dump_selector_is_structured_and_never_reaches_runner() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);
    let args = partial_dump_args(&workspace, &["Catalog:Missing"], false);

    let blocked = app.call_tool("unica.runtime.execute", &args).unwrap();

    assert!(!blocked.ok);
    let details = blocked.details.as_ref().unwrap();
    assert_eq!(details["requested"].as_array().unwrap().len(), 1);
    assert_eq!(details["processed"], json!([]));
    assert_eq!(details["skipped"], json!([]));
    assert_eq!(details["conflicted"].as_array().unwrap().len(), 1);
    assert_eq!(details["requested"][0]["ownerSelector"], "Catalog:Missing");
    assert_eq!(details["requested"][0]["sourceSet"], "main");
    assert_eq!(details["requested"][0]["targetId"], Value::Null);
    assert_eq!(details["conflicted"][0]["ownerSelector"], "Catalog:Missing");
    assert_eq!(details["conflicted"][0]["sourceSet"], "main");
    assert_eq!(details["conflicted"][0]["targetId"], Value::Null);
    assert_eq!(details["conflicted"][0]["reason"], "baselineMissing");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mixed_known_and_unknown_dump_batch_has_one_terminal_per_request() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);
    let args = partial_dump_args(
        &workspace,
        &["CommonModule:SampleService", "Catalog:Missing"],
        false,
    );

    let blocked = app.call_tool("unica.runtime.execute", &args).unwrap();

    assert!(!blocked.ok);
    let details = blocked.details.as_ref().unwrap();
    let requested = details["requested"].as_array().unwrap();
    let processed = details["processed"].as_array().unwrap();
    let skipped = details["skipped"].as_array().unwrap();
    let conflicted = details["conflicted"].as_array().unwrap();
    assert_eq!(requested.len(), 2);
    assert!(processed.is_empty());
    assert!(skipped.is_empty());
    assert_eq!(conflicted.len(), 2);

    let known = requested
        .iter()
        .find(|entry| entry["ownerSelector"] == "CommonModule:SampleService")
        .unwrap();
    let known_id = known["targetId"].as_str().unwrap();
    assert_eq!(
        conflicted
            .iter()
            .filter(|entry| entry["targetId"] == known_id)
            .count(),
        1
    );
    assert!(conflicted.iter().any(|entry| {
        entry["targetId"] == known_id && entry["reason"] == "localSourceDiverged"
    }));
    assert_eq!(
        conflicted
            .iter()
            .filter(|entry| entry["ownerSelector"] == "Catalog:Missing")
            .count(),
        1
    );
    let unknown = conflicted
        .iter()
        .find(|entry| entry["ownerSelector"] == "Catalog:Missing")
        .unwrap();
    assert_eq!(unknown["sourceSet"], "main");
    assert_eq!(unknown["targetId"], Value::Null);
    assert_eq!(unknown["reason"], "baselineMissing");
    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn committed_change_emits_cache_event_and_notification_even_when_finalization_fails() {
    let (root, workspace, _module_path) = round_trip_workspace();
    let configuration = workspace.join("src/Configuration.xml");
    let before = std::fs::read(&configuration).unwrap();
    let ports = Rc::new(PostCommitFailurePorts::default());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));

    let result = app
        .call_tool("unica.cf.edit", &cf_edit_args(&workspace))
        .unwrap();

    assert!(!result.ok);
    assert!(!result.changes.is_empty());
    assert_ne!(std::fs::read(&configuration).unwrap(), before);
    assert_eq!(result.cache.mode, "applied");
    assert_eq!(result.cache.events, ["ConfigXmlChanged"]);
    assert_eq!(ports.cache_events.borrow().len(), 1);
    assert_eq!(
        ports.cache_events.borrow()[0].kind,
        DomainEventKind::ConfigXmlChanged
    );
    assert_eq!(ports.notifications.borrow().len(), 1);
    assert_eq!(
        ports.notifications.borrow()[0].kind,
        DomainEventKind::ConfigXmlChanged
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn forced_dump_terminal_audits_observed_and_published_manifests() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    activate_source_sync(&app, &workspace, &module_path);
    assert!(
        app.call_tool("unica.runtime.execute", &build_args(&workspace))
            .unwrap()
            .ok
    );

    let infobase_bytes = b"\xef\xbb\xbfProcedure FromInfobase()\r\nEndProcedure\r\n".to_vec();
    ports.set_dump_behavior(DumpBehavior::Divergent(infobase_bytes.clone()));
    let forced = app
        .call_tool("unica.runtime.execute", &dump_args(&workspace, true))
        .unwrap();

    assert!(forced.ok, "{}: {:?}", forced.summary, forced.errors);
    assert_eq!(std::fs::read(&module_path).unwrap(), infobase_bytes);
    let terminal = &forced.details.as_ref().unwrap()["processed"][0];
    assert_eq!(terminal["reason"], "forcedInfobasePublication");
    assert_eq!(terminal["forced"], true);
    assert!(terminal["observedShadow"].is_object(), "{terminal}");
    assert!(terminal["publishedManifest"].is_object(), "{terminal}");
    assert_eq!(terminal["observedShadow"], terminal["publishedManifest"]);

    let state = repository(&workspace).load_state().unwrap();
    let published = &state.targets.values().next().unwrap().current;
    assert_eq!(terminal["observedShadow"], json!(published));
    assert_eq!(terminal["publishedManifest"], json!(published));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn source_sync_dry_runs_neither_create_nor_mutate_state_lock_or_shadow() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let transaction_root = repository(&workspace).transaction_root().to_path_buf();
    let original_module = std::fs::read(&module_path).unwrap();

    run_source_sync_dry_runs(&app, &workspace);

    assert_eq!(std::fs::read(&module_path).unwrap(), original_module);
    assert!(!transaction_root.exists());
    assert!(shadow_artifacts(&workspace).is_empty());

    activate_source_sync(&app, &workspace, &module_path);
    let committed_module = std::fs::read(&module_path).unwrap();
    let before = file_tree_snapshot(&transaction_root);
    assert!(!before.is_empty());

    run_source_sync_dry_runs(&app, &workspace);

    assert_eq!(std::fs::read(&module_path).unwrap(), committed_module);
    assert_eq!(file_tree_snapshot(&transaction_root), before);
    assert!(shadow_artifacts(&workspace).is_empty());
    assert!(ports.runtime_args.borrow().iter().all(|args| {
        args.get("config")
            .and_then(Value::as_str)
            .is_none_or(|config| !config.contains("/transactions/shadow-dump-"))
    }));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn dry_run_blocks_without_recovering_pending_runtime_artifacts() {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports));
    activate_source_sync(&app, &workspace, &module_path);
    let transaction_root = repository(&workspace).transaction_root().to_path_buf();

    let build_snapshot = transaction_root.join(format!("build-snapshot-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&build_snapshot).unwrap();
    let before = file_tree_snapshot(&transaction_root);
    let mut build = build_args(&workspace);
    build.insert("dryRun".to_string(), json!(true));
    let result = app.call_tool("unica.runtime.execute", &build).unwrap();
    assert!(!result.ok);
    assert!(result.errors.join("\n").contains("pinned build recovery"));
    assert_eq!(file_tree_snapshot(&transaction_root), before);
    std::fs::remove_dir(&build_snapshot).unwrap();

    let publication = transaction_root
        .join("publications")
        .join(format!("publication-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&publication).unwrap();
    let before = file_tree_snapshot(&transaction_root);
    let result = app.call_tool("unica.runtime.execute", &build).unwrap();
    assert!(!result.ok);
    assert!(result
        .errors
        .join("\n")
        .contains("publication recovery is pending"));
    assert_eq!(file_tree_snapshot(&transaction_root), before);
    std::fs::remove_dir(&publication).unwrap();
    std::fs::remove_dir(transaction_root.join("publications")).unwrap();

    let shadow = transaction_root
        .join("transactions")
        .join(format!("shadow-dump-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&shadow).unwrap();
    let before = file_tree_snapshot(&transaction_root);
    let mut dump = dump_args(&workspace, false);
    dump.insert("dryRun".to_string(), json!(true));
    let result = app.call_tool("unica.runtime.execute", &dump).unwrap();
    assert!(!result.ok);
    assert!(result
        .errors
        .join("\n")
        .contains("shadow transaction recovery"));
    assert_eq!(file_tree_snapshot(&transaction_root), before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn config_switch_between_preflight_and_code_writer_cannot_redirect_the_patch() {
    let (root, workspace, module_path) = round_trip_workspace();
    seed_alternate_source_root(&workspace);
    let alternate_module = workspace.join("other/CommonModules/SampleService/Ext/Module.bsl");
    let before_primary = std::fs::read(&module_path).unwrap();
    let before_alternate = std::fs::read(&alternate_module).unwrap();
    let ports = Rc::new(RoundTripPorts::new());
    ports.switch_config_before_patch("other");
    let app = UnicaApplication::with_ports(Box::new(ports));

    let result = app
        .call_tool("unica.code.patch", &code_patch_args(&workspace))
        .unwrap();

    assert!(!result.ok);
    assert!(result.errors.join("\n").contains("sourceDir"));
    assert_eq!(std::fs::read(&module_path).unwrap(), before_primary);
    assert_eq!(std::fs::read(&alternate_module).unwrap(), before_alternate);
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn config_switch_during_build_cannot_clear_the_original_dirty_target() {
    let (root, workspace, module_path) = round_trip_workspace();
    seed_alternate_source_root(&workspace);
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    assert!(
        app.call_tool("unica.code.patch", &code_patch_args(&workspace))
            .unwrap()
            .ok
    );
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .all(crate::domain::source_sync::SourceTargetRecord::is_dirty));
    ports.switch_config_during_build("other");

    let result = app
        .call_tool("unica.runtime.execute", &build_args(&workspace))
        .unwrap();

    assert!(!result.ok);
    let details = result.details.as_ref().unwrap();
    assert!(details["processed"].as_array().unwrap().is_empty());
    assert_eq!(details["conflicted"][0]["reason"], "runtimeConfigChanged");
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .all(crate::domain::source_sync::SourceTargetRecord::is_dirty));
    assert!(std::fs::read(&module_path)
        .unwrap()
        .starts_with(b"\xef\xbb\xbf"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn meta_edit_rejects_autodetected_roots_before_writer_or_state() {
    for keep_empty_config in [false, true] {
        let (root, workspace, _module_path) = round_trip_workspace();
        let descriptor = workspace.join("src/Catalogs/Items.xml");
        let before = std::fs::read(&descriptor).unwrap();
        if keep_empty_config {
            std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        } else {
            std::fs::remove_file(workspace.join("v8project.yaml")).unwrap();
        }
        let transaction_root = repository(&workspace).transaction_root().to_path_buf();
        let app = UnicaApplication::with_ports(Box::new(Rc::new(RoundTripPorts::new())));

        let error = app
            .call_tool("unica.meta.edit", &meta_edit_args(&workspace))
            .unwrap_err();

        assert!(error.contains("explicit non-empty `source-set`"));
        assert_eq!(std::fs::read(&descriptor).unwrap(), before);
        assert!(!transaction_root.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn untracked_meta_edit_without_platform_root_keeps_native_compatibility() {
    let (root, workspace, _module_path) = round_trip_workspace();
    let descriptor = workspace.join("src/Catalogs/Items.xml");
    std::fs::remove_file(workspace.join("src/Configuration.xml")).unwrap();
    let transaction_root = repository(&workspace).transaction_root().to_path_buf();
    let app = UnicaApplication::new();
    let mut args = meta_edit_args(&workspace);
    args.insert("NoValidate".to_string(), json!(true));

    let result = app.call_tool("unica.meta.edit", &args).unwrap();

    assert!(result.ok, "{}: {:?}", result.summary, result.errors);
    assert!(std::fs::read_to_string(&descriptor)
        .unwrap()
        .contains("<CodeLength>10</CodeLength>"));
    assert!(
        !transaction_root.exists(),
        "an untracked incomplete source-set must not create source-sync state"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_meta_edit_target_keeps_native_error_surface_without_sync_state() {
    let (root, workspace, _module_path) = round_trip_workspace();
    let app = UnicaApplication::new();
    let mut args = meta_edit_args(&workspace);
    args.insert(
        "ObjectPath".to_string(),
        json!("src/Catalogs/DoesNotExist.xml"),
    );
    let transaction_root = repository(&workspace).transaction_root().to_path_buf();

    let result = app.call_tool("unica.meta.edit", &args).unwrap();

    assert!(!result.ok);
    assert!(result
        .errors
        .iter()
        .any(|error| error.contains("Object file not found")));
    assert!(!transaction_root.exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn descriptorless_meta_edit_directory_keeps_native_error_surface_without_sync_state() {
    let (root, workspace, _module_path) = round_trip_workspace();
    let directory = workspace.join("src/Catalogs/DoesNotExist");
    std::fs::create_dir_all(&directory).unwrap();
    let app = UnicaApplication::new();
    let mut args = meta_edit_args(&workspace);
    args.insert(
        "ObjectPath".to_string(),
        json!(directory.display().to_string()),
    );
    let transaction_root = repository(&workspace).transaction_root().to_path_buf();

    let result = app.call_tool("unica.meta.edit", &args).unwrap();

    assert!(!result.ok);
    assert!(result
        .errors
        .iter()
        .any(|error| error.contains("Directory given but no DoesNotExist.xml")));
    assert!(!transaction_root.exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn configured_source_dir_patch_records_source_set_and_normal_build_clears_it() {
    let (root, workspace, module_path) = round_trip_workspace();
    let before = std::fs::read(&module_path).unwrap();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports));
    let mut patch_args = code_patch_args(&workspace);
    patch_args.remove("sourceSet");
    patch_args.insert(
        "sourceDir".to_string(),
        json!(workspace.join("src").display().to_string()),
    );

    let patched = app.call_tool("unica.code.patch", &patch_args).unwrap();

    assert!(patched.ok, "{}: {:?}", patched.summary, patched.errors);
    assert_eq!(
        patched.details.as_ref().unwrap()["affectedTargets"][0]["sourceSet"],
        "main"
    );
    assert_ne!(std::fs::read(&module_path).unwrap(), before);

    let built = app
        .call_tool("unica.runtime.execute", &build_args(&workspace))
        .unwrap();
    assert!(built.ok, "{}: {:?}", built.summary, built.errors);
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .all(|record| !record.is_dirty()));

    let _ = std::fs::remove_dir_all(root);
}

fn shadow_source_root(args: &Map<String, Value>) -> Result<PathBuf, String> {
    let config = args
        .get("config")
        .and_then(Value::as_str)
        .ok_or_else(|| "shadow runtime call has no config".to_string())?;
    let yaml = std::fs::read_to_string(config)
        .map_err(|error| format!("failed to read shadow config: {error}"))?;
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(&yaml)
        .map_err(|error| format!("failed to parse shadow config: {error}"))?;
    yaml.get("source-set")
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|sets| {
            sets.iter()
                .find(|entry| entry.get("name").and_then(serde_yaml::Value::as_str) == Some("main"))
        })
        .and_then(|entry| entry.get("path"))
        .and_then(serde_yaml::Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| "shadow config has no main source-set path".to_string())
}

fn repository(workspace: &Path) -> SourceSyncRepository {
    let context = WorkspaceContext::discover(workspace.to_path_buf()).unwrap();
    SourceSyncRepository::new(&context).unwrap()
}

fn build_args(workspace: &Path) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "dryRun": false,
        "operation": "build",
        "sourceSet": "main",
    })
    .as_object()
    .unwrap()
    .clone()
}

fn cf_edit_args(workspace: &Path) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "dryRun": false,
        "ConfigPath": "src",
        "Operation": "modify-property",
        "Value": "Version=2.0",
    })
    .as_object()
    .unwrap()
    .clone()
}

fn run_source_sync_dry_runs(app: &UnicaApplication, workspace: &Path) {
    let mut patch = code_patch_args(workspace);
    patch.insert("dryRun".to_string(), json!(true));
    let patch_result = app.call_tool("unica.code.patch", &patch).unwrap();
    assert!(
        patch_result.ok,
        "{}: {:?}",
        patch_result.summary, patch_result.errors
    );

    let mut build = build_args(workspace);
    build.insert("dryRun".to_string(), json!(true));
    let build_result = app.call_tool("unica.runtime.execute", &build).unwrap();
    assert!(
        build_result.ok,
        "{}: {:?}",
        build_result.summary, build_result.errors
    );
    assert_read_only_sync_preview(&build_result);

    let mut dump = dump_args(workspace, false);
    dump.insert("dryRun".to_string(), json!(true));
    let dump_result = app.call_tool("unica.runtime.execute", &dump).unwrap();
    assert!(!dump_result.ok);
    assert_read_only_sync_preview(&dump_result);
}

fn assert_read_only_sync_preview(result: &OperationResult) {
    let details = result
        .details
        .as_ref()
        .expect("runtime source-sync dry-run exposes structured details");
    assert_eq!(details["dryRun"], true);
    for terminal in ["requested", "processed", "skipped", "conflicted"] {
        assert!(
            details[terminal].is_array(),
            "missing terminal array {terminal}"
        );
    }
    assert_eq!(
        details["processed"].as_array().unwrap().len()
            + details["skipped"].as_array().unwrap().len()
            + details["conflicted"].as_array().unwrap().len(),
        details["requested"].as_array().unwrap().len()
    );
}

fn unsafe_dump_args(workspace: &Path, mode: &str) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "dryRun": false,
        "operation": "dump",
        "mode": mode,
        "sourceSet": "main",
    })
    .as_object()
    .unwrap()
    .clone()
}

fn partial_dump_args(workspace: &Path, objects: &[&str], force: bool) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "dryRun": false,
        "operation": "dump",
        "mode": "partial",
        "objects": objects,
        "sourceSet": "main",
        "force": force,
    })
    .as_object()
    .unwrap()
    .clone()
}

fn dump_args(workspace: &Path, force: bool) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "dryRun": false,
        "operation": "dump",
        "mode": "partial",
        "object": " CommonModule : SampleService ",
        "objects": ["CommonModule:SampleService"],
        "sourceSet": "main",
        "force": force,
    })
    .as_object()
    .unwrap()
    .clone()
}

fn metadata_dump_args(workspace: &Path) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "dryRun": false,
        "operation": "dump",
        "mode": "partial",
        "object": "Catalog:Items",
        "sourceSet": "main",
    })
    .as_object()
    .unwrap()
    .clone()
}

fn code_patch_args(workspace: &Path) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "sourceSet": "main",
        "modulePath": "CommonModules/SampleService/Ext/Module.bsl",
        "selector": "anchor",
        "methodName": "ЗаписатьДанные",
        "anchor": "МенеджерЗаписи.Записать();",
        "operation": "insertBefore",
        "content": "ПодготовитьДанные();\n    ",
        "expectedCount": 1,
        "platformSyntax": "none",
        "dryRun": false,
    })
    .as_object()
    .unwrap()
    .clone()
}

fn meta_edit_args(workspace: &Path) -> Map<String, Value> {
    json!({
        "cwd": workspace.display().to_string(),
        "dryRun": false,
        "ObjectPath": "src/Catalogs/Items.xml",
        "Operation": "modify-property",
        "Value": "CodeLength=10",
    })
    .as_object()
    .unwrap()
    .clone()
}

fn round_trip_workspace() -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "unica-source-sync-roundtrip-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let module_dir = src.join("CommonModules/SampleService/Ext");
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::create_dir_all(src.join("Catalogs/Items/Ext")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nbuilder: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        src.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
  <Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
    <Properties><Name>RoundTrip</Name></Properties>
    <ChildObjects>
      <CommonModule>SampleService</CommonModule>
      <Catalog>Items</Catalog>
    </ChildObjects>
  </Configuration>
</MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(src.join("ConfigDumpInfo.xml"), b"<ConfigDumpInfo />\n").unwrap();
    std::fs::write(
        src.join("CommonModules/SampleService.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
  <CommonModule uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
    <Properties><Name>SampleService</Name></Properties>
  </CommonModule>
</MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        src.join("Catalogs/Items.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
  <Catalog uuid="cccccccc-cccc-cccc-cccc-cccccccccccc">
    <Properties><Name>Items</Name><CodeLength>9</CodeLength></Properties>
  </Catalog>
</MetaDataObject>"#,
    )
    .unwrap();
    let module_path = module_dir.join("Module.bsl");
    std::fs::write(
        &module_path,
        b"\xef\xbb\xbf\xd0\x9f\xd1\x80\xd0\xbe\xd1\x86\xd0\xb5\xd0\xb4\xd1\x83\xd1\x80\xd0\xb0 \xd0\x97\xd0\xb0\xd0\xbf\xd0\xb8\xd1\x81\xd0\xb0\xd1\x82\xd1\x8c\xd0\x94\xd0\xb0\xd0\xbd\xd0\xbd\xd1\x8b\xd0\xb5()\r\n    \xd0\x9c\xd0\xb5\xd0\xbd\xd0\xb5\xd0\xb4\xd0\xb6\xd0\xb5\xd1\x80\xd0\x97\xd0\xb0\xd0\xbf\xd0\xb8\xd1\x81\xd0\xb8.\xd0\x97\xd0\xb0\xd0\xbf\xd0\xb8\xd1\x81\xd0\xb0\xd1\x82\xd1\x8c();\r\n\xd0\x9a\xd0\xbe\xd0\xbd\xd0\xb5\xd1\x86\xd0\x9f\xd1\x80\xd0\xbe\xd1\x86\xd0\xb5\xd0\xb4\xd1\x83\xd1\x80\xd1\x8b\r\n",
    )
    .unwrap();
    (root, workspace, module_path)
}

fn assert_active_state_blocks_unsafe_dump_mode(mode: &str) {
    let (root, workspace, module_path) = round_trip_workspace();
    let ports = Rc::new(RoundTripPorts::new());
    let app = UnicaApplication::with_ports(Box::new(ports.clone()));
    let before = activate_source_sync(&app, &workspace, &module_path);

    let blocked = app
        .call_tool("unica.runtime.execute", &unsafe_dump_args(&workspace, mode))
        .expect("unsafe dump mode must be represented as an operation result");

    assert!(ports.runtime_args.borrow().is_empty());
    assert_eq!(std::fs::read(&module_path).unwrap(), before);
    assert_blocked_with_conflict_reason(&blocked, "unsafeDumpMode");
    assert!(repository(&workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .any(|record| record.is_dirty()));
    assert!(shadow_artifacts(&workspace).is_empty());

    let _ = std::fs::remove_dir_all(root);
}

fn activate_source_sync(app: &UnicaApplication, workspace: &Path, module_path: &Path) -> Vec<u8> {
    let patched = app
        .call_tool("unica.code.patch", &code_patch_args(workspace))
        .unwrap();
    assert!(patched.ok, "{}: {:?}", patched.summary, patched.errors);
    let bytes = std::fs::read(module_path).unwrap();
    assert!(repository(workspace)
        .load_state()
        .unwrap()
        .targets
        .values()
        .any(|record| record.is_dirty()));
    bytes
}

fn assert_blocked_with_conflict_reason(result: &OperationResult, reason: &str) {
    assert!(
        !result.ok,
        "operation unexpectedly succeeded: {}",
        result.summary
    );
    let details = result
        .details
        .as_ref()
        .expect("blocked source-sync operation must include details");
    assert!(
        details["conflicted"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["reason"] == reason)),
        "missing conflict reason {reason}: {details}"
    );
}

fn seed_alternate_source_root(workspace: &Path) {
    let source = workspace.join("src");
    let alternate = workspace.join("other");
    std::fs::create_dir_all(alternate.join("CommonModules/SampleService/Ext")).unwrap();
    for relative in [
        "Configuration.xml",
        "ConfigDumpInfo.xml",
        "CommonModules/SampleService.xml",
        "CommonModules/SampleService/Ext/Module.bsl",
    ] {
        let destination = alternate.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::copy(source.join(relative), destination).unwrap();
    }
}

fn write_project_config(path: &Path, source_root: &str) {
    std::fs::write(
        path,
        format!(
            "format: DESIGNER\nbuilder: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: {source_root}\n"
        ),
    )
    .unwrap();
}

fn shadow_artifacts(workspace: &Path) -> Vec<PathBuf> {
    let mut artifacts = Vec::new();
    if let Ok(entries) = std::fs::read_dir(workspace) {
        artifacts.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".unica-shadow-dump-"))
        }));
    }
    let transaction_root = workspace.join(".build/unica/source-sync");
    if transaction_root.exists() {
        collect_shadow_artifacts(&transaction_root, &mut artifacts);
    }
    artifacts
}

fn file_tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, snapshot: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                visit(root, &path, snapshot);
            } else {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                snapshot.insert(relative, std::fs::read(path).unwrap());
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    if root.exists() {
        visit(root, root, &mut snapshot);
    }
    snapshot
}

fn collect_shadow_artifacts(directory: &Path, artifacts: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("shadow-dump-"))
        {
            artifacts.push(path.clone());
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            collect_shadow_artifacts(&path, artifacts);
        }
    }
}
