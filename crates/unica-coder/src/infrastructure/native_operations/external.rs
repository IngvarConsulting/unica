use super::common::guard_active_format_containing_owner_for_new_output;
use super::meta::validate_metadata_owner_shape_8_3_27;
use crate::infrastructure::form_composition::validate_form;
use serde_json::{Map, Value};
use std::path::PathBuf;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_forms_designer::codec::{
    form_add_content_xml, form_add_metadata_xml, form_add_module_bsl,
};
use unica_metadata_designer::external::{
    descriptor_xml, plan_scaffold, validate_xml, ExternalArtifactKind, ScaffoldPlan,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;
use unica_workspace::compile_transaction::CompileTransaction;
#[cfg(test)]
use uuid::Uuid;

const FORMAT_VERSION: &str = ACTIVE_FORMAT_PROFILE.export_format;
const OBJECT_MODULE_STUB: &str = "#Область ПрограммныйИнтерфейс\r\n\r\n#КонецОбласти\r\n";

struct ScaffoldContent {
    descriptor: String,
    form_metadata: Option<String>,
    form_content: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum ScaffoldMode {
    Preview,
    Apply,
}

pub(crate) fn preview(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    invoke(operation, tool_name, args, context, ScaffoldMode::Preview)
}

pub(crate) fn apply(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    invoke(operation, tool_name, args, context, ScaffoldMode::Apply)
}

/// Applies the scaffold and reports the files it created as typed data.
pub(crate) fn apply_with_data(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<(
    AdapterOutcome,
    Option<unica_execution_contract::mutation::MutationData>,
)> {
    let kind = ExternalArtifactKind::from_operation(operation)?;
    Some(match prepare_plan(kind, args, context) {
        Ok(plan) => match create_scaffold(&plan, context) {
            Ok(warnings) => (
                success_outcome(tool_name, &plan, ScaffoldMode::Apply, warnings),
                Some(scaffold_mutation(&plan, ScaffoldMode::Apply)),
            ),
            Err(error) => (failure_outcome(tool_name, error), None),
        },
        Err(error) => (failure_outcome(tool_name, error), None),
    })
}

fn invoke(
    operation: &str,
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: ScaffoldMode,
) -> Option<AdapterOutcome> {
    let kind = ExternalArtifactKind::from_operation(operation)?;
    Some(match (prepare_plan(kind, args, context), mode) {
        (Ok(plan), ScaffoldMode::Preview) => {
            success_outcome(tool_name, &plan, ScaffoldMode::Preview, Vec::new())
        }
        (Ok(plan), ScaffoldMode::Apply) => match create_scaffold(&plan, context) {
            Ok(warnings) => success_outcome(tool_name, &plan, ScaffoldMode::Apply, warnings),
            Err(error) => failure_outcome(tool_name, error),
        },
        (Err(error), ScaffoldMode::Preview | ScaffoldMode::Apply) => {
            failure_outcome(tool_name, error)
        }
    })
}

fn prepare_plan(
    kind: ExternalArtifactKind,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ScaffoldPlan, String> {
    let plan = plan_scaffold(
        kind,
        args,
        &context.cwd,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.workspace_root),
    )?;
    for target in [&plan.descriptor, &plan.object_dir] {
        if target.exists() {
            return Err(format!("target already exists: {}", target.display()));
        }
    }
    Ok(plan)
}

pub(crate) fn external_init_planned_xml_paths(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let kind = ExternalArtifactKind::from_operation(operation)
        .ok_or_else(|| format!("unsupported external init operation: {operation}"))?;
    let plan = plan_scaffold(
        kind,
        args,
        &context.cwd,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.workspace_root),
    )?;
    Ok(plan
        .artifacts
        .into_iter()
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("xml"))
        .collect())
}

fn create_scaffold(plan: &ScaffoldPlan, context: &WorkspaceContext) -> Result<Vec<String>, String> {
    let content = build_content(plan)?;
    validate_xml("external descriptor", &content.descriptor)?;
    if let Some(form_metadata) = content.form_metadata.as_deref() {
        validate_xml("form metadata", form_metadata)?;
    }
    if let Some(form_content) = content.form_content.as_deref() {
        validate_xml("managed form", form_content)?;
    }

    let mut transaction = CompileTransaction::new();
    transaction.create_utf8_bom_text(&plan.descriptor, &content.descriptor)?;
    transaction.create_utf8_bom_text(
        plan.object_dir.join("Ext/ObjectModule.bsl"),
        OBJECT_MODULE_STUB,
    )?;
    if let (Some(form_name), Some(form_metadata), Some(form_content)) = (
        plan.form_name.as_deref(),
        content.form_metadata.as_deref(),
        content.form_content.as_deref(),
    ) {
        let form_dir = plan.object_dir.join("Forms").join(form_name);
        transaction.create_utf8_bom_text(
            plan.object_dir
                .join("Forms")
                .join(format!("{form_name}.xml")),
            form_metadata,
        )?;
        transaction.create_utf8_bom_text(form_dir.join("Ext/Form.xml"), form_content)?;
        transaction
            .create_utf8_bom_text(form_dir.join("Ext/Form/Module.bsl"), form_add_module_bsl())?;
    }
    guard_active_format_containing_owner_for_new_output(
        &mut transaction,
        &plan.output_dir,
        context,
    )?;
    for target in plan
        .artifacts
        .iter()
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("xml"))
    {
        guard_active_format_containing_owner_for_new_output(&mut transaction, target, context)?;
    }

    let report =
        transaction.commit_with_post_validation(|| validate_published_scaffold(plan, context))?;
    Ok(report.cleanup_warnings)
}

