//! `ExtendedEdit`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "ExtendedEdit",
    MetaPropertyKey::ExtendedEdit,
    MetaPropertyValueKind::Boolean,
    &[MetadataKind::Constant],
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn extended_edit_is_a_constant_property() {
        assert_property_kind_matrix(
            "ExtendedEdit",
            MetaPropertyValue::Boolean(true),
            &[MetadataKind::Constant],
        );
    }
}
