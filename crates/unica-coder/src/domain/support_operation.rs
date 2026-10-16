use super::apply::{OperationApplicability, OperationSkeleton};
use super::operation_contract::{object, text, OperationContract};
use serde_json::json;

macro_rules! support_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum SupportOperation {
            $($variant),+
        }

        impl SupportOperation {
            pub(crate) const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            pub(super) const fn applicability(self) -> OperationApplicability {
                match self {
                    $(Self::$variant => OperationApplicability::$applicability),+
                }
            }

            pub(super) const fn skeleton(self) -> OperationSkeleton {
                match self {
                    $(Self::$variant => OperationSkeleton::$skeleton),+
                }
            }
        }
    };
}

support_operations!(
    CapabilitySet => ("supportCapability.set", Configuration, Values),
    RuleSet => ("supportRule.set", SupportRule, Values),
);

impl SupportOperation {
    pub(crate) const fn changes_support_state(self) -> bool {
        match self {
            Self::CapabilitySet | Self::RuleSet => true,
        }
    }

    pub(crate) fn contract(self) -> OperationContract {
        let (values, target, effect, notes, example_args) = match self {
            Self::CapabilitySet => (
                json!({"anyOf":[
                    object(&[("enabled",json!({"type":"boolean"}))], &["enabled"]),
                    object(&[("rule",json!({"type":"string","enum":["on","off","EditableSupportEnabled","EditableSupportDisabled","enabled","disabled"]}))], &["rule"])
                ]}),
                "The configuration root: <source-set>:Configuration.",
                "Switch editing capability of a configuration already on vendor support. An actual switch resets vendor object rules to locked; setting the current state leaves them unchanged.",
                "Supply exactly one of enabled (boolean) or rule. on/EditableSupportEnabled/enabled mean true; off/EditableSupportDisabled/disabled mean false. This Support-only apply must be separate from ordinary edits; project support policy and vendor payload validation still apply.",
                json!({"values":{"enabled":true}}),
            ),
            Self::RuleSet => (
                object(&[("rule",json!({"type":"string","enum":["locked","editable","off-support","NotEditable","EditableWithSupport","NotSupported"]}))], &["rule"]),
                "One existing top-level metadata object or its named form: <source-set>:<Kind>.<Name>[.Form.<FormName>].",
                "Set the vendor support rule for the addressed object's registered UUID. Other object rules stay unchanged.",
                "locked/NotEditable prohibit edits; editable/EditableWithSupport allow edits while keeping vendor support; off-support/NotSupported remove the object from vendor support. This Support-only apply must be separate from ordinary edits; source profile, registration and support state are still checked.",
                json!({"values":{"rule":"editable"}}),
            ),
        };
        OperationContract {
            schema: object(&[("at", text(true)), ("values", values)], &["values"]),
            target,
            effect,
            notes,
            example_args,
            help: "skills/release-support/SKILL.md",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OperationApplicability, OperationSkeleton, SupportOperation};
    use serde_json::json;

    #[test]
    fn registration_metadata_preserves_the_published_support_operations() {
        for (operation, name, applicability) in [
            (
                SupportOperation::CapabilitySet,
                "supportCapability.set",
                OperationApplicability::Configuration,
            ),
            (
                SupportOperation::RuleSet,
                "supportRule.set",
                OperationApplicability::SupportRule,
            ),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), applicability, "{name}");
            assert_eq!(operation.skeleton(), OperationSkeleton::Values, "{name}");
        }
    }

    #[test]
    fn normalization_preserves_support_alias_payloads() {
        for (operation, args) in [
            (
                SupportOperation::CapabilitySet,
                json!({"values":{"rule":"EditableSupportEnabled"}}),
            ),
            (
                SupportOperation::RuleSet,
                json!({"values":{"rule":"EditableWithSupport"}}),
            ),
        ] {
            let contract = operation.contract();
            let validator = jsonschema::validator_for(&contract.schema).unwrap();
            assert!(validator.is_valid(&args), "{}: {args}", operation.name());
            assert_eq!(
                contract.normalize(&args, "args").unwrap(),
                args,
                "{}",
                operation.name()
            );
        }
    }
}