fn build_content(plan: &ScaffoldPlan) -> Result<ScaffoldContent, String> {
    let descriptor = descriptor_xml(plan, crate::infrastructure::source_ids::fresh_uuid);
    let (form_metadata, form_content) = match plan.form_name.as_deref() {
        Some(form_name) => (
            Some(form_add_metadata_xml(
                form_name,
                form_name,
                plan.kind.root_tag(),
                FORMAT_VERSION,
                &crate::infrastructure::source_ids::fresh_uuid(),
                // External processors have no language profile of their own.
                unica_source_designer::language::EXTERNAL_SOURCE_TEXT_LANGUAGE,
            )),
            Some(form_add_content_xml(
                plan.kind.root_tag(),
                &plan.name,
                "Object",
                FORMAT_VERSION,
            )?),
        ),
        None => (None, None),
    };
    Ok(ScaffoldContent {
        descriptor,
        form_metadata,
        form_content,
    })
}

fn validate_published_scaffold(
    plan: &ScaffoldPlan,
    context: &WorkspaceContext,
) -> Result<(), String> {
    validate_metadata_owner_shape_8_3_27(&plan.descriptor, context, "external.init")?;

    if let Some(form_name) = plan.form_name.as_deref() {
        let form_path = plan
            .object_dir
            .join("Forms")
            .join(form_name)
            .join("Ext/Form.xml");
        let form_args = Map::from_iter([(
            "FormPath".to_string(),
            Value::String(form_path.display().to_string()),
        )]);
        require_validation("managed form", validate_form(&form_args, context))?;
    }
    Ok(())
}

fn require_validation(label: &str, outcome: AdapterOutcome) -> Result<(), String> {
    if outcome.ok {
        return Ok(());
    }
    let details = if outcome.errors.is_empty() {
        outcome
            .stdout
            .unwrap_or_else(|| "validation returned no diagnostics".to_string())
    } else {
        outcome.errors.join("; ")
    };
    Err(format!("{label} validation failed: {details}"))
}

/// The scaffold's answer is the set of files it created (ADR-0023).
fn scaffold_mutation(
    plan: &ScaffoldPlan,
    mode: ScaffoldMode,
) -> unica_execution_contract::mutation::MutationData {
    let mut mutation =
        unica_execution_contract::mutation::MutationData::new(matches!(mode, ScaffoldMode::Apply));
    for path in &plan.artifacts {
        mutation = mutation.created(path);
    }
    mutation
}

