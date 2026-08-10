use super::{MetaDiagnostic, MetaDiagnosticCode};
use serde::Serialize;

/// A platform class that owns subscription events.
///
/// The enum is a domain identity. XML QNames are mapped to it only by the
/// Platform XML adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EventSourceClass {
    AccountingRegisterManager,
    AccountingRegisterRecordSet,
    AccumulationRegisterManager,
    AccumulationRegisterRecordSet,
    BusinessProcessManager,
    BusinessProcessObject,
    CalculationRegisterManager,
    CalculationRegisterRecordSet,
    CatalogManager,
    CatalogObject,
    ChartOfAccountsManager,
    ChartOfAccountsObject,
    ChartOfCalculationTypesManager,
    ChartOfCalculationTypesObject,
    ChartOfCharacteristicTypesManager,
    ChartOfCharacteristicTypesObject,
    ConstantManager,
    ConstantValueManager,
    DataProcessorManager,
    DataProcessorObject,
    DocumentJournalManager,
    DocumentManager,
    DocumentObject,
    EnumManager,
    ExchangePlanManager,
    ExchangePlanObject,
    ExternalDataSourceCubeDimensionTableManager,
    ExternalDataSourceCubeManager,
    ExternalDataSourceTableManager,
    ExternalDataSourceTableObject,
    ExternalDataSourceTableRecordSet,
    FilterCriterionManager,
    InformationRegisterManager,
    InformationRegisterRecordSet,
    RecalculationRecordSet,
    ReportManager,
    ReportObject,
    SequenceRecordSet,
    SettingsStorageManager,
    TaskManager,
    TaskObject,
}

