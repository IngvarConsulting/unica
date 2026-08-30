use crate::infrastructure::daemon::client::{DaemonClient, DaemonClientConfig, DaemonOwner};
use crate::infrastructure::daemon::client_v5::V5DaemonProcessOwner;
use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonProtocolIdentity};
use crate::infrastructure::daemon::server::DaemonServerConfig;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(30);
const MAX_TEST_IDLE_GRACE_MS: u128 = 600_000;

pub fn run_from_args(args: &[String]) -> Result<(), String> {
    let parsed = parse_daemon_args(args)?;
    let state_root = PathBuf::from(parsed.state_root);
    let core_identity = CoreIdentity::from_str(&parsed.core_identity)?;
    let idle_grace = parsed.idle_grace;
    let config = DaemonServerConfig::new(state_root, core_identity.clone(), idle_grace);
    match runtime_selection(&core_identity) {
        DaemonRuntimeSelection::V3 => crate::infrastructure::daemon::server::run_daemon(config),
        DaemonRuntimeSelection::V5 => crate::infrastructure::daemon::runtime_v5::run_daemon(config),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonRuntimeSelection {
    V3,
    V5,
}

fn runtime_selection(core_identity: &CoreIdentity) -> DaemonRuntimeSelection {
    match core_identity.protocol_identity() {
        DaemonProtocolIdentity::V3 => DaemonRuntimeSelection::V3,
        DaemonProtocolIdentity::V5 => DaemonRuntimeSelection::V5,
    }
}

#[allow(
    dead_code,
    reason = "Task 7 routes invocations through this dormant lazy client seam"
)]
pub(crate) fn connect_default_user_daemon(state_root: &Path) -> Result<DaemonOwner, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate current unica executable: {error}"))?;
    DaemonClient::new(DaemonClientConfig::new(
        state_root.to_path_buf(),
        CoreIdentity::production(),
        executable,
        DEFAULT_IDLE_GRACE,
    ))
    .connect_or_spawn()
}

/// Process-level protocol fixture used by the daemon race regression. This is not an MCP tool,
/// is absent from ordinary stdio startup, and requires the caller to provide the exact binary and
/// private state root explicitly.
#[doc(hidden)]
pub fn connect_owner_for_protocol_test(
    state_root: &Path,
    core_identity: &str,
    executable: &Path,
    idle_grace_ms: u64,
) -> Result<DaemonOwnerLease, String> {
    let identity = CoreIdentity::from_str(core_identity)?;
    let idle_grace = Duration::from_millis(idle_grace_ms);
    if idle_grace.is_zero() || idle_grace.as_millis() > MAX_TEST_IDLE_GRACE_MS {
        return Err("daemon test idle grace is outside the supported range".to_string());
    }
    let inner = match identity.protocol_identity() {
        DaemonProtocolIdentity::V3 => DaemonClient::new(DaemonClientConfig::new(
            state_root.to_path_buf(),
            identity,
            executable.to_path_buf(),
            idle_grace,
        ))
        .connect_or_spawn()
        .map(DaemonOwnerLeaseInner::V3),
        DaemonProtocolIdentity::V5 => V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            state_root,
            identity,
            executable.to_path_buf(),
            idle_grace,
        )
        .map(DaemonOwnerLeaseInner::V5),
    }?;
    Ok(DaemonOwnerLease { inner })
}

/// Resolve the versioned daemon endpoint used by process-level protocol fixtures without
/// duplicating the protocol generation in integration tests.
#[doc(hidden)]
pub fn endpoint_path_for_protocol_test(state_root: &Path, core_identity: &str) -> PathBuf {
    let identity_path = CoreIdentity::from_str(core_identity).ok().map(|identity| {
        crate::infrastructure::daemon::identity::DaemonStateDirectory::path_for(
            state_root, &identity,
        )
    });
    identity_path
        .unwrap_or_else(|| state_root.join(format!("daemon-p3-{core_identity}")))
        .join("endpoint.json")
}

#[doc(hidden)]
pub struct DaemonOwnerLease {
    inner: DaemonOwnerLeaseInner,
}

enum DaemonOwnerLeaseInner {
    V3(DaemonOwner),
    V5(V5DaemonProcessOwner),
}

impl DaemonOwnerLease {
    #[doc(hidden)]
    pub fn daemon_pid(&self) -> u32 {
        match &self.inner {
            DaemonOwnerLeaseInner::V3(owner) => owner.daemon_pid(),
            DaemonOwnerLeaseInner::V5(owner) => owner.daemon_pid(),
        }
    }

    #[doc(hidden)]
    pub fn ping(&mut self) -> Result<(), String> {
        match &mut self.inner {
            DaemonOwnerLeaseInner::V3(owner) => owner.ping(),
            DaemonOwnerLeaseInner::V5(owner) => owner.ping(),
        }
    }
}

