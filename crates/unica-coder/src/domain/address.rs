use crate::domain::source_target::{metadata_address_kind_matches, xml_ncname_is_valid};
use serde::{Deserialize, Serialize, Serializer};
use std::borrow::Cow;
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
    /// Локальное описание формы; содержимое остаётся в корне формы.
    Metadata,
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
    /// Источник данных схемы компоновки: `Template.T.DataSource.<имя>`.
    DataSource,
    /// Вычисляемое поле схемы: `Template.T.CalculatedField.<dataPath>`.
    CalculatedField,
    /// Итог (ресурс) схемы: `Template.T.Total.<dataPath>`.
    Total,
    Setting,
    Area,
    /// Группа командного интерфейса: место, куда платформа кладёт команду.
    /// Не путать с `CommandGroup` — тот объект метаданных, а это ссылка на
    /// группу панели, стандартную либо заведённую таким объектом.
    Group,
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
    /// Ссылка объекта метаданных наружу: владелец, движение, основание,
    /// источник события. Элемент этой ветви адресует чужой объект, а не
    /// потомка.
    Relation,
    /// Характеристика плана видов характеристик: пара источников типов и
    /// значений. Прикладного имени у неё нет, поэтому элементы этой ветви
    /// остаются строками данных.
    Characteristic,
    StandardTabularSection,
    /// Метод, который зовёт этот метод. Элемент этой ветви адресует чужой
    /// метод, а не потомка: граф вызовов показывает наружу.
    Caller,
    /// Метод, которого зовёт этот метод.
    Callee,
    /// Предопределённый элемент объекта: носит имя и идентификатор, поэтому
    /// адресуется — в отличие от характеристики, названной парой источников.
    PredefinedItem,
    /// Операция словаря узла: `<узел>.Can.<op>`. Сегмент терминальный, и
    /// его имя — весь остаток адреса, потому что имя операции само содержит
    /// точку (`query.set`). Узел, чей словарь спрашивают, — адрес до `Can`.
    Can,
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
    spelling(NodeKind::Metadata, &["Метаданные"]),
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
    spelling(NodeKind::DataSource, &["ИсточникДанных"]),
    spelling(NodeKind::CalculatedField, &["ВычисляемоеПоле"]),
    spelling(NodeKind::Total, &["Итог"]),
    spelling(NodeKind::Setting, &["Настройка"]),
    spelling(NodeKind::Area, &["Область"]),
    spelling(NodeKind::Module, &["Модуль"]),
    spelling(NodeKind::Method, &["Метод"]),
    spelling(NodeKind::Body, &["Тело"]),
    spelling(NodeKind::Region, &["ОбластьКода"]),
    spelling(NodeKind::Interface, &["Интерфейс"]),
    spelling(NodeKind::Group, &["Группа"]),
    spelling(NodeKind::Event, &["Событие"]),
    spelling(NodeKind::Compilation, &["Компиляция"]),
    spelling(NodeKind::UrlTemplate, &["ШаблонURL"]),
    spelling(NodeKind::Operation, &["Операция"]),
    spelling(NodeKind::Channel, &["Канал"]),
    spelling(NodeKind::Namespace, &["ПространствоИмен"]),
    spelling(NodeKind::Type, &["Тип"]),
    spelling(NodeKind::Property, &["Свойство"]),
    spelling(NodeKind::Relation, &["Связь"]),
    spelling(NodeKind::Characteristic, &["Характеристика"]),
    spelling(
        NodeKind::StandardTabularSection,
        &["СтандартнаяТабличнаяЧасть"],
    ),
    spelling(NodeKind::Caller, &["Вызывающий"]),
    spelling(NodeKind::Callee, &["Вызываемый"]),
    spelling(NodeKind::PredefinedItem, &["ПредопределённыйЭлемент"]),
    spelling(NodeKind::Can, &[]),
];

/// Сегменты до элемента структуры варианта СКД: `…Setting.V[.Item.I…]` —
/// после именованного `Setting` идут только именованные `Item`.
fn structure_item_route(owners: &[AddressSegment]) -> bool {
    owners
        .iter()
        .rposition(|owner| owner.kind == NodeKind::Setting)
        .is_some_and(|position| {
            owners[position].name.is_some()
                && owners[position + 1..]
                    .iter()
                    .all(|owner| owner.kind == NodeKind::Item && owner.name.is_some())
        })
}

const fn spelling(kind: NodeKind, aliases: &'static [&'static str]) -> V13KindSpelling {
    V13KindSpelling { kind, aliases }
}