impl EventSourceClass {
    pub(crate) const ALL: &'static [Self] = &[
        Self::AccountingRegisterManager,
        Self::AccountingRegisterRecordSet,
        Self::AccumulationRegisterManager,
        Self::AccumulationRegisterRecordSet,
        Self::BusinessProcessManager,
        Self::BusinessProcessObject,
        Self::CalculationRegisterManager,
        Self::CalculationRegisterRecordSet,
        Self::CatalogManager,
        Self::CatalogObject,
        Self::ChartOfAccountsManager,
        Self::ChartOfAccountsObject,
        Self::ChartOfCalculationTypesManager,
        Self::ChartOfCalculationTypesObject,
        Self::ChartOfCharacteristicTypesManager,
        Self::ChartOfCharacteristicTypesObject,
        Self::ConstantManager,
        Self::ConstantValueManager,
        Self::DataProcessorManager,
        Self::DataProcessorObject,
        Self::DocumentJournalManager,
        Self::DocumentManager,
        Self::DocumentObject,
        Self::EnumManager,
        Self::ExchangePlanManager,
        Self::ExchangePlanObject,
        Self::ExternalDataSourceCubeDimensionTableManager,
        Self::ExternalDataSourceCubeManager,
        Self::ExternalDataSourceTableManager,
        Self::ExternalDataSourceTableObject,
        Self::ExternalDataSourceTableRecordSet,
        Self::FilterCriterionManager,
        Self::InformationRegisterManager,
        Self::InformationRegisterRecordSet,
        Self::RecalculationRecordSet,
        Self::ReportManager,
        Self::ReportObject,
        Self::SequenceRecordSet,
        Self::SettingsStorageManager,
        Self::TaskManager,
        Self::TaskObject,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AccountingRegisterManager => "accountingRegisterManager",
            Self::AccountingRegisterRecordSet => "accountingRegisterRecordSet",
            Self::AccumulationRegisterManager => "accumulationRegisterManager",
            Self::AccumulationRegisterRecordSet => "accumulationRegisterRecordSet",
            Self::BusinessProcessManager => "businessProcessManager",
            Self::BusinessProcessObject => "businessProcessObject",
            Self::CalculationRegisterManager => "calculationRegisterManager",
            Self::CalculationRegisterRecordSet => "calculationRegisterRecordSet",
            Self::CatalogManager => "catalogManager",
            Self::CatalogObject => "catalogObject",
            Self::ChartOfAccountsManager => "chartOfAccountsManager",
            Self::ChartOfAccountsObject => "chartOfAccountsObject",
            Self::ChartOfCalculationTypesManager => "chartOfCalculationTypesManager",
            Self::ChartOfCalculationTypesObject => "chartOfCalculationTypesObject",
            Self::ChartOfCharacteristicTypesManager => "chartOfCharacteristicTypesManager",
            Self::ChartOfCharacteristicTypesObject => "chartOfCharacteristicTypesObject",
            Self::ConstantManager => "constantManager",
            Self::ConstantValueManager => "constantValueManager",
            Self::DataProcessorManager => "dataProcessorManager",
            Self::DataProcessorObject => "dataProcessorObject",
            Self::DocumentJournalManager => "documentJournalManager",
            Self::DocumentManager => "documentManager",
            Self::DocumentObject => "documentObject",
            Self::EnumManager => "enumManager",
            Self::ExchangePlanManager => "exchangePlanManager",
            Self::ExchangePlanObject => "exchangePlanObject",
            Self::ExternalDataSourceCubeDimensionTableManager => {
                "externalDataSourceCubeDimensionTableManager"
            }
            Self::ExternalDataSourceCubeManager => "externalDataSourceCubeManager",
            Self::ExternalDataSourceTableManager => "externalDataSourceTableManager",
            Self::ExternalDataSourceTableObject => "externalDataSourceTableObject",
            Self::ExternalDataSourceTableRecordSet => "externalDataSourceTableRecordSet",
            Self::FilterCriterionManager => "filterCriterionManager",
            Self::InformationRegisterManager => "informationRegisterManager",
            Self::InformationRegisterRecordSet => "informationRegisterRecordSet",
            Self::RecalculationRecordSet => "recalculationRecordSet",
            Self::ReportManager => "reportManager",
            Self::ReportObject => "reportObject",
            Self::SequenceRecordSet => "sequenceRecordSet",
            Self::SettingsStorageManager => "settingsStorageManager",
            Self::TaskManager => "taskManager",
            Self::TaskObject => "taskObject",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| {
                MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    format!("unsupported event source class `{value}`"),
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EventDefinition {
    pub(crate) name: &'static str,
    pub(crate) parameters: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventHandlerMethodKind {
    Procedure,
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EventHandlerFacts {
    pub(crate) module_global: bool,
    pub(crate) module_server: bool,
    pub(crate) method_kind: EventHandlerMethodKind,
    pub(crate) exported: bool,
    pub(crate) parameter_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventBindingError {
    EmptySource,
    EventUnavailable {
        source_class: EventSourceClass,
        event: String,
    },
    SignatureConflict {
        event: String,
        expected_class: EventSourceClass,
        expected_parameters: &'static [&'static str],
        actual_class: EventSourceClass,
        actual_parameters: &'static [&'static str],
    },
    HandlerModuleIsGlobal,
    HandlerModuleIsNotServer,
    HandlerIsNotProcedure,
    HandlerIsNotExported,
    HandlerArity {
        event: String,
        event_parameter_count: usize,
        expected: usize,
        actual: usize,
    },
}

#[derive(Debug, Clone, Copy)]
struct EventSourceProfile {
    source_class: EventSourceClass,
    events: &'static [EventDefinition],
}

macro_rules! event {
    ($constant:ident, $name:literal $(, $parameter:literal)*) => {
        const $constant: EventDefinition = EventDefinition {
            name: $name,
            parameters: &[$($parameter),*],
        };
    };
}

event!(
    AFTER_HISTORY_WRITE,
    "AfterWriteDataHistoryVersionsProcessing",
    "WriteVersionsInformation"
);
event!(
    BEFORE_BEGIN_SEND_MASTER,
    "BeforeBeginSendDataToMaster",
    "DataExchangeStream"
);
event!(
    BEFORE_BEGIN_SEND_SLAVE,
    "BeforeBeginSendDataToSlave",
    "DataExchangeStream"
);
event!(
    BEFORE_CREATE_INITIAL_IMAGE,
    "BeforeCreateInitialImage",
    "OnlyRecorded",
    "DataExchangeStream"
);
event!(BEFORE_DELETE, "BeforeDelete", "Cancel");
event!(BEFORE_EXECUTE, "BeforeExecute", "Cancel");
event!(
    BEFORE_EXECUTE_INTERACTIVELY,
    "BeforeExecuteInteractively",
    "Cancel"
);
event!(BEFORE_WRITE, "BeforeWrite", "Cancel");
event!(BEFORE_WRITE_REPLACING, "BeforeWrite", "Cancel", "Replacing");
event!(
    BEFORE_WRITE_CALCULATION,
    "BeforeWrite",
    "Cancel",
    "Replacing",
    "WriteOnly",
    "WriteActualActionPeriod",
    "WriteRecalculations"
);
event!(
    BEFORE_WRITE_DOCUMENT,
    "BeforeWrite",
    "Cancel",
    "WriteMode",
    "PostingMode"
);
event!(
    CHOICE_DATA_GET,
    "ChoiceDataGetProcessing",
    "ChoiceData",
    "Parameters",
    "StandardProcessing"
);
event!(
    FILL_CHECK,
    "FillCheckProcessing",
    "Cancel",
    "CheckedAttributes"
);
event!(
    FILLING_OBJECT,
    "Filling",
    "FillingData",
    "FillingText",
    "StandardProcessing"
);
event!(
    FILLING_INFORMATION_REGISTER,
    "Filling",
    "FillingData",
    "StandardProcessing"
);
event!(
    FILLING_EXTERNAL_DATA_SOURCE,
    "Filling",
    "StandardProcessing",
    "FillingData"
);
event!(
    FORM_GET,
    "FormGetProcessing",
    "FormType",
    "Parameters",
    "SelectedForm",
    "AdditionalInformation",
    "StandardProcessing"
);
event!(
    GENERATE_HISTORY_EMPTY,
    "GenerateFromDataHistoryVersionProcessing"
);
event!(
    GENERATE_HISTORY_OBJECT,
    "GenerateFromDataHistoryVersionProcessing",
    "VersionData",
    "VersionNumber",
    "ExcludedData",
    "StandardProcessing"
);
event!(
    GENERATE_HISTORY_INFORMATION_REGISTER,
    "GenerateFromDataHistoryVersionProcessing",
    "VersionData",
    "VersionNumber",
    "RecordKey",
    "ExcludedData",
    "StandardProcessing"
);
event!(
    GENERATE_HISTORY_TASK,
    "GenerateFromDataHistoryVersionProcessing",
    "VersionData",
    "VersionNumber",
    "StandardProcessing",
    "ExcludedData"
);
event!(
    GET_DESCRIPTION,
    "GetDescriptionProcessing",
    "ObjectKey",
    "SettingsKey",
    "SettingsDescription",
    "User"
);
event!(
    INTERACTIVE_ACTIVATION,
    "InteractiveActivationProcessing",
    "StandardProcessing"
);
event!(
    LOAD_SETTINGS,
    "LoadProcessing",
    "ObjectKey",
    "SettingsKey",
    "Settings",
    "SettingsDescription",
    "User"
);
event!(ON_AUTO_CREATE_NODE, "OnAutoCreateNewNode", "Cancel");
event!(ON_CHECK_EXECUTION, "OnCheckExecutionProcessing", "Result");
event!(
    ON_COMPOSE_RESULT,
    "OnComposeResult",
    "ResultDocument",
    "DetailsData",
    "StandardProcessing"
);
event!(ON_COPY, "OnCopy", "CopiedObject");
event!(ON_EXECUTE, "OnExecute", "Cancel");
event!(
    ON_RECEIVE_MASTER,
    "OnReceiveDataFromMaster",
    "DataItem",
    "ItemReceive",
    "SendBack"
);
event!(
    ON_RECEIVE_SLAVE,
    "OnReceiveDataFromSlave",
    "DataItem",
    "ItemReceive",
    "SendBack"
);
event!(
    ON_RECEIVE_NODE_MASTER,
    "OnReceiveNodeDataFromMaster",
    "DataItem",
    "Ignore"
);
event!(ON_SEND_MASTER, "OnSendDataToMaster", "DataItem", "ItemSend");
event!(
    ON_SEND_SLAVE,
    "OnSendDataToSlave",
    "DataItem",
    "ItemSend",
    "InitialImageCreating"
);
event!(
    ON_SEND_NODE_SLAVE,
    "OnSendNodeDataToSlave",
    "DataItem",
    "Ignore"
);
event!(
    ON_SET_NEW_CODE,
    "OnSetNewCode",
    "StandardProcessing",
    "Prefix"
);
event!(
    ON_SET_NEW_NUMBER,
    "OnSetNewNumber",
    "StandardProcessing",
    "Prefix"
);
event!(ON_WRITE, "OnWrite", "Cancel");
event!(ON_WRITE_REPLACING, "OnWrite", "Cancel", "Replacing");
event!(
    ON_WRITE_CALCULATION,
    "OnWrite",
    "Cancel",
    "Replacing",
    "WriteOnly",
    "WriteActualActionPeriod",
    "WriteRecalculations"
);
event!(POSTING, "Posting", "Cancel", "PostingMode");
event!(
    PRESENTATION_FIELDS_GET,
    "PresentationFieldsGetProcessing",
    "Fields",
    "StandardProcessing"
);
event!(
    PRESENTATION_GET,
    "PresentationGetProcessing",
    "Data",
    "Presentation",
    "StandardProcessing"
);
event!(
    SAVE_SETTINGS,
    "SaveProcessing",
    "ObjectKey",
    "SettingsKey",
    "Settings",
    "SettingsDescription",
    "User"
);
event!(
    SET_DESCRIPTION,
    "SetDescriptionProcessing",
    "ObjectKey",
    "SettingsKey",
    "SettingsDescription",
    "User"
);
event!(UNDO_POSTING, "UndoPosting", "Cancel");

const HISTORY_MANAGER_EVENTS: &[EventDefinition] = &[
    AFTER_HISTORY_WRITE,
    CHOICE_DATA_GET,
    FORM_GET,
    PRESENTATION_FIELDS_GET,
    PRESENTATION_GET,
];
const CHOICE_MANAGER_EVENTS: &[EventDefinition] = &[
    CHOICE_DATA_GET,
    FORM_GET,
    PRESENTATION_FIELDS_GET,
    PRESENTATION_GET,
];
const FORM_MANAGER_EVENTS: &[EventDefinition] = &[FORM_GET];
const HISTORY_FORM_MANAGER_EVENTS: &[EventDefinition] = &[AFTER_HISTORY_WRITE, FORM_GET];
const ENUM_MANAGER_EVENTS: &[EventDefinition] = &[CHOICE_DATA_GET, FORM_GET];
const BUSINESS_PROCESS_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_DELETE,
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_OBJECT,
    GENERATE_HISTORY_OBJECT,
    INTERACTIVE_ACTIVATION,
    ON_COPY,
    ON_SET_NEW_NUMBER,
    ON_WRITE,
];
const EXTERNAL_TABLE_RECORD_SET_EVENTS: &[EventDefinition] = &[
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_EXTERNAL_DATA_SOURCE,
    ON_WRITE,
];
const EXTERNAL_TABLE_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_DELETE,
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_EXTERNAL_DATA_SOURCE,
    ON_COPY,
    ON_WRITE,
];
const DOCUMENT_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_DELETE,
    BEFORE_WRITE_DOCUMENT,
    FILL_CHECK,
    FILLING_OBJECT,
    GENERATE_HISTORY_OBJECT,
    ON_COPY,
    ON_SET_NEW_NUMBER,
    ON_WRITE,
    POSTING,
    UNDO_POSTING,
];
const TASK_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_DELETE,
    BEFORE_EXECUTE,
    BEFORE_EXECUTE_INTERACTIVELY,
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_OBJECT,
    GENERATE_HISTORY_TASK,
    INTERACTIVE_ACTIVATION,
    ON_CHECK_EXECUTION,
    ON_COPY,
    ON_EXECUTE,
    ON_SET_NEW_NUMBER,
    ON_WRITE,
];
const CONSTANT_VALUE_EVENTS: &[EventDefinition] =
    &[BEFORE_WRITE, FILL_CHECK, GENERATE_HISTORY_EMPTY, ON_WRITE];
