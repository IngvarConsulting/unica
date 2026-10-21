//! Lossless existing role XML edits and exact postimage validation.
use roxmltree::Document;
use std::collections::HashSet;
use unica_roles::{RoleEditEffect, RoleEditEffectAction};
use unica_source_designer::roots::{platform_xml_publication_policy, PlatformXmlPublicationPolicy};
use unica_xml_format::text::{
    resolve_line_ending, EolPolicy, LineEndingProfile, SourceTextSnapshot,
};

pub fn decode_role_xml(raw: &[u8]) -> Result<(bool, String), String> {
    let (bom, body) = if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (true, &raw[3..])
    } else {
        (false, raw)
    };
    let text = std::str::from_utf8(body)
        .map_err(|_| "role XML must be UTF-8 with an optional BOM".to_string())?;
    Ok((bom, text.to_string()))
}

pub fn encode_role_xml(bom: bool, body: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(body.len() + usize::from(bom) * 3);
    if bom {
        output.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    output.extend_from_slice(body.as_bytes());
    output
}

pub fn direct_role_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
    namespace: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == name
                && child.tag_name().namespace() == Some(namespace)
        })
        .collect()
}

pub fn role_text_content(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants()
        .filter(|descendant| descendant.is_text())
        .filter_map(|descendant| descendant.text())
        .collect()
}

fn role_direct_boolean(node: roxmltree::Node<'_, '_>) -> Option<bool> {
    let mut text_nodes = node.children().filter(|child| child.is_text());
    let value = text_nodes.next()?.text()?;
    if text_nodes.next().is_some() {
        return None;
    }
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub fn validate_role_rights_document(text: &str, values: RoleValueScope<'_>) -> Result<(), String> {
    let document =
        Document::parse(text).map_err(|_| "Rights.xml is not well-formed XML".to_string())?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or("");
    if (namespace, root.tag_name().name()) != (ROLE_RIGHTS_NAMESPACE, "Rights")
        || platform_xml_publication_policy(namespace, root.tag_name().name())
            != Some(PlatformXmlPublicationPolicy::ExactRootVersion)
        || unica_source_designer::roots::root_version_literal(text, root).as_deref() != Some("2.20")
        || root.attribute((XSI_NAMESPACE, "type")) != Some("Rights")
    {
        return Err("Rights.xml must use exact roles format 2.20".to_string());
    }
    for flag in [
        "setForNewObjects",
        "setForAttributesByDefault",
        "independentRightsOfChildObjects",
    ] {
        let nodes = direct_role_children(root, flag, ROLE_RIGHTS_NAMESPACE);
        if nodes.len() != 1 || role_direct_boolean(nodes[0]).is_none() {
            return Err(format!(
                "Rights.xml must contain one direct boolean `{flag}` element"
            ));
        }
    }

    let mut seen_objects = HashSet::new();
    for object in direct_role_children(root, "object", ROLE_RIGHTS_NAMESPACE) {
        let names = direct_role_children(object, "name", ROLE_RIGHTS_NAMESPACE);
        if names.len() > 1 {
            return Err("a direct role object must not have duplicate direct names".to_string());
        }
        let Some(name) = names.first() else {
            continue;
        };
        let object_name = role_text_content(*name);
        if object_name.is_empty() {
            continue;
        }
        if !seen_objects.insert(object_name.clone()) {
            return Err(format!("duplicate role object `{object_name}`"));
        }
        let mut seen_rights = HashSet::new();
        for right in direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE) {
            let names = direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE);
            if names.len() > 1 {
                return Err(format!(
                    "a right of `{object_name}` must not have duplicate direct names"
                ));
            }
            let Some(name) = names.first() else {
                continue;
            };
            let right_name = role_text_content(*name);
            if right_name.is_empty() {
                continue;
            }
            if !seen_rights.insert(right_name.clone()) {
                return Err(format!(
                    "duplicate right `{right_name}` for object `{object_name}`"
                ));
            }
            let value_nodes = direct_role_children(right, "value", ROLE_RIGHTS_NAMESPACE);
            if value_nodes.len() != 1 {
                return Err(format!(
                    "right `{right_name}` of `{object_name}` must have one direct value"
                ));
            }
            if values.requires_boolean(&object_name, &right_name)
                && role_direct_boolean(value_nodes[0]).is_none()
            {
                return Err(format!(
                    "right `{right_name}` of `{object_name}` must have a boolean value"
                ));
            }
        }
    }
    Ok(())
}

