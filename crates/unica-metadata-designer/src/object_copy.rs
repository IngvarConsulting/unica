//! Independent Catalog copies, planned entirely through the retained source proposal.
use roxmltree::{Document, Node};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_designer::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner, MD_CLASSES_NS,
};
use unica_source_designer::registration::{
    owner_registration_image, require_untouched_staged_state,
};

const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
const V8: &str = "http://v8.1c.ru/8.1/data/core";
const CFG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub(crate) fn stage_copy(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    source: &str,
    name: &str,
    op_index: usize,
    effects: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at = format!("ops[{op_index}].args.at");
    let name_at = format!("ops[{op_index}].args.values.name");
    require_untouched_staged_state(staged, "object.copy", &at)?;
    let invalid = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(&at)
    };
    let conflict = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message).at_path(&name_at)
    };
    let root = Path::new("Configuration.xml");
    let owner = required_read(staged, root, &at)?;
    let evidence = prove_already_read_source_set_owner(root, &owner, authority.source_kind())
        .map_err(|error| invalid(error.message))?;
    require_profile(root, evidence.version()).map_err(invalid)?;
    let (_, document) = parse_xml(&owner).map_err(invalid)?;
    let registrations = document
        .descendants()
        .filter(|node| {
            node.has_tag_name((MD_CLASSES_NS, "Catalog"))
                && node
                    .parent_element()
                    .is_some_and(|parent| parent.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
        })
        .collect::<Vec<_>>();
    if registrations
        .iter()
        .filter(|node| node.text() == Some(source))
        .count()
        != 1
    {
        return Err(invalid(format!(
            "Catalog.{source} must be registered exactly once"
        )));
    }
    if registrations.iter().any(|node| {
        node.text()
            .is_some_and(|text| text.to_lowercase() == name.to_lowercase())
    }) {
        return Err(conflict(format!(
            "the configuration already registers Catalog.{name}"
        )));
    }
    let destination = PathBuf::from(format!("Catalogs/{name}.xml"));
    if staged
        .read(&destination)
        .map_err(|error| ApplyPlanError::staging(error, &name_at))?
        .is_some()
    {
        return Err(conflict(format!("Catalog.{name} already exists")));
    }
    let destination_dir = PathBuf::from(format!("Catalogs/{name}"));
    let (_, absent) = staged
        .enumerate_tree_with_root_absence(&destination_dir)
        .map_err(|error| ApplyPlanError::staging(error, &name_at))?;
    if !absent {
        return Err(conflict(format!(
            "the resource directory of Catalog.{name} already exists"
        )));
    }

    let descriptor = PathBuf::from(format!("Catalogs/{source}.xml"));
    let source_dir = PathBuf::from(format!("Catalogs/{source}"));
    let mut files = BTreeMap::new();
    files.insert(descriptor.clone(), required_read(staged, &descriptor, &at)?);
    for path in staged
        .enumerate_tree(&source_dir)
        .map_err(|error| ApplyPlanError::staging(error, &at))?
    {
        files.insert(path.clone(), required_read(staged, &path, &at)?);
    }
    let directories = staged
        .enumerate_directories(&source_dir)
        .map_err(|error| ApplyPlanError::staging(error, &at))?;
    let mut identities = BTreeMap::new();
    for (path, bytes) in &files {
        staged
            .checkpoint("copy metadata resources")
            .map_err(|error| ApplyPlanError::staging(error, &at))?;
        if path.extension().and_then(|ext| ext.to_str()) != Some("xml") {
            continue;
        }
        let (_, document) =
            parse_xml(bytes).map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        use unica_source_designer::roots::{
            platform_xml_publication_policy, root_version_literal, PlatformXmlPublicationPolicy,
        };
        let xml_root = document.root_element();
        let raw_version = root_version_literal(document.input_text(), xml_root);
        match xml_root.tag_name().namespace().and_then(|namespace| {
            platform_xml_publication_policy(namespace, xml_root.tag_name().name())
        }) {
            Some(PlatformXmlPublicationPolicy::ExactRootVersion) => {
                require_profile(path, raw_version.as_deref()).map_err(invalid)?
            }
            Some(PlatformXmlPublicationPolicy::Versionless) if raw_version.is_some() => {
                return Err(invalid(format!(
                    "{} has a versionless XML root and must not declare version",
                    path.display()
                )))
            }
            _ => {}
        }
        let descriptor_kind = if path == &descriptor {
            Some(("Catalog", source))
        } else if path.parent() == Some(source_dir.join("Forms").as_path()) {
            Some((
                "Form",
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
            ))
        } else if path.parent() == Some(source_dir.join("Templates").as_path()) {
            Some((
                "Template",
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
            ))
        } else {
            None
        };
        if let Some((kind, expected_name)) = descriptor_kind {
            let evidence = prove_already_read_metadata_owner(path, bytes)
                .map_err(|error| invalid(error.message))?;
            require_profile(path, evidence.version()).map_err(invalid)?;
            if evidence.artifact_kind() != kind || evidence.artifact_name() != Some(expected_name) {
                return Err(invalid(format!(
                    "{} does not describe {kind}.{expected_name}",
                    path.display()
                )));
            }
            let owner = document
                .root_element()
                .children()
                .find(|node| node.is_element())
                .unwrap();
            validate_registered_resources(owner, path, &files).map_err(invalid)?;
            validate_required_payload(owner, kind, path, &files).map_err(invalid)?;
            collect_identities(&document, source, &mut identities).map_err(invalid)?;
        } else if path.starts_with(source_dir.join("Forms"))
            && path.file_name().is_some_and(|file| file == "Form.xml")
        {
            let root = document.root_element();
            if !root.has_tag_name(("http://v8.1c.ru/8.3/xcf/logform", "Form")) {
                return Err(invalid(format!(
                    "{} must contain a managed Form root with its exact namespace",
                    path.display()
                )));
            }
        }
    }
    for directory in directories {
        let target = destination_dir.join(
            directory
                .strip_prefix(&source_dir)
                .expect("enumerated source directory"),
        );
        staged
            .create_directory(&target)
            .map_err(|error| ApplyPlanError::staging(error, &name_at))?;
    }
    // Effects reconcile file diffs; directory intentions are guarded separately.
    let mut touched = Vec::new();
    for (path, bytes) in files {
        let target = if path == descriptor {
            destination.clone()
        } else {
            destination_dir.join(
                path.strip_prefix(&source_dir)
                    .expect("enumerated source subtree"),
            )
        };
        let image = if path.extension().and_then(|ext| ext.to_str()) == Some("xml") {
            copy_xml(&bytes, source, name, path == descriptor, &identities).map_err(invalid)?
        } else {
            bytes
        };
        staged
            .create(&target, image)
            .map_err(|error| ApplyPlanError::staging(error, &name_at))?;
        touched.push(target);
    }
    let updated =
        owner_registration_image(&owner, "Catalog", name, true, op_index)?.ok_or_else(|| {
            conflict(format!(
                "the configuration already registers Catalog.{name}"
            ))
        })?;
    staged
        .replace(root, &owner, updated)
        .map_err(|error| ApplyPlanError::staging(error, &at))?;
    touched.push(root.to_owned());
    effects.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(DomainEventKind::MetadataChanged, format!("Catalog.{name}")),
        op_index,
    ));
    Ok(())
}