impl NodeKind {
    #[cfg(test)]
    pub(crate) const fn metadata_kinds() -> &'static [Self] {
        LEGACY_METADATA_KINDS
    }

    /// Носит ли вид прикладное имя.
    ///
    /// Безымянный вид единственен у своего владельца, поэтому имени у него нет
    /// и занимать под него сегмент нельзя: следующий сегмент — это уже вид.
    /// Терминальный вид без имени — другое: это адрес ветки, и он допустим у
    /// любого вида. См. `arch/rules/mcp/qualified-logical-address.md`.
    pub(crate) const fn takes_name(self) -> bool {
        !matches!(self, Self::Configuration | Self::Interface | Self::Metadata)
    }

    /// Может ли имя узла этого вида в этом месте адреса содержать точку.
    ///
    /// Имена объектов метаданных и их членов — идентификаторы платформы,
    /// точки в них не бывает. Точку несут данные внутри документов, и решает
    /// маршрут, а не один вид: составной `dataPath` поля, параметр и область
    /// внутри макета, вычисляемое поле, итог и элемент структуры варианта
    /// СКД, группа интерфейса `CommandGroup.X`, тип и свойство
    /// пакета XDTO, запись права роли на вложенный объект. Тот же `Parameter`
    /// у формы или операции веб-сервиса — член метаданных, и экранированная
    /// точка в нём — не имя, а склейка нескольких сегментов, которую разбор
    /// отклоняет. `owners` — сегменты адреса до этого.
    pub(crate) fn name_may_contain_dot(self, owners: &[AddressSegment]) -> bool {
        let first = owners.first().map(AddressSegment::kind);
        let previous = owners.last().map(AddressSegment::kind);
        // Тело макета: СКД или табличный документ внутри именованного макета
        // объекта или общего макета.
        let inside_template = first == Some(Self::CommonTemplate)
            || owners
                .iter()
                .any(|owner| owner.kind == Self::Template && owner.name.is_some());
        match self {
            Self::Field | Self::Parameter | Self::Area => inside_template,
            // Путь вычисляемого поля и итога — `dataPath` схемы; имя элемента
            // структуры варианта — строка данных настроек.
            Self::CalculatedField | Self::Total => inside_template,
            Self::Item => inside_template && structure_item_route(owners),
            Self::Group => previous == Some(Self::Interface),
            Self::Type | Self::Property => first == Some(Self::XdtoPackage),
            Self::Right => first == Some(Self::Role) && owners.len() == 1,
            _ => false,
        }
    }

    /// Может ли имя узла этого вида в этом месте адреса быть свободным
    /// текстом: с пробелами, скобками и другими знаками вне грамматики
    /// идентификатора. Такое имя носит только элемент структуры варианта
    /// СКД: платформа хранит его строкой настроек («По периодам»), а не
    /// идентификатором. Точка в нём по-прежнему экранируется `\.` (#1357).
    pub(crate) fn name_is_free_text(self, owners: &[AddressSegment]) -> bool {
        self == Self::Item && self.name_may_contain_dot(owners)
    }

    // Параметр назван `token`, а не `raw`: `&raw` — это спеллинг оператора
    // сырого заимствования, и в позиции выражения он читается как начало
    // `&raw const`/`&raw mut` и человеком, и разборщиками. Один такой случай
    // уже делал этот файл неразбираемым для стража неизменности продукта.
    pub(crate) fn parse(token: &str) -> Result<Self, AddressError> {
        if let Some(kind) = LEGACY_METADATA_KINDS
            .iter()
            .find(|kind| metadata_address_kind_matches(kind.as_str(), token))
        {
            return Ok(*kind);
        }
        V13_KIND_SPELLINGS
            .iter()
            .find(|spelling| spelling.kind.as_str() == token || spelling.aliases.contains(&token))
            .map(|spelling| spelling.kind)
            .ok_or_else(|| {
                AddressError::new(
                    AddressErrorCode::UnknownKind,
                    format!("unknown logical node kind `{token}`"),
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
            Self::Metadata => "Metadata",
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
            Self::DataSource => "DataSource",
            Self::CalculatedField => "CalculatedField",
            Self::Total => "Total",
            Self::Setting => "Setting",
            Self::Area => "Area",
            Self::Group => "Group",
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
            Self::Relation => "Relation",
            Self::Characteristic => "Characteristic",
            Self::StandardTabularSection => "StandardTabularSection",
            Self::Caller => "Caller",
            Self::Callee => "Callee",
            Self::PredefinedItem => "PredefinedItem",
            Self::Can => "Can",
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
    pub(crate) line_range: Option<BodyLineRange>,
}

/// Inclusive absolute file lines; a single-line selector keeps its spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BodyLineRange {
    pub(crate) from: usize,
    pub(crate) to: usize,
    single: bool,
}

impl BodyLineRange {
    fn parse(raw: &str) -> Result<Self, AddressError> {
        let bad = || {
            AddressError::new(AddressErrorCode::InvalidName, "Body line selector must be #L<n> or #L<a>-L<b>, with positive ascending file lines")
        };
        let (from, to, single) = match raw.strip_prefix('L').and_then(|raw| raw.split_once("-L")) {
            Some((from, to)) => (from, to, false),
            None => {
                let from = raw.strip_prefix('L').ok_or_else(bad)?;
                (from, from, true)
            }
        };
        let number = |s: &str| {
            if s.is_empty() || !s.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(bad());
            }
            s.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(bad)
        };
        let (from, to) = (number(from)?, number(to)?);
        if to < from {
            return Err(bad());
        }
        Ok(Self { from, to, single })
    }
}

impl fmt::Display for BodyLineRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.single {
            write!(f, "#L{}", self.from)
        } else {
            write!(f, "#L{}-L{}", self.from, self.to)
        }
    }
}

