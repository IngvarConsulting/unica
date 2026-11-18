//! `CodeLength`: its typed property specification and checks.
use super::*;

const CODE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ExchangePlan,
];

pub(super) const SPEC: MetadataPropertySpec = property(
    "CodeLength",
    MetaPropertyKey::CodeLength,
    MetaPropertyValueKind::UnsignedInteger,
    CODE_KINDS,
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn code_length_uses_the_current_writer_kind_matrix() {
        assert_property_kind_matrix(
            "CodeLength",
            MetaPropertyValue::UnsignedInteger(9),
            &[
                MetadataKind::Catalog,
                MetadataKind::ChartOfAccounts,
                MetadataKind::ChartOfCharacteristicTypes,
                MetadataKind::ChartOfCalculationTypes,
                MetadataKind::ExchangePlan,
            ],
        );
    }
}
