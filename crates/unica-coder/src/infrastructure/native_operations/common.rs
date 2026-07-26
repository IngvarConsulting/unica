#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::{CFE_VALIDATE_PATH, CF_PATH, RIGHTS_PATH};
use crate::application::{AdapterOutcome, SupportGuardRequirement};
use crate::domain::format_profile::{FormatCompatibility, ACTIVE_FORMAT_PROFILE};
use crate::domain::source_adapters::FormatVersion;
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::ports::{EffectiveSupportRule, SupportSourceState};

use super::compile_transaction::CompileTransaction;
use super::single_file_publisher::{publish, PublishMode, PublishRequest};
use super::{
    cf::*, cfe::*, dcs::*, form::*, interface::*, meta::*, mxl::*, role::*, subsystem::*,
    template::*,
};
pub(crate) fn resolve_form_info_path(mut form_path: PathBuf) -> PathBuf {
    if form_path.is_dir() {
        form_path = form_path.join("Ext").join("Form.xml");
    }
    if !form_path.is_file()
        && form_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "Form.xml")
    {
        let candidate = form_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Form.xml");
        if candidate.is_file() {
            form_path = candidate;
        }
    }
    if !form_path.is_file()
        && form_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
    {
        let stem = form_path.file_stem().and_then(|stem| stem.to_str());
        if let (Some(stem), Some(parent)) = (stem, form_path.parent()) {
            let candidate = parent.join(stem).join("Ext").join("Form.xml");
            if candidate.is_file() {
                form_path = candidate;
            }
        }
    }
    form_path
}

pub(crate) fn resolve_form_add_object_path(mut object_path: PathBuf) -> Result<PathBuf, String> {
    if object_path.is_dir() {
        let dir_name = object_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let candidate = object_path.join(format!("{dir_name}.xml"));
        let sibling = object_path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")))
            .unwrap_or_else(|| PathBuf::from(format!("{dir_name}.xml")));
        if candidate.is_file() {
            object_path = candidate;
        } else if sibling.is_file() {
            object_path = sibling;
        }
    }
    if !object_path.is_file() {
        return Err(format!("Файл объекта не найден: {}", object_path.display()));
    }
    Ok(object_path.canonicalize().unwrap_or(object_path))
}

pub(crate) fn detect_form_add_object(object_text: &str) -> Result<(String, String), String> {
    let supported = form_add_supported_object_types();
    let doc = Document::parse(object_text)
        .map_err(|err| format!("XML parse error in object XML: {err}"))?;
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        let object_type = node.tag_name().name();
        if !supported.contains(&object_type) {
            continue;
        }
        let Some(props) = meta_info_child(node, "Properties") else {
            continue;
        };
        let Some(object_name) = meta_info_child_text(props, "Name") else {
            continue;
        };
        if !object_name.is_empty() {
            return Ok((object_type.to_string(), object_name));
        }
    }
    Err(format!(
        "Не удалось определить тип объекта. Поддерживаемые типы: {}",
        supported.join(", ")
    ))
}

pub(crate) fn validate_form_purpose(object_type: &str, purpose: &str) -> Result<(), String> {
    const VALID: &[&str] = &["Object", "List", "Choice", "Record"];
    if !VALID.contains(&purpose) {
        return Err(format!(
            "Недопустимое назначение: {purpose}. Допустимые: Object, List, Choice, Record"
        ));
    }
    if purpose == "List" && object_type == "DataProcessor" {
        return Err("Purpose=List недопустим для DataProcessor".to_string());
    }
    if purpose == "Choice"
        && (form_add_processor_like(object_type) || object_type == "InformationRegister")
    {
        return Err(format!("Purpose=Choice недопустим для {object_type}"));
    }
    if purpose == "Record" && object_type != "InformationRegister" {
        return Err("Purpose=Record допустим только для InformationRegister".to_string());
    }
    Ok(())
}

pub(crate) fn register_form_in_object_text(text: &str, form_name: &str) -> String {
    let form_tag = format!("<Form>{form_name}</Form>");
    if let Some(child_start) = text.find("<ChildObjects>") {
        if let Some(relative_end) = text[child_start..].find("</ChildObjects>") {
            let child_end = child_start + relative_end;
            let section = &text[child_start..child_end];
            let template_idx = section.find("\n\t\t\t<Template");
            let tabular_idx = section.find("\n\t\t\t<TabularSection");
            let insert_text = format!("\t\t\t{form_tag}\n");
            if let Some(insert_idx) = template_idx.or(tabular_idx).map(|idx| idx + 1) {
                let absolute_insert = child_start + insert_idx;
                return format!(
                    "{}{}{}",
                    &text[..absolute_insert],
                    insert_text,
                    &text[absolute_insert..]
                );
            }
            return format!(
                "{}\t\t\t{}\n\t\t{}",
                &text[..child_end],
                form_tag,
                &text[child_end..]
            );
        }
    }

    if text.contains("<ChildObjects/>") {
        return text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n\t\t\t{form_tag}\n\t\t</ChildObjects>"),
            1,
        );
    }
    text.to_string()
}

#[derive(Clone)]
pub(crate) struct Utf8TextSnapshot {
    pub(crate) raw: Vec<u8>,
    pub(crate) text: String,
}

/// Exact regular-file bytes used to derive a mutation plan.
///
/// Binding the input to the same [`CompileTransaction`] as the derived output
/// makes a concurrent edit fail the transaction instead of publishing output
/// calculated from stale bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExactFileInput {
    path: PathBuf,
    raw: Vec<u8>,
}

impl ExactFileInput {
    pub(crate) fn new(path: impl Into<PathBuf>, raw: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            raw: raw.into(),
        }
    }

    pub(crate) fn bind_to(&self, transaction: &mut CompileTransaction) -> Result<(), String> {
        transaction.guard_or_verify_exact_preimage(&self.path, &self.raw)
    }
}

/// JSON parsed from one exact byte snapshot. The parsed value is deliberately
/// unavailable until those source bytes are bound to the publication
/// transaction.
pub(crate) struct FileBackedJson {
    input: ExactFileInput,
    value: Value,
}

impl FileBackedJson {
    pub(crate) fn read(
        path: &Path,
        parse_error: impl FnOnce(serde_json::Error) -> String,
    ) -> Result<Self, String> {
        let snapshot = read_utf8_sig_snapshot(path)?;
        let value = serde_json::from_str(snapshot.text.as_str()).map_err(parse_error)?;
        Ok(Self {
            input: ExactFileInput::new(path, snapshot.raw),
            value,
        })
    }

    pub(crate) fn bind_to(self, transaction: &mut CompileTransaction) -> Result<Value, String> {
        self.input.bind_to(transaction)?;
        Ok(self.value)
    }
}

pub(crate) fn read_utf8_sig_snapshot(path: &Path) -> Result<Utf8TextSnapshot, String> {
    let raw =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let text = std::str::from_utf8(&raw)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?
        .trim_start_matches('\u{feff}')
        .to_string();
    Ok(Utf8TextSnapshot { raw, text })
}

pub(crate) fn read_utf8_sig(path: &Path) -> Result<String, String> {
    let mut text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    while text.starts_with('\u{feff}') {
        text.remove(0);
    }
    Ok(text)
}

pub(crate) fn ensure_trailing_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

pub(crate) fn count_files_recursive(path: &Path) -> usize {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                count_files_recursive(&path)
            } else if path.is_file() {
                1
            } else {
                0
            }
        })
        .sum()
}

pub(crate) fn relative_display(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(crate) fn remove_object_from_subsystems(
    dir: &Path,
    obj_type: &str,
    obj_name: &str,
    dry_run: bool,
    stdout: &mut String,
    subsystems_cleaned: &mut usize,
    changes: &mut Vec<String>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|err| format!("failed to read {}: {err}", dir.display()))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
        {
            continue;
        }

        let mut text = match read_utf8_sig(&path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let subsystem_name =
            first_tag_text_in_xml(&text, "Name").unwrap_or_else(|| file_stem_string(&path));
        let mut modified = false;
        loop {
            let (next_text, removed) = remove_metadata_child_text_with_flag(
                &text,
                "Item",
                &format!("{obj_type}.{obj_name}"),
            );
            if !removed {
                break;
            }
            stdout.push_str(&format!(
                "[OK]    Removed from subsystem '{subsystem_name}'\n"
            ));
            *subsystems_cleaned += 1;
            modified = true;
            text = next_text;
        }

        if modified && !dry_run {
            write_utf8_bom(&path, &ensure_trailing_newline(text))?;
            changes.push(format!("updated {}", path.display()));
        }

        let child_dir = path
            .parent()
            .unwrap_or(dir)
            .join(file_stem_string(&path))
            .join("Subsystems");
        if child_dir.is_dir() {
            remove_object_from_subsystems(
                &child_dir,
                obj_type,
                obj_name,
                dry_run,
                stdout,
                subsystems_cleaned,
                changes,
            )?;
        }
    }

    Ok(())
}

