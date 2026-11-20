use super::{ProviderReadAuthority, ViewError};
use std::path::Path;
use unica_metadata::read_codec::ExchangeContentFacts;
use unica_refusal::RefusalDetail;

impl ProviderReadAuthority {
    /// Состав плана обмена `plan` из `ExchangePlans/<plan>/Ext/Content.xml`.
    ///
    /// `Ok(None)` — файла нет: платформа не пишет его для пустого состава,
    /// а заимствованный план без файла несёт состав родителя. Внутренний
    /// `Err` — файл есть, но не читается; отмена и срок остаются внешней
    /// ошибкой со своими кодами.
    pub(crate) fn exchange_plan_content(
        &self,
        plan: &str,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Result<Option<ExchangeContentFacts>, String>, ViewError> {
        let relative = Path::new("ExchangePlans")
            .join(plan)
            .join("Ext/Content.xml");
        match self.read_optional_relative_with_checkpoint(&relative, checkpoint) {
            Ok(Some(bytes)) => Ok(std::str::from_utf8(&bytes)
                .map_err(|error| error.to_string())
                .and_then(|text| {
                    crate::infrastructure::family_codecs::metadata_read_codec()
                        .exchange_content(text)
                })
                .map(Some)),
            Ok(None) => Ok(Ok(None)),
            Err(error) if matches!(error.detail(), Some(RefusalDetail::SourceUnreadable)) => {
                Ok(Err(error.to_string()))
            }
            Err(error) => Err(error),
        }
    }
}