fn required_read(
    staged: &mut ApplyProposal<'_>,
    path: &Path,
    at: &str,
) -> Result<Vec<u8>, ApplyPlanError> {
    staged
        .read(path)
        .map_err(|error| ApplyPlanError::staging(error, at))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("{} was not found", path.display()),
            )
            .at_path(at)
        })
}

fn parse_xml(bytes: &[u8]) -> Result<(&str, Document<'_>), String> {
    unica_xml_format::read::parse_metadata_image(bytes)
}

fn require_profile(path: &Path, raw: Option<&str>) -> Result<(), String> {
    match unica_source_designer::format_profile::writable_profile_refusal(path.display(), raw) {
        Some(refusal) => Err(refusal.message),
        None => Ok(()),
    }
}

fn validate_registered_resources(
    owner: Node<'_, '_>,
    path: &Path,
    files: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for child in owner
        .children()
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
        .flat_map(|node| node.children())
        .filter(|node| node.is_element())
    {
        let kind = child.tag_name().name();
        if !matches!(kind, "Form" | "Template") {
            continue;
        }
        let name = child.text().unwrap_or_default();
        if !unica_metadata::identifier::is_1c_identifier(name) || !seen.insert((kind, name)) {
            return Err(format!(
                "{} contains an invalid or duplicate {kind} registration",
                path.display()
            ));
        }
        let directory =
            path.with_extension("")
                .join(if kind == "Form" { "Forms" } else { "Templates" });
        if !files.contains_key(&directory.join(format!("{name}.xml"))) {
            return Err(format!("registered {kind}.{name} descriptor is absent"));
        }
    }
    Ok(())
}

