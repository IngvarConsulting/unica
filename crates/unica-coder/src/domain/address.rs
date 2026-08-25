use crate::domain::source_target::{metadata_address_kind_spellings, xml_ncname_is_valid};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeKind {
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
    WsReference,
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
    Enum,
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
    Form,
    Template,
    Command,
    ExternalDataProcessor,
    ExternalReport,
    WebSocketClient,
    Attribute,
    StandardAttribute,
    TabularSection,
    Dimension,
    Resource,
    EnumValue,
    Column,
    Recalculation,
    Item,
    Parameter,
    Right,
    Rls,
    DataSet,
    Field,
    Query,
    Calculation,
    Setting,
    Area,
    Module,
    Method,
    Body,
    Region,
    Interface,
    Event,
    Compilation,
    UrlTemplate,
    Operation,
    Channel,
    Namespace,
    Type,
    Property,
}

const LEGACY_METADATA_KINDS: &[NodeKind] = &[
    NodeKind::Language,
    NodeKind::Subsystem,
    NodeKind::StyleItem,
    NodeKind::Style,
    NodeKind::CommonPicture,
    NodeKind::SessionParameter,
    NodeKind::Role,
    NodeKind::CommonTemplate,
    NodeKind::FilterCriterion,
    NodeKind::CommonModule,
    NodeKind::Bot,
    NodeKind::CommonAttribute,
    NodeKind::ExchangePlan,
    NodeKind::XdtoPackage,
    NodeKind::WebService,
    NodeKind::HttpService,
    NodeKind::WsReference,
    NodeKind::EventSubscription,
    NodeKind::ScheduledJob,
    NodeKind::SettingsStorage,
    NodeKind::FunctionalOption,
    NodeKind::FunctionalOptionsParameter,
    NodeKind::DefinedType,
    NodeKind::CommonCommand,
    NodeKind::CommandGroup,
    NodeKind::Constant,
    NodeKind::CommonForm,
    NodeKind::Catalog,
    NodeKind::Document,
    NodeKind::DocumentNumerator,
    NodeKind::Sequence,
    NodeKind::DocumentJournal,
    NodeKind::Enum,
    NodeKind::Report,
    NodeKind::DataProcessor,
    NodeKind::InformationRegister,
    NodeKind::AccumulationRegister,
    NodeKind::ChartOfCharacteristicTypes,
    NodeKind::ChartOfAccounts,
    NodeKind::AccountingRegister,
    NodeKind::ChartOfCalculationTypes,
    NodeKind::CalculationRegister,
    NodeKind::BusinessProcess,
    NodeKind::Task,
    NodeKind::IntegrationService,
    NodeKind::Form,
    NodeKind::Template,
    NodeKind::Command,
    NodeKind::ExternalDataProcessor,
    NodeKind::ExternalReport,
];

#[derive(Clone, Copy)]
struct V13KindSpelling {
    kind: NodeKind,
    aliases: &'static [&'static str],
}

