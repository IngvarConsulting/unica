// module: private
use std::collections::BTreeSet;
use unica_address::source_target::MetadataAddress;
use unica_metadata::read_codec::{
    ExternalInlineMembers, ExternalInlineReadError, ReadFinding, ReadStopped,
};
use unica_metadata::MetaDiagnosticSeverity;
use unica_source_designer::owner::MD_CLASSES_NS;

pub(crate) fn read(
    descriptor: &[u8],
    target: &MetadataAddress,
    checkpoint: &mut dyn FnMut() -> Result<(), ReadStopped>,
) -> Result<ExternalInlineMembers, ExternalInlineReadError> {
    checkpoint()?;
    let text =
        std::str::from_utf8(descriptor).map_err(|error| invalid_descriptor(error.to_string()))?;
    let xml = text.trim_start_matches('\u{feff}');
    let document =
        roxmltree::Document::parse(xml).map_err(|error| invalid_descriptor(error.to_string()))?;
    // The caller has already proved the owner using these bytes.
    let owner = document
        .root_element()
        .first_element_child()
        .ok_or_else(|| invalid_descriptor("missing owner".into()))?;
    let mut uuids = owner
        .attribute("uuid")
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .into_iter()
        .collect();
    let children = direct_md_container(owner, "ChildObjects").map_err(invalid_descriptor)?;
    if let Some(children) = children {
        validate_inline_members(children, true, &mut uuids, checkpoint)?;
    }

    let mut diagnostics = Vec::new();
    checkpoint()?;
    let attributes = crate::info::typed_elements_with_diagnostics(
        xml,
        children,
        "Attribute",
        false,
        "collections.attributes",
        target,
        &mut diagnostics,
    );
    checkpoint()?;
    let tabular_sections = crate::info::typed_elements_with_diagnostics(
        xml,
        children,
        "TabularSection",
        true,
        "collections.tabularSections",
        target,
        &mut diagnostics,
    );
    let unreadable = diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
        .filter_map(|diagnostic| {
            Some(ReadFinding {
                field: diagnostic.field?,
                message: diagnostic.message,
            })
        })
        .collect();
    Ok(ExternalInlineMembers {
        attributes,
        tabular_sections,
        unreadable,
    })
}

fn invalid_descriptor(message: String) -> ExternalInlineReadError {
    ExternalInlineReadError::InvalidDescriptor(message)
}

fn direct_md_container<'a, 'input>(
    owner: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Result<Option<roxmltree::Node<'a, 'input>>, String> {
    let mut nodes = owner
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == name);
    let first = nodes.next();
    if first.is_some_and(|node| !node.has_tag_name((MD_CLASSES_NS, name))) || nodes.next().is_some()
    {
        return Err(format!("duplicate or foreign {name}"));
    }
    Ok(first)
}

fn validate_inline_members(
    children: roxmltree::Node<'_, '_>,
    allow_sections: bool,
    uuids: &mut BTreeSet<uuid::Uuid>,
    checkpoint: &mut dyn FnMut() -> Result<(), ReadStopped>,
) -> Result<(), ExternalInlineReadError> {
    use crate::info_projection::{direct_text_content, strict_text_leaf_is_valid};
    use unica_metadata::identifier::is_1c_identifier;

    let mut names = BTreeSet::new();
    for member in children.children().filter(roxmltree::Node::is_element) {
        let tag = member.tag_name().name();
        if tag != "Attribute" && !(allow_sections && tag == "TabularSection") {
            continue;
        }
        checkpoint()?;
        if !member.has_tag_name((MD_CLASSES_NS, tag)) {
            return Err(invalid_descriptor(format!("foreign {tag}")));
        }
        let properties = direct_md_container(member, "Properties")
            .map_err(invalid_descriptor)?
            .ok_or_else(|| invalid_descriptor(format!("{tag} has no Properties")))?;
        let name = direct_md_container(properties, "Name")
            .map_err(invalid_descriptor)?
            .filter(|node| strict_text_leaf_is_valid(*node))
            .map(direct_text_content)
            .filter(|name| is_1c_identifier(name))
            .ok_or_else(|| invalid_descriptor(format!("{tag} has no unique valid Name")))?;
        let uuid = member
            .attribute("uuid")
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .ok_or_else(|| invalid_descriptor(format!("{tag}.{name} has no valid UUID")))?;
        if !uuids.insert(uuid) || !names.insert((tag, name.to_lowercase())) {
            return Err(invalid_descriptor(format!(
                "duplicate identity for {tag}.{name}"
            )));
        }
        if tag == "TabularSection" {
            if let Some(columns) =
                direct_md_container(member, "ChildObjects").map_err(invalid_descriptor)?
            {
                validate_inline_members(columns, false, uuids, checkpoint)?;
            }
        }
    }
    Ok(())
}
