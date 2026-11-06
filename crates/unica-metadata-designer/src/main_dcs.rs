//! Main DCS references are checked against the final staged owner and template.
use crate::validation::{
    exact_mdclasses_children, exact_metadata_artifact, validate_exact_owner_identity,
    MetadataAdmission,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_xml_format::read::{direct_text, parse_metadata_image};

fn reference(bytes: &[u8], owner: &MetadataAddress) -> Result<Option<String>, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let artifact = exact_metadata_artifact(&document, MetadataAdmission::Writable)?;
    validate_exact_owner_identity(artifact, owner)?;
    let properties = exact_mdclasses_children(artifact, "Properties", "report descriptor")?;
    let [properties] = properties.as_slice() else {
        return Err("report must contain one Properties".into());
    };
    let references = exact_mdclasses_children(
        *properties,
        "MainDataCompositionSchema",
        "report descriptor",
    )?;
    match references.as_slice() {
        [] => Ok(None),
        [node] if !node.children().any(|child| child.is_element()) => {
            Ok(Some(direct_text(*node).trim().into()))
        }
        _ => Err("report must contain at most one scalar MainDataCompositionSchema".into()),
    }
}

/// Only a registered DCS of this owner can be selected. The descriptor read
/// is retained by the proposal so a changed template makes publication stale.
pub(crate) fn validate_selected(
    staged: &mut ApplyProposal<'_>,
    owner_relative: &Path,
    owner: &MetadataAddress,
    owner_source: &[u8],
    selected: &str,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    let refuse = |message: String| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path)
    };
    // Prove the exact owner/property even when clearing the value.
    reference(owner_source, owner).map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path)
    })?;
    let selected = selected.trim();
    if selected.is_empty() {
        return Ok(());
    }
    let parsed =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, selected).map_err(|_| {
            refuse("MainDataCompositionSchema must name a template of this report".into())
        })?;
    let prefix = format!("{owner}.Template.");
    let name = parsed
        .as_str()
        .strip_prefix(&prefix)
        .filter(|name| !name.contains('.') && !name.is_empty())
        .ok_or_else(|| {
            refuse("MainDataCompositionSchema must name a template of this report".into())
        })?;
    let relative = owner_relative
        .with_extension("")
        .join("Templates")
        .join(format!("{name}.xml"));
    let bytes = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?
        .ok_or_else(|| refuse(format!("the selected template `{selected}` was not found")))?;
    let text =
        std::str::from_utf8(owner_source).map_err(|_| refuse("the report is not UTF-8".into()))?;
    // This shared exact identity check proves the owner registration, template
    // namespace, Name, UUID and its single TemplateType, without editing it.
    let identity =
        crate::validation::validate_template_removal_identity(text, &bytes, owner, name, false)
            .map_err(refuse)?;
    if identity.template_type.as_deref() != Some("DataCompositionSchema") {
        return Err(refuse(format!(
            "the selected template `{selected}` is not DataCompositionSchema"
        )));
    }
    let body_relative = relative.with_extension("").join("Ext/Template.xml");
    let body = staged
        .read(&body_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?
        .ok_or_else(|| {
            refuse(format!(
                "the selected schema `{selected}` has no primary XML resource"
            ))
        })?;
    crate::validation::validate_template_removal_primary_image("DataCompositionSchema", &body)
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path)
        })?;
    Ok(())
}

pub(crate) fn validate_changes(
    staged: &mut ApplyProposal<'_>,
    relative: &Path,
    owner: &MetadataAddress,
    source: &[u8],
    operation: &unica_metadata::MetaEditOperation,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let unica_metadata::MetaEditOperation::SetProperties { values } = operation else {
        return Ok(());
    };
    for (key, value) in values.entries() {
        if *key == unica_metadata::MetaPropertyKey::MainDataCompositionSchema {
            let unica_metadata::MetaPropertyValue::String(selected) = value else {
                unreachable!("typed property is a string")
            };
            validate_selected(
                staged,
                relative,
                owner,
                source,
                selected,
                &format!("ops[{op_index}].args.values.MainDataCompositionSchema"),
            )?;
        }
    }
    Ok(())
}

/// Validate affected report owners, including a change of TemplateType whose
/// owner XML did not change. Never walk the workspace to find unrelated reports.
pub(crate) fn validate_final(
    staged: &mut ApplyProposal<'_>,
    source_set: &str,
) -> Result<(), ApplyPlanError> {
    let mut owners = BTreeMap::<PathBuf, BTreeSet<String>>::new();
    for change in staged.planned_changes() {
        let path = &change.relative_path;
        let parts = path
            .iter()
            .filter_map(|part| part.to_str())
            .collect::<Vec<_>>();
        match parts.as_slice() {
            ["Reports", file] if file.ends_with(".xml") => {
                owners.entry(path.clone()).or_default();
            }
            ["Reports", owner, "Templates", file] if file.ends_with(".xml") => {
                owners
                    .entry(Path::new("Reports").join(format!("{owner}.xml")))
                    .or_default()
                    .insert(file.trim_end_matches(".xml").to_string());
            }
            _ => {}
        }
    }
    for (relative, templates) in owners {
        let Some(bytes) = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, "MainDataCompositionSchema"))?
        else {
            continue;
        };
        let Some(name) = relative.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let owner =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("Report.{name}"))
                .map_err(|error| {
                    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
                })?;
        let selected = reference(&bytes, &owner);
        // An unrelated owner-property edit does not turn an old bad reference
        // into this operation's responsibility. A changed selected template is checked.
        let reference_unchanged = staged
            .original_image(&relative)
            .is_some_and(|original| reference(original, &owner) == selected);
        let selected_template_changed = selected
            .as_ref()
            .ok()
            .and_then(Option::as_deref)
            .and_then(|selected| selected.strip_prefix(&format!("{owner}.Template.")))
            .is_some_and(|name| templates.contains(name));
        if reference_unchanged && !selected_template_changed {
            continue;
        }
        let at = format!("{source_set}:{owner}.Props.MainDataCompositionSchema");
        let selected = selected.map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(&at)
        })?;
        if let Some(selected) = selected {
            validate_selected(staged, &relative, &owner, &bytes, &selected, &at)?;
        }
    }
    Ok(())
}
