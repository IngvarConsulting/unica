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
    TabularSectionAdd => ("tabularSection.add", MetadataTabularSections, Items),
    TabularSectionSet => ("tabularSection.set", MetadataTabularSection, Values),
    TabularSectionRemove => ("tabularSection.remove", MetadataTabularSection, Target),
    DimensionAdd => ("dimension.add", MetadataDimensions, Items),
    DimensionSet => ("dimension.set", MetadataDimension, Values),
    DimensionRemove => ("dimension.remove", MetadataDimension, Target),
    ResourceAdd => ("resource.add", MetadataResources, Items),
    ResourceSet => ("resource.set", MetadataResource, Values),
    ResourceRemove => ("resource.remove", MetadataResource, Target),
    EnumValueAdd => ("enumValue.add", MetadataEnumValues, Items),
    EnumValueSet => ("enumValue.set", MetadataEnumValue, Values),
    EnumValueRemove => ("enumValue.remove", MetadataEnumValue, Target),
    ColumnAdd => ("column.add", MetadataColumns, Items),
    ColumnSet => ("column.set", MetadataColumn, Values),
    ColumnRemove => ("column.remove", MetadataColumn, Target),
    CommandAdd => ("command.add", MetadataCommands, Items),
    CommandSet => ("command.set", MetadataCommand, Values),
    CommandRemove => ("command.remove", MetadataCommand, Target),
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
            Self::TabularSectionAdd => {
                OperationContract::metadata_member_add(MetaCollection::TabularSections, site)
            }
            Self::TabularSectionSet => {
                OperationContract::metadata_member_set(MetaCollection::TabularSections, site)
            }
            Self::TabularSectionRemove => OperationContract::metadata_member_remove(),
            Self::DimensionAdd => {
                OperationContract::metadata_member_add(MetaCollection::Dimensions, site)
            }
            Self::DimensionSet => {
                OperationContract::metadata_member_set(MetaCollection::Dimensions, site)
            }
            Self::DimensionRemove => OperationContract::metadata_member_remove(),
            Self::ResourceAdd => {
                OperationContract::metadata_member_add(MetaCollection::Resources, site)
            }
            Self::ResourceSet => {
                OperationContract::metadata_member_set(MetaCollection::Resources, site)
            }
            Self::ResourceRemove => OperationContract::metadata_member_remove(),
            Self::EnumValueAdd => {
                OperationContract::metadata_member_add(MetaCollection::EnumValues, site)
            }
            Self::EnumValueSet => {
                OperationContract::metadata_member_set(MetaCollection::EnumValues, site)
            }
            Self::EnumValueRemove => OperationContract::metadata_member_remove(),
            Self::ColumnAdd => {
                OperationContract::metadata_member_add(MetaCollection::Columns, site)
            }
            Self::ColumnSet => {
                OperationContract::metadata_member_set(MetaCollection::Columns, site)
            }
            Self::ColumnRemove => OperationContract::metadata_member_remove(),
            Self::CommandAdd => {
                OperationContract::metadata_member_add(MetaCollection::Commands, site)
            }
            Self::CommandSet => {
                OperationContract::metadata_member_set(MetaCollection::Commands, site)
            }
            Self::CommandRemove => OperationContract::metadata_member_remove(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::MetadataKind;
    use serde_json::json;

    #[test]
    fn registration_metadata_preserves_all_typed_member_operations() {
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
                MetadataOperation::TabularSectionAdd,
                "tabularSection.add",
                OperationApplicability::MetadataTabularSections,
                OperationSkeleton::Items,
            ),
            (
                MetadataOperation::TabularSectionSet,
                "tabularSection.set",
                OperationApplicability::MetadataTabularSection,
                OperationSkeleton::Values,
            ),
            (
                MetadataOperation::TabularSectionRemove,
                "tabularSection.remove",
                OperationApplicability::MetadataTabularSection,
                OperationSkeleton::Target,
            ),
            (
                MetadataOperation::DimensionAdd,
                "dimension.add",
                OperationApplicability::MetadataDimensions,
                OperationSkeleton::Items,
            ),
            (
                MetadataOperation::DimensionSet,
                "dimension.set",
                OperationApplicability::MetadataDimension,
                OperationSkeleton::Values,
            ),
            (
                MetadataOperation::DimensionRemove,
                "dimension.remove",
                OperationApplicability::MetadataDimension,
                OperationSkeleton::Target,
            ),
            (
                MetadataOperation::ResourceAdd,
                "resource.add",
                OperationApplicability::MetadataResources,
                OperationSkeleton::Items,
            ),
            (
                MetadataOperation::ResourceSet,
                "resource.set",
                OperationApplicability::MetadataResource,
                OperationSkeleton::Values,
            ),
            (
                MetadataOperation::ResourceRemove,
                "resource.remove",
                OperationApplicability::MetadataResource,
                OperationSkeleton::Target,
            ),
            (
                MetadataOperation::EnumValueAdd,
                "enumValue.add",
                OperationApplicability::MetadataEnumValues,
                OperationSkeleton::Items,
            ),
            (
                MetadataOperation::EnumValueSet,
                "enumValue.set",
                OperationApplicability::MetadataEnumValue,
                OperationSkeleton::Values,
            ),
            (
                MetadataOperation::EnumValueRemove,
                "enumValue.remove",
                OperationApplicability::MetadataEnumValue,
                OperationSkeleton::Target,
            ),
            (
                MetadataOperation::ColumnAdd,
                "column.add",
                OperationApplicability::MetadataColumns,
                OperationSkeleton::Items,
            ),
            (
                MetadataOperation::ColumnSet,
                "column.set",
                OperationApplicability::MetadataColumn,
                OperationSkeleton::Values,
            ),
            (
                MetadataOperation::ColumnRemove,
                "column.remove",
                OperationApplicability::MetadataColumn,
                OperationSkeleton::Target,
            ),
            (
                MetadataOperation::CommandAdd,
                "command.add",
                OperationApplicability::MetadataCommands,
                OperationSkeleton::Items,
            ),
            (
                MetadataOperation::CommandSet,
                "command.set",
                OperationApplicability::MetadataCommand,
                OperationSkeleton::Values,
            ),
            (
                MetadataOperation::CommandRemove,
                "command.remove",
                OperationApplicability::MetadataCommand,
                OperationSkeleton::Target,
            ),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), applicability);
            assert_eq!(operation.skeleton(), skeleton);
            assert!(MetadataOperation::is_named(name));
        }
        for name in [
            "attribute.rename",
            "command.rename",
            "attribute.add.extra",
            "ATTRIBUTE.ADD",
            "",
        ] {
            assert!(!MetadataOperation::is_named(name), "{name}");
        }
    }

    #[test]
    fn tabular_section_add_contract_keeps_owner_context_for_nested_attributes() {
        for (owner, allows_fill) in [
            (MetadataKind::Document, false),
            (MetadataKind::Report, true),
        ] {
            let contract = MetadataOperation::TabularSectionAdd.contract_at(Some(MetadataSite {
                owner,
                in_section: false,
            }));
            let fields = &contract.schema["properties"]["items"]["items"]["properties"];
            let attributes = &fields["attributes"]["items"]["properties"];
            assert!(attributes.get("type").is_some(), "{owner:?}");
            assert_eq!(
                attributes.get("fillValue").is_some(),
                allows_fill,
                "{owner:?}"
            );
            assert!(contract
                .normalize(
                    &json!({"items":[{"name":"Lines","attributes":[{"name":"Value"}]}]}),
                    "args",
                )
                .is_ok());
            assert_eq!(
                contract
                    .normalize(
                        &json!({"items":[{"name":"Lines","attributes":[{
                            "name":"Value",
                            "type":{"variants":[{"kind":"boolean"}]},
                            "fillValue":{"kind":"boolean","value":true}
                        }]}]}),
                        "args",
                    )
                    .is_ok(),
                allows_fill,
                "{owner:?}"
            );
            let set = MetadataOperation::TabularSectionSet.contract_at(Some(MetadataSite {
                owner,
                in_section: true,
            }));
            assert!(set.schema["properties"]["values"]["properties"]
                .get("attributes")
                .is_none());
        }
    }

    #[test]
    fn command_set_contract_keeps_metadata_fields_and_refuses_form_fields() {
        let contract = MetadataOperation::CommandSet.contract_at(None);
        assert_eq!(contract.help, "skills/meta-edit/SKILL.md");
        assert!(contract.normalize(&contract.example_args, "args").is_ok());
        for values in [
            json!({"newName":"PrintCopy"}),
            json!({"synonym":"Print","comment":"kept"}),
            json!({"position":{"before":"Other"}}),
        ] {
            assert!(
                contract
                    .normalize(&json!({"values":values}), "args")
                    .is_ok(),
                "{values}"
            );
        }
        for args in [
            json!({}),
            json!({"values":{}}),
            json!({"values":{"newName":"1Print"}}),
            json!({"values":{"name":"Print"}}),
            json!({"values":{"title":"Form-only"}}),
            json!({"values":{"action":"Print"}}),
            json!({"values":{"type":{"variants":[{"kind":"boolean"}]}}}),
            json!({"values":{"fillValue":{"kind":"boolean","value":true}}}),
            json!({"values":{"required":true}}),
            json!({"values":{"indexing":"Index"}}),
            json!({"values":{"comment":"kept"},"items":[]}),
            json!({"values":{"comment":"kept"},"scope":{"tabularSection":"Lines"}}),
        ] {
            assert!(contract.normalize(&args, "args").is_err(), "{args}");
        }
        for owner in [MetadataKind::Document, MetadataKind::Report] {
            for in_section in [false, true] {
                assert_eq!(
                    MetadataOperation::CommandSet
                        .contract_at(Some(MetadataSite { owner, in_section }))
                        .details(),
                    contract.details(),
                );
            }
        }
    }

    #[test]
    fn command_remove_contract_takes_only_the_address() {
        let contract = MetadataOperation::CommandRemove.contract_at(None);
        assert_eq!(contract.example_args, json!({}));
        assert_eq!(contract.help, "skills/meta-edit/SKILL.md");
        for args in [json!({}), json!({"at":"Document.Order.Command.Print"})] {
            assert!(contract.normalize(&args, "args").is_ok(), "{args}");
        }
        for args in [
            json!({"name":"Print"}),
            json!({"values":{}}),
            json!({"items":["Print"]}),
            json!({"scope":{"tabularSection":"Lines"}}),
        ] {
            assert!(contract.normalize(&args, "args").is_err(), "{args}");
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
