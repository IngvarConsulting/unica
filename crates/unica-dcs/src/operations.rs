use crate::applicability::OperationApplicability;
use crate::arguments::*;
use serde_json::Value;
use unica_operation_contract::OperationSkeleton;
use unica_operation_contract::{ContractError, OperationContract};

macro_rules! dcs_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident, $payload:ty)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Primitive { $($variant),+ }

        #[derive(Debug, Clone)]
        pub enum DcsIntent { $($variant($payload)),+ }

        impl Primitive {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<Primitive> = unica_operations::TypedOperationBundle::new(Primitive::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }

            #[cfg(any(test, feature = "test-support"))]
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
            pub const fn applicability(self) -> OperationApplicability {
                match self { $(Self::$variant => OperationApplicability::$applicability),+ }
            }
            pub const fn skeleton(self) -> OperationSkeleton {
                match self { $(Self::$variant => OperationSkeleton::$skeleton),+ }
            }
            #[cfg(any(test, feature = "test-support"))]
            pub fn named(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|operation| operation.name() == name)
            }
        }
        impl DcsIntent {
            pub const fn operation(&self) -> Primitive {
                match self { $(Self::$variant(_) => Primitive::$variant),+ }
            }
        }
    };
}

dcs_operations!(
    DataSourceAdd => ("dataSource.add", DcsSchema, Items, Vec<DataSourceSpec>),
    DataSourceSet => ("dataSource.set", DcsDataSource, Values, DataSourceKind),
    DataSourceRemove => ("dataSource.remove", DcsDataSource, Target, ()),
    DataSetAdd => ("dataSet.add", DcsDataContainer, Items, Vec<DataSetSpec>),
    DataSetSet => ("dataSet.set", DcsData, Values, DataSetPatch),
    DataSetRemove => ("dataSet.remove", DcsData, Target, ()),
    FieldAdd => ("field.add", DcsData, Items, Vec<FieldSpec>),
    FieldSet => ("field.set", DcsField, Values, FieldPatch),
    FieldRemove => ("field.remove", DcsField, Target, ()),
    FieldRoleSet => ("fieldRole.set", DcsField, Values, FieldRole),
    ParameterAdd => ("parameter.add", DcsSchema, Items, Vec<ParameterSpec>),
    ParameterSet => ("parameter.set", DcsParameter, Values, ParameterPatch),
    ParameterRemove => ("parameter.remove", DcsParameter, Target, ()),
    ParameterRename => ("parameter.rename", DcsParameter, Values, String),
    ParameterReorder => ("parameter.reorder", DcsSchema, Items, Vec<String>),
    CalculatedFieldAdd => ("calculatedField.add", DcsSchema, Items, Vec<CalculatedFieldSpec>),
    CalculatedFieldRemove => ("calculatedField.remove", DcsCalculatedField, Target, ()),
    TotalAdd => ("total.add", DcsSchema, Items, Vec<TotalSpec>),
    TotalRemove => ("total.remove", DcsTotal, Target, ()),
    VariantAdd => ("variant.add", DcsSchema, Items, Vec<VariantSpec>),
    VariantSet => ("variant.set", DcsSettings, Values, LocalizedText),
    VariantRemove => ("variant.remove", DcsSettings, Target, ()),
    QuerySet => ("query.set", DcsQuery, Values, String),
    QueryPatch => ("query.patch", DcsQuery, Values, QueryPatch),
    FilterAdd => ("filter.add", DcsSettingsContainer, Items, Vec<FilterSpec>),
    FilterSet => ("filter.set", DcsSettingsContainer, Values, FilterPatch),
    FilterRemove => ("filter.remove", DcsSettingsContainer, Values, String),
    FilterClear => ("filter.clear", DcsSettingsContainer, Values, ()),
    SelectionAdd => ("selection.add", DcsSettingsContainer, Items, Vec<SelectionSpec>),
    SelectionClear => ("selection.clear", DcsSettingsContainer, Values, ()),
    OrderAdd => ("order.add", DcsSettingsContainer, Items, Vec<OrderSpec>),
    OrderClear => ("order.clear", DcsSettingsContainer, Values, ()),
    DataParameterAdd => ("dataParameter.add", DcsSettings, Items, Vec<SettingsParameterSpec>),
    DataParameterSet => ("dataParameter.set", DcsSettings, Values, SettingsParameterSpec),
    OutputParameterSet => ("outputParameter.set", DcsSettingsContainer, Values, SettingsParameterSpec),
    ConditionalAppearanceAdd => ("conditionalAppearance.add", DcsSettingsContainer, Items, Vec<ConditionalAppearanceSpec>),
    ConditionalAppearanceClear => ("conditionalAppearance.clear", DcsSettingsContainer, Values, ()),
    StructureAdd => ("structure.add", DcsSettingsContainer, Items, Vec<StructureSpec>),
    StructureSet => ("structure.set", DcsSettings, Values, StructureSet),
    StructurePatch => ("structure.patch", DcsStructureItem, Values, StructurePatch),
    StructureRemove => ("structure.remove", DcsStructureItem, Target, ()),
    DataSetLinkAdd => ("dataSetLink.add", DcsSchema, Items, Vec<DataSetLinkSpec>),
    CalculatedFieldSet => ("calculatedField.set", DcsCalculatedField, Values, CalculatedFieldPatch),
    TotalSet => ("total.set", DcsTotal, Values, TotalPatch),
    SelectionSet => ("selection.set", DcsSettingsContainer, Values, SelectionPatch),
    SelectionRemove => ("selection.remove", DcsSettingsContainer, Values, SelectionRemove),
    OrderSet => ("order.set", DcsSettingsContainer, Values, OrderPatch),
    OrderRemove => ("order.remove", DcsSettingsContainer, Values, SelectionKey),
    DataParameterRemove => ("dataParameter.remove", DcsSettings, Values, String),
    OutputParameterRemove => ("outputParameter.remove", DcsSettingsContainer, Values, String),
    ConditionalAppearanceRemove => ("conditionalAppearance.remove", DcsSettingsContainer, Values, DcsIndex),
    DataSetLinkSet => ("dataSetLink.set", DcsSchema, Values, DataSetLinkSpec),
    DataSetLinkRemove => ("dataSetLink.remove", DcsSchema, Values, DataSetLinkSelector),
);

