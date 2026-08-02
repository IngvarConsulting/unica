#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;

use super::super::common::{escape_xml, multilang_text};
use super::super::form::form_is_xml_ncname;
use super::super::role::role_info_element;
use super::info::meta_info_object_type_ru;
use super::legacy_dsl::{
    meta_compile_is_config_type, parse_meta_number_type, parse_meta_string_type, resolve_meta_type,
};

pub(crate) fn meta_info_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| role_info_element(*child, local_name, None))
}

pub(crate) fn meta_info_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| role_info_element(*child, local_name, None))
        .collect()
}

pub(crate) fn meta_info_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
) -> Option<String> {
    meta_info_child(node, local_name).map(meta_info_inner_text)
}

pub(crate) fn meta_info_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").to_string()
}

pub(super) fn meta_info_ml_text(node: roxmltree::Node<'_, '_>) -> String {
    let value = multilang_text(node);
    if value.is_empty() {
        node.text().unwrap_or("").trim().to_string()
    } else {
        value
    }
}

pub(super) fn meta_info_ml_child_text(
    node: Option<roxmltree::Node<'_, '_>>,
    local_name: &str,
) -> Option<String> {
    node.and_then(|node| meta_info_child(node, local_name))
        .map(meta_info_ml_text)
}

pub(super) fn meta_info_attr_by_local<'a>(
    node: roxmltree::Node<'a, '_>,
    local_name: &str,
) -> Option<&'a str> {
    node.attributes()
        .find(|attr| attr.name() == local_name)
        .map(|attr| attr.value())
}

pub(super) fn meta_info_normalize_cfg_prefix(raw: &str) -> String {
    let Some((prefix, rest)) = raw.split_once(':') else {
        return raw.to_string();
    };
    if prefix.starts_with('d')
        && prefix[1..]
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == 'p')
    {
        format!("cfg:{rest}")
    } else {
        raw.to_string()
    }
}

pub(super) fn meta_info_format_source_type(raw: &str) -> String {
    let normalized = meta_info_normalize_cfg_prefix(raw);
    let Some(rest) = normalized.strip_prefix("cfg:") else {
        return normalized;
    };
    let Some((prefix, name)) = rest.split_once('.') else {
        return rest.to_string();
    };
    if let Some(object_type) = meta_info_object_type_ru(prefix) {
        format!("{object_type}.{name}")
    } else {
        rest.to_string()
    }
}

pub(super) fn emit_meta_mltext(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
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

pub(super) fn emit_meta_value_type(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    lines.push(format!("{indent}<Type>"));
    emit_meta_type_content(lines, &format!("{indent}\t"), type_name);
    lines.push(format!("{indent}</Type>"));
}

pub(super) fn emit_meta_type_content(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    emit_meta_type_contents(lines, indent, std::iter::once(type_name));
}

pub(super) fn emit_meta_type_contents<'a>(
    lines: &mut Vec<String>,
    indent: &str,
    type_names: impl IntoIterator<Item = &'a str>,
) {
    emit_meta_type_contents_with_string_length(lines, indent, type_names, None);
}

pub(super) fn emit_meta_event_subscription_source_type_contents<'a>(
    lines: &mut Vec<String>,
    indent: &str,
    type_names: impl IntoIterator<Item = &'a str>,
) {
    // Event sources are type identities, not constrained values. 8.3.27
    // canonicalizes every string source to the unbounded Length=0 form.
    emit_meta_type_contents_with_string_length(lines, indent, type_names, Some(0));
}

fn emit_meta_type_contents_with_string_length<'a>(
    lines: &mut Vec<String>,
    indent: &str,
    type_names: impl IntoIterator<Item = &'a str>,
    string_length_override: Option<u32>,
) {
    let mut resolved_types = type_names
        .into_iter()
        .flat_map(|type_name| type_name.split('+'))
        .map(str::trim)
        .filter(|type_name| !type_name.is_empty())
        .map(resolve_meta_type)
        .collect::<Vec<_>>();
    // 8.3.27 groups concrete configuration types before primitive types, but
    // orders configuration types by their xr:TypeId from the surrounding
    // configuration. This pure serializer has no workspace TypeId index, so
    // the stable sort deliberately preserves DSL order inside that group.
    resolved_types.sort_by_key(|resolved| meta_type_platform_group_rank(resolved));

    for resolved in resolved_types
        .iter()
        .filter(|resolved| !resolved.starts_with("DefinedType."))
    {
        emit_meta_type_tag(lines, indent, resolved);
    }
    for resolved in resolved_types
        .iter()
        .filter(|resolved| resolved.starts_with("DefinedType."))
    {
        emit_meta_type_tag(lines, indent, resolved);
    }
    for resolved in &resolved_types {
        emit_meta_number_qualifiers(lines, indent, resolved);
    }
    for resolved in &resolved_types {
        emit_meta_string_qualifiers_with_length(lines, indent, resolved, string_length_override);
    }
    for resolved in &resolved_types {
        emit_meta_date_qualifiers(lines, indent, resolved);
    }
}

fn meta_type_platform_group_rank(resolved: &str) -> u8 {
    let (tag, wire_name) = meta_type_wire_contract(resolved);
    match (tag, wire_name.as_str()) {
        ("TypeSet", _) => 6,
        (_, "xs:boolean") => 1,
        (_, "xs:string") => 2,
        (_, "xs:dateTime") => 3,
        (_, "xs:decimal") => 4,
        (_, "v8:ValueStorage") => 5,
        _ => 0,
    }
}

