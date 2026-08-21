use super::{MetaObservedPropertyValue, MetaPropertyKey, MetaPropertyValueKind, MetadataKind};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaInfoPropertyValueKind {
    String,
    LegacyLocalizedString,
    LocalizedString,
    Boolean,
    UnsignedInteger,
    TypedValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum MetaInfoPropertyValue {
    String(String),
    Boolean(bool),
    UnsignedInteger(u32),
    Structured(MetaObservedPropertyValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaInfoPropertySpec {
    pub(crate) key: String,
    pub(crate) value_kind: MetaInfoPropertyValueKind,
}

/// Closed read-side profile for root properties.
///
/// This profile intentionally does not inspect `METADATA_PROPERTY_SPECS` or
/// its `allowed_kinds`: those entries grant mutation rights, while this table
/// only states what `unica.meta.info` may observe.
pub(crate) struct MetaInfoPropertyProfile;

pub(crate) const META_INFO_PROPERTY_PROFILE: MetaInfoPropertyProfile = MetaInfoPropertyProfile;

/// Enumerable vocabulary of the independent root-property read profile.
///
/// `resolve` deliberately refuses names outside this list, so adding a route
/// cannot bypass the executable declared-vs-observed coverage guard.
pub(crate) const META_INFO_PROPERTY_NAMES: &[&str] = &[
    "ActionPeriod",
    "ActionPeriodUse",
    "Addressing",
    "AutoOrderByCode",
    "Autonumbering",
    "AuxiliaryChoiceForm",
    "AuxiliaryFolderChoiceForm",
    "AuxiliaryFolderForm",
    "AuxiliaryForm",
    "AuxiliaryListForm",
    "AuxiliaryObjectForm",
    "AuxiliaryRecordForm",
    "AuxiliarySettingsForm",
    "AuxiliaryVariantForm",
    "BasePeriod",
    "CharacteristicExtValues",
    "ChartOfAccounts",
    "ChartOfCalculationTypes",
    "CheckUnique",
    "ChoiceDataGetModeOnInputByString",
    "ChoiceFoldersAndItems",
    "ChoiceForm",
    "ChoiceHistoryOnInput",
    "ChoiceMode",
    "Client",
    "ClientManagedApplication",
    "ClientOrdinaryApplication",
    "CodeAllowedLength",
    "CodeLength",
    "CodeMask",
    "CodeSeries",
    "CodeType",
    "Comment",
    "Correspondence",
    "CreateOnInput",
    "CreateTaskInPrivilegedMode",
    "CurrentPerformer",
    "DataHistory",
    "DataLockControlMode",
    "DefaultChoiceForm",
    "DefaultFolderChoiceForm",
    "DefaultFolderForm",
    "DefaultForm",
    "DefaultListForm",
    "DefaultObjectForm",
    "DefaultPresentation",
    "DefaultRecordForm",
    "DefaultSettingsForm",
    "DefaultVariantForm",
    "DependenceOnCalculationTypes",
    "Description",
    "DescriptionLength",
    "DescriptorFileName",
    "DistributedInfoBase",
    "EditFormat",
    "EditType",
    "EnableTotalsSliceFirst",
    "EnableTotalsSliceLast",
    "EnableTotalsSplitting",
    "Event",
    "ExecuteAfterWriteDataHistoryVersionProcessing",
    "Explanation",
    "ExtDimensionTypes",
    "ExtendedEdit",
    "ExtendedListPresentation",
    "ExtendedObjectPresentation",
    "ExtendedPresentation",
    "ExtendedRecordPresentation",
    "ExternalConnection",
    "FillChecking",
    "FoldersOnTop",
    "Format",
    "FullTextSearch",
    "FullTextSearchOnInputByString",
    "Global",
    "Handler",
    "Hierarchical",
    "HierarchyType",
    "IncludeConfigurationExtensions",
    "IncludeHelpInContents",
    "InformationRegisterPeriodicity",
    "Key",
    "LevelCount",
    "LimitLevelCount",
    "LinkByType",
    "ListPresentation",
    "MainAddressingAttribute",
    "MainDataCompositionSchema",
    "MainFilterOnPeriod",
    "MarkNegatives",
    "Mask",
    "MaxExtDimensionCount",
    "MaxValue",
    "MinValue",
    "MultiLine",
    "Namespace",
    "NumberAllowedLength",
    "NumberLength",
    "NumberPeriodicity",
    "NumberType",
    "Numerator",
    "ObjectPresentation",
    "OrderLength",
    "PasswordMode",
    "PeriodAdjustmentLength",
    "Periodicity",
    "PostInPrivilegedMode",
    "Posting",
    "Predefined",
    "PredefinedDataUpdate",
    "Privileged",
    "QuickChoice",
    "RealTimePosting",
    "RecordPresentation",
    "RegisterRecordsDeletion",
    "RegisterRecordsWritingOnPost",
    "RegisterType",
    "RestartCountOnFailure",
    "RestartIntervalOnFailure",
    "ReturnValuesReuse",
    "ReuseSessions",
    "RootURL",
    "SearchStringModeOnInputByString",
    "SequenceFilling",
    "Server",
    "ServerCall",
    "SessionMaxAge",
    "SettingsStorage",
    "SubordinationUse",
    "Synonym",
    "Task",
    "TaskNumberAutoPrefix",
    "ToolTip",
    "UnpostInPrivilegedMode",
    "UpdateDataHistoryImmediatelyAfterWrite",
    "Use",
    "UseStandardCommands",
    "VariantsStorage",
    "WriteMode",
];

impl MetaInfoPropertyProfile {
    pub(crate) fn resolve(
        self,
        kind: MetadataKind,
        xml_name: &str,
    ) -> Option<MetaInfoPropertySpec> {
        if !META_INFO_PROPERTY_NAMES.contains(&xml_name) {
            return None;
        }
        if let Some((key, mutation_key)) = mutation_compatible_read_key(xml_name) {
            if mutation_key == MetaPropertyKey::Periodicity
                && !matches!(
                    (kind, xml_name),
                    (
                        MetadataKind::InformationRegister,
                        "InformationRegisterPeriodicity"
                    ) | (MetadataKind::CalculationRegister, "Periodicity")
                )
            {
                return None;
            }
            if mutation_property_is_observable(kind, mutation_key) {
                return Some(MetaInfoPropertySpec {
                    key: key.to_string(),
                    value_kind: if mutation_key == MetaPropertyKey::Synonym {
                        MetaInfoPropertyValueKind::LegacyLocalizedString
                    } else {
                        mutation_value_kind(mutation_key).into()
                    },
                });
            }
        }
        if additional_property_is_observable(kind, xml_name) {
            return Some(MetaInfoPropertySpec {
                key: xml_name.to_string(),
                value_kind: additional_value_kind(kind, xml_name),
            });
        }
        None
    }
}

impl From<MetaPropertyValueKind> for MetaInfoPropertyValueKind {
    fn from(value: MetaPropertyValueKind) -> Self {
        match value {
            MetaPropertyValueKind::String => Self::String,
            MetaPropertyValueKind::Boolean => Self::Boolean,
            MetaPropertyValueKind::UnsignedInteger => Self::UnsignedInteger,
        }
    }
}

fn mutation_compatible_read_key(xml_name: &str) -> Option<(&'static str, MetaPropertyKey)> {
    use MetaPropertyKey::*;
    Some(match xml_name {
        "Synonym" => ("Synonym", Synonym),
        "Comment" => ("Comment", Comment),
        "ActionPeriod" => ("ActionPeriod", ActionPeriod),
        "ActionPeriodUse" => ("ActionPeriodUse", ActionPeriodUse),
        "AutoOrderByCode" => ("AutoOrderByCode", AutoOrderByCode),
        "BasePeriod" => ("BasePeriod", BasePeriod),
        "ChoiceMode" => ("ChoiceMode", ChoiceMode),
        "ClientManagedApplication" => ("ClientManagedApplication", ClientManagedApplication),
        "CodeAllowedLength" => ("CodeAllowedLength", CodeAllowedLength),
        "CodeMask" => ("CodeMask", CodeMask),
        "CodeType" => ("CodeType", CodeType),
        "Correspondence" => ("Correspondence", Correspondence),
        "DefaultPresentation" => ("DefaultPresentation", DefaultPresentation),
        "DependenceOnCalculationTypes" => {
            ("DependenceOnCalculationTypes", DependenceOnCalculationTypes)
        }
        "Description" => ("Description", Description),
        "Event" => ("Event", Event),
        "DistributedInfoBase" => ("DistributedInfoBase", DistributedInfoBase),
        "EnableTotalsSplitting" => ("EnableTotalsSplitting", EnableTotalsSplitting),
        "ExternalConnection" => ("ExternalConnection", ExternalConnection),
        "FoldersOnTop" => ("FoldersOnTop", FoldersOnTop),
        "Global" => ("Global", Global),
        "Handler" => ("Handler", Handler),
        "HierarchyType" => ("HierarchyType", HierarchyType),
        "LevelCount" => ("LevelCount", LevelCount),
        "LimitLevelCount" => ("LimitLevelCount", LimitLevelCount),
        "MainFilterOnPeriod" => ("MainFilterOnPeriod", MainFilterOnPeriod),
        "MaxExtDimensionCount" => ("MaxExtDimensionCount", MaxExtDimensionCount),
        "Namespace" => ("Namespace", Namespace),
        "NumberAllowedLength" => ("NumberAllowedLength", NumberAllowedLength),
        "NumberLength" => ("NumberLength", NumberLength),
        "NumberPeriodicity" => ("NumberPeriodicity", NumberPeriodicity),
        "NumberType" => ("NumberType", NumberType),
        "OrderLength" => ("OrderLength", OrderLength),
        "PeriodAdjustmentLength" => ("PeriodAdjustmentLength", PeriodAdjustmentLength),
        "Periodicity" | "InformationRegisterPeriodicity" => ("Periodicity", Periodicity),
        "PostInPrivilegedMode" => ("PostInPrivilegedMode", PostInPrivilegedMode),
        "Posting" => ("Posting", Posting),
        "Predefined" => ("Predefined", Predefined),
        "Privileged" => ("Privileged", Privileged),
        "QuickChoice" => ("QuickChoice", QuickChoice),
        "RealTimePosting" => ("RealTimePosting", RealTimePosting),
        "RegisterRecordsDeletion" => ("RegisterRecordsDeletion", RegisterRecordsDeletion),
        "RegisterRecordsWritingOnPost" => {
            ("RegisterRecordsWritingOnPost", RegisterRecordsWritingOnPost)
        }
        "RestartCountOnFailure" => ("RestartCountOnFailure", RestartCountOnFailure),
        "RestartIntervalOnFailure" => ("RestartIntervalOnFailure", RestartIntervalOnFailure),
        "ReturnValuesReuse" => ("ReturnValuesReuse", ReturnValuesReuse),
        "ReuseSessions" => ("ReuseSessions", ReuseSessions),
        "RootURL" => ("RootURL", RootUrl),
        "Server" => ("Server", Server),
        "ServerCall" => ("ServerCall", ServerCall),
        "SessionMaxAge" => ("SessionMaxAge", SessionMaxAge),
        "SubordinationUse" => ("SubordinationUse", SubordinationUse),
        "UnpostInPrivilegedMode" => ("UnpostInPrivilegedMode", UnpostInPrivilegedMode),
        "Use" => ("Use", Use),
        "WriteMode" => ("WriteMode", WriteMode),
        "CheckUnique" => ("CheckUnique", CheckUnique),
        "CodeLength" => ("CodeLength", CodeLength),
        "DescriptionLength" => ("DescriptionLength", DescriptionLength),
        "Hierarchical" => ("Hierarchical", Hierarchical),
        "Autonumbering" => ("Autonumbering", Autonumbering),
        "UseStandardCommands" => ("UseStandardCommands", UseStandardCommands),
        _ => return None,
    })
}

fn mutation_value_kind(key: MetaPropertyKey) -> MetaPropertyValueKind {
    use MetaPropertyKey::*;
    if matches!(
        key,
        ActionPeriod
            | ActionPeriodUse
            | AutoOrderByCode
            | BasePeriod
            | ClientManagedApplication
            | Correspondence
            | DistributedInfoBase
            | EnableTotalsSplitting
            | ExternalConnection
            | FoldersOnTop
            | Global
            | LimitLevelCount
            | MainFilterOnPeriod
            | PostInPrivilegedMode
            | Predefined
            | Privileged
            | QuickChoice
            | Server
            | ServerCall
            | UnpostInPrivilegedMode
            | Use
            | CheckUnique
            | Hierarchical
            | Autonumbering
            | UseStandardCommands
    ) {
        MetaPropertyValueKind::Boolean
    } else if matches!(
        key,
        LevelCount
            | MaxExtDimensionCount
            | NumberLength
            | OrderLength
            | PeriodAdjustmentLength
            | RestartCountOnFailure
            | RestartIntervalOnFailure
            | SessionMaxAge
            | CodeLength
            | DescriptionLength
    ) {
        MetaPropertyValueKind::UnsignedInteger
    } else {
        MetaPropertyValueKind::String
    }
}

fn mutation_property_is_observable(kind: MetadataKind, key: MetaPropertyKey) -> bool {
    use MetaPropertyKey::*;
    use MetadataKind::*;

    match key {
        Synonym | Comment => true,
        ActionPeriod | BasePeriod => kind == CalculationRegister,
        ActionPeriodUse | DependenceOnCalculationTypes => kind == ChartOfCalculationTypes,
        AutoOrderByCode | CodeMask | MaxExtDimensionCount | OrderLength => kind == ChartOfAccounts,
        ChoiceMode | CodeAllowedLength | CodeType | DefaultPresentation | FoldersOnTop
        | HierarchyType | LevelCount | LimitLevelCount | QuickChoice | SubordinationUse => {
            kind == Catalog
        }
        ClientManagedApplication
        | ExternalConnection
        | Global
        | Privileged
        | ReturnValuesReuse
        | Server
        | ServerCall => kind == CommonModule,
        Correspondence | PeriodAdjustmentLength => kind == AccountingRegister,
        Description | Predefined | RestartCountOnFailure | RestartIntervalOnFailure | Use => {
            kind == ScheduledJob
        }
        Event | Handler => kind == EventSubscription,
        DistributedInfoBase => kind == ExchangePlan,
        EnableTotalsSplitting => kind == AccumulationRegister,
        MainFilterOnPeriod | WriteMode => kind == InformationRegister,
        NumberAllowedLength
        | NumberPeriodicity
        | PostInPrivilegedMode
        | Posting
        | RealTimePosting
        | RegisterRecordsDeletion
        | RegisterRecordsWritingOnPost
        | UnpostInPrivilegedMode => kind == Document,
        Periodicity => matches!(kind, InformationRegister | CalculationRegister),
        ReuseSessions | SessionMaxAge => matches!(kind, HTTPService | WebService),
        RootUrl => kind == HTTPService,
        Namespace => kind == WebService,
        NumberLength | NumberType => matches!(kind, Document | BusinessProcess | Task),
        CheckUnique => matches!(
            kind,
            Catalog
                | Document
                | ChartOfAccounts
                | ChartOfCharacteristicTypes
                | BusinessProcess
                | Task
        ),
        CodeLength => matches!(
            kind,
            Catalog
                | ChartOfAccounts
                | ChartOfCharacteristicTypes
                | ChartOfCalculationTypes
                | ExchangePlan
        ),
        DescriptionLength => matches!(
            kind,
            Catalog
                | ChartOfAccounts
                | ChartOfCharacteristicTypes
                | ChartOfCalculationTypes
                | Task
                | ExchangePlan
        ),
        Hierarchical => matches!(kind, Catalog | ChartOfCharacteristicTypes),
        Autonumbering => matches!(
            kind,
            Catalog | Document | ChartOfCharacteristicTypes | BusinessProcess | Task
        ),
        UseStandardCommands => matches!(
            kind,
            Catalog
                | Document
                | Enum
                | Constant
                | InformationRegister
                | AccumulationRegister
                | AccountingRegister
                | CalculationRegister
                | ChartOfAccounts
                | ChartOfCharacteristicTypes
                | ChartOfCalculationTypes
                | BusinessProcess
                | Task
                | ExchangePlan
                | DocumentJournal
                | Report
                | DataProcessor
        ),
    }
}

fn reference_property_owner(kind: MetadataKind) -> bool {
    matches!(
        kind,
        MetadataKind::Catalog
            | MetadataKind::Document
            | MetadataKind::ChartOfAccounts
            | MetadataKind::ChartOfCharacteristicTypes
            | MetadataKind::ChartOfCalculationTypes
            | MetadataKind::BusinessProcess
            | MetadataKind::Task
            | MetadataKind::ExchangePlan
    )
}

fn additional_property_is_observable(kind: MetadataKind, name: &str) -> bool {
    use MetadataKind::*;

    if reference_property_owner(kind)
        && matches!(
            name,
            "SearchStringModeOnInputByString"
                | "FullTextSearchOnInputByString"
                | "ChoiceDataGetModeOnInputByString"
                | "DefaultObjectForm"
                | "DefaultListForm"
                | "DefaultChoiceForm"
                | "AuxiliaryObjectForm"
                | "AuxiliaryListForm"
                | "AuxiliaryChoiceForm"
                | "IncludeHelpInContents"
                | "DataLockControlMode"
                | "FullTextSearch"
                | "ObjectPresentation"
                | "ExtendedObjectPresentation"
                | "ListPresentation"
                | "ExtendedListPresentation"
                | "Explanation"
                | "CreateOnInput"
                | "ChoiceHistoryOnInput"
                | "DataHistory"
                | "UpdateDataHistoryImmediatelyAfterWrite"
                | "ExecuteAfterWriteDataHistoryVersionProcessing"
        )
    {
        return true;
    }

    match kind {
        Catalog => matches!(
            name,
            "CodeSeries"
                | "EditType"
                | "DefaultFolderForm"
                | "DefaultFolderChoiceForm"
                | "AuxiliaryFolderForm"
                | "AuxiliaryFolderChoiceForm"
                | "PredefinedDataUpdate"
        ),
        Document => matches!(name, "SequenceFilling" | "Numerator"),
        Enum => matches!(
            name,
            "QuickChoice"
                | "ChoiceMode"
                | "DefaultListForm"
                | "DefaultChoiceForm"
                | "AuxiliaryListForm"
                | "AuxiliaryChoiceForm"
                | "ListPresentation"
                | "ExtendedListPresentation"
                | "Explanation"
                | "ChoiceHistoryOnInput"
        ),
        Constant => matches!(
            name,
            "DefaultForm"
                | "ExtendedPresentation"
                | "Explanation"
                | "PasswordMode"
                | "Format"
                | "EditFormat"
                | "ToolTip"
                | "MarkNegatives"
                | "Mask"
                | "MultiLine"
                | "ExtendedEdit"
                | "MinValue"
                | "MaxValue"
                | "FillChecking"
                | "ChoiceFoldersAndItems"
                | "QuickChoice"
                | "ChoiceForm"
                | "LinkByType"
                | "ChoiceHistoryOnInput"
                | "DataLockControlMode"
                | "DataHistory"
                | "UpdateDataHistoryImmediatelyAfterWrite"
                | "ExecuteAfterWriteDataHistoryVersionProcessing"
        ),
        InformationRegister => matches!(
            name,
            "EditType"
                | "DefaultRecordForm"
                | "DefaultListForm"
                | "AuxiliaryRecordForm"
                | "AuxiliaryListForm"
                | "IncludeHelpInContents"
                | "DataLockControlMode"
                | "FullTextSearch"
                | "EnableTotalsSliceFirst"
                | "EnableTotalsSliceLast"
                | "RecordPresentation"
                | "ExtendedRecordPresentation"
                | "ListPresentation"
                | "ExtendedListPresentation"
                | "Explanation"
                | "DataHistory"
                | "UpdateDataHistoryImmediatelyAfterWrite"
                | "ExecuteAfterWriteDataHistoryVersionProcessing"
        ),
        AccumulationRegister => matches!(
            name,
            "DefaultListForm"
                | "AuxiliaryListForm"
                | "RegisterType"
                | "IncludeHelpInContents"
                | "DataLockControlMode"
                | "FullTextSearch"
                | "ListPresentation"
                | "ExtendedListPresentation"
                | "Explanation"
        ),
        AccountingRegister => matches!(
            name,
            "ChartOfAccounts"
                | "DefaultListForm"
                | "AuxiliaryListForm"
                | "IncludeHelpInContents"
                | "DataLockControlMode"
                | "EnableTotalsSplitting"
                | "FullTextSearch"
                | "ListPresentation"
                | "ExtendedListPresentation"
                | "Explanation"
        ),
        CalculationRegister => matches!(
            name,
            "DefaultListForm"
                | "AuxiliaryListForm"
                | "ChartOfCalculationTypes"
                | "IncludeHelpInContents"
                | "DataLockControlMode"
                | "FullTextSearch"
                | "ListPresentation"
                | "ExtendedListPresentation"
                | "Explanation"
        ),
        ChartOfAccounts => matches!(
            name,
            "CodeSeries"
                | "EditType"
                | "QuickChoice"
                | "ChoiceMode"
                | "DefaultPresentation"
                | "ExtDimensionTypes"
                | "PredefinedDataUpdate"
        ),
        ChartOfCharacteristicTypes => matches!(
            name,
            "FoldersOnTop"
                | "CodeAllowedLength"
                | "CodeSeries"
                | "DefaultPresentation"
                | "EditType"
                | "QuickChoice"
                | "ChoiceMode"
                | "DefaultFolderForm"
                | "DefaultFolderChoiceForm"
                | "AuxiliaryFolderForm"
                | "AuxiliaryFolderChoiceForm"
                | "CharacteristicExtValues"
                | "PredefinedDataUpdate"
        ),
        ChartOfCalculationTypes => matches!(
            name,
            "CodeType"
                | "CodeAllowedLength"
                | "DefaultPresentation"
                | "EditType"
                | "QuickChoice"
                | "ChoiceMode"
                | "PredefinedDataUpdate"
        ),
        BusinessProcess => matches!(
            name,
            "EditType"
                | "NumberAllowedLength"
                | "NumberPeriodicity"
                | "Task"
                | "CreateTaskInPrivilegedMode"
        ),
        Task => matches!(
            name,
            "NumberAllowedLength"
                | "TaskNumberAutoPrefix"
                | "DefaultPresentation"
                | "EditType"
                | "Addressing"
                | "MainAddressingAttribute"
                | "CurrentPerformer"
        ),
        ExchangePlan => matches!(
            name,
            "CodeAllowedLength"
                | "DefaultPresentation"
                | "EditType"
                | "QuickChoice"
                | "ChoiceMode"
                | "IncludeConfigurationExtensions"
        ),
        DocumentJournal => matches!(
            name,
            "DefaultForm"
                | "AuxiliaryForm"
                | "IncludeHelpInContents"
                | "ListPresentation"
                | "ExtendedListPresentation"
                | "Explanation"
        ),
        Report => matches!(
            name,
            "DefaultForm"
                | "AuxiliaryForm"
                | "MainDataCompositionSchema"
                | "DefaultSettingsForm"
                | "AuxiliarySettingsForm"
                | "DefaultVariantForm"
                | "AuxiliaryVariantForm"
                | "VariantsStorage"
                | "SettingsStorage"
                | "IncludeHelpInContents"
                | "ExtendedPresentation"
                | "Explanation"
        ),
        DataProcessor => matches!(
            name,
            "DefaultForm"
                | "AuxiliaryForm"
                | "IncludeHelpInContents"
                | "ExtendedPresentation"
                | "Explanation"
        ),
        CommonModule => matches!(name, "Client" | "ClientOrdinaryApplication"),
        ScheduledJob => name == "Key",
        HTTPService => false,
        WebService => name == "DescriptorFileName",
        ExternalDataSource => name == "DataLockControlMode",
        DefinedType | EventSubscription => false,
    }
}

fn additional_value_kind(kind: MetadataKind, name: &str) -> MetaInfoPropertyValueKind {
    if matches!(
        name,
        "Synonym"
            | "ObjectPresentation"
            | "ExtendedObjectPresentation"
            | "ListPresentation"
            | "ExtendedListPresentation"
            | "RecordPresentation"
            | "ExtendedRecordPresentation"
            | "ExtendedPresentation"
            | "Explanation"
            | "Format"
            | "EditFormat"
            | "ToolTip"
    ) {
        MetaInfoPropertyValueKind::LocalizedString
    } else if matches!(
        name,
        "IncludeHelpInContents"
            | "ActionPeriod"
            | "ActionPeriodUse"
            | "AutoOrderByCode"
            | "BasePeriod"
            | "ClientManagedApplication"
            | "Correspondence"
            | "DistributedInfoBase"
            | "ExternalConnection"
            | "FoldersOnTop"
            | "Global"
            | "LimitLevelCount"
            | "MainFilterOnPeriod"
            | "PostInPrivilegedMode"
            | "Predefined"
            | "Privileged"
            | "Server"
            | "ServerCall"
            | "UnpostInPrivilegedMode"
            | "Use"
            | "CheckUnique"
            | "Hierarchical"
            | "Autonumbering"
            | "UseStandardCommands"
            | "UpdateDataHistoryImmediatelyAfterWrite"
            | "ExecuteAfterWriteDataHistoryVersionProcessing"
            | "PasswordMode"
            | "MarkNegatives"
            | "MultiLine"
            | "ExtendedEdit"
            | "EnableTotalsSliceFirst"
            | "EnableTotalsSliceLast"
            | "CreateTaskInPrivilegedMode"
            | "IncludeConfigurationExtensions"
            | "ClientOrdinaryApplication"
            | "Client"
            | "EnableTotalsSplitting"
    ) || (name == "QuickChoice" && kind != MetadataKind::Constant)
    {
        MetaInfoPropertyValueKind::Boolean
    } else if matches!(
        name,
        "LevelCount"
            | "MaxExtDimensionCount"
            | "NumberLength"
            | "OrderLength"
            | "PeriodAdjustmentLength"
            | "RestartCountOnFailure"
            | "RestartIntervalOnFailure"
            | "SessionMaxAge"
            | "CodeLength"
            | "DescriptionLength"
    ) {
        MetaInfoPropertyValueKind::UnsignedInteger
    } else if matches!(name, "MinValue" | "MaxValue") {
        MetaInfoPropertyValueKind::TypedValue
    } else {
        MetaInfoPropertyValueKind::String
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::METADATA_PROPERTY_SPECS;

    #[test]
    fn read_profile_preserves_every_published_writer_property_without_using_it_at_runtime() {
        for writer in METADATA_PROPERTY_SPECS {
            for kind in MetadataKind::ALL {
                if !writer.allowed_kinds.contains(kind) {
                    continue;
                }
                let observed = META_INFO_PROPERTY_PROFILE
                    .resolve(*kind, writer.xml_name)
                    .unwrap_or_else(|| {
                        panic!(
                            "read profile lost writer property {} for {}",
                            writer.public_name,
                            kind.as_str()
                        )
                    });
                assert_eq!(observed.key, writer.public_name);
                let expected_kind = if writer.key == MetaPropertyKey::Synonym {
                    MetaInfoPropertyValueKind::LegacyLocalizedString
                } else {
                    writer.value_kind.into()
                };
                assert_eq!(
                    observed.value_kind,
                    expected_kind,
                    "{} for {}",
                    writer.public_name,
                    kind.as_str()
                );
            }
        }
    }

    #[test]
    fn read_profile_keeps_register_periodicity_xml_names_kind_specific() {
        let information = META_INFO_PROPERTY_PROFILE
            .resolve(
                MetadataKind::InformationRegister,
                "InformationRegisterPeriodicity",
            )
            .unwrap();
        assert_eq!(information.key, "Periodicity");
        assert!(META_INFO_PROPERTY_PROFILE
            .resolve(MetadataKind::InformationRegister, "Periodicity")
            .is_none());

        let calculation = META_INFO_PROPERTY_PROFILE
            .resolve(MetadataKind::CalculationRegister, "Periodicity")
            .unwrap();
        assert_eq!(calculation.key, "Periodicity");
        assert!(META_INFO_PROPERTY_PROFILE
            .resolve(
                MetadataKind::CalculationRegister,
                "InformationRegisterPeriodicity",
            )
            .is_none());
    }
}
