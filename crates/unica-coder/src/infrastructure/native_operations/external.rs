use super::cf::cf_validate_identifier;
use super::common::{escape_xml, path_arg, string_arg, write_utf8_bom};
use super::form::{form_add_content_xml, form_add_metadata_xml, form_add_module_bsl};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::AdapterOutcome;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

const FORMAT_VERSION: &str = "2.17";
const OBJECT_MODULE_STUB: &str = "#Область ПрограммныйИнтерфейс\n\n#КонецОбласти\n";

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
        (Ok(plan), ScaffoldMode::Apply) => match create_scaffold(&plan) {
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
    for target in [&descriptor, &object_dir] {
        if target.exists() {
            return Err(format!("target already exists: {}", target.display()));
        }
    }

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

fn create_scaffold(plan: &ScaffoldPlan) -> Result<Vec<String>, String> {
    let content = build_content(plan)?;
    validate_xml("external descriptor", &content.descriptor)?;
    if let Some(form_metadata) = content.form_metadata.as_deref() {
        validate_xml("form metadata", form_metadata)?;
    }
    if let Some(form_content) = content.form_content.as_deref() {
        validate_xml("managed form", form_content)?;
    }

    let output_existed = plan.output_dir.exists();
    fs::create_dir_all(&plan.output_dir)
        .map_err(|error| format!("failed to create {}: {error}", plan.output_dir.display()))?;
    let staging = plan
        .output_dir
        .join(format!(".unica-external-init-{}.tmp", Uuid::new_v4()));
    if let Err(error) = fs::create_dir(&staging) {
        let mut cleanup_errors = Vec::new();
        if !output_existed {
            remove_empty_dir(&plan.output_dir, &mut cleanup_errors);
        }
        return Err(with_cleanup_errors(
            format!("failed to create staging directory: {error}"),
            cleanup_errors,
        ));
    }

    let result = write_and_commit_staging(plan, &content, &staging);
    match result {
        Ok(warnings) => Ok(warnings),
        Err(error) => {
            let mut cleanup_errors = Vec::new();
            if let Err(cleanup_error) = fs::remove_dir_all(&staging) {
                cleanup_errors.push(format!(
                    "failed to remove staging directory {}: {cleanup_error}",
                    staging.display()
                ));
            }
            if !output_existed {
                remove_empty_dir(&plan.output_dir, &mut cleanup_errors);
            }
            Err(with_cleanup_errors(error, cleanup_errors))
        }
    }
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

fn write_and_commit_staging(
    plan: &ScaffoldPlan,
    content: &ScaffoldContent,
    staging: &Path,
) -> Result<Vec<String>, String> {
    let staged_descriptor = staging.join(format!("{}.xml", plan.name));
    let staged_object_dir = staging.join(&plan.name);
    let staged_object_ext = staged_object_dir.join("Ext");
    fs::create_dir_all(&staged_object_ext)
        .map_err(|error| format!("failed to create object module directory: {error}"))?;
    write_utf8_bom(&staged_descriptor, &content.descriptor)?;
    write_utf8_bom(
        &staged_object_ext.join("ObjectModule.bsl"),
        OBJECT_MODULE_STUB,
    )?;

    if let (Some(form_name), Some(form_metadata), Some(form_content)) = (
        plan.form_name.as_deref(),
        content.form_metadata.as_deref(),
        content.form_content.as_deref(),
    ) {
        let forms_dir = staged_object_dir.join("Forms");
        let form_ext = forms_dir.join(form_name).join("Ext");
        let form_module_dir = form_ext.join("Form");
        fs::create_dir_all(&form_module_dir)
            .map_err(|error| format!("failed to create managed form directories: {error}"))?;
        write_utf8_bom(&forms_dir.join(format!("{form_name}.xml")), form_metadata)?;
        write_utf8_bom(&form_ext.join("Form.xml"), form_content)?;
        write_utf8_bom(&form_module_dir.join("Module.bsl"), form_add_module_bsl())?;
    }

    for target in [&plan.descriptor, &plan.object_dir] {
        if target.exists() {
            return Err(format!("target already exists: {}", target.display()));
        }
    }
    fs::create_dir(&plan.object_dir).map_err(|error| {
        format!(
            "failed to reserve object directory {} without overwrite: {error}",
            plan.object_dir.display()
        )
    })?;
    let mut published_subtrees = Vec::new();
    let publish_object = (|| -> Result<(), String> {
        let final_ext = plan.object_dir.join("Ext");
        fs::rename(staged_object_dir.join("Ext"), &final_ext)
            .map_err(|error| format!("failed to publish object module directory: {error}"))?;
        published_subtrees.push(final_ext);
        let staged_forms = staged_object_dir.join("Forms");
        if staged_forms.exists() {
            let final_forms = plan.object_dir.join("Forms");
            fs::rename(&staged_forms, &final_forms)
                .map_err(|error| format!("failed to publish managed form directory: {error}"))?;
            published_subtrees.push(final_forms);
        }
        Ok(())
    })();
    if let Err(error) = publish_object {
        return Err(with_cleanup_errors(
            error,
            rollback_published_object(plan, &published_subtrees),
        ));
    }
    if let Err(error) = fs::hard_link(&staged_descriptor, &plan.descriptor) {
        return Err(with_cleanup_errors(
            format!(
                "failed to publish descriptor {} without overwrite: {error}",
                plan.descriptor.display()
            ),
            rollback_published_object(plan, &published_subtrees),
        ));
    }

    let mut warnings = Vec::new();
    if let Err(error) = fs::remove_dir_all(staging) {
        warnings.push(format!(
            "scaffold committed but staging cleanup failed for {}: {error}",
            staging.display()
        ));
    }
    Ok(warnings)
}

fn rollback_published_object(plan: &ScaffoldPlan, published_subtrees: &[PathBuf]) -> Vec<String> {
    let mut cleanup_errors = Vec::new();
    let published_files = plan.artifacts.iter().skip(1).filter(|path| {
        published_subtrees
            .iter()
            .any(|subtree| path.starts_with(subtree))
    });
    for path in published_files.rev() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => cleanup_errors.push(format!(
                "failed to remove published file {}: {error}",
                path.display()
            )),
        }
    }

    let mut directories = Vec::new();
    for path in plan.artifacts.iter().skip(1).filter(|path| {
        published_subtrees
            .iter()
            .any(|subtree| path.starts_with(subtree))
    }) {
        let mut parent = path.parent();
        while let Some(directory) = parent {
            if !directory.starts_with(&plan.object_dir) {
                break;
            }
            directories.push(directory.to_path_buf());
            if directory == plan.object_dir {
                break;
            }
            parent = directory.parent();
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    directories.dedup();
    for directory in directories {
        remove_empty_dir(&directory, &mut cleanup_errors);
    }
    if plan.object_dir.exists() {
        remove_empty_dir(&plan.object_dir, &mut cleanup_errors);
    }
    cleanup_errors
}

fn remove_empty_dir(path: &Path, cleanup_errors: &mut Vec<String>) {
    match fs::remove_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => cleanup_errors.push(format!(
            "failed to remove empty directory {}: {error}",
            path.display()
        )),
    }
}

fn with_cleanup_errors(error: String, cleanup_errors: Vec<String>) -> String {
    if cleanup_errors.is_empty() {
        error
    } else {
        format!("{error}; cleanup incomplete: {}", cleanup_errors.join("; "))
    }
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
    use crate::infrastructure::native_operations::NativeOperationAdapter;
    use serde_json::{json, Map, Value};
    use std::collections::BTreeSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn epf_init_creates_make_ready_layout_with_optional_managed_form() {
        let root = temp_root("epf-init-layout");
        let context = WorkspaceContext::discover(root.clone()).unwrap();
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
        assert_metadata_uuids_v4(&form_metadata, "Form", 1);

        let form_path = object_dir.join("Forms/ОсновнаяФорма/Ext/Form.xml");
        let form_bytes = fs::read(&form_path).unwrap();
        assert!(form_bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let form_xml = String::from_utf8(form_bytes[3..].to_vec()).unwrap();
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
            assert!(fs::read(path).unwrap().starts_with(&[0xef, 0xbb, 0xbf]));
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
        let context = WorkspaceContext::discover(root.clone()).unwrap();
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
        let context = WorkspaceContext::discover(root.clone()).unwrap();
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
        let context = WorkspaceContext::discover(root.clone()).unwrap();
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
        let context = WorkspaceContext::discover(root.clone()).unwrap();

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
    fn rollback_removes_only_known_files_and_preserves_unexpected_entries() {
        let root = temp_root("rollback-preserves-concurrent");
        let context = WorkspaceContext::discover(root.clone()).unwrap();
        let args = map(json!({"Name": "Rollback", "OutputDir": "external"}));
        let plan = prepare_plan(ExternalArtifactKind::Processor, &args, &context).unwrap();
        fs::create_dir_all(plan.object_dir.join("Ext")).unwrap();
        fs::write(
            plan.object_dir.join("Ext/ObjectModule.bsl"),
            "published by this operation",
        )
        .unwrap();
        let unexpected = plan.object_dir.join("concurrent-sentinel.txt");
        fs::write(&unexpected, "must survive rollback").unwrap();

        let cleanup_errors = rollback_published_object(&plan, &[plan.object_dir.join("Ext")]);

        assert!(!plan.object_dir.join("Ext/ObjectModule.bsl").exists());
        assert_eq!(
            fs::read_to_string(&unexpected).unwrap(),
            "must survive rollback"
        );
        assert!(plan.object_dir.is_dir());
        assert!(cleanup_errors
            .iter()
            .any(|error| error.contains("failed to remove empty directory")));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn normalized_output_path_never_traverses_symlink_before_parent_component() {
        let root = temp_root("normalized-output");
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(outside.join("target")).unwrap();
        std::os::unix::fs::symlink(outside.join("target"), workspace.join("link")).unwrap();
        let context = WorkspaceContext::discover(workspace.clone()).unwrap();
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
