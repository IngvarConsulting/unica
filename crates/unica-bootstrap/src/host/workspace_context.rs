//! Workspace context supplied by the host, independently of tool arguments.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::descriptor::KNOWN;

/// Captured launch context. Request metadata can override it without changing
/// the context of other calls sharing the same frontend.
#[derive(Clone, Debug)]
pub struct HostWorkspaceContext {
    launch_directory: Result<PathBuf, String>,
    require_existing_directory: bool,
}

/// Capture once at frontend startup; errors are deferred until a request needs
/// the launch context, since the host may supply a valid context per request.
pub fn capture_host_workspace_context() -> HostWorkspaceContext {
    capture_with(
        &|name| std::env::var_os(name),
        std::env::current_dir().map_err(|e| e.to_string()),
    )
}

/// Capabilities that ask the host to attach workspace context to tool calls.
pub fn host_workspace_capabilities() -> BTreeMap<String, Map<String, Value>> {
    KNOWN
        .iter()
        .filter_map(|host| host.workspace_metadata)
        .map(|channel| (channel.capability.to_owned(), Map::new()))
        .collect()
}

/// Project-specific environment inherited by a frontend, never by a shared daemon.
pub fn host_workspace_environment_keys() -> Vec<&'static str> {
    KNOWN
        .iter()
        .flat_map(|host| host.workspace_environment.iter().copied())
        .collect()
}

impl HostWorkspaceContext {
    /// Explicit launch directory for direct invocation and embedded consumers.
    pub fn from_directory(directory: PathBuf) -> Self {
        Self {
            launch_directory: Ok(directory),
            require_existing_directory: false,
        }
    }

    /// Resolve only this request's workspace. A malformed supplied context is
    /// an error, never permission to fall back to another project's directory.
    pub fn resolve(&self, metadata: &Map<String, Value>) -> Result<String, String> {
        let mut request_directory: Option<PathBuf> = None;
        for channel in KNOWN.iter().filter_map(|host| host.workspace_metadata) {
            if let Some(context) = metadata.get(channel.capability) {
                let directory = context
                    .get(channel.directory_field)
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        format!(
                            "Host workspace metadata must contain a string {}",
                            channel.directory_field
                        )
                    })?;
                let directory =
                    PathBuf::from(existing_directory(&parse_request_directory(directory)?)?);
                if request_directory
                    .as_ref()
                    .is_some_and(|previous| !same_directory(previous, &directory))
                {
                    return Err(
                        "Host workspace metadata declares conflicting directories".to_owned()
                    );
                }
                request_directory.get_or_insert(directory);
            }
        }
        if let Some(directory) = request_directory {
            return absolute_directory(&directory);
        }
        let directory = self.launch_directory.as_ref().map_err(Clone::clone)?;
        if self.require_existing_directory {
            existing_directory(directory)
        } else {
            absolute_directory(directory)
        }
    }
}

fn capture_with(
    read_env: &dyn Fn(&str) -> Option<OsString>,
    cwd: Result<PathBuf, String>,
) -> HostWorkspaceContext {
    let launch_directory = (|| {
        let mut selected: Option<PathBuf> = None;
        for key in host_workspace_environment_keys() {
            if let Some(value) = read_env(key) {
                let value = value
                    .to_str()
                    .ok_or_else(|| format!("Host project environment {key} is not Unicode"))?;
                let directory = PathBuf::from(existing_directory(Path::new(value))?);
                if selected
                    .as_ref()
                    .is_some_and(|previous| !same_directory(previous, &directory))
                {
                    return Err(
                        "Host project environment declares conflicting directories".to_owned()
                    );
                }
                if selected.is_none() {
                    selected = Some(directory);
                }
            }
        }
        if let Some(directory) = selected {
            return Ok(directory);
        }
        let required = read_env("UNICA_HOST_CONTEXT_REQUIRED").is_some_and(|v| v == "1")
            || read_env("UNICA_RUNTIME_MANIFEST").is_some_and(|v| !v.is_empty());
        if required {
            return Err("The host did not supply workspace context for this request".to_owned());
        }
        cwd
    })();
    HostWorkspaceContext {
        launch_directory,
        require_existing_directory: true,
    }
}

