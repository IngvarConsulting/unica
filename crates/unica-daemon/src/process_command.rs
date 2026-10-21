use crate::identity::CoreIdentity;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

pub(crate) fn daemon_process_command(
    executable: &Path,
    state_root: &Path,
    core_identity: &CoreIdentity,
    idle_grace: Duration,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--daemon")
        .arg("--state-root")
        .arg(state_root)
        .arg("--core-identity")
        .arg(core_identity.as_str())
        .arg("--idle-grace-ms")
        .arg(idle_grace.as_millis().to_string())
        .current_dir(state_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for key in unica_bootstrap::host_workspace_environment_keys() {
        command.env_remove(key);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn daemon_process_is_anchored_outside_the_caller_workspace() {
        let state_root = std::env::temp_dir().join("unica-daemon-command-state");
        let command = super::daemon_process_command(
            PathBuf::from("unica").as_path(),
            &state_root,
            &CoreIdentity::production(),
            Duration::from_secs(30),
        );

        assert_eq!(command.get_current_dir(), Some(state_root.as_path()));
        for key in unica_bootstrap::host_workspace_environment_keys() {
            assert!(command
                .get_envs()
                .any(|(name, value)| name == key && value.is_none()));
        }
    }
}
