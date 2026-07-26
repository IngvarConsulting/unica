use super::cf::cf_validate_identifier;
use super::common::{
    escape_xml, guard_active_format_containing_owner_for_new_output, path_arg, string_arg,
};
use super::compile_transaction::CompileTransaction;
use super::form::{
    form_add_content_xml, form_add_metadata_xml, form_add_module_bsl, validate_form,
};
use super::meta::validate_meta;
use crate::application::AdapterOutcome;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

const FORMAT_VERSION: &str = ACTIVE_FORMAT_PROFILE.export_format;
const OBJECT_MODULE_STUB: &str = "#Область ПрограммныйИнтерфейс\r\n\r\n#КонецОбласти\r\n";

#[derive(Debug, Clone, Copy)]
enum ExternalArtifactKind {
    Processor,
    Report,
}

impl ExternalArtifactKind {
    fn from_operation(operation: &str) -> Option<Self> {
        match operation {
            "epf-init" => Some(Self::Processor),
            "erf-init" => Some(Self::Report),
            _ => None,
        }
    }

    fn root_tag(self) -> &'static str {
        match self {
            Self::Processor => "ExternalDataProcessor",
            Self::Report => "ExternalReport",
        }
    }

    fn class_id(self) -> &'static str {
        match self {
            Self::Processor => "c3831ec8-d8d5-4f93-8a22-f9bfae07327f",
            Self::Report => "e41aff26-25cf-4bb6-b6c1-3f478a75f374",
        }
    }

    fn object_type(self) -> &'static str {
        match self {
            Self::Processor => "ExternalDataProcessorObject",
            Self::Report => "ExternalReportObject",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Processor => "EPF",
            Self::Report => "ERF",
        }
    }
}

#[derive(Debug)]
struct ScaffoldPlan {
    kind: ExternalArtifactKind,
    name: String,
    synonym: String,
    form_name: Option<String>,
    output_dir: PathBuf,
    descriptor: PathBuf,
    object_dir: PathBuf,
    artifacts: Vec<PathBuf>,
}

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
    let plan = plan_scaffold(kind, args, context)?;
    for target in [&plan.descriptor, &plan.object_dir] {
        if target.exists() {
            return Err(format!("target already exists: {}", target.display()));
        }
    }
    Ok(plan)
}

fn plan_scaffold(
    kind: ExternalArtifactKind,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ScaffoldPlan, String> {
    let name =
        string_arg(args, &["Name"]).ok_or_else(|| "missing required Name argument".to_string())?;
    validate_identifier("Name", name)?;
    let synonym = string_arg(args, &["Synonym"]).unwrap_or(name);
    let form_name = string_arg(args, &["FormName"])
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    if let Some(form_name) = form_name.as_deref() {
        validate_identifier("FormName", form_name)?;
    } else if args.get("FormName").is_some() {
        return Err("FormName must be a non-empty 1C identifier".to_string());
    }

    let output_dir = path_arg(args, &["OutputDir"])
        .ok_or_else(|| "missing required OutputDir argument".to_string())?;
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        context.cwd.join(output_dir)
    };
    let output_dir = normalize_lexical_path(&output_dir);
    if output_dir.exists() && !output_dir.is_dir() {
        return Err(format!(
            "OutputDir is not a directory: {}",
            output_dir.display()
        ));
    }
    let descriptor = output_dir.join(format!("{name}.xml"));
    let object_dir = output_dir.join(name);

    let mut artifacts = vec![descriptor.clone(), object_dir.join("Ext/ObjectModule.bsl")];
    if let Some(form_name) = form_name.as_deref() {
        artifacts.extend([
            object_dir.join("Forms").join(format!("{form_name}.xml")),
            object_dir
                .join("Forms")
                .join(form_name)
                .join("Ext/Form.xml"),
            object_dir
                .join("Forms")
                .join(form_name)
                .join("Ext/Form/Module.bsl"),
        ]);
    }

    Ok(ScaffoldPlan {
        kind,
        name: name.to_string(),
        synonym: synonym.to_string(),
        form_name,
        output_dir,
        descriptor,
        object_dir,
        artifacts,
    })
}

pub(crate) fn external_init_planned_xml_paths(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let kind = ExternalArtifactKind::from_operation(operation)
        .ok_or_else(|| format!("unsupported external init operation: {operation}"))?;
    let plan = plan_scaffold(kind, args, context)?;
    Ok(plan
        .artifacts
        .into_iter()
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("xml"))
        .collect())
}

