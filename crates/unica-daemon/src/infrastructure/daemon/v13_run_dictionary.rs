use super::protocol::InvocationRequest;
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::tool_catalog::{
    catalog_for, run_dictionary, RunIntent, RunnerContract, DEBUG_SERVER_START_CONTRACT_VERSION,
    PINNED_RUNNER,
};
use crate::infrastructure::bundled_tools::{declared_tool_at_version, DeclaredTool};
use crate::infrastructure::plugin_runtime::find_plugin_root;
use serde_json::{json, Value};
use std::path::Path;
use unica_artifact_delivery::engine::BundledTool;
use unica_execution::invocation_store::ToolIdentity;
use unica_execution_contract::invocation::DomainResult;
use unica_refusal::RefusalCode;

/// Словарь операций `run` и отказ по неизвестному имени.
///
/// Исполнителя здесь больше нет: `v8project.yaml` заводит человек или модель
/// своими файловыми средствами, а инструмента записи проектного файла в
/// продукте не остаётся вовсе.
pub(super) fn execute_run_dictionary(request: &InvocationRequest) -> Option<DomainResult> {
    if request.tool() != ToolIdentity::Run {
        return None;
    }
    if request.arguments().contains_key("ifRev") {
        return Some(DomainResult::canonical_rejection(
            None,
            RefusalCode::BadValue,
            "unica.run does not accept ifRev; use dryRun: true to preview or dryRun: false to execute",
        ));
    }
    match request.arguments().get("op") {
        None if request.arguments().is_empty() => {
            Some(run_dictionary_result(Path::new(request.workspace_hint())))
        }
        None => Some(DomainResult::canonical_rejection(
            None,
            RefusalCode::BadValue,
            "run without op lists the operation dictionary and accepts no other arguments",
        )),
        Some(Value::String(op)) => {
            if is_test_seam_operation(op) {
                return None;
            }
            let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
            if catalog
                .run_dictionary
                .iter()
                .any(|operation| operation.name() == op)
            {
                // The handler checks the installed bytes immediately before use.
                reject_unavailable_run_for(request, || PINNED_RUNNER.debug_server_start_v2)
            } else {
                Some(reject_run_operation(
                    op,
                    format!("unknown canonical run operation `{op}`"),
                ))
            }
        }
        Some(_) => None,
    }
}

pub(super) fn reject_unavailable_run_before_admission(
    request: &InvocationRequest,
) -> Option<DomainResult> {
    reject_unavailable_run_for(request, || {
        debug_server_for(Path::new(request.workspace_hint()), &PINNED_RUNNER)
            .is_some_and(DeclaredTool::available)
    })
}

fn reject_unavailable_run_for(
    request: &InvocationRequest,
    debug_server_available: impl FnOnce() -> bool,
) -> Option<DomainResult> {
    if request.tool() != ToolIdentity::Run {
        return None;
    }
    let op = match request.arguments().get("op") {
        Some(Value::String(op)) => op,
        Some(_) => {
            return Some(DomainResult::canonical_rejection(
                None,
                RefusalCode::BadValue,
                "run op must be a string",
            ))
        }
        None => return None,
    };
    if request
        .arguments()
        .get("infobase")
        .is_some_and(|v| v.as_str() != Some("origin"))
    {
        return Some(reject_run_operation(op, "the pinned runner adapter supports only the named infobase origin; no fallback to another target"));
    }
    if is_test_seam_operation(op) {
        return None;
    }
    let debug_server = matches!(op.as_str(), "debug-server.start" | "debug-server.stop")
        && debug_server_available();
    match run_dictionary(debug_server)
        .into_iter()
        .find(|operation| operation.name() == op)
    {
        Some(operation) if operation.implemented => None,
        Some(operation) => Some(reject_run_operation(
            op,
            operation
                .support_reason()
                .unwrap_or("operation is unavailable")
                .to_string(),
        )),
        None => Some(reject_run_operation(
            op,
            format!("unknown canonical run operation `{op}`"),
        )),
    }
}

/// Тестовый шов: пока словарь не был реализован целиком, нереализованная
/// операция доходила до актора рабочего пространства, и тесты долгой работы
/// демона ходили через неё. Теперь каждая операция готовится до admission, и
/// тесты зовут актора именем `test.*`, которого в продукте не существует:
/// вне тестов такое имя — неизвестная операция.
#[cfg(test)]
fn is_test_seam_operation(op: &str) -> bool {
    op.starts_with("test.")
}

#[cfg(not(test))]
const fn is_test_seam_operation(_op: &str) -> bool {
    false
}

