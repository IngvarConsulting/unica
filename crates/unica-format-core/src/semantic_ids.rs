//! Closed semantic vocabulary owned by the compiler.

use std::{
    borrow::Borrow,
    fmt::{Display, Formatter},
};

use serde::{Serialize, Serializer};

macro_rules! semantic_id_registry {
    (
        $(#[$meta:meta])*
        $name:ident {
            $($constant:ident => $value:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(&'static str);

        impl $name {
            const fn core(value: &'static str) -> Self {
                Self(value)
            }

            $(pub const $constant: Self = Self::core($value);)+

            pub const ALL: &'static [Self] = &[$(Self::$constant),+];

            pub const fn as_str(self) -> &'static str {
                self.0
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$constant),)+
                    _ => None,
                }
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.0)
            }
        }
    };
}

semantic_id_registry!(
    /// Compiler-owned semantic property identifier.
    ///
    /// Adapter-defined strings cannot become semantic property IDs:
    ///
    /// ```compile_fail
    /// use unica_format_core::semantic_ids::SemanticPropertyId;
    /// let _: SemanticPropertyId = "adapter.custom".into();
    /// ```
    SemanticPropertyId {
        METADATA_KIND => "metadata.kind",
        METADATA_NAME => "metadata.name",
        METADATA_UUID => "metadata.uuid",
        METADATA_SYNONYM => "metadata.synonym",
        METADATA_COMMENT => "metadata.comment",
        METADATA_CODE => "metadata.code",
        METADATA_DESCRIPTION => "metadata.description",
        PRESENTATION_OBJECT => "presentation.object",
        PRESENTATION_EXTENDED_OBJECT => "presentation.extendedObject",
        PRESENTATION_LIST => "presentation.list",
        PRESENTATION_EXTENDED_LIST => "presentation.extendedList",
        SUPPORT_STATE => "support.state",
        SUPPORT_AUTHORABILITY => "support.authorability",
        SUPPORT_EDIT_CAPABILITY => "support.editCapability",
        DOCUMENT_NUMBER_TYPE => "document.number.type",
        DOCUMENT_NUMBER_LENGTH => "document.number.length",
        DOCUMENT_NUMBER_PERIODICITY => "document.number.periodicity",
        DOCUMENT_NUMBER_AUTO => "document.number.auto",
        DOCUMENT_POSTING_MODE => "document.posting.mode",
        DOCUMENT_REAL_TIME_POSTING_MODE => "document.realTimePosting.mode",
        DOCUMENT_REGISTER_RECORDS_DELETION_MODE => "document.registerRecordsDeletion.mode",
        DOCUMENT_REGISTER_RECORDS_WRITING_ON_POST_MODE => "document.registerRecordsWritingOnPost.mode",
        CATALOG_HIERARCHICAL => "catalog.hierarchy.enabled",
        CATALOG_HIERARCHY_TYPE => "catalog.hierarchy.type",
        CATALOG_HIERARCHY_LEVEL_LIMITED => "catalog.hierarchy.levelLimit.enabled",
        CATALOG_HIERARCHY_LEVEL_COUNT => "catalog.hierarchy.levelCount",
        CATALOG_HIERARCHY_LEVEL_LIMIT => "catalog.hierarchy.levelLimit",
        CATALOG_CODE_LENGTH => "catalog.code.length",
        CATALOG_CODE_SERIES => "catalog.code.series",
        CATALOG_DESCRIPTION_LENGTH => "catalog.description.length",
        REGISTER_PERIODICITY => "register.periodicity",
        REGISTER_WRITE_MODE => "register.writeMode",
        REGISTER_TYPE => "register.type",
        CONSTANT_VALUE_TYPE => "constant.value.type",
        REPORT_MAIN_DATA_COMPOSITION_SCHEMA => "report.mainDataCompositionSchema",
        DEFINED_TYPE => "definedType.type",
        MODULE_GLOBAL => "module.global",
        MODULE_CLIENT_MANAGED_APPLICATION => "module.clientManagedApplication",
        MODULE_SERVER => "module.server",
        MODULE_EXTERNAL_CONNECTION => "module.externalConnection",
        MODULE_CLIENT_ORDINARY_APPLICATION => "module.clientOrdinaryApplication",
        MODULE_SERVER_CALL => "module.serverCall",
        MODULE_PRIVILEGED => "module.privileged",
        MODULE_RETURN_VALUES_REUSE => "module.returnValuesReuse",
        JOB_METHOD => "job.method",
        JOB_USE => "job.use",
        JOB_PREDEFINED => "job.predefined",
        JOB_RESTART_COUNT => "job.restart.count",
        JOB_RESTART_INTERVAL => "job.restart.interval",
        JOB_KEY => "job.key",
        SUBSCRIPTION_EVENT => "subscription.event",
        SUBSCRIPTION_HANDLER => "subscription.handler",
        SUBSCRIPTION_SOURCE_TYPE => "subscription.source.type",
        HTTP_SERVICE_ROOT_URL => "httpService.rootUrl",
        HTTP_SERVICE_REUSE_SESSIONS => "httpService.reuseSessions",
        HTTP_SERVICE_SESSION_MAX_AGE => "httpService.sessionMaxAge",
        HTTP_SERVICE_URL_TEMPLATE => "httpService.urlTemplate.template",
        HTTP_SERVICE_METHOD => "httpService.method.httpMethod",
        HTTP_SERVICE_HANDLER => "httpService.method.handler",
        WEB_SERVICE_NAMESPACE => "webService.namespace",
        WEB_SERVICE_XDTO_PACKAGES => "webService.xdtoPackages",
        WEB_SERVICE_DESCRIPTOR_FILE_NAME => "webService.descriptorFileName",
        WEB_SERVICE_REUSE_SESSIONS => "webService.reuseSessions",
        WEB_SERVICE_SESSION_MAX_AGE => "webService.sessionMaxAge",
        WEB_SERVICE_OPERATION_RETURN_TYPE => "webService.operation.returnType",
        WEB_SERVICE_OPERATION_NILLABLE => "webService.operation.nillable",
        WEB_SERVICE_OPERATION_TRANSACTIONED => "webService.operation.transactioned",
        WEB_SERVICE_OPERATION_PROCEDURE_NAME => "webService.operation.procedureName",
        WEB_SERVICE_PARAMETER_TYPE => "webService.parameter.type",
        WEB_SERVICE_PARAMETER_NILLABLE => "webService.parameter.nillable",
        WEB_SERVICE_PARAMETER_DIRECTION => "webService.parameter.direction",
        FIELD_TYPE => "field.type",
        FIELD_REQUIRED => "field.required",
        FIELD_FILL_CHECKING => "field.fillChecking",
        FIELD_INDEXING => "field.indexing",
        FIELD_MULTI_LINE => "field.multiLine",
        FIELD_USE => "field.use",
        FIELD_FILL_VALUE => "field.fillValue",
        FIELD_MASTER => "field.master",
        FIELD_MAIN_FILTER => "field.mainFilter",
        FIELD_DENY_INCOMPLETE_VALUES => "field.denyIncompleteValues",
        FIELD_LENGTH => "field.length",
        FIELD_DIGITS => "field.digits",
        FIELD_FRACTION_DIGITS => "field.fractionDigits",
        TABULAR_SECTION_ORDER => "tabularSection.order",
        TABULAR_SECTION_LINE_NUMBER_LENGTH => "tabularSection.lineNumberLength",
        FORM_TYPE => "form.type",
        TEMPLATE_TYPE => "template.type",
        COMMAND_GROUP => "command.group",
        COMMAND_REPRESENTATION => "command.representation",
        COMMAND_USE_STANDARD => "command.useStandard",
        HELP_INCLUDE_IN_CONTENTS => "help.includeInContents",
        ACCESS_NEW_OBJECTS_DEFAULT => "access.newObjects.defaultAllowed",
        ACCESS_ATTRIBUTES_DEFAULT => "access.attributes.defaultAllowed",
        ACCESS_CHILD_OBJECTS_INDEPENDENT => "access.childObjects.independent",
        ACCESS_PERMISSION_NAME => "access.permission.name",
        ACCESS_PERMISSION_ALLOWED => "access.permission.allowed",
        ACCESS_RESTRICTION_CONDITIONS => "access.restriction.conditions",
        BACKING_DESCRIPTOR_AVAILABLE => "backing.descriptor.available",
        BACKING_DESCRIPTOR_UUID => "backing.descriptor.uuid",
        BACKING_CONTENT_AVAILABLE => "backing.content.available",
        BACKING_CONTENT_OPAQUE => "backing.content.opaque",
        UNKNOWN_FACTS => "unknown.facts",
    }
);