const DATA_PROCESSOR_OBJECT_EVENTS: &[EventDefinition] = &[FILL_CHECK];
const REPORT_OBJECT_EVENTS: &[EventDefinition] = &[FILL_CHECK, ON_COMPOSE_RESULT];
const STANDARD_RECORD_SET_EVENTS: &[EventDefinition] =
    &[BEFORE_WRITE_REPLACING, FILL_CHECK, ON_WRITE_REPLACING];
const CALCULATION_TYPE_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_DELETE,
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_OBJECT,
    GENERATE_HISTORY_EMPTY,
    ON_COPY,
    ON_WRITE,
];
const CATALOG_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_DELETE,
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_OBJECT,
    GENERATE_HISTORY_OBJECT,
    ON_COPY,
    ON_SET_NEW_CODE,
    ON_WRITE,
];
const EXCHANGE_PLAN_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_BEGIN_SEND_MASTER,
    BEFORE_BEGIN_SEND_SLAVE,
    BEFORE_CREATE_INITIAL_IMAGE,
    BEFORE_DELETE,
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_OBJECT,
    GENERATE_HISTORY_EMPTY,
    ON_AUTO_CREATE_NODE,
    ON_COPY,
    ON_RECEIVE_MASTER,
    ON_RECEIVE_SLAVE,
    ON_RECEIVE_NODE_MASTER,
    ON_SEND_MASTER,
    ON_SEND_SLAVE,
    ON_SEND_NODE_SLAVE,
    ON_SET_NEW_CODE,
    ON_WRITE,
];
const ACCOUNT_OBJECT_EVENTS: &[EventDefinition] = &[
    BEFORE_DELETE,
    BEFORE_WRITE,
    FILL_CHECK,
    FILLING_OBJECT,
    GENERATE_HISTORY_OBJECT,
    ON_COPY,
    ON_WRITE,
];
const CALCULATION_RECORD_SET_EVENTS: &[EventDefinition] =
    &[BEFORE_WRITE_CALCULATION, FILL_CHECK, ON_WRITE_CALCULATION];
