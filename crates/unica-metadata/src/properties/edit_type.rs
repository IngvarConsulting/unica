//! `EditType`: its typed property specification and checks.
use super::*;

const EDIT_TYPE_VALUES: &[&str] = &["InDialog", "InList", "BothWays"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "EditType",
    "EditType",
    MetaPropertyKey::EditType,
    &[MetadataKind::Catalog, MetadataKind::InformationRegister],
    EDIT_TYPE_VALUES,
);

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn edit_type_is_a_closed_catalog_and_information_register_property() {
        for kind in MetadataKind::ALL {
            for value in ["InDialog", "InList", "BothWays"] {
                let result = MetaPropertyChanges::convert(
                    *kind,
                    vec![MetaPropertyInput::new(
                        "EditType",
                        MetaPropertyValue::String(value.into()),
                    )],
                );
                assert_eq!(
                    result.is_ok(),
                    matches!(
                        kind,
                        MetadataKind::Catalog | MetadataKind::InformationRegister
                    ),
                    "{kind:?}/{value}: {result:?}"
                );
            }
        }
        for value in ["", "Unknown", "inlist", "Both"] {
            assert!(MetaPropertyChanges::convert(
                MetadataKind::Catalog,
                vec![MetaPropertyInput::new(
                    "EditType",
                    MetaPropertyValue::String(value.into())
                )]
            )
            .is_err());
        }
    }
}