pub fn apply_role_edit_operation(
    text: &str,
    operation: &unica_roles::RoleEditOperation,
    operation_index: usize,
) -> Result<(String, RoleEditEffect), String> {
    let document =
        Document::parse(text).map_err(|_| "Rights.xml is not well-formed XML".to_string())?;
    let root = document.root_element();
    let mut objects = direct_role_children(root, "object", ROLE_RIGHTS_NAMESPACE)
        .into_iter()
        .filter(|object| {
            let names = direct_role_children(*object, "name", ROLE_RIGHTS_NAMESPACE);
            names.len() == 1
                && role_text_content(names[0]).as_str() == operation.object_name.as_str()
        })
        .collect::<Vec<_>>();
    if objects.len() > 1 {
        return Err(format!(
            "role object `{}` is ambiguous",
            operation.object_name
        ));
    }
    let mut segments = operation.object_name.split('.');
    let top_level =
        segments.next().is_some() && segments.next().is_some() && segments.next().is_none();
    let data_processor_cascade = !operation.value
        && operation.right == "Use"
        && top_level
        && operation.object_name.starts_with("DataProcessor.");
    // ibcmd 8.3.27.2074 хранит объектное право только при значении, отличном
    // от умолчания роли (`setForNewObjects`), и выбрасывает опустевший блок
    // `object`; явное право с умолчанием — не платформенная форма, и запись
    // до умолчания выражается удалением элемента.
    let stores_to_default =
        top_level && !data_processor_cascade && operation.value == role_set_for_new_objects(root);
    let Some(object) = objects.pop() else {
        if data_processor_cascade {
            return Ok((
                text.to_string(),
                RoleEditEffect {
                    operation_index,
                    operation: "setRight",
                    object_name: operation.object_name.clone(),
                    right: operation.right.clone(),
                    before: None,
                    after: false,
                    action: RoleEditEffectAction::RemoveObject,
                    changed: false,
                },
            ));
        }
        if stores_to_default {
            return Ok((
                text.to_string(),
                RoleEditEffect {
                    operation_index,
                    operation: "setRight",
                    object_name: operation.object_name.clone(),
                    right: operation.right.clone(),
                    before: None,
                    after: operation.value,
                    action: RoleEditEffectAction::SetRight,
                    changed: false,
                },
            ));
        }
        return Err(format!(
            "role object `{}` was not found",
            operation.object_name
        ));
    };

    let matching_rights = direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
        .into_iter()
        .filter(|right| {
            let names = direct_role_children(*right, "name", ROLE_RIGHTS_NAMESPACE);
            names.len() == 1 && role_text_content(names[0]).as_str() == operation.right.as_str()
        })
        .collect::<Vec<_>>();
    if matching_rights.len() > 1 {
        return Err(format!(
            "right `{}` of `{}` is ambiguous",
            operation.right, operation.object_name
        ));
    }
    let before = matching_rights.first().and_then(|right| {
        let values = direct_role_children(*right, "value", ROLE_RIGHTS_NAMESPACE);
        (values.len() == 1)
            .then(|| role_direct_boolean(values[0]))
            .flatten()
    });

    let removes_whole_block = stores_to_default
        && matching_rights.len() == 1
        && before != Some(operation.value)
        && direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE).len() == 1
        && object.children().filter(|child| child.is_element()).count() == 2;
    // `removeObject` — только платформенное правило DataProcessor; каноническое
    // опустошение блока остаётся логическим `setRight`: физическая форма
    // хранения не меняет смысла операции.
    let action = if data_processor_cascade {
        RoleEditEffectAction::RemoveObject
    } else {
        RoleEditEffectAction::SetRight
    };
    let edit = if data_processor_cascade {
        RoleTextEdit::Remove(line_aware_removal_range(text, object.range()))
    } else if let Some(right) = matching_rights.first() {
        if before == Some(operation.value) {
            RoleTextEdit::None
        } else if stores_to_default {
            // Переход к умолчанию — удаление элемента: платформа хранит только
            // отличающиеся права, а опустевший блок `object` не переживает её
            // экспорт. Блок с неизвестными детьми сохраняется — уходит лишь
            // само право.
            if removes_whole_block {
                RoleTextEdit::Remove(line_aware_removal_range(text, object.range()))
            } else {
                RoleTextEdit::Remove(line_aware_removal_range(text, right.range()))
            }
        } else {
            let values = direct_role_children(*right, "value", ROLE_RIGHTS_NAMESPACE);
            if values.len() != 1 {
                return Err(format!(
                    "right `{}` of `{}` must have one direct value",
                    operation.right, operation.object_name
                ));
            }
            let value = values[0];
            let range = value.range();
            let raw = &text[range.clone()];
            if raw.trim_end().ends_with("/>") {
                let trimmed_end = raw.trim_end().len();
                let slash = raw[..trimmed_end]
                    .rfind("/>")
                    .ok_or_else(|| "role value self-closing tag is malformed".to_string())?;
                let name_start = raw
                    .find('<')
                    .map(|offset| offset + 1)
                    .ok_or_else(|| "role value opening tag is malformed".to_string())?;
                let name_end = raw[name_start..]
                    .find(|character: char| {
                        character.is_whitespace() || matches!(character, '/' | '>')
                    })
                    .map(|offset| name_start + offset)
                    .ok_or_else(|| "role value QName is malformed".to_string())?;
                let qualified_name = &raw[name_start..name_end];
                RoleTextEdit::Replace(
                    range,
                    format!(
                        "{}>{}</{}>{}",
                        raw[..slash].trim_end(),
                        operation.value,
                        qualified_name,
                        &raw[trimmed_end..]
                    ),
                )
            } else if value.children().next().is_none() {
                // `<value></value>` инфосет-эквивалентен self-closing форме:
                // текст значения вставляется внутрь существующих тегов.
                let open_end = raw
                    .find('>')
                    .map(|offset| range.start + offset + 1)
                    .ok_or_else(|| "role value opening tag is malformed".to_string())?;
                RoleTextEdit::Insert(open_end, operation.value.to_string())
            } else {
                let text_nodes = value
                    .children()
                    .filter(|child| child.is_text())
                    .collect::<Vec<_>>();
                if text_nodes.len() != 1 || !matches!(text_nodes[0].text(), Some("true" | "false"))
                {
                    return Err(format!(
                        "right `{}` of `{}` must have one direct boolean text value",
                        operation.right, operation.object_name
                    ));
                }
                RoleTextEdit::Replace(text_nodes[0].range(), operation.value.to_string())
            }
        }
    } else if stores_to_default {
        // Отсутствующее право уже несёт умолчание роли: запись того же
        // значения — семантический no-op без нормализации чужих байтов.
        RoleTextEdit::None
    } else {
        let (offset, insertion) = right_insertion(text, object, &operation.right, operation.value)?;
        RoleTextEdit::Insert(offset, insertion)
    };
    drop(document);

    let mut updated = text.to_string();
    let changed = !matches!(edit, RoleTextEdit::None);
    match edit {
        RoleTextEdit::Replace(range, replacement) => updated.replace_range(range, &replacement),
        RoleTextEdit::Insert(offset, insertion) => updated.insert_str(offset, &insertion),
        RoleTextEdit::Remove(range) => updated.replace_range(range, ""),
        RoleTextEdit::None => {}
    }
    Ok((
        updated,
        RoleEditEffect {
            operation_index,
            operation: "setRight",
            object_name: operation.object_name.clone(),
            right: operation.right.clone(),
            before,
            after: operation.value,
            action,
            changed,
        },
    ))
}

