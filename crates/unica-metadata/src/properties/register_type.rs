//! `RegisterType`: its typed property specification and checks.
use super::*;

const REGISTER_TYPE_VALUES: &[&str] = &["Balance", "Turnovers"];

pub(super) const SPEC: MetadataPropertySpec = enum_property(
    "RegisterType",
    "RegisterType",
    MetaPropertyKey::RegisterType,
    ACCUMULATION_REGISTER_KINDS,
    REGISTER_TYPE_VALUES,
);

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{MetaDiagnosticCode, MetadataKind};

    #[test]
    fn register_type_accepts_only_the_platform_enumeration() {
        for accepted in ["Balance", "Turnovers"] {
            let changes = MetaPropertyChanges::convert(
                MetadataKind::AccumulationRegister,
                vec![MetaPropertyInput::new(
                    "RegisterType",
                    MetaPropertyValue::String(accepted.to_string()),
                )],
            )
            .unwrap();
            assert_eq!(
                changes.entries(),
                &[(
                    MetaPropertyKey::RegisterType,
                    MetaPropertyValue::String(accepted.to_string()),
                )]
            );
        }
        // `Balances` is a documentation typo, not a platform value.
        let diagnostic = MetaPropertyChanges::convert(
            MetadataKind::AccumulationRegister,
            vec![MetaPropertyInput::new(
                "RegisterType",
                MetaPropertyValue::String("Balances".to_string()),
            )],
        )
        .expect_err("Balances is not an AccumulationRegisterType member");
        assert_eq!(diagnostic.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(
            diagnostic.message,
            "property `RegisterType` value `Balances` is invalid; expected one of: Balance, Turnovers"
        );
    }
}