const INFORMATION_RECORD_SET_EVENTS: &[EventDefinition] = &[
    BEFORE_WRITE_REPLACING,
    FILL_CHECK,
    FILLING_INFORMATION_REGISTER,
    GENERATE_HISTORY_INFORMATION_REGISTER,
    ON_WRITE_REPLACING,
];
const SETTINGS_STORAGE_EVENTS: &[EventDefinition] = &[
    GET_DESCRIPTION,
    LOAD_SETTINGS,
    SAVE_SETTINGS,
    SET_DESCRIPTION,
];

macro_rules! profile {
    ($source_class:ident, $events:ident) => {
        EventSourceProfile {
            source_class: EventSourceClass::$source_class,
            events: $events,
        }
    };
}

const EVENT_SOURCE_PROFILES: &[EventSourceProfile] = &[
    profile!(AccountingRegisterManager, FORM_MANAGER_EVENTS),
    profile!(AccountingRegisterRecordSet, STANDARD_RECORD_SET_EVENTS),
    profile!(AccumulationRegisterManager, FORM_MANAGER_EVENTS),
    profile!(AccumulationRegisterRecordSet, STANDARD_RECORD_SET_EVENTS),
    profile!(BusinessProcessManager, HISTORY_MANAGER_EVENTS),
    profile!(BusinessProcessObject, BUSINESS_PROCESS_OBJECT_EVENTS),
    profile!(CalculationRegisterManager, FORM_MANAGER_EVENTS),
    profile!(CalculationRegisterRecordSet, CALCULATION_RECORD_SET_EVENTS),
    profile!(CatalogManager, HISTORY_MANAGER_EVENTS),
    profile!(CatalogObject, CATALOG_OBJECT_EVENTS),
    profile!(ChartOfAccountsManager, HISTORY_MANAGER_EVENTS),
    profile!(ChartOfAccountsObject, ACCOUNT_OBJECT_EVENTS),
    profile!(ChartOfCalculationTypesManager, HISTORY_MANAGER_EVENTS),
    profile!(
        ChartOfCalculationTypesObject,
        CALCULATION_TYPE_OBJECT_EVENTS
    ),
    profile!(ChartOfCharacteristicTypesManager, HISTORY_MANAGER_EVENTS),
    profile!(ChartOfCharacteristicTypesObject, CATALOG_OBJECT_EVENTS),
    profile!(ConstantManager, HISTORY_FORM_MANAGER_EVENTS),
    profile!(ConstantValueManager, CONSTANT_VALUE_EVENTS),
    profile!(DataProcessorManager, FORM_MANAGER_EVENTS),
    profile!(DataProcessorObject, DATA_PROCESSOR_OBJECT_EVENTS),
    profile!(DocumentJournalManager, FORM_MANAGER_EVENTS),
    profile!(DocumentManager, HISTORY_MANAGER_EVENTS),
    profile!(DocumentObject, DOCUMENT_OBJECT_EVENTS),
    profile!(EnumManager, ENUM_MANAGER_EVENTS),
    profile!(ExchangePlanManager, HISTORY_MANAGER_EVENTS),
    profile!(ExchangePlanObject, EXCHANGE_PLAN_OBJECT_EVENTS),
    profile!(
        ExternalDataSourceCubeDimensionTableManager,
        CHOICE_MANAGER_EVENTS
    ),
    profile!(ExternalDataSourceCubeManager, FORM_MANAGER_EVENTS),
    profile!(ExternalDataSourceTableManager, CHOICE_MANAGER_EVENTS),
    profile!(ExternalDataSourceTableObject, EXTERNAL_TABLE_OBJECT_EVENTS),
    profile!(
        ExternalDataSourceTableRecordSet,
        EXTERNAL_TABLE_RECORD_SET_EVENTS
    ),
    profile!(FilterCriterionManager, FORM_MANAGER_EVENTS),
    profile!(InformationRegisterManager, HISTORY_FORM_MANAGER_EVENTS),
    profile!(InformationRegisterRecordSet, INFORMATION_RECORD_SET_EVENTS),
    profile!(RecalculationRecordSet, STANDARD_RECORD_SET_EVENTS),
    profile!(ReportManager, FORM_MANAGER_EVENTS),
    profile!(ReportObject, REPORT_OBJECT_EVENTS),
    profile!(SequenceRecordSet, STANDARD_RECORD_SET_EVENTS),
    profile!(SettingsStorageManager, SETTINGS_STORAGE_EVENTS),
    profile!(TaskManager, HISTORY_MANAGER_EVENTS),
    profile!(TaskObject, TASK_OBJECT_EVENTS),
];