pub(crate) fn first_tag_text_in_xml(xml_text: &str, local_name: &str) -> Option<String> {
    for tag in [local_name.to_string(), format!("md:{local_name}")] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let Some(start) = xml_text.find(&open) else {
            continue;
        };
        let content_start = start + open.len();
        let Some(close_rel) = xml_text[content_start..].find(&close) else {
            continue;
        };
        let text = xml_text[content_start..content_start + close_rel].trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

pub(crate) fn file_stem_string(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn clear_main_data_composition_schema_text(
    xml_text: &str,
    template_name: &str,
) -> (String, bool) {
    clear_metadata_reference_text(
        xml_text,
        "MainDataCompositionSchema",
        &format!("Template.{template_name}"),
    )
}

pub(crate) fn clear_metadata_reference_text(
    xml_text: &str,
    local_name: &str,
    suffix: &str,
) -> (String, bool) {
    for tag in [local_name.to_string(), format!("md:{local_name}")] {
        let Some(open_start) = xml_text.find(&format!("<{tag}")) else {
            continue;
        };
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            continue;
        };
        let content_start = open_start + open_end_rel + 1;
        let close = format!("</{tag}>");
        let Some(close_start_rel) = xml_text[content_start..].find(&close) else {
            continue;
        };
        let close_start = content_start + close_start_rel;
        let content = &xml_text[content_start..close_start];
        if !content.trim().ends_with(suffix) {
            continue;
        }
        let mut result = String::with_capacity(xml_text.len() - content.len());
        result.push_str(&xml_text[..content_start]);
        result.push_str(&xml_text[close_start..]);
        return (result, true);
    }
    (xml_text.to_string(), false)
}

pub(crate) fn resolve_subsystem_edit_xml(mut path: PathBuf) -> Result<PathBuf, String> {
    if path.is_dir() {
        let dir_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let candidate = path.join(format!("{dir_name}.xml"));
        let sibling = path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")))
            .unwrap_or_else(|| PathBuf::from(format!("{dir_name}.xml")));
        if candidate.is_file() {
            path = candidate;
        } else if sibling.is_file() {
            path = sibling;
        } else {
            return Err(format!(
                "No {dir_name}.xml found in directory or as sibling"
            ));
        }
    }

    if !path.is_file() {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if stem
            == parent
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
        {
            if let Some(grand) = parent.parent() {
                let candidate = grand.join(format!("{stem}.xml"));
                if candidate.is_file() {
                    path = candidate;
                }
            }
        }
    }

    if !path.is_file() {
        return Err(format!("File not found: {}", path.display()));
    }
    Ok(path.canonicalize().unwrap_or(path))
}

pub(crate) fn load_subsystem_edit_model(path: &Path) -> Result<SubsystemEditModel, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", path.display()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "Expected <MetaDataObject> root element, got <{}>",
            root.tag_name().name()
        ));
    }
    let Some(sub) = root
        .children()
        .find(|node| role_info_element(*node, "Subsystem", Some("http://v8.1c.ru/8.3/MDClasses")))
    else {
        return Err("No <Subsystem> element found".to_string());
    };
    let Some(props) = meta_info_child(sub, "Properties") else {
        return Err("No <Properties> element found".to_string());
    };
    let Some(child_objects) = meta_info_child(sub, "ChildObjects") else {
        return Err("No <ChildObjects> element found".to_string());
    };

    let content = meta_info_child(props, "Content")
        .map(|content| {
            content
                .children()
                .filter(|node| role_info_element(*node, "Item", None))
                .filter_map(|node| node.text())
                .map(str::trim)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let children = child_objects
        .children()
        .filter(|node| role_info_element(*node, "Subsystem", None))
        .filter_map(|node| node.text())
        .map(str::trim)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    Ok(SubsystemEditModel {
        version: root
            .attribute("version")
            .unwrap_or(ACTIVE_FORMAT_PROFILE.export_format)
            .to_string(),
        uuid: sub
            .attribute("uuid")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| stable_uuid(70)),
        name: meta_info_child_text(props, "Name").unwrap_or_default(),
        synonym: subsystem_edit_ml_text(props, "Synonym"),
        comment: meta_info_child_text(props, "Comment").unwrap_or_default(),
        include_help: meta_info_child_text(props, "IncludeHelpInContents")
            .unwrap_or_else(|| "true".to_string()),
        include_ci: meta_info_child_text(props, "IncludeInCommandInterface")
            .unwrap_or_else(|| "true".to_string()),
        use_one_command: meta_info_child_text(props, "UseOneCommand")
            .unwrap_or_else(|| "false".to_string()),
        explanation: subsystem_edit_ml_text(props, "Explanation"),
        picture: subsystem_edit_picture_text(props),
        content,
        children,
    })
}

pub(crate) fn emit_subsystem_edit_model(model: &SubsystemEditModel) -> String {
    let mut lines = Vec::new();
    lines.push("<?xml version=\"1.0\" encoding=\"utf-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        full_md_namespace_declarations(),
        escape_xml(&model.version)
    ));
    lines.push(format!(
        "\t<Subsystem uuid=\"{}\">",
        escape_xml(&model.uuid)
    ));
    lines.push("\t\t<Properties>".to_string());
    lines.push(format!("\t\t\t<Name>{}</Name>", escape_xml(&model.name)));
    emit_subsystem_edit_ml(&mut lines, "\t\t\t", "Synonym", &model.synonym);
    if model.comment.is_empty() {
        lines.push("\t\t\t<Comment/>".to_string());
    } else {
        lines.push(format!(
            "\t\t\t<Comment>{}</Comment>",
            escape_xml(&model.comment)
        ));
    }
    lines.push(format!(
        "\t\t\t<IncludeHelpInContents>{}</IncludeHelpInContents>",
        escape_xml(&model.include_help)
    ));
    lines.push(format!(
        "\t\t\t<IncludeInCommandInterface>{}</IncludeInCommandInterface>",
        escape_xml(&model.include_ci)
    ));
    lines.push(format!(
        "\t\t\t<UseOneCommand>{}</UseOneCommand>",
        escape_xml(&model.use_one_command)
    ));
    emit_subsystem_edit_ml(&mut lines, "\t\t\t", "Explanation", &model.explanation);
    if model.picture.is_empty() {
        lines.push("\t\t\t<Picture/>".to_string());
    } else {
        lines.push("\t\t\t<Picture>&#13;".to_string());
        lines.push(format!(
            "\t\t\t\t<xr:Ref>{}</xr:Ref>&#13;",
            escape_xml(&model.picture)
        ));
        lines.push("\t\t\t\t<xr:LoadTransparent>false</xr:LoadTransparent>&#13;".to_string());
        lines.push("\t\t\t</Picture>".to_string());
    }
    if model.content.is_empty() {
        lines.push("\t\t\t<Content/>".to_string());
    } else {
        lines.push("\t\t\t<Content>&#13;".to_string());
        for item in &model.content {
            lines.push(format!(
                "\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                escape_xml(item)
            ));
        }
        lines.push("\t\t\t</Content>".to_string());
    }
    lines.push("\t\t</Properties>".to_string());
    if model.children.is_empty() {
        lines.push("\t\t<ChildObjects/>".to_string());
    } else {
        lines.push("\t\t<ChildObjects>&#13;".to_string());
        for child in &model.children {
            lines.push(format!(
                "\t\t\t<Subsystem>{}</Subsystem>",
                escape_xml(child)
            ));
        }
        lines.push("\t\t</ChildObjects>".to_string());
    }
    lines.push("\t</Subsystem>".to_string());
    lines.push("</MetaDataObject>".to_string());
    format!("{}\n", lines.join("\n"))
}

