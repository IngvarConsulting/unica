//! Workspace context supplied by the host, independently of tool arguments.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::descriptor::KNOWN;

/// Captured launch context. Request metadata can override it without changing
/// the context of other calls sharing the same frontend.
#[derive(Clone, Debug)]
pub struct HostWorkspaceContext {
    launch_directory: Result<LaunchDirectory, LaunchFailure>,
    require_existing_directory: bool,
}

/// Startup capture is retained, but a valid per-call channel can override it.
#[derive(Clone, Debug)]
enum LaunchFailure {
    MissingRequired,
    Captured(String),
}

impl From<String> for LaunchFailure {
    fn from(message: String) -> Self {
        Self::Captured(message)
    }
}

impl LaunchFailure {
    fn describe(&self, request: &HostRequest) -> String {
        let variables = host_workspace_environment_keys().join(", ");
        let metadata = KNOWN
            .iter()
            .filter_map(|host| host.workspace_metadata)
            .map(|channel| format!("{} ({})", channel.capability, channel.directory_field))
            .collect::<Vec<_>>()
            .join(", ");
        let roots = match &request.roots {
            ClientRoots::NotDeclared => "not supplied for this call",
            ClientRoots::Listed(_) => "empty",
            ClientRoots::Unavailable(RootsFailure::Error) => "roots/list error",
            ClientRoots::Unavailable(RootsFailure::Timeout) => "roots/list timeout",
            ClientRoots::Unavailable(RootsFailure::Closed) => "roots/list transport closed",
        };
        let reason = match self {
            Self::MissingRequired => String::from("No host workspace context for this call: no startup project was captured. This connection requires host context (UNICA_HOST_CONTEXT_REQUIRED=1 or packaged runtime); launch cwd is not an allowed fallback."),
            Self::Captured(message) => format!("The startup workspace context captured for this MCP connection is invalid: {message}."),
        };
        format!("{reason} No recognized request metadata ({metadata}); client roots: {roots}. Project variables ({variables}) are captured once: set one to an existing absolute project directory and reconnect the MCP server from that project. The host can restore a later call without reconnecting by supplying valid recognized metadata or client roots when this session supports roots/list. Startup capture stays unchanged until reconnect.")
    }
}

/// The launch context and the project variable that named it, if any.
#[derive(Clone, Debug)]
struct LaunchDirectory {
    path: PathBuf,
    variable: Option<&'static str>,
}

/// How a `roots/list` exchange failed. A closed set: the client's own error
/// text is neither carried to the daemon nor shown in a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RootsFailure {
    Error,
    Timeout,
    Closed,
}

/// Why a client that declared roots still left the launch context in charge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RootsFallback {
    /// The client listed no roots.
    Empty,
    Error,
    Timeout,
    Closed,
}

/// Which channel chose the directory a call was resolved to.
///
/// `selected` says the model chose the directory with the `workspace`
/// argument inside the set the channel allows; `available` lists that set
/// for `unica.view {}`, so the model learns what it may choose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "channel",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkspaceOrigin {
    /// Per-call host metadata under this capability.
    RequestMetadata {
        capability: String,
        #[serde(default, skip_serializing_if = "is_false")]
        selected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available: Option<Vec<AllowedDirectory>>,
    },
    /// A root the client listed for this call: the first one, or the one
    /// holding the selected directory.
    ClientRoots {
        root: String,
        #[serde(default, skip_serializing_if = "is_false")]
        selected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available: Option<Vec<AllowedDirectory>>,
    },
    /// The project variable captured when the frontend started.
    StartupEnvironment {
        variable: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        roots: Option<RootsFallback>,
        #[serde(default, skip_serializing_if = "is_false")]
        selected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available: Option<Vec<AllowedDirectory>>,
    },
    /// The directory the frontend was started in.
    LaunchCwd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        roots: Option<RootsFallback>,
        #[serde(default, skip_serializing_if = "is_false")]
        selected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available: Option<Vec<AllowedDirectory>>,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One directory the host allows for this call, as `unica.view {}` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllowedDirectory {
    pub directory: String,
    /// `v8project.yaml` lies directly in the directory.
    pub project_file: bool,
    /// Subdirectories, relative and `/`-separated, that hold their own
    /// `v8project.yaml`: workspaces the model may select inside this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
}

