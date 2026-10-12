use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::logical_route::{classify_route, RouteClass};
use crate::domain::metadata::{metadata_kind_collections, MetaCollection, MetadataKind};
use crate::domain::node_view::OperationRef;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::domain::refusal::RefusalCode;
use serde_json::{Map, Value};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApplyRequest {
    at: QualifiedAddress,
    ops: NonEmptyVec<ApplyOp>,
    dry_run: bool,
    if_rev: Option<String>,
}

impl ApplyRequest {
    pub(crate) fn parse(
        input: &Map<String, Value>,
        available_source_sets: &[&str],
    ) -> Result<Self, ApplyValidationError> {
        for key in input.keys() {
            if !["at", "ops", "dryRun", "ifRev"].contains(&key.as_str()) {
                return Err(ApplyValidationError::bad_value(
                    key,
                    format!("unknown apply argument `{key}`"),
                ));
            }
        }
        let at = parse_address(input.get("at"), "at", available_source_sets)?;
        let raw_ops = input
            .get("ops")
            .and_then(Value::as_array)
            .ok_or_else(|| ApplyValidationError::bad_value("ops", "ops must be an array"))?;
        if raw_ops.is_empty() {
            return Err(ApplyValidationError::bad_value(
                "ops",
                "ops must contain at least one operation",
            ));
        }

        let mut operations = Vec::with_capacity(raw_ops.len());
        let mut created_objects = Vec::<QualifiedAddress>::new();
        for (index, raw_operation) in raw_ops.iter().enumerate() {
            let operation_location = format!("ops[{index}]");
            let object = raw_operation.as_object().ok_or_else(|| {
                ApplyValidationError::bad_value(&operation_location, "operation must be an object")
            })?;
            for key in object.keys() {
                if !["op", "args"].contains(&key.as_str()) {
                    return Err(ApplyValidationError::bad_value(
                        format!("{operation_location}.{key}"),
                        format!("unknown operation member `{key}`"),
                    ));
                }
            }
            let op_location = format!("{operation_location}.op");
            let name = object
                .get("op")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ApplyValidationError::bad_value(&op_location, "op must be non-empty text")
                })?
                .to_string();
            let args_location = format!("{operation_location}.args");
            let mut args = match object.get("args") {
                None => Map::new(),
                Some(Value::Object(args)) => args.clone(),
                Some(_) => {
                    return Err(ApplyValidationError::bad_value(
                        &args_location,
                        "args must be an object",
                    ))
                }
            };
            let target_location = format!("{args_location}.at");
            let operation_at = match args.get("at") {
                None => at.clone(),
                Some(value) => parse_address(Some(value), &target_location, available_source_sets)?,
            };
            let within = |root: &QualifiedAddress| {
                operation_at.source_set() == root.source_set()
                    && operation_at.segments().starts_with(root.segments())
            };
            if !within(&at) && !created_objects.iter().any(within) {
                return Err(ApplyValidationError::bad_value(
                    &target_location,
                    "operation target must be the same logical node or its descendant in the top-level source set",
                ));
            }
            if name == "object.borrow" {
                let location = format!("{args_location}.from");
                let raw = args.get("from").and_then(Value::as_str).ok_or_else(|| {
                    ApplyValidationError::bad_value(
                        &location,
                        "from must be an explicitly qualified parent object address",
                    )
                })?;
                let parent = QualifiedAddress::parse(raw).map_err(|error| {
                    ApplyValidationError::bad_value(&location, error.to_string())
                })?;
                if !available_source_sets.contains(&parent.source_set())
                    || parent.source_set() == at.source_set()
                    || parent.segments().len() != 1
                    || parent.segments()[0].name().is_none()
                {
                    return Err(ApplyValidationError::bad_value(&location, "from must name a top-level object in a distinct admitted parent source set"));
                }
            }
            // Configuration owns newly created metadata although its address is
            // not their textual prefix. Only earlier declarations extend this
            // request's scope; the staged planner still proves creation itself.
            // Роль и подсистема создаются той же операцией корня: их имя тоже
            // в `values.name`, а вид задан самой операцией.
            let created_kind = match name.as_str() {
                "role.create" => Some("Role"),
                "subsystem.create" => Some("Subsystem"),
                _ => None,
            };
            if (name == "object.create" || created_kind.is_some())
                && matches!(operation_at.segments(), [root] if root.kind() == NodeKind::Configuration)
            {
                let created = args.get("values").and_then(|value| {
                    let kind = match created_kind {
                        Some(kind) => kind,
                        None => value.get("kind")?.as_str()?,
                    };
                    if created_kind.is_none() {
                        MetadataKind::parse(kind).ok()?;
                    }
                    let object_name = value.get("name")?.as_str()?;
                    if !crate::domain::metadata::metadata_identifier_is_valid(object_name) {
                        return None;
                    }
                    QualifiedAddress::parse(&format!(
                        "{}:{kind}.{object_name}",
                        operation_at.source_set()
                    ))
                    .ok()
                });
                if let Some(created) = created {
                    created_objects.push(created);
                }
            }
            args.insert("at".to_string(), Value::String(operation_at.to_string()));
            operations.push(ApplyOp {
                name,
                args,
                at: operation_at,
            });
        }

        let dry_run = match input.get("dryRun") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(ApplyValidationError::bad_value(
                    "dryRun",
                    "dryRun must be a boolean",
                ))
            }
        };
        let if_rev = match input.get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(ApplyValidationError::bad_value(
                    "ifRev",
                    "ifRev must be non-empty text",
                ))
            }
        };
        // Забор ревизии обязателен при применении.
        //
        // Без него два плана, построенных на одной ревизии, публикуются оба, и
        // второй молча уничтожает правку первого: замер показал потерю записи
        // без единой диагностики. Предпросмотр при этом забора не требует —
        // ему нечего защищать, — и словарь `run` держит ровно это правило с
        // самого начала; у `apply` оно было только в прозе скиллов.
        if !dry_run && if_rev.is_none() {
            return Err(ApplyValidationError::bad_value(
                "ifRev",
                "apply requires ifRev from a prior dryRun preview",
            ));
        }
        Ok(Self {
            at,
            ops: NonEmptyVec::new(operations)
                .expect("the request parser rejects an empty operation array"),
            dry_run,
            if_rev,
        })
    }

    pub(crate) fn at(&self) -> &QualifiedAddress {
        &self.at
    }

    pub(crate) fn ops(&self) -> &[ApplyOp] {
        self.ops.as_slice()
    }

    pub(crate) const fn dry_run(&self) -> bool {
        self.dry_run
    }

    pub(crate) fn if_rev(&self) -> Option<&str> {
        self.if_rev.as_deref()
    }
}

fn parse_address(
    value: Option<&Value>,
    location: &str,
    available_source_sets: &[&str],
) -> Result<QualifiedAddress, ApplyValidationError> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| ApplyValidationError::bad_value(location, "logical address must be text"))?;
    QualifiedAddress::resolve_input(raw, available_source_sets)
        .map_err(|error| ApplyValidationError::bad_value(location, error.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApplyOp {
    name: String,
    args: Map<String, Value>,
    at: QualifiedAddress,
}

impl ApplyOp {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn args(&self) -> &Map<String, Value> {
        &self.args
    }

    pub(crate) fn at(&self) -> &QualifiedAddress {
        &self.at
    }
}

#[derive(Debug, Clone, PartialEq)]
struct NonEmptyVec<T>(Vec<T>);

impl<T> NonEmptyVec<T> {
    fn new(values: Vec<T>) -> Option<Self> {
        (!values.is_empty()).then_some(Self(values))
    }

    fn as_slice(&self) -> &[T] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyValidationError {
    code: RefusalCode,
    location: String,
    message: String,
}

impl ApplyValidationError {
    fn bad_value(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: RefusalCode::BadValue,
            location: location.into(),
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> RefusalCode {
        self.code
    }

    pub(crate) fn location(&self) -> &str {
        &self.location
    }
}

impl fmt::Display for ApplyValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.location, self.message)
    }
}

impl std::error::Error for ApplyValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationFamily {
    Metadata,
    Properties,
    Form,
    Role,
    Dcs,
    Mxl,
    Xdto,
    Subsystem,
    /// Командный интерфейс: видимость, размещение и порядок. Предмет не сам
    /// объект, а место, которое подсистема ему отводит.
    Interface,
    Support,
    Code,
    Event,
}

/// Набор исходников как место записи: его вид и точный записываемый профиль
/// платформы, если он есть.
///
/// Единственный владелец признака «набор записываемый»: его спрашивают словарь
/// `can`, предварительная проверка `apply` и разбор семейства планировщиком.
/// Гранулярность — вид набора × семейство операций; сегодня все семейства
/// пишут конфигурацию и расширение с точным профилем Platform XML и не пишут
/// внешние наборы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceSetTraits {
    kind: SourceSetKind,
    platform: Option<PlatformProfile>,
}

impl SourceSetTraits {
    /// Признаки допущенного набора: точный профиль есть только у Platform XML
    /// с известным форматом выгрузки.
    pub(crate) fn of_source(
        kind: SourceSetKind,
        format: SourceFormat,
        profile: SourceProfile,
    ) -> Self {
        let platform = (format == SourceFormat::PlatformXml
            && profile.serialization_format().is_some())
        .then(|| profile.platform_profile())
        .flatten();
        Self { kind, platform }
    }

    /// Признаки набора, который читается логическим читателем: тот существует
    /// только для точного профиля платформы.
    pub(crate) const fn read(kind: SourceSetKind, platform: PlatformProfile) -> Self {
        Self {
            kind,
            platform: Some(platform),
        }
    }

    pub(crate) const fn kind(self) -> SourceSetKind {
        self.kind
    }

    /// Профиль, которым классифицируется адрес.
    pub(crate) fn profile(self) -> PlatformProfile {
        self.platform.unwrap_or_else(PlatformProfile::v8_3_27)
    }

    /// Пишет ли семейство `family` в этот набор.
    pub(crate) fn writes(self, family: OperationFamily) -> bool {
        let platform_xml = self.platform.is_some()
            && matches!(
                self.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            );
        match family {
            OperationFamily::Metadata
            | OperationFamily::Properties
            | OperationFamily::Form
            | OperationFamily::Role
            | OperationFamily::Dcs
            | OperationFamily::Mxl
            | OperationFamily::Xdto
            | OperationFamily::Subsystem
            | OperationFamily::Interface
            | OperationFamily::Support
            | OperationFamily::Code
            | OperationFamily::Event => platform_xml,
        }
    }
}

/// Отказ записи в набор, который семейство не пишет. Один текст для
/// предварительной проверки и разбора планировщиком.
pub(crate) const UNWRITABLE_SOURCE_MESSAGE: &str =
    "admitted source has no writable exact Platform XML profile";