fn parse_request_directory(value: &str) -> Result<PathBuf, String> {
    validate_path_text(value)?;
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    let uri = url::Url::parse(value)
        .map_err(|_| "Host workspace must be an absolute path or local file URI".to_owned())?;
    if !value.starts_with("file://")
        || uri.scheme() != "file"
        || uri.query().is_some()
        || uri.fragment().is_some()
        || uri.host_str().is_some_and(|host| host != "localhost")
        || !uri.username().is_empty()
        || uri.password().is_some()
    {
        return Err(
            "Host workspace must be an absolute path or local file URI without query or fragment"
                .to_owned(),
        );
    }
    uri.to_file_path()
        .map_err(|_| "Host workspace file URI is not a native absolute path".to_owned())
}

fn absolute_directory(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("Host workspace directory must be absolute".to_owned());
    }
    let value = path
        .to_str()
        .ok_or_else(|| "Host workspace directory is not Unicode".to_owned())?;
    validate_path_text(value)?;
    Ok(value.to_owned())
}

fn existing_directory(path: &Path) -> Result<String, String> {
    let value = absolute_directory(path)?;
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Cannot access host workspace directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("Host workspace path is not a directory".to_owned());
    }
    Ok(value)
}

fn validate_path_text(value: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("Host workspace directory must not contain control characters".to_owned());
    }
    Ok(())
}

