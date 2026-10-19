use super::apply::{OperationApplicability, OperationSkeleton};
use super::dcs_arguments::*;
use super::operation_contract::{ContractError, OperationContract};
use serde_json::Value;

macro_rules! dcs_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident, $payload:ty)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Primitive { $($variant),+ }

        #[derive(Debug, Clone)]
        pub(crate) enum DcsIntent { $($variant($payload)),+ }

        impl Primitive {
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub(crate) const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
            pub(super) const fn applicability(self) -> OperationApplicability {
                match self { $(Self::$variant => OperationApplicability::$applicability),+ }
            }
            pub(super) const fn skeleton(self) -> OperationSkeleton {
                match self { $(Self::$variant => OperationSkeleton::$skeleton),+ }
            }
            #[cfg(test)]
            pub(crate) fn named(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|operation| operation.name() == name)
            }
        }
        impl DcsIntent {
            pub(crate) const fn operation(&self) -> Primitive {
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
    pub(crate) fn contract(self) -> OperationContract {
        OperationContract::dcs(self)
    }

    pub(crate) fn decode(self, args: &Value, path: &str) -> Result<DcsIntent, ContractError> {
        super::dcs_arguments::decode(self, args, path)
    }
}
