use super::{
    DateFractions, MetaDiagnostic, MetaDiagnosticCode, MetadataMutationCapability, MetadataType,
    MetadataTypeVariant, NumberSign, StringLengthMode,
};
use crate::domain::source_target::MetadataAddress;
use serde::Serialize;
use std::collections::HashSet;

/// A type proved by reading platform XML.
///
/// This algebra is deliberately wider than [`MetadataTypeVariant`], which is
/// the public mutation allowlist. Adding a variant here must not grant write
/// access to `unica.meta.add` or `unica.meta.edit`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum ObservedMetadataTypeVariant {
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
    Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObservedMetadataType {
    pub(crate) variants: Vec<ObservedMetadataTypeVariant>,
    pub(crate) mutation_capability: MetadataMutationCapability,
}

impl ObservedMetadataType {
    pub(crate) fn new(variants: Vec<ObservedMetadataTypeVariant>) -> Result<Self, MetaDiagnostic> {
        if variants.is_empty() {
            return Err(observed_type_error(
                "observed type variants must not be empty",
            ));
        }
        let mut seen = HashSet::new();
        for variant in &variants {
            if !seen.insert(variant.clone()) {
                return Err(observed_type_error(
                    "duplicate observed platform type variant",
                ));
            }
        }
        let writable_variants = variants
            .iter()
            .cloned()
            .map(observed_variant_into_writer)
            .collect::<Vec<_>>();
        MetadataType::new(writable_variants).map_err(|diagnostic| {
            observed_type_error(format!(
                "observed platform type violates the format profile: {}",
                diagnostic.message
            ))
        })?;
        Ok(Self {
            variants,
            mutation_capability: MetadataMutationCapability::Editable,
        })
    }
}

fn observed_type_error(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message).with_field("type")
}

impl From<MetadataType> for ObservedMetadataType {
    fn from(value: MetadataType) -> Self {
        Self {
            variants: value
                .variants
                .into_iter()
                .map(ObservedMetadataTypeVariant::from)
                .collect(),
            mutation_capability: MetadataMutationCapability::Editable,
        }
    }
}

impl From<MetadataTypeVariant> for ObservedMetadataTypeVariant {
    fn from(value: MetadataTypeVariant) -> Self {
        match value {
            MetadataTypeVariant::String {
                length,
                allowed_length,
            } => Self::String {
                length,
                allowed_length,
            },
            MetadataTypeVariant::Number {
                digits,
                fraction,
                sign,
            } => Self::Number {
                digits,
                fraction,
                sign,
            },
            MetadataTypeVariant::Boolean => Self::Boolean,
            MetadataTypeVariant::Uuid => Self::Uuid,
            MetadataTypeVariant::Date { fractions } => Self::Date { fractions },
            MetadataTypeVariant::BinaryData {
                length,
                allowed_length,
            } => Self::BinaryData {
                length,
                allowed_length,
            },
            MetadataTypeVariant::ValueStorage => Self::ValueStorage,
            MetadataTypeVariant::Reference { metadata_path } => Self::Reference { metadata_path },
            MetadataTypeVariant::DefinedType { metadata_path } => {
                Self::DefinedType { metadata_path }
            }
        }
    }
}

impl TryFrom<ObservedMetadataType> for MetadataType {
    type Error = MetaDiagnostic;

    fn try_from(value: ObservedMetadataType) -> Result<Self, Self::Error> {
        if value.mutation_capability == MetadataMutationCapability::ReadOnly {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::InvalidArguments,
                "read-only observed metadata type cannot be used for mutation",
            )
            .with_field("type"));
        }
        let variants = value
            .variants
            .into_iter()
            .map(observed_variant_into_writer)
            .collect::<Vec<_>>();
        MetadataType::new(variants)
    }
}

fn observed_variant_into_writer(variant: ObservedMetadataTypeVariant) -> MetadataTypeVariant {
    match variant {
        ObservedMetadataTypeVariant::String {
            length,
            allowed_length,
        } => MetadataTypeVariant::String {
            length,
            allowed_length,
        },
        ObservedMetadataTypeVariant::Number {
            digits,
            fraction,
            sign,
        } => MetadataTypeVariant::Number {
            digits,
            fraction,
            sign,
        },
        ObservedMetadataTypeVariant::Boolean => MetadataTypeVariant::Boolean,
        ObservedMetadataTypeVariant::Date { fractions } => MetadataTypeVariant::Date { fractions },
        ObservedMetadataTypeVariant::BinaryData {
            length,
            allowed_length,
        } => MetadataTypeVariant::BinaryData {
            length,
            allowed_length,
        },
        ObservedMetadataTypeVariant::ValueStorage => MetadataTypeVariant::ValueStorage,
        ObservedMetadataTypeVariant::Reference { metadata_path } => {
            MetadataTypeVariant::Reference { metadata_path }
        }
        ObservedMetadataTypeVariant::DefinedType { metadata_path } => {
            MetadataTypeVariant::DefinedType { metadata_path }
        }
        ObservedMetadataTypeVariant::Uuid => MetadataTypeVariant::Uuid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_is_narrowed_only_by_the_explicit_observation_bridge() {
        let observed = ObservedMetadataType::new(vec![ObservedMetadataTypeVariant::Uuid]).unwrap();

        let editable = MetadataType::try_from(observed).unwrap();

        assert_eq!(editable.variants, vec![MetadataTypeVariant::Uuid]);
    }

    #[test]
    fn read_only_observation_cannot_be_narrowed_into_the_writer_algebra() {
        let observed = ObservedMetadataType {
            variants: vec![ObservedMetadataTypeVariant::Boolean],
            mutation_capability: MetadataMutationCapability::ReadOnly,
        };

        let diagnostic = MetadataType::try_from(observed).unwrap_err();

        assert_eq!(diagnostic.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(diagnostic.field.as_deref(), Some("type"));
    }

    #[test]
    fn malformed_platform_type_is_rejected_instead_of_downgraded_to_read_only() {
        let diagnostic = ObservedMetadataType::new(vec![ObservedMetadataTypeVariant::String {
            length: 2048,
            allowed_length: StringLengthMode::Variable,
        }])
        .unwrap_err();

        assert_eq!(diagnostic.code, MetaDiagnosticCode::ValidationFailed);
        assert_eq!(diagnostic.field.as_deref(), Some("type"));
    }
}
