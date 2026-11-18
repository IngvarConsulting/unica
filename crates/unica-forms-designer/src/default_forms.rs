//! Evidence for the closed, platform-confirmed CommonForm assignment profile.
use crate::root::{require_form_root, FORM_LOGFORM_NS};
use roxmltree::{Document, Node};
use std::path::{Path, PathBuf};
use unica_address::source_target::MetadataAddress;
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority,
};
use unica_source_designer::format_profile::writable_profile_refusal;
use unica_source_designer::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner, MD_CLASSES_NS,
};
use unica_source_designer::registration::{owner_descriptor_relative, read_required, utf8_body};
const V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
const CFG_NS: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";

fn non_nil(node: Node<'_, '_>) -> Result<(), String> {
    if node
        .attribute(("http://www.w3.org/2001/XMLSchema-instance", "nil"))
        .is_some_and(|value| !matches!(value.trim_matches([' ', '\t', '\r', '\n']), "false" | "0"))
    {
        return Err("nil source node cannot prove a CommonForm assignment".into());
    }
    Ok(())
}

fn single<'a, 'input>(
    node: Node<'a, 'input>,
    ns: &str,
    name: &str,
) -> Result<Node<'a, 'input>, String> {
    let mut nodes = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let result = nodes.next().ok_or_else(|| format!("missing {name}"))?;
    non_nil(result)?;
    if nodes.next().is_some() || result.tag_name().namespace() != Some(ns) {
        return Err(format!("ambiguous or foreign {name}"));
    }
    Ok(result)
}
fn text(node: Node<'_, '_>) -> Result<String, String> {
    non_nil(node)?;
    if node.children().any(|node| node.is_element()) {
        return Err("expected scalar form evidence".into());
    }
    let value = unica_xml_format::read::direct_text(node);
    let value = value.trim_matches([' ', '\t', '\r', '\n']);
    if value.is_empty() {
        Err("missing scalar form evidence".into())
    } else {
        Ok(value.to_string())
    }
}

#[derive(Debug)]
enum CompatibilityIssue {
    Invalid(String),
    Unsupported(String),
}
impl From<String> for CompatibilityIssue {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
fn compatible(
    content: &str,
    owner: &MetadataAddress,
    property: &str,
) -> Result<(), CompatibilityIssue> {
    let doc = Document::parse(content.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    require_form_root(root)?;
    non_nil(root)?;
    let version = unica_source_designer::roots::root_version_literal(
        content.trim_start_matches('\u{feff}'),
        root,
    );
    if let Some(refusal) = writable_profile_refusal("CommonForm Form.xml", version.as_deref()) {
        return Err(CompatibilityIssue::Invalid(refusal.message));
    }
    if root
        .children()
        .any(|n| n.is_element() && n.tag_name().name() == "BaseForm")
    {
        return Err(CompatibilityIssue::Unsupported(
            "inherited CommonForm content is outside the assignment profile".into(),
        ));
    }
    let attrs = single(root, FORM_LOGFORM_NS, "Attributes")?;
    let mut main = Vec::new();
    for attribute in attrs
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Attribute")
    {
        if attribute.tag_name().namespace() != Some(FORM_LOGFORM_NS) {
            return Err("foreign form Attribute".to_string().into());
        }
        let flags = attribute
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "MainAttribute")
            .count();
        if flags == 0 {
            continue;
        }
        match text(single(attribute, FORM_LOGFORM_NS, "MainAttribute")?)?.as_str() {
            "true" | "1" => main.push(attribute),
            "false" | "0" => {}
            _ => return Err("invalid MainAttribute boolean".to_string().into()),
        }
    }
    if main.len() != 1 {
        return Err(CompatibilityIssue::Unsupported(
            "CommonForm must have one direct main attribute".into(),
        ));
    }
    non_nil(main[0])?;
    let types = single(main[0], FORM_LOGFORM_NS, "Type")?;
    let ty = single(types, V8_NS, "Type")?;
    if types.children().filter(|n| n.is_element()).count() != 1 {
        return Err(CompatibilityIssue::Unsupported(
            "CommonForm main attribute must have one concrete type".into(),
        ));
    }
    let value = text(ty)?;
    let (prefix, local) = match value.split_once(':') {
        Some((prefix, local))
            if unica_xml_format::names::xml_ncname_is_valid(prefix)
                && unica_xml_format::names::xml_ncname_is_valid(local) =>
        {
            (Some(prefix), local)
        }
        None if unica_xml_format::names::xml_ncname_is_valid(&value) => (None, value.as_str()),
        _ => return Err("invalid CommonForm main attribute QName".to_string().into()),
    };
    let ns = ty
        .lookup_namespace_uri(prefix)
        .ok_or_else(|| "unbound CommonForm main attribute QName".to_string())?;
    if ns != CFG_NS {
        return Err(CompatibilityIssue::Unsupported(
            "CommonForm main attribute must use the configuration type namespace".into(),
        ));
    }
    match owner.segments().collect::<Vec<_>>().as_slice() {
        ["Report", _] if local == "ReportObject" => {
            // A real 8.3.27 import/re-export materializes Main when this
            // property is absent on a generic ReportObject common form.
            let purpose = if root
                .children()
                .any(|node| node.is_element() && node.tag_name().name() == "ReportFormType")
            {
                text(single(root, FORM_LOGFORM_NS, "ReportFormType")?)?
            } else {
                "Main".to_string()
            };
            let expected = match property {
                "DefaultForm" => "Main",
                "DefaultSettingsForm" => "Settings",
                "DefaultVariantForm" => "Variant",
                _ => {
                    return Err(CompatibilityIssue::Unsupported(
                        "unsupported CommonForm report slot".into(),
                    ))
                }
            };
            if purpose != expected {
                return Err(CompatibilityIssue::Unsupported(format!(
                    "this report slot requires a CommonForm with ReportFormType={expected}"
                )));
            }
        }
        ["Constant", _] if property == "DefaultForm" && local == "ConstantsSet" => {}
        _ => {
            return Err(CompatibilityIssue::Unsupported(
                "CommonForm main attribute is incompatible with this owner".into(),
            ))
        }
    }
    Ok(())
}

pub fn common_form_reference(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    property: &str,
    target: &MetadataAddress,
    op_index: usize,
) -> Result<String, ApplyPlanError> {
    let location = format!("ops[{op_index}].args.values");
    let invalid = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(location.clone())
    };
    let unsupported = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(location.clone())
    };
    let root = Path::new("Configuration.xml");
    let parent = read_required(staged, root, "the configuration descriptor", &location)?;
    let evidence = prove_already_read_source_set_owner(root, &parent, authority.source_kind())
        .map_err(|e| invalid(e.message))?;
    if let Some(refusal) = writable_profile_refusal(root.portable(), evidence.version()) {
        return Err(invalid(refusal.message));
    }
    let parts = target.segments().collect::<Vec<_>>();
    let ["CommonForm", name] = parts.as_slice() else {
        return Err(unsupported(
            "a concrete CommonForm address is required".into(),
        ));
    };
    let name = *name;
    if !evidence.registers("CommonForm", name) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "CommonForm is not registered in this source set",
        )
        .at_path(location));
    }
    let relative = owner_descriptor_relative(target, authority, op_index)?;
    let raw = read_required(staged, &relative, "the CommonForm descriptor", &location)?;
    let evidence =
        prove_already_read_metadata_owner(&relative, &raw).map_err(|e| invalid(e.message))?;
    if let Some(refusal) = writable_profile_refusal(relative.portable(), evidence.version()) {
        return Err(invalid(refusal.message));
    }
    if evidence.artifact_kind() != "CommonForm" || evidence.artifact_name() != Some(name) {
        return Err(invalid(
            "CommonForm descriptor does not match the reference".into(),
        ));
    }
    let source = utf8_body(&raw, "the CommonForm descriptor", &location)?;
    let doc = Document::parse(&source).map_err(|e| invalid(e.to_string()))?;
    non_nil(doc.root_element()).map_err(invalid)?;
    let form = single(doc.root_element(), MD_CLASSES_NS, "CommonForm").map_err(invalid)?;
    let props = single(form, MD_CLASSES_NS, "Properties").map_err(invalid)?;
    if text(single(props, MD_CLASSES_NS, "Name").map_err(invalid)?).map_err(invalid)? != name {
        return Err(invalid(
            "CommonForm Name does not match the reference".into(),
        ));
    }
    if text(single(props, MD_CLASSES_NS, "FormType").map_err(invalid)?).map_err(invalid)?
        != "Managed"
    {
        return Err(unsupported(
            "only Managed CommonForm assignments are supported".into(),
        ));
    }
    let content_relative = PathBuf::from("CommonForms").join(name).join("Ext/Form.xml");
    let content = read_required(
        staged,
        &content_relative,
        "the CommonForm content",
        &location,
    )?;
    let content = utf8_body(&content, "the CommonForm content", &location)?;
    compatible(&content, owner, property).map_err(|error| match error {
        CompatibilityIssue::Invalid(message) => invalid(message),
        CompatibilityIssue::Unsupported(message) => unsupported(message),
    })?;
    Ok(target.as_str().to_string())
}