semantic_id_registry!(
    /// Compiler-owned semantic relation identifier.
    ///
    /// ```compile_fail
    /// use unica_format_core::semantic_ids::SemanticRelationId;
    /// let _: SemanticRelationId = "adapter.children".into();
    /// ```
    SemanticRelationId {
        CHILDREN => "children",
        ATTRIBUTES => "attributes",
        DIMENSIONS => "dimensions",
        RESOURCES => "resources",
        TABULAR_SECTIONS => "tabularSections",
        COLUMNS => "columns",
        FORMS => "forms",
        COMMANDS => "commands",
        TEMPLATES => "templates",
        ENUM_VALUES => "enumValues",
        BASED_ON => "basedOn",
        REGISTER_RECORDS => "registerRecords",
        URL_TEMPLATES => "urlTemplates",
        METHODS => "methods",
        OPERATIONS => "operations",
        PARAMETERS => "parameters",
        REFERENCES => "references",
        ACCESS_PERMISSIONS => "accessPermissions",
        ACCESS_TARGET => "accessTarget",
        RESTRICTION_TEMPLATES => "restrictionTemplates",
        UNKNOWN => "unknown",
    }
);

// Transitional source code may retain the old enum-style spelling while every
// stored value is already the closed semantic ID.
#[allow(non_upper_case_globals)]
impl SemanticRelationId {
    pub const Children: Self = Self::CHILDREN;
    pub const Attributes: Self = Self::ATTRIBUTES;
    pub const Dimensions: Self = Self::DIMENSIONS;
    pub const Resources: Self = Self::RESOURCES;
    pub const TabularSections: Self = Self::TABULAR_SECTIONS;
    pub const Columns: Self = Self::COLUMNS;
    pub const Forms: Self = Self::FORMS;
    pub const Commands: Self = Self::COMMANDS;
    pub const Templates: Self = Self::TEMPLATES;
    pub const EnumValues: Self = Self::ENUM_VALUES;
    pub const BasedOn: Self = Self::BASED_ON;
    pub const RegisterRecords: Self = Self::REGISTER_RECORDS;
    pub const UrlTemplates: Self = Self::URL_TEMPLATES;
    pub const Methods: Self = Self::METHODS;
    pub const Operations: Self = Self::OPERATIONS;
    pub const Parameters: Self = Self::PARAMETERS;
    pub const References: Self = Self::REFERENCES;
    pub const AccessPermissions: Self = Self::ACCESS_PERMISSIONS;
    pub const AccessTarget: Self = Self::ACCESS_TARGET;
    pub const RestrictionTemplates: Self = Self::RESTRICTION_TEMPLATES;
    pub const Unknown: Self = Self::UNKNOWN;
}

