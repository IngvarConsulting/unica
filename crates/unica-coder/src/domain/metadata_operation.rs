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
    AttributeSet => ("attribute.set", MetadataAttribute, Values),
    AttributeRemove => ("attribute.remove", MetadataAttribute, Target),
    CommandAdd => ("command.add", MetadataCommands, Items),
);

impl MetadataOperation {
    pub(crate) fn contract_at(self, site: Option<MetadataSite>) -> OperationContract {
        match self {
            Self::AttributeAdd => {
                OperationContract::metadata_member_add(MetaCollection::Attributes, site)
            }
            Self::AttributeSet => {
                OperationContract::metadata_member_set(MetaCollection::Attributes, site)
            }
            Self::AttributeRemove => OperationContract::metadata_member_remove(),
            Self::CommandAdd => {
                OperationContract::metadata_member_add(MetaCollection::Commands, site)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::MetadataKind;
    use serde_json::json;

    #[test]
    fn registration_metadata_preserves_attribute_operations_and_command_addition() {
        for (operation, name, applicability, skeleton) in [
            (
                MetadataOperation::AttributeAdd,
                "attribute.add",
                OperationApplicability::MetadataAttributeContainer,
                OperationSkeleton::Items,
            ),
            (
                MetadataOperation::AttributeSet,
                "attribute.set",
                OperationApplicability::MetadataAttribute,
                OperationSkeleton::Values,
            ),
            (
                MetadataOperation::AttributeRemove,
                "attribute.remove",
                OperationApplicability::MetadataAttribute,
                OperationSkeleton::Target,
            ),
            (
                MetadataOperation::CommandAdd,
                "command.add",
                OperationApplicability::MetadataCommands,
                OperationSkeleton::Items,
            ),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), applicability);
            assert_eq!(operation.skeleton(), skeleton);
            assert!(MetadataOperation::is_named(name));
        }
        for name in [
            "attribute.rename",
            "command.remove",
            "attribute.add.extra",
            "ATTRIBUTE.ADD",
            "",
        ] {
            assert!(!MetadataOperation::is_named(name), "{name}");
        }
    }

    #[test]
    fn attribute_set_contract_keeps_owner_and_section_fields_without_selectors() {
        let string_type = json!({"variants":[{
            "kind":"string","length":10,"allowedLength":"variable"
        }]});
        for (owner, in_section, allows_fill) in [
            (MetadataKind::Document, false, true),
            (MetadataKind::Document, true, false),
            (MetadataKind::Report, false, false),
            (MetadataKind::Report, true, true),
        ] {
            let contract = MetadataOperation::AttributeSet
                .contract_at(Some(MetadataSite { owner, in_section }));
            let fields = &contract.schema["properties"]["values"]["properties"];
            assert_eq!(
                fields.get("fillValue").is_some(),
                allows_fill,
                "{owner:?}, section={in_section}"
            );
            assert_eq!(
                contract
                    .normalize(
                        &json!({"values":{
                            "type":string_type,"fillValue":{"kind":"string","value":"x"}
                        }}),
                        "args",
                    )
                    .is_ok(),
                allows_fill,
                "{owner:?}, section={in_section}"
            );
            assert!(fields.get("newName").is_some());
            assert!(fields.get("name").is_none());
            assert!(fields.get("attributes").is_none());
            assert_eq!(contract.help, "skills/meta-edit/SKILL.md");
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
            for args in [
                json!({}),
                json!({"values":{}}),
                json!({"values":{"name":"Total"}}),
                json!({"values":{"synonym":"Sample"},"scope":{"tabularSection":"Lines"}}),
            ] {
                assert!(contract.normalize(&args, "args").is_err(), "{args}");
            }
        }
    }

    #[test]
    fn attribute_remove_contract_is_closed_and_independent_of_metadata_site() {
        let contract = MetadataOperation::AttributeRemove.contract_at(None);
        assert_eq!(contract.example_args, json!({}));
        assert!(contract
            .normalize(&json!({"at":"Document.Order.Attribute.Total"}), "args")
            .is_ok());
        for args in [
            json!({"values":{}}),
            json!({"items":["Total"]}),
            json!({"name":"Total"}),
        ] {
            assert!(contract.normalize(&args, "args").is_err(), "{args}");
        }
        for owner in [MetadataKind::Document, MetadataKind::Report] {
            for in_section in [false, true] {
                assert_eq!(
                    MetadataOperation::AttributeRemove
                        .contract_at(Some(MetadataSite { owner, in_section }))
                        .details(),
                    contract.details(),
                );
            }
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