impl WorkspaceOrigin {
    /// Captured once at start, so it does not follow a session that later
    /// changes its project.
    pub fn can_be_stale(&self) -> bool {
        matches!(
            self,
            Self::StartupEnvironment { .. } | Self::LaunchCwd { .. }
        )
    }

    fn choose(&mut self, explicit: bool, listed: Option<Vec<AllowedDirectory>>) {
        let (Self::RequestMetadata {
            selected,
            available,
            ..
        }
        | Self::ClientRoots {
            selected,
            available,
            ..
        }
        | Self::StartupEnvironment {
            selected,
            available,
            ..
        }
        | Self::LaunchCwd {
            selected,
            available,
            ..
        }) = self;
        *selected = explicit;
        *available = listed;
    }
}

/// The directory of one call and the channel that chose it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedWorkspace {
    pub directory: String,
    pub origin: WorkspaceOrigin,
}

impl ResolvedWorkspace {
    /// A directory the process was started in, with no host channel: direct
    /// invocations and fixtures that name their workspace themselves.
    pub fn launch_cwd(directory: impl Into<String>) -> Self {
        Self {
            directory: directory.into(),
            origin: WorkspaceOrigin::LaunchCwd {
                roots: None,
                selected: false,
                available: None,
            },
        }
    }
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
    Unavailable(RootsFailure),
}

/// Host context of one tool call: wire metadata and the client's roots.
#[derive(Clone, Debug, Default)]
pub struct HostRequest {
    pub metadata: Map<String, Value>,
    pub roots: ClientRoots,
    /// Сколько байтов ответа этот клиент передаёт модели целиком.
    pub response_budget: super::ResponseBudget,
    /// The `workspace` argument of the call: a directory the model chose
    /// inside the set the host allows. The door takes it off the arguments.
    pub selection: Option<String>,
    /// List the allowed set in the origin (`unica.view {}`).
    pub list_allowed: bool,
}

impl HostRequest {
    pub fn from_metadata(metadata: Map<String, Value>) -> Self {
        Self {
            metadata,
            ..Self::default()
        }
    }
}

/// Name of the tool argument that selects a directory inside the host's set.
pub const WORKSPACE_ARGUMENT: &str = "workspace";

/// The workspace root of `directory`: the nearest ancestor (the directory
/// itself included) holding `v8project.yaml`, or a linked worktree's `.git`
/// file, which bounds the search. The one owner of this walk: the daemon
/// discovers workspaces with it, the door bounds a selection with it.
pub fn find_workspace_root(directory: &Path) -> Option<PathBuf> {
    directory
        .ancestors()
        .find(|base| is_workspace_root(base))
        .map(Path::to_path_buf)
}