fn validate_required_payload(
    owner: Node<'_, '_>,
    kind: &str,
    path: &Path,
    files: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), String> {
    use unica_metadata::validation_subject::{MetadataChildProfile, MetadataTemplateType};
    let Some(properties) = owner
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
    else {
        return Err("metadata descriptor has no Properties".to_owned());
    };
    let child_root = path.with_extension("");
    let required = match kind {
        "Form" => Some(match crate::validation::form_profile(properties)? {
            MetadataChildProfile::OrdinaryForm => crate::edit::TYPED_ORDINARY_FORM_CONTENT_PATH,
            _ => crate::edit::TYPED_FORM_CONTENT_PATH,
        }),
        "Template" => {
            let values = properties
                .children()
                .filter(|node| node.has_tag_name((MD_CLASSES_NS, "TemplateType")))
                .filter_map(|node| node.text())
                .collect::<Vec<_>>();
            let [value] = values.as_slice() else {
                return Err("template must declare exactly one TemplateType".to_owned());
            };
            let template_type = MetadataTemplateType::from_descriptor_value(value)
                .ok_or_else(|| "template has an unsupported TemplateType".to_owned())?;
            if template_type == MetadataTemplateType::BinaryData {
                None
            } else {
                Some(crate::edit::typed_template_primary_path(template_type))
            }
        }
        _ => None,
    };
    if let Some(required) = required {
        if !files.contains_key(&child_root.join(required)) {
            return Err(format!(
                "{} requires {}",
                path.display(),
                child_root.join(required).display()
            ));
        }
    }
    Ok(())
}

fn collect_identities(
    document: &Document<'_>,
    source: &str,
    identities: &mut BTreeMap<uuid::Uuid, String>,
) -> Result<(), String> {
    for node in document.descendants().filter(|node| node.is_element()) {
        if node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node
                .children()
                .any(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))
        {
            let value = node
                .attribute("uuid")
                .ok_or_else(|| format!("{} has no owned UUID", node.tag_name().name()))?;
            insert_identity(value, identities)?;
        }
        if node.has_tag_name((XR, "GeneratedType"))
            && node
                .parent_element()
                .is_some_and(|parent| parent.has_tag_name((MD_CLASSES_NS, "InternalInfo")))
        {
            if !node
                .attribute("name")
                .is_some_and(|name| renamed_path(name, source, "Copy").is_some())
            {
                return Err("an owned GeneratedType must name the source Catalog".to_owned());
            }
            for kind in ["TypeId", "ValueId"] {
                if node
                    .children()
                    .filter(|child| child.has_tag_name((XR, kind)))
                    .count()
                    != 1
                {
                    return Err(format!(
                        "an owned GeneratedType requires exactly one {kind}"
                    ));
                }
            }
            for child in node.children().filter(|child| {
                child.has_tag_name((XR, "TypeId")) || child.has_tag_name((XR, "ValueId"))
            }) {
                insert_identity(child.text().unwrap_or_default(), identities)?;
            }
        }
    }
    Ok(())
}

fn insert_identity(
    value: &str,
    identities: &mut BTreeMap<uuid::Uuid, String>,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| format!("invalid owned UUID {value}"))?;
    if id.is_nil()
        || identities
            .insert(id, uuid::Uuid::new_v4().to_string())
            .is_some()
    {
        return Err(format!("nil or duplicate owned UUID {value}"));
    }
    Ok(())
}

fn renamed_path(value: &str, source: &str, name: &str) -> Option<String> {
    let (kind, rest) = value.split_once('.')?;
    if !matches!(
        kind,
        "Catalog"
            | "CatalogObject"
            | "CatalogRef"
            | "CatalogSelection"
            | "CatalogList"
            | "CatalogManager"
            | "CatalogTabularSection"
            | "CatalogTabularSectionRow"
    ) {
        return None;
    }
    let suffix = rest.strip_prefix(source)?;
    if !suffix.is_empty() && !suffix.starts_with('.') {
        return None;
    }
    Some(format!("{kind}.{name}{suffix}"))
}

