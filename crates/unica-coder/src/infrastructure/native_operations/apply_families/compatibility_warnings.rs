//! The platform does not apply an extension whose compatibility mode is above
//! its configuration's. An edit that brings a set there is allowed, as the
//! Designer allows it, and its answer warns (INV.SOURCE.WRITABLE-PROFILE).
//! `DontUse` — the mode of the running platform — is not compared.

use std::path::Path;
use unica_metadata::configuration::{
    extension_mode_above_base_warning, extension_mode_is_above_base,
};
use unica_source_contract::proposal::ApplyPlanError;
use unica_source_contract::SourceSetKind;
use unica_workspace::actor::{ApplyAdmission, ProviderRootBinding};
use unica_workspace::apply::ApplyStagedState;

const CONFIGURATION: &str = "Configuration.xml";
const MODE: &str = "CompatibilityMode";
const EXTENSION_MODE: &str = "ConfigurationExtensionCompatibilityMode";

/// The mode `property` a `Configuration.xml` records.
fn mode(bytes: &[u8], property: &str) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(text).ok()?;
    unica_xml_format::read::first_text(&document, property)
}

/// The warnings of a plan whose root edit leaves an extension above its
/// configuration, against the sets the apply was admitted to read.
pub(super) fn warnings(
    staged: &ApplyStagedState,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
) -> Result<Vec<serde_json::Value>, ApplyPlanError> {
    let dependencies = admission.read_dependency_sets();
    let changed_root = staged
        .planned_changes()
        .into_iter()
        .any(|change| change.relative_path == Path::new(CONFIGURATION));
    if dependencies.is_empty() || !changed_root {
        return Ok(Vec::new());
    }
    let Some(own) = staged.read(Path::new(CONFIGURATION)).ok().flatten() else {
        return Ok(Vec::new());
    };
    let warning = |message: String| {
        serde_json::json!({"code": "extensionCompatibilityAboveBase", "message": message})
    };
    let mut warnings = Vec::new();
    match binding.source_kind() {
        SourceSetKind::Extension => {
            let Some(extension_mode) = mode(&own, EXTENSION_MODE) else {
                return Ok(Vec::new());
            };
            for (name, kind) in dependencies {
                if kind != SourceSetKind::Configuration {
                    continue;
                }
                let base = admission.read_apply_dependency(name, Path::new(CONFIGURATION))?;
                if let Some(base_mode) = mode(&base, MODE) {
                    if extension_mode_is_above_base(&extension_mode, &base_mode) {
                        warnings.push(warning(extension_mode_above_base_warning(
                            &extension_mode,
                            None,
                            &base_mode,
                            name,
                        )));
                    }
                }
            }
        }
        SourceSetKind::Configuration => {
            let Some(base_mode) = mode(&own, MODE) else {
                return Ok(Vec::new());
            };
            for (name, kind) in dependencies {
                if kind != SourceSetKind::Extension {
                    continue;
                }
                let extension = admission.read_apply_dependency(name, Path::new(CONFIGURATION))?;
                if let Some(extension_mode) = mode(&extension, EXTENSION_MODE) {
                    if extension_mode_is_above_base(&extension_mode, &base_mode) {
                        warnings.push(warning(extension_mode_above_base_warning(
                            &extension_mode,
                            Some(name),
                            &base_mode,
                            binding.source_set_name(),
                        )));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(warnings)
}
