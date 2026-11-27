use super::*;
use unica_source_contract::debug_source::DebugServerUrl;

impl PreparedDebugServer {
    pub(super) fn prepare(
        &self,
        tool: &BundledTool,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
        owner_pid: u32,
    ) -> DomainResult {
        let args = vec!["--owner-pid".to_owned(), owner_pid.to_string()];
        let data = match self.invoke(tool, runner, PREPARE, args, self.dry_run, cancellation) {
            Ok(data) => data,
            Err(result) => return result,
        };
        let invalid = || {
            failure(
                PREPARE,
                RefusalCode::InvalidResult,
                "runner did not confirm the debug server preparation",
            )
        };
        if self.dry_run {
            if data["action"] != "planned"
                || ["url", "scope", "release"]
                    .iter()
                    .any(|field| !data[*field].is_null())
            {
                return invalid();
            }
            let mut result = DomainResult::success("debug server preparation planned");
            result.data = Some(json!({"op":PREPARE, "contractVersion":1, "action":"planned"}));
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
        let lease = match release_lease(&data, owner_pid) {
            Ok(lease) => lease,
            Err(()) => return invalid(),
        };
        if let Some(mut result) = ready_endpoint(&data, url, lease) {
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
            // Each prepare owns one new lease, including reuse of an existing
            // process. Releasing this token leaves any previously handed-off
            // lease under the caller's control.
            let release = self.stop(
                tool,
                runner,
                session,
                false,
                true,
                &CancellationToken::new(),
            );
            if !release.ok {
                let mut result = failure(PREPARE, RefusalCode::ProviderFailed,
                    "debug server preparation was not handed off and lease release was not confirmed");
                result.data = Some(json!({"op":PREPARE, "release":release_operation(session)}));
                return result;
            }
        }
        if cancellation.is_cancelled() {
            failure(
                PREPARE,
                RefusalCode::Cancelled,
                "debug server preparation cancelled",
            )
        } else {
            invalid()
        }
    }
}

fn release_lease(data: &Value, owner_pid: u32) -> Result<Option<&str>, ()> {
    match data["owner"].as_str() {
        Some("cluster" | "standalone" | "external")
            if data.get("release") == Some(&Value::Null) =>
        {
            Ok(None)
        }
        Some("runner") if data["owner_pid"] == owner_pid => {
            let release = data["release"].as_object().ok_or(())?;
            if release.len() != 2 || release.get("action").and_then(Value::as_str) != Some("stop") {
                return Err(());
            }
            release
                .get("session")
                .and_then(Value::as_str)
                .filter(|session| valid_session(session))
                .map(Some)
                .ok_or(())
        }
        _ => Err(()),
    }
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

fn ready_endpoint(data: &Value, url: DebugServerUrl, lease: Option<&str>) -> Option<DomainResult> {
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
        json!({"op":PREPARE, "contractVersion":1, "action":"prepared",
        "url":url.to_string(), "infoBaseAlias":alias, "scope":scope,
        "release":lease.map(release_operation)}),
    );
    Some(result)
}