fn copy_xml(
    bytes: &[u8],
    source: &str,
    name: &str,
    root_descriptor: bool,
    identities: &BTreeMap<uuid::Uuid, String>,
) -> Result<Vec<u8>, String> {
    let (text, document) = parse_xml(bytes)?;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let artifact = document
        .root_element()
        .children()
        .find(|node| node.is_element());
    for node in document.descendants().filter(|node| node.is_element()) {
        for attribute in node.attributes() {
            let replacement = if attribute.name() == "uuid"
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && document
                    .root_element()
                    .has_tag_name((MD_CLASSES_NS, "MetaDataObject"))
            {
                remap_uuid(attribute.value(), identities)
            } else if (node.has_tag_name((XR, "GeneratedType")) && attribute.name() == "name")
                || (attribute.namespace() == Some(XSI) && attribute.name() == "type")
            {
                rename_reference(node, attribute.value(), source, name)
            } else {
                None
            };
            if let Some(value) = replacement {
                edits.push((
                    attribute.range_value(),
                    unica_xml_format::text::escape_xml(&value),
                ));
            }
        }
        let Some(child) = node.children().find(|child| child.is_text()) else {
            continue;
        };
        if node.children().any(|child| child.is_element()) {
            continue;
        }
        let value = node.text().unwrap_or_default();
        let replacement = if root_descriptor
            && node.has_tag_name((MD_CLASSES_NS, "Name"))
            && node
                .parent_element()
                .and_then(|parent| parent.parent_element())
                == artifact
        {
            Some(name.to_owned())
        } else if matches!(
            (node.tag_name().namespace(), node.tag_name().name()),
            (Some(XR | V8), "TypeId" | "ValueId")
        ) {
            remap_uuid(value, identities)
        } else if is_reference_node(node) {
            rename_reference(node, value, source, name)
        } else {
            None
        };
        if let Some(value) = replacement {
            edits.push((child.range(), unica_xml_format::text::escape_xml(&value)));
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut updated = text.to_owned();
    for (range, value) in edits.into_iter().rev() {
        updated.replace_range(range, &value);
    }
    let mut image = Vec::with_capacity(bytes.len());
    if bytes.starts_with(b"\xef\xbb\xbf") {
        image.extend_from_slice(b"\xef\xbb\xbf");
    }
    image.extend_from_slice(updated.as_bytes());
    Ok(image)
}

fn remap_uuid(value: &str, identities: &BTreeMap<uuid::Uuid, String>) -> Option<String> {
    identities.get(&uuid::Uuid::parse_str(value).ok()?).cloned()
}

fn rename_reference(node: Node<'_, '_>, value: &str, source: &str, name: &str) -> Option<String> {
    if let Some((prefix, path)) = value.split_once(':') {
        if node.lookup_namespace_uri(Some(prefix)) == Some(CFG) {
            return renamed_path(path, source, name).map(|path| format!("{prefix}:{path}"));
        }
        None
    } else {
        renamed_path(value, source, name)
    }
}

fn is_reference_node(node: Node<'_, '_>) -> bool {
    if node
        .attribute((XSI, "type"))
        .and_then(|kind| kind.split_once(':'))
        .is_some_and(|(prefix, _)| {
            node.lookup_namespace_uri(Some(prefix)) == Some("http://www.w3.org/2001/XMLSchema")
        })
    {
        return false;
    }
    match (node.tag_name().namespace(), node.tag_name().name()) {
        (Some(XR), "Field" | "ChoiceForm") | (Some(V8), "Type") => true,
        (
            Some(MD_CLASSES_NS),
            "DefaultObjectForm"
            | "DefaultFolderForm"
            | "DefaultListForm"
            | "DefaultChoiceForm"
            | "DefaultFolderChoiceForm"
            | "AuxiliaryChoiceForm"
            | "AuxiliaryFolderChoiceForm"
            | "ChoiceForm",
        ) => true,
        (Some(MD_CLASSES_NS), "Item") => node
            .attribute((XSI, "type"))
            .and_then(|kind| kind.split_once(':'))
            .is_some_and(|(prefix, kind)| {
                kind == "MDObjectRef" && node.lookup_namespace_uri(Some(prefix)) == Some(XR)
            }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_preserves_typed_string_values_that_look_like_metadata_references() {
        let original = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" xmlns:xsi="{XSI}" xmlns:xr="{XR}" xmlns:scalar="http://www.w3.org/2001/XMLSchema" version="2.20"><Catalog uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Source</Name><DefaultObjectForm>Catalog.Source.Form.Main</DefaultObjectForm><xr:ChoiceForm>Catalog.Source.Form.Choice</xr:ChoiceForm></Properties><ChildObjects><Attribute uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Value</Name><FillValue xsi:type="scalar:string">Catalog.Source</FillValue></Properties></Attribute><Attribute uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Other</Name><FillValue xsi:type="scalar:string">CatalogRef.Source</FillValue></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#
        );
        let copied = copy_xml(
            original.as_bytes(),
            "Source",
            "Copy",
            true,
            &BTreeMap::new(),
        )
        .unwrap();
        let copied = String::from_utf8(copied).unwrap();
        let doc = Document::parse(&copied).unwrap();
        let strings = doc
            .descendants()
            .filter(|node| node.has_tag_name((MD_CLASSES_NS, "FillValue")))
            .map(|node| node.text().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(strings, ["Catalog.Source", "CatalogRef.Source"]);
        assert!(copied.contains("Catalog.Copy.Form.Main"));
        assert!(copied.contains("Catalog.Copy.Form.Choice"));
    }
}
