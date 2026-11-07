use super::{ProviderReadAuthority, ViewError};
use serde_json::Value;
use unica_address::source_target::MetadataAddress;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_roles_designer::info::parse_role_info_xml;

impl ProviderReadAuthority {
    pub(crate) fn role_payload(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let rights =
            self.read_relative_whole(&self.attached_resource_relative(target, "Rights.xml")?)?;
        let rights = std::str::from_utf8(&rights).map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "Rights.xml is not UTF-8")
        })?;
        let descriptor = self.metadata_descriptor(target)?;
        let descriptor = std::str::from_utf8(&descriptor).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "role descriptor is not UTF-8",
            )
        })?;
        let fallback_name = target.as_str().rsplit('.').next().unwrap_or("Role");
        let data = parse_role_info_xml(
            rights,
            Some(descriptor),
            fallback_name,
            self.object_support(target, checkpoint)?,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        serde_json::to_value(data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }
}
