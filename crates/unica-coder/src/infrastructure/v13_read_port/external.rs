use super::{identity_metadata_payload_from_evidence, ProviderReadAuthority, ViewError};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use unica_address::source_target::MetadataAddress;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_designer::owner::{prove_already_read_source_set_owner, MD_CLASSES_NS};

impl ProviderReadAuthority {
    fn external_inventory_with_checkpoint(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Arc<unica_source_designer::external_artifact::ExternalArtifactInventory>, ViewError>
    {
        checkpoint()?;
        if let Some(inventory) = self
            .external_artifacts
            .lock()
            .map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::CachePoisoned,
                    "external inventory cache is poisoned",
                )
            })?
            .as_ref()
            .cloned()
        {
            return Ok(inventory);
        }
        let inventory = Arc::new(
            crate::infrastructure::external_artifact_inventory::read_inventory(
                &self.root,
                self.source_set_kind,
                checkpoint,
            )?,
        );
        checkpoint()?;
        // An empty root cannot establish an external source inventory. Do not
        // retain that negative observation as a successful owner mapping.
        if inventory.is_empty() {
            return Ok(inventory);
        }
        *self.external_artifacts.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "external inventory cache is poisoned",
            )
        })? = Some(Arc::clone(&inventory));
        Ok(inventory)
    }

    pub(super) fn external_inventory_payload(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let inventory = self.external_inventory_with_checkpoint(checkpoint)?;
        if inventory.is_empty() {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "external source set has no valid top-level owner descriptors",
            ));
        }
        Ok(json!({"format":"PlatformXml","name":self.source_set,
            "registeredObjects":inventory.iter().map(|binding|json!({"kind":binding.kind(),"name":binding.name()})).collect::<Vec<_>>()
        }))
    }

    pub(super) fn external_physical_path(
        &self,
        target: &MetadataAddress,
        canonical: PathBuf,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<PathBuf, ViewError> {
        let (_, name, _) =
            unica_source_designer::layout::external_owner_parts(target, self.source_set_kind)
                .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        if let Some(binding) = &self.point_external_artifact {
            if binding.name() != name {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "target is outside the point-proven external owner",
                ));
            }
            checkpoint()?;
            return binding
                .physical_path(&canonical)
                .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message));
        }
        let was_cached = self
            .external_artifacts
            .lock()
            .map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::CachePoisoned,
                    "external inventory cache is poisoned",
                )
            })?
            .is_some();
        let mut inventory = self.external_inventory_with_checkpoint(checkpoint)?;
        if was_cached && inventory.get(name).is_none() {
            // Positive bindings do not prove the absence of a later descriptor.
            // Refresh on a miss before deciding absent versus unreadable.
            *self.external_artifacts.lock().map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::CachePoisoned,
                    "external inventory cache is poisoned",
                )
            })? = None;
            inventory = self.external_inventory_with_checkpoint(checkpoint)?;
        }
        let binding = inventory.get(name).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("external owner `{}` is not registered", target.as_str()),
            )
        })?;
        binding
            .physical_path(&canonical)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))
    }

    pub(crate) fn external_metadata_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<Value>, ViewError> {
        if !self.is_external_source_set() {
            return Ok(None);
        }
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let [expected_kind, expected_name] = parts.as_slice() else {
            return Err(ViewError::new(
                RefusalCode::ProviderUnavailable,
                "external typed metadata target must name its root owner",
            ));
        };
        let descriptor = self.metadata_descriptor_with_checkpoint(target, checkpoint)?;
        let relative = self.metadata_descriptor_relative(target)?;
        let evidence =
            prove_already_read_source_set_owner(&relative, &descriptor, self.source_set_kind)
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.message)
                })?;
        if evidence.artifact_kind() != *expected_kind
            || evidence.artifact_name() != Some(*expected_name)
        {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!(
                    "external descriptor identity does not match `{}`",
                    target.as_str()
                ),
            ));
        }
        let mut payload = identity_metadata_payload_from_evidence(target, &evidence)?;
        project_inline_members(&descriptor, target, checkpoint, &mut payload)?;
        Ok(Some(payload))
    }
}

