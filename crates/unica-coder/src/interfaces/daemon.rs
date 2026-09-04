use crate::infrastructure::daemon::client::{DaemonClient, DaemonClientConfig, DaemonOwner};
use crate::infrastructure::daemon::client_v5::V5DaemonProcessOwner;
use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonProtocolIdentity};
use crate::infrastructure::daemon::server::DaemonServerConfig;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

/// Сколько демон ждёт работы, прежде чем выйти. Тёплый индекс BSL стоит минут,
/// поэтому пауза между задачами не должна стоить пользователю повторной сборки.
const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(15 * 60);
/// Верхняя граница принимаемого значения. Её проверяет и производственный
/// запуск демона, и тестовый вход, поэтому она обязана быть не меньше
/// `DEFAULT_IDLE_GRACE`.
const MAX_IDLE_GRACE_MS: u128 = 60 * 60 * 1_000;
/// Пауза, назначенная снаружи. Демон переживает процесс, который его поднял,
/// поэтому вызывающему нужен способ не оставлять его после себя на четверть
/// часа: интеграционный тест запускает демона десятками, а живут они до конца
/// всего прогона. Значение проходит ту же проверку, что и `--idle-grace-ms`.
const IDLE_GRACE_ENV: &str = "UNICA_DAEMON_IDLE_GRACE_MS";

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

/// Resolve the persistent state root of the user daemon without mutating the
/// process environment. Packaged hosts supply the provider root explicitly;
/// an interactive user falls back to a private directory beneath their home.
pub(crate) fn default_user_daemon_state_root() -> Result<PathBuf, String> {
    resolve_default_user_daemon_state_root(&|name| std::env::var_os(name))
}

fn resolve_default_user_daemon_state_root(
    read_env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, String> {
    let root = read_env("UNICA_PROVIDER_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            read_env("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".unica").join("provider-state"))
        })
        .or_else(|| {
            read_env("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".unica").join("provider-state"))
        })
        .ok_or_else(|| {
            "UNICA_PROVIDER_STATE_DIR, HOME, or USERPROFILE is required for the user daemon state"
                .to_string()
        })?;
    if !root.is_absolute() {
        return Err("user daemon state root must be absolute".to_string());
    }
    Ok(root)
}

/// Пауза, с которой поднимается демон: назначенная снаружи или та, что по
/// умолчанию. Пустое значение читается как «не назначено», негодное —
/// отказом: назначить паузу и молча получить другую хуже, чем не запуститься.
fn configured_idle_grace(read_env: &dyn Fn(&str) -> Option<OsString>) -> Result<Duration, String> {
    let Some(value) = read_env(IDLE_GRACE_ENV).filter(|value| !value.is_empty()) else {
        return Ok(DEFAULT_IDLE_GRACE);
    };
    let milliseconds = value
        .to_str()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("{IDLE_GRACE_ENV} must be an integer number of milliseconds"))?;
    let duration = Duration::from_millis(milliseconds);
    if duration.is_zero() || duration.as_millis() > MAX_IDLE_GRACE_MS {
        return Err("daemon idle grace is outside the supported range".to_string());
    }
    Ok(duration)
}