#[cfg(test)]
pub(crate) fn event_binding_row_count() -> usize {
    EVENT_SOURCE_PROFILES
        .iter()
        .map(|profile| profile.events.len())
        .sum()
}

pub(crate) fn event_definition(
    source_class: EventSourceClass,
    event: &str,
) -> Option<&'static EventDefinition> {
    EVENT_SOURCE_PROFILES
        .iter()
        .find(|profile| profile.source_class == source_class)?
        .events
        .iter()
        .find(|definition| definition.name == event)
}

#[cfg(test)]
pub(crate) fn shared_event_names(source_classes: &[EventSourceClass]) -> Vec<&'static str> {
    let Some(first) = source_classes.first().and_then(|source_class| {
        EVENT_SOURCE_PROFILES
            .iter()
            .find(|profile| profile.source_class == *source_class)
    }) else {
        return Vec::new();
    };
    first
        .events
        .iter()
        .filter(|event| {
            source_classes[1..]
                .iter()
                .all(|source_class| event_definition(*source_class, event.name).is_some())
        })
        .map(|event| event.name)
        .collect()
}

pub(crate) fn validate_event_subscription_binding(
    source_classes: &[EventSourceClass],
    event: &str,
    handler: EventHandlerFacts,
) -> Result<&'static EventDefinition, EventBindingError> {
    let Some(first_class) = source_classes.first().copied() else {
        return Err(EventBindingError::EmptySource);
    };
    let expected = event_definition(first_class, event).ok_or_else(|| {
        EventBindingError::EventUnavailable {
            source_class: first_class,
            event: event.to_string(),
        }
    })?;
    for source_class in source_classes.iter().copied().skip(1) {
        let actual = event_definition(source_class, event).ok_or_else(|| {
            EventBindingError::EventUnavailable {
                source_class,
                event: event.to_string(),
            }
        })?;
        if actual.parameters != expected.parameters {
            return Err(EventBindingError::SignatureConflict {
                event: event.to_string(),
                expected_class: first_class,
                expected_parameters: expected.parameters,
                actual_class: source_class,
                actual_parameters: actual.parameters,
            });
        }
    }
    if handler.module_global {
        return Err(EventBindingError::HandlerModuleIsGlobal);
    }
    if !handler.module_server {
        return Err(EventBindingError::HandlerModuleIsNotServer);
    }
    if handler.method_kind != EventHandlerMethodKind::Procedure {
        return Err(EventBindingError::HandlerIsNotProcedure);
    }
    if !handler.exported {
        return Err(EventBindingError::HandlerIsNotExported);
    }
    let expected_arity = expected.parameters.len() + 1;
    if handler.parameter_count != expected_arity {
        return Err(EventBindingError::HandlerArity {
            event: event.to_string(),
            event_parameter_count: expected.parameters.len(),
            expected: expected_arity,
            actual: handler.parameter_count,
        });
    }
    Ok(expected)
}