fn same_directory(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

impl From<String> for HostWorkspaceContext {
    fn from(directory: String) -> Self {
        Self::from_directory(PathBuf::from(directory))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("unica-host-context-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(path.join("Проект с пробелом")).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn project(&self) -> PathBuf {
            self.0.join("Проект с пробелом")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn env<'a>(values: &'a [(&'a str, OsString)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| {
            values
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        }
    }
    fn metadata(value: Value) -> Map<String, Value> {
        Map::from_iter([("codex/sandbox-state-meta".into(), value)])
    }

    #[test]
    fn request_context_overrides_launch_environment_and_does_not_leak_to_next_request() {
        let fixture = Fixture::new();
        let context = capture_with(
            &env(&[("CLAUDE_PROJECT_DIR", fixture.0.clone().into())]),
            Ok(fixture.0.clone()),
        );
        for value in [
            fixture.project().to_str().unwrap().to_owned(),
            url::Url::from_file_path(fixture.project())
                .unwrap()
                .to_string(),
        ] {
            assert_eq!(
                context
                    .resolve(&metadata(serde_json::json!({"sandboxCwd": value})))
                    .unwrap(),
                fixture.project().to_str().unwrap()
            );
        }
        assert_eq!(
            context.resolve(&Map::new()).unwrap(),
            fixture.0.to_str().unwrap()
        );
    }

    #[test]
    fn malformed_request_never_falls_back_to_valid_launch_directory() {
        let fixture = Fixture::new();
        let context = HostWorkspaceContext::from_directory(fixture.0.clone());
        for value in [
            Value::Null,
            serde_json::json!({}),
            serde_json::json!({"sandboxCwd": 7}),
        ] {
            assert!(context.resolve(&metadata(value)).is_err());
        }
        let file_uri = url::Url::from_directory_path(&fixture.0).unwrap();
        for value in [
            "".into(),
            "relative".into(),
            "file:relative".into(),
            "file:///workspace\nwrong".into(),
            "file:///workspace%00wrong".into(),
            "file:///workspace%0awrong".into(),
            "file:///%FF".into(),
            fixture.0.join("bad\0path").to_str().unwrap().to_owned(),
            "https://example.com/path".into(),
            "file://other-host/tmp".into(),
            format!("{file_uri}?query"),
            format!("{file_uri}#fragment"),
        ] {
            assert!(context
                .resolve(&metadata(serde_json::json!({"sandboxCwd": value})))
                .is_err());
        }
    }

    #[test]
    fn environment_aliases_must_resolve_to_one_directory() {
        let fixture = Fixture::new();
        let agreeing = [
            ("ZCODE_PROJECT_DIR", fixture.0.clone().into()),
            ("CLAUDE_PROJECT_DIR", fixture.0.join(".").into()),
        ];
        assert_eq!(
            capture_with(&env(&agreeing), Err("no cwd".into()))
                .resolve(&Map::new())
                .unwrap(),
            fixture.0.to_str().unwrap()
        );
        let conflicting = [
            ("ZCODE_PROJECT_DIR", fixture.0.clone().into()),
            ("CLAUDE_PROJECT_DIR", fixture.project().into()),
        ];
        let context = capture_with(&env(&conflicting), Ok(fixture.0.clone()));
        assert!(context.resolve(&Map::new()).is_err());
        assert_eq!(
            context
                .resolve(&metadata(
                    serde_json::json!({"sandboxCwd": fixture.project()})
                ))
                .unwrap(),
            fixture.project().to_str().unwrap()
        );
    }

    #[test]
    fn declared_invalid_environment_is_not_treated_as_absent() {
        let fixture = Fixture::new();
        for value in [
            OsString::new(),
            "relative".into(),
            fixture.0.join("bad\npath").into(),
            fixture.0.join("bad\0path").into(),
        ] {
            let context = capture_with(
                &env(&[("CLAUDE_PROJECT_DIR", value)]),
                Ok(fixture.0.clone()),
            );
            assert!(context.resolve(&Map::new()).is_err());
        }
    }

    #[test]
    fn explicit_directory_constructor_leaves_filesystem_admission_to_its_caller() {
        let fixture = Fixture::new();
        let absent = fixture.0.join("not-created");
        assert_eq!(
            HostWorkspaceContext::from(absent.to_str().unwrap().to_owned())
                .resolve(&Map::new())
                .unwrap(),
            absent.to_str().unwrap()
        );
    }

    #[test]
    fn host_paths_must_exist_as_directories_and_deleted_launch_context_is_rejected() {
        let fixture = Fixture::new();
        let file = fixture.0.join("file");
        std::fs::write(&file, "not a directory").unwrap();
        let direct = HostWorkspaceContext::from_directory(fixture.0.clone());
        for invalid in [file, fixture.0.join("missing")] {
            assert!(direct
                .resolve(&metadata(serde_json::json!({"sandboxCwd": invalid})))
                .is_err());
            let captured = capture_with(
                &env(&[("CLAUDE_PROJECT_DIR", invalid.into())]),
                Ok(fixture.0.clone()),
            );
            assert!(captured.resolve(&Map::new()).is_err());
            assert!(captured
                .resolve(&metadata(serde_json::json!({"sandboxCwd": fixture.0})))
                .is_ok());
        }
        let captured = capture_with(
            &env(&[("CLAUDE_PROJECT_DIR", fixture.project().into())]),
            Ok(fixture.0.clone()),
        );
        assert!(captured.resolve(&Map::new()).is_ok());
        std::fs::remove_dir(fixture.project()).unwrap();
        assert!(captured.resolve(&Map::new()).is_err());
        assert!(captured
            .resolve(&metadata(serde_json::json!({"sandboxCwd": fixture.0})))
            .is_ok());
    }

    #[test]
    fn packaged_launch_requires_host_context_but_direct_launch_retains_cwd() {
        let fixture = Fixture::new();
        assert_eq!(
            capture_with(&env(&[]), Ok(fixture.0.clone()))
                .resolve(&Map::new())
                .unwrap(),
            fixture.0.to_str().unwrap()
        );
        for marker in [
            ("UNICA_HOST_CONTEXT_REQUIRED", "1".into()),
            ("UNICA_RUNTIME_MANIFEST", "runtime.json".into()),
        ] {
            let context = capture_with(&env(&[marker]), Ok(fixture.0.clone()));
            assert!(context.resolve(&Map::new()).is_err());
            assert!(context
                .resolve(&metadata(
                    serde_json::json!({"sandboxCwd": fixture.project()})
                ))
                .is_ok());
        }
    }
}
