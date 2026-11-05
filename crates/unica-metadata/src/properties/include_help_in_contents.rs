//! `IncludeHelpInContents`: its typed property specification and checks.
use super::*;

const INCLUDE_HELP_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
    MetadataKind::DocumentJournal,
    MetadataKind::Report,
    MetadataKind::DataProcessor,
];

pub(super) const SPEC: MetadataPropertySpec = property(
    "IncludeHelpInContents",
    MetaPropertyKey::IncludeHelpInContents,
    MetaPropertyValueKind::Boolean,
    INCLUDE_HELP_KINDS,
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::{MetaDiagnosticCode, MetadataKind};

    #[test]
    fn include_help_in_contents_follows_the_8_3_27_dump_kind_matrix() {
        assert_property_kind_matrix(
            "IncludeHelpInContents",
            MetaPropertyValue::Boolean(true),
            &[
                MetadataKind::Catalog,
                MetadataKind::Document,
                MetadataKind::InformationRegister,
                MetadataKind::AccumulationRegister,
                MetadataKind::AccountingRegister,
                MetadataKind::CalculationRegister,
                MetadataKind::ChartOfAccounts,
                MetadataKind::ChartOfCharacteristicTypes,
                MetadataKind::ChartOfCalculationTypes,
                MetadataKind::BusinessProcess,
                MetadataKind::Task,
                MetadataKind::ExchangePlan,
                MetadataKind::DocumentJournal,
                MetadataKind::Report,
                MetadataKind::DataProcessor,
            ],
        );

        let diagnostic = MetaPropertyChanges::convert(
            MetadataKind::Enum,
            vec![MetaPropertyInput::new(
                "IncludeHelpInContents",
                MetaPropertyValue::Boolean(true),
            )],
        )
        .expect_err("an enum descriptor has no help-contents flag");
        assert_eq!(diagnostic.code, MetaDiagnosticCode::UnsupportedKind);
        assert_eq!(
            diagnostic.message,
            "property `IncludeHelpInContents` is not supported for Enum"
        );
        assert_eq!(
            diagnostic.field.as_deref(),
            Some("values.IncludeHelpInContents")
        );
    }
}