fn role_set_for_new_objects(root: roxmltree::Node<'_, '_>) -> bool {
    direct_role_children(root, "setForNewObjects", ROLE_RIGHTS_NAMESPACE)
        .first()
        .and_then(|node| role_direct_boolean(*node))
        .unwrap_or(false)
}

fn line_aware_removal_range(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    // Начало и конец строки считаются по `\r|\n`, как в `right_insertion`:
    // CR-only файл иначе оставлял бы после удаления пустую строку с отступом.
    let bytes = text.as_bytes();
    let line_start = bytes[..range.start]
        .iter()
        .rposition(|byte| matches!(byte, b'\r' | b'\n'))
        .map(|offset| offset + 1)
        .unwrap_or(0);
    let starts_on_own_line = text[line_start..range.start]
        .chars()
        .all(|character| matches!(character, ' ' | '\t'));
    let next_break = bytes[range.end..]
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map(|offset| range.end + offset);
    let ends_on_own_line = next_break.is_some_and(|line_end| {
        text[range.end..line_end]
            .chars()
            .all(|character| matches!(character, ' ' | '\t'))
    });

    if starts_on_own_line && ends_on_own_line {
        let line_end = next_break.expect("ends_on_own_line requires a line break");
        let after = if text[line_end..].starts_with("\r\n") {
            line_end + 2
        } else {
            line_end + 1
        };
        line_start..after
    } else {
        range
    }
}

