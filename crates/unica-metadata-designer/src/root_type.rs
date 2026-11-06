//! Structural Type observations and final dependencies of root Type edits.

use crate::decode::{DescriptorPropertyChanges, MetadataPlanKind, MetadataPlanOperation};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::{
    MetaEditOperation, MetaPropertyKey, ObservedMetadataType, ObservedMetadataTypeVariant,
};
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, IndexedPlanOperation,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::metadata_descriptor_relative;
use unica_source_designer::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
};

pub(crate) fn trim_xml_space(value: &str) -> &str {
    value.trim_matches([' ', '\t', '\r', '\n'])
}

pub(crate) fn has_nil_type_context(properties: roxmltree::Node<'_, '_>) -> bool {
    properties.ancestors().any(|node| {
        node.attribute(("http://www.w3.org/2001/XMLSchema-instance", "nil"))
            .is_some_and(|value| !matches!(trim_xml_space(value), "false" | "0"))
    })
}

pub fn observe(raw: &[u8]) -> Result<ObservedMetadataType, String> {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    let text = std::str::from_utf8(raw).map_err(|_| "metadata descriptor is not UTF-8")?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|_| "metadata descriptor is not valid XML")?;
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("metadata descriptor root has an unexpected namespace or kind".into());
    }
    let objects = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    let [object] = objects.as_slice() else {
        return Err("metadata descriptor must contain one object".into());
    };
    let properties = object
        .children()
        .filter(|node| node.has_tag_name((MD, "Properties")))
        .collect::<Vec<_>>();
    let [properties] = properties.as_slice() else {
        return Err("metadata descriptor must contain one Properties".into());
    };
    if has_nil_type_context(*properties) {
        return Err("Type is inside a nil or invalid nil context".into());
    }
    crate::info_projection::parse_observed_metadata_type_node(*properties).map_err(|diagnostic| {
        let code = if crate::info_projection::observed_type_is_strict_but_unmodelled(*properties) {
            "validation_warning"
        } else {
            "validation_failed"
        };
        format!("{code}: {}", diagnostic.message)
    })
}

/// Identity has already been proved by the caller. A damaged Type does not
/// replace other observed facts, including a SessionParameter's Comment.
pub fn enrich(raw: &[u8], payload: &mut Value) {
    match observe(raw) {
        Ok(metadata_type) => payload["details"]["details"]["type"] = json!(metadata_type),
        Err(message) => {
            let mut unreadable = payload["unreadable"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            unreadable.push(json!({"field":"properties.Type", "message":message}));
            payload["unreadable"] = json!(unreadable);
        }
    }
}

fn invalid(index: usize, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
        .at_path(format!("ops[{index}].args.values.Type"))
}

fn validate_active_fill(
    raw: &[u8],
    metadata_type: &ObservedMetadataType,
    index: usize,
) -> Result<(), ApplyPlanError> {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    let text =
        std::str::from_utf8(raw).map_err(|_| invalid(index, "CommonAttribute is not UTF-8"))?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|_| invalid(index, "CommonAttribute is not valid XML"))?;
    let properties = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "CommonAttribute")))
        .and_then(|object| {
            object
                .children()
                .find(|node| node.has_tag_name((MD, "Properties")))
        })
        .ok_or_else(|| invalid(index, "CommonAttribute Properties are unavailable"))?;
    let flags = properties
        .children()
        .filter(|node| node.has_tag_name((MD, "FillFromFillingValue")))
        .collect::<Vec<_>>();
    let [flag] = flags.as_slice() else {
        return Err(invalid(
            index,
            "CommonAttribute FillFromFillingValue must be explicit and unique",
        ));
    };
    if flag.attributes().len() != 0 || flag.children().any(|child| child.is_element()) {
        return Err(invalid(
            index,
            "CommonAttribute FillFromFillingValue is malformed",
        ));
    }
    match trim_xml_space(&crate::info_projection::direct_text_content(*flag)) {
        "false" => return Ok(()),
        "true" => {}
        _ => {
            return Err(invalid(
                index,
                "CommonAttribute FillFromFillingValue must be true or false",
            ))
        }
    }
    let fill_value = crate::edit::parse_typed_fill_value_node(properties)
        .map_err(|diagnostic| invalid(index, diagnostic.message))?;
    let metadata_type = unica_metadata::MetadataType::try_from(metadata_type.clone())
        .map_err(|diagnostic| invalid(index, diagnostic.message))?;
    unica_metadata::validate_metadata_element_value_profile(
        Some(&metadata_type),
        fill_value.as_ref(),
        unica_metadata::MetaValueProfileContext::Patch,
    )
    .map_err(|diagnostic| {
        invalid(
            index,
            format!(
                "Type must preserve the active CommonAttribute FillValue: {}",
                diagnostic.message
            ),
        )
    })
}

