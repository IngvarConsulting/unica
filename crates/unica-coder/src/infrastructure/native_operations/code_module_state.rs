use super::staged_code_error;
use crate::domain::{
    events::{DomainEvent, DomainEventKind},
    project_sources::SourceSetKind,
};
use crate::infrastructure::{
    native_operations::{
        apply::{ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState},
        cfe::cfe_patch_mark_extended_property,
    },
    platform_xml_source_targets::{PlatformXmlModuleIdentity, PlatformXmlModuleRole},
    workspace_actor::CodeApplyAuthority,
};
use std::path::PathBuf;

/// The descriptor is a normal staged postimage: it shares the BSL revision
/// fence, dry run, cache effects and rollback journal (INV.SOURCE.CODE-BORROWED-MODULE-STATE).
pub(super) fn stage_borrowed_module_state(
    staged: &mut ApplyStagedState,
    effects: &mut Vec<(PathBuf, DomainEvent)>,
    authority: &CodeApplyAuthority<'_>,
    identity: &PlatformXmlModuleIdentity,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if authority.source_kind() != SourceSetKind::Extension {
        return Ok(());
    }
    let invalid = || {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "borrowed module has invalid or incompatible property-state metadata",
        )
        .at_path(at_path)
    };
    let relative = identity.descriptors.last().ok_or_else(invalid)?;
    let before = staged
        .read(relative)
        .map_err(|error| staged_code_error(error, at_path))?
        .ok_or_else(invalid)?;
    let text = std::str::from_utf8(&before).map_err(|_| invalid())?;
    let document =
        roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|_| invalid())?;
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    let object = document
        .root_element()
        .children()
        .find(|n| n.is_element())
        .ok_or_else(invalid)?;
    let properties = object
        .children()
        .find(|n| n.has_tag_name((MD, "Properties")))
        .ok_or_else(invalid)?;
    let belonging: Vec<_> = properties
        .children()
        .filter(|n| n.has_tag_name((MD, "ObjectBelonging")))
        .collect();
    match belonging.as_slice() {
        [] => return Ok(()),
        [node] if node.text() == Some("Own") => return Ok(()),
        [node] if node.text() == Some("Adopted") => {}
        _ => return Err(invalid()),
    }
    let property = match identity.role {
        PlatformXmlModuleRole::FormModule => "Form",
        role => role.as_str(),
    };
    let after =
        cfe_patch_mark_extended_property(identity.address.as_str(), relative, &before, property)
            .map_err(|_| invalid())?;
    if after == before {
        return Ok(());
    }
    staged
        .replace(relative, &before, after)
        .map_err(|error| staged_code_error(error, at_path))?;
    let owner = identity
        .address
        .as_str()
        .rsplit_once('.')
        .ok_or_else(invalid)?
        .0;
    let event = if owner == "Configuration" {
        DomainEventKind::ConfigXmlChanged
    } else {
        DomainEventKind::MetadataChanged
    };
    effects.push((
        relative.clone(),
        DomainEvent::new(event, format!("{}:{owner}", authority.source_set_name())),
    ));
    Ok(())
}
