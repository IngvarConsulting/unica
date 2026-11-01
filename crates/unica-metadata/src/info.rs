use super::{MetaDiagnostic, MetadataKind, ObservedMetadataType};
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
    #[serde(flatten)]
    pub descriptive: MetaServiceMemberProperties,
    pub name: String,
    pub http_method: String,
    pub handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaHttpUrlTemplate {
    #[serde(flatten)]
    pub descriptive: MetaServiceMemberProperties,
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
    /// Допустимая декларация платформы, которую профиль чтения не моделирует:
    /// её содержимое не разбирается и не выдаётся за пустое.
    Unmodelled {},
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
    #[serde(flatten)]
    pub descriptive: MetaServiceMemberProperties,
    pub name: String,
    pub r#type: MetaExpandedName,
    pub nillable: bool,
    pub direction: MetaTransferDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaWebServiceOperation {
    #[serde(flatten)]
    pub descriptive: MetaServiceMemberProperties,
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

/// Closed observations of descriptive properties on HTTP and SOAP members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetaServiceMemberProperties {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<MetaObservedProperty>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub property_diagnostics: Vec<MetaDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaServiceMemberContext {
    URLTemplate,
    HTTPMethod,
    Operation,
    Parameter,
}

impl MetaServiceMemberContext {
    pub const fn property_names(self) -> &'static [&'static str] {
        match self {
            Self::Operation => &["Synonym", "Comment", "DataLockControlMode"],
            Self::URLTemplate | Self::HTTPMethod | Self::Parameter => &["Synonym", "Comment"],
        }
    }
}

impl MetaServiceMemberContext {
    /// These indexed member facts are checked by the reader rather than the
    /// structural validator. Ownership includes the root kind and whole field.
    pub fn owns_diagnostic(diagnostic: &MetaDiagnostic) -> bool {
        let Some(path) = diagnostic.metadata_path.as_ref() else {
            return false;
        };
        let parts = path.segments().collect::<Vec<_>>();
        let [kind, _] = parts.as_slice() else {
            return false;
        };
        let Some(field) = diagnostic.field.as_deref() else {
            return false;
        };
        fn indexed<'a>(field: &'a str, prefix: &str) -> Option<&'a str> {
            let (index, rest) = field.strip_prefix(prefix)?.split_once(']')?;
            let number = index.parse::<usize>().ok()?;
            (number.to_string() == index).then_some(rest)
        }
        let (context, key) = match *kind {
            "HTTPService" => {
                let Some(rest) = indexed(field, "details.urlTemplates[") else {
                    return false;
                };
                if rest.starts_with(".methods[") {
                    let Some(key) = indexed(rest, ".methods[") else {
                        return false;
                    };
                    (Self::HTTPMethod, key)
                } else {
                    (Self::URLTemplate, rest)
                }
            }
            "WebService" => {
                let Some(rest) = indexed(field, "details.operations[") else {
                    return false;
                };
                if rest.starts_with(".parameters[") {
                    let Some(key) = indexed(rest, ".parameters[") else {
                        return false;
                    };
                    (Self::Parameter, key)
                } else {
                    (Self::Operation, rest)
                }
            }
            _ => return false,
        };
        match key {
            ".synonym" | ".comment" => true,
            ".dataLockControlMode" => context == Self::Operation,
            _ => false,
        }
    }
}

#[cfg(test)]
mod service_member_diagnostic_tests {
    use super::*;
    use crate::MetaDiagnosticCode;
    use unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20;

    #[test]
    fn service_diagnostic_profile_owns_exactly_nine_fields_of_the_matching_owner() {
        for (owner, fields) in [
            (
                "HTTPService.API",
                vec![
                    "details.urlTemplates[0].synonym",
                    "details.urlTemplates[0].comment",
                    "details.urlTemplates[12].methods[3].synonym",
                    "details.urlTemplates[12].methods[3].comment",
                ],
            ),
            (
                "WebService.API",
                vec![
                    "details.operations[0].synonym",
                    "details.operations[0].comment",
                    "details.operations[0].dataLockControlMode",
                    "details.operations[9].parameters[3].synonym",
                    "details.operations[9].parameters[3].comment",
                ],
            ),
        ] {
            for field in fields {
                let diagnostic =
                    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, "cause")
                        .with_metadata_path(
                            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, owner).unwrap(),
                        )
                        .with_field(field);
                assert!(
                    MetaServiceMemberContext::owns_diagnostic(&diagnostic),
                    "{owner}:{field}"
                );
            }
        }
    }

    #[test]
    fn service_diagnostic_profile_rejects_foreign_roots_fields_indices_and_tails() {
        for (owner, field) in [
            ("Catalog.API", "details.urlTemplates[0].comment"),
            ("WebService.API", "details.urlTemplates[0].comment"),
            ("HTTPService.API", "details.operations[0].comment"),
            (
                "HTTPService.API.Form.Form",
                "details.urlTemplates[0].comment",
            ),
            ("HTTPService.API", "details.urlTemplates[0].name"),
            (
                "HTTPService.API",
                "details.urlTemplates[0].methods[0].handler",
            ),
            (
                "WebService.API",
                "details.operations[0].parameters[0].dataLockControlMode",
            ),
            (
                "WebService.API",
                "details.operations[0].dataLockControlMode.extra",
            ),
            ("WebService.API", "details.operations[].comment"),
            ("WebService.API", "details.operations[-1].comment"),
            ("WebService.API", "details.operations[01].comment"),
            ("WebService.API", "details.operations[+1].comment"),
            ("WebService.API", "details.operations[0]].comment"),
            ("WebService.API", "properties.Comment"),
        ] {
            let diagnostic =
                MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, "cause")
                    .with_metadata_path(
                        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, owner).unwrap(),
                    )
                    .with_field(field);
            assert!(
                !MetaServiceMemberContext::owns_diagnostic(&diagnostic),
                "{owner}:{field}"
            );
        }
        assert!(!MetaServiceMemberContext::owns_diagnostic(
            &MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, "cause")
                .with_field("details.operations[0].comment")
        ));
    }
}
