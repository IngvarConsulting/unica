//! Explicit working/database configuration transitions through the compatible runner.
#![allow(clippy::result_large_err)]
use super::protocol::InvocationRequest;
use super::runner_011::Runner011ProcessRunner;
use super::v13_infobase_exports::{
    digest_optional_workspace_file, digest_required_workspace_file, resolve_bundled_runner,
    valid_1c_identifier,
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
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    Apply,
    Reset,
}
impl Transition {
    fn name(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Reset => "reset",
        }
    }
}
#[derive(Debug, Clone)]
pub(super) struct PreparedConfigurationTransition {
    operation: Transition,
    extension: Option<String>,
    dry_run: bool,
    if_rev: Option<String>,
    context: WorkspaceContext,
}
pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedConfigurationTransition>),
}
pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run {
        return Preparation::NotApplicable;
    }
    let operation = match request.arguments().get("op").and_then(Value::as_str) {
        Some("apply") => Transition::Apply,
        Some("reset") => Transition::Reset,
        _ => return Preparation::NotApplicable,
    };
    match PreparedConfigurationTransition::parse(request, operation) {
        Ok(value) => Preparation::Ready(Arc::new(value)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}
impl PreparedConfigurationTransition {
    fn parse(request: &InvocationRequest, operation: Transition) -> Result<Self, DomainResult> {
        let fail = |message| {
            DomainResult::canonical_rejection(
                Some(operation.name().into()),
                RefusalCode::BadValue,
                message,
            )
        };
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| fail("run args must be an object"))?;
        if args
            .keys()
            .any(|k| k != "extension" && !(operation == Transition::Reset && k == "force"))
        {
            return Err(fail("only extension and, for reset, force are supported"));
        }
        if operation == Transition::Reset && args.get("force") != Some(&Value::Bool(true)) {
            return Err(fail("reset requires force:true because the compatibility runner cannot distinguish your pending work from another session's work"));
        }
        let extension = match args.get("extension") {
            None => None,
            Some(Value::String(name)) if valid_1c_identifier(name) => Some(name.clone()),
            _ => return Err(fail("extension must name exactly one installed extension")),
        };
        let dry_run = arguments
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| fail("dryRun must be boolean"))?;
        let if_rev = match arguments.get("ifRev") {
            None => None,
            Some(Value::String(v)) if !v.trim().is_empty() => Some(v.clone()),
            _ => return Err(fail("ifRev must be non-empty text")),
        };
        if dry_run == if_rev.is_some() {
            return Err(fail(
                "preview takes no ifRev; execution requires its preview revision",
            ));
        }
        let context = discover_workspace(Some(PathBuf::from(request.workspace_hint())))
            .map_err(|_| fail("workspace discovery failed"))?;
        Ok(Self {
            operation,
            extension,
            dry_run,
            if_rev,
            context,
        })
    }
    fn fail(&self, code: RefusalCode, message: impl Into<String>) -> DomainResult {
        DomainResult::canonical_rejection(Some(self.operation.name().into()), code, message)
    }
    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hash = Sha256::new();
        hash.update(b"unica-configuration-transition-v1\0");
        hash.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hash.finalize().into())
    }
    fn inputs(&self) -> Result<Value, DomainResult> {
        let root = &self.context.workspace_root;
        let config = digest_required_workspace_file(root, Path::new("v8project.yaml"))
            .map_err(|e| self.fail(RefusalCode::InvalidState, e))?;
        let local = digest_optional_workspace_file(root, Path::new("v8project.local.yaml"))
            .map_err(|e| self.fail(RefusalCode::InvalidState, e))?;
        Ok(json!({"config":config,"local":local}))
    }
    fn args(&self) -> Value {
        let mut args = json!({});
        if let Some(extension) = &self.extension {
            args["extension"] = json!(extension);
        }
        if self.operation == Transition::Reset {
            args["force"] = json!(true);
        }
        args
    }
    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        match resolve_bundled_runner(&self.context.cwd) {
            Ok(resolved) => self.execute_with(
                &Runner011ProcessRunner,
                &resolved.tool,
                &resolved.version,
                cancellation,
            ),
            Err(message) => super::v13_infobase_exports::missing_runner_rejection(
                Some(self.operation.name().into()),
                message,
            ),
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
                "cancelled before starting the runner",
            ));
        }
        let mut args = vec![
            "--config".into(),
            self.context
                .workspace_root
                .join("v8project.yaml")
                .display()
                .to_string(),
            "--json-message".into(),
            self.operation.name().into(),
        ];
        if let Some(extension) = &self.extension {
            args.extend(["--extension".into(), extension.clone()]);
        }
        if self.operation == Transition::Reset {
            args.push("--force".into());
        }
        if preview {
            args.push("--dry-run".into());
        }
        let output = runner
            .run(&ProcessCommand {
                program: tool.program.clone(),
                args,
                cwd: self.context.workspace_root.clone(),
                env: vec![],
                env_remove: vec![],
                capture_limits: Some((1_048_576, 1_048_576)),
                timeout: None,
                // A started database mutation must finish; cancellation is checked between phases.
                cancellation: if preview {
                    cancellation.clone()
                } else {
                    cancellation.protect_process_on_spawn()
                },
            })
            .map_err(|error| {
                if error.starts_with(CANCELLED_PREFIX) {
                    return self.fail(RefusalCode::Cancelled, "cancelled before provider launch");
                }
                super::v13_infobase_exports::missing_runner_rejection(
                    Some(self.operation.name().into()),
                    redactor(&error),
                )
            })?;
        if output.stdout_truncated
            || output.stdout_had_invalid_utf8
            || output.timed_out
            || output.cancelled
        {
            return Err(self.fail(
                RefusalCode::InvalidResult,
                "runner did not return a complete terminal receipt; infobase effects are unknown",
            ));
        }
        let value: Value = serde_json::from_str(&output.stdout).map_err(|_| {
            self.fail(
                RefusalCode::InvalidResult,
                "runner returned invalid JSON; infobase effects are unknown",
            )
        })?;
        let data = &value["data"];
        if value["command"] != self.operation.name()
            || data["dry_run"] != preview
            || data["extension"] != json!(self.extension)
        {
            return Err(self.fail(
                RefusalCode::InvalidResult,
                "runner receipt does not match requested transition and target",
            ));
        }
        if !output.status_success || value["ok"] != true {
            let mut failure = self.fail(
                RefusalCode::InvalidState,
                "configuration transition failed; inspect the provider receipt before retrying",
            );
            failure.data = Some(
                json!({"op":self.operation.name(),"providerDispatched":data["provider_dispatched"],"completed":data["completed"],"infobaseEffectsKnown":data["provider_dispatched"] == false}),
            );
            return Err(failure);
        }
        if data["status"] != "succeeded"
            || data["provider_dispatched"] != !preview
            || data["completed"] != !preview
            || data["provider"]["selected"] != "designer"
        {
            return Err(self.fail(
                RefusalCode::InvalidResult,
                "runner transition receipt omitted execution or provider evidence",
            ));
        }
        super::v13_infobase_exports::validate_provider_receipt(&data["provider"])
            .map_err(|message| self.fail(RefusalCode::InvalidResult, message))?;
        Ok(value)
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
        let inputs = self.inputs()?;
        let preview = self.invoke(runner, tool, &cancellation, true)?;
        if inputs != self.inputs()? {
            return Err(self.fail(
                RefusalCode::ConcurrentChange,
                "configuration changed during preview",
            ));
        }
        let provider = &preview["data"]["provider"];
        let revision=format!("unica-configuration-transition-v1:{:x}",Sha256::digest(serde_json::to_vec(&json!({"workspace":self.workspace_identity_hash().as_str(),"op":self.operation.name(),"args":self.args(),"inputs":inputs,"version":version,"provider":provider})).expect("revision serializes")));
        if !self.dry_run && self.if_rev.as_deref() != Some(&revision) {
            return Err(self.fail(
                RefusalCode::StaleRevision,
                "transition plan changed; preview again",
            ));
        }
        let mut result = DomainResult::success(if self.dry_run {
            "configuration transition planned without changing the infobase"
        } else {
            "configuration transition completed; state is attested by the platform provider"
        });
        if self.dry_run {
            result.data = Some(
                json!({"op":self.operation.name(),"dryRun":true,"plan":{"infobase":"origin","args":self.args(),"provider":provider["selected"],"discardsPendingChanges":self.operation==Transition::Reset,"generationProtection":false,"extensionPresenceChecked":false,"databaseConfigurationUpdated":self.operation==Transition::Apply}}),
            );
            result.next.push(json!({"tool":"unica.run","args":{"op":self.operation.name(),"args":self.args(),"dryRun":false,"ifRev":revision},"reason":"execute exactly the previewed transition"}));
        } else {
            let applied = self.invoke(runner, tool, &cancellation, false)?;
            result.data = Some(
                json!({"op":self.operation.name(),"dryRun":false,"extension":self.extension,"provider":applied["data"]["provider"]["selected"],"targetStateAttestedBy":"provider","completed":true,"cancellationDeferred":cancellation.is_cancelled(),"interruption":public_interruption(&applied["data"]["interruption"])}),
            );
            result.changed.push(
                json!({"infobase":true,"extension":self.extension,"kind":self.operation.name()}),
            );
        }
        result.rev = Some(revision);
        Ok(result)
    }
}