fn is_workspace_root(base: &Path) -> bool {
    base.join("v8project.yaml").is_file() || base.join(".git").is_file()
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
            launch_directory: Ok(LaunchDirectory {
                path: directory,
                variable: None,
            }),
            require_existing_directory: false,
        }
    }

    /// Resolve only this request's workspace. A malformed supplied context is
    /// an error, never permission to fall back to another project's directory.
    pub fn resolve(&self, metadata: &Map<String, Value>) -> Result<String, String> {
        self.resolve_request(&HostRequest::from_metadata(metadata.clone()))
            .map(|resolved| resolved.directory)
    }

    /// The channel decides the set of directories this call may use: request
    /// metadata and the launch context allow one, client roots allow all
    /// listed roots. Without a selection the call uses the first of the set,
    /// as before; with one, the selected directory must exist and lie inside
    /// a member of the set after symlinks and junctions are resolved. A
    /// selection outside the set is refused, never replaced by another
    /// directory.
    pub fn resolve_request(&self, request: &HostRequest) -> Result<ResolvedWorkspace, String> {
        let (allowed, mut origin) = self.allowed_set(request)?;
        let listed = request.list_allowed.then(|| list_allowed(&allowed));
        let Some(selection) = request.selection.as_deref() else {
            origin.choose(false, listed);
            return Ok(ResolvedWorkspace {
                directory: allowed[0].clone(),
                origin,
            });
        };
        let (directory, index) = select_directory(&allowed, selection)?;
        if let WorkspaceOrigin::ClientRoots { root, .. } = &mut origin {
            root.clone_from(&allowed[index]);
        }
        origin.choose(true, listed);
        Ok(ResolvedWorkspace { directory, origin })
    }

    /// The set of directories the host allows for this call, first the one a
    /// call without selection uses, and the channel that supplied it.
    fn allowed_set(&self, request: &HostRequest) -> Result<(Vec<String>, WorkspaceOrigin), String> {
        let metadata = &request.metadata;
        let mut request_directory: Option<(PathBuf, &'static str)> = None;
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
                    .is_some_and(|(previous, _)| !same_directory(previous, &directory))
                {
                    return Err(
                        "Host workspace metadata declares conflicting directories".to_owned()
                    );
                }
                request_directory.get_or_insert((directory, channel.capability));
            }
        }
        if let Some((directory, capability)) = request_directory {
            return Ok((
                vec![absolute_directory(&directory)?],
                WorkspaceOrigin::RequestMetadata {
                    capability: capability.to_owned(),
                    selected: false,
                    available: None,
                },
            ));
        }
        let roots =
            match &request.roots {
                ClientRoots::Listed(roots) => match roots.split_first() {
                    Some((first, others)) => {
                        // The first root is the session's project: a malformed
                        // one is refused as before. Another root that is not a
                        // usable directory cannot be selected and is left out.
                        let first = existing_directory(&parse_root_uri(first)?)?;
                        let mut allowed = vec![first.clone()];
                        allowed.extend(others.iter().filter_map(|root| {
                            existing_directory(&parse_root_uri(root).ok()?).ok()
                        }));
                        return Ok((
                            allowed,
                            WorkspaceOrigin::ClientRoots {
                                root: first,
                                selected: false,
                                available: None,
                            },
                        ));
                    }
                    None => Some(RootsFallback::Empty),
                },
                ClientRoots::Unavailable(failure) => Some(match failure {
                    RootsFailure::Error => RootsFallback::Error,
                    RootsFailure::Timeout => RootsFallback::Timeout,
                    RootsFailure::Closed => RootsFallback::Closed,
                }),
                ClientRoots::NotDeclared => None,
            };
        let launch = self
            .launch_directory
            .as_ref()
            .map_err(|failure| failure.describe(request))?;
        let directory = if self.require_existing_directory {
            existing_directory(&launch.path)?
        } else {
            absolute_directory(&launch.path)?
        };
        let origin = match launch.variable {
            Some(variable) => WorkspaceOrigin::StartupEnvironment {
                variable: variable.to_owned(),
                roots,
                selected: false,
                available: None,
            },
            None => WorkspaceOrigin::LaunchCwd {
                roots,
                selected: false,
                available: None,
            },
        };
        Ok((vec![directory], origin))
    }
}