#[cfg(test)]
mod event_subscription_binding_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn event_subscription_binding_profile_is_the_exact_8_3_27_matrix() {
        let class_names = EventSourceClass::ALL
            .iter()
            .copied()
            .map(EventSourceClass::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            class_names,
            [
                "accountingRegisterManager",
                "accountingRegisterRecordSet",
                "accumulationRegisterManager",
                "accumulationRegisterRecordSet",
                "businessProcessManager",
                "businessProcessObject",
                "calculationRegisterManager",
                "calculationRegisterRecordSet",
                "catalogManager",
                "catalogObject",
                "chartOfAccountsManager",
                "chartOfAccountsObject",
                "chartOfCalculationTypesManager",
                "chartOfCalculationTypesObject",
                "chartOfCharacteristicTypesManager",
                "chartOfCharacteristicTypesObject",
                "constantManager",
                "constantValueManager",
                "dataProcessorManager",
                "dataProcessorObject",
                "documentJournalManager",
                "documentManager",
                "documentObject",
                "enumManager",
                "exchangePlanManager",
                "exchangePlanObject",
                "externalDataSourceCubeDimensionTableManager",
                "externalDataSourceCubeManager",
                "externalDataSourceTableManager",
                "externalDataSourceTableObject",
                "externalDataSourceTableRecordSet",
                "filterCriterionManager",
                "informationRegisterManager",
                "informationRegisterRecordSet",
                "recalculationRecordSet",
                "reportManager",
                "reportObject",
                "sequenceRecordSet",
                "settingsStorageManager",
                "taskManager",
                "taskObject",
            ]
        );
        assert_eq!(
            class_names.iter().copied().collect::<HashSet<_>>().len(),
            41
        );
        assert_eq!(event_binding_row_count(), 183);
        assert_eq!(profile_evidence_digest(), 0x2ddc_527c_aafd_e0bf);