impl QualifiedAddress {
    /// Resolves user input against the source sets already selected by the
    /// workspace. Identity parsing remains strict; this contextual boundary is
    /// the only place where an omitted source-set prefix is inferred.
    pub(crate) fn resolve_input(
        raw: &str,
        available_source_sets: &[&str],
    ) -> Result<Self, AddressError> {
        // Двоеточие отделяет набор исходников, только если префикс до него —
        // имя набора: без точки или один из доступных. Имя элемента структуры
        // СКД — свободный текст и может содержать двоеточие (#1357).
        let qualified = raw.split_once(':').is_some_and(|(prefix, _)| {
            !prefix.contains('.') || available_source_sets.contains(&prefix)
        });
        if qualified {
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
        // A free-text DCS Item can itself end in #L1. Only a proven nameless
        // Body treats # as a selector; other names retain their old grammar.
        if let Some((owner, selector)) = raw.split_once('#') {
            if let Ok(mut body) = Self::parse(owner) {
                if body
                    .segments
                    .last()
                    .is_some_and(|s| s.kind == NodeKind::Body && s.name.is_none())
                {
                    if body.line_range.is_some() {
                        return Err(AddressError::new(
                            AddressErrorCode::InvalidName,
                            "Body accepts one line selector",
                        ));
                    }
                    if crate::domain::platform_profile::PlatformProfile::v8_3_27()
                        .module_prefix_capability(&body)
                        .is_none()
                    {
                        return Err(AddressError::new(
                            AddressErrorCode::InvalidName,
                            "line selectors address only a BSL module Body or Method Body",
                        ));
                    }
                    body.line_range = Some(BodyLineRange::parse(selector)?);
                    return Ok(body);
                }
            }
        }
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
        // `\.` — буквальная точка имени, остальные точки разделяют сегменты.
        // Вид и имя сравниваются по снятому экранированию; имя операции
        // за `Can` собирается из исходных частей и экранирования не знает.
        let (raw_parts, unescaped) = split_segments(logical_path)?;
        let parts = unescaped.iter().map(String::as_str).collect::<Vec<_>>();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(AddressError::new(
                AddressErrorCode::EmptySegment,
                "qualified logical address contains an empty segment",
            ));
        }

        let mut segments = Vec::with_capacity(parts.len().div_ceil(2));
        let mut index = 0;
        while index < parts.len() {
            let kind = NodeKind::parse(parts[index])?;
            index += 1;
            // Имя операции содержит точку, поэтому `Can` забирает весь
            // остаток адреса и всегда последний.
            if kind == NodeKind::Can {
                let operation = raw_parts[index..].join(".");
                index = parts.len();
                if !operation.is_empty() && !xml_ncname_is_valid(&operation) {
                    return Err(AddressError::new(
                        AddressErrorCode::InvalidName,
                        format!("invalid operation name `{operation}`"),
                    ));
                }
                segments.push(AddressSegment {
                    kind,
                    name: (!operation.is_empty()).then_some(operation),
                });
                continue;
            }
            // Пары вслепую здесь не годятся: безымянный вид забрал бы
            // следующий сегмент себе под имя, и объявленная ветвь перестала бы
            // разрешаться. Имя берётся только у вида, который его носит.
            // Ветвь без имени (`Attribute`, `Body`) тоже спрашивает свой
            // словарь: `Can`, за которым идёт не вид, а имя операции, —
            // это вопрос к ветви, а не её имя. Справочник с именем `Can`
            // (`Catalog.Can`, `Catalog.Can.Attribute`) так и остаётся именем.
            let asks_dictionary = parts.get(index) == Some(&NodeKind::Can.as_str())
                && parts
                    .get(index + 1)
                    .is_some_and(|next| NodeKind::parse(next).is_err());
            let name = if kind.takes_name() && !asks_dictionary {
                let taken = parts.get(index).copied();
                if taken.is_some() {
                    index += 1;
                }
                taken
            } else {
                // Следующий сегмент у безымянного вида обязан быть видом. Если
                // он им не читается, вызывающий написал имя — и сказать об
                // этом надо прямо: «неизвестный вид» отправило бы его искать
                // опечатку в том, что именем и было.
                if let Some(written) = parts.get(index) {
                    if NodeKind::parse(written).is_err() {
                        return Err(AddressError::new(
                            AddressErrorCode::ConfigurationRootOnly,
                            format!(
                                "`{}` has no application name; `{written}` cannot follow it",
                                kind.as_str()
                            ),
                        ));
                    }
                }
                None
            };
            if let Some(name) = name.filter(|name| name.contains('.')) {
                if !kind.name_may_contain_dot(&segments) {
                    return Err(AddressError::new(
                        AddressErrorCode::InvalidName,
                        format!(
                            "a `{}` name has no dot; `{name}` joins several segments",
                            kind.as_str()
                        ),
                    ));
                }
            }
            if name.is_some_and(|name| {
                !xml_ncname_is_valid(name) && !kind.name_is_free_text(&segments)
            }) {
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

        // `main:Can.<op>` — тот же вопрос к корню; канонически он пишется
        // `main:Configuration.Can.<op>`, как псевдоним вида — английским именем.
        if matches!(segments.as_slice(), [can] if can.kind == NodeKind::Can) {
            segments.insert(
                0,
                AddressSegment {
                    kind: NodeKind::Configuration,
                    name: None,
                },
            );
        }
        // Словарь корня спрашивают адресом `Configuration.Can.<op>`: корень
        // остаётся единственным, `Can` — не его потомок, а вопрос к нему.
        let sole_configuration_root = match segments.as_slice() {
            [root] => root.kind == NodeKind::Configuration && root.name.is_none(),
            [root, can] => {
                root.kind == NodeKind::Configuration
                    && root.name.is_none()
                    && can.kind == NodeKind::Can
            }
            _ => false,
        };
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

        let address = Self {
            line_range: None,
            source_set: source_set.to_string(),
            segments,
        };
        let metadata_subject = address
            .operation_query()
            .map_or_else(|| address.clone(), |(owner, _)| owner);
        if metadata_subject
            .segments
            .iter()
            .any(|segment| segment.kind == NodeKind::Metadata)
            && metadata_subject.form_metadata_owner().is_none()
        {
            return Err(AddressError::new(
                AddressErrorCode::InvalidName,
                "Metadata is the terminal nameless descriptor of a named form",
            ));
        }
        Ok(address)
    }

    /// Владелец самостоятельного XML-описания формы, без содержимого и модуля.
    pub(crate) fn form_metadata_owner(&self) -> Option<Self> {
        let valid = match self.segments.as_slice() {
            [form, metadata] => {
                form.kind == NodeKind::CommonForm
                    && form.name.is_some()
                    && metadata.kind == NodeKind::Metadata
                    && metadata.name.is_none()
            }
            [owner, form, metadata] => {
                owner.kind.is_metadata_kind()
                    && owner.name.is_some()
                    && form.kind == NodeKind::Form
                    && form.name.is_some()
                    && metadata.kind == NodeKind::Metadata
                    && metadata.name.is_none()
            }
            _ => false,
        };
        valid.then(|| self.prefix(self.segments.len() - 1))
    }

    pub(crate) fn source_set(&self) -> &str {
        &self.source_set
    }

    pub(crate) fn segments(&self) -> &[AddressSegment] {
        &self.segments
    }

    /// Адрес из первых `len` сегментов: владелец узла по его адресу.
    pub(crate) fn prefix(&self, len: usize) -> Self {
        Self {
            line_range: None,
            source_set: self.source_set.clone(),
            segments: self.segments[..len.min(self.segments.len())].to_vec(),
        }
    }

    pub(crate) fn logical_path(&self) -> String {
        let mut rendered = render_segments(&self.segments);
        if let Some(range) = self.line_range {
            rendered.push_str(&range.to_string());
        }
        rendered
    }

    pub(crate) fn line_range(&self) -> Option<BodyLineRange> {
        self.line_range
    }

    pub(crate) fn without_line_range(&self) -> Self {
        self.prefix(self.segments.len())
    }

    /// Член коллекции `self` с именем `name`: адрес ветви и имя, где точка
    /// имени экранирована. Ошибка — имя не годится в адрес.
    pub(crate) fn member(&self, name: &str) -> Result<Self, AddressError> {
        Self::parse(&member_address(self, name))
    }

    /// Вопрос к словарю операций: узел, чей словарь спрашивают, и имя
    /// операции (`None` у голого `<узел>.Can`). `None` — адрес не про словарь.
    pub(crate) fn operation_query(&self) -> Option<(Self, Option<&str>)> {
        let (last, owner) = self.segments.split_last()?;
        if last.kind != NodeKind::Can {
            return None;
        }
        Some((
            Self {
                line_range: None,
                source_set: self.source_set.clone(),
                segments: owner.to_vec(),
            },
            last.name(),
        ))
    }
}

/// Делит логический путь на сегменты по неэкранированной точке.
///
/// Возвращает исходные части (для имени операции за `Can`) и части со
/// снятым экранированием. `\.` — буквальная точка имени; другой символ
/// после `\` и `\` в конце отклоняются: имя — XML NCName, обратной косой
/// черты в нём не бывает, и второго вида экранирования грамматика не знает.
fn split_segments(logical_path: &str) -> Result<(Vec<&str>, Vec<String>), AddressError> {
    let mut raw = Vec::new();
    let mut unescaped = Vec::new();
    let mut start = 0;
    let mut current = String::new();
    let mut characters = logical_path.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some((_, '.')) => current.push('.'),
                _ => {
                    return Err(AddressError::new(
                        AddressErrorCode::InvalidName,
                        "`\\` in a logical address escapes only a literal dot of a name: `\\.`",
                    ))
                }
            },
            '.' => {
                raw.push(&logical_path[start..index]);
                unescaped.push(std::mem::take(&mut current));
                start = index + 1;
            }
            other => current.push(other),
        }
    }
    raw.push(&logical_path[start..]);
    unescaped.push(current);
    Ok((raw, unescaped))
}

