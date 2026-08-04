use super::{MetaDiagnostic, MetaDiagnosticCode, MetadataKind};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) enum MetaPropertyKey {
    Synonym,
    Comment,
    ActionPeriod,
    ActionPeriodUse,
    AutoOrderByCode,
    BasePeriod,
    ChoiceMode,
    ClientManagedApplication,
    CodeAllowedLength,
    CodeMask,
    CodeType,
    Correspondence,
    DefaultPresentation,
    DependenceOnCalculationTypes,
    Description,
    DistributedInfoBase,
    EnableTotalsSplitting,
    ExternalConnection,
    FoldersOnTop,
    Global,
    HierarchyType,
    LevelCount,
    LimitLevelCount,
    MainFilterOnPeriod,
    MaxExtDimensionCount,
    Namespace,
    NumberAllowedLength,
    NumberLength,
    NumberPeriodicity,
    NumberType,
    OrderLength,
    PeriodAdjustmentLength,
    Periodicity,
    PostInPrivilegedMode,
    Posting,
    Predefined,
    Privileged,
    QuickChoice,
    RealTimePosting,
    RegisterRecordsDeletion,
    RegisterRecordsWritingOnPost,
    RegisterType,
    RestartCountOnFailure,
    RestartIntervalOnFailure,
    ReturnValuesReuse,
    ReuseSessions,
    #[serde(rename = "RootURL")]
    RootUrl,
    Server,
    ServerCall,
    SessionMaxAge,
    SubordinationUse,
    UnpostInPrivilegedMode,
    Use,
    WriteMode,
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
    pub(crate) xml_name: &'static str,
    pub(crate) key: MetaPropertyKey,
    pub(crate) value_kind: MetaPropertyValueKind,
    pub(crate) allowed_kinds: &'static [MetadataKind],
    pub(crate) enum_values: &'static [&'static str],
}

const fn property(
    public_name: &'static str,
    key: MetaPropertyKey,
    value_kind: MetaPropertyValueKind,
    allowed_kinds: &'static [MetadataKind],
) -> MetadataPropertySpec {
    MetadataPropertySpec {
        public_name,
        xml_name: public_name,
        key,
        value_kind,
        allowed_kinds,
        enum_values: &[],
    }
}

const fn enum_property(
    public_name: &'static str,
    xml_name: &'static str,
    key: MetaPropertyKey,
    allowed_kinds: &'static [MetadataKind],
    enum_values: &'static [&'static str],
) -> MetadataPropertySpec {
    MetadataPropertySpec {
        public_name,
        xml_name,
        key,
        value_kind: MetaPropertyValueKind::String,
        allowed_kinds,
        enum_values,
    }
}

const CATALOG_KINDS: &[MetadataKind] = &[MetadataKind::Catalog];
const DOCUMENT_KINDS: &[MetadataKind] = &[MetadataKind::Document];
const INFORMATION_REGISTER_KINDS: &[MetadataKind] = &[MetadataKind::InformationRegister];
const ACCUMULATION_REGISTER_KINDS: &[MetadataKind] = &[MetadataKind::AccumulationRegister];
const ACCOUNTING_REGISTER_KINDS: &[MetadataKind] = &[MetadataKind::AccountingRegister];
const CALCULATION_REGISTER_KINDS: &[MetadataKind] = &[MetadataKind::CalculationRegister];
const CHART_OF_ACCOUNTS_KINDS: &[MetadataKind] = &[MetadataKind::ChartOfAccounts];
const CHART_OF_CALCULATION_TYPES_KINDS: &[MetadataKind] = &[MetadataKind::ChartOfCalculationTypes];
const COMMON_MODULE_KINDS: &[MetadataKind] = &[MetadataKind::CommonModule];
const SCHEDULED_JOB_KINDS: &[MetadataKind] = &[MetadataKind::ScheduledJob];
const EXCHANGE_PLAN_KINDS: &[MetadataKind] = &[MetadataKind::ExchangePlan];
const HTTP_SERVICE_KINDS: &[MetadataKind] = &[MetadataKind::HTTPService];
const WEB_SERVICE_KINDS: &[MetadataKind] = &[MetadataKind::WebService];
const WEB_SERVICE_SESSION_KINDS: &[MetadataKind] =
    &[MetadataKind::HTTPService, MetadataKind::WebService];

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

