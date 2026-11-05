//! Root CommonAttribute settings; row settings and the type stay byte-identical.

use roxmltree::{Document, Node};
use std::path::Path;
use unica_metadata::descriptor_operations::{
    DescriptorProperty, DescriptorPropertyKind, PropertiesSite,
};
use unica_metadata::property_input::CommonAttributePropertyChanges;
use unica_metadata::ObservedMetadataTypeVariant;
use unica_source_contract::{proposal::ApplyProposal, SourceSetKind};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn child<'a>(parent: Node<'a, 'a>, name: &str) -> Result<Node<'a, 'a>, String> {
    child_in(parent, MD, name)
}

fn child_in<'a>(parent: Node<'a, 'a>, namespace: &str, name: &str) -> Result<Node<'a, 'a>, String> {
    let mut found = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == name);
    match (found.next(), found.next()) {
        (Some(node), None) if node.has_tag_name((namespace, name)) => Ok(node),
        _ => Err(format!(
            "expected one direct {name} in the metadata namespace"
        )),
    }
}

fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    if node.attributes().next().is_some() || node.children().any(|node| node.is_element()) {
        return Err(format!("{} must be a plain scalar", node.tag_name().name()));
    }
    Ok(node
        .children()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect())
}

fn constant_is_in_content(
    props: Node<'_, '_>,
    reference: &str,
    automatic: bool,
) -> Result<bool, String> {
    let content = child(props, "Content")?;
    let mut selected = None;
    for row in content.children().filter(Node::is_element) {
        if !row.has_tag_name((XR, "Item")) {
            return Err("CommonAttribute Content has an unsupported row".into());
        }
        if scalar(child_in(row, XR, "Metadata")?)? == reference {
            let use_mode = scalar(child_in(row, XR, "Use")?)?;
            let included = match use_mode.as_str() {
                "Use" => true,
                "DontUse" => false,
                "Auto" => automatic,
                _ => return Err("CommonAttribute Content has an unknown Use value".into()),
            };
            if selected.replace(included).is_some() {
                return Err(
                    "CommonAttribute Content has duplicate entries for the conditional constant"
                        .into(),
                );
            }
        }
    }
    Ok(selected.unwrap_or(automatic))
}

fn properties<'a>(
    document: &'a Document<'a>,
    kind: &str,
    name: &str,
) -> Result<Node<'a, 'a>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("expected a metadata descriptor".into());
    }
    let mut owners = root.children().filter(Node::is_element);
    let owner = match (owners.next(), owners.next()) {
        (Some(owner), None) if owner.has_tag_name((MD, kind)) => owner,
        _ => return Err(format!("expected one {kind} owner")),
    };
    let props = child(owner, "Properties")?;
    if scalar(child(props, "Name")?)? != name {
        return Err(format!("{kind} Name differs from its address"));
    }
    Ok(props)
}

pub fn read(source: &str, name: &str) -> Result<Vec<(DescriptorProperty, String)>, String> {
    read_for_source(source, name, SourceSetKind::Configuration)
}

pub fn read_for_source(
    source: &str,
    name: &str,
    source_kind: SourceSetKind,
) -> Result<Vec<(DescriptorProperty, String)>, String> {
    let document = Document::parse(source.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())?;
    let props = properties(&document, "CommonAttribute", name)?;
    PropertiesSite::CommonAttribute
        .for_source_kind(source_kind)
        .descriptor_fields()
        .map(|field| {
            let value = scalar(child(props, field.names()[0])?)?;
            if field.kind() == DescriptorPropertyKind::Enum
                && !field.allowed_values().contains(&value.as_str())
            {
                return Err(format!(
                    "{} has an unknown platform value",
                    field.names()[0]
                ));
            }
            Ok((field, value))
        })
        .collect()
}

pub fn patch(
    source: &str,
    name: &str,
    changes: &CommonAttributePropertyChanges,
) -> Result<String, String> {
    patch_for_source(source, name, changes, SourceSetKind::Configuration)
}

