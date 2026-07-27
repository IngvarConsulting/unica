use unica_format_core::{
    ports::{
        OperationalValidationPort, OperationalValidationRequest, OperationalValidationResult,
        ValidationContextPort, ValidationContextRequest, ValidationContextResult,
    },
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

impl OperationalValidationPort for PlatformXmlValidation {
    fn validate(
        &self,
        request: &OperationalValidationRequest,
    ) -> Result<OperationalValidationResult, SourceAdapterError> {
        v2_20::validation::validate(request.sessions(), request.options())
    }
}
