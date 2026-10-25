use super::{ProviderReadAuthority, ViewError};
use serde_json::{json, Value};
use unica_address::source_target::MetadataAddress;
use unica_metadata_designer::job_schedule::parse_job_schedule;
use unica_refusal::RefusalDetail;

impl ProviderReadAuthority {
    pub(crate) fn job_schedule(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        checkpoint()?;
        let relative = self.attached_resource_relative(target, "Schedule.xml")?;
        // Сопутствующий файл читается целиком, как и прочие (#2104).
        let bytes = self.read_optional_relative_with_checkpoint(&relative, checkpoint)?;
        let Some(bytes) = bytes else {
            return Ok(json!({"state":"absent"}));
        };
        let text =
            std::str::from_utf8(&bytes).map_err(|_| unreadable("Schedule.xml is not UTF-8"))?;
        let result = parse_job_schedule(text).map_err(unreadable);
        checkpoint()?;
        result
    }
}

fn unreadable(message: impl Into<String>) -> ViewError {
    ViewError::detailed(RefusalDetail::SourceUnreadable, message)
}
