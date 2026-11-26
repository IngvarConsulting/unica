//! A login probe uses the project's real configuration and a private workPath.
//! It does not prepare an exchange, acquire its workspace lock, or start a live agent.
use super::protocol::InvocationRequest;
use super::runner::{origin_kind, OriginKind};
use super::v13_infobase_exports::{
    missing_runner_rejection, resolve_bundled_runner, runner_start_rejection, CONFIG_NAME,
    RUNNER_OUTPUT_LIMIT,
};
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unica_execution::invocation_store::ToolIdentity;
use unica_execution_contract::invocation::{DomainResult, SafeIdentityHash};
use unica_process::runner::{BreakawayProcessRunner, ProcessCommand, ProcessOutput, ProcessRunner};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::paths::{normalize_path_identity, path_starts_with_host_root};
use unica_workspace::state_fs::{
    create_state_directory_child, open_state_directory_nofollow, verify_state_owner_only,
};

const OP: &str = "infobase.check";

pub(super) struct PreparedInfobaseCheck {
    context: WorkspaceContext,
    pub(super) dry_run: bool,
}

pub(super) fn prepare(
    request: &InvocationRequest,
) -> Option<Result<Arc<PreparedInfobaseCheck>, Box<DomainResult>>> {
    if request.tool() != ToolIdentity::Run || request.arguments().get("op")?.as_str()? != OP {
        return None;
    }
    let args = request.arguments();
    if !args
        .get("args")
        .and_then(Value::as_object)
        .is_some_and(|args| args.is_empty())
    {
        return Some(Err(Box::new(refuse(
            RefusalCode::BadValue,
            "infobase.check requires an empty args object",
        ))));
    }
    let Some(dry_run) = args.get("dryRun").and_then(Value::as_bool) else {
        return Some(Err(Box::new(refuse(
            RefusalCode::BadValue,
            "infobase.check requires explicit dryRun: true to preview or false to check login",
        ))));
    };
    Some(
        discover_workspace(Some(PathBuf::from(request.workspace_hint())))
            .map(|context| Arc::new(PreparedInfobaseCheck { context, dry_run }))
            .map_err(|_| {
                Box::new(refuse(
                    RefusalCode::InvalidState,
                    "the project for the login check could not be opened",
                ))
            }),
    )
}

impl PreparedInfobaseCheck {
    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hash = Sha256::new();
        hash.update(b"unica-v13-infobase-check-workspace-v1\0");
        hash.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hash.finalize().into())
    }

    pub(super) fn preview(&self) -> DomainResult {
        let mut result = DomainResult::success(
            "checking login to origin is planned; no platform session was started",
        );
        result.data = Some(json!({"infobase":"origin","dryRun":true}));
        result.next.push(json!({"tool":"unica.run","args":{"op":OP,"args":{},"dryRun":false},"reason":"check login to the configured origin infobase"}));
        result
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        match resolve_bundled_runner(&self.context.cwd) {
            Ok(resolved) => self.execute_with(
                &BreakawayProcessRunner,
                &resolved.tool.program,
                cancellation,
            ),
            Err(absence) => missing_runner_rejection(Some(OP.to_owned()), absence),
        }
    }

    fn execute_with(
        &self,
        runner: &dyn ProcessRunner,
        program: &Path,
        cancellation: CancellationToken,
    ) -> DomainResult {
        if cancellation.is_cancelled() {
            return refuse(
                RefusalCode::Cancelled,
                "infobase.check cancelled before login",
            );
        }
        let root = &self.context.workspace_root;
        match origin_kind(root) {
            Ok(OriginKind::Undeclared) => {
                return refuse(
                    RefusalCode::InvalidState,
                    "declare the origin infobase in the project before checking login",
                )
            }
            Err(_) => {
                return refuse(
                    RefusalCode::InvalidState,
                    "the project's infobase declaration could not be read",
                )
            }
            Ok(_) => {}
        }
        let scratch_error = || {
            refuse(
                RefusalCode::InvalidState,
                "a private work directory outside the project is required for the login check",
            )
        };
        let Ok(temp_root) = normalize_path_identity(&std::env::temp_dir()) else {
            return scratch_error();
        };
        let Ok(project_root) = normalize_path_identity(root) else {
            return scratch_error();
        };
        if path_starts_with_host_root(&temp_root, &project_root) {
            return scratch_error();
        }
        let Ok(scratch) = tempfile::Builder::new()
            .prefix("unica-infobase-check-")
            .tempdir_in(temp_root)
        else {
            return scratch_error();
        };
        let private = open_state_directory_nofollow(scratch.path())
            .and_then(|parent| create_state_directory_child(&parent, OsStr::new("work")))
            .and_then(|work| {
                verify_state_owner_only(&work)?;
                Ok(work)
            });
        let Ok(private) = private else {
            return scratch_error();
        };
        let command = ProcessCommand {
            program: program.to_owned(),
            args: vec![
                "--config".into(),
                root.join(CONFIG_NAME).display().to_string(),
                "--workdir".into(),
                scratch.path().join("work").display().to_string(),
                "--infobase".into(),
                "origin".into(),
                "--json-message".into(),
                "infobase".into(),
                "check".into(),
            ],
            cwd: root.clone(),
            env: vec![],
            env_remove: vec![],
            capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
            timeout: None,
            cancellation,
        };
        let output = runner.run_staged(&command);
        drop(private);
        if scratch.close().is_err() {
            return refuse(
                RefusalCode::ProviderFailed,
                "the temporary directory of the login check could not be removed",
            );
        }
        match output {
            Ok(output) => login_result(&output),
            Err(error) => runner_start_rejection(Some(OP.to_owned()), &error, program),
        }
    }
}

