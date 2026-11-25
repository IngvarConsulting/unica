use super::{ProviderReadAuthority, ViewError};
use crate::infrastructure::v13_large_configuration::ExchangeMembershipStamps;
use std::path::Path;
use unica_metadata::read_codec::ExchangeContentFacts;
use unica_refusal::RefusalDetail;
use unica_source_contract::read::FileStamp;

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

impl ProviderReadAuthority {
    /// Состав планов обмена, сохранённый прежним чтением того же дерева.
    /// Открытые буферы сеанса подменяют файлы, и стампы их не видят: при
    /// них сохранённое не используется.
    pub(crate) fn retained_exchange_membership(
        &self,
        configuration: FileStamp,
    ) -> Option<std::sync::Arc<crate::infrastructure::v13_read::ExchangeMembership>> {
        if self.documents.is_some() {
            return None;
        }
        self.artifacts
            .retained_exchange_membership(&self.root, configuration)
    }

    /// Стамп уже прочитанных регистраций и стампы файлов проверенных имён
    /// планов, снятые до чтения их состава.
    pub(crate) fn exchange_membership_stamps(
        &self,
        configuration: FileStamp,
        plans: &[String],
    ) -> Option<ExchangeMembershipStamps> {
        if self.documents.is_some() {
            return None;
        }
        let mut paths = Vec::with_capacity(plans.len() * 2);
        for plan in plans {
            let directory = Path::new("ExchangePlans");
            paths.push(directory.join(format!("{plan}.xml")));
            paths.push(directory.join(plan).join("Ext/Content.xml"));
        }
        let plans = paths
            .into_iter()
            .map(|path| {
                self.root
                    .relative_file_stamp(&path)
                    .ok()
                    .map(|stamp| (path, stamp))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(ExchangeMembershipStamps {
            configuration,
            plans,
        })
    }

    pub(crate) fn retain_exchange_membership(
        &self,
        stamps: ExchangeMembershipStamps,
        membership: std::sync::Arc<crate::infrastructure::v13_read::ExchangeMembership>,
    ) {
        self.artifacts
            .retain_exchange_membership(&self.root, stamps, membership);
    }
}
