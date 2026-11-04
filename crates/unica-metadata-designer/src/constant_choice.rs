//! Choice-form references are resolved through the proposal, so their observed
//! registration and descriptors participate in saved-plan input equality.
use std::path::Path;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::metadata_descriptor_relative;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn choice_form(bytes: &[u8]) -> Result<String, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let constant = child(document.root_element(), "Constant")?;
    let property = child(child(constant, "Properties")?, "ChoiceForm")?;
    if property.attributes().len() != 0 || property.children().any(|node| node.is_element()) {
        return Err("ChoiceForm must be plain text".into());
    }
    Ok(unica_xml_format::read::direct_text(property))
}

fn child<'a>(
    parent: roxmltree::Node<'a, 'a>,
    name: &str,
) -> Result<roxmltree::Node<'a, 'a>, String> {
    let mut matching = parent
        .children()
        .filter(|node| node.has_tag_name((MD, name)));
    let first = matching.next().ok_or_else(|| format!("missing {name}"))?;
    if matching.next().is_some() {
        return Err(format!("duplicate {name}"));
    }
    Ok(first)
}

pub(crate) fn validate_choice_form(
    staged: &mut ApplyProposal<'_>,
    relative: &Path,
    bytes: &[u8],
    source_kind: SourceSetKind,
    force: bool,
) -> Result<(), ApplyPlanError> {
    let final_form = choice_form(bytes);
    if !force
        && staged
            .original_image(relative)
            .is_some_and(|original| choice_form(original) == final_form)
    {
        return Ok(());
    }
    let validate = || -> Result<(), String> {
        let form = final_form?;
        if form.is_empty() {
            return Ok(());
        }
        let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &form)
            .map_err(|error| error.to_string())?;
        let parts = form.split('.').collect::<Vec<_>>();
        let (owner_kind, owner_name, form_name) = match parts.as_slice() {
            ["CommonForm", name] => ("CommonForm", *name, *name),
            [kind, owner, "Form", name] => (*kind, *owner, *name),
            _ => return Err("expected CommonForm.Name or Kind.Name.Form.Name".into()),
        };
        let config = staged
            .read(Path::new("Configuration.xml"))
            .map_err(|error| error.to_string())?
            .ok_or("configuration registration is missing")?;
        let registrations =
            crate::validation_context::inspect_metadata_registration_image(&config)?;
        if !registrations
            .registrations
            .iter()
            .any(|(kind, name)| kind == owner_kind && name == owner_name)
        {
            return Err(format!(
                "{owner_kind}.{owner_name} is not registered in Configuration"
            ));
        }
        if owner_kind != "CommonForm" {
            let owner = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("{owner_kind}.{owner_name}"),
            )
            .map_err(|error| error.to_string())?;
            let owner_path = metadata_descriptor_relative(&owner, source_kind)?;
            let owner_bytes = staged
                .read(&owner_path)
                .map_err(|error| error.to_string())?
                .ok_or("form owner is missing")?;
            let identity = crate::validation_context::inspect_metadata_image_identity(&owner_bytes)
                .map_err(|error| error.to_string())?;
            if identity.object_type != owner_kind || identity.object_name != owner_name {
                return Err("form owner identity mismatch".into());
            }
            let (_, document) = unica_xml_format::read::parse_metadata_image(&owner_bytes)?;
            let children = child(child(document.root_element(), owner_kind)?, "ChildObjects")?;
            if !children
                .children()
                .any(|node| node.has_tag_name((MD, "Form")) && node.text() == Some(form_name))
            {
                return Err(format!("{form} is not registered by its owner"));
            }
        }
        let path = metadata_descriptor_relative(&address, source_kind)?;
        let form_bytes = staged
            .read(&path)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{form} was not found"))?;
        let identity = crate::validation_context::inspect_form_metadata_image_identity(&form_bytes)
            .map_err(|error| error.to_string())?;
        let expected_kind = if owner_kind == "CommonForm" {
            "CommonForm"
        } else {
            "Form"
        };
        if identity.object_type != expected_kind || identity.object_name != form_name {
            return Err("form identity mismatch".into());
        }
        Ok(())
    };
    validate().map_err(|message| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("values.ChoiceForm: {message}"),
        )
        .at_path(relative.portable().to_string())
    })
}
