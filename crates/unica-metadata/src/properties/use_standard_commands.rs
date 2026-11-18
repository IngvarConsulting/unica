//! `UseStandardCommands`: its typed property specification and checks.
use super::*;

const STANDARD_COMMAND_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::Enum,
    MetadataKind::Constant,
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
    "UseStandardCommands",
    MetaPropertyKey::UseStandardCommands,
    MetaPropertyValueKind::Boolean,
    STANDARD_COMMAND_KINDS,
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn use_standard_commands_uses_the_current_writer_kind_matrix() {
        assert_property_kind_matrix(
            "UseStandardCommands",
            MetaPropertyValue::Boolean(true),
            &[
                MetadataKind::Catalog,
                MetadataKind::Document,
                MetadataKind::Enum,
                MetadataKind::Constant,
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
    }
}