semantic_id_registry!(
    /// Compiler-owned semantic facet identifier.
    SemanticFacetId {
        IDENTITY => "identity",
        PRESENTATION => "presentation",
        SUPPORT => "support",
        NUMBERING => "numbering",
        POSTING => "posting",
        HIERARCHY => "hierarchy",
        TYPING => "typing",
        FIELDS => "fields",
        MODULE_EXECUTION => "moduleExecution",
        SCHEDULING => "scheduling",
        SUBSCRIPTION => "subscription",
        SERVICE => "service",
        OPERATION => "operation",
        STRUCTURE => "structure",
        ACCESS => "access",
        BACKING => "backing",
        UNKNOWN => "unknown",
    }
);

/// Compiler-owned semantic object kind. No variant carries adapter text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticObjectKind {
    SourceRoot,
    Unknown,
    Configuration,
    Language,
    Subsystem,
    StyleItem,
    Style,
    CommonPicture,
    SessionParameter,
    Role,
    CommonTemplate,
    FilterCriterion,
    CommonModule,
    Bot,
    CommonAttribute,
    ExchangePlan,
    XdtoPackage,
    WebService,
    HttpService,
    WebServiceReference,
    EventSubscription,
    ScheduledJob,
    SettingsStorage,
    FunctionalOption,
    FunctionalOptionsParameter,
    DefinedType,
    CommonCommand,
    CommandGroup,
    Constant,
    CommonForm,
    Catalog,
    Document,
    DocumentNumerator,
    Sequence,
    DocumentJournal,
    Enumeration,
    Report,
    DataProcessor,
    InformationRegister,
    AccumulationRegister,
    ChartOfCharacteristicTypes,
    ChartOfAccounts,
    AccountingRegister,
    ChartOfCalculationTypes,
    CalculationRegister,
    BusinessProcess,
    Task,
    IntegrationService,
    HttpServiceUrlTemplate,
    HttpServiceMethod,
    WebServiceOperation,
    WebServiceParameter,
    EnumerationValue,
    Attribute,
    Dimension,
    Resource,
    TabularSection,
    Form,
    FormAttribute,
    FormCommand,
    FormElement,
    Template,
    SpreadsheetDocumentTemplate,
    Command,
    AccessPermission,
    AccessRestrictionTemplate,
}