/// Узел, к которому обращена операция: адрес, признаки набора исходников и
/// класс маршрута, выведенный из адреса профилем платформы.
///
/// Применимость — чистая функция этого места: файлы не читаются, поэтому один
/// и тот же ответ получают словарь `can`, предварительная проверка `apply` и
/// планировщик. Состояние узла (заимствование, поддержка, существование)
/// сюда не входит: отказ по состоянию — не «не тот узел».
#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationSite<'a> {
    at: &'a QualifiedAddress,
    source: SourceSetTraits,
    route: RouteClass,
}

impl<'a> OperationSite<'a> {
    /// `None` — адрес не принадлежит ни одному дереву профиля.
    pub(crate) fn new(at: &'a QualifiedAddress, source: SourceSetTraits) -> Option<Self> {
        Some(Self {
            at,
            source,
            route: classify_route(at, source.profile())?,
        })
    }

    pub(crate) const fn source(&self) -> SourceSetTraits {
        self.source
    }

    pub(crate) const fn route(&self) -> RouteClass {
        self.route
    }

    pub(crate) fn at(&self) -> &QualifiedAddress {
        self.at
    }

    fn segments(&self) -> &[AddressSegment] {
        self.at.segments()
    }

    fn terminal(&self) -> &AddressSegment {
        self.segments()
            .last()
            .expect("a parsed logical address has a terminal segment")
    }

    /// Объект метаданных верхнего уровня одного из типизированных видов:
    /// `Catalog.X`, `CommonModule.X`. Маршрут не важен — общий модуль читается
    /// как модуль, но остаётся объектом метаданных.
    fn metadata_object(&self) -> Option<MetadataKind> {
        match self.segments() {
            [owner] if owner.name().is_some() => MetadataKind::parse(owner.kind().as_str()).ok(),
            _ => None,
        }
    }

    fn metadata_owner_with(&self, collection: MetaCollection) -> bool {
        self.metadata_object()
            .is_some_and(|kind| metadata_kind_collections(kind).contains(&collection))
    }

    /// Форма объекта: `Owner.X.Form.F`, где у владельца есть коллекция форм.
    fn owned_form(&self) -> bool {
        matches!(self.segments(), [owner, form]
            if owner.name().is_some()
                && form.kind() == NodeKind::Form
                && form.name().is_some()
                && metadata_collection_matches(owner.kind(), MetaCollection::Forms))
    }

    /// Табличный документ или схема как узел владельца: `Owner.X.Template.T`.
    fn owned_template(&self) -> bool {
        self.metadata_member(NodeKind::Template, MetaCollection::Templates)
    }

    /// Член коллекции объекта метаданных верхнего уровня:
    /// `Owner.X.<kind>.Name`, где у вида владельца есть коллекция `collection`.
    fn metadata_member(&self, kind: NodeKind, collection: MetaCollection) -> bool {
        self.route == RouteClass::Metadata
            && matches!(self.segments(), [owner, member]
                if owner.name().is_some()
                    && member.kind() == kind
                    && member.name().is_some()
                    && metadata_collection_matches(owner.kind(), collection))
    }

    fn dcs_terminal(&self, kinds: &[NodeKind]) -> bool {
        self.route == RouteClass::Dcs
            && kinds.contains(&self.terminal().kind())
            && self.terminal().name().is_some()
    }
}

/// Где операция исполнима. Каждый вариант — форма адреса, которую принимает
/// разбор цели планировщиком, а не только вид последнего сегмента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationApplicability {
    /// Корень набора исходников.
    Configuration,
    /// Корень расширения: заимствование.
    ExtensionRoot,
    /// Объект метаданных верхнего уровня.
    Metadata,
    /// Объект метаданных, роль или подсистема верхнего уровня: свойства.
    MetadataOrRoleOrSubsystem,
    /// Объект метаданных с коллекцией: операция владельца над коллекцией.
    /// Реквизиты добавляются и в табличную часть — по её адресу.
    MetadataAttributeContainer,
    MetadataTabularSections,
    MetadataDimensions,
    MetadataResources,
    MetadataEnumValues,
    MetadataColumns,
    MetadataForms,
    MetadataTemplates,
    MetadataCommands,
    MetadataPredefinedItems,
    /// Сам реквизит объекта или его табличной части.
    MetadataAttribute,
    /// Сам член коллекции владельца: табличная часть, измерение, ресурс,
    /// значение перечисления, графа журнала, команда, макет,
    /// предопределённый элемент.
    MetadataTabularSection,
    MetadataDimension,
    MetadataResource,
    MetadataEnumValue,
    MetadataColumn,
    MetadataCommand,
    MetadataTemplate,
    MetadataPredefinedItem,
    /// Форма объекта.
    Form,
    /// Форма объекта или её элемент.
    FormOrItem,
    /// Роль верхнего уровня.
    Role,
    /// Подсистема любой вложенности.
    Subsystem,
    /// Командный интерфейс подсистемы.
    Interface,
    /// Макет-схема компоновки как узел владельца.
    DcsSchema,
    /// Макет-схема или её набор данных: место нового набора.
    DcsDataContainer,
    /// Набор данных или запрос схемы.
    DcsData,
    /// Вариант настроек схемы.
    DcsSettings,
    /// Параметр схемы.
    DcsParameter,
    /// Поле набора данных.
    DcsField,
    /// Макет-табличный документ как узел владельца.
    Mxl,
    /// Пакет XDTO.
    XdtoPackage,
    /// Тип пакета XDTO.
    XdtoType,
    /// Объект верхнего уровня любого вида: правило поддержки.
    SupportRule,
    /// Модуль или его тело: вставка текста.
    CodeInsert,
    /// Модуль, его тело, метод или тело метода: замена текста.
    Code,
    /// Событие модуля или формы.
    Event,
}

impl OperationApplicability {
    const fn shape(self) -> &'static str {
        match self {
            Self::Configuration => "`Configuration`",
            Self::ExtensionRoot => "`Configuration` of an extension",
            Self::Metadata | Self::SupportRule => "`Kind.Name`",
            Self::MetadataOrRoleOrSubsystem => "`Kind.Name`, `Role.Name` or `Subsystem.Name`",
            Self::MetadataAttributeContainer => {
                "`Kind.Name` of a kind with attributes or `Kind.Name.TabularSection.T`"
            }
            Self::MetadataTabularSections => "`Kind.Name` of a kind with tabular sections",
            Self::MetadataDimensions => "`Kind.Name` of a register with dimensions",
            Self::MetadataResources => "`Kind.Name` of a register with resources",
            Self::MetadataEnumValues => "`Enum.Name`",
            Self::MetadataColumns => "`DocumentJournal.Name`",
            Self::MetadataForms => "`Kind.Name` of a kind with forms",
            Self::MetadataTemplates => "`Kind.Name` of a kind with templates",
            Self::MetadataCommands => "`Kind.Name` of a kind with commands",
            Self::MetadataPredefinedItems => "`Kind.Name` of a kind with predefined items",
            Self::MetadataAttribute => "`Kind.Name[.TabularSection.T].Attribute.A`",
            Self::MetadataTabularSection => "`Kind.Name.TabularSection.T`",
            Self::MetadataDimension => "`Kind.Name.Dimension.D` of a register",
            Self::MetadataResource => "`Kind.Name.Resource.R` of a register",
            Self::MetadataEnumValue => "`Enum.Name.EnumValue.V`",
            Self::MetadataColumn => "`DocumentJournal.Name.Column.C`",
            Self::MetadataCommand => "`Kind.Name.Command.C`",
            Self::MetadataTemplate => "`Kind.Name.Template.T`",
            Self::MetadataPredefinedItem => "`Kind.Name.PredefinedItem.P`",
            Self::Form => "`Kind.Name.Form.F`",
            Self::FormOrItem => "`Kind.Name.Form.F[.Item.I]`",
            Self::Role => "`Role.Name`",
            Self::Subsystem => "`Subsystem.Name[.Subsystem.Child]`",
            Self::Interface => "`Subsystem.Name[.Subsystem.Child].Interface`",
            Self::DcsSchema => "`Kind.Name.Template.T` of a data composition schema",
            Self::DcsDataContainer => "`Kind.Name.Template.T[.DataSet.D]` of a schema",
            Self::DcsData => "`Kind.Name.Template.T.DataSet.D` or `.Query.Q`",
            Self::DcsSettings => "`Kind.Name.Template.T.Setting.S`",
            Self::DcsParameter => "`Kind.Name.Template.T.Parameter.P`",
            Self::DcsField => "`Kind.Name.Template.T.DataSet.D.Field.F`",
            Self::Mxl => "`Kind.Name.Template.T` of a spreadsheet",
            Self::XdtoPackage => "`XDTOPackage.Name`",
            Self::XdtoType => "`XDTOPackage.Name.Type.T`",
            Self::CodeInsert => "a module address or its `Body`",
            Self::Code => "a module address, its `Body`, `Method.M` or `Method.M.Body`",
            Self::Event => "an `Event` of a module or a form",
        }
    }

    fn matches(self, site: &OperationSite<'_>) -> bool {
        let segments = site.segments();
        let terminal = site.terminal();
        match self {
            Self::Configuration => site.route == RouteClass::Configuration,
            Self::ExtensionRoot => {
                site.route == RouteClass::Configuration
                    && site.source.kind() == SourceSetKind::Extension
            }
            Self::Metadata => site.metadata_object().is_some(),
            Self::MetadataOrRoleOrSubsystem => {
                site.metadata_object().is_some()
                    || matches!(segments, [only]
                        if only.name().is_some()
                            && matches!(only.kind(), NodeKind::Role | NodeKind::Subsystem))
            }
            Self::MetadataAttributeContainer => {
                site.metadata_owner_with(MetaCollection::Attributes)
                    || site
                        .metadata_member(NodeKind::TabularSection, MetaCollection::TabularSections)
            }
            Self::MetadataTabularSections => {
                site.metadata_owner_with(MetaCollection::TabularSections)
            }
            Self::MetadataDimensions => site.metadata_owner_with(MetaCollection::Dimensions),
            Self::MetadataResources => site.metadata_owner_with(MetaCollection::Resources),
            Self::MetadataEnumValues => site.metadata_owner_with(MetaCollection::EnumValues),
            Self::MetadataColumns => site.metadata_owner_with(MetaCollection::Columns),
            Self::MetadataForms => site.metadata_owner_with(MetaCollection::Forms),
            Self::MetadataTemplates => site.metadata_owner_with(MetaCollection::Templates),
            Self::MetadataCommands => site.metadata_owner_with(MetaCollection::Commands),
            Self::MetadataPredefinedItems => {
                site.metadata_owner_with(MetaCollection::PredefinedItems)
            }
            Self::MetadataAttribute => {
                site.route == RouteClass::Metadata
                    && terminal.kind() == NodeKind::Attribute
                    && terminal.name().is_some()
                    && match segments {
                        [owner, _] => {
                            owner.name().is_some()
                                && metadata_collection_matches(
                                    owner.kind(),
                                    MetaCollection::Attributes,
                                )
                        }
                        [owner, section, _] => {
                            owner.name().is_some()
                                && section.kind() == NodeKind::TabularSection
                                && section.name().is_some()
                                && metadata_collection_matches(
                                    owner.kind(),
                                    MetaCollection::TabularSections,
                                )
                        }
                        _ => false,
                    }
            }
            Self::MetadataTabularSection => {
                site.metadata_member(NodeKind::TabularSection, MetaCollection::TabularSections)
            }
            Self::MetadataDimension => {
                site.metadata_member(NodeKind::Dimension, MetaCollection::Dimensions)
            }
            Self::MetadataResource => {
                site.metadata_member(NodeKind::Resource, MetaCollection::Resources)
            }
            Self::MetadataEnumValue => {
                site.metadata_member(NodeKind::EnumValue, MetaCollection::EnumValues)
            }
            Self::MetadataColumn => site.metadata_member(NodeKind::Column, MetaCollection::Columns),
            Self::MetadataCommand => {
                site.metadata_member(NodeKind::Command, MetaCollection::Commands)
            }
            Self::MetadataTemplate => site.owned_template(),
            Self::MetadataPredefinedItem => {
                site.metadata_member(NodeKind::PredefinedItem, MetaCollection::PredefinedItems)
            }
            Self::Form => site.route == RouteClass::Form && site.owned_form(),
            Self::FormOrItem => {
                site.route == RouteClass::Form
                    && match segments {
                        [_, _] => site.owned_form(),
                        [owner, form, item] => {
                            item.kind() == NodeKind::Item
                                && item.name().is_some()
                                && form.kind() == NodeKind::Form
                                && form.name().is_some()
                                && owner.name().is_some()
                                && metadata_collection_matches(owner.kind(), MetaCollection::Forms)
                        }
                        _ => false,
                    }
            }
            Self::Role => {
                site.route == RouteClass::Role
                    && matches!(segments, [role] if role.kind() == NodeKind::Role && role.name().is_some())
            }
            Self::Subsystem => {
                site.route == RouteClass::Subsystem
                    && segments.iter().all(|segment| {
                        segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                    })
            }
            Self::Interface => {
                site.route == RouteClass::Interface
                    && terminal.kind() == NodeKind::Interface
                    && segments[..segments.len() - 1].iter().all(|segment| {
                        segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                    })
            }
            Self::DcsSchema | Self::Mxl => site.owned_template(),
            Self::DcsDataContainer => {
                site.owned_template() || site.dcs_terminal(&[NodeKind::DataSet])
            }
            Self::DcsData => site.dcs_terminal(&[NodeKind::DataSet, NodeKind::Query]),
            Self::DcsSettings => site.dcs_terminal(&[NodeKind::Setting]),
            Self::DcsParameter => site.dcs_terminal(&[NodeKind::Parameter]),
            Self::DcsField => site.dcs_terminal(&[NodeKind::Field]),
            Self::XdtoPackage => {
                site.route == RouteClass::Xdto
                    && matches!(segments, [package] if package.name().is_some())
            }
            Self::XdtoType => {
                site.route == RouteClass::Xdto
                    && matches!(segments, [package, ty]
                        if package.name().is_some()
                            && ty.kind() == NodeKind::Type
                            && ty.name().is_some())
            }
            Self::SupportRule => matches!(segments, [owner]
                if owner.name().is_some()
                    && owner.kind().is_metadata_kind()
                    && !matches!(owner.kind(), NodeKind::Form | NodeKind::Template | NodeKind::Command)),
            Self::CodeInsert => {
                site.route == RouteClass::Module
                    && (module_terminal(segments)
                        || (terminal.kind() == NodeKind::Body && !method_body(segments)))
            }
            Self::Code => {
                site.route == RouteClass::Module
                    && (module_terminal(segments)
                        || terminal.kind() == NodeKind::Body
                        || (terminal.kind() == NodeKind::Method && terminal.name().is_some()))
            }
            Self::Event => {
                terminal.kind() == NodeKind::Event
                    && terminal.name().is_some()
                    && match site.route {
                        RouteClass::Form => true,
                        // Событие объекта пишется в модуль платформы, которого
                        // у расширения нет.
                        RouteClass::Module | RouteClass::Metadata => {
                            site.source.kind() == SourceSetKind::Configuration
                        }
                        _ => false,
                    }
            }
        }
    }
}