const V13_KIND_SPELLINGS: &[V13KindSpelling] = &[
    spelling(NodeKind::Configuration, &["Конфигурация"]),
    spelling(NodeKind::WebSocketClient, &[]),
    spelling(NodeKind::Attribute, &["Реквизит"]),
    spelling(NodeKind::StandardAttribute, &["СтандартныйРеквизит"]),
    spelling(NodeKind::TabularSection, &["ТабличнаяЧасть"]),
    spelling(NodeKind::Dimension, &["Измерение"]),
    spelling(NodeKind::Resource, &["Ресурс"]),
    spelling(NodeKind::EnumValue, &["ЗначениеПеречисления"]),
    spelling(NodeKind::Column, &["Колонка"]),
    spelling(
        NodeKind::Recalculation,
        &["Перерасчет", "Перерасчёт", "Перерасчеты", "Перерасчёты"],
    ),
    spelling(NodeKind::Item, &["Элемент"]),
    spelling(NodeKind::Parameter, &["Параметр"]),
    spelling(NodeKind::Right, &["Право"]),
    spelling(NodeKind::Rls, &[]),
    spelling(NodeKind::DataSet, &["НаборДанных"]),
    spelling(NodeKind::Field, &["Поле"]),
    spelling(NodeKind::Query, &["Запрос"]),
    spelling(NodeKind::Calculation, &["Вычисление"]),
    spelling(NodeKind::Setting, &["Настройка"]),
    spelling(NodeKind::Area, &["Область"]),
    spelling(NodeKind::Module, &["Модуль"]),
    spelling(NodeKind::Method, &["Метод"]),
    spelling(NodeKind::Body, &["Тело"]),
    spelling(NodeKind::Region, &["ОбластьКода"]),
    spelling(NodeKind::Interface, &["Интерфейс"]),
    spelling(NodeKind::Event, &["Событие"]),
    spelling(NodeKind::Compilation, &["Компиляция"]),
    spelling(NodeKind::UrlTemplate, &["ШаблонURL"]),
    spelling(NodeKind::Operation, &["Операция"]),
    spelling(NodeKind::Channel, &["Канал"]),
    spelling(NodeKind::Namespace, &["ПространствоИмен"]),
    spelling(NodeKind::Type, &["Тип"]),
    spelling(NodeKind::Property, &["Свойство"]),
];

const fn spelling(kind: NodeKind, aliases: &'static [&'static str]) -> V13KindSpelling {
    V13KindSpelling { kind, aliases }
}

