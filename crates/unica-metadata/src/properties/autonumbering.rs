//! `Autonumbering`: its typed property specification and checks.
use super::*;

const AUTONUMBER_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];

pub(super) const SPEC: MetadataPropertySpec = property(
    "Autonumbering",
    MetaPropertyKey::Autonumbering,
    MetaPropertyValueKind::Boolean,
    AUTONUMBER_KINDS,
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn autonumbering_excludes_both_chart_kinds_forbidden_by_the_validator() {
        assert_property_kind_matrix(
            "Autonumbering",
            MetaPropertyValue::Boolean(true),
            &[
                MetadataKind::Catalog,
                MetadataKind::Document,
                MetadataKind::ChartOfCharacteristicTypes,
                MetadataKind::BusinessProcess,
                MetadataKind::Task,
            ],
        );
    }
}
