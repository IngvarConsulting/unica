use super::{MetadataKind, ObservedMetadataType};
use serde::{Deserialize, Serialize};
use unica_address::source_target::MetadataAddress;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaScheduledMethod {
    pub metadata_path: MetadataAddress,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCalculationSchedule {
    pub register: MetadataAddress,
    pub value_field: String,
    pub date_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaHttpMethod {
    pub name: String,
    pub http_method: String,
    pub handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaHttpUrlTemplate {
    pub name: String,
    pub template: String,
    pub methods: Vec<MetaHttpMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaExpandedName {
    pub namespace: String,
    pub local_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaLocalizedText {
    pub language: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum MetaObservedPropertyValue {
    UnsignedInteger {
        value: u32,
    },
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaObservedProperty {
    pub name: String,
    pub value: MetaObservedPropertyValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaStandardAttribute {
    pub name: String,
    pub properties: Vec<MetaObservedProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCharacteristicTypes {
    pub source: String,
    pub key_field: String,
    pub types_filter_field: String,
    pub types_filter_value: MetaObservedPropertyValue,
    pub data_path_field: String,
    pub multiple_values_use_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCharacteristicValues {
    pub source: String,
    pub object_field: String,
    pub type_field: String,
    pub value_field: String,
    pub multiple_values_key_field: String,
    pub multiple_values_order_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCharacteristic {
    pub types: MetaCharacteristicTypes,
    pub values: MetaCharacteristicValues,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaStandardTabularSection {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synonym: Option<Vec<MetaLocalizedText>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_tip: Option<Vec<MetaLocalizedText>>,
    pub fill_checking: String,
    pub standard_attributes: Vec<MetaStandardAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetaInfoDeclarations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standard_attributes: Option<Option<Vec<MetaStandardAttribute>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristics: Option<Option<Vec<MetaCharacteristic>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standard_tabular_sections: Option<Option<Vec<MetaStandardTabularSection>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum MetaXdtoPackage {
    Package { metadata_path: MetadataAddress },
    Namespace { namespace: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MetaTransferDirection {
    In,
    Out,
    InOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaWebServiceParameter {
    pub name: String,
    pub r#type: MetaExpandedName,
    pub nillable: bool,
    pub direction: MetaTransferDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaWebServiceOperation {
    pub name: String,
    pub return_type: MetaExpandedName,
    pub nillable: bool,
    pub transactioned: bool,
    pub procedure: String,
    pub parameters: Vec<MetaWebServiceParameter>,
}

/// Kind-specific part of the public `unica.meta.info` read model.
///
/// Keeping `kind` and `details` in one adjacent-tagged enum prevents callers
/// inside Unica from constructing a payload whose discriminator and details
/// describe different metadata kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "details", rename_all_fields = "camelCase")]
pub enum MetaInfoDetails {
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
    pub const fn empty(kind: MetadataKind) -> Self {
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

/// Closed vocabulary of properties observed on a source StandardAttribute.
pub const META_STANDARD_ATTRIBUTE_PROPERTY_NAMES: &[&str] = &[
    "LinkByType",
    "FillChecking",
    "MultiLine",
    "FillFromFillingValue",
    "CreateOnInput",
    "TypeReductionMode",
    "MaxValue",
    "ToolTip",
    "ExtendedEdit",
    "Format",
    "ChoiceForm",
    "QuickChoice",
    "ChoiceHistoryOnInput",
    "EditFormat",
    "PasswordMode",
    "DataHistory",
    "MarkNegatives",
    "MinValue",
    "Synonym",
    "Comment",
    "FullTextSearch",
    "ChoiceParameterLinks",
    "FillValue",
    "Mask",
    "ChoiceParameters",
];
