//! The existing EditType declaration; form assignments and contents stay outside it.
use roxmltree::{Document, Node};
use unica_address::source_target::MetadataAddress;
use unica_metadata::{MetaEditOperation, MetaInfoPropertyValue, MetaPropertyKey, MetadataKind};
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
// Reading an existing mode is independent of the mutation registry.
const OBSERVED_MODES: &[&str] = &["InDialog", "InList", "BothWays"];

pub(crate) fn existing<'doc, 'input>(
    object: Node<'doc, 'input>,
    kind: MetadataKind,
) -> Result<(Node<'doc, 'input>, String), String> {
    if !matches!(
        kind,
        MetadataKind::Catalog | MetadataKind::InformationRegister
    ) || !object.has_tag_name((MD, kind.as_str()))
    {
        return Err("EditType owner is outside the writable metadata profile".into());
    }
    let containers: Vec<_> = object
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Properties")
        .collect();
    let [properties] = containers.as_slice() else {
        return Err("EditType requires one direct Properties container".into());
    };
    if !properties.has_tag_name((MD, "Properties")) || properties.attributes().len() != 0 {
        return Err("EditType Properties has an invalid namespace or attributes".into());
    }
    let value = crate::info_projection::observe_meta_info_property(*properties, kind, "EditType")
        .map_err(|e| e.message)?
        .ok_or("metadata owner has no EditType property")?;
    let MetaInfoPropertyValue::String(value) = value else {
        return Err("EditType is not a scalar mode".into());
    };
    if !OBSERVED_MODES.contains(&value.as_str()) {
        return Err("existing EditType is not a recognized platform mode".into());
    }
    let field = properties
        .children()
        .find(|n| n.has_tag_name((MD, "EditType")))
        .expect("the observer proved one direct field");
    Ok((field, value))
}

pub(crate) fn replacement(
    source: &str,
    field: Node<'_, '_>,
    value: &str,
) -> Result<String, String> {
    let fragment = &source[field.range()];
    let end = crate::predefined::opening_tag_end(fragment)?;
    let opening = &fragment[..end];
    let opening = opening.strip_suffix('/').unwrap_or(opening);
    let name_end = opening[1..]
        .find(|c: char| c.is_whitespace() || c == '/')
        .map_or(opening.len(), |i| i + 1);
    let name = &opening[1..name_end];
    Ok(format!(
        "{opening}>{}</{name}>",
        unica_xml_format::text::escape_xml_text(value)
    ))
}

