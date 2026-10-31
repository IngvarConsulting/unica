//! The report's existing scalar reference; storage contents are not read here.
use roxmltree::{Document, Node};
use unica_address::source_target::MetadataAddress;
use unica_metadata::{report_variants_storage_target, MetaInfoPropertyValue, MetadataKind};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Shared by property edits and Relation observation. Empty is known empty;
/// missing, nil, duplicate, foreign and malformed values remain errors.
pub fn observe(properties: Node<'_, '_>) -> Result<Option<MetadataAddress>, String> {
    let value = crate::info_projection::observe_meta_info_property(
        properties,
        MetadataKind::Report,
        "VariantsStorage",
    )
    .map_err(|error| error.message)?
    .ok_or("Report has no VariantsStorage property")?;
    let MetaInfoPropertyValue::String(value) = value else {
        return Err("VariantsStorage must be a scalar reference".into());
    };
    if value.is_empty() {
        return Ok(None);
    }
    report_variants_storage_target(&value)
        .map(Some)
        .map_err(|error| error.message)
}

/// A writer proves the complete direct owner identity, rather than accepting
/// a first matching Name or Properties container from a permissive lookup.
pub(crate) fn owner_properties<'doc, 'input>(
    doc: &'doc Document<'input>,
    kind: &str,
    name: &str,
) -> Result<Node<'doc, 'input>, String> {
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("metadata owner has the wrong root".into());
    }
    let owners: Vec<_> = root.children().filter(Node::is_element).collect();
    let [owner] = owners.as_slice() else {
        return Err("metadata descriptor requires one direct owner".into());
    };
    if !owner.has_tag_name((MD, kind)) {
        return Err("metadata owner kind does not match its target".into());
    }
    let containers: Vec<_> = owner
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect();
    let [properties] = containers.as_slice() else {
        return Err("metadata owner requires one direct Properties container".into());
    };
    if !properties.has_tag_name((MD, "Properties")) {
        return Err("metadata Properties has the wrong namespace".into());
    }
    let names: Vec<_> = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Name")
        .collect();
    let [identity] = names.as_slice() else {
        return Err("metadata owner requires one exact scalar Name".into());
    };
    if !identity.has_tag_name((MD, "Name"))
        || identity.attributes().len() != 0
        || identity
            .children()
            .any(|node| node.is_element() || node.is_pi())
        || crate::info_projection::direct_text_content(*identity) != name
    {
        return Err("metadata owner requires one exact scalar Name".into());
    }
    Ok(*properties)
}

