use crate::domain::logical_route::classify_route;
use unica_bsl::operations::CodeOperation;
use unica_bsl::profile::PlatformProfile;
use unica_dcs::operations::Primitive;
use unica_forms::operations::FormOperation;
use unica_forms::{FormItemRole, FormObservation};

use unica_metadata::event::EventOperation;
use unica_metadata::MetadataKind;
use unica_metadata::{
    interface_operations::InterfaceOperation, subsystem_operations::SubsystemOperation,
};
use unica_operations::{OperationDescriptor, OperationFamily, OperationRegistry};
use unica_source_contract::context::{OperationSite, RegistryContext, SourceSetTraits};
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_templates::operations::MxlOperation;
use unica_templates::TemplateFormat;

use serde_json::{Map, Value};
use std::fmt;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_metadata::descriptor_operations::PropertiesOperation;
use unica_metadata::support::SupportOperation;
#[cfg(test)]
use unica_operation_contract::OperationSkeleton;
use unica_refusal::RefusalCode;
use unica_roles::operations::RoleOperation;
use unica_xdto::operations::XdtoOperation;

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
            let metadata_operation =
                unica_metadata::apply_operations::MetadataOperation::named(&name);
            if metadata_operation
                == Some(unica_metadata::apply_operations::MetadataOperation::ObjectBorrow)
            {
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
                    return Err(ApplyValidationError::bad_value(&location, "from must name a top-level object, Enum.EnumValue or object Form in a distinct admitted parent source set"));
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
            if (metadata_operation
                == Some(unica_metadata::apply_operations::MetadataOperation::ObjectCreate)
                || created_kind.is_some())
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
                    let object_name = if created_kind.is_some() {
                        unica_metadata::canonical_creation_identifier(object_name)?
                    } else {
                        if !unica_metadata::identifier::is_1c_identifier(object_name) {
                            return None;
                        }
                        object_name
                    };
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
                && ((parent.segments()[0].kind() == NodeKind::Enum
                    && parent.segments()[1].kind() == NodeKind::EnumValue)
                    || parent.segments()[1].kind() == NodeKind::Form)
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

/// Exact writable identity is admitted here; classification keeps the historical
/// read fallback separately so an unknown revision cannot gain write admission.
pub(crate) fn source_set_traits_of_source(
    kind: SourceSetKind,
    format: SourceFormat,
    profile: SourceProfile,
) -> SourceSetTraits {
    let exact = (format == SourceFormat::PlatformXml && profile.serialization_format().is_some())
        .then(|| crate::domain::source_capabilities::bsl_profile_for_source(profile))
        .flatten()
        .map(|_| profile);
    SourceSetTraits::from_profiles(
        kind,
        exact,
        exact.unwrap_or_else(SourceProfile::platform_xml_8_3_27_format_2_20),
    )
}

/// A logical reader has already admitted the closed semantic platform profile.
pub(crate) fn source_set_traits_for_read(
    kind: SourceSetKind,
    platform: PlatformProfile,
) -> SourceSetTraits {
    let profile = SourceProfile::platform_xml_8_3_27_format_2_20();
    debug_assert_eq!(
        crate::domain::source_capabilities::bsl_profile_for_source(profile),
        Some(platform)
    );
    SourceSetTraits::from_profiles(kind, Some(profile), profile)
}

pub(crate) fn source_classification_profile(source: SourceSetTraits) -> PlatformProfile {
    crate::domain::source_capabilities::bsl_profile_for_source(source.classification_profile())
        .unwrap_or_else(PlatformProfile::v8_3_27)
}

/// The sole semantic classifier binds pure source facts to a logical address.
pub(crate) fn operation_site(
    at: &QualifiedAddress,
    source: SourceSetTraits,
) -> Option<OperationSite<'_>> {
    let route = classify_route(at, source_classification_profile(source))?;
    Some(OperationSite::from_route(at, source, route))
}

/// Family write policy remains with operation composition during extraction.
pub(crate) trait SourceSetWriteSupport {
    fn writes(self, family: OperationFamily) -> bool;
}

impl SourceSetWriteSupport for SourceSetTraits {
    fn writes(self, family: OperationFamily) -> bool {
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
            | OperationFamily::Event => self.is_writable(),
        }
    }
}

/// Отказ записи в набор, который семейство не пишет. Один текст для
/// предварительной проверки и разбора планировщиком.
pub(crate) const UNWRITABLE_SOURCE_MESSAGE: &str =
    "admitted source has no writable exact Platform XML profile";

/// Composition adapters for already-observed family facts. All semantic
/// predicates/contracts are supplied by the bound family operation.
pub(crate) trait OperationDescriptorExt {
    fn admits_template_format(self, format: Option<TemplateFormat>) -> bool;
    fn admits_form_item(self, role: Option<FormItemRole>) -> bool;
    fn applies_to_node(
        self,
        site: &OperationSite<'_>,
        format: Option<TemplateFormat>,
        role: Option<FormItemRole>,
    ) -> bool;
    fn nearest_applicable_ancestor(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        template: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    ) -> Option<QualifiedAddress>;
    fn nearest_applicable_ancestor_with(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        template: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<QualifiedAddress>;
}

pub(crate) fn observation_lookup<'a>(
    template: &'a dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    item: &'a dyn Fn(&QualifiedAddress) -> Option<FormObservation>,
) -> impl Fn(&QualifiedAddress, unica_operations::ObservationType) -> Option<Box<dyn std::any::Any>> + 'a
{
    move |at, requested| {
        if requested.is::<TemplateFormat>() {
            template(at).map(|value| Box::new(value) as Box<dyn std::any::Any>)
        } else if requested.is::<FormObservation>() {
            item(at).map(|value| Box::new(value) as Box<dyn std::any::Any>)
        } else {
            None
        }
    }
}

