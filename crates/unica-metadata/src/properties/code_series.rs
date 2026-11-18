//! `CodeSeries`: its typed property specification and checks.
use super::*;

const CATALOG_CODE_SERIES_VALUES: &[&str] = &[
    "WholeCatalog",
    "WithinOwnerSubordination",
    "WithinSubordination",
];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "CodeSeries",
    "CodeSeries",
    MetaPropertyKey::CodeSeries,
    CATALOG_KINDS,
    CATALOG_CODE_SERIES_VALUES,
)
.written_in_place(InPlace::Always);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn catalog_code_series_has_a_closed_value_and_owner_matrix() {
        for value in [
            "WholeCatalog",
            "WithinOwnerSubordination",
            "WithinSubordination",
        ] {
            assert_property_kind_matrix(
                "CodeSeries",
                MetaPropertyValue::String(value.into()),
                &[MetadataKind::Catalog],
            );
        }
        for value in ["WholeChartOfAccounts", "WithinOwner", "", "wholeCatalog"] {
            assert!(
                MetaPropertyChanges::convert(
                    MetadataKind::Catalog,
                    vec![MetaPropertyInput::new(
                        "CodeSeries",
                        MetaPropertyValue::String(value.into())
                    )],
                )
                .is_err(),
                "{value}"
            );
        }
        assert!(MetaPropertyChanges::convert(
            MetadataKind::Catalog,
            vec![MetaPropertyInput::new(
                "CodeSeries",
                MetaPropertyValue::Boolean(true)
            )],
        )
        .is_err());
    }
}
