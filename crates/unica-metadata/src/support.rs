use crate::applicability::OperationApplicability;
use serde_json::json;
use unica_operation_contract::OperationSkeleton;
use unica_operation_contract::{object, text, OperationContract};

pub const SUPPORT_RESTORATION: unica_source_contract::proposal::PlanDetailKey =
    unica_source_contract::proposal::PlanDetailKey::new("supportRestoration");

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportRestorationData {
    pub at: String,
    pub suppliers: Vec<RestorationSupplier>,
    pub files_compared: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct RestorationSupplier {
    pub name: String,
    pub vendor: String,
    pub version: String,
}

macro_rules! support_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum SupportOperation {
            $($variant),+
        }

        impl SupportOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<SupportOperation> = unica_operations::TypedOperationBundle::new(SupportOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            pub const fn applicability(self) -> OperationApplicability {
                match self {
                    $(Self::$variant => OperationApplicability::$applicability),+
                }
            }

            pub const fn skeleton(self) -> OperationSkeleton {
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
    RuleRestore => ("supportRule.restore", SupportRestore, Target),
);

impl SupportOperation {
    pub const fn changes_support_state(self) -> bool {
        match self {
            Self::CapabilitySet | Self::RuleSet | Self::RuleRestore => true,
        }
    }

    pub fn contract(self) -> OperationContract {
        if self == Self::RuleRestore {
            return OperationContract {
                schema: object(&[("at", text(true))], &[]),
                target: "One existing metadata object or its named form in a configuration on vendor support.",
                effect: "Restore the locked rule only after every file of the object is byte-for-byte equal to its own version at every supporting supplier.",
                notes: "The preview checks descriptor, complete companion trees, modules, forms, templates and unknown files. Formatting differences also prevent restoration. The saved plan checks that sources and supplier payloads are unchanged before applying only the support rule. Ordinary supportRule.set keeps its existing behavior. Use a separate Support-only apply.",
                example_args: json!({}),
                help: "skills/release-support/SKILL.md",
                format_validators: &[],
            };
        }
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
            Self::RuleRestore => unreachable!("restoration has no values"),
        };
        OperationContract {
            schema: object(&[("at", text(true)), ("values", values)], &["values"]),
            target,
            effect,
            notes,
            example_args,
            help: "skills/release-support/SKILL.md",
            format_validators: &[],
        }
    }
}

impl unica_operations::FamilyOperation for SupportOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    type Observation = ();
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Support
    }
    fn name(self) -> &'static str {
        self.name()
    }
    fn skeleton(self) -> unica_operation_contract::OperationSkeleton {
        self.skeleton()
    }
    fn address_shape(self) -> &'static str {
        self.applicability().shape()
    }
    fn matches(self, site: &unica_source_contract::context::OperationSite<'_>) -> bool {
        self.applicability().matches(site)
    }
    fn contract(self) -> Option<unica_operation_contract::OperationContract> {
        Some(self.contract())
    }
    fn content_resource(self) -> Option<&'static str> {
        None
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