impl SemanticObjectKind {
    pub const ALL: &'static [Self] = &[
        Self::SourceRoot,
        Self::Unknown,
        Self::Configuration,
        Self::Language,
        Self::Subsystem,
        Self::StyleItem,
        Self::Style,
        Self::CommonPicture,
        Self::SessionParameter,
        Self::Role,
        Self::CommonTemplate,
        Self::FilterCriterion,
        Self::CommonModule,
        Self::Bot,
        Self::CommonAttribute,
        Self::ExchangePlan,
        Self::XdtoPackage,
        Self::WebService,
        Self::HttpService,
        Self::WebServiceReference,
        Self::EventSubscription,
        Self::ScheduledJob,
        Self::SettingsStorage,
        Self::FunctionalOption,
        Self::FunctionalOptionsParameter,
        Self::DefinedType,
        Self::CommonCommand,
        Self::CommandGroup,
        Self::Constant,
        Self::CommonForm,
        Self::Catalog,
        Self::Document,
        Self::DocumentNumerator,
        Self::Sequence,
        Self::DocumentJournal,
        Self::Enumeration,
        Self::Report,
        Self::DataProcessor,
        Self::InformationRegister,
        Self::AccumulationRegister,
        Self::ChartOfCharacteristicTypes,
        Self::ChartOfAccounts,
        Self::AccountingRegister,
        Self::ChartOfCalculationTypes,
        Self::CalculationRegister,
        Self::BusinessProcess,
        Self::Task,
        Self::IntegrationService,
        Self::HttpServiceUrlTemplate,
        Self::HttpServiceMethod,
        Self::WebServiceOperation,
        Self::WebServiceParameter,
        Self::EnumerationValue,
        Self::Attribute,
        Self::Dimension,
        Self::Resource,
        Self::TabularSection,
        Self::Form,
        Self::FormAttribute,
        Self::FormCommand,
        Self::FormElement,
        Self::Template,
        Self::SpreadsheetDocumentTemplate,
        Self::Command,
        Self::AccessPermission,
        Self::AccessRestrictionTemplate,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceRoot => "sourceRoot",
            Self::Unknown => "unknown",
            Self::Configuration => "configuration",
            Self::Language => "language",
            Self::Subsystem => "subsystem",
            Self::StyleItem => "styleItem",
            Self::Style => "style",
            Self::CommonPicture => "commonPicture",
            Self::SessionParameter => "sessionParameter",
            Self::Role => "role",
            Self::CommonTemplate => "commonTemplate",
            Self::FilterCriterion => "filterCriterion",
            Self::CommonModule => "commonModule",
            Self::Bot => "bot",
            Self::CommonAttribute => "commonAttribute",
            Self::ExchangePlan => "exchangePlan",
            Self::XdtoPackage => "xdtoPackage",
            Self::WebService => "webService",
            Self::HttpService => "httpService",
            Self::WebServiceReference => "webServiceReference",
            Self::EventSubscription => "eventSubscription",
            Self::ScheduledJob => "scheduledJob",
            Self::SettingsStorage => "settingsStorage",
            Self::FunctionalOption => "functionalOption",
            Self::FunctionalOptionsParameter => "functionalOptionsParameter",
            Self::DefinedType => "definedType",
            Self::CommonCommand => "commonCommand",
            Self::CommandGroup => "commandGroup",
            Self::Constant => "constant",
            Self::CommonForm => "commonForm",
            Self::Catalog => "catalog",
            Self::Document => "document",
            Self::DocumentNumerator => "documentNumerator",
            Self::Sequence => "sequence",
            Self::DocumentJournal => "documentJournal",
            Self::Enumeration => "enumeration",
            Self::Report => "report",
            Self::DataProcessor => "dataProcessor",
            Self::InformationRegister => "informationRegister",
            Self::AccumulationRegister => "accumulationRegister",
            Self::ChartOfCharacteristicTypes => "chartOfCharacteristicTypes",
            Self::ChartOfAccounts => "chartOfAccounts",
            Self::AccountingRegister => "accountingRegister",
            Self::ChartOfCalculationTypes => "chartOfCalculationTypes",
            Self::CalculationRegister => "calculationRegister",
            Self::BusinessProcess => "businessProcess",
            Self::Task => "task",
            Self::IntegrationService => "integrationService",
            Self::HttpServiceUrlTemplate => "httpServiceUrlTemplate",
            Self::HttpServiceMethod => "httpServiceMethod",
            Self::WebServiceOperation => "webServiceOperation",
            Self::WebServiceParameter => "webServiceParameter",
            Self::EnumerationValue => "enumerationValue",
            Self::Attribute => "attribute",
            Self::Dimension => "dimension",
            Self::Resource => "resource",
            Self::TabularSection => "tabularSection",
            Self::Form => "form",
            Self::FormAttribute => "formAttribute",
            Self::FormCommand => "formCommand",
            Self::FormElement => "formElement",
            Self::Template => "template",
            Self::SpreadsheetDocumentTemplate => "spreadsheetDocumentTemplate",
            Self::Command => "command",
            Self::AccessPermission => "accessPermission",
            Self::AccessRestrictionTemplate => "accessRestrictionTemplate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
    }
}

