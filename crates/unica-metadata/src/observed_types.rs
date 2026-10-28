use super::{
    DateFractions, MetaDiagnostic, MetaDiagnosticCode, MetadataMutationCapability, MetadataType,
    MetadataTypeVariant, NumberSign, StringLengthMode,
};
use serde::Serialize;
use std::collections::HashSet;
use unica_address::source_target::MetadataAddress;

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
pub enum ObservedMetadataTypeVariant {
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
    /// Generated object identity, readable but outside the mutation algebra.
    Object {
        metadata_path: MetadataAddress,
    },
    /// Generated record-set identity, readable but outside the mutation algebra.
    RecordSet {
        metadata_path: MetadataAddress,
    },
    DefinedType {
        metadata_path: MetadataAddress,
    },
    /// The characteristic type set of a chart of characteristic types
    /// (`cfg:Characteristic.X`): readable, but outside the writer algebra.
    CharacteristicTypes {
        metadata_path: MetadataAddress,
    },
    /// A generated type named by its UUID (`v8:TypeId`). Designer writes an
    /// extension's reference to a type of the extended configuration this
    /// way; the identifier is kept verbatim and is outside the writer algebra.
    TypeId {
        type_id: String,
    },
    Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedMetadataType {
    pub variants: Vec<ObservedMetadataTypeVariant>,
    pub mutation_capability: MetadataMutationCapability,
}

impl ObservedMetadataType {
    pub fn new(variants: Vec<ObservedMetadataTypeVariant>) -> Result<Self, MetaDiagnostic> {
        if variants.is_empty() {
            return Err(observed_type_error(
                "observed type variants must not be empty",
            ));
        }
        if variants.len() > 1
            && variants
                .iter()
                .any(|variant| matches!(variant, ObservedMetadataTypeVariant::ValueStorage))
        {
            return Err(observed_type_error(
                "ValueStorage must be the only type variant",
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
        // Read-only members do not waive scalar constraints or scalar identity.
        // Validate every writable member with the same algebra used by writes.
        let writable_variants = variants
            .iter()
            .cloned()
            .filter_map(observed_variant_into_writer)
            .collect::<Vec<_>>();
        let mutation_capability = if writable_variants.len() == variants.len() {
            MetadataMutationCapability::Editable
        } else {
            MetadataMutationCapability::ReadOnly
        };
        if !writable_variants.is_empty() {
            MetadataType::new(writable_variants).map_err(|diagnostic| {
                observed_type_error(format!(
                    "observed platform type violates the format profile: {}",
                    diagnostic.message
                ))
            })?;
        }
        Ok(Self {
            variants,
            mutation_capability,
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
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| observed_type_error("observed platform type has no writer form"))?;
        MetadataType::new(variants)
    }
}

fn observed_variant_into_writer(
    variant: ObservedMetadataTypeVariant,
) -> Option<MetadataTypeVariant> {
    Some(match variant {
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
        // These observed identities have no public writer representation.
        ObservedMetadataTypeVariant::CharacteristicTypes { .. }
        | ObservedMetadataTypeVariant::TypeId { .. }
        | ObservedMetadataTypeVariant::Object { .. }
        | ObservedMetadataTypeVariant::RecordSet { .. } => return None,
        ObservedMetadataTypeVariant::Uuid => MetadataTypeVariant::Uuid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readonly_members() -> Vec<ObservedMetadataTypeVariant> {
        let path = |name| {
            MetadataAddress::parse(
                unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                name,
            )
            .unwrap()
        };
        vec![
            ObservedMetadataTypeVariant::TypeId {
                type_id: "cc442f88-d708-4a83-af58-60c85a5d28a8".into(),
            },
            ObservedMetadataTypeVariant::CharacteristicTypes {
                metadata_path: path("ChartOfCharacteristicTypes.Properties"),
            },
            ObservedMetadataTypeVariant::Object {
                metadata_path: path("Catalog.Items"),
            },
            ObservedMetadataTypeVariant::RecordSet {
                metadata_path: path("InformationRegister.Facts"),
            },
        ]
    }

    #[test]
    fn readonly_members_do_not_bypass_scalar_validation() {
        for readonly in readonly_members() {
            for mut variants in [
                vec![ObservedMetadataTypeVariant::Number {
                    digits: 39,
                    fraction: 40,
                    sign: NumberSign::Any,
                }],
                vec![
                    ObservedMetadataTypeVariant::String {
                        length: 10,
                        allowed_length: StringLengthMode::Variable,
                    },
                    ObservedMetadataTypeVariant::String {
                        length: 20,
                        allowed_length: StringLengthMode::Variable,
                    },
                ],
                vec![ObservedMetadataTypeVariant::ValueStorage],
            ] {
                variants.push(readonly.clone());
                assert!(
                    ObservedMetadataType::new(variants.clone()).is_err(),
                    "{variants:?}"
                );
            }
        }
    }

    #[test]
    fn valid_readonly_members_remain_visible_and_cannot_be_written() {
        for readonly in readonly_members() {
            let variants = vec![ObservedMetadataTypeVariant::Boolean, readonly.clone()];
            let observed = ObservedMetadataType::new(variants.clone()).unwrap();
            assert_eq!(observed.variants, variants);
            assert_eq!(
                observed.mutation_capability,
                MetadataMutationCapability::ReadOnly
            );
            assert!(MetadataType::try_from(observed).is_err());
            assert!(MetadataType::try_from(ObservedMetadataType {
                variants,
                mutation_capability: MetadataMutationCapability::Editable
            })
            .is_err());
            assert!(ObservedMetadataType::new(vec![readonly.clone(), readonly]).is_err());
        }
    }

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