/// Replace the one direct, namespace-proven owner slot, retaining every
/// other byte. Descriptor/content validation is independent of this edit.
pub fn set_common_form_slot(
    source: &str,
    owner: &MetadataAddress,
    property: &str,
    reference: &str,
) -> Result<String, String> {
    let doc = Document::parse(source).map_err(|error| error.to_string())?;
    let root = doc.root_element();
    non_nil(root)?;
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err("invalid form owner XML root".into());
    }
    let parts = owner.segments().collect::<Vec<_>>();
    let [kind, name] = parts.as_slice() else {
        return Err("a top-level form owner is required".into());
    };
    let object = single(root, MD_CLASSES_NS, kind)?;
    let props = single(object, MD_CLASSES_NS, "Properties")?;
    if text(single(props, MD_CLASSES_NS, "Name")?)? != *name {
        return Err("form owner Name does not match its address".into());
    }
    let slot = single(props, MD_CLASSES_NS, property)?;
    if slot.children().any(|node| node.is_element()) {
        return Err("a form slot must be a scalar reference".into());
    }
    let current = slot
        .children()
        .filter(|node| node.is_text())
        .filter_map(|node| node.text())
        .collect::<String>();
    if current == reference {
        return Ok(source.to_string());
    }
    let range = slot.range();
    let open_end = crate::root::form_edit_opening_tag_end(source, range.start)
        .ok_or_else(|| "invalid form slot opening tag".to_string())?;
    let opening = &source[range.start..=open_end];
    let escaped = unica_xml_format::text::escape_xml_text(reference);
    let mut result = source.to_string();
    if opening.ends_with("/>") {
        let slash = opening.len() - 2;
        let name = opening[1..]
            .split([' ', '\t', '\r', '\n', '/', '>'])
            .next()
            .ok_or_else(|| "missing form slot name".to_string())?;
        result.replace_range(
            range,
            &format!("{}>{escaped}</{name}>", opening[..slash].trim_end()),
        );
    } else {
        let close = source[range.clone()]
            .rfind("</")
            .ok_or_else(|| "missing form slot closing tag".to_string())?
            + range.start;
        result.replace_range(open_end + 1..close, &escaped);
    }
    Ok(result)
}