impl OperationDescriptorExt for OperationDescriptor {
    fn admits_template_format(self, format: Option<TemplateFormat>) -> bool {
        !self.observation_type().is::<TemplateFormat>()
            || self
                .admits_context(&|_| format.map(|value| Box::new(value) as Box<dyn std::any::Any>))
    }
    fn admits_form_item(self, role: Option<FormItemRole>) -> bool {
        !self.observation_type().is::<FormObservation>()
            || self.admits_context(&|_| {
                role.map(|value| Box::new(FormObservation::Item(value)) as Box<dyn std::any::Any>)
            })
    }
    fn applies_to_node(
        self,
        site: &OperationSite<'_>,
        format: Option<TemplateFormat>,
        role: Option<FormItemRole>,
    ) -> bool {
        self.applies_with_observations(
            site,
            &observation_lookup(&|_| format, &|_| role.map(FormObservation::Item)),
        )
    }
    /// Ближайший узел по цепочке предков `at` (до корня набора включительно),
    /// где операция исполнима. Туда ведёт `next` отказа «не тот узел».
    ///
    /// Для макета-предка учитывается и формат его тела: `template_format`
    /// читает его у вызывающего, домен файлов не читает. Неизвестный формат
    /// макет не принимает — `next` не уводит туда, где планировщик откажет.
    fn nearest_applicable_ancestor(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
    ) -> Option<QualifiedAddress> {
        self.nearest_ancestor_with_observations(
            site,
            context,
            &observation_lookup(template_format, &|_| None),
        )
    }
    fn nearest_applicable_ancestor_with(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<QualifiedAddress> {
        self.nearest_ancestor_with_observations(
            site,
            context,
            &observation_lookup(template_format, &|at| {
                form_item(at).map(FormObservation::Item)
            }),
        )
    }
}

macro_rules! operation_descriptors {
    ($(($operation:path)),+ $(,)?) => {
        const OPERATION_DESCRIPTORS: &[OperationDescriptor] = &[$($operation.descriptor()),+];
    };
}

operation_descriptors!(
    (unica_metadata::apply_operations::MetadataOperation::ObjectBorrow),
    (unica_metadata::apply_operations::MetadataOperation::ObjectCreate),
    (unica_metadata::apply_operations::MetadataOperation::ObjectRemove),
    (PropertiesOperation::Set),
    (unica_metadata::apply_operations::MetadataOperation::ContentSet),
    (unica_metadata::apply_operations::MetadataOperation::RelationAdd),
    (unica_metadata::apply_operations::MetadataOperation::RelationRemove),
    (unica_metadata::apply_operations::MetadataOperation::RelationReplace),
    (unica_metadata::apply_operations::MetadataOperation::HelpCreate),
    (unica_metadata::apply_operations::MetadataOperation::AttributeAdd),
    (unica_metadata::apply_operations::MetadataOperation::AttributeSet),
    (unica_metadata::apply_operations::MetadataOperation::AttributeRemove),
    (unica_metadata::apply_operations::MetadataOperation::AttributeCopy),
    (unica_metadata::apply_operations::MetadataOperation::TabularSectionAdd),
    (unica_metadata::apply_operations::MetadataOperation::TabularSectionSet),
    (unica_metadata::apply_operations::MetadataOperation::TabularSectionRemove),
    (unica_metadata::apply_operations::MetadataOperation::DimensionAdd),
    (unica_metadata::apply_operations::MetadataOperation::DimensionSet),
    (unica_metadata::apply_operations::MetadataOperation::DimensionRemove),
    (unica_metadata::apply_operations::MetadataOperation::ResourceAdd),
    (unica_metadata::apply_operations::MetadataOperation::ResourceSet),
    (unica_metadata::apply_operations::MetadataOperation::ResourceRemove),
    (unica_metadata::apply_operations::MetadataOperation::EnumValueAdd),
    (unica_metadata::apply_operations::MetadataOperation::EnumValueSet),
    (unica_metadata::apply_operations::MetadataOperation::EnumValueRemove),
    (unica_metadata::apply_operations::MetadataOperation::ColumnAdd),
    (unica_metadata::apply_operations::MetadataOperation::ColumnSet),
    (unica_metadata::apply_operations::MetadataOperation::ColumnRemove),
    (FormOperation::Add),
    (FormOperation::Set),
    (FormOperation::Remove),
    (unica_metadata::apply_operations::MetadataOperation::TemplateAdd),
    (unica_metadata::apply_operations::MetadataOperation::TemplateSet),
    (unica_metadata::apply_operations::MetadataOperation::TemplateRemove),
    (unica_metadata::apply_operations::MetadataOperation::CommandAdd),
    (unica_metadata::apply_operations::MetadataOperation::CommandSet),
    (unica_metadata::apply_operations::MetadataOperation::CommandRemove),
    (unica_metadata::apply_operations::MetadataOperation::PredefinedItemAdd),
    (unica_metadata::apply_operations::MetadataOperation::PredefinedItemSet),
    (unica_metadata::apply_operations::MetadataOperation::PredefinedItemRemove),
    (FormOperation::ItemAdd),
    (FormOperation::ItemRemove),
    (FormOperation::ItemMove),
    (FormOperation::AttributeAdd),
    (FormOperation::CommandAdd),
    (FormOperation::EventBind),
    (FormOperation::PropsSet),
    (RoleOperation::Create),
    (RoleOperation::RightAdd),
    (RoleOperation::RightSet),
    (Primitive::DataSourceAdd),
    (Primitive::DataSourceSet),
    (Primitive::DataSourceRemove),
    (Primitive::DataSetAdd),
    (Primitive::DataSetSet),
    (Primitive::DataSetRemove),
    (Primitive::FieldAdd),
    (Primitive::FieldSet),
    (Primitive::FieldRemove),
    (Primitive::FieldRoleSet),
    (Primitive::ParameterAdd),
    (Primitive::ParameterSet),
    (Primitive::ParameterRemove),
    (Primitive::ParameterRename),
    (Primitive::ParameterReorder),
    (Primitive::CalculatedFieldAdd),
    (Primitive::CalculatedFieldRemove),
    (Primitive::TotalAdd),
    (Primitive::TotalRemove),
    (Primitive::VariantAdd),
    (Primitive::VariantSet),
    (Primitive::VariantRemove),
    (Primitive::QuerySet),
    (Primitive::QueryPatch),
    (Primitive::FilterAdd),
    (Primitive::FilterSet),
    (Primitive::FilterRemove),
    (Primitive::FilterClear),
    (Primitive::SelectionAdd),
    (Primitive::SelectionClear),
    (Primitive::OrderAdd),
    (Primitive::OrderClear),
    // Параметры данных есть только у самих настроек варианта: у элемента
    // структуры их коллекции нет.
    (Primitive::DataParameterAdd),
    (Primitive::DataParameterSet),
    (Primitive::OutputParameterSet),
    (Primitive::ConditionalAppearanceAdd),
    (Primitive::ConditionalAppearanceClear),
    (Primitive::StructureAdd),
    (Primitive::StructureSet),
    (Primitive::StructurePatch),
    (Primitive::StructureRemove),
    (Primitive::DataSetLinkAdd),
    (Primitive::CalculatedFieldSet),
    (Primitive::TotalSet),
    (Primitive::SelectionSet),
    (Primitive::SelectionRemove),
    (Primitive::OrderSet),
    (Primitive::OrderRemove),
    (Primitive::DataParameterRemove),
    (Primitive::OutputParameterRemove),
    (Primitive::ConditionalAppearanceRemove),
    (Primitive::DataSetLinkSet),
    (Primitive::DataSetLinkRemove),
    (MxlOperation::Set),
    (MxlOperation::AreaAdd),
    (XdtoOperation::AddValueType),
    (XdtoOperation::AddObjectType),
    (XdtoOperation::AddProperty),
    (XdtoOperation::RemoveType),
    (XdtoOperation::RemoveProperty),
    (XdtoOperation::RenameProperty),
    (XdtoOperation::SetType),
    (SubsystemOperation::Create),
    (SubsystemOperation::ContentAdd),
    (SubsystemOperation::ContentRemove),
    (SubsystemOperation::ChildAdd),
    (SubsystemOperation::ChildRemove),
    (SupportOperation::CapabilitySet),
    (SupportOperation::RuleSet),
    (InterfaceOperation::VisibilitySet),
    (InterfaceOperation::PlacementSet),
    (InterfaceOperation::CommandOrderSet),
    (InterfaceOperation::GroupOrderSet),
    // Порядок подсистем верхнего уровня живёт в корневом документе, а
    // маршрута `main:Interface` намеренно нет: корень описывается свойством,
    // а не узлом. Интерфейс подсистемы эту операцию не принимает.
    (InterfaceOperation::SubsystemOrderSet),
    // Панели и начальная страница — документы корня конфигурации, у
    // расширения их нет.
    (InterfaceOperation::PanelsSet),
    (InterfaceOperation::HomePageSet),
    (CodeOperation::Insert),
    (CodeOperation::Replace),
    (EventOperation::Implement),
);

/// Product composition supplies the exact ordered family bindings and the
/// sole semantic Site classifier. Generic operations imports no family owner.
pub(crate) const fn operation_registry() -> OperationRegistry {
    OperationRegistry::new(
        OPERATION_DESCRIPTORS,
        unica_forms::recovery::RETIRED_OPERATIONS,
        RegistryContext::new(operation_site),
    )
}

pub(crate) fn retired_operation(
    name: &str,
) -> Option<unica_operations::recovery::RetiredOperation> {
    operation_registry().retired(name)
}

#[cfg(test)]
fn retired_arguments() -> Vec<unica_operations::recovery::BoundRetiredArgument> {
    operation_registry().retired_arguments()
}

pub(crate) trait OperationRegistryExt {
    fn nearest_applicable_ancestor_with(
        self,
        name: &str,
        site: &OperationSite<'_>,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<(OperationDescriptor, QualifiedAddress)>;
}
impl OperationRegistryExt for OperationRegistry {
    fn nearest_applicable_ancestor_with(
        self,
        name: &str,
        site: &OperationSite<'_>,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<(OperationDescriptor, QualifiedAddress)> {
        self.nearest_ancestor_with_observations(
            name,
            site,
            &observation_lookup(template_format, &|at| {
                form_item(at).map(FormObservation::Item)
            }),
        )
    }
}

pub(crate) trait RetiredOperationExt {
    fn replacement_target(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<QualifiedAddress>;
}
impl RetiredOperationExt for unica_operations::recovery::RetiredOperation {
    fn replacement_target(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        template_format: &dyn Fn(&QualifiedAddress) -> Option<TemplateFormat>,
        form_item: &dyn Fn(&QualifiedAddress) -> Option<FormItemRole>,
    ) -> Option<QualifiedAddress> {
        self.replacement_with_observations(
            site,
            context,
            &observation_lookup(template_format, &|at| {
                form_item(at).map(FormObservation::Item)
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        operation_registry, operation_site, retired_arguments, source_set_traits_for_read,
        ApplyRequest, EventOperation, FormOperation, MxlOperation, OperationDescriptor,
        OperationFamily, OperationSkeleton, Primitive, RoleOperation, SupportOperation,
        TemplateFormat, OPERATION_DESCRIPTORS,
    };
    use super::{OperationDescriptorExt, OperationRegistryExt, RetiredOperationExt};
    use serde_json::{json, Value};
    use unica_address::address::QualifiedAddress;
    use unica_bsl::profile::PlatformProfile;
    use unica_metadata::descriptor_operations::PropertiesOperation;
    use unica_source_contract::route::RouteClass;
    use unica_source_contract::SourceSetKind;
    use unica_xdto::operations::XdtoOperation;

    #[test]
    fn implemented_operations_have_contracts_and_target_skeletons_have_no_empty_address() {
        for descriptor in OPERATION_DESCRIPTORS {
            if descriptor.family() != OperationFamily::Properties {
                assert!(
                    descriptor.argument_contract().is_some(),
                    "{}",
                    descriptor.name()
                );
            }
            assert_ne!(
                serde_json::to_value(descriptor.copyable_skeleton()).unwrap()["args"].get("at"),
                Some(&json!("")),
                "{}",
                descriptor.name()
            );
        }
    }

    fn address(raw: &str) -> QualifiedAddress {
        QualifiedAddress::parse(raw).unwrap_or_else(|error| panic!("{raw}: {error}"))
    }

    fn applies_in(source: SourceSetKind, op: &str, raw: &str) -> bool {
        let at = address(raw);
        let site = operation_site(
            &at,
            source_set_traits_for_read(source, PlatformProfile::v8_3_27()),
        )
        .unwrap_or_else(|| panic!("{raw} has no route"));
        crate::domain::apply::operation_registry()
            .lookup_at(op, &site)
            .unwrap_or_else(|| panic!("{op} is registered"))
            .applies_at(&site)
    }

    #[test]
    fn form_metadata_does_not_advertise_or_accept_owner_and_content_writers() {
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for raw in [
            "main:CommonForm.Main.Metadata",
            "main:Document.Order.Form.Main.Metadata",
        ] {
            let at = address(raw);
            let site = operation_site(&at, traits).unwrap();
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
        let registry = crate::domain::apply::operation_registry();
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
        assert!(request("main:Catalog.Orders.Form.ItemForm").is_ok());
        for from in [
            "main:Catalog.Orders.Form",
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
        let descriptor = crate::domain::apply::operation_registry()
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

    fn padded_created_name_scope(create: &str, kind: &str) {
        for source in ["main", "extension"] {
            for (raw, canonical) in [
                (" Reader ", "Reader"),
                ("\u{a0}Читатель\u{2003}", "Читатель"),
            ] {
                let target = format!("{source}:{kind}.{canonical}");
                let ops = json!([
                    {"op":create,"args":{"values":{"name":raw}}},
                    {"op":"props.set","args":{"at":target,"values":{"Comment":"x"}}}
                ]);
                let result =
                    parse(json!({"at":format!("{source}:Configuration"),"ops":ops,"dryRun":true}));
                assert!(result.is_ok(), "{create} {raw:?} {source}: {result:?}");
                assert_eq!(result.unwrap().ops()[0].args()["values"]["name"], raw);
            }
        }
    }

    #[test]
    fn padded_role_creation_scope_uses_the_canonical_name() {
        padded_created_name_scope("role.create", "Role");
    }

    #[test]
    fn padded_subsystem_creation_scope_uses_the_canonical_name() {
        padded_created_name_scope("subsystem.create", "Subsystem");
    }

    #[test]
    fn padded_object_create_does_not_extend_scope_with_a_trimmed_name() {
        for raw in [" Enabled ", "\u{a0}Enabled\u{2003}"] {
            let error = parse(json!({"at":"main:Configuration","ops":[
                {"op":"object.create","args":{"values":{"kind":"Constant","name":raw}}},
                {"op":"props.set","args":{"at":"main:Constant.Enabled","values":{"Comment":"x"}}}
            ]}))
            .unwrap_err();
            assert_eq!(error.location(), "ops[1].args.at");
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
        fn typed<O: unica_operations::FamilyOperation>(
            descriptor: OperationDescriptor,
        ) -> Option<bool> {
            descriptor.operation::<O>().map(|operation| {
                descriptor.family() == operation.family()
                    && descriptor.name() == operation.name()
                    && descriptor.address_shape() == operation.address_shape()
                    && descriptor.skeleton() == operation.skeleton()
            })
        }
        let bindings = [
            typed::<XdtoOperation>(descriptor),
            typed::<PropertiesOperation>(descriptor),
            typed::<Primitive>(descriptor),
            typed::<MxlOperation>(descriptor),
            typed::<EventOperation>(descriptor),
            typed::<RoleOperation>(descriptor),
            typed::<SupportOperation>(descriptor),
            typed::<unica_metadata::apply_operations::MetadataOperation>(descriptor),
            typed::<FormOperation>(descriptor),
            typed::<unica_bsl::operations::CodeOperation>(descriptor),
            typed::<unica_metadata::subsystem_operations::SubsystemOperation>(descriptor),
            typed::<unica_metadata::interface_operations::InterfaceOperation>(descriptor),
        ];
        bindings.iter().flatten().count() == 1 && bindings.iter().flatten().all(|valid| *valid)
    }

    #[test]
    fn properties_registration_requires_its_typed_identity_and_proven_context() {
        let valid = PropertiesOperation::descriptor(PropertiesOperation::Set);
        assert!(typed_registration_is_consistent(valid));
        assert!(valid.argument_contract().is_none());
        assert_eq!(
            OPERATION_DESCRIPTORS
                .iter()
                .filter(|row| row.family() == OperationFamily::Properties)
                .collect::<Vec<_>>(),
            vec![&valid]
        );
        assert_eq!(valid.operation::<RoleOperation>(), None);
        assert_eq!(
            valid.operation::<unica_metadata::apply_operations::MetadataOperation>(),
            None
        );
        assert_ne!(
            valid,
            unica_metadata::apply_operations::MetadataOperation::AttributeAdd.descriptor()
        );
        assert_eq!(valid.name(), "props.set");
        assert_eq!(valid.family(), OperationFamily::Properties);
        assert_eq!(valid.skeleton(), OperationSkeleton::Values);
        for (kind, raw, expected) in [
            (
                SourceSetKind::Configuration,
                "main:CommonModule.Utilities",
                true,
            ),
            (SourceSetKind::Configuration, "main:Role.Reader", true),
            (SourceSetKind::Configuration, "main:Subsystem.Sales", true),
            (SourceSetKind::Configuration, "main:Configuration", true),
            (SourceSetKind::Extension, "main:Configuration", true),
            (
                SourceSetKind::ExternalProcessor,
                "main:Configuration",
                false,
            ),
            (
                SourceSetKind::Configuration,
                "main:Catalog.Items.Attribute.Code",
                false,
            ),
            (
                SourceSetKind::Configuration,
                "main:Subsystem.Sales.Subsystem.Child",
                true,
            ),
            (
                SourceSetKind::Configuration,
                "main:Subsystem.Sales.Interface",
                false,
            ),
        ] {
            let source = source_set_traits_for_read(kind, PlatformProfile::v8_3_27());
            let at = QualifiedAddress::parse(raw).unwrap();
            let site = operation_site(&at, source).unwrap();
            assert_eq!(
                valid.argument_contract_at(&site).is_some(),
                expected,
                "{raw}"
            );
            assert_eq!(valid.applies_at(&site), expected, "{raw}");
        }
    }

    #[test]
    fn xdto_registration_requires_its_typed_identity_and_contract() {
        for &operation in XdtoOperation::ALL {
            let valid = operation.descriptor();
            assert!(typed_registration_is_consistent(valid));
            assert_eq!(valid.operation::<XdtoOperation>(), Some(operation));
            assert_eq!(valid.operation::<RoleOperation>(), None);
            assert_eq!(
                valid.operation::<unica_metadata::apply_operations::MetadataOperation>(),
                None
            );
            assert!(valid.argument_contract().is_some());
            assert_eq!(
                crate::domain::apply::operation_registry()
                    .descriptors()
                    .iter()
                    .filter(|descriptor| descriptor.operation::<XdtoOperation>() == Some(operation))
                    .count(),
                1
            );
            assert_ne!(valid, RoleOperation::Create.descriptor());
        }
        assert_eq!(
            crate::domain::apply::operation_registry()
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.family() == OperationFamily::Xdto)
                .count(),
            XdtoOperation::ALL.len()
        );
    }

    #[test]
    fn role_registration_rejects_missing_and_mismatched_typed_operations() {
        for operation in [
            RoleOperation::Create,
            RoleOperation::RightAdd,
            RoleOperation::RightSet,
        ] {
            let valid = operation.descriptor();
            assert!(typed_registration_is_consistent(valid));
            assert_eq!(valid.operation::<RoleOperation>(), Some(operation));
            assert_eq!(valid.operation::<SupportOperation>(), None);
            assert_eq!(
                valid.operation::<unica_metadata::apply_operations::MetadataOperation>(),
                None
            );
            assert_ne!(valid, SupportOperation::CapabilitySet.descriptor());
            assert_eq!(valid.skeleton(), operation.skeleton());
            assert_eq!(valid.address_shape(), operation.applicability().shape());
        }
        assert_ne!(
            RoleOperation::Create.descriptor(),
            RoleOperation::RightSet.descriptor()
        );
    }

    #[test]
    fn support_registration_rejects_missing_and_mismatched_typed_operations() {
        for operation in [SupportOperation::CapabilitySet, SupportOperation::RuleSet] {
            let valid = operation.descriptor();
            assert!(typed_registration_is_consistent(valid));
            assert_eq!(valid.operation::<SupportOperation>(), Some(operation));
            assert_eq!(valid.operation::<RoleOperation>(), None);
            assert_eq!(
                valid.operation::<unica_metadata::apply_operations::MetadataOperation>(),
                None
            );
            assert_ne!(valid, RoleOperation::Create.descriptor());
        }
        assert_ne!(
            SupportOperation::CapabilitySet.descriptor(),
            SupportOperation::RuleSet.descriptor()
        );
    }

    #[test]
    fn dcs_mxl_and_event_registration_reject_missing_and_mismatched_typed_operations() {
        let descriptors = Primitive::ALL
            .iter()
            .copied()
            .map(Primitive::descriptor)
            .chain(
                MxlOperation::ALL
                    .iter()
                    .copied()
                    .map(MxlOperation::descriptor),
            )
            .chain(
                EventOperation::ALL
                    .iter()
                    .copied()
                    .map(EventOperation::descriptor),
            );
        for valid in descriptors {
            assert!(typed_registration_is_consistent(valid));
            assert_eq!(valid.operation::<FormOperation>(), None);
            assert_eq!(valid.operation::<PropertiesOperation>(), None);
            assert_ne!(valid, FormOperation::EventBind.descriptor());
            let rows = OPERATION_DESCRIPTORS
                .iter()
                .filter(|row| **row == valid)
                .collect::<Vec<_>>();
            assert_eq!(rows, vec![&valid]);
        }
        for descriptor in OPERATION_DESCRIPTORS {
            assert!(typed_registration_is_consistent(*descriptor));
        }
    }

    #[test]
    fn contextual_member_contracts_keep_owner_and_section_discriminators() {
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for (at, allows_fill) in [
            ("main:Document.Order", true),
            ("main:Document.Order.TabularSection.Lines", false),
            ("main:Report.Sales", false),
            ("main:Report.Sales.TabularSection.Lines", true),
        ] {
            let at = QualifiedAddress::parse(at).unwrap();
            let site = operation_site(&at, traits).unwrap();
            let descriptor = registry.lookup_at("attribute.add", &site).unwrap();
            assert_eq!(
                descriptor.operation::<unica_metadata::apply_operations::MetadataOperation>(),
                Some(unica_metadata::apply_operations::MetadataOperation::AttributeAdd)
            );
            assert_eq!(descriptor.operation::<FormOperation>(), None);
            let contract = descriptor.argument_contract_at(&site).unwrap();
            assert_eq!(contract.help, "skills/meta-edit/SKILL.md");
            let fields = &contract.schema["properties"]["items"]["items"]["properties"];
            assert_eq!(fields.get("fillValue").is_some(), allows_fill, "{at}");
            assert!(fields.get("savedData").is_none());

            let member = QualifiedAddress::parse(&format!("{at}.Attribute.Total")).unwrap();
            let site = operation_site(&member, traits).unwrap();
            let descriptor = registry.lookup_at("attribute.set", &site).unwrap();
            assert_eq!(
                descriptor.operation::<unica_metadata::apply_operations::MetadataOperation>(),
                Some(unica_metadata::apply_operations::MetadataOperation::AttributeSet)
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
            let site = operation_site(&at, traits).unwrap();
            let descriptor = registry.lookup_at(name, &site).unwrap();
            assert_eq!(descriptor.operation::<FormOperation>(), Some(expected));
            assert_eq!(
                descriptor.operation::<unica_metadata::apply_operations::MetadataOperation>(),
                None
            );
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
        let metadata = unica_metadata::apply_operations::MetadataOperation::ALL
            .iter()
            .copied()
            .map(unica_metadata::apply_operations::MetadataOperation::descriptor);
        let forms = [
            FormOperation::descriptor(FormOperation::Add),
            FormOperation::descriptor(FormOperation::Set),
            FormOperation::descriptor(FormOperation::Remove),
            FormOperation::descriptor(FormOperation::ItemAdd),
            FormOperation::descriptor(FormOperation::ItemRemove),
            FormOperation::descriptor(FormOperation::ItemMove),
            FormOperation::descriptor(FormOperation::AttributeAdd),
            FormOperation::descriptor(FormOperation::CommandAdd),
            FormOperation::descriptor(FormOperation::EventBind),
        ];
        for valid in metadata.chain(forms) {
            assert!(typed_registration_is_consistent(valid));
            assert_eq!(valid.operation::<PropertiesOperation>(), None);
            if valid.family() == OperationFamily::Metadata {
                assert!(valid
                    .operation::<unica_metadata::apply_operations::MetadataOperation>()
                    .is_some());
                assert_eq!(valid.operation::<FormOperation>(), None);
            } else {
                assert!(valid.operation::<FormOperation>().is_some());
                assert_eq!(
                    valid.operation::<unica_metadata::apply_operations::MetadataOperation>(),
                    None
                );
            }
        }
        let metadata_attribute =
            unica_metadata::apply_operations::MetadataOperation::AttributeAdd.descriptor();
        let form_attribute = FormOperation::AttributeAdd.descriptor();
        assert_eq!(metadata_attribute.name(), form_attribute.name());
        assert_ne!(metadata_attribute, form_attribute);
        assert_ne!(
            metadata_attribute.address_shape(),
            form_attribute.address_shape()
        );
        let descriptor = crate::domain::apply::operation_registry()
            .named("template.set")
            .find(|d| d.family() == OperationFamily::Metadata)
            .unwrap();
        assert_eq!(
            descriptor.operation::<unica_metadata::apply_operations::MetadataOperation>(),
            Some(unica_metadata::apply_operations::MetadataOperation::TemplateSet)
        );
        assert!(typed_registration_is_consistent(descriptor));
    }

    #[test]
    fn operation_registry_is_exact_closed_unique_and_drives_skeletons_and_dispatch() {
        let expected = [
            "object.borrow",
            "object.create",
            "object.remove",
            "props.set",
            "content.set",
            "relation.add",
            "relation.remove",
            "relation.replace",
            "help.create",
            "attribute.add",
            "attribute.set",
            "attribute.remove",
            "attribute.copy",
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
            "props.set",
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
            "property.rename",
            "type.set",
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
            "panels.set",
            "homePage.set",
            "code.insert",
            "code.replace",
            "event.implement",
        ];
        let registry = crate::domain::apply::operation_registry();
        assert_eq!(expected.len(), 129);
        assert_eq!(registry.names(), expected);
        // Реквизит и команда формы названы по своему узлу, как у объекта
        // метаданных (#1465), свойства формы — как свойства описания (#1984):
        // имя одно, семейство выбирает адрес. Других повторов в реестре нет.
        const SHARED_NAMES: &[(&str, OperationFamily)] = &[
            ("attribute.add", OperationFamily::Metadata),
            ("command.add", OperationFamily::Metadata),
            ("props.set", OperationFamily::Properties),
        ];
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
        assert_eq!(
            unica_metadata::apply_operations::MetadataOperation::ALL.len(),
            36
        );
        assert_eq!(
            registry
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.family() == OperationFamily::Metadata)
                .count(),
            unica_metadata::apply_operations::MetadataOperation::ALL.len()
        );
        for operation in unica_metadata::apply_operations::MetadataOperation::ALL {
            assert_eq!(
                registry
                    .descriptors()
                    .iter()
                    .filter(|descriptor| descriptor
                        .operation::<unica_metadata::apply_operations::MetadataOperation>(
                    ) == Some(*operation))
                    .count(),
                1,
                "every Metadata identity must be registered exactly once: {operation:?}"
            );
        }
        for (name, first) in SHARED_NAMES {
            let families = registry
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.name() == *name)
                .map(|descriptor| descriptor.family())
                .collect::<Vec<_>>();
            assert_eq!(families, [*first, OperationFamily::Form], "{name}");
        }

        assert_eq!(
            registry.lookup("event.implement").unwrap().family(),
            OperationFamily::Event
        );
        assert!(registry.lookup("module.create").is_none());
        assert!(applies("props.set", "main:Subsystem.Sales"));
        assert!(applies("props.set", "main:Document.Order"));

        let event_at = address("main:Catalog.Items.Module.Object.Event.OnWrite");
        let event_site = operation_site(
            &event_at,
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        let skeletons = registry.copyable_skeletons(&event_site);
        let event = skeletons
            .iter()
            .map(|item| serde_json::to_value(item).unwrap())
            .find(|value| value["op"] == "event.implement")
            .expect("Event.can must be projected from the dispatch registry");
        assert_eq!(event, json!({"op": "event.implement"}));
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
            // Копия — только реквизит табличной части справочника.
            (
                "attribute.copy",
                "main:Catalog.Goods.TabularSection.Prices.Attribute.Price",
                true,
            ),
            (
                "attribute.copy",
                "main:Document.Order.TabularSection.Goods.Attribute.Price",
                false,
            ),
            ("attribute.copy", "main:Catalog.Goods.Attribute.Code", false),
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
            (
                "property.rename",
                "main:XDTOPackage.Exchange.Type.Order.Property.Lines.Property.Item",
                true,
            ),
            (
                "property.rename",
                "main:XDTOPackage.Exchange.Type.Order",
                false,
            ),
            ("type.set", "main:XDTOPackage.Exchange.Type.Order", true),
            (
                "type.set",
                "main:XDTOPackage.Exchange.Type.Order.Property.Id",
                false,
            ),
            ("type.set", "main:XDTOPackage.Exchange", false),
            // Глобальное свойство пакета — не член типа.
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Property.Id",
                false,
            ),
            (
                "property.rename",
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
        // Панели и начальная страница — документы корня конфигурации.
        for name in ["panels.set", "homePage.set"] {
            assert!(applies(name, "main:Configuration"), "{name}");
            assert!(!applies(name, "main:Subsystem.Sales.Interface"), "{name}");
            assert!(
                !applies_in(SourceSetKind::Extension, name, "main:Configuration"),
                "{name}"
            );
        }
    }

    /// Формат макета — отдельный фильтр с одним владельцем: семейства СКД и
    /// табличного документа применимы только к макету своего формата.
    #[test]
    fn template_format_is_a_separate_filter_for_dcs_and_mxl_families() {
        let at = address("main:Report.Sales.Template.Layout");
        let site = operation_site(
            &at,
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        let registry = crate::domain::apply::operation_registry();
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
        let registry = crate::domain::apply::operation_registry();
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
            let site = operation_site(
                &at,
                source_set_traits_for_read(
                    SourceSetKind::Configuration,
                    PlatformProfile::v8_3_27(),
                ),
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

    #[test]
    fn recovery_reclassifies_dcs_and_form_ancestors_in_the_canonical_classifier() {
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for (raw, initial_route, op, expected, expected_route) in [
            (
                "main:Report.Sales.Template.Schema.DataSet.Main.Field.Amount",
                RouteClass::Dcs,
                "template.set",
                "main:Report.Sales.Template.Schema",
                RouteClass::Metadata,
            ),
            (
                "main:Report.Sales.Template.Schema.DataSet.Main.Field.Amount",
                RouteClass::Dcs,
                "role.create",
                "main:Configuration",
                RouteClass::Configuration,
            ),
            (
                "main:Document.Order.Form.Main.Module.Form.Body",
                RouteClass::Module,
                "code.replace",
                "main:Document.Order.Form.Main.Module.Form",
                RouteClass::Module,
            ),
            (
                "main:Document.Order.Form.Main.Module.Form.Body",
                RouteClass::Module,
                "attribute.add",
                "main:Document.Order.Form.Main",
                RouteClass::Form,
            ),
            (
                "main:Document.Order.Form.Main.Module.Form.Body",
                RouteClass::Module,
                "form.add",
                "main:Document.Order",
                RouteClass::Metadata,
            ),
        ] {
            let at = address(raw);
            let site = operation_site(&at, traits).unwrap();
            assert_eq!(site.route(), initial_route, "{raw}");
            let (_, target) = registry
                .nearest_applicable_ancestor_with(op, &site, &|_| None, &|_| None)
                .unwrap();
            assert_eq!(target.to_string(), expected, "{op} at {raw}");
            let target_site = operation_site(&target, traits).unwrap();
            assert_eq!(target_site.route(), expected_route, "{target}");
            assert_eq!(target_site.source(), site.source());
        }
    }

    #[test]
    fn recovery_candidates_use_the_registry_context_instead_of_the_original_route() {
        use unica_source_contract::context::{OperationSite, RegistryContext, SourceSetTraits};
        fn reject_candidate(_: &QualifiedAddress, _: SourceSetTraits) -> Option<OperationSite<'_>> {
            None
        }
        let registry = crate::domain::apply::operation_registry();
        let blocked = operation_registry().with_context(RegistryContext::new(reject_candidate));
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let child = address("main:Report.Sales.Template.Schema.DataSet.Main.Field.Amount");
        let site = operation_site(&child, traits).unwrap();
        assert!(registry
            .nearest_applicable_ancestor_with("template.set", &site, &|_| None, &|_| None)
            .is_some());
        assert!(blocked
            .nearest_applicable_ancestor_with("template.set", &site, &|_| None, &|_| None)
            .is_none());

        let alias = address("main:Report.Sales.Template.Schema.Query.Main");
        let alias_site = operation_site(&alias, traits).unwrap();
        let query = registry.lookup("query.set").unwrap();
        assert_eq!(
            query
                .retired_query_alias_target(
                    &alias_site,
                    registry.context(),
                    &unica_operations::recovery::no_member_lookup
                )
                .unwrap()
                .to_string(),
            "main:Report.Sales.Template.Schema.DataSet.Main.Query"
        );
        assert!(query
            .retired_query_alias_target(
                &alias_site,
                blocked.context(),
                &unica_operations::recovery::no_member_lookup
            )
            .is_none());

        let data_set = address("main:Report.Sales.Template.Schema.DataSet.Main");
        let data_site = operation_site(&data_set, traits).unwrap();
        let args = json!({"values": {"dataPath": "Amount", "title": "x"}});
        let redirect = registry
            .retired_argument_redirect("field.set", &data_site, args.as_object().unwrap())
            .unwrap();
        assert_eq!(redirect.at.to_string(), child.to_string());
        assert!(blocked
            .retired_argument_redirect("field.set", &data_site, args.as_object().unwrap())
            .is_none());

        let form_child = address("main:Document.Order.Form.Main.Item.Header.Event.OnChange");
        let form_site = operation_site(&form_child, traits).unwrap();
        let retired = crate::domain::apply::retired_operation("element.add").unwrap();
        assert!(retired
            .replacement_target(&form_site, registry.context(), &|_| None, &|_| Some(
                super::FormItemRole::Container
            ))
            .is_some());
        assert!(retired
            .replacement_target(&form_site, blocked.context(), &|_| None, &|_| Some(
                super::FormItemRole::Container
            ))
            .is_none());
    }

    #[test]
    fn classification_fallback_never_grants_writes_or_crosses_source_isolation() {
        use super::source_set_traits_of_source;
        use unica_source_contract::{SourceFormat, SourceProfile};
        let active = SourceProfile::platform_xml_8_3_27_format_2_20();
        let registry = crate::domain::apply::operation_registry();
        let root = address("main:Configuration");
        for (format, profile) in [
            (SourceFormat::Unknown, active),
            (SourceFormat::Edt, active),
            (SourceFormat::Invalid, active),
            (
                SourceFormat::PlatformXml,
                SourceProfile::legacy_workspace_service_compatibility(),
            ),
            (
                SourceFormat::PlatformXml,
                SourceProfile::TestPlatform8_3_28Format2_20,
            ),
            (
                SourceFormat::PlatformXml,
                SourceProfile::TestPlatform8_3_27Format2_21,
            ),
        ] {
            let traits = source_set_traits_of_source(SourceSetKind::Configuration, format, profile);
            assert_eq!(
                traits.exact_writable_profile(),
                None,
                "{format:?} {profile:?}"
            );
            assert_eq!(traits.classification_profile(), active);
            let site = operation_site(&root, traits).unwrap();
            assert!(registry.copyable_skeletons(&site).is_empty());
        }
        for kind in [
            SourceSetKind::ExternalProcessor,
            SourceSetKind::ExternalReport,
        ] {
            let traits = source_set_traits_of_source(kind, SourceFormat::PlatformXml, active);
            assert_eq!(traits.exact_writable_profile(), Some(active));
            let site = operation_site(&root, traits).unwrap();
            assert!(registry.copyable_skeletons(&site).is_empty());
        }
        let extension = source_set_traits_of_source(
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            active,
        );
        let child = address("extension:Document.Order.Form.Main.Item.Header.Event.OnChange");
        let site = operation_site(&child, extension).unwrap();
        let (_, target) = registry
            .nearest_applicable_ancestor_with("role.create", &site, &|_| None, &|_| None)
            .unwrap();
        assert_eq!(target.source_set(), "extension");
        assert_eq!(target.to_string(), "extension:Configuration");
        assert_eq!(
            operation_site(&target, extension).unwrap().source(),
            extension
        );
    }

    /// Каждая запись таблицы снятых аргументов строит адрес листа владельца
    /// того вида, который называет шаблон, и распознаёт снятый ключ по своему
    /// пути. Переехавшая операция на владельце больше не исполнима, и прежний
    /// вызов получает отказ с адресом листа; вызов на самом листе отказа по
    /// таблице не получает. Запись ещё не переехавшей операции не срабатывает.
    #[test]
    fn every_retired_argument_entry_builds_a_routable_member_address() {
        let registry = crate::domain::apply::operation_registry();
        let duplicates = [
            DUPLICATE_ARGUMENT_OPERATIONS,
            SUBNODE_OPERATIONS,
            DCS_NODE_OPERATIONS,
            FORM_OPERATIONS,
        ]
        .concat();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
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
        for entry in retired_arguments()
            .into_iter()
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
                let site = operation_site(&at, traits).unwrap();
                let redirect = entry
                    .redirect(&at, args)
                    .unwrap_or_else(|| panic!("{} {} builds no address", entry.op, entry.key));
                assert_eq!(redirect.location, entry.key);
                let member = if redirect.listing {
                    address(&format!("{}.Member", redirect.at))
                } else {
                    redirect.at.clone()
                };
                let target = operation_site(&member, traits)
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
        for entry in retired_arguments() {
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
        let registry = crate::domain::apply::operation_registry();
        let owner = address("main:Catalog.Items");
        let site = operation_site(
            &owner,
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
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
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let form = "main:Catalog.Items.Form.Item";
        let lookup = |container: &QualifiedAddress, name: &str| {
            assert_eq!(container.to_string(), form);
            match name {
                "Field" => unica_operations::recovery::NamedLookup::Found(vec![
                    "Header".to_string(),
                    "Field".to_string(),
                ]),
                "Header" => {
                    unica_operations::recovery::NamedLookup::Found(vec!["Header".to_string()])
                }
                _ => unica_operations::recovery::NamedLookup::Missing,
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
            let site = operation_site(&at, traits).unwrap();
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
            let site = operation_site(&at, traits).unwrap();
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
                retired_arguments()
                    .into_iter()
                    .any(|entry| entry.op == *op && entry.retired),
                "{op} has a retired entry"
            );
        }
    }

    /// Роль элемента формы сужает словарь: в поле не добавить вложенный
    /// элемент, автоматическую командную панель не удалить отдельно.
    #[test]
    fn a_form_item_role_narrows_element_operations() {
        let registry = crate::domain::apply::operation_registry();
        let at = address("main:Catalog.Items.Form.Item.Item.Header");
        let site = operation_site(
            &at,
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
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
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
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
                json!({"values": {"dataPath": "Amount", "title": "x"}}),
                "values.dataPath",
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
            let site = operation_site(&at, traits).unwrap();
            let args = args.as_object().unwrap();
            let redirect = registry
                .retired_argument_redirect(op, &site, args)
                .unwrap_or_else(|| panic!("{op} at {raw} keeps its retired argument"));
            assert_eq!(redirect.location, location, "{op} at {raw}");
            assert_eq!(redirect.at.to_string(), expected, "{op} at {raw}");
            let target = operation_site(&redirect.at, traits).unwrap();
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
        let site = operation_site(&data_set, traits).unwrap();
        let redirect = registry
            .retired_argument_redirect(
                "field.set",
                &site,
                json!({"values": {"dataPath": "Owner.Code", "title": "x"}})
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
        let site = operation_site(&role, traits).unwrap();
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
                retired_arguments()
                    .into_iter()
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
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
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
            let site = operation_site(&at, traits).unwrap();
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
            if let Some(site) = operation_site(&at, traits) {
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
            let site = operation_site(&at, traits).unwrap();
            let args = args.as_object().unwrap();
            let redirect = registry
                .retired_argument_redirect(op, &site, args)
                .unwrap_or_else(|| panic!("{op} at {raw} keeps its retired argument"));
            assert_eq!(redirect.location, location, "{op} at {raw}");
            assert_eq!(redirect.at.to_string(), expected, "{op} at {raw}");
            assert!(!redirect.ambiguous, "{op} at {raw}");
            let target = operation_site(&redirect.at, traits).unwrap();
            assert!(
                registry.lookup(op).unwrap().applies_at(&target),
                "{op} applies at {expected}"
            );
        }
        for op in DCS_NODE_OPERATIONS {
            assert!(
                retired_arguments()
                    .into_iter()
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
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let schema = "main:Report.Sales.Template.Schema";
        let lookup = |owner: &QualifiedAddress, name: &str| {
            let owner = owner.to_string();
            match (owner.as_str(), name) {
                (owner, "Twin") if owner == schema => {
                    unica_operations::recovery::NamedLookup::Ambiguous(vec![
                        vec!["Left".to_string(), "Twin".to_string()],
                        vec!["Right".to_string(), "Twin".to_string()],
                    ])
                }
                (owner, "Rows") if owner.ends_with(".Setting.Main") => {
                    unica_operations::recovery::NamedLookup::Ambiguous(vec![
                        vec!["Table".to_string(), "Rows".to_string()],
                        vec!["Chart".to_string(), "Rows".to_string()],
                    ])
                }
                (owner, "Details") if owner.ends_with(".Setting.Main") => {
                    unica_operations::recovery::NamedLookup::Found(vec![
                        "Table".to_string(),
                        "Details".to_string(),
                    ])
                }
                _ => unica_operations::recovery::NamedLookup::Missing,
            }
        };
        let lookup = &lookup as unica_operations::recovery::MemberPathLookup<'_>;
        let at = address(&format!("{schema}.DataSet.Main.Query"));
        let site = operation_site(&at, traits).unwrap();
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
        let site = operation_site(&setting, traits).unwrap();
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
        let registry = crate::domain::apply::operation_registry();
        use super::FormItemRole;
        let form = "main:Catalog.Items.Form.List";
        let bar = format!("{form}.Item.List.Item.ListCommandBar");
        let field = format!("{form}.Item.Header.Item.Code");
        let role = |at: &QualifiedAddress| match at.to_string() {
            text if text == bar => Some(FormItemRole::AutoCommandBar),
            text if text == field => Some(FormItemRole::Leaf),
            _ => Some(FormItemRole::Container),
        };
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
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
            let site = operation_site(&at, traits).unwrap();
            let target = crate::domain::apply::retired_operation(op)
                .unwrap()
                .replacement_target(&site, registry.context(), &|_| None, &role);
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
            crate::domain::apply::retired_operation("form.create").expect("form.create is retired");
        assert_eq!(retired.replacement(), "form.add");
        let registry = crate::domain::apply::operation_registry();
        assert!(registry.lookup("form.create").is_none());
        assert!(registry.lookup(retired.replacement()).is_some());
        for (raw, expected) in [
            ("main:Catalog.Items.Form.Choice", "main:Catalog.Items"),
            ("main:Catalog.Items", "main:Catalog.Items"),
        ] {
            let target = retired.replacement_at(&address(raw));
            assert_eq!(target.to_string(), expected, "{raw}");
            let site = operation_site(
                &target,
                source_set_traits_for_read(
                    SourceSetKind::Configuration,
                    PlatformProfile::v8_3_27(),
                ),
            )
            .unwrap();
            assert!(
                registry.lookup("form.add").unwrap().applies_at(&site),
                "{raw}"
            );
        }
        assert!(crate::domain::apply::retired_operation("form.add").is_none());
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
            let retired =
                crate::domain::apply::retired_operation(op).expect("the old name is retired");
            assert_eq!(retired.replacement(), replacement, "{op}");
            assert!(registry.lookup(op).is_none(), "{op}");
            let at = address(raw);
            let target = retired.replacement_at(&at);
            assert_eq!(target, at, "{op} at {raw}");
            let site = operation_site(
                &target,
                source_set_traits_for_read(
                    SourceSetKind::Configuration,
                    PlatformProfile::v8_3_27(),
                ),
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
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
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
            let site = operation_site(&at, traits).unwrap();
            let args = args.as_object().unwrap();
            let redirect = registry
                .retired_argument_redirect(op, &site, args)
                .unwrap_or_else(|| panic!("{op} at {raw} keeps its retired argument"));
            assert_eq!(redirect.location, location, "{op} at {raw}");
            assert_eq!(redirect.at.to_string(), expected, "{op} at {raw}");
            assert!(!redirect.listing, "{op} at {raw}");
            let target = operation_site(&redirect.at, traits).unwrap();
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
        let site = operation_site(&at, traits).unwrap();
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
        let site = operation_site(&at, traits).unwrap();
        let args = json!({"values": {"dataSet": "Member", "query": "ВЫБРАТЬ 1"}});
        let args = args.as_object().unwrap();
        let lookup = |schema: &QualifiedAddress, name: &str| {
            assert_eq!(schema.to_string(), template);
            if name == "Member" {
                unica_operations::recovery::NamedLookup::Found(vec![
                    "Union".to_string(),
                    "Member".to_string(),
                ])
            } else {
                unica_operations::recovery::NamedLookup::Missing
            }
        };
        for (lookup, expected) in [
            (
                &lookup as unica_operations::recovery::MemberPathLookup<'_>,
                format!("{template}.DataSet.Union.DataSet.Member.Query"),
            ),
            (
                &unica_operations::recovery::no_member_lookup
                    as unica_operations::recovery::MemberPathLookup<'_>,
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
        let site = operation_site(&data_set, traits).unwrap();
        assert_eq!(
            registry
                .lookup("query.set")
                .unwrap()
                .nearest_applicable_ancestor(&site, registry.context(), &|_| None)
                .map(|at| at.to_string()),
            Some(format!("{template}.DataSet.Main.Query"))
        );
        // Якорь в коде — аргумент, а не адрес: он не снят.
        let module = address("main:CommonModule.Shared");
        let site = operation_site(&module, traits).unwrap();
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
                retired_arguments()
                    .into_iter()
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
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let redirect = |op: &str, raw: &str, args: Value| {
            let at = address(raw);
            let site = operation_site(&at, traits).unwrap();
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
            let site = operation_site(&at, traits).unwrap();
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
        let registry = crate::domain::apply::operation_registry();
        let traits =
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        for (op, raw) in [
            ("commandOrder.set", "main:Subsystem.Sales.Interface.Group"),
            (
                "property.remove",
                "main:XDTOPackage.Exchange.Type.Order.Property",
            ),
        ] {
            let at = address(raw);
            let site = operation_site(&at, traits).unwrap();
            let nearest = registry
                .lookup(op)
                .unwrap()
                .nearest_applicable_ancestor(&site, registry.context(), &|_| None)
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
        let registry = crate::domain::apply::operation_registry();
        let owner = address("main:Report.Sales");
        let site = operation_site(
            &owner,
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
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
        let misdirected = unica_operations::recovery::RetiredArgument {
            op: "attribute.set",
            key: "values.name",
            address: "{at}.TabularSection.{value}",
            retired: true,
        };
        let document = address("main:Document.Order");
        let site = operation_site(
            &document,
            source_set_traits_for_read(SourceSetKind::Configuration, PlatformProfile::v8_3_27()),
        )
        .unwrap();
        assert_eq!(
            unica_operations::recovery::retired_argument_redirect_in(
                &[misdirected],
                registry.lookup("attribute.set").unwrap(),
                &site,
                registry.context(),
                json!({"values": {"name": "Goods"}}).as_object().unwrap(),
                &unica_operations::recovery::no_member_lookup,
            ),
            None
        );
        // Вложенная позиция в массиве называет номер элемента.
        let nested = unica_operations::recovery::RetiredArgument {
            op: "item.add",
            key: "items[].into",
            address: "{item}",
            retired: true,
        };
        let form = address("main:Document.Order.Form.Main");
        let redirect = nested
            .bind(FormOperation::ItemAdd.descriptor())
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
            .bind(FormOperation::ItemAdd.descriptor())
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
        let registry = crate::domain::apply::operation_registry();
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
            ("event.implement", json!({"op": "event.implement"})),
            // The owner is the address; the removed entries are `items`, as
            // the subsystem decoder has always required.
            (
                "content.remove",
                json!({"op": "content.remove", "args": {"items": []}}),
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
            assert!(value.get("args").is_none_or(Value::is_object));
            assert_ne!(value["args"].get("at"), Some(&json!("")));
            assert!(value.to_string().len() <= 256, "{value}");
        }
    }
}
