use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::error::{BootstrapError, Result};

use super::descriptor::{PluginHost, KNOWN};

/// Host-neutral override: whoever installs the plugin may point the runtime
/// cache anywhere.
const CACHE_OVERRIDE_ENV: &str = "UNICA_RUNTIME_CACHE_DIR";
/// Shell-style token a host was expected to expand before passing a value on.
const UNEXPANDED_TOKEN: &str = "${";
/// User home directory variables, shared by every host.
const USER_HOME_ENVS: [&str; 2] = ["HOME", "USERPROFILE"];

/// Reads one environment variable. Injected so the resolution chain can be
/// exercised without touching the environment of the test process.
type ReadEnv<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// One way of deriving the runtime cache root from a single host descriptor.
type CacheSource = fn(&PluginHost, ReadEnv<'_>) -> Option<PathBuf>;

/// Sources of the runtime cache root, most authoritative first: a directory the
/// host publishes itself, then the home directory the host names, then the host
/// state directory under the user home. Within one source the hosts are tried
/// in [`KNOWN`] order.
const CACHE_SOURCES: [CacheSource; 3] = [published_data_dir, declared_home_root, user_home_root];

/// Directory that holds the installed Unica runtimes for the current host.
pub fn runtime_cache_root() -> Result<PathBuf> {
    resolve_runtime_cache_root(&|name| env::var_os(name))
}

fn resolve_runtime_cache_root(read_env: ReadEnv<'_>) -> Result<PathBuf> {
    // The package points UNICA_RUNTIME_CACHE_DIR at a data-directory token that
    // only a host which understands it expands. A host that passes the value
    // through literally would otherwise create a directory named after the
    // token, so an unexpanded value is discarded in favour of the host chain.
    if let Some(value) = read_env(CACHE_OVERRIDE_ENV) {
        let value = PathBuf::from(value);
        if !value.to_string_lossy().contains(UNEXPANDED_TOKEN) {
            return Ok(value);
        }
    }
    for source in CACHE_SOURCES {
        for host in KNOWN {
            if let Some(root) = source(host, read_env) {
                return Ok(root);
            }
        }
    }
    Err(BootstrapError::new(format!(
        "{} is required for the runtime cache",
        required_home_envs()
    )))
}

fn published_data_dir(host: &PluginHost, read_env: ReadEnv<'_>) -> Option<PathBuf> {
    let data_dir = host.data_dir?;
    let value = read_env(data_dir.env)?;
    Some(join_segments(PathBuf::from(value), data_dir.runtime_subdir))
}

fn declared_home_root(host: &PluginHost, read_env: ReadEnv<'_>) -> Option<PathBuf> {
    let home_root = host.home_root?;
    let value = read_env(home_root.env)?;
    Some(join_segments(
        PathBuf::from(value),
        home_root.runtime_subdir,
    ))
}

fn user_home_root(host: &PluginHost, read_env: ReadEnv<'_>) -> Option<PathBuf> {
    let home_root = host.home_root?;
    let value = USER_HOME_ENVS.iter().find_map(|name| read_env(name))?;
    Some(join_segments(
        PathBuf::from(value).join(home_root.user_home_segment),
        home_root.runtime_subdir,
    ))
}

fn join_segments(root: PathBuf, segments: &[&str]) -> PathBuf {
    segments
        .iter()
        .fold(root, |path, segment| path.join(segment))
}

/// Names every variable the last resort of the chain reads, so the failure
/// tells the operator exactly what to set.
fn required_home_envs() -> String {
    let mut names: Vec<&str> = KNOWN
        .iter()
        .filter_map(|host| host.home_root.map(|home_root| home_root.env))
        .collect();
    names.extend(USER_HOME_ENVS);
    match names.split_last() {
        Some((last, leading)) if !leading.is_empty() => {
            format!("{}, or {last}", leading.join(", "))
        }
        _ => names.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let entries: BTreeMap<String, OsString> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(*value)))
            .collect();
        move |name: &str| entries.get(name).cloned()
    }

    fn resolve(pairs: &[(&str, &str)]) -> Result<PathBuf> {
        resolve_runtime_cache_root(&environment(pairs))
    }

    #[test]
    fn the_explicit_override_outranks_every_host_source() {
        let root = resolve(&[
            ("UNICA_RUNTIME_CACHE_DIR", "/cache/unica"),
            ("CLAUDE_PLUGIN_DATA", "/data/claude"),
            ("CODEX_HOME", "/home/user/.codex"),
            ("HOME", "/home/user"),
        ])
        .unwrap();

        assert_eq!(root, PathBuf::from("/cache/unica"));
    }

    #[test]
    fn an_unexpanded_override_falls_through_to_the_host_chain() {
        // A host that does not understand the token passes it through literally;
        // creating a directory named after the token would strand the runtime.
        let root = resolve(&[
            ("UNICA_RUNTIME_CACHE_DIR", "${CLAUDE_PLUGIN_DATA}/runtimes"),
            ("CLAUDE_PLUGIN_DATA", "/data/claude"),
        ])
        .unwrap();

        assert_eq!(root, PathBuf::from("/data/claude").join("runtimes"));
    }

    #[test]
    fn an_unexpanded_override_falls_through_to_the_home_directory_too() {
        let root = resolve(&[
            ("UNICA_RUNTIME_CACHE_DIR", "${CLAUDE_PLUGIN_DATA}/runtimes"),
            ("HOME", "/home/user"),
        ])
        .unwrap();

        assert_eq!(
            root,
            PathBuf::from("/home/user")
                .join(".codex")
                .join("unica")
                .join("runtimes")
        );
    }

    #[test]
    fn a_published_data_directory_outranks_every_home_directory() {
        let root = resolve(&[
            ("CLAUDE_PLUGIN_DATA", "/data/claude"),
            ("CODEX_HOME", "/elsewhere/.codex"),
            ("HOME", "/home/user"),
        ])
        .unwrap();

        assert_eq!(root, PathBuf::from("/data/claude").join("runtimes"));
    }

    #[test]
    fn a_declared_host_home_is_used_as_it_stands() {
        let root = resolve(&[("CODEX_HOME", "/elsewhere/.codex"), ("HOME", "/home/user")]).unwrap();

        assert_eq!(
            root,
            PathBuf::from("/elsewhere/.codex")
                .join("unica")
                .join("runtimes")
        );
    }

    #[test]
    fn the_user_home_carries_the_host_state_directory() {
        let root = resolve(&[("HOME", "/home/user")]).unwrap();

        assert_eq!(
            root,
            PathBuf::from("/home/user")
                .join(".codex")
                .join("unica")
                .join("runtimes")
        );
    }

    #[test]
    fn the_windows_user_home_is_the_last_resort() {
        let root = resolve(&[("USERPROFILE", "C:/Users/user")]).unwrap();

        assert_eq!(
            root,
            PathBuf::from("C:/Users/user")
                .join(".codex")
                .join("unica")
                .join("runtimes")
        );
    }

    #[test]
    fn the_posix_user_home_wins_over_the_windows_one() {
        let root = resolve(&[("HOME", "/home/user"), ("USERPROFILE", "C:/Users/user")]).unwrap();

        assert_eq!(
            root,
            PathBuf::from("/home/user")
                .join(".codex")
                .join("unica")
                .join("runtimes")
        );
    }

    #[test]
    fn an_empty_environment_names_every_variable_that_would_help() {
        let error = resolve(&[]).unwrap_err();

        assert_eq!(
            error.to_string(),
            "CODEX_HOME, HOME, or USERPROFILE is required for the runtime cache"
        );
    }

    #[test]
    fn an_unexpanded_override_alone_is_not_a_cache_root() {
        let error =
            resolve(&[("UNICA_RUNTIME_CACHE_DIR", "${CLAUDE_PLUGIN_DATA}/runtimes")]).unwrap_err();

        assert_eq!(
            error.to_string(),
            "CODEX_HOME, HOME, or USERPROFILE is required for the runtime cache"
        );
    }
}