pub(super) fn installed_debug_server_runner(cwd: &Path) -> Option<BundledTool> {
    debug_server_for(cwd, &PINNED_RUNNER)?.verified_tool()
}

pub(super) fn debug_server_declared(cwd: &Path) -> bool {
    debug_server_for(cwd, &PINNED_RUNNER).is_some()
}

fn debug_server_for(cwd: &Path, runner: &RunnerContract) -> Option<DeclaredTool> {
    if !runner.debug_server_start_v2 {
        return None;
    }
    let root = find_plugin_root(cwd)?;
    declared_tool_at_version(&root, "v8-runner", runner.version)
}

pub(super) fn run_dictionary_result(cwd: &Path) -> DomainResult {
    dictionary_for(cwd, &PINNED_RUNNER)
}

fn dictionary_for(cwd: &Path, runner: &RunnerContract) -> DomainResult {
    let operations = run_dictionary(debug_server_for(cwd, runner).is_some_and(DeclaredTool::available))
        .into_iter()
        // An unavailable operation is not listed; its call keeps its refusal.
        .filter(|operation| operation.implemented)
        .map(|operation| {
            let dry_run_required = matches!(
                operation.intent,
                RunIntent::WebPublish
                    | RunIntent::InfobaseCheck
                    | RunIntent::InfobaseCreate
                    | RunIntent::SourceImport
                    | RunIntent::SourceExport
                    | RunIntent::SourceReexport
                    | RunIntent::ArtifactBuild
                    | RunIntent::CfExport
                    | RunIntent::CfImport
                    | RunIntent::InfobaseExport
                    | RunIntent::InfobaseImport
                    | RunIntent::ExtensionList
                    | RunIntent::ConfigurationApply
                    | RunIntent::ConfigurationReset
                    | RunIntent::ExtensionActivate
            );
            let mut entry = json!({
                "op": operation.name(),
                "description": operation.description(),
                "argsSchema": operation.args_schema(),
                "execution": operation.execution(),
                "effects": operation.effects(),
                "support": {"adapter":format!("v8-runner/{}", runner.version), "state": if operation.support_reason().is_some() {"limited"} else {"supported"}, "reason":operation.support_reason(), "supportedInfobases":["origin"], "supportedArgs": operation.args_schema()},
                "terminal": operation.terminal,
                "rejectsSessions": operation.rejects_sessions,
                "previewRequired": false,
                "dryRunRequired": dry_run_required,
            });
            if operation.intent == RunIntent::DebugServerStart {
                entry["contractVersion"] = json!(DEBUG_SERVER_START_CONTRACT_VERSION);
            }
            entry
        })
        .collect::<Vec<_>>();
    let mut result = DomainResult::success("canonical run operation dictionary returned");
    result.data = Some(json!({"operations": operations}));
    result
}

fn reject_run_operation(op: &str, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(
        Some(op.to_string()),
        RefusalCode::UnsupportedOperation,
        message,
    )
}

#[cfg(test)]
mod medium_tests {
    use super::super::v13_client_run::{prepare, Preparation};
    use super::*;
    use crate::infrastructure::plugin_runtime::with_plugin_root_for_test;
    use sha2::{Digest, Sha256};
    use std::fs;

