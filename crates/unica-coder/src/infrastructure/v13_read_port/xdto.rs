use super::{ProviderReadAuthority, ViewError};
use serde_json::{json, Value};
use unica_address::source_target::MetadataAddress;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_xdto_designer::info::{parse_xdto_info_bytes, XdtoInfoError};

impl ProviderReadAuthority {
    pub(crate) fn xdto_check_outcome(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<crate::application::v13::check::NativeCheckOutcome, ViewError> {
        checkpoint()?;
        let descriptor = self.metadata_descriptor(target)?;
        let package = self.read_relative_with_checkpoint(
            &self.attached_resource_relative(target, "Package.bin")?,
            checkpoint,
        )?;
        let data = parse_xdto_info_bytes(&package, &descriptor, self.source_set(), target, None)
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        checkpoint()?;
        Ok(crate::infrastructure::native_operations::xdto::xdto_check_outcome(data))
    }

    pub(crate) fn xdto_payload(
        &self,
        target: &MetadataAddress,
        type_name: Option<&str>,
    ) -> Result<Value, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let package =
            self.read_relative_whole(&self.attached_resource_relative(target, "Package.bin")?)?;
        let data =
            parse_xdto_info_bytes(&package, &descriptor, self.source_set(), target, type_name)
                .map_err(|error| {
                    let message = error.to_string();
                    match error {
                        XdtoInfoError::TypeNotFound(_) => {
                            ViewError::new(RefusalCode::NotFound, message)
                        }
                        XdtoInfoError::Source(_) => {
                            ViewError::detailed(RefusalDetail::SourceUnreadable, message)
                        }
                    }
                })?;
        let mut payload = serde_json::to_value(data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))?;
        if type_name.is_none() {
            match crate::infrastructure::family_codecs::metadata_read_codec()
                .descriptor_comment(&descriptor, "XDTOPackage")
            {
                Ok(Some(comment)) => payload["Comment"] = json!(comment),
                Ok(None) => {}
                Err(message) => payload["commentError"] = json!(message),
            }
        }
        Ok(payload)
    }
}