pub(crate) fn emit_subsystem_edit_ml(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>&#13;"));
    lines.push(format!("{indent}\t<v8:item>&#13;"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>&#13;"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>&#13;",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>&#13;"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn resolve_subsystem_info_xml(
    mut path: PathBuf,
    directory_hint: bool,
) -> Result<PathBuf, String> {
    if path.is_dir() {
        let dir_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let candidate = path.join(format!("{dir_name}.xml"));
        let sibling = path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")))
            .unwrap_or_else(|| PathBuf::from(format!("{dir_name}.xml")));
        if candidate.is_file() {
            path = candidate;
        } else if sibling.is_file() {
            path = sibling;
        } else if directory_hint {
            return Err(format!(
                "[ERROR] No {dir_name}.xml found in directory. Use -Mode tree for directory listing."
            ));
        } else {
            return Err(format!("[ERROR] File not found: {}", path.display()));
        }
    }

    if !path.is_file() {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if stem
            == parent
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
        {
            if let Some(grand) = parent.parent() {
                let candidate = grand.join(format!("{stem}.xml"));
                if candidate.is_file() {
                    path = candidate;
                }
            }
        }
    }

    if !path.is_file() {
        return Err(format!("[ERROR] File not found: {}", path.display()));
    }
    Ok(path)
}

pub(crate) fn resolve_subsystem_validate_xml(mut path: PathBuf) -> Result<PathBuf, String> {
    if path.is_dir() {
        let dir_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let candidate = path.join(format!("{dir_name}.xml"));
        let sibling = path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")))
            .unwrap_or_else(|| PathBuf::from(format!("{dir_name}.xml")));
        if candidate.exists() {
            path = candidate;
        } else if sibling.exists() {
            path = sibling;
        } else {
            return Err(format!(
                "[ERROR] No {dir_name}.xml found in directory: {}",
                path.display()
            ));
        }
    }

    if !path.exists() {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if stem
            == parent
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
        {
            if let Some(grand) = parent.parent() {
                let candidate = grand.join(format!("{stem}.xml"));
                if candidate.exists() {
                    path = candidate;
                }
            }
        }
    }

    if !path.exists() {
        return Err(format!("[ERROR] File not found: {}", path.display()));
    }
    Ok(path)
}

pub(crate) fn load_subsystem_info_data(
    path: &Path,
) -> Result<(SubsystemInfoData, Vec<String>), String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in {}: {err}", path.display()))?;
    let root = doc.root_element();
    let Some(sub) = root
        .children()
        .find(|node| role_info_element(*node, "Subsystem", Some("http://v8.1c.ru/8.3/MDClasses")))
    else {
        return Err(format!(
            "[ERROR] Not a valid subsystem XML: {}",
            path.display()
        ));
    };
    let Some(props) = sub
        .children()
        .find(|node| role_info_element(*node, "Properties", Some("http://v8.1c.ru/8.3/MDClasses")))
    else {
        return Err(format!(
            "[ERROR] Not a valid subsystem XML: {}",
            path.display()
        ));
    };

    let name = child_text(props, "Name", Some("http://v8.1c.ru/8.3/MDClasses"));
    let synonym = props
        .children()
        .find(|node| role_info_element(*node, "Synonym", Some("http://v8.1c.ru/8.3/MDClasses")))
        .map(multilang_text)
        .unwrap_or_default();
    let comment = child_text(props, "Comment", Some("http://v8.1c.ru/8.3/MDClasses"));
    let include_ci = child_text(
        props,
        "IncludeInCommandInterface",
        Some("http://v8.1c.ru/8.3/MDClasses"),
    );
    let use_one_command = child_text(
        props,
        "UseOneCommand",
        Some("http://v8.1c.ru/8.3/MDClasses"),
    );
    let explanation = props
        .children()
        .find(|node| role_info_element(*node, "Explanation", Some("http://v8.1c.ru/8.3/MDClasses")))
        .map(multilang_text)
        .unwrap_or_default();
    let picture = props
        .children()
        .find(|node| role_info_element(*node, "Picture", Some("http://v8.1c.ru/8.3/MDClasses")))
        .and_then(|node| {
            node.children()
                .find(|child| role_info_element(*child, "Ref", None))
                .and_then(|child| child.text())
        })
        .unwrap_or("")
        .to_string();
    let content_items = subsystem_content_items(props);
    let groups = subsystem_group_content(&content_items);
    let child_names = subsystem_child_names(sub);
    let sub_dir = subsystem_dir_for_xml(path);
    let has_ci = sub_dir.join("Ext").join("CommandInterface.xml").is_file();

    Ok((
        SubsystemInfoData {
            name,
            synonym,
            comment,
            include_ci,
            use_one_command,
            explanation,
            picture,
            content_items,
            groups,
            child_names,
            has_ci,
        },
        Vec::new(),
    ))
}

pub(crate) fn append_subsystem_overview(lines: &mut Vec<String>, data: &SubsystemInfoData) {
    lines.push(format!("Подсистема: {}", data.name));
    if !data.synonym.is_empty() && data.synonym != data.name {
        lines.push(format!("Синоним: {}", data.synonym));
    }
    if !data.comment.is_empty() {
        lines.push(format!("Комментарий: {}", data.comment));
    }
    lines.push(format!("ВключатьВКомандныйИнтерфейс: {}", data.include_ci));
    lines.push(format!("ИспользоватьОднуКоманду: {}", data.use_one_command));
    if !data.explanation.is_empty() {
        lines.push(format!("Пояснение: {}", data.explanation));
    }
    if !data.picture.is_empty() {
        lines.push(format!("Картинка: {}", data.picture));
    }
    if data.content_items.is_empty() {
        lines.push("Состав: пусто".to_string());
    } else {
        let parts = data
            .groups
            .iter()
            .map(|(type_name, items)| format!("{type_name}: {}", items.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "Состав: {} объектов ({})",
            data.content_items.len(),
            parts.join(", ")
        ));
    }
    if !data.child_names.is_empty() {
        lines.push(format!(
            "Дочерние подсистемы ({}): {}",
            data.child_names.len(),
            data.child_names.join(", ")
        ));
    }
    if data.has_ci {
        lines.push("Командный интерфейс: есть".to_string());
    }
}

pub(crate) fn append_subsystem_content(
    lines: &mut Vec<String>,
    data: &SubsystemInfoData,
    name_filter: &str,
) {
    lines.push(format!(
        "Состав подсистемы {} ({} объектов):",
        data.name,
        data.content_items.len()
    ));
    lines.push(String::new());
    if !name_filter.is_empty() {
        if let Some((_, items)) = data
            .groups
            .iter()
            .find(|(type_name, _)| type_name == name_filter)
        {
            lines.push(format!("{name_filter} ({}):", items.len()));
            for item in items {
                lines.push(format!("  {item}"));
            }
        } else {
            lines.push(format!("[INFO] Тип '{name_filter}' не найден в составе."));
            lines.push(format!(
                "Доступные типы: {}",
                data.groups
                    .iter()
                    .map(|(type_name, _)| type_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    } else {
        for (type_name, items) in &data.groups {
            lines.push(format!("{type_name} ({}):", items.len()));
            for item in items {
                lines.push(format!("  {item}"));
            }
            lines.push(String::new());
        }
    }
}

pub(crate) fn build_subsystem_tree_entry(
    xml_path: &Path,
    prefix: &str,
    is_last: bool,
    is_root: bool,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let (data, _) = load_subsystem_info_data(xml_path)?;
    let mut markers = Vec::new();
    if data.has_ci {
        markers.push("CI");
    }
    if data.use_one_command == "true" {
        markers.push("OneCmd");
    }
    if data.include_ci == "false" {
        markers.push("Скрыт");
    }
    let marker = if markers.is_empty() {
        String::new()
    } else {
        format!(" [{}]", markers.join(", "))
    };
    let child_str = if data.child_names.is_empty() {
        String::new()
    } else {
        format!(", {} дочерних", data.child_names.len())
    };
    let connector = if is_root {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };
    lines.push(format!(
        "{prefix}{connector}{}{} ({} объектов{child_str})",
        data.name,
        marker,
        data.content_items.len()
    ));

    if !data.child_names.is_empty() {
        let child_prefix = if is_root {
            String::new()
        } else if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };
        let subs_dir = subsystem_dir_for_xml(xml_path).join("Subsystems");
        for (idx, child_name) in data.child_names.iter().enumerate() {
            let child_xml = subs_dir.join(format!("{child_name}.xml"));
            let child_is_last = idx == data.child_names.len() - 1;
            if child_xml.is_file() {
                build_subsystem_tree_entry(&child_xml, &child_prefix, child_is_last, false, lines)?;
            } else {
                let conn = if child_is_last {
                    "└── "
                } else {
                    "├── "
                };
                lines.push(format!("{child_prefix}{conn}{child_name} [NOT FOUND]"));
            }
        }
    }
    Ok(())
}

pub(crate) fn paginate_subsystem_info(
    lines: &mut Vec<String>,
    args: &Map<String, Value>,
) -> Option<String> {
    let total_lines = lines.len();
    let offset = int_arg(args, &["offset", "Offset"]).unwrap_or(0);
    let limit = int_arg(args, &["limit", "Limit"]).unwrap_or(150);
    if offset > 0 {
        if offset as usize >= total_lines {
            return Some(format!(
                "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show.\n"
            ));
        }
        *lines = lines[offset as usize..].to_vec();
    }
    if limit > 0 && lines.len() > limit as usize {
        let mut shown = lines[..limit as usize].to_vec();
        shown.push(String::new());
        shown.push(format!(
            "[ОБРЕЗАНО] Показано {limit} из {total_lines} строк. Используйте -Offset {} для продолжения.",
            offset + limit
        ));
        *lines = shown;
    }
    None
}

pub(crate) fn push_group_item(groups: &mut Vec<(String, Vec<String>)>, group: &str, item: String) {
    if let Some((_, items)) = groups.iter_mut().find(|(name, _)| name == group) {
        items.push(item);
    } else {
        groups.push((group.to_string(), vec![item]));
    }
}

pub(crate) fn looks_like_uuid_prefix(value: &str) -> bool {
    value.len() >= 9
        && value.chars().take(8).all(|ch| ch.is_ascii_hexdigit())
        && value.as_bytes().get(8) == Some(&b'-')
}

pub(crate) fn is_subsystem_content_ref(value: &str) -> bool {
    let Some((prefix, rest)) = value.split_once('.') else {
        return false;
    };
    !prefix.is_empty() && !rest.is_empty() && prefix.chars().all(|ch| ch.is_ascii_alphabetic())
}

pub(crate) fn attribute_by_local_name<'a>(
    node: roxmltree::Node<'a, '_>,
    local_name: &str,
) -> Option<&'a str> {
    node.attributes()
        .find(|attr| attr.name() == local_name)
        .map(|attr| attr.value())
}

pub(crate) fn duplicates_preserve_order(items: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for item in items {
        let count = items.iter().filter(|candidate| *candidate == item).count();
        if count > 1 && !result.iter().any(|existing| existing == item) {
            result.push(item.clone());
        }
    }
    result
}

pub(crate) fn multilang_text(node: roxmltree::Node<'_, '_>) -> String {
    for item in node.children().filter(|child| child.is_element()) {
        let mut lang = "";
        let mut content = "";
        for child in item.children().filter(|child| child.is_element()) {
            match child.tag_name().name() {
                "lang" => lang = child.text().unwrap_or(""),
                "content" => content = child.text().unwrap_or(""),
                _ => {}
            }
        }
        if lang == "ru" && !content.is_empty() {
            return content.to_string();
        }
    }
    for item in node.children().filter(|child| child.is_element()) {
        for child in item.children().filter(|child| child.is_element()) {
            if child.tag_name().name() == "content" {
                if let Some(text) = child.text() {
                    if !text.is_empty() {
                        return text.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

pub(crate) fn child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> String {
    node.children()
        .find(|child| role_info_element(*child, local_name, namespace))
        .and_then(|child| child.text())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn add_role_info_right(
    groups: &mut Vec<RoleInfoGroup>,
    type_prefix: &str,
    short_name: &str,
    right: RoleInfoRightSummary,
) {
    let group_index = groups
        .iter()
        .position(|group| group.type_prefix == type_prefix)
        .unwrap_or_else(|| {
            groups.push(RoleInfoGroup {
                type_prefix: type_prefix.to_string(),
                objects: Vec::new(),
            });
            groups.len() - 1
        });

    let group = &mut groups[group_index];
    let object_index = group
        .objects
        .iter()
        .position(|object| object.short_name == short_name)
        .unwrap_or_else(|| {
            group.objects.push(RoleInfoObjectSummary {
                short_name: short_name.to_string(),
                rights: Vec::new(),
            });
            group.objects.len() - 1
        });
    group.objects[object_index].rights.push(right);
}

pub(crate) fn append_role_info_group(
    lines: &mut Vec<String>,
    objects: &[RoleInfoObjectSummary],
    is_denied: bool,
) {
    for object in objects {
        let rights = object
            .rights
            .iter()
            .map(|right| {
                if is_denied {
                    format!("-{}", right.name)
                } else if right.rls {
                    format!("{} [RLS]", right.name)
                } else {
                    right.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("    {}: {rights}", object.short_name));
    }
}

pub(crate) fn resolve_role_validate_rights_path(path: PathBuf) -> PathBuf {
    let mut rights_path = path;
    if rights_path.is_dir() {
        rights_path = rights_path.join("Ext").join("Rights.xml");
    }
    if !rights_path.exists()
        && rights_path.file_name().and_then(|value| value.to_str()) == Some("Rights.xml")
    {
        if let Some(parent) = rights_path.parent() {
            let candidate = parent.join("Ext").join("Rights.xml");
            if candidate.exists() {
                rights_path = candidate;
            }
        }
    }
    rights_path
}

pub(crate) fn resolve_role_read_rights_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let raw = required_path(args, RIGHTS_PATH, "RightsPath")?;
    Ok(resolve_role_validate_rights_path(absolutize(
        raw,
        &context.cwd,
    )))
}

pub(crate) fn is_valid_uuid(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    let expected = [8usize, 4, 4, 4, 12];
    parts.len() == expected.len()
        && parts
            .iter()
            .zip(expected)
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

pub(crate) fn replace_first_xml_element_text(
    xml_text: &mut String,
    tag: &str,
    value: &str,
) -> bool {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let Some(start) = xml_text.find(&open) else {
        return false;
    };
    let content_start = start + open.len();
    let Some(relative_end) = xml_text[content_start..].find(&close) else {
        return false;
    };
    let content_end = content_start + relative_end;
    xml_text.replace_range(content_start..content_end, &escape_xml(value));
    true
}

pub(crate) fn insert_meta_property_before_child_objects(
    xml_text: &mut String,
    tag: &str,
    value: &str,
) -> Result<(), String> {
    let Some(properties_end) = xml_text.find("\n\t\t</Properties>") else {
        return Err("No <Properties> element found".to_string());
    };
    let insertion = format!("\n\t\t\t<{tag}>{}</{tag}>", escape_xml(value));
    xml_text.insert_str(properties_end, &insertion);
    Ok(())
}

pub(crate) fn resolve_cf_edit_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let mut config_path =
        required_path(args, CF_PATH, "ConfigPath").map(|path| absolutize(path, &context.cwd))?;
    if config_path.is_dir() {
        let candidate = config_path.join("Configuration.xml");
        if candidate.is_file() {
            config_path = candidate;
        } else {
            return Err("No Configuration.xml in directory".to_string());
        }
    }
    if !config_path.is_file() {
        return Err(format!("File not found: {}", config_path.display()));
    }
    Ok(config_path)
}

pub(crate) fn resolve_cf_read_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    resolve_configuration_read_path(args, CF_PATH, "ConfigPath", context)
}

pub(crate) fn resolve_cfe_validate_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    resolve_configuration_read_path(args, CFE_VALIDATE_PATH, "ExtensionPath", context)
}

fn resolve_configuration_read_path(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let raw = required_path(args, names, label)?;
    let mut path = absolutize(raw, &context.cwd);
    if path.is_dir() {
        let candidate = path.join("Configuration.xml");
        if candidate.is_file() {
            path = candidate;
        } else {
            return Err(format!(
                "[ERROR] No Configuration.xml found in directory: {}",
                path.display()
            ));
        }
    }
    if !path.is_file() {
        return Err(format!("[ERROR] File not found: {}", path.display()));
    }
    Ok(path.canonicalize().unwrap_or(path))
}

pub(crate) fn ensure_trailing_lf(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}

pub(crate) fn lxml_tree_serialized_text(text: &str) -> String {
    let mut output = text.to_string();
    output = output.replace(" />", "/>");
    output = output.replace("\r\n", "\n");
    output = output.replace('\r', "&#13;");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

pub(crate) fn lxml_tree_serialized_text_like_source(text: &str, source_text: &str) -> String {
    let output = lxml_tree_serialized_text(text);
    if source_text.contains("\r\n") {
        output.replace('\n', "\r\n")
    } else {
        output
    }
}

pub(crate) fn lxml_tree_serialized_text_like_source_preserving_final_newline(
    text: &str,
    source_text: &str,
) -> String {
    preserve_source_final_newline(
        lxml_tree_serialized_text_like_source(text, source_text),
        source_text,
    )
}

pub(crate) fn preserve_source_final_newline(mut output: String, source_text: &str) -> String {
    let source_final_newline = if source_text.ends_with("\r\n") {
        Some("\r\n")
    } else if source_text.ends_with('\n') {
        Some("\n")
    } else if source_text.ends_with('\r') {
        Some("\r")
    } else {
        None
    };

    match source_final_newline {
        Some(line_ending) if !output.ends_with('\n') && !output.ends_with('\r') => {
            output.push_str(line_ending);
        }
        None if output.ends_with("\r\n") => {
            output.truncate(output.len() - 2);
        }
        None if output.ends_with('\n') || output.ends_with('\r') => {
            output.pop();
        }
        _ => {}
    }
    output
}

pub(crate) fn lxml_parser_normalized_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn unescape_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

pub(crate) fn output_dir_arg(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
    default: &str,
) -> PathBuf {
    let path = path_arg(args, names).unwrap_or_else(|| PathBuf::from(default));
    absolutize(path, &context.cwd)
}

pub(crate) fn write_utf8_bom(path: &Path, content: &str) -> Result<(), String> {
    let bytes = utf8_bom_bytes(content);
    let expected_preimage = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "failed to inspect UTF-8 BOM publication target {}: {error}",
                path.display()
            ));
        }
    };
    let mode = match expected_preimage.as_deref() {
        Some(expected_preimage) => PublishMode::ReplaceExisting { expected_preimage },
        None => PublishMode::CreateOnly,
    };
    let report = publish(PublishRequest {
        target: path,
        replacement: &bytes,
        mode,
    })
    .map_err(|error| format!("failed to publish {}: {error}", path.display()))?;
    if report.cleanup_warnings.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "published {} but publication cleanup is incomplete: {}",
            path.display(),
            report
                .cleanup_warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        ))
    }
}

pub(crate) fn utf8_bom_bytes(content: &str) -> Vec<u8> {
    let content = content.trim_start_matches('\u{feff}');
    let mut bytes = Vec::with_capacity(content.len() + 3);
    bytes.extend_from_slice(b"\xef\xbb\xbf");
    bytes.extend_from_slice(content.as_bytes());
    bytes
}

pub(crate) fn stable_uuid(index: usize) -> String {
    format!("00000000-0000-0000-0000-{index:012x}")
}

#[cfg(test)]
mod mutation_tests {
    use super::{
        format_compatibility_warning, guard_active_format_owner, read_utf8_sig_snapshot,
        utf8_bom_bytes, write_utf8_bom,
    };
    use crate::domain::format_profile::FormatCompatibility;
    use crate::domain::source_adapters::FormatVersion;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
    use crate::infrastructure::native_operations::single_file_publisher::{
        with_before_commit_hook, with_publish_failpoints, PublishCheckpoint,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn format_owner_context(name: &str, version: &str) -> (WorkspaceContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-common-format-owner-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("src");
        fs::create_dir_all(source.join("Templates/Guarded/Ext")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"{version}\"><Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"><Properties><Name>Demo</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
        let target = source.join("Templates/Guarded/Ext/Template.xml");
        (
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            target,
        )
    }

    #[test]
    fn utf8_bom_bytes_emits_exactly_one_bom() {
        assert_eq!(utf8_bom_bytes("<xml/>"), b"\xef\xbb\xbf<xml/>");
        assert_eq!(
            utf8_bom_bytes("\u{feff}\u{feff}<xml/>"),
            b"\xef\xbb\xbf<xml/>"
        );
    }

    #[test]
    fn utf8_snapshot_keeps_raw_preimage_and_decodes_text_without_bom() {
        let root = std::env::temp_dir().join(format!(
            "unica-common-snapshot-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Configuration.xml");
        let raw = b"\xef\xbb\xbf<xml/>\r\n";
        fs::write(&path, raw).unwrap();

        let snapshot = read_utf8_sig_snapshot(&path).unwrap();

        assert_eq!(snapshot.raw, raw);
        assert_eq!(snapshot.text, "<xml/>\r\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn utf8_bom_writer_never_overwrites_a_concurrent_replacement() {
        let root = std::env::temp_dir().join(format!(
            "unica-common-publish-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Configuration.xml");
        fs::write(&path, b"\xef\xbb\xbforiginal").unwrap();
        let concurrent = b"\xef\xbb\xbfconcurrent";

        let result = with_before_commit_hook(
            move |target| fs::write(target, concurrent).unwrap(),
            || write_utf8_bom(&path, "replacement"),
        );

        let error = result.expect_err("stale preimage must abort publication");
        assert!(
            error.contains("differs from the expected preimage"),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), concurrent);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn utf8_bom_writer_surfaces_cleanup_warning_after_committed_create() {
        let root = std::env::temp_dir().join(format!(
            "unica-common-cleanup-warning-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Configuration.xml");

        let result = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
            write_utf8_bom(&path, "<MetaDataObject/>")
        });

        let error = result.expect_err("committed cleanup warning must never be hidden");
        assert!(error.contains("published"), "{error}");
        assert!(error.contains("cleanup is incomplete"), "{error}");
        assert!(
            error.contains("injected publication cleanup failure"),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbf<MetaDataObject/>");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn older_format_warning_offers_explicit_platform_reexport_without_auto_migration() {
        let warning = format_compatibility_warning(&FormatCompatibility::Older {
            actual: FormatVersion::parse("2.19").unwrap(),
            target: FormatVersion::parse("2.20").unwrap(),
        });

        assert!(
            warning.contains("will not migrate it automatically"),
            "{warning}"
        );
        assert!(warning.contains("re-export"), "{warning}");
        assert!(warning.contains("1C:Enterprise 8.3.27"), "{warning}");
        assert!(!warning.contains("migrate the configuration before writing"));
    }

    #[test]
    fn newer_format_warning_keeps_8_5_roadmap_copy_without_downgrade_offer() {
        let warning = format_compatibility_warning(&FormatCompatibility::Newer {
            actual: FormatVersion::parse("2.21").unwrap(),
            target: FormatVersion::parse("2.20").unwrap(),
        });

        assert!(warning.contains("1C 8.5 support is planned"), "{warning}");
        assert!(!warning.to_ascii_lowercase().contains("downgrade"));
    }

    #[test]
    fn active_format_owner_guard_reauthorizes_and_rejects_newer_snapshot() {
        let (context, target) = format_owner_context("newer", "2.21");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(&target, "<Template/>")
            .unwrap();

        let error = guard_active_format_owner(&mut transaction, &target, &context)
            .expect_err("handler-side guard must reject a newer owner snapshot");

        assert!(error.contains("2.21"), "{error}");
        assert!(error.contains("2.20"), "{error}");
        assert!(!target.exists());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn active_format_owner_guard_binds_supported_snapshot_to_commit() {
        let (context, target) = format_owner_context("concurrent", "2.20");
        let owner = context.workspace_root.join("src/Configuration.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(&target, "<Template/>")
            .unwrap();
        guard_active_format_owner(&mut transaction, &target, &context).unwrap();
        fs::write(
            &owner,
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration/></MetaDataObject>",
        )
        .unwrap();

        let error = transaction
            .commit()
            .expect_err("owner drift after handler reauthorization must abort publication");

        assert!(error.contains("read guard"), "{error}");
        assert!(!target.exists());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }
}

#[cfg(test)]
mod support_state_tests {
    use super::support_status_for_path;
    use crate::application::SupportGuardRequirement;
    use crate::infrastructure::support_guard::support_guard_violation;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn support_fixture(label: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-common-support-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        let target = root.join("Documents/Shipment.xml");
        fs::create_dir_all(target.parent().expect("target parent")).expect("create fixture");
        fs::write(
            root.join("Configuration.xml"),
            "<MetaDataObject uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"/>",
        )
        .expect("write configuration");
        fs::write(
            &target,
            "<MetaDataObject uuid=\"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\"/>",
        )
        .expect("write target");
        (root, target)
    }

    #[test]
    fn absent_parent_configurations_remains_not_supported_and_unblocked() {
        let (root, target) = support_fixture("absent");

        assert_eq!(support_status_for_path(&target), "не на поддержке");
        assert_eq!(
            support_guard_violation(&target, SupportGuardRequirement::Editable),
            None
        );

        fs::remove_dir_all(root).expect("clean fixture");
    }

    #[test]
    fn malformed_parent_configurations_blocks_edits_without_claiming_no_support() {
        let (root, target) = support_fixture("malformed");
        let ext = root.join("Ext");
        fs::create_dir_all(&ext).expect("create Ext");
        fs::write(
            ext.join("ParentConfigurations.bin"),
            "malformed ParentConfigurations.bin content longer than 32 bytes",
        )
        .expect("write malformed support state");

        assert_eq!(
            support_status_for_path(&target),
            "состояние поддержки не удалось прочитать — правки не подтверждены"
        );
        let violation = support_guard_violation(&target, SupportGuardRequirement::Editable)
            .expect("malformed support state must block edits");
        assert_eq!(violation.code, "support-state-unreadable");
        assert!(violation.reason.contains("не удалось прочитать"));

        fs::remove_dir_all(root).expect("clean fixture");
    }

    #[test]
    fn short_malformed_parent_configurations_is_not_mistaken_for_removed_support() {
        let (root, target) = support_fixture("short-malformed");
        let ext = root.join("Ext");
        fs::create_dir_all(&ext).expect("create Ext");
        fs::write(ext.join("ParentConfigurations.bin"), "not a state")
            .expect("write malformed short support state");

        assert_eq!(
            support_status_for_path(&target),
            "состояние поддержки не удалось прочитать — правки не подтверждены"
        );
        assert_eq!(
            support_guard_violation(&target, SupportGuardRequirement::Editable)
                .expect("short malformed support state must block edits")
                .code,
            "support-state-unreadable"
        );

        fs::remove_dir_all(root).expect("clean fixture");
    }

    #[test]
    fn empty_parent_configurations_remains_removed_support() {
        let (root, target) = support_fixture("empty");
        let ext = root.join("Ext");
        fs::create_dir_all(&ext).expect("create Ext");
        fs::write(ext.join("ParentConfigurations.bin"), []).expect("write empty support state");

        assert_eq!(
            support_status_for_path(&target),
            "снято с поддержки (правки свободны)"
        );
        assert_eq!(
            support_guard_violation(&target, SupportGuardRequirement::Editable),
            None
        );

        fs::remove_dir_all(root).expect("clean fixture");
    }

    #[test]
    fn zero_vendor_payload_is_not_mistaken_for_removed_support() {
        let (root, target) = support_fixture("zero-vendor");
        let ext = root.join("Ext");
        fs::create_dir_all(&ext).expect("create Ext");
        fs::write(ext.join("ParentConfigurations.bin"), "{6,1,0}")
            .expect("write zero-vendor read-only state");

        assert_eq!(
            support_status_for_path(&target),
            "конфигурация read-only (возможность изменения выключена) — правки невозможны без включения"
        );
        assert_eq!(
            support_guard_violation(&target, SupportGuardRequirement::Editable)
                .expect("global flag must block edits")
                .code,
            "capability-off"
        );

        fs::remove_dir_all(root).expect("clean fixture");
    }

    #[test]
    fn non_regular_parent_configurations_blocks_edits_without_claiming_no_support() {
        let (root, target) = support_fixture("non-regular");
        fs::create_dir_all(root.join("Ext/ParentConfigurations.bin"))
            .expect("create non-regular support state");

        assert_eq!(
            support_status_for_path(&target),
            "состояние поддержки не удалось прочитать — правки не подтверждены"
        );
        let violation = support_guard_violation(&target, SupportGuardRequirement::Editable)
            .expect("non-regular support state must block edits");
        assert_eq!(violation.code, "support-state-unreadable");
        assert!(violation.reason.contains("не удалось прочитать"));

        fs::remove_dir_all(root).expect("clean fixture");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_target_uses_the_same_locked_support_state_as_the_guard() {
        use std::os::unix::fs::symlink;

        let (root, target) = support_fixture("symlink");
        let ext = root.join("Ext");
        fs::create_dir_all(&ext).expect("create Ext");
        fs::write(
            ext.join("ParentConfigurations.bin"),
            "{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,cccccccc-cccc-cccc-cccc-cccccccccccc,cccccccc-cccc-cccc-cccc-cccccccccccc}",
        )
        .expect("write locked support state");
        let alias = root.with_extension("alias.xml");
        symlink(&target, &alias).expect("create fixture symlink");
        let linked_target = alias;

        assert!(support_status_for_path(&linked_target).starts_with("на замке"));
        let violation = support_guard_violation(&linked_target, SupportGuardRequirement::Editable)
            .expect("locked symlink target must be blocked");
        assert_eq!(violation.code, "locked");

        fs::remove_file(&linked_target).expect("remove fixture symlink");
        fs::remove_dir_all(root).expect("clean fixture");
    }
}

pub(crate) fn analyze_xml(
    operation: &str,
    tool_name: &str,
    target: &Path,
    text: &str,
) -> AdapterOutcome {
    match Document::parse(text) {
        Ok(doc) => {
            let root = doc.root_element();
            let element_count = doc.descendants().filter(|node| node.is_element()).count();
            let summary = json!({
                "operation": operation,
                "file": target.display().to_string(),
                "root": root.tag_name().name(),
                "name": first_text(&doc, "Name"),
                "synonym": first_text(&doc, "Synonym"),
                "elementCount": element_count,
                "topLevel": root
                    .children()
                    .filter(|node| node.is_element())
                    .map(|node| node.tag_name().name().to_string())
                    .collect::<Vec<_>>(),
            });
            AdapterOutcome {
                ok: true,
                summary: format!("{tool_name} completed with native XML parser"),
                changes: Vec::new(),
                warnings: validation_warnings(operation, &doc),
                errors: Vec::new(),
                artifacts: vec![target.display().to_string()],
                stdout: Some(
                    serde_json::to_string_pretty(&summary).unwrap_or_else(|_| summary.to_string()),
                ),
                stderr: None,
                command: None,
            }
        }
        Err(err) => AdapterOutcome {
            ok: false,
            summary: format!("{tool_name} failed native XML validation"),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![format!("XML parse error in {}: {err}", target.display())],
            artifacts: vec![target.display().to_string()],
            stdout: None,
            stderr: None,
            command: None,
        },
    }
}

pub(crate) fn validation_warnings(operation: &str, doc: &Document<'_>) -> Vec<String> {
    let mut warnings = Vec::new();
    let root = doc.root_element().tag_name().name();
    if operation.starts_with("cf-") && root != "MetaDataObject" {
        warnings.push(format!("expected MetaDataObject root, got {root}"));
    }
    if operation.starts_with("role-") && !has_element(doc, "Rights") {
        warnings.push("expected role Rights content".to_string());
    }
    if operation.starts_with("form-") && !has_element(doc, "Form") && root != "Form" {
        warnings.push("expected managed form XML content".to_string());
    }
    warnings
}

pub(crate) fn resolve_target(
    operation: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let path = if operation.starts_with("cf-") {
        required_path(
            args,
            &["configPath", "ConfigPath", "path", "Path"],
            "ConfigPath",
        )?
    } else if operation.starts_with("cfe-") {
        required_path(
            args,
            &["extensionPath", "ExtensionPath", "path", "Path"],
            "ExtensionPath",
        )?
    } else if operation.starts_with("meta-") {
        required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?
    } else if operation.starts_with("form-") {
        required_path(args, &["formPath", "FormPath", "path", "Path"], "FormPath")?
    } else if operation.starts_with("interface-") {
        required_path(args, &["ciPath", "CIPath", "path", "Path"], "CIPath")?
    } else if operation.starts_with("subsystem-") {
        required_path(
            args,
            &["subsystemPath", "SubsystemPath", "path", "Path"],
            "SubsystemPath",
        )?
    } else if operation.starts_with("dcs-") || operation.starts_with("mxl-") {
        required_path(
            args,
            &["templatePath", "TemplatePath", "path", "Path"],
            "TemplatePath",
        )?
    } else if operation.starts_with("role-") {
        required_path(
            args,
            &["rightsPath", "RightsPath", "path", "Path"],
            "RightsPath",
        )?
    } else {
        return Err(format!(
            "native operation {operation} does not define a path argument"
        ));
    };

    Ok(resolve_existing_path(
        operation,
        absolutize(path, &context.cwd),
    ))
}

pub(crate) fn resolve_existing_path(operation: &str, path: PathBuf) -> PathBuf {
    if path.is_dir() {
        let leaf = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        for candidate in directory_candidates(operation, &path, leaf) {
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    if !path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("xml") {
        if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
            if let Some(parent) = path.parent() {
                let candidate = parent.join(stem).join("Ext").join(special_file(operation));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }

    path
}

pub(crate) fn directory_candidates(operation: &str, path: &Path, leaf: &str) -> Vec<PathBuf> {
    if operation.starts_with("cf-") || operation.starts_with("cfe-") {
        vec![path.join("Configuration.xml")]
    } else if operation.starts_with("form-") {
        vec![path.join("Ext").join("Form.xml")]
    } else if operation.starts_with("interface-") {
        vec![path.join("Ext").join("CommandInterface.xml")]
    } else if operation.starts_with("dcs-") || operation.starts_with("mxl-") {
        vec![path.join("Ext").join("Template.xml")]
    } else if operation.starts_with("role-") {
        vec![path.join("Ext").join("Rights.xml")]
    } else {
        vec![path.join(format!("{leaf}.xml"))]
    }
}

pub(crate) fn special_file(operation: &str) -> &'static str {
    if operation.starts_with("form-") {
        "Form.xml"
    } else if operation.starts_with("role-") {
        "Rights.xml"
    } else {
        "Template.xml"
    }
}

pub(crate) fn required_path(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
) -> Result<PathBuf, String> {
    path_arg(args, names).ok_or_else(|| format!("missing required {label} argument"))
}

pub(crate) fn required_string<'a>(
    args: &'a Map<String, Value>,
    names: &[&str],
    label: &str,
) -> Result<&'a str, String> {
    string_arg(args, names).ok_or_else(|| format!("missing required {label} argument"))
}

pub(crate) fn path_arg(args: &Map<String, Value>, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

pub(crate) fn string_arg<'a>(args: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn bool_arg(args: &Map<String, Value>, names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| args.get(*name).and_then(Value::as_bool).unwrap_or(false))
}

pub(crate) fn optional_bool_arg(args: &Map<String, Value>, names: &[&str]) -> Option<bool> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_bool))
}

pub(crate) fn int_arg(args: &Map<String, Value>, names: &[&str]) -> Option<i64> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(json_i64_value))
}

pub(crate) fn absolutize(path: PathBuf, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

pub(crate) fn extension_name_prefix(config: &Path) -> Option<String> {
    let text = fs::read_to_string(config).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    doc.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "NamePrefix")
        .and_then(|node| node.text())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn detect_format_version(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<FormatVersion, String> {
    let owners =
        crate::infrastructure::platform_xml_owner::resolve_platform_xml_owners(target, context)
            .map_err(|error| error.message)?;
    require_supported_platform_xml_owners(&owners)?;
    FormatVersion::parse(ACTIVE_FORMAT_PROFILE.export_format).map_err(|error| error.to_string())
}

/// Re-authorize the current version-owning XML bytes at handler planning time
/// and bind that exact snapshot to the transaction. The application guard is
/// user-facing early rejection; this guard closes the authorization/publication
/// window for cooperating Unica writers.
pub(crate) fn guard_active_format_owner(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    guard_active_format_dependencies(transaction, &[target], context)
}

pub(crate) fn guard_active_format_dependencies(
    transaction: &mut CompileTransaction,
    targets: &[&Path],
    context: &WorkspaceContext,
) -> Result<(), String> {
    let mut owners = BTreeMap::new();
    let mut provenances = Vec::new();
    for target in targets {
        let resolution =
            crate::infrastructure::platform_xml_owner::resolve_platform_xml_owners_with_provenance(
                target, context,
            )
            .map_err(|error| error.message)?;
        for owner in resolution.owners {
            owners.entry(owner.path.clone()).or_insert(owner);
        }
        provenances.push(resolution.provenance);
    }
    let owners = owners.into_values().collect::<Vec<_>>();
    require_supported_platform_xml_owners(&owners)?;
    for provenance in provenances {
        provenance.bind_to(transaction)?;
    }
    Ok(())
}

pub(crate) fn guard_active_format_containing_owner_for_new_output(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let resolution = crate::infrastructure::platform_xml_owner::
        resolve_existing_platform_xml_owners_for_new_output_with_provenance(target, context)
        .map_err(|error| error.message)?;
    let owners = resolution.owners;
    require_supported_platform_xml_owners(&owners)?;
    resolution.provenance.bind_to(transaction)?;
    Ok(())
}

pub(crate) fn guard_active_format_xml_tree(
    transaction: &mut CompileTransaction,
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    guard_active_format_dependencies_and_xml_trees(transaction, &[], &[target], context)
}

pub(crate) fn guard_active_format_dependencies_and_xml_trees(
    transaction: &mut CompileTransaction,
    dependencies: &[&Path],
    trees: &[&Path],
    context: &WorkspaceContext,
) -> Result<(), String> {
    let mut paths = Vec::new();
    paths.extend(dependencies.iter().map(|path| (*path).to_path_buf()));
    for tree in trees {
        collect_platform_xml_tree_paths(tree, &mut paths)?;
    }
    paths.sort();
    paths.dedup();
    let targets = paths.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    guard_active_format_dependencies(transaction, &targets, context)
}

fn collect_platform_xml_tree_paths(target: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!("failed to inspect {}: {error}", target.display()));
        }
    };
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "platform XML dependency must not be a symbolic link or reparse point: {}",
            target.display()
        ));
    }
    if metadata.is_file() {
        if target
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            paths.push(target.to_path_buf());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(format!(
            "platform XML dependency is neither a regular file nor a directory: {}",
            target.display()
        ));
    }
    let mut entries = fs::read_dir(target)
        .map_err(|error| format!("failed to inspect {}: {error}", target.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect {}: {error}", target.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        collect_platform_xml_tree_paths(&entry.path(), paths)?;
    }
    Ok(())
}

fn require_supported_platform_xml_owners(
    owners: &[crate::infrastructure::platform_xml_owner::PlatformXmlOwner],
) -> Result<(), String> {
    let mut older = None;
    let mut newer = None;
    for owner in owners {
        match owner.format.clone() {
            FormatCompatibility::Supported { .. } => {}
            compatibility @ FormatCompatibility::Older { .. } if older.is_none() => {
                older = Some(compatibility);
            }
            compatibility @ FormatCompatibility::Newer { .. } if newer.is_none() => {
                newer = Some(compatibility);
            }
            FormatCompatibility::Older { .. } | FormatCompatibility::Newer { .. } => {}
        }
    }
    if let Some(compatibility) = newer.or(older) {
        return Err(format_compatibility_warning(&compatibility));
    }
    Ok(())
}

/// Bind bytes that were already used to derive a write plan. This differs
/// from taking a fresh snapshot immediately before commit: a later, still
/// valid `2.20` document must not authorize output calculated from older
/// bytes.
pub(crate) fn guard_exact_preimage_if_unprotected(
    transaction: &mut CompileTransaction,
    path: &Path,
    expected_preimage: impl AsRef<[u8]>,
) -> Result<(), String> {
    if !transaction.protects_path(path)? {
        transaction.guard_exact_preimage(path, expected_preimage)?;
    }
    Ok(())
}

pub(crate) fn format_compatibility_warning(compatibility: &FormatCompatibility) -> String {
    match compatibility {
        FormatCompatibility::Older { actual, .. } => format!(
            "Export format {actual} is older than supported {}. Unica will not migrate it automatically; re-export the source explicitly with 1C:Enterprise {}, then retry.",
            ACTIVE_FORMAT_PROFILE.export_format,
            ACTIVE_FORMAT_PROFILE.platform_line
        ),
        FormatCompatibility::Newer { actual, .. } => format!(
            "Export format {actual} is newer than supported {}; platform 1C 8.5 support is planned in upcoming releases.",
            ACTIVE_FORMAT_PROFILE.export_format
        ),
        FormatCompatibility::Supported { .. } => format!(
            "Export format is supported ({})",
            ACTIVE_FORMAT_PROFILE.export_format
        ),
    }
}

pub(crate) fn support_state_lines_for_configuration(
    config_path: &Path,
    is_extension: bool,
) -> Vec<String> {
    let config_dir = if config_path.is_dir() {
        config_path
    } else {
        config_path.parent().unwrap_or_else(|| Path::new(""))
    };
    let Ok(evidence) = inspect_support_state(config_dir, "") else {
        return vec![
            "Поддержка:      состояние ParentConfigurations.bin не удалось прочитать — правки не подтверждены"
                .to_string(),
        ];
    };
    match evidence.source {
        SupportSourceState::Absent => vec![if is_extension {
            "Поддержка:      расширение (CFE), правки свободны".to_string()
        } else {
            "Поддержка:      не на поддержке (своя конфигурация)".to_string()
        }],
        SupportSourceState::Unreadable { .. } => vec![
            "Поддержка:      состояние ParentConfigurations.bin не удалось прочитать — правки не подтверждены"
                .to_string(),
        ],
        SupportSourceState::Removed => {
            vec!["Поддержка:      снята с поддержки полностью".to_string()]
        }
        SupportSourceState::Parsed if evidence.global_editing_enabled == Some(false) => vec![
            "Поддержка:      на поддержке".to_string(),
            "  Возможность изменения: выключена — вся конфигурация read-only (правки заблокированы)"
                .to_string(),
            format!("  Конфигураций поставщика: {}", evidence.vendors.len()),
        ],
        SupportSourceState::Parsed => {
            let mut lines = vec![
                "Поддержка:      на поддержке".to_string(),
                "  Возможность изменения: включена".to_string(),
                format!(
                    "  Объектов: на замке {} / редактируется {} / снято {}",
                    evidence.rule_counts[0], evidence.rule_counts[1], evidence.rule_counts[2]
                ),
                format!("  Конфигураций поставщика: {}", evidence.vendors.len()),
            ];
            if evidence.vendors.len() > 1 {
                for vendor in evidence.vendors {
                    lines.push(format!(
                        "  Поставщик: {} — {} {}",
                        vendor.vendor, vendor.name, vendor.version
                    ));
                }
            }
            lines
        }
    }
}

pub(crate) fn support_status_for_path(target_path: &Path) -> String {
    let Some(config_dir) = find_support_config_dir(target_path) else {
        return "не на поддержке".to_string();
    };
    let object_uuid = support_object_uuid_for_path(target_path)
        .or_else(|| support_root_uuid(&config_dir.join("Configuration.xml")));
    let Ok(evidence) = inspect_support_state(&config_dir, object_uuid.as_deref().unwrap_or(""))
    else {
        return "состояние поддержки не удалось прочитать — правки не подтверждены".to_string();
    };
    match evidence.effective_rule {
        EffectiveSupportRule::Absent => "не на поддержке".to_string(),
        EffectiveSupportRule::Removed => "снято с поддержки (правки свободны)".to_string(),
        EffectiveSupportRule::Editable => {
            "редактируется с сохранением поддержки".to_string()
        }
        EffectiveSupportRule::Locked => "на замке — прямая правка сломает обновления; дорабатывай через cfe-* либо включи редактирование объекта".to_string(),
        EffectiveSupportRule::ConfigurationReadOnly => "конфигурация read-only (возможность изменения выключена) — правки невозможны без включения".to_string(),
        EffectiveSupportRule::UnknownReadOnly => "состояние нескольких поставщиков нельзя однозначно применить — правки не подтверждены".to_string(),
        EffectiveSupportRule::Unreadable => "состояние поддержки не удалось прочитать — правки не подтверждены".to_string(),
    }
}

/*
 * The native support-edit operation still consumes this compatibility view, but
 * all parsing is performed by platform_xml::support above.
 */
pub(crate) fn is_uuid_text(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

pub(crate) fn find_support_config_dir(target_path: &Path) -> Option<PathBuf> {
    let target_path = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.to_path_buf());
    let mut current = if target_path.is_dir() {
        target_path
    } else {
        target_path.parent()?.to_path_buf()
    };
    for _ in 0..20 {
        if current
            .join("Ext")
            .join("ParentConfigurations.bin")
            .exists()
            || current.join("Configuration.xml").exists()
        {
            return Some(current);
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    None
}

pub(crate) fn support_uuid_dependency_paths(target_path: &Path) -> Vec<PathBuf> {
    let mut dependencies = Vec::new();
    if target_path.is_file()
        && target_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
    {
        dependencies.push(target_path.to_path_buf());
        if support_root_uuid(target_path).is_some() {
            return dependencies;
        }
    }
    let mut current = if target_path.is_dir() {
        target_path.to_path_buf()
    } else {
        let Some(parent) = target_path.parent() else {
            return dependencies;
        };
        parent.to_path_buf()
    };
    for _ in 0..20 {
        let candidate = current.with_extension("xml");
        if candidate.is_file() && !dependencies.contains(&candidate) {
            dependencies.push(candidate.clone());
            if support_root_uuid(&candidate).is_some() {
                return dependencies;
            }
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    dependencies
}

pub(crate) fn support_uuid_dependency_path(target_path: &Path) -> Option<PathBuf> {
    support_uuid_dependency_paths(target_path)
        .into_iter()
        .find(|path| support_root_uuid(path).is_some())
}

pub(crate) fn support_object_uuid_for_path(target_path: &Path) -> Option<String> {
    support_uuid_dependency_path(target_path)
        .as_deref()
        .and_then(support_root_uuid)
}

pub(crate) fn support_root_uuid(xml_path: &Path) -> Option<String> {
    let raw = fs::read(xml_path).ok()?;
    support_root_uuid_from_bytes(&raw)
}

pub(crate) fn support_root_uuid_from_bytes(raw: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(raw).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    if let Some(uuid) = root.attribute("uuid") {
        return Some(uuid.to_ascii_lowercase());
    }
    root.children()
        .find(|node| node.is_element() && node.attribute("uuid").is_some())
        .and_then(|node| node.attribute("uuid"))
        .map(str::to_ascii_lowercase)
}

pub(crate) fn parse_support_header(path: &Path) -> Option<(u8, usize)> {
    let config_dir = path.parent()?.parent()?;
    let evidence = inspect_support_state(config_dir, "").ok()?;
    if !matches!(evidence.source, SupportSourceState::Parsed) {
        return None;
    }
    Some((
        if evidence.global_editing_enabled? {
            0
        } else {
            1
        },
        evidence.vendors.len(),
    ))
}

pub(crate) fn inspect_support_state(
    config_dir: &Path,
    object_uuid: &str,
) -> Result<unica_format_core::ports::SupportEvidence, unica_format_core::source::SourceAdapterError>
{
    let target = config_dir.join("Ext").join("ParentConfigurations.bin");
    let object = if object_uuid.is_empty() {
        None
    } else {
        Some(unica_format_core::navigation::ObjectKey::new(object_uuid)?)
    };
    PlatformXmlAdapterFactory::new()
        .registration()
        .support
        .inspect(&unica_format_core::ports::SupportInspectionRequest {
            source: unica_format_core::source::SourceContext::new(
                unica_format_core::source::SourceLocation::new(
                    config_dir.to_path_buf(),
                    config_dir.to_path_buf(),
                    target,
                ),
                None,
                unica_format_core::source::SourceFamily::PlatformXml,
                None,
            ),
            object,
        })
}

pub(crate) fn extract_xml_attr(text: &str, element: &str, attr: &str) -> Option<String> {
    let start = text.find(&format!("<{element}"))?;
    let rest = &text[start..];
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let needle = format!("{attr}=\"");
    let attr_start = tag.find(&needle)? + needle.len();
    let value_rest = &tag[attr_start..];
    let attr_end = value_rest.find('"')?;
    Some(value_rest[..attr_end].to_string())
}

pub(crate) fn emit_mltext(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn split_camel_case(name: &str) -> String {
    if name.is_empty() {
        return name.to_string();
    }
    let mut result = String::new();
    let mut previous_lower = false;
    for ch in name.chars() {
        if previous_lower && ch.is_uppercase() {
            result.push(' ');
        }
        result.push(ch);
        previous_lower = ch.is_lowercase();
    }
    let mut chars = result.chars();
    let Some(first) = chars.next() else {
        return result;
    };
    format!("{}{}", first, chars.as_str().to_lowercase())
}

pub(crate) fn json_string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).map(json_value_to_python_string)
}

pub(crate) fn json_value_to_python_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => {
            if *value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn json_value_to_python_lower(value: &Value) -> String {
    json_value_to_python_string(value).to_lowercase()
}

pub(crate) fn truthy_json_field(value: &Value, field: &str) -> bool {
    truthy_value(value.get(field))
}

pub(crate) fn truthy_value(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Null) | None => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().unwrap_or(1) != 0,
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(value)) => !value.is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
    }
}

pub(crate) fn json_i64_field(value: &Value, field: &str) -> Option<i64> {
    value.get(field).and_then(json_i64_value)
}

pub(crate) fn json_i64_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}

pub(crate) fn register_mxl_cell_format(
    style_name: &str,
    fill_type: &str,
    defn: &Value,
    font_map: &std::collections::BTreeMap<String, usize>,
    thin_line_index: i64,
    thick_line_index: i64,
    registry: &mut MxlFormatRegistry,
) -> usize {
    let props = mxl_resolve_style(
        style_name,
        fill_type,
        defn,
        font_map,
        thin_line_index,
        thick_line_index,
    );
    registry.register(mxl_format_key(&props), props)
}

pub(crate) fn first_text(doc: &Document<'_>, local_name: &str) -> Option<String> {
    doc.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == local_name)
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn has_element(doc: &Document<'_>, local_name: &str) -> bool {
    doc.descendants()
        .any(|node| node.is_element() && node.tag_name().name() == local_name)
}

pub(crate) fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
