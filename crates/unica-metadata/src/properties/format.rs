//! `Format`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Format",
    MetaPropertyKey::Format,
    MetaPropertyValueKind::String,
    &[MetadataKind::Constant],
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn format_is_a_constant_property() {
        assert_property_kind_matrix(
            "Format",
            MetaPropertyValue::String(String::new()),
            &[MetadataKind::Constant],
        );
    }
}
