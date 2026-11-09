use super::{ProviderReadAuthority, ViewError};
use serde_json::Value;
use std::path::Path;
use unica_address::source_target::MetadataAddress;
use unica_refusal::{RefusalCode, RefusalDetail};

impl ProviderReadAuthority {
    pub(crate) fn wsreference_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        // Prove the admitted descriptor's identity before opening its companion.
        let mut payload = self.identity_metadata_payload(target)?;
        let relative = self.attached_resource_relative(target, "WSDefinition.xml")?;
        let definition = if self.open_buffer(&relative).is_none()
            && self.wsreference_companion_is_directory(&relative, checkpoint)?
        {
            unica_metadata::wsdl::Definition::unavailable(
                "local WSDL companion WSDefinition.xml is a directory; its declarations are unknown",
            )
        } else {
            match self.read_optional_relative_with_checkpoint(&relative, checkpoint)? {
                Some(bytes) => {
                    // Порт знает только сам факт остановки; её причину
                    // `ViewError` хранит этот вызов.
                    let mut stopped = None;
                    let read = crate::infrastructure::family_codecs::metadata_read_codec().wsdl(
                        &bytes,
                        &mut || {
                            checkpoint().map_err(|error| {
                                stopped = Some(error);
                                unica_metadata::read_codec::ReadStopped
                            })
                        },
                    );
                    match (read, stopped) {
                        (Ok(definition), None) => definition,
                        (_, Some(error)) => return Err(error),
                        (Err(_), None) => {
                            return Err(ViewError::new(
                                RefusalCode::InvalidResult,
                                "local WSDL read stopped without a checkpoint refusal",
                            ))
                        }
                    }
                }
                None => unica_metadata::wsdl::Definition::unavailable(
                    "local WSDL companion WSDefinition.xml is absent; its declarations are unknown",
                ),
            }
        };
        payload["wsdlDefinition"] = serde_json::to_value(definition).map_err(|error| {
            ViewError::new(
                RefusalCode::InvalidResult,
                format!("cannot serialize local WSDL observation: {error}"),
            )
        })?;
        Ok(payload)
    }

    fn wsreference_companion_is_directory(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<bool, ViewError> {
        use unica_source_contract::read::SourceEntry;

        let mut directory = self.root.as_ref().clone();
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            checkpoint()?;
            let entry = match directory.retain_immediate_child_nofollow(component.as_os_str()) {
                Ok(entry) => entry,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        error.to_string(),
                    ))
                }
            };
            checkpoint()?;
            if components.peek().is_none() {
                return Ok(matches!(entry, SourceEntry::Directory(_)));
            }
            match entry {
                SourceEntry::Directory(child) => directory = child,
                // Let the existing no-follow reader retain its precise refusal.
                _ => return Ok(false),
            }
        }
        Ok(false)
    }
}
