//! `Mask`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = property(
    "Mask",
    MetaPropertyKey::Mask,
    MetaPropertyValueKind::String,
    &[MetadataKind::Constant],
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn mask_is_a_constant_property() {
        assert_property_kind_matrix(
            "Mask",
            MetaPropertyValue::String(String::new()),
            &[MetadataKind::Constant],
        );
    }
}