pub fn patch_for_source(
    source: &str,
    name: &str,
    changes: &CommonAttributePropertyChanges,
    source_kind: SourceSetKind,
) -> Result<String, String> {
    let observed = read_for_source(source, name, source_kind)?;
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let props = properties(&document, "CommonAttribute", name)?;
    let mut replacements = Vec::new();
    for (field, value) in &changes.changes {
        let before = observed
            .iter()
            .find(|(candidate, _)| candidate == field)
            .ok_or_else(|| "property is outside the CommonAttribute root profile".to_string())?;
        if value == &before.1 {
            continue;
        }
        let node = child(props, field.names()[0])?;
        let original = &source[node.range()];
        let qualified = original
            .trim_start_matches('<')
            .split([' ', '\t', '\r', '\n', '/', '>'])
            .next()
            .expect("element name");
        let opening = &original[..original.find('>').expect("element opening")];
        let opening = opening.strip_suffix('/').unwrap_or(opening);
        let escaped = value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        replacements.push((node.range(), format!("{opening}>{escaped}</{qualified}>")));
    }
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = source.to_string();
    for (range, replacement) in replacements {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

/// All referenced descriptors and their registrations belong to the saved plan,
/// including when the requested root settings already have their desired values.
pub fn validate_final(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    name: &str,
    relative: &Path,
) -> Result<(), String> {
    let bytes = staged
        .read(relative)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "CommonAttribute descriptor is absent".to_string())?;
    let source = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
    let values = read_for_source(source, name, source_kind)?;
    if source_kind == SourceSetKind::Extension {
        if crate::xml_model::parse_meta_borrowing(&bytes)?
            .extends
            .is_none()
        {
            return Err(
                "8.3.27 permits AutoUse changes only on a borrowed CommonAttribute in an extension"
                    .into(),
            );
        }
        return validate_registration(staged, source_kind, name).map(|_| ());
    }
    let value = |field| {
        values
            .iter()
            .find(|(candidate, _)| *candidate == field)
            .expect("closed root profile")
            .1
            .as_str()
    };
    if value(DescriptorProperty::DataSeparation) == "Separate"
        && (value(DescriptorProperty::DataSeparationValue).is_empty()
            || value(DescriptorProperty::DataSeparationUse).is_empty())
    {
        return Err(
            "DataSeparation=Separate requires nonempty DataSeparationValue and DataSeparationUse"
                .into(),
        );
    }
    let root = validate_registration(staged, source_kind, name)?;
    let document = Document::parse(source.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())?;
    let props = properties(&document, "CommonAttribute", name)?;
    let own_type = crate::info_projection::parse_observed_metadata_type_node(props)
        .map_err(|error| error.message)?;
    let own_type = resolve_type(staged, &root, source_kind, own_type.variants)?;
    if value(DescriptorProperty::DataSeparation) == "Separate" {
        for variant in &own_type {
            match variant {
                unica_metadata::ObservedMetadataTypeVariant::ValueStorage => return Err("DataSeparation=Separate cannot use ValueStorage: the platform cannot hash it".into()),
                unica_metadata::ObservedMetadataTypeVariant::String { length: 0, .. } => return Err("DataSeparation=Separate cannot use an unbounded String: the platform cannot hash it".into()),
                unica_metadata::ObservedMetadataTypeVariant::BinaryData { .. } => return Err("DataSeparation=Separate does not support BinaryData in the 8.3.27 write profile".into()),
                unica_metadata::ObservedMetadataTypeVariant::TypeId { .. } => return Err("DataSeparation=Separate requires a type whose identity can be resolved in this source set".into()),
                _ => {}
            }
        }
    }
    let condition = value(DescriptorProperty::ConditionalSeparation);
    if value(DescriptorProperty::DataSeparation) == "Separate"
        && !condition.is_empty()
        && constant_is_in_content(
            props,
            condition,
            value(DescriptorProperty::AutoUse) == "Use",
        )?
    {
        return Err(
            "ConditionalSeparation constant must not be included in CommonAttribute Content".into(),
        );
    }
    for (field, kind, directory) in [
        (
            DescriptorProperty::DataSeparationValue,
            "SessionParameter",
            "SessionParameters",
        ),
        (
            DescriptorProperty::DataSeparationUse,
            "SessionParameter",
            "SessionParameters",
        ),
        (
            DescriptorProperty::ConditionalSeparation,
            "Constant",
            "Constants",
        ),
    ] {
        let reference = value(field);
        if reference.is_empty() {
            continue;
        }
        let at = unica_address::address::QualifiedAddress::parse(&format!("reference:{reference}"))
            .map_err(|error| error.to_string())?;
        let [owner] = at.segments() else {
            return Err(format!("{} must name {kind}.Name", field.names()[0]));
        };
        let target_name = owner
            .name()
            .filter(|_| owner.kind().as_str() == kind)
            .ok_or_else(|| format!("{} must name {kind}.Name", field.names()[0]))?;
        if !root.registers(kind, target_name) {
            return Err(format!("{reference} is not registered in this source set"));
        }
        let target_path = Path::new(directory).join(format!("{target_name}.xml"));
        let target = staged
            .read(&target_path)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{reference} does not exist"))?;
        unica_source_designer::owner::prove_already_read_metadata_owner(&target_path, &target)
            .map_err(|error| error.message)?;
        let target_source = std::str::from_utf8(&target).map_err(|error| error.to_string())?;
        let target_doc = Document::parse(target_source.trim_start_matches('\u{feff}'))
            .map_err(|error| error.to_string())?;
        let target_props = properties(&target_doc, kind, target_name)?;
        let target_type = crate::info_projection::parse_observed_metadata_type_node(target_props)
            .map_err(|error| error.message)?;
        let target_type = resolve_type(staged, &root, source_kind, target_type.variants)?;
        if field == DescriptorProperty::DataSeparationValue {
            if own_type.len() != target_type.len()
                || own_type
                    .iter()
                    .any(|variant| !target_type.contains(variant))
            {
                return Err("DataSeparationValue type must match the CommonAttribute type".into());
            }
        } else if target_type != [unica_metadata::ObservedMetadataTypeVariant::Boolean] {
            return Err(format!(
                "{} target must have Boolean type",
                field.names()[0]
            ));
        }
    }
    Ok(())
}

/// Named declarations describe effective types, not extra Type identities.
/// Iterative enter/leave frames detect cycles without growing the call stack.
fn resolve_type(
    staged: &mut ApplyProposal<'_>,
    root: &unica_source_designer::owner::PlatformXmlSourceSetOwnerEvidence,
    source_kind: SourceSetKind,
    variants: Vec<ObservedMetadataTypeVariant>,
) -> Result<Vec<ObservedMetadataTypeVariant>, String> {
    enum Visit {
        Type(ObservedMetadataTypeVariant),
        Leave(String),
    }
    let mut pending: Vec<_> = variants.into_iter().rev().map(Visit::Type).collect();
    let mut active = std::collections::BTreeSet::new();
    let mut complete = std::collections::BTreeSet::new();
    let mut resolved = Vec::new();
    while let Some(visit) = pending.pop() {
        let variant = match visit {
            Visit::Leave(identity) => {
                active.remove(&identity);
                complete.insert(identity);
                continue;
            }
            Visit::Type(variant) => variant,
        };
        let (address, expected_kind) = match &variant {
            ObservedMetadataTypeVariant::DefinedType { metadata_path } => {
                (metadata_path, "DefinedType")
            }
            ObservedMetadataTypeVariant::CharacteristicTypes { metadata_path } => {
                (metadata_path, "ChartOfCharacteristicTypes")
            }
            _ => {
                if !resolved.contains(&variant) {
                    resolved.push(variant);
                }
                continue;
            }
        };
        let identity = address.as_str().to_string();
        if complete.contains(&identity) {
            continue;
        }
        if !active.insert(identity.clone()) {
            return Err(format!("cyclic metadata type {identity}"));
        }
        let mut parts = address.segments();
        let (Some(kind), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err("named type must identify one top-level declaration".into());
        };
        if kind != expected_kind || !root.registers(kind, name) {
            return Err(format!(
                "type {identity} is not registered in this source set"
            ));
        }
        let relative =
            unica_source_designer::layout::metadata_descriptor_relative(address, source_kind)?;
        let bytes = staged
            .read(&relative)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("type {identity} does not exist"))?;
        let evidence =
            unica_source_designer::owner::prove_already_read_metadata_owner(&relative, &bytes)
                .map_err(|error| error.message)?;
        if evidence.artifact_kind() != kind || evidence.artifact_name() != Some(name) {
            return Err(format!(
                "type {identity} descriptor differs from its address"
            ));
        }
        let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        let document = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|error| error.to_string())?;
        let props = properties(&document, kind, name)?;
        let expanded = crate::info_projection::parse_observed_metadata_type_node(props)
            .map_err(|error| error.message)?;
        pending.push(Visit::Leave(identity));
        pending.extend(expanded.variants.into_iter().rev().map(Visit::Type));
    }
    Ok(resolved)
}

