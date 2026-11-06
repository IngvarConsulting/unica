//! `MultiLine`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "MultiLine",
    MetaPropertyKey::MultiLine,
    MetaPropertyValueKind::Boolean,
    &[MetadataKind::Constant],
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn multi_line_is_a_constant_property() {
        assert_property_kind_matrix(
            "MultiLine",
            MetaPropertyValue::Boolean(true),
            &[MetadataKind::Constant],
        );
    }
}
