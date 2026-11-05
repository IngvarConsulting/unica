//! `PasswordMode`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "PasswordMode",
    MetaPropertyKey::PasswordMode,
    MetaPropertyValueKind::Boolean,
    &[MetadataKind::Constant],
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn password_mode_is_a_constant_property() {
        assert_property_kind_matrix(
            "PasswordMode",
            MetaPropertyValue::Boolean(true),
            &[MetadataKind::Constant],
        );
    }
}