/// The directory a selection names and the index of the allowed directory
/// holding it. Checked on the resolved path: a symlink or junction inside an
/// allowed directory that leads outside it does not pass. The daemon gets the
/// resolved path too, so what was checked is what it reads.
fn select_directory(allowed: &[String], selection: &str) -> Result<(String, usize), String> {
    validate_path_text(selection)?;
    let candidate = if selection.starts_with("file:") {
        parse_root_uri(selection)?
    } else {
        // An absolute path stays itself; a relative one is read from the
        // first allowed directory, the one a call without selection uses.
        Path::new(&allowed[0]).join(selection)
    };
    let canonical = candidate.canonicalize().map_err(|error| {
        format!("Selected workspace `{selection}` is not an accessible directory: {error}")
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "Selected workspace `{selection}` is not a directory"
        ));
    }
    let containing = allowed
        .iter()
        .enumerate()
        .filter_map(|(index, directory)| {
            let root = Path::new(directory).canonicalize().ok()?;
            canonical.starts_with(&root).then_some((index, root))
        })
        // Nested allowed directories: the innermost one holds the selection.
        .max_by_key(|(_, root)| root.components().count());
    let Some((index, root)) = containing else {
        return Err(format!(
            "Selected workspace `{selection}` is outside the directories the host allows for this call ({}). Choose one of them or a directory inside one; unica.view {{}} lists them.",
            allowed.join("; ")
        ));
    };
    // The project file of the selection must not come from above the allowed
    // directory: the person did not open that directory to the agent.
    let within = canonical
        .ancestors()
        .take_while(|base| base.starts_with(&root))
        .any(is_workspace_root);
    if !within {
        if let Some(outer) = root.parent().and_then(find_workspace_root) {
            if outer.join("v8project.yaml").is_file() {
                return Err(format!(
                    "Selected workspace `{selection}` has no v8project.yaml up to its allowed directory {}; the nearest one lies above it, in {}, which the host has not allowed. Select a directory with its own v8project.yaml, or ask the person to open {} to the session.",
                    allowed[index],
                    display_path(&outer),
                    display_path(&outer)
                ));
            }
        }
    }
    let directory = display_path(&canonical);
    validate_path_text(&directory)?;
    Ok((directory, index))
}

/// A resolved path without the Windows verbatim prefix, which tools that
/// read the workspace (the platform among them) do not accept.
fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

// resource-limit: host.allowed-workspace-scan
// class: count
// unit: directories
// scope: request
// value: 2000
// resource: Сколько каталогов читает перечень разрешённых папок `unica.view {}` в поиске вложенных v8project.yaml.
// status: unproven
// origin: прототип IngvarConsulting/unica#1420
// problem: Перечень обходит диск на каждый `unica.view {}`; большая или сетевая папка не должна его задерживать.
// on-reach: Поиск вложенных рабочих областей останавливается; найденное до предела выдаётся.
const ALLOWED_SCAN_DIRECTORIES: usize = 2000;
const ALLOWED_SCAN_DEPTH: usize = 2;
const ALLOWED_SCAN_SKIPPED: &[&str] = &["node_modules", "target"];

/// The allowed set as `unica.view {}` shows it: each directory, whether it
/// holds `v8project.yaml`, and its subdirectories that hold their own.
fn list_allowed(allowed: &[String]) -> Vec<AllowedDirectory> {
    allowed
        .iter()
        .map(|directory| {
            let root = Path::new(directory);
            let mut workspaces = Vec::new();
            let mut budget = ALLOWED_SCAN_DIRECTORIES;
            let mut level = vec![root.to_path_buf()];
            for _ in 0..ALLOWED_SCAN_DEPTH {
                let mut next = Vec::new();
                for parent in &level {
                    let Ok(entries) = std::fs::read_dir(parent) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        if budget == 0 {
                            break;
                        }
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if name.starts_with('.') || ALLOWED_SCAN_SKIPPED.contains(&name.as_ref()) {
                            continue;
                        }
                        // A symlink or junction is not followed: it may lead
                        // outside the allowed directory.
                        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                            continue;
                        }
                        budget -= 1;
                        let path = entry.path();
                        if path.join("v8project.yaml").is_file() {
                            if let Ok(relative) = path.strip_prefix(root) {
                                workspaces.push(
                                    relative
                                        .components()
                                        .map(|part| part.as_os_str().to_string_lossy())
                                        .collect::<Vec<_>>()
                                        .join("/"),
                                );
                            }
                        }
                        next.push(path);
                    }
                }
                level = next;
            }
            workspaces.sort();
            AllowedDirectory {
                directory: directory.clone(),
                project_file: root.join("v8project.yaml").is_file(),
                workspaces,
            }
        })
        .collect()
}