fn meta_type_wire_contract(resolved: &str) -> (&'static str, String) {
    if resolved.starts_with("DefinedType.") {
        ("TypeSet", format!("cfg:{resolved}"))
    } else if resolved == "Boolean" {
        ("Type", "xs:boolean".to_string())
    } else if matches!(resolved, "Date" | "DateTime") {
        ("Type", "xs:dateTime".to_string())
    } else if resolved == "ValueStorage" {
        ("Type", "v8:ValueStorage".to_string())
    } else if resolved == "String" || resolved.starts_with("String(") {
        ("Type", "xs:string".to_string())
    } else if resolved == "Number" || parse_meta_number_type(resolved).is_some() {
        ("Type", "xs:decimal".to_string())
    } else if meta_compile_is_config_type(resolved) {
        ("Type", format!("cfg:{resolved}"))
    } else {
        ("Type", resolved.to_string())
    }
}

pub(super) fn validate_meta_type_union<'a>(
    type_names: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let mut seen = BTreeMap::<(String, String), String>::new();
    let mut type_count = 0usize;
    let mut has_value_storage = false;
    for raw in type_names {
        for type_name in raw
            .split('+')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let resolved = resolve_meta_type(type_name);
            validate_meta_resolved_type(type_name, &resolved)?;
            type_count += 1;
            has_value_storage |= resolved == "ValueStorage";
            let (tag, wire_name) = meta_type_wire_contract(&resolved);
            let key = (tag.to_string(), wire_name.clone());
            if let Some(previous) = seen.insert(key, type_name.to_string()) {
                return Err(format!(
                    "duplicate platform type in valueTypes: {previous} and {type_name} both map to v8:{tag} {wire_name}"
                ));
            }
        }
    }
    if has_value_storage && type_count > 1 {
        return Err(
            "ValueStorage must be the only platform type in an 8.3.27 type description".to_string(),
        );
    }
    Ok(())
}

pub(super) fn validate_meta_resolved_type(raw: &str, resolved: &str) -> Result<(), String> {
    if resolved == "String" {
        return Ok(());
    }
    if resolved.starts_with("String") {
        if parse_meta_string_type(resolved).is_none() {
            return Err(format!(
                "type {raw} is not valid for 8.3.27; expected String or String(integer length 0..=1024)"
            ));
        }
        return Ok(());
    }
    if resolved == "Number" {
        return Ok(());
    }
    if resolved.starts_with("Number") {
        if parse_meta_number_type(resolved).is_none() {
            return Err(format!(
                "type {raw} is not valid for 8.3.27; expected Number(integer digits 0..=38, integer fraction 0..=digits[,nonneg])"
            ));
        }
        return Ok(());
    }
    if resolved.contains(['(', ')']) {
        return Err(format!(
            "type {raw} is not valid for 8.3.27; parameters are supported only for String and Number"
        ));
    }
    if meta_compile_is_config_type(resolved) {
        let invalid_name = resolved
            .split_once('.')
            .is_none_or(|(_, name)| name.trim().is_empty() || name.contains('.'));
        if invalid_name || !form_is_xml_ncname(resolved) {
            return Err(format!(
                "type {raw} is not valid for 8.3.27; configuration type name is not an XML NCName"
            ));
        }
        return Ok(());
    }
    if matches!(resolved, "Boolean" | "Date" | "DateTime" | "ValueStorage") {
        return Ok(());
    }
    Err(format!(
        "type {raw} is not supported by the fixed 8.3.27 metadata DSL"
    ))
}

pub(super) fn emit_meta_type_tag(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    let (tag, wire_name) = meta_type_wire_contract(resolved);
    lines.push(format!(
        "{indent}<v8:{tag}>{}</v8:{tag}>",
        escape_xml(&wire_name)
    ));
}

pub(super) fn emit_meta_number_qualifiers(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    let number = if resolved == "Number" {
        Some((10, 0, false))
    } else {
        parse_meta_number_type(resolved)
    };
    if let Some((digits, fraction, nonnegative)) = number {
        lines.push(format!("{indent}<v8:NumberQualifiers>"));
        lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
        lines.push(format!(
            "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
        ));
        lines.push(format!(
            "{indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
            if nonnegative { "Nonnegative" } else { "Any" }
        ));
        lines.push(format!("{indent}</v8:NumberQualifiers>"));
    }
}

pub(super) fn emit_meta_string_qualifiers(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    emit_meta_string_qualifiers_with_length(lines, indent, resolved, None);
}

fn emit_meta_string_qualifiers_with_length(
    lines: &mut Vec<String>,
    indent: &str,
    resolved: &str,
    length_override: Option<u32>,
) {
    let length = if resolved == "String" {
        Some(length_override.unwrap_or(10))
    } else {
        parse_meta_string_type(resolved).map(|length| length_override.unwrap_or(length))
    };
    if let Some(length) = length {
        lines.push(format!("{indent}<v8:StringQualifiers>"));
        lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
        lines.push(format!(
            "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{indent}</v8:StringQualifiers>"));
    }
}

pub(super) fn emit_meta_date_qualifiers(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    if matches!(resolved, "Date" | "DateTime") {
        lines.push(format!("{indent}<v8:DateQualifiers>"));
        lines.push(format!(
            "{indent}\t<v8:DateFractions>{resolved}</v8:DateFractions>"
        ));
        lines.push(format!("{indent}</v8:DateQualifiers>"));
    }
}

pub(super) fn emit_meta_fill_value(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    if type_name.is_empty() {
        lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>"));
        return;
    }
    let resolved = resolve_meta_type(type_name);
    if resolved == "Boolean" {
        lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:boolean\">false</FillValue>"
        ));
    } else if resolved.starts_with("String") {
        lines.push(format!("{indent}<FillValue xsi:type=\"xs:string\"/>"));
    } else if resolved.starts_with("Number") {
        lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:decimal\">0</FillValue>"
        ));
    } else {
        lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>"));
    }
}