const CHOICE_MODE_VALUES: &[&str] = &["BothWays", "FromForm", "QuickChoice"];
const CODE_ALLOWED_LENGTH_VALUES: &[&str] = &["Variable", "Fixed"];
const CODE_TYPE_VALUES: &[&str] = &["String", "Number"];
const DEFAULT_PRESENTATION_VALUES: &[&str] = &["AsDescription", "AsCode"];
const DEPENDENCE_VALUES: &[&str] = &["DontUse", "OnActionPeriod"];
const HIERARCHY_TYPE_VALUES: &[&str] = &["HierarchyFoldersAndItems", "HierarchyOfItems"];
const NUMBER_ALLOWED_LENGTH_VALUES: &[&str] = &["Variable", "Fixed"];
const NUMBER_PERIODICITY_VALUES: &[&str] = &["Day", "Month", "Nonperiodical", "Quarter", "Year"];
const NUMBER_TYPE_VALUES: &[&str] = &["String", "Number"];
const INFORMATION_REGISTER_PERIODICITY_VALUES: &[&str] = &[
    "Nonperiodical",
    "Second",
    "Day",
    "Month",
    "Quarter",
    "Year",
    "RecorderPosition",
];
const CALCULATION_REGISTER_PERIODICITY_VALUES: &[&str] = &["Day", "Month", "Quarter", "Year"];
const POSTING_VALUES: &[&str] = &["Allow", "Deny"];
const REGISTER_RECORDS_DELETION_VALUES: &[&str] =
    &["AutoDelete", "AutoDeleteOnUnpost", "AutoDeleteOff"];
const REGISTER_RECORDS_WRITING_VALUES: &[&str] = &["WriteModified", "WriteSelected", "WriteAll"];
const REGISTER_TYPE_VALUES: &[&str] = &["Balance", "Turnovers"];
const RETURN_VALUES_REUSE_VALUES: &[&str] = &["DontUse", "DuringRequest", "DuringSession"];
const REUSE_SESSIONS_VALUES: &[&str] = &["DontUse", "AutoUse"];
const SUBORDINATION_USE_VALUES: &[&str] = &["ToFolders", "ToFoldersAndItems", "ToItems"];
const WRITE_MODE_VALUES: &[&str] = &["Independent", "RecorderSubordinate"];

