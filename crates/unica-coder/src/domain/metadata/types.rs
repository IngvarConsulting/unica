use super::{EventSourceClass, MetaDiagnostic, MetaDiagnosticCode};
use crate::domain::source_target::MetadataAddress;
#[cfg(test)]
use crate::domain::source_target::SourceTargetError;
use serde::Serialize;
use std::collections::HashSet;

macro_rules! metadata_kinds {
    ($($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        pub(crate) enum MetadataKind {
            $($variant),+
        }

        impl MetadataKind {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub(crate) const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant)),+
                }
            }

            pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
                match value {
                    $(stringify!($variant) => Ok(Self::$variant)),+,
                    _ => Err(MetaDiagnostic::error(
                        MetaDiagnosticCode::UnsupportedKind,
                        format!("unsupported metadata kind `{value}`"),
                    )
                    .with_field("kind")),
                }
            }
        }
    };
}

metadata_kinds! {
    Catalog,
    Document,
    Enum,
    Constant,
    InformationRegister,
    AccumulationRegister,
    AccountingRegister,
    CalculationRegister,
    ChartOfAccounts,
    ChartOfCharacteristicTypes,
    ChartOfCalculationTypes,
    BusinessProcess,
    Task,
    ExchangePlan,
    DocumentJournal,
    Report,
    DataProcessor,
    CommonModule,
    ScheduledJob,
    EventSubscription,
    HTTPService,
    WebService,
    DefinedType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StringLengthMode {
    Variable,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NumberSign {
    Any,
    NonNegative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DateFractions {
    Date,
    Time,
    DateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MetaEventSourceDateFractions {
    Date,
    DateTime,
}

/// One source type of an EventSubscription.
///
/// This algebra is intentionally distinct from [`MetadataTypeVariant`]: an
/// event source names generated platform identities such as an object or a
/// record set, while a metadata field type names values and references.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum MetaEventSource {
    String {
        length: u32,
        allowed_length: StringLengthMode,
    },
    Number {
        digits: u32,
        fraction: u32,
        sign: NumberSign,
    },
    Boolean,
    Date {
        fractions: MetaEventSourceDateFractions,
    },
    ValueStorage,
    Object {
        metadata_path: MetadataAddress,
    },
    Manager {
        metadata_path: MetadataAddress,
    },
    Reference {
        metadata_path: MetadataAddress,
    },
    RecordSet {
        metadata_path: MetadataAddress,
    },
    DefinedType {
        metadata_path: MetadataAddress,
    },
    Family {
        source_class: EventSourceClass,
    },
}

const META_EVENT_SOURCE_OBJECT_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ExchangePlan,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::Report,
    MetadataKind::DataProcessor,
];

const META_EVENT_SOURCE_REFERENCE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::Enum,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ExchangePlan,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];

const META_EVENT_SOURCE_MANAGER_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::Enum,
    MetadataKind::Constant,
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
    MetadataKind::DocumentJournal,
    MetadataKind::Report,
    MetadataKind::DataProcessor,
];

const META_EVENT_SOURCE_RECORD_SET_KINDS: &[MetadataKind] = &[
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
];
const META_EVENT_SOURCE_DEFINED_TYPE_KINDS: &[MetadataKind] = &[MetadataKind::DefinedType];

pub(crate) fn metadata_event_source_object_kinds() -> &'static [MetadataKind] {
    META_EVENT_SOURCE_OBJECT_KINDS
}

pub(crate) fn metadata_event_source_manager_kinds() -> &'static [MetadataKind] {
    META_EVENT_SOURCE_MANAGER_KINDS
}

pub(crate) fn metadata_event_source_record_set_kinds() -> &'static [MetadataKind] {
    META_EVENT_SOURCE_RECORD_SET_KINDS
}

