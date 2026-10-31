use crate::applicability::OperationApplicability;
use crate::contracts::MetadataSite;
use crate::{MetaCollection, RelationEditMode};
use unica_operation_contract::OperationContract;
use unica_operation_contract::OperationSkeleton;

macro_rules! metadata_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum MetadataOperation {
            $($variant),+
        }

        impl MetadataOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<MetadataOperation> = unica_operations::TypedOperationBundle::new(MetadataOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }

            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

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

            /// Convert the public spelling at the request boundary. Dispatch
            /// and contracts use the returned identity rather than its name.
            pub fn named(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|operation| operation.name() == name)
            }

            #[cfg(any(test, feature = "test-support"))]
            pub fn is_named(name: &str) -> bool {
                Self::named(name).is_some()
            }
        }
    };
}

metadata_operations!(
    ObjectBorrow => ("object.borrow", ExtensionRoot, From),
    ObjectCreate => ("object.create", Configuration, Values),
    ObjectRemove => ("object.remove", MetadataOrCommonTemplate, Target),
    RelationAdd => ("relation.add", Metadata, Values),
    RelationRemove => ("relation.remove", Metadata, Values),
    RelationReplace => ("relation.replace", Metadata, Values),
    HelpCreate => ("help.create", Metadata, Values),
    AttributeAdd => ("attribute.add", MetadataAttributeContainer, Items),
    AttributeSet => ("attribute.set", MetadataAttribute, Values),
    AttributeRemove => ("attribute.remove", MetadataAttribute, Target),
    AttributeCopy => ("attribute.copy", CatalogTabularAttribute, Values),
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
    TemplateAdd => ("template.add", MetadataTemplates, Items),
    TemplateSet => ("template.set", MetadataTemplate, Values),
    TemplateRemove => ("template.remove", MetadataTemplate, Target),
    CommandAdd => ("command.add", MetadataCommands, Items),
    CommandSet => ("command.set", MetadataCommand, Values),
    CommandRemove => ("command.remove", MetadataCommand, Target),
    PredefinedItemAdd => ("predefinedItem.add", MetadataPredefinedItems, Items),
    PredefinedItemSet => ("predefinedItem.set", MetadataPredefinedItem, Values),
    PredefinedItemRemove => ("predefinedItem.remove", MetadataPredefinedItem, Target),
    ContentSet => ("content.set", ExchangePlan, Items),
);

impl MetadataOperation {
    /// Inline member arguments use the staged member reader. Templates and
    /// predefined items keep their own resource parsers and diagnostic order.
    pub const fn member_collection(self) -> Option<MetaCollection> {
        match self {
            Self::AttributeAdd | Self::AttributeSet | Self::AttributeRemove => {
                Some(MetaCollection::Attributes)
            }
            Self::TabularSectionAdd | Self::TabularSectionSet | Self::TabularSectionRemove => {
                Some(MetaCollection::TabularSections)
            }
            Self::DimensionAdd | Self::DimensionSet | Self::DimensionRemove => {
                Some(MetaCollection::Dimensions)
            }
            Self::ResourceAdd | Self::ResourceSet | Self::ResourceRemove => {
                Some(MetaCollection::Resources)
            }
            Self::EnumValueAdd | Self::EnumValueSet | Self::EnumValueRemove => {
                Some(MetaCollection::EnumValues)
            }
            Self::ColumnAdd | Self::ColumnSet | Self::ColumnRemove => Some(MetaCollection::Columns),
            Self::CommandAdd | Self::CommandSet | Self::CommandRemove => {
                Some(MetaCollection::Commands)
            }
            Self::ObjectBorrow
            | Self::ObjectCreate
            | Self::ObjectRemove
            | Self::RelationAdd
            | Self::RelationRemove
            | Self::RelationReplace
            | Self::HelpCreate
            | Self::TemplateAdd
            | Self::TemplateSet
            | Self::TemplateRemove
            | Self::PredefinedItemAdd
            | Self::PredefinedItemSet
            | Self::PredefinedItemRemove
            | Self::ContentSet
            | Self::AttributeCopy => None,
        }
    }

    pub const fn relation_mode(self) -> Option<RelationEditMode> {
        match self {
            Self::RelationAdd => Some(RelationEditMode::Add),
            Self::RelationRemove => Some(RelationEditMode::Remove),
            Self::RelationReplace => Some(RelationEditMode::Replace),
            _ => None,
        }
    }