/// Only this new property captures the owner registration. Other metadata
/// property edits retain their existing admission and interpretation.
pub(crate) fn guard(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    target: &MetadataAddress,
    bytes: &[u8],
    operation: &MetaEditOperation,
    index: usize,
) -> Result<(), ApplyPlanError> {
    let MetaEditOperation::SetProperties { values } = operation else {
        return Ok(());
    };
    if !values
        .entries()
        .iter()
        .any(|(key, _)| *key == MetaPropertyKey::EditType)
    {
        return Ok(());
    }
    let at = format!("ops[{index}].args.values.EditType");
    let invalid = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(&at)
    };
    if source_kind != SourceSetKind::Configuration {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "EditType editing requires a configuration source set",
        )
        .at_path(&at));
    }
    let parts: Vec<_> = target.segments().collect();
    let [kind, name] = parts.as_slice() else {
        return Err(invalid("EditType requires one named metadata owner".into()));
    };
    let kind = MetadataKind::parse(kind).map_err(|e| invalid(e.message))?;
    if !matches!(
        kind,
        MetadataKind::Catalog | MetadataKind::InformationRegister
    ) {
        return Err(invalid(
            "EditType owner is outside the writable profile".into(),
        ));
    }
    let root_path = std::path::Path::new("Configuration.xml");
    let root = staged
        .read(root_path)
        .map_err(|e| ApplyPlanError::staging(e, &at))?
        .ok_or_else(|| invalid("configuration descriptor was not found".into()))?;
    let root = unica_source_designer::owner::prove_already_read_source_set_owner(
        root_path,
        &root,
        source_kind,
    )
    .map_err(|e| invalid(e.message))?;
    if root.version() != Some("2.20") || !root.registers(kind.as_str(), name) {
        return Err(invalid(
            "EditType owner is not registered in a format 2.20 configuration".into(),
        ));
    }
    let relative = unica_source_designer::layout::metadata_descriptor_relative(target, source_kind)
        .map_err(invalid)?;
    let owner = unica_source_designer::owner::prove_already_read_metadata_owner(&relative, bytes)
        .map_err(|e| invalid(e.message))?;
    if owner.version() != Some("2.20")
        || owner.artifact_kind() != kind.as_str()
        || owner.artifact_name() != Some(*name)
    {
        return Err(invalid(
            "EditType descriptor identity does not match its target".into(),
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
    let doc =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|e| invalid(e.to_string()))?;
    let object = doc
        .root_element()
        .children()
        .find(Node::is_element)
        .ok_or_else(|| invalid("metadata owner is absent".into()))?;
    let (field, _) = existing(object, kind).map_err(invalid)?;
    let properties = field
        .parent()
        .expect("direct metadata field has a Properties parent");
    let names: Vec<_> = properties
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Name")
        .collect();
    let [identity] = names.as_slice() else {
        return Err(invalid(
            "EditType owner requires one exact scalar Name".into(),
        ));
    };
    if !identity.has_tag_name((MD, "Name"))
        || identity.attributes().len() != 0
        || identity.children().any(|n| n.is_element() || n.is_pi())
        || crate::info_projection::direct_text_content(*identity) != *name
    {
        return Err(invalid(
            "EditType owner requires one exact scalar Name".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn xml(field: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Catalog><Properties><Name>Probe</Name>{field}<DefaultObjectForm>Catalog.Probe.Form.Main</DefaultObjectForm></Properties></Catalog></MetaDataObject>")
    }
    #[test]
    fn existing_edit_type_keeps_the_read_profile_and_rejects_ambiguous_shapes() {
        for value in OBSERVED_MODES {
            let text = xml(&format!("<EditType>{value}</EditType>"));
            let doc = Document::parse(&text).unwrap();
            assert_eq!(
                existing(
                    doc.root_element().first_element_child().unwrap(),
                    MetadataKind::Catalog
                )
                .unwrap()
                .1,
                *value
            );
        }
        for field in [
            "",
            "<EditType/>",
            "<EditType>Unknown</EditType>",
            "<EditType>InList</EditType><EditType>InDialog</EditType>",
            "<EditType xmlns=\"urn:foreign\">InDialog</EditType>",
            "<EditType xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/>",
            "<EditType><Nested/></EditType>",
        ] {
            let text = xml(field);
            let doc = Document::parse(&text).unwrap();
            assert!(
                existing(
                    doc.root_element().first_element_child().unwrap(),
                    MetadataKind::Catalog
                )
                .is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn edit_type_replacement_preserves_local_namespace_and_other_fields() {
        let text =
            xml("<m:EditType xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">InDialog</m:EditType>");
        let doc = Document::parse(&text).unwrap();
        let (field, _) = existing(
            doc.root_element().first_element_child().unwrap(),
            MetadataKind::Catalog,
        )
        .unwrap();
        let old = &text[field.range()];
        let changed = text.replacen(old, &replacement(&text, field, "InList").unwrap(), 1);
        let output = Document::parse(&changed).unwrap();
        let (field, value) = existing(
            output.root_element().first_element_child().unwrap(),
            MetadataKind::Catalog,
        )
        .unwrap();
        assert_eq!(value, "InList");
        assert_eq!(changed.replacen(&changed[field.range()], old, 1), text);
    }
}

#[cfg(test)]
#[path = "../tests/platform/edit_type.rs"]
mod native_tests;