/// Адрес самого модуля: `CommonModule.X`, `Owner.X.Module.Object`,
/// `Module.Session`.
fn module_terminal(segments: &[AddressSegment]) -> bool {
    match segments.last() {
        Some(last) if last.kind() == NodeKind::Module => last.name().is_some(),
        Some(last) if last.kind() == NodeKind::CommonModule => {
            segments.len() == 1 && last.name().is_some()
        }
        _ => false,
    }
}

fn method_body(segments: &[AddressSegment]) -> bool {
    matches!(segments, [.., method, body]
        if method.kind() == NodeKind::Method && body.kind() == NodeKind::Body)
}

fn metadata_collection_matches(kind: NodeKind, collection: MetaCollection) -> bool {
    MetadataKind::parse(kind.as_str())
        .ok()
        .is_some_and(|kind| metadata_kind_collections(kind).contains(&collection))
}

/// Формат тела макета. Семейства СКД и табличного документа применимы к
/// макету только своего формата; это единственный владелец фильтра —
/// им пользуются и словарь `can`, и планировщик, в том числе на
/// промежуточном состоянии пакета (`template.add` → `mxl.set`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemplateFormat {
    DataCompositionSchema,
    SpreadsheetDocument,
    Other,
}

impl TemplateFormat {
    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "DataCompositionSchema" => Self::DataCompositionSchema,
            "SpreadsheetDocument" => Self::SpreadsheetDocument,
            _ => Self::Other,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DataCompositionSchema => "DataCompositionSchema",
            Self::SpreadsheetDocument => "SpreadsheetDocument",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationDescriptor {
    name: &'static str,
    family: OperationFamily,
    applicability: OperationApplicability,
    skeleton: OperationSkeleton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationSkeleton {
    From,
    Items,
    Values,
    Text,
    Target,
}

impl OperationSkeleton {
    /// The single root argument key the operation expects; the published
    /// `can` dictionary and refusal texts both speak this name.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::Items => "items",
            Self::Values => "values",
            Self::Text => "text",
            Self::Target => "at",
        }
    }

    fn args(self) -> Map<String, Value> {
        let value = match self {
            Self::From => Value::String(String::new()),
            Self::Items => Value::Array(Vec::new()),
            Self::Values => Value::Object(Map::new()),
            Self::Text => Value::String(String::new()),
            Self::Target => Value::String(String::new()),
        };
        Map::from_iter([(self.key().to_string(), value)])
    }
}

/// Apply operations with a proven end-to-end v0.13 planner and publication.
/// The `can` dictionary prints this as `implemented`, mirroring the honesty
/// rule of the Run dictionary: a name in the registry is not support.
pub(crate) const IMPLEMENTED_APPLY_OPERATIONS: &[&str] = &[
    "object.borrow",
    "commandVisibility.set",
    "commandPlacement.set",
    "commandOrder.set",
    "groupOrder.set",
    "subsystemOrder.set",
    "mxl.set",
    "event.implement",
    "form.add",
    "form.set",
    "form.remove",
    "form.create",
    "element.add",
    "element.remove",
    "formAttribute.add",
    "formCommand.add",
    "event.bind",
    "right.set",
    "role.create",
    "subsystem.create",
    "content.add",
    "content.remove",
    "childSubsystem.add",
    "childSubsystem.remove",
    "supportCapability.set",
    "supportRule.set",
    "dataSource.add",
    "dataSource.set",
    "dataSource.remove",
    "dataSet.add",
    "dataSet.set",
    "dataSet.remove",
    "field.add",
    "field.set",
    "field.remove",
    "fieldRole.set",
    "parameter.add",
    "parameter.set",
    "parameter.remove",
    "parameter.rename",
    "parameter.reorder",
    "calculatedField.add",
    "calculatedField.remove",
    "total.add",
    "total.remove",
    "variant.add",
    "variant.set",
    "variant.remove",
    "query.set",
    "query.patch",
    "filter.add",
    "filter.set",
    "filter.remove",
    "filter.clear",
    "selection.add",
    "selection.clear",
    "order.add",
    "order.clear",
    "dataParameter.add",
    "dataParameter.set",
    "outputParameter.set",
    "conditionalAppearance.add",
    "conditionalAppearance.clear",
    "structure.add",
    "structure.set",
    "structure.patch",
    "structure.remove",
    "dataSetLink.add",
    "calculatedField.set",
    "total.set",
    "selection.set",
    "selection.remove",
    "order.set",
    "order.remove",
    "dataParameter.remove",
    "outputParameter.remove",
    "conditionalAppearance.remove",
    "dataSetLink.set",
    "dataSetLink.remove",
    "object.create",
    "object.remove",
    "help.create",
    "props.set",
    "attribute.add",
    "attribute.set",
    "attribute.remove",
    "tabularSection.add",
    "tabularSection.set",
    "tabularSection.remove",
    "dimension.add",
    "dimension.set",
    "dimension.remove",
    "resource.add",
    "resource.set",
    "resource.remove",
    "enumValue.add",
    "enumValue.set",
    "enumValue.remove",
    "column.add",
    "column.set",
    "column.remove",
    "template.add",
    "template.set",
    "template.remove",
    "command.add",
    "command.set",
    "command.remove",
    "predefinedItem.add",
    "predefinedItem.set",
    "predefinedItem.remove",
    "relation.add",
    "relation.replace",
    "relation.remove",
    "code.insert",
    "code.replace",
    "valueType.add",
    "objectType.add",
    "property.add",
    "type.remove",
    "property.remove",
];