pub(crate) fn replacement(
    source: &str,
    property: Node<'_, '_>,
    value: &str,
) -> Result<String, String> {
    let fragment = &source[property.range()];
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

use std::path::Path;
use unica_metadata::{MetaEditOperation, MetaPropertyKey, MetaPropertyValue};
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;

fn invalid(message: impl Into<String>, at: &str) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at)
}
fn read(
    staged: &mut ApplyProposal<'_>,
    path: &Path,
    at: &str,
) -> Result<Option<Vec<u8>>, ApplyPlanError> {
    staged
        .read(path)
        .map_err(|error| ApplyPlanError::staging(error, at))
}
fn root(
    staged: &mut ApplyProposal<'_>,
    at: &str,
) -> Result<unica_source_designer::owner::PlatformXmlSourceSetOwnerEvidence, ApplyPlanError> {
    let path = Path::new("Configuration.xml");
    let bytes = read(staged, path, at)?
        .ok_or_else(|| invalid("configuration descriptor was not found", at))?;
    let root = unica_source_designer::owner::prove_already_read_source_set_owner(
        path,
        &bytes,
        SourceSetKind::Configuration,
    )
    .map_err(|error| invalid(error.message, at))?;
    if root.version() != Some("2.20") {
        return Err(invalid("VariantsStorage requires platform XML 2.20", at));
    }
    Ok(root)
}
fn owner_reference(
    report: &MetadataAddress,
    bytes: &[u8],
    at: &str,
) -> Result<Option<MetadataAddress>, ApplyPlanError> {
    let parts: Vec<_> = report.segments().collect();
    let ["Report", name] = parts.as_slice() else {
        return Err(invalid("VariantsStorage requires a Report owner", at));
    };
    let path = Path::new("Reports").join(format!("{name}.xml"));
    let evidence = unica_source_designer::owner::prove_already_read_metadata_owner(&path, bytes)
        .map_err(|error| invalid(error.message, at))?;
    if evidence.version() != Some("2.20")
        || evidence.artifact_kind() != "Report"
        || evidence.artifact_name() != Some(*name)
    {
        return Err(invalid(
            "Report descriptor does not match its target in format 2.20",
            at,
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|error| invalid(error.to_string(), at))?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| invalid(error.to_string(), at))?;
    let properties =
        owner_properties(&doc, "Report", name).map_err(|message| invalid(message, at))?;
    observe(properties).map_err(|message| invalid(message, at))
}
fn guard_storage(
    staged: &mut ApplyProposal<'_>,
    root: &unica_source_designer::owner::PlatformXmlSourceSetOwnerEvidence,
    target: &MetadataAddress,
    at: &str,
) -> Result<(), ApplyPlanError> {
    let parts: Vec<_> = target.segments().collect();
    let ["SettingsStorage", name] = parts.as_slice() else {
        return Err(invalid(
            "VariantsStorage target is not a SettingsStorage",
            at,
        ));
    };
    if !root.registers("SettingsStorage", name) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!("SettingsStorage `{name}` is not registered in this source set"),
        )
        .at_path(at));
    }
    let path = Path::new("SettingsStorages").join(format!("{name}.xml"));
    let bytes = read(staged, &path, at)?.ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!("SettingsStorage descriptor `{name}` was not found"),
        )
        .at_path(at)
    })?;
    let evidence = unica_source_designer::owner::prove_already_read_metadata_owner(&path, &bytes)
        .map_err(|error| invalid(error.message, at))?;
    if evidence.version() != Some("2.20")
        || evidence.artifact_kind() != "SettingsStorage"
        || evidence.artifact_name() != Some(*name)
    {
        return Err(invalid(
            "SettingsStorage descriptor does not match its registered target in format 2.20",
            at,
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|error| invalid(error.to_string(), at))?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| invalid(error.to_string(), at))?;
    owner_properties(&doc, "SettingsStorage", name).map_err(|message| invalid(message, at))?;
    Ok(())
}

pub(crate) fn guard_assignment(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    report: &MetadataAddress,
    bytes: &[u8],
    operation: &MetaEditOperation,
    index: usize,
) -> Result<(), ApplyPlanError> {
    let MetaEditOperation::SetProperties { values } = operation else {
        return Ok(());
    };
    let Some((_, MetaPropertyValue::String(value))) = values
        .entries()
        .iter()
        .find(|(key, _)| *key == MetaPropertyKey::VariantsStorage)
    else {
        return Ok(());
    };
    let at = format!("ops[{index}].args.values.VariantsStorage");
    if source_kind != SourceSetKind::Configuration {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "VariantsStorage editing requires a configuration source set",
        )
        .at_path(at));
    }
    let target = report_variants_storage_target(value).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(&at)
    })?;
    let root = root(staged, &at)?;
    let name = report
        .segments()
        .nth(1)
        .ok_or_else(|| invalid("Report name is absent", &at))?;
    if !root.registers("Report", name) {
        return Err(invalid("Report is not registered in this source set", &at));
    }
    owner_reference(report, bytes, &at)?;
    guard_storage(staged, &root, &target, &at)
}

