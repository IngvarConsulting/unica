#![allow(clippy::result_large_err)]

use super::protocol::InvocationRequest;
use super::runner::with_resolved_runner;
use super::v13_infobase_exports::{
    digest_required_workspace_file, runner_envelope_refusal, runner_rejection,
    runner_start_rejection, CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
};
use super::v13_run_dictionary::{debug_server_declared, installed_debug_server_runner};
use crate::application::v13::tool_catalog::DEBUG_SERVER_START_CONTRACT_VERSION;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use unica_artifact_delivery::engine::BundledTool;
use unica_execution_contract::invocation::{DomainResult, SafeIdentityHash};
use unica_process::runner::{ProcessCommand, ProcessRunner};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;

mod start;

pub(super) const START: &str = "debug-server.start";
pub(super) const STOP: &str = "debug-server.stop";

#[derive(Debug)]
enum Request {
    Start { owner_pid: u32 },
    Stop { session: String },
}

#[derive(Debug)]
pub(super) struct PreparedDebugServer {
    request: Request,
    context: WorkspaceContext,
    dry_run: bool,
}

impl Request {
    fn parse(
        invocation: &InvocationRequest,
        operation: &str,
    ) -> Result<(Self, bool), DomainResult> {
        let reject = |message| failure(operation, RefusalCode::BadValue, message);
        let arguments = invocation.arguments();
        if arguments.contains_key("ifRev") {
            return Err(reject("a debug server operation does not accept ifRev"));
        }
        let dry_run = match arguments.get("dryRun") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(reject("dryRun must be boolean")),
        };
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject("run args must be an object"))?;
        let request = match operation {
            START => {
                if args.keys().any(|key| key != "ownerPid") {
                    return Err(reject("debug-server.start accepts only ownerPid"));
                }
                let owner_pid = args
                    .get("ownerPid")
                    .and_then(positive_u32)
                    .ok_or_else(|| reject("ownerPid must be a positive process ID"))?;
                Request::Start { owner_pid }
            }
            STOP => {
                if args.len() != 1 {
                    return Err(reject("debug-server.stop accepts only session"));
                }
                let session = args
                    .get("session")
                    .and_then(Value::as_str)
                    .filter(|s| valid_session(s))
                    .ok_or_else(|| {
                        reject("session must be the lowercase hexadecimal token from start")
                    })?;
                Request::Stop {
                    session: session.to_owned(),
                }
            }
            _ => return Err(reject("unknown debug server operation")),
        };
        Ok((request, dry_run))
    }
}

impl PreparedDebugServer {
    pub(super) fn parse(
        invocation: &InvocationRequest,
        operation: &str,
    ) -> Result<Self, DomainResult> {
        if !debug_server_declared(Path::new(invocation.workspace_hint())) {
            return Err(failure(
                operation,
                RefusalCode::UnsupportedOperation,
                "the v8-runner package does not provide debug-server.start contract version 2",
            ));
        }
        let (request, dry_run) = Request::parse(invocation, operation)?;
        let context = discover_workspace(Some(PathBuf::from(invocation.workspace_hint())))
            .map_err(|_| {
                DomainResult::canonical_rejection_detailed(
                    Some(operation.into()),
                    RefusalDetail::ProviderAbsent,
                    "cannot discover debug server workspace",
                )
            })?;
        Ok(Self {
            request,
            context,
            dry_run,
        })
    }