fn capture_with(
    read_env: &dyn Fn(&str) -> Option<OsString>,
    cwd: Result<PathBuf, String>,
) -> HostWorkspaceContext {
    let launch_directory = (|| {
        let mut selected: Option<LaunchDirectory> = None;
        for key in host_workspace_environment_keys() {
            if let Some(value) = read_env(key) {
                let value = value
                    .to_str()
                    .ok_or_else(|| format!("Host project environment {key} is not Unicode"))?;
                let directory = PathBuf::from(existing_directory(Path::new(value))?);
                if selected
                    .as_ref()
                    .is_some_and(|previous| !same_directory(&previous.path, &directory))
                {
                    return Err(LaunchFailure::Captured(
                        "Host project environment declares conflicting directories".to_owned(),
                    ));
                }
                if selected.is_none() {
                    selected = Some(LaunchDirectory {
                        path: directory,
                        variable: Some(key),
                    });
                }
            }
        }
        if let Some(directory) = selected {
            return Ok(directory);
        }
        let required = read_env("UNICA_HOST_CONTEXT_REQUIRED").is_some_and(|v| v == "1")
            || read_env("UNICA_RUNTIME_MANIFEST").is_some_and(|v| !v.is_empty());
        if required {
            return Err(LaunchFailure::MissingRequired);
        }
        cwd.map(|path| LaunchDirectory {
            path,
            variable: None,
        })
        .map_err(LaunchFailure::Captured)
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
        HostRequest {
            metadata,
            roots,
            ..HostRequest::default()
        }
    }

    fn root_uri(path: &Path) -> String {
        url::Url::from_directory_path(path).unwrap().to_string()
    }

    /// Окружение запуска фиксируется один раз, а roots клиент строит заново
    /// на каждый запрос: сессия, сменившая каталог, видна только через них.
    /// rule: INV.WORKSPACE.HOST-CONTEXT
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
        assert!(same_directory(
            Path::new(&selected.directory),
            &fixture.project()
        ));
        assert!(matches!(
            &selected.origin,
            WorkspaceOrigin::ClientRoots { root, selected: false, available: None }
                if same_directory(Path::new(root), &fixture.project())
        ));

        let selected = context
            .resolve_request(&with_roots(
                metadata(serde_json::json!({"sandboxCwd": other})),
                listed,
            ))
            .unwrap();
        assert!(same_directory(Path::new(&selected.directory), &other));
        assert_eq!(
            selected.origin,
            WorkspaceOrigin::RequestMetadata {
                capability: "codex/sandbox-state-meta".to_owned(),
                selected: false,
                available: None,
            }
        );

        // Пустой список и сорванный обмен ничего не передают: остаётся запуск,
        // но ответ помнит, что roots были объявлены и почему не сработали.
        for (roots, fallback) in [
            (ClientRoots::Listed(Vec::new()), Some(RootsFallback::Empty)),
            (
                ClientRoots::Unavailable(RootsFailure::Timeout),
                Some(RootsFallback::Timeout),
            ),
            (ClientRoots::NotDeclared, None),
        ] {
            let resolved = context
                .resolve_request(&with_roots(Map::new(), roots))
                .unwrap();
            assert_eq!(resolved.directory, fixture.0.to_str().unwrap());
            assert_eq!(
                resolved.origin,
                WorkspaceOrigin::StartupEnvironment {
                    variable: "CLAUDE_PROJECT_DIR".to_owned(),
                    roots: fallback,
                    selected: false,
                    available: None,
                }
            );
            assert!(resolved.origin.can_be_stale());
        }
        let launch_cwd = capture_with(&env(&[]), Ok(fixture.0.clone()))
            .resolve_request(&HostRequest::default())
            .unwrap();
        assert_eq!(
            launch_cwd.origin,
            WorkspaceOrigin::LaunchCwd {
                roots: None,
                selected: false,
                available: None,
            }
        );
        assert_eq!(
            serde_json::to_value(&launch_cwd.origin).unwrap(),
            serde_json::json!({"channel": "launchCwd"})
        );
    }

    /// rule: INV.WORKSPACE.HOST-CONTEXT
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
        assert!(same_directory(
            Path::new(&selected.directory),
            &fixture.project()
        ));

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

    #[test]
    fn missing_launch_context_names_channels_and_recovers_without_reusing_a_previous_project() {
        let fixture = Fixture::new();
        let context = capture_with(
            &env(&[("UNICA_HOST_CONTEXT_REQUIRED", "1".into())]),
            Ok(fixture.0.clone()),
        );
        let missing = context
            .resolve_request(&HostRequest::default())
            .unwrap_err();
        for channel in [
            "CLAUDE_PROJECT_DIR",
            "ZCODE_PROJECT_DIR",
            "codex/sandbox-state-meta",
            "sandboxCwd",
            "roots",
            "reconnect",
        ] {
            assert!(
                missing.contains(channel),
                "missing recovery channel {channel}: {missing}"
            );
        }
        let selected = context
            .resolve_request(&HostRequest::from_metadata(metadata(
                serde_json::json!({"sandboxCwd":fixture.project()}),
            )))
            .unwrap();
        assert!(same_directory(
            Path::new(&selected.directory),
            &fixture.project()
        ));
        assert_eq!(
            context
                .resolve_request(&HostRequest::default())
                .unwrap_err(),
            missing
        );
        let selected = context
            .resolve_request(&with_roots(
                Map::new(),
                ClientRoots::Listed(vec![root_uri(&fixture.project())]),
            ))
            .unwrap();
        assert!(matches!(
            selected.origin,
            WorkspaceOrigin::ClientRoots {
                selected: false,
                ..
            }
        ));
        assert_eq!(
            context
                .resolve_request(&HostRequest::default())
                .unwrap_err(),
            missing
        );
    }

    #[test]
    fn both_required_context_triggers_and_root_failures_report_their_actual_scope() {
        let fixture = Fixture::new();
        for trigger in ["UNICA_HOST_CONTEXT_REQUIRED", "UNICA_RUNTIME_MANIFEST"] {
            let context = capture_with(&env(&[(trigger, "1".into())]), Ok(fixture.0.clone()));
            for (roots, status) in [
                (ClientRoots::NotDeclared, "not supplied for this call"),
                (ClientRoots::Listed(Vec::new()), "empty"),
                (
                    ClientRoots::Unavailable(RootsFailure::Error),
                    "roots/list error",
                ),
                (
                    ClientRoots::Unavailable(RootsFailure::Timeout),
                    "roots/list timeout",
                ),
                (
                    ClientRoots::Unavailable(RootsFailure::Closed),
                    "roots/list transport closed",
                ),
            ] {
                let request = with_roots(
                    Map::from_iter([(
                        "unknown-host-context".into(),
                        serde_json::json!({"private":"must-not-leak"}),
                    )]),
                    roots,
                );
                let error = context.resolve_request(&request).unwrap_err();
                assert!(error.contains(status), "{error}");
                assert!(
                    !error.contains("must-not-leak") && !error.contains("unknown-host-context"),
                    "{error}"
                );
                assert!(matches!(
                    context.launch_directory,
                    Err(LaunchFailure::MissingRequired)
                ));
            }
        }
    }

    #[test]
    fn a_captured_startup_error_does_not_poison_future_metadata_or_roots() {
        let fixture = Fixture::new();
        let context = capture_with(
            &env(&[("ZCODE_PROJECT_DIR", "relative/project".into())]),
            Ok(fixture.0.clone()),
        );
        assert!(matches!(
            context.launch_directory,
            Err(LaunchFailure::Captured(_))
        ));
        let missing = context.resolve(&Map::new()).unwrap_err();
        assert!(missing.contains("startup workspace context"), "{missing}");
        assert!(missing.contains("must be absolute"), "{missing}");
        assert!(missing.contains("reconnect"), "{missing}");
        for channel in [
            "CLAUDE_PROJECT_DIR",
            "ZCODE_PROJECT_DIR",
            "codex/sandbox-state-meta",
            "sandboxCwd",
            "roots",
        ] {
            assert!(missing.contains(channel), "{missing}");
        }
        assert!(context
            .resolve(&metadata(
                serde_json::json!({"sandboxCwd":fixture.project()})
            ))
            .is_ok());
        assert_eq!(context.resolve(&Map::new()).unwrap_err(), missing);
        assert!(context
            .resolve_request(&with_roots(
                Map::new(),
                ClientRoots::Listed(vec![root_uri(&fixture.project())])
            ))
            .is_ok());
        assert_eq!(context.resolve(&Map::new()).unwrap_err(), missing);
    }

    #[test]
    fn a_later_launch_directory_failure_remains_recoverable_without_recapturing_startup() {
        let fixture = Fixture::new();
        let launch = fixture.0.join("launch");
        std::fs::create_dir(&launch).unwrap();
        let context = capture_with(
            &env(&[("CLAUDE_PROJECT_DIR", launch.clone().into())]),
            Err("no cwd".into()),
        );
        std::fs::remove_dir(&launch).unwrap();
        assert!(context.resolve(&Map::new()).is_err());
        assert!(context.launch_directory.is_ok());
        assert!(context
            .resolve(&metadata(
                serde_json::json!({"sandboxCwd":fixture.project()})
            ))
            .is_ok());
        std::fs::create_dir(&launch).unwrap();
        assert!(same_directory(
            Path::new(&context.resolve(&Map::new()).unwrap()),
            &launch
        ));
    }

    fn selecting(roots: ClientRoots, metadata: Map<String, Value>, selection: &str) -> HostRequest {
        HostRequest {
            metadata,
            roots,
            selection: Some(selection.to_owned()),
            ..HostRequest::default()
        }
    }

    /// A link inside an allowed directory that leads outside it: a junction
    /// on Windows, a symbolic link elsewhere.
    fn link_directory(link: &Path, target: &Path) {
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    /// Хост задаёт множество папок, модель выбирает внутри него: каждая папка
    /// из roots, вложенная в неё папка, но не папка вне множества и не выход
    /// наружу через ссылку. Канал с одной папкой разрешает только её.
    /// rule: INV.WORKSPACE.HOST-CONTEXT
    #[test]
    fn a_selection_stays_inside_the_set_the_channel_allows() {
        let fixture = Fixture::new();
        let first = fixture.project();
        let second = fixture.0.join("второй проект");
        let foreign = fixture.0.join("чужой");
        for directory in [&second, &foreign, &first.join("sub")] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let context = HostWorkspaceContext::from_directory(first.clone());
        let roots = ClientRoots::Listed(vec![root_uri(&first), root_uri(&second)]);

        for selection in [
            second.to_str().unwrap().to_owned(),
            root_uri(&second),
            "../второй проект".to_owned(),
        ] {
            let chosen = context
                .resolve_request(&selecting(roots.clone(), Map::new(), &selection))
                .unwrap();
            assert!(
                same_directory(Path::new(&chosen.directory), &second),
                "{selection}"
            );
            assert!(
                matches!(&chosen.origin, WorkspaceOrigin::ClientRoots { root, selected: true, .. }
                    if same_directory(Path::new(root), &second)),
                "{:?}",
                chosen.origin
            );
        }
        let refused = context
            .resolve_request(&selecting(
                roots.clone(),
                Map::new(),
                foreign.to_str().unwrap(),
            ))
            .unwrap_err();
        assert!(refused.contains("outside the directories"), "{refused}");
        assert!(context
            .resolve_request(&selecting(roots.clone(), Map::new(), "../чужой"))
            .is_err());

        // Codex names one directory per call: only it and its subdirectories.
        let codex = metadata(serde_json::json!({"sandboxCwd": first}));
        assert!(context
            .resolve_request(&selecting(
                roots.clone(),
                codex.clone(),
                second.to_str().unwrap()
            ))
            .is_err());
        let inner = context
            .resolve_request(&selecting(roots.clone(), codex, "sub"))
            .unwrap();
        assert!(same_directory(
            Path::new(&inner.directory),
            &first.join("sub")
        ));
        // The startup project alone, when the client lists no roots.
        assert!(context
            .resolve_request(&selecting(
                ClientRoots::NotDeclared,
                Map::new(),
                second.to_str().unwrap()
            ))
            .is_err());

        link_directory(&first.join("ссылка"), &foreign);
        let escaped = context
            .resolve_request(&selecting(roots.clone(), Map::new(), "ссылка"))
            .unwrap_err();
        assert!(escaped.contains("outside the directories"), "{escaped}");
    }

    /// Выбранная папка не получает файл проекта из каталога выше разрешённого:
    /// человек его агенту не открывал. Граница worktree действует как у демона.
    /// rule: INV.WORKSPACE.HOST-CONTEXT
    #[test]
    fn a_selected_workspace_does_not_borrow_a_project_file_above_its_root() {
        let fixture = Fixture::new();
        let root = fixture.project();
        let nested = root.join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(fixture.0.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        let context = HostWorkspaceContext::from_directory(root.clone());
        let roots = ClientRoots::Listed(vec![root_uri(&root)]);

        let refused = context
            .resolve_request(&selecting(roots.clone(), Map::new(), "sub"))
            .unwrap_err();
        assert!(refused.contains("lies above"), "{refused}");
        // Without a selection the session keeps today's discovery.
        assert!(context
            .resolve_request(&with_roots(Map::new(), roots.clone()))
            .is_ok());

        for (marker, contents) in [
            (".git", "gitdir: elsewhere\n"),
            ("v8project.yaml", "format: DESIGNER\n"),
        ] {
            std::fs::write(nested.join(marker), contents).unwrap();
            assert!(
                context
                    .resolve_request(&selecting(roots.clone(), Map::new(), "sub"))
                    .is_ok(),
                "{marker}"
            );
            std::fs::remove_file(nested.join(marker)).unwrap();
        }
    }

    /// `unica.view {}` lists every allowed directory and the workspaces it
    /// holds, without following links or entering hidden directories.
    #[test]
    fn the_allowed_set_names_project_files_and_nested_workspaces() {
        let fixture = Fixture::new();
        let repository = fixture.project();
        let other = fixture.0.join("other");
        for (directory, project) in [
            (repository.join("smb"), true),
            (repository.join("acc/main"), true),
            (repository.join(".hidden/x"), true),
            (repository.join("docs"), false),
            (other.clone(), true),
        ] {
            std::fs::create_dir_all(&directory).unwrap();
            if project {
                std::fs::write(directory.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
            }
        }
        let request = HostRequest {
            roots: ClientRoots::Listed(vec![root_uri(&repository), root_uri(&other)]),
            list_allowed: true,
            ..HostRequest::default()
        };
        let resolved = HostWorkspaceContext::from_directory(repository.clone())
            .resolve_request(&request)
            .unwrap();
        let WorkspaceOrigin::ClientRoots {
            available: Some(available),
            selected: false,
            ..
        } = resolved.origin
        else {
            panic!("{:?}", resolved.origin)
        };
        assert_eq!(available.len(), 2);
        assert!(!available[0].project_file);
        assert_eq!(available[0].workspaces, vec!["acc/main", "smb"]);
        assert!(available[1].project_file);
        assert!(available[1].workspaces.is_empty());
    }
}