    pub fn contract_at(self, site: Option<MetadataSite>) -> OperationContract {
        let owner = site.map(|site| site.owner);
        match self {
            Self::AttributeAdd
            | Self::TabularSectionAdd
            | Self::DimensionAdd
            | Self::ResourceAdd
            | Self::EnumValueAdd
            | Self::ColumnAdd
            | Self::CommandAdd => crate::contracts::metadata_member_add(
                self.member_collection().expect("inline member collection"),
                site,
            ),
            Self::AttributeSet
            | Self::TabularSectionSet
            | Self::DimensionSet
            | Self::ResourceSet
            | Self::EnumValueSet
            | Self::ColumnSet
            | Self::CommandSet => crate::contracts::metadata_member_set(
                self.member_collection().expect("inline member collection"),
                site,
            ),
            Self::AttributeRemove
            | Self::TabularSectionRemove
            | Self::DimensionRemove
            | Self::ResourceRemove
            | Self::EnumValueRemove
            | Self::ColumnRemove
            | Self::CommandRemove => crate::contracts::metadata_member_remove(),
            Self::AttributeCopy => crate::contracts::metadata_attribute_copy(),
            Self::ObjectBorrow => crate::contracts::metadata_object_borrow(),
            Self::ObjectCreate => crate::contracts::metadata_object_create(),
            Self::ObjectRemove => crate::contracts::metadata_object_remove(),
            Self::TemplateAdd => crate::contracts::metadata_template_add(),
            Self::TemplateSet => crate::contracts::metadata_template_set(),
            Self::TemplateRemove => crate::contracts::metadata_template_remove(),
            Self::HelpCreate => crate::contracts::metadata_help_create(),
            Self::PredefinedItemRemove => crate::contracts::metadata_predefined_remove(),
            Self::PredefinedItemAdd => crate::contracts::metadata_predefined_add(owner),
            Self::PredefinedItemSet => crate::contracts::metadata_predefined_set(owner),
            Self::ContentSet => crate::contracts::metadata_exchange_content_set(),
            Self::RelationAdd | Self::RelationRemove | Self::RelationReplace => {
                crate::contracts::metadata_relation(
                    self.relation_mode().expect("relation operation"),
                    owner,
                )
            }
        }
    }
}

impl unica_operations::FamilyOperation for MetadataOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    type Observation = ();
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Metadata
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
        Some(self.contract_at(None))
    }
    fn content_resource(self) -> Option<&'static str> {
        None
    }

    fn contract_at(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
    ) -> Option<unica_operation_contract::OperationContract> {
        let owner = site
            .segments()
            .first()
            .and_then(|segment| crate::MetadataKind::parse(segment.kind().as_str()).ok());
        let in_section = site
            .segments()
            .iter()
            .any(|segment| segment.kind() == unica_address::address::NodeKind::TabularSection);
        Some(
            self.contract_at(
                owner.map(|owner| crate::contracts::MetadataSite { owner, in_section }),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MetadataKind;
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

    #[test]
    fn deny_incomplete_values_is_published_only_for_dimensions_of_its_register_kinds() {
        use crate::contracts::MetadataSite;
        let field = |operation: MetadataOperation, owner| {
            operation
                .contract_at(Some(MetadataSite {
                    owner,
                    in_section: false,
                }))
                .schema["properties"]["values"]["properties"]
                .get("denyIncompleteValues")
                .cloned()
        };
        for owner in [
            MetadataKind::AccumulationRegister,
            MetadataKind::AccountingRegister,
            MetadataKind::CalculationRegister,
        ] {
            assert_eq!(
                field(MetadataOperation::DimensionSet, owner),
                Some(json!({"type": "boolean"})),
                "{owner:?}"
            );
            assert_eq!(
                field(MetadataOperation::ResourceSet, owner),
                None,
                "{owner:?}"
            );
        }
        assert_eq!(
            field(
                MetadataOperation::DimensionSet,
                MetadataKind::InformationRegister
            ),
            None
        );
        let update = |owner| {
            crate::validate_metadata_operation_capabilities(
                owner,
                &unica_address::source_target::MetadataAddress::parse(
                    unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "InformationRegister.Prices",
                )
                .unwrap(),
                &crate::MetaEditOperation::Update {
                    collection: crate::MetaCollection::Dimensions,
                    scope: None,
                    elements: vec![crate::MetaElementUpdate {
                        name: "Item".into(),
                        new_name: None,
                        synonym: None,
                        comment: None,
                        r#type: None,
                        required: None,
                        fill_value: None,
                        position: None,
                        indexing: None,
                        deny_incomplete_values: Some(true),
                        base_dimension: None,
                    }],
                },
            )
        };
        let refused = update(MetadataKind::InformationRegister).unwrap_err();
        assert_eq!(
            refused.field.as_deref(),
            Some("elements[0].denyIncompleteValues")
        );
        update(MetadataKind::AccumulationRegister).unwrap();
    }
}