impl OperationDescriptor {
    pub(crate) const fn name(self) -> &'static str {
        self.name
    }

    pub(crate) const fn skeleton_key(self) -> &'static str {
        self.skeleton.key()
    }

    pub(crate) const fn family(self) -> OperationFamily {
        self.family
    }

    /// Применима ли операция к месту: вид набора, маршрут и форма адреса.
    /// Формат макета этим ответом не проверяется — см. `admits_template_format`.
    pub(crate) fn applies_at(self, site: &OperationSite<'_>) -> bool {
        site.source.writes(self.family) && self.applicability.matches(site)
    }

    /// Фильтр формата макета. Операции СКД применимы к макету-схеме,
    /// табличного документа — к макету-табличному документу; остальным
    /// формат безразличен. `None` — формат не установлен.
    pub(crate) fn admits_template_format(self, format: Option<TemplateFormat>) -> bool {
        match self.family {
            OperationFamily::Dcs => format == Some(TemplateFormat::DataCompositionSchema),
            OperationFamily::Mxl => format == Some(TemplateFormat::SpreadsheetDocument),
            _ => true,
        }
    }

    /// Применимость к прочитанному узлу: место и, для самого макета, формат
    /// его тела. Этим ответом строится словарь `can`.
    pub(crate) fn applies_to_node(
        self,
        site: &OperationSite<'_>,
        template_format: Option<TemplateFormat>,
    ) -> bool {
        self.applies_at(site)
            && (site.terminal().kind() != NodeKind::Template
                || self.admits_template_format(template_format))
    }

    /// Ближайший узел по цепочке предков `at` (до корня набора включительно),
    /// где операция исполнима. Туда ведёт `next` отказа «не тот узел».
    ///
    /// Для макета-предка учитывается и формат его тела: `template_format`
    /// читает его у вызывающего, домен файлов не читает. Неизвестный формат
    /// макет не принимает — `next` не уводит туда, где планировщик откажет.
    pub(crate) fn nearest_applicable_ancestor(
        self,
        site: &OperationSite<'_>,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    ) -> Option<QualifiedAddress> {
        let segments = site.segments();
        let mut candidates = (1..segments.len())
            .rev()
            .map(|length| QualifiedAddress {
                source_set: site.at.source_set().to_string(),
                segments: segments[..length].to_vec(),
            })
            .collect::<Vec<_>>();
        if site.route != RouteClass::Configuration {
            candidates.push(configuration_root(site.at.source_set()));
        }
        candidates.into_iter().find(|candidate| {
            OperationSite::new(candidate, site.source).is_some_and(|candidate| {
                self.applies_at(&candidate)
                    && (candidate.terminal().kind() != NodeKind::Template
                        || self.admits_template_format(template_format(candidate.at)))
            })
        })
    }

    /// Форма адреса, на котором операция исполнима: подсказка отказа, когда
    /// ближайшего такого узла нет.
    pub(crate) const fn address_shape(self) -> &'static str {
        self.applicability.shape()
    }

    pub(crate) fn argument_contract(self) -> Option<super::operation_contract::OperationContract> {
        match self.family {
            OperationFamily::Dcs => super::operation_contract::OperationContract::dcs(self.name),
            OperationFamily::Mxl => super::operation_contract::OperationContract::mxl(self.name),
            OperationFamily::Metadata if self.name == "template.add" => {
                super::operation_contract::OperationContract::mxl(self.name)
            }
            OperationFamily::Metadata => {
                super::operation_contract::OperationContract::metadata(self.name)
            }
            _ => None,
        }
    }

    /// Контракт на конкретном узле: для членов метаданных поля, допустимость
    /// которых зависит от вида владельца и от табличной части, сужены до
    /// этого узла.
    pub(crate) fn argument_contract_at(
        self,
        site: &OperationSite<'_>,
    ) -> Option<super::operation_contract::OperationContract> {
        if self.family == OperationFamily::Metadata && self.name != "template.add" {
            let owner = site
                .segments()
                .first()
                .and_then(|segment| MetadataKind::parse(segment.kind().as_str()).ok());
            let in_section = site
                .segments()
                .iter()
                .any(|segment| segment.kind() == NodeKind::TabularSection);
            return super::operation_contract::OperationContract::metadata_at(
                self.name,
                owner.map(|owner| super::operation_contract::MetadataSite { owner, in_section }),
            );
        }
        self.argument_contract()
    }

    pub(crate) fn copyable_skeleton(self) -> OperationRef {
        OperationRef::new(self.name, self.skeleton.args())
    }

    fn copyable_skeleton_at(self, at: impl Into<String>) -> OperationRef {
        let mut args = self.skeleton.args();
        args.insert("at".to_string(), Value::String(at.into()));
        OperationRef::new(self.name, args)
    }
}

macro_rules! operation_descriptors {
    ($(($name:literal, $family:ident, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        const OPERATION_DESCRIPTORS: &[OperationDescriptor] = &[
            $(OperationDescriptor {
                name: $name,
                family: OperationFamily::$family,
                applicability: OperationApplicability::$applicability,
                skeleton: OperationSkeleton::$skeleton,
            }),+
        ];
    };
}

operation_descriptors!(
    ("object.borrow", Metadata, ExtensionRoot, From),
    ("object.create", Metadata, Configuration, Values),
    ("object.remove", Metadata, Metadata, Target),
    ("props.set", Properties, MetadataOrRoleOrSubsystem, Values),
    ("relation.add", Metadata, Metadata, Values),
    ("relation.remove", Metadata, Metadata, Values),
    ("relation.replace", Metadata, Metadata, Values),
    ("help.create", Metadata, Metadata, Values),
    ("attribute.add", Metadata, MetadataAttributeContainer, Items),
    ("attribute.set", Metadata, MetadataAttribute, Values),
    ("attribute.remove", Metadata, MetadataAttribute, Target),
    (
        "tabularSection.add",
        Metadata,
        MetadataTabularSections,
        Items
    ),
    (
        "tabularSection.set",
        Metadata,
        MetadataTabularSection,
        Values
    ),
    (
        "tabularSection.remove",
        Metadata,
        MetadataTabularSection,
        Target
    ),
    ("dimension.add", Metadata, MetadataDimensions, Items),
    ("dimension.set", Metadata, MetadataDimension, Values),
    ("dimension.remove", Metadata, MetadataDimension, Target),
    ("resource.add", Metadata, MetadataResources, Items),
    ("resource.set", Metadata, MetadataResource, Values),
    ("resource.remove", Metadata, MetadataResource, Target),
    ("enumValue.add", Metadata, MetadataEnumValues, Items),
    ("enumValue.set", Metadata, MetadataEnumValue, Values),
    ("enumValue.remove", Metadata, MetadataEnumValue, Target),
    ("column.add", Metadata, MetadataColumns, Items),
    ("column.set", Metadata, MetadataColumn, Values),
    ("column.remove", Metadata, MetadataColumn, Target),
    ("form.add", Form, MetadataForms, Items),
    ("form.set", Form, MetadataForms, Values),
    ("form.remove", Form, Form, Target),
    ("template.add", Metadata, MetadataTemplates, Items),
    ("template.set", Metadata, MetadataTemplate, Values),
    ("template.remove", Metadata, MetadataTemplate, Target),
    ("command.add", Metadata, MetadataCommands, Items),
    ("command.set", Metadata, MetadataCommand, Values),
    ("command.remove", Metadata, MetadataCommand, Target),
    (
        "predefinedItem.add",
        Metadata,
        MetadataPredefinedItems,
        Items
    ),
    (
        "predefinedItem.set",
        Metadata,
        MetadataPredefinedItem,
        Values
    ),
    (
        "predefinedItem.remove",
        Metadata,
        MetadataPredefinedItem,
        Target
    ),
    ("form.create", Form, Form, Values),
    ("element.add", Form, Form, Items),
    ("element.remove", Form, FormOrItem, Target),
    ("formAttribute.add", Form, Form, Items),
    ("formCommand.add", Form, Form, Items),
    ("event.bind", Form, FormOrItem, Values),
    ("role.create", Role, Configuration, Values),
    ("right.set", Role, Role, Values),
    ("dataSource.add", Dcs, DcsSchema, Items),
    ("dataSource.set", Dcs, DcsSchema, Values),
    ("dataSource.remove", Dcs, DcsSchema, Values),
    ("dataSet.add", Dcs, DcsDataContainer, Items),
    ("dataSet.set", Dcs, DcsData, Values),
    ("dataSet.remove", Dcs, DcsData, Target),
    ("field.add", Dcs, DcsData, Items),
    ("field.set", Dcs, DcsData, Values),
    ("field.remove", Dcs, DcsField, Target),
    ("fieldRole.set", Dcs, DcsData, Values),
    ("parameter.add", Dcs, DcsSchema, Items),
    ("parameter.set", Dcs, DcsSchema, Values),
    ("parameter.remove", Dcs, DcsParameter, Target),
    ("parameter.rename", Dcs, DcsSchema, Values),
    ("parameter.reorder", Dcs, DcsSchema, Items),
    ("calculatedField.add", Dcs, DcsSchema, Items),
    ("calculatedField.remove", Dcs, DcsSchema, Values),
    ("total.add", Dcs, DcsSchema, Items),
    ("total.remove", Dcs, DcsSchema, Values),
    ("variant.add", Dcs, DcsSchema, Items),
    ("variant.set", Dcs, DcsSchema, Values),
    ("variant.remove", Dcs, DcsSchema, Values),
    ("query.set", Dcs, DcsData, Values),
    ("query.patch", Dcs, DcsData, Values),
    ("filter.add", Dcs, DcsSettings, Items),
    ("filter.set", Dcs, DcsSettings, Values),
    ("filter.remove", Dcs, DcsSettings, Values),
    ("filter.clear", Dcs, DcsSettings, Values),
    ("selection.add", Dcs, DcsSettings, Items),
    ("selection.clear", Dcs, DcsSettings, Values),
    ("order.add", Dcs, DcsSettings, Items),
    ("order.clear", Dcs, DcsSettings, Values),
    ("dataParameter.add", Dcs, DcsSettings, Items),
    ("dataParameter.set", Dcs, DcsSettings, Values),
    ("outputParameter.set", Dcs, DcsSettings, Values),
    ("conditionalAppearance.add", Dcs, DcsSettings, Items),
    ("conditionalAppearance.clear", Dcs, DcsSettings, Values),
    ("structure.add", Dcs, DcsSettings, Items),
    ("structure.set", Dcs, DcsSettings, Values),
    ("structure.patch", Dcs, DcsSettings, Values),
    ("structure.remove", Dcs, DcsSettings, Values),
    ("dataSetLink.add", Dcs, DcsSchema, Items),
    ("calculatedField.set", Dcs, DcsSchema, Values),
    ("total.set", Dcs, DcsSchema, Values),
    ("selection.set", Dcs, DcsSettings, Values),
    ("selection.remove", Dcs, DcsSettings, Values),
    ("order.set", Dcs, DcsSettings, Values),
    ("order.remove", Dcs, DcsSettings, Values),
    ("dataParameter.remove", Dcs, DcsSettings, Values),
    ("outputParameter.remove", Dcs, DcsSettings, Values),
    ("conditionalAppearance.remove", Dcs, DcsSettings, Values),
    ("dataSetLink.set", Dcs, DcsSchema, Values),
    ("dataSetLink.remove", Dcs, DcsSchema, Values),
    ("mxl.set", Mxl, Mxl, Values),
    ("valueType.add", Xdto, XdtoPackage, Values),
    ("objectType.add", Xdto, XdtoPackage, Values),
    ("property.add", Xdto, XdtoType, Values),
    ("type.remove", Xdto, XdtoType, Target),
    ("property.remove", Xdto, XdtoType, Values),
    ("subsystem.create", Subsystem, Configuration, Values),
    ("content.add", Subsystem, Subsystem, Items),
    ("content.remove", Subsystem, Subsystem, Target),
    ("childSubsystem.add", Subsystem, Subsystem, Items),
    ("childSubsystem.remove", Subsystem, Subsystem, Target),
    ("supportCapability.set", Support, Configuration, Values),
    ("supportRule.set", Support, SupportRule, Values),
    ("commandVisibility.set", Interface, Interface, Items),
    ("commandPlacement.set", Interface, Interface, Items),
    ("commandOrder.set", Interface, Interface, Values),
    ("groupOrder.set", Interface, Interface, Values),
    // Порядок подсистем верхнего уровня живёт в корневом документе, а
    // маршрута `main:Interface` намеренно нет: корень описывается свойством,
    // а не узлом. Интерфейс подсистемы эту операцию не принимает.
    ("subsystemOrder.set", Interface, Configuration, Values),
    ("code.insert", Code, CodeInsert, Text),
    ("code.replace", Code, Code, Text),
    ("event.implement", Event, Event, Target),
);

/// Аргумент, которым операция раньше называла свой предмет, и шаблон
/// адреса, где этот предмет теперь задаётся.
///
/// Предмет операции задаёт `at`; аргумент, называвший подузел, снимается,
/// когда операция переезжает на адрес подузла. Таблица — единственный
/// владелец этого отображения: предварительная проверка `apply` читает её до
/// вердикта «не тот узел», и старый вызов получает `bad_value` с адресом в
/// `next`, а не тупик «операция не применима здесь».
///
/// `key` — путь внутри `args`: точки разделяют ключи объектов, `[]` обходит
/// массив (`items[].into`). В `address` подставляются `{at}` — адрес вызова —
/// и `{value}` — строковое значение снятого ключа. Шаблон, который
/// кончается на `.{name}` вместо `{value}`, — для ключа, который называл
/// подузел не его именем (идентификатором предопределённого элемента):
/// адрес из значения не строится, и отказ ведёт к чтению коллекции, где
/// у подузла видно имя.
///
/// Запись действует, когда операция переехала (`retired`) и исполнима по
/// построенному адресу. Пока операция не переехала, прежний вызов работает
/// без отказа; перенос (П1–П6) переключает `retired` вместе с применимостью
/// операции, а адреса и места ключей таблица уже знает.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetiredArgument {
    pub(crate) op: &'static str,
    pub(crate) key: &'static str,
    pub(crate) address: &'static str,
    /// Операция переехала на адрес, и снятый ключ получает отказ.
    pub(crate) retired: bool,
}

macro_rules! retired_arguments {
    ($(($op:literal, $key:literal, $address:literal, $retired:literal)),+ $(,)?) => {
        pub(crate) const RETIRED_ARGUMENTS: &[RetiredArgument] = &[
            $(RetiredArgument { op: $op, key: $key, address: $address, retired: $retired }),+
        ];
    };
}

// Члены метаданных (П1, #1352): табличная часть, член коллекции и
// предопределённый элемент задаются адресом листа. Формы — П4.
retired_arguments!(
    (
        "attribute.add",
        "scope.tabularSection",
        "{at}.TabularSection.{value}",
        true
    ),
    (
        "tabularSection.set",
        "values.name",
        "{at}.TabularSection.{value}",
        true
    ),
    (
        "tabularSection.remove",
        "values.name",
        "{at}.TabularSection.{value}",
        true
    ),
    (
        "dimension.set",
        "values.name",
        "{at}.Dimension.{value}",
        true
    ),
    (
        "dimension.remove",
        "values.name",
        "{at}.Dimension.{value}",
        true
    ),
    ("resource.set", "values.name", "{at}.Resource.{value}", true),
    (
        "resource.remove",
        "values.name",
        "{at}.Resource.{value}",
        true
    ),
    (
        "enumValue.set",
        "values.name",
        "{at}.EnumValue.{value}",
        true
    ),
    (
        "enumValue.remove",
        "values.name",
        "{at}.EnumValue.{value}",
        true
    ),
    ("column.set", "values.name", "{at}.Column.{value}", true),
    ("column.remove", "values.name", "{at}.Column.{value}", true),
    ("command.set", "values.name", "{at}.Command.{value}", true),
    (
        "command.remove",
        "values.name",
        "{at}.Command.{value}",
        true
    ),
    ("template.set", "values.name", "{at}.Template.{value}", true),
    (
        "template.remove",
        "values.name",
        "{at}.Template.{value}",
        true
    ),
    // Предопределённый элемент прежде назывался идентификатором, а адрес
    // называет его по имени: имя видно в коллекции.
    (
        "predefinedItem.set",
        "values.id",
        "{at}.PredefinedItem.{name}",
        true
    ),
    (
        "predefinedItem.remove",
        "values.id",
        "{at}.PredefinedItem.{name}",
        true
    ),
    // Формы (П4, #1355): элемент события задаётся адресом `Item`.
    ("event.bind", "values.element", "{at}.Item.{value}", false),
);

/// Отказ по снятому аргументу: где он записан и куда перенести вызов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArgumentRedirect {
    /// Путь снятого ключа внутри `args`, с номером элемента массива.
    pub(crate) location: String,
    /// Адрес, на котором операция исполнима, или — при `listing` —
    /// коллекция, где подузел виден по имени.
    pub(crate) at: QualifiedAddress,
    /// Значение ключа не было именем подузла: операция исполнима на
    /// `<at>.<Имя>`, а имя читается в коллекции `at`.
    pub(crate) listing: bool,
}