impl MetaEventSource {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::String { .. } => "string",
            Self::Number { .. } => "number",
            Self::Boolean => "boolean",
            Self::Date { .. } => "date",
            Self::ValueStorage => "valueStorage",
            Self::Object { .. } => "object",
            Self::Manager { .. } => "manager",
            Self::Reference { .. } => "reference",
            Self::RecordSet { .. } => "recordSet",
            Self::DefinedType { .. } => "definedType",
            Self::Family { .. } => "family",
        }
    }

    pub(crate) fn metadata_path(&self) -> Option<&MetadataAddress> {
        match self {
            Self::Object { metadata_path }
            | Self::Manager { metadata_path }
            | Self::Reference { metadata_path }
            | Self::RecordSet { metadata_path }
            | Self::DefinedType { metadata_path } => Some(metadata_path),
            Self::String { .. }
            | Self::Number { .. }
            | Self::Boolean
            | Self::Date { .. }
            | Self::ValueStorage
            | Self::Family { .. } => None,
        }
    }

    pub(crate) const fn compatible_metadata_kinds(&self) -> Option<&'static [MetadataKind]> {
        match self {
            Self::Object { .. } => Some(META_EVENT_SOURCE_OBJECT_KINDS),
            Self::Manager { .. } => Some(META_EVENT_SOURCE_MANAGER_KINDS),
            Self::Reference { .. } => Some(META_EVENT_SOURCE_REFERENCE_KINDS),
            Self::RecordSet { .. } => Some(META_EVENT_SOURCE_RECORD_SET_KINDS),
            Self::DefinedType { .. } => Some(META_EVENT_SOURCE_DEFINED_TYPE_KINDS),
            Self::String { .. }
            | Self::Number { .. }
            | Self::Boolean
            | Self::Date { .. }
            | Self::ValueStorage
            | Self::Family { .. } => None,
        }
    }

    /// Stable platform identity used for duplicate detection. Primitive
    /// qualifiers describe the one primitive identity and therefore do not
    /// make a second string/number/date source distinct.
    pub(crate) fn identity_key(&self) -> String {
        if let Self::Family { source_class } = self {
            return format!("family:{}", source_class.as_str());
        }
        match self.metadata_path() {
            Some(path) => format!("{}:{}", self.as_str(), path.as_str().to_lowercase()),
            None => self.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum MetadataTypeVariant {
    String {
        length: u32,
        allowed_length: StringLengthMode,
    },
    Number {
        digits: u32,
        fraction: u32,
        sign: NumberSign,
    },
    Boolean,
    Date {
        fractions: DateFractions,
    },
    BinaryData {
        length: u32,
        allowed_length: StringLengthMode,
    },
    ValueStorage,
    Reference {
        metadata_path: MetadataAddress,
    },
    DefinedType {
        metadata_path: MetadataAddress,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataType {
    pub(crate) variants: Vec<MetadataTypeVariant>,
}

impl MetadataType {
    pub(crate) fn new(variants: Vec<MetadataTypeVariant>) -> Result<Self, MetaDiagnostic> {
        if variants.is_empty() {
            return Err(invalid_type(
                "type.variants",
                "type variants must not be empty",
            ));
        }
        if variants.len() > 1
            && variants
                .iter()
                .any(|variant| matches!(variant, MetadataTypeVariant::ValueStorage))
        {
            return Err(invalid_type(
                "type.variants",
                "ValueStorage must be the only type variant",
            ));
        }

        let mut seen = HashSet::new();
        for (index, variant) in variants.iter().enumerate() {
            validate_variant(variant, index)?;
            let identity = variant_identity(variant);
            if !seen.insert(identity) {
                return Err(invalid_type(
                    format!("type.variants[{index}]"),
                    "duplicate platform type variant",
                ));
            }
        }
        Ok(Self { variants })
    }
}

fn invalid_type(field: impl Into<String>, message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message).with_field(field)
}

fn validate_variant(variant: &MetadataTypeVariant, index: usize) -> Result<(), MetaDiagnostic> {
    let field = || format!("type.variants[{index}]");
    match variant {
        MetadataTypeVariant::Reference { metadata_path }
        | MetadataTypeVariant::DefinedType { metadata_path }
            if metadata_path.segments().count() != 2 =>
        {
            Err(invalid_type(
                format!("{}.metadataPath", field()),
                "reference-like type metadataPath must identify a top-level metadata object",
            ))
        }
        MetadataTypeVariant::String {
            length,
            allowed_length,
        } if *length > 1024 || (*allowed_length == StringLengthMode::Fixed && *length == 0) => {
            Err(invalid_type(
                field(),
                "string length must be 0..=1024 and Fixed requires a positive length",
            ))
        }
        MetadataTypeVariant::Number {
            digits, fraction, ..
        } if *digits > 38 || *fraction > *digits => Err(invalid_type(
            field(),
            "number digits must be 0..=38 and fraction must not exceed digits",
        )),
        MetadataTypeVariant::BinaryData {
            length,
            allowed_length,
        } if *allowed_length == StringLengthMode::Fixed && *length == 0 => Err(invalid_type(
            field(),
            "fixed binary data requires a positive length",
        )),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MetadataTypeIdentity {
    String,
    Number,
    Boolean,
    Date,
    BinaryData,
    ValueStorage,
    Reference(MetadataAddress),
    DefinedType(MetadataAddress),
}

fn variant_identity(variant: &MetadataTypeVariant) -> MetadataTypeIdentity {
    match variant {
        MetadataTypeVariant::String { .. } => MetadataTypeIdentity::String,
        MetadataTypeVariant::Number { .. } => MetadataTypeIdentity::Number,
        MetadataTypeVariant::Boolean => MetadataTypeIdentity::Boolean,
        MetadataTypeVariant::Date { .. } => MetadataTypeIdentity::Date,
        MetadataTypeVariant::BinaryData { .. } => MetadataTypeIdentity::BinaryData,
        MetadataTypeVariant::ValueStorage => MetadataTypeIdentity::ValueStorage,
        MetadataTypeVariant::Reference { metadata_path } => {
            MetadataTypeIdentity::Reference(metadata_path.clone())
        }
        MetadataTypeVariant::DefinedType { metadata_path } => {
            MetadataTypeIdentity::DefinedType(metadata_path.clone())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataReference {
    pub(crate) metadata_path: MetadataAddress,
}

impl MetadataReference {
    #[cfg(test)]
    pub(crate) fn parse(profile: &str, raw: &str) -> Result<Self, MetaDiagnostic> {
        MetadataAddress::parse(profile, raw)
            .map(|metadata_path| Self { metadata_path })
            .map_err(|error| address_diagnostic(raw, error))
    }
}

#[cfg(test)]
fn address_diagnostic(raw: &str, error: SourceTargetError) -> MetaDiagnostic {
    MetaDiagnostic::error(
        MetaDiagnosticCode::InvalidArguments,
        format!("invalid metadataPath `{raw}`: {}", error.message),
    )
    .with_field("metadataPath")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20;

    #[test]
    fn metadata_kind_accepts_exactly_the_23_creation_spellings() {
        const EXPECTED: &[&str] = &[
            "Catalog",
            "Document",
            "Enum",
            "Constant",
            "InformationRegister",
            "AccumulationRegister",
            "AccountingRegister",
            "CalculationRegister",
            "ChartOfAccounts",
            "ChartOfCharacteristicTypes",
            "ChartOfCalculationTypes",
            "BusinessProcess",
            "Task",
            "ExchangePlan",
            "DocumentJournal",
            "Report",
            "DataProcessor",
            "CommonModule",
            "ScheduledJob",
            "EventSubscription",
            "HTTPService",
            "WebService",
            "DefinedType",
        ];

        assert_eq!(
            MetadataKind::ALL
                .iter()
                .copied()
                .map(MetadataKind::as_str)
                .collect::<Vec<_>>(),
            EXPECTED
        );
        for spelling in EXPECTED {
            assert_eq!(
                MetadataKind::parse(spelling).map(MetadataKind::as_str),
                Ok(*spelling)
            );
        }
    }

    #[test]
    fn reference_like_types_require_top_level_metadata_addresses() {
        for variant in [
            MetadataTypeVariant::Reference {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Catalog.Products.Form.Main",
                )
                .unwrap(),
            },
            MetadataTypeVariant::DefinedType {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "DefinedType.Code.Form.Main",
                )
                .unwrap(),
            },
        ] {
            let error = MetadataType::new(vec![variant]).unwrap_err();
            assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
            assert_eq!(
                error.field.as_deref(),
                Some("type.variants[0].metadataPath")
            );
        }
    }

    #[test]
    fn metadata_kind_rejects_non_creation_unknown_and_case_aliases() {
        for spelling in ["Bot", "SyntheticMetadata", "catalog", "CATALOG"] {
            let diagnostic = MetadataKind::parse(spelling).expect_err(spelling);
            assert_eq!(diagnostic.code, MetaDiagnosticCode::UnsupportedKind);
            assert_eq!(diagnostic.field.as_deref(), Some("kind"));
        }
    }

    #[test]
    fn metadata_type_rejects_empty_duplicate_and_value_storage_composites() {
        assert_eq!(
            MetadataType::new(vec![]).unwrap_err().code,
            MetaDiagnosticCode::InvalidArguments
        );
        assert_eq!(
            MetadataType::new(vec![
                MetadataTypeVariant::String {
                    length: 10,
                    allowed_length: StringLengthMode::Variable,
                },
                MetadataTypeVariant::String {
                    length: 20,
                    allowed_length: StringLengthMode::Variable,
                },
            ])
            .unwrap_err()
            .field
            .as_deref(),
            Some("type.variants[1]")
        );
        assert!(MetadataType::new(vec![
            MetadataTypeVariant::ValueStorage,
            MetadataTypeVariant::Boolean,
        ])
        .is_err());
    }

    #[test]
    fn metadata_type_enforces_string_and_number_bounds() {
        assert!(MetadataType::new(vec![MetadataTypeVariant::String {
            length: 1024,
            allowed_length: StringLengthMode::Variable,
        }])
        .is_ok());
        assert!(MetadataType::new(vec![MetadataTypeVariant::String {
            length: 1025,
            allowed_length: StringLengthMode::Variable,
        }])
        .is_err());
        assert!(MetadataType::new(vec![MetadataTypeVariant::String {
            length: 0,
            allowed_length: StringLengthMode::Fixed,
        }])
        .is_err());
        assert!(MetadataType::new(vec![MetadataTypeVariant::Number {
            digits: 38,
            fraction: 38,
            sign: NumberSign::Any,
        }])
        .is_ok());
        assert!(MetadataType::new(vec![MetadataTypeVariant::Number {
            digits: 39,
            fraction: 0,
            sign: NumberSign::NonNegative,
        }])
        .is_err());
        assert!(MetadataType::new(vec![MetadataTypeVariant::Number {
            digits: 10,
            fraction: 11,
            sign: NumberSign::Any,
        }])
        .is_err());
    }

    #[test]
    fn metadata_type_identity_uses_domain_discriminants_and_logical_addresses() {
        let catalog =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Products").unwrap();
        assert_eq!(
            variant_identity(&MetadataTypeVariant::String {
                length: 10,
                allowed_length: StringLengthMode::Variable,
            }),
            MetadataTypeIdentity::String
        );
        assert_eq!(
            variant_identity(&MetadataTypeVariant::Number {
                digits: 15,
                fraction: 2,
                sign: NumberSign::Any,
            }),
            MetadataTypeIdentity::Number
        );
        assert_eq!(
            variant_identity(&MetadataTypeVariant::ValueStorage),
            MetadataTypeIdentity::ValueStorage
        );
        assert_eq!(
            variant_identity(&MetadataTypeVariant::Reference {
                metadata_path: catalog.clone(),
            }),
            MetadataTypeIdentity::Reference(catalog)
        );
    }

    #[test]
    fn metadata_type_serialization_contains_no_provider_qnames() {
        let value_type = MetadataType::new(vec![
            MetadataTypeVariant::String {
                length: 10,
                allowed_length: StringLengthMode::Variable,
            },
            MetadataTypeVariant::Number {
                digits: 15,
                fraction: 2,
                sign: NumberSign::Any,
            },
        ])
        .unwrap();

        let serialized = serde_json::to_string(&value_type).unwrap();
        assert!(!serialized.contains("xs:"), "{serialized}");
        assert!(!serialized.contains("v8:"), "{serialized}");
    }

    #[test]
    fn event_source_serialization_uses_the_public_typed_shape() {
        let record_set =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "InformationRegister.Facts")
                .unwrap();
        let manager =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Products").unwrap();
        assert_eq!(
            serde_json::to_value(MetaEventSource::Manager {
                metadata_path: manager,
            })
            .unwrap(),
            serde_json::json!({
                "kind": "manager",
                "metadataPath": "Catalog.Products",
            })
        );
        assert_eq!(
            serde_json::to_value(MetaEventSource::Family {
                source_class: EventSourceClass::CatalogObject,
            })
            .unwrap(),
            serde_json::json!({
                "kind": "family",
                "sourceClass": "catalogObject",
            })
        );
        assert_eq!(
            serde_json::to_value(MetaEventSource::RecordSet {
                metadata_path: record_set,
            })
            .unwrap(),
            serde_json::json!({
                "kind": "recordSet",
                "metadataPath": "InformationRegister.Facts",
            })
        );
    }

    #[test]
    fn metadata_reference_reuses_the_logical_address_parser() {
        let reference =
            MetadataReference::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Products")
                .expect("canonical logical reference");
        assert_eq!(reference.metadata_path.as_str(), "Catalog.Products");

        let diagnostic = MetadataReference::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog")
            .expect_err("incomplete logical address must not be reparsed loosely");
        assert_eq!(diagnostic.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(diagnostic.field.as_deref(), Some("metadataPath"));
    }
}
