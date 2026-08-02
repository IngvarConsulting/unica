use super::{MetaDiagnostic, MetaDiagnosticCode};
use crate::domain::source_target::{MetadataAddress, SourceTargetError};
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
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MetadataTypeIdentity {
    String,
    Number,
    Boolean,
    Date,
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
    pub(crate) fn parse(profile: &str, raw: &str) -> Result<Self, MetaDiagnostic> {
        MetadataAddress::parse(profile, raw)
            .map(|metadata_path| Self { metadata_path })
            .map_err(|error| address_diagnostic(raw, error))
    }
}

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