struct ParsedDaemonArgs {
    state_root: String,
    core_identity: String,
    idle_grace: Duration,
}

fn parse_daemon_args(args: &[String]) -> Result<ParsedDaemonArgs, String> {
    let mut daemon_seen = false;
    let mut state_root = None;
    let mut core_identity = None;
    let mut idle_grace = None;
    let mut position = 1;
    while position < args.len() {
        let argument = args[position].as_str();
        match argument {
            "--daemon" => {
                if daemon_seen {
                    return Err("daemon mode flag must appear exactly once".to_string());
                }
                daemon_seen = true;
                position += 1;
            }
            "--state-root" | "--core-identity" | "--idle-grace-ms" => {
                let value = args
                    .get(position + 1)
                    .ok_or_else(|| format!("daemon argument {argument} requires one value"))?;
                if value.is_empty() || value.starts_with("--") {
                    return Err(format!("daemon argument {argument} requires one value"));
                }
                let target = match argument {
                    "--state-root" => &mut state_root,
                    "--core-identity" => &mut core_identity,
                    "--idle-grace-ms" => &mut idle_grace,
                    _ => unreachable!(),
                };
                if target.replace(value.clone()).is_some() {
                    return Err(format!("daemon argument {argument} must not be duplicated"));
                }
                position += 2;
            }
            _ => return Err(format!("unknown or conflicting daemon argument {argument}")),
        }
    }
    if !daemon_seen {
        return Err("missing daemon mode flag".to_string());
    }
    let state_root =
        state_root.ok_or_else(|| "missing daemon argument --state-root".to_string())?;
    let core_identity =
        core_identity.ok_or_else(|| "missing daemon argument --core-identity".to_string())?;
    let idle_grace = match idle_grace {
        Some(value) => {
            let milliseconds = value
                .parse::<u64>()
                .map_err(|_| "daemon idle grace must be an integer".to_string())?;
            let duration = Duration::from_millis(milliseconds);
            if duration.is_zero() || duration.as_millis() > MAX_TEST_IDLE_GRACE_MS {
                return Err("daemon idle grace is outside the supported range".to_string());
            }
            duration
        }
        None => DEFAULT_IDLE_GRACE,
    };
    Ok(ParsedDaemonArgs {
        state_root,
        core_identity,
        idle_grace,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_daemon_args, runtime_selection, DaemonRuntimeSelection};
    use crate::infrastructure::daemon::identity::CoreIdentity;
    use std::str::FromStr;

    fn base_args() -> Vec<String> {
        vec![
            "unica".into(),
            "--daemon".into(),
            "--state-root".into(),
            "/private/state".into(),
            "--core-identity".into(),
            "a".repeat(64),
        ]
    }

    #[test]
    fn hidden_daemon_cli_rejects_unknown_duplicate_and_conflicting_modes() {
        for extra in [
            vec!["--unknown".to_string()],
            vec!["--daemon".to_string()],
            vec!["--workspace-service".to_string()],
            vec!["--state-root".to_string(), "/other".to_string()],
        ] {
            let mut args = base_args();
            args.extend(extra);
            assert!(parse_daemon_args(&args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn exact_v5_identity_alone_selects_the_distinct_runtime() {
        assert_eq!(
            runtime_selection(&CoreIdentity::production_v5()),
            DaemonRuntimeSelection::V5
        );
        assert_eq!(
            runtime_selection(&CoreIdentity::production()),
            DaemonRuntimeSelection::V3
        );
        for encoded in [
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "b1966ce0792d157e8716a0f29a386a2d8efe801b0abb752c342014bc6eec2d77",
        ] {
            assert_eq!(
                runtime_selection(&CoreIdentity::from_str(encoded).unwrap()),
                DaemonRuntimeSelection::V3
            );
        }
    }

    #[test]
    fn endpoint_fixture_path_uses_the_protocol_owned_by_the_typed_identity() {
        let state_root = std::path::Path::new("/provider-state");

        assert_eq!(
            super::endpoint_path_for_protocol_test(
                state_root,
                CoreIdentity::production_v5().as_str()
            ),
            state_root
                .join(format!(
                    "daemon-p5-{}",
                    CoreIdentity::production_v5().as_str()
                ))
                .join("endpoint.json")
        );
        assert_eq!(
            super::endpoint_path_for_protocol_test(state_root, CoreIdentity::production().as_str()),
            state_root
                .join(format!("daemon-p3-{}", CoreIdentity::production().as_str()))
                .join("endpoint.json")
        );
        assert_eq!(
            super::endpoint_path_for_protocol_test(state_root, "legacy-fixture-identity"),
            state_root
                .join("daemon-p3-legacy-fixture-identity")
                .join("endpoint.json"),
            "the pre-v5 fixture helper accepted opaque identities and must keep that ABI"
        );
    }
}
