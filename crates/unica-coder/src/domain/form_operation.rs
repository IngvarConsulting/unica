use super::apply::{OperationApplicability, OperationSkeleton};
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
    AttributeAdd => ("attribute.add", Form, Items),
    CommandAdd => ("command.add", Form, Items),
);

impl FormOperation {
    pub(crate) fn contract(self) -> OperationContract {
        match self {
            Self::AttributeAdd => OperationContract::form_attribute_add(),
            Self::CommandAdd => OperationContract::form_command_add(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registration_metadata_preserves_the_two_form_root_additions() {
        for (operation, name) in [
            (FormOperation::AttributeAdd, "attribute.add"),
            (FormOperation::CommandAdd, "command.add"),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), OperationApplicability::Form);
            assert_eq!(operation.skeleton(), OperationSkeleton::Items);
            assert!(FormOperation::is_named(name));
        }
        for name in [
            "form.add",
            "item.add",
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