impl Display for SemanticObjectKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for SemanticObjectKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

semantic_id_registry!(
    /// Closed symbols used by semantic enum-valued properties.
    SemanticEnumValue {
        STRING => "string",
        NUMBER => "number",
        NONPERIODICAL => "nonperiodical",
        SECOND => "second",
        DAY => "day",
        MONTH => "month",
        QUARTER => "quarter",
        YEAR => "year",
        WHOLE_COLLECTION => "wholeCollection",
        WITHIN_OWNER_SCOPE => "withinOwnerScope",
        WITHIN_PARENT_SCOPE => "withinParentScope",
        RECORDER_POSITION => "recorderPosition",
        ALLOW => "allow",
        DENY => "deny",
        DELETE_AUTOMATIC => "deleteAutomatic",
        DELETE_ON_REVERSAL => "deleteOnReversal",
        DELETE_DISABLED => "deleteDisabled",
        WRITE_MODIFIED => "writeModified",
        WRITE_SELECTED => "writeSelected",
        WRITE_ALL => "writeAll",
        HIERARCHY_OF_ITEMS => "hierarchyOfItems",
        HIERARCHY_OF_GROUPS_AND_ITEMS => "hierarchyOfGroupsAndItems",
        BALANCE => "balance",
        TURNOVERS => "turnovers",
        INDEPENDENT => "independent",
        RECORDER_SUBORDINATE => "recorderSubordinate",
        DONT_CHECK => "dontCheck",
        SHOW_ERROR => "showError",
        SHOW_WARNING => "showWarning",
        DONT_INDEX => "dontIndex",
        INDEX => "index",
        INDEX_WITH_ADDITIONAL_ORDER => "indexWithAdditionalOrder",
        USE => "use",
        DONT_USE => "dontUse",
        FOR_ITEM => "forItem",
        GROUP_ONLY => "groupOnly",
        GROUPS_AND_ITEMS => "groupsAndItems",
        DURING_REQUEST => "duringRequest",
        DURING_SESSION => "duringSession",
        IN => "in",
        OUT => "out",
        IN_OUT => "inOut",
        MANAGED => "managed",
        ORDINARY => "ordinary",
        DATA_COMPOSITION_SCHEMA => "dataCompositionSchema",
        SPREADSHEET_DOCUMENT => "spreadsheetDocument",
        BINARY_DATA => "binaryData",
        TEXT_DOCUMENT => "textDocument",
        HTML_DOCUMENT => "htmlDocument",
    }
);
