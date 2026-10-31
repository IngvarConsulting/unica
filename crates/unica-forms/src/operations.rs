use crate::applicability::OperationApplicability;
use crate::FormItemRole;
use unica_operation_contract::OperationContract;
use unica_operation_contract::OperationSkeleton;

macro_rules! form_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum FormOperation {
            $($variant),+
        }

        impl FormOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<FormOperation> = unica_operations::TypedOperationBundle::new(FormOperation::$variant);
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

            #[cfg(any(test, feature = "test-support"))]
            pub const fn is_named(name: &str) -> bool {
                $(if operation_names_equal(name, $name) {
                    return true;
                })+
                false
            }
        }
    };
}

form_operations!(
    Add => ("form.add", MetadataForms, Items),
    Set => ("form.set", MetadataForms, Values),
    Remove => ("form.remove", Form, Target),
    ItemAdd => ("item.add", FormItemContainer, Items),
    ItemRemove => ("item.remove", FormItem, Target),
    ItemMove => ("item.move", FormItem, Values),
    AttributeAdd => ("attribute.add", Form, Items),
    CommandAdd => ("command.add", Form, Items),
    EventBind => ("event.bind", FormEvent, Values),
    PropsSet => ("props.set", FormProperties, Values),
);

impl FormOperation {
    pub fn contract(self) -> OperationContract {
        crate::contracts::form(self)
    }

    pub fn contract_for_address(
        self,
        at: &unica_address::address::QualifiedAddress,
    ) -> OperationContract {
        if self == Self::PropsSet && crate::applicability::is_form_column(at.segments()) {
            crate::contracts::form_column_type_set()
        } else {
            self.contract()
        }
    }

    pub const fn content_resource(self) -> Option<&'static str> {
        match self {
            Self::Add | Self::Set | Self::Remove => None,
            Self::ItemAdd
            | Self::ItemRemove
            | Self::ItemMove
            | Self::AttributeAdd
            | Self::CommandAdd
            | Self::EventBind
            | Self::PropsSet => Some("Form.xml"),
        }
    }

    pub fn admits_form_item(self, role: Option<FormItemRole>) -> bool {
        match self {
            Self::ItemAdd => role != Some(FormItemRole::Leaf),
            Self::ItemRemove | Self::ItemMove => role != Some(FormItemRole::AutoCommandBar),
            Self::Add
            | Self::Set
            | Self::Remove
            | Self::AttributeAdd
            | Self::CommandAdd
            | Self::EventBind
            | Self::PropsSet => true,
        }
    }
}

/// Const equality for the test-only Form name lookup.
#[cfg(any(test, feature = "test-support"))]
const fn operation_names_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

impl unica_operations::FamilyOperation for FormOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    fn prepare_redirect(
        self,
        template: &str,
        at: &unica_address::address::QualifiedAddress,
        args: &serde_json::Map<String, serde_json::Value>,
        value: &str,
        members: unica_operations::recovery::MemberPathLookup<'_>,
    ) -> Option<unica_operations::recovery::PreparedRedirect> {
        crate::recovery::prepare(template, at, args, value, members)
    }
    type Observation = crate::FormObservation;
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Form
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
        self.content_resource()
    }
    fn contract_at(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
    ) -> Option<unica_operation_contract::OperationContract> {
        Some(match self {
            Self::Set => crate::contracts::form_set(
                site.segments()
                    .first()
                    .map(|segment| segment.kind().as_str()),
            ),
            _ => self.contract_for_address(site.at()),
        })
    }

    fn needs_observation(
        self,
        site: Option<&unica_source_contract::context::OperationSite<'_>>,
    ) -> bool {
        site.is_none_or(|site| {
            site.terminal().kind() == unica_address::address::NodeKind::Item
                || (self == Self::PropsSet && crate::applicability::is_form_column(site.segments()))
        })
    }
    fn admits_observation(self, observation: Option<&Self::Observation>) -> bool {
        if self == Self::PropsSet {
            observation == Some(&crate::FormObservation::ValueTableColumn)
        } else {
            self.admits_form_item(
                observation
                    .copied()
                    .and_then(crate::FormObservation::item_role),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registration_metadata_preserves_every_form_operation() {
        for (operation, name, applicability, skeleton) in [
            (
                FormOperation::Add,
                "form.add",
                OperationApplicability::MetadataForms,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::Set,
                "form.set",
                OperationApplicability::MetadataForms,
                OperationSkeleton::Values,
            ),
            (
                FormOperation::Remove,
                "form.remove",
                OperationApplicability::Form,
                OperationSkeleton::Target,
            ),
            (
                FormOperation::ItemAdd,
                "item.add",
                OperationApplicability::FormItemContainer,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::ItemRemove,
                "item.remove",
                OperationApplicability::FormItem,
                OperationSkeleton::Target,
            ),
            (
                FormOperation::ItemMove,
                "item.move",
                OperationApplicability::FormItem,
                OperationSkeleton::Values,
            ),
            (
                FormOperation::AttributeAdd,
                "attribute.add",
                OperationApplicability::Form,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::CommandAdd,
                "command.add",
                OperationApplicability::Form,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::EventBind,
                "event.bind",
                OperationApplicability::FormEvent,
                OperationSkeleton::Values,
            ),
            (
                FormOperation::PropsSet,
                "props.set",
                OperationApplicability::FormProperties,
                OperationSkeleton::Values,
            ),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), applicability);
            assert_eq!(operation.skeleton(), skeleton);
            assert!(FormOperation::is_named(name));
        }
        for name in [
            "form.extra",
            "item.extra",
            "command.add.extra",
            "COMMAND.ADD",
            "",
        ] {
            assert!(!FormOperation::is_named(name), "{name}");
        }
    }

    #[test]
    fn form_contracts_keep_examples_and_distinct_writer_fields() {
        for operation in [FormOperation::AttributeAdd, FormOperation::CommandAdd] {
            let contract = operation.contract();
            assert_eq!(contract.help, "skills/form-edit/SKILL.md");
            assert!(contract.target.starts_with("The form itself:"));
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
            assert_eq!(contract.schema["additionalProperties"], false);
        }
        let attribute = FormOperation::AttributeAdd.contract();
        assert_eq!(
            attribute.example_args,
            json!({"items":[{"name":"Sample","type":"String"}]})
        );
        assert!(
            attribute.schema["properties"]["items"]["items"]["properties"]
                .get("savedData")
                .is_some()
        );
        let command = FormOperation::CommandAdd.contract();
        let fields = &command.schema["properties"]["items"]["items"]["properties"];
        assert!(fields.get("action").is_some());
        assert!(fields.get("savedData").is_none());
        assert_eq!(
            command
                .normalize(&json!({"items":[{"name":"A","type":"String"}]}), "args")
                .unwrap_err()
                .path,
            "args.items[0].type"
        );
    }
}