fn refuse(code: RefusalCode, summary: &str) -> DomainResult {
    DomainResult::canonical_rejection(Some(OP.to_owned()), code, summary)
}

fn login_result(output: &ProcessOutput) -> DomainResult {
    if output.cancelled {
        return refuse(RefusalCode::Cancelled, "the login check was cancelled");
    }
    if output.timed_out {
        return refuse(
            RefusalCode::DeadlineExceeded,
            "the login check exceeded its deadline",
        );
    }
    let invalid = || {
        refuse(
            RefusalCode::InvalidResult,
            "v8-runner did not return a valid login check result",
        )
    };
    if output.stdout_had_invalid_utf8 || output.stdout_truncated {
        return invalid();
    }
    let Ok(envelope) = serde_json::from_str::<Value>(&output.stdout) else {
        return invalid();
    };
    if envelope["command"] != OP || envelope["ok"].as_bool() != Some(output.status_success) {
        return invalid();
    }
    if !output.status_success {
        let Some(code) = envelope["error"]["code"].as_str() else {
            return invalid();
        };
        let mut result = match code {
            "credentials_rejected" => DomainResult::canonical_rejection_detailed(
                Some(OP.into()),
                RefusalDetail::CredentialsRejected,
                "the infobase rejected the login credentials",
            ),
            "infobase_designer_busy" => refuse(
                RefusalCode::ConcurrentChange,
                "another Designer session holds the infobase; retry after it closes",
            ),
            "infobase_not_found" => refuse(
                RefusalCode::NotFound,
                "the configured file infobase does not exist",
            ),
            "cancelled" => return refuse(RefusalCode::Cancelled, "the login check was cancelled"),
            "timed_out" => {
                return refuse(
                    RefusalCode::DeadlineExceeded,
                    "the login check exceeded its deadline",
                )
            }
            "invalid_output" => return invalid(),
            _ => {
                return refuse(
                    RefusalCode::ProviderFailed,
                    "v8-runner could not confirm login to the configured infobase",
                )
            }
        };
        result.data = Some(json!({"runnerCode":code}));
        return result;
    }
    if !envelope["error"].is_null() {
        return invalid();
    }
    let Some(version) = envelope["data"]["platform_version"].as_str() else {
        return invalid();
    };
    if version.split('.').count() != 4
        || !version
            .split('.')
            .all(|part| part.bytes().all(|c| c.is_ascii_digit()) && part.parse::<u32>().is_ok())
    {
        return invalid();
    }
    let mut result = DomainResult::success("login to the configured infobase succeeded");
    result.data = Some(json!({"platformVersion":version}));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    fn output(envelope: Value, success: bool) -> ProcessOutput {
        ProcessOutput {
            status_success: success,
            status: "test".into(),
            stdout: envelope.to_string(),
            stderr: "private-login-detail".into(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        }
    }

    struct Probe {
        envelope: Value,
        work_paths: Mutex<Vec<PathBuf>>,
    }

    impl ProcessRunner for Probe {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let args = &command.args;
            assert_eq!(args[0], "--config");
            assert_eq!(Path::new(&args[1]), command.cwd.join(CONFIG_NAME));
            assert_eq!(args[2], "--workdir");
            let work = PathBuf::from(&args[3]);
            assert!(!work.starts_with(&command.cwd));
            verify_state_owner_only(&open_state_directory_nofollow(&work).unwrap()).unwrap();
            // The real runner can leave its empty temp/ parent on both success and refusal.
            fs::create_dir(work.join("temp")).unwrap();
            assert_eq!(
                args[4..],
                [
                    "--infobase",
                    "origin",
                    "--json-message",
                    "infobase",
                    "check"
                ]
            );
            assert!(command.env.is_empty());
            self.work_paths.lock().unwrap().push(work);
            Ok(output(self.envelope.clone(), self.envelope["ok"] == true))
        }
    }

    fn request(root: &Path, args: Value) -> InvocationRequest {
        InvocationRequest::new(ToolIdentity::Run, args, root.to_str().unwrap(), 7000).unwrap()
    }

    #[test]
    fn file_and_server_probe_use_real_config_without_changing_workspace() {
        for connection in ["File=ib", "Srvr=localhost;Ref=demo"] {
            let project = tempfile::tempdir().unwrap();
            let root = project.path();
            let base =
                "format: DESIGNER\nsource-set: [{name: main, type: CONFIGURATION, path: src}]\n";
            let local = format!("infobases:\n  origin:\n    connection: '{connection}'\n    password: private-login-detail\n");
            fs::write(root.join(CONFIG_NAME), base).unwrap();
            fs::write(root.join("v8project.local.yaml"), &local).unwrap();
            for (ok, data) in [
                (true, json!({"platformVersion":"8.3.27.2342"})),
                (false, json!({"runnerCode":"credentials_rejected"})),
            ] {
                let call = request(root, json!({"op":OP,"args":{},"dryRun":false}));
                let check = prepare(&call).unwrap().unwrap();
                let probe = Probe {
                    envelope: if ok {
                        json!({"command":OP,"ok":true,"data":{"platform_version":"8.3.27.2342"}})
                    } else {
                        json!({"command":OP,"ok":false,"data":{},"error":{"code":"credentials_rejected","message":"private-login-detail"}})
                    },
                    work_paths: Mutex::new(Vec::new()),
                };
                let result = check.execute_with(
                    &probe,
                    Path::new("v8-runner"),
                    CancellationToken::default(),
                );
                assert_eq!(result.data, Some(data), "{result:?}");
                assert_eq!(result.ok, ok);
                let paths = probe.work_paths.lock().unwrap();
                assert_eq!(paths.len(), 1);
                assert!(!paths[0].parent().unwrap().exists());
                assert_eq!(fs::read_to_string(root.join(CONFIG_NAME)).unwrap(), base);
                assert_eq!(
                    fs::read_to_string(root.join("v8project.local.yaml")).unwrap(),
                    local
                );
                assert_eq!(fs::read_dir(root).unwrap().count(), 2);
            }
        }
    }

    #[test]
    fn reports_only_attested_reason_and_never_returns_provider_prose() {
        for (code, proved) in [
            ("credentials_rejected", true),
            ("infobase_designer_busy", true),
            ("infobase_not_found", true),
            ("connection_failed", false),
            ("private-login-detail", false),
        ] {
            let envelope = json!({"command":OP,"ok":false,"error":{"code":code,"message":"private-login-detail credentials_rejected"},"data":{"sessions":["private-login-detail"]},"warnings":["private-login-detail"]});
            let result = login_result(&output(envelope, false));
            assert!(!result.ok);
            assert_eq!(result.data, proved.then(|| json!({"runnerCode":code})));
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains("private-login-detail"));
        }
        let contradictory = json!({"command":OP,"ok":true,"data":{"platform_version":"8.3.27.2342"},"error":{"code":"credentials_rejected"}});
        assert!(!login_result(&output(contradictory, true)).ok);
        for (command, ok, status, version) in [
            ("pull", true, true, "8.3.27.2342"),
            (OP, true, false, "8.3.27.2342"),
            (OP, false, true, "8.3.27.2342"),
            (OP, true, true, "private-login-detail"),
            (OP, true, true, "8.3.27"),
        ] {
            let result = login_result(&output(
                json!({"command":command,"ok":ok,"data":{"platform_version":version}}),
                status,
            ));
            assert!(!result.ok);
            assert!(result.data.is_none());
        }
    }

    #[test]
    fn preview_makes_no_login_claim_and_binding_refuses_other_profiles_and_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let call = request(dir.path(), json!({"op":OP,"args":{},"dryRun":true}));
        let preview = prepare(&call).unwrap().unwrap().preview();
        assert!(preview.ok);
        assert_eq!(
            preview.data,
            Some(json!({"infobase":"origin","dryRun":true}))
        );
        assert_eq!(
            preview.next[0]["args"],
            json!({"op":OP,"args":{},"dryRun":false})
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        for args in [
            json!({"op":OP,"args":{"password":"private-login-detail"},"dryRun":false}),
            json!({"op":OP,"args":{}}),
        ] {
            assert!(prepare(&request(dir.path(), args)).unwrap().is_err());
        }
        let other = request(
            dir.path(),
            json!({"op":OP,"infobase":"other","args":{},"dryRun":false}),
        );
        assert!(
            !super::super::v13_run_dictionary::execute_run_dictionary(&other)
                .unwrap()
                .ok
        );
    }
}