    /// rule: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
    #[test]
    fn debug_server_start_v2_requires_the_matching_installed_provider() {
        let plugin = tempfile::tempdir().unwrap();
        let manifest = plugin.path().join("third-party/manifest.json");
        fs::create_dir(manifest.parent().unwrap()).unwrap();
        let binary = plugin.path().join("runner-test-file");
        // These bytes cannot be executed as a CLI on any supported host.
        let provider_bytes = b"compatible runner test fixture";
        let checksum = format!("{:x}", Sha256::digest(provider_bytes));
        let cwd = Path::new("/workspace-does-not-exist");
        with_plugin_root_for_test(plugin.path(), || {
            for (version, contents, available) in [
                (PINNED_RUNNER.version, provider_bytes.as_slice(), true),
                ("0.19.1", provider_bytes.as_slice(), false),
                (PINNED_RUNNER.version, b"replaced runner".as_slice(), false),
            ] {
                fs::write(&binary, contents).unwrap();
                fs::write(
                    &manifest,
                    json!({"tools":[{"name":"v8-runner","version":version,
                    "binaryPath":"runner-test-file","sha256":checksum}]})
                    .to_string(),
                )
                .unwrap();
                assert_eq!(debug_server_declared(cwd), PINNED_RUNNER.version == version);
                let installed = installed_debug_server_runner(cwd);
                assert_eq!(
                    installed.map(|tool| tool.program),
                    available.then(|| binary.clone())
                );
                let dictionary = run_dictionary_result(cwd);
                let operations = dictionary.data.as_ref().unwrap()["operations"]
                    .as_array()
                    .unwrap();
                assert!(operations.iter().any(|row| row["op"] == "launch"));
                assert!(!operations
                    .iter()
                    .any(|row| row["op"] == "debug-server.prepare"));
                for op in ["debug-server.start", "debug-server.stop"] {
                    let entry = operations.iter().find(|row| row["op"] == op);
                    assert_eq!(entry.is_some(), available);
                    if let Some(row) = entry {
                        assert_eq!(row["execution"], "terminal");
                        if op == "debug-server.start" {
                            assert_eq!(row["contractVersion"], 2);
                            assert_eq!(row["argsSchema"]["required"], json!(["ownerPid"]));
                        } else {
                            assert!(row.get("contractVersion").is_none());
                        }
                    }
                    let request = InvocationRequest::new(
                        ToolIdentity::Run,
                        json!({"op":op,"args":{}}),
                        cwd.to_string_lossy(),
                        7000,
                    )
                    .unwrap();
                    let refusal = reject_unavailable_run_before_admission(&request);
                    assert_eq!(refusal.is_none(), available);
                    if let Some(refusal) = refusal {
                        assert_eq!(refusal.diagnostics[0]["code"], "unsupported_operation");
                    }
                }
                if version == "0.19.1" {
                    assert_debug_server_calls_are_unavailable(cwd);
                }
            }
        });
    }
    /// rule: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
    #[test]
    fn source_manifest_does_not_offer_debug_server_operations() {
        let plugin = tempfile::tempdir().unwrap();
        let tools = plugin.path().join("third-party");
        let target = crate::infrastructure::current_target_id().unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        let version = PINNED_RUNNER.version;
        fs::create_dir(&tools).unwrap();
        fs::write(
            tools.join("manifest.json"),
            json!({"schemaVersion":2,"sourceManifest":true,"tools":[]}).to_string(),
        )
        .unwrap();
        fs::write(
            tools.join("tools.lock.json"),
            json!({
                "schemaVersion":2,
                "targets":{(target):{"exe":suffix}},
                "tools":[{"name":"v8-runner","version":version,"binaryName":"v8-runner",
                    "assets":{(target):{"assetName":"runner.tar.gz","sha256":"a".repeat(64)}}}]
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            crate::infrastructure::bundled_tools::bundled_tool_version(plugin.path(), "v8-runner")
                .unwrap(),
            version
        );
        with_plugin_root_for_test(plugin.path(), || {
            let cwd = Path::new("/workspace-does-not-exist");
            assert_debug_server_calls_are_unavailable(cwd);
            let bin = plugin.path().join("bin").join(target);
            fs::create_dir_all(&bin).unwrap();
            fs::write(bin.join(format!("v8-runner{suffix}")), b"unverified binary").unwrap();
            assert_debug_server_calls_are_unavailable(cwd);
        });
    }

    fn assert_debug_server_calls_are_unavailable(cwd: &Path) {
        let dictionary = run_dictionary_result(cwd);
        let operations = dictionary.data.as_ref().unwrap()["operations"]
            .as_array()
            .unwrap();
        let launch = operations.iter().find(|op| op["op"] == "launch").unwrap();
        assert_eq!(
            launch["support"]["adapter"],
            format!("v8-runner/{}", PINNED_RUNNER.version)
        );
        assert!(launch["argsSchema"]["properties"]["interactiveLogin"].is_object());
        for (op, args, reason) in [
            ("debug-server.start", json!({"ownerPid":42}), "version 2"),
            (
                "debug-server.stop",
                json!({"session":"a".repeat(32)}),
                "version 2",
            ),
            (
                "debug-server.prepare",
                json!({"ownerPid":42}),
                "unknown canonical run operation",
            ),
        ] {
            assert!(!operations.iter().any(|row| row["op"] == op));
            for dry_run in [true, false] {
                let request = InvocationRequest::new(
                    ToolIdentity::Run,
                    json!({"op":op,"args":args,"dryRun":dry_run}),
                    cwd.to_string_lossy(),
                    7000,
                )
                .unwrap();
                let result = execute_run_dictionary(&request).unwrap_or_else(|| {
                    let Preparation::Rejected(refusal) = prepare(&request) else {
                        panic!("{op} must be refused before workspace admission");
                    };
                    *refusal
                });
                assert_eq!(result.diagnostics[0]["code"], "unsupported_operation");
                assert!(result.summary.contains(reason), "{}", result.summary);
            }
        }
    }
}

#[cfg(test)]
mod runner_one_tests {
    use super::*;
    /// rule: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
    #[test]
    fn development_cycle_admits_the_explicit_compatibility_subset() {
        for (op, args) in [
            ("push", json!({"force":true,"full":true})),
            ("pull", json!({"force":true})),
            ("infobase.create", json!({})),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":op,"args":args,"dryRun":true}),
                "/not-a-workspace",
                7000,
            )
            .unwrap();
            assert!(
                execute_run_dictionary(&request).is_none(),
                "{op} must reach its typed compatibility handler"
            );
            assert!(reject_unavailable_run_for(&request, || {
                panic!("{op} must not resolve the debug server executable")
            })
            .is_none());
        }
    }

    /// Раннер 0.15 умеет `push --no-apply`, `apply` и `reset`, но адаптер их ещё не
    /// исполняет, а `upload` раннера по-прежнему применяет конфигурацию БД:
    /// словарь называет операции недоступными, а вызов получает типизированный
    /// отказ с разрывом до рабочего пространства и процесса.
    /// rule: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
    #[test]
    fn runner_015_refuses_upload_apply_and_reset_naming_the_gap() {
        let dictionary = run_dictionary_result(Path::new("/workspace-does-not-exist"));
        let operations = dictionary.data.as_ref().unwrap()["operations"]
            .as_array()
            .unwrap();
        for (op, args) in [
            ("upload", json!({"input":"dist/main.cf"})),
            ("apply", json!({})),
            ("reset", json!({"force":true})),
        ] {
            for dry_run in [true, false] {
                let request = InvocationRequest::new(
                    ToolIdentity::Run,
                    json!({"op":op,"args":args,"dryRun":dry_run}),
                    "/workspace-does-not-exist",
                    7000,
                )
                .unwrap();
                let result =
                    execute_run_dictionary(&request).expect("refused before workspace or process");
                assert!(!result.ok, "{op}");
                assert_eq!(result.diagnostics[0]["code"], "unsupported_operation");
                assert!(result.summary.contains("issues/1246"), "{}", result.summary);
                assert!(result.summary.contains("push"), "{}", result.summary);
            }
            // An unavailable operation is not listed; its call above is refused.
            assert!(operations.iter().all(|v| v["op"] != op), "{op}");
        }
    }

    /// rule: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
    #[test]
    fn runner_one_refuses_unsupported_semantics_and_old_names_before_admission() {
        for (op, args) in [
            ("source.import", json!({})),
            ("source.export", json!({})),
            ("cf.import", json!({"input":"x.cf"})),
            ("extension.create", json!({"name":"X"})),
            ("extension.info", json!({"name":"X"})),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":op,"args":args,"dryRun":true}),
                "/workspace-does-not-exist",
                7000,
            )
            .unwrap();
            let result =
                execute_run_dictionary(&request).expect("refused before workspace or process");
            assert!(!result.ok, "{op}");
            assert_eq!(result.diagnostics[0]["code"], "unsupported_operation");
            assert!(result.rev.is_none());
        }
    }
    /// rule: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
    #[test]
    fn runner_one_never_redirects_an_unsupported_infobase_to_origin() {
        let request = InvocationRequest::new(ToolIdentity::Run,json!({"op":"download","infobase":"production","args":{"state":"working","output":"x.cf"},"dryRun":true}),"/workspace-does-not-exist",7000).unwrap();
        let result = execute_run_dictionary(&request).unwrap();
        assert!(!result.ok);
        assert!(result.summary.contains("no fallback"));
    }
    /// rule: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
    #[test]
    fn limited_delete_and_known_operations_reach_their_typed_parsers() {
        for (op, args) in [
            ("push", json!({"delete":"Patch"})),
            ("extensions.set", json!({"name":"Patch","active":false})),
            ("download", json!({"state":"working","output":"x.cf"})),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":op,"infobase":"origin","args":args,"dryRun":true}),
                "/workspace",
                7000,
            )
            .unwrap();
            assert!(execute_run_dictionary(&request).is_none());
        }
        let dictionary = run_dictionary_result(Path::new("/workspace-does-not-exist"));
        let operations = dictionary.data.as_ref().unwrap()["operations"]
            .as_array()
            .unwrap();
        let push = operations.iter().find(|v| v["op"] == "push").unwrap();
        assert!(push.get("implemented").is_none());
        assert_eq!(push["support"]["state"], "limited");
        assert!(push["support"]["supportedArgs"]["properties"]
            .get("delete")
            .is_some());
    }
}
