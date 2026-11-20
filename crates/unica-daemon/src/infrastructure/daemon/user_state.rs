//! The version-independent user state root shared by daemon startup and
//! remembered permissions. Both callers use the same environment policy.

use std::ffi::OsString;
use std::path::PathBuf;

/// Resolve the persistent state root without mutating the process environment.
/// Packaged hosts supply it explicitly; interactive users fall back to home.
pub(crate) fn default_user_daemon_state_root() -> Result<PathBuf, String> {
    let root = resolve_default_user_daemon_state_root(&|name| std::env::var_os(name))?;
    unica_workspace::state_fs::state_root_with_resolved_ancestors(&root)
        .map_err(|error| format!("cannot resolve the user daemon state root: {error}"))
}

pub(crate) fn resolve_default_user_daemon_state_root(
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
