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
    let text = std::str::from_utf8(bytes)
        .ok()?
        .trim_start_matches('\u{feff}');
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
    if dependencies.is_empty() {
        return Ok(Vec::new());
    }
    // The root as the plan leaves it; a plan that keeps the root warns nothing.
    let Some(own) = staged
        .planned_changes()
        .into_iter()
        .find(|change| change.relative_path == Path::new(CONFIGURATION))
        .and_then(|change| match change.current {
            unica_source_contract::proposal::StagedFileState::Bytes(bytes) => Some(bytes),
            unica_source_contract::proposal::StagedFileState::Absent => None,
        })
    else {
        return Ok(Vec::new());
    };
    let warning = |message: String| serde_json::json!({"code": "extensionCompatibilityAboveBase", "message": message});
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
                // An unreadable root has no mode: a warning never refuses.
                let base = admission
                    .read_apply_dependency(name, Path::new(CONFIGURATION))
                    .unwrap_or_default();
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
                let extension = admission
                    .read_apply_dependency(name, Path::new(CONFIGURATION))
                    .unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::super::plan_hidden_v13_apply;
    use crate::domain::apply::ApplyRequest;
    use serde_json::json;
    use std::path::Path;
    use std::time::Duration;
    use unica_source_contract::format_identity::PlatformLine;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;
    use unica_workspace::actor::{WorkspaceIdentity, WorkspaceSourceSetInput};

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// The warnings of `values` set at the root of `at_set` in a project of
    /// the 2.21 corpora: `main` (all_kinds, mode 8.3.27) and `ext`
    /// (empty_extension, mode 8.3.27), the other set admitted to read.
    fn warnings(at_set: &str, values: serde_json::Value) -> Vec<serde_json::Value> {
        warnings_with(at_set, values, &|_, _| {})
    }

    /// [`warnings`] after `prepare` has changed the admitted `cf` and `ext`.
    fn warnings_with(
        at_set: &str,
        values: serde_json::Value,
        prepare: &dyn Fn(&Path, &Path),
    ) -> Vec<serde_json::Value> {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = tempfile::tempdir().unwrap();
        let workspace_root = std::fs::canonicalize(root.path()).unwrap();
        let main = workspace_root.join("cf");
        let ext = workspace_root.join("ext");
        copy_tree(
            &crate_root.join("../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src"),
            &main,
        );
        copy_tree(
            &crate_root.join("../../tests/fixtures/formats/platform_xml/2.21/empty_extension/ext"),
            &ext,
        );
        std::fs::write(
            workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cf\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
        )
        .unwrap();
        let profile = SourceProfile::platform_xml_of_line(PlatformLine::new(8, 5, 1)).unwrap();
        let context = WorkspaceContext {
            cwd: workspace_root.clone(),
            workspace_root: workspace_root.clone(),
            cache_root: workspace_root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [
                WorkspaceSourceSetInput::new(
                    "main",
                    &main,
                    SourceSetKind::Configuration,
                    SourceFormat::PlatformXml,
                    profile,
                ),
                WorkspaceSourceSetInput::new(
                    "ext",
                    &ext,
                    SourceSetKind::Extension,
                    SourceFormat::PlatformXml,
                    profile,
                ),
            ],
            "compatibility-warnings",
        )
        .unwrap();
        let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
        let (own, other) = if at_set == "ext" {
            (&ext, ("main", &main))
        } else {
            (&main, ("ext", &ext))
        };
        let binding = actor.bind_provider_root(at_set, own).unwrap();
        let read = actor.bind_provider_root(other.0, other.1).unwrap();
        let admission = actor
            .admit_apply_with_dependencies(
                &binding,
                &[read],
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        // After admission: a root that turns unreadable while the plan reads it.
        prepare(&main, &ext);
        let at = format!("{at_set}:Configuration");
        let args =
            json!({"at": at, "dryRun": true, "ops": [{"op":"props.set","args":{"values":values}}]});
        let request = ApplyRequest::parse(args.as_object().unwrap(), &[at_set, other.0]).unwrap();
        let (_, effects) = plan_hidden_v13_apply(&request, &binding, &admission).unwrap();
        effects.warnings().to_vec()
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn raising_an_extension_above_its_configuration_warns_and_still_plans() {
        assert_eq!(
            warnings(
                "ext",
                json!({"ConfigurationExtensionCompatibilityMode":"Version8_5_1"})
            ),
            vec![json!({"code":"extensionCompatibilityAboveBase","message":
                "Extension compatibility mode `Version8_5_1` is above compatibility mode `Version8_3_27` of configuration `main`; the platform does not apply such an extension."})]
        );
        assert!(warnings(
            "ext",
            json!({"ConfigurationExtensionCompatibilityMode":"Version8_3_24"})
        )
        .is_empty());
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn lowering_a_configuration_below_its_extension_warns_and_still_plans() {
        assert_eq!(
            warnings("main", json!({"CompatibilityMode":"Version8_3_24"})),
            vec![json!({"code":"extensionCompatibilityAboveBase","message":
                "Extension compatibility mode `Version8_3_27` of extension `ext` is above compatibility mode `Version8_3_24` of configuration `main`; the platform does not apply such an extension."})]
        );
        assert!(warnings("main", json!({"CompatibilityMode":"Version8_5_1"})).is_empty());
    }

    /// The admission reads an extension too, so a borrow names its own rule:
    /// the parent is a configuration.
    #[test]
    fn a_borrow_from_an_extension_is_refused() {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = tempfile::tempdir().unwrap();
        let workspace_root = std::fs::canonicalize(root.path()).unwrap();
        let ext = workspace_root.join("ext");
        let other = workspace_root.join("other");
        for dir in [&ext, &other] {
            copy_tree(
                &crate_root
                    .join("../../tests/fixtures/formats/platform_xml/2.21/empty_extension/ext"),
                dir,
            );
        }
        std::fs::write(
            workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: ext\n    type: EXTENSION\n    path: ext\n  - name: other\n    type: EXTENSION\n    path: other\n",
        )
        .unwrap();
        let profile = SourceProfile::platform_xml_of_line(PlatformLine::new(8, 5, 1)).unwrap();
        let context = WorkspaceContext {
            cwd: workspace_root.clone(),
            workspace_root: workspace_root.clone(),
            cache_root: workspace_root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [
                WorkspaceSourceSetInput::new(
                    "ext",
                    &ext,
                    SourceSetKind::Extension,
                    SourceFormat::PlatformXml,
                    profile,
                ),
                WorkspaceSourceSetInput::new(
                    "other",
                    &other,
                    SourceSetKind::Extension,
                    SourceFormat::PlatformXml,
                    profile,
                ),
            ],
            "borrow-from-extension",
        )
        .unwrap();
        let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
        let binding = actor.bind_provider_root("ext", &ext).unwrap();
        let parent = actor.bind_provider_root("other", &other).unwrap();
        let admission = actor
            .admit_apply_with_dependencies(
                &binding,
                &[parent],
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let args = json!({"at":"ext:Configuration","dryRun":true,"ops":[{"op":"object.borrow","args":{"from":"other:Language.Русский"}}]});
        let request = ApplyRequest::parse(args.as_object().unwrap(), &["ext", "other"]).unwrap();
        let error = plan_hidden_v13_apply(&request, &binding, &admission).unwrap_err();
        assert_eq!(
            error.to_string(),
            "from must name an object of a configuration source set"
        );
    }

    /// A peer whose root cannot be read has no mode: the edit plans without
    /// a warning, never a refusal.
    #[test]
    fn an_unreadable_peer_root_leaves_the_edit_without_a_warning() {
        let unreadable = |main: &Path, _: &Path| {
            std::fs::remove_file(main.join("Configuration.xml")).unwrap();
        };
        assert!(warnings_with(
            "ext",
            json!({"ConfigurationExtensionCompatibilityMode":"Version8_5_1"}),
            &unreadable
        )
        .is_empty());
    }
}
