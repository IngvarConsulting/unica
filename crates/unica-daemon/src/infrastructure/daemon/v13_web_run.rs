#![allow(clippy::result_large_err)]
//! Safe projections of the runner's web publication and browser receipts.

use super::protocol::InvocationRequest;
use super::v13_infobase_exports::{
    digest_required_workspace_file, missing_runner_rejection, resolve_bundled_runner,
    runner_envelope_refusal, runner_rejection, runner_start_rejection, CONFIG_NAME,
    RUNNER_OUTPUT_LIMIT,
};
use crate::infrastructure::workspace::discover_workspace;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_process::runner::{ProcessCommand, ProcessRunner};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;

use serde_json::{json, Value};
use unica_execution_contract::invocation::DomainResult;
use unica_refusal::RefusalCode;
use url::Url;

fn debugger_endpoint(raw: &str) -> Option<String> {
    let (scheme, authority) = raw.split_once("://")?;
    let authority = authority.strip_suffix('/').unwrap_or(authority);
    let (host, port) = authority.rsplit_once(':')?;
    if !scheme.eq_ignore_ascii_case("http")
        || host.is_empty()
        || authority.contains(['/', '?', '#', '@', '\\'])
        || authority.chars().any(char::is_whitespace)
        || port.is_empty()
        || !port.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
    let uri = Url::parse(raw).ok()?;
    Some(format!("http://{}:{port}", uri.host_str()?))
}

fn publication_url(value: &Value) -> Option<Url> {
    let raw = value.as_str()?;
    let url = Url::parse(raw).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

fn failed(operation: &str, code: RefusalCode, message: &str) -> DomainResult {
    DomainResult::canonical_rejection(Some(operation.to_owned()), code, message)
}

fn invalid(operation: &str) -> DomainResult {
    failed(
        operation,
        RefusalCode::InvalidResult,
        "inconsistent v8-runner web receipt",
    )
}

fn prepare_receipt(
    operation: &str,
    data: &Value,
    ide_url: &str,
    dry_run: bool,
    succeeded: bool,
) -> DomainResult {
    let debug = &data["debug"];
    if data["action"] != "publish"
        || data["ok"] != succeeded
        || debug["ide_url"]
            .as_str()
            .and_then(debugger_endpoint)
            .as_deref()
            != Some(ide_url)
    {
        return invalid(operation);
    }
    let (Some(state), Some(changed), Some(dispatched)) = (
        debug["state"].as_str(),
        debug["publication_changed"].as_bool(),
        data["provider_dispatched"].as_bool(),
    ) else {
        return invalid(operation);
    };
    if !succeeded && state == "route_unavailable" && !changed {
        let mut result = failed(operation, RefusalCode::ProviderUnavailable,
            "Check infobase.web.url and the active publication. Configure infobase.web.debugger-url to reach the HTTP debugger from the web server, enable it in the publication descriptor and reload that publication. For standalone, enable debugging on the server.");
        result.data = Some(
            json!({"op":operation,"state":state,"runnerCode":state,"publicationChanged":false}),
        );
        return result;
    }
    let state_matches = match state {
        "planned" => dry_run && !changed && !dispatched,
        "external_route_verified" => !dry_run && !changed && !dispatched,
        "ready" => !dry_run,
        _ => false,
    };
    let debugger = debug["debugger_url"].as_str().and_then(debugger_endpoint);
    let page = publication_url(&data["url"]);
    if !succeeded || !state_matches || page.is_none() || debugger.is_none() {
        return invalid(operation);
    }
    let mut result = DomainResult::success("web publication debug route checked");
    result.data = Some(json!({"op":operation,"state":state,"ideUrl":ide_url,
        "debuggerUrl":debugger,"url":data["url"],"publicationChanged":changed,"providerDispatched":dispatched}));
    if changed {
        result.changed.push(json!({"webPublication":true}));
    }
    result
}

fn launch_receipt(data: &Value, requested_debugger: Option<&str>, dry_run: bool) -> DomainResult {
    let Some(page) = publication_url(&data["url"]) else {
        return invalid("launch");
    };
    if data["ok"] != true
        || data["mode"] != "web"
        || data["via"] != "web"
        || data["provider_dispatched"] != !dry_run
        || (dry_run && !data["pid"].is_null())
    {
        return invalid("launch");
    }
    if let Some(debugger) = requested_debugger {
        let keys = |name: &str| {
            page.query_pairs()
                .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.into_owned())
                .collect::<Vec<_>>()
        };
        if data["debug"]["url"]
            .as_str()
            .and_then(debugger_endpoint)
            .as_deref()
            != Some(debugger)
            || keys("Debug") != ["http,attach"]
            || keys("DebuggerURL") != [debugger]
        {
            return invalid("launch");
        }
    } else if !data["debug"].is_null() {
        return invalid("launch");
    }
    let mut result = DomainResult::success(if dry_run {
        "web browser launch planned"
    } else {
        "URL dispatched to the system browser"
    });
    result.data = Some(json!({"op":"launch","clientMode":"web","url":data["url"],
        "providerDispatched":!dry_run,"debuggerUrl":requested_debugger}));
    if !dry_run {
        result
            .changed
            .push(json!({"clientSession":true,"mode":"web"}));
    }
    result
}

