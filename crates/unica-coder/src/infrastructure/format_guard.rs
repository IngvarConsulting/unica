use crate::application::operation_descriptors::{
    native_operation_descriptor, FormatGuardPolicy, FormatPathPolicy,
};
use crate::application::ports::FormatGuardCheck;
use crate::application::{AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::form::{
    form_compile_infer_from_object_target, form_compile_normalize_from_object_output_label,
};
use crate::infrastructure::platform_xml_owner::{resolve_platform_xml_owner, PlatformXmlOwnerKind};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub(crate) fn evaluate_format_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<FormatGuardCheck, String> {
    let ToolHandler::NativeOperation { operation, .. } = spec.handler else {
        return Ok(FormatGuardCheck::Allow);
    };
    let Some(descriptor) = native_operation_descriptor(operation) else {
        return Ok(FormatGuardCheck::Allow);
    };
    if !matches!(
        descriptor.format_guard,
        FormatGuardPolicy::ExistingDump | FormatGuardPolicy::OptionalExistingBase
    ) {
        return Ok(FormatGuardCheck::Allow);
    }
    for target in effective_format_paths(descriptor, args, context) {
        let owner = match resolve_platform_xml_owner(&target, context) {
            Ok(owner) => owner,
            Err(error) => {
                let warning = format!(
                    "Некорректный корневой файл формата выгрузки {}: {}",
                    error.path.display(),
                    error.message
                );
                let diagnostic = json!({
                    "code": "formatVersionInvalid",
                    "actualFormat": Value::Null,
                    "targetFormat": ACTIVE_FORMAT_PROFILE.export_format,
                    "targetPlatform": ACTIVE_FORMAT_PROFILE.platform_line,
                    "compatibility": "invalid",
                    "root": error.path.display().to_string(),
                });
                return Ok(format_check(spec, warning, diagnostic));
            }
        };
        let Some(owner) = owner else {
            continue;
        };
        let compatibility = match classify_root_version(owner.version.as_deref()) {
            Ok(compatibility) => compatibility,
            Err(error) => {
                let diagnostic = json!({
                    "code": error.code(),
                    "actualFormat": owner.version,
                    "targetFormat": ACTIVE_FORMAT_PROFILE.export_format,
                    "targetPlatform": ACTIVE_FORMAT_PROFILE.platform_line,
                    "compatibility": "invalid",
                    "root": owner.path.display().to_string(),
                    "ownerKind": owner.kind.label(),
                });
                return Ok(format_check(
                    spec,
                    format!(
                        "Некорректная версия формата выгрузки в {}",
                        owner.path.display()
                    ),
                    diagnostic,
                ));
            }
        };
        if matches!(compatibility, FormatCompatibility::Supported { .. }) {
            continue;
        }
        let actual = compatibility.actual().to_string();
        let (code, warning) = match compatibility {
            FormatCompatibility::Older { .. } => {
                let warning = match owner.kind {
                    PlatformXmlOwnerKind::Configuration => format!(
                        "Формат выгрузки {actual} старше поддерживаемого {} для платформы 1С {}. Изменение отменено; предложите пользователю явную миграцию через unica.cf.migrate_format.",
                        ACTIVE_FORMAT_PROFILE.export_format, ACTIVE_FORMAT_PROFILE.platform_line
                    ),
                    PlatformXmlOwnerKind::Extension => format!(
                        "Формат выгрузки {actual} старше поддерживаемого {} для платформы 1С {}. Изменение отменено; предложите пользователю явную миграцию через unica.cfe.migrate_format.",
                        ACTIVE_FORMAT_PROFILE.export_format, ACTIVE_FORMAT_PROFILE.platform_line
                    ),
                    PlatformXmlOwnerKind::ExternalProcessor
                    | PlatformXmlOwnerKind::ExternalReport => format!(
                        "Формат выгрузки {actual} старше поддерживаемого {} для платформы 1С {}. Изменение отменено; требуется явная повторная выгрузка через платформу 1С 8.3.27. Автоматическая миграция EPF/ERF пока не реализована.",
                        ACTIVE_FORMAT_PROFILE.export_format, ACTIVE_FORMAT_PROFILE.platform_line
                    ),
                };
                ("formatMigrationAvailable", warning)
            }
            FormatCompatibility::Newer { .. } => (
                "platformVersionUnsupported",
                format!(
                    "Формат выгрузки {actual} новее поддерживаемого {} для платформы 1С {}. Unica пока не поддерживает работу с этой выгрузкой. Поддержка платформы 1С 8.5 планируется в ближайших версиях.",
                    ACTIVE_FORMAT_PROFILE.export_format, ACTIVE_FORMAT_PROFILE.platform_line
                ),
            ),
            FormatCompatibility::Supported { .. } => unreachable!(),
        };
        let diagnostic = json!({
            "code": code,
            "actualFormat": actual,
            "targetFormat": ACTIVE_FORMAT_PROFILE.export_format,
            "targetPlatform": ACTIVE_FORMAT_PROFILE.platform_line,
            "compatibility": compatibility.label(),
            "root": owner.path.display().to_string(),
            "ownerKind": owner.kind.label(),
        });
        return Ok(format_check(spec, warning, diagnostic));
    }
    Ok(FormatGuardCheck::Allow)
}

fn format_check(spec: ToolSpec, warning: String, diagnostic: Value) -> FormatGuardCheck {
    if !spec.mutating {
        return FormatGuardCheck::Warn {
            warning,
            diagnostic,
        };
    }
    FormatGuardCheck::Block {
        outcome: AdapterOutcome {
            ok: false,
            summary: format!("{} blocked by export format guard", spec.name),
            changes: Vec::new(),
            warnings: vec![warning.clone()],
            errors: vec![warning.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{warning}\n")),
            command: None,
        },
        diagnostic,
    }
}

fn effective_format_paths(
    descriptor: &crate::application::operation_descriptors::OperationDescriptor,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    match descriptor.format_path_policy {
        FormatPathPolicy::DeclaredArgs => descriptor
            .source_path_args
            .iter()
            .filter_map(|name| args.get(*name).and_then(Value::as_str))
            .map(|raw| absolutize(raw, &context.cwd))
            .collect(),
        FormatPathPolicy::DefaultSrcObject => {
            let src = ["SrcDir", "srcDir"]
                .iter()
                .find_map(|name| args.get(*name).and_then(Value::as_str))
                .unwrap_or("src");
            let object = ["ObjectName", "objectName", "ProcessorName", "processorName"]
                .iter()
                .find_map(|name| args.get(*name).and_then(Value::as_str));
            object
                .map(|name| {
                    absolutize(src, &context.cwd)
                        .join(name)
                        .with_extension("xml")
                })
                .into_iter()
                .collect()
        }
        FormatPathPolicy::FormCompile => form_compile_format_paths(args, context),
    }
}

fn form_compile_format_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    let Some(raw_output) = ["OutputPath", "outputPath"]
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
    else {
        return Vec::new();
    };
    let from_object = ["FromObject", "fromObject"]
        .iter()
        .any(|name| args.get(*name).and_then(Value::as_bool).unwrap_or(false));
    let output_label = if from_object {
        form_compile_normalize_from_object_output_label(raw_output)
            .map(|(path, _)| path)
            .unwrap_or_else(|| raw_output.to_string())
    } else {
        raw_output.to_string()
    };
    let output = absolutize(&output_label, &context.cwd);
    let mut paths = vec![output.clone()];
    if from_object {
        if let Some(raw_object) = ["ObjectPath", "objectPath"]
            .iter()
            .find_map(|name| args.get(*name).and_then(Value::as_str))
        {
            let mut object = absolutize(raw_object, &context.cwd);
            if object.extension().is_none() {
                object.set_extension("xml");
            }
            paths.push(object);
        } else if let (Some(inferred), _) = form_compile_infer_from_object_target(&output, context)
        {
            paths.push(inferred);
        }
    }
    paths
}

fn absolutize(raw: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::evaluate_format_guard;
    use crate::application::ports::FormatGuardCheck;
    use crate::application::tools;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{Map, Value};

    fn context(root: &std::path::Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn config(root: &std::path::Path, version: Option<&str>) -> std::path::PathBuf {
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let version = version
            .map(|value| format!(r#" version="{value}""#))
            .unwrap_or_default();
        std::fs::write(
            src.join("Configuration.xml"),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"{version}/>"#),
        )
        .unwrap();
        src.join("Configuration.xml")
    }

    fn spec(name: &str) -> crate::application::ToolSpec {
        tools().into_iter().find(|tool| tool.name == name).unwrap()
    }

    fn external_source_set(
        root: &std::path::Path,
        kind: &str,
        dir: &str,
        artifact: &str,
        version: &str,
    ) -> std::path::PathBuf {
        std::fs::write(
            root.join("v8project.yaml"),
            format!(
                "format: DESIGNER\nsource-set:\n  - name: external\n    type: {kind}\n    path: {dir}\n"
            ),
        )
        .unwrap();
        let source_root = root.join(dir);
        std::fs::create_dir_all(source_root.join(artifact)).unwrap();
        let tag = if kind == "EXTERNAL_REPORTS" {
            "ExternalReport"
        } else {
            "ExternalDataProcessor"
        };
        std::fs::write(
            source_root.join(format!("{artifact}.xml")),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><{tag}/></MetaDataObject>"#
            ),
        )
        .unwrap();
        source_root
    }

    #[test]
    fn older_dump_blocks_mutation_and_offers_explicit_migration() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-old-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let path = config(&root, Some("2.19"));
        let before = std::fs::read(&path).unwrap();
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.cf.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("older mutation must be blocked");
        };
        assert!(!outcome.ok);
        assert_eq!(diagnostic["code"], "formatMigrationAvailable");
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert!(outcome
            .warnings
            .join("\n")
            .contains("unica.cf.migrate_format"));
        assert_eq!(std::fs::read(path).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn older_extension_dump_offers_extension_migration() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-old-cfe-{}", std::process::id()));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let path = src.join("Configuration.xml");
        std::fs::write(
            &path,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ExtensionPath".into(),
            Value::String(path.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.cfe.patch_method"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { outcome, .. } = check else {
            panic!("older extension mutation must be blocked");
        };
        assert!(outcome
            .warnings
            .join("\n")
            .contains("unica.cfe.migrate_format"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_init_preflights_its_optional_cf_base_with_cf_migration() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-cfe-init-{}",
            std::process::id()
        ));
        let path = config(&root, Some("2.19"));
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.cfe.init"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("older optional CF base must block CFE init");
        };
        assert_eq!(diagnostic["code"], "formatMigrationAvailable");
        assert!(outcome
            .warnings
            .join("\n")
            .contains("unica.cf.migrate_format"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn supported_dump_allows_mutation_preflight() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-ok-{}", std::process::id()));
        let path = config(&root, Some("2.20"));
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );
        assert!(matches!(
            evaluate_format_guard(spec("unica.cf.edit"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn newer_dump_warns_for_read_only_with_roadmap_copy() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-new-{}", std::process::id()));
        let path = config(&root, Some("2.21"));
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );
        let check = evaluate_format_guard(spec("unica.cf.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn {
            warning,
            diagnostic,
        } = check
        else {
            panic!("newer read-only input must warn and continue");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert!(warning.contains("Поддержка платформы 1С 8.5 планируется"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_root_version_is_classified_as_1_0() {
        let root =
            std::env::temp_dir().join(format!("unica-format-guard-v1-{}", std::process::id()));
        let path = config(&root, None);
        let mut args = Map::new();
        args.insert(
            "ConfigPath".into(),
            Value::String(path.display().to_string()),
        );
        let check =
            evaluate_format_guard(spec("unica.cf.validate"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("missing root version must be old-format warning");
        };
        assert_eq!(diagnostic["actualFormat"], "1.0");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dcs_edit_blocks_old_external_source_set_via_owner_descriptor() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-old-external-dcs-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "epf",
            "PriceLoader",
            "2.19",
        );
        let target = source_root.join("PriceLoader/Templates/Main/Ext/Template.xml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "<DataCompositionSchema/>").unwrap();
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(target.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.dcs.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("old EPF owner must block DCS edit");
        };
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert!(outcome.warnings.join("\n").contains("повторная выгрузка"));
        assert!(!outcome.warnings.join("\n").contains("migrate_format"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mxl_info_warns_old_external_source_set_via_owner_descriptor() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-old-external-mxl-info-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(&root, "EXTERNAL_REPORTS", "erf", "Sales", "2.19");
        let target = source_root.join("Sales/Templates/Print/Ext/Template.xml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "<document/>").unwrap();
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(target.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.mxl.info"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Warn { diagnostic, .. } = check else {
            panic!("old ERF owner must warn for read-only MXL info");
        };
        assert_eq!(diagnostic["actualFormat"], "2.19");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_remove_default_src_blocks_older_dump() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-default-form-remove-{}",
            std::process::id()
        ));
        config(&root, Some("2.19"));
        let mut args = Map::new();
        args.insert("ObjectName".into(), Value::String("Catalogs/Items".into()));

        assert!(matches!(
            evaluate_format_guard(spec("unica.form.remove"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn template_remove_default_src_blocks_newer_dump() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-default-template-remove-{}",
            std::process::id()
        ));
        config(&root, Some("2.21"));
        let mut args = Map::new();
        args.insert("ObjectName".into(), Value::String("Reports/Sales".into()));

        let check =
            evaluate_format_guard(spec("unica.template.remove"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("newer default source dump must block template removal");
        };
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn help_add_default_src_blocks_old_external_source_set() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-default-help-external-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "src",
            "PriceLoader",
            "2.19",
        );
        let mut args = Map::new();
        args.insert("ObjectName".into(), Value::String("PriceLoader".into()));

        assert!(matches!(
            evaluate_format_guard(spec("unica.help.add"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn format_guard_normalizes_parent_segments_before_owner_lookup() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-normalized-parent-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "epf",
            "PriceLoader",
            "2.19",
        );
        let target = source_root.join("PriceLoader/Templates/../Templates/Main/Ext/Template.xml");
        let mut args = Map::new();
        args.insert(
            "TemplatePath".into(),
            Value::String(target.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.dcs.edit"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn standalone_compile_does_not_inherit_unrelated_workspace_configuration() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-standalone-output-{}",
            std::process::id()
        ));
        config(&root, Some("2.19"));
        let standalone = root.join("generated/report.xml");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(standalone.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Allow
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mxl_compile_blocks_write_inside_older_dump() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-mxl-compile-old-{}",
            std::process::id()
        ));
        config(&root, Some("2.19"));
        let output = root.join("src/Reports/Sales/Templates/Print/Ext/Template.xml");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, b"original bytes").unwrap();
        let before = std::fs::read(&output).unwrap();
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        assert_eq!(std::fs::read(&output).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_compile_blocks_old_external_source_set_before_create() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-form-compile-old-external-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(
            &root,
            "EXTERNAL_DATA_PROCESSORS",
            "epf",
            "PriceLoader",
            "2.19",
        );
        let output = source_root.join("PriceLoader/Forms/Main/Ext/Form.xml");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );

        assert!(matches!(
            evaluate_format_guard(spec("unica.form.compile"), &args, &context(&root)).unwrap(),
            FormatGuardCheck::Block { .. }
        ));
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_compile_from_object_checks_input_and_output_formats() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-form-compile-input-output-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("old")).unwrap();
        std::fs::create_dir_all(root.join("active")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: old\n    type: CONFIGURATION\n    path: old\n  - name: active\n    type: CONFIGURATION\n    path: active\n",
        )
        .unwrap();
        std::fs::write(
            root.join("old/Configuration.xml"),
            r#"<MetaDataObject version="2.19"/>"#,
        )
        .unwrap();
        std::fs::write(
            root.join("active/Configuration.xml"),
            r#"<MetaDataObject version="2.20"/>"#,
        )
        .unwrap();
        let object = root.join("old/Catalogs/Items.xml");
        std::fs::create_dir_all(object.parent().unwrap()).unwrap();
        std::fs::write(&object, "<MetaDataObject/>").unwrap();
        let output = root.join("active/Catalogs/Items/Forms/Main/Ext/Form.xml");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );
        args.insert("FromObject".into(), Value::Bool(true));
        args.insert(
            "ObjectPath".into(),
            Value::String(object.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.form.compile"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("old from-object input must block even when output is active");
        };
        assert_eq!(diagnostic["actualFormat"], "2.19");
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_owner_returns_structured_format_version_invalid() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-malformed-owner-{}",
            std::process::id()
        ));
        let owner = config(&root, Some("2.20"));
        std::fs::write(&owner, "<broken").unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(root.join("src/Catalogs/Items.xml").display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("malformed owner must produce a structured blocking diagnostic");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        assert!(diagnostic["root"]
            .as_str()
            .is_some_and(|path| path.ends_with("/src/Configuration.xml")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unreadable_utf8_owner_returns_structured_format_version_invalid() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-unreadable-owner-{}",
            std::process::id()
        ));
        let owner = config(&root, Some("2.20"));
        std::fs::write(&owner, [0xff, 0xfe, 0xfd]).unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(root.join("src/Catalogs/Items.xml").display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("non-UTF-8 owner must produce a structured blocking diagnostic");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_owner_inside_recognized_source_set_is_invalid() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-missing-owner-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src/Catalogs")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(root.join("src/Catalogs/Items.xml").display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("missing owner in a configured source set must block");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        assert!(diagnostic["root"]
            .as_str()
            .is_some_and(|path| path.ends_with("/src/Configuration.xml")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_existing_standalone_xml_is_invalid_not_new_output() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-malformed-standalone-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let output = root.join("standalone.xml");
        std::fs::write(&output, "<broken").unwrap();
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("malformed existing standalone XML must not be treated as a new output");
        };
        assert_eq!(diagnostic["code"], "formatVersionInvalid");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_external_descriptor_uses_external_owner_copy() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-direct-external-owner-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let descriptor = root.join("PriceLoader.xml");
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject version="2.19"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "ObjectPath".into(),
            Value::String(descriptor.display().to_string()),
        );

        let check = evaluate_format_guard(spec("unica.meta.edit"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } = check
        else {
            panic!("old direct EPF descriptor must block");
        };
        assert_eq!(diagnostic["ownerKind"], "external_processor");
        assert!(outcome.warnings.join("\n").contains("повторная выгрузка"));
        assert!(!outcome.warnings.join("\n").contains("migrate_format"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_source_root_with_one_descriptor_resolves_that_owner() {
        let root = std::env::temp_dir().join(format!(
            "unica-format-guard-external-root-owner-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_root = external_source_set(&root, "EXTERNAL_REPORTS", "erf", "Sales", "2.19");
        let mut args = Map::new();
        args.insert(
            "OutputPath".into(),
            Value::String(source_root.display().to_string()),
        );

        let check =
            evaluate_format_guard(spec("unica.mxl.compile"), &args, &context(&root)).unwrap();
        let FormatGuardCheck::Block { diagnostic, .. } = check else {
            panic!("external source root with one descriptor must resolve its owner");
        };
        assert_eq!(diagnostic["ownerKind"], "external_report");
        assert!(diagnostic["root"]
            .as_str()
            .is_some_and(|path| path.ends_with("/erf/Sales.xml")));
        let _ = std::fs::remove_dir_all(root);
    }
}