fn success_outcome(
    tool_name: &str,
    plan: &ScaffoldPlan,
    mode: ScaffoldMode,
    warnings: Vec<String>,
) -> AdapterOutcome {
    let (verb, summary) = match mode {
        ScaffoldMode::Preview => (
            "would create",
            format!(
                "dry run: {tool_name} would create {} scaffold",
                plan.kind.label()
            ),
        ),
        ScaffoldMode::Apply => (
            "created",
            format!("{tool_name} created {} scaffold", plan.kind.label()),
        ),
    };
    let artifacts = plan
        .artifacts
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    AdapterOutcome {
        ok: true,
        summary,
        changes: artifacts
            .iter()
            .map(|path| format!("{verb} {path}"))
            .collect(),
        warnings,
        errors: Vec::new(),
        artifacts,
        // ADR-0023: the scaffold's answer is the files it created, which
        // `artifacts` and `changes` already carry; the note about what to do
        // next belongs to the skill, not to the result.
        stdout: None,
        stderr: None,
        command: None,
    }
}

fn failure_outcome(tool_name: &str, error: String) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!("{tool_name} failed to create external artifact scaffold"),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![error.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{error}\n")),
        command: None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use crate::infrastructure::native_operations::NativeOperationAdapter;
    use crate::infrastructure::workspace::discover_workspace;
    use serde_json::{json, Map, Value};
    use std::collections::BTreeSet;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_workspace::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use unica_workspace::single_file_publisher::with_before_commit_hook;
    use unica_workspace::test_support as testing;

    #[test]
    fn epf_init_creates_make_ready_layout_with_optional_managed_form() {
        let root = temp_root("epf-init-layout");
        let context = discover_workspace(Some(root.clone())).unwrap();
        let args = map(json!({
            "Name": "ИмпортТоваров",
            "Synonym": "Импорт товаров & цен",
            "OutputDir": "external/epf",
            "FormName": "ОсновнаяФорма"
        }));

        let outcome = apply("epf-init", "unica.epf.init", &args, &context).unwrap();
        assert!(outcome.ok, "{:?}", outcome.errors);
        let descriptor = root.join("external/epf/ИмпортТоваров.xml");
        let object_dir = root.join("external/epf/ИмпортТоваров");
        for path in [
            descriptor.clone(),
            object_dir.join("Ext/ObjectModule.bsl"),
            object_dir.join("Forms/ОсновнаяФорма.xml"),
            object_dir.join("Forms/ОсновнаяФорма/Ext/Form.xml"),
            object_dir.join("Forms/ОсновнаяФорма/Ext/Form/Module.bsl"),
        ] {
            assert!(path.is_file(), "missing {}", path.display());
        }

        let bytes = fs::read(&descriptor).unwrap();
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let xml = String::from_utf8(bytes[3..].to_vec()).unwrap();
        assert!(xml.contains(r#"version="2.20""#), "{xml}");
        assert!(!xml.contains(r#"version="2.17""#), "{xml}");
        assert!(xml.contains("<ExternalDataProcessor uuid=\""));
        assert!(xml.contains("<xr:ClassId>c3831ec8-d8d5-4f93-8a22-f9bfae07327f</xr:ClassId>"));
        assert!(xml.contains("name=\"ExternalDataProcessorObject.ИмпортТоваров\""));
        assert!(xml.contains("<v8:content>Импорт товаров &amp; цен</v8:content>"));
        assert!(xml.contains(
            "<DefaultForm>ExternalDataProcessor.ИмпортТоваров.Form.ОсновнаяФорма</DefaultForm>"
        ));
        assert_eq!(xml.matches("<Form>ОсновнаяФорма</Form>").count(), 1);
        assert_metadata_uuids_v4(&xml, "ExternalDataProcessor", 4);

        let form_metadata_bytes = fs::read(object_dir.join("Forms/ОсновнаяФорма.xml")).unwrap();
        assert!(form_metadata_bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let form_metadata = String::from_utf8(form_metadata_bytes[3..].to_vec()).unwrap();
        assert!(
            form_metadata.contains(r#"version="2.20""#),
            "{form_metadata}"
        );
        assert_metadata_uuids_v4(&form_metadata, "Form", 1);

        let form_path = object_dir.join("Forms/ОсновнаяФорма/Ext/Form.xml");
        let form_bytes = fs::read(&form_path).unwrap();
        assert!(form_bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let form_xml = String::from_utf8(form_bytes[3..].to_vec()).unwrap();
        assert!(form_xml.contains(r#"version="2.20""#), "{form_xml}");
        assert!(
            form_xml.contains("<v8:Type>cfg:ExternalDataProcessorObject.ИмпортТоваров</v8:Type>")
        );
        assert!(form_xml.contains("<MainAttribute>true</MainAttribute>"));
        assert!(!form_xml.contains("<SavedData>"));
        assert!(roxmltree::Document::parse(&form_xml).is_ok());
        for path in [
            object_dir.join("Ext/ObjectModule.bsl"),
            object_dir.join("Forms/ОсновнаяФорма/Ext/Form/Module.bsl"),
        ] {
            let bytes = fs::read(path).unwrap();
            assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
            let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
            assert!(text.contains("\r\n"), "{text:?}");
            assert!(!text.replace("\r\n", "").contains('\n'), "{text:?}");
        }

        let validate_args = map(json!({"FormPath": form_path}));
        let validation = NativeOperationAdapter::invoke(
            "form-validate",
            "unica.form.validate",
            &validate_args,
            &context,
            false,
            false,
        )
        .unwrap();
        assert!(validation.ok, "{:?}", validation.errors);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn erf_init_creates_minimal_report_layout_without_a_form() {
        let root = temp_root("erf-init-layout");
        let context = discover_workspace(Some(root.clone())).unwrap();
        let args = map(json!({"Name": "Остатки", "OutputDir": "external/erf"}));
        let outcome = apply("erf-init", "unica.erf.init", &args, &context).unwrap();
        assert!(outcome.ok, "{:?}", outcome.errors);

        let descriptor = root.join("external/erf/Остатки.xml");
        let object_dir = root.join("external/erf/Остатки");
        assert!(object_dir.join("Ext/ObjectModule.bsl").is_file());
        assert!(!object_dir.join("Forms").exists());
        let bytes = fs::read(&descriptor).unwrap();
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let xml = String::from_utf8(bytes[3..].to_vec()).unwrap();
        assert!(xml.contains(r#"version="2.20""#), "{xml}");
        assert!(!xml.contains(r#"version="2.17""#), "{xml}");
        assert!(xml.contains("<ExternalReport uuid=\""));
        assert!(xml.contains("<xr:ClassId>e41aff26-25cf-4bb6-b6c1-3f478a75f374</xr:ClassId>"));
        assert!(xml.contains("name=\"ExternalReportObject.Остатки\""));
        assert_metadata_uuids_v4(&xml, "ExternalReport", 4);
        let document = roxmltree::Document::parse(&xml).unwrap();
        let properties = document
            .descendants()
            .find(|node| node.tag_name().name() == "Properties")
            .unwrap();
        assert_eq!(
            properties
                .children()
                .filter(|node| node.is_element())
                .map(|node| node.tag_name().name())
                .collect::<Vec<_>>(),
            vec![
                "Name",
                "Synonym",
                "Comment",
                "DefaultForm",
                "AuxiliaryForm",
                "MainDataCompositionSchema",
                "DefaultSettingsForm",
                "AuxiliarySettingsForm",
                "DefaultVariantForm",
                "VariantsStorage",
                "SettingsStorage",
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn erf_init_optional_form_uses_external_report_object_type() {
        let root = temp_root("erf-init-form");
        let context = discover_workspace(Some(root.clone())).unwrap();
        let args = map(json!({
            "Name": "Продажи",
            "OutputDir": "external/erf",
            "FormName": "ФормаОтчета"
        }));
        let outcome = apply("erf-init", "unica.erf.init", &args, &context).unwrap();
        assert!(outcome.ok, "{:?}", outcome.errors);

        let object_dir = root.join("external/erf/Продажи");
        let descriptor = fs::read_to_string(root.join("external/erf/Продажи.xml")).unwrap();
        assert!(descriptor
            .contains("<DefaultForm>ExternalReport.Продажи.Form.ФормаОтчета</DefaultForm>"));
        let form_path = object_dir.join("Forms/ФормаОтчета/Ext/Form.xml");
        let form_bytes = fs::read(&form_path).unwrap();
        let form_xml = String::from_utf8(form_bytes[3..].to_vec()).unwrap();
        assert!(form_xml.contains("<v8:Type>cfg:ExternalReportObject.Продажи</v8:Type>"));
        assert!(!form_xml.contains("ExternalDataProcessorObject"));

        let validate_args = map(json!({"FormPath": form_path}));
        let validation = NativeOperationAdapter::invoke(
            "form-validate",
            "unica.form.validate",
            &validate_args,
            &context,
            false,
            false,
        )
        .unwrap();
        assert!(validation.ok, "{:?}", validation.errors);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn external_init_dry_run_lists_files_without_writing() {
        let root = temp_root("external-init-dry-run");
        let context = discover_workspace(Some(root.clone())).unwrap();
        let args = map(json!({
            "Name": "Preview",
            "OutputDir": "external",
            "FormName": "Form"
        }));
        let outcome = NativeOperationAdapter::invoke(
            "epf-init",
            "unica.epf.init",
            &args,
            &context,
            true,
            true,
        )
        .unwrap();
        assert!(outcome.ok, "{:?}", outcome.errors);
        assert!(outcome.summary.contains("dry run"));
        assert_eq!(outcome.artifacts.len(), 5);
        assert!(outcome
            .changes
            .iter()
            .any(|change| change.contains("Preview.xml")));
        assert!(!root.join("external").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn external_init_rejects_invalid_name_and_existing_targets_without_mutation() {
        let root = temp_root("external-init-collision");
        let output = root.join("external");
        fs::create_dir_all(&output).unwrap();
        let existing = output.join("Existing.xml");
        fs::write(&existing, "sentinel").unwrap();
        let context = discover_workspace(Some(root.clone())).unwrap();

        for (name, expected) in [("Existing", "already exists"), ("1Invalid", "identifier")] {
            let args = map(json!({"Name": name, "OutputDir": "external"}));
            let outcome = apply("epf-init", "unica.epf.init", &args, &context).unwrap();
            assert!(!outcome.ok, "{name} unexpectedly succeeded");
            assert!(outcome.errors.iter().any(|error| error.contains(expected)));
        }
        assert_eq!(fs::read_to_string(&existing).unwrap(), "sentinel");
        assert!(!output.join("Existing").exists());
        assert!(!output.join("1Invalid.xml").exists());

        let directory_target = output.join("DirectoryOnly");
        fs::create_dir(&directory_target).unwrap();
        fs::write(directory_target.join("sentinel"), "keep").unwrap();
        let args = map(json!({"Name": "DirectoryOnly", "OutputDir": "external"}));
        let outcome = apply("erf-init", "unica.erf.init", &args, &context).unwrap();
        assert!(!outcome.ok);
        assert_eq!(
            fs::read_to_string(directory_target.join("sentinel")).unwrap(),
            "keep"
        );
        assert!(!output.join("DirectoryOnly.xml").exists());

        let args = map(json!({
            "Name": "Valid",
            "OutputDir": "external",
            "FormName": "../Escape"
        }));
        let outcome = apply("epf-init", "unica.epf.init", &args, &context).unwrap();
        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("FormName")));
        assert!(!output.join("Valid.xml").exists());
        assert!(!output.join("Valid").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn external_init_rolls_back_complete_scaffold_after_partial_publication_failure() {
        for (operation, tool_name, name, form_name) in [
            (
                "epf-init",
                "unica.epf.init",
                "RollbackProcessor",
                Some("MainForm"),
            ),
            ("erf-init", "unica.erf.init", "RollbackReport", None),
        ] {
            let root = temp_root(name);
            let context = discover_workspace(Some(root.clone())).unwrap();
            let mut args = map(json!({"Name": name, "OutputDir": "external"}));
            if let Some(form_name) = form_name {
                args.insert("FormName".to_string(), Value::String(form_name.to_string()));
            }

            let outcome = with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
                apply(operation, tool_name, &args, &context).unwrap()
            });

            assert!(!outcome.ok, "{operation} unexpectedly succeeded");
            assert!(
                outcome
                    .errors
                    .iter()
                    .any(|error| error.contains("after object files")),
                "{:?}",
                outcome.errors
            );
            assert!(
                !root.join("external").exists(),
                "{operation} left a partial output tree"
            );
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn external_init_rolls_back_when_locked_post_validation_fails() {
        let root = temp_root("post-validation-rollback");
        let context = discover_workspace(Some(root.clone())).unwrap();
        let args = map(json!({
            "Name": "Validated",
            "OutputDir": "external",
            "FormName": "MainForm"
        }));

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            apply("epf-init", "unica.epf.init", &args, &context).unwrap()
        });

        assert!(
            !outcome.ok,
            "post-validation failure unexpectedly succeeded"
        );
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("post-write validation")),
            "{:?}",
            outcome.errors
        );
        assert!(
            !root.join("external").exists(),
            "post-validation failure left a partial output tree"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn external_init_never_overwrites_a_concurrent_create_target() {
        let root = temp_root("concurrent-create");
        let context = discover_workspace(Some(root.clone())).unwrap();
        let args = map(json!({
            "Name": "Concurrent",
            "OutputDir": "external",
            "FormName": "MainForm"
        }));
        let concurrent_target = Arc::new(Mutex::new(None::<PathBuf>));
        let hook_target = Arc::clone(&concurrent_target);

        let outcome = with_before_commit_hook(
            move |target| {
                fs::write(target, b"concurrent replacement").unwrap();
                *hook_target.lock().unwrap() = Some(target.to_path_buf());
            },
            || apply("epf-init", "unica.epf.init", &args, &context).unwrap(),
        );

        assert!(!outcome.ok, "concurrent target was overwritten");
        let concurrent_target = concurrent_target
            .lock()
            .unwrap()
            .clone()
            .expect("publication hook was not reached");
        assert_eq!(
            fs::read(&concurrent_target).unwrap(),
            b"concurrent replacement"
        );
        for artifact in [
            root.join("external/Concurrent.xml"),
            root.join("external/Concurrent/Ext/ObjectModule.bsl"),
            root.join("external/Concurrent/Forms/MainForm.xml"),
            root.join("external/Concurrent/Forms/MainForm/Ext/Form.xml"),
            root.join("external/Concurrent/Forms/MainForm/Ext/Form/Module.bsl"),
        ] {
            if artifact != concurrent_target {
                assert!(
                    !artifact.exists(),
                    "failed transaction left {}",
                    artifact.display()
                );
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn external_init_reauthorizes_containing_owner_immediately_before_publication() {
        for (operation, tool) in [
            ("epf-init", "unica.epf.init"),
            ("erf-init", "unica.erf.init"),
        ] {
            let root = temp_root(&format!("external-owner-race-{operation}"));
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            let owner = root.join("src/Configuration.xml");
            fs::write(
                &owner,
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
            )
            .unwrap();
            let context = discover_workspace(Some(root.clone())).unwrap();
            let args = map(json!({
                "Name": "ConcurrentOwner",
                "OutputDir": "src/external"
            }));
            let owner_for_hook = owner.clone();
            let concurrent_owner = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#.to_vec();

            let outcome = with_before_commit_hook(
                move |_| fs::write(&owner_for_hook, &concurrent_owner).unwrap(),
                || apply(operation, tool, &args, &context).unwrap(),
            );

            assert!(!outcome.ok, "{operation}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains("changed after planning"),
                "{operation}: {outcome:?}"
            );
            assert!(fs::read_to_string(&owner)
                .unwrap()
                .contains(r#"version="2.21""#));
            assert!(!root.join("src/external").exists());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn public_external_init_rolls_back_if_an_unplanned_root_descriptor_appears() {
        let root = temp_root("external-root-membership-race");
        let external_root = root.join("external");
        fs::create_dir_all(&external_root).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        fs::write(
            external_root.join("Existing.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();
        let concurrent = external_root.join("Bar.xml");
        let concurrent_bytes = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#.to_vec();
        let concurrent_for_hook = concurrent.clone();
        let concurrent_bytes_for_hook = concurrent_bytes.clone();
        let args = map(json!({
            "cwd": root.display().to_string(),
            "Name": "Planned",
            "OutputDir": "external",
            "dryRun": false
        }));

        let outcome = with_before_commit_hook(
            move |_| {
                fs::write(&concurrent_for_hook, &concurrent_bytes_for_hook).unwrap();
            },
            || {
                UnicaApplication::new()
                    .call_tool("unica.epf.init", &args)
                    .unwrap()
            },
        );

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("directory membership guard"), "{error}");
        assert_eq!(fs::read(&concurrent).unwrap(), concurrent_bytes);
        assert!(!external_root.join("Planned.xml").exists());
        assert!(!external_root.join("Planned").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_external_init_accepts_multiple_supported_root_descriptors() {
        let root = temp_root("external-root-multiple-supported");
        let external_root = root.join("external");
        fs::create_dir_all(&external_root).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        let supported = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#;
        fs::write(external_root.join("First.xml"), supported).unwrap();
        fs::write(external_root.join("Second.xml"), supported).unwrap();
        let args = map(json!({
            "cwd": root.display().to_string(),
            "Name": "Third",
            "OutputDir": "external",
            "dryRun": false
        }));

        let outcome = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap();

        assert!(outcome.ok, "{outcome:?}");
        assert!(external_root.join("Third.xml").is_file());
        assert!(external_root.join("Third/Ext/ObjectModule.bsl").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normalized_output_path_never_traverses_symlink_before_parent_component() {
        let root = temp_root("normalized-output");
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(outside.join("target")).unwrap();
        let Some(symlink) =
            testing::create_dir_symlink_for_test(outside.join("target"), workspace.join("link"))
        else {
            let _ = fs::remove_dir_all(root);
            return;
        };
        symlink.unwrap();
        let context = discover_workspace(Some(workspace.clone())).unwrap();
        let args = map(json!({
            "Name": "Safe",
            "OutputDir": "link/../external"
        }));

        let outcome = apply("epf-init", "unica.epf.init", &args, &context).unwrap();

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert!(workspace.join("external/Safe.xml").is_file());
        assert!(!outside.join("external/Safe.xml").exists());
        let _ = fs::remove_dir_all(root);
    }

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn assert_metadata_uuids_v4(xml: &str, root_tag: &str, expected_count: usize) {
        let document = roxmltree::Document::parse(xml).unwrap();
        let root = document
            .descendants()
            .find(|node| node.tag_name().name() == root_tag)
            .unwrap();
        let mut values = vec![root.attribute("uuid").unwrap().to_string()];
        for tag in ["ObjectId", "TypeId", "ValueId"] {
            if let Some(value) = document
                .descendants()
                .find(|node| node.tag_name().name() == tag)
                .and_then(|node| node.text())
            {
                values.push(value.to_string());
            }
        }
        assert_eq!(values.len(), expected_count);
        assert_eq!(values.iter().collect::<BTreeSet<_>>().len(), expected_count);
        for value in values {
            let uuid = Uuid::parse_str(&value).unwrap();
            assert!(!uuid.is_nil());
            assert_eq!(uuid.get_version(), Some(uuid::Version::Random));
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-external-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