pub(super) const PUBLISH: &str = "web.publish";

#[derive(Debug)]
enum WebRequest {
    Prepare(String),
    Launch(Option<String>),
}

#[derive(Debug)]
pub(super) struct PreparedWebRun {
    request: WebRequest,
    context: WorkspaceContext,
    dry_run: bool,
}

impl PreparedWebRun {
    pub(super) fn parse(
        invocation: &InvocationRequest,
        operation: &str,
    ) -> Result<Self, DomainResult> {
        let reject = |message| failed(operation, RefusalCode::BadValue, message);
        let input = invocation.arguments();
        if input.contains_key("ifRev") {
            return Err(reject("run does not accept ifRev"));
        }
        let dry_run = match input.get("dryRun") {
            Some(Value::Bool(value)) => *value,
            None if operation == "launch" => false,
            _ => return Err(reject("dryRun must be boolean")),
        };
        let args = input
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject("args must be an object"))?;
        let endpoint = |name| {
            args.get(name).and_then(Value::as_str).and_then(debugger_endpoint)
            .ok_or_else(||reject("expected an HTTP debugger address with an explicit port, without paths or credentials"))
        };
        let request = if operation == PUBLISH {
            if args.len() != 1 {
                return Err(reject("web.publish accepts only prepareDebug"));
            }
            let ide_url = endpoint("prepareDebug")?;
            if !ide_url.starts_with("http://127.0.0.1:")
                && !ide_url.starts_with("http://localhost:")
            {
                return Err(reject(
                    "the IDE debugger endpoint must be local to this DAP adapter",
                ));
            }
            WebRequest::Prepare(ide_url)
        } else {
            if args.get("clientMode") != Some(&json!("web"))
                || args
                    .keys()
                    .any(|k| !matches!(k.as_str(), "clientMode" | "webDebugUrl"))
            {
                return Err(reject("web launch accepts only clientMode and webDebugUrl"));
            }
            WebRequest::Launch(
                args.contains_key("webDebugUrl")
                    .then(|| endpoint("webDebugUrl"))
                    .transpose()?,
            )
        };
        let context = discover_workspace(Some(PathBuf::from(invocation.workspace_hint())))
            .map_err(|_| {
                failed(
                    operation,
                    RefusalCode::ProviderUnavailable,
                    "cannot discover the web publication workspace",
                )
            })?;
        Ok(Self {
            request,
            context,
            dry_run,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hash = Sha256::new();
        hash.update(b"unica-v13-web-run-workspace-v1\0");
        hash.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hash.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        let root = &self.context.workspace_root;
        let (op, wire, tail) = match &self.request {
            WebRequest::Prepare(endpoint) => (
                PUBLISH,
                "publish",
                vec!["web", "publish", "--prepare-debug", endpoint.as_str()],
            ),
            WebRequest::Launch(debug) => {
                let mut tail = vec!["launch", "web"];
                if let Some(endpoint) = debug {
                    tail.extend(["--web-debug", endpoint]);
                }
                ("launch", "launch", tail)
            }
        };
        let tool = match resolve_bundled_runner(&self.context.cwd) {
            Ok(runner) => runner.tool,
            Err(error) => return missing_runner_rejection(Some(op.into()), error),
        };
        if digest_required_workspace_file(root, Path::new(CONFIG_NAME)).is_err() {
            return failed(
                op,
                RefusalCode::InvalidState,
                "the web publication requires a project configuration",
            );
        }
        super::runner::with_bundled_runner(
            root,
            &self.context.cwd,
            op,
            cancellation,
            |runner, cancellation| {
                let mut args = vec![
                    "--config".to_owned(),
                    root.join(CONFIG_NAME).display().to_string(),
                    "--json-message".into(),
                ];
                if self.dry_run {
                    args.push("--dry-run".into());
                }
                args.extend(tail.iter().map(|s| s.to_string()));
                let owns_client = matches!(self.request, WebRequest::Launch(_)) && !self.dry_run;
                let command = ProcessCommand {
                    program: tool.program.clone(),
                    args,
                    cwd: root.clone(),
                    env: if owns_client {
                        vec![("V8_RUNNER_CLIENT_OWNER".into(), "unica".into())]
                    } else {
                        vec![]
                    },
                    env_remove: vec!["V8_RUNNER_CLIENT_OWNER".into()],
                    capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
                    timeout: None,
                    cancellation: cancellation.clone(),
                };
                let output = if owns_client {
                    runner
                        .run_pending_handoff_staged(&command)
                        .map(|(output, guard)| (output, Some(guard)))
                } else {
                    runner.run_staged(&command).map(|output| (output, None))
                };
                let (output, mut guard) = match output {
                    Ok(output) => output,
                    Err(error) => {
                        return runner_start_rejection(Some(op.into()), &error, &tool.program)
                    }
                };
                if output.cancelled {
                    return failed(op, RefusalCode::Cancelled, "web operation cancelled");
                }
                if output.timed_out {
                    return failed(op, RefusalCode::DeadlineExceeded, "web operation timed out");
                }
                if output.stdout_truncated || output.stdout_had_invalid_utf8 {
                    return invalid(op);
                }
                let Ok(envelope) = serde_json::from_str::<Value>(&output.stdout) else {
                    return invalid(op);
                };
                if envelope["command"] != wire || envelope["ok"] != output.status_success {
                    return invalid(op);
                }
                let data = &envelope["data"];
                if self.dry_run && data["provider_dispatched"] == true {
                    return invalid(op);
                }
                let result = match &self.request {
                    WebRequest::Prepare(endpoint)
                        if output.status_success
                            || data["debug"]["state"] == "route_unavailable" =>
                    {
                        prepare_receipt(op, data, endpoint, self.dry_run, output.status_success)
                    }
                    WebRequest::Launch(endpoint) if output.status_success => {
                        launch_receipt(data, endpoint.as_deref(), self.dry_run)
                    }
                    _ => runner_envelope_refusal(
                        runner_rejection(
                            Some(op.into()),
                            envelope["error"]["code"]
                                .as_str()
                                .unwrap_or("provider_failed"),
                            envelope["error"]["layer"].as_str(),
                            "v8-runner refused the web operation",
                        ),
                        &envelope,
                    ),
                };
                if result.ok {
                    if let Some(guard) = guard.as_mut() {
                        if cancellation.handoff_with_gate(|| guard.release()).is_err() {
                            return failed(
                                op,
                                if cancellation.is_cancelled() {
                                    RefusalCode::Cancelled
                                } else {
                                    RefusalCode::ProviderFailed
                                },
                                "could not hand off the browser launch",
                            );
                        }
                    }
                }
                result
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDE: &str = "http://127.0.0.1:1561";
    const PUBLISHER: &str = "http://debug-host:1562";
    const PAGE: &str = "https://publication.example/base/?L=ru";

    fn publication(state: &str, changed: bool) -> Value {
        json!({"action":"publish","ok": state!="route_unavailable","provider_dispatched":false,
            "url":PAGE,"dir":"private-path","debug":{"ide_url":IDE,"debugger_url":PUBLISHER,
            "publication_changed":changed,"state":state}})
    }

    #[test]
    fn prepare_and_repeat_preserve_three_independent_addresses_and_report_actual_changes() {
        for (state, dry_run, changed) in [
            ("planned", true, false),
            ("ready", false, true),
            ("ready", false, false),
            ("external_route_verified", false, false),
        ] {
            let r = prepare_receipt(PUBLISH, &publication(state, changed), IDE, dry_run, true);
            assert!(r.ok, "{r:?}");
            let data = r.data.unwrap();
            assert_eq!(data["ideUrl"], IDE);
            assert_eq!(data["debuggerUrl"], PUBLISHER);
            assert_eq!(data["url"], PAGE);
            assert_eq!(data["publicationChanged"], changed);
            assert_eq!(r.changed.is_empty(), !changed);
            assert!(data.get("dir").is_none());
        }
    }

    #[test]
    fn unavailable_route_names_owner_action_and_never_publishes_runner_paths() {
        let mut receipt = publication("route_unavailable", false);
        receipt["message"] = json!("private-path");
        receipt["debug"]["debugger_url"] = Value::Null;
        let r = prepare_receipt(PUBLISH, &receipt, IDE, false, false);
        assert!(!r.ok);
        assert_eq!(r.data.unwrap()["runnerCode"], "route_unavailable");
        assert!(r.summary.contains("infobase.web.debugger-url"));
        assert!(!r.summary.contains("private-path"));
        assert!(r.changed.is_empty());
    }

    #[test]
    fn malformed_or_inconsistent_web_receipts_are_rejected() {
        for (state, dry_run, changed, ok) in [
            ("ready", true, false, true),
            ("planned", false, false, true),
            ("external_route_verified", false, true, true),
            ("route_unavailable", false, false, true),
        ] {
            assert!(!prepare_receipt(PUBLISH, &publication(state, changed), IDE, dry_run, ok).ok);
        }
        let mut receipt = publication("ready", false);
        receipt["debug"]["ide_url"] = json!(PUBLISHER);
        assert!(!prepare_receipt(PUBLISH, &receipt, IDE, false, true).ok);
        for url in [
            "http://host:0",
            "tcp://host:12",
            "http://host:12/path",
            "http://a:b@host:12",
            "http://host:12?",
            "http://host:123999",
            "http://host",
        ] {
            assert_eq!(debugger_endpoint(url), None, "{url}");
        }
        assert_eq!(
            debugger_endpoint("http://[::1]:80/").as_deref(),
            Some("http://[::1]:80")
        );
    }

    #[test]
    fn web_launch_distinguishes_dispatch_from_attachment_and_preserves_ordinary_launch() {
        let mut receipt = json!({"ok":true,"mode":"web","via":"web","url":PAGE,"provider_dispatched":true,"pid":123});
        assert!(launch_receipt(&receipt, None, false).ok);
        assert!(!launch_receipt(&receipt, Some(PUBLISHER), false).ok);
        let mut url = Url::parse(PAGE).unwrap();
        url.query_pairs_mut()
            .append_pair("Debug", "http,attach")
            .append_pair("DebuggerURL", PUBLISHER);
        receipt["url"] = json!(url.as_str());
        receipt["debug"] = json!({"url":PUBLISHER});
        let r = launch_receipt(&receipt, Some(PUBLISHER), false);
        assert!(r.ok);
        assert_eq!(r.data.as_ref().unwrap()["providerDispatched"], true);
        assert!(r.data.unwrap().get("attached").is_none());
        assert!(!launch_receipt(&receipt, Some(IDE), false).ok);
        assert!(!launch_receipt(&receipt, Some(PUBLISHER), true).ok);
        receipt["provider_dispatched"] = json!(false);
        receipt["pid"] = Value::Null;
        assert!(launch_receipt(&receipt, Some(PUBLISHER), true).ok);
        url.query_pairs_mut().append_pair("debuggerurl", IDE);
        receipt["url"] = json!(url.as_str());
        assert!(!launch_receipt(&receipt, Some(PUBLISHER), true).ok);
    }
}
