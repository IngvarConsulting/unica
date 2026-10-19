use super::apply::{FormItemRole, OperationApplicability, OperationSkeleton};
use super::operation_contract::OperationContract;

macro_rules! form_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum FormOperation {
            $($variant),+
        }

        impl FormOperation {
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

            #[cfg(test)]
            pub(super) const fn is_named(name: &str) -> bool {
                $(if super::apply::operation_names_equal(name, $name) {
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
);

impl FormOperation {
    pub(crate) fn contract(self) -> OperationContract {
        OperationContract::form(self)
    }

    pub(crate) const fn content_resource(self) -> Option<&'static str> {
        match self {
            Self::Add | Self::Set | Self::Remove => None,
            Self::ItemAdd
            | Self::ItemRemove
            | Self::ItemMove
            | Self::AttributeAdd
            | Self::CommandAdd
            | Self::EventBind => Some("Form.xml"),
        }
    }

    pub(crate) fn admits_form_item(self, role: Option<FormItemRole>) -> bool {
        match self {
            Self::ItemAdd => role != Some(FormItemRole::Leaf),
            Self::ItemRemove | Self::ItemMove => role != Some(FormItemRole::AutoCommandBar),
            Self::Add
            | Self::Set
            | Self::Remove
            | Self::AttributeAdd
            | Self::CommandAdd
            | Self::EventBind => true,
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
