use unica_format_core::{
    ports::{ValidationContextPort, ValidationContextRequest, ValidationContextResult},
    source::SourceAdapterError,
};

use crate::versions::v2_20;

pub(crate) struct PlatformXmlValidation;

impl ValidationContextPort for PlatformXmlValidation {
    fn inspect(
        &self,
        request: &ValidationContextRequest,
    ) -> Result<ValidationContextResult, SourceAdapterError> {
        let session = v2_20::operations::session_from_handle(request.session())?;
        Ok(v2_20::operations::validation(session))
    }
}
