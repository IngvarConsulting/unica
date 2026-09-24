#![allow(clippy::result_large_err)]
//! Installed extensions belong to the infobase, not to the source-set tree.
//! Even inventory starts a platform session, so every operation is previewApply.

use super::protocol::InvocationRequest;
use super::runner_011::Runner011ProcessRunner;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, missing_runner_rejection,
    resolve_bundled_runner, runner_rejection, valid_1c_identifier, validate_provider_receipt,
    CONFIG_NAME, LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
};
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::BundledTool;
use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessRunner};
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    List,
    Delete,
    Activate,
}
impl Operation {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "extensions.list" => Some(Self::List),
            "push" => Some(Self::Delete),
            "extensions.set" => Some(Self::Activate),
            _ => None,
        }
    }
    const fn name(self) -> &'static str {
        match self {
            Self::List => "extensions.list",
            Self::Delete => "push",
            Self::Activate => "extensions.set",
        }
    }
    const fn command(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Delete => "delete",
            Self::Activate => "activate",
        }
    }
    const fn reads(self) -> bool {
        matches!(self, Self::List)
    }
}

#[derive(Debug, Clone)]
pub(super) struct PreparedExtensions {
    operation: Operation,
    args: Map<String, Value>,
    dry_run: bool,
    if_rev: Option<String>,
    context: WorkspaceContext,
}
pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedExtensions>),
}
pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run {
        return Preparation::NotApplicable;
    }
    let Some(operation) = request
        .arguments()
        .get("op")
        .and_then(Value::as_str)
        .and_then(Operation::parse)
    else {
        return Preparation::NotApplicable;
    };
    if operation == Operation::Delete
        && request
            .arguments()
            .get("args")
            .and_then(|v| v.get("delete"))
            .is_none()
    {
        return Preparation::NotApplicable;
    }
    match PreparedExtensions::parse(request, operation) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}