/// Подстановка в шаблон адреса для записи, чей ключ называл подузел не
/// именем: проверяет, что операция исполнима на члене коллекции.
const LISTED_MEMBER: &str = "Member";

impl RetiredArgument {
    /// Адрес, который строит запись для вызова на `at` с `args`; `None`, если
    /// снятого ключа в вызове нет или значение не даёт адреса.
    pub(crate) fn redirect(
        self,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
    ) -> Option<ArgumentRedirect> {
        let (location, value) = find_key(&Value::Object(args.clone()), self.key, String::new())?;
        // Адрес собирается подстановкой в строку и разбирается заново: единого
        // построителя адреса пока нет, он появится в П5 (#1356) вместе с
        // экранированием точки в именах; тогда подстановку заменит он.
        let raw = self.address.replace("{at}", &at.to_string());
        if let Some(collection) = raw.strip_suffix(".{name}") {
            return Some(ArgumentRedirect {
                location,
                at: QualifiedAddress::parse(collection).ok()?,
                listing: true,
            });
        }
        let target = QualifiedAddress::parse(&raw.replace("{value}", &value)).ok()?;
        Some(ArgumentRedirect {
            location,
            at: target,
            listing: false,
        })
    }
}

impl ArgumentRedirect {
    /// Адрес, на котором операция исполнима: для записи с `listing` — член
    /// коллекции с подставленным именем.
    fn executable_at(&self) -> Option<QualifiedAddress> {
        if !self.listing {
            return Some(self.at.clone());
        }
        QualifiedAddress::parse(&format!("{}.{LISTED_MEMBER}", self.at)).ok()
    }
}

/// Первое строковое значение по пути `key` и его точное место.
fn find_key(value: &Value, key: &str, prefix: String) -> Option<(String, String)> {
    let (head, rest) = match key.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (key, None),
    };
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    if let Some(name) = head.strip_suffix("[]") {
        let items = value.get(name)?.as_array()?;
        let rest = rest?;
        return items
            .iter()
            .enumerate()
            .find_map(|(index, item)| find_key(item, rest, format!("{}[{index}]", join(name))));
    }
    let child = value.get(head)?;
    match rest {
        Some(rest) => find_key(child, rest, join(head)),
        None => child
            .as_str()
            .filter(|text| !text.is_empty())
            .map(|text| (join(head), text.to_string())),
    }
}

/// Отказ по снятому аргументу из таблицы `entries`: запись операции, которая
/// переехала, чей ключ передан и по чьему адресу операция исполнима.
fn retired_argument_redirect_in(
    entries: &[RetiredArgument],
    descriptor: OperationDescriptor,
    site: &OperationSite<'_>,
    args: &Map<String, Value>,
) -> Option<ArgumentRedirect> {
    entries
        .iter()
        .filter(|entry| entry.retired && entry.op == descriptor.name)
        .filter_map(|entry| entry.redirect(site.at, args))
        .find(|redirect| {
            redirect.executable_at().is_some_and(|target| {
                OperationSite::new(&target, site.source)
                    .is_some_and(|target| descriptor.applies_at(&target))
            })
        })
}

