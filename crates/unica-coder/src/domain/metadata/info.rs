use super::{MetadataKind, ObservedMetadataType};
use crate::domain::source_target::MetadataAddress;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaScheduledMethod {
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaCalculationSchedule {
    pub(crate) register: MetadataAddress,
    pub(crate) value_field: String,
    pub(crate) date_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaHttpMethod {
    pub(crate) name: String,
    pub(crate) http_method: String,
    pub(crate) handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaHttpUrlTemplate {
    pub(crate) name: String,
    pub(crate) template: String,
    pub(crate) methods: Vec<MetaHttpMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaExpandedName {
    pub(crate) namespace: String,
    pub(crate) local_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaLocalizedText {
    pub(crate) language: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum MetaObservedPropertyValue {
    Text {
        value: String,
    },
    Boolean {
        value: bool,
    },
    LocalizedString {
        values: Vec<MetaLocalizedText>,
    },
    Typed {
        r#type: MetaExpandedName,
        value: String,
    },
    Nil {},
    Empty {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaObservedProperty {
    pub(crate) name: String,
    pub(crate) value: MetaObservedPropertyValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaStandardAttribute {
    pub(crate) name: String,
    pub(crate) properties: Vec<MetaObservedProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaCharacteristicTypes {
    pub(crate) source: String,
    pub(crate) key_field: String,
    pub(crate) types_filter_field: String,
    pub(crate) types_filter_value: MetaObservedPropertyValue,
    pub(crate) data_path_field: String,
    pub(crate) multiple_values_use_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaCharacteristicValues {
    pub(crate) source: String,
    pub(crate) object_field: String,
    pub(crate) type_field: String,
    pub(crate) value_field: String,
    pub(crate) multiple_values_key_field: String,
    pub(crate) multiple_values_order_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaCharacteristic {
    pub(crate) types: MetaCharacteristicTypes,
    pub(crate) values: MetaCharacteristicValues,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaStandardTabularSection {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) synonym: Option<Vec<MetaLocalizedText>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_tip: Option<Vec<MetaLocalizedText>>,
    pub(crate) fill_checking: String,
    pub(crate) standard_attributes: Vec<MetaStandardAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoDeclarations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) standard_attributes: Option<Option<Vec<MetaStandardAttribute>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) characteristics: Option<Option<Vec<MetaCharacteristic>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) standard_tabular_sections: Option<Option<Vec<MetaStandardTabularSection>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum MetaXdtoPackage {
    Package { metadata_path: MetadataAddress },
    Namespace { namespace: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MetaTransferDirection {
    In,
    Out,
    InOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaWebServiceParameter {
    pub(crate) name: String,
    pub(crate) r#type: MetaExpandedName,
    pub(crate) nillable: bool,
    pub(crate) direction: MetaTransferDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaWebServiceOperation {
    pub(crate) name: String,
    pub(crate) return_type: MetaExpandedName,
    pub(crate) nillable: bool,
    pub(crate) transactioned: bool,
    pub(crate) procedure: String,
    pub(crate) parameters: Vec<MetaWebServiceParameter>,
}

/// Kind-specific part of the public `unica.meta.info` read model.
///
/// Keeping `kind` and `details` in one adjacent-tagged enum prevents callers
/// inside Unica from constructing a payload whose discriminator and details
/// describe different metadata kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "details", rename_all_fields = "camelCase")]
pub(crate) enum MetaInfoDetails {
    Catalog {},
    Document {},
    Enum {},
    Constant {
        r#type: Option<ObservedMetadataType>,
    },
    InformationRegister {},
    AccumulationRegister {},
    AccountingRegister {},
    CalculationRegister {
        schedule: Option<MetaCalculationSchedule>,
    },
    ChartOfAccounts {},
    ChartOfCharacteristicTypes {
        r#type: Option<ObservedMetadataType>,
    },
    ChartOfCalculationTypes {
        base_calculation_types: Option<Vec<MetadataAddress>>,
    },
    BusinessProcess {},
    Task {},
    ExchangePlan {},
    DocumentJournal {
        registered_documents: Option<Vec<MetadataAddress>>,
    },
    Report {},
    DataProcessor {},
    CommonModule {},
    ScheduledJob {
        method: Option<MetaScheduledMethod>,
    },
    EventSubscription {},
    HTTPService {
        url_templates: Option<Vec<MetaHttpUrlTemplate>>,
    },
    WebService {
        xdto_packages: Option<Vec<MetaXdtoPackage>>,
        operations: Option<Vec<MetaWebServiceOperation>>,
    },
    DefinedType {
        r#type: Option<ObservedMetadataType>,
    },
}

impl MetaInfoDetails {
    pub(crate) const fn empty(kind: MetadataKind) -> Self {
        match kind {
            MetadataKind::Catalog => Self::Catalog {},
            MetadataKind::Document => Self::Document {},
            MetadataKind::Enum => Self::Enum {},
            MetadataKind::Constant => Self::Constant { r#type: None },
            MetadataKind::InformationRegister => Self::InformationRegister {},
            MetadataKind::AccumulationRegister => Self::AccumulationRegister {},
            MetadataKind::AccountingRegister => Self::AccountingRegister {},
            MetadataKind::CalculationRegister => Self::CalculationRegister { schedule: None },
            MetadataKind::ChartOfAccounts => Self::ChartOfAccounts {},
            MetadataKind::ChartOfCharacteristicTypes => {
                Self::ChartOfCharacteristicTypes { r#type: None }
            }
            MetadataKind::ChartOfCalculationTypes => Self::ChartOfCalculationTypes {
                base_calculation_types: None,
            },
            MetadataKind::BusinessProcess => Self::BusinessProcess {},
            MetadataKind::Task => Self::Task {},
            MetadataKind::ExchangePlan => Self::ExchangePlan {},
            MetadataKind::DocumentJournal => Self::DocumentJournal {
                registered_documents: None,
            },
            MetadataKind::Report => Self::Report {},
            MetadataKind::DataProcessor => Self::DataProcessor {},
            MetadataKind::CommonModule => Self::CommonModule {},
            MetadataKind::ScheduledJob => Self::ScheduledJob { method: None },
            MetadataKind::EventSubscription => Self::EventSubscription {},
            MetadataKind::HTTPService => Self::HTTPService {
                url_templates: None,
            },
            MetadataKind::WebService => Self::WebService {
                xdto_packages: None,
                operations: None,
            },
            MetadataKind::DefinedType => Self::DefinedType { r#type: None },
        }
    }
}
