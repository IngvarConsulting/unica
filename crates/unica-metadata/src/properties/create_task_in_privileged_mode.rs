//! `CreateTaskInPrivilegedMode`: its typed property specification and checks.
use super::*;

const BUSINESS_PROCESS_KINDS: &[MetadataKind] = &[MetadataKind::BusinessProcess];

pub(super) const SPEC: MetadataPropertySpec = property(
    "CreateTaskInPrivilegedMode",
    MetaPropertyKey::CreateTaskInPrivilegedMode,
    MetaPropertyValueKind::Boolean,
    BUSINESS_PROCESS_KINDS,
);

#[cfg(test)]
mod tests {
    use super::super::tests::assert_property_kind_matrix;
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn create_task_privileged_mode_is_boolean_and_business_process_only() {
        for value in [false, true] {
            assert_property_kind_matrix(
                "CreateTaskInPrivilegedMode",
                MetaPropertyValue::Boolean(value),
                &[MetadataKind::BusinessProcess],
            );
        }
        assert!(MetaPropertyChanges::convert(
            MetadataKind::BusinessProcess,
            vec![MetaPropertyInput::new(
                "CreateTaskInPrivilegedMode",
                MetaPropertyValue::String("true".into())
            )],
        )
        .is_err());
    }
}
