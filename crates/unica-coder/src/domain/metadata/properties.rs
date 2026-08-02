use super::{MetaDiagnostic, MetaDiagnosticCode, MetadataKind};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) enum MetaPropertyKey {
    Synonym,
    Comment,
    NumberLength,
    CheckUnique,
    CodeLength,
    DescriptionLength,
    Hierarchical,
    Autonumbering,
    UseStandardCommands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaPropertyValueKind {
    String,
    Boolean,
    UnsignedInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum MetaPropertyValue {
    String(String),
    Boolean(bool),
    UnsignedInteger(u32),
}

impl MetaPropertyValue {
    fn kind(&self) -> MetaPropertyValueKind {
        match self {
            Self::String(_) => MetaPropertyValueKind::String,
            Self::Boolean(_) => MetaPropertyValueKind::Boolean,
            Self::UnsignedInteger(_) => MetaPropertyValueKind::UnsignedInteger,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaPropertyInput {
    pub(crate) name: String,
    pub(crate) value: MetaPropertyValue,
}

impl MetaPropertyInput {
    pub(crate) fn new(name: impl Into<String>, value: MetaPropertyValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataPropertySpec {
    pub(crate) public_name: &'static str,
    pub(crate) key: MetaPropertyKey,
    pub(crate) value_kind: MetaPropertyValueKind,
    pub(crate) allowed_kinds: &'static [MetadataKind],
}

const DOCUMENT_NUMBER_KINDS: &[MetadataKind] = &[
    MetadataKind::Document,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];
const CODE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ExchangePlan,
];
const DESCRIPTION_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
];
const HIERARCHICAL_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::ChartOfCharacteristicTypes,
];
const CHECK_UNIQUE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];
const AUTONUMBER_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];
const STANDARD_COMMAND_KINDS: &[MetadataKind] = &[
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

pub(crate) const METADATA_PROPERTY_SPECS: &[MetadataPropertySpec] = &[
    MetadataPropertySpec {
        public_name: "Synonym",
        key: MetaPropertyKey::Synonym,
        value_kind: MetaPropertyValueKind::String,
        allowed_kinds: MetadataKind::ALL,
    },
    MetadataPropertySpec {
        public_name: "Comment",
        key: MetaPropertyKey::Comment,
        value_kind: MetaPropertyValueKind::String,
        allowed_kinds: MetadataKind::ALL,
    },
    MetadataPropertySpec {
        public_name: "NumberLength",
        key: MetaPropertyKey::NumberLength,
        value_kind: MetaPropertyValueKind::UnsignedInteger,
        allowed_kinds: DOCUMENT_NUMBER_KINDS,
    },
    MetadataPropertySpec {
        public_name: "CheckUnique",
        key: MetaPropertyKey::CheckUnique,
        value_kind: MetaPropertyValueKind::Boolean,
        allowed_kinds: CHECK_UNIQUE_KINDS,
    },
    MetadataPropertySpec {
        public_name: "CodeLength",
        key: MetaPropertyKey::CodeLength,
        value_kind: MetaPropertyValueKind::UnsignedInteger,
        allowed_kinds: CODE_KINDS,
    },
    MetadataPropertySpec {
        public_name: "DescriptionLength",
        key: MetaPropertyKey::DescriptionLength,
        value_kind: MetaPropertyValueKind::UnsignedInteger,
        allowed_kinds: DESCRIPTION_KINDS,
    },
    MetadataPropertySpec {
        public_name: "Hierarchical",
        key: MetaPropertyKey::Hierarchical,
        value_kind: MetaPropertyValueKind::Boolean,
        allowed_kinds: HIERARCHICAL_KINDS,
    },
    MetadataPropertySpec {
        public_name: "Autonumbering",
        key: MetaPropertyKey::Autonumbering,
        value_kind: MetaPropertyValueKind::Boolean,
        allowed_kinds: AUTONUMBER_KINDS,
    },
    MetadataPropertySpec {
        public_name: "UseStandardCommands",
        key: MetaPropertyKey::UseStandardCommands,
        value_kind: MetaPropertyValueKind::Boolean,
        allowed_kinds: STANDARD_COMMAND_KINDS,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaPropertyChanges {
    entries: Vec<(MetaPropertyKey, MetaPropertyValue)>,
}

impl MetaPropertyChanges {
    pub(crate) fn convert(
        kind: MetadataKind,
        inputs: Vec<MetaPropertyInput>,
    ) -> Result<Self, MetaDiagnostic> {
        if inputs.is_empty() {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::InvalidArguments,
                "property changes must not be empty",
            )
            .with_field("values"));
        }
        let mut seen = HashSet::new();
        let mut entries = Vec::with_capacity(inputs.len());
        for input in inputs {
            let field = format!("values.{}", input.name);
            let spec = METADATA_PROPERTY_SPECS
                .iter()
                .find(|spec| spec.public_name == input.name)
                .ok_or_else(|| {
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::InvalidArguments,
                        format!("unknown metadata property `{}`", input.name),
                    )
                    .with_field(&field)
                })?;
            if !spec.allowed_kinds.contains(&kind) {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::UnsupportedKind,
                    format!(
                        "property `{}` is not supported for {}",
                        input.name,
                        kind.as_str()
                    ),
                )
                .with_field(field));
            }
            if input.value.kind() != spec.value_kind {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    format!("property `{}` has the wrong value kind", input.name),
                )
                .with_field(field));
            }
            if !seen.insert(spec.key) {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    format!("property `{}` is duplicated", input.name),
                )
                .with_field(field));
            }
            entries.push((spec.key, input.value));
        }
        Ok(Self { entries })
    }

    pub(crate) fn entries(&self) -> &[(MetaPropertyKey, MetaPropertyValue)] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::{MetaDiagnosticCode, MetadataKind};

    fn assert_property_kind_matrix(
        property_name: &str,
        value: MetaPropertyValue,
        allowed: &[MetadataKind],
    ) {
        for kind in MetadataKind::ALL {
            let result = MetaPropertyChanges::convert(
                *kind,
                vec![MetaPropertyInput::new(property_name, value.clone())],
            );
            assert_eq!(
                result.is_ok(),
                allowed.contains(kind),
                "{property_name} for {}",
                kind.as_str()
            );
        }
    }

    #[test]
    fn code_length_uses_the_current_writer_kind_matrix() {
        assert_property_kind_matrix(
            "CodeLength",
            MetaPropertyValue::UnsignedInteger(9),
            &[
                MetadataKind::Catalog,
                MetadataKind::ChartOfAccounts,
                MetadataKind::ChartOfCharacteristicTypes,
                MetadataKind::ChartOfCalculationTypes,
                MetadataKind::ExchangePlan,
            ],
        );
    }

    #[test]
    fn description_length_uses_its_distinct_current_writer_kind_matrix() {
        assert_property_kind_matrix(
            "DescriptionLength",
            MetaPropertyValue::UnsignedInteger(100),
            &[
                MetadataKind::Catalog,
                MetadataKind::ChartOfAccounts,
                MetadataKind::ChartOfCharacteristicTypes,
                MetadataKind::ChartOfCalculationTypes,
                MetadataKind::Task,
                MetadataKind::ExchangePlan,
            ],
        );
    }

    #[test]
    fn hierarchical_uses_the_current_writer_and_validator_kind_matrix() {
        assert_property_kind_matrix(
            "Hierarchical",
            MetaPropertyValue::Boolean(true),
            &[
                MetadataKind::Catalog,
                MetadataKind::ChartOfCharacteristicTypes,
            ],
        );
    }

    #[test]
    fn autonumbering_excludes_both_chart_kinds_forbidden_by_the_validator() {
        assert_property_kind_matrix(
            "Autonumbering",
            MetaPropertyValue::Boolean(true),
            &[
                MetadataKind::Catalog,
                MetadataKind::Document,
                MetadataKind::ChartOfCharacteristicTypes,
                MetadataKind::BusinessProcess,
                MetadataKind::Task,
            ],
        );
    }

    #[test]
    fn use_standard_commands_uses_the_current_writer_kind_matrix() {
        assert_property_kind_matrix(
            "UseStandardCommands",
            MetaPropertyValue::Boolean(true),
            &[
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
            ],
        );
    }

    #[test]
    fn property_conversion_rejects_unknown_public_key() {
        let diagnostic = MetaPropertyChanges::convert(
            MetadataKind::Document,
            vec![MetaPropertyInput::new(
                "UnknownProperty",
                MetaPropertyValue::Boolean(true),
            )],
        )
        .expect_err("unknown property must not survive conversion");

        assert_eq!(diagnostic.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(diagnostic.field.as_deref(), Some("values.UnknownProperty"));
    }

    #[test]
    fn property_conversion_rejects_a_known_property_for_the_wrong_kind() {
        let diagnostic = MetaPropertyChanges::convert(
            MetadataKind::Catalog,
            vec![MetaPropertyInput::new(
                "NumberLength",
                MetaPropertyValue::UnsignedInteger(12),
            )],
        )
        .expect_err("document number property is not a catalog property");

        assert_eq!(diagnostic.code, MetaDiagnosticCode::UnsupportedKind);
        assert_eq!(diagnostic.field.as_deref(), Some("values.NumberLength"));
    }

    #[test]
    fn property_conversion_retains_only_closed_keys_and_values() {
        let changes = MetaPropertyChanges::convert(
            MetadataKind::Document,
            vec![
                MetaPropertyInput::new("NumberLength", MetaPropertyValue::UnsignedInteger(12)),
                MetaPropertyInput::new("CheckUnique", MetaPropertyValue::Boolean(true)),
            ],
        )
        .unwrap();

        assert_eq!(
            changes.entries(),
            &[
                (
                    MetaPropertyKey::NumberLength,
                    MetaPropertyValue::UnsignedInteger(12)
                ),
                (
                    MetaPropertyKey::CheckUnique,
                    MetaPropertyValue::Boolean(true)
                ),
            ]
        );
    }

    #[test]
    fn property_key_serialization_uses_the_public_registry_name() {
        assert_eq!(
            serde_json::to_string(&MetaPropertyKey::NumberLength).unwrap(),
            "\"NumberLength\""
        );
    }
}