fn validate_registration(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    name: &str,
) -> Result<unica_source_designer::owner::PlatformXmlSourceSetOwnerEvidence, String> {
    let root_path = Path::new("Configuration.xml");
    let root = staged
        .read(root_path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "source-set descriptor is absent".to_string())?;
    let root = unica_source_designer::owner::prove_already_read_source_set_owner(
        root_path,
        &root,
        source_kind,
    )
    .map_err(|error| error.message)?;
    if !root.registers("CommonAttribute", name) {
        return Err("CommonAttribute is not registered in its source set".into());
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> String {
        include_str!("../../../tests/fixtures/acceptance/workspace-common-attribute/src/CommonAttributes/ОбластьДанных.xml")
            .trim_start_matches('\u{feff}').to_string()
    }

    #[test]
    fn root_patch_preserves_row_settings_unknown_fields_and_semantic_noops() {
        let before = source().replace("<Content/>", "<Content><xr:Item><xr:Metadata>Catalog.Kept</xr:Metadata><xr:Use>DontUse</xr:Use><xr:ConditionalSeparation>Constant.RowCondition</xr:ConditionalSeparation></xr:Item></Content><Future xmlns=\"urn:future\">kept &amp; safe</Future>");
        for (field, new) in [
            (DescriptorProperty::AutoUse, "Use"),
            (
                DescriptorProperty::ConditionalSeparation,
                "Constant.RootCondition",
            ),
        ] {
            let changes = CommonAttributePropertyChanges {
                changes: vec![(field, new.into())],
            };
            let after = patch(&before, "ОбластьДанных", &changes).unwrap();
            let old = if field == DescriptorProperty::AutoUse {
                "<AutoUse>DontUse</AutoUse>"
            } else {
                "<ConditionalSeparation/>"
            };
            let tag = field.names()[0];
            assert_eq!(
                after,
                before.replacen(old, &format!("<{tag}>{new}</{tag}>"), 1)
            );
            assert_eq!(patch(&after, "ОбластьДанных", &changes).unwrap(), after);
        }
        let before = before.replace(
            "<AutoUse>DontUse</AutoUse>",
            "<AutoUse>Dont<!-- retained -->Use</AutoUse>",
        );
        let same = CommonAttributePropertyChanges {
            changes: vec![
                (DescriptorProperty::AutoUse, "DontUse".into()),
                (DescriptorProperty::DataSeparationValue, "".into()),
            ],
        };
        assert_eq!(patch(&before, "ОбластьДанных", &same).unwrap(), before);
    }

    #[test]
    fn root_patch_refuses_duplicate_foreign_nested_and_missing_fields() {
        let changes = CommonAttributePropertyChanges {
            changes: vec![(DescriptorProperty::AutoUse, "Use".into())],
        };
        for bad in [
            source().replace(
                "<AutoUse>DontUse</AutoUse>",
                "<AutoUse>DontUse</AutoUse><AutoUse>Use</AutoUse>",
            ),
            source().replace("<AutoUse>", "<AutoUse xmlns=\"urn:foreign\">"),
            source().replace("<AutoUse>DontUse</AutoUse>", "<AutoUse><Nested/></AutoUse>"),
            source().replace("<AutoUse>DontUse</AutoUse>", ""),
            source().replace("<AutoUse>", "<AutoUse invalid=\"true\">"),
            source().replace("<AutoUse>DontUse</AutoUse>", "<AutoUse>Unknown</AutoUse>"),
        ] {
            assert!(patch(&bad, "ОбластьДанных", &changes).is_err(), "{bad}");
        }
    }
}
