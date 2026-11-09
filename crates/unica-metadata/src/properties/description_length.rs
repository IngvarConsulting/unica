//! `DescriptionLength`: its typed property specification and checks.
use super::*;

const DESCRIPTION_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
];

pub(super) const SPEC: MetadataPropertySpec = property(
    "DescriptionLength",
    MetaPropertyKey::DescriptionLength,
    MetaPropertyValueKind::UnsignedInteger,
    DESCRIPTION_KINDS,
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn description_length_uses_its_distinct_current_writer_kind_matrix() {
        assert_property_kind_matrix(
            "DescriptionLength",
            MetaPropertyValue::UnsignedInteger(100),
            &[
                MetadataKind::Catalog,
                MetadataKind::ChartOfAccounts,
                MetadataKind::ChartOfCharacteristicTypes,
                MetadataKind::ChartOfCalculationTypes,
                MetadataKind::Task,
                MetadataKind::ExchangePlan,
            ],
        );
    }
}