impl NodeKind {
    #[cfg(test)]
    pub(crate) const fn metadata_kinds() -> &'static [Self] {
        LEGACY_METADATA_KINDS
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, AddressError> {
        for kind in LEGACY_METADATA_KINDS {
            let spellings = metadata_address_kind_spellings(kind.as_str())
                .expect("the v0.13 metadata variants mirror the proven v0.12 registry");
            if spellings.contains(&raw) {
                return Ok(*kind);
            }
        }
        V13_KIND_SPELLINGS
            .iter()
            .find(|spelling| spelling.kind.as_str() == raw || spelling.aliases.contains(&raw))
            .map(|spelling| spelling.kind)
            .ok_or_else(|| {
                AddressError::new(
                    AddressErrorCode::UnknownKind,
                    format!("unknown logical node kind `{raw}`"),
                )
            })
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "Configuration",
            Self::Language => "Language",
            Self::Subsystem => "Subsystem",
            Self::StyleItem => "StyleItem",
            Self::Style => "Style",
            Self::CommonPicture => "CommonPicture",
            Self::SessionParameter => "SessionParameter",
            Self::Role => "Role",
            Self::CommonTemplate => "CommonTemplate",
            Self::FilterCriterion => "FilterCriterion",
            Self::CommonModule => "CommonModule",
            Self::Bot => "Bot",
            Self::CommonAttribute => "CommonAttribute",
            Self::ExchangePlan => "ExchangePlan",
            Self::XdtoPackage => "XDTOPackage",
            Self::WebService => "WebService",
            Self::HttpService => "HTTPService",
            Self::WsReference => "WSReference",
            Self::EventSubscription => "EventSubscription",
            Self::ScheduledJob => "ScheduledJob",
            Self::SettingsStorage => "SettingsStorage",
            Self::FunctionalOption => "FunctionalOption",
            Self::FunctionalOptionsParameter => "FunctionalOptionsParameter",
            Self::DefinedType => "DefinedType",
            Self::CommonCommand => "CommonCommand",
            Self::CommandGroup => "CommandGroup",
            Self::Constant => "Constant",
            Self::CommonForm => "CommonForm",
            Self::Catalog => "Catalog",
            Self::Document => "Document",
            Self::DocumentNumerator => "DocumentNumerator",
            Self::Sequence => "Sequence",
            Self::DocumentJournal => "DocumentJournal",
            Self::Enum => "Enum",
            Self::Report => "Report",
            Self::DataProcessor => "DataProcessor",
            Self::InformationRegister => "InformationRegister",
            Self::AccumulationRegister => "AccumulationRegister",
            Self::ChartOfCharacteristicTypes => "ChartOfCharacteristicTypes",
            Self::ChartOfAccounts => "ChartOfAccounts",
            Self::AccountingRegister => "AccountingRegister",
            Self::ChartOfCalculationTypes => "ChartOfCalculationTypes",
            Self::CalculationRegister => "CalculationRegister",
            Self::BusinessProcess => "BusinessProcess",
            Self::Task => "Task",
            Self::IntegrationService => "IntegrationService",
            Self::Form => "Form",
            Self::Template => "Template",
            Self::Command => "Command",
            Self::ExternalDataProcessor => "ExternalDataProcessor",
            Self::ExternalReport => "ExternalReport",
            Self::WebSocketClient => "WebSocketClient",
            Self::Attribute => "Attribute",
            Self::StandardAttribute => "StandardAttribute",
            Self::TabularSection => "TabularSection",
            Self::Dimension => "Dimension",
            Self::Resource => "Resource",
            Self::EnumValue => "EnumValue",
            Self::Column => "Column",
            Self::Recalculation => "Recalculation",
            Self::Item => "Item",
            Self::Parameter => "Parameter",
            Self::Right => "Right",
            Self::Rls => "RLS",
            Self::DataSet => "DataSet",
            Self::Field => "Field",
            Self::Query => "Query",
            Self::Calculation => "Calculation",
            Self::Setting => "Setting",
            Self::Area => "Area",
            Self::Module => "Module",
            Self::Method => "Method",
            Self::Body => "Body",
            Self::Region => "Region",
            Self::Interface => "Interface",
            Self::Event => "Event",
            Self::Compilation => "Compilation",
            Self::UrlTemplate => "URLTemplate",
            Self::Operation => "Operation",
            Self::Channel => "Channel",
            Self::Namespace => "Namespace",
            Self::Type => "Type",
            Self::Property => "Property",
        }
    }

    pub(crate) fn is_metadata_kind(self) -> bool {
        self == Self::WebSocketClient || LEGACY_METADATA_KINDS.contains(&self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AddressSegment {
    kind: NodeKind,
    name: Option<String>,
}

impl AddressSegment {
    pub(crate) const fn kind(&self) -> NodeKind {
        self.kind
    }

    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct QualifiedAddress {
    pub(crate) source_set: String,
    pub(crate) segments: Vec<AddressSegment>,
}

impl QualifiedAddress {
    /// Resolves user input against the source sets already selected by the
    /// workspace. Identity parsing remains strict; this contextual boundary is
    /// the only place where an omitted source-set prefix is inferred.
    pub(crate) fn resolve_input(
        raw: &str,
        available_source_sets: &[&str],
    ) -> Result<Self, AddressError> {
        if raw.contains(':') {
            return Self::parse(raw);
        }
        match available_source_sets {
            [source_set] => Self::parse(&format!("{source_set}:{raw}")),
            [] => Err(AddressError::new(
                AddressErrorCode::SourceSetRequired,
                "unqualified logical address requires one available source set",
            )),
            _ => Err(AddressError::new(
                AddressErrorCode::AmbiguousAddress,
                "unqualified logical address is ambiguous across source sets",
            )),
        }
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, AddressError> {
        let Some((source_set, logical_path)) = raw.split_once(':') else {
            return Err(AddressError::new(
                AddressErrorCode::SourceSetRequired,
                "qualified logical address requires a source-set prefix",
            ));
        };
        if source_set.is_empty() || !xml_ncname_is_valid(source_set) {
            return Err(AddressError::new(
                AddressErrorCode::SourceSetRequired,
                "qualified logical address requires a valid non-empty source set",
            ));
        }
        if logical_path.is_empty() {
            return Err(AddressError::new(
                AddressErrorCode::AddressEmpty,
                "qualified logical address has no logical path",
            ));
        }
        let parts = logical_path.split('.').collect::<Vec<_>>();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(AddressError::new(
                AddressErrorCode::EmptySegment,
                "qualified logical address contains an empty segment",
            ));
        }

        let mut segments = Vec::with_capacity(parts.len().div_ceil(2));
        for pair in parts.chunks(2) {
            let kind = NodeKind::parse(pair[0])?;
            let name = pair.get(1).copied();
            if name.is_some_and(|name| !xml_ncname_is_valid(name)) {
                return Err(AddressError::new(
                    AddressErrorCode::InvalidName,
                    format!("invalid application name `{}`", name.unwrap_or_default()),
                ));
            }
            segments.push(AddressSegment {
                kind,
                name: name.map(str::to_string),
            });
        }

        let sole_configuration_root = matches!(
            segments.as_slice(),
            [root] if root.kind == NodeKind::Configuration && root.name.is_none()
        );
        if !sole_configuration_root
            && segments
                .iter()
                .any(|segment| segment.kind == NodeKind::Configuration)
        {
            return Err(AddressError::new(
                AddressErrorCode::ConfigurationRootOnly,
                "Configuration is the single un-named configuration-root address",
            ));
        }

        Ok(Self {
            source_set: source_set.to_string(),
            segments,
        })
    }

    pub(crate) fn source_set(&self) -> &str {
        &self.source_set
    }

    pub(crate) fn segments(&self) -> &[AddressSegment] {
        &self.segments
    }

    pub(crate) fn logical_path(&self) -> String {
        render_segments(&self.segments)
    }
}

fn render_segments(segments: &[AddressSegment]) -> String {
    let mut rendered = Vec::with_capacity(segments.len() * 2);
    for segment in segments {
        rendered.push(segment.kind.as_str());
        if let Some(name) = segment.name() {
            rendered.push(name);
        }
    }
    rendered.join(".")
}

impl fmt::Display for QualifiedAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.source_set, self.logical_path())
    }
}

