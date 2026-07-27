use crate::domain::navigation::{
    NodeKind, RelationRole, SemanticPropertyId, SemanticRelationId,
};

use super::schema::{MetadataClassProfile, MetadataClassRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeValueKind {
    Boolean,
    Integer,
    Uuid,
    String,
    LocalizedString,
    Enum,
    TypeSet,
    Polymorphic,
    StringList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PropertyMapping {
    pub(crate) semantic_id: SemanticPropertyId,
    pub(crate) value_kind: NativeValueKind,
}

const fn property(
    semantic_id: SemanticPropertyId,
    value_kind: NativeValueKind,
) -> Option<PropertyMapping> {
    Some(PropertyMapping {
        semantic_id,
        value_kind,
    })
}

pub(crate) fn property_mapping(
    kind: NodeKind,
    native_name: &str,
) -> Option<PropertyMapping> {
    use NativeValueKind as Value;
    use SemanticPropertyId as Id;

    let contextual = match (kind, native_name) {
        (NodeKind::Document, "NumberType") => property(Id::DOCUMENT_NUMBER_TYPE, Value::Enum),
        (NodeKind::Document, "NumberLength") => {
            property(Id::DOCUMENT_NUMBER_LENGTH, Value::Integer)
        }
        (NodeKind::Document, "NumberPeriodicity") => {
            property(Id::DOCUMENT_NUMBER_PERIODICITY, Value::Enum)
        }
        (NodeKind::Document, "Autonumbering" | "AutoNumbering") => {
            property(Id::DOCUMENT_NUMBER_AUTO, Value::Boolean)
        }
        (NodeKind::Document, "Posting") => property(Id::DOCUMENT_POSTING_MODE, Value::Enum),
        (NodeKind::Document, "RealTimePosting") => {
            property(Id::DOCUMENT_REAL_TIME_POSTING_MODE, Value::Enum)
        }
        (NodeKind::Document, "RegisterRecordsDeletion") => property(
            Id::DOCUMENT_REGISTER_RECORDS_DELETION_MODE,
            Value::Enum,
        ),
        (NodeKind::Document, "RegisterRecordsWritingOnPost") => property(
            Id::DOCUMENT_REGISTER_RECORDS_WRITING_ON_POST_MODE,
            Value::Enum,
        ),
        (NodeKind::Catalog, "HierarchyType") => {
            property(Id::CATALOG_HIERARCHY_TYPE, Value::Enum)
        }
        (NodeKind::Catalog, "HierarchyLevelCount" | "LevelCount") => {
            property(Id::CATALOG_HIERARCHY_LEVEL_LIMIT, Value::Integer)
        }
        (NodeKind::Catalog, "CodeLength") => {
            property(Id::CATALOG_CODE_LENGTH, Value::Integer)
        }
        (NodeKind::Catalog, "DescriptionLength") => {
            property(Id::CATALOG_DESCRIPTION_LENGTH, Value::Integer)
        }
        (
            NodeKind::InformationRegister
            | NodeKind::AccumulationRegister
            | NodeKind::AccountingRegister
            | NodeKind::CalculationRegister,
            "InformationRegisterPeriodicity" | "Periodicity",
        ) => property(Id::REGISTER_PERIODICITY, Value::Enum),
        (
            NodeKind::InformationRegister
            | NodeKind::AccumulationRegister
            | NodeKind::AccountingRegister
            | NodeKind::CalculationRegister,
            "WriteMode",
        ) => property(Id::REGISTER_WRITE_MODE, Value::Enum),
        (
            NodeKind::InformationRegister
            | NodeKind::AccumulationRegister
            | NodeKind::AccountingRegister
            | NodeKind::CalculationRegister,
            "RegisterType",
        ) => property(Id::REGISTER_TYPE, Value::Enum),
        (NodeKind::Constant, "Type" | "TypeDescription" | "DataType") => {
            property(Id::CONSTANT_VALUE_TYPE, Value::TypeSet)
        }
        (NodeKind::DefinedType, "Type" | "TypeDescription" | "DataType") => {
            property(Id::DEFINED_TYPE, Value::TypeSet)
        }
        (NodeKind::Report, "MainDataCompositionSchema") => {
            property(Id::REPORT_MAIN_DATA_COMPOSITION_SCHEMA, Value::String)
        }
        (NodeKind::CommonModule, "Global") => property(Id::MODULE_GLOBAL, Value::Boolean),
        (NodeKind::CommonModule, "ClientManagedApplication") => {
            property(Id::MODULE_CLIENT_MANAGED_APPLICATION, Value::Boolean)
        }
        (NodeKind::CommonModule, "Server") => property(Id::MODULE_SERVER, Value::Boolean),
        (NodeKind::CommonModule, "ExternalConnection") => {
            property(Id::MODULE_EXTERNAL_CONNECTION, Value::Boolean)
        }
        (NodeKind::CommonModule, "ClientOrdinaryApplication") => {
            property(Id::MODULE_CLIENT_ORDINARY_APPLICATION, Value::Boolean)
        }
        (NodeKind::CommonModule, "ServerCall") => {
            property(Id::MODULE_SERVER_CALL, Value::Boolean)
        }
        (NodeKind::CommonModule, "Privileged") => {
            property(Id::MODULE_PRIVILEGED, Value::Boolean)
        }
        (NodeKind::CommonModule, "ReturnValuesReuse") => {
            property(Id::MODULE_RETURN_VALUES_REUSE, Value::Enum)
        }
        (NodeKind::ScheduledJob, "MethodName") => property(Id::JOB_METHOD, Value::String),
        (NodeKind::ScheduledJob, "Use") => property(Id::JOB_USE, Value::Boolean),
        (NodeKind::ScheduledJob, "Predefined") => property(Id::JOB_PREDEFINED, Value::Boolean),
        (NodeKind::ScheduledJob, "RestartCountOnFailure") => {
            property(Id::JOB_RESTART_COUNT, Value::Integer)
        }
        (NodeKind::ScheduledJob, "RestartIntervalOnFailure") => {
            property(Id::JOB_RESTART_INTERVAL, Value::Integer)
        }
        (NodeKind::ScheduledJob, "Key") => property(Id::JOB_KEY, Value::String),
        (NodeKind::EventSubscription, "Event") => {
            property(Id::SUBSCRIPTION_EVENT, Value::String)
        }
        (NodeKind::EventSubscription, "Handler") => {
            property(Id::SUBSCRIPTION_HANDLER, Value::String)
        }
        (NodeKind::EventSubscription, "Source") => {
            property(Id::SUBSCRIPTION_SOURCE_TYPE, Value::TypeSet)
        }
        (NodeKind::HttpService, "RootURL" | "RootUrl") => {
            property(Id::HTTP_SERVICE_ROOT_URL, Value::String)
        }
        (NodeKind::HttpService, "ReuseSessions") => {
            property(Id::HTTP_SERVICE_REUSE_SESSIONS, Value::Enum)
        }
        (NodeKind::HttpService, "SessionMaxAge") => {
            property(Id::HTTP_SERVICE_SESSION_MAX_AGE, Value::Integer)
        }
        (NodeKind::HttpServiceUrlTemplate, "Template") => {
            property(Id::HTTP_SERVICE_URL_TEMPLATE, Value::String)
        }
        (NodeKind::HttpServiceMethod, "HTTPMethod") => {
            property(Id::HTTP_SERVICE_METHOD, Value::String)
        }
        (NodeKind::HttpServiceMethod, "Handler") => {
            property(Id::HTTP_SERVICE_HANDLER, Value::String)
        }
        (NodeKind::WebService, "Namespace") => {
            property(Id::WEB_SERVICE_NAMESPACE, Value::String)
        }
        (NodeKind::WebService, "XDTOPackages") => {
            property(Id::WEB_SERVICE_XDTO_PACKAGES, Value::StringList)
        }
        (NodeKind::WebService, "DescriptorFileName") => {
            property(Id::WEB_SERVICE_DESCRIPTOR_FILE_NAME, Value::String)
        }
        (NodeKind::WebService, "ReuseSessions") => {
            property(Id::WEB_SERVICE_REUSE_SESSIONS, Value::Enum)
        }
        (NodeKind::WebService, "SessionMaxAge") => {
            property(Id::WEB_SERVICE_SESSION_MAX_AGE, Value::Integer)
        }
        (NodeKind::WebServiceOperation, "XDTOReturningValueType") => {
            property(Id::WEB_SERVICE_OPERATION_RETURN_TYPE, Value::TypeSet)
        }
        (NodeKind::WebServiceOperation, "Nillable") => {
            property(Id::WEB_SERVICE_OPERATION_NILLABLE, Value::Boolean)
        }
        (NodeKind::WebServiceOperation, "Transactioned") => {
            property(Id::WEB_SERVICE_OPERATION_TRANSACTIONED, Value::Boolean)
        }
        (NodeKind::WebServiceOperation, "ProcedureName") => {
            property(Id::WEB_SERVICE_OPERATION_PROCEDURE_NAME, Value::String)
        }
        (NodeKind::WebServiceParameter, "XDTOValueType") => {
            property(Id::WEB_SERVICE_PARAMETER_TYPE, Value::TypeSet)
        }
        (NodeKind::WebServiceParameter, "Nillable") => {
            property(Id::WEB_SERVICE_PARAMETER_NILLABLE, Value::Boolean)
        }
        (NodeKind::WebServiceParameter, "TransferDirection") => {
            property(Id::WEB_SERVICE_PARAMETER_DIRECTION, Value::Enum)
        }
        (
            NodeKind::Attribute
            | NodeKind::Dimension
            | NodeKind::Resource
            | NodeKind::WebServiceParameter,
            "Type" | "TypeDescription" | "DataType",
        ) => property(Id::FIELD_TYPE, Value::TypeSet),
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "FillChecking") => {
            property(Id::FIELD_FILL_CHECKING, Value::Enum)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "Indexing") => {
            property(Id::FIELD_INDEXING, Value::Enum)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "MultiLine") => {
            property(Id::FIELD_MULTI_LINE, Value::Boolean)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "Use") => {
            property(Id::FIELD_USE, Value::Enum)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "FillValue") => {
            property(Id::FIELD_FILL_VALUE, Value::Polymorphic)
        }
        (NodeKind::Dimension, "Master") => property(Id::FIELD_MASTER, Value::Boolean),
        (NodeKind::Dimension, "MainFilter") => property(Id::FIELD_MAIN_FILTER, Value::Boolean),
        (NodeKind::Dimension, "DenyIncompleteValues") => {
            property(Id::FIELD_DENY_INCOMPLETE_VALUES, Value::Boolean)
        }
        (NodeKind::TabularSection, "Order") => {
            property(Id::TABULAR_SECTION_ORDER, Value::Integer)
        }
        (NodeKind::TabularSection, "LineNumberLength") => {
            property(Id::TABULAR_SECTION_LINE_NUMBER_LENGTH, Value::Integer)
        }
        (NodeKind::Form, "FormType") => property(Id::FORM_TYPE, Value::String),
        (NodeKind::Template | NodeKind::SpreadsheetDocumentTemplate, "TemplateType") => {
            property(Id::TEMPLATE_TYPE, Value::String)
        }
        (NodeKind::Command, "Group") => property(Id::COMMAND_GROUP, Value::String),
        (NodeKind::Command, "Representation") => {
            property(Id::COMMAND_REPRESENTATION, Value::String)
        }
        _ => None,
    };
    contextual.or_else(|| match native_name {
        "Name" => property(Id::METADATA_NAME, Value::String),
        "Uuid" | "UUID" => property(Id::METADATA_UUID, Value::Uuid),
        "Synonym" => property(Id::METADATA_SYNONYM, Value::LocalizedString),
        "Comment" => property(Id::METADATA_COMMENT, Value::String),
        "Code" => property(Id::METADATA_CODE, Value::String),
        "Description" => property(Id::METADATA_DESCRIPTION, Value::String),
        "ObjectPresentation" => property(Id::PRESENTATION_OBJECT, Value::LocalizedString),
        "ExtendedObjectPresentation" => {
            property(Id::PRESENTATION_EXTENDED_OBJECT, Value::LocalizedString)
        }
        "ListPresentation" => property(Id::PRESENTATION_LIST, Value::LocalizedString),
        "ExtendedListPresentation" => {
            property(Id::PRESENTATION_EXTENDED_LIST, Value::LocalizedString)
        }
        "Length" => property(Id::FIELD_LENGTH, Value::Integer),
        "Digits" => property(Id::FIELD_DIGITS, Value::Integer),
        "FractionDigits" => property(Id::FIELD_FRACTION_DIGITS, Value::Integer),
        "UseStandardCommands" => property(Id::COMMAND_USE_STANDARD, Value::Boolean),
        "IncludeHelpInContents" => property(Id::HELP_INCLUDE_IN_CONTENTS, Value::Boolean),
        _ => None,
    })
}

pub(crate) fn relation_property_role(
    kind: NodeKind,
    native_name: &str,
) -> Option<RelationRole> {
    match (kind, native_name) {
        (NodeKind::Document, "BasedOn") => Some(SemanticRelationId::BASED_ON),
        (NodeKind::Document, "RegisterRecords") => {
            Some(SemanticRelationId::REGISTER_RECORDS)
        }
        _ => None,
    }
}

pub(crate) fn object_kind(profile: &MetadataClassProfile) -> Option<NodeKind> {
    let kind = match profile.role {
        MetadataClassRole::Configuration => NodeKind::Configuration,
        MetadataClassRole::TopLevelObject => match profile.class_name {
            "Language" => NodeKind::Language,
            "Subsystem" => NodeKind::Subsystem,
            "StyleItem" => NodeKind::StyleItem,
            "Style" => NodeKind::Style,
            "CommonPicture" => NodeKind::CommonPicture,
            "SessionParameter" => NodeKind::SessionParameter,
            "Role" => NodeKind::Role,
            "CommonTemplate" => NodeKind::CommonTemplate,
            "FilterCriterion" => NodeKind::FilterCriterion,
            "CommonModule" => NodeKind::CommonModule,
            "Bot" => NodeKind::Bot,
            "CommonAttribute" => NodeKind::CommonAttribute,
            "ExchangePlan" => NodeKind::ExchangePlan,
            "XDTOPackage" => NodeKind::XdtoPackage,
            "WebService" => NodeKind::WebService,
            "HTTPService" => NodeKind::HttpService,
            "WSReference" => NodeKind::WebServiceReference,
            "EventSubscription" => NodeKind::EventSubscription,
            "ScheduledJob" => NodeKind::ScheduledJob,
            "SettingsStorage" => NodeKind::SettingsStorage,
            "FunctionalOption" => NodeKind::FunctionalOption,
            "FunctionalOptionsParameter" => NodeKind::FunctionalOptionsParameter,
            "DefinedType" => NodeKind::DefinedType,
            "CommonCommand" => NodeKind::CommonCommand,
            "CommandGroup" => NodeKind::CommandGroup,
            "Constant" => NodeKind::Constant,
            "CommonForm" => NodeKind::CommonForm,
            "Catalog" => NodeKind::Catalog,
            "Document" => NodeKind::Document,
            "DocumentNumerator" => NodeKind::DocumentNumerator,
            "Sequence" => NodeKind::Sequence,
            "DocumentJournal" => NodeKind::DocumentJournal,
            "Enum" => NodeKind::Enumeration,
            "Report" => NodeKind::Report,
            "DataProcessor" => NodeKind::DataProcessor,
            "InformationRegister" => NodeKind::InformationRegister,
            "AccumulationRegister" => NodeKind::AccumulationRegister,
            "ChartOfCharacteristicTypes" => NodeKind::ChartOfCharacteristicTypes,
            "ChartOfAccounts" => NodeKind::ChartOfAccounts,
            "AccountingRegister" => NodeKind::AccountingRegister,
            "ChartOfCalculationTypes" => NodeKind::ChartOfCalculationTypes,
            "CalculationRegister" => NodeKind::CalculationRegister,
            "BusinessProcess" => NodeKind::BusinessProcess,
            "Task" => NodeKind::Task,
            "IntegrationService" => NodeKind::IntegrationService,
            _ => return None,
        },
        MetadataClassRole::Attribute | MetadataClassRole::Column => NodeKind::Attribute,
        MetadataClassRole::Dimension => NodeKind::Dimension,
        MetadataClassRole::Resource => NodeKind::Resource,
        MetadataClassRole::EnumerationValue => NodeKind::EnumerationValue,
        MetadataClassRole::TabularSection => NodeKind::TabularSection,
        MetadataClassRole::Form => NodeKind::Form,
        MetadataClassRole::Template => NodeKind::Template,
        MetadataClassRole::Command => NodeKind::Command,
        MetadataClassRole::HttpServiceUrlTemplate => NodeKind::HttpServiceUrlTemplate,
        MetadataClassRole::HttpServiceMethod => NodeKind::HttpServiceMethod,
        MetadataClassRole::WebServiceOperation => NodeKind::WebServiceOperation,
        MetadataClassRole::WebServiceParameter => NodeKind::WebServiceParameter,
        MetadataClassRole::Unsupported => return None,
    };
    Some(kind)
}

pub(crate) fn child_relation_role(
    owner: &MetadataClassProfile,
    child: &MetadataClassProfile,
) -> Option<RelationRole> {
    let role = match child.role {
        MetadataClassRole::Attribute
            if owner.role == MetadataClassRole::TabularSection =>
        {
            SemanticRelationId::COLUMNS
        }
        MetadataClassRole::Attribute => SemanticRelationId::ATTRIBUTES,
        MetadataClassRole::Column => SemanticRelationId::COLUMNS,
        MetadataClassRole::Dimension => SemanticRelationId::DIMENSIONS,
        MetadataClassRole::Resource => SemanticRelationId::RESOURCES,
        MetadataClassRole::EnumerationValue => SemanticRelationId::ENUM_VALUES,
        MetadataClassRole::TabularSection => SemanticRelationId::TABULAR_SECTIONS,
        MetadataClassRole::Form => SemanticRelationId::FORMS,
        MetadataClassRole::Template => SemanticRelationId::TEMPLATES,
        MetadataClassRole::Command => SemanticRelationId::COMMANDS,
        MetadataClassRole::HttpServiceUrlTemplate => SemanticRelationId::URL_TEMPLATES,
        MetadataClassRole::HttpServiceMethod => SemanticRelationId::METHODS,
        MetadataClassRole::WebServiceOperation => SemanticRelationId::OPERATIONS,
        MetadataClassRole::WebServiceParameter => SemanticRelationId::PARAMETERS,
        MetadataClassRole::TopLevelObject => SemanticRelationId::CHILDREN,
        MetadataClassRole::Configuration | MetadataClassRole::Unsupported => return None,
    };
    Some(role)
}

pub(crate) fn reference_kind(native_class: &str) -> Option<NodeKind> {
    match native_class {
        "Catalog" | "CatalogRef" => Some(NodeKind::Catalog),
        "Document" | "DocumentRef" => Some(NodeKind::Document),
        "Enum" | "EnumRef" => Some(NodeKind::Enumeration),
        "DefinedType" => Some(NodeKind::DefinedType),
        "ExchangePlan" | "ExchangePlanRef" => Some(NodeKind::ExchangePlan),
        "ChartOfCharacteristicTypes" | "ChartOfCharacteristicTypesRef" | "Characteristic" => {
            Some(NodeKind::ChartOfCharacteristicTypes)
        }
        "ChartOfAccounts" | "ChartOfAccountsRef" => Some(NodeKind::ChartOfAccounts),
        "ChartOfCalculationTypes" | "ChartOfCalculationTypesRef" => {
            Some(NodeKind::ChartOfCalculationTypes)
        }
        "DocumentJournal" | "DocumentJournalRef" => Some(NodeKind::DocumentJournal),
        "BusinessProcess" | "BusinessProcessRef" => Some(NodeKind::BusinessProcess),
        "Task" | "TaskRef" => Some(NodeKind::Task),
        "InformationRegister" | "InformationRegisterRecordKey" => {
            Some(NodeKind::InformationRegister)
        }
        "AccumulationRegister" | "AccumulationRegisterRecordKey" => {
            Some(NodeKind::AccumulationRegister)
        }
        "AccountingRegister" | "AccountingRegisterRecordKey" => {
            Some(NodeKind::AccountingRegister)
        }
        "CalculationRegister" | "CalculationRegisterRecordKey" => {
            Some(NodeKind::CalculationRegister)
        }
        _ => None,
    }
}

pub(crate) fn is_field_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource
    )
}