fn configuration_root(source_set: &str) -> QualifiedAddress {
    QualifiedAddress::parse(&format!("{source_set}:Configuration"))
        .expect("an admitted source set name forms the configuration-root address")
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationRegistry;

impl OperationRegistry {
    pub(crate) const fn closed() -> Self {
        Self
    }

    pub(crate) const fn descriptors(self) -> &'static [OperationDescriptor] {
        OPERATION_DESCRIPTORS
    }

    pub(crate) fn names(self) -> Vec<&'static str> {
        self.descriptors()
            .iter()
            .map(|descriptor| descriptor.name)
            .collect()
    }

    /// Снятый аргумент, который вызов `op` всё ещё передаёт, и адрес, где
    /// операция исполнима. Читается до вердикта «не тот узел».
    pub(crate) fn retired_argument_redirect(
        self,
        op: &str,
        site: &OperationSite<'_>,
        args: &Map<String, Value>,
    ) -> Option<ArgumentRedirect> {
        let descriptor = self.lookup(op)?;
        retired_argument_redirect_in(RETIRED_ARGUMENTS, descriptor, site, args)
    }

    pub(crate) fn lookup(self, name: &str) -> Option<OperationDescriptor> {
        self.descriptors()
            .iter()
            .copied()
            .find(|descriptor| descriptor.name == name)
    }

    pub(crate) fn copyable_skeletons(self, site: &OperationSite<'_>) -> Vec<OperationRef> {
        self.descriptors()
            .iter()
            .copied()
            .filter(|descriptor| descriptor.applies_at(site))
            .map(OperationDescriptor::copyable_skeleton)
            .collect()
    }

    pub(crate) fn event_implementation_skeleton(
        self,
        at: impl Into<String>,
        call_type: Option<&str>,
    ) -> OperationRef {
        let descriptor = self
            .lookup("event.implement")
            .expect("the closed operation registry owns event implementation");
        let mut args = descriptor.skeleton.args();
        args.insert("at".to_string(), Value::String(at.into()));
        if let Some(call_type) = call_type {
            args.insert("callType".to_string(), Value::String(call_type.to_string()));
        }
        OperationRef::new(descriptor.name, args)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApplyRequest, OperationFamily, OperationRegistry, OperationSite, SourceSetTraits,
        TemplateFormat, RETIRED_ARGUMENTS,
    };
    use crate::domain::address::QualifiedAddress;
    use crate::domain::platform_profile::PlatformProfile;
    use crate::domain::project_sources::SourceSetKind;
    use serde_json::{json, Value};

    fn address(raw: &str) -> QualifiedAddress {
        QualifiedAddress::parse(raw).unwrap_or_else(|error| panic!("{raw}: {error}"))
    }

    fn applies_in(source: SourceSetKind, op: &str, raw: &str) -> bool {
        let at = address(raw);
        let site = OperationSite::new(
            &at,
            SourceSetTraits::read(source, PlatformProfile::v8_3_27()),
        )
        .unwrap_or_else(|| panic!("{raw} has no route"));
        OperationRegistry::closed()
            .lookup(op)
            .unwrap_or_else(|| panic!("{op} is registered"))
            .applies_at(&site)
    }

    fn applies(op: &str, raw: &str) -> bool {
        applies_in(SourceSetKind::Configuration, op, raw)
    }

    fn parse(value: Value) -> Result<ApplyRequest, super::ApplyValidationError> {
        ApplyRequest::parse(
            value.as_object().expect("apply fixture must be an object"),
            &["main", "extension"],
        )
    }

    #[test]
    fn object_borrow_is_available_only_at_the_configuration_root() {
        let descriptor = OperationRegistry::closed()
            .lookup("object.borrow")
            .expect("canonical top-level borrowing must be registered");
        assert_eq!(descriptor.name(), "object.borrow");
        assert!(applies_in(
            SourceSetKind::Extension,
            "object.borrow",
            "extension:Configuration"
        ));
        assert!(!applies_in(
            SourceSetKind::Extension,
            "object.borrow",
            "extension:Catalog.Products"
        ));
        // Корень конфигурации — не расширение: заимствовать в него нечего.
        assert!(!applies("object.borrow", "main:Configuration"));
    }

    #[test]
    fn apply_request_retains_order_inherits_targets_and_keeps_items_as_data() {
        let request = parse(json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set", "args": {"items": [{"at": "not-an-operation-target"}]}},
                {"op": "attribute.add", "args": {"at": "main:Document.Order.Attribute", "name": "Total"}},
                {"op": "code.replace", "args": {"at": "main:Document.Order.Module.Object.Body", "text": "Procedure X()\nEndProcedure"}}
            ],
            "ifRev": "unica-source-sha256-v1:7:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .expect("valid ordered apply request");

        assert_eq!(request.at().to_string(), "main:Document.Order");
        assert_eq!(
            request
                .ops()
                .iter()
                .map(|operation| operation.name())
                .collect::<Vec<_>>(),
            ["props.set", "attribute.add", "code.replace"]
        );
        assert_eq!(request.ops()[0].at(), request.at());
        assert_eq!(
            request.ops()[1].at().to_string(),
            "main:Document.Order.Attribute"
        );
        assert_eq!(
            request.ops()[2].at().to_string(),
            "main:Document.Order.Module.Object.Body"
        );
        assert_eq!(
            request.ops()[0].args()["items"],
            json!([{"at": "not-an-operation-target"}]),
            "nested items[] are operation data, not sibling operations or targets"
        );
        assert!(!request.dry_run());
        assert_eq!(
            request.if_rev(),
            Some("unica-source-sha256-v1:7:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn apply_request_rejects_empty_ops_and_reports_the_exact_nested_target_location() {
        let empty = parse(json!({"at": "main:Document.Order", "ops": []})).unwrap_err();
        assert_eq!(empty.location(), "ops");
        assert_eq!(empty.code().as_str(), "bad_value");

        let cross_source = parse(json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set"},
                {"op": "attribute.add", "args": {"at": "extension:Document.Order.Attribute"}}
            ]
        }))
        .unwrap_err();
        assert_eq!(cross_source.location(), "ops[1].args.at");

        let sibling = parse(json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set"},
                {"op": "attribute.add", "args": {"at": "main:Catalog.Products.Attribute"}}
            ]
        }))
        .unwrap_err();
        assert_eq!(sibling.location(), "ops[1].args.at");
    }

    /// Узел создаёт операция родителя, поэтому новая роль или подсистема,
    /// созданная на корне, сразу доступна следующим операциям пакета.
    #[test]
    fn role_and_subsystem_created_at_the_root_extend_the_request_scope() {
        for (create, target) in [
            ("role.create", "main:Role.Reader"),
            ("subsystem.create", "main:Subsystem.Sales"),
        ] {
            let ops = json!([
                {"op": create, "args": {"values": {"name": target.rsplit('.').next().unwrap()}}},
                {"op": "props.set", "args": {"at": target, "values": {"Comment": "x"}}}
            ]);
            assert!(
                parse(json!({"at": "main:Configuration", "ops": ops, "dryRun": true})).is_ok(),
                "{create}"
            );
            let other = json!([
                {"op": create, "args": {"values": {"name": "Other"}}},
                {"op": "props.set", "args": {"at": target, "values": {"Comment": "x"}}}
            ]);
            assert_eq!(
                parse(json!({"at": "main:Configuration", "ops": other, "dryRun": true}))
                    .unwrap_err()
                    .location(),
                "ops[1].args.at"
            );
        }
    }

    #[test]
    fn create_then_configure_scope_only_accepts_earlier_created_object() {
        let create = json!({"op": "object.create", "args": {"values": {"kind": "Constant", "name": "Enabled"}}});
        let configure = |target: &str| json!({"op": "props.set", "args": {"at": target, "values": {"Comment": "x"}}});
        for target in [
            "main:Constant.Enabled",
            "main:Constant.Enabled.Module.ValueManager",
        ] {
            assert!(parse(json!({"at": "main:Configuration", "ops": [create.clone(), configure(target)], "dryRun": true})).is_ok());
        }
        for target in [
            "main:Constant.Other",
            "extension:Constant.Enabled",
            "main:Catalog.Enabled",
        ] {
            let error = parse(json!({"at": "main:Configuration", "ops": [create.clone(), configure(target)], "dryRun": true})).unwrap_err();
            assert_eq!(error.location(), "ops[1].args.at");
        }
        let error = parse(json!({"at": "main:Configuration", "ops": [configure("main:Constant.Enabled"), create.clone()], "dryRun": true})).unwrap_err();
        assert_eq!(error.location(), "ops[0].args.at");
        for malformed in [
            json!({"kind": "Unknown", "name": "Enabled"}),
            json!({"kind": "Constant", "name": "Enabled.Form.Main"}),
        ] {
            let error = parse(json!({"at": "main:Configuration", "ops": [{"op": "object.create", "args": {"values": malformed}}, configure("main:Constant.Enabled")], "dryRun": true})).unwrap_err();
            assert_eq!(error.location(), "ops[1].args.at");
        }
    }

    #[test]
    fn apply_request_preserves_explicit_dry_run_and_one_batch_revision() {
        let request = parse(json!({
            "at": "main:Document.Order",
            "ops": [{"op": "props.set", "args": {}}],
            "dryRun": true,
            "ifRev": "rev-42"
        }))
        .unwrap();
        assert!(request.dry_run());
        assert_eq!(request.if_rev(), Some("rev-42"));
    }

    #[test]
    fn operation_registry_is_exact_closed_unique_and_drives_skeletons_and_dispatch() {
        let expected = [
            "object.borrow",
            "object.create",
            "object.remove",
            "props.set",
            "relation.add",
            "relation.remove",
            "relation.replace",
            "help.create",
            "attribute.add",
            "attribute.set",
            "attribute.remove",
            "tabularSection.add",
            "tabularSection.set",
            "tabularSection.remove",
            "dimension.add",
            "dimension.set",
            "dimension.remove",
            "resource.add",
            "resource.set",
            "resource.remove",
            "enumValue.add",
            "enumValue.set",
            "enumValue.remove",
            "column.add",
            "column.set",
            "column.remove",
            "form.add",
            "form.set",
            "form.remove",
            "template.add",
            "template.set",
            "template.remove",
            "command.add",
            "command.set",
            "command.remove",
            "predefinedItem.add",
            "predefinedItem.set",
            "predefinedItem.remove",
            "form.create",
            "element.add",
            "element.remove",
            "formAttribute.add",
            "formCommand.add",
            "event.bind",
            "role.create",
            "right.set",
            "dataSource.add",
            "dataSource.set",
            "dataSource.remove",
            "dataSet.add",
            "dataSet.set",
            "dataSet.remove",
            "field.add",
            "field.set",
            "field.remove",
            "fieldRole.set",
            "parameter.add",
            "parameter.set",
            "parameter.remove",
            "parameter.rename",
            "parameter.reorder",
            "calculatedField.add",
            "calculatedField.remove",
            "total.add",
            "total.remove",
            "variant.add",
            "variant.set",
            "variant.remove",
            "query.set",
            "query.patch",
            "filter.add",
            "filter.set",
            "filter.remove",
            "filter.clear",
            "selection.add",
            "selection.clear",
            "order.add",
            "order.clear",
            "dataParameter.add",
            "dataParameter.set",
            "outputParameter.set",
            "conditionalAppearance.add",
            "conditionalAppearance.clear",
            "structure.add",
            "structure.set",
            "structure.patch",
            "structure.remove",
            "dataSetLink.add",
            "calculatedField.set",
            "total.set",
            "selection.set",
            "selection.remove",
            "order.set",
            "order.remove",
            "dataParameter.remove",
            "outputParameter.remove",
            "conditionalAppearance.remove",
            "dataSetLink.set",
            "dataSetLink.remove",
            "mxl.set",
            "valueType.add",
            "objectType.add",
            "property.add",
            "type.remove",
            "property.remove",
            "subsystem.create",
            "content.add",
            "content.remove",
            "childSubsystem.add",
            "childSubsystem.remove",
            "supportCapability.set",
            "supportRule.set",
            "commandVisibility.set",
            "commandPlacement.set",
            "commandOrder.set",
            "groupOrder.set",
            "subsystemOrder.set",
            "code.insert",
            "code.replace",
            "event.implement",
        ];
        let registry = OperationRegistry::closed();
        assert_eq!(expected.len(), 120);
        assert_eq!(registry.names(), expected);
        let mut unique = registry.names().to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), expected.len());

        assert_eq!(
            registry.lookup("event.implement").unwrap().family(),
            OperationFamily::Event
        );
        assert!(registry.lookup("module.create").is_none());
        assert!(applies("props.set", "main:Subsystem.Sales"));
        assert!(applies("props.set", "main:Document.Order"));

        let event_at = address("main:Catalog.Items.Module.Object.Event.OnWrite");
        let event_site = OperationSite::new(
            &event_at,
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        let skeletons = registry.copyable_skeletons(&event_site);
        let event = skeletons
            .iter()
            .map(|item| serde_json::to_value(item).unwrap())
            .find(|value| value["op"] == "event.implement")
            .expect("Event.can must be projected from the dispatch registry");
        assert_eq!(event, json!({"op": "event.implement", "args": {"at": ""}}));
        for skeleton in registry
            .descriptors()
            .iter()
            .map(|descriptor| descriptor.copyable_skeleton())
        {
            let value = serde_json::to_value(skeleton).unwrap();
            assert!(
                value.to_string().len() <= 256,
                "skeleton must stay bounded: {value}"
            );
            let rendered = value.to_string();
            for forbidden in [
                "jsonPath",
                "definitionFile",
                "outputPath",
                "provider",
                "providerId",
            ] {
                assert!(
                    !rendered.contains(forbidden),
                    "physical/provider field leaked: {value}"
                );
            }
        }
    }

    #[test]
    fn metadata_collection_operations_follow_the_exact_typed_kind_matrix() {
        let cases = [
            ("attribute.add", "main:Catalog.Items", true),
            ("attribute.add", "main:InformationRegister.Prices", true),
            ("attribute.add", "main:Enum.Kinds", false),
            ("tabularSection.add", "main:Document.Order", true),
            (
                "tabularSection.add",
                "main:InformationRegister.Prices",
                false,
            ),
            ("dimension.add", "main:InformationRegister.Prices", true),
            ("dimension.add", "main:Document.Order", false),
            (
                "resource.remove",
                "main:CalculationRegister.Payroll.Resource.Amount",
                true,
            ),
            (
                "resource.remove",
                "main:Catalog.Items.Resource.Amount",
                false,
            ),
            ("enumValue.set", "main:Enum.Kinds.EnumValue.Main", true),
            ("enumValue.set", "main:Catalog.Items.EnumValue.Main", false),
            (
                "column.remove",
                "main:DocumentJournal.All.Column.Date",
                true,
            ),
            ("column.remove", "main:Document.Order.Column.Date", false),
            ("predefinedItem.add", "main:Catalog.Items", true),
            ("predefinedItem.add", "main:ChartOfAccounts.Main", true),
            ("predefinedItem.add", "main:Document.Order", false),
            ("form.add", "main:Constant.Rate", true),
            ("template.add", "main:Constant.Rate", false),
            ("command.add", "main:Constant.Rate", false),
            ("attribute.add", "main:CommonModule.Shared", false),
        ];
        for (name, raw, expected) in cases {
            assert_eq!(applies(name, raw), expected, "{name} at {raw}");
        }
    }

    /// Применимость зависит от маршрута, а не от вида последнего сегмента:
    /// одинаковые виды в разных деревьях получают разные операции. Строки —
    /// известные расхождения #1351 и их правильные места.
    #[test]
    fn applicability_follows_the_route_not_the_terminal_kind() {
        let cases = [
            // Реквизит объекта метаданных: свои операции есть, форм — нет.
            ("attribute.set", "main:Document.Order.Attribute.Total", true),
            (
                "attribute.remove",
                "main:Document.Order.Attribute.Total",
                true,
            ),
            (
                "attribute.set",
                "main:Document.Order.TabularSection.Goods.Attribute.Price",
                true,
            ),
            ("attribute.set", "main:Document.Order", false),
            ("element.add", "main:Document.Order.Attribute.Total", false),
            ("event.bind", "main:Document.Order.Attribute.Total", false),
            ("element.add", "main:Document.Order.Command.Print", false),
            // Реквизит формы — узел формы, не метаданных.
            (
                "attribute.set",
                "main:Document.Order.Form.Main.Attribute.Object",
                false,
            ),
            ("element.add", "main:Document.Order.Form.Main", true),
            (
                "element.add",
                "main:Document.Order.Form.Main.Item.Header",
                false,
            ),
            (
                "element.remove",
                "main:Document.Order.Form.Main.Item.Header",
                true,
            ),
            (
                "event.bind",
                "main:Document.Order.Form.Main.Item.Header",
                true,
            ),
            ("form.set", "main:Document.Order", true),
            ("form.set", "main:Document.Order.Form.Main", false),
            ("form.remove", "main:Document.Order.Form.Main", true),
            ("form.remove", "main:Document.Order", false),
            // Права пишутся на роли; узел права разбор цели не принимает.
            ("right.set", "main:Role.Reader", true),
            ("right.set", "main:Role.Reader.Right.Catalog_Items", false),
            // Ячейки пишутся в макет; область — его подузел.
            ("mxl.set", "main:Report.Sales.Template.Print", true),
            (
                "mxl.set",
                "main:Report.Sales.Template.Print.Area.Header",
                false,
            ),
            // Код пишется в модуль, его тело и метод; область — нет.
            ("code.replace", "main:CommonModule.Shared", true),
            ("code.replace", "main:CommonModule.Shared.Method.Run", true),
            ("code.insert", "main:CommonModule.Shared.Method.Run", false),
            ("code.insert", "main:CommonModule.Shared.Body", true),
            (
                "code.insert",
                "main:CommonModule.Shared.Method.Run.Body",
                false,
            ),
            (
                "code.replace",
                "main:Document.Order.Module.Object.Region.Main",
                false,
            ),
            // Метод HTTP-сервиса — член метаданных, не метод модуля.
            (
                "code.replace",
                "main:HTTPService.Api.URLTemplate.Items.Method.Get",
                false,
            ),
            // Параметр СКД и параметр операции веб-сервиса — разные узлы.
            (
                "parameter.remove",
                "main:Report.Sales.Template.Schema.Parameter.Period",
                true,
            ),
            (
                "parameter.remove",
                "main:WebService.Exchange.Operation.Send.Parameter.Body",
                false,
            ),
            // XDTO: пакет и тип, но не пространство имён и не свойство.
            ("objectType.add", "main:XDTOPackage.Exchange", true),
            ("property.add", "main:XDTOPackage.Exchange.Type.Order", true),
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Type.Order.Property.Id",
                false,
            ),
            // Узел создаёт операция родителя.
            ("role.create", "main:Configuration", true),
            ("role.create", "main:Role.Reader", false),
            ("subsystem.create", "main:Configuration", true),
            ("subsystem.create", "main:Subsystem.Sales", false),
            ("object.create", "main:Catalog.Items", false),
            // Возможность изменения — свойство всей конфигурации.
            ("supportCapability.set", "main:Configuration", true),
            ("supportCapability.set", "main:Catalog.Items", false),
            ("supportRule.set", "main:Catalog.Items", true),
            // Командный интерфейс — у подсистемы, порядок верхнего уровня — у корня.
            ("commandOrder.set", "main:Subsystem.Sales.Interface", true),
            ("subsystemOrder.set", "main:Configuration", true),
            (
                "subsystemOrder.set",
                "main:Subsystem.Sales.Interface",
                false,
            ),
        ];
        for (name, raw, expected) in cases {
            assert_eq!(applies(name, raw), expected, "{name} at {raw}");
        }
        // Внешний набор исходников не пишется ни одной операцией.
        assert!(!applies_in(
            SourceSetKind::ExternalProcessor,
            "props.set",
            "main:ExternalDataProcessor.Tool"
        ));
        // Событие объекта пишется в модуль платформы, которого у расширения нет.
        assert!(applies(
            "event.implement",
            "main:Catalog.Items.Module.Object.Event.OnWrite"
        ));
        assert!(!applies_in(
            SourceSetKind::Extension,
            "event.implement",
            "main:Catalog.Items.Module.Object.Event.OnWrite"
        ));
    }

    /// Формат макета — отдельный фильтр с одним владельцем: семейства СКД и
    /// табличного документа применимы только к макету своего формата.
    #[test]
    fn template_format_is_a_separate_filter_for_dcs_and_mxl_families() {
        let at = address("main:Report.Sales.Template.Layout");
        let site = OperationSite::new(
            &at,
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        let registry = OperationRegistry::closed();
        for (op, format, expected) in [
            ("mxl.set", Some(TemplateFormat::SpreadsheetDocument), true),
            (
                "mxl.set",
                Some(TemplateFormat::DataCompositionSchema),
                false,
            ),
            ("mxl.set", None, false),
            (
                "dataSet.add",
                Some(TemplateFormat::DataCompositionSchema),
                true,
            ),
            (
                "dataSet.add",
                Some(TemplateFormat::SpreadsheetDocument),
                false,
            ),
            ("dataSet.add", Some(TemplateFormat::Other), false),
            ("template.set", Some(TemplateFormat::Other), true),
        ] {
            assert_eq!(
                registry.lookup(op).unwrap().applies_to_node(&site, format),
                expected,
                "{op} on {format:?}"
            );
        }
    }

    /// `next` неприменимой операции ведёт к ближайшему узлу, где она
    /// исполнима; для операции верхнего уровня это корень набора.
    #[test]
    fn a_misplaced_operation_names_the_nearest_node_where_it_applies() {
        let registry = OperationRegistry::closed();
        let format_of = |at: &QualifiedAddress| match at.segments().last()?.name()? {
            "Print" => Some(TemplateFormat::SpreadsheetDocument),
            "Schema" => Some(TemplateFormat::DataCompositionSchema),
            _ => None,
        };
        for (op, raw, expected) in [
            (
                "mxl.set",
                "main:Report.Sales.Template.Print.Area.Header",
                Some("main:Report.Sales.Template.Print"),
            ),
            (
                "right.set",
                "main:Role.Reader.Right.Catalog_Items",
                Some("main:Role.Reader"),
            ),
            (
                "role.create",
                "main:Role.Reader",
                Some("main:Configuration"),
            ),
            (
                "element.add",
                "main:Document.Order.Form.Main.Item.Header",
                Some("main:Document.Order.Form.Main"),
            ),
            ("element.add", "main:Document.Order.Attribute.Total", None),
            // Под макетом-схемой ячеек нет: `next` не ведёт туда, где
            // планировщик откажет по формату.
            (
                "mxl.set",
                "main:Report.Sales.Template.Schema.Area.Header",
                None,
            ),
        ] {
            let at = address(raw);
            let site = OperationSite::new(
                &at,
                SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
            )
            .unwrap();
            assert_eq!(
                registry
                    .lookup(op)
                    .unwrap()
                    .nearest_applicable_ancestor(&site, &format_of)
                    .map(|at| at.to_string()),
                expected.map(str::to_string),
                "{op} at {raw}"
            );
        }
    }

    /// Каждая запись таблицы снятых аргументов строит адрес листа владельца
    /// того вида, который называет шаблон, и распознаёт снятый ключ по своему
    /// пути. Переехавшая операция на владельце больше не исполнима, и прежний
    /// вызов получает отказ с адресом листа; вызов на самом листе отказа по
    /// таблице не получает. Запись ещё не переехавшей операции не срабатывает.
    #[test]
    fn every_retired_argument_entry_builds_a_routable_member_address() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let owners = [
            "main:Document.Order",
            "main:Enum.Kinds",
            "main:InformationRegister.Prices",
            "main:DocumentJournal.All",
            "main:Report.Sales",
            "main:Catalog.Items",
            "main:Document.Order.Form.Main",
        ];
        for entry in RETIRED_ARGUMENTS {
            let descriptor = registry
                .lookup(entry.op)
                .unwrap_or_else(|| panic!("{} is registered", entry.op));
            let (container, key) = entry.key.split_once('.').expect("nested key");
            let args = json!({container: {key: "Member"}});
            let args = args.as_object().unwrap();
            let mut proved = false;
            for owner in owners {
                let at = address(owner);
                let site = OperationSite::new(&at, traits).unwrap();
                let redirect = entry
                    .redirect(&at, args)
                    .unwrap_or_else(|| panic!("{} {} builds no address", entry.op, entry.key));
                assert_eq!(redirect.location, entry.key);
                let member = if redirect.listing {
                    address(&format!("{}.Member", redirect.at))
                } else {
                    redirect.at.clone()
                };
                let target = OperationSite::new(&member, traits)
                    .unwrap_or_else(|| panic!("{member} is not routable"));
                if !descriptor.applies_at(&target) {
                    continue;
                }
                // Адрес подузла остаётся в дереве владельца.
                assert_eq!(target.route(), site.route(), "{member}");
                assert_eq!(
                    member.segments().len(),
                    at.segments().len() + 1,
                    "{member} names one member below the owner"
                );
                assert_eq!(member.segments().last().unwrap().name(), Some("Member"));
                let refused = registry.retired_argument_redirect(entry.op, &site, args);
                if entry.retired {
                    assert_eq!(refused, Some(redirect), "{} at {owner}", entry.op);
                    assert_eq!(
                        registry.retired_argument_redirect(entry.op, &target, args),
                        None,
                        "{} at its own address is not redirected",
                        entry.op
                    );
                } else {
                    assert!(
                        descriptor.applies_at(&site),
                        "{} still at {owner}",
                        entry.op
                    );
                    assert_eq!(refused, None, "{} has not moved yet", entry.op);
                }
                proved = true;
            }
            assert!(
                proved,
                "{} {}: no owner leads to a node where the operation applies",
                entry.op, entry.key
            );
        }
    }

    /// Пакет П1 перенёс члены метаданных: каждая их запись в таблице
    /// переключена, а предопределённый элемент, прежде названный
    /// идентификатором, ведёт к коллекции, где видно его имя.
    #[test]
    fn metadata_member_entries_are_retired_and_predefined_items_route_to_their_listing() {
        for entry in RETIRED_ARGUMENTS {
            let member_package = entry.op != "event.bind";
            assert_eq!(entry.retired, member_package, "{}", entry.op);
            assert_eq!(
                entry.address.ends_with(".{name}"),
                entry.op.starts_with("predefinedItem."),
                "{}",
                entry.op
            );
        }
        let registry = OperationRegistry::closed();
        let owner = address("main:Catalog.Items");
        let site = OperationSite::new(
            &owner,
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        let redirect = registry
            .retired_argument_redirect(
                "predefinedItem.remove",
                &site,
                json!({"values": {"id": "a1b2c3d4-1111-4111-8111-111111111111"}})
                    .as_object()
                    .unwrap(),
            )
            .expect("the id selector is retired");
        assert!(redirect.listing);
        assert_eq!(redirect.location, "values.id");
        assert_eq!(redirect.at.to_string(), "main:Catalog.Items.PredefinedItem");
    }

    /// Механизм перехода: запись переехавшей операции отдаёт место снятого
    /// ключа и адрес, где операция исполнима; запись, чья операция по
    /// построенному адресу не исполнима, не срабатывает.
    #[test]
    fn a_retired_argument_redirects_once_the_operation_has_moved() {
        let registry = OperationRegistry::closed();
        let owner = address("main:Report.Sales");
        let site = OperationSite::new(
            &owner,
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        let args = json!({"values": {"name": "Print", "synonym": "x"}});
        let redirect = registry
            .retired_argument_redirect("template.set", &site, args.as_object().unwrap())
            .expect("a moved operation refuses its retired argument");
        assert_eq!(redirect.location, "values.name");
        assert_eq!(redirect.at.to_string(), "main:Report.Sales.Template.Print");
        assert!(!redirect.listing);
        // `attribute.set` не исполним на табличной части: даже переключённая
        // запись не уводит вызов туда, где его отвергнут.
        let misdirected = super::RetiredArgument {
            op: "attribute.set",
            key: "values.name",
            address: "{at}.TabularSection.{value}",
            retired: true,
        };
        let document = address("main:Document.Order");
        let site = OperationSite::new(
            &document,
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        assert_eq!(
            super::retired_argument_redirect_in(
                &[misdirected],
                registry.lookup("attribute.set").unwrap(),
                &site,
                json!({"values": {"name": "Goods"}}).as_object().unwrap(),
            ),
            None
        );
        // Вложенная позиция в массиве называет номер элемента.
        let nested = super::RetiredArgument {
            op: "element.add",
            key: "items[].into",
            address: "{at}.Item.{value}",
            retired: true,
        };
        let form = address("main:Document.Order.Form.Main");
        let redirect = nested
            .redirect(
                &form,
                json!({"items": [{"name": "A"}, {"name": "B", "into": "Header"}]})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(redirect.location, "items[1].into");
        assert_eq!(
            redirect.at.to_string(),
            "main:Document.Order.Form.Main.Item.Header"
        );
        assert!(nested
            .redirect(
                &form,
                json!({"items": [{"name": "A"}]}).as_object().unwrap()
            )
            .is_none());
    }

    /// Члены метаданных адресуются (#1352): изменение и удаление исполнимы на
    /// самом члене и не исполнимы на владельце; реквизит добавляется и на
    /// владельце, и на его табличной части.
    #[test]
    fn member_operations_apply_at_the_member_address_not_at_the_owner() {
        for (op, member, owner) in [
            (
                "tabularSection.set",
                "main:Document.Order.TabularSection.Goods",
                "main:Document.Order",
            ),
            (
                "tabularSection.remove",
                "main:Document.Order.TabularSection.Goods",
                "main:Document.Order",
            ),
            (
                "dimension.set",
                "main:InformationRegister.Prices.Dimension.Item",
                "main:InformationRegister.Prices",
            ),
            (
                "dimension.remove",
                "main:InformationRegister.Prices.Dimension.Item",
                "main:InformationRegister.Prices",
            ),
            (
                "resource.set",
                "main:InformationRegister.Prices.Resource.Price",
                "main:InformationRegister.Prices",
            ),
            (
                "resource.remove",
                "main:AccumulationRegister.Stock.Resource.Quantity",
                "main:AccumulationRegister.Stock",
            ),
            (
                "enumValue.set",
                "main:Enum.Kinds.EnumValue.Main",
                "main:Enum.Kinds",
            ),
            (
                "enumValue.remove",
                "main:Enum.Kinds.EnumValue.Main",
                "main:Enum.Kinds",
            ),
            (
                "column.set",
                "main:DocumentJournal.All.Column.Date",
                "main:DocumentJournal.All",
            ),
            (
                "column.remove",
                "main:DocumentJournal.All.Column.Date",
                "main:DocumentJournal.All",
            ),
            (
                "command.set",
                "main:Document.Order.Command.Print",
                "main:Document.Order",
            ),
            (
                "command.remove",
                "main:Document.Order.Command.Print",
                "main:Document.Order",
            ),
            (
                "template.set",
                "main:Report.Sales.Template.Print",
                "main:Report.Sales",
            ),
            (
                "template.remove",
                "main:Report.Sales.Template.Print",
                "main:Report.Sales",
            ),
            (
                "predefinedItem.set",
                "main:Catalog.Items.PredefinedItem.Main",
                "main:Catalog.Items",
            ),
            (
                "predefinedItem.remove",
                "main:ChartOfAccounts.Main.PredefinedItem.Cash",
                "main:ChartOfAccounts.Main",
            ),
        ] {
            assert!(applies(op, member), "{op} at {member}");
            assert!(!applies(op, owner), "{op} at {owner}");
        }
        // Табличная часть — место реквизитов, но не измерений и не самой
        // себя; узел чужого вида операцию члена не принимает.
        assert!(applies(
            "attribute.add",
            "main:Document.Order.TabularSection.Goods"
        ));
        assert!(applies("attribute.add", "main:Document.Order"));
        assert!(!applies(
            "attribute.add",
            "main:Document.Order.TabularSection.Goods.Attribute.Price"
        ));
        assert!(!applies(
            "attribute.add",
            "main:InformationRegister.Prices.TabularSection.Goods"
        ));
        assert!(!applies(
            "tabularSection.add",
            "main:Document.Order.TabularSection.Goods"
        ));
        assert!(!applies(
            "dimension.set",
            "main:Document.Order.TabularSection.Goods"
        ));
        assert!(!applies(
            "command.set",
            "main:Document.Order.Form.Main.Command.Ok"
        ));
        // Создание остаётся на владельце: у адреса члена `add` нет.
        assert!(applies("predefinedItem.add", "main:Catalog.Items"));
        assert!(!applies(
            "predefinedItem.add",
            "main:Catalog.Items.PredefinedItem.Main"
        ));
        assert!(!applies("template.add", "main:Report.Sales.Template.Print"));
    }

    #[test]
    fn registry_skeletons_are_bounded_useful_and_copyable_operation_objects() {
        let registry = OperationRegistry::closed();
        let cases = [
            (
                "props.set",
                json!({"op": "props.set", "args": {"values": {}}}),
            ),
            (
                "attribute.add",
                json!({"op": "attribute.add", "args": {"items": []}}),
            ),
            (
                "code.replace",
                json!({"op": "code.replace", "args": {"text": ""}}),
            ),
            (
                "event.implement",
                json!({"op": "event.implement", "args": {"at": ""}}),
            ),
        ];
        for (name, expected) in cases {
            let descriptor = registry.lookup(name).unwrap();
            assert_eq!(
                serde_json::to_value(descriptor.copyable_skeleton()).unwrap(),
                expected
            );
            assert_eq!(descriptor.family(), registry.lookup(name).unwrap().family());
        }
        for descriptor in registry.descriptors() {
            let value = serde_json::to_value(descriptor.copyable_skeleton()).unwrap();
            assert!(value["args"]
                .as_object()
                .is_some_and(|args| !args.is_empty()));
            assert!(value.to_string().len() <= 256, "{value}");
        }
    }
}