fn validate_borrowed_attribute_type(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    target: &MetadataAddress,
    bytes: &[u8],
    metadata_type: &ObservedMetadataType,
    index: usize,
) -> Result<(), ApplyPlanError> {
    if source_kind != SourceSetKind::Extension {
        return Ok(());
    }
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid(index, "borrowed CommonAttribute is not UTF-8"))?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|_| invalid(index, "borrowed CommonAttribute is not valid XML"))?;
    let properties = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "CommonAttribute")))
        .and_then(|object| {
            object
                .children()
                .find(|node| node.has_tag_name((MD, "Properties")))
        })
        .ok_or_else(|| invalid(index, "borrowed CommonAttribute Properties are unavailable"))?;
    let parents = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "ExtendedConfigurationObject")
        .collect::<Vec<_>>();
    if parents.is_empty() {
        return Ok(());
    }
    if !matches!(parents.as_slice(), [parent] if parent.has_tag_name((MD, "ExtendedConfigurationObject"))
        && parent.attributes().len() == 0 && !parent.children().any(|child| child.is_element())
        && uuid::Uuid::parse_str(trim_xml_space(&crate::info_projection::direct_text_content(*parent))).is_ok())
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "CommonAttribute ExtendedConfigurationObject must be one well-formed UUID",
        )
        .at_path(format!("ops[{index}].args.at")));
    }
    let relative = metadata_descriptor_relative(target, source_kind)
        .map_err(|message| invalid(index, message))?;
    let Some(change) = staged
        .planned_changes()
        .into_iter()
        .find(|change| change.relative_path == relative)
    else {
        return Ok(());
    };
    if let unica_source_contract::proposal::StagedFileState::Bytes(original) = change.original {
        if observe(&original).as_ref().ok() == Some(metadata_type) {
            return Ok(());
        }
    }
    let modes = properties
        .children()
        .filter(|node| node.has_tag_name((MD, "DataSeparation")))
        .collect::<Vec<_>>();
    let [mode] = modes.as_slice() else {
        return Err(invalid(
            index,
            "cannot determine DataSeparation of the borrowed CommonAttribute",
        ));
    };
    if mode.attributes().len() != 0 || mode.children().any(|child| child.is_element()) {
        return Err(invalid(
            index,
            "borrowed CommonAttribute DataSeparation is malformed",
        ));
    }
    match trim_xml_space(&crate::info_projection::direct_text_content(*mode)) {
        "DontUse" => Ok(()),
        "Separate" => Err(invalid(
            index,
            "the Type of a borrowed data separator cannot be changed",
        )),
        _ => Err(invalid(
            index,
            "cannot determine DataSeparation of the borrowed CommonAttribute",
        )),
    }
}

