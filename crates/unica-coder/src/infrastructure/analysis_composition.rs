//! Trusted provider bindings and the captured executable build identity.
use super::{bundled_tools, plugin_runtime, source_roots, source_workspace, workspace};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unica_analysis_services::backend::ServiceEnvironment;
use unica_artifact_delivery::engine::{BundledTool, BundledToolError, MissingEngine};
use unica_code_intelligence::source_locations::SearchSourceBindings;
use unica_daemon::identity::{verify_executable_build, CoreIdentity};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_source_contract::workspace::{ResolvedSourceRoot, WorkspaceContext};
use unica_workspace::policies::WorkspacePolicies;

struct CapturedServiceEnvironment {
    build: CoreIdentity,
}
impl ServiceEnvironment for CapturedServiceEnvironment {
    fn core_identity(&self) -> &str {
        self.build.as_str()
    }
    fn verify_executable_build(&self, executable: &Path) -> Result<(), String> {
        verify_executable_build(executable, &self.build)
    }
    fn find_plugin_root(&self, cwd: &Path) -> Option<PathBuf> {
        plugin_runtime::find_plugin_root(cwd)
    }
    fn workspace_policies(&self) -> WorkspacePolicies {
        source_workspace::policies()
    }
}
pub(crate) fn service_environment() -> Arc<dyn ServiceEnvironment> {
    Arc::new(CapturedServiceEnvironment {
        build: CoreIdentity::production(),
    })
}
pub(crate) fn service_manager() -> unica_analysis_services::WorkspaceServiceManager<'static> {
    unica_analysis_services::WorkspaceServiceManager::new(service_environment())
}
pub(crate) struct AnalysisEnvironment;
static ANALYSIS_ENVIRONMENT: AnalysisEnvironment = AnalysisEnvironment;
pub(crate) fn environment() -> &'static AnalysisEnvironment {
    &ANALYSIS_ENVIRONMENT
}
impl unica_adapter_rlm::environment::RlmEnvironment for AnalysisEnvironment {
    fn find_plugin_root(&self, cwd: &Path) -> Option<PathBuf> {
        plugin_runtime::find_plugin_root(cwd)
    }
    fn resolve_bundled_tool(
        &self,
        root: &Path,
        tool: &str,
        allow_download: bool,
    ) -> Result<BundledTool, BundledToolError> {
        bundled_tools::resolve_bundled_tool(root, tool, allow_download)
    }
    fn resolve_source_root(
        &self,
        context: &WorkspaceContext,
    ) -> Result<ResolvedSourceRoot, String> {
        source_roots::resolve_source_root(context, None)
    }
}
impl unica_adapter_bsl_analyzer::environment::BslEnvironment for AnalysisEnvironment {
    fn find_plugin_root(&self, cwd: &Path) -> Option<PathBuf> {
        plugin_runtime::find_plugin_root(cwd)
    }
    fn resolve_bundled_tool(
        &self,
        root: &Path,
        tool: &str,
        allow_download: bool,
    ) -> Result<BundledTool, BundledToolError> {
        bundled_tools::resolve_bundled_tool(root, tool, allow_download)
    }
    fn missing_engine(&self, root: &Path, tool: &str) -> Option<MissingEngine> {
        bundled_tools::missing_engine(root, tool)
    }
    fn bundled_tool_version(&self, root: &Path, tool: &str) -> Result<String, String> {
        bundled_tools::bundled_tool_version(root, tool)
    }
    fn resolve_source_root(
        &self,
        context: &WorkspaceContext,
        explicit: Option<&str>,
    ) -> Result<ResolvedSourceRoot, String> {
        source_roots::resolve_source_root(context, explicit)
    }
    fn discover_workspace(&self, cwd: Option<PathBuf>) -> Result<WorkspaceContext, String> {
        workspace::discover_workspace(cwd)
    }
    fn service_environment(&self) -> Arc<dyn ServiceEnvironment> {
        service_environment()
    }
    fn dry_run_outcome(
        &self,
        tool_name: &str,
        label: &str,
        mutating: bool,
        bundled_tool: BundledTool,
        command: Vec<String>,
    ) -> AdapterOutcome {
        super::internal_adapters::dry_run_outcome(tool_name, label, mutating, bundled_tool, command)
    }
    fn cli_args(&self, args: &Map<String, Value>, redact: bool) -> Result<Vec<String>, String> {
        super::internal_adapters::cli_args(args, redact)
    }
}

