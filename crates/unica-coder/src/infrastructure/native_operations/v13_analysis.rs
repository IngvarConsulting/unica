use crate::application::ports::FormatGuardCheck;
use crate::application::v13::check::{
    CheckDiagnostic, CheckValidator, NativeCheckOutcome, NodeFacts, TemplateFlavour,
};
use crate::domain::address::QualifiedAddress;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{json, Map, Value};

/// Closed native validator registry for the hidden A0 analysis seam. The
/// family validators remain the source of validation truth; this adapter only
/// translates their result into the bounded v0.13 diagnostic model.
pub(crate) fn validate(
    validator: CheckValidator,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> NativeCheckOutcome {
    validate_with_availability(validator, args, context, ValidatorAvailability::Available)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidatorAvailability {
    Available,
    Unavailable,
}

pub(crate) fn validate_with_availability(
    validator: CheckValidator,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    availability: ValidatorAvailability,
) -> NativeCheckOutcome {
    if availability == ValidatorAvailability::Unavailable {
        return unavailable_dependency();
    }
    // The retired `*.validate` tools ran the export-format guard before their
    // handler; the canonical check keeps that read-only warning so a root
    // outside the active profile is never reported as a silent pass.
    let format_warning = match crate::infrastructure::format_guard::evaluate_read_format_guard(
        validator.native_operation(),
        args,
        context,
    ) {
        Ok(FormatGuardCheck::Allow) | Err(_) => None,
        Ok(FormatGuardCheck::Warn {
            warning,
            diagnostic,
        }) => Some(format_diagnostic(&warning, &diagnostic)),
        Ok(FormatGuardCheck::Block {
            outcome,
            diagnostic,
            ..
        }) => Some(format_diagnostic(&outcome.warnings.join(" "), &diagnostic)),
    };
    let outcome = match validator {
        CheckValidator::Cf => super::cf::validate_cf(args, context),
        CheckValidator::Cfe => super::cfe::validate_cfe(args, context),
        CheckValidator::Form => super::form::validate_form(args, context),
        CheckValidator::Dcs => super::dcs::validate_dcs(args, context),
        CheckValidator::Mxl => super::mxl::validate_mxl(args, context),
        CheckValidator::Role => super::role::validate_role(args, context),
        CheckValidator::Subsystem => super::subsystem::validate_subsystem(args, context),
        CheckValidator::Interface => super::interface::validate_interface(args, context),
    };
    let native = NativeCheckOutcome::from_adapter(&outcome);
    match format_warning {
        Some(diagnostic) => native.with_leading_diagnostic(diagnostic),
        None => native,
    }
}

/// The v0.12 metadata selector of a logical node: `Kind.Name` pairs joined by
/// dots, or `None` for the configuration root. A trailing segment without a
/// name of its own (the `Interface` of a subsystem) selects its owner; a
/// nameless segment anywhere else names no descriptor.
pub(crate) fn validator_metadata_path(address: &QualifiedAddress) -> Option<String> {
    let segments = address.segments();
    if segments.len() == 1 && segments[0].kind() == crate::domain::address::NodeKind::Configuration
    {
        return None;
    }
    let mut parts = Vec::with_capacity(segments.len() * 2);
    for (index, segment) in segments.iter().enumerate() {
        let Some(name) = segment.name() else {
            if index + 1 == segments.len() && index > 0 {
                break;
            }
            return None;
        };
        parts.push(segment.kind().as_str().to_string());
        parts.push(name.to_string());
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("."))
}

/// The selector a native validator reads, built from the logical address the
/// caller gave to `check`: `{sourceSet, metadataPath}` for the bridged
/// validators, plus the file the extension and command-interface validators
/// still address by path. Nothing here comes from the wire.
pub(crate) fn validator_selector(
    validator: CheckValidator,
    address: &QualifiedAddress,
    context: &WorkspaceContext,
) -> Result<Map<String, Value>, String> {
    let mut selector = Map::new();
    selector.insert(
        "sourceSet".to_string(),
        Value::String(address.source_set().to_string()),
    );
    if let Some(path) = validator_metadata_path(address) {
        selector.insert("metadataPath".to_string(), Value::String(path));
    }
    match validator {
        CheckValidator::Cfe => {
            let resolved = crate::infrastructure::source_roots::resolve_named_source_set(
                context,
                address.source_set(),
            )
            .map_err(|error| error.to_string())?;
            selector.insert(
                "ExtensionPath".to_string(),
                json!(resolved.path.display().to_string()),
            );
        }
        CheckValidator::Interface => {
            let resolved = crate::infrastructure::source_roots::resolve_named_source_set(
                context,
                address.source_set(),
            )
            .map_err(|error| error.to_string())?;
            let mut path = resolved.path.clone();
            let segments = address.segments();
            let owner = match segments.last() {
                Some(last) if last.name().is_none() => &segments[..segments.len() - 1],
                _ => segments,
            };
            if owner.is_empty()
                || owner
                    .iter()
                    .any(|segment| segment.kind() != crate::domain::address::NodeKind::Subsystem)
            {
                return Err(format!(
                    "`{address}` does not name the command interface of a subsystem"
                ));
            }
            for segment in owner {
                path.push("Subsystems");
                path.push(segment.name().unwrap_or_default());
            }
            path.push("Ext");
            path.push("CommandInterface.xml");
            selector.insert("CIPath".to_string(), json!(path.display().to_string()));
        }
        _ => {}
    }
    Ok(selector)
}

/// Whether the source set of an address is declared as an extension.
pub(crate) fn source_set_is_extension(
    address: &QualifiedAddress,
    context: &WorkspaceContext,
) -> bool {
    crate::infrastructure::source_roots::resolve_named_source_set(context, address.source_set())
        .map(|resolved| resolved.source_set.kind == SourceSetKind::Extension)
        .unwrap_or(false)
}

/// The facts `check` derives from a readable node before it chooses
/// validators: the source-set kind and, for a template, the flavour its
/// descriptor declares (`TemplateType`). The projection states the flavour
/// only for a data composition schema (its `DataSet` branch), so the
/// descriptor is the authority for both kinds of template.
pub(crate) fn node_facts(
    address: &QualifiedAddress,
    viewed: Option<&Value>,
    context: &WorkspaceContext,
) -> NodeFacts {
    let from_branches = viewed
        .and_then(|data| data.get("branches"))
        .and_then(Value::as_array)
        .and_then(|branches| {
            branches.iter().find_map(|branch| {
                let at = branch.get("at").and_then(Value::as_str)?;
                if at.ends_with(".DataSet") {
                    Some(TemplateFlavour::DataCompositionSchema)
                } else if at.ends_with(".Area") {
                    Some(TemplateFlavour::SpreadsheetDocument)
                } else {
                    None
                }
            })
        });
    let is_template = address
        .segments()
        .last()
        .map(|segment| segment.kind() == crate::domain::address::NodeKind::Template)
        .unwrap_or(false);
    let template = match from_branches {
        Some(flavour) => Some(flavour),
        None if is_template => template_flavour_from_descriptor(address, context),
        None => None,
    };
    NodeFacts {
        extension: source_set_is_extension(address, context),
        template,
    }
}

/// Reads `TemplateType` from the template descriptor the logical selector
/// resolves for the address; unknown or unreadable descriptors stay `None`.
fn template_flavour_from_descriptor(
    address: &QualifiedAddress,
    context: &WorkspaceContext,
) -> Option<TemplateFlavour> {
    use super::logical_selector::{logical_selection, AttachedResource};

    let mut selector = Map::new();
    selector.insert(
        "sourceSet".to_string(),
        Value::String(address.source_set().to_string()),
    );
    selector.insert(
        "metadataPath".to_string(),
        Value::String(validator_metadata_path(address)?),
    );
    let resolved = logical_selection(
        &selector,
        context,
        AttachedResource::Template,
        super::mxl::TEMPLATE_KINDS,
    )?
    .ok()?;
    // `<Templates>/<Name>/Ext/Template.xml` sits next to `<Templates>/<Name>.xml`.
    let payload_dir = resolved.resource_path.parent()?.parent()?;
    let descriptor = payload_dir.with_extension("xml");
    let text = std::fs::read_to_string(descriptor).ok()?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let template_type = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "TemplateType")
        .and_then(|node| node.text())
        .map(str::trim)?;
    match template_type {
        "DataCompositionSchema" => Some(TemplateFlavour::DataCompositionSchema),
        "SpreadsheetDocument" => Some(TemplateFlavour::SpreadsheetDocument),
        _ => None,
    }
}

fn format_diagnostic(warning: &str, diagnostic: &Value) -> CheckDiagnostic {
    let code = diagnostic
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("formatVersionInvalid");
    let actual = diagnostic
        .get("actualFormat")
        .and_then(Value::as_str)
        .map(|actual| format!(" (export format {actual})"))
        .unwrap_or_default();
    CheckDiagnostic::warning(code, &format!("{warning}{actual}"))
}

/// A dependency that is not installed is not a validation failure and must be
/// represented as a typed unavailable outcome by the caller.
pub(crate) fn unavailable_dependency() -> NativeCheckOutcome {
    NativeCheckOutcome::unavailable("native validator dependency is unavailable")
}

#[cfg(test)]
mod tests {
    use super::{
        node_facts, validate, validate_with_availability, validator_selector, ValidatorAvailability,
    };
    use crate::application::v13::check::{
        normalize_native_outcome, CheckValidator, NodeFacts, TemplateFlavour,
    };
    use crate::domain::address::QualifiedAddress;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn context() -> (WorkspaceContext, std::path::PathBuf) {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "unica-a0-check-{}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        (
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            root,
        )
    }

    fn write_project(root: &std::path::Path, sets: &str) {
        fs::write(
            root.join("v8project.yaml"),
            format!("format: DESIGNER\nsource-set:\n{sets}"),
        )
        .unwrap();
    }

    #[test]
    fn cf_validator_is_real_and_normalized_before_application_projection() {
        let (context, root) = context();
        fs::write(root.join("Configuration.xml"), "<not-configuration />").unwrap();
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
        let native = validate(
            CheckValidator::Cf,
            &Map::from_iter([(
                "ConfigPath".to_string(),
                json!(root.join("Configuration.xml").display().to_string()),
            )]),
            &context,
        );
        let result =
            normalize_native_outcome(&at, "Configuration", CheckValidator::Cf, native).unwrap();
        assert!(!result.ok());
        assert!(!result.diagnostics().is_empty());
        for diagnostic in result.diagnostics() {
            assert!(!diagnostic.message().contains(root.to_str().unwrap()));
            assert!(!diagnostic.message().contains("stdout"));
            assert!(!diagnostic.message().contains("stderr"));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_dependency_stays_typed_and_does_not_claim_success() {
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
        let error = normalize_native_outcome(
            &at,
            "Configuration",
            CheckValidator::Cf,
            validate_with_availability(
                CheckValidator::Cf,
                &Map::new(),
                &WorkspaceContext {
                    cwd: std::path::PathBuf::new(),
                    workspace_root: std::path::PathBuf::new(),
                    cache_root: std::path::PathBuf::new(),
                    workspace_epoch: 1,
                },
                ValidatorAvailability::Unavailable,
            ),
        )
        .unwrap_err();
        assert_eq!(error.code(), "dependency_unavailable");
    }

    /// `check` over a 2.21 root keeps the validator verdict and leads with
    /// the format warning of the retired guard: the active profile is single
    /// and a newer export never passes silently.
    #[test]
    fn newer_configuration_root_leads_with_the_format_warning() {
        let (context, root) = context();
        write_project(
            &root,
            "  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
        let selector = validator_selector(CheckValidator::Cf, &at, &context).unwrap();
        assert_eq!(selector["sourceSet"], "main");
        assert!(selector.get("metadataPath").is_none());
        let native = validate(CheckValidator::Cf, &selector, &context);
        let result =
            normalize_native_outcome(&at, "Configuration", CheckValidator::Cf, native).unwrap();
        let first = result
            .diagnostics()
            .first()
            .expect("the format warning leads the diagnostics");
        assert_eq!(first.severity(), "warning");
        assert_eq!(first.code(), "platformVersionUnsupported");
        fs::remove_dir_all(root).unwrap();
    }

    /// The extension and command-interface validators still address a file;
    /// the selector derives it from the logical address, never from the wire.
    #[test]
    fn extension_and_interface_selectors_come_from_the_address() {
        let (context, root) = context();
        write_project(
            &root,
            "  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: src-ext\n",
        );
        fs::create_dir_all(root.join("src/Subsystems/Sales/Ext")).unwrap();
        fs::create_dir_all(root.join("src-ext")).unwrap();
        let root_address = QualifiedAddress::parse("ext:Configuration").unwrap();
        let selector = validator_selector(CheckValidator::Cfe, &root_address, &context).unwrap();
        assert_eq!(
            selector["ExtensionPath"],
            json!(root.join("src-ext").display().to_string())
        );
        assert!(super::source_set_is_extension(&root_address, &context));
        let main_root = QualifiedAddress::parse("main:Configuration").unwrap();
        assert!(!super::source_set_is_extension(&main_root, &context));

        let interface = QualifiedAddress::parse("main:Subsystem.Sales.Interface").unwrap();
        let selector = validator_selector(CheckValidator::Interface, &interface, &context).unwrap();
        assert_eq!(selector["metadataPath"], "Subsystem.Sales");
        assert_eq!(
            selector["CIPath"],
            json!(root
                .join("src/Subsystems/Sales/Ext/CommandInterface.xml")
                .display()
                .to_string())
        );
        let catalog = QualifiedAddress::parse("main:Catalog.Items").unwrap();
        assert!(validator_selector(CheckValidator::Interface, &catalog, &context).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn node_facts_read_the_template_flavour_from_the_projection_branches() {
        let (context, root) = context();
        write_project(
            &root,
            "  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let at = QualifiedAddress::parse("main:Report.Sales.Template.Schema").unwrap();
        let dcs =
            json!({"branches": [{"at": "main:Report.Sales.Template.Schema.DataSet", "count": 1}]});
        assert_eq!(
            node_facts(&at, Some(&dcs), &context),
            NodeFacts {
                extension: false,
                template: Some(TemplateFlavour::DataCompositionSchema)
            }
        );
        let mxl =
            json!({"branches": [{"at": "main:Report.Sales.Template.Print.Area", "count": 3}]});
        assert_eq!(
            node_facts(&at, Some(&mxl), &context).template,
            Some(TemplateFlavour::SpreadsheetDocument)
        );
        assert_eq!(node_facts(&at, None, &context).template, None);
        fs::remove_dir_all(root).unwrap();
    }
}
