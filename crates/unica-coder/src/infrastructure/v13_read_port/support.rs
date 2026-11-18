use super::{ProviderReadAuthority, ViewError};
use std::path::Path;
use std::sync::Arc;
use unica_address::source_target::MetadataAddress;
use unica_metadata::support_state::{
    ConfigurationSupportData, ConfigurationSupportState, ObjectSupportData, ObjectSupportState,
    SupportCounts, SupportState,
};
use unica_metadata::MetaSupportStatus;
use unica_refusal::RefusalDetail;
use unica_source_contract::SourceSetKind;

impl ProviderReadAuthority {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn support_state_read_count(&self) -> usize {
        self.support_state_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn object_support(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<ObjectSupportData, ViewError> {
        self.object_support_with_descriptor(target, None, checkpoint)
    }

    /// The parsed support marker of the source set; `None` when it is absent.
    pub(crate) fn support_marker(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<Arc<SupportState>>, ViewError> {
        self.support_state(checkpoint)
    }

    /// A caller that already holds the descriptor bytes passes them in so the
    /// support lookup does not read the same file a second time.
    pub(super) fn object_support_with_descriptor(
        &self,
        target: &MetadataAddress,
        descriptor: Option<&[u8]>,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<ObjectSupportData, ViewError> {
        let Some(state) = self.support_state(checkpoint)? else {
            return Ok(unsupported_object());
        };
        // Preserve the established early global answer for other readers:
        // only the form-specific own-rule projection requires UUID evidence
        // even when the global state already determines effective support.
        if state.removed() || !state.global_editing_enabled() {
            return Ok(object_support_from_state(Some(&state), None));
        }
        let owned;
        let descriptor = match descriptor {
            Some(bytes) => bytes,
            None => {
                owned = self.metadata_descriptor(target)?;
                owned.as_slice()
            }
        };
        let uuid = crate::infrastructure::family_codecs::metadata_read_codec()
            .support_root_uuid(descriptor)
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "metadata descriptor has no support-state UUID evidence",
                )
            })?;
        Ok(object_support_from_state(
            Some(&state),
            object_support_rule(state.object_rule(&uuid)),
        ))
    }

    pub(super) fn configuration_support(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<ConfigurationSupportData, ViewError> {
        let Some(state) = self.support_state(checkpoint)? else {
            return Ok(ConfigurationSupportData {
                state: if self.source_set_kind == SourceSetKind::Extension {
                    ConfigurationSupportState::Extension
                } else {
                    ConfigurationSupportState::NotSupported
                },
                editing_enabled: None,
                objects: None,
            });
        };
        if state.removed() {
            return Ok(ConfigurationSupportData {
                state: ConfigurationSupportState::Removed,
                editing_enabled: None,
                objects: None,
            });
        }
        let counts = state.counts();
        Ok(ConfigurationSupportData {
            state: ConfigurationSupportState::Supported,
            editing_enabled: Some(state.global_editing_enabled()),
            objects: Some(SupportCounts {
                locked: counts[0] as u64,
                editable: counts[1] as u64,
                removed: counts[2] as u64,
            }),
        })
    }

    pub(super) fn support_state(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<Arc<SupportState>>, ViewError> {
        checkpoint()?;
        let mut memo = self.support_state.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "support-state cache is poisoned",
            )
        })?;
        // The memo answers only for the marker stamp it was read under: a
        // marker replaced during this operation is read again, so a payload
        // built later never combines a new stamp with an old state.
        if let Some((stamp, state)) = memo.as_ref() {
            let current = self
                .root
                .relative_file_stamp(Path::new("Ext/ParentConfigurations.bin"))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
            if current == *stamp {
                checkpoint()?;
                return Ok(state.clone());
            }
        }
        let (stamp, state) =
            self.artifacts
                .support_state(&self.root, checkpoint, |file, checkpoint| {
                    #[cfg(any(test, feature = "test-support"))]
                    self.support_state_reads
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    parse_support_state(file, checkpoint)
                })?;
        checkpoint()?;
        *memo = Some((stamp, state.clone()));
        Ok(state)
    }
}

/// Support state of an object whose descriptor `uuid` the caller already
/// holds: a collection lists its members without reading them again.
pub(crate) fn object_support_by_uuid(
    state: &SupportState,
    uuid: Option<&str>,
) -> Result<ObjectSupportState, ViewError> {
    if state.removed() || !state.global_editing_enabled() {
        return Ok(object_support_from_state(Some(state), None).state);
    }
    let uuid = uuid.ok_or_else(|| {
        ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            "metadata descriptor has no support-state UUID evidence",
        )
    })?;
    Ok(object_support_from_state(Some(state), object_support_rule(state.object_rule(uuid))).state)
}

pub(super) fn object_support_rule(rule: Option<u8>) -> Option<ObjectSupportState> {
    match rule {
        Some(0) => Some(ObjectSupportState::Locked),
        Some(1) => Some(ObjectSupportState::EditableWithSupport),
        Some(2) => Some(ObjectSupportState::RemovedFromSupport),
        _ => None,
    }
}

pub(super) fn object_support_from_state(
    state: Option<&SupportState>,
    own_rule: Option<ObjectSupportState>,
) -> ObjectSupportData {
    let Some(state) = state else {
        return unsupported_object();
    };
    let state = if state.removed() {
        ObjectSupportState::RemovedFromSupport
    } else if !state.global_editing_enabled() {
        ObjectSupportState::ConfigurationReadOnly
    } else {
        own_rule.unwrap_or(ObjectSupportState::NotSupported)
    };
    let direct_edit_safe = match state {
        ObjectSupportState::Locked | ObjectSupportState::ConfigurationReadOnly => Some(false),
        ObjectSupportState::EditableWithSupport | ObjectSupportState::RemovedFromSupport => {
            Some(true)
        }
        ObjectSupportState::NotSupported => None,
    };
    ObjectSupportData {
        state,
        direct_edit_safe,
    }
}

fn unsupported_object() -> ObjectSupportData {
    ObjectSupportData {
        state: ObjectSupportState::NotSupported,
        direct_edit_safe: None,
    }
}

pub(super) fn metadata_support_status(support: ObjectSupportData) -> MetaSupportStatus {
    match support.state {
        ObjectSupportState::Locked | ObjectSupportState::ConfigurationReadOnly => {
            MetaSupportStatus::Locked
        }
        ObjectSupportState::RemovedFromSupport => MetaSupportStatus::Unsupported,
        ObjectSupportState::EditableWithSupport | ObjectSupportState::NotSupported => {
            MetaSupportStatus::Supported
        }
    }
}

/// Streams `Ext/ParentConfigurations.bin` through the support-state parser in
/// bounded chunks, observing the caller's checkpoint before each chunk.
fn parse_support_state(
    mut file: impl std::io::Read,
    checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
) -> Result<SupportState, ViewError> {
    let mut parser = crate::infrastructure::family_codecs::metadata_read_codec().support_stream();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        checkpoint()?;
        let count = match file.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?,
        };
        if count == 0 {
            break;
        }
        parser.feed(&buffer[..count]).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
    }
    parser
        .finish()
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))
}
