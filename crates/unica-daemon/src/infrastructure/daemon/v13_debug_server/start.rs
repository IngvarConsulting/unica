use super::*;
use unica_source_contract::debug_source::DebugServerUrl;

impl PreparedDebugServer {
    pub(super) fn start(
        &self,
        tool: &BundledTool,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
        owner_pid: u32,
    ) -> DomainResult {
        let args = vec!["--owner-pid".to_owned(), owner_pid.to_string()];
        let data = match self.invoke(tool, runner, START, args, self.dry_run, cancellation) {
            Ok(data) => data,
            Err(result) => return result,
        };
        let invalid = || {
            failure(
                START,
                RefusalCode::InvalidResult,
                "runner did not confirm the debug server preparation",
            )
        };
        if self.dry_run {
            if data != json!({"action":"planned"}) {
                return invalid();
            }
            let mut result = DomainResult::success("debug server preparation planned");
            result.data = Some(
                json!({"op":START, "contractVersion":DEBUG_SERVER_START_CONTRACT_VERSION, "action":"planned"}),
            );
            return result;
        }
        if data["action"] != "prepared" {
            return invalid();
        }
        let Some(raw_url) = data["url"].as_str() else {
            return invalid();
        };
        let Some(url) = DebugServerUrl::parse(raw_url) else {
            return invalid();
        };
        if data["owner"] == "runner" && !is_loopback(raw_url) {
            return invalid();
        }
        let Some((owner, lease)) = start_authority(&data, owner_pid) else {
            return invalid();
        };
        if let Some(mut result) = ready_endpoint(&data, url, owner, lease) {
            if cancellation.handoff_with_gate(|| Ok(())).is_ok() {
                if lease.is_some() {
                    result
                        .changed
                        .push(json!({"debugServerSession":true, "action":"prepared"}));
                }
                return result;
            }
        }
        if let Some(session) = lease {
            // Each start owns one new lease, including reuse of an existing
            // process. Releasing this token leaves any previously handed-off
            // lease under the caller's control.
            let release = self.stop(tool, runner, session, false, &CancellationToken::new());
            if !release.ok {
                let mut result = failure(START, RefusalCode::ProviderFailed,
                    "debug server preparation was not handed off and lease release was not confirmed");
                result.data = Some(json!({"op":START, "release":release_operation(session)}));
                return result;
            }
        }
        if cancellation.is_cancelled() {
            failure(
                START,
                RefusalCode::Cancelled,
                "debug server preparation cancelled",
            )
        } else {
            invalid()
        }
    }
}

fn start_authority(data: &Value, owner_pid: u32) -> Option<(&str, Option<&str>)> {
    let owner = data["owner"].as_str()?;
    let lease = match owner {
        "cluster" | "standalone" | "external"
            if data.get("release") == Some(&Value::Null) && data["owner_pid"].is_null() =>
        {
            None
        }
        "runner" if data["owner_pid"] == owner_pid => {
            let release = data["release"].as_object()?;
            if release.len() != 2 || release.get("action").and_then(Value::as_str) != Some("stop") {
                return None;
            }
            Some(
                release
                    .get("session")?
                    .as_str()
                    .filter(|session| valid_session(session))?,
            )
        }
        _ => return None,
    };
    Some((owner, lease))
}

fn release_operation(session: &str) -> Value {
    json!({"op":STOP,"args":{"session":session}})
}

fn is_loopback(raw_url: &str) -> bool {
    url::Url::parse(raw_url).is_ok_and(|parsed| match parsed.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name == "localhost",
        None => false,
    })
}

fn ready_endpoint(
    data: &Value,
    url: DebugServerUrl,
    owner: &str,
    lease: Option<&str>,
) -> Option<DomainResult> {
    let alias = data["infobase_alias"]
        .as_str()
        .filter(|alias| !alias.trim().is_empty() && !alias.chars().any(char::is_control))?;
    let scope = match data["scope"].as_str()? {
        "client" if lease.is_some() => "client",
        "client_and_server" => "clientAndServer",
        _ => return None,
    };
    let mut result = DomainResult::success("debug server endpoint prepared");
    result.data = Some(
        json!({"op":START, "contractVersion":DEBUG_SERVER_START_CONTRACT_VERSION, "action":"prepared",
        "url":url.to_string(), "infoBaseAlias":alias, "scope":scope,
        "owner":owner, "release":lease.map(release_operation)}),
    );
    Some(result)
}
