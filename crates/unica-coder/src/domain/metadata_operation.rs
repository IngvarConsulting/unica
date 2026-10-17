use super::apply::{OperationApplicability, OperationSkeleton};
use super::metadata::MetaCollection;
use super::operation_contract::MetadataSite;
use super::operation_contract::OperationContract;

macro_rules! metadata_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum MetadataOperation {
            $($variant),+
        }

        impl MetadataOperation {
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

metadata_operations!(
    AttributeAdd => ("attribute.add", MetadataAttributeContainer, Items),
    CommandAdd => ("command.add", MetadataCommands, Items),
);

impl MetadataOperation {
    pub(crate) fn contract_at(self, site: Option<MetadataSite>) -> OperationContract {
        let collection = match self {
            Self::AttributeAdd => MetaCollection::Attributes,
            Self::CommandAdd => MetaCollection::Commands,
        };
        OperationContract::metadata_member_add(collection, site)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::MetadataKind;
    use serde_json::json;

    #[test]
    fn registration_metadata_preserves_the_two_contextual_member_additions() {
        for (operation, name, applicability) in [
            (
                MetadataOperation::AttributeAdd,
                "attribute.add",
                OperationApplicability::MetadataAttributeContainer,
            ),
            (
                MetadataOperation::CommandAdd,
                "command.add",
                OperationApplicability::MetadataCommands,
            ),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), applicability);
            assert_eq!(operation.skeleton(), OperationSkeleton::Items);
            assert!(MetadataOperation::is_named(name));
        }
        for name in [
            "attribute.set",
            "command.remove",
            "attribute.add.extra",
            "ATTRIBUTE.ADD",
            "",
        ] {
            assert!(!MetadataOperation::is_named(name), "{name}");
        }
    }

    #[test]
    fn attribute_contract_keeps_owner_and_tabular_section_fill_value_rules() {
        for (owner, in_section, allows_fill) in [
            (MetadataKind::Document, false, true),
            (MetadataKind::Document, true, false),
            (MetadataKind::Report, false, false),
            (MetadataKind::Report, true, true),
        ] {
            let contract = MetadataOperation::AttributeAdd
                .contract_at(Some(MetadataSite { owner, in_section }));
            let fields = &contract.schema["properties"]["items"]["items"]["properties"];
            assert_eq!(
                fields.get("fillValue").is_some(),
                allows_fill,
                "{owner:?}, section={in_section}"
            );
            assert!(fields.get("type").is_some());
            assert!(fields.get("action").is_none());
            assert_eq!(contract.help, "skills/meta-edit/SKILL.md");
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
        }
        let command = MetadataOperation::CommandAdd.contract_at(None);
        assert_eq!(command.example_args, json!({"items":[{"name":"Sample"}]}));
        assert!(command.schema["properties"]["items"]["items"]["properties"]
            .get("type")
            .is_none());
        assert!(command
            .normalize(
                &json!({"items":[{"name":"Sample","title":"Form-only"}]}),
                "args"
            )
            .is_err());
    }
}
