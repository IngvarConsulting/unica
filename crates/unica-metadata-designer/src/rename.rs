//! Identity-preserving catalog renames over the actor-owned proposal.
use roxmltree::{Document, Node};
use std::path::{Path, PathBuf};
use unica_address::source_target::MetadataAddress;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority,
    ProvisionalApplyEffect, SupportPolicyMode,
};
use unica_source_contract::read::{SourceDirectory, SourceEntry};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const CORE: &str = "http://v8.1c.ru/8.1/data/core";
const CFG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
const DUMP: &str = "http://v8.1c.ru/8.3/xcf/dumpinfo";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn invalid(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message)
}
fn xml_error(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
}
fn lower(value: &str) -> String {
    value.to_lowercase()
}
fn identifier(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Match the complete owner segment, including references to its children.
fn renamed_reference(value: &str, kind: &str, old: &str, new: &str) -> Option<String> {
    let (head, tail) = value.split_once('.')?;
    if head != kind {
        return None;
    }
    let owner_end = tail.find('.').unwrap_or(tail.len());
    (lower(&tail[..owner_end]) == lower(old)).then(|| format!("{kind}.{new}{}", &tail[owner_end..]))
}

fn bare_mentions(value: &str, old: &str) -> bool {
    let value = lower(value);
    let old = lower(old);
    value.match_indices(&old).any(|(start, _)| {
        !value[..start].chars().next_back().is_some_and(identifier)
            && !value[start + old.len()..]
                .chars()
                .next()
                .is_some_and(identifier)
    })
}

fn composition_slot(path: &Path, node: Node<'_, '_>, attribute: Option<&str>) -> bool {
    use crate::remove::CompositionKey;
    let Some((rules, _)) = crate::remove::meta_remove_composition(path) else {
        return false;
    };
    let root_ns = node.document().root_element().tag_name().namespace();
    if !matches!(
        root_ns,
        Some(MD | "http://v8.1c.ru/8.2/roles" | "http://v8.1c.ru/8.3/xcf/extrnprops")
    ) || (node.tag_name().namespace() != root_ns && node.tag_name().namespace() != Some(XR))
    {
        return false;
    }
    rules.iter().any(|rule| {
        let entry = match rule.key {
            CompositionKey::Text if attribute.is_none() => Some(node),
            CompositionKey::Attribute(name) if attribute == Some(name) => Some(node),
            CompositionKey::Child(name)
                if attribute.is_none() && node.tag_name().name() == name =>
            {
                node.parent_element()
            }
            _ => None,
        };
        entry.is_some_and(|entry| {
            entry.tag_name().name() == rule.entry
                && rule.parent.is_none_or(|parent| {
                    entry
                        .parent_element()
                        .is_some_and(|p| p.tag_name().name() == parent)
                })
        })
    })
}

fn code_mention_patterns(old: &str) -> Vec<String> {
    let mut heads = vec![
        "Catalog".to_string(),
        "Catalogs".into(),
        "Справочник".into(),
        "Справочники".into(),
    ];
    for kind in crate::remove::meta_remove_derived_type_names("Catalog") {
        heads.extend(crate::remove::meta_remove_ru_derived_type_names(
            "Catalog",
            &kind,
            &["Справочник"],
        ));
        heads.push(kind);
    }
    heads
        .iter()
        .map(|head| lower(&format!("{head}.{old}")))
        .collect()
}

fn code_mentions(value: &str, patterns: &[String]) -> bool {
    let value = lower(value);
    patterns.iter().any(|pattern| {
        value.match_indices(pattern.as_str()).any(|(start, _)| {
            !value[..start].chars().next_back().is_some_and(identifier)
                && !value[start + pattern.len()..]
                    .chars()
                    .next()
                    .is_some_and(identifier)
        })
    })
}

fn qname_rename(node: Node<'_, '_>, value: &str, old: &str, new: &str) -> Option<String> {
    let (prefix, local) = value.split_once(':')?;
    if node.lookup_namespace_uri(Some(prefix)) != Some(CFG) {
        return None;
    }
    crate::remove::meta_remove_derived_type_names("Catalog")
        .iter()
        .find_map(|kind| renamed_reference(local, kind, old, new))
        .map(|renamed| format!("{prefix}:{renamed}"))
}

fn catalog_properties(node: Node<'_, '_>) -> bool {
    node.tag_name().namespace() == Some(MD)
        && node.tag_name().name() == "Properties"
        && node.parent_element().is_some_and(|catalog| {
            catalog.tag_name().namespace() == Some(MD)
                && catalog.tag_name().name() == "Catalog"
                && catalog.parent_element().is_some_and(|root| {
                    root == node.document().root_element()
                        && root.tag_name().namespace() == Some(MD)
                        && root.tag_name().name() == "MetaDataObject"
                })
        })
}

fn catalog_form_reference_slot(node: Node<'_, '_>) -> bool {
    let Some(parent) = node.parent_element() else {
        return false;
    };
    if node.tag_name().namespace() == Some(MD)
        && matches!(
            node.tag_name().name(),
            "DefaultObjectForm"
                | "DefaultFolderForm"
                | "DefaultListForm"
                | "DefaultChoiceForm"
                | "DefaultFolderChoiceForm"
                | "AuxiliaryObjectForm"
                | "AuxiliaryFolderForm"
                | "AuxiliaryListForm"
                | "AuxiliaryChoiceForm"
                | "AuxiliaryFolderChoiceForm"
        )
    {
        return catalog_properties(parent);
    }
    node.tag_name().namespace() == Some(XR)
        && node.tag_name().name() == "ChoiceForm"
        && parent.tag_name().namespace() == Some(XR)
        && parent.tag_name().name() == "StandardAttribute"
        && parent.parent_element().is_some_and(|attributes| {
            attributes.tag_name().namespace() == Some(MD)
                && attributes.tag_name().name() == "StandardAttributes"
                && attributes.parent_element().is_some_and(catalog_properties)
        })
}

fn renamed_form_reference(value: &str, old: &str, new: &str) -> Option<String> {
    let mut parts = value.split('.');
    if parts.next() != Some("Catalog") || parts.next().is_none() || parts.next() != Some("Form") {
        return None;
    }
    let form = parts.next()?;
    if !unica_metadata::identifier::is_1c_identifier(form) || parts.next().is_some() {
        return None;
    }
    renamed_reference(value, "Catalog", old, new)
}

fn is_prose(node: Node<'_, '_>) -> bool {
    node.ancestors().any(|ancestor| {
        ancestor.is_element()
            && ancestor.tag_name().namespace() == Some(MD)
            && matches!(ancestor.tag_name().name(), "Synonym" | "Comment")
    })
}

/// Only the explicitly supported reference slots are rewritten. Other
/// significant mentions, including query expressions, refuse the entire plan.
fn rename_xml(
    path: &Path,
    bytes: &[u8],
    old: &str,
    new: &str,
    owner: bool,
    code_patterns: &[String],
) -> Result<Vec<u8>, ApplyPlanError> {
    let text = std::str::from_utf8(bytes).map_err(|_| xml_error("rename input is not UTF-8"))?;
    let source = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bom = text.len() - source.len();
    let doc = Document::parse(source).map_err(|error| {
        xml_error(format!(
            "cannot inspect rename XML {}: {error}",
            path.display()
        ))
    })?;
    let mut replacements = Vec::new();
    let mut owner_names = 0;
    let mut registrations = 0;
    for node in doc.descendants().filter(|node| node.is_element()) {
        let ns = node.tag_name().namespace();
        let tag = node.tag_name().name();
        for attribute in node.attributes() {
            let value = attribute.value();
            let generated = owner
                && attribute.namespace().is_none()
                && ns == Some(XR)
                && tag == "GeneratedType"
                && attribute.name() == "name";
            let dump = attribute.namespace().is_none()
                && ns == Some(DUMP)
                && tag == "Metadata"
                && attribute.name() == "name";
            let updated = if generated {
                crate::remove::meta_remove_derived_type_names("Catalog")
                    .iter()
                    .find_map(|kind| renamed_reference(value, kind, old, new))
            } else if dump
                || (attribute.namespace().is_none()
                    && composition_slot(path, node, Some(attribute.name())))
            {
                renamed_reference(value, "Catalog", old, new)
            } else if attribute.namespace() == Some(XSI) && attribute.name() == "type" {
                qname_rename(node, value, old, new)
            } else {
                None
            };
            if let Some(updated) = updated {
                replacements.push((
                    attribute.range_value(),
                    unica_xml_format::text::escape_xml(&updated),
                ));
            } else if bare_mentions(value, old) && !is_prose(node) {
                return Err(invalid(format!(
                    "unhandled rename dependency in {} @{}",
                    path.display(),
                    attribute.name()
                )));
            }
        }
        if node.children().any(|child| child.is_element()) {
            let direct_text: String = node
                .children()
                .filter(|child| child.is_text())
                .filter_map(|child| child.text())
                .collect();
            if !is_prose(node) && bare_mentions(&direct_text, old) {
                return Err(invalid(format!(
                    "unhandled mixed-content rename dependency in {} <{tag}>",
                    path.display()
                )));
            }
            continue;
        }
        let value: String = node
            .children()
            .filter(|child| child.is_text())
            .filter_map(|child| child.text())
            .collect();
        if value.is_empty() {
            continue;
        }
        let trimmed = value.trim();
        let catalog_name = owner
            && ns == Some(MD)
            && tag == "Name"
            && node.parent_element().is_some_and(|p| {
                p.tag_name().namespace() == Some(MD)
                    && p.tag_name().name() == "Properties"
                    && p.parent_element().is_some_and(|p| {
                        p.tag_name().namespace() == Some(MD)
                            && p.tag_name().name() == "Catalog"
                            && p.parent_element() == Some(doc.root_element())
                    })
            });
        let registration = path == Path::new("Configuration.xml")
            && ns == Some(MD)
            && tag == "Catalog"
            && node.parent_element().is_some_and(|p| {
                p.tag_name().namespace() == Some(MD)
                    && p.tag_name().name() == "ChildObjects"
                    && p.parent_element().is_some_and(|p| {
                        p.tag_name().namespace() == Some(MD)
                            && p.tag_name().name() == "Configuration"
                    })
            });
        let updated = if catalog_name {
            if trimmed != old {
                return Err(xml_error(
                    "catalog descriptor Name differs from its address",
                ));
            }
            owner_names += 1;
            Some(new.to_string())
        } else if registration && lower(trimmed) == lower(old) {
            registrations += 1;
            Some(new.to_string())
        } else if catalog_form_reference_slot(node) {
            renamed_form_reference(trimmed, old, new)
        } else if composition_slot(path, node, None) || (ns == Some(XR) && tag == "Field") {
            renamed_reference(trimmed, "Catalog", old, new)
        } else if ns == Some(CORE) && tag == "Type" {
            qname_rename(node, trimmed, old, new)
        } else if ns == Some(XR)
            && tag == "Item"
            && node.attribute((XSI, "type")).is_some_and(|typ| {
                typ.split_once(':').is_some_and(|(prefix, local)| {
                    local == "MDObjectRef" && node.lookup_namespace_uri(Some(prefix)) == Some(XR)
                })
            })
        {
            renamed_reference(trimmed, "Catalog", old, new)
        } else {
            None
        };
        if let Some(updated) = updated {
            let children: Vec<_> = node.children().filter(|child| child.is_text()).collect();
            if children.len() != 1 || source[children[0].range()] != value {
                return Err(invalid(format!(
                    "encoded or split rename dependency in {}; normalize this XML value first",
                    path.display()
                )));
            }
            let start = value.len() - value.trim_start().len();
            let end = value.trim_end().len();
            let range = children[0].range();
            replacements.push((
                range.start + start..range.start + end,
                unica_xml_format::text::escape_xml_text(&updated),
            ));
        } else if !is_prose(node)
            && (code_mentions(&value, code_patterns)
                || (bare_mentions(&value, old) && !(ns == Some(MD) && tag == "Name")))
        {
            return Err(invalid(format!(
                "unhandled rename dependency in {} <{tag}>",
                path.display()
            )));
        }
    }
    if owner && owner_names != 1 {
        return Err(xml_error("rename requires one catalog owner Name"));
    }
    if path == Path::new("Configuration.xml") && registrations != 1 {
        return Err(xml_error(
            "rename requires one existing catalog registration",
        ));
    }
    if replacements.is_empty() {
        return Ok(bytes.to_vec());
    }
    unica_source_designer::full_dump_validation::validate_xml_raw(path, bytes)
        .map_err(|error| xml_error(format!("rename XML format validation failed: {error:?}")))?;
    let mut updated = text.to_string();
    replacements.sort_by_key(|(range, _)| range.start);
    for (range, value) in replacements.into_iter().rev() {
        updated.replace_range(range.start + bom..range.end + bom, &value);
    }
    unica_source_designer::full_dump_validation::validate_xml_raw(path, updated.as_bytes())
        .map_err(|error| xml_error(format!("rename post-image validation failed: {error:?}")))?;
    Ok(updated.into_bytes())
}

/// The proposal's namespace guards retain these directories. Inspect them
/// through its issued read capability; never recreate an empty directory as
/// a side effect outside the source transaction.
fn refuse_empty_directories(
    directory: &SourceDirectory,
    staged: &ApplyProposal<'_>,
    depth: usize,
) -> Result<(), ApplyPlanError> {
    if depth > 256 {
        return Err(invalid("rename resource directory nesting exceeds 256"));
    }
    let names = directory
        .read_immediate_names_bounded(usize::MAX, || {
            staged
                .checkpoint("rename resource topology")
                .map_err(std::io::Error::other)
        })
        .map_err(|error| xml_error(format!("cannot inspect rename resource directory: {error}")))?;
    if names.is_empty() {
        return Err(invalid(
            "object.rename cannot preserve an empty resource directory; remove it before renaming",
        ));
    }
    for name in names {
        match directory
            .retain_immediate_child_nofollow(&name)
            .map_err(|error| xml_error(error.to_string()))?
        {
            SourceEntry::Directory(child) => refuse_empty_directories(&child, staged, depth + 1)?,
            SourceEntry::RegularFile(_) => {}
            _ => {
                return Err(invalid(
                    "rename resource tree contains an unsupported entry",
                ))
            }
        }
    }
    Ok(())
}

fn check_support(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    changed: &[(PathBuf, Vec<u8>, Vec<u8>)],
) -> Result<Vec<serde_json::Value>, ApplyPlanError> {
    let marker = staged
        .read(Path::new("Ext/ParentConfigurations.bin"))
        .map_err(|e| ApplyPlanError::staging(e, "ops"))?;
    let state = crate::support::parse_support_state_compat_bytes(marker.as_deref())
        .filter(|state| !state.removed());
    let mut locked = std::collections::BTreeSet::new();
    if state
        .as_ref()
        .is_some_and(|state| !state.global_editing_enabled())
    {
        locked.insert("Configuration".to_string());
    }
    for (path, bytes, _) in changed {
        // Every nested resource inherits the vendor rule of its owner.
        let mut owner = None;
        let parts = path.components().collect::<Vec<_>>();
        if parts.len() >= 2 {
            if let Some(layout) = parts[0]
                .as_os_str()
                .to_str()
                .and_then(unica_source_designer::layout::metadata_kind_by_directory)
            {
                let name = if parts.len() == 2 {
                    Path::new(parts[1].as_os_str()).file_stem()
                } else {
                    Some(parts[1].as_os_str())
                };
                if let Some(name) = name.and_then(|name| name.to_str()) {
                    owner = Some((
                        format!("{}.{name}", layout.tag),
                        PathBuf::from(layout.directory).join(format!("{name}.xml")),
                    ));
                }
            }
        }
        let owner_bytes = if let Some((_, owner_path)) = &owner {
            staged
                .read(owner_path)
                .map_err(|e| ApplyPlanError::staging(e, "ops"))?
                .ok_or_else(|| xml_error("rename dependency owner is missing"))?
        } else {
            bytes.clone()
        };
        if let Some((_, owner_path)) = &owner {
            unica_source_designer::full_dump_validation::validate_xml_raw(owner_path, &owner_bytes)
                .map_err(|error| {
                    xml_error(format!(
                        "rename dependency owner format validation failed: {error:?}"
                    ))
                })?;
        }
        if let Some(uuid) = crate::support::support_root_uuid_from_bytes(&owner_bytes) {
            if state
                .as_ref()
                .is_some_and(|state| state.object_rule(&uuid) == Some(0))
            {
                locked.insert(
                    owner
                        .map(|(name, _)| name)
                        .unwrap_or_else(|| "Configuration".into()),
                );
            }
        }
    }
    if locked.is_empty() || authority.support_policy_mode() == SupportPolicyMode::Off {
        return Ok(Vec::new());
    }
    let message = format!(
        "rename would change locked vendor objects: {}",
        locked.into_iter().collect::<Vec<_>>().join(", ")
    );
    if authority.support_policy_mode() == SupportPolicyMode::Deny {
        return Err(invalid(message));
    }
    Ok(vec![
        serde_json::json!({"code":"support_locked", "severity":"warning", "message": message}),
    ])
}

pub fn stage_catalog_rename(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    target: &MetadataAddress,
    old: &str,
    new: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at = format!("ops[{op_index}].args.at");
    let stage_error = |error| ApplyPlanError::staging(error, at.clone());
    unica_source_designer::registration::require_untouched_staged_state(
        staged,
        "object.rename",
        &at,
    )?;
    let descriptor = PathBuf::from(format!("Catalogs/{old}.xml"));
    staged
        .require_exact_path_spelling(&descriptor)
        .map_err(stage_error)?;
    let original = staged
        .read(&descriptor)
        .map_err(stage_error)?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "rename catalog descriptor was not found",
            )
        })?;
    if old == new {
        return Ok(());
    }
    if lower(old) == lower(new) {
        return Err(invalid("object.rename does not support case-only renames"));
    }
    let files = staged.enumerate_tree(Path::new("")).map_err(stage_error)?;
    let payload = PathBuf::from(format!("Catalogs/{old}"));
    let destination_payload = PathBuf::from(format!("Catalogs/{new}"));
    let destination = PathBuf::from(format!("Catalogs/{new}.xml"));
    let (_, absent) = staged
        .enumerate_tree_with_root_absence(&destination_payload)
        .map_err(stage_error)?;
    if !absent || staged.read(&destination).map_err(stage_error)?.is_some() {
        return Err(invalid("rename destination already exists"));
    }
    let catalogs = authority
        .source_reader()
        .retain_directory_child(std::ffi::OsStr::new("Catalogs"))
        .map_err(|e| xml_error(e.to_string()))?;
    for name in catalogs
        .read_immediate_names_bounded(usize::MAX, || {
            staged
                .checkpoint("rename name collision")
                .map_err(std::io::Error::other)
        })
        .map_err(|e| xml_error(e.to_string()))?
    {
        let name = name.to_string_lossy();
        if lower(&name) == lower(old) && name != old {
            return Err(invalid("rename source resource directory has a case alias"));
        }
        if lower(&name) == lower(new) || lower(&name) == lower(&format!("{new}.xml")) {
            return Err(invalid(
                "rename destination name collides with an existing catalog",
            ));
        }
    }
    let (_, payload_absent) = staged
        .enumerate_tree_with_root_absence(&payload)
        .map_err(stage_error)?;
    if !payload_absent {
        let directory = catalogs
            .retain_directory_child(std::ffi::OsStr::new(old))
            .map_err(|e| xml_error(e.to_string()))?;
        refuse_empty_directories(&directory, staged, 0)?;
    }
    let configuration = staged
        .read(Path::new("Configuration.xml"))
        .map_err(stage_error)?
        .ok_or_else(|| xml_error("Configuration.xml is missing"))?;
    unica_source_designer::full_dump_validation::validate_required_owner_raw(
        Path::new("Configuration.xml"),
        authority.source_kind(),
        &configuration,
    )
    .map_err(|error| {
        xml_error(format!(
            "rename configuration owner format validation failed: {error:?}"
        ))
    })?;
    let configuration_text = std::str::from_utf8(&configuration)
        .map_err(|_| xml_error("Configuration.xml is not UTF-8"))?;
    let registrations =
        unica_source_designer::registration::cf_edit_child_object_entries(configuration_text)
            .map_err(xml_error)?;
    if registrations
        .iter()
        .any(|entry| entry.type_name == "Catalog" && lower(&entry.object_name) == lower(new))
    {
        return Err(invalid("rename destination is already registered"));
    }
    let mut changed = Vec::new();
    let mut moved = Vec::new();
    let code_patterns = code_mention_patterns(old);
    for path in files {
        staged
            .checkpoint("rename dependencies")
            .map_err(stage_error)?;
        let is_payload = path.starts_with(&payload);
        let bytes = staged
            .read_guarded_bounded(&path, usize::MAX)
            .map_err(stage_error)?
            .ok_or_else(|| invalid("rename input disappeared"))?;
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let updated = if extension.eq_ignore_ascii_case("xml") {
            rename_xml(&path, &bytes, old, new, path == descriptor, &code_patterns)?
        } else {
            let text = String::from_utf8_lossy(&bytes);
            if bare_mentions(&text, old) {
                return Err(invalid(format!(
                    "unhandled code or resource rename dependency in {}",
                    path.display()
                )));
            }
            bytes.clone()
        };
        if path == descriptor || is_payload {
            let dest = if path == descriptor {
                destination.clone()
            } else {
                destination_payload.join(path.strip_prefix(&payload).expect("payload prefix"))
            };
            if extension.eq_ignore_ascii_case("xml") {
                unica_source_designer::full_dump_validation::validate_xml_raw(&path, &bytes)
                    .map_err(|error| {
                        xml_error(format!(
                            "rename resource format validation failed: {error:?}"
                        ))
                    })?;
            }
            changed.push((path.clone(), bytes.clone(), updated.clone()));
            moved.push((path, dest, bytes, updated));
        } else if updated != bytes {
            changed.push((path, bytes, updated));
        }
    }
    if !moved
        .iter()
        .any(|(path, _, bytes, _)| path == &descriptor && bytes == &original)
    {
        return Err(invalid("rename owner changed during inspection"));
    }
    let warnings = check_support(staged, authority, &changed)?;
    for (path, bytes, updated) in &changed {
        if path == &descriptor || path.starts_with(&payload) {
            continue;
        }
        staged
            .replace(path, bytes, updated.clone())
            .map_err(stage_error)?;
    }
    for (path, dest, bytes, updated) in moved {
        staged.create(&dest, updated).map_err(stage_error)?;
        staged.remove(&path, &bytes).map_err(stage_error)?;
    }
    staged
        .remove_owned_directories(&payload, &descriptor, Path::new("Catalogs"))
        .map_err(stage_error)?;
    // Relocated modules/forms change logical addresses even if their bytes do not.
    let mut effect = ProvisionalApplyEffect::single(
        destination,
        DomainEvent::new(DomainEventKind::SourceSetChanged, target.as_str()),
        op_index,
    );
    for warning in warnings {
        effect = effect.with_warning(warning);
    }
    for (path, _, _) in &changed {
        let kind = if path.starts_with("Roles") {
            Some(DomainEventKind::RoleChanged)
        } else if path.starts_with("Subsystems")
            || path.file_name().is_some_and(|name| {
                name == "CommandInterface.xml" || name == "MainSectionCommandInterface.xml"
            })
        {
            Some(DomainEventKind::SubsystemChanged)
        } else {
            None
        };
        if let Some(kind) = kind {
            provisional.push(ProvisionalApplyEffect::single(
                path,
                DomainEvent::new(
                    kind,
                    format!(
                        "{}:{}",
                        authority.source_set_name(),
                        crate::remove::meta_remove_referrer(path).0
                    ),
                ),
                op_index,
            ));
        }
    }
    provisional.push(effect);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_mentions_preserves_case_and_identifier_boundaries() {
        for old in ["Old", "СтарыйСклад"] {
            let patterns = code_mention_patterns(old);
            for head in [
                "Catalog",
                "Catalogs",
                "CatalogRef",
                "Справочники",
                "СправочникСсылка",
            ] {
                for reference in [
                    format!("{head}.{old}"),
                    format!("{head}.{old}").to_uppercase(),
                ] {
                    assert!(
                        code_mentions(&format!("({reference}).Select()"), &patterns),
                        "{reference}"
                    );
                    for value in [
                        format!("Prefix{reference}"),
                        format!("_{reference}"),
                        format!("{reference}Long"),
                        format!("{reference}_"),
                        format!("{reference}1"),
                    ] {
                        assert!(!code_mentions(&value, &patterns), "{value}");
                    }
                }
            }
        }
    }
    #[test]
    fn catalog_rename_updates_its_default_form_reference() {
        let original = xml("<Catalog><Properties><Name>Old</Name><DefaultObjectForm>Catalog.Old.Form.ItemForm</DefaultObjectForm><DefaultListForm>Catalog.OldLong.Form.ListForm</DefaultListForm></Properties><ChildObjects><Form>ItemForm</Form></ChildObjects></Catalog>");
        let actual = rename_xml(
            Path::new("Catalogs/Old.xml"),
            &original,
            "Old",
            "New",
            true,
            &code_mention_patterns("Old"),
        )
        .unwrap();
        assert_eq!(
            actual,
            String::from_utf8(original)
                .unwrap()
                .replace("<Name>Old</Name>", "<Name>New</Name>")
                .replace("Catalog.Old.Form.ItemForm", "Catalog.New.Form.ItemForm")
                .into_bytes()
        );
    }
    #[test]
    fn catalog_rename_updates_only_typed_catalog_form_slots() {
        for property in [
            "DefaultObjectForm",
            "DefaultFolderForm",
            "DefaultListForm",
            "DefaultChoiceForm",
            "DefaultFolderChoiceForm",
            "AuxiliaryObjectForm",
            "AuxiliaryFolderForm",
            "AuxiliaryListForm",
            "AuxiliaryChoiceForm",
            "AuxiliaryFolderChoiceForm",
        ] {
            let original = xml(&format!("<Catalog><Properties><Name>Old</Name><{property}>Catalog.Old.Form.ItemForm</{property}></Properties><ChildObjects><Form>ItemForm</Form></ChildObjects></Catalog>"));
            let actual = rename_xml(
                Path::new("Catalogs/Old.xml"),
                &original,
                "Old",
                "New",
                true,
                &code_mention_patterns("Old"),
            )
            .unwrap();
            assert_eq!(
                actual,
                String::from_utf8(original)
                    .unwrap()
                    .replace("<Name>Old</Name>", "<Name>New</Name>")
                    .replace("Catalog.Old.Form.ItemForm", "Catalog.New.Form.ItemForm")
                    .into_bytes(),
                "{property}"
            );
        }
        let original = xml("<Catalog><Properties><Name>Old</Name><DefaultObjectForm/><StandardAttributes><xr:StandardAttribute name=\"Ref\"><xr:ChoiceForm>Catalog.Old.Form.SelectForm</xr:ChoiceForm></xr:StandardAttribute></StandardAttributes></Properties></Catalog>");
        let actual = rename_xml(
            Path::new("Catalogs/Old.xml"),
            &original,
            "Old",
            "New",
            true,
            &code_mention_patterns("Old"),
        )
        .unwrap();
        assert_eq!(
            actual,
            String::from_utf8(original)
                .unwrap()
                .replace("<Name>Old</Name>", "<Name>New</Name>")
                .replace("Catalog.Old.Form.SelectForm", "Catalog.New.Form.SelectForm")
                .into_bytes()
        );
    }
    #[test]
    fn catalog_rename_refuses_untyped_or_malformed_form_references() {
        for property in [
            "<Unknown><DefaultObjectForm>Catalog.Old.Form.Main</DefaultObjectForm></Unknown>",
            "<DefaultObjectForm xmlns=\"urn:foreign\">Catalog.Old.Form.Main</DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Attribute.Main</DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Form.</DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Form.Main.Extra</DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Form.Not A Name</DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Form.Main²</DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Form.名</DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Form.Ma&#105;n</DefaultObjectForm>",
            "<DefaultObjectForm><![CDATA[Catalog.Old.Form.Main]]></DefaultObjectForm>",
            "<DefaultObjectForm>Catalog.Old.Form.Mai<!--split-->n</DefaultObjectForm>",
            "<StandardAttributes><StandardAttribute xmlns=\"urn:foreign\"><xr:ChoiceForm>Catalog.Old.Form.Main</xr:ChoiceForm></StandardAttribute></StandardAttributes>",
            "<StandardAttributes><xr:StandardAttribute><ChoiceForm>Catalog.Old.Form.Main</ChoiceForm></xr:StandardAttribute></StandardAttributes>",
        ] {
            let original = xml(&format!("<Catalog><Properties><Name>Old</Name>{property}</Properties></Catalog>"));
            assert!(rename_xml(Path::new("Catalogs/Old.xml"), &original, "Old", "New", true, &code_mention_patterns("Old")).is_err(), "{property}");
        }
        let original = xml("<Catalog><Properties><Name>Old</Name></Properties><ChildObjects><Attribute><Properties><ChoiceForm>Catalog.Old.Form.Main</ChoiceForm></Properties></Attribute></ChildObjects></Catalog>");
        assert!(rename_xml(
            Path::new("Catalogs/Old.xml"),
            &original,
            "Old",
            "New",
            true,
            &code_mention_patterns("Old")
        )
        .is_err());
    }
    fn xml(body: &str) -> Vec<u8> {
        format!(r#"<MetaDataObject xmlns="{MD}" xmlns:v8="{CORE}" xmlns:t="{CFG}" xmlns:xr="{XR}" xmlns:xsi="{XSI}" version="2.20">{body}</MetaDataObject>"#).into_bytes()
    }
    #[test]
    fn catalog_rename_preserves_ids_and_prose_and_renames_generated_children() {
        let original = xml(
            r#"<Catalog uuid="11111111-1111-4111-8111-111111111111"><InternalInfo><xr:GeneratedType name="CatalogRef.Old" category="Ref"><xr:TypeId>type-id</xr:TypeId><xr:ValueId>value-id</xr:ValueId></xr:GeneratedType><xr:GeneratedType name="CatalogTabularSectionRow.Old.Lines" category="TabularSectionRow"/></InternalInfo><Properties><Name>Old</Name><Synonym><v8:item><v8:content>Catalog.Old</v8:content></v8:item></Synonym><Comment>Old</Comment></Properties><ChildObjects/></Catalog>"#,
        );
        let actual = rename_xml(
            Path::new("Catalogs/Old.xml"),
            &original,
            "Old",
            "New",
            true,
            &code_mention_patterns("Old"),
        )
        .unwrap();
        let expected = String::from_utf8(original)
            .unwrap()
            .replace("<Name>Old</Name>", "<Name>New</Name>")
            .replace("CatalogRef.Old", "CatalogRef.New")
            .replace(
                "CatalogTabularSectionRow.Old.Lines",
                "CatalogTabularSectionRow.New.Lines",
            );
        assert_eq!(actual, expected.as_bytes());
    }
    #[test]
    fn catalog_rename_resolves_qname_namespace_and_keeps_similar_names() {
        let original = xml(
            r#"<Document><Properties><Name>Order</Name></Properties><ChildObjects><Attribute><Properties><Type><v8:Type>t:CatalogRef.Old</v8:Type><v8:Type>t:CatalogRef.OldLong</v8:Type></Type></Properties></Attribute></ChildObjects></Document>"#,
        );
        let actual = rename_xml(
            Path::new("Documents/Order.xml"),
            &original,
            "Old",
            "New",
            false,
            &code_mention_patterns("Old"),
        )
        .unwrap();
        assert_eq!(
            actual,
            String::from_utf8(original)
                .unwrap()
                .replace("CatalogRef.Old<", "CatalogRef.New<")
                .into_bytes()
        );
        let foreign = xml(
            r#"<Document xmlns:t="urn:foreign"><Properties><Type><v8:Type>t:CatalogRef.Old</v8:Type></Type></Properties></Document>"#,
        );
        assert!(rename_xml(
            Path::new("Documents/Order.xml"),
            &foreign,
            "Old",
            "New",
            false,
            &code_mention_patterns("Old")
        )
        .is_err());
    }
    #[test]
    fn catalog_rename_refuses_encoded_unknown_and_query_dependencies() {
        for body in [
            "<Document><Unknown>Catalog.Old</Unknown></Document>",
            "<Document xmlns:evil=\"urn:foreign\"><xr:GeneratedType evil:name=\"CatalogRef.Old\"/></Document>",
            "<Document><Unknown>Catalog.Old<Part/></Unknown></Document>",
            "<Document><Unknown><![CDATA[Catalog.Old]]><Part/></Unknown></Document>",
            "<Document><QueryText>SELECT * FROM Catalog . Old</QueryText></Document>",
            "<Document><v8:Type>t:CatalogRef.O&#108;d</v8:Type></Document>",
            "<Document><v8:Type><![CDATA[t:CatalogRef.Old]]></v8:Type></Document>",
        ] {
            assert!(
                rename_xml(
                    Path::new("Documents/Order.xml"),
                    &xml(body),
                    "Old",
                    "New",
                    false,
                    &code_mention_patterns("Old"),
                )
                .is_err(),
                "{body}"
            );
        }
        for code in [
            "Справочники . Old.Выбрать()",
            "SELECT * FROM Catalog.Old",
            "Catalogs.oLD",
            "Метаданные.Справочники.Old",
        ] {
            assert!(bare_mentions(code, "Old"), "{code}");
        }
        assert!(!bare_mentions("Catalogs.OldLong", "Old"));
    }
    #[test]
    fn catalog_rename_updates_its_standard_field_references() {
        let original = xml("<Catalog><Properties><Name>Old</Name><InputByString><xr:Field>Catalog.Old.StandardAttribute.Code</xr:Field><xr:Field>Catalog.OldLong.StandardAttribute.Code</xr:Field></InputByString></Properties></Catalog>");
        let actual = rename_xml(
            Path::new("Catalogs/Old.xml"),
            &original,
            "Old",
            "New",
            true,
            &code_mention_patterns("Old"),
        )
        .unwrap();
        let expected = String::from_utf8(original)
            .unwrap()
            .replace("<Name>Old</Name>", "<Name>New</Name>")
            .replace(
                "Catalog.Old.StandardAttribute",
                "Catalog.New.StandardAttribute",
            );
        assert_eq!(actual, expected.as_bytes());
    }
    #[test]
    fn catalog_rename_refuses_foreign_attributes_in_typed_reference_nodes() {
        let original = xml("<Catalog xmlns:evil=\"urn:foreign\"><InternalInfo><xr:GeneratedType evil:name=\"CatalogRef.Old\"/></InternalInfo><Properties><Name>Old</Name></Properties></Catalog>");
        assert!(rename_xml(
            Path::new("Catalogs/Old.xml"),
            &original,
            "Old",
            "New",
            true,
            &code_mention_patterns("Old")
        )
        .is_err());
        let dump = format!(
            r#"<ConfigDumpInfo xmlns="{DUMP}" xmlns:evil="urn:foreign" version="2.20"><Metadata evil:name="Catalog.Old"/></ConfigDumpInfo>"#
        );
        assert!(rename_xml(
            Path::new("ConfigDumpInfo.xml"),
            dump.as_bytes(),
            "Old",
            "New",
            false,
            &code_mention_patterns("Old")
        )
        .is_err());
        let command = r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:evil="urn:foreign" version="2.20"><Command evil:name="Catalog.Old.StandardCommand.OpenList"/></CommandInterface>"#;
        assert!(rename_xml(
            Path::new("Ext/CommandInterface.xml"),
            command.as_bytes(),
            "Old",
            "New",
            false,
            &code_mention_patterns("Old")
        )
        .is_err());
    }
    #[test]
    fn catalog_rename_changes_exact_configuration_registration() {
        let original = xml("<Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Catalog>Old</Catalog><Catalog>OldLong</Catalog></ChildObjects></Configuration>");
        let actual = rename_xml(
            Path::new("Configuration.xml"),
            &original,
            "Old",
            "New",
            false,
            &code_mention_patterns("Old"),
        )
        .unwrap();
        assert_eq!(
            actual,
            String::from_utf8(original)
                .unwrap()
                .replace("<Catalog>Old</Catalog>", "<Catalog>New</Catalog>")
                .into_bytes()
        );
    }
}
