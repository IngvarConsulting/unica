use unica_metadata::MetadataKind;
use unica_source_designer::layout::{metadata_kind, MetadataLayout};

pub fn metadata_layout(kind: MetadataKind) -> &'static MetadataLayout {
    metadata_kind(kind.as_str()).expect("every domain metadata kind must have a physical layout")
}

/// Platform XML value-type spellings of a metadata kind.
///
/// An event subscription names its source as a *type*, not as a metadata path:
/// `Catalog.Товары` appears there as `CatalogObject.Товары` or
/// `CatalogManager.Товары`, and inside a defined type as `CatalogRef.Товары`.
/// Which spelling is used depends on the position, so a reader that knows only
/// one of them misses matches without saying so. Measured on an 8.3.27
/// vendor-class dump: reference spellings occur only inside defined types, and
/// 26% of subscriptions reach their source through one.
///
/// A kind that cannot be the value of a type reference — a common module, an
/// HTTP service, a subscription itself — answers with an empty slice.
pub fn metadata_kind_value_types(kind: MetadataKind) -> &'static [&'static str] {
    match kind {
        MetadataKind::Catalog => &["CatalogObject", "CatalogRef", "CatalogManager"],
        MetadataKind::Document => &["DocumentObject", "DocumentRef", "DocumentManager"],
        MetadataKind::Enum => &["EnumRef", "EnumManager"],
        MetadataKind::Constant => &["ConstantValueManager"],
        MetadataKind::InformationRegister => {
            &["InformationRegisterRecordSet", "InformationRegisterManager"]
        }
        MetadataKind::AccumulationRegister => &[
            "AccumulationRegisterRecordSet",
            "AccumulationRegisterManager",
        ],
        MetadataKind::AccountingRegister => {
            &["AccountingRegisterRecordSet", "AccountingRegisterManager"]
        }
        MetadataKind::CalculationRegister => {
            &["CalculationRegisterRecordSet", "CalculationRegisterManager"]
        }
        MetadataKind::ChartOfAccounts => &[
            "ChartOfAccountsObject",
            "ChartOfAccountsRef",
            "ChartOfAccountsManager",
        ],
        MetadataKind::ChartOfCharacteristicTypes => &[
            "ChartOfCharacteristicTypesObject",
            "ChartOfCharacteristicTypesRef",
            "ChartOfCharacteristicTypesManager",
        ],
        MetadataKind::ChartOfCalculationTypes => &[
            "ChartOfCalculationTypesObject",
            "ChartOfCalculationTypesRef",
            "ChartOfCalculationTypesManager",
        ],
        MetadataKind::BusinessProcess => &[
            "BusinessProcessObject",
            "BusinessProcessRef",
            "BusinessProcessManager",
        ],
        MetadataKind::Task => &["TaskObject", "TaskRef", "TaskManager"],
        MetadataKind::ExchangePlan => &[
            "ExchangePlanObject",
            "ExchangePlanRef",
            "ExchangePlanManager",
        ],
        MetadataKind::DocumentJournal => &["DocumentJournalManager"],
        MetadataKind::Report => &["ReportObject", "ReportManager"],
        MetadataKind::DataProcessor => &["DataProcessorObject", "DataProcessorManager"],
        MetadataKind::DefinedType => &["DefinedType"],
        MetadataKind::CommonModule
        | MetadataKind::ScheduledJob
        | MetadataKind::EventSubscription
        | MetadataKind::HTTPService
        | MetadataKind::WebService => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_metadata::MetadataKind as DomainMetadataKind;
    #[test]
    fn every_creation_kind_keys_one_physical_layout() {
        for kind in DomainMetadataKind::ALL {
            let layout = metadata_layout(*kind);
            assert_eq!(layout.tag, kind.as_str());
            assert!(!layout.directory.is_empty());
        }
    }

    #[test]
    fn every_kind_states_its_value_type_spellings() {
        // The table is what a subscription reader matches against, so a kind
        // added without an entry would silently stop matching rather than fail
        // to compile. Object and record-set spellings are what `<Source>`
        // carries; the reference spelling is what a defined type carries, and
        // dropping it loses every indirect source.
        for kind in MetadataKind::ALL.iter().copied() {
            let spellings = metadata_kind_value_types(kind);
            for spelling in spellings {
                assert!(
                    spelling.starts_with(kind.as_str())
                        || matches!(kind, MetadataKind::Constant | MetadataKind::Enum),
                    "{}: {spelling} does not name its own kind",
                    kind.as_str()
                );
            }
            let writable_value = !matches!(
                kind,
                MetadataKind::CommonModule
                    | MetadataKind::ScheduledJob
                    | MetadataKind::EventSubscription
                    | MetadataKind::HTTPService
                    | MetadataKind::WebService
            );
            assert_eq!(
                !spellings.is_empty(),
                writable_value,
                "{} states the wrong value-type presence",
                kind.as_str()
            );
        }

        assert_eq!(
            metadata_kind_value_types(MetadataKind::Catalog),
            &["CatalogObject", "CatalogRef", "CatalogManager"]
        );
        assert_eq!(
            metadata_kind_value_types(MetadataKind::InformationRegister),
            &["InformationRegisterRecordSet", "InformationRegisterManager"]
        );
        assert!(metadata_kind_value_types(MetadataKind::CommonModule).is_empty());
    }
}