impl Primitive {
    pub fn contract(self) -> OperationContract {
        crate::contracts::dcs(self)
    }

    pub fn decode(self, args: &Value, path: &str) -> Result<DcsIntent, ContractError> {
        crate::arguments::decode(self, args, path)
    }
}

impl unica_operations::FamilyOperation for Primitive {
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
    fn additional_recovery_candidates(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
    ) -> Vec<unica_address::address::QualifiedAddress> {
        (site.terminal().kind() == unica_address::address::NodeKind::DataSet
            && site.terminal().name().is_some())
        .then(|| {
            unica_address::address::QualifiedAddress::parse(&format!("{}.Query", site.at())).ok()
        })
        .flatten()
        .into_iter()
        .collect()
    }
    fn retired_query_alias_target(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
        context: unica_source_contract::context::RegistryContext,
        members: unica_operations::recovery::MemberPathLookup<'_>,
    ) -> Option<unica_address::address::QualifiedAddress> {
        crate::recovery::query_alias(self, site, context, members)
    }
    type Observation = unica_templates::TemplateFormat;
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Dcs
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
        Some("Template.xml")
    }

    fn needs_observation(
        self,
        site: Option<&unica_source_contract::context::OperationSite<'_>>,
    ) -> bool {
        site.is_none_or(|site| site.terminal().kind() == unica_address::address::NodeKind::Template)
    }
    fn admits_observation(self, observation: Option<&Self::Observation>) -> bool {
        observation == Some(&unica_templates::TemplateFormat::DataCompositionSchema)
    }
}