pub(crate) fn source_bindings() -> SearchSourceBindings {
    SearchSourceBindings {
        resolver: super::platform_xml_source_targets::locate_platform_xml_source_path,
        engine_absent: provider_engine_absent,
    }
}
fn provider_engine_absent(cwd: &Path, tool: &str) -> bool {
    match plugin_runtime::find_plugin_root(cwd) {
        None => true,
        Some(plugin_root) => bundled_tools::engine_absence(&plugin_root, tool).is_some(),
    }
}
static BSL_CLIENT: unica_adapter_bsl_analyzer::provider::WorkspaceBslToolClient =
    unica_adapter_bsl_analyzer::provider::WorkspaceBslToolClient {
        environment: service_environment,
    };
static RLM_SEARCH_CLIENT: unica_adapter_rlm::provider::WorkspaceRlmSearchClient =
    unica_adapter_rlm::provider::WorkspaceRlmSearchClient {
        environment: service_environment,
    };
static RLM_NAV_CLIENT: unica_adapter_rlm::navigation::WorkspaceRlmNavigationClient =
    unica_adapter_rlm::navigation::WorkspaceRlmNavigationClient {
        environment: service_environment,
    };
static BSL_DIAGNOSTIC_BACKEND:
    unica_adapter_bsl_analyzer::diagnostics::WorkspaceBslDiagnosticBackend =
    unica_adapter_bsl_analyzer::diagnostics::WorkspaceBslDiagnosticBackend {
        environment: &ANALYSIS_ENVIRONMENT,
    };
pub(crate) fn bsl_provider() -> unica_adapter_bsl_analyzer::provider::BslAnalyzerProvider<'static> {
    unica_adapter_bsl_analyzer::provider::BslAnalyzerProvider::new(&BSL_CLIENT, source_bindings())
}
/// The analyzer behind the symbol port for one source set.
pub(crate) fn bsl_symbols(
    context: unica_code_intelligence::model::CodeIntelligenceContext,
) -> unica_adapter_bsl_analyzer::symbols::BslAnalyzerSymbols<'static> {
    unica_adapter_bsl_analyzer::symbols::BslAnalyzerSymbols::new(&BSL_CLIENT, context)
}
pub(crate) fn rlm_provider() -> unica_adapter_rlm::provider::RlmProvider<'static> {
    unica_adapter_rlm::provider::RlmProvider::new(
        &RLM_SEARCH_CLIENT,
        &RLM_NAV_CLIENT,
        source_bindings(),
    )
}
pub(crate) fn lexical_provider() -> unica_adapter_lexical::GitGrepProvider<'static> {
    unica_adapter_lexical::GitGrepProvider::new(source_bindings())
}
pub(crate) fn diagnostic_provider(
) -> unica_adapter_bsl_analyzer::diagnostics::BslAnalyzerDiagnosticProvider<'static> {
    unica_adapter_bsl_analyzer::diagnostics::BslAnalyzerDiagnosticProvider::new(
        &BSL_DIAGNOSTIC_BACKEND,
        crate::infrastructure::diagnostics::findings_module_path,
    )
}
#[cfg(test)]
pub(crate) fn rlm_navigation_client(
) -> &'static dyn unica_adapter_rlm::navigation::RlmNavigationClient {
    &RLM_NAV_CLIENT
}
pub(crate) fn index_is_ready(context: &WorkspaceContext) -> bool {
    unica_adapter_rlm::index::bsl_index_is_ready(context, environment())
}
pub(crate) fn run_workspace_service(args: &[String]) -> Result<(), String> {
    unica_analysis_services::run_workspace_service_from_args(
        args,
        Arc::new(
            unica_adapter_bsl_analyzer::session::BslWorkspaceBackend::new(Arc::new(
                AnalysisEnvironment,
            )),
        ),
        Arc::new(unica_adapter_rlm::backend::RlmWorkspaceBackend::new(
            Arc::new(AnalysisEnvironment),
        )),
        service_environment(),
    )
}