fn validate_identifier(argument: &str, value: &str) -> Result<(), String> {
    if cf_validate_identifier(value) {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid 1C identifier: {value:?}"
        ))
    }
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
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
    let descriptor = descriptor_xml(plan);
    let (form_metadata, form_content) = match plan.form_name.as_deref() {
        Some(form_name) => (
            Some(form_add_metadata_xml(
                form_name,
                form_name,
                plan.kind.root_tag(),
                FORMAT_VERSION,
                &Uuid::new_v4().to_string(),
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
    let descriptor_args = Map::from_iter([(
        "ObjectPath".to_string(),
        Value::String(plan.descriptor.display().to_string()),
    )]);
    require_validation(
        "external descriptor",
        validate_meta(&descriptor_args, context),
    )?;

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

fn descriptor_xml(plan: &ScaffoldPlan) -> String {
    let root_tag = plan.kind.root_tag();
    let default_form = plan.form_name.as_deref().map_or_else(
        || "\t\t\t<DefaultForm/>".to_string(),
        |form_name| {
            format!(
                "\t\t\t<DefaultForm>{}.{}.Form.{}</DefaultForm>",
                root_tag,
                escape_xml(&plan.name),
                escape_xml(form_name)
            )
        },
    );
    let child_objects = plan.form_name.as_deref().map_or_else(
        || "\t\t<ChildObjects/>".to_string(),
        |form_name| {
            format!(
                "\t\t<ChildObjects>\n\t\t\t<Form>{}</Form>\n\t\t</ChildObjects>",
                escape_xml(form_name)
            )
        },
    );
    let report_properties = match plan.kind {
        ExternalArtifactKind::Processor => String::new(),
        ExternalArtifactKind::Report => concat!(
            "\n\t\t\t<MainDataCompositionSchema/>",
            "\n\t\t\t<DefaultSettingsForm/>",
            "\n\t\t\t<AuxiliarySettingsForm/>",
            "\n\t\t\t<DefaultVariantForm/>",
            "\n\t\t\t<VariantsStorage/>",
            "\n\t\t\t<SettingsStorage/>"
        )
        .to_string(),
    };
    let root_uuid = Uuid::new_v4();
    let object_id = Uuid::new_v4();
    let type_id = Uuid::new_v4();
    let value_id = Uuid::new_v4();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{FORMAT_VERSION}">
	<{root_tag} uuid="{root_uuid}">
		<InternalInfo>
			<xr:ContainedObject>
				<xr:ClassId>{class_id}</xr:ClassId>
				<xr:ObjectId>{object_id}</xr:ObjectId>
			</xr:ContainedObject>
			<xr:GeneratedType name="{object_type}.{name}" category="Object">
				<xr:TypeId>{type_id}</xr:TypeId>
				<xr:ValueId>{value_id}</xr:ValueId>
			</xr:GeneratedType>
		</InternalInfo>
		<Properties>
			<Name>{name}</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>{synonym}</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
{default_form}
			<AuxiliaryForm/>{report_properties}
		</Properties>
{child_objects}
	</{root_tag}>
</MetaDataObject>"#,
        class_id = plan.kind.class_id(),
        object_type = plan.kind.object_type(),
        name = escape_xml(&plan.name),
        synonym = escape_xml(&plan.synonym),
    )
}

fn validate_xml(label: &str, text: &str) -> Result<(), String> {
    roxmltree::Document::parse(text)
        .map(|_| ())
        .map_err(|error| format!("generated {label} is invalid XML: {error}"))
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
        stdout: Some(format!(
            "{} scaffold: {}\nSource-set root: {}\nGenerated XML structure validated before publication.\nNext: ensure this root is declared in v8project.yaml and run unica.runtime.execute operation=make.\n",
            plan.kind.label(),
            plan.name,
            plan.output_dir.display()
        )),
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
mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_commit_failpoint, CommitFailpoint,
    };
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use crate::infrastructure::native_operations::NativeOperationAdapter;
    use crate::infrastructure::platform::testing;
    use crate::infrastructure::workspace::discover_workspace;
    use serde_json::{json, Map, Value};
    use std::collections::BTreeSet;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn external_init_rejects_invalid_name_and_existing_targets_without_mutation() {
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
    fn external_init_reauthorizes_containing_owner_immediately_before_publication() {
        let root = temp_root("external-owner-race");
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
            || apply("epf-init", "unica.epf.init", &args, &context).unwrap(),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("changed after planning"),
            "{outcome:?}"
        );
        assert!(fs::read_to_string(&owner)
            .unwrap()
            .contains(r#"version="2.21""#));
        assert!(!root.join("src/external").exists());
        let _ = fs::remove_dir_all(root);
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
