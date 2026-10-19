use crate::domain::address::{escape_name, AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::form_operation::FormOperation;
use crate::domain::logical_route::{classify_route, RouteClass};
use crate::domain::metadata::{metadata_kind_collections, MetaCollection, MetadataKind};
use crate::domain::metadata_operation::MetadataOperation;
use crate::domain::node_view::OperationRef;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::domain::refusal::RefusalCode;
use crate::domain::role_operation::RoleOperation;
use crate::domain::support_operation::SupportOperation;
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
                    || !valid_borrow_source(&parent, at.source_set())
                {
                    return Err(ApplyValidationError::bad_value(&location, "from must name a top-level object or Enum.EnumValue in a distinct admitted parent source set"));
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

pub(crate) fn valid_borrow_source(parent: &QualifiedAddress, destination_source_set: &str) -> bool {
    parent.source_set() != destination_source_set
        && (parent.segments().len() == 1
            || (parent.segments().len() == 2
                && parent.segments()[0].kind() == NodeKind::Enum
                && parent.segments()[1].kind() == NodeKind::EnumValue
                && parent.segments()[1].name().is_some()))
        && parent.segments()[0].name().is_some()
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

    /// Цепочка элементов формы объекта: `Owner.X.Form.F[.Item.A[.Item.B…]]`
    /// даёт сегменты `Item` после формы (пустые у самой формы). `None` —
    /// `segments` не такой адрес или маршрут не формы.
    fn form_items<'s>(&self, segments: &'s [AddressSegment]) -> Option<&'s [AddressSegment]> {
        match segments {
            [owner, form, items @ ..]
                if self.route == RouteClass::Form
                    && owner.name().is_some()
                    && form.kind() == NodeKind::Form
                    && form.name().is_some()
                    && metadata_collection_matches(owner.kind(), MetaCollection::Forms)
                    && items
                        .iter()
                        .all(|item| item.kind() == NodeKind::Item && item.name().is_some()) =>
            {
                Some(items)
            }
            _ => None,
        }
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

    /// Узел схемы прямо под макетом владельца: `Owner.X.Template.T.<kind>.N`.
    fn dcs_schema_member(&self, kind: NodeKind) -> bool {
        self.route == RouteClass::Dcs
            && matches!(self.segments(), [owner, template, member]
                if owner.name().is_some()
                    && template.kind() == NodeKind::Template
                    && template.name().is_some()
                    && member.kind() == kind
                    && member.name().is_some()
                    && metadata_collection_matches(owner.kind(), MetaCollection::Templates))
    }

    /// Вариант настроек макета владельца и цепочка именованных элементов его
    /// структуры: число сегментов `Item` после `Setting`, `None` — адрес не
    /// такой формы.
    fn dcs_structure_depth(&self) -> Option<usize> {
        if self.route != RouteClass::Dcs {
            return None;
        }
        let [owner, template, setting, items @ ..] = self.segments() else {
            return None;
        };
        (owner.name().is_some()
            && template.kind() == NodeKind::Template
            && template.name().is_some()
            && setting.kind() == NodeKind::Setting
            && setting.name().is_some()
            && metadata_collection_matches(owner.kind(), MetaCollection::Templates)
            && items
                .iter()
                .all(|item| item.kind() == NodeKind::Item && item.name().is_some()))
        .then_some(items.len())
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
pub(super) enum OperationApplicability {
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
    /// Форма объекта или её элемент любой вложенности: место нового элемента.
    FormItemContainer,
    /// Элемент формы любой вложенности: `Form.F.Item.A[.Item.B…]`.
    FormItem,
    /// Событие формы или её элемента: `Form.F[.Item.A…].Event.E`.
    FormEvent,
    /// Роль верхнего уровня.
    Role,
    /// Запись прав роли на один объект: `Role.R.Right.<Вид_Имя>`.
    RoleRight,
    /// Подсистема любой вложенности.
    Subsystem,
    /// Вложенная подсистема: исключение из состава её родителя.
    NestedSubsystem,
    /// Командный интерфейс подсистемы.
    Interface,
    /// Макет-схема компоновки как узел владельца.
    DcsSchema,
    /// Макет-схема или её набор данных: место нового набора.
    DcsDataContainer,
    /// Набор данных схемы.
    DcsData,
    /// Запрос набора данных схемы `DataSet.D.Query`.
    DcsQuery,
    /// Вариант настроек схемы.
    DcsSettings,
    /// Параметр схемы.
    DcsParameter,
    /// Поле набора данных.
    DcsField,
    /// Источник данных схемы: `Template.T.DataSource.S` (#1357).
    DcsDataSource,
    /// Вычисляемое поле схемы: `Template.T.CalculatedField.F`.
    DcsCalculatedField,
    /// Итог схемы: `Template.T.Total.F`.
    DcsTotal,
    /// Вариант настроек или именованный элемент его структуры любой
    /// вложенности: `Setting.S[.Item.I…]`. Коллекции настроек элемента
    /// (отбор, выбор, порядок, оформление, параметры вывода) и вложенные
    /// элементы структуры берут контейнер из адреса.
    DcsSettingsContainer,
    /// Именованный элемент структуры варианта: `Setting.S.Item.I[.Item.J…]`.
    DcsStructureItem,
    /// Макет-табличный документ как узел владельца.
    Mxl,
    /// Именованная область макета-табличного документа.
    MxlArea,
    /// Пакет XDTO.
    XdtoPackage,
    /// Тип пакета XDTO.
    XdtoType,
    /// Свойство типа XDTO любой вложенности:
    /// `Type.T.Property.A[.Property.B…]`.
    XdtoProperty,
    /// Место нового свойства XDTO: тип или свойство, во вложенный
    /// анонимный тип которого оно добавляется.
    XdtoPropertyContainer,
    /// Группа командного интерфейса подсистемы.
    InterfaceGroup,
    /// Объект верхнего уровня любого вида: правило поддержки.
    SupportRule,
    /// Модуль, его тело или метод: вставка текста. У метода место вставки —
    /// до или после него.
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
            Self::Metadata => "`Kind.Name`",
            Self::SupportRule => "`Kind.Name` or `Kind.Name.Form.F`",
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
            Self::FormItemContainer => "`Kind.Name.Form.F[.Item.I…]`",
            Self::FormItem => "`Kind.Name.Form.F.Item.I[.Item.J…]`",
            Self::FormEvent => "`Kind.Name.Form.F[.Item.I…].Event.E`",
            Self::Role => "`Role.Name`",
            Self::RoleRight => "`Role.Name.Right.<Kind>_<Name>`",
            Self::Subsystem => "`Subsystem.Name[.Subsystem.Child]`",
            Self::NestedSubsystem => "`Subsystem.Parent.Subsystem.Child`",
            Self::Interface => "`Subsystem.Name[.Subsystem.Child].Interface`",
            Self::DcsSchema => "`Kind.Name.Template.T` of a data composition schema",
            Self::DcsDataContainer => "`Kind.Name.Template.T[.DataSet.D]` of a schema",
            Self::DcsData => "`Kind.Name.Template.T.DataSet.D`",
            Self::DcsQuery => "`Kind.Name.Template.T.DataSet.D.Query`",
            Self::DcsSettings => "`Kind.Name.Template.T.Setting.S`",
            Self::DcsParameter => "`Kind.Name.Template.T.Parameter.P`",
            Self::DcsField => "`Kind.Name.Template.T.DataSet.D.Field.F`",
            Self::DcsDataSource => "`Kind.Name.Template.T.DataSource.S`",
            Self::DcsCalculatedField => "`Kind.Name.Template.T.CalculatedField.F`",
            Self::DcsTotal => "`Kind.Name.Template.T.Total.F`",
            Self::DcsSettingsContainer => "`Kind.Name.Template.T.Setting.S[.Item.I…]`",
            Self::DcsStructureItem => "`Kind.Name.Template.T.Setting.S.Item.I[.Item.J…]`",
            Self::Mxl => "`Kind.Name.Template.T` of a spreadsheet",
            Self::MxlArea => "`Kind.Name.Template.T.Area.A` of a spreadsheet",
            Self::XdtoPackage => "`XDTOPackage.Name`",
            Self::XdtoType => "`XDTOPackage.Name.Type.T`",
            Self::XdtoProperty => "`XDTOPackage.Name.Type.T.Property.P[.Property.Q…]`",
            Self::XdtoPropertyContainer => "`XDTOPackage.Name.Type.T[.Property.P…]`",
            Self::InterfaceGroup => "`Subsystem.Name[.Subsystem.Child].Interface.Group.G`",
            Self::CodeInsert => "a module address, its `Body` or `Method.M`",
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
            Self::FormItemContainer => site.form_items(segments).is_some(),
            Self::FormItem => site
                .form_items(segments)
                .is_some_and(|items| !items.is_empty()),
            Self::FormEvent => match segments {
                [path @ .., event] => {
                    event.kind() == NodeKind::Event
                        && event.name().is_some()
                        && site.form_items(path).is_some()
                }
                [] => false,
            },
            Self::Role => {
                site.route == RouteClass::Role
                    && matches!(segments, [role] if role.kind() == NodeKind::Role && role.name().is_some())
            }
            Self::RoleRight => {
                site.route == RouteClass::Role
                    && matches!(segments, [role, right]
                        if role.kind() == NodeKind::Role
                            && role.name().is_some()
                            && right.kind() == NodeKind::Right
                            && right.name().is_some())
            }
            Self::Subsystem => {
                site.route == RouteClass::Subsystem
                    && segments.iter().all(|segment| {
                        segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                    })
            }
            Self::NestedSubsystem => Self::Subsystem.matches(site) && site.segments().len() > 1,
            Self::Interface => {
                site.route == RouteClass::Interface
                    && terminal.kind() == NodeKind::Interface
                    && segments[..segments.len() - 1].iter().all(|segment| {
                        segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                    })
            }
            Self::DcsSchema | Self::Mxl => site.owned_template(),
            Self::MxlArea => {
                site.route == RouteClass::Mxl
                    && matches!(segments, [owner, template, area]
                        if owner.name().is_some()
                            && template.kind() == NodeKind::Template
                            && template.name().is_some()
                            && area.kind() == NodeKind::Area
                            && area.name().is_some()
                            && metadata_collection_matches(owner.kind(), MetaCollection::Templates))
            }
            Self::DcsDataContainer => {
                site.owned_template() || site.dcs_terminal(&[NodeKind::DataSet])
            }
            Self::DcsData => site.dcs_terminal(&[NodeKind::DataSet]),
            Self::DcsQuery => {
                site.route == RouteClass::Dcs
                    && matches!(segments, [.., data_set, query]
                        if query.kind() == NodeKind::Query
                            && query.name().is_none()
                            && data_set.kind() == NodeKind::DataSet
                            && data_set.name().is_some())
            }
            Self::DcsSettings => site.dcs_terminal(&[NodeKind::Setting]),
            Self::DcsParameter => site.dcs_terminal(&[NodeKind::Parameter]),
            Self::DcsField => site.dcs_terminal(&[NodeKind::Field]),
            Self::DcsDataSource => site.dcs_schema_member(NodeKind::DataSource),
            Self::DcsCalculatedField => site.dcs_schema_member(NodeKind::CalculatedField),
            Self::DcsTotal => site.dcs_schema_member(NodeKind::Total),
            Self::DcsSettingsContainer => site.dcs_structure_depth().is_some(),
            Self::DcsStructureItem => site.dcs_structure_depth().is_some_and(|depth| depth > 0),
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
            Self::XdtoProperty | Self::XdtoPropertyContainer => {
                site.route == RouteClass::Xdto
                    && match segments {
                        [package, ty, properties @ ..] => {
                            package.name().is_some()
                                && ty.kind() == NodeKind::Type
                                && ty.name().is_some()
                                && (self == Self::XdtoPropertyContainer || !properties.is_empty())
                                && properties.iter().all(|property| {
                                    property.kind() == NodeKind::Property
                                        && property.name().is_some()
                                })
                        }
                        _ => false,
                    }
            }
            Self::InterfaceGroup => {
                site.route == RouteClass::Interface
                    && matches!(segments, [owners @ .., interface, group]
                    if interface.kind() == NodeKind::Interface
                        && group.kind() == NodeKind::Group
                        && group.name().is_some()
                        && !owners.is_empty()
                        && owners.iter().all(|segment| {
                            segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                        }))
            }
            Self::SupportRule => match segments {
                [owner] | [owner, _] => {
                    owner.name().is_some()
                        && owner.kind().is_metadata_kind()
                        && !matches!(
                            owner.kind(),
                            NodeKind::Form | NodeKind::Template | NodeKind::Command
                        )
                        && (segments.len() == 1
                            || (terminal.kind() == NodeKind::Form && terminal.name().is_some()))
                }
                _ => false,
            },
            Self::CodeInsert => {
                site.route == RouteClass::Module
                    && (module_terminal(segments)
                        || (terminal.kind() == NodeKind::Body && !method_body(segments))
                        || (terminal.kind() == NodeKind::Method && terminal.name().is_some()))
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

/// Роль элемента формы для словаря операций. Единственный владелец
/// отображения: `view` узнаёт роль по `props.tag` элемента, планировщик —
/// по тому же признаку элемента прочитанной формы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormItemRole {
    /// Принимает вложенные элементы: группа, страницы, страница, таблица,
    /// группа колонок, командная панель, группа кнопок, подменю.
    Container,
    /// Автоматическая командная панель формы или таблицы: в неё добавляются
    /// кнопки, но удалить её отдельно от владельца нельзя.
    AutoCommandBar,
    /// Поле, декорация, кнопка: вложенных элементов не принимает.
    Leaf,
}

impl FormItemRole {
    /// Роль по признаку `tag` элемента, как его публикует `view`
    /// (`[Group:V]`, `[Table]`, `[AutoCommandBar]`, `[Input]`).
    pub(crate) fn from_tag(tag: &str) -> Self {
        let kind = tag
            .trim_start_matches('[')
            .split([':', ',', ']'])
            .next()
            .unwrap_or_default();
        match kind {
            "AutoCommandBar" => Self::AutoCommandBar,
            "Group" | "Table" | "ColumnGroup" | "CmdBar" | "Pages" | "Page" | "Popup"
            | "BtnGroup" => Self::Container,
            _ => Self::Leaf,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedOperation {
    Metadata(MetadataOperation),
    Form(FormOperation),
    Role(RoleOperation),
    Support(SupportOperation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationDescriptor {
    name: &'static str,
    family: OperationFamily,
    applicability: OperationApplicability,
    skeleton: OperationSkeleton,
    typed_operation: Option<TypedOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationSkeleton {
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
    "area.add",
    "event.implement",
    "form.add",
    "form.set",
    "form.remove",
    "item.add",
    "item.remove",
    "item.move",
    "event.bind",
    "right.add",
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
    const fn metadata(operation: MetadataOperation) -> Self {
        Self {
            name: operation.name(),
            family: OperationFamily::Metadata,
            applicability: operation.applicability(),
            skeleton: operation.skeleton(),
            typed_operation: Some(TypedOperation::Metadata(operation)),
        }
    }

    pub(crate) const fn metadata_operation(self) -> Option<MetadataOperation> {
        match self.typed_operation {
            Some(TypedOperation::Metadata(operation)) => Some(operation),
            _ => None,
        }
    }

    const fn form(operation: FormOperation) -> Self {
        Self {
            name: operation.name(),
            family: OperationFamily::Form,
            applicability: operation.applicability(),
            skeleton: operation.skeleton(),
            typed_operation: Some(TypedOperation::Form(operation)),
        }
    }

    pub(crate) const fn form_operation(self) -> Option<FormOperation> {
        match self.typed_operation {
            Some(TypedOperation::Form(operation)) => Some(operation),
            _ => None,
        }
    }

    const fn role(operation: RoleOperation) -> Self {
        Self {
            name: operation.name(),
            family: OperationFamily::Role,
            applicability: operation.applicability(),
            skeleton: operation.skeleton(),
            typed_operation: Some(TypedOperation::Role(operation)),
        }
    }

    const fn support(operation: SupportOperation) -> Self {
        Self {
            name: operation.name(),
            family: OperationFamily::Support,
            applicability: operation.applicability(),
            skeleton: operation.skeleton(),
            typed_operation: Some(TypedOperation::Support(operation)),
        }
    }

    pub(crate) const fn role_operation(self) -> Option<RoleOperation> {
        match self.typed_operation {
            Some(TypedOperation::Role(operation)) => Some(operation),
            _ => None,
        }
    }

    pub(crate) const fn support_operation(self) -> Option<SupportOperation> {
        match self.typed_operation {
            Some(TypedOperation::Support(operation)) => Some(operation),
            _ => None,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        self.name
    }

    pub(crate) const fn skeleton_key(self) -> &'static str {
        self.skeleton.key()
    }

    pub(crate) const fn family(self) -> OperationFamily {
        self.family
    }

    /// Сопутствующий файл, содержимое которого пишет операция: права роли,
    /// описание формы (её модуль и обработчики событий тоже), тело макета или
    /// пакет XDTO. У заимствованного объекта без такого файла содержимое
    /// унаследовано от родителя, и операция там не исполнима. `None` —
    /// операция пишет описание объекта или вне такого содержимого.
    pub(crate) fn content_resource(self) -> Option<&'static str> {
        match self.family {
            OperationFamily::Role => self
                .role_operation()
                .and_then(RoleOperation::content_resource),
            OperationFamily::Form => self
                .form_operation()
                .and_then(FormOperation::content_resource),
            OperationFamily::Event | OperationFamily::Code => Some("Form.xml"),
            OperationFamily::Dcs | OperationFamily::Mxl => Some("Template.xml"),
            OperationFamily::Xdto => Some("Package.bin"),
            _ => None,
        }
    }

    /// Применима ли операция к месту: вид набора, маршрут и форма адреса.
    /// Формат макета этим ответом не проверяется — см. `admits_template_format`.
    pub(crate) fn applies_at(self, site: &OperationSite<'_>) -> bool {
        site.at.line_range().is_none()
            && site.source.writes(self.family)
            && self.applicability.matches(site)
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

    /// Фильтр роли элемента формы. Элемент добавляется только в контейнер
    /// (группу, страницы, страницу, таблицу, группу колонок, командную панель), а
    /// автоматическая командная панель — часть своего владельца и не
    /// удаляется. `None` — роль не установлена.
    pub(crate) fn admits_form_item(self, role: Option<FormItemRole>) -> bool {
        self.form_operation()
            .is_none_or(|operation| operation.admits_form_item(role))
    }

    /// Применимость к прочитанному узлу: место и, для самого макета, формат
    /// его тела, для элемента формы — его роль. Этим ответом строится
    /// словарь `can`.
    pub(crate) fn applies_to_node(
        self,
        site: &OperationSite<'_>,
        template_format: Option<TemplateFormat>,
        form_item: Option<FormItemRole>,
    ) -> bool {
        self.applies_at(site)
            && (site.terminal().kind() != NodeKind::Template
                || self.admits_template_format(template_format))
            && (site.terminal().kind() != NodeKind::Item || self.admits_form_item(form_item))
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
        self.nearest_applicable_ancestor_with(site, template_format, &|_| None)
    }

    /// То же с ролью элемента-предка формы: `form_item` читает её у
    /// вызывающего, и `next` не уводит в поле, куда элемент не добавить.
    pub(crate) fn nearest_applicable_ancestor_with(
        self,
        site: &OperationSite<'_>,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<QualifiedAddress> {
        let segments = site.segments();
        // Запрос — безымянный узел набора: операцию запроса, вызванную на
        // самом наборе, отказ ведёт к его `Query`.
        // Только у набора: после вида с именем (`Group`, `Property`) `.Query`
        // разобралось бы как имя несуществующего узла.
        let query = (site.terminal().kind() == NodeKind::DataSet
            && site.terminal().name().is_some())
        .then(|| QualifiedAddress::parse(&format!("{}.Query", site.at)).ok())
        .flatten();
        let mut candidates = query
            .into_iter()
            .chain((1..segments.len()).rev().map(|length| QualifiedAddress {
                line_range: None,
                source_set: site.at.source_set().to_string(),
                segments: segments[..length].to_vec(),
            }))
            .collect::<Vec<_>>();
        if site.route != RouteClass::Configuration {
            candidates.push(configuration_root(site.at.source_set()));
        }
        candidates.into_iter().find(|candidate| {
            OperationSite::new(candidate, site.source).is_some_and(|candidate| {
                self.applies_at(&candidate)
                    && (candidate.terminal().kind() != NodeKind::Template
                        || self.admits_template_format(template_format(candidate.at)))
                    && (candidate.terminal().kind() != NodeKind::Item
                        || self.admits_form_item(form_item(candidate.at)))
            })
        })
    }

    /// Операция родителя, которая создаёт узел этой операции: `*.set`
    /// пишет только в существующий узел, а отсутствующий создаёт `*.add`
    /// на родителе с именем в аргументах (#1354).
    pub(crate) const fn creator(self) -> Option<&'static str> {
        match self.name.as_bytes() {
            b"right.set" => Some("right.add"),
            b"mxl.set" => Some("area.add"),
            _ => None,
        }
    }

    /// Адрес, на котором операция исполнима вместо снятого псевдонима
    /// запроса `Template.T.Query.D` (#1353): запрос набора
    /// `Template.T.DataSet.D.Query` или сам набор.
    pub(crate) fn retired_query_alias_target(
        self,
        site: &OperationSite<'_>,
        data_sets: MemberPathLookup<'_>,
    ) -> Option<QualifiedAddress> {
        let segments = site.segments();
        let position = segments
            .iter()
            .position(|segment| segment.kind() == NodeKind::Query && segment.name().is_some())?;
        if site.route != RouteClass::Dcs
            || position == 0
            || segments[position - 1].kind() != NodeKind::Template
        {
            return None;
        }
        let name = segments[position].name()?;
        let template = QualifiedAddress {
            line_range: None,
            source_set: site.at.source_set().to_string(),
            segments: segments[..position].to_vec(),
        };
        let NamedNode::Found(data_set) = named_data_set(&template, name, data_sets)? else {
            return None;
        };
        [".Query", ""].into_iter().find_map(|suffix| {
            let candidate = QualifiedAddress::parse(&format!("{data_set}{suffix}")).ok()?;
            OperationSite::new(&candidate, site.source)
                .is_some_and(|target| self.applies_at(&target))
                .then_some(candidate)
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
            OperationFamily::Metadata => match self.metadata_operation() {
                Some(operation) => Some(operation.contract_at(None)),
                None => super::operation_contract::OperationContract::metadata(self.name),
            },
            OperationFamily::Role => self.role_operation().map(RoleOperation::contract),
            OperationFamily::Support => self.support_operation().map(SupportOperation::contract),
            OperationFamily::Form => self.form_operation().map(FormOperation::contract),
            OperationFamily::Event => {
                super::operation_contract::OperationContract::event(self.name)
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
            let metadata_site =
                owner.map(|owner| super::operation_contract::MetadataSite { owner, in_section });
            return match self.metadata_operation() {
                Some(operation) => Some(operation.contract_at(metadata_site)),
                None => super::operation_contract::OperationContract::metadata_at(
                    self.name,
                    metadata_site,
                ),
            };
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

/// Const equality for the registration gate; operation names remain owned by
/// each typed declaration table.
pub(super) const fn operation_names_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

macro_rules! operation_descriptor {
    (($name:literal, Role, $applicability:ident, $skeleton:ident)) => {
        compile_error!(
            "Role operations must register a typed RoleOperation with a mandatory contract"
        )
    };
    (($name:literal, Support, $applicability:ident, $skeleton:ident)) => {
        compile_error!(
            "Support operations must register a typed SupportOperation with a mandatory contract"
        )
    };
    (($name:literal, Form, $applicability:ident, $skeleton:ident)) => {
        compile_error!(
            "Form operations must register a typed FormOperation with a mandatory contract"
        )
    };
    ((RoleOperation::$variant:ident)) => {
        OperationDescriptor::role(RoleOperation::$variant)
    };
    ((SupportOperation::$variant:ident)) => {
        OperationDescriptor::support(SupportOperation::$variant)
    };
    ((MetadataOperation::$variant:ident)) => {
        OperationDescriptor::metadata(MetadataOperation::$variant)
    };
    ((FormOperation::$variant:ident)) => {
        OperationDescriptor::form(FormOperation::$variant)
    };
    (($name:literal, $family:ident, $applicability:ident, $skeleton:ident)) => {{
        const {
            match OperationFamily::$family {
                OperationFamily::Metadata => assert!(
                    !MetadataOperation::is_named($name),
                    "Typed Metadata operations must register a MetadataOperation with a mandatory contextual contract"
                ),
                _ => (),
            }
        }
        OperationDescriptor {
            name: $name,
            family: OperationFamily::$family,
            applicability: OperationApplicability::$applicability,
            skeleton: OperationSkeleton::$skeleton,
            typed_operation: None,
        }
    }};
}

macro_rules! operation_descriptors {
    ($($entry:tt),+ $(,)?) => {
        const OPERATION_DESCRIPTORS: &[OperationDescriptor] = &[
            $(operation_descriptor!($entry)),+
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
    (MetadataOperation::AttributeAdd),
    (MetadataOperation::AttributeSet),
    (MetadataOperation::AttributeRemove),
    (MetadataOperation::TabularSectionAdd),
    (MetadataOperation::TabularSectionSet),
    (MetadataOperation::TabularSectionRemove),
    (MetadataOperation::DimensionAdd),
    (MetadataOperation::DimensionSet),
    (MetadataOperation::DimensionRemove),
    (MetadataOperation::ResourceAdd),
    (MetadataOperation::ResourceSet),
    (MetadataOperation::ResourceRemove),
    (MetadataOperation::EnumValueAdd),
    (MetadataOperation::EnumValueSet),
    (MetadataOperation::EnumValueRemove),
    (MetadataOperation::ColumnAdd),
    (MetadataOperation::ColumnSet),
    (MetadataOperation::ColumnRemove),
    (FormOperation::Add),
    (FormOperation::Set),
    (FormOperation::Remove),
    ("template.add", Metadata, MetadataTemplates, Items),
    ("template.set", Metadata, MetadataTemplate, Values),
    ("template.remove", Metadata, MetadataTemplate, Target),
    (MetadataOperation::CommandAdd),
    (MetadataOperation::CommandSet),
    (MetadataOperation::CommandRemove),
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
    (FormOperation::ItemAdd),
    (FormOperation::ItemRemove),
    (FormOperation::ItemMove),
    (FormOperation::AttributeAdd),
    (FormOperation::CommandAdd),
    (FormOperation::EventBind),
    (RoleOperation::Create),
    (RoleOperation::RightAdd),
    (RoleOperation::RightSet),
    ("dataSource.add", Dcs, DcsSchema, Items),
    ("dataSource.set", Dcs, DcsDataSource, Values),
    ("dataSource.remove", Dcs, DcsDataSource, Target),
    ("dataSet.add", Dcs, DcsDataContainer, Items),
    ("dataSet.set", Dcs, DcsData, Values),
    ("dataSet.remove", Dcs, DcsData, Target),
    ("field.add", Dcs, DcsData, Items),
    ("field.set", Dcs, DcsField, Values),
    ("field.remove", Dcs, DcsField, Target),
    ("fieldRole.set", Dcs, DcsField, Values),
    ("parameter.add", Dcs, DcsSchema, Items),
    ("parameter.set", Dcs, DcsParameter, Values),
    ("parameter.remove", Dcs, DcsParameter, Target),
    ("parameter.rename", Dcs, DcsParameter, Values),
    ("parameter.reorder", Dcs, DcsSchema, Items),
    ("calculatedField.add", Dcs, DcsSchema, Items),
    ("calculatedField.remove", Dcs, DcsCalculatedField, Target),
    ("total.add", Dcs, DcsSchema, Items),
    ("total.remove", Dcs, DcsTotal, Target),
    ("variant.add", Dcs, DcsSchema, Items),
    ("variant.set", Dcs, DcsSettings, Values),
    ("variant.remove", Dcs, DcsSettings, Target),
    ("query.set", Dcs, DcsQuery, Values),
    ("query.patch", Dcs, DcsQuery, Values),
    ("filter.add", Dcs, DcsSettingsContainer, Items),
    ("filter.set", Dcs, DcsSettingsContainer, Values),
    ("filter.remove", Dcs, DcsSettingsContainer, Values),
    ("filter.clear", Dcs, DcsSettingsContainer, Values),
    ("selection.add", Dcs, DcsSettingsContainer, Items),
    ("selection.clear", Dcs, DcsSettingsContainer, Values),
    ("order.add", Dcs, DcsSettingsContainer, Items),
    ("order.clear", Dcs, DcsSettingsContainer, Values),
    // Параметры данных есть только у самих настроек варианта: у элемента
    // структуры их коллекции нет.
    ("dataParameter.add", Dcs, DcsSettings, Items),
    ("dataParameter.set", Dcs, DcsSettings, Values),
    ("outputParameter.set", Dcs, DcsSettingsContainer, Values),
    (
        "conditionalAppearance.add",
        Dcs,
        DcsSettingsContainer,
        Items
    ),
    (
        "conditionalAppearance.clear",
        Dcs,
        DcsSettingsContainer,
        Values
    ),
    ("structure.add", Dcs, DcsSettingsContainer, Items),
    ("structure.set", Dcs, DcsSettings, Values),
    ("structure.patch", Dcs, DcsStructureItem, Values),
    ("structure.remove", Dcs, DcsStructureItem, Target),
    ("dataSetLink.add", Dcs, DcsSchema, Items),
    ("calculatedField.set", Dcs, DcsCalculatedField, Values),
    ("total.set", Dcs, DcsTotal, Values),
    ("selection.set", Dcs, DcsSettingsContainer, Values),
    ("selection.remove", Dcs, DcsSettingsContainer, Values),
    ("order.set", Dcs, DcsSettingsContainer, Values),
    ("order.remove", Dcs, DcsSettingsContainer, Values),
    ("dataParameter.remove", Dcs, DcsSettings, Values),
    ("outputParameter.remove", Dcs, DcsSettingsContainer, Values),
    (
        "conditionalAppearance.remove",
        Dcs,
        DcsSettingsContainer,
        Values
    ),
    ("dataSetLink.set", Dcs, DcsSchema, Values),
    ("dataSetLink.remove", Dcs, DcsSchema, Values),
    ("mxl.set", Mxl, MxlArea, Values),
    ("area.add", Mxl, Mxl, Items),
    ("valueType.add", Xdto, XdtoPackage, Values),
    ("objectType.add", Xdto, XdtoPackage, Values),
    ("property.add", Xdto, XdtoPropertyContainer, Values),
    ("type.remove", Xdto, XdtoType, Target),
    ("property.remove", Xdto, XdtoProperty, Target),
    ("subsystem.create", Subsystem, Configuration, Values),
    ("content.add", Subsystem, Subsystem, Items),
    ("content.remove", Subsystem, Subsystem, Target),
    ("childSubsystem.add", Subsystem, Subsystem, Items),
    ("childSubsystem.remove", Subsystem, NestedSubsystem, Target),
    (SupportOperation::CapabilitySet),
    (SupportOperation::RuleSet),
    ("commandVisibility.set", Interface, Interface, Items),
    ("commandPlacement.set", Interface, Interface, Items),
    ("commandOrder.set", Interface, InterfaceGroup, Values),
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
/// когда операция переезжает на адрес подузла, а аргумент, дублировавший
/// или переопределявший `at`, — без переезда (П2, #1353). Таблица — единственный
/// владелец этого отображения: предварительная проверка `apply` читает её до
/// вердикта «не тот узел», и старый вызов получает `bad_value` с адресом в
/// `next`, а не тупик «операция не применима здесь».
///
/// `key` — путь внутри `args`: точки разделяют ключи объектов, `[]` обходит
/// массив (`items[].into`), а `items[]` без продолжения берёт элемент-строку. В `address` подставляются `{at}` — адрес вызова,
/// `{at^Kind}` — начало адреса вызова до последнего сегмента вида `Kind`
/// включительно (`Module` — и общий модуль), и `{value}` — строковое
/// значение снятого ключа. `{right}` — то же значение, где первая точка
/// заменена на `_`: объект роли `Catalog.Items` даёт имя записи прав
/// `Catalog_Items`. Если в адресе вызова нет сегмента `Kind`, запись
/// адреса не строит. `{dataSet}` — набор схемы, названный значением: сегмент
/// `DataSet` адреса вызова с этим именем, иначе набор, найденный по имени
/// в схеме макета (вложенный набор объединения тоже), иначе набор верхнего
/// уровня; если наборов с этим именем несколько, отказ ведёт к ветви наборов
/// макета и перечисляет их адреса. `{item}` — элемент, названный значением,
/// под ближайшим контейнером адреса вызова: в форме — путь `Item.A.Item.B`
/// элемента с этим именем, в варианте настроек — цепочка именованных
/// элементов структуры `Setting.V.Item.<имя>[.Item.<имя>…]`; не найденное
/// имя даёт элемент верхнего уровня `Item.<значение>`, повторённое ведёт
/// к ветви `Item` контейнера и перечисляет совпавшие элементы. `{event}` —
/// событие формы, названное `values.event` (русское имя переводится), иначе
/// событие самого адреса вызова; без них запись адреса не строит.
/// `{at-Event}` — адрес вызова без последнего сегмента `Event`. Шаблон, который
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
    // Аргументы, дублировавшие `at` (П2, #1353). Значение называло узел
    // в обход адреса: отказ ведёт к узлу, который оно называло.
    // `{dataSet}` — набор, названный значением: см. `RetiredArgument`.
    ("query.set", "values.dataSet", "{dataSet}.Query", true),
    ("query.patch", "values.dataSet", "{dataSet}.Query", true),
    (
        "structure.set",
        "values.variant",
        "{at^Template}.Setting.{value}",
        true
    ),
    // Правка элемента структуры переехала на его адрес (#1357): вариант,
    // названный аргументом, ведёт к элементам его структуры.
    (
        "structure.patch",
        "values.variant",
        "{at^Template}.Setting.{value}.Item.{name}",
        true
    ),
    (
        "childSubsystem.remove",
        "items[].name",
        "{at}.Subsystem.{value}",
        true
    ),
    (
        "childSubsystem.remove",
        "items[]",
        "{at}.Subsystem.{value}",
        true
    ),
    (
        "childSubsystem.remove",
        "values.name",
        "{at}.Subsystem.{value}",
        true
    ),
    (
        "code.replace",
        "selector.method",
        "{at^Module}.Method.{value}",
        true
    ),
    // Адресация (П5, #1356). Свойство XDTO любой вложенности задаётся
    // адресом `Type.T.Property.A[.Property.B…]`: прежний `propertyPath`
    // и имя удаляемого свойства переходят в `at`. Путь с удаляемым именем
    // ведёт к коллекции свойств, где видно это имя.
    (
        "property.add",
        "values.propertyPath",
        "{at}.Property.{path}",
        true
    ),
    (
        "property.remove",
        "values.propertyPath",
        "{at}.Property.{path}.Property.{name}",
        true
    ),
    (
        "property.remove",
        "values.name",
        "{at}.Property.{value}",
        true
    ),
    // Плоская форма XDTO без `values` ещё разбирается: её ключи снимаются
    // так же.
    ("property.add", "propertyPath", "{at}.Property.{path}", true),
    (
        "property.remove",
        "propertyPath",
        "{at}.Property.{path}.Property.{name}",
        true
    ),
    ("property.remove", "name", "{at}.Property.{value}", true),
    // Порядок команд принадлежит группе: она задаётся адресом
    // `Interface.Group.<g>`, имя с точкой (`CommandGroup.X`) экранируется.
    (
        "commandOrder.set",
        "values.group",
        "{at}.Group.{value}",
        true
    ),
    // Операции подузлов (П3, #1354): подузел, прежде названный аргументом
    // на владельце, задаётся адресом. Узел создаёт операция родителя
    // (`right.add`, `area.add`), а `*.set` пишет только в существующий.
    ("right.set", "values.object", "{at}.Right.{right}", true),
    ("mxl.set", "values.area", "{at}.Area.{value}", true),
    (
        "variant.set",
        "values.name",
        "{at^Template}.Setting.{value}",
        true
    ),
    (
        "variant.remove",
        "values.name",
        "{at^Template}.Setting.{value}",
        true
    ),
    (
        "parameter.set",
        "values.name",
        "{at^Template}.Parameter.{value}",
        true
    ),
    (
        "parameter.rename",
        "values.name",
        "{at^Template}.Parameter.{value}",
        true
    ),
    // Составной `dataPath` (`Owner.Code`) становится одним экранированным
    // именем поля: `Field.Owner\.Code`.
    (
        "field.set",
        "values.field",
        "{at^DataSet}.Field.{value}",
        true
    ),
    (
        "field.set",
        "values.dataPath",
        "{at^DataSet}.Field.{value}",
        true
    ),
    (
        "field.set",
        "values.name",
        "{at^DataSet}.Field.{value}",
        true
    ),
    (
        "fieldRole.set",
        "values.field",
        "{at^DataSet}.Field.{value}",
        true
    ),
    (
        "fieldRole.set",
        "values.dataPath",
        "{at^DataSet}.Field.{value}",
        true
    ),
    (
        "fieldRole.set",
        "values.name",
        "{at^DataSet}.Field.{value}",
        true
    ),
    (
        "code.insert",
        "selector.method",
        "{at^Module}.Method.{value}",
        true
    ),
    // Новые узлы СКД (П6, #1357): источник, вычисляемое поле и итог
    // адресуются под макетом, элемент структуры варианта — цепочкой
    // `Setting.V.Item.<имя>[.Item.<имя>…]`. `{item}` — элемент, названный
    // значением в структуре варианта адреса вызова: см. `RetiredArgument`.
    (
        "dataSource.set",
        "values.name",
        "{at^Template}.DataSource.{value}",
        true
    ),
    (
        "dataSource.remove",
        "values.name",
        "{at^Template}.DataSource.{value}",
        true
    ),
    (
        "calculatedField.set",
        "values.name",
        "{at^Template}.CalculatedField.{value}",
        true
    ),
    (
        "calculatedField.remove",
        "values.name",
        "{at^Template}.CalculatedField.{value}",
        true
    ),
    (
        "total.set",
        "values.field",
        "{at^Template}.Total.{value}",
        true
    ),
    (
        "total.remove",
        "values.name",
        "{at^Template}.Total.{value}",
        true
    ),
    ("filter.add", "items[].group", "{item}", true),
    ("filter.set", "values.group", "{item}", true),
    ("filter.remove", "values.group", "{item}", true),
    ("filter.clear", "values.group", "{item}", true),
    ("selection.add", "items[].group", "{item}", true),
    ("selection.set", "values.group", "{item}", true),
    ("selection.remove", "values.group", "{item}", true),
    ("selection.clear", "values.group", "{item}", true),
    ("order.add", "items[].group", "{item}", true),
    ("order.set", "values.group", "{item}", true),
    ("order.remove", "values.group", "{item}", true),
    ("order.clear", "values.group", "{item}", true),
    ("conditionalAppearance.add", "items[].group", "{item}", true),
    (
        "conditionalAppearance.remove",
        "values.group",
        "{item}",
        true
    ),
    (
        "conditionalAppearance.clear",
        "values.group",
        "{item}",
        true
    ),
    ("outputParameter.set", "values.group", "{item}", true),
    ("outputParameter.remove", "values.group", "{item}", true),
    ("structure.add", "items[].parent", "{item}", true),
    ("structure.patch", "values.name", "{item}", true),
    ("structure.remove", "values.name", "{item}", true),
    // Формы (П4, #1355): контейнер нового элемента, удаляемый элемент и
    // событие задаются адресом `Item…Item[.Event]`. Позиция `after` остаётся
    // именем соседа.
    ("item.add", "items[].into", "{item}", true),
    ("item.remove", "items[].name", "{item}", true),
    ("item.remove", "items[]", "{item}", true),
    ("item.remove", "values.name", "{item}", true),
    ("event.bind", "values.element", "{item}.Event.{event}", true),
    // Без имени события `next` ведёт к событиям элемента, где видны имена.
    ("event.bind", "values.element", "{item}.Event.{name}", true),
    (
        "event.bind",
        "values.event",
        "{at-Event}.Event.{event}",
        true
    ),
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
    /// Значение называет несколько узлов: `at` — их ветвь, здесь — адреса
    /// узлов, где операция исполнима. Пусто — значение назвало один узел.
    pub(crate) candidates: Vec<QualifiedAddress>,
    /// Узлов с таким именем несколько: `at` — ветвь, где они видны.
    pub(crate) ambiguous: bool,
}

/// Подстановка цепочки имён: значение — путь, где неэкранированная точка
/// разделяет имена, а `\.` — буквальная точка имени (прежний
/// `propertyPath` XDTO). Каждая точка-разделитель становится переходом к
/// члену того же вида, что стоит перед подстановкой: `{at}.Property.{path}`
/// со значением `A\.B.C` — `{at}.Property.A\.B.Property.C`.
const PATH_PLACEHOLDER: &str = "{path}";

/// Путь из значения в записи адреса; `None` — путь пуст, содержит пустое имя
/// или экранирование, которого грамматика адреса не знает.
fn escaped_path(value: &str, kind: &str) -> Option<String> {
    let mut names = vec![String::new()];
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                characters.next().filter(|next| *next == '.')?;
                names.last_mut()?.push('.');
            }
            '.' => names.push(String::new()),
            other => names.last_mut()?.push(other),
        }
    }
    if names.iter().any(String::is_empty) {
        return None;
    }
    Some(
        names
            .iter()
            .map(|name| escape_name(name).into_owned())
            .collect::<Vec<_>>()
            .join(&format!(".{kind}.")),
    )
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
        self.redirect_with(at, args, &no_member_lookup)
    }

    /// То же, что `redirect`, с поиском набора схемы и элемента формы по имени.
    pub(crate) fn redirect_with(
        self,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
        members: MemberPathLookup<'_>,
    ) -> Option<ArgumentRedirect> {
        let (location, value) = find_key(&Value::Object(args.clone()), self.key, String::new())?;
        // Шаблон подставляется строкой и разбирается заново; имя из значения
        // проходит `escape_name`, поэтому точка в нём остаётся внутри имени.
        // Событие и владелец события подставляются до поиска узла: суффикс
        // шаблона за найденным узлом уже не содержит подстановок.
        let template = if self.address.contains("{event}") {
            self.address
                .replace("{event}", &escape_name(&named_form_event(at, args)?))
        } else {
            self.address.to_string()
        };
        let template = template.replace("{at-Event}", &without_event(at).to_string());
        let named = if template.contains("{dataSet}") {
            Some(("{dataSet}", named_data_set(at, &value, members)?))
        } else if template.contains("{item}") {
            Some(("{item}", named_item(at, &value, members)?))
        } else {
            None
        };
        let raw = match named {
            Some((placeholder, NamedNode::Found(node))) => {
                template.replace(placeholder, &node.to_string())
            }
            Some((placeholder, NamedNode::Ambiguous { branch, candidates })) => {
                let suffix = template.split_once(placeholder)?.1;
                let candidates = candidates
                    .iter()
                    .map(|candidate| QualifiedAddress::parse(&format!("{candidate}{suffix}")))
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                return Some(ArgumentRedirect {
                    location,
                    at: branch,
                    listing: false,
                    candidates,
                    ambiguous: true,
                });
            }
            None => expand_anchor(&template, at)?.replace("{at}", &at.to_string()),
        };
        let raw = match raw.find(PATH_PLACEHOLDER) {
            Some(index) => {
                let separator = raw[..index].rsplit('.').nth(1)?;
                raw.replace(PATH_PLACEHOLDER, &escaped_path(&value, separator)?)
            }
            None => raw,
        };
        if let Some(collection) = raw.strip_suffix(".{name}") {
            return Some(ArgumentRedirect {
                location,
                at: QualifiedAddress::parse(&collection.replace("{value}", &escape_name(&value)))
                    .ok()?,
                listing: true,
                candidates: Vec::new(),
                ambiguous: false,
            });
        }
        // Запись прав роли называется `<Вид>_<Имя>[.Вложенный…]`: первая
        // точка ссылки на объект становится `_`, остальные экранируются.
        let target = QualifiedAddress::parse(
            &raw.replace("{value}", &escape_name(&value))
                .replace("{right}", &escape_name(&value.replacen('.', "_", 1))),
        )
        .ok()?;
        Some(ArgumentRedirect {
            location,
            at: target,
            listing: false,
            candidates: Vec::new(),
            ambiguous: false,
        })
    }
}

impl ArgumentRedirect {
    /// Адрес, на котором операция исполнима: для записи с `listing` — член
    /// коллекции с подставленным именем.
    fn executable_at(&self) -> Option<QualifiedAddress> {
        if let Some(candidate) = self.candidates.first() {
            return Some(candidate.clone());
        }
        if !self.listing && !self.ambiguous {
            return Some(self.at.clone());
        }
        QualifiedAddress::parse(&format!("{}.{LISTED_MEMBER}", self.at)).ok()
    }
}

/// Поиск узла по имени под контейнером. Контейнер — последний сегмент
/// адреса: макет (набор данных, в том числе член объединения), вариант
/// настроек (именованный элемент структуры) или форма (элемент формы).
pub(crate) type MemberPathLookup<'a> = &'a dyn Fn(&QualifiedAddress, &str) -> NamedLookup;

/// Ответ поиска узла по имени: путь имён от контейнера (`["A", "B"]` —
/// `DataSet.A.DataSet.B` под макетом или `Item.A.Item.B` под формой
/// и вариантом настроек).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NamedLookup {
    /// Узел с таким именем один.
    Found(Vec<String>),
    /// Такого узла нет или тело контейнера не читается.
    Missing,
    /// Узлов с таким именем несколько; пути тех, что адресуемы.
    Ambiguous(Vec<Vec<String>>),
}

/// Поиск, который ничего не знает о телах контейнеров.
pub(crate) fn no_member_lookup(_: &QualifiedAddress, _: &str) -> NamedLookup {
    NamedLookup::Missing
}

/// Узел, который назвало значение снятого аргумента.
enum NamedNode {
    Found(QualifiedAddress),
    /// Имя повторяется: ветвь узлов этого вида и адреса совпавших.
    Ambiguous {
        branch: QualifiedAddress,
        candidates: Vec<QualifiedAddress>,
    },
}

/// Адрес владельца с путём имён: `{owner}.Kind.A.Kind.B`.
fn owner_path(owner: &QualifiedAddress, kind: &str, path: &[String]) -> Option<QualifiedAddress> {
    QualifiedAddress::parse(&format!(
        "{owner}{}",
        path.iter()
            .map(|name| format!(".{kind}.{}", escape_name(name)))
            .collect::<String>()
    ))
    .ok()
}

/// Начало адреса до последнего сегмента, вид которого подходит `kind`,
/// включительно.
fn prefix_through(
    at: &QualifiedAddress,
    kind: impl Fn(NodeKind) -> bool,
) -> Option<QualifiedAddress> {
    let length = at
        .segments()
        .iter()
        .rposition(|segment| kind(segment.kind()))?
        + 1;
    Some(QualifiedAddress {
        line_range: None,
        source_set: at.source_set().to_string(),
        segments: at.segments()[..length].to_vec(),
    })
}

/// Найденное по имени: один узел, несколько или ни одного (тогда — узел
/// верхнего уровня с этим именем, адрес которого `view` честно не найдёт).
fn resolve_named(
    owner: &QualifiedAddress,
    kind: &str,
    name: &str,
    lookup: MemberPathLookup<'_>,
) -> Option<NamedNode> {
    match lookup(owner, name) {
        NamedLookup::Found(path) => owner_path(owner, kind, &path).map(NamedNode::Found),
        NamedLookup::Missing => owner_path(owner, kind, &[name.to_string()]).map(NamedNode::Found),
        NamedLookup::Ambiguous(paths) => Some(NamedNode::Ambiguous {
            branch: QualifiedAddress::parse(&format!("{owner}.{kind}")).ok()?,
            candidates: paths
                .iter()
                .map(|path| owner_path(owner, kind, path))
                .collect::<Option<Vec<_>>>()?,
        }),
    }
}

/// Набор схемы, названный значением снятого `dataSet`: сегмент адреса вызова
/// с этим именем; иначе найденный в схеме макета; иначе набор верхнего уровня.
/// Несколько наборов с этим именем (члены разных объединений) — ветвь
/// наборов макета и их адреса (#1357).
fn named_data_set(
    at: &QualifiedAddress,
    name: &str,
    data_sets: MemberPathLookup<'_>,
) -> Option<NamedNode> {
    let segments = at.segments();
    if let Some(index) = segments
        .iter()
        .rposition(|segment| segment.kind() == NodeKind::DataSet && segment.name() == Some(name))
    {
        return Some(NamedNode::Found(QualifiedAddress {
            line_range: None,
            source_set: at.source_set().to_string(),
            segments: segments[..=index].to_vec(),
        }));
    }
    let template = prefix_through(at, |kind| kind == NodeKind::Template)?;
    resolve_named(&template, "DataSet", name, data_sets)
}

/// Элемент, названный значением снятого аргумента, под ближайшим
/// контейнером адреса вызова: элемент формы (`into`, `name`, `element`)
/// или именованный элемент структуры варианта настроек (`group`, `parent`,
/// `name`).
fn named_item(at: &QualifiedAddress, name: &str, items: MemberPathLookup<'_>) -> Option<NamedNode> {
    let container = prefix_through(at, |kind| {
        matches!(kind, NodeKind::Form | NodeKind::Setting)
    })?;
    resolve_named(&container, "Item", name, items)
}

/// Событие формы для снятого аргумента `event.bind`: `values.event` в имени
/// реестра, иначе событие самого адреса вызова.
fn named_form_event(at: &QualifiedAddress, args: &Map<String, Value>) -> Option<String> {
    if let Some(event) = args
        .get("values")
        .and_then(|values| values.get("event"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|event| !event.is_empty())
    {
        return Some(crate::domain::form_edit::canonical_form_event_name(event).to_string());
    }
    at.segments()
        .last()
        .filter(|segment| segment.kind() == NodeKind::Event)
        .and_then(|segment| segment.name())
        .map(str::to_string)
}

/// Адрес вызова без последнего сегмента `Event`: владелец события.
fn without_event(at: &QualifiedAddress) -> QualifiedAddress {
    match at.segments() {
        [owner @ .., last] if last.kind() == NodeKind::Event && !owner.is_empty() => {
            QualifiedAddress {
                line_range: None,
                source_set: at.source_set().to_string(),
                segments: owner.to_vec(),
            }
        }
        _ => at.clone(),
    }
}

/// Подставляет `{at^Kind}`: начало `at` до последнего сегмента вида `Kind`
/// включительно. `None` — такого сегмента в адресе нет.
fn expand_anchor(template: &str, at: &QualifiedAddress) -> Option<String> {
    let Some(start) = template.find("{at^") else {
        return Some(template.to_string());
    };
    let rest = &template[start + "{at^".len()..];
    let end = rest.find('}')?;
    let kind = &rest[..end];
    let matches = |segment: &AddressSegment| match kind {
        "Module" => matches!(segment.kind(), NodeKind::Module | NodeKind::CommonModule),
        kind => segment.kind().as_str() == kind,
    };
    let length = at.segments().iter().rposition(matches)? + 1;
    let anchor = QualifiedAddress {
        line_range: None,
        source_set: at.source_set().to_string(),
        segments: at.segments()[..length].to_vec(),
    };
    Some(format!(
        "{}{anchor}{}",
        &template[..start],
        &rest[end + 1..]
    ))
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
        // `items[]` без продолжения — сам элемент массива строкой.
        let Some(rest) = rest else {
            return items.iter().enumerate().find_map(|(index, item)| {
                item.as_str()
                    .filter(|text| !text.is_empty())
                    .map(|text| (format!("{}[{index}]", join(name)), text.to_string()))
            });
        };
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
    members: MemberPathLookup<'_>,
) -> Option<ArgumentRedirect> {
    entries
        .iter()
        .filter(|entry| entry.retired && entry.op == descriptor.name)
        .filter_map(|entry| entry.redirect_with(site.at, args, members))
        .find(|redirect| {
            redirect.executable_at().is_some_and(|target| {
                OperationSite::new(&target, site.source)
                    .is_some_and(|target| descriptor.applies_at(&target))
            })
        })
}

/// Снятая операция и та, что её заменила (#1354, #1465). Узел создаёт
/// операция родителя с именем в аргументах, а не адрес ещё несуществующего
/// узла; операции элемента, реквизита и команды формы называются по виду
/// своего узла (`Item`, `Attribute`, `Command`). Псевдонима нет — вызов
/// снятой операции получает отказ без записи с адресом, где исполнима
/// заменившая.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetiredOperation {
    pub(crate) op: &'static str,
    pub(crate) replacement: &'static str,
    /// Семейство заменившей: имя `attribute.add` носит и объект
    /// метаданных, и форма, а снятая `formAttribute.add` ведёт только
    /// к форме.
    pub(crate) family: OperationFamily,
    /// Вид узла, который снятая операция создавала по своему адресу;
    /// `None` — операция переименована и исполнима на том же адресе.
    pub(crate) creates: Option<NodeKind>,
    /// Как аргументы прежнего вызова выражаются в заменившей операции.
    pub(crate) arguments: &'static str,
}

const RENAMED_ARGUMENTS: &str = "the arguments are unchanged";

pub(crate) const RETIRED_OPERATIONS: &[RetiredOperation] = &[
    RetiredOperation {
        op: "form.create",
        replacement: "form.add",
        family: OperationFamily::Form,
        creates: Some(NodeKind::Form),
        arguments: "`items: [{name, type}]` names the form and its type",
    },
    RetiredOperation {
        op: "element.add",
        replacement: "item.add",
        family: OperationFamily::Form,
        creates: None,
        arguments: RENAMED_ARGUMENTS,
    },
    RetiredOperation {
        op: "element.remove",
        replacement: "item.remove",
        family: OperationFamily::Form,
        creates: None,
        arguments: RENAMED_ARGUMENTS,
    },
    RetiredOperation {
        op: "formAttribute.add",
        replacement: "attribute.add",
        family: OperationFamily::Form,
        creates: None,
        arguments: RENAMED_ARGUMENTS,
    },
    RetiredOperation {
        op: "formCommand.add",
        replacement: "command.add",
        family: OperationFamily::Form,
        creates: None,
        arguments: RENAMED_ARGUMENTS,
    },
];

impl RetiredOperation {
    pub(crate) fn lookup(op: &str) -> Option<Self> {
        RETIRED_OPERATIONS
            .iter()
            .copied()
            .find(|entry| entry.op == op)
    }

    /// Запись реестра заменившей операции в её семействе.
    pub(crate) fn descriptor(self) -> Option<OperationDescriptor> {
        OperationRegistry::closed()
            .named(self.replacement)
            .find(|descriptor| descriptor.family == self.family)
    }

    /// Узел, на котором исполнима заменившая операция: родитель узла вида
    /// `creates` в адресе вызова, иначе сам адрес вызова.
    pub(crate) fn replacement_at(self, at: &QualifiedAddress) -> QualifiedAddress {
        let Some(creates) = self.creates else {
            return at.clone();
        };
        match at
            .segments()
            .iter()
            .rposition(|segment| segment.kind() == creates)
        {
            Some(index) if index > 0 => QualifiedAddress {
                line_range: None,
                source_set: at.source_set().to_string(),
                segments: at.segments()[..index].to_vec(),
            },
            _ => at.clone(),
        }
    }

    /// Ближайший узел, где исполнима заменившая операция. Создающая
    /// операция исполнима на родителе созданного узла; переименованная —
    /// на самом адресе вызова, если место и роль элемента её принимают,
    /// иначе на ближайшем предке, как в отказе «не тот узел». `None` —
    /// такого узла в цепочке адреса нет.
    pub(crate) fn replacement_target(
        self,
        site: &OperationSite<'_>,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<QualifiedAddress> {
        if self.creates.is_some() {
            return Some(self.replacement_at(site.at));
        }
        let replacement = self.descriptor()?;
        let placed = replacement.applies_at(site);
        let here = placed
            && (site.terminal().kind() != NodeKind::Template
                || replacement.admits_template_format(template_format(site.at)))
            && (site.terminal().kind() != NodeKind::Item
                || replacement.admits_form_item(form_item(site.at)));
        if here {
            return Some(site.at.clone());
        }
        // Операция над самим узлом (`item.remove`), которую его роль не
        // принимает: автоматическую командную панель отдельно не удалить, и
        // другой узел — например, её таблица — вместо неё не подставляется.
        if placed && replacement.skeleton_key() == "at" {
            return None;
        }
        replacement.nearest_applicable_ancestor_with(site, template_format, form_item)
    }
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
        self.retired_argument_redirect_with(op, site, args, &no_member_lookup)
    }

    /// То же с поиском узла по имени: снятый `dataSet` ведёт к настоящему,
    /// в том числе вложенному, набору, а имя элемента формы — к его пути.
    pub(crate) fn retired_argument_redirect_with(
        self,
        op: &str,
        site: &OperationSite<'_>,
        args: &Map<String, Value>,
        members: MemberPathLookup<'_>,
    ) -> Option<ArgumentRedirect> {
        let descriptor = self.lookup_at(op, site)?;
        retired_argument_redirect_in(RETIRED_ARGUMENTS, descriptor, site, args, members)
    }

    /// Первая запись реестра с этим именем. Имя `attribute.add` и
    /// `command.add` носят и объект метаданных, и форма (#1465): где адрес
    /// известен, запись выбирает `lookup_at`.
    pub(crate) fn lookup(self, name: &str) -> Option<OperationDescriptor> {
        self.named(name).next()
    }

    /// Записи реестра с этим именем в порядке реестра.
    pub(crate) fn named(self, name: &str) -> impl Iterator<Item = OperationDescriptor> + '_ {
        self.descriptors()
            .iter()
            .copied()
            .filter(move |descriptor| descriptor.name == name)
    }

    /// Запись с этим именем, применимая к месту, а если ни одна не
    /// применима — первая: её отказ «не тот узел» называет ближайший узел.
    pub(crate) fn lookup_at(
        self,
        name: &str,
        site: &OperationSite<'_>,
    ) -> Option<OperationDescriptor> {
        self.named(name)
            .find(|descriptor| descriptor.applies_at(site))
            .or_else(|| self.lookup(name))
    }

    /// Ближайший к адресу узел, где исполнима операция с этим именем, и её
    /// запись: у общего имени побеждает запись с более близким узлом.
    pub(crate) fn nearest_applicable_ancestor_with(
        self,
        name: &str,
        site: &OperationSite<'_>,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<(OperationDescriptor, QualifiedAddress)> {
        self.named(name)
            .filter_map(|descriptor| {
                descriptor
                    .nearest_applicable_ancestor_with(site, template_format, form_item)
                    .map(|target| (descriptor, target))
            })
            .max_by_key(|(_, target)| target.segments().len())
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
        ApplyRequest, FormOperation, MetadataOperation, OperationApplicability,
        OperationDescriptor, OperationFamily, OperationRegistry, OperationSite, OperationSkeleton,
        RoleOperation, SourceSetTraits, SupportOperation, TemplateFormat, TypedOperation,
        RETIRED_ARGUMENTS,
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
            .lookup_at(op, &site)
            .unwrap_or_else(|| panic!("{op} is registered"))
            .applies_at(&site)
    }

    #[test]
    fn form_metadata_does_not_advertise_or_accept_owner_and_content_writers() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for raw in [
            "main:CommonForm.Main.Metadata",
            "main:Document.Order.Form.Main.Metadata",
        ] {
            let at = address(raw);
            let site = OperationSite::new(&at, traits).unwrap();
            assert!(registry.copyable_skeletons(&site).is_empty(), "{raw}");
            for operation in registry.descriptors() {
                assert!(
                    !operation.applies_at(&site),
                    "{} must not write through {raw}",
                    operation.name()
                );
            }
        }
        assert!(applies("item.add", "main:Document.Order.Form.Main"));
        assert!(applies(
            "code.replace",
            "main:Document.Order.Form.Main.Module.Form"
        ));
    }

    #[test]
    fn support_rule_addresses_a_form_without_addressing_its_contents() {
        for at in ["main:Catalog.Items", "main:Catalog.Items.Form.Card"] {
            assert!(applies("supportRule.set", at), "{at}");
        }
        for at in [
            "main:Catalog.Items.Form.Card.Item.Field",
            "main:Catalog.Items.Form.Card.Module.Form",
            "main:Catalog.Items.Attribute.Code",
        ] {
            assert!(!applies("supportRule.set", at), "{at}");
        }
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

    /// #1381: операции, которые пишут содержимое из сопутствующего файла,
    /// называют его; операции над описанием объекта — нет.
    #[test]
    fn content_writers_name_the_companion_file_they_write() {
        let registry = OperationRegistry::closed();
        let resource = |op: &str| registry.lookup(op).unwrap().content_resource();
        for (op, expected) in [
            ("right.set", Some("Rights.xml")),
            ("right.add", Some("Rights.xml")),
            ("item.add", Some("Form.xml")),
            ("event.bind", Some("Form.xml")),
            ("event.implement", Some("Form.xml")),
            ("code.insert", Some("Form.xml")),
            ("mxl.set", Some("Template.xml")),
            ("area.add", Some("Template.xml")),
            ("query.set", Some("Template.xml")),
            ("objectType.add", Some("Package.bin")),
            ("role.create", None),
            ("form.remove", None),
            ("form.add", None),
            ("form.set", None),
            ("props.set", None),
            ("template.set", None),
            ("supportRule.set", None),
        ] {
            assert_eq!(resource(op), expected, "{op}");
        }
    }

    #[test]
    fn object_borrow_accepts_only_supported_nested_enum_values() {
        let request = |from| {
            parse(
                json!({"at":"extension:Configuration", "dryRun":true, "ops":[{"op":"object.borrow","args":{"from":from}}]}),
            )
        };
        assert!(request("main:Enum.Operations.EnumValue.Costs").is_ok());
        for from in [
            "main:Enum.Operations.EnumValue",
            "main:Catalog.Orders.Attribute.Code",
            "extension:Enum.Operations.EnumValue.Costs",
            "unknown:Enum.Operations.EnumValue.Costs",
        ] {
            assert!(request(from).is_err(), "{from}");
        }
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

    fn typed_registration_is_consistent(descriptor: OperationDescriptor) -> bool {
        let (family, name, applicability, skeleton) = match descriptor.typed_operation {
            Some(TypedOperation::Role(operation)) => (
                OperationFamily::Role,
                operation.name(),
                operation.applicability(),
                operation.skeleton(),
            ),
            Some(TypedOperation::Support(operation)) => (
                OperationFamily::Support,
                operation.name(),
                operation.applicability(),
                operation.skeleton(),
            ),
            Some(TypedOperation::Metadata(operation)) => (
                OperationFamily::Metadata,
                operation.name(),
                operation.applicability(),
                operation.skeleton(),
            ),
            Some(TypedOperation::Form(operation)) => (
                OperationFamily::Form,
                operation.name(),
                operation.applicability(),
                operation.skeleton(),
            ),
            None => {
                return match descriptor.family() {
                    OperationFamily::Role | OperationFamily::Support | OperationFamily::Form => {
                        false
                    }
                    OperationFamily::Metadata => !MetadataOperation::is_named(descriptor.name()),
                    _ => true,
                }
            }
        };
        descriptor.family() == family
            && descriptor.name() == name
            && descriptor.applicability == applicability
            && descriptor.skeleton == skeleton
            && descriptor.argument_contract().is_some()
    }

    #[test]
    fn role_registration_rejects_missing_and_mismatched_typed_operations() {
        let valid = OperationDescriptor::role(RoleOperation::Create);
        assert!(typed_registration_is_consistent(valid));
        assert_eq!(valid.role_operation(), Some(RoleOperation::Create));
        assert_eq!(valid.support_operation(), None);
        for invalid in [
            OperationDescriptor {
                typed_operation: None,
                ..valid
            },
            OperationDescriptor {
                family: OperationFamily::Metadata,
                ..valid
            },
            OperationDescriptor {
                name: "role.uncontracted",
                ..valid
            },
            OperationDescriptor {
                applicability: OperationApplicability::Role,
                ..valid
            },
            OperationDescriptor {
                skeleton: OperationSkeleton::Items,
                ..valid
            },
            OperationDescriptor {
                typed_operation: Some(TypedOperation::Role(RoleOperation::RightSet)),
                ..valid
            },
            OperationDescriptor {
                typed_operation: Some(TypedOperation::Support(SupportOperation::CapabilitySet)),
                ..valid
            },
        ] {
            assert!(!typed_registration_is_consistent(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn support_registration_rejects_missing_and_mismatched_typed_operations() {
        let valid = OperationDescriptor::support(SupportOperation::CapabilitySet);
        assert!(typed_registration_is_consistent(valid));
        assert_eq!(
            valid.support_operation(),
            Some(SupportOperation::CapabilitySet)
        );
        assert_eq!(valid.role_operation(), None);
        for invalid in [
            OperationDescriptor {
                typed_operation: None,
                ..valid
            },
            OperationDescriptor {
                family: OperationFamily::Metadata,
                ..valid
            },
            OperationDescriptor {
                name: "support.uncontracted",
                ..valid
            },
            OperationDescriptor {
                applicability: OperationApplicability::SupportRule,
                ..valid
            },
            OperationDescriptor {
                skeleton: OperationSkeleton::Items,
                ..valid
            },
            OperationDescriptor {
                typed_operation: Some(TypedOperation::Support(SupportOperation::RuleSet)),
                ..valid
            },
            OperationDescriptor {
                typed_operation: Some(TypedOperation::Role(RoleOperation::Create)),
                ..valid
            },
        ] {
            assert!(!typed_registration_is_consistent(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn contextual_member_contracts_keep_owner_and_section_discriminators() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for (at, allows_fill) in [
            ("main:Document.Order", true),
            ("main:Document.Order.TabularSection.Lines", false),
            ("main:Report.Sales", false),
            ("main:Report.Sales.TabularSection.Lines", true),
        ] {
            let at = QualifiedAddress::parse(at).unwrap();
            let site = OperationSite::new(&at, traits).unwrap();
            let descriptor = registry.lookup_at("attribute.add", &site).unwrap();
            assert_eq!(
                descriptor.metadata_operation(),
                Some(MetadataOperation::AttributeAdd)
            );
            assert_eq!(descriptor.form_operation(), None);
            let contract = descriptor.argument_contract_at(&site).unwrap();
            assert_eq!(contract.help, "skills/meta-edit/SKILL.md");
            let fields = &contract.schema["properties"]["items"]["items"]["properties"];
            assert_eq!(fields.get("fillValue").is_some(), allows_fill, "{at}");
            assert!(fields.get("savedData").is_none());

            let member = QualifiedAddress::parse(&format!("{at}.Attribute.Total")).unwrap();
            let site = OperationSite::new(&member, traits).unwrap();
            let descriptor = registry.lookup_at("attribute.set", &site).unwrap();
            assert_eq!(
                descriptor.metadata_operation(),
                Some(MetadataOperation::AttributeSet)
            );
            let contract = descriptor.argument_contract_at(&site).unwrap();
            let fields = &contract.schema["properties"]["values"]["properties"];
            assert_eq!(fields.get("fillValue").is_some(), allows_fill, "{member}");
            assert!(fields.get("name").is_none());
        }
        for (name, expected) in [
            ("attribute.add", FormOperation::AttributeAdd),
            ("command.add", FormOperation::CommandAdd),
        ] {
            let at = QualifiedAddress::parse("main:Document.Order.Form.Main").unwrap();
            let site = OperationSite::new(&at, traits).unwrap();
            let descriptor = registry.lookup_at(name, &site).unwrap();
            assert_eq!(descriptor.form_operation(), Some(expected));
            assert_eq!(descriptor.metadata_operation(), None);
            let contract = descriptor.argument_contract_at(&site).unwrap();
            assert_eq!(contract.help, "skills/form-edit/SKILL.md");
            assert!(
                contract.schema["properties"]["items"]["items"]["properties"]
                    .get("fillValue")
                    .is_none()
            );
        }
    }

    #[test]
    fn contextual_registration_rejects_missing_and_mismatched_typed_operations() {
        for valid in [
            OperationDescriptor::metadata(MetadataOperation::AttributeAdd),
            OperationDescriptor::metadata(MetadataOperation::AttributeSet),
            OperationDescriptor::metadata(MetadataOperation::AttributeRemove),
            OperationDescriptor::metadata(MetadataOperation::TabularSectionAdd),
            OperationDescriptor::metadata(MetadataOperation::TabularSectionSet),
            OperationDescriptor::metadata(MetadataOperation::TabularSectionRemove),
            OperationDescriptor::metadata(MetadataOperation::DimensionAdd),
            OperationDescriptor::metadata(MetadataOperation::DimensionSet),
            OperationDescriptor::metadata(MetadataOperation::DimensionRemove),
            OperationDescriptor::metadata(MetadataOperation::ResourceAdd),
            OperationDescriptor::metadata(MetadataOperation::ResourceSet),
            OperationDescriptor::metadata(MetadataOperation::ResourceRemove),
            OperationDescriptor::metadata(MetadataOperation::EnumValueAdd),
            OperationDescriptor::metadata(MetadataOperation::EnumValueSet),
            OperationDescriptor::metadata(MetadataOperation::EnumValueRemove),
            OperationDescriptor::metadata(MetadataOperation::ColumnAdd),
            OperationDescriptor::metadata(MetadataOperation::ColumnSet),
            OperationDescriptor::metadata(MetadataOperation::ColumnRemove),
            OperationDescriptor::metadata(MetadataOperation::CommandAdd),
            OperationDescriptor::metadata(MetadataOperation::CommandSet),
            OperationDescriptor::metadata(MetadataOperation::CommandRemove),
            OperationDescriptor::form(FormOperation::Add),
            OperationDescriptor::form(FormOperation::Set),
            OperationDescriptor::form(FormOperation::Remove),
            OperationDescriptor::form(FormOperation::ItemAdd),
            OperationDescriptor::form(FormOperation::ItemRemove),
            OperationDescriptor::form(FormOperation::ItemMove),
            OperationDescriptor::form(FormOperation::AttributeAdd),
            OperationDescriptor::form(FormOperation::CommandAdd),
            OperationDescriptor::form(FormOperation::EventBind),
        ] {
            assert!(typed_registration_is_consistent(valid));
            let other = if valid.family() == OperationFamily::Metadata {
                TypedOperation::Form(FormOperation::AttributeAdd)
            } else {
                TypedOperation::Metadata(MetadataOperation::AttributeAdd)
            };
            for invalid in [
                OperationDescriptor {
                    typed_operation: None,
                    ..valid
                },
                OperationDescriptor {
                    typed_operation: Some(other),
                    ..valid
                },
                OperationDescriptor {
                    family: OperationFamily::Properties,
                    ..valid
                },
                OperationDescriptor {
                    name: "member.uncontracted",
                    ..valid
                },
                OperationDescriptor {
                    applicability: OperationApplicability::Configuration,
                    ..valid
                },
                OperationDescriptor {
                    skeleton: if valid.skeleton == OperationSkeleton::Values {
                        OperationSkeleton::Items
                    } else {
                        OperationSkeleton::Values
                    },
                    ..valid
                },
            ] {
                assert!(!typed_registration_is_consistent(invalid), "{invalid:?}");
            }
        }
        let descriptor = OperationRegistry::closed()
            .named("template.set")
            .find(|d| d.family() == OperationFamily::Metadata)
            .unwrap();
        assert_eq!(descriptor.typed_operation, None);
        assert!(typed_registration_is_consistent(descriptor));
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
            "item.add",
            "item.remove",
            "item.move",
            "attribute.add",
            "command.add",
            "event.bind",
            "role.create",
            "right.add",
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
            "area.add",
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
        assert_eq!(expected.len(), 122);
        assert_eq!(registry.names(), expected);
        // Реквизит и команда формы названы по своему узлу, как у объекта
        // метаданных (#1465): имя одно, семейство выбирает адрес. Других
        // повторов в реестре нет.
        const SHARED_NAMES: &[&str] = &["attribute.add", "command.add"];
        let mut unique = registry.names().to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), expected.len() - SHARED_NAMES.len());
        for descriptor in registry.descriptors() {
            assert!(
                typed_registration_is_consistent(*descriptor),
                "{descriptor:?}"
            );
        }
        for name in SHARED_NAMES {
            let families = registry
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.name() == *name)
                .map(|descriptor| descriptor.family())
                .collect::<Vec<_>>();
            assert_eq!(
                families,
                [OperationFamily::Metadata, OperationFamily::Form],
                "{name}"
            );
        }

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
            ("item.add", "main:Document.Order.Attribute.Total", false),
            ("event.bind", "main:Document.Order.Attribute.Total", false),
            ("item.add", "main:Document.Order.Command.Print", false),
            // Реквизит формы — узел формы, не метаданных.
            (
                "attribute.set",
                "main:Document.Order.Form.Main.Attribute.Object",
                false,
            ),
            // Элементы и события формы адресуются цепочкой `Item` любой
            // глубины (#1355): контейнер нового элемента — сам адрес.
            ("item.add", "main:Document.Order.Form.Main", true),
            (
                "item.add",
                "main:Document.Order.Form.Main.Item.Header",
                true,
            ),
            (
                "item.add",
                "main:Document.Order.Form.Main.Item.Header.Item.Inner",
                true,
            ),
            (
                "item.add",
                "main:Document.Order.Form.Main.Item.Header.Event.OnChange",
                false,
            ),
            ("item.remove", "main:Document.Order.Form.Main", false),
            (
                "item.remove",
                "main:Document.Order.Form.Main.Item.Header",
                true,
            ),
            (
                "item.remove",
                "main:Document.Order.Form.Main.Item.Header.Item.Inner",
                true,
            ),
            ("event.bind", "main:Document.Order.Form.Main", false),
            (
                "event.bind",
                "main:Document.Order.Form.Main.Item.Header",
                false,
            ),
            (
                "event.bind",
                "main:Document.Order.Form.Main.Event.OnOpen",
                true,
            ),
            (
                "event.bind",
                "main:Document.Order.Form.Main.Item.Header.Item.Inner.Event.OnChange",
                true,
            ),
            (
                "event.bind",
                "main:Document.Order.Form.Main.Command.Ok.Event.Execute",
                false,
            ),
            (
                "attribute.add",
                "main:Document.Order.Form.Main.Item.Header",
                false,
            ),
            ("form.set", "main:Document.Order", true),
            ("form.set", "main:Document.Order.Form.Main", false),
            ("form.remove", "main:Document.Order.Form.Main", true),
            ("form.remove", "main:Document.Order", false),
            // Запись прав создаёт роль, а меняет сама запись (#1354).
            ("right.add", "main:Role.Reader", true),
            ("right.add", "main:Role.Reader.Right.Catalog_Items", false),
            ("right.set", "main:Role.Reader", false),
            ("right.set", "main:Role.Reader.Right.Catalog_Items", true),
            ("right.set", "main:Role.Reader.Right", false),
            ("right.set", "main:Role.Reader.Attribute.Code", false),
            // Область создаёт макет, а ячейки пишутся в саму область.
            ("area.add", "main:Report.Sales.Template.Print", true),
            (
                "area.add",
                "main:Report.Sales.Template.Print.Area.Header",
                false,
            ),
            ("mxl.set", "main:Report.Sales.Template.Print", false),
            (
                "mxl.set",
                "main:Report.Sales.Template.Print.Area.Header",
                true,
            ),
            // Подузлы СКД: вариант, параметр и поле — по своему адресу.
            ("variant.set", "main:Report.Sales.Template.Schema", false),
            (
                "variant.set",
                "main:Report.Sales.Template.Schema.Setting.Main",
                true,
            ),
            (
                "variant.remove",
                "main:Report.Sales.Template.Schema.Setting.Main",
                true,
            ),
            ("variant.add", "main:Report.Sales.Template.Schema", true),
            ("parameter.set", "main:Report.Sales.Template.Schema", false),
            (
                "parameter.set",
                "main:Report.Sales.Template.Schema.Parameter.Period",
                true,
            ),
            (
                "parameter.rename",
                "main:Report.Sales.Template.Schema.Parameter.Period",
                true,
            ),
            (
                "field.set",
                "main:Report.Sales.Template.Schema.DataSet.Main",
                false,
            ),
            (
                "field.set",
                "main:Report.Sales.Template.Schema.DataSet.Main.Field.Amount",
                true,
            ),
            (
                "fieldRole.set",
                "main:Report.Sales.Template.Schema.DataSet.Main.Field.Amount",
                true,
            ),
            (
                "fieldRole.set",
                "main:Report.Sales.Template.Schema.DataSet.Main",
                false,
            ),
            // Код пишется в модуль, его тело и метод; область — нет.
            ("code.replace", "main:CommonModule.Shared", true),
            ("code.replace", "main:CommonModule.Shared.Method.Run", true),
            ("code.insert", "main:CommonModule.Shared.Method.Run", true),
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
            // XDTO: свойство любой вложенности задаётся адресом; новое
            // свойство добавляется в тип или во вложенный тип свойства.
            ("objectType.add", "main:XDTOPackage.Exchange", true),
            ("property.add", "main:XDTOPackage.Exchange.Type.Order", true),
            (
                "property.add",
                r"main:XDTOPackage.Exchange.Type.Order.Property.Lines.Property.A\.B",
                true,
            ),
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Type.Order",
                false,
            ),
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Type.Order.Property.Id",
                true,
            ),
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Type.Order.Property.Lines.Property.Item",
                true,
            ),
            (
                "type.remove",
                "main:XDTOPackage.Exchange.Type.Order.Property.Id",
                false,
            ),
            // Глобальное свойство пакета — не член типа.
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Property.Id",
                false,
            ),
            (
                "property.add",
                "main:XDTOPackage.Exchange.Property.Id",
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
            // Запрос — узел набора без имени; прежний псевдоним набора
            // `Template.T.Query.D` снят (#1353), и остальные операции набора
            // на запросе не исполнимы.
            (
                "query.set",
                "main:Report.Sales.Template.Schema.DataSet.Main",
                false,
            ),
            (
                "query.set",
                "main:Report.Sales.Template.Schema.DataSet.Union.DataSet.Member.Query",
                true,
            ),
            (
                "query.patch",
                "main:Report.Sales.Template.Schema.DataSet.Main.Query",
                true,
            ),
            (
                "query.set",
                "main:Report.Sales.Template.Schema.Query.Main",
                false,
            ),
            (
                "field.add",
                "main:Report.Sales.Template.Schema.DataSet.Main.Query",
                false,
            ),
            (
                "field.add",
                "main:Report.Sales.Template.Schema.Query.Main",
                false,
            ),
            // Дочерняя подсистема исключается по своему адресу.
            ("childSubsystem.remove", "main:Subsystem.Sales", false),
            (
                "childSubsystem.remove",
                "main:Subsystem.Sales.Subsystem.Orders",
                true,
            ),
            ("childSubsystem.add", "main:Subsystem.Sales", true),
            // Командный интерфейс — у подсистемы, порядок верхнего уровня — у корня.
            // Порядок команд — у группы интерфейса.
            ("commandOrder.set", "main:Subsystem.Sales.Interface", false),
            (
                "commandOrder.set",
                r"main:Subsystem.Sales.Interface.Group.CommandGroup\.Print",
                true,
            ),
            (
                "commandOrder.set",
                "main:Subsystem.Sales.Subsystem.Orders.Interface.Group.NavigationPanelImportant",
                true,
            ),
            (
                "groupOrder.set",
                "main:Subsystem.Sales.Interface.Group.NavigationPanelImportant",
                false,
            ),
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
            ("area.add", Some(TemplateFormat::SpreadsheetDocument), true),
            (
                "area.add",
                Some(TemplateFormat::DataCompositionSchema),
                false,
            ),
            ("area.add", None, false),
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
                registry
                    .lookup(op)
                    .unwrap()
                    .applies_to_node(&site, format, None),
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
                "area.add",
                "main:Report.Sales.Template.Print.Area.Header",
                Some("main:Report.Sales.Template.Print"),
            ),
            (
                "right.add",
                "main:Role.Reader.Right.Catalog_Items",
                Some("main:Role.Reader"),
            ),
            // Запись `*.set` создаёт не предок: у владельца её нет, и отказ
            // называет форму адреса подузла.
            ("right.set", "main:Role.Reader", None),
            ("mxl.set", "main:Report.Sales.Template.Print", None),
            (
                "role.create",
                "main:Role.Reader",
                Some("main:Configuration"),
            ),
            (
                "item.add",
                "main:Document.Order.Form.Main.Item.Header.Event.OnChange",
                Some("main:Document.Order.Form.Main.Item.Header"),
            ),
            (
                "attribute.add",
                "main:Document.Order.Form.Main.Item.Header",
                Some("main:Document.Order.Form.Main"),
            ),
            ("item.add", "main:Document.Order.Attribute.Total", None),
            // Под макетом-схемой областей нет: `next` не ведёт туда, где
            // планировщик откажет по формату.
            (
                "area.add",
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
                    .nearest_applicable_ancestor_with(op, &site, &format_of, &|_| None)
                    .map(|(_, at)| at.to_string()),
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
        let duplicates = [
            DUPLICATE_ARGUMENT_OPERATIONS,
            SUBNODE_OPERATIONS,
            DCS_NODE_OPERATIONS,
            FORM_OPERATIONS,
        ]
        .concat();
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
            "main:XDTOPackage.Exchange.Type.Order",
            "main:Subsystem.Sales.Interface",
        ];
        for entry in RETIRED_ARGUMENTS
            .iter()
            .filter(|entry| !duplicates.contains(&entry.op))
        {
            let descriptor = registry
                .lookup(entry.op)
                .unwrap_or_else(|| panic!("{} is registered", entry.op));
            let args = match entry.key.split_once('.') {
                Some((container, key)) => json!({container: {key: "Member"}}),
                None => json!({entry.key: "Member"}),
            };
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
                // Адрес подузла остаётся в дереве владельца. Путь с удаляемым
                // именем (`{path}.Property.{name}`) спускается на два уровня:
                // по пути и к члену, имя которого видно в коллекции.
                assert_eq!(target.route(), site.route(), "{member}");
                let depth = 1 + usize::from(redirect.listing && entry.address.contains("{path}"));
                assert_eq!(
                    member.segments().len(),
                    at.segments().len() + depth,
                    "{member} names one member below the owner"
                );
                assert_eq!(member.segments().last().unwrap().name(), Some("Member"));
                let refused = registry.retired_argument_redirect(entry.op, &site, args);
                // Свойство XDTO — само место вложенных свойств: прежний ключ,
                // оставшийся в вызове на свойстве, называет член глубже и
                // отказывает и там.
                let recursive = entry.op.starts_with("property.");
                if entry.retired {
                    assert_eq!(refused, Some(redirect), "{} at {owner}", entry.op);
                    assert_eq!(
                        registry
                            .retired_argument_redirect(entry.op, &target, args)
                            .is_some(),
                        recursive,
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
            assert!(entry.retired, "{} {} has moved", entry.op, entry.key);
            assert_eq!(
                entry.address.ends_with(".{name}"),
                entry.op.starts_with("predefinedItem.")
                    || (entry.op == "property.remove" && entry.key.ends_with("propertyPath"))
                    || (entry.op == "structure.patch" && entry.key == "values.variant")
                    || (entry.op == "event.bind"
                        && entry.key == "values.element"
                        && !entry.address.contains("{event}")),
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

    /// Операции пакета П4 (#1355): элемент и событие формы задаются адресом.
    const FORM_OPERATIONS: &[&str] = &["item.add", "item.remove", "event.bind"];

    /// Контейнер, удаляемый элемент и событие формы, прежде названные
    /// аргументом, задаются адресом `Item…Item[.Event]` (#1355). Имя
    /// элемента ведёт к его пути в форме, а без формы — к элементу верхнего
    /// уровня; русское имя события переводится в имя реестра; аргумент,
    /// повторивший адрес, отказывает тем же адресом.
    #[test]
    fn a_form_item_or_event_named_by_an_argument_is_refused_with_its_address() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let form = "main:Catalog.Items.Form.Item";
        let lookup = |container: &QualifiedAddress, name: &str| {
            assert_eq!(container.to_string(), form);
            match name {
                "Field" => {
                    super::NamedLookup::Found(vec!["Header".to_string(), "Field".to_string()])
                }
                "Header" => super::NamedLookup::Found(vec!["Header".to_string()]),
                _ => super::NamedLookup::Missing,
            }
        };
        let cases = [
            (
                "item.add",
                form.to_string(),
                json!({"items": [{"name": "New"}, {"name": "Other", "into": "Header"}]}),
                "items[1].into",
                format!("{form}.Item.Header"),
                false,
            ),
            (
                "item.add",
                format!("{form}.Item.Header"),
                json!({"items": [{"name": "New", "into": "Field"}]}),
                "items[0].into",
                format!("{form}.Item.Header.Item.Field"),
                false,
            ),
            (
                "item.remove",
                form.to_string(),
                json!({"items": [{"name": "Field"}]}),
                "items[0].name",
                format!("{form}.Item.Header.Item.Field"),
                false,
            ),
            (
                "item.remove",
                form.to_string(),
                json!({"items": ["Missing"]}),
                "items[0]",
                format!("{form}.Item.Missing"),
                false,
            ),
            (
                "item.remove",
                form.to_string(),
                json!({"values": {"name": "Header"}}),
                "values.name",
                format!("{form}.Item.Header"),
                false,
            ),
            (
                "event.bind",
                form.to_string(),
                json!({"values": {"event": "ПриОткрытии", "handler": "H"}}),
                "values.event",
                format!("{form}.Event.OnOpen"),
                false,
            ),
            (
                "event.bind",
                form.to_string(),
                json!({"values": {"event": "OnChange", "element": "Field", "handler": "H"}}),
                "values.element",
                format!("{form}.Item.Header.Item.Field.Event.OnChange"),
                false,
            ),
            (
                "event.bind",
                format!("{form}.Item.Header.Item.Field.Event.OnChange"),
                json!({"values": {"element": "Field", "handler": "H"}}),
                "values.element",
                format!("{form}.Item.Header.Item.Field.Event.OnChange"),
                false,
            ),
            (
                "event.bind",
                format!("{form}.Item.Header.Item.Field.Event.OnChange"),
                json!({"values": {"event": "StartChoice", "handler": "H"}}),
                "values.event",
                format!("{form}.Item.Header.Item.Field.Event.StartChoice"),
                false,
            ),
            // Без имени события `next` ведёт к событиям элемента.
            (
                "event.bind",
                form.to_string(),
                json!({"values": {"element": "Field", "handler": "H"}}),
                "values.element",
                format!("{form}.Item.Header.Item.Field.Event"),
                true,
            ),
        ];
        for (op, raw, args, location, expected, listing) in cases {
            let at = address(&raw);
            let site = OperationSite::new(&at, traits).unwrap();
            let redirect = registry
                .retired_argument_redirect_with(op, &site, args.as_object().unwrap(), &lookup)
                .unwrap_or_else(|| panic!("{op} at {raw} keeps its retired argument"));
            assert_eq!(redirect.location, location, "{op} at {raw}");
            assert_eq!(redirect.at.to_string(), expected, "{op} at {raw}");
            assert_eq!(redirect.listing, listing, "{op} at {raw}");
        }
        // Вызов на адресе без снятых ключей таблицу не трогает.
        for (op, raw, args) in [
            (
                "item.add",
                format!("{form}.Item.Header"),
                json!({"items": [{"name": "New", "after": "Field"}]}),
            ),
            ("item.remove", format!("{form}.Item.Header"), json!({})),
            (
                "event.bind",
                format!("{form}.Event.OnOpen"),
                json!({"values": {"handler": "H", "callType": "After"}}),
            ),
        ] {
            let at = address(&raw);
            let site = OperationSite::new(&at, traits).unwrap();
            assert_eq!(
                registry.retired_argument_redirect_with(
                    op,
                    &site,
                    args.as_object().unwrap(),
                    &lookup
                ),
                None,
                "{op} at {raw}"
            );
        }
        for op in FORM_OPERATIONS {
            assert!(
                RETIRED_ARGUMENTS
                    .iter()
                    .any(|entry| entry.op == *op && entry.retired),
                "{op} has a retired entry"
            );
        }
    }

    /// Роль элемента формы сужает словарь: в поле не добавить вложенный
    /// элемент, автоматическую командную панель не удалить отдельно.
    #[test]
    fn a_form_item_role_narrows_element_operations() {
        let registry = OperationRegistry::closed();
        let at = address("main:Catalog.Items.Form.Item.Item.Header");
        let site = OperationSite::new(
            &at,
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        for (tag, add, remove) in [
            ("[Group:V]", true, true),
            ("[Group]", true, true),
            ("[Table]", true, true),
            ("[ColumnGroup]", true, true),
            ("[Pages]", true, true),
            ("[Page]", true, true),
            ("[CmdBar]", true, true),
            ("[BtnGroup]", true, true),
            ("[Popup]", true, true),
            ("[AutoCommandBar]", true, false),
            ("[Input]", false, true),
            ("[Button]", false, true),
            ("[Label]", false, true),
        ] {
            let role = Some(super::FormItemRole::from_tag(tag));
            let verdict = |op: &str| {
                registry
                    .lookup(op)
                    .unwrap()
                    .applies_to_node(&site, None, role)
            };
            assert_eq!(verdict("item.add"), add, "{tag}");
            assert_eq!(verdict("item.remove"), remove, "{tag}");
            assert_eq!(verdict("item.move"), remove, "{tag}");
        }
    }

    /// Операции пакета П3 (#1354): подузел, прежде названный аргументом на
    /// владельце, задаётся адресом.
    const SUBNODE_OPERATIONS: &[&str] = &[
        "right.set",
        "mxl.set",
        "variant.set",
        "variant.remove",
        "parameter.set",
        "parameter.rename",
        "field.set",
        "fieldRole.set",
        "code.insert",
    ];

    /// Подузел, названный аргументом на владельце, отказывает с адресом
    /// подузла; на самом подузле тот же аргумент лишний и тоже отказывает,
    /// а вызов без него таблицу не трогает. Каждая операция пакета
    /// представлена в таблице, и её запись переключена.
    #[test]
    fn a_subnode_named_by_an_argument_is_refused_with_its_address() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let schema = "main:Report.Sales.Template.Schema";
        let cases = [
            (
                "right.set",
                "main:Role.Reader".to_string(),
                json!({"values": {"object": "Catalog.Items", "right": "Read", "value": true}}),
                "values.object",
                "main:Role.Reader.Right.Catalog_Items".to_string(),
            ),
            (
                "mxl.set",
                "main:Report.Sales.Template.Print".to_string(),
                json!({"values": {"area": "Header", "cells": {"R1C1": "x"}}}),
                "values.area",
                "main:Report.Sales.Template.Print.Area.Header".to_string(),
            ),
            (
                "variant.set",
                schema.to_string(),
                json!({"values": {"name": "Main", "title": "x"}}),
                "values.name",
                format!("{schema}.Setting.Main"),
            ),
            (
                "variant.remove",
                format!("{schema}.Setting.Main"),
                json!({"values": {"name": "Extra"}}),
                "values.name",
                format!("{schema}.Setting.Extra"),
            ),
            (
                "parameter.set",
                schema.to_string(),
                json!({"values": {"name": "Period", "title": "x"}}),
                "values.name",
                format!("{schema}.Parameter.Period"),
            ),
            (
                "parameter.rename",
                schema.to_string(),
                json!({"values": {"name": "Period", "newName": "Range"}}),
                "values.name",
                format!("{schema}.Parameter.Period"),
            ),
            (
                "field.set",
                format!("{schema}.DataSet.Main"),
                json!({"values": {"field": "Amount", "title": "x"}}),
                "values.field",
                format!("{schema}.DataSet.Main.Field.Amount"),
            ),
            (
                "field.set",
                format!("{schema}.DataSet.Main.Field.Amount"),
                json!({"values": {"dataPath": "Amount", "title": "x"}}),
                "values.dataPath",
                format!("{schema}.DataSet.Main.Field.Amount"),
            ),
            (
                "fieldRole.set",
                format!("{schema}.DataSet.Union.DataSet.Member"),
                json!({"values": {"name": "Amount", "role": {"dimension": true}}}),
                "values.name",
                format!("{schema}.DataSet.Union.DataSet.Member.Field.Amount"),
            ),
            (
                "code.insert",
                "main:CommonModule.Shared".to_string(),
                json!({"selector": {"method": "Run"}, "position": "after", "text": "x"}),
                "selector.method",
                "main:CommonModule.Shared.Method.Run".to_string(),
            ),
            (
                "code.insert",
                "main:Catalog.Items.Module.Object.Body".to_string(),
                json!({"selector": {"method": "Run"}, "position": "before", "text": "x"}),
                "selector.method",
                "main:Catalog.Items.Module.Object.Method.Run".to_string(),
            ),
        ];
        for (op, raw, args, location, expected) in cases {
            let at = address(&raw);
            let site = OperationSite::new(&at, traits).unwrap();
            let args = args.as_object().unwrap();
            let redirect = registry
                .retired_argument_redirect(op, &site, args)
                .unwrap_or_else(|| panic!("{op} at {raw} keeps its retired argument"));
            assert_eq!(redirect.location, location, "{op} at {raw}");
            assert_eq!(redirect.at.to_string(), expected, "{op} at {raw}");
            let target = OperationSite::new(&redirect.at, traits).unwrap();
            assert!(
                registry.lookup(op).unwrap().applies_at(&target),
                "{op} applies at {expected}"
            );
            let mut cleaned = args.clone();
            let (container, rest) = location.split_once('.').unwrap();
            cleaned
                .get_mut(container)
                .and_then(Value::as_object_mut)
                .unwrap()
                .remove(rest)
                .expect("the retired key is present");
            assert_eq!(
                registry.retired_argument_redirect(op, &target, &cleaned),
                None,
                "{op} at {expected} without {location}"
            );
        }
        // Составной `dataPath` становится одним экранированным именем поля,
        // а вложенный объект роли — одной записью прав.
        let data_set = address(&format!("{schema}.DataSet.Main"));
        let site = OperationSite::new(&data_set, traits).unwrap();
        let redirect = registry
            .retired_argument_redirect(
                "field.set",
                &site,
                json!({"values": {"field": "Owner.Code", "title": "x"}})
                    .as_object()
                    .unwrap(),
            )
            .expect("a composite dataPath is addressable");
        assert_eq!(
            redirect.at.to_string(),
            format!("{schema}.DataSet.Main.Field.Owner\\.Code")
        );
        assert_eq!(
            redirect.at.segments().last().and_then(|s| s.name()),
            Some("Owner.Code")
        );
        let role = address("main:Role.Reader");
        let site = OperationSite::new(&role, traits).unwrap();
        let redirect = registry
            .retired_argument_redirect(
                "right.set",
                &site,
                json!({"values": {"object": "Catalog.Items.Command.Open", "right": "View", "value": true}})
                    .as_object()
                    .unwrap(),
            )
            .expect("a nested role object is addressable");
        assert_eq!(
            redirect.at.to_string(),
            "main:Role.Reader.Right.Catalog_Items\\.Command\\.Open"
        );
        for op in SUBNODE_OPERATIONS {
            assert!(
                RETIRED_ARGUMENTS
                    .iter()
                    .any(|entry| entry.op == *op && entry.retired),
                "{op} has a retired entry"
            );
        }
    }

    /// Операции пакета П6 (#1357): источник, вычисляемое поле, итог и
    /// элемент структуры варианта задаются адресом.
    const DCS_NODE_OPERATIONS: &[&str] = &[
        "dataSource.set",
        "dataSource.remove",
        "calculatedField.set",
        "calculatedField.remove",
        "total.set",
        "total.remove",
        "filter.add",
        "filter.set",
        "filter.remove",
        "filter.clear",
        "selection.add",
        "selection.set",
        "selection.remove",
        "selection.clear",
        "order.add",
        "order.set",
        "order.remove",
        "order.clear",
        "conditionalAppearance.add",
        "conditionalAppearance.remove",
        "conditionalAppearance.clear",
        "outputParameter.set",
        "outputParameter.remove",
        "structure.add",
        "structure.patch",
        "structure.remove",
    ];

    /// Новые узлы схемы (#1357): операция исполнима ровно на своём узле, а
    /// прежний аргумент, называвший источник, вычисляемое поле, итог или
    /// контейнер настроек, отказывает с адресом узла. Без схемы элемент
    /// структуры, названный аргументом, — элемент верхнего уровня варианта.
    #[test]
    fn dcs_schema_nodes_and_structure_items_take_their_subject_from_the_address() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let schema = "main:Report.Sales.Template.Schema";
        let setting = format!("{schema}.Setting.Main");
        let group = format!("{setting}.Item.ByCategory");
        let nested = format!("{group}.Item.Details");
        // Матрица применимости: узел и операции, которые на нём исполнимы
        // из этого набора; остальные операции набора на нём не исполнимы.
        let container = [
            "filter.add",
            "filter.remove",
            "selection.set",
            "order.clear",
            "conditionalAppearance.remove",
            "outputParameter.set",
            "outputParameter.remove",
            "structure.add",
        ];
        let item = [&container[..], &["structure.patch", "structure.remove"]].concat();
        let variant = [
            &container[..],
            &[
                "dataParameter.add",
                "dataParameter.remove",
                "structure.set",
                "variant.set",
            ],
        ]
        .concat();
        let matrix: Vec<(String, Vec<&str>)> = vec![
            (
                schema.to_string(),
                vec![
                    "dataSource.add",
                    "calculatedField.add",
                    "total.add",
                    "variant.add",
                ],
            ),
            (
                format!("{schema}.DataSource.Local"),
                vec!["dataSource.set", "dataSource.remove"],
            ),
            (
                format!("{schema}.CalculatedField.Owner\\.Code"),
                vec!["calculatedField.set", "calculatedField.remove"],
            ),
            (
                format!("{schema}.Total.Amount"),
                vec!["total.set", "total.remove"],
            ),
            (setting.clone(), variant),
            (group.clone(), item.clone()),
            (nested.clone(), item),
        ];
        let checked = [
            "dataSource.add",
            "dataSource.set",
            "dataSource.remove",
            "calculatedField.add",
            "calculatedField.set",
            "calculatedField.remove",
            "total.add",
            "total.set",
            "total.remove",
            "variant.add",
            "variant.set",
            "filter.add",
            "filter.remove",
            "selection.set",
            "order.clear",
            "conditionalAppearance.remove",
            "outputParameter.set",
            "outputParameter.remove",
            "dataParameter.add",
            "dataParameter.remove",
            "structure.add",
            "structure.set",
            "structure.patch",
            "structure.remove",
        ];
        for (raw, applicable) in matrix {
            let at = address(&raw);
            let site = OperationSite::new(&at, traits).unwrap();
            for op in checked {
                assert_eq!(
                    registry.lookup(op).unwrap().applies_at(&site),
                    applicable.contains(&op),
                    "{op} at {raw}"
                );
            }
        }
        // Безымянная ветвь и элемент структуры вне варианта — не узлы
        // операций.
        for raw in [
            format!("{schema}.DataSource"),
            format!("{setting}.Item"),
            format!("{schema}.Item.Link"),
        ] {
            let at = address(&raw);
            if let Some(site) = OperationSite::new(&at, traits) {
                for op in DCS_NODE_OPERATIONS {
                    assert!(
                        !registry.lookup(op).unwrap().applies_at(&site),
                        "{op} at {raw}"
                    );
                }
            }
        }
        let cases = [
            (
                "dataSource.set",
                schema.to_string(),
                json!({"values": {"name": "Local", "kind": "Local"}}),
                "values.name",
                format!("{schema}.DataSource.Local"),
            ),
            (
                "dataSource.remove",
                format!("{schema}.DataSet.Main"),
                json!({"values": {"name": "Local"}}),
                "values.name",
                format!("{schema}.DataSource.Local"),
            ),
            (
                "calculatedField.set",
                schema.to_string(),
                json!({"values": {"name": "Owner.Code", "expression": "1"}}),
                "values.name",
                format!("{schema}.CalculatedField.Owner\\.Code"),
            ),
            (
                "calculatedField.remove",
                schema.to_string(),
                json!({"values": {"name": "Double"}}),
                "values.name",
                format!("{schema}.CalculatedField.Double"),
            ),
            (
                "total.set",
                schema.to_string(),
                json!({"values": {"field": "Amount", "expression": "Сумма(Amount)"}}),
                "values.field",
                format!("{schema}.Total.Amount"),
            ),
            (
                "total.remove",
                schema.to_string(),
                json!({"values": {"name": "Amount"}}),
                "values.name",
                format!("{schema}.Total.Amount"),
            ),
            (
                "filter.add",
                setting.clone(),
                json!({"items": [{"field": "Amount", "group": "ByCategory"}]}),
                "items[0].group",
                group.clone(),
            ),
            (
                "selection.set",
                setting.clone(),
                json!({"values": {"field": "Amount", "use": false, "group": "ByCategory"}}),
                "values.group",
                group.clone(),
            ),
            (
                "outputParameter.remove",
                group.clone(),
                json!({"values": {"name": "Title", "group": "ByCategory"}}),
                "values.group",
                group.clone(),
            ),
            (
                "structure.add",
                setting.clone(),
                json!({"items": [{"name": "Details", "parent": "ByCategory"}]}),
                "items[0].parent",
                group.clone(),
            ),
            (
                "structure.patch",
                setting.clone(),
                json!({"values": {"name": "ByCategory", "groupBy": ["Category"]}}),
                "values.name",
                group.clone(),
            ),
            (
                "structure.remove",
                setting.clone(),
                json!({"values": {"name": "Report.Rows"}}),
                "values.name",
                format!("{setting}.Item.Report\\.Rows"),
            ),
        ];
        for (op, raw, args, location, expected) in cases {
            let at = address(&raw);
            let site = OperationSite::new(&at, traits).unwrap();
            let args = args.as_object().unwrap();
            let redirect = registry
                .retired_argument_redirect(op, &site, args)
                .unwrap_or_else(|| panic!("{op} at {raw} keeps its retired argument"));
            assert_eq!(redirect.location, location, "{op} at {raw}");
            assert_eq!(redirect.at.to_string(), expected, "{op} at {raw}");
            assert!(!redirect.ambiguous, "{op} at {raw}");
            let target = OperationSite::new(&redirect.at, traits).unwrap();
            assert!(
                registry.lookup(op).unwrap().applies_at(&target),
                "{op} applies at {expected}"
            );
        }
        for op in DCS_NODE_OPERATIONS {
            assert!(
                RETIRED_ARGUMENTS
                    .iter()
                    .any(|entry| entry.op == *op && entry.retired),
                "{op} has a retired entry"
            );
        }
    }

    /// Поиск по имени трёхзначен (#1357): найденный узел даёт свой адрес,
    /// повторённое имя — ветвь узлов этого вида и адреса совпавших, а не
    /// выдуманный узел верхнего уровня.
    #[test]
    fn a_repeated_schema_name_routes_next_to_the_branch_and_every_match() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let schema = "main:Report.Sales.Template.Schema";
        let lookup = |owner: &QualifiedAddress, name: &str| {
            let owner = owner.to_string();
            match (owner.as_str(), name) {
                (owner, "Twin") if owner == schema => super::NamedLookup::Ambiguous(vec![
                    vec!["Left".to_string(), "Twin".to_string()],
                    vec!["Right".to_string(), "Twin".to_string()],
                ]),
                (owner, "Rows") if owner.ends_with(".Setting.Main") => {
                    super::NamedLookup::Ambiguous(vec![
                        vec!["Table".to_string(), "Rows".to_string()],
                        vec!["Chart".to_string(), "Rows".to_string()],
                    ])
                }
                (owner, "Details") if owner.ends_with(".Setting.Main") => {
                    super::NamedLookup::Found(vec!["Table".to_string(), "Details".to_string()])
                }
                _ => super::NamedLookup::Missing,
            }
        };
        let lookup = &lookup as super::MemberPathLookup<'_>;
        let at = address(&format!("{schema}.DataSet.Main.Query"));
        let site = OperationSite::new(&at, traits).unwrap();
        let redirect = registry
            .retired_argument_redirect_with(
                "query.set",
                &site,
                json!({"values": {"dataSet": "Twin", "query": "ВЫБРАТЬ 1"}})
                    .as_object()
                    .unwrap(),
                lookup,
            )
            .expect("the dataSet argument is retired");
        assert!(redirect.ambiguous);
        assert_eq!(redirect.at.to_string(), format!("{schema}.DataSet"));
        assert_eq!(
            redirect
                .candidates
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                format!("{schema}.DataSet.Left.DataSet.Twin.Query"),
                format!("{schema}.DataSet.Right.DataSet.Twin.Query"),
            ]
        );
        let setting = address(&format!("{schema}.Setting.Main"));
        let site = OperationSite::new(&setting, traits).unwrap();
        let redirect = registry
            .retired_argument_redirect_with(
                "filter.clear",
                &site,
                json!({"values": {"group": "Rows"}}).as_object().unwrap(),
                lookup,
            )
            .expect("the group argument is retired");
        assert!(redirect.ambiguous);
        assert_eq!(redirect.at.to_string(), format!("{setting}.Item"));
        assert_eq!(
            redirect
                .candidates
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                format!("{setting}.Item.Table.Item.Rows"),
                format!("{setting}.Item.Chart.Item.Rows"),
            ]
        );
        let redirect = registry
            .retired_argument_redirect_with(
                "structure.remove",
                &site,
                json!({"values": {"name": "Details"}}).as_object().unwrap(),
                lookup,
            )
            .expect("the name argument is retired");
        assert!(!redirect.ambiguous);
        assert_eq!(
            redirect.at.to_string(),
            format!("{setting}.Item.Table.Item.Details")
        );
    }

    /// Переименованная операция ведёт к ближайшему узлу, где исполнима
    /// заменившая, с учётом роли элемента (#1465): элемент добавляется
    /// в контейнер над полем, а автоматическую командную панель, которую
    /// отдельно не удалить, не подменяет её таблица.
    #[test]
    fn a_renamed_operation_target_respects_the_item_role() {
        use super::{FormItemRole, RetiredOperation};
        let form = "main:Catalog.Items.Form.List";
        let bar = format!("{form}.Item.List.Item.ListCommandBar");
        let field = format!("{form}.Item.Header.Item.Code");
        let role = |at: &QualifiedAddress| match at.to_string() {
            text if text == bar => Some(FormItemRole::AutoCommandBar),
            text if text == field => Some(FormItemRole::Leaf),
            _ => Some(FormItemRole::Container),
        };
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for (op, raw, expected) in [
            ("element.remove", bar.clone(), None),
            ("element.add", bar.clone(), Some(bar.clone())),
            ("element.remove", field.clone(), Some(field.clone())),
            (
                "element.add",
                field.clone(),
                Some(format!("{form}.Item.Header")),
            ),
            (
                "element.add",
                format!("{field}.Event.OnChange"),
                Some(format!("{form}.Item.Header")),
            ),
        ] {
            let at = address(&raw);
            let site = OperationSite::new(&at, traits).unwrap();
            let target =
                RetiredOperation::lookup(op)
                    .unwrap()
                    .replacement_target(&site, &|_| None, &role);
            assert_eq!(
                target.map(|target| target.to_string()),
                expected,
                "{op} at {raw}"
            );
        }
    }

    /// Узел создаёт операция родителя: снятая `form.create` называет
    /// `form.add` на владельце формы, а `*.set` подузла знает, какая
    /// операция родителя создаёт его узел (#1354).
    #[test]
    fn a_retired_creation_names_the_parent_operation() {
        let retired =
            super::RetiredOperation::lookup("form.create").expect("form.create is retired");
        assert_eq!(retired.replacement, "form.add");
        let registry = OperationRegistry::closed();
        assert!(registry.lookup("form.create").is_none());
        assert!(registry.lookup(retired.replacement).is_some());
        for (raw, expected) in [
            ("main:Catalog.Items.Form.Choice", "main:Catalog.Items"),
            ("main:Catalog.Items", "main:Catalog.Items"),
        ] {
            let target = retired.replacement_at(&address(raw));
            assert_eq!(target.to_string(), expected, "{raw}");
            let site = OperationSite::new(
                &target,
                SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
            )
            .unwrap();
            assert!(
                registry.lookup("form.add").unwrap().applies_at(&site),
                "{raw}"
            );
        }
        assert!(super::RetiredOperation::lookup("form.add").is_none());
        // Операции элемента, реквизита и команды формы переименованы по
        // своему узлу (#1465): заменившая формы исполнима на том же адресе,
        // снятого имени в реестре нет.
        for (op, replacement, raw) in [
            ("element.add", "item.add", "main:Catalog.Items.Form.Choice"),
            (
                "element.add",
                "item.add",
                "main:Catalog.Items.Form.Choice.Item.Header",
            ),
            (
                "element.remove",
                "item.remove",
                "main:Catalog.Items.Form.Choice.Item.Header",
            ),
            (
                "formAttribute.add",
                "attribute.add",
                "main:Catalog.Items.Form.Choice",
            ),
            (
                "formCommand.add",
                "command.add",
                "main:Catalog.Items.Form.Choice",
            ),
        ] {
            let retired = super::RetiredOperation::lookup(op).expect("the old name is retired");
            assert_eq!(retired.replacement, replacement, "{op}");
            assert!(registry.lookup(op).is_none(), "{op}");
            let at = address(raw);
            let target = retired.replacement_at(&at);
            assert_eq!(target, at, "{op} at {raw}");
            let site = OperationSite::new(
                &target,
                SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
            )
            .unwrap();
            assert!(
                retired.descriptor().unwrap().applies_at(&site),
                "{replacement} at {raw}"
            );
        }
        for (op, creator) in [
            ("right.set", Some("right.add")),
            ("mxl.set", Some("area.add")),
            ("attribute.set", None),
        ] {
            assert_eq!(registry.lookup(op).unwrap().creator(), creator, "{op}");
        }
    }

    /// Операции пакета П2 (#1353): их снятый аргумент дублировал `at` или
    /// называл узел в обход адреса, а сами они не переезжали.
    const DUPLICATE_ARGUMENT_OPERATIONS: &[&str] = &[
        "query.set",
        "query.patch",
        "structure.set",
        "structure.patch",
        "childSubsystem.remove",
        "code.replace",
    ];

    /// Аргумент, который дублировал или переопределял `at`, отказывает и
    /// тогда, когда операция исполнима на самом адресе вызова: `next` ведёт
    /// к узлу, который называло значение, и операция там исполнима. Без
    /// снятого ключа вызов отказа по таблице не получает. Каждая операция
    /// пакета представлена в таблице.
    #[test]
    fn a_duplicate_argument_is_refused_with_the_node_its_value_named() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let template = "main:Report.Sales.Template.Schema";
        let cases = [
            (
                "query.set",
                format!("{template}.DataSet.Main.Query"),
                json!({"values": {"dataSet": "Other", "query": "ВЫБРАТЬ 1"}}),
                "values.dataSet",
                format!("{template}.DataSet.Other.Query"),
            ),
            (
                "query.patch",
                format!("{template}.DataSet.Main.Query"),
                json!({"values": {"dataSet": "Main", "find": "1"}}),
                "values.dataSet",
                format!("{template}.DataSet.Main.Query"),
            ),
            // Вложенный набор, названный в адресе вызова, — он сам.
            (
                "query.set",
                format!("{template}.DataSet.Union.DataSet.Member.Query"),
                json!({"values": {"dataSet": "Member", "query": "ВЫБРАТЬ 1"}}),
                "values.dataSet",
                format!("{template}.DataSet.Union.DataSet.Member.Query"),
            ),
            (
                "query.set",
                format!("{template}.DataSet.Union"),
                json!({"values": {"dataSet": "Union", "query": "ВЫБРАТЬ 1"}}),
                "values.dataSet",
                format!("{template}.DataSet.Union.Query"),
            ),
            // На самом макете операция не исполнима, но значение называет
            // набор: отказ ведёт к нему, а не к форме адреса.
            (
                "query.set",
                template.to_string(),
                json!({"values": {"dataSet": "Main", "query": "ВЫБРАТЬ 1"}}),
                "values.dataSet",
                format!("{template}.DataSet.Main.Query"),
            ),
            (
                "structure.set",
                format!("{template}.Setting.Main"),
                json!({"values": {"variant": "Main", "groupBy": []}}),
                "values.variant",
                format!("{template}.Setting.Main"),
            ),
            (
                "childSubsystem.remove",
                "main:Subsystem.Sales".to_string(),
                json!({"items": [{"name": "Orders"}]}),
                "items[0].name",
                "main:Subsystem.Sales.Subsystem.Orders".to_string(),
            ),
            (
                "childSubsystem.remove",
                "main:Subsystem.Sales".to_string(),
                json!({"items": ["Orders"]}),
                "items[0]",
                "main:Subsystem.Sales.Subsystem.Orders".to_string(),
            ),
            (
                "childSubsystem.remove",
                "main:Subsystem.Sales.Subsystem.Orders".to_string(),
                json!({"values": {"name": "Archive"}}),
                "values.name",
                "main:Subsystem.Sales.Subsystem.Orders.Subsystem.Archive".to_string(),
            ),
            (
                "code.replace",
                "main:CommonModule.Shared".to_string(),
                json!({"selector": {"method": "Run"}, "text": "x"}),
                "selector.method",
                "main:CommonModule.Shared.Method.Run".to_string(),
            ),
            (
                "code.replace",
                "main:Catalog.Items.Module.Object.Method.Other".to_string(),
                json!({"selector": {"method": "Run"}, "text": "x"}),
                "selector.method",
                "main:Catalog.Items.Module.Object.Method.Run".to_string(),
            ),
        ];
        for (op, raw, args, location, expected) in cases {
            let at = address(&raw);
            let site = OperationSite::new(&at, traits).unwrap();
            let args = args.as_object().unwrap();
            let redirect = registry
                .retired_argument_redirect(op, &site, args)
                .unwrap_or_else(|| panic!("{op} at {raw} keeps its retired argument"));
            assert_eq!(redirect.location, location, "{op} at {raw}");
            assert_eq!(redirect.at.to_string(), expected, "{op} at {raw}");
            assert!(!redirect.listing, "{op} at {raw}");
            let target = OperationSite::new(&redirect.at, traits).unwrap();
            assert!(
                registry.lookup(op).unwrap().applies_at(&target),
                "{op} applies at {expected}"
            );
            // Тот же вызов без снятого ключа таблица не трогает.
            let mut cleaned = args.clone();
            let (container, rest) = location.split_once(['.', '[']).unwrap();
            match cleaned.get_mut(container).unwrap() {
                Value::Object(object) => {
                    object.remove(rest).expect("the retired key is present");
                }
                Value::Array(items) => {
                    items.remove(0);
                }
                _ => unreachable!(),
            }
            assert_eq!(
                registry.retired_argument_redirect(op, &site, &cleaned),
                None,
                "{op} at {raw} without {location}"
            );
        }
        // Правка элемента структуры исполняется на его адресе (#1357):
        // вариант, названный аргументом, ведёт к элементам его структуры.
        let at = address(&format!("{template}.Setting.Main.Item.G"));
        let site = OperationSite::new(&at, traits).unwrap();
        let redirect = registry
            .retired_argument_redirect(
                "structure.patch",
                &site,
                json!({"values": {"variant": "Extra", "groupBy": ["A"]}})
                    .as_object()
                    .unwrap(),
            )
            .expect("the variant argument is retired");
        assert!(redirect.listing);
        assert_eq!(redirect.location, "values.variant");
        assert_eq!(
            redirect.at.to_string(),
            format!("{template}.Setting.Extra.Item")
        );
        // Набор, не названный в адресе вызова, ищется в схеме: член
        // объединения получает свой вложенный адрес; без схемы — верхний
        // уровень.
        let at = address(&format!("{template}.DataSet.Main.Query"));
        let site = OperationSite::new(&at, traits).unwrap();
        let args = json!({"values": {"dataSet": "Member", "query": "ВЫБРАТЬ 1"}});
        let args = args.as_object().unwrap();
        let lookup = |schema: &QualifiedAddress, name: &str| {
            assert_eq!(schema.to_string(), template);
            if name == "Member" {
                super::NamedLookup::Found(vec!["Union".to_string(), "Member".to_string()])
            } else {
                super::NamedLookup::Missing
            }
        };
        for (lookup, expected) in [
            (
                &lookup as super::MemberPathLookup<'_>,
                format!("{template}.DataSet.Union.DataSet.Member.Query"),
            ),
            (
                &super::no_member_lookup as super::MemberPathLookup<'_>,
                format!("{template}.DataSet.Member.Query"),
            ),
        ] {
            let redirect = registry
                .retired_argument_redirect_with("query.set", &site, args, lookup)
                .expect("the dataSet argument is retired");
            assert_eq!(redirect.at.to_string(), expected);
        }
        // Операция запроса на самом наборе ведёт к его запросу.
        let data_set = address(&format!("{template}.DataSet.Main"));
        let site = OperationSite::new(&data_set, traits).unwrap();
        assert_eq!(
            registry
                .lookup("query.set")
                .unwrap()
                .nearest_applicable_ancestor(&site, &|_| None)
                .map(|at| at.to_string()),
            Some(format!("{template}.DataSet.Main.Query"))
        );
        // Якорь в коде — аргумент, а не адрес: он не снят.
        let module = address("main:CommonModule.Shared");
        let site = OperationSite::new(&module, traits).unwrap();
        assert_eq!(
            registry.retired_argument_redirect(
                "code.replace",
                &site,
                json!({"selector": {"anchor": "Run"}, "text": "x"})
                    .as_object()
                    .unwrap()
            ),
            None
        );
        for op in DUPLICATE_ARGUMENT_OPERATIONS {
            assert!(
                RETIRED_ARGUMENTS
                    .iter()
                    .any(|entry| entry.op == *op && entry.retired),
                "{op} has a retired entry"
            );
        }
    }

    /// Имя из снятого аргумента попадает в адрес с экранированной точкой:
    /// группа `CommandGroup.X` и свойство XDTO `A.B` — один сегмент. Прежний
    /// `propertyPath` становится цепочкой `Property`, где `\.` остаётся
    /// точкой имени, а путь с удаляемым именем ведёт к коллекции свойств.
    #[test]
    fn retired_names_with_a_dot_route_to_escaped_addresses() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let redirect = |op: &str, raw: &str, args: Value| {
            let at = address(raw);
            let site = OperationSite::new(&at, traits).unwrap();
            registry
                .retired_argument_redirect(op, &site, args.as_object().unwrap())
                .unwrap_or_else(|| panic!("{op} at {raw} is redirected"))
        };
        let group = redirect(
            "commandOrder.set",
            "main:Subsystem.Sales.Interface",
            json!({"values": {"group": "CommandGroup.Печать", "commands": []}}),
        );
        assert_eq!(group.location, "values.group");
        assert_eq!(
            group.at.to_string(),
            r"main:Subsystem.Sales.Interface.Group.CommandGroup\.Печать"
        );
        assert_eq!(
            group.at.segments().last().unwrap().name(),
            Some("CommandGroup.Печать")
        );

        let type_at = "main:XDTOPackage.Exchange.Type.Order";
        let added = redirect(
            "property.add",
            type_at,
            json!({"values": {"propertyPath": r"Lines.A\.B", "property": {"name": "X", "type": "xs:string"}}}),
        );
        assert_eq!(added.location, "values.propertyPath");
        assert!(!added.listing);
        assert_eq!(
            added.at.to_string(),
            format!(r"{type_at}.Property.Lines.Property.A\.B")
        );
        let removed = redirect(
            "property.remove",
            type_at,
            json!({"values": {"name": "Item", "propertyPath": "Lines"}}),
        );
        assert_eq!(removed.location, "values.propertyPath");
        assert!(removed.listing);
        assert_eq!(
            removed.at.to_string(),
            format!("{type_at}.Property.Lines.Property")
        );
        let named = redirect(
            "property.remove",
            type_at,
            json!({"values": {"name": "A.B"}}),
        );
        assert_eq!(named.at.to_string(), format!(r"{type_at}.Property.A\.B"));
        // Путь с пустым именем или чужим экранированием адреса не строит.
        for path in ["Lines..A", r"Lines\x", ""] {
            let at = address(type_at);
            let site = OperationSite::new(&at, traits).unwrap();
            let args = json!({"values": {"propertyPath": path, "property": {}}});
            assert_eq!(
                registry
                    .retired_argument_redirect("property.add", &site, args.as_object().unwrap())
                    .map(|redirect| redirect.at.to_string()),
                None,
                "{path}"
            );
        }
    }

    /// Ветвь коллекции (`Interface.Group`, `Type.T.Property`) — не узел
    /// операции, и подсказка не выдумывает узел `…Group.Query` из вида
    /// запроса набора: ближайший исполнимый предок ищется только по цепочке
    /// адреса.
    #[test]
    fn the_nearest_ancestor_never_invents_a_query_name_under_a_named_kind() {
        let registry = OperationRegistry::closed();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for (op, raw) in [
            ("commandOrder.set", "main:Subsystem.Sales.Interface.Group"),
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Type.Order.Property",
            ),
        ] {
            let at = address(raw);
            let site = OperationSite::new(&at, traits).unwrap();
            let nearest = registry
                .lookup(op)
                .unwrap()
                .nearest_applicable_ancestor(&site, &|_| None)
                .map(|at| at.to_string());
            assert!(
                nearest.as_deref().is_none_or(|at| !at.ends_with(".Query")),
                "{op} at {raw}: {nearest:?}"
            );
        }
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
                &super::no_member_lookup,
            ),
            None
        );
        // Вложенная позиция в массиве называет номер элемента.
        let nested = super::RetiredArgument {
            op: "item.add",
            key: "items[].into",
            address: "{item}",
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
