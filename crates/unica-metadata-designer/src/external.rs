//! Designer external processor/report scaffold layout and descriptor encoding.
use serde_json::{Map, Value};
use std::path::{Component, Path, PathBuf};
use unica_metadata::identifier::is_1c_identifier;
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;

const FORMAT_VERSION: &str = ACTIVE_FORMAT_PROFILE.export_format;

#[derive(Debug, Clone, Copy)]
pub enum ExternalArtifactKind {
    Processor,
    Report,
}

impl ExternalArtifactKind {
    pub fn from_operation(operation: &str) -> Option<Self> {
        match operation {
            "epf-init" => Some(Self::Processor),
            "erf-init" => Some(Self::Report),
            _ => None,
        }
    }

    pub fn root_tag(self) -> &'static str {
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Processor => "EPF",
            Self::Report => "ERF",
        }
    }
}

#[derive(Debug)]
pub struct ScaffoldPlan {
    pub kind: ExternalArtifactKind,
    pub name: String,
    pub synonym: String,
    pub form_name: Option<String>,
    pub output_dir: PathBuf,
    pub descriptor: PathBuf,
    pub object_dir: PathBuf,
    pub artifacts: Vec<PathBuf>,
}

pub fn plan_scaffold(
    kind: ExternalArtifactKind,
    args: &Map<String, Value>,
    cwd: &Path,
    reader: &dyn ProjectDiscoveryRead,
) -> Result<ScaffoldPlan, String> {
    let name = unica_operation_contract::input::string_arg(args, &["Name"])
        .ok_or_else(|| "missing required Name argument".to_string())?;
    validate_identifier("Name", name)?;
    let synonym = unica_operation_contract::input::string_arg(args, &["Synonym"]).unwrap_or(name);
    let form_name = unica_operation_contract::input::string_arg(args, &["FormName"])
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    if let Some(form_name) = form_name.as_deref() {
        validate_identifier("FormName", form_name)?;
    } else if args.get("FormName").is_some() {
        return Err("FormName must be a non-empty 1C identifier".to_string());
    }

    let output_dir = unica_operation_contract::input::path_arg(args, &["OutputDir"])
        .ok_or_else(|| "missing required OutputDir argument".to_string())?;
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        cwd.join(output_dir)
    };
    let output_dir = normalize_lexical_path(&output_dir);
    if unica_source_designer::read::path_exists(reader, &output_dir)
        && !unica_source_designer::read::path_is_dir(reader, &output_dir)
    {
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

fn validate_identifier(argument: &str, value: &str) -> Result<(), String> {
    if is_1c_identifier(value) {
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

pub fn descriptor_xml(plan: &ScaffoldPlan, fresh_uuid: fn() -> String) -> String {
    let root_tag = plan.kind.root_tag();
    let default_form = plan.form_name.as_deref().map_or_else(
        || "\t\t\t<DefaultForm/>".to_string(),
        |form_name| {
            format!(
                "\t\t\t<DefaultForm>{}.{}.Form.{}</DefaultForm>",
                root_tag,
                unica_xml_format::text::escape_xml(&plan.name),
                unica_xml_format::text::escape_xml(form_name)
            )
        },
    );
    let child_objects = plan.form_name.as_deref().map_or_else(
        || "\t\t<ChildObjects/>".to_string(),
        |form_name| {
            format!(
                "\t\t<ChildObjects>\n\t\t\t<Form>{}</Form>\n\t\t</ChildObjects>",
                unica_xml_format::text::escape_xml(form_name)
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
    let root_uuid = fresh_uuid();
    let object_id = fresh_uuid();
    let type_id = fresh_uuid();
    let value_id = fresh_uuid();

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
        name = unica_xml_format::text::escape_xml(&plan.name),
        synonym = unica_xml_format::text::escape_xml(&plan.synonym),
    )
}

pub fn validate_xml(label: &str, text: &str) -> Result<(), String> {
    roxmltree::Document::parse(text)
        .map(|_| ())
        .map_err(|error| format!("generated {label} is invalid XML: {error}"))
}