/// Connect to the production v3 user daemon, starting it if it is absent.
pub(crate) fn connect_default_user_daemon(state_root: &Path) -> Result<DaemonOwner, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate current unica executable: {error}"))?;
    let idle_grace = configured_idle_grace(&|name| std::env::var_os(name))?;
    DaemonClient::new(DaemonClientConfig::new(
        state_root.to_path_buf(),
        CoreIdentity::production(),
        executable,
        idle_grace,
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
    if idle_grace.is_zero() || idle_grace.as_millis() > MAX_IDLE_GRACE_MS {
        return Err("daemon idle grace is outside the supported range".to_string());
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
            if duration.is_zero() || duration.as_millis() > MAX_IDLE_GRACE_MS {
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
    use super::{
        configured_idle_grace, parse_daemon_args, resolve_default_user_daemon_state_root,
        runtime_selection, DaemonRuntimeSelection, DEFAULT_IDLE_GRACE, IDLE_GRACE_ENV,
        MAX_IDLE_GRACE_MS,
    };
    use crate::infrastructure::daemon::identity::CoreIdentity;
    use std::str::FromStr;
    use std::time::Duration;

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

    /// Родитель всегда передаёт демону `--idle-grace-ms`, и это же значение
    /// проверяет парсер. Дефолт выше границы означал бы, что демон не
    /// запускается вовсе, а обнаружилось бы это только на живом хосте.
    #[test]
    fn default_idle_grace_is_accepted_by_the_parser_that_receives_it() {
        assert!(DEFAULT_IDLE_GRACE.as_millis() <= MAX_IDLE_GRACE_MS);

        let mut args = base_args();
        args.push("--idle-grace-ms".into());
        args.push(DEFAULT_IDLE_GRACE.as_millis().to_string());
        let parsed = parse_daemon_args(&args).expect("default idle grace must parse");
        assert_eq!(parsed.idle_grace, DEFAULT_IDLE_GRACE);
    }

    /// Демон переживает процесс, который его поднял, поэтому пауза назначается
    /// снаружи — иначе интеграционный тест оставляет за собой демона на всю
    /// четверть часа, и к концу прогона их набирается столько, сколько было
    /// тестов. Негодное значение — отказ, а не тихий возврат к умолчанию.
    #[test]
    fn idle_grace_is_assignable_from_the_environment_and_refuses_junk() {
        let read = |value: Option<&str>| {
            let value = value.map(|value| value.to_string());
            move |name: &str| {
                if name == IDLE_GRACE_ENV {
                    value.clone().map(Into::into)
                } else {
                    None
                }
            }
        };

        assert_eq!(
            Ok(DEFAULT_IDLE_GRACE),
            configured_idle_grace(&read(None)),
            "без назначения демон живёт столько же, сколько и раньше"
        );
        assert_eq!(
            Ok(DEFAULT_IDLE_GRACE),
            configured_idle_grace(&read(Some("")))
        );
        assert_eq!(
            Ok(Duration::from_millis(5_000)),
            configured_idle_grace(&read(Some("5000")))
        );
        for junk in ["", "0", "-1", "5s", "  "].iter().skip(1) {
            assert!(
                configured_idle_grace(&read(Some(junk))).is_err(),
                "негодное значение {junk} обязано быть отказом"
            );
        }
        let above = (MAX_IDLE_GRACE_MS + 1).to_string();
        assert!(configured_idle_grace(&read(Some(&above))).is_err());
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
    fn default_user_daemon_state_root_is_pure_and_requires_an_absolute_root() {
        let absolute_root = std::env::temp_dir().join("unica-provider-state");
        let alice_home = std::env::temp_dir().join("unica-home-alice");
        let bob_home = std::env::temp_dir().join("unica-home-bob");
        let environment = |name: &str| match name {
            "UNICA_PROVIDER_STATE_DIR" => Some(absolute_root.clone().into_os_string()),
            "HOME" => Some(alice_home.clone().into_os_string()),
            _ => None,
        };
        assert_eq!(
            resolve_default_user_daemon_state_root(&environment).unwrap(),
            absolute_root
        );

        let home_only = |name: &str| (name == "HOME").then(|| alice_home.clone().into_os_string());
        assert_eq!(
            resolve_default_user_daemon_state_root(&home_only).unwrap(),
            alice_home.join(".unica/provider-state")
        );

        let user_profile_only =
            |name: &str| (name == "USERPROFILE").then(|| bob_home.clone().into_os_string());
        assert_eq!(
            resolve_default_user_daemon_state_root(&user_profile_only).unwrap(),
            bob_home.join(".unica/provider-state")
        );

        let relative = |name: &str| (name == "UNICA_PROVIDER_STATE_DIR").then(|| "state".into());
        assert!(resolve_default_user_daemon_state_root(&relative).is_err());
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
