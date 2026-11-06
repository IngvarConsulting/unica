//! `EditFormat`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "EditFormat",
    MetaPropertyKey::EditFormat,
    MetaPropertyValueKind::String,
    &[MetadataKind::Constant],
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn edit_format_is_a_constant_property() {
        assert_property_kind_matrix(
            "EditFormat",
            MetaPropertyValue::String(String::new()),
            &[MetadataKind::Constant],
        );
    }
}
