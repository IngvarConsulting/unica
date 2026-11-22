use super::{identity_metadata_payload_from_evidence, ProviderReadAuthority, ViewError};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use unica_address::source_target::MetadataAddress;
use unica_metadata::read_codec::{ExternalInlineReadError, ReadStopped};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_designer::owner::prove_already_read_source_set_owner;

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
    let mut stopped = None;
    let read = crate::infrastructure::family_codecs::metadata_read_codec().external_inline_members(
        descriptor,
        target,
        &mut || {
            checkpoint().map_err(|error| {
                stopped = Some(error);
                ReadStopped
            })
        },
    );
    let invalid = |message| {
        ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            format!("invalid inline metadata in external descriptor {target}: {message}"),
        )
    };
    let facts = match (read, stopped) {
        (Ok(facts), None) => facts,
        (_, Some(error)) => return Err(error),
        (Err(ExternalInlineReadError::InvalidDescriptor(message)), None) => {
            return Err(invalid(message))
        }
        (Err(ExternalInlineReadError::Stopped), None) => {
            return Err(ViewError::new(
                RefusalCode::InvalidResult,
                "external inline metadata read stopped without a checkpoint refusal",
            ));
        }
    };
    payload["collections"]["attributes"] =
        serde_json::to_value(facts.attributes).map_err(|error| invalid(error.to_string()))?;
    payload["collections"]["tabularSections"] =
        serde_json::to_value(facts.tabular_sections).map_err(|error| invalid(error.to_string()))?;
    if !facts.unreadable.is_empty() {
        payload["unreadable"] = json!(facts
            .unreadable
            .into_iter()
            .map(|finding| { json!({"field": finding.field, "message": finding.message}) })
            .collect::<Vec<_>>());
    }
    checkpoint()
}
