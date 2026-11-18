//! `Hierarchical`: its typed property specification and checks.
use super::*;

const HIERARCHICAL_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfCharacteristicTypes,
];

pub(super) const SPEC: MetadataPropertySpec = property(
    "Hierarchical",
    MetaPropertyKey::Hierarchical,
    MetaPropertyValueKind::Boolean,
    HIERARCHICAL_KINDS,
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn hierarchical_uses_the_current_writer_and_validator_kind_matrix() {
        assert_property_kind_matrix(
            "Hierarchical",
            MetaPropertyValue::Boolean(true),
            &[
                MetadataKind::Catalog,
                MetadataKind::ChartOfCharacteristicTypes,
            ],
        );
    }
}