fn project_inline_members(
    descriptor: &[u8],
    target: &MetadataAddress,
    checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    payload: &mut Value,
) -> Result<(), ViewError> {
    use unica_metadata_designer::info::typed_elements_with_diagnostics;

    let invalid = |message| {
        ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            format!("invalid inline metadata in external descriptor {target}: {message}"),
        )
    };
    checkpoint()?;
    let text = std::str::from_utf8(descriptor).map_err(|error| invalid(error.to_string()))?;
    let xml = text.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(xml).map_err(|error| invalid(error.to_string()))?;
    // The same bytes have already established the descriptor's owner.
    let owner = document
        .root_element()
        .first_element_child()
        .ok_or_else(|| invalid("missing owner".into()))?;
    let mut uuids = owner
        .attribute("uuid")
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .into_iter()
        .collect();
    let children = direct_md_container(owner, "ChildObjects").map_err(invalid)?;
    if let Some(children) = children {
        validate_inline_members(children, true, &mut uuids, target, checkpoint)?;
    }

    let mut diagnostics = Vec::new();
    for (tag, key, nested) in [
        ("Attribute", "attributes", false),
        ("TabularSection", "tabularSections", true),
    ] {
        checkpoint()?;
        let members = typed_elements_with_diagnostics(
            xml,
            children,
            tag,
            nested,
            &format!("collections.{key}"),
            target,
            &mut diagnostics,
        );
        payload["collections"][key] =
            serde_json::to_value(members).map_err(|error| invalid(error.to_string()))?;
    }
    let unreadable: Vec<_> = diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == unica_metadata::MetaDiagnosticSeverity::Error)
        .filter_map(|diagnostic| {
            let field = diagnostic.field?;
            Some(json!({"field": field, "message": diagnostic.message}))
        })
        .collect();
    if !unreadable.is_empty() {
        payload["unreadable"] = json!(unreadable);
    }
    checkpoint()
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
    target: &MetadataAddress,
    checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
) -> Result<(), ViewError> {
    use unica_metadata::identifier::is_1c_identifier;
    use unica_metadata_designer::info_projection::{
        direct_text_content, strict_text_leaf_is_valid,
    };

    let invalid = |message| {
        ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            format!("invalid inline metadata in external descriptor {target}: {message}"),
        )
    };
    let mut names = BTreeSet::new();
    for member in children.children().filter(roxmltree::Node::is_element) {
        let tag = member.tag_name().name();
        if tag != "Attribute" && !(allow_sections && tag == "TabularSection") {
            continue;
        }
        checkpoint()?;
        if !member.has_tag_name((MD_CLASSES_NS, tag)) {
            return Err(invalid(format!("foreign {tag}")));
        }
        let properties = direct_md_container(member, "Properties")
            .map_err(invalid)?
            .ok_or_else(|| invalid(format!("{tag} has no Properties")))?;
        let name = direct_md_container(properties, "Name")
            .map_err(invalid)?
            .filter(|node| strict_text_leaf_is_valid(*node))
            .map(direct_text_content)
            .filter(|name| is_1c_identifier(name))
            .ok_or_else(|| invalid(format!("{tag} has no unique valid Name")))?;
        let uuid = member
            .attribute("uuid")
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .ok_or_else(|| invalid(format!("{tag}.{name} has no valid UUID")))?;
        if !uuids.insert(uuid) || !names.insert((tag, name.to_lowercase())) {
            return Err(invalid(format!("duplicate identity for {tag}.{name}")));
        }
        if tag == "TabularSection" {
            if let Some(columns) = direct_md_container(member, "ChildObjects").map_err(invalid)? {
                validate_inline_members(columns, false, uuids, target, checkpoint)?;
            }
        }
    }
    Ok(())
}