    fn operation(&self) -> &'static str {
        match self.request {
            Request::Start { .. } => START,
            Request::Stop { .. } => STOP,
        }
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hash = Sha256::new();
        hash.update(b"unica-v13-debug-server-workspace-v1\0");
        hash.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hash.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        let operation = self.operation();
        let root = &self.context.workspace_root;
        // Standard delivery has finished. Verify the binary after admission, including
        // a package that was replaced while the request waited for its workspace.
        let Some(tool) = installed_debug_server_runner(&self.context.cwd) else {
            return failure(
                operation,
                RefusalCode::UnsupportedOperation,
                "the installed v8-runner does not provide debug-server.start contract version 2",
            );
        };
        if let Err(error) = digest_required_workspace_file(root, Path::new(CONFIG_NAME)) {
            return failure(operation, RefusalCode::InvalidState, error);
        }
        with_resolved_runner(
            root,
            &tool.program,
            operation,
            cancellation,
            |runner, cancellation| {
                if cancellation.is_cancelled() {
                    return failure(
                        operation,
                        RefusalCode::Cancelled,
                        "debug server operation cancelled before dispatch",
                    );
                }
                match &self.request {
                    Request::Start { owner_pid } => {
                        self.start(&tool, runner, &cancellation, *owner_pid)
                    }
                    Request::Stop { session } => {
                        let result = self.stop(&tool, runner, session, self.dry_run, &cancellation);
                        if !self.dry_run && result.ok {
                            // A returned stop receipt is factual even if cancellation raced the stop.
                            cancellation.record_answered_interrupt();
                        }
                        result
                    }
                }
            },
        )
    }

    fn stop(
        &self,
        tool: &BundledTool,
        runner: &dyn ProcessRunner,
        session: &str,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let data = match self.invoke(
            tool,
            runner,
            STOP,
            vec![session.to_owned()],
            dry_run,
            cancellation,
        ) {
            Ok(data) => data,
            Err(result) => return result,
        };
        let action = data["action"].as_str();
        let fresh = data.get("stale_record").is_none_or(|stale| stale == false);
        let stale = data["stale_record"] == true;
        let known = data["known_session"] == true;
        let owned_stop = action == Some("stopped")
            && !dry_run
            && data["owner"] == "runner"
            && data["known_session"] == true
            && fresh
            && positive_u32(&data["stopped_pid"]).is_some();
        let released = !dry_run
            && action == Some("released")
            && data["known_session"] == true
            && data["stopped_pid"].is_null()
            && data.get("owner").is_none_or(|owner| owner == "runner")
            && fresh;
        let no_effect = data["stopped_pid"].is_null()
            && (fresh || stale)
            && match action {
                Some("nothing_to_stop") => known == stale,
                Some("planned") => dry_run && (known || fresh),
                _ => false,
            };
        if data["session"] != session
            || !data["known_session"].is_boolean()
            || (!owned_stop && !released && !no_effect)
        {
            return failure(
                STOP,
                RefusalCode::InvalidResult,
                "runner did not confirm the debug server release outcome",
            );
        }
        let mut result = DomainResult::success("debug server release resolved");
        result.data = Some(
            json!({"op":STOP,"contractVersion":1,"action":action,"session":session,
            "knownSession":data["known_session"],"staleRecord":data["stale_record"] == true,
            "providerDispatched":!dry_run}),
        );
        if owned_stop {
            result
                .changed
                .push(json!({"debugServerSession":true,"action":"stopped"}));
        }
        result
    }

    fn invoke(
        &self,
        tool: &BundledTool,
        runner: &dyn ProcessRunner,
        operation: &'static str,
        args: Vec<String>,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<Value, DomainResult> {
        let subcommand = operation
            .strip_prefix("debug-server.")
            .expect("debug server operation");
        let mut command = vec![
            "--config".to_owned(),
            self.context
                .workspace_root
                .join(CONFIG_NAME)
                .display()
                .to_string(),
        ];
        if operation == START {
            command.extend(["--infobase".to_owned(), "origin".to_owned()]);
        }
        command.push("--json-message".to_owned());
        if dry_run && operation != START {
            command.push("--dry-run".to_owned());
        }
        command.extend(["debug-server".to_owned(), subcommand.to_owned()]);
        command.extend(args);
        if dry_run && operation == START {
            command.push("--dry-run".to_owned());
        }
        let command = ProcessCommand {
            program: tool.program.clone(),
            args: command,
            cwd: self.context.workspace_root.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
            timeout: None,
            cancellation: cancellation.linked_child().protect_process_on_spawn(),
        };
        // On Windows a console process group on the launcher can keep dbgs
        // alive after the owner exits. This protected start already waits
        // for its receipt and cleans up if cancellation wins at handoff.
        let output = if operation == START && !dry_run {
            runner.run_staged_without_windows_interrupt(&command)
        } else {
            runner.run_staged(&command)
        }
        .map_err(|error| runner_start_rejection(Some(operation.into()), &error, &tool.program))?;
        let invalid = || {
            failure(
                operation,
                RefusalCode::InvalidResult,
                "invalid v8-runner debug server receipt",
            )
        };
        if output.stdout_truncated || output.stdout_had_invalid_utf8 {
            return Err(invalid());
        }
        let envelope: Value = serde_json::from_str(&output.stdout).map_err(|_| invalid())?;
        if envelope["command"] != format!("debug-server {subcommand}") {
            return Err(invalid());
        }
        let Some(ok) = envelope["ok"].as_bool() else {
            return Err(invalid());
        };
        if ok != output.status_success || (ok && !envelope["error"].is_null()) {
            return Err(invalid());
        }
        if !ok {
            if operation == START {
                if let Some(
                    code @ ("debug_server_external_unavailable"
                    | "debug_server_external_target_unverified"),
                ) = envelope["error"]["code"].as_str()
                {
                    let (detail, message) = if code == "debug_server_external_unavailable" {
                        (
                            RefusalDetail::EndpointUnreachable,
                            "the configured debug server is unavailable",
                        )
                    } else {
                        (
                            RefusalDetail::EndpointTargetUnverified,
                            "the configured debug server did not confirm the selected infobase",
                        )
                    };
                    let mut result = DomainResult::canonical_rejection_detailed(
                        Some(START.into()),
                        detail,
                        message,
                    );
                    result.data = Some(json!({"runnerCode":code}));
                    return Err(result);
                }
            }
            return Err(runner_envelope_refusal(
                runner_rejection(
                    Some(operation.into()),
                    envelope["error"]["code"]
                        .as_str()
                        .unwrap_or("provider_failed"),
                    envelope["error"]["layer"].as_str(),
                    format!("v8-runner refused {operation}"),
                ),
                &envelope,
            ));
        }
        if operation == START {
            if !envelope["data"].is_object()
                || envelope["data"].get("ok").is_some_and(|ok| ok != true)
            {
                return Err(invalid());
            }
        } else if envelope["data"]["ok"] != true {
            return Err(invalid());
        }
        Ok(envelope["data"].clone())
    }
}