pub(crate) const METADATA_PROPERTY_SPECS: &[MetadataPropertySpec] = &[
    property(
        "Synonym",
        MetaPropertyKey::Synonym,
        MetaPropertyValueKind::String,
        MetadataKind::ALL,
    ),
    property(
        "Comment",
        MetaPropertyKey::Comment,
        MetaPropertyValueKind::String,
        MetadataKind::ALL,
    ),
    property(
        "ActionPeriod",
        MetaPropertyKey::ActionPeriod,
        MetaPropertyValueKind::Boolean,
        CALCULATION_REGISTER_KINDS,
    ),
    property(
        "ActionPeriodUse",
        MetaPropertyKey::ActionPeriodUse,
        MetaPropertyValueKind::Boolean,
        CHART_OF_CALCULATION_TYPES_KINDS,
    ),
    property(
        "AutoOrderByCode",
        MetaPropertyKey::AutoOrderByCode,
        MetaPropertyValueKind::Boolean,
        CHART_OF_ACCOUNTS_KINDS,
    ),
    property(
        "BasePeriod",
        MetaPropertyKey::BasePeriod,
        MetaPropertyValueKind::Boolean,
        CALCULATION_REGISTER_KINDS,
    ),
    enum_property(
        "ChoiceMode",
        "ChoiceMode",
        MetaPropertyKey::ChoiceMode,
        CATALOG_KINDS,
        CHOICE_MODE_VALUES,
    ),
    property(
        "ClientManagedApplication",
        MetaPropertyKey::ClientManagedApplication,
        MetaPropertyValueKind::Boolean,
        COMMON_MODULE_KINDS,
    ),
    enum_property(
        "CodeAllowedLength",
        "CodeAllowedLength",
        MetaPropertyKey::CodeAllowedLength,
        CATALOG_KINDS,
        CODE_ALLOWED_LENGTH_VALUES,
    ),
    property(
        "CodeMask",
        MetaPropertyKey::CodeMask,
        MetaPropertyValueKind::String,
        CHART_OF_ACCOUNTS_KINDS,
    ),
    enum_property(
        "CodeType",
        "CodeType",
        MetaPropertyKey::CodeType,
        CATALOG_KINDS,
        CODE_TYPE_VALUES,
    ),
    property(
        "Correspondence",
        MetaPropertyKey::Correspondence,
        MetaPropertyValueKind::Boolean,
        ACCOUNTING_REGISTER_KINDS,
    ),
    enum_property(
        "DefaultPresentation",
        "DefaultPresentation",
        MetaPropertyKey::DefaultPresentation,
        CATALOG_KINDS,
        DEFAULT_PRESENTATION_VALUES,
    ),
    enum_property(
        "DependenceOnCalculationTypes",
        "DependenceOnCalculationTypes",
        MetaPropertyKey::DependenceOnCalculationTypes,
        CHART_OF_CALCULATION_TYPES_KINDS,
        DEPENDENCE_VALUES,
    ),
    property(
        "Description",
        MetaPropertyKey::Description,
        MetaPropertyValueKind::String,
        SCHEDULED_JOB_KINDS,
    ),
    property(
        "DistributedInfoBase",
        MetaPropertyKey::DistributedInfoBase,
        MetaPropertyValueKind::Boolean,
        EXCHANGE_PLAN_KINDS,
    ),
    property(
        "EnableTotalsSplitting",
        MetaPropertyKey::EnableTotalsSplitting,
        MetaPropertyValueKind::Boolean,
        ACCUMULATION_REGISTER_KINDS,
    ),
    property(
        "ExternalConnection",
        MetaPropertyKey::ExternalConnection,
        MetaPropertyValueKind::Boolean,
        COMMON_MODULE_KINDS,
    ),
    property(
        "FoldersOnTop",
        MetaPropertyKey::FoldersOnTop,
        MetaPropertyValueKind::Boolean,
        CATALOG_KINDS,
    ),
    property(
        "Global",
        MetaPropertyKey::Global,
        MetaPropertyValueKind::Boolean,
        COMMON_MODULE_KINDS,
    ),
    enum_property(
        "HierarchyType",
        "HierarchyType",
        MetaPropertyKey::HierarchyType,
        CATALOG_KINDS,
        HIERARCHY_TYPE_VALUES,
    ),
    property(
        "LevelCount",
        MetaPropertyKey::LevelCount,
        MetaPropertyValueKind::UnsignedInteger,
        CATALOG_KINDS,
    ),
    property(
        "LimitLevelCount",
        MetaPropertyKey::LimitLevelCount,
        MetaPropertyValueKind::Boolean,
        CATALOG_KINDS,
    ),
    property(
        "MainFilterOnPeriod",
        MetaPropertyKey::MainFilterOnPeriod,
        MetaPropertyValueKind::Boolean,
        INFORMATION_REGISTER_KINDS,
    ),
    property(
        "MaxExtDimensionCount",
        MetaPropertyKey::MaxExtDimensionCount,
        MetaPropertyValueKind::UnsignedInteger,
        CHART_OF_ACCOUNTS_KINDS,
    ),
    property(
        "Namespace",
        MetaPropertyKey::Namespace,
        MetaPropertyValueKind::String,
        WEB_SERVICE_KINDS,
    ),
    enum_property(
        "NumberAllowedLength",
        "NumberAllowedLength",
        MetaPropertyKey::NumberAllowedLength,
        DOCUMENT_KINDS,
        NUMBER_ALLOWED_LENGTH_VALUES,
    ),
    enum_property(
        "NumberPeriodicity",
        "NumberPeriodicity",
        MetaPropertyKey::NumberPeriodicity,
        DOCUMENT_KINDS,
        NUMBER_PERIODICITY_VALUES,
    ),
    enum_property(
        "NumberType",
        "NumberType",
        MetaPropertyKey::NumberType,
        DOCUMENT_NUMBER_KINDS,
        NUMBER_TYPE_VALUES,
    ),
    property(
        "OrderLength",
        MetaPropertyKey::OrderLength,
        MetaPropertyValueKind::UnsignedInteger,
        CHART_OF_ACCOUNTS_KINDS,
    ),
    property(
        "PeriodAdjustmentLength",
        MetaPropertyKey::PeriodAdjustmentLength,
        MetaPropertyValueKind::UnsignedInteger,
        ACCOUNTING_REGISTER_KINDS,
    ),
    enum_property(
        "Periodicity",
        "InformationRegisterPeriodicity",
        MetaPropertyKey::Periodicity,
        INFORMATION_REGISTER_KINDS,
        INFORMATION_REGISTER_PERIODICITY_VALUES,
    ),
    enum_property(
        "Periodicity",
        "Periodicity",
        MetaPropertyKey::Periodicity,
        CALCULATION_REGISTER_KINDS,
        CALCULATION_REGISTER_PERIODICITY_VALUES,
    ),
    property(
        "PostInPrivilegedMode",
        MetaPropertyKey::PostInPrivilegedMode,
        MetaPropertyValueKind::Boolean,
        DOCUMENT_KINDS,
    ),
    enum_property(
        "Posting",
        "Posting",
        MetaPropertyKey::Posting,
        DOCUMENT_KINDS,
        POSTING_VALUES,
    ),
    property(
        "Predefined",
        MetaPropertyKey::Predefined,
        MetaPropertyValueKind::Boolean,
        SCHEDULED_JOB_KINDS,
    ),
    property(
        "Privileged",
        MetaPropertyKey::Privileged,
        MetaPropertyValueKind::Boolean,
        COMMON_MODULE_KINDS,
    ),
    property(
        "QuickChoice",
        MetaPropertyKey::QuickChoice,
        MetaPropertyValueKind::Boolean,
        CATALOG_KINDS,
    ),
    enum_property(
        "RealTimePosting",
        "RealTimePosting",
        MetaPropertyKey::RealTimePosting,
        DOCUMENT_KINDS,
        POSTING_VALUES,
    ),
    enum_property(
        "RegisterRecordsDeletion",
        "RegisterRecordsDeletion",
        MetaPropertyKey::RegisterRecordsDeletion,
        DOCUMENT_KINDS,
        REGISTER_RECORDS_DELETION_VALUES,
    ),
    enum_property(
        "RegisterRecordsWritingOnPost",
        "RegisterRecordsWritingOnPost",
        MetaPropertyKey::RegisterRecordsWritingOnPost,
        DOCUMENT_KINDS,
        REGISTER_RECORDS_WRITING_VALUES,
    ),
    enum_property(
        "RegisterType",
        "RegisterType",
        MetaPropertyKey::RegisterType,
        ACCUMULATION_REGISTER_KINDS,
        REGISTER_TYPE_VALUES,
    ),
    property(
        "RestartCountOnFailure",
        MetaPropertyKey::RestartCountOnFailure,
        MetaPropertyValueKind::UnsignedInteger,
        SCHEDULED_JOB_KINDS,
    ),
    property(
        "RestartIntervalOnFailure",
        MetaPropertyKey::RestartIntervalOnFailure,
        MetaPropertyValueKind::UnsignedInteger,
        SCHEDULED_JOB_KINDS,
    ),
    enum_property(
        "ReturnValuesReuse",
        "ReturnValuesReuse",
        MetaPropertyKey::ReturnValuesReuse,
        COMMON_MODULE_KINDS,
        RETURN_VALUES_REUSE_VALUES,
    ),
    enum_property(
        "ReuseSessions",
        "ReuseSessions",
        MetaPropertyKey::ReuseSessions,
        WEB_SERVICE_SESSION_KINDS,
        REUSE_SESSIONS_VALUES,
    ),
    property(
        "RootURL",
        MetaPropertyKey::RootUrl,
        MetaPropertyValueKind::String,
        HTTP_SERVICE_KINDS,
    ),
    property(
        "Server",
        MetaPropertyKey::Server,
        MetaPropertyValueKind::Boolean,
        COMMON_MODULE_KINDS,
    ),
    property(
        "ServerCall",
        MetaPropertyKey::ServerCall,
        MetaPropertyValueKind::Boolean,
        COMMON_MODULE_KINDS,
    ),
    property(
        "SessionMaxAge",
        MetaPropertyKey::SessionMaxAge,
        MetaPropertyValueKind::UnsignedInteger,
        WEB_SERVICE_SESSION_KINDS,
    ),
    enum_property(
        "SubordinationUse",
        "SubordinationUse",
        MetaPropertyKey::SubordinationUse,
        CATALOG_KINDS,
        SUBORDINATION_USE_VALUES,
    ),
    property(
        "UnpostInPrivilegedMode",
        MetaPropertyKey::UnpostInPrivilegedMode,
        MetaPropertyValueKind::Boolean,
        DOCUMENT_KINDS,
    ),
    property(
        "Use",
        MetaPropertyKey::Use,
        MetaPropertyValueKind::Boolean,
        SCHEDULED_JOB_KINDS,
    ),
    enum_property(
        "WriteMode",
        "WriteMode",
        MetaPropertyKey::WriteMode,
        INFORMATION_REGISTER_KINDS,
        WRITE_MODE_VALUES,
    ),
    property(
        "NumberLength",
        MetaPropertyKey::NumberLength,
        MetaPropertyValueKind::UnsignedInteger,
        DOCUMENT_NUMBER_KINDS,
    ),
    property(
        "CheckUnique",
        MetaPropertyKey::CheckUnique,
        MetaPropertyValueKind::Boolean,
        CHECK_UNIQUE_KINDS,
    ),
    property(
        "CodeLength",
        MetaPropertyKey::CodeLength,
        MetaPropertyValueKind::UnsignedInteger,
        CODE_KINDS,
    ),
    property(
        "DescriptionLength",
        MetaPropertyKey::DescriptionLength,
        MetaPropertyValueKind::UnsignedInteger,
        DESCRIPTION_KINDS,
    ),
    property(
        "Hierarchical",
        MetaPropertyKey::Hierarchical,
        MetaPropertyValueKind::Boolean,
        HIERARCHICAL_KINDS,
    ),
    property(
        "Autonumbering",
        MetaPropertyKey::Autonumbering,
        MetaPropertyValueKind::Boolean,
        AUTONUMBER_KINDS,
    ),
    property(
        "UseStandardCommands",
        MetaPropertyKey::UseStandardCommands,
        MetaPropertyValueKind::Boolean,
        STANDARD_COMMAND_KINDS,
    ),
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
            let matching = METADATA_PROPERTY_SPECS
                .iter()
                .filter(|spec| spec.public_name == input.name)
                .collect::<Vec<_>>();
            if matching.is_empty() {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    format!("unknown metadata property `{}`", input.name),
                )
                .with_field(&field));
            }
            let Some(spec) = matching
                .into_iter()
                .find(|spec| spec.allowed_kinds.contains(&kind))
            else {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::UnsupportedKind,
                    format!(
                        "property `{}` is not supported for {}",
                        input.name,
                        kind.as_str()
                    ),
                )
                .with_field(&field));
            };
            if input.value.kind() != spec.value_kind {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    format!("property `{}` has the wrong value kind", input.name),
                )
                .with_field(&field));
            }
            if let MetaPropertyValue::String(value) = &input.value {
                if !spec.enum_values.is_empty() && !spec.enum_values.contains(&value.as_str()) {
                    return Err(MetaDiagnostic::error(
                        MetaDiagnosticCode::InvalidArguments,
                        format!(
                            "property `{}` value `{value}` is invalid; expected one of: {}",
                            input.name,
                            spec.enum_values.join(", ")
                        ),
                    )
                    .with_field(&field));
                }
            }
            if !seen.insert(spec.key) {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    format!("property `{}` is duplicated", input.name),
                )
                .with_field(&field));
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
    fn retired_dsl_scalar_capabilities_have_exact_typed_kind_matrices() {
        use MetadataKind::*;

        let cases: &[(&str, MetaPropertyValue, &[MetadataKind])] = &[
            (
                "ActionPeriod",
                MetaPropertyValue::Boolean(true),
                &[CalculationRegister],
            ),
            (
                "ActionPeriodUse",
                MetaPropertyValue::Boolean(true),
                &[ChartOfCalculationTypes],
            ),
            (
                "AutoOrderByCode",
                MetaPropertyValue::Boolean(false),
                &[ChartOfAccounts],
            ),
            (
                "BasePeriod",
                MetaPropertyValue::Boolean(true),
                &[CalculationRegister],
            ),
            (
                "ChoiceMode",
                MetaPropertyValue::String("FromForm".into()),
                &[Catalog],
            ),
            (
                "ClientManagedApplication",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "CodeAllowedLength",
                MetaPropertyValue::String("Fixed".into()),
                &[Catalog],
            ),
            (
                "CodeLength",
                MetaPropertyValue::UnsignedInteger(12),
                &[
                    Catalog,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    ExchangePlan,
                ],
            ),
            (
                "CodeMask",
                MetaPropertyValue::String("@@@.@@".into()),
                &[ChartOfAccounts],
            ),
            (
                "CodeType",
                MetaPropertyValue::String("Number".into()),
                &[Catalog],
            ),
            (
                "Correspondence",
                MetaPropertyValue::Boolean(true),
                &[AccountingRegister],
            ),
            (
                "DefaultPresentation",
                MetaPropertyValue::String("AsCode".into()),
                &[Catalog],
            ),
            (
                "DependenceOnCalculationTypes",
                MetaPropertyValue::String("OnActionPeriod".into()),
                &[ChartOfCalculationTypes],
            ),
            (
                "Description",
                MetaPropertyValue::String("Night import".into()),
                &[ScheduledJob],
            ),
            (
                "DescriptionLength",
                MetaPropertyValue::UnsignedInteger(120),
                &[
                    Catalog,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    Task,
                    ExchangePlan,
                ],
            ),
            (
                "DistributedInfoBase",
                MetaPropertyValue::Boolean(true),
                &[ExchangePlan],
            ),
            (
                "EnableTotalsSplitting",
                MetaPropertyValue::Boolean(false),
                &[AccumulationRegister],
            ),
            (
                "ExternalConnection",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "FoldersOnTop",
                MetaPropertyValue::Boolean(false),
                &[Catalog],
            ),
            ("Global", MetaPropertyValue::Boolean(true), &[CommonModule]),
            (
                "HierarchyType",
                MetaPropertyValue::String("HierarchyOfItems".into()),
                &[Catalog],
            ),
            (
                "LevelCount",
                MetaPropertyValue::UnsignedInteger(4),
                &[Catalog],
            ),
            (
                "LimitLevelCount",
                MetaPropertyValue::Boolean(true),
                &[Catalog],
            ),
            (
                "MainFilterOnPeriod",
                MetaPropertyValue::Boolean(true),
                &[InformationRegister],
            ),
            (
                "MaxExtDimensionCount",
                MetaPropertyValue::UnsignedInteger(4),
                &[ChartOfAccounts],
            ),
            (
                "Namespace",
                MetaPropertyValue::String("urn:unica:test".into()),
                &[WebService],
            ),
            (
                "NumberAllowedLength",
                MetaPropertyValue::String("Fixed".into()),
                &[Document],
            ),
            (
                "NumberLength",
                MetaPropertyValue::UnsignedInteger(15),
                &[Document, BusinessProcess, Task],
            ),
            (
                "NumberPeriodicity",
                MetaPropertyValue::String("Month".into()),
                &[Document],
            ),
            (
                "NumberType",
                MetaPropertyValue::String("Number".into()),
                &[Document, BusinessProcess, Task],
            ),
            (
                "OrderLength",
                MetaPropertyValue::UnsignedInteger(6),
                &[ChartOfAccounts],
            ),
            (
                "PeriodAdjustmentLength",
                MetaPropertyValue::UnsignedInteger(2),
                &[AccountingRegister],
            ),
            (
                "Periodicity",
                MetaPropertyValue::String("Quarter".into()),
                &[InformationRegister, CalculationRegister],
            ),
            (
                "PostInPrivilegedMode",
                MetaPropertyValue::Boolean(false),
                &[Document],
            ),
            (
                "Posting",
                MetaPropertyValue::String("Deny".into()),
                &[Document],
            ),
            (
                "Predefined",
                MetaPropertyValue::Boolean(true),
                &[ScheduledJob],
            ),
            (
                "Privileged",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            ("QuickChoice", MetaPropertyValue::Boolean(true), &[Catalog]),
            (
                "RealTimePosting",
                MetaPropertyValue::String("Allow".into()),
                &[Document],
            ),
            (
                "RegisterRecordsDeletion",
                MetaPropertyValue::String("AutoDeleteOff".into()),
                &[Document],
            ),
            (
                "RegisterRecordsWritingOnPost",
                MetaPropertyValue::String("WriteAll".into()),
                &[Document],
            ),
            (
                "RegisterType",
                MetaPropertyValue::String("Turnovers".into()),
                &[AccumulationRegister],
            ),
            (
                "RestartCountOnFailure",
                MetaPropertyValue::UnsignedInteger(5),
                &[ScheduledJob],
            ),
            (
                "RestartIntervalOnFailure",
                MetaPropertyValue::UnsignedInteger(30),
                &[ScheduledJob],
            ),
            (
                "ReturnValuesReuse",
                MetaPropertyValue::String("DuringRequest".into()),
                &[CommonModule],
            ),
            (
                "ReuseSessions",
                MetaPropertyValue::String("AutoUse".into()),
                &[HTTPService, WebService],
            ),
            (
                "RootURL",
                MetaPropertyValue::String("v2".into()),
                &[HTTPService],
            ),
            ("Server", MetaPropertyValue::Boolean(false), &[CommonModule]),
            (
                "ServerCall",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "SessionMaxAge",
                MetaPropertyValue::UnsignedInteger(45),
                &[HTTPService, WebService],
            ),
            (
                "SubordinationUse",
                MetaPropertyValue::String("ToFoldersAndItems".into()),
                &[Catalog],
            ),
            (
                "UnpostInPrivilegedMode",
                MetaPropertyValue::Boolean(false),
                &[Document],
            ),
            ("Use", MetaPropertyValue::Boolean(true), &[ScheduledJob]),
            (
                "WriteMode",
                MetaPropertyValue::String("RecorderSubordinate".into()),
                &[InformationRegister],
            ),
        ];

        for (name, value, allowed) in cases {
            assert_property_kind_matrix(name, value.clone(), allowed);
        }
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
