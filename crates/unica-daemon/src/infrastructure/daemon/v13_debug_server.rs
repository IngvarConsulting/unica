#![allow(clippy::result_large_err)]

use super::protocol::InvocationRequest;
use super::runner::{origin_kind, with_bundled_runner, OriginKind};
use super::v13_infobase_exports::{
    digest_required_workspace_file, missing_runner_rejection, resolve_bundled_runner,
    runner_envelope_refusal, runner_rejection, runner_start_rejection, CONFIG_NAME,
    RUNNER_OUTPUT_LIMIT,
};
use crate::application::v13::tool_catalog::RUNNER_SUPPORTS_DEBUG_SERVER_PREPARE;
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

mod prepare;

pub(super) const START: &str = "debug-server.start";
pub(super) const STOP: &str = "debug-server.stop";
pub(super) const PREPARE: &str = "debug-server.prepare";

#[derive(Debug)]
enum Request {
    Start { owner_pid: u32, port: Option<u16> },
    Stop { session: String },
    Prepare { owner_pid: u32 },
}

#[derive(Debug)]
pub(super) struct PreparedDebugServer {
    request: Request,
    context: WorkspaceContext,
    dry_run: bool,
}

impl PreparedDebugServer {
    pub(super) fn parse(
        invocation: &InvocationRequest,
        operation: &str,
    ) -> Result<Self, DomainResult> {
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
        let owner_pid = || {
            args.get("ownerPid")
                .and_then(positive_u32)
                .ok_or_else(|| reject("ownerPid must be a positive process ID"))
        };
        let request = match operation {
            PREPARE => {
                if args.keys().any(|key| key != "ownerPid") {
                    return Err(reject("debug-server.prepare accepts only ownerPid"));
                }
                Request::Prepare {
                    owner_pid: owner_pid()?,
                }
            }
            START => {
                if args
                    .keys()
                    .any(|key| !matches!(key.as_str(), "ownerPid" | "port"))
                {
                    return Err(reject("debug-server.start accepts only ownerPid and port"));
                }
                let port = args
                    .get("port")
                    .map(|value| {
                        positive_u32(value)
                            .and_then(|port| u16::try_from(port).ok())
                            .ok_or_else(|| reject("port must be between 1 and 65535"))
                    })
                    .transpose()?;
                Request::Start {
                    owner_pid: owner_pid()?,
                    port,
                }
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
            Request::Prepare { .. } => PREPARE,
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
        let tool = match resolve_bundled_runner(&self.context.cwd) {
            Ok(runner) => runner.tool,
            Err(error) => return missing_runner_rejection(Some(operation.into()), error),
        };
        if let Err(error) = digest_required_workspace_file(root, Path::new(CONFIG_NAME)) {
            return failure(operation, RefusalCode::InvalidState, error);
        }
        if matches!(self.request, Request::Start { .. })
            && !matches!(origin_kind(root), Ok(OriginKind::File))
        {
            return failure(
                operation,
                RefusalCode::BadValue,
                "debug server operations require a project file infobase",
            );
        }
        with_bundled_runner(
            root,
            &self.context.cwd,
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
                    Request::Prepare { owner_pid } => {
                        self.prepare(&tool, runner, &cancellation, *owner_pid)
                    }
                    Request::Start { owner_pid, port } => {
                        self.start(&tool, runner, &cancellation, *owner_pid, *port)
                    }
                    Request::Stop { session } => {
                        let result = self.stop(
                            &tool,
                            runner,
                            session,
                            self.dry_run,
                            RUNNER_SUPPORTS_DEBUG_SERVER_PREPARE,
                            &cancellation,
                        );
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

    fn start(
        &self,
        tool: &BundledTool,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
        owner_pid: u32,
        port: Option<u16>,
    ) -> DomainResult {
        let mut args = vec!["--owner-pid".to_owned(), owner_pid.to_string()];
        if let Some(port) = port {
            args.extend(["--port".to_owned(), port.to_string()]);
        }
        // Do not interrupt the runner between starting its supervised process
        // and returning the session token needed to stop it safely.
        let data = match self.invoke(tool, runner, START, args, self.dry_run, cancellation) {
            Ok(data) => data,
            Err(result) => return result,
        };
        let session = data["session"]
            .as_str()
            .filter(|session| valid_session(session));
        let owned = data["owner"] == "runner" && data["owner_pid"] == owner_pid;
        if self.dry_run
            && owned
            && data["action"] == "planned"
            && data["session"].is_null()
            && data["pid"].is_null()
        {
            let mut result = DomainResult::success("debug server start planned");
            result.data = Some(
                json!({"op":START,"contractVersion":1,"action":"planned","owner":"runner",
                "ownerPid":owner_pid,"providerDispatched":false}),
            );
            return result;
        }
        let actual_port = positive_u32(&data["port"]).and_then(|port| u16::try_from(port).ok());
        let url = actual_port.map(|port| format!("http://127.0.0.1:{port}"));
        let started_here = !self.dry_run
            && owned
            && data["action"] == "started"
            && positive_u32(&data["pid"]).is_some_and(|pid| pid != owner_pid);
        let ready = started_here
            && url.as_deref().is_some_and(|url| data["url"] == url)
            && port.is_none_or(|expected| actual_port == Some(expected))
            && data["infobase_alias"] == "DefAlias";
        if let Some(session) = session {
            if ready && cancellation.handoff_with_gate(|| Ok(())).is_ok() {
                let mut result = DomainResult::success("owned debug server is ready");
                result.data = Some(json!({"op":START,"contractVersion":1,"action":"started",
                    "owner":"runner","ownerPid":owner_pid,"pid":data["pid"],"port":actual_port,
                    "url":url,"infoBaseAlias":"DefAlias","session":session,"providerDispatched":true}));
                result
                    .changed
                    .push(json!({"debugServerSession":true,"action":"started"}));
                return result;
            }
            if started_here {
                let stopped = self.stop(
                    tool,
                    runner,
                    session,
                    false,
                    false,
                    &CancellationToken::new(),
                );
                if !stopped.ok {
                    let mut failure = failure(START, RefusalCode::ProviderFailed,
                        "debug server start was not handed off and its cleanup did not confirm release");
                    failure.data = Some(
                        json!({"op":START,"session":session,"owner":"runner","ownerPid":owner_pid}),
                    );
                    return failure;
                }
            }
        }
        if cancellation.is_cancelled() {
            failure(
                START,
                RefusalCode::Cancelled,
                "debug server start cancelled before handoff",
            )
        } else {
            failure(
                START,
                RefusalCode::InvalidResult,
                "runner did not confirm an owned ready debug server",
            )
        }
    }

    fn stop(
        &self,
        tool: &BundledTool,
        runner: &dyn ProcessRunner,
        session: &str,
        dry_run: bool,
        accepts_lease_release: bool,
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
        let owned_stop = action == Some("stopped")
            && !dry_run
            && data["owner"] == "runner"
            && data["known_session"] == true
            && (!accepts_lease_release || fresh)
            && positive_u32(&data["stopped_pid"]).is_some();
        let released = accepts_lease_release
            && !dry_run
            && action == Some("released")
            && data["known_session"] == true
            && data["stopped_pid"].is_null()
            && data.get("owner").is_none_or(|owner| owner == "runner")
            && fresh;
        let no_effect = (matches!(action, Some("nothing_to_stop"))
            || (dry_run && action == Some("planned")))
            && (!accepts_lease_release || data["stopped_pid"].is_null());
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
        if operation == PREPARE {
            command.extend(["--infobase".to_owned(), "origin".to_owned()]);
        }
        command.push("--json-message".to_owned());
        if dry_run && operation != PREPARE {
            command.push("--dry-run".to_owned());
        }
        command.extend(["debug-server".to_owned(), subcommand.to_owned()]);
        command.extend(args);
        if dry_run && operation == PREPARE {
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
        // alive after --ownerPID exits. This protected start already waits
        // for its receipt and cleans up if cancellation wins at handoff.
        let output = if matches!(operation, START | PREPARE) && !dry_run {
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
        if !output.status_success || envelope["ok"] != true {
            if operation == PREPARE {
                if envelope["ok"] != false || output.status_success {
                    return Err(invalid());
                }
                if let Some(
                    code @ ("debug_server_external_unavailable"
                    | "debug_server_external_target_unverified"),
                ) = envelope["error"]["code"].as_str()
                {
                    let message = if code == "debug_server_external_unavailable" {
                        "the configured debug server is unavailable"
                    } else {
                        "the configured debug server did not confirm the selected infobase"
                    };
                    let mut result = failure(PREPARE, RefusalCode::ProviderUnavailable, message);
                    result.data = Some(json!({"runnerCode":code}));
                    return Err(result);
                }
            }
            let message = match self.request {
                Request::Start {
                    port: Some(port), ..
                } if subcommand == "start"
                    && envelope["error"]["message"]
                        .as_str()
                        .is_some_and(|message| {
                            message.starts_with(&format!("port {port} is taken by "))
                        }) =>
                {
                    format!("debug server port {port} is occupied; choose a different port or omit port")
                }
                _ => format!("v8-runner refused {operation}"),
            };
            return Err(runner_envelope_refusal(
                runner_rejection(
                    Some(operation.into()),
                    envelope["error"]["code"]
                        .as_str()
                        .unwrap_or("provider_failed"),
                    envelope["error"]["layer"].as_str(),
                    message,
                ),
                &envelope,
            ));
        }
        if operation == PREPARE {
            if !envelope["error"].is_null()
                || !envelope["data"].is_object()
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
            if command
                .args
                .iter()
                .any(|arg| arg == "start" || arg == "prepare")
            {
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

    fn start_data() -> Value {
        envelope(
            "start",
            json!({
                "ok":true,"action":"started","owner":"runner","owner_pid":42,"pid":44,
                "session":SESSION,"port":12345,"url":"http://127.0.0.1:12345","infobase_alias":"DefAlias"
            }),
        )
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
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            json!({"op":START,"args":{"ownerPid":42}}),
            root.to_string_lossy(),
            7000,
        )
        .unwrap();
        let prepared = PreparedDebugServer::parse(&request, START).unwrap();
        let tool = BundledTool {
            program: root.join("v8-runner"),
            warnings: vec![],
            missing: None,
        };
        (prepared, tool)
    }

    mod prepare;

    #[test]
    fn debug_server_handoff_uses_reported_port_and_rolls_back_on_cancel() {
        let root = tempfile::tempdir().unwrap();
        let (prepared, tool) = fixture(root.path());
        let cancellation = CancellationToken::new();
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![start_data()]),
            cancel_during_reply: None,
        };
        let result = prepared.start(&tool, &runner, &cancellation, 42, None);
        assert_eq!(result.data.unwrap()["url"], "http://127.0.0.1:12345");
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1, "real start must avoid a Windows console group");
        drop(calls);
        let cancellation = CancellationToken::new();
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![start_data(), stop_data(SESSION)]),
            cancel_during_reply: Some(cancellation.clone()),
        };
        let result = prepared.start(&tool, &runner, &cancellation, 42, None);
        assert_eq!(result.diagnostics[0]["code"], "cancelled");
        let calls = runner.calls.borrow();
        assert!(
            calls[0].1,
            "cancel-at-handoff still used the protected start route"
        );
        assert!(!calls[1].1, "handoff cleanup keeps the existing stop route");
        assert!(calls[1].0.ends_with(&[
            "debug-server".to_owned(),
            "stop".to_owned(),
            SESSION.to_owned()
        ]));
    }

    #[test]
    fn debug_server_preview_keeps_the_existing_runner_route() {
        let root = tempfile::tempdir().unwrap();
        let (mut prepared, tool) = fixture(root.path());
        prepared.dry_run = true;
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![envelope(
                "start",
                json!({
                    "ok":true,"action":"planned","owner":"runner","owner_pid":42
                }),
            )]),
            cancel_during_reply: None,
        };
        let result = prepared.start(&tool, &runner, &CancellationToken::new(), 42, None);
        assert_eq!(result.data.unwrap()["action"], "planned");
        assert!(!runner.calls.borrow()[0].1);
    }

    #[test]
    fn debug_server_start_leaves_a_different_owner_session_alone() {
        let root = tempfile::tempdir().unwrap();
        let (prepared, tool) = fixture(root.path());
        let mut receipt = start_data();
        receipt["data"]["owner_pid"] = json!(43);
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![receipt, stop_data(SESSION)]),
            cancel_during_reply: None,
        };

        let result = prepared.start(&tool, &runner, &CancellationToken::new(), 42, None);

        assert_eq!(result.diagnostics[0]["code"], "invalid_result");
        assert_eq!(
            runner.calls.borrow().len(),
            1,
            "foreign session was stopped"
        );
    }