/// All families have contributed their final images. Explicit intents include
/// no-ops and are grouped by owner, so A→B validates B once. Unrelated Report
/// edits do not acquire a new target-existence obligation.
pub fn validate_final<'a>(
    staged: &mut ApplyProposal<'_>,
    source_set_name: &str,
    source_kind: SourceSetKind,
    operations: impl Iterator<Item = &'a crate::decode::MetadataPlanOperation>,
) -> Result<(), ApplyPlanError> {
    let mut reports = std::collections::BTreeMap::new();
    for operation in operations {
        if let crate::decode::MetadataPlanKind::Edit {
            target,
            operation: MetaEditOperation::SetProperties { values },
            ..
        } = &operation.kind
        {
            if values
                .entries()
                .iter()
                .any(|(key, _)| *key == MetaPropertyKey::VariantsStorage)
            {
                reports.insert(target.as_str().to_owned(), target.clone());
            }
        }
    }
    if reports.is_empty() {
        return Ok(());
    }
    if source_kind != SourceSetKind::Configuration {
        return Err(invalid(
            "VariantsStorage editing requires a configuration source set",
            "ops",
        ));
    }
    for report in reports.values() {
        let at = format!(
            "{source_set_name}:{}.Props.VariantsStorage",
            report.as_str()
        );
        let name = report
            .segments()
            .nth(1)
            .ok_or_else(|| invalid("Report name is absent", &at))?;
        let root = root(staged, &at)?;
        let path = Path::new("Reports").join(format!("{name}.xml"));
        let bytes = read(staged, &path, &at)?;
        match bytes {
            None if !root.registers("Report", name) => continue,
            None => {
                return Err(invalid(
                    "registered Report descriptor is absent from the final state",
                    &at,
                ))
            }
            Some(_) if !root.registers("Report", name) => {
                return Err(invalid(
                    "Report descriptor is not registered in the final state",
                    &at,
                ))
            }
            Some(bytes) => {
                if let Some(target) = owner_reference(report, &bytes, &at)? {
                    guard_storage(staged, &root, &target, &at)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(field: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Report uuid=\"16970000-0000-4000-8000-000000000001\"><Properties><Name>Probe</Name>{field}<SettingsStorage>SettingsStorage.Other</SettingsStorage></Properties></Report></MetaDataObject>")
    }
    #[test]
    fn variant_reference_observation_distinguishes_empty_from_missing_or_damage() {
        let empty = source("<VariantsStorage/>");
        let doc = Document::parse(&empty).unwrap();
        let properties = owner_properties(&doc, "Report", "Probe").unwrap();
        assert_eq!(observe(properties).unwrap(), None);
        for field in ["","<VariantsStorage/><VariantsStorage/>","<VariantsStorage xmlns=\"urn:foreign\"/>","<VariantsStorage xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/>","<VariantsStorage><Nested/></VariantsStorage>","<VariantsStorage>Catalog.Other</VariantsStorage>"] {
            let xml=source(field);let doc=Document::parse(&xml).unwrap();let properties=owner_properties(&doc,"Report","Probe").unwrap();assert!(observe(properties).is_err(),"{field}");
        }
    }
    #[test]
    fn variant_reference_replacement_preserves_local_namespace_and_all_other_bytes() {
        let xml=source("<m:VariantsStorage xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">SettingsStorage.Old</m:VariantsStorage>");
        let doc = Document::parse(&xml).unwrap();
        let properties = owner_properties(&doc, "Report", "Probe").unwrap();
        let field = properties
            .children()
            .find(|n| n.has_tag_name((MD, "VariantsStorage")))
            .unwrap();
        let raw = &xml[field.range()];
        let updated = xml.replacen(
            raw,
            &replacement(&xml, field, "SettingsStorage.New").unwrap(),
            1,
        );
        let output = Document::parse(&updated).unwrap();
        let props = owner_properties(&output, "Report", "Probe").unwrap();
        assert_eq!(
            observe(props).unwrap().unwrap().as_str(),
            "SettingsStorage.New"
        );
        let field = props
            .children()
            .find(|n| n.has_tag_name((MD, "VariantsStorage")))
            .unwrap();
        assert_eq!(updated.replacen(&updated[field.range()], raw, 1), xml);
    }
}
