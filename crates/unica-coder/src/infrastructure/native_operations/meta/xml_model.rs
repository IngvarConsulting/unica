use super::super::common::{escape_xml, multilang_text};
use super::super::role::role_info_element;
use super::info::meta_info_object_type_ru;
use crate::domain::metadata::{
    DateFractions, MetaFillValue, MetadataType, MetadataTypeVariant, NumberSign, StringLengthMode,
};

pub(crate) fn parse_metadata_image(
    bytes: &[u8],
) -> Result<(&str, roxmltree::Document<'_>), String> {
    let source = std::str::from_utf8(bytes)
        .map_err(|error| format!("metadata image is not UTF-8: {error}"))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(source)
        .map_err(|error| format!("metadata XML parse failed: {error}"))?;
    Ok((source, document))
}

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

pub(super) enum MetadataXmlType<'a> {
    Boolean,
    String { length: u32 },
    Number { digits: u32, fraction: u32 },
    DateTime,
    Configuration(&'a str),
}

pub(super) fn emit_metadata_xml_value_type(
    lines: &mut Vec<String>,
    indent: &str,
    metadata_types: &[MetadataXmlType<'_>],
) {
    lines.push(format!("{indent}<Type>"));
    emit_metadata_xml_type_contents(lines, &format!("{indent}\t"), metadata_types);
    lines.push(format!("{indent}</Type>"));
}

pub(super) fn emit_metadata_xml_type_contents(
    lines: &mut Vec<String>,
    indent: &str,
    metadata_types: &[MetadataXmlType<'_>],
) {
    for metadata_type in metadata_types {
        let wire_name = match metadata_type {
            MetadataXmlType::Boolean => "xs:boolean".to_string(),
            MetadataXmlType::String { .. } => "xs:string".to_string(),
            MetadataXmlType::Number { .. } => "xs:decimal".to_string(),
            MetadataXmlType::DateTime => "xs:dateTime".to_string(),
            MetadataXmlType::Configuration(name) => format!("cfg:{name}"),
        };
        lines.push(format!(
            "{indent}<v8:Type>{}</v8:Type>",
            escape_xml(&wire_name)
        ));
    }
    for qualifier_rank in 0..3 {
        for metadata_type in metadata_types {
            let rank = match metadata_type {
                MetadataXmlType::Number { .. } => 0,
                MetadataXmlType::String { .. } => 1,
                MetadataXmlType::DateTime => 2,
                MetadataXmlType::Boolean | MetadataXmlType::Configuration(_) => continue,
            };
            if rank != qualifier_rank {
                continue;
            }
            match metadata_type {
                MetadataXmlType::Number { digits, fraction } => {
                    lines.push(format!("{indent}<v8:NumberQualifiers>"));
                    lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
                    lines.push(format!(
                        "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
                    ));
                    lines.push(format!("{indent}\t<v8:AllowedSign>Any</v8:AllowedSign>"));
                    lines.push(format!("{indent}</v8:NumberQualifiers>"));
                }
                MetadataXmlType::String { length } => {
                    lines.push(format!("{indent}<v8:StringQualifiers>"));
                    lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
                    lines.push(format!(
                        "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
                    ));
                    lines.push(format!("{indent}</v8:StringQualifiers>"));
                }
                MetadataXmlType::DateTime => {
                    lines.push(format!("{indent}<v8:DateQualifiers>"));
                    lines.push(format!(
                        "{indent}\t<v8:DateFractions>DateTime</v8:DateFractions>"
                    ));
                    lines.push(format!("{indent}</v8:DateQualifiers>"));
                }
                MetadataXmlType::Boolean | MetadataXmlType::Configuration(_) => {}
            }
        }
    }
}

/// Emit the closed metadata type algebra directly to Platform XML. This is the
/// typed writer boundary: it intentionally does not round-trip through the
/// legacy string grammar.
pub(super) fn emit_meta_typed_value_type(
    lines: &mut Vec<String>,
    indent: &str,
    metadata_type: &MetadataType,
) {
    lines.push(format!("{indent}<Type>"));
    let content_indent = format!("{indent}\t");
    for expected_tag in ["Type", "TypeSet"] {
        for variant in &metadata_type.variants {
            let (tag, wire_name) = typed_wire_type(variant);
            if tag != expected_tag {
                continue;
            }
            lines.push(format!(
                "{content_indent}<v8:{tag}>{}</v8:{tag}>",
                escape_xml(&wire_name)
            ));
        }
    }
    for qualifier_rank in 0..4 {
        for variant in &metadata_type.variants {
            let variant_rank = match variant {
                MetadataTypeVariant::Number { .. } => 0,
                MetadataTypeVariant::String { .. } => 1,
                MetadataTypeVariant::Date { .. } => 2,
                MetadataTypeVariant::BinaryData { .. } => 3,
                _ => continue,
            };
            if variant_rank != qualifier_rank {
                continue;
            }
            match variant {
                MetadataTypeVariant::String {
                    length,
                    allowed_length,
                } => {
                    lines.push(format!("{content_indent}<v8:StringQualifiers>"));
                    lines.push(format!("{content_indent}\t<v8:Length>{length}</v8:Length>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:AllowedLength>{}</v8:AllowedLength>",
                        match allowed_length {
                            StringLengthMode::Variable => "Variable",
                            StringLengthMode::Fixed => "Fixed",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:StringQualifiers>"));
                }
                MetadataTypeVariant::Number {
                    digits,
                    fraction,
                    sign,
                } => {
                    lines.push(format!("{content_indent}<v8:NumberQualifiers>"));
                    lines.push(format!("{content_indent}\t<v8:Digits>{digits}</v8:Digits>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
                    ));
                    lines.push(format!(
                        "{content_indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
                        match sign {
                            NumberSign::Any => "Any",
                            NumberSign::NonNegative => "Nonnegative",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:NumberQualifiers>"));
                }
                MetadataTypeVariant::Date { fractions } => {
                    lines.push(format!("{content_indent}<v8:DateQualifiers>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:DateFractions>{}</v8:DateFractions>",
                        match fractions {
                            DateFractions::Date => "Date",
                            DateFractions::Time => "Time",
                            DateFractions::DateTime => "DateTime",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:DateQualifiers>"));
                }
                MetadataTypeVariant::BinaryData {
                    length,
                    allowed_length,
                } => {
                    lines.push(format!("{content_indent}<v8:BinaryDataQualifiers>"));
                    lines.push(format!("{content_indent}\t<v8:Length>{length}</v8:Length>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:AllowedLength>{}</v8:AllowedLength>",
                        match allowed_length {
                            StringLengthMode::Variable => "Variable",
                            StringLengthMode::Fixed => "Fixed",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:BinaryDataQualifiers>"));
                }
                MetadataTypeVariant::Boolean
                | MetadataTypeVariant::ValueStorage
                | MetadataTypeVariant::Reference { .. }
                | MetadataTypeVariant::DefinedType { .. } => {}
            }
        }
    }
    lines.push(format!("{indent}</Type>"));
}

fn typed_wire_type(variant: &MetadataTypeVariant) -> (&'static str, String) {
    match variant {
        MetadataTypeVariant::String { .. } => ("Type", "xs:string".to_string()),
        MetadataTypeVariant::Number { .. } => ("Type", "xs:decimal".to_string()),
        MetadataTypeVariant::Boolean => ("Type", "xs:boolean".to_string()),
        MetadataTypeVariant::Date { .. } => ("Type", "xs:dateTime".to_string()),
        MetadataTypeVariant::BinaryData { .. } => ("Type", "xs:binary".to_string()),
        MetadataTypeVariant::ValueStorage => ("Type", "v8:ValueStorage".to_string()),
        MetadataTypeVariant::Reference { metadata_path } => {
            let mut segments = metadata_path.segments();
            let kind = segments.next().unwrap_or_default();
            let tail = segments.collect::<Vec<_>>().join(".");
            ("Type", format!("cfg:{kind}Ref.{tail}"))
        }
        MetadataTypeVariant::DefinedType { metadata_path } => {
            ("TypeSet", format!("cfg:{}", metadata_path.as_str()))
        }
    }
}

pub(super) fn emit_meta_typed_fill_value(
    lines: &mut Vec<String>,
    indent: &str,
    fill_value: Option<&MetaFillValue>,
) {
    match fill_value {
        None => lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>")),
        Some(MetaFillValue::String(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:string\">{}</FillValue>",
            escape_xml(value)
        )),
        Some(MetaFillValue::Number(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:decimal\">{}</FillValue>",
            escape_xml(value)
        )),
        Some(MetaFillValue::Boolean(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:boolean\">{value}</FillValue>"
        )),
        Some(MetaFillValue::DateTime(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:dateTime\">{}</FillValue>",
            escape_xml(value)
        )),
        Some(MetaFillValue::Reference(reference)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xr:DesignTimeRef\">{}</FillValue>",
            escape_xml(reference.metadata_path.as_str())
        )),
    }
}
