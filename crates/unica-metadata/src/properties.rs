use super::{MetaDiagnostic, MetaDiagnosticCode, MetadataKind, MetadataType};
use serde::Serialize;
use std::collections::HashSet;

// Each property lives in `properties/<key>.rs`; build.rs declares those
// modules, the `MetaPropertyKey` enum and `METADATA_PROPERTY_SPECS` sorted by
// public name, so a new property does not edit a shared list here.
include!(concat!(env!("OUT_DIR"), "/properties_modules.rs"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaPropertyValueKind {
    Type,
    String,
    Boolean,
    UnsignedInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum MetaPropertyValue {
    Type(MetadataType),
    String(String),
    Boolean(bool),
    UnsignedInteger(u32),
}

impl MetaPropertyValue {
    fn kind(&self) -> MetaPropertyValueKind {
        match self {
            Self::Type(_) => MetaPropertyValueKind::Type,
            Self::String(_) => MetaPropertyValueKind::String,
            Self::Boolean(_) => MetaPropertyValueKind::Boolean,
            Self::UnsignedInteger(_) => MetaPropertyValueKind::UnsignedInteger,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaPropertyInput {
    pub name: String,
    pub value: MetaPropertyValue,
}

impl MetaPropertyInput {
    pub fn new(name: impl Into<String>, value: MetaPropertyValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataPropertySpec {
    pub public_name: &'static str,
    pub xml_name: &'static str,
    pub key: MetaPropertyKey,
    pub value_kind: MetaPropertyValueKind,
    pub allowed_kinds: &'static [MetadataKind],
    pub enum_values: &'static [&'static str],
    pub string_pattern: Option<&'static str>,
    /// Owners whose existing enum element `props.set` rewrites in place.
    pub in_place: InPlace,
}

/// Owners whose existing element of an enum property is written in place:
/// the single direct element must hold a confirmed value, and only its text
/// changes, keeping the qualified element and its namespace declarations.
/// Other owners use the general property writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InPlace {
    Never,
    /// Every owner the property allows.
    Always,
    /// Only these owners.
    Owners(&'static [MetadataKind]),
}

impl MetadataPropertySpec {
    /// Whether `props.set` rewrites this property of `kind` in place.
    pub fn in_place(&self, kind: MetadataKind) -> bool {
        self.allowed_kinds.contains(&kind)
            && match self.in_place {
                InPlace::Never => false,
                InPlace::Always => true,
                InPlace::Owners(owners) => owners.contains(&kind),
            }
    }

    const fn written_in_place(mut self, in_place: InPlace) -> Self {
        self.in_place = in_place;
        self
    }
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
        string_pattern: None,
        in_place: InPlace::Never,
    }
}

/// Provider-neutral metadata property vocabulary used by diagnostic focus.
/// Object properties come from the same registry as typed metadata mutation;
/// element-local properties are the closed names exposed by the typed element
/// model. XML-only field names intentionally fail this boundary.
pub fn diagnostic_metadata_property_is_canonical(value: &str) -> bool {
    const ELEMENT_PROPERTIES: &[&str] = &["Name", "Synonym", "Comment", "Type", "FillValue"];
    ELEMENT_PROPERTIES.contains(&value)
        || METADATA_PROPERTY_SPECS
            .iter()
            .any(|spec| spec.public_name == value)
}

pub fn unknown_metadata_property_message(kind: MetadataKind, name: &str) -> String {
    let mut seen = HashSet::new();
    let supported = METADATA_PROPERTY_SPECS
        .iter()
        .filter(|spec| spec.allowed_kinds.contains(&kind))
        .filter_map(|spec| seen.insert(spec.public_name).then_some(spec.public_name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "unknown metadata property `{name}`; supported properties for {}: {supported}",
        kind.as_str()
    )
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
        string_pattern: None,
        in_place: InPlace::Never,
    }
}

const fn patterned_string_property(
    public_name: &'static str,
    key: MetaPropertyKey,
    allowed_kinds: &'static [MetadataKind],
    string_pattern: &'static str,
) -> MetadataPropertySpec {
    MetadataPropertySpec {
        public_name,
        xml_name: public_name,
        key,
        value_kind: MetaPropertyValueKind::String,
        allowed_kinds,
        enum_values: &[],
        string_pattern: Some(string_pattern),
        in_place: InPlace::Never,
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
const EVENT_SUBSCRIPTION_KINDS: &[MetadataKind] = &[MetadataKind::EventSubscription];
const WEB_SERVICE_SESSION_KINDS: &[MetadataKind] =
    &[MetadataKind::HTTPService, MetadataKind::WebService];

const NUMBER_PERIODICITY_KINDS: &[MetadataKind] =
    &[MetadataKind::Document, MetadataKind::BusinessProcess];
const DOCUMENT_NUMBER_KINDS: &[MetadataKind] = &[
    MetadataKind::Document,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];
const CODE_TYPE_KINDS: &[MetadataKind] =
    &[MetadataKind::Catalog, MetadataKind::ChartOfCalculationTypes];
/// Kinds whose 8.3.27 descriptor carries a root `IncludeHelpInContents`.
///
/// Source: the Sendbox vendor configuration dumped by Designer 8.3.27
/// (format 2.20) and the v8-runner designer fixture
/// `tests/fixtures/designer/configuration` (format 2.20). The vendor dump
/// has no AccountingRegister, CalculationRegister, ChartOfAccounts or
/// ChartOfCalculationTypes; the v8-runner fixture confirms those four. Every
/// object of the listed kinds in both dumps has the element; no object of the
/// other typed kinds has it (Constant, Enum, CommonModule, ScheduledJob,
/// EventSubscription, HTTPService, WebService, DefinedType).

const POSTING_VALUES: &[&str] = &["Allow", "Deny"];
const REGISTER_RECORDS_DELETION_VALUES: &[&str] =
    &["AutoDelete", "AutoDeleteOnUnpost", "AutoDeleteOff"];
/// Platform enumeration `AccumulationRegisterType`: `Balance` (остатки) or
/// `Turnovers` (обороты). There is no `Balances` member.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaPropertyChanges {
    entries: Vec<(MetaPropertyKey, MetaPropertyValue)>,
}

impl MetaPropertyChanges {
    pub fn convert(
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
                    unknown_metadata_property_message(kind, &input.name),
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
                match spec.key {
                    MetaPropertyKey::Event
                        if value.is_empty()
                            || !value
                                .starts_with(|character: char| character.is_ascii_uppercase())
                            || !value
                                .chars()
                                .all(|character| character.is_ascii_alphanumeric()) =>
                    {
                        return Err(MetaDiagnostic::error(
                            MetaDiagnosticCode::InvalidArguments,
                            "Event must be a non-empty canonical platform event candidate",
                        )
                        .with_field(&field));
                    }
                    MetaPropertyKey::Handler => {
                        let parts = value.split('.').collect::<Vec<_>>();
                        if !matches!(parts.as_slice(), ["CommonModule", module, procedure]
                            if crate::identifier::is_1c_identifier(module)
                                && crate::identifier::is_1c_identifier(procedure))
                        {
                            return Err(MetaDiagnostic::error(
                                MetaDiagnosticCode::InvalidArguments,
                                "Handler must use CommonModule.<Module>.<Procedure>",
                            )
                            .with_field(&field));
                        }
                    }
                    _ => {}
                }
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

    pub fn entries(&self) -> &[(MetaPropertyKey, MetaPropertyValue)] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetaDiagnosticCode, MetadataKind};

    #[test]
    fn in_place_writes_only_confirmed_enum_elements_of_allowed_owners() {
        for spec in METADATA_PROPERTY_SPECS {
            if spec.in_place == InPlace::Never {
                continue;
            }
            assert!(
                spec.value_kind == MetaPropertyValueKind::String && !spec.enum_values.is_empty(),
                "{}: only an enum value can be confirmed before an in-place write",
                spec.public_name
            );
            if let InPlace::Owners(owners) = spec.in_place {
                assert!(
                    !owners.is_empty(),
                    "{}: use InPlace::Never",
                    spec.public_name
                );
                for owner in owners {
                    assert!(
                        spec.allowed_kinds.contains(owner),
                        "{}: in-place owner {} is not an allowed owner",
                        spec.public_name,
                        owner.as_str()
                    );
                }
            }
        }
    }

    pub(super) fn assert_property_kind_matrix(
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
    fn event_subscription_properties_are_owner_specific_canonical_strings() {
        let changes = MetaPropertyChanges::convert(
            MetadataKind::EventSubscription,
            vec![
                MetaPropertyInput::new(
                    "Event",
                    MetaPropertyValue::String("BeforeWrite".to_string()),
                ),
                MetaPropertyInput::new(
                    "Handler",
                    MetaPropertyValue::String(
                        "CommonModule.EventHandlers.OnBeforeWrite".to_string(),
                    ),
                ),
            ],
        )
        .expect("the complete scalar side of a subscription is public");
        assert_eq!(
            changes.entries(),
            &[
                (
                    MetaPropertyKey::Event,
                    MetaPropertyValue::String("BeforeWrite".to_string()),
                ),
                (
                    MetaPropertyKey::Handler,
                    MetaPropertyValue::String(
                        "CommonModule.EventHandlers.OnBeforeWrite".to_string(),
                    ),
                ),
            ]
        );

        for (kind, name, value) in [
            (MetadataKind::Catalog, "Event", "BeforeWrite"),
            (
                MetadataKind::Catalog,
                "Handler",
                "CommonModule.EventHandlers.OnBeforeWrite",
            ),
            (MetadataKind::EventSubscription, "Event", ""),
            (
                MetadataKind::EventSubscription,
                "Handler",
                "EventHandlers.OnBeforeWrite",
            ),
        ] {
            assert!(
                MetaPropertyChanges::convert(
                    kind,
                    vec![MetaPropertyInput::new(
                        name,
                        MetaPropertyValue::String(value.to_string()),
                    )],
                )
                .is_err(),
                "{kind:?}.{name} accepted invalid value {value:?}"
            );
        }
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
                &[
                    Catalog,
                    Enum,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    ChartOfAccounts,
                    ExchangePlan,
                ],
            ),
            (
                "ClientManagedApplication",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "ClientOrdinaryApplication",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "CodeAllowedLength",
                MetaPropertyValue::String("Fixed".into()),
                &[
                    Catalog,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    ExchangePlan,
                ],
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
                &[Catalog, ChartOfCalculationTypes],
            ),
            (
                "Correspondence",
                MetaPropertyValue::Boolean(true),
                &[AccountingRegister],
            ),
            (
                "FullTextSearch",
                MetaPropertyValue::String("Use".into()),
                &[Catalog, Document],
            ),
            (
                "SearchStringModeOnInputByString",
                MetaPropertyValue::String("Begin".into()),
                &[Catalog, Document],
            ),
            (
                "FullTextSearchOnInputByString",
                MetaPropertyValue::String("Use".into()),
                &[Catalog, Document],
            ),
            (
                "ChoiceDataGetModeOnInputByString",
                MetaPropertyValue::String("Directly".into()),
                &[Catalog, Document],
            ),
            (
                "CreateOnInput",
                MetaPropertyValue::String("Auto".into()),
                &[Catalog, Document],
            ),
            (
                "ChoiceHistoryOnInput",
                MetaPropertyValue::String("Auto".into()),
                &[Catalog, Document],
            ),
            (
                "DataLockControlMode",
                MetaPropertyValue::String("Managed".into()),
                &[
                    Catalog,
                    Document,
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
                ],
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
                &[Document, BusinessProcess, Task],
            ),
            (
                "NumberLength",
                MetaPropertyValue::UnsignedInteger(15),
                &[Document, BusinessProcess, Task],
            ),
            (
                "NumberPeriodicity",
                MetaPropertyValue::String("Month".into()),
                &[Document, BusinessProcess],
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
                "PredefinedDataUpdate",
                MetaPropertyValue::String("AutoUpdate".into()),
                &[
                    Catalog,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                ],
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
            (
                "RegisterType",
                MetaPropertyValue::String("Turnovers".into()),
                &[AccumulationRegister],
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
    fn unknown_common_module_property_names_list_the_supported_alternatives() {
        let diagnostic = MetaPropertyChanges::convert(
            MetadataKind::CommonModule,
            vec![MetaPropertyInput::new(
                "ClientOrdinaryApplications",
                MetaPropertyValue::Boolean(true),
            )],
        )
        .unwrap_err();

        assert_eq!(diagnostic.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(
            diagnostic.message,
            "unknown metadata property `ClientOrdinaryApplications`; supported properties for CommonModule: ClientManagedApplication, ClientOrdinaryApplication, Comment, ExternalConnection, Global, Privileged, ReturnValuesReuse, Server, ServerCall, Synonym"
        );
        assert_eq!(
            diagnostic.field.as_deref(),
            Some("values.ClientOrdinaryApplications")
        );
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

    #[test]
    fn diagnostic_focus_property_vocabulary_contains_only_typed_model_names() {
        for canonical in ["Name", "Synonym", "Comment", "Type", "FillValue"] {
            assert!(diagnostic_metadata_property_is_canonical(canonical));
        }
        assert!(!diagnostic_metadata_property_is_canonical("Required"));
    }
}