fn public_interruption(value: &Value) -> Value {
    match (value["kind"].as_str(), value["deferred"].as_bool()) {
        (Some(kind @ ("timed_out" | "cancelled")), Some(deferred)) => {
            json!({"kind":kind,"deferred":deferred})
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::internal_adapters::ProcessOutput;
    use std::sync::Mutex;
    struct Probe {
        calls: Mutex<Vec<ProcessCommand>>,
        replies: Mutex<std::collections::VecDeque<Value>>,
    }
    impl Probe {
        fn new(replies: Vec<Value>) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                replies: Mutex::new(replies.into()),
            }
        }
    }
    impl ProcessRunner for Probe {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.lock().unwrap().push(command.clone());
            let v = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected process");
            Ok(ProcessOutput {
                status_success: v["ok"] == true,
                status: "fixture".into(),
                stdout: v.to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }
    fn fixture(
        root: &Path,
        operation: Transition,
        extension: Option<&str>,
    ) -> PreparedConfigurationTransition {
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\ninfobases:\n  origin:\n    connection: 'File=base'\n",
        )
        .unwrap();
        PreparedConfigurationTransition {
            operation,
            extension: extension.map(str::to_owned),
            dry_run: true,
            if_rev: None,
            context: WorkspaceContext {
                cwd: root.into(),
                workspace_root: root.into(),
                cache_root: root.join(".cache"),
                workspace_epoch: 1,
            },
        }
    }
    fn tool(root: &Path) -> BundledTool {
        BundledTool {
            program: root.join("runner"),
            warnings: vec![],
            missing: None,
        }
    }
    fn envelope(p: &PreparedConfigurationTransition, preview: bool) -> Value {
        json!({"ok":true,"command":p.operation.name(),"data":{"dry_run":preview,"extension":p.extension,"provider_dispatched":!preview,"completed":!preview,"provider":{"selected":"designer","origin":{"kind":"default"}},"status":"succeeded","interruption":null,"warnings":[]}})
    }
    #[test]
    fn runner_start_failure_identifies_the_absent_provider() {
        struct UnavailableRunner;
        impl ProcessRunner for UnavailableRunner {
            fn run(&self, _: &ProcessCommand) -> Result<ProcessOutput, String> {
                Err("runner could not start".into())
            }
        }
        for operation in [Transition::Apply, Transition::Reset] {
            let root = tempfile::tempdir().unwrap();
            let prepared = fixture(root.path(), operation, None);
            let result = prepared.execute_with(
                &UnavailableRunner,
                &tool(root.path()),
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            assert!(!result.ok);
            assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
            assert_eq!(result.diagnostics[0]["detailCode"], "provider_absent");
            assert!(result.rev.is_none());
            assert!(result.changed.is_empty());
        }
    }

    #[test]
    fn transitions_apply_only_the_previewed_target_and_never_lose_the_force_flag() {
        for op in [Transition::Apply, Transition::Reset] {
            for ext in [None, Some("Sales")] {
                let root = tempfile::tempdir().unwrap();
                let mut p = fixture(root.path(), op, ext);
                let preview = envelope(&p, true);
                let apply = envelope(&p, false);
                let result = p.execute_with(
                    &Probe::new(vec![preview.clone()]),
                    &tool(root.path()),
                    super::super::runner_011::VERSION,
                    CancellationToken::new(),
                );
                assert!(result.ok);
                assert!(result.changed.is_empty());
                p.if_rev = result.rev;
                p.dry_run = false;
                let runner = Probe::new(vec![preview, apply]);
                let result = p.execute_with(
                    &runner,
                    &tool(root.path()),
                    super::super::runner_011::VERSION,
                    CancellationToken::new(),
                );
                assert!(result.ok, "{result:?}");
                assert_eq!(result.changed.len(), 1);
                let calls = runner.calls.lock().unwrap();
                assert_eq!(calls.len(), 2);
                assert_eq!(
                    calls[1].args.contains(&"--force".into()),
                    op == Transition::Reset
                );
                assert_eq!(calls[1].args.contains(&"--extension".into()), ext.is_some());
            }
        }
    }
    #[test]
    fn transition_receipts_require_success_status_and_do_not_publish_raw_interruption_text() {
        let root = tempfile::tempdir().unwrap();
        let mut p = fixture(root.path(), Transition::Apply, None);
        let preview = envelope(&p, true);
        let planned = p.execute_with(
            &Probe::new(vec![preview.clone()]),
            &tool(root.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        p.if_rev = planned.rev;
        p.dry_run = false;
        let mut bad = envelope(&p, false);
        bad["data"]["status"] = json!("failed");
        let result = p.execute_with(
            &Probe::new(vec![preview.clone(), bad]),
            &tool(root.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        assert!(!result.ok, "failed status cannot be successful");
        let mut done = envelope(&p, false);
        done["data"]["interruption"] = json!({"kind":"timed_out","deferred":true,"message":"secret-connection-and-private-path"});
        let result = p.execute_with(
            &Probe::new(vec![preview, done]),
            &tool(root.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        assert!(result.ok);
        assert!(!serde_json::to_string(&result.data)
            .unwrap()
            .contains("secret-connection"));
        assert_eq!(result.data.unwrap()["interruption"]["kind"], "timed_out");
    }

    #[test]
    fn stale_transition_revision_stops_before_mutation() {
        let root = tempfile::tempdir().unwrap();
        let mut p = fixture(root.path(), Transition::Apply, None);
        p.dry_run = false;
        p.if_rev = Some("stale".into());
        let runner = Probe::new(vec![envelope(&p, true)]);
        let result = p.execute_with(
            &runner,
            &tool(root.path()),
            super::super::runner_011::VERSION,
            CancellationToken::new(),
        );
        assert!(!result.ok);
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }
    #[test]
    fn wrong_transition_target_or_false_dispatch_receipt_is_never_success() {
        let root = tempfile::tempdir().unwrap();
        let p = fixture(root.path(), Transition::Reset, Some("Sales"));
        for field in ["extension", "provider_dispatched", "completed"] {
            let mut v = envelope(&p, true);
            v["data"][field] = json!("wrong");
            let result = p.execute_with(
                &Probe::new(vec![v]),
                &tool(root.path()),
                super::super::runner_011::VERSION,
                CancellationToken::new(),
            );
            assert!(!result.ok, "{field}");
            assert!(result.rev.is_none());
        }
    }
    #[test]
    fn reset_requires_explicit_force_and_no_transition_accepts_source_replacement() {
        let root = tempfile::tempdir().unwrap();
        for (op, args) in [
            (Transition::Reset, json!({})),
            (Transition::Reset, json!({"force":false})),
            (Transition::Apply, json!({"force":true})),
            (Transition::Apply, json!({"sourceSet":"main"})),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":op.name(),"args":args,"dryRun":true}),
                root.path().display().to_string(),
                7000,
            )
            .unwrap();
            assert!(PreparedConfigurationTransition::parse(&request, op).is_err());
        }
    }
}
