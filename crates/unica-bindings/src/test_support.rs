//! Exact fixture entry points and observations of private pure calculations.
use crate::projection::{
    project_module, DeclarativeBinding, ModuleProjectionRequest, PlatformEventWriteCapability,
};
use unica_address::address::QualifiedAddress;
use unica_bsl::profile::{ModuleCapability, PlatformProfile};
use unica_bsl::projection::{CommonModuleProperties, ModuleProjectionSet};

pub fn evaluate_guard(guard: &str, context: &str) -> bool {
    crate::projection::evaluate_guard(guard, context)
}

pub fn base_contexts(
    capability: ModuleCapability,
    common: Option<&CommonModuleProperties>,
) -> Option<Vec<String>> {
    crate::projection::base_contexts(capability, common)
}

pub fn project_declarative_binding(
    binding: DeclarativeBinding<'_>,
    source: &str,
) -> Result<ModuleProjectionSet, String> {
    let module_at = match binding.owner {
        "httpMethod" => "main:HTTPService.API.Module.HTTPService",
        "webServiceOperation" => "main:WebService.Обмен.Module.WebService",
        "integrationServiceChannel" => "main:IntegrationService.Шина.Module.IntegrationService",
        other => return Err(format!("unknown declarative service owner `{other}`")),
    };
    let at = QualifiedAddress::parse(module_at).map_err(|error| error.to_string())?;
    let capability = PlatformProfile::v8_3_27()
        .module_capability(&at)
        .ok_or_else(|| format!("service module `{module_at}` is not in profile 8.3.27"))?;
    project_module(ModuleProjectionRequest {
        at: &at,
        capability,
        title: format!("{} module", capability.role().as_str()),
        rev: "fixture-rev",
        source: Some(source),
        common_module: None,
        handles: &[],
        declarative_bindings: std::slice::from_ref(&binding),
        extension_targets: &[],
        platform_event_write: PlatformEventWriteCapability::Proven,
    })
}
