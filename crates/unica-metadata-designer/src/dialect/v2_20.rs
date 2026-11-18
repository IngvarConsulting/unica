//! The metadata dialect of Designer export format 2.20 (platform 8.3.27),
//! the tables the platform corpus of that version proves.
use unica_metadata::MetadataKind;

/// Whether a kind declares a root `ChildObjects` container. The platform
/// corpus proves a total split: a supported kind either always carries this
/// container or rejects it even when the container is empty.
pub(crate) fn kind_declares_child_objects(kind: MetadataKind) -> bool {
    unica_source_designer::layout::metadata_kind_requires_child_objects_8_3_27(kind.as_str())
        .expect("every typed metadata kind has a physical ChildObjects profile")
}

/// The properties of an object type whose value is a canonical `xs:boolean`.
pub(crate) fn boolean_properties(object_type: &str) -> &'static [&'static str] {
    match object_type {
        "AccountingFlag" | "AddressingAttribute" | "Attribute" | "ExtDimensionAccountingFlag" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "FillFromFillingValue",
        ],
        "AccountingRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "Correspondence",
            "EnableTotalsSplitting",
        ],
        "AccumulationRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "EnableTotalsSplitting",
        ],
        "BusinessProcess" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "CreateTaskInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "CalculationRegister" => &[
            "UseStandardCommands",
            "ActionPeriod",
            "BasePeriod",
            "IncludeHelpInContents",
        ],
        "Catalog" => &[
            "Hierarchical",
            "LimitLevelCount",
            "FoldersOnTop",
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "QuickChoice",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfAccounts" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "CheckUnique",
            "QuickChoice",
            "AutoOrderByCode",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCalculationTypes" => &[
            "UseStandardCommands",
            "QuickChoice",
            "ActionPeriodUse",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCharacteristicTypes" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "Hierarchical",
            "FoldersOnTop",
            "CheckUnique",
            "Autonumbering",
            "QuickChoice",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Command" => &["ModifiesData"],
        "CommonModule" => &[
            "Global",
            "ClientManagedApplication",
            "Server",
            "ExternalConnection",
            "ClientOrdinaryApplication",
            "Client",
            "ServerCall",
            "Privileged",
        ],
        "Constant" => &[
            "UseStandardCommands",
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "DataProcessor" | "DocumentJournal" | "Report" => {
            &["UseStandardCommands", "IncludeHelpInContents"]
        }
        "Dimension" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "DenyIncompleteValues",
            "BaseDimension",
            "UseInTotals",
            "FillFromFillingValue",
            "Master",
            "MainFilter",
            "Balance",
        ],
        "Document" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "PostInPrivilegedMode",
            "UnpostInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Enum" => &["UseStandardCommands", "QuickChoice"],
        "ExchangePlan" => &[
            "UseStandardCommands",
            "QuickChoice",
            "DistributedInfoBase",
            "IncludeConfigurationExtensions",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "InformationRegister" => &[
            "UseStandardCommands",
            "MainFilterOnPeriod",
            "IncludeHelpInContents",
            "EnableTotalsSliceFirst",
            "EnableTotalsSliceLast",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Operation" => &["Nillable", "Transactioned"],
        "Parameter" => &["Nillable"],
        "Resource" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "Balance",
            "FillFromFillingValue",
        ],
        "ScheduledJob" => &["Use", "Predefined"],
        "Task" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        _ => &[],
    }
}