/// Имя сегмента в записи адреса: буквальная точка экранируется `\.`.
///
/// Единственное место, где имя превращается в текст адреса: и печать
/// `QualifiedAddress`, и адреса, которые собираются строкой от уже
/// напечатанного адреса, проходят здесь. Имя без точки не меняется.
pub(crate) fn escape_name(name: &str) -> Cow<'_, str> {
    if name.contains('.') {
        Cow::Owned(name.replace('.', "\\."))
    } else {
        Cow::Borrowed(name)
    }
}

/// Адрес члена коллекции: напечатанный адрес ветви и имя члена.
pub(crate) fn member_address(branch: &impl fmt::Display, name: &str) -> String {
    format!("{branch}.{}", escape_name(name))
}

fn render_segments(segments: &[AddressSegment]) -> String {
    let mut rendered = Vec::with_capacity(segments.len() * 2);
    for segment in segments {
        rendered.push(Cow::Borrowed(segment.kind.as_str()));
        if let Some(name) = segment.name() {
            // Имя операции за `Can` — остаток адреса со своими точками.
            rendered.push(if segment.kind == NodeKind::Can {
                Cow::Borrowed(name)
            } else {
                escape_name(name)
            });
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

    /// Ни одна из двух форм не годится, и обе обязаны отказывать — но по
    /// разным причинам, и причину надо называть.
    ///
    /// `Configuration.Interface` неверен по смыслу: `Configuration` — узел, а
    /// не приставка пути, дети конфигурации адресуются от набора напрямую.
    /// `Interface.main` неверен по форме: имени у интерфейса не бывает, и
    /// `main` тут — выдуманный заполнитель. До правила безымянных видов вторая
    /// форма молча разбиралась в «интерфейс по имени main», то есть негодный
    /// адрес принимался за годный.
    #[test]
    fn neither_a_configuration_prefix_nor_an_invented_interface_name_is_accepted() {
        let prefixed = QualifiedAddress::parse("main:Configuration.Interface")
            .expect_err("приставка пути не годится");
        assert_eq!(prefixed.code(), AddressErrorCode::ConfigurationRootOnly);

        let invented = QualifiedAddress::parse("main:Subsystem.Sales.Interface.main")
            .expect_err("имени у интерфейса не бывает");
        assert!(
            invented.to_string().contains("has no application name"),
            "отказ обязан назвать причину, а не звать искать опечатку в имени: {invented}"
        );

        // А законная форма — вид сразу за безымянным видом — проходит.
        QualifiedAddress::parse("main:Subsystem.Sales.Interface.Command")
            .expect("вид за безымянным видом законен");
    }

    /// Безымянный вид единственен у владельца, поэтому следующий сегмент —
    /// это вид, а не его имя. Пока разбор шёл парами вслепую, `Interface`
    /// забирал `Command` себе под имя, и объявленная узлом ветвь
    /// `...Interface.Command` разрешалась обратно в тот же узел: счёт называл
    /// команды, спуск по названному адресу их не показывал.
    #[test]
    fn nameless_kinds_do_not_consume_the_next_segment_as_a_name() {
        let at = QualifiedAddress::parse("main:Subsystem.Администрирование.Interface.Command")
            .expect("адрес разбирается");
        let shape: Vec<(&str, Option<&str>)> = at
            .segments()
            .iter()
            .map(|segment| (segment.kind().as_str(), segment.name()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("Subsystem", Some("Администрирование")),
                ("Interface", None),
                ("Command", None),
            ],
            "`Command` — это вид коллекции команд, а не имя интерфейса"
        );

        let root = QualifiedAddress::parse("main:Interface").expect("корневой интерфейс");
        assert_eq!(root.segments().len(), 1);
        assert_eq!(root.segments()[0].name(), None);
    }

    /// Обратная сторона того же правила: вид, который имя носит, забирает
    /// следующий сегмент как прежде. Иначе правка сломала бы каждый обычный
    /// адрес, а терминальная ветка перестала бы быть веткой.
    #[test]
    fn a_named_kind_still_takes_the_segment_that_follows_it() {
        let deep = QualifiedAddress::parse("main:Catalog.Валюты.Module.Object")
            .expect("адрес разбирается");
        let shape: Vec<(&str, Option<&str>)> = deep
            .segments()
            .iter()
            .map(|segment| (segment.kind().as_str(), segment.name()))
            .collect();
        assert_eq!(
            shape,
            vec![("Catalog", Some("Валюты")), ("Module", Some("Object"))]
        );

        let branch = QualifiedAddress::parse("main:Catalog.Валюты.Attribute").expect("адрес ветки");
        assert_eq!(
            branch
                .segments()
                .last()
                .map(|segment| (segment.kind().as_str(), segment.name())),
            Some(("Attribute", None)),
            "терминальный вид без имени — это адрес ветки, и он остаётся допустимым"
        );
    }

    /// `<узел>.Can.<op>` спрашивает контракт одной операции. Имя операции
    /// само содержит точку, поэтому `Can` забирает весь остаток адреса, а
    /// узел, чей словарь спрашивают, — всё, что до него.
    #[test]
    fn a_can_segment_takes_the_rest_of_the_address_as_the_operation_name() {
        let at = QualifiedAddress::parse("main:Catalog.Валюты.Can.query.set").unwrap();
        assert_eq!(at.to_string(), "main:Catalog.Валюты.Can.query.set");
        let last = at.segments().last().unwrap();
        assert_eq!(
            (last.kind(), last.name()),
            (NodeKind::Can, Some("query.set"))
        );
        let (owner, op) = at
            .operation_query()
            .expect("a Can address asks the dictionary");
        assert_eq!(owner.to_string(), "main:Catalog.Валюты");
        assert_eq!(op, Some("query.set"));

        let deep = QualifiedAddress::parse(
            "cf:Report.F05Report.Template.F05Schema.DataSet.F05Data.Can.field.add",
        )
        .unwrap();
        let (owner, op) = deep.operation_query().unwrap();
        assert_eq!(
            owner.to_string(),
            "cf:Report.F05Report.Template.F05Schema.DataSet.F05Data"
        );
        assert_eq!(op, Some("field.add"));

        // Корень остаётся единственным: `Can` — вопрос к нему, а не потомок.
        for root in [
            "main:Configuration.Can.object.create",
            "main:Can.object.create",
        ] {
            let parsed =
                QualifiedAddress::parse(root).unwrap_or_else(|error| panic!("{root}: {error}"));
            let (owner, op) = parsed.operation_query().unwrap();
            assert_eq!(owner.to_string(), "main:Configuration", "{root}");
            assert_eq!(op, Some("object.create"), "{root}");
        }
        assert_eq!(
            QualifiedAddress::parse("main:Configuration.Catalog.Валюты")
                .unwrap_err()
                .code(),
            AddressErrorCode::ConfigurationRootOnly
        );

        assert_eq!(
            QualifiedAddress::parse("main:Can.object.create")
                .unwrap()
                .to_string(),
            "main:Configuration.Can.object.create",
            "one question has one canonical spelling"
        );

        // Ветвь без имени тоже спрашивает словарь: `Can` с именем операции
        // за ним — не имя ветви.
        for (at, owner, op) in [
            (
                "main:Catalog.Валюты.Module.Object.Body.Can.code.insert",
                "main:Catalog.Валюты.Module.Object.Body",
                "code.insert",
            ),
            (
                "main:Catalog.Валюты.Attribute.Can.attribute.add",
                "main:Catalog.Валюты.Attribute",
                "attribute.add",
            ),
        ] {
            let parsed =
                QualifiedAddress::parse(at).unwrap_or_else(|error| panic!("{at}: {error}"));
            let (parsed_owner, parsed_op) = parsed.operation_query().expect(at);
            assert_eq!(parsed_owner.to_string(), owner);
            assert_eq!(parsed_op, Some(op));
        }
        // А имя `Can`, за которым идёт вид, остаётся именем.
        let attribute = QualifiedAddress::parse("main:Catalog.Can.Attribute.Код").unwrap();
        assert!(attribute.operation_query().is_none());
        assert_eq!(attribute.segments()[0].name(), Some("Can"));

        let bare = QualifiedAddress::parse("main:Catalog.Валюты.Can").unwrap();
        assert_eq!(bare.operation_query().unwrap().1, None);

        // На месте имени `Can` остаётся именем: справочник с таким именем.
        let named = QualifiedAddress::parse("main:Catalog.Can").unwrap();
        assert!(named.operation_query().is_none());
        assert!(QualifiedAddress::parse("main:Catalog.Валюты")
            .unwrap()
            .operation_query()
            .is_none());

        assert_eq!(
            QualifiedAddress::parse("main:Catalog.Валюты.Can.1query")
                .unwrap_err()
                .code(),
            AddressErrorCode::InvalidName
        );
        assert_eq!(
            QualifiedAddress::parse("main:Catalog.Валюты.Can.query..set")
                .unwrap_err()
                .code(),
            AddressErrorCode::EmptySegment
        );
    }

    /// Точку в имени разрешает маршрут, а не вид сегмента: `Parameter`
    /// макета и области несёт точку, `Parameter` формы и операции
    /// веб-сервиса — член метаданных и её не несёт; то же для поля, области,
    /// группы, типа, свойства и записи права вне своего документа.
    #[test]
    fn an_escaped_dot_is_a_name_only_on_document_routes() {
        for accepted in [
            r"main:Report.R.Template.S.DataSet.D.Field.A\.B",
            r"main:Report.R.Template.S.Parameter.A\.B",
            r"main:CommonTemplate.S.Parameter.A\.B",
            r"main:CommonTemplate.S.DataSet.D.Field.A\.B",
            r"main:Report.R.Template.P.Area.A\.B",
            r"main:Report.R.Template.P.Area.A\.B.Parameter.C\.D",
            r"main:Report.R.Template.S.CalculatedField.A\.B",
            r"main:Report.R.Template.S.Total.A\.B",
            r"main:Report.R.Template.S.Setting.V.Item.A\.B.Item.C\.D",
            r"main:Report.R.Template.S.Setting.V.Item.По периодам (итог).Item.1 группа",
            r"main:CommonTemplate.S.Setting.V.Item.По периодам",
            r"main:Subsystem.S.Interface.Group.CommandGroup\.X",
            r"main:XDTOPackage.P.Type.T\.U.Property.A\.B.Property.C\.D",
            r"main:XDTOPackage.P.Property.A\.B",
            r"main:Role.R.Right.Catalog_X\.Command\.Y",
        ] {
            let at = QualifiedAddress::parse(accepted)
                .unwrap_or_else(|error| panic!("{accepted}: {error:?}"));
            assert_eq!(at.to_string(), accepted);
        }
        for rejected in [
            r"main:WebService.Exchange.Operation.Send.Parameter.A\.B",
            r"main:Catalog.Items.Form.Main.Parameter.Key\.X",
            r"main:CommonForm.Main.Parameter.Key\.X",
            r"main:Catalog.Items.Form.Main.Group.G\.H",
            r"main:Report.R.Template.S.DataSource.A\.B",
            r"main:Report.R.Template.S.Item.A\.B",
            r"main:Report.R.Template.S.Setting.По периодам",
            r"main:Report.R.Template.S.Setting.V.Property.P.Item.A B",
            r"main:Report.R.Template.S.DataSet.D.Field.A B",
            r"main:Catalog.Items.Form.Main.Item.A B",
            r"main:Catalog.Items.Template.T\.U",
            r"main:Catalog.Items.Field.A\.B",
            r"main:Catalog.Items.Area.A\.B",
            r"main:Catalog.Items.Type.A\.B",
            r"main:Catalog.Items.Property.A\.B",
            r"main:Catalog.Items.Right.A\.B",
            r"main:Subsystem.S.Group.A\.B",
            r"main:XDTOPackage.P\.Q.Type.T",
        ] {
            assert_eq!(
                QualifiedAddress::parse(rejected)
                    .map(|at| at.to_string())
                    .unwrap_err()
                    .code(),
                AddressErrorCode::InvalidName,
                "{rejected}"
            );
        }
    }

    /// Точка внутри имени экранируется `\.`: составной `dataPath` поля
    /// схемы, имя группы `CommandGroup.X`, свойство XDTO с точкой — один
    /// сегмент, а не переход к виду. Печать возвращает то же написание, и
    /// разбор напечатанного адреса даёт тот же адрес; имена без точки
    /// пишутся как прежде.
    #[test]
    fn an_escaped_dot_stays_inside_one_name_and_round_trips() {
        let raw = r"main:Report.R.Template.S.DataSet.D.Field.Контрагент\.ИНН";
        let at = QualifiedAddress::parse(raw).unwrap();
        let last = at.segments().last().unwrap();
        assert_eq!(
            (last.kind(), last.name()),
            (NodeKind::Field, Some("Контрагент.ИНН"))
        );
        assert_eq!(at.segments().len(), 4);
        assert_eq!(at.to_string(), raw);
        assert_eq!(QualifiedAddress::parse(&at.to_string()).unwrap(), at);

        // Строитель члена экранирует имя сам: ветвь и имя дают тот же адрес.
        let branch = QualifiedAddress::parse("main:Subsystem.S.Interface.Group").unwrap();
        let group = branch.member("CommandGroup.Печать").unwrap();
        assert_eq!(
            group.to_string(),
            r"main:Subsystem.S.Interface.Group.CommandGroup\.Печать"
        );
        assert_eq!(
            group.segments().last().unwrap().name(),
            Some("CommandGroup.Печать")
        );
        assert_eq!(super::member_address(&"x", "plain"), "x.plain");

        // Без экранирования та же запись — переход к виду, которого нет.
        assert_eq!(
            QualifiedAddress::parse("main:Report.R.Template.S.DataSet.D.Field.Контрагент.ИНН")
                .unwrap_err()
                .code(),
            AddressErrorCode::UnknownKind
        );
        // Несколько точек и точка в конце имени.
        let deep = QualifiedAddress::parse(r"main:Role.R.Right.Catalog_X\.Command\.Y\.").unwrap();
        assert_eq!(deep.segments()[1].name(), Some("Catalog_X.Command.Y."));
        assert_eq!(
            deep.to_string(),
            r"main:Role.R.Right.Catalog_X\.Command\.Y\."
        );

        // Имя объекта метаданных и его члена точки не несёт: экранированная
        // точка там склеивает сегменты, а не входит в имя.
        for glued in [
            r"main:Catalog.Валюты\.Form\.ФормаЭлемента",
            r"main:Catalog.Валюты.Attribute.Код\.X",
            r"main:Catalog.Валюты.Form.Ф\.Э",
        ] {
            assert_eq!(
                QualifiedAddress::parse(glued).unwrap_err().code(),
                AddressErrorCode::InvalidName,
                "{glued}"
            );
        }

        // Другого экранирования нет, и пустое имя остаётся пустым сегментом.
        for invalid in [
            r"main:Catalog.A\B",
            r"main:Catalog.A\",
            r"main:Catalog.A\\.B",
        ] {
            assert_eq!(
                QualifiedAddress::parse(invalid).unwrap_err().code(),
                AddressErrorCode::InvalidName,
                "{invalid}"
            );
        }
        assert_eq!(
            QualifiedAddress::parse(r"main:Catalog.\.A")
                .unwrap_err()
                .code(),
            AddressErrorCode::InvalidName,
            "имя обязано начинаться как XML NCName"
        );

        // Экранированное имя перед `Can` не мешает вопросу к словарю, а
        // имя операции за `Can` печатается без экранирования.
        let question =
            QualifiedAddress::parse(r"main:Report.R.Template.S.DataSet.D.Field.A\.B.Can.field.set")
                .unwrap();
        let (owner, op) = question.operation_query().unwrap();
        assert_eq!(owner.segments().last().unwrap().name(), Some("A.B"));
        assert_eq!(op, Some("field.set"));
        assert_eq!(
            question.to_string(),
            r"main:Report.R.Template.S.DataSet.D.Field.A\.B.Can.field.set"
        );
    }

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

    /// Имя элемента структуры СКД — свободный текст: двоеточие в нём не
    /// делает неполный адрес квалифицированным, а в полном адресе набор
    /// отделяет только первое двоеточие.
    #[test]
    fn a_colon_in_a_free_text_item_name_is_not_a_source_set_separator() {
        let unqualified = "Report.R.Template.S.Setting.V.Item.Итоги: по периодам";
        let qualified = "main:Report.R.Template.S.Setting.V.Item.Итоги: по периодам";
        assert_eq!(
            QualifiedAddress::resolve_input(unqualified, &["main"])
                .unwrap()
                .to_string(),
            qualified
        );
        assert_eq!(
            QualifiedAddress::resolve_input(qualified, &["main"])
                .unwrap()
                .to_string(),
            qualified
        );
        assert_eq!(
            QualifiedAddress::resolve_input(qualified, &["main", "ext"])
                .unwrap()
                .to_string(),
            qualified
        );
        assert_eq!(
            QualifiedAddress::resolve_input(unqualified, &["main", "ext"])
                .unwrap_err()
                .code(),
            AddressErrorCode::AmbiguousAddress
        );
        // Набор с точкой в имени по-прежнему квалифицирует адрес.
        assert_eq!(
            QualifiedAddress::resolve_input("ext.v2:Catalog.Items", &["main", "ext.v2"])
                .unwrap()
                .to_string(),
            "ext.v2:Catalog.Items"
        );
    }

    #[test]
    fn configuration_kind_is_rejected_everywhere_except_the_sole_root() {
        let error = QualifiedAddress::parse("main:Document.Заказ.Configuration").unwrap_err();
        assert_eq!(error.code(), AddressErrorCode::ConfigurationRootOnly);
    }
}

#[cfg(test)]
mod form_metadata_addresses {
    use super::QualifiedAddress;
    #[test]
    fn metadata_is_a_terminal_nameless_form_descriptor_and_keeps_operation_queries() {
        for (raw, owner) in [
            ("main:CommonForm.Main.Metadata", "main:CommonForm.Main"),
            (
                "main:Document.Order.Form.Main.Metadata",
                "main:Document.Order.Form.Main",
            ),
            (
                "main:ExternalDataProcessor.Report.Form.Main.Metadata",
                "main:ExternalDataProcessor.Report.Form.Main",
            ),
            (
                "main:ОбщаяФорма.Главная.Метаданные",
                "main:CommonForm.Главная",
            ),
        ] {
            let at = QualifiedAddress::parse(raw).unwrap();
            assert_eq!(at.form_metadata_owner().unwrap().to_string(), owner);
            assert!(at.segments().last().unwrap().name().is_none());
            let query = QualifiedAddress::parse(&format!("{at}.Can.form.set")).unwrap();
            assert_eq!(query.operation_query().unwrap().0, at);
        }
        for raw in [
            "main:Metadata",
            "main:Document.Order.Metadata",
            "main:CommonForm.Main.Metadata.Fake",
            "main:CommonForm.Main.Metadata.Item.A",
            "main:CommonForm.Main.Item.A.Metadata",
            "main:CommonForm.Main.Metadata.Module.Form",
            "main:CommonForm.Main.Metadata.Metadata",
        ] {
            assert!(QualifiedAddress::parse(raw).is_err(), "{raw}");
        }
    }
}

#[cfg(test)]
mod body_line_addresses {
    use super::{NodeKind, QualifiedAddress};

    #[test]
    fn body_line_suffix_keeps_the_bsl_node_and_round_trips() {
        for raw in [
            "main:CommonModule.Probe.Body#L1",
            "main:CommonModule.Probe.Body#L2-L5",
            "main:Catalog.Items.Module.Object.Method.Ping.Body#L4-L5",
            "ext:Catalog.Items.Form.Main.Module.Form.Body#L12",
            "main:ExternalDataProcessor.Import.Form.Main.Module.Form.Body#L12-L15",
        ] {
            let at = QualifiedAddress::parse(raw).unwrap_or_else(|error| panic!("{raw}: {error}"));
            assert_eq!(at.to_string(), raw);
            let last = at.segments().last().unwrap();
            assert_eq!(last.kind(), NodeKind::Body);
            assert!(
                last.name().is_none(),
                "the line selector is not an application name"
            );
            assert_eq!(QualifiedAddress::parse(&at.to_string()).unwrap(), at);
        }
    }

    #[test]
    fn body_line_suffix_rejects_malformed_ranges_and_other_nodes() {
        for raw in [
            "main:CommonModule.Probe.Body#L0",
            "main:CommonModule.Probe.Body#L5-L3",
            "main:CommonModule.Probe.Body#L",
            "main:CommonModule.Probe.Body#L1-2",
            "main:CommonModule.Probe.Body#L-1",
            "main:CommonModule.Probe.Body#L+1",
            "main:CommonModule.Probe.Body#L1#L2",
            "main:CommonModule.Probe.Body#L1-L2.Method.Ping",
            "main:CommonModule.Probe.Body#L184467440737095516160",
            "main:CommonModule.Probe#L1",
            "main:CommonModule.Probe.Method.Ping#L1",
            "main:CommonModule.Probe.Body.Can.code.replace#L1",
            "main:CommonModule.Probe.Body#L1.Can.code.replace",
            "main:Catalog.Items.Attribute.Code#L1",
        ] {
            assert!(QualifiedAddress::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn a_hash_inside_a_free_text_dcs_item_is_not_a_body_line_selector() {
        for raw in [
            "main:Report.R.Template.S.Setting.V.Item.Итоги #1",
            "main:Report.R.Template.S.Setting.V.Item.Body#L1",
            "main:Report.R.Template.S.Setting.V.Item.Группа#L1-L2",
        ] {
            let at = QualifiedAddress::parse(raw).unwrap();
            assert_eq!(at.to_string(), raw);
            assert_eq!(at.segments().last().unwrap().kind(), NodeKind::Item);
        }
    }
}
