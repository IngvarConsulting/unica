//! SettingsStorage form references: closed observations and retained-input edits.
use roxmltree::{Document, Node};
use std::path::Path;
use unica_address::source_target::MetadataAddress;
use unica_forms::storage_defaults::{StorageFormAssignment, StorageFormSlot};
use unica_metadata::{MetaObservedProperty, MetaObservedPropertyValue};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_designer::format_profile::writable_profile_refusal;
use unica_source_designer::owner::{prove_already_read_source_set_owner, MD_CLASSES_NS};
use unica_source_designer::registration::{
    owner_descriptor_relative, preserve_descriptor_image, read_required, utf8_body,
};
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
fn non_nil(node: Node<'_, '_>) -> Result<(), String> {
    if node
        .attribute((XSI, "nil"))
        .is_some_and(|v| !matches!(v.trim_matches([' ', '\t', '\r', '\n']), "false" | "0"))
    {
        return Err("nil XML cannot prove a storage form assignment".into());
    }
    Ok(())
}
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Result<Node<'a, 'input>, String> {
    let items = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .collect::<Vec<_>>();
    let [item] = items.as_slice() else {
        return Err(format!("expected one direct {name}"));
    };
    if item.tag_name().namespace() != Some(MD_CLASSES_NS) {
        return Err(format!("{name} has a foreign namespace"));
    }
    non_nil(*item)?;
    Ok(*item)
}
fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    non_nil(node)?;
    if node.children().any(|n| n.is_element()) {
        return Err("expected a scalar metadata reference".into());
    }
    Ok(unica_xml_format::read::direct_text(node)
        .trim_matches([' ', '\t', '\r', '\n'])
        .to_string())
}
fn owner<'a, 'input>(
    doc: &'a Document<'input>,
    kind: &str,
    name: &str,
    write: bool,
) -> Result<Node<'a, 'input>, String> {
    let root = doc.root_element();
    non_nil(root)?;
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err("invalid metadata descriptor root".into());
    }
    let artifacts = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [object] = artifacts.as_slice() else {
        return Err("metadata descriptor must contain one owner".into());
    };
    if !object.has_tag_name((MD_CLASSES_NS, kind)) {
        return Err("metadata descriptor kind does not match its owner".into());
    }
    non_nil(*object)?;
    if write
        && !object
            .attribute("uuid")
            .is_some_and(unica_xml_format::names::is_valid_uuid)
    {
        return Err("metadata owner UUID is missing or invalid".into());
    }
    let props = child(*object, "Properties")?;
    if scalar(child(props, "Name")?)? != name {
        return Err("metadata descriptor Name does not match its owner".into());
    }
    Ok(*object)
}
#[derive(Default)]
pub struct StorageDefaultsFacts {
    pub properties: Vec<MetaObservedProperty>,
    pub unreadable: Vec<(String, String)>,
}
pub fn read(bytes: &[u8], name: &str) -> Result<StorageDefaultsFacts, String> {
    let source = std::str::from_utf8(bytes)
        .map_err(|e| e.to_string())?
        .trim_start_matches('\u{feff}');
    let doc = Document::parse(source).map_err(|e| e.to_string())?;
    let object = owner(&doc, "SettingsStorage", name, false)?;
    let props = child(object, "Properties")?;
    let mut result = StorageDefaultsFacts::default();
    for slot in StorageFormSlot::ALL {
        match child(props, slot.property()).and_then(scalar) {
            Ok(value) => result.properties.push(MetaObservedProperty {
                name: slot.property().to_string(),
                value: MetaObservedPropertyValue::Text { value },
            }),
            Err(message) => result
                .unreadable
                .push((format!("properties.{}", slot.property()), message)),
        }
    }
    Ok(result)
}
fn registered(object: Node<'_, '_>, kind: &str, name: &str) -> Result<bool, String> {
    if !object
        .children()
        .any(|node| node.is_element() && node.tag_name().name() == "ChildObjects")
    {
        return Ok(false);
    }
    let list = child(object, "ChildObjects")?;
    for item in list
        .children()
        .filter(|n| n.has_tag_name((MD_CLASSES_NS, kind)))
    {
        if unica_xml_format::read::direct_text(item).trim_matches([' ', '\t', '\r', '\n']) == name {
            scalar(item)?;
            return Ok(true);
        }
    }
    Ok(false)
}
fn replace_slot(
    source: &str,
    props: Node<'_, '_>,
    slot: StorageFormSlot,
    value: &str,
) -> Result<String, String> {
    let node = child(props, slot.property())?;
    if scalar(node)? == value {
        return Ok(source.to_string());
    }
    let range = node.range();
    let end = crate::root::form_edit_opening_tag_end(source, range.start)
        .ok_or_else(|| "invalid form slot opening tag".to_string())?;
    let open = &source[range.start..=end];
    let value = unica_xml_format::text::escape_xml_text(value);
    let mut result = source.to_string();
    if let Some(prefix) = open.strip_suffix("/>") {
        let name = open[1..]
            .split([' ', '\t', '\r', '\n', '/', '>'])
            .next()
            .ok_or_else(|| "missing slot name".to_string())?;
        result.replace_range(range, &format!("{}>{value}</{name}>", prefix.trim_end()));
    } else {
        let close = source[range.clone()]
            .rfind("</")
            .ok_or_else(|| "missing form slot closing tag".to_string())?
            + range.start;
        result.replace_range(end + 1..close, &value);
    }
    Ok(result)
}
fn version(source: &str, path: &Path) -> Result<(), String> {
    let doc = Document::parse(source).map_err(|e| e.to_string())?;
    let literal = unica_source_designer::roots::root_version_literal(source, doc.root_element());
    if let Some(refusal) = writable_profile_refusal(path.portable(), literal.as_deref()) {
        return Err(refusal.message);
    }
    Ok(())
}
pub fn stage(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    target: &MetadataAddress,
    assignments: &[StorageFormAssignment],
    op_index: usize,
    effects: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let location = format!("ops[{op_index}].args.values");
    let invalid = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(location.clone())
    };
    let missing = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::NotFound, message).at_path(location.clone())
    };
    let parts = target.segments().collect::<Vec<_>>();
    let ["SettingsStorage", name] = parts.as_slice() else {
        return Err(invalid("a named SettingsStorage owner is required".into()));
    };
    let name = *name;
    let parent_path = Path::new("Configuration.xml");
    let parent = read_required(
        staged,
        parent_path,
        "the configuration descriptor",
        &location,
    )?;
    let evidence =
        prove_already_read_source_set_owner(parent_path, &parent, authority.source_kind())
            .map_err(|e| invalid(e.message))?;
    if let Some(refusal) = writable_profile_refusal(parent_path.portable(), evidence.version()) {
        return Err(invalid(refusal.message));
    }
    let parent_source = utf8_body(&parent, "the configuration descriptor", &location)?;
    let parent_doc = Document::parse(&parent_source).map_err(|e| invalid(e.to_string()))?;
    non_nil(parent_doc.root_element()).map_err(invalid)?;
    let parent_object =
        child(parent_doc.root_element(), evidence.artifact_kind()).map_err(invalid)?;
    if !registered(parent_object, "SettingsStorage", name).map_err(invalid)? {
        return Err(missing(
            "SettingsStorage is not registered in this source set".into(),
        ));
    }
    let relative = owner_descriptor_relative(target, authority, op_index)?;
    let before = read_required(
        staged,
        &relative,
        "the SettingsStorage descriptor",
        &location,
    )?;
    let source = utf8_body(&before, "the SettingsStorage descriptor", &location)?;
    version(&source, &relative).map_err(invalid)?;
    let doc = Document::parse(&source).map_err(|e| invalid(e.to_string()))?;
    let object = owner(&doc, "SettingsStorage", name, true).map_err(invalid)?;
    let mut updated = source.clone();
    for assignment in assignments {
        let reference = if let Some(form) = &assignment.form {
            let parts = form.segments().collect::<Vec<_>>();
            let ["SettingsStorage", form_owner, "Form", form_name] = parts.as_slice() else {
                return Err(invalid("invalid owned storage form address".into()));
            };
            if *form_owner != name {
                return Err(invalid(
                    "form owner differs from the SettingsStorage".into(),
                ));
            }
            if !registered(object, "Form", form_name).map_err(invalid)? {
                return Err(missing(
                    "the selected form is not registered by this SettingsStorage".into(),
                ));
            }
            let form_relative = owner_descriptor_relative(form, authority, op_index)?;
            let descriptor = read_required(
                staged,
                &form_relative,
                "the storage form descriptor",
                &location,
            )?;
            let text = utf8_body(&descriptor, "the storage form descriptor", &location)?;
            version(&text, &form_relative).map_err(invalid)?;
            let doc = Document::parse(&text).map_err(|e| invalid(e.to_string()))?;
            let form_owner = owner(&doc, "Form", form_name, true).map_err(invalid)?;
            if scalar(
                child(
                    child(form_owner, "Properties").map_err(invalid)?,
                    "FormType",
                )
                .map_err(invalid)?,
            )
            .map_err(invalid)?
                != "Managed"
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    "only existing Managed storage forms are supported",
                )
                .at_path(location.clone()));
            }
            let content_relative = form_relative.with_extension("").join("Ext/Form.xml");
            let content = read_required(
                staged,
                &content_relative,
                "the storage form content",
                &location,
            )?;
            let content = utf8_body(&content, "the storage form content", &location)?;
            let doc = Document::parse(&content).map_err(|e| invalid(e.to_string()))?;
            crate::root::require_form_root(doc.root_element()).map_err(invalid)?;
            non_nil(doc.root_element()).map_err(invalid)?;
            version(&content, &content_relative).map_err(invalid)?;
            form.as_str()
        } else {
            ""
        };
        let current = Document::parse(&updated).map_err(|e| invalid(e.to_string()))?;
        let current_owner = owner(&current, "SettingsStorage", name, true).map_err(invalid)?;
        let props = child(current_owner, "Properties").map_err(invalid)?;
        updated = replace_slot(&updated, props, assignment.slot, reference).map_err(invalid)?;
    }
    let checked = Document::parse(&updated).map_err(|e| invalid(e.to_string()))?;
    let props = child(
        owner(&checked, "SettingsStorage", name, true).map_err(invalid)?,
        "Properties",
    )
    .map_err(invalid)?;
    for assignment in assignments {
        let wanted = assignment.form.as_ref().map_or("", MetadataAddress::as_str);
        if scalar(child(props, assignment.slot.property()).map_err(invalid)?).map_err(invalid)?
            != wanted
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::Postcondition,
                "storage form slot postcondition failed",
            )
            .at_path(location.clone()));
        }
    }
    let after = preserve_descriptor_image(&before, &source, &updated);
    if after == before {
        return Ok(());
    }
    staged
        .replace(&relative, &before, after)
        .map_err(|e| ApplyPlanError::staging(e, location))?;
    effects.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::MetadataChanged,
            target.as_str().to_string(),
        ),
        op_index,
    ));
    Ok(())
}