impl PreparedExtensions {
    fn parse(request: &InvocationRequest, operation: Operation) -> Result<Self, DomainResult> {
        let reject = |message: &str| rejection(operation, RefusalCode::BadValue, message);
        let args = request
            .arguments()
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject("run args must be an object"))?;
        validate_arguments(operation, args).map_err(reject)?;
        let dry_run = request
            .arguments()
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                reject("extension operations require explicit dryRun: true, or false with ifRev")
            })?;
        let if_rev = match request.arguments().get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
            _ => return Err(reject("ifRev must be non-empty text")),
        };
        if dry_run == if_rev.is_some() {
            return Err(reject(
                "preview takes no ifRev; apply requires the revision from preview",
            ));
        }
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                rejection(
                    operation,
                    RefusalCode::InvalidState,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        Ok(Self {
            operation,
            args: args.clone(),
            dry_run,
            if_rev,
            context,
        })
    }
    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-extension-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }
    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        match resolve_bundled_runner(&self.context.cwd) {
            Ok(resolved) => self.execute_with(
                &Runner011ProcessRunner,
                &resolved.tool,
                &resolved.version,
                cancellation,
            ),
            Err(message) => missing_runner_rejection(Some(self.operation.name().into()), message),
        }
    }

    fn fail(&self, code: RefusalCode, message: impl Into<String>) -> DomainResult {
        rejection(self.operation, code, message)
    }
    fn inputs(&self) -> Result<Value, DomainResult> {
        let root = &self.context.workspace_root;
        let config = digest_required_workspace_file(root, Path::new(CONFIG_NAME))
            .map_err(|e| self.fail(RefusalCode::InvalidState, e))?;
        let local = digest_optional_workspace_file(root, Path::new(LOCAL_CONFIG_NAME))
            .map_err(|e| self.fail(RefusalCode::InvalidState, e))?;
        Ok(json!({"config":config, "localConfig":local}))
    }
    /// Display declarations from structured config, never parse the runner's prose.
    /// Relative connection paths retain their declared form and name their base.
    fn target_description(&self) -> Result<Value, DomainResult> {
        let mut connection = Value::Null;
        let mut account = Value::Null;
        let mut gate = Value::Null;
        let mut connection_from = CONFIG_NAME;
        for name in [CONFIG_NAME, LOCAL_CONFIG_NAME] {
            let config = super::v13_workspace_bootstrap::read_yaml_config(
                &self.context.workspace_root,
                name,
            )
            .map_err(|error| self.fail(RefusalCode::InvalidState, error))?;
            let Some(config) = config else { continue };
            let Some(infobase) = config
                .get("infobases")
                .and_then(|v| v.get("origin"))
                .or_else(|| config.get("infobase"))
                .and_then(serde_yaml::Value::as_mapping)
            else {
                continue;
            };
            if let Some(value) = infobase.get(serde_yaml::Value::from("connection")) {
                connection_from = name;
                connection = value
                    .as_str()
                    .map(|text| {
                        // Raw CLI connection syntax may carry /P credentials. It is not
                        // echoed; the configuration reference remains explicit.
                        if text.trim_start().starts_with(['/', '-']) {
                            json!("[raw connection arguments withheld]")
                        } else {
                            json!(redactor(text))
                        }
                    })
                    .unwrap_or(Value::Null);
            }
            if let Some(value) = infobase.get(serde_yaml::Value::from("user")) {
                account = value
                    .as_str()
                    .map(|v| json!(redactor(v)))
                    .unwrap_or(Value::Null);
            }
            if let Some(value) = infobase.get(serde_yaml::Value::from("standalone")) {
                if value.is_null() {
                    gate = Value::Null;
                } else if let Some(value) = value.get("gate") {
                    gate = value
                        .as_str()
                        .map(|v| json!(redactor(v)))
                        .unwrap_or(Value::Null);
                }
            }
        }
        Ok(
            json!({"declaredConnection":connection,"connectionFrom":connection_from,"relativePathsFrom":"workspace","account":account,"standaloneGate":gate}),
        )
    }

    fn extension_name(&self) -> &Value {
        &self.args[if self.operation == Operation::Delete {
            "delete"
        } else {
            "name"
        }]
    }
    fn requested(&self) -> Value {
        json!({"kind":"all"})
    }
    fn action(&self) -> &str {
        if self.operation == Operation::Activate && self.args["active"] == false {
            "deactivate"
        } else {
            self.operation.command()
        }
    }
    fn invoke(
        &self,
        runner: &dyn ProcessRunner,
        tool: &BundledTool,
        cancellation: &CancellationToken,
        preview: bool,
    ) -> Result<Value, DomainResult> {
        if cancellation.is_cancelled() {
            return Err(self.fail(
                RefusalCode::Cancelled,
                "extension operation cancelled before provider launch",
            ));
        }
        let mut args = vec![
            "--config".into(),
            self.context
                .workspace_root
                .join(CONFIG_NAME)
                .display()
                .to_string(),
            "--json-message".into(),
            "extensions".into(),
            self.operation.command().into(),
        ];
        if self.operation != Operation::List {
            args.extend([
                "--name".into(),
                self.extension_name()
                    .as_str()
                    .expect("validated name")
                    .into(),
            ]);
        }
        if let Some(active) = self.args.get("active").and_then(Value::as_bool) {
            args.extend(["--active".into(), if active { "yes" } else { "no" }.into()]);
        }
        if preview {
            args.push("--dry-run".into());
        }
        let output = runner
            .run(&ProcessCommand {
                program: tool.program.clone(),
                args,
                cwd: self.context.workspace_root.clone(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
                timeout: None,
                // Listing and previews remain cancellable. A dispatched
                // mutation must return its provider receipt before settling.
                cancellation: if preview || self.operation.reads() {
                    cancellation.clone()
                } else {
                    cancellation.protect_process_on_spawn()
                },
            })
            .map_err(|error| {
                if error.starts_with(CANCELLED_PREFIX) {
                    return self.fail(RefusalCode::Cancelled, "cancelled before provider launch");
                }
                missing_runner_rejection(
                    Some(self.operation.name().into()),
                    format!("failed to start bundled v8-runner: {}", redactor(&error)),
                )
            })?;
        if output.cancelled {
            return Err(self.fail(RefusalCode::Cancelled, "v8-runner was cancelled"));
        }
        if output.timed_out {
            return Err(self.fail(
                RefusalCode::DeadlineExceeded,
                "v8-runner exceeded its deadline",
            ));
        }
        if output.stdout_truncated || output.stdout_had_invalid_utf8 {
            return Err(self.fail(
                RefusalCode::InvalidResult,
                "v8-runner returned unreadable or oversized JSON",
            ));
        }
        let envelope: Value = serde_json::from_str(&output.stdout).map_err(|_| {
            self.fail(
                RefusalCode::InvalidResult,
                "v8-runner returned invalid JSON",
            )
        })?;
        if envelope["command"] != "extensions" {
            return Err(self.fail(
                RefusalCode::InvalidResult,
                "v8-runner answered for another command",
            ));
        }
        if preview && envelope["data"]["provider_dispatched"] == true {
            return Err(self.fail(
                RefusalCode::InvalidResult,
                "v8-runner dispatched the platform during preview",
            ));
        }
        if !output.status_success || envelope["ok"] != true {
            return Err(runner_rejection(
                Some(self.operation.name().into()),
                envelope["error"]["code"]
                    .as_str()
                    .unwrap_or("provider_failed"),
                envelope["error"]["message"]
                    .as_str()
                    .map(redactor)
                    .unwrap_or_else(|| "v8-runner failed without a typed message".into()),
            ));
        }
        Ok(envelope)
    }
    fn validate(&self, envelope: &Value, preview: bool) -> Result<Value, DomainResult> {
        let invalid = |message| self.fail(RefusalCode::InvalidResult, message);
        let data = &envelope["data"];
        if data["ok"] != true || data["provider_dispatched"] != !preview {
            return Err(invalid(
                "extension result does not prove its execution state",
            ));
        }
        let receipt = validate_provider_receipt(&data["provider"]).map_err(invalid)?;
        if self.operation.reads() {
            if data["requested"] != self.requested() {
                return Err(invalid("extension inventory answered for another subject"));
            }
            let items = data["extensions"]
                .as_array()
                .ok_or_else(|| invalid("extension inventory has no records"))?;
            if preview {
                if !items.is_empty() {
                    return Err(invalid(
                        "extension preview claims to have read the platform",
                    ));
                }
            } else {
                if items.len() > 4096 {
                    return Err(invalid("extension inventory exceeds the result limit"));
                }
                let mut names = std::collections::BTreeSet::new();
                for item in items {
                    let name = item["name"]
                        .as_str()
                        .filter(|v| valid_1c_identifier(v))
                        .ok_or_else(|| invalid("extension record has no valid name"))?;
                    if !names.insert(name) || !valid_inventory_record(item) {
                        return Err(invalid(
                            "extension inventory contains duplicate or malformed records",
                        ));
                    }
                }
            }
        } else {
            let steps = data["steps"]
                .as_array()
                .ok_or_else(|| invalid("extension change has no steps"))?;
            if steps.len() != 1
                || steps[0]["target"] != *self.extension_name()
                || steps[0]["action"] != self.action()
                || steps[0]["ok"] != true
            {
                return Err(invalid(
                    "extension change does not match its requested target and action",
                ));
            }
        }
        Ok(receipt)
    }
    fn execute_with(
        &self,
        runner: &dyn ProcessRunner,
        tool: &BundledTool,
        version: &str,
        cancellation: CancellationToken,
    ) -> DomainResult {
        match self.execute_checked(runner, tool, version, cancellation) {
            Ok(result) | Err(result) => result,
        }
    }
    fn execute_checked(
        &self,
        runner: &dyn ProcessRunner,
        tool: &BundledTool,
        version: &str,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, DomainResult> {
        if cancellation.is_cancelled() {
            return Err(self.fail(
                RefusalCode::Cancelled,
                "extension operation cancelled before preview",
            ));
        }
        let before = self.inputs()?;
        let preview = self.invoke(runner, tool, &cancellation, true)?;
        let receipt = self.validate(&preview, true)?;
        let target = self.target_description()?;
        if before != self.inputs()? {
            return Err(self.fail(
                RefusalCode::ConcurrentChange,
                "project configuration changed during preview",
            ));
        }
        let encoded = serde_json::to_vec(&json!({"op":self.operation.name(),"args":self.args,"inputs":before,"workspace":self.workspace_identity_hash().as_str(),"runnerVersion":version,"provider":receipt})).expect("revision serializes");
        let revision = format!("unica-extension-sha256-v1:{:x}", Sha256::digest(encoded));
        let plan = json!({"op":self.operation.name(),"args":self.args,"target":target,"provider":receipt["selected"],"providerOrigin":receipt["origin"]["kind"],"requiresPlatform":true,"deletesExtensionData":self.operation == Operation::Delete});
        if self.dry_run {
            let mut result =
                DomainResult::success("extension operation planned without starting the platform");
            result.data = Some(json!({"op":self.operation.name(),"dryRun":true,"plan":plan}));
            result.rev = Some(revision.clone());
            result.next.push(json!({"tool":"unica.run","args":{"op":self.operation.name(),"args":self.args,"dryRun":false,"ifRev":revision},"reason":"execute exactly this previewed operation; the preview does not inspect infobase state"}));
            return Ok(result);
        }
        if self.if_rev.as_deref() != Some(revision.as_str()) {
            return Err(self.fail(
                RefusalCode::StaleRevision,
                "extension plan or configuration changed; preview again",
            ));
        }
        let applied = self.invoke(runner, tool, &cancellation, false)?;
        let applied_receipt = self.validate(&applied, false)?;
        if applied_receipt != receipt {
            return Err(self.fail(
                RefusalCode::InvalidResult,
                "extension operation used a different provider than previewed",
            ));
        }
        let mut result = DomainResult::success(
            "extension operation completed; infobase facts are attested by the platform provider",
        );
        let mut data = json!({"op":self.operation.name(),"dryRun":false,"provider":receipt["selected"],"targetStateAttestedBy":"provider"});
        if self.operation.reads() {
            data["requested"] = self.requested();
            let items = applied["data"]["extensions"]
                .as_array()
                .expect("validated records");
            data["extensions"] = Value::Array(items.iter().map(public_record).collect());
            data["namePrefixAvailable"] = json!(
                !items.is_empty() && items.iter().all(|item| item["name_prefix"].is_string())
            );
        } else {
            data["name"] = self.extension_name().clone();
            data["action"] = json!(self.action());
            result.changed.push(
                json!({"infobase":true,"extension":self.extension_name(),"kind":self.action()}),
            );
        }
        result.data = Some(data);
        result.rev = Some(revision);
        Ok(result)
    }
}
fn validate_arguments(op: Operation, args: &Map<String, Value>) -> Result<(), &'static str> {
    let allowed: &[&str] = match op {
        Operation::List => &[],
        Operation::Delete => &["delete"],
        Operation::Activate => &["name", "active"],
    };
    if args.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("unsupported argument for runner 0.11 extension operation");
    }
    if op != Operation::List
        && !args
            .get(if op == Operation::Delete {
                "delete"
            } else {
                "name"
            })
            .and_then(Value::as_str)
            .is_some_and(|v| v.len() <= 256 && valid_1c_identifier(v))
    {
        return Err("extension name must be a bounded 1C identifier");
    }
    if op == Operation::Activate && !args.get("active").is_some_and(Value::is_boolean) {
        return Err("active must be boolean");
    }
    Ok(())
}
fn valid_inventory_record(item: &Value) -> bool {
    [
        "active",
        "safe_mode",
        "unsafe_action_protection",
        "used_in_distributed_infobase",
    ]
    .iter()
    .all(|k| item[k].is_boolean())
        && ["purpose", "scope", "hash_sum"]
            .iter()
            .all(|k| item[k].as_str().is_some_and(|v| v.len() <= 4096))
        && ["version", "security_profile_name"]
            .iter()
            .all(|k| item[k].is_null() || item[k].as_str().is_some_and(|v| v.len() <= 4096))
        && item.get("name_prefix").is_some_and(|prefix| {
            prefix.is_null() || prefix.as_str().is_some_and(|value| value.len() <= 4096)
        })
}
fn public_record(item: &Value) -> Value {
    json!({"name":item["name"],"version":item["version"],"namePrefix":item["name_prefix"],"purpose":item["purpose"],"active":item["active"],"safeMode":item["safe_mode"],"unsafeActionProtection":item["unsafe_action_protection"],"usedInDistributedInfobase":item["used_in_distributed_infobase"],"scope":item["scope"],"hashSum":item["hash_sum"],"securityProfileName":item["security_profile_name"]})
}
fn rejection(operation: Operation, code: RefusalCode, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(Some(operation.name().into()), code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::internal_adapters::ProcessOutput;
    use std::sync::Mutex;

    struct SequenceRunner {
        outputs: Mutex<std::collections::VecDeque<Value>>,
        calls: Mutex<Vec<ProcessCommand>>,
    }
    impl SequenceRunner {
        fn new(values: Vec<Value>) -> Self {
            Self {
                outputs: Mutex::new(values.into()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }
    impl ProcessRunner for SequenceRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.lock().unwrap().push(command.clone());
            let value = self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected provider call");
            Ok(ProcessOutput {
                status_success: value["ok"] == true,
                status: "fixture".into(),
                stdout: value.to_string(),
                stderr: "private stderr".into(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }
    fn fixture(root: &Path, op: Operation) -> PreparedExtensions {
        std::fs::write(
            root.join(CONFIG_NAME),
            "format: DESIGNER\ninfobase:\n  connection: 'File=base'\n",
        )
        .unwrap();
        let args = match op {
            Operation::List => json!({}),
            Operation::Activate => json!({"name":"Тест","active":false}),
            _ => json!({"delete":"Тест"}),
        };
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            json!({"op":op.name(),"args":args,"dryRun":true}),
            root.display().to_string(),
            7_000,
        )
        .unwrap();
        PreparedExtensions::parse(&request, op).unwrap()
    }
    fn tool(root: &Path) -> BundledTool {
        BundledTool {
            program: root.join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        }
    }
    fn record() -> Value {
        json!({"name":"Тест","version":null,"name_prefix":null,"purpose":"Patch","active":false,"safe_mode":true,"unsafe_action_protection":false,"used_in_distributed_infobase":false,"scope":"Infobase","hash_sum":"abc","security_profile_name":null})
    }
    fn envelope(prepared: &PreparedExtensions, preview: bool) -> Value {
        let mut data = json!({"ok":true,"provider_dispatched":!preview,"provider":{"selected":"ibcmd","origin":{"kind":"default"}}});
        if prepared.operation.reads() {
            data["requested"] = prepared.requested();
            data["extensions"] = if preview {
                json!([])
            } else {
                json!([record()])
            };
            data["plan"] = json!("PRIVATE CONNECTION AND EXECUTABLE");
        } else {
            data["steps"] = json!([{"target":"Тест","action":prepared.action(),"ok":true,"duration_ms":0,"message":"PRIVATE CONNECTION AND EXECUTABLE"}]);
        }
        json!({"ok":true,"command":"extensions","data":data})
    }
    #[test]
    fn delete_preview_explicitly_names_the_extension_data_loss() {
        let root = tempfile::tempdir().unwrap();
        let prepared = fixture(root.path(), Operation::Delete);
        let result = prepared.execute_with(
            &SequenceRunner::new(vec![envelope(&prepared, true)]),
            &tool(root.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        assert!(result.ok);
        assert_eq!(result.data.unwrap()["plan"]["deletesExtensionData"], true);
    }

    #[test]
    fn all_extension_operations_preview_then_apply_with_a_provider_receipt() {
        for op in [Operation::List, Operation::Delete, Operation::Activate] {
            let root = tempfile::tempdir().unwrap();
            let mut prepared = fixture(root.path(), op);
            let tool = tool(root.path());
            let preview = envelope(&prepared, true);
            let applied = envelope(&prepared, false);
            let runner = SequenceRunner::new(vec![preview.clone()]);
            let result = prepared.execute_with(
                &runner,
                &tool,
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            assert!(result.ok, "{result:?}");
            assert!(result.changed.is_empty());
            assert_eq!(runner.calls.lock().unwrap().len(), 1);
            assert!(runner.calls.lock().unwrap()[0]
                .args
                .contains(&"--dry-run".into()));
            assert!(!serde_json::to_string(&result).unwrap().contains("PRIVATE"));
            prepared.dry_run = false;
            prepared.if_rev = result.rev;
            let runner = SequenceRunner::new(vec![preview, applied]);
            let result = prepared.execute_with(
                &runner,
                &tool,
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            assert!(result.ok, "{op:?}: {result:?}");
            assert_eq!(result.changed.is_empty(), op.reads());
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert!(!calls[1].args.contains(&"--dry-run".into()));
            if op == Operation::Activate {
                assert!(calls[1].args.windows(2).any(|v| v == ["--active", "no"]));
            }
            if op.reads() {
                assert_eq!(
                    result.data.as_ref().unwrap()["extensions"][0]["safeMode"],
                    true
                );
            }
            assert!(!serde_json::to_string(&result).unwrap().contains("PRIVATE"));
        }
    }
    #[test]
    fn changed_args_config_version_or_provider_never_pass_the_extension_fence() {
        for change in ["args", "config", "local", "version", "provider"] {
            let root = tempfile::tempdir().unwrap();
            let mut p = fixture(root.path(), Operation::Activate);
            let tool = tool(root.path());
            let preview = envelope(&p, true);
            p.if_rev = p
                .execute_with(
                    &SequenceRunner::new(vec![preview.clone()]),
                    &tool,
                    super::super::runner_011::VERSION,
                    CancellationToken::new(),
                )
                .rev;
            p.dry_run = false;
            let mut new_preview = preview;
            match change {
                "args" => {
                    p.args.insert("active".into(), json!(true));
                    new_preview["data"]["steps"][0]["action"] = json!("activate");
                }
                "config" => std::fs::write(
                    root.path().join(CONFIG_NAME),
                    "format: DESIGNER\ninfobase:\n  connection: 'File=other'\n",
                )
                .unwrap(),
                "local" => std::fs::write(
                    root.path().join(LOCAL_CONFIG_NAME),
                    "infobase:\n  user: another\n",
                )
                .unwrap(),
                "provider" => new_preview["data"]["provider"]["selected"] = json!("agent"),
                _ => {}
            }
            let runner = SequenceRunner::new(vec![new_preview]);
            let version = if change == "version" {
                "0.11.1"
            } else {
                super::super::runner_011::VERSION
            };
            let result = p.execute_with(&runner, &tool, version, CancellationToken::new());
            assert!(!result.ok, "{change}");
            assert_eq!(result.diagnostics[0]["code"], "stale_revision");
            assert_eq!(runner.calls.lock().unwrap().len(), 1);
        }
    }
    #[test]
    fn extension_contract_rejects_wrong_subject_action_and_false_execution_claims() {
        let root = tempfile::tempdir().unwrap();
        for op in [Operation::List, Operation::Delete, Operation::Activate] {
            let p = fixture(root.path(), op);
            let mut wrong = envelope(&p, true);
            wrong["data"]["provider_dispatched"] = json!(true);
            assert!(p.validate(&wrong, true).is_err());
            let mut wrong = envelope(&p, false);
            wrong["data"]["provider_dispatched"] = json!(false);
            assert!(p.validate(&wrong, false).is_err());
            let mut wrong = envelope(&p, false);
            if op.reads() {
                wrong["data"]["requested"] = json!({"kind":"named","name":"Other"});
            } else {
                wrong["data"]["steps"][0]["action"] = json!("other");
            }
            assert!(p.validate(&wrong, false).is_err());
        }
        let p = fixture(root.path(), Operation::List);
        let mut wrong = envelope(&p, false);
        wrong["data"]["extensions"] = json!([record(), record()]);
        assert!(p.validate(&wrong, false).is_err());
        wrong = envelope(&p, false);
        wrong["data"]["extensions"][0]["active"] = json!("yes");
        assert!(p.validate(&wrong, false).is_err());
        wrong = envelope(&p, false);
        wrong["data"]["extensions"][0]
            .as_object_mut()
            .unwrap()
            .remove("name_prefix");
        assert!(p.validate(&wrong, false).is_err());
        wrong = envelope(&p, false);
        wrong["data"]["extensions"][0]["name_prefix"] = json!(42);
        assert!(p.validate(&wrong, false).is_err());
    }
    #[test]
    fn installed_prefix_keeps_known_value_empty_value_and_unknown_distinct() {
        for (prefix, available) in [
            (json!("A8_"), true),
            (json!(""), true),
            (Value::Null, false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut prepared = fixture(root.path(), Operation::List);
            let preview = envelope(&prepared, true);
            let mut applied = envelope(&prepared, false);
            applied["data"]["extensions"][0]["name_prefix"] = prefix.clone();
            let tool = tool(root.path());
            let preview_result = prepared.execute_with(
                &SequenceRunner::new(vec![preview.clone()]),
                &tool,
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            prepared.dry_run = false;
            prepared.if_rev = preview_result.rev;
            let result = prepared.execute_with(
                &SequenceRunner::new(vec![preview, applied]),
                &tool,
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            assert!(result.ok, "{result:?}");
            let data = result.data.unwrap();
            assert_eq!(data["extensions"][0]["namePrefix"], prefix);
            assert_eq!(data["namePrefixAvailable"], available);
        }
        for prefixes in [Vec::<Value>::new(), vec![json!("A8_"), Value::Null]] {
            let root = tempfile::tempdir().unwrap();
            let mut prepared = fixture(root.path(), Operation::List);
            let preview = envelope(&prepared, true);
            let mut applied = envelope(&prepared, false);
            applied["data"]["extensions"] = Value::Array(
                prefixes
                    .iter()
                    .enumerate()
                    .map(|(index, prefix)| {
                        let mut item = record();
                        item["name"] = json!(format!("Extension{index}"));
                        item["name_prefix"] = prefix.clone();
                        item
                    })
                    .collect(),
            );
            let tool = tool(root.path());
            let planned = prepared.execute_with(
                &SequenceRunner::new(vec![preview.clone()]),
                &tool,
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            prepared.dry_run = false;
            prepared.if_rev = planned.rev;
            let result = prepared.execute_with(
                &SequenceRunner::new(vec![preview, applied]),
                &tool,
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            assert!(result.ok, "{result:?}");
            let data = result.data.unwrap();
            assert_eq!(data["namePrefixAvailable"], false);
            assert_eq!(data["extensions"].as_array().unwrap().len(), prefixes.len());
        }
    }
    #[test]
    fn extension_arguments_are_closed_and_apply_requires_the_preview_revision() {
        let root = tempfile::tempdir().unwrap();
        for (op, args) in [
            (Operation::List, json!({"name":"X"})),
            (Operation::Activate, json!({"name":"X","active":"yes"})),
            (Operation::Delete, json!({"name":"X","all":true})),
        ] {
            assert!(validate_arguments(op, args.as_object().unwrap()).is_err());
        }
        for (dry, rev) in [(false, None), (true, Some("rev"))] {
            let req = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":"extensions.list","args":{},"dryRun":dry,"ifRev":rev}),
                root.path().display().to_string(),
                7_000,
            )
            .unwrap();
            assert!(PreparedExtensions::parse(&req, Operation::List).is_err());
        }
    }
    #[test]
    fn extension_revision_cannot_be_replayed_in_another_workspace() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let p1 = fixture(first.path(), Operation::Delete);
        let mut p2 = fixture(second.path(), Operation::Delete);
        let receipt = p1.execute_with(
            &SequenceRunner::new(vec![envelope(&p1, true)]),
            &tool(first.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        p2.dry_run = false;
        p2.if_rev = receipt.rev;
        let runner = SequenceRunner::new(vec![envelope(&p2, true), envelope(&p2, false)]);
        let result = p2.execute_with(
            &runner,
            &tool(second.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        assert!(
            !result.ok,
            "a relative File=base in a different workspace is a different infobase"
        );
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn extension_preview_names_the_effective_target_and_account_without_credentials() {
        let root = tempfile::tempdir().unwrap();
        let p = fixture(root.path(), Operation::List);
        std::fs::write(root.path().join(LOCAL_CONFIG_NAME),"infobase:\n  connection: 'Srvr=server;Ref=test;Pwd=hidden'\n  user: operator\n  password: another-hidden\n").unwrap();
        let result = p.execute_with(
            &SequenceRunner::new(vec![envelope(&p, true)]),
            &tool(root.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        assert!(result.ok);
        let plan = &result.data.as_ref().unwrap()["plan"];
        assert_eq!(plan["target"]["account"], "operator");
        assert_eq!(plan["target"]["connectionFrom"], LOCAL_CONFIG_NAME);
        assert!(plan["target"]["declaredConnection"]
            .as_str()
            .unwrap()
            .contains("Ref=test"));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("hidden"));
    }

    #[test]
    fn local_exchange_override_preserves_the_primary_standalone_target() {
        let root = tempfile::tempdir().unwrap();
        let p = fixture(root.path(), Operation::List);
        std::fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER\ninfobase:\n  standalone:\n    gate: localhost:1543\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join(LOCAL_CONFIG_NAME),
            "infobase:\n  standalone:\n    exchange: sftp\n",
        )
        .unwrap();
        assert_eq!(
            p.target_description().unwrap()["standaloneGate"],
            "localhost:1543"
        );
    }

    #[test]
    fn captured_runner_011_extension_envelopes_match_the_adapter() {
        let root = tempfile::tempdir().unwrap();
        for (op, label) in [
            (Operation::List, "list-final"),
            (Operation::Delete, "delete"),
            (Operation::Activate, "deactivate"),
        ] {
            let mut p = fixture(root.path(), op);
            if op != Operation::List {
                p.args.insert(
                    if op == Operation::Delete {
                        "delete"
                    } else {
                        "name"
                    }
                    .into(),
                    json!("UnicaRunnerProbe"),
                );
            }
            for preview in [true, false] {
                let phase = if preview { "preview" } else { "apply" };
                let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures/v8_runner_011")
                    .join(format!("{label}-{phase}.json"));
                let envelope: Value =
                    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                assert!(p.validate(&envelope, preview).is_ok(), "{op:?} {phase}");
            }
        }
    }

    #[test]
    fn cancelled_extension_request_never_starts_the_runner() {
        let root = tempfile::tempdir().unwrap();
        let p = fixture(root.path(), Operation::List);
        let runner = SequenceRunner::new(vec![]);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(
            !p.execute_with(
                &runner,
                &tool(root.path()),
                super::super::runner_011::VERSION,
                cancellation
            )
            .ok
        );
        assert!(runner.calls.lock().unwrap().is_empty());
    }
    #[test]
    fn extension_provider_failure_is_not_reported_as_a_change() {
        let root = tempfile::tempdir().unwrap();
        let p = fixture(root.path(), Operation::Delete);
        let runner = SequenceRunner::new(vec![
            json!({"ok":false,"command":"extensions","error":{"code":"environment_unavailable","message":"platform missing"}}),
        ]);
        let result = p.execute_with(
            &runner,
            &tool(root.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        assert!(!result.ok);
        assert!(result.changed.is_empty());
        assert_eq!(result.diagnostics[0]["detailCode"], "provider_absent");
    }

    #[test]
    fn only_mutating_extension_calls_detach_from_cancellation() {
        for operation in [Operation::List, Operation::Delete, Operation::Activate] {
            let root = tempfile::tempdir().unwrap();
            let prepared = fixture(root.path(), operation);
            let runner = SequenceRunner::new(vec![envelope(&prepared, false)]);
            let cancellation = CancellationToken::new();
            assert!(prepared
                .invoke(&runner, &tool(root.path()), &cancellation, false)
                .is_ok());
            let (_, child) = runner.calls.lock().unwrap()[0]
                .cancellation
                .spawn_with_gate(|| Ok(()))
                .unwrap();
            cancellation.cancel();
            assert_eq!(child.is_cancelled(), operation.reads());
            assert_eq!(cancellation.protected_process_started(), !operation.reads());
        }
    }
}