fn registered_descriptor(
    staged: &mut ApplyProposal<'_>,
    target: &MetadataAddress,
    source_kind: SourceSetKind,
    index: usize,
) -> Result<Vec<u8>, ApplyPlanError> {
    let path = format!("ops[{index}].args.values.Type");
    staged
        .checkpoint("root Type dependency")
        .map_err(|error| ApplyPlanError::staging(error, &path))?;
    let read = |staged: &mut ApplyProposal<'_>, relative: &Path| {
        staged
            .read(relative)
            .map_err(|error| ApplyPlanError::staging(error, &path))?
            .ok_or_else(|| {
                invalid(
                    index,
                    format!("Type dependency `{}` is missing", relative.portable()),
                )
            })
    };
    let root_path = Path::new("Configuration.xml");
    let root_bytes = read(staged, root_path)?;
    let root = prove_already_read_source_set_owner(root_path, &root_bytes, source_kind)
        .map_err(|error| invalid(index, error.message))?;
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [kind, name] = parts.as_slice() else {
        return Err(invalid(
            index,
            "Type dependency must name a top-level object",
        ));
    };
    // The general owner evidence deduplicates registrations for navigation.
    // A Type dependency must additionally prove a unique raw declaration.
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    let text = std::str::from_utf8(&root_bytes)
        .map_err(|_| invalid(index, "Configuration.xml is not UTF-8"))?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|_| invalid(index, "Configuration.xml is not valid XML"))?;
    let count = document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name((MD, "Configuration")))
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((MD, "ChildObjects")))
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((MD, *kind)))
        .filter(|node| {
            unica_source_designer::owner::registered_child_name(*node).as_deref() == Some(*name)
        })
        .count();
    if !root.registers(kind, name) || count != 1 {
        return Err(invalid(
            index,
            format!("Type dependency `{target}` is not registered"),
        ));
    }
    let relative = metadata_descriptor_relative(target, source_kind)
        .map_err(|message| invalid(index, message))?;
    let bytes = read(staged, &relative)?;
    let evidence = prove_already_read_metadata_owner(&relative, &bytes)
        .map_err(|error| invalid(index, error.message))?;
    if evidence.artifact_kind() != *kind
        || evidence.artifact_name() != Some(*name)
        || evidence.version() != root.version()
    {
        return Err(invalid(
            index,
            format!("Type dependency `{target}` does not match its registered descriptor"),
        ));
    }
    Ok(bytes)
}

/// Run after every operation family has staged its changes. Only surviving
/// root Type edits are roots of traversal; reachable existing DefinedTypes
/// participate too. Reads join the saved token's input dependencies.
pub fn validate_final(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    operations: &[IndexedPlanOperation<MetadataPlanOperation>],
) -> Result<(), ApplyPlanError> {
    let mut roots = BTreeMap::new();
    for operation in operations {
        let index = operation.index();
        match &operation.operation().kind {
            MetadataPlanKind::DescriptorProperties {
                name,
                changes: DescriptorPropertyChanges::RootType { owner, .. },
                ..
            } => {
                let target = format!("{}.{name}", owner.node_kind().as_str());
                roots.insert(target, index);
            }
            MetadataPlanKind::Edit {
                target,
                operation: MetaEditOperation::SetProperties { values },
                ..
            } if target.as_str().starts_with("DefinedType.")
                && values
                    .entries()
                    .iter()
                    .any(|(key, _)| *key == MetaPropertyKey::Type) =>
            {
                roots.insert(target.to_string(), index);
            }
            MetadataPlanKind::Remove { target, .. } => {
                roots.remove(target.as_str());
            }
            MetadataPlanKind::Create { kind, name } => {
                roots.remove(&format!("{}.{name}", kind.as_str()));
            }
            _ => {}
        }
    }
    let mut roots = roots.into_iter().collect::<Vec<_>>();
    roots.sort_by_key(|(_, index)| *index);
    for (root, index) in roots {
        let root = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &root)
            .map_err(|error| invalid(index, error.to_string()))?;
        let mut active = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut stack = vec![(root, false)];
        while let Some((target, exit)) = stack.pop() {
            let key = target.to_string();
            if exit {
                active.remove(&key);
                visited.insert(key);
                continue;
            }
            if visited.contains(&key) {
                continue;
            }
            if !active.insert(key.clone()) {
                return Err(invalid(
                    index,
                    format!("DefinedType cycle contains `{target}`"),
                ));
            }
            let bytes = registered_descriptor(staged, &target, source_kind, index)?;
            let metadata_type = observe(&bytes).map_err(|message| invalid(index, message))?;
            if key.starts_with("CommonAttribute.") {
                validate_borrowed_attribute_type(
                    staged,
                    source_kind,
                    &target,
                    &bytes,
                    &metadata_type,
                    index,
                )?;
                validate_active_fill(&bytes, &metadata_type, index)?;
            }
            stack.push((target, true));
            let mut definitions = Vec::new();
            for variant in metadata_type.variants {
                match variant {
                    ObservedMetadataTypeVariant::DefinedType { metadata_path } => {
                        definitions.push(metadata_path)
                    }
                    ObservedMetadataTypeVariant::Reference { metadata_path }
                    | ObservedMetadataTypeVariant::CharacteristicTypes { metadata_path } => {
                        registered_descriptor(staged, &metadata_path, source_kind, index)?;
                    }
                    _ => {}
                }
            }
            stack.extend(definitions.into_iter().rev().map(|target| (target, false)));
        }
    }
    Ok(())
}
