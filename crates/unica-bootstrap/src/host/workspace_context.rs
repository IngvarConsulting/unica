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

/// Roots the MCP client reported for this request (`roots/list`).
///
/// A client that declares the `roots` capability answers with its current
/// project on every request, so this channel follows a session whose
/// directory changed after the frontend started. Launch environment cannot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ClientRoots {
    /// The client did not declare roots, or the request did not need them.
    #[default]
    NotDeclared,
    /// Root URIs in the order the client listed them.
    Listed(Vec<String>),
    /// The client declared roots but the exchange failed: an error answer,
    /// a timeout or a closed transport. Nothing was supplied, so the launch
    /// context still applies.
    Unavailable(String),
}

/// Host context of one tool call: wire metadata and the client's roots.
#[derive(Clone, Debug, Default)]
pub struct HostRequest {
    pub metadata: Map<String, Value>,
    pub roots: ClientRoots,
}

impl HostRequest {
    pub fn from_metadata(metadata: Map<String, Value>) -> Self {
        Self {
            metadata,
            roots: ClientRoots::NotDeclared,
        }
    }
}

/// Whether a call with this metadata still needs the client's roots: a
/// per-call host channel outranks roots, so they are not asked for then.
pub fn request_needs_client_roots(metadata: &Map<String, Value>) -> bool {
    !KNOWN
        .iter()
        .filter_map(|host| host.workspace_metadata)
        .any(|channel| metadata.contains_key(channel.capability))
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
        self.resolve_request(&HostRequest::from_metadata(metadata.clone()))
    }

    /// Precedence: per-call host metadata, then the first client root, then
    /// the launch context. The first root is the session's project in the
    /// order Claude Code lists it; the MCP specification does not order roots.
    pub fn resolve_request(&self, request: &HostRequest) -> Result<String, String> {
        let metadata = &request.metadata;
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
        if let ClientRoots::Listed(roots) = &request.roots {
            if let Some(root) = roots.first() {
                return existing_directory(&parse_root_uri(root)?);
            }
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

/// MCP roots are `file://` URIs; a bare path is not a root. On Windows a
/// project on a share (`\\server\share`, `\\wsl.localhost\…`) is spelled
/// with the server as the URI host, so a host is accepted there.
fn parse_root_uri(value: &str) -> Result<PathBuf, String> {
    if !value.starts_with("file://") {
        return Err("Client root must be a local file URI".to_owned());
    }
    if crate::platform::file_uri_host_names_share() {
        validate_path_text(value)?;
        let uri = url::Url::parse(value)
            .map_err(|_| "Client root must be a local file URI".to_owned())?;
        if uri.query().is_some()
            || uri.fragment().is_some()
            || !uri.username().is_empty()
            || uri.password().is_some()
        {
            return Err("Client root must be a file URI without query or fragment".to_owned());
        }
        return uri
            .to_file_path()
            .map_err(|_| "Client root file URI is not a native absolute path".to_owned());
    }
    parse_request_directory(value)
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
    fn workspace_environment_keys_include_both_aliases_once() {
        assert_eq!(
            host_workspace_environment_keys(),
            vec!["CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR"]
        );
        assert_eq!(
            host_workspace_capabilities(),
            BTreeMap::from([("codex/sandbox-state-meta".to_owned(), Map::new())])
        );
    }

    #[test]
    fn request_context_overrides_launch_environment_and_does_not_leak_to_next_request() {
        let fixture = Fixture::new();
        for key in ["CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR"] {
            let context = capture_with(
                &env(&[(key, fixture.0.clone().into())]),
                Ok(fixture.project()),
            );
            for value in [
                fixture.project().to_str().unwrap().to_owned(),
                url::Url::from_file_path(fixture.project())
                    .unwrap()
                    .to_string(),
            ] {
                let resolved = context
                    .resolve(&metadata(serde_json::json!({"sandboxCwd": value})))
                    .unwrap();
                let resolved = Path::new(&resolved);
                assert!(resolved.is_absolute());
                // A file URI can spell the same Windows directory without its verbatim prefix.
                assert!(same_directory(resolved, &fixture.project()));
            }
            assert_eq!(
                context.resolve(&Map::new()).unwrap(),
                fixture.0.to_str().unwrap()
            );
        }
    }

    fn with_roots(metadata: Map<String, Value>, roots: ClientRoots) -> HostRequest {
        HostRequest { metadata, roots }
    }

    fn root_uri(path: &Path) -> String {
        url::Url::from_directory_path(path).unwrap().to_string()
    }

    /// Окружение запуска фиксируется один раз, а roots клиент строит заново
    /// на каждый запрос: сессия, сменившая каталог, видна только через них.
    #[test]
    fn first_client_root_outranks_stale_launch_environment_but_not_request_metadata() {
        let fixture = Fixture::new();
        let other = fixture.0.join("второй");
        std::fs::create_dir_all(&other).unwrap();
        let context = capture_with(
            &env(&[("CLAUDE_PROJECT_DIR", fixture.0.clone().into())]),
            Err("no cwd".into()),
        );
        let listed = ClientRoots::Listed(vec![root_uri(&fixture.project()), root_uri(&other)]);

        let selected = context
            .resolve_request(&with_roots(Map::new(), listed.clone()))
            .unwrap();
        assert!(same_directory(Path::new(&selected), &fixture.project()));

        let selected = context
            .resolve_request(&with_roots(
                metadata(serde_json::json!({"sandboxCwd": other})),
                listed,
            ))
            .unwrap();
        assert!(same_directory(Path::new(&selected), &other));

        // Пустой список и сорванный обмен ничего не передают: остаётся запуск.
        for roots in [
            ClientRoots::Listed(Vec::new()),
            ClientRoots::Unavailable("timed out".into()),
            ClientRoots::NotDeclared,
        ] {
            assert_eq!(
                context
                    .resolve_request(&with_roots(Map::new(), roots))
                    .unwrap(),
                fixture.0.to_str().unwrap()
            );
        }
    }

    #[test]
    fn a_supplied_root_satisfies_required_context_and_a_malformed_one_never_falls_back() {
        let fixture = Fixture::new();
        let required = capture_with(
            &env(&[("UNICA_HOST_CONTEXT_REQUIRED", "1".into())]),
            Ok(fixture.0.clone()),
        );
        assert!(required.resolve(&Map::new()).is_err());
        let selected = required
            .resolve_request(&with_roots(
                Map::new(),
                ClientRoots::Listed(vec![root_uri(&fixture.project())]),
            ))
            .unwrap();
        assert!(same_directory(Path::new(&selected), &fixture.project()));

        let launch = HostWorkspaceContext::from_directory(fixture.0.clone());
        for root in [
            fixture.project().to_str().unwrap().to_owned(),
            "https://example.com/project".into(),
            "file://other-host/tmp".into(),
            root_uri(&fixture.0.join("not-created")),
            format!("{}?query", root_uri(&fixture.project())),
        ] {
            assert!(
                launch
                    .resolve_request(&with_roots(
                        Map::new(),
                        ClientRoots::Listed(vec![root.clone()])
                    ))
                    .is_err(),
                "{root}"
            );
        }
    }

    /// Проект на сетевом ресурсе Windows приходит root-ом с сервером в
    /// роли хоста URI; на других ОС такой URI локальной папки не называет.
    #[test]
    fn a_root_with_a_host_names_a_share_only_where_the_platform_has_shares() {
        let parsed = parse_root_uri("file://server/share/%D0%BF%D1%80%D0%BE%D0%B5%D0%BA%D1%82");
        assert_eq!(
            parsed.is_ok(),
            crate::platform::file_uri_host_names_share(),
            "{parsed:?}"
        );
        if let Ok(path) = parsed {
            let text = path.to_str().unwrap();
            assert!(text.starts_with(r"\\server\share"), "{text}");
            assert!(text.ends_with("проект"), "{text}");
        }
    }

    #[test]
    fn only_a_call_without_host_metadata_asks_for_client_roots() {
        assert!(request_needs_client_roots(&Map::new()));
        assert!(!request_needs_client_roots(&metadata(
            serde_json::json!({"sandboxCwd": "/tmp"})
        )));
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
        for values in [
            vec![("ZCODE_PROJECT_DIR", fixture.0.clone().into())],
            vec![("CLAUDE_PROJECT_DIR", fixture.0.clone().into())],
            vec![
                ("ZCODE_PROJECT_DIR", fixture.0.clone().into()),
                ("CLAUDE_PROJECT_DIR", fixture.0.clone().into()),
            ],
            vec![
                (
                    "ZCODE_PROJECT_DIR",
                    fixture.0.join("Проект с пробелом/..").into(),
                ),
                ("CLAUDE_PROJECT_DIR", fixture.0.clone().into()),
            ],
        ] {
            let selected = capture_with(&env(&values), Err("no cwd".into()))
                .resolve(&Map::new())
                .unwrap();
            assert_eq!(Path::new(&selected).canonicalize().unwrap(), fixture.0);
        }
        let conflicting = [
            ("ZCODE_PROJECT_DIR", fixture.0.clone().into()),
            ("CLAUDE_PROJECT_DIR", fixture.project().into()),
        ];
        let context = capture_with(&env(&conflicting), Ok(fixture.0.clone()));
        let error = context.resolve(&Map::new()).unwrap_err();
        assert!(error.contains("conflicting directories"), "{error}");
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
        for key in ["CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR"] {
            for value in [
                OsString::new(),
                "relative".into(),
                "${ZCODE_PROJECT_DIR}".into(),
                fixture.0.join("bad\npath").into(),
                fixture.0.join("bad\0path").into(),
            ] {
                let context = capture_with(&env(&[(key, value)]), Ok(fixture.0.clone()));
                assert!(context.resolve(&Map::new()).is_err());
            }
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
