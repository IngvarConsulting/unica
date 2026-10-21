use crate::environment::BslEnvironment;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use unica_analysis_services::persistent_provider_working_directory;
use unica_process::mcp_session::PersistentMcpSession;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_workspace::{paths::normalize_path_identity, state_fs::short_private_runtime_dir};
const PROVIDER_STATE_KEY_DOMAIN: &[u8] = b"unica-provider-state-v1\0";
/// Keep the analyzer's Unix socket names within the platform path budget.
pub fn configure_bsl_analyzer_runtime_dir(command: &mut Command) -> Result<(), String> {
    if let Some(runtime_dir) = short_private_runtime_dir().map_err(|error| {
        format!("failed to prepare short runtime directory for bsl-analyzer: {error}")
    })? {
        command.env("XDG_RUNTIME_DIR", &runtime_dir);
    }
    Ok(())
}

pub fn bsl_analyzer_command(
    program: &Path,
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<Command, String> {
    let source_root = normalize_path_identity(source_root)?;
    let cache_dir = bsl_analyzer_cache_dir(context, &source_root)?;
    let mut command = Command::new(program);
    command
        .arg("mcp")
        .arg("serve")
        .arg("--profile")
        .arg("workspace")
        .arg("--source-dir")
        .arg(&source_root)
        .arg("--cache-dir")
        .arg(cache_dir)
        .arg("--mode")
        .arg("stdio")
        // Workspace profile otherwise promotes stdio to a persistent broker.
        // The helper must own the analyzer it retires after invalidation.
        .env("BSL_MCP_BROKER", "0")
        .current_dir(persistent_provider_working_directory(
            &context.workspace_root,
            &source_root,
        )?);
    configure_bsl_analyzer_runtime_dir(&mut command)?;
    Ok(command)
}

pub fn provider_state_key(workspace_root: &Path, source_root: &Path) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(PROVIDER_STATE_KEY_DOMAIN);
    digest.update(unica_workspace::paths::stable_path_identity_bytes(
        workspace_root,
    )?);
    digest.update([0]);
    digest.update(unica_workspace::paths::stable_path_identity_bytes(
        source_root,
    )?);
    Ok(format!("source-{:x}", digest.finalize()))
}

pub fn bsl_analyzer_cache_dir(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    let workspace_root = normalize_path_identity(&context.workspace_root)?;
    let source_root = normalize_path_identity(source_root)?;
    let cache_root = normalize_path_identity(&context.cache_root)?;
    let key = provider_state_key(&workspace_root, &source_root)?;
    let preferred = cache_root
        .join("providers")
        .join("bsl-analyzer")
        .join("owned-stdio")
        .join(&key);
    let preferred = normalize_path_identity(&preferred)?;
    if !unica_workspace::paths::path_starts_with_host_root(&preferred, &source_root) {
        return Ok(preferred);
    }

    // The default cache root lives below the workspace. For sourceDir = "."
    // that would put analyzer databases back inside the indexed source tree,
    // recreating the defect this path is meant to prevent. Preserve the same
    // stable provider key, but place this exceptional case under the private
    // runtime root already used by the analyzer transport.
    let external_root = short_private_runtime_dir()
        .map_err(|error| {
            format!("failed to prepare external bsl-analyzer cache directory: {error}")
        })?
        .unwrap_or_else(|| std::env::temp_dir().join("unica-bsl"));
    let external = external_root
        .join("cache")
        .join("providers")
        .join("bsl-analyzer")
        .join("owned-stdio")
        .join(key);
    let external = normalize_path_identity(&external)?;
    if unica_workspace::paths::path_starts_with_host_root(&external, &source_root) {
        return Err(
            "failed to place bsl-analyzer cache outside the indexed source tree".to_string(),
        );
    }
    Ok(external)
}

fn start_bsl_transport(
    context: &WorkspaceContext,
    source_root: &Path,
    cancellation: &CancellationToken,
    environment: &dyn BslEnvironment,
) -> Result<PersistentMcpSession, String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error("persistent MCP start stopped"));
    }
    let plugin_root = environment.find_plugin_root(&context.cwd).ok_or_else(|| {
        "could not locate Unica plugin root for workspace bsl-analyzer service".to_string()
    })?;
    let program = environment
        .resolve_bundled_tool(&plugin_root, "bsl-analyzer", true)?
        .program;
    let command = bsl_analyzer_command(&program, context, source_root)?;
    PersistentMcpSession::start_with_command(command, cancellation)
}

pub struct BslWorkspaceBackend {
    environment: std::sync::Arc<dyn BslEnvironment>,
}
impl BslWorkspaceBackend {
    pub fn new(environment: std::sync::Arc<dyn BslEnvironment>) -> Self {
        Self { environment }
    }
}
impl unica_analysis_services::backend::BslBackend for BslWorkspaceBackend {
    fn start(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancel: &CancellationToken,
    ) -> Result<PersistentMcpSession, String> {
        start_bsl_transport(context, source_root, cancel, self.environment.as_ref())
    }
    fn report_failure(&self, snapshot: unica_process::transport_failure::PrivateFailureSnapshot) {
        let stderr = std::io::stderr();
        let _ = crate::private_failure::write_record_to(&mut stderr.lock(), snapshot);
    }
}