fn valid_session(session: &str) -> bool {
    session.len() == 32
        && session
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn positive_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value != 0)
}

fn failure(operation: &str, code: RefusalCode, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(Some(operation.into()), code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use unica_execution::invocation_store::ToolIdentity;
    use unica_process::runner::{ProcessOutput, ProcessStreamFailure};

    const SESSION: &str = "3640f748f9c24f728aebf81fbe891cb4";

    struct Provider {
        calls: RefCell<Vec<(Vec<String>, bool)>>,
        results: RefCell<Vec<Value>>,
        cancel_during_reply: Option<CancellationToken>,
    }

    impl Provider {
        fn reply(
            &self,
            command: &ProcessCommand,
            without_windows_interrupt: bool,
        ) -> ProcessOutput {
            self.calls
                .borrow_mut()
                .push((command.args.clone(), without_windows_interrupt));
            let value = self.results.borrow_mut().remove(0);
            if command.args.iter().any(|arg| arg == "start") {
                if let Some(cancellation) = &self.cancel_during_reply {
                    cancellation.cancel();
                }
            }
            ProcessOutput {
                status_success: value["ok"] == true,
                status: "provider fixture".into(),
                stdout: value.to_string(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
                timed_out: false,
                cancelled: false,
            }
        }
    }

    impl ProcessRunner for Provider {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            Ok(self.reply(command, false))
        }

        fn run_staged_without_windows_interrupt(
            &self,
            command: &ProcessCommand,
        ) -> Result<ProcessOutput, ProcessStreamFailure> {
            Ok(self.reply(command, true))
        }
    }

    fn envelope(command: &str, data: Value) -> Value {
        json!({"command": format!("debug-server {command}"), "ok": true, "data": data})
    }

    fn stop_data(session: &str) -> Value {
        envelope(
            "stop",
            json!({
                "ok":true,"action":"stopped","owner":"runner","session":session,
                "known_session":true,"stopped_pid":44
            }),
        )
    }

    fn fixture(root: &Path) -> (PreparedDebugServer, BundledTool) {
        let tool = BundledTool {
            program: root.join("v8-runner"),
            warnings: vec![],
            missing: None,
        };
        let request = PreparedDebugServer {
            request: Request::Start { owner_pid: 42 },
            context: discover_workspace(Some(root.to_owned())).unwrap(),
            dry_run: false,
        };
        (request, tool)
    }

    mod start;

    #[test]
    fn debug_server_stop_refuses_a_receipt_for_a_different_session() {
        let root = tempfile::tempdir().unwrap();
        let (prepared, tool) = fixture(root.path());
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![stop_data("ffffffffffffffffffffffffffffffff")]),
            cancel_during_reply: None,
        };
        let result = prepared.stop(&tool, &runner, SESSION, false, &CancellationToken::new());
        assert_eq!(result.diagnostics[0]["code"], "invalid_result");
        assert!(!runner.calls.borrow()[0].1);
    }
}
