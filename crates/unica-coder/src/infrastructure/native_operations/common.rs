#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::{CFE_VALIDATE_PATH, CF_PATH, RIGHTS_PATH};
use crate::application::{AdapterOutcome, SupportGuardRequirement};
use crate::domain::format_profile::{
    classify_root_version, ExportFormatVersion, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

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

pub(crate) struct Utf8TextSnapshot {
    pub(crate) raw: Vec<u8>,
    pub(crate) text: String,
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

pub(crate) fn is_1c_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_1c_identifier_start(first) {
        return false;
    }
    chars.all(is_1c_identifier_part)
}

pub(crate) fn is_1c_identifier_start(ch: char) -> bool {
    ch == '_'
        || ch.is_ascii_alphabetic()
        || ('А'..='Я').contains(&ch)
        || ('а'..='я').contains(&ch)
        || ch == 'Ё'
        || ch == 'ё'
}

pub(crate) fn is_1c_identifier_part(ch: char) -> bool {
    is_1c_identifier_start(ch) || ch.is_ascii_digit()
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
    let mut file = File::create(path)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    file.write_all(&bytes)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
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
    use super::{read_utf8_sig_snapshot, utf8_bom_bytes};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

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
) -> Result<ExportFormatVersion, String> {
    if let Some(owner) =
        crate::infrastructure::platform_xml_owner::resolve_platform_xml_owner(target, context)
            .map_err(|error| error.message)?
    {
        return match classify_root_version(owner.version.as_deref()) {
            Ok(FormatCompatibility::Supported { actual }) => Ok(actual),
            Ok(compatibility) => Err(format_compatibility_warning(&compatibility)),
            Err(error) => Err(error.to_string()),
        };
    }
    ExportFormatVersion::parse(ACTIVE_FORMAT_PROFILE.export_format)
        .map_err(|error| error.to_string())
}

pub(crate) fn format_compatibility_warning(compatibility: &FormatCompatibility) -> String {
    match compatibility {
        FormatCompatibility::Older { actual } => format!(
            "Export format {actual} is older than supported {}; migrate the configuration before writing.",
            ACTIVE_FORMAT_PROFILE.export_format
        ),
        FormatCompatibility::Newer { actual } => format!(
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
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    let Some(state) = read_support_state(&bin_path) else {
        return vec![if is_extension {
            "Поддержка:      расширение (CFE), правки свободны".to_string()
        } else {
            "Поддержка:      не на поддержке (своя конфигурация)".to_string()
        }];
    };
    if state.removed {
        return vec!["Поддержка:      снята с поддержки полностью".to_string()];
    }

    let mut lines = vec!["Поддержка:      на поддержке".to_string()];
    if state.global_editing_enabled {
        lines.push("  Возможность изменения: включена".to_string());
        lines.push(format!(
            "  Объектов: на замке {} / редактируется {} / снято {}",
            state.counts[0], state.counts[1], state.counts[2]
        ));
    } else {
        lines.push(
            "  Возможность изменения: выключена — вся конфигурация read-only (правки заблокированы)"
                .to_string(),
        );
    }
    lines.push(format!("  Конфигураций поставщика: {}", state.vendor_count));
    if state.vendor_count > 1 {
        for vendor in &state.vendors {
            lines.push(format!(
                "  Поставщик: {} — {} {}",
                vendor.vendor, vendor.name, vendor.version
            ));
        }
    }
    lines
}

pub(crate) fn support_status_for_path(target_path: &Path) -> String {
    let Some(config_dir) = find_support_config_dir(target_path) else {
        return "не на поддержке".to_string();
    };
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    let Some(state) = read_support_state(&bin_path) else {
        return "не на поддержке".to_string();
    };
    if state.removed {
        return "снято с поддержки (правки свободны)".to_string();
    }
    if !state.global_editing_enabled {
        return "конфигурация read-only (возможность изменения выключена) — правки невозможны без включения"
            .to_string();
    }
    let Some(object_uuid) = support_object_uuid_for_path(target_path) else {
        return "не на поддержке".to_string();
    };
    match state.object_rule(&object_uuid) {
        Some(0) => "на замке — прямая правка сломает обновления; дорабатывай через cfe-* либо включи редактирование объекта".to_string(),
        Some(1) => "редактируется с сохранением поддержки".to_string(),
        Some(2) => "снято с поддержки (правки свободны)".to_string(),
        _ => "не на поддержке".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportGuardViolation {
    pub code: &'static str,
    pub reason: String,
    pub target_path: PathBuf,
    pub config_dir: PathBuf,
}

pub(crate) fn support_guard_violation(
    target_path: &Path,
    requirement: SupportGuardRequirement,
) -> Option<SupportGuardViolation> {
    let target_path = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.to_path_buf());
    let config_dir = find_support_config_dir(&target_path)?;
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    let state = read_support_state(&bin_path)?;
    if state.removed {
        return None;
    }
    if !state.global_editing_enabled {
        return Some(SupportGuardViolation {
            code: "capability-off",
            reason: "возможность изменения конфигурации выключена (вся конфигурация read-only)"
                .to_string(),
            target_path,
            config_dir,
        });
    }

    let object_uuid = support_object_uuid_for_path(&target_path)
        .or_else(|| support_root_uuid(&config_dir.join("Configuration.xml")));
    let object_rule = object_uuid
        .as_deref()
        .and_then(|uuid| state.object_rule(uuid));
    match requirement {
        SupportGuardRequirement::Removed if object_rule.is_some_and(|rule| rule != 2) => {
            Some(SupportGuardViolation {
                code: "not-removed",
                reason: "объект не снят с поддержки — удаление сломает обновления".to_string(),
                target_path,
                config_dir,
            })
        }
        SupportGuardRequirement::Editable if object_rule == Some(0) => {
            Some(SupportGuardViolation {
                code: "locked",
                reason: "объект на замке — редактирование сломает обновления".to_string(),
                target_path,
                config_dir,
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SupportState {
    global_editing_enabled: bool,
    vendor_count: usize,
    removed: bool,
    counts: [usize; 3],
    object_rules: HashMap<String, u8>,
    vendors: Vec<SupportVendor>,
}

impl SupportState {
    fn object_rule(&self, object_uuid: &str) -> Option<u8> {
        self.object_rules
            .get(&object_uuid.to_ascii_lowercase())
            .copied()
    }

    pub(crate) fn global_editing_enabled(&self) -> bool {
        self.global_editing_enabled
    }

    pub(crate) fn removed(&self) -> bool {
        self.removed
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SupportVendor {
    version: String,
    vendor: String,
    name: String,
}

pub(crate) fn read_support_state(bin_path: &Path) -> Option<SupportState> {
    if !bin_path.is_file() {
        return None;
    }
    let data = fs::read(bin_path).ok()?;
    if data.len() <= 32 {
        return Some(SupportState {
            global_editing_enabled: true,
            vendor_count: 0,
            removed: true,
            counts: [0, 0, 0],
            object_rules: HashMap::new(),
            vendors: Vec::new(),
        });
    }
    let text = String::from_utf8_lossy(data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&data));
    let (global_flag, vendor_count) = parse_support_header(&text)?;
    if vendor_count == 0 {
        return Some(SupportState {
            global_editing_enabled: true,
            vendor_count,
            removed: true,
            counts: [0, 0, 0],
            object_rules: HashMap::new(),
            vendors: Vec::new(),
        });
    }
    let (counts, object_rules) = parse_support_object_rules(&text);
    Some(SupportState {
        global_editing_enabled: global_flag == 0,
        vendor_count,
        removed: false,
        counts,
        object_rules,
        vendors: parse_support_vendors(&text),
    })
}

pub(crate) fn parse_support_header(text: &str) -> Option<(u8, usize)> {
    let mut parts = text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .strip_prefix('{')?
        .split(',')
        .map(str::trim);
    if parts.next()? != "6" {
        return None;
    }
    let global_flag = parts.next()?.parse::<u8>().ok()?;
    let vendor_count = parts.next()?.parse::<usize>().ok()?;
    Some((global_flag, vendor_count))
}

pub(crate) fn parse_support_object_rules(text: &str) -> ([usize; 3], HashMap<String, u8>) {
    let mut counts = [0usize; 3];
    let mut object_rules = HashMap::<String, u8>::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 40 <= bytes.len() {
        let flag = bytes[i];
        if matches!(flag, b'0'..=b'2') && bytes.get(i + 1..i + 4) == Some(b",0,") {
            let uuid_start = i + 4;
            let uuid_end = uuid_start + 36;
            if uuid_end <= bytes.len() {
                let uuid = &text[uuid_start..uuid_end];
                if is_uuid_text(uuid) {
                    let flag_value = flag - b'0';
                    counts[flag_value as usize] += 1;
                    let entry = object_rules
                        .entry(uuid.to_ascii_lowercase())
                        .or_insert(flag_value);
                    if flag_value < *entry {
                        *entry = flag_value;
                    }
                    i = uuid_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    (counts, object_rules)
}

pub(crate) fn parse_support_vendors(text: &str) -> Vec<SupportVendor> {
    let quoted = parse_quoted_support_strings(text);
    quoted
        .chunks(3)
        .filter_map(|chunk| {
            if chunk.len() == 3 {
                Some(SupportVendor {
                    version: chunk[0].clone(),
                    vendor: chunk[1].clone(),
                    name: chunk[2].clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn parse_quoted_support_strings(text: &str) -> Vec<String> {
    let mut result = Vec::<String>::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(next) = chars.next() {
            if next == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    value.push('"');
                    continue;
                }
                break;
            }
            value.push(next);
        }
        result.push(value);
    }
    result
}

pub(crate) fn is_uuid_text(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, ch)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                ch == '-'
            } else {
                ch.is_ascii_hexdigit()
            }
        })
}

pub(crate) fn find_support_config_dir(target_path: &Path) -> Option<PathBuf> {
    let mut current = if target_path.is_dir() {
        target_path.to_path_buf()
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

pub(crate) fn support_object_uuid_for_path(target_path: &Path) -> Option<String> {
    if target_path.is_file() {
        if let Some(uuid) = support_root_uuid(target_path) {
            return Some(uuid);
        }
    }
    let mut current = if target_path.is_dir() {
        target_path.to_path_buf()
    } else {
        target_path.parent()?.to_path_buf()
    };
    for _ in 0..20 {
        let candidate = current.with_extension("xml");
        if candidate.is_file() {
            if let Some(uuid) = support_root_uuid(&candidate) {
                return Some(uuid);
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
    None
}

pub(crate) fn support_root_uuid(xml_path: &Path) -> Option<String> {
    let text = fs::read_to_string(xml_path).ok()?;
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