pub fn right_insertion(
    text: &str,
    object: roxmltree::Node<'_, '_>,
    right_name: &str,
    value: bool,
) -> Result<(usize, String), String> {
    let range = object.range();
    let raw = &text[range.clone()];
    let direct_rights = direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE);
    let object_prefix = lexical_role_prefix(text, object)?;
    // The object's own QName binding is necessarily in scope for inserted
    // descendants. A prefix copied from an existing right can be declared on
    // that sibling only and would be unbound on the newly emitted element.
    let right_prefix = object_prefix.clone();
    let name_prefix = object_prefix.clone();
    let value_prefix = object_prefix;
    let close_relative = raw
        .rfind("</")
        .ok_or_else(|| "role object closing tag is malformed".to_string())?;
    let close = range.start + close_relative;
    let line_start = text.as_bytes()[..close]
        .iter()
        .rposition(|byte| matches!(byte, b'\r' | b'\n'))
        .map(|offset| offset + 1)
        .unwrap_or(close);
    let has_line_layout = line_start < close
        && text[line_start..close]
            .chars()
            .all(|character| character == ' ' || character == '\t' || character == '\r');
    if !has_line_layout {
        return Ok((
            close,
            format!(
                "<{right_prefix}right><{name_prefix}name>{right_name}</{name_prefix}name><{value_prefix}value>{value}</{value_prefix}value></{right_prefix}right>"
            ),
        ));
    }
    let closing_indent = text[line_start..close].trim_end_matches('\r');
    let eol = role_source_eol(text)?;
    let right_indent = direct_rights
        .first()
        .map(|right| line_indent(text, right.range().start))
        .or_else(|| {
            direct_role_children(object, "name", ROLE_RIGHTS_NAMESPACE)
                .first()
                .map(|name| line_indent(text, name.range().start))
        })
        .filter(|indent| !indent.is_empty())
        .unwrap_or_else(|| format!("{closing_indent}\t"));
    let child_indent = format!("{right_indent}\t");
    Ok((
        line_start,
        format!(
            "{right_indent}<{right_prefix}right>{eol}{child_indent}<{name_prefix}name>{right_name}</{name_prefix}name>{eol}{child_indent}<{value_prefix}value>{value}</{value_prefix}value>{eol}{right_indent}</{right_prefix}right>{eol}"
        ),
    ))
}

fn role_source_eol(text: &str) -> Result<&'static str, String> {
    let snapshot = SourceTextSnapshot::from_bytes(text.as_bytes())
        .map_err(|error| format!("unsupported_node: {error}"))?;
    let policy = if snapshot.line_endings() == LineEndingProfile::None {
        EolPolicy::Lf
    } else {
        EolPolicy::Preserve
    };
    resolve_line_ending(policy, &snapshot, None)
        .map(|ending| ending.as_str())
        .map_err(|error| format!("unsupported_node: {error}"))
}

fn lexical_role_prefix(text: &str, node: roxmltree::Node<'_, '_>) -> Result<String, String> {
    let raw = &text[node.range()];
    let name_start = raw
        .find('<')
        .map(|offset| offset + 1)
        .ok_or_else(|| "role element opening tag is malformed".to_string())?;
    let name_end = raw[name_start..]
        .find(|character: char| character.is_whitespace() || matches!(character, '/' | '>'))
        .map(|offset| name_start + offset)
        .ok_or_else(|| "role element QName is malformed".to_string())?;
    let qname = &raw[name_start..name_end];
    Ok(qname
        .rsplit_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default())
}

fn line_indent(text: &str, offset: usize) -> String {
    let start = text[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    text[start..offset]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t' | '\r'))
        .filter(|character| *character != '\r')
        .collect()
}

/// Чьё значение права обязано быть булевым. Прообраз источника писателю не
/// принадлежит: небулевое значение соседнего права — свойство исходника, а не
/// дефект мутации, и запрещать им правку другого права нельзя. Пост-образ
/// строго проверяется ровно там, где писатель менял байты.
#[derive(Clone, Copy)]
pub enum RoleValueScope<'a> {
    SourceImage,
    WrittenRights(&'a [(String, String)]),
}

enum RoleTextEdit {
    Replace(std::ops::Range<usize>, String),
    Insert(usize, String),
    Remove(std::ops::Range<usize>),
    None,
}

impl RoleValueScope<'_> {
    fn requires_boolean(self, object_name: &str, right_name: &str) -> bool {
        match self {
            Self::SourceImage => false,
            Self::WrittenRights(written) => written
                .iter()
                .any(|(object, right)| object == object_name && right == right_name),
        }
    }
}

pub const ROLE_RIGHTS_NAMESPACE: &str = "http://v8.1c.ru/8.2/roles";

pub const ROLE_METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";

pub const XSI_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