        assert_event(EventSourceClass::CatalogObject, "BeforeWrite", &["Cancel"]);
        assert_event(
            EventSourceClass::CatalogManager,
            "FormGetProcessing",
            &[
                "FormType",
                "Parameters",
                "SelectedForm",
                "AdditionalInformation",
                "StandardProcessing",
            ],
        );
        assert_event(
            EventSourceClass::InformationRegisterRecordSet,
            "GenerateFromDataHistoryVersionProcessing",
            &[
                "VersionData",
                "VersionNumber",
                "RecordKey",
                "ExcludedData",
                "StandardProcessing",
            ],
        );
        assert_event(
            EventSourceClass::SequenceRecordSet,
            "OnWrite",
            &["Cancel", "Replacing"],
        );
        assert_event(
            EventSourceClass::RecalculationRecordSet,
            "BeforeWrite",
            &["Cancel", "Replacing"],
        );
        assert_event(
            EventSourceClass::CalculationRegisterRecordSet,
            "BeforeWrite",
            &[
                "Cancel",
                "Replacing",
                "WriteOnly",
                "WriteActualActionPeriod",
                "WriteRecalculations",
            ],
        );
        assert_event(
            EventSourceClass::SettingsStorageManager,
            "LoadProcessing",
            &[
                "ObjectKey",
                "SettingsKey",
                "Settings",
                "SettingsDescription",
                "User",
            ],
        );
        assert_event(
            EventSourceClass::FilterCriterionManager,
            "FormGetProcessing",
            &[
                "FormType",
                "Parameters",
                "SelectedForm",
                "AdditionalInformation",
                "StandardProcessing",
            ],
        );
        assert_event(
            EventSourceClass::ExternalDataSourceTableRecordSet,
            "Filling",
            &["StandardProcessing", "FillingData"],
        );
    }

    #[test]
    fn event_subscription_binding_requires_exact_case_presence_and_one_signature() {
        assert!(event_definition(EventSourceClass::CatalogObject, "beforewrite").is_none());
        assert!(matches!(
            validate_event_subscription_binding(
                &[EventSourceClass::CatalogObject],
                "beforewrite",
                valid_handler(2),
            ),
            Err(EventBindingError::EventUnavailable { .. })
        ));

        let shared = shared_event_names(&[
            EventSourceClass::CatalogObject,
            EventSourceClass::ChartOfAccountsObject,
        ]);
        assert!(shared.contains(&"BeforeWrite"));
        assert!(!shared.contains(&"OnSetNewCode"));
        assert!(validate_event_subscription_binding(
            &[
                EventSourceClass::CatalogObject,
                EventSourceClass::ChartOfAccountsObject,
            ],
            "BeforeWrite",
            valid_handler(2),
        )
        .is_ok());

        assert!(matches!(
            validate_event_subscription_binding(
                &[
                    EventSourceClass::CatalogObject,
                    EventSourceClass::InformationRegisterRecordSet,
                ],
                "BeforeWrite",
                valid_handler(2),
            ),
            Err(EventBindingError::SignatureConflict { .. })
        ));
        assert!(matches!(
            validate_event_subscription_binding(
                &[
                    EventSourceClass::ExternalDataSourceTableRecordSet,
                    EventSourceClass::InformationRegisterRecordSet,
                ],
                "Filling",
                valid_handler(3),
            ),
            Err(EventBindingError::SignatureConflict { .. })
        ));
    }

    #[test]
    fn event_subscription_binding_validates_the_complete_handler_contract() {
        let classes = [EventSourceClass::CatalogObject];
        assert!(
            validate_event_subscription_binding(&classes, "BeforeWrite", valid_handler(2),).is_ok()
        );
        assert!(matches!(
            validate_event_subscription_binding(
                &classes,
                "BeforeWrite",
                EventHandlerFacts {
                    parameter_count: 1,
                    ..valid_handler(2)
                },
            ),
            Err(EventBindingError::HandlerArity {
                expected: 2,
                actual: 1,
                ..
            })
        ));
        assert!(matches!(
            validate_event_subscription_binding(
                &classes,
                "BeforeWrite",
                EventHandlerFacts {
                    method_kind: EventHandlerMethodKind::Function,
                    ..valid_handler(2)
                },
            ),
            Err(EventBindingError::HandlerIsNotProcedure)
        ));
        assert!(matches!(
            validate_event_subscription_binding(
                &classes,
                "BeforeWrite",
                EventHandlerFacts {
                    exported: false,
                    ..valid_handler(2)
                },
            ),
            Err(EventBindingError::HandlerIsNotExported)
        ));
        assert!(matches!(
            validate_event_subscription_binding(
                &classes,
                "BeforeWrite",
                EventHandlerFacts {
                    module_global: true,
                    ..valid_handler(2)
                },
            ),
            Err(EventBindingError::HandlerModuleIsGlobal)
        ));
        assert!(matches!(
            validate_event_subscription_binding(
                &classes,
                "BeforeWrite",
                EventHandlerFacts {
                    module_server: false,
                    ..valid_handler(2)
                },
            ),
            Err(EventBindingError::HandlerModuleIsNotServer)
        ));
        assert!(matches!(
            validate_event_subscription_binding(&[], "BeforeWrite", valid_handler(2)),
            Err(EventBindingError::EmptySource)
        ));
    }

    fn assert_event(source_class: EventSourceClass, name: &str, expected_parameters: &[&str]) {
        let event = event_definition(source_class, name).expect("profile event");
        assert_eq!(
            event.parameters, expected_parameters,
            "{source_class:?}.{name}"
        );
    }

    fn valid_handler(parameter_count: usize) -> EventHandlerFacts {
        EventHandlerFacts {
            module_global: false,
            module_server: true,
            method_kind: EventHandlerMethodKind::Procedure,
            exported: true,
            parameter_count,
        }
    }

    fn profile_evidence_digest() -> u64 {
        let mut rows = EVENT_SOURCE_PROFILES
            .iter()
            .flat_map(|profile| {
                profile.events.iter().map(|event| {
                    format!(
                        "{}|{}|{}\n",
                        profile.source_class.as_str(),
                        event.name,
                        event.parameters.join(",")
                    )
                })
            })
            .collect::<Vec<_>>();
        rows.sort_unstable();
        rows.concat()
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }
}