    #[test]
    fn debug_server_preview_does_not_stop_a_session_from_a_malformed_receipt() {
        let root = tempfile::tempdir().unwrap();
        let (mut prepared, tool) = fixture(root.path());
        prepared.dry_run = true;
        let mut receipt = start_data();
        receipt["data"]["action"] = json!("planned");
        receipt["data"]["pid"] = Value::Null;
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![receipt, stop_data(SESSION)]),
            cancel_during_reply: None,
        };

        let result = prepared.start(&tool, &runner, &CancellationToken::new(), 42, None);

        assert_eq!(result.diagnostics[0]["code"], "invalid_result");
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1, "preview dispatched a stop");
        assert!(calls[0].0.iter().any(|arg| arg == "--dry-run"));
    }

    #[test]
    fn a_session_in_a_nonstarted_receipt_is_never_released() {
        let root = tempfile::tempdir().unwrap();
        let (mut prepared, tool) = fixture(root.path());
        for (preview, action, session) in [
            (false, "planned", SESSION),
            (false, "reused", SESSION),
            (true, "planned", "invalid-token"),
        ] {
            prepared.dry_run = preview;
            let runner = Provider {
                calls: RefCell::default(),
                results: RefCell::new(vec![envelope(
                    "start",
                    json!({
                        "ok":true, "action":action, "owner":"runner", "owner_pid":42,
                        "session":session
                    }),
                )]),
                cancel_during_reply: None,
            };
            let result = prepared.start(&tool, &runner, &CancellationToken::new(), 42, None);
            assert_eq!(result.diagnostics[0]["code"], "invalid_result");
            assert_eq!(runner.calls.borrow().len(), 1);
        }
    }

    #[test]
    fn cancellation_never_releases_a_session_without_a_distinct_server_and_owner() {
        let root = tempfile::tempdir().unwrap();
        let (mut prepared, tool) = fixture(root.path());
        for (preview, key, value) in [
            (false, "pid", Value::Null),
            (false, "pid", json!("invalid")),
            (false, "pid", json!(42)),
            (false, "session", Value::Null),
            (false, "session", json!("invalid")),
            (false, "owner_pid", json!(43)),
            (false, "action", json!("planned")),
            (true, "action", json!("started")),
        ] {
            prepared.dry_run = preview;
            let cancellation = CancellationToken::new();
            let mut receipt = start_data();
            receipt["data"][key] = value;
            let runner = Provider {
                calls: RefCell::default(),
                results: RefCell::new(vec![receipt]),
                cancel_during_reply: Some(cancellation.clone()),
            };
            let result = prepared.start(&tool, &runner, &cancellation, 42, None);
            assert!(!result.ok);
            assert_eq!(runner.calls.borrow().len(), 1);
        }
    }

    #[test]
    fn debug_server_start_cleans_up_its_session_when_readiness_is_invalid() {
        let root = tempfile::tempdir().unwrap();
        let (prepared, tool) = fixture(root.path());
        let mut receipt = start_data();
        receipt["data"]["url"] = Value::Null;
        for (stop_session, expected_code) in [
            (SESSION, "invalid_result"),
            ("ffffffffffffffffffffffffffffffff", "provider_failed"),
        ] {
            let runner = Provider {
                calls: RefCell::default(),
                results: RefCell::new(vec![receipt.clone(), stop_data(stop_session)]),
                cancel_during_reply: None,
            };

            let result = prepared.start(&tool, &runner, &CancellationToken::new(), 42, None);

            assert_eq!(result.diagnostics[0]["code"], expected_code);
            let calls = runner.calls.borrow();
            assert_eq!(calls.len(), 2);
            assert!(calls[1].0.ends_with(&[
                "debug-server".to_owned(),
                "stop".to_owned(),
                SESSION.to_owned()
            ]));
            assert!(!calls[1].0.iter().any(|arg| arg == "--dry-run"));
            if stop_session != SESSION {
                let data = result.data.unwrap();
                assert_eq!(data["session"], SESSION);
                assert_eq!(data["ownerPid"], 42);
            }
        }
    }

    #[test]
    fn debug_server_stop_refuses_a_receipt_for_a_different_session() {
        let root = tempfile::tempdir().unwrap();
        let (prepared, tool) = fixture(root.path());
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![stop_data("ffffffffffffffffffffffffffffffff")]),
            cancel_during_reply: None,
        };
        let result = prepared.stop(
            &tool,
            &runner,
            SESSION,
            false,
            false,
            &CancellationToken::new(),
        );
        assert_eq!(result.diagnostics[0]["code"], "invalid_result");
        assert!(!runner.calls.borrow()[0].1);
    }
}