impl Serialize for QualifiedAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AddressErrorCode {
    SourceSetRequired,
    AmbiguousAddress,
    AddressEmpty,
    EmptySegment,
    UnknownKind,
    InvalidName,
    ConfigurationRootOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AddressError {
    code: AddressErrorCode,
    message: String,
}

impl AddressError {
    fn new(code: AddressErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> AddressErrorCode {
        self.code
    }
}

impl fmt::Display for AddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AddressError {}

#[cfg(test)]
mod tests {
    use super::{AddressErrorCode, NodeKind, QualifiedAddress};
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AddressFixture {
        valid_addresses: Vec<ValidAddressCase>,
        contextual_inputs: Vec<ContextualInputCase>,
        invalid_addresses: Vec<InvalidAddressCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ValidAddressCase {
        case: String,
        input: String,
        canonical: String,
        terminal_kind: String,
        route: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct InvalidAddressCase {
        case: String,
        input: String,
        error: AddressErrorCode,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ContextualInputCase {
        case: String,
        input: String,
        available_source_sets: Vec<String>,
        canonical: Option<String>,
        error: Option<AddressErrorCode>,
    }

    fn fixture() -> AddressFixture {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v013/address-profile-8.3.27.json"
        ))
        .expect("the checked address fixture must be valid JSON")
    }

    #[test]
    fn qualified_addresses_are_table_driven_canonical_and_arbitrarily_deep() {
        let fixture = fixture();
        let mut saw_configuration_root = false;
        let mut saw_metadata_branch = false;
        let mut saw_branch_ending_kind = false;
        let mut saw_arbitrary_depth = false;

        for case in fixture.valid_addresses {
            let address = QualifiedAddress::parse(&case.input)
                .unwrap_or_else(|error| panic!("{}: {error}", case.case));
            assert_eq!(address.to_string(), case.canonical, "{}", case.case);
            assert_eq!(
                address
                    .segments()
                    .last()
                    .expect("a qualified address has a segment")
                    .kind()
                    .as_str(),
                case.terminal_kind,
                "{}",
                case.case
            );
            assert_eq!(
                serde_json::to_value(&address).unwrap(),
                serde_json::Value::String(case.canonical.clone()),
                "{}",
                case.case
            );

            saw_configuration_root |= case.case == "configuration-root";
            saw_metadata_branch |= case.case == "metadata-branch-russian-alias";
            saw_branch_ending_kind |= address
                .segments()
                .last()
                .is_some_and(|segment| segment.name().is_none());
            saw_arbitrary_depth |= address.segments().len() >= 5;
            assert!(
                !case.route.is_empty(),
                "{} must name its reader route",
                case.case
            );
        }

        assert!(saw_configuration_root);
        assert!(saw_metadata_branch);
        assert!(saw_branch_ending_kind);
        assert!(saw_arbitrary_depth);
    }

    #[test]
    fn qualified_addresses_reject_unqualified_malformed_and_noncanonical_roots() {
        for case in fixture().invalid_addresses {
            let error = QualifiedAddress::parse(&case.input)
                .unwrap_err_or_else(|| panic!("{} unexpectedly parsed", case.case));
            assert_eq!(error.code(), case.error, "{}: {error}", case.case);
        }
    }

    trait UnwrapErrOrElse<T, E> {
        fn unwrap_err_or_else(self, on_ok: impl FnOnce() -> E) -> E;
    }

    impl<T, E> UnwrapErrOrElse<T, E> for Result<T, E> {
        fn unwrap_err_or_else(self, on_ok: impl FnOnce() -> E) -> E {
            match self {
                Ok(_) => on_ok(),
                Err(error) => error,
            }
        }
    }

    #[test]
    fn metadata_aliases_reuse_v12_evidence_while_structural_aliases_stay_separate() {
        let legacy =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Документ.Заказ").unwrap();
        let qualified = QualifiedAddress::parse("main:Документ.Заказ").unwrap();
        assert_eq!(qualified.logical_path(), legacy.as_str());
        assert_eq!(qualified.to_string(), "main:Document.Заказ");

        assert_eq!(NodeKind::parse("Реквизит").unwrap(), NodeKind::Attribute);
        assert!(MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Реквизит.Заказ").is_err());
        assert!(NodeKind::parse("Документы").is_err());
    }

    #[test]
    fn unqualified_input_resolves_only_with_one_source_set_and_stays_qualified() {
        assert!(QualifiedAddress::parse("Document.Заказ").is_err());
        for case in fixture().contextual_inputs {
            let available = case
                .available_source_sets
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            match QualifiedAddress::resolve_input(&case.input, &available) {
                Ok(address) => {
                    assert_eq!(Some(address.to_string()), case.canonical, "{}", case.case)
                }
                Err(error) => assert_eq!(Some(error.code()), case.error, "{}", case.case),
            }
        }
    }

    #[test]
    fn configuration_kind_is_rejected_everywhere_except_the_sole_root() {
        let error = QualifiedAddress::parse("main:Document.Заказ.Configuration").unwrap_err();
        assert_eq!(error.code(), AddressErrorCode::ConfigurationRootOnly);
    }

    #[test]
    fn qualified_logical_address_contract_is_complete() {
        qualified_addresses_are_table_driven_canonical_and_arbitrarily_deep();
        qualified_addresses_reject_unqualified_malformed_and_noncanonical_roots();
        metadata_aliases_reuse_v12_evidence_while_structural_aliases_stay_separate();
        unqualified_input_resolves_only_with_one_source_set_and_stays_qualified();
        configuration_kind_is_rejected_everywhere_except_the_sole_root();
    }
}
