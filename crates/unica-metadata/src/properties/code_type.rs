//! `CodeType`: its typed property specification and checks.
use super::*;

const CODE_TYPE_VALUES: &[&str] = &["String", "Number"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "CodeType",
    "CodeType",
    MetaPropertyKey::CodeType,
    CODE_TYPE_KINDS,
    CODE_TYPE_VALUES,
);

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{MetaDiagnosticCode, MetadataKind};

    #[test]
    fn code_type_keeps_its_closed_enum_and_argument_type_for_both_supported_owners() {
        for kind in [MetadataKind::Catalog, MetadataKind::ChartOfCalculationTypes] {
            for value in ["String", "Number"] {
                assert!(MetaPropertyChanges::convert(
                    kind,
                    vec![MetaPropertyInput::new(
                        "CodeType",
                        MetaPropertyValue::String(value.into())
                    )]
                )
                .is_ok());
            }
            for value in [
                MetaPropertyValue::Boolean(true),
                MetaPropertyValue::UnsignedInteger(9),
                MetaPropertyValue::String("Unknown".into()),
            ] {
                let error = MetaPropertyChanges::convert(
                    kind,
                    vec![MetaPropertyInput::new("CodeType", value)],
                )
                .expect_err("CodeType keeps its exact value kind and enum");
                assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
                assert_eq!(error.field.as_deref(), Some("values.CodeType"));
            }
        }
    }
}
