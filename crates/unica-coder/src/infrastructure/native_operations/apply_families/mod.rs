mod borrow;
pub(crate) mod dcs_mxl;
pub(crate) mod form_resource;
pub(crate) mod metadata;
mod request;

use crate::domain::apply::{ApplyRequest, OperationFamily};
use crate::domain::project_sources::SourceSetKind;
use crate::infrastructure::native_operations::apply::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState,
    PlannedApplyEffects,
};
use crate::infrastructure::workspace_actor::{ApplyAdmission, ProviderRootBinding};
use request::{reconcile_effects, IndexedPlanOperation, ProvisionalApplyEffect};

/// The writable platform XML profile is single (`DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE`):
/// a write outside it refuses before the first byte. The retired v0.12
/// mutators proved this through the dispatcher's format guard; the canonical
/// planner proves it here, over the owner chain of the addressed node, read
/// through the staged state so no ambient file is touched after admission.
fn refuse_targets_outside_writable_profile(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    staged: &mut ApplyStagedState,
) -> Result<(), ApplyPlanError> {
    let at = request.at().to_string();
    for relative in writable_profile_owner_chain(request, binding.source_kind()) {
        let Some(bytes) = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, at.clone()))?
        else {
            continue;
        };
        if let Some(finding) =
            crate::infrastructure::format_guard::classify_staged_platform_xml_root(
                &relative, &bytes,
            )
        {
            return Err(
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message).at_path(at),
            );
        }
    }
    Ok(())
}

/// Operations that change the vendor-support state itself: they are the way
/// to unlock an object, so the lock they edit never refuses them.
const SUPPORT_STATE_OPERATIONS: &[&str] = &["supportCapability.set", "supportRule.set"];

/// Vendor-support gate of the canonical mutation path. When the actor's
/// support policy is `deny`, a request whose owner is a vendor object with
/// the rule `locked`, or whose configuration is on support with changes
/// disabled, refuses `invalid_state` before any family stages a byte. The
/// owner is the top-level metadata object of the addressed node, or the
/// configuration root for root-level targets; nested forms, templates and
/// attributes inherit the rule of their owner, as they do in the platform.
fn refuse_targets_on_locked_vendor_objects(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    staged: &mut ApplyStagedState,
) -> Result<(), ApplyPlanError> {
    use crate::infrastructure::native_operations::common::{
        parse_support_state_compat_bytes, support_root_uuid_from_bytes,
    };
    use crate::infrastructure::support_policy_evidence::SupportPolicyMode;

    if admission.support_policy_mode() != SupportPolicyMode::Deny {
        return Ok(());
    }
    if request
        .ops()
        .iter()
        .all(|operation| SUPPORT_STATE_OPERATIONS.contains(&operation.name()))
    {
        return Ok(());
    }
    let at = request.at().to_string();
    let marker_path = std::path::Path::new("Ext/ParentConfigurations.bin");
    let marker = staged
        .read(marker_path)
        .map_err(|error| ApplyPlanError::staging(error, at.clone()))?;
    let Some(state) = parse_support_state_compat_bytes(marker.as_deref()) else {
        return Ok(());
    };
    if state.removed() {
        return Ok(());
    }
    if !state.global_editing_enabled() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "the configuration is on vendor support with changes disabled; enable editing with `supportCapability.set` first",
        )
        .at_path(at));
    }
    let Some((owner, owner_relative)) = support_owner_relative(request, binding.source_kind())
    else {
        return Ok(());
    };
    let Some(bytes) = staged
        .read(&owner_relative)
        .map_err(|error| ApplyPlanError::staging(error, at.clone()))?
    else {
        return Ok(());
    };
    let locked =
        support_root_uuid_from_bytes(&bytes).and_then(|uuid| state.object_rule(&uuid)) == Some(0);
    if locked {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "`{owner}` is a vendor object with the support rule `locked`; allow editing with `supportRule.set` first"
            ),
        )
        .at_path(at));
    }
    Ok(())
}

/// The descriptor that carries the support rule of the addressed node: the
/// top-level metadata object, or the configuration root for root targets.
fn support_owner_relative(
    request: &ApplyRequest,
    source_kind: SourceSetKind,
) -> Option<(String, std::path::PathBuf)> {
    use crate::domain::address::NodeKind;
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::logical_event_source::metadata_descriptor_relative;

    let segments = request.at().segments();
    let owner = segments.first()?;
    let root_owned = matches!(
        source_kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    );
    if owner.kind() == NodeKind::Configuration || !owner.kind().is_metadata_kind() {
        return root_owned.then(|| {
            (
                "Configuration".to_string(),
                std::path::PathBuf::from("Configuration.xml"),
            )
        });
    }
    let owner_path = format!("{}.{}", owner.kind().as_str(), owner.name()?);
    let owner_address =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner_path).ok()?;
    let relative = metadata_descriptor_relative(&owner_address, source_kind).ok()?;
    Some((owner_path, relative))
}

/// The platform XML documents whose root version decides whether the
/// addressed node may be written: the source-set root descriptor, the owner
/// descriptor of every metadata segment, the wrapper of a nested form,
/// template or command, and the attached content the kind owns.
fn writable_profile_owner_chain(
    request: &ApplyRequest,
    source_kind: SourceSetKind,
) -> Vec<std::path::PathBuf> {
    use crate::domain::address::NodeKind;
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::logical_event_source::{
        attached_resource_relative, metadata_descriptor_relative,
    };

    let mut chain = Vec::new();
    if matches!(
        source_kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        chain.push(std::path::PathBuf::from("Configuration.xml"));
    }
    let segments = request.at().segments();
    let Some(owner) = segments.first() else {
        return chain;
    };
    let Some(owner_name) = owner.name() else {
        return chain;
    };
    if owner.kind() == NodeKind::Configuration || !owner.kind().is_metadata_kind() {
        return chain;
    }
    let owner_path = format!("{}.{}", owner.kind().as_str(), owner_name);
    let Ok(owner_address) = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner_path)
    else {
        return chain;
    };
    if let Ok(descriptor) = metadata_descriptor_relative(&owner_address, source_kind) {
        chain.push(descriptor);
    }
    let owner_resource = match owner.kind() {
        NodeKind::Role => Some("Rights.xml"),
        NodeKind::Subsystem => Some("CommandInterface.xml"),
        NodeKind::CommonForm => Some("Form.xml"),
        _ => None,
    };
    if let Some(resource) = owner_resource {
        if let Ok(content) = attached_resource_relative(&owner_address, resource, source_kind) {
            chain.push(content);
        }
    }
    if let Some(nested) = segments.get(1) {
        let nested_resource = match nested.kind() {
            NodeKind::Form => Some("Form.xml"),
            NodeKind::Template => Some("Template.xml"),
            _ => None,
        };
        if let (Some(resource), Some(nested_name)) = (nested_resource, nested.name()) {
            let nested_path = format!("{owner_path}.{}.{nested_name}", nested.kind().as_str());
            if let Ok(nested_address) =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &nested_path)
            {
                if let Ok(wrapper) = metadata_descriptor_relative(&nested_address, source_kind) {
                    chain.push(wrapper);
                }
                if let Ok(content) =
                    attached_resource_relative(&nested_address, resource, source_kind)
                {
                    chain.push(content);
                }
            }
        }
    }
    chain
}

enum ParsedApplyOperation {
    Borrow(borrow::BorrowOperation),
    Metadata(IndexedPlanOperation<metadata::MetadataPlanOperation>),
    FormResource(IndexedPlanOperation<form_resource::FormResourcePlanOperation>),
    DcsMxl(IndexedPlanOperation<dcs_mxl::DcsMxlPlanOperation>),
    Code(IndexedPlanOperation<crate::infrastructure::native_operations::code::CodePlanOperation>),
    Xdto(IndexedPlanOperation<crate::infrastructure::native_operations::xdto::XdtoPlanOperation>),
    Event(
        IndexedPlanOperation<crate::infrastructure::native_operations::event::EventImplementArgs>,
    ),
    Unsupported(usize),
}

impl ParsedApplyOperation {
    /// Consecutive operations of one family plan as a single batch.
    fn same_family(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A planner that already reconciled its events against its own staged
/// changes reports them tied to every path the batch changed: the request
/// finalizer then keeps an event only while all of those paths stay changed
/// in the final postimage.
/// Object and manager events by their Russian names, as developers write them.
const RUSSIAN_EVENT_NAMES: &[(&str, &str)] = &[
    ("ПередЗаписью", "BeforeWrite"),
    ("ПриЗаписи", "OnWrite"),
    ("ПередУдалением", "BeforeDelete"),
    ("ОбработкаЗаполнения", "Filling"),
    ("ОбработкаПроверкиЗаполнения", "FillCheckProcessing"),
    ("ПриКопировании", "OnCopy"),
    ("ПриУстановкеНовогоНомера", "OnSetNewNumber"),
    ("ПриУстановкеНовогоКода", "OnSetNewCode"),
    ("ОбработкаПроведения", "Posting"),
    ("ОбработкаУдаленияПроведения", "UndoPosting"),
    ("ОбработкаПолученияДанныхВыбора", "ChoiceDataGetProcessing"),
    ("ОбработкаПолученияФормы", "FormGetProcessing"),
    (
        "ОбработкаПолученияПолейПредставления",
        "PresentationFieldsGetProcessing",
    ),
    (
        "ОбработкаПолученияПредставления",
        "PresentationGetProcessing",
    ),
    (
        "ОбработкаФормированияИзВерсииИсторииДанных",
        "GenerateFromDataHistoryVersionProcessing",
    ),
    (
        "ОбработкаПослеЗаписиВерсийИсторииДанных",
        "AfterWriteDataHistoryVersionsProcessing",
    ),
];

const MANAGER_EVENTS: &[&str] = &[
    "ChoiceDataGetProcessing",
    "FormGetProcessing",
    "PresentationFieldsGetProcessing",
    "PresentationGetProcessing",
    "AfterWriteDataHistoryVersionsProcessing",
];

/// `Owner.Name.Event.ПриЗаписи` is how a developer names an object event; the
/// event planner addresses the module that hosts it
/// (`Owner.Name.Module.Object.Event.OnWrite`). Both spellings are accepted:
/// the module role follows the owner kind and the event, and Russian event
/// names map onto the registry. Anything else passes through unchanged.
fn canonical_event_args(args: serde_json::Value) -> serde_json::Value {
    use crate::domain::address::{NodeKind, QualifiedAddress};
    let Some(raw) = args.get("at").and_then(serde_json::Value::as_str) else {
        return args;
    };
    let Ok(address) = QualifiedAddress::parse(raw) else {
        return args;
    };
    let segments = address.segments();
    let english = |name: &str| -> String {
        RUSSIAN_EVENT_NAMES
            .iter()
            .find(|(russian, _)| *russian == name)
            .map_or_else(|| name.to_string(), |(_, english)| (*english).to_string())
    };
    let rewritten = match segments {
        [owner, event] if event.kind() == NodeKind::Event => {
            let (Some(owner_name), Some(event_name)) = (owner.name(), event.name()) else {
                return args;
            };
            let event_name = english(event_name);
            let role = if MANAGER_EVENTS.contains(&event_name.as_str()) {
                "Manager"
            } else {
                match owner.kind() {
                    NodeKind::InformationRegister
                    | NodeKind::AccumulationRegister
                    | NodeKind::AccountingRegister
                    | NodeKind::CalculationRegister => "RecordSet",
                    NodeKind::Constant => "ValueManager",
                    _ => "Object",
                }
            };
            format!(
                "{}:{}.{owner_name}.Module.{role}.Event.{event_name}",
                address.source_set(),
                owner.kind().as_str()
            )
        }
        [owner, module, role, event]
            if module.kind() == NodeKind::Module && event.kind() == NodeKind::Event =>
        {
            let (Some(owner_name), Some(role_name), Some(event_name)) = (
                owner.name(),
                role.name().or(Some(role.kind().as_str())),
                event.name(),
            ) else {
                return args;
            };
            format!(
                "{}:{}.{owner_name}.Module.{role_name}.Event.{}",
                address.source_set(),
                owner.kind().as_str(),
                english(event_name)
            )
        }
        _ => return args,
    };
    let mut args = args;
    if let Some(object) = args.as_object_mut() {
        object.insert("at".to_string(), serde_json::Value::String(rewritten));
    }
    args
}

/// The event planner reports through its own error type; the apply seam
/// speaks one refusal vocabulary, so the kinds map one to one.
fn event_plan_error_to_apply(
    error: crate::infrastructure::native_operations::event::EventPlanError,
) -> ApplyPlanError {
    use crate::infrastructure::native_operations::event::EventPlanErrorKind;
    let kind = match error.kind() {
        EventPlanErrorKind::BadValue => ApplyPlanErrorKind::BadValue,
        EventPlanErrorKind::NotFound => ApplyPlanErrorKind::NotFound,
        EventPlanErrorKind::ProviderUnavailable => ApplyPlanErrorKind::ProviderUnavailable,
        EventPlanErrorKind::InvalidState => ApplyPlanErrorKind::InvalidState,
        EventPlanErrorKind::InvalidSource => ApplyPlanErrorKind::InvalidSource,
        EventPlanErrorKind::Staging(kind) => ApplyPlanErrorKind::Staging(kind),
        EventPlanErrorKind::Postcondition => ApplyPlanErrorKind::Postcondition,
    };
    let path = error.path().map(str::to_string);
    let planned = ApplyPlanError::new(kind, error.to_string());
    match path {
        Some(path) => planned.at_path(path),
        None => planned,
    }
}

fn batch_effects_as_provisional(
    before: &[std::path::PathBuf],
    staged: &ApplyStagedState,
    effects: PlannedApplyEffects,
    op_index: usize,
) -> Vec<ProvisionalApplyEffect> {
    let changed = staged
        .planned_changes()
        .into_iter()
        .map(|change| change.relative_path)
        .filter(|path| !before.contains(path))
        .collect::<Vec<_>>();
    effects
        .into_events_with_paths()
        .into_iter()
        .map(|(event, paths)| {
            // A planner that named the file behind its event keeps that
            // association; only an anonymous event spans the whole batch.
            if paths.is_empty() {
                ProvisionalApplyEffect::spanning(changed.clone(), event, op_index)
            } else {
                ProvisionalApplyEffect::spanning(paths, event, op_index)
            }
        })
        .collect()
}

/// Пишет ли семейство операции в допущенный набор — ответ единственного
/// владельца признака (`SourceSetTraits::writes`), общий с предварительной
/// проверкой `apply` и словарём `can`.
/// Запись реестра для вызова: общее имя (`attribute.add` и `command.add`
/// объекта и формы, #1465) выбирает её адрес вызова, как предварительная
/// проверка `apply`.
pub(super) fn resolved_descriptor(
    operation: &str,
    at: Option<&crate::domain::address::QualifiedAddress>,
    binding: &ProviderRootBinding,
) -> Option<crate::domain::apply::OperationDescriptor> {
    use crate::domain::apply::{OperationRegistry, OperationSite, SourceSetTraits};
    let registry = OperationRegistry::closed();
    let traits = SourceSetTraits::of_source(
        binding.source_kind(),
        binding.source_format(),
        binding.source_profile(),
    );
    at.and_then(|at| OperationSite::new(at, traits))
        .and_then(|site| registry.lookup_at(operation, &site))
        .or_else(|| registry.lookup(operation))
}

fn validate_platform_xml_binding(
    binding: &ProviderRootBinding,
    operation: &str,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let traits = crate::domain::apply::SourceSetTraits::of_source(
        binding.source_kind(),
        binding.source_format(),
        binding.source_profile(),
    );
    let writes = crate::application::v13::apply::dispatch_family(operation)
        .is_none_or(|family| traits.writes(family));
    if !writes {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            crate::domain::apply::UNWRITABLE_SOURCE_MESSAGE,
        )
        .at_path(format!("ops[{op_index}].op")));
    }
    Ok(())
}

/// Применимость операции к своему узлу решает реестр (`domain::apply`), а не
/// разбор цели семейства: тот лишь извлекает части адреса. Проверка здесь
/// повторяет предварительную проверку `apply` для вызовов, которые доходят до
/// планировщика иначе, — чтобы ни одна операция не написала в другой узел.
/// Запись в набор проверяет `validate_platform_xml_binding` тем же владельцем.
fn refuse_misplaced_operation(
    op_index: usize,
    operation: &crate::domain::apply::ApplyOp,
    binding: &ProviderRootBinding,
) -> Result<(), ApplyPlanError> {
    use crate::domain::apply::{OperationSite, SourceSetTraits};
    let Some(descriptor) = resolved_descriptor(operation.name(), Some(operation.at()), binding)
    else {
        return Ok(());
    };
    validate_platform_xml_binding(binding, operation.name(), op_index)?;
    let traits = SourceSetTraits::of_source(
        binding.source_kind(),
        binding.source_format(),
        binding.source_profile(),
    );
    let applies =
        OperationSite::new(operation.at(), traits).is_some_and(|site| descriptor.applies_at(&site));
    if applies {
        return Ok(());
    }
    let kind = operation
        .at()
        .segments()
        .last()
        .map_or("", |segment| segment.kind().as_str());
    Err(ApplyPlanError::new(
        ApplyPlanErrorKind::BadValue,
        format!(
            "apply operation `{}` does not apply to {kind} `{}`",
            operation.name(),
            operation.at()
        ),
    )
    .at_path(format!("ops[{op_index}].args.at")))
}

/// Разбор одной операции планировщиком её семейства, без стадирования.
fn parse_apply_operation(
    op_index: usize,
    operation: &crate::domain::apply::ApplyOp,
    binding: &ProviderRootBinding,
) -> Result<ParsedApplyOperation, ApplyPlanError> {
    if crate::application::v13::apply::dispatch_family(operation.name()).is_none() {
        return Err(hidden_apply_family_unimplemented(op_index));
    }
    refuse_misplaced_operation(op_index, operation, binding)?;
    refuse_inherited_content(op_index, operation, binding)?;
    parse_family_operation(op_index, operation, binding)
}

/// Содержимое заимствованного объекта, которого расширение не несёт
/// (`Rights.xml`, `Form.xml` с модулем формы, `Template.xml`, `Package.bin`),
/// принадлежит родителю: запись туда платформа не сохранит или прочитать
/// нечего. Решение о наследовании — то же, что у чтения
/// (`v13_read_port::inherited_companion`); предметный отказ называет родителя
/// через `props.extends` объекта расширения.
fn refuse_inherited_content(
    op_index: usize,
    operation: &crate::domain::apply::ApplyOp,
    binding: &ProviderRootBinding,
) -> Result<(), ApplyPlanError> {
    use crate::infrastructure::v13_read_port::{companion_owner, inherited_companion};
    let Some(resource) = resolved_descriptor(operation.name(), Some(operation.at()), binding)
        .and_then(|descriptor| descriptor.content_resource())
    else {
        return Ok(());
    };
    let Some(target) = companion_owner(operation.at(), resource) else {
        return Ok(());
    };
    let inherited = inherited_companion(
        &binding.retained_root(),
        binding.source_kind(),
        &target,
        resource,
    )
    .map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::ProviderUnavailable, error)
            .at_path(format!("ops[{op_index}].args.at"))
    })?;
    if inherited.is_none() {
        return Ok(());
    }
    let owner = crate::infrastructure::v13_read::extension_object(operation.at());
    Err(ApplyPlanError::new(
        ApplyPlanErrorKind::NotFound,
        format!(
            "apply operation `{}` writes content of `{}:{}` that is inherited from the parent: the extension does not hold its `{resource}`; read the parent named by props.extends of `{owner}`",
            operation.name(),
            operation.at().source_set(),
            target.as_str(),
        ),
    )
    .at_path(format!("ops[{op_index}].args.at"))
    .with_next(serde_json::json!({
        "tool": "unica.view",
        "args": {"at": owner},
        "reason": "props.extends names the parent that holds this content",
    })))
}

/// Разбор операции семейством: извлечение частей адреса и аргументов. О
/// применимости он не спрашивает — это сделал `refuse_misplaced_operation`.
fn parse_family_operation(
    op_index: usize,
    operation: &crate::domain::apply::ApplyOp,
    binding: &ProviderRootBinding,
) -> Result<ParsedApplyOperation, ApplyPlanError> {
    let Some(family) = resolved_descriptor(operation.name(), Some(operation.at()), binding)
        .map(|descriptor| descriptor.family())
    else {
        return Err(hidden_apply_family_unimplemented(op_index));
    };
    let args = serde_json::Value::Object(operation.args().clone());
    if operation.name() == "object.borrow" {
        return borrow::parse(&args, op_index, binding).map(ParsedApplyOperation::Borrow);
    }
    let parsed = match family {
        OperationFamily::Metadata | OperationFamily::Properties => {
            let parsed = metadata::parse_metadata_plan_operation(
                operation.name(),
                &args,
                op_index,
                binding,
            )?;
            ParsedApplyOperation::Metadata(IndexedPlanOperation::new(op_index, parsed))
        }
        // Командный интерфейс — присоединённый ресурс подсистемы,
        // как `Rights.xml` у роли: тот же планировщик, тот же способ
        // стадирования.
        OperationFamily::Form
        | OperationFamily::Role
        | OperationFamily::Subsystem
        | OperationFamily::Interface
        | OperationFamily::Support => {
            let parsed = form_resource::parse_form_resource_plan_operation(
                operation.name(),
                &args,
                op_index,
                binding,
            )?;
            ParsedApplyOperation::FormResource(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Dcs | OperationFamily::Mxl => {
            let parsed =
                dcs_mxl::parse_dcs_mxl_plan_operation(operation.name(), &args, op_index, binding)?;
            ParsedApplyOperation::DcsMxl(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Code => {
            let parsed = crate::infrastructure::native_operations::code::parse_code_plan_operation(
                operation.name(),
                &args,
                op_index,
                binding,
            )?;
            ParsedApplyOperation::Code(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Xdto => {
            let parsed = crate::infrastructure::native_operations::xdto::parse_xdto_plan_operation(
                operation.name(),
                &args,
                op_index,
                binding,
            )?;
            ParsedApplyOperation::Xdto(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Event => {
            let args = canonical_event_args(args);
            let parsed =
                crate::infrastructure::native_operations::event::parse_event_implement_args(
                    &args,
                    &format!("ops[{op_index}].args"),
                )
                .map_err(event_plan_error_to_apply)?;
            ParsedApplyOperation::Event(IndexedPlanOperation::new(op_index, parsed))
        }
    };
    Ok(parsed)
}

/// Разбор операции планировщиком без стадирования: проверка, что разбор цели
/// и аргументов семейства принимает вызов. Им двусторонняя проверка `can`
/// сверяет словарь узла с планировщиком.
/// Разбор семейством в обход реестра: двусторонняя проверка ищет адреса,
/// которые семейство приняло бы, а реестр — нет.
#[cfg(test)]
pub(crate) fn parse_family_operation_for_test(
    op_index: usize,
    operation: &crate::domain::apply::ApplyOp,
    binding: &ProviderRootBinding,
) -> Result<(), ApplyPlanError> {
    parse_family_operation(op_index, operation, binding).map(|_| ())
}

#[cfg(test)]
pub(crate) fn parse_apply_operation_for_test(
    op_index: usize,
    operation: &crate::domain::apply::ApplyOp,
    binding: &ProviderRootBinding,
) -> Result<(), ApplyPlanError> {
    parse_apply_operation(op_index, operation, binding).map(|_| ())
}

/// Описание узла с собственным файлом: объект `Kind.Name` или его форма или
/// макет `Kind.Name.Form.F` / `Kind.Name.Template.T`, и число сегментов
/// адреса, которые это описание покрывает.
fn descriptor_node(
    at: &crate::domain::address::QualifiedAddress,
) -> Option<(crate::domain::source_target::MetadataAddress, usize)> {
    use crate::domain::address::NodeKind;
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    let segments = at.segments();
    let owner = segments.first()?;
    let owner_path = format!("{}.{}", owner.kind().as_str(), owner.name()?);
    match segments.get(1) {
        Some(nested)
            if matches!(nested.kind(), NodeKind::Form | NodeKind::Template)
                && nested.name().is_some() =>
        {
            let path = format!("{owner_path}.{}.{}", nested.kind().as_str(), nested.name()?);
            Some((
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &path).ok()?,
                2,
            ))
        }
        _ => Some((
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &owner_path).ok()?,
            1,
        )),
    }
}

/// Формат макета по его описанию в стадированном состоянии допуска.
pub(crate) fn staged_template_format(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    template: &crate::domain::address::QualifiedAddress,
) -> Option<crate::domain::apply::TemplateFormat> {
    let (address, covered) = descriptor_node(template)?;
    if covered != template.segments().len() {
        return None;
    }
    let relative = crate::infrastructure::logical_event_source::metadata_descriptor_relative(
        &address,
        binding.source_kind(),
    )
    .ok()?;
    let bytes = admission.staged_state().ok()?.read(&relative).ok()??;
    dcs_mxl::template_type_of(&bytes).map(|raw| crate::domain::apply::TemplateFormat::parse(&raw))
}

/// Поиск узла схемы по имени в стадированном макете: владелец — адрес
/// макета (набор данных, в том числе член объединения, #1353) или его
/// варианта настроек (именованный элемент структуры, #1357). Схемы нет —
/// `Missing`.
fn staged_schema_lookup(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    owner: &crate::domain::address::QualifiedAddress,
    name: &str,
) -> crate::domain::apply::NamedLookup {
    use crate::domain::address::NodeKind;
    use crate::domain::apply::NamedLookup;
    let segments = owner.segments();
    let (template, variant) = match segments.last() {
        Some(last) if last.kind() == NodeKind::Template => (owner.clone(), None),
        Some(last) if last.kind() == NodeKind::Setting && segments.len() > 1 => (
            crate::domain::address::QualifiedAddress {
                source_set: owner.source_set().to_string(),
                segments: segments[..segments.len() - 1].to_vec(),
            },
            last.name(),
        ),
        _ => return NamedLookup::Missing,
    };
    let Some((address, covered)) = descriptor_node(&template) else {
        return NamedLookup::Missing;
    };
    if covered != template.segments().len() {
        return NamedLookup::Missing;
    }
    let Some(bytes) = crate::infrastructure::logical_event_source::attached_resource_relative(
        &address,
        "Template.xml",
        binding.source_kind(),
    )
    .ok()
    .and_then(|relative| admission.staged_state().ok()?.read(&relative).ok()?) else {
        return NamedLookup::Missing;
    };
    match variant {
        Some(variant) => dcs_mxl::structure_item_path_in_schema(&bytes, variant, name),
        None => dcs_mxl::data_set_path_in_schema(&bytes, name),
    }
}

/// Путь узла с именем `name` под контейнером в стадированном состоянии:
/// набора схемы под макетом, элемента структуры под вариантом настроек или
/// элемента под формой. Им снятый аргумент, называвший узел по имени, ведёт
/// к его настоящему адресу (#1353, #1355, #1357).
pub(crate) fn staged_member_path(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    container: &crate::domain::address::QualifiedAddress,
    name: &str,
) -> crate::domain::apply::NamedLookup {
    match container.segments().last().map(|segment| segment.kind()) {
        Some(crate::domain::address::NodeKind::Form) => {
            match staged_form_items(admission, binding, container) {
                Some(items) => form_resource::form_item_path(&items, name),
                None => crate::domain::apply::NamedLookup::Missing,
            }
        }
        _ => staged_schema_lookup(admission, binding, container, name),
    }
}

/// Роль элемента стадированной формы по его адресу
/// `Owner.X.Form.F.Item.A[.Item.B…]`. `None` — элемента или формы нет.
pub(crate) fn staged_form_item_role(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    item: &crate::domain::address::QualifiedAddress,
) -> Option<crate::domain::apply::FormItemRole> {
    use crate::domain::address::NodeKind;
    let segments = item.segments();
    if segments.len() < 3
        || segments[1].kind() != NodeKind::Form
        || !segments[2..]
            .iter()
            .all(|segment| segment.kind() == NodeKind::Item && segment.name().is_some())
    {
        return None;
    }
    let form = crate::domain::address::QualifiedAddress {
        source_set: item.source_set().to_string(),
        segments: segments[..2].to_vec(),
    };
    let items = staged_form_items(admission, binding, &form)?;
    let chain = segments[2..]
        .iter()
        .filter_map(|segment| segment.name().map(str::to_string))
        .collect::<Vec<_>>();
    form_resource::form_item_at(&items, &chain)
        .map(|element| crate::domain::apply::FormItemRole::from_tag(&element.tag))
}

/// Дерево элементов стадированной формы `Owner.X.Form.F`, как его читает
/// `view`. `None` — формы или её `Form.xml` нет.
fn staged_form_items(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    form: &crate::domain::address::QualifiedAddress,
) -> Option<Vec<crate::infrastructure::native_operations::form::FormInfoElement>> {
    let (address, covered) = descriptor_node(form)?;
    if covered != 2 {
        return None;
    }
    let relative = crate::infrastructure::logical_event_source::attached_resource_relative(
        &address,
        "Form.xml",
        binding.source_kind(),
    )
    .ok()?;
    let bytes = admission.staged_state().ok()?.read(&relative).ok()??;
    form_resource::form_items_of(std::str::from_utf8(&bytes).ok()?).ok()
}

/// Есть ли узел `at` в стадированном состоянии допуска. Узел с собственным
/// описанием есть, когда есть его файл; член объекта метаданных (реквизит,
/// табличная часть) при существующем описании владельца считается
/// отсутствующим — отказ «не найден» по адресу операции говорит о нём.
/// `None` — по описаниям этого не установить.
pub(crate) fn staged_node_existence(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    at: &crate::domain::address::QualifiedAddress,
) -> Option<bool> {
    let (address, covered) = descriptor_node(at)?;
    let relative = crate::infrastructure::logical_event_source::metadata_descriptor_relative(
        &address,
        binding.source_kind(),
    )
    .ok()?;
    if admission
        .staged_state()
        .ok()?
        .read(&relative)
        .ok()?
        .is_none()
    {
        return Some(false);
    }
    if covered == at.segments().len() {
        // Описание есть: «не найден» касается присоединённого ресурса
        // (`Rights.xml`, `Form.xml`), а не узла.
        return Some(true);
    }
    let route = crate::domain::logical_route::classify_route(
        at,
        binding
            .source_profile()
            .platform_profile()
            .unwrap_or_else(crate::domain::platform_profile::PlatformProfile::v8_3_27),
    )?;
    // Запись прав роли и область макета живут в присоединённом файле: их
    // создаёт операция родителя (`right.add`, `area.add`, #1354).
    use crate::domain::address::NodeKind;
    use crate::domain::logical_route::RouteClass;
    let terminal = at.segments().last()?;
    let resource = |name: &str| -> Option<Vec<u8>> {
        let relative = crate::infrastructure::logical_event_source::attached_resource_relative(
            &address,
            name,
            binding.source_kind(),
        )
        .ok()?;
        admission.staged_state().ok()?.read(&relative).ok()?
    };
    match (route, terminal.kind(), at.segments().len() - covered) {
        (RouteClass::Metadata, _, _) => Some(false),
        (RouteClass::Role, NodeKind::Right, 1) => {
            let (kind, name) = terminal.name()?.split_once('_')?;
            form_resource::staged_rights_entry_exists(
                &resource("Rights.xml")?,
                &format!("{kind}.{name}"),
            )
        }
        (RouteClass::Mxl, NodeKind::Area, 1) => {
            dcs_mxl::spreadsheet_area_exists(&resource("Template.xml")?, terminal.name()?)
        }
        // Элемент формы и его событие: есть, когда цепочка `Item` ведёт к
        // элементу формы; имя события проверяет матрица событий.
        (RouteClass::Form, NodeKind::Item | NodeKind::Event, _) if covered == 2 => {
            let tail = &at.segments()[covered..];
            let chain = match tail {
                [items @ .., last] if last.kind() == NodeKind::Event => items,
                items => items,
            };
            if chain.is_empty()
                || !chain
                    .iter()
                    .all(|segment| segment.kind() == NodeKind::Item && segment.name().is_some())
            {
                return None;
            }
            let form = crate::domain::address::QualifiedAddress {
                source_set: at.source_set().to_string(),
                segments: at.segments()[..covered].to_vec(),
            };
            let items = staged_form_items(admission, binding, &form)?;
            let chain = chain
                .iter()
                .filter_map(|segment| segment.name().map(str::to_string))
                .collect::<Vec<_>>();
            Some(form_resource::form_item_at(&items, &chain).is_some())
        }
        _ => None,
    }
}

pub(crate) fn plan_hidden_v13_apply(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    let parsed = request
        .ops()
        .iter()
        .enumerate()
        .map(|(op_index, operation)| parse_apply_operation(op_index, operation, binding))
        .collect::<Result<Vec<_>, ApplyPlanError>>()?;

    let mut staged = admission
        .staged_state()
        .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
    refuse_targets_outside_writable_profile(request, binding, &mut staged)?;
    refuse_targets_on_locked_vendor_objects(request, binding, admission, &mut staged)?;
    let mut provisional = Vec::new();
    let mut borrowing = Vec::new();
    let mut cursor = 0;
    while cursor < parsed.len() {
        if let ParsedApplyOperation::Unsupported(index) = &parsed[cursor] {
            return Err(hidden_apply_family_unimplemented(*index));
        }
        let end = parsed[cursor..]
            .iter()
            .take_while(|operation| operation.same_family(&parsed[cursor]))
            .count()
            + cursor;
        match &parsed[cursor] {
            ParsedApplyOperation::Borrow(_) => {
                for operation in &parsed[cursor..end] {
                    let ParsedApplyOperation::Borrow(operation) = operation else {
                        unreachable!()
                    };
                    let (effects, detail) =
                        borrow::plan(&mut staged, admission, binding, operation)?;
                    provisional.extend(effects);
                    borrowing.push(detail);
                }
            }
            ParsedApplyOperation::Code(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Code(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is code-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.code_planning_authority(binding)?;
                let planned = crate::infrastructure::native_operations::code::plan_code_batch(
                    staged,
                    authority,
                    &operations,
                )?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Event(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Event(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is event-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.metadata_planning_authority(binding)?;
                let planned =
                    crate::infrastructure::native_operations::event::plan_event_implement_batch(
                        staged,
                        authority.source_set_name(),
                        authority.source_kind(),
                        authority.profile(),
                        &operations,
                    )
                    .map_err(event_plan_error_to_apply)?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Xdto(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Xdto(operation) => operation.operation().clone(),
                        _ => unreachable!("the selected run is XDTO-only"),
                    })
                    .collect::<Vec<_>>();
                let before = staged
                    .planned_changes()
                    .into_iter()
                    .map(|change| change.relative_path)
                    .collect::<Vec<_>>();
                let authority = admission.xdto_planning_authority(binding)?;
                let planned = crate::infrastructure::native_operations::xdto::plan_xdto_batch(
                    staged,
                    authority,
                    &operations,
                )?;
                staged = planned.0;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned.1, cursor,
                ));
            }
            ParsedApplyOperation::Metadata(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Metadata(operation) => operation.clone(),
                        _ => unreachable!("the selected run is metadata-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.metadata_planning_authority(binding)?;
                let planned = metadata::plan_metadata_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::FormResource(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::FormResource(operation) => operation.clone(),
                        _ => unreachable!("the selected run is form/resource-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.form_resource_planning_authority(binding)?;
                let planned =
                    form_resource::plan_form_resource_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::DcsMxl(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::DcsMxl(operation) => operation.clone(),
                        _ => unreachable!("the selected run is DCS/MXL-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.dcs_mxl_planning_authority(binding)?;
                let planned = dcs_mxl::plan_dcs_mxl_batch(staged, authority, &operations)?;
                staged = planned.0;
                provisional.extend(planned.1);
            }
            ParsedApplyOperation::Unsupported(_) => unreachable!("unsupported run handled above"),
        }
        cursor = end;
    }

    let mut effects = reconcile_effects(&staged, provisional);
    effects.set_borrowing(borrowing);
    Ok((staged, effects))
}

#[cfg(test)]
mod inherited_content_tests;

#[cfg(test)]
mod tests {
    use super::plan_hidden_v13_apply;
    use super::request::{reconcile_effects, ProvisionalApplyEffect};
    use crate::domain::apply::ApplyRequest;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::apply::ApplyPlanErrorKind;
    use crate::infrastructure::workspace_actor::{
        ApplyAdmission, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
        WorkspaceSourceSetInput,
    };
    use std::sync::Arc;
    use std::time::Duration;

    pub(super) struct ApplySeamFixture {
        _root: tempfile::TempDir,
        actor: Arc<WorkspaceActor>,
        pub(super) binding: ProviderRootBinding,
    }

    impl ApplySeamFixture {
        /// The physical `src` root, for tests that lay out extra resources.
        pub(super) fn source_dir(&self) -> std::path::PathBuf {
            std::fs::canonicalize(self._root.path().join("src")).unwrap()
        }

        pub(super) fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("src");
            std::fs::create_dir_all(source.join("Documents")).unwrap();
            std::fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>First</Document><Document>Second</Document></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            std::fs::create_dir_all(source.join("Languages")).unwrap();
            std::fs::write(
                source.join("Languages/Русский.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
            )
            .unwrap();
            for (name, comment) in [("First", ""), ("Second", "")] {
                std::fs::write(
                    source.join(format!("Documents/{name}.xml")),
                    format!(
                        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>{name}</Name><Synonym/><Comment>{comment}</Comment></Properties><ChildObjects><Attribute uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Total</Name><Comment>original</Comment></Properties><ChildObjects/></Attribute></ChildObjects></Document></MetaDataObject>"#
                    ),
                )
                .unwrap();
            }
            let workspace_root = std::fs::canonicalize(root.path()).unwrap();
            let source = std::fs::canonicalize(source).unwrap();
            let context = WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    &source,
                    SourceSetKind::Configuration,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "apply-family-seam-test",
            )
            .unwrap();
            let actor = Arc::new(WorkspaceActor::new(identity, context).unwrap());
            let binding = actor.bind_provider_root("main", &source).unwrap();
            Self {
                _root: root,
                actor,
                binding,
            }
        }

        /// Switches the fixture's only language from Russian to English.
        pub(super) fn use_english_only_language(&self) {
            let configuration = self.source_dir().join("Configuration.xml");
            let text = std::fs::read_to_string(&configuration)
                .unwrap()
                .replace("Language.Русский", "Language.English")
                .replace(
                    "<Language>Русский</Language>",
                    "<Language>English</Language>",
                );
            std::fs::write(&configuration, text).unwrap();
            std::fs::remove_file(self.source_dir().join("Languages/Русский.xml")).unwrap();
            std::fs::write(
                self.source_dir().join("Languages/English.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>English</Name><Comment/><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#,
            )
            .unwrap();
        }

        pub(super) fn admission(&self) -> ApplyAdmission {
            self.admission_with_dry_run(true)
        }

        pub(super) fn actor(&self) -> &WorkspaceActor {
            &self.actor
        }

        pub(super) fn admission_with_dry_run(&self, dry_run: bool) -> ApplyAdmission {
            self.actor
                .admit_apply(
                    &self.binding,
                    None,
                    dry_run,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }
    }

    fn seam_request(at: &str, op: &str, args: serde_json::Value) -> ApplyRequest {
        let input = serde_json::json!({
            "at": at,
            "ops": [{"op": op, "args": args}],
            "dryRun": true,
        });
        ApplyRequest::parse(input.as_object().unwrap(), &["main"]).unwrap()
    }

    /// Формат макета — фильтр реестра, и планировщик применяет его к
    /// промежуточному состоянию пакета: макет, добавленный раньше в том же
    /// вызове, уже имеет формат, и `area.add` принимается только для
    /// табличного документа.
    #[test]
    fn template_format_filter_reads_the_template_added_in_the_same_request() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let request = |template_type: &str| {
            ApplyRequest::parse(
                serde_json::json!({
                    "at": "main:Document.First",
                    "ops": [
                        {"op": "template.add", "args": {"items": [{"name": "Print", "templateType": template_type}]}},
                        {"op": "area.add", "args": {"at": "main:Document.First.Template.Print", "items": [{"name": "Header", "cells": {"R1C1": "Total"}}]}}
                    ],
                    "dryRun": true,
                })
                .as_object()
                .unwrap(),
                &["main"],
            )
            .unwrap()
        };
        plan_hidden_v13_apply(
            &request("SpreadsheetDocument"),
            &fixture.binding,
            &admission,
        )
        .unwrap_or_else(|error| panic!("a spreadsheet added earlier accepts cells: {error}"));
        let Err(refused) = plan_hidden_v13_apply(
            &request("DataCompositionSchema"),
            &fixture.binding,
            &admission,
        ) else {
            panic!("cells never go into a data composition schema");
        };
        assert_eq!(refused.kind(), ApplyPlanErrorKind::BadValue, "{refused}");
        assert_eq!(refused.path(), Some("ops[1].args.at"));
        assert!(
            refused.to_string().contains("DataCompositionSchema"),
            "{refused}"
        );
    }

    /// Область создаёт только `area.add` на макете, а `mxl.set` пишет только
    /// в существующую область по её адресу (#1354): опечатка в имени области
    /// не заводит новую, повтор имени в `area.add` не перекрывает ячейки.
    #[test]
    fn an_area_is_written_only_where_it_exists_and_created_only_by_area_add() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let template = "main:Document.First.Template.Print";
        let request = |ops: serde_json::Value| {
            let mut all = vec![
                serde_json::json!({"op": "template.add", "args": {"items": [{"name": "Print", "templateType": "SpreadsheetDocument"}]}}),
            ];
            all.extend(ops.as_array().unwrap().iter().cloned());
            ApplyRequest::parse(
                serde_json::json!({"at": "main:Document.First", "ops": all, "dryRun": true})
                    .as_object()
                    .unwrap(),
                &["main"],
            )
            .unwrap()
        };
        let area = |name: &str| serde_json::json!({"op": "area.add", "args": {"at": template, "items": [{"name": name, "cells": {"R1C1": "Total"}}]}});
        let set = |name: &str| serde_json::json!({"op": "mxl.set", "args": {"at": format!("{template}.Area.{name}"), "values": {"cells": {"R1C2": "Sum"}}}});
        let (staged, _) = plan_hidden_v13_apply(
            &request(serde_json::json!([area("Header"), set("Header")])),
            &fixture.binding,
            &admission,
        )
        .unwrap_or_else(|error| panic!("an added area accepts cells: {error}"));
        let written = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path.ends_with("Print/Ext/Template.xml"))
            .expect("the spreadsheet body is staged");
        let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) =
            written.current
        else {
            panic!("the spreadsheet keeps bytes");
        };
        assert_eq!(
            super::dcs_mxl::spreadsheet_area_exists(&bytes, "Header"),
            Some(true)
        );
        assert_eq!(
            super::dcs_mxl::spreadsheet_area_exists(&bytes, "Headr"),
            Some(false)
        );
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("Total") && text.contains("Sum"),
            "both cells are written"
        );
        // Точка в имени области экранируется в адресе: созданную область
        // `mxl.set` находит по `Area.Head\.er`.
        let (staged, _) = plan_hidden_v13_apply(
            &request(serde_json::json!([area("Head.er"), set("Head\\.er")])),
            &fixture.binding,
            &admission,
        )
        .unwrap_or_else(|error| panic!("an area with a dot is addressable: {error}"));
        let written = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path.ends_with("Print/Ext/Template.xml"))
            .expect("the spreadsheet body is staged");
        let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) =
            written.current
        else {
            panic!("the spreadsheet keeps bytes");
        };
        assert_eq!(
            super::dcs_mxl::spreadsheet_area_exists(&bytes, "Head.er"),
            Some(true)
        );
        assert!(
            String::from_utf8_lossy(&bytes).contains("Sum"),
            "the cell is written"
        );
        for (ops, kind, path) in [
            (
                serde_json::json!([area("Header"), set("Headr")]),
                ApplyPlanErrorKind::NotFound,
                "ops[2].args.at",
            ),
            (
                serde_json::json!([area("Header"), area("Header")]),
                ApplyPlanErrorKind::BadValue,
                "ops[2].args.items[0].name",
            ),
            (
                serde_json::json!([area("1Header")]),
                ApplyPlanErrorKind::BadValue,
                "ops[1].args.items[0].name",
            ),
        ] {
            let Err(error) = plan_hidden_v13_apply(&request(ops), &fixture.binding, &admission)
            else {
                panic!("the area's existence decides at {path}");
            };
            assert_eq!(error.kind(), kind, "{path}");
            assert_eq!(error.path(), Some(path));
        }
    }

    /// Запись прав и область макета живут в присоединённом файле: их
    /// существование читается из него, и `not_found` по адресу такого узла
    /// ведёт к родителю, где их создаёт `right.add` или `area.add` (#1354).
    #[test]
    fn a_missing_rights_entry_or_area_routes_next_to_the_parent() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        std::fs::create_dir_all(source.join("Roles/Reader/Ext")).unwrap();
        std::fs::write(
            source.join("Roles/Reader.xml"),
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Role uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(
            source.join("Roles/Reader/Ext/Rights.xml"),
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" version=\"2.20\"><setForNewObjects>false</setForNewObjects><setForAttributesByDefault>true</setForAttributesByDefault><independentRightsOfChildObjects>false</independentRightsOfChildObjects><object><name>Document.First</name><right><name>Read</name><value>true</value></right></object></Rights>\n",
        )
        .unwrap();
        let templates = source.join("Documents/First/Templates");
        std::fs::create_dir_all(templates.join("Print/Ext")).unwrap();
        let mxl = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates",
        );
        std::fs::copy(mxl.join("F06Template.xml"), templates.join("Print.xml")).unwrap();
        std::fs::copy(
            mxl.join("F06Template/Ext/Template.xml"),
            templates.join("Print/Ext/Template.xml"),
        )
        .unwrap();
        let admission = fixture.admission();
        let exists = |raw: &str| {
            super::staged_node_existence(
                &admission,
                &fixture.binding,
                &crate::domain::address::QualifiedAddress::parse(raw).unwrap(),
            )
        };
        for (raw, expected) in [
            ("main:Role.Reader.Right.Document_First", Some(true)),
            ("main:Role.Reader.Right.Document_Second", Some(false)),
            ("main:Document.First.Template.Print.Area.A", Some(true)),
            ("main:Document.First.Template.Print.Area.Z", Some(false)),
            ("main:Role.Missing.Right.Document_First", Some(false)),
        ] {
            assert_eq!(exists(raw), expected, "{raw}");
        }
    }

    /// Операция, исполнимая не на своём узле, отказывает в разборе цели,
    /// а не пишет молча в форму или во владельца.
    #[test]
    fn misplaced_form_operations_refuse_instead_of_writing_elsewhere() {
        let fixture = ApplySeamFixture::new();
        for (at, op, args) in [
            (
                "main:Document.First.Form.Main.Item.Header.Event.OnChange",
                "item.add",
                serde_json::json!({"items": [{"name": "Comment"}]}),
            ),
            (
                "main:Document.First.Form.Main",
                "item.remove",
                serde_json::json!({}),
            ),
            (
                "main:Document.First.Form.Main.Item.Header",
                "event.bind",
                serde_json::json!({"values": {"handler": "H"}}),
            ),
            (
                "main:Document.First.Form.Main.Item.Header",
                "attribute.add",
                serde_json::json!({"items": [{"name": "Comment"}]}),
            ),
            (
                "main:Document.First.Form.Main.Item.Header",
                "form.remove",
                serde_json::json!({}),
            ),
            (
                "main:Document.First.Form.Main",
                "form.set",
                serde_json::json!({"values": {"defaultObjectForm": "Main"}}),
            ),
        ] {
            let request = seam_request(at, op, args);
            let error = super::parse_apply_operation(0, &request.ops()[0], &fixture.binding)
                .err()
                .unwrap_or_else(|| panic!("{op} at {at} must refuse"));
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op}: {error}");
            assert_eq!(error.path(), Some("ops[0].args.at"), "{op}: {error}");
        }
        // Неизвестное свойство элемента — отказ, а не молча потерянный ключ.
        let request = seam_request(
            "main:Document.First.Form.Main",
            "item.add",
            serde_json::json!({"items": [{"name": "Comment", "titel": "typo"}]}),
        );
        let error = super::parse_apply_operation(0, &request.ops()[0], &fixture.binding)
            .err()
            .expect("an unknown element property refuses");
        assert_eq!(error.path(), Some("ops[0].args.items[0].titel"), "{error}");
        // Путь привязки, которого писатель вида не пишет (`headerDataPath`
        // у обычной группы), отказывает по контракту вида до записи (#1447).
        let request = seam_request(
            "main:Document.First.Form.Main",
            "item.add",
            serde_json::json!({"items": [{"name": "Header", "type": "Group", "headerDataPath": "Object.Ref"}]}),
        );
        let error = super::parse_apply_operation(0, &request.ops()[0], &fixture.binding)
            .err()
            .expect("a binding path the group writer does not write refuses");
        assert_eq!(
            error.path(),
            Some("ops[0].args.items[0].headerDataPath"),
            "{error}"
        );
        // Элемент и событие названы адресом: перенаправить событие или
        // удаление на другой узел аргументом нельзя (#1355).
        for (at, op, args, path) in [
            (
                "main:Document.First.Form.Main.Item.Header.Event.OnChange",
                "event.bind",
                serde_json::json!({"values": {"handler": "H", "element": "Other"}}),
                "ops[0].args.values.element",
            ),
            (
                "main:Document.First.Form.Main.Item.Header",
                "item.remove",
                serde_json::json!({"items": ["Other"]}),
                "ops[0].args.items",
            ),
        ] {
            let request = seam_request(at, op, args);
            let error = super::parse_apply_operation(0, &request.ops()[0], &fixture.binding)
                .err()
                .unwrap_or_else(|| panic!("{op} must not redirect to another element"));
            assert_eq!(error.path(), Some(path), "{op}: {error}");
        }
    }

    /// The vendor-support gate of the canonical path: a locked vendor object
    /// refuses every family before a byte is staged, the support operations
    /// stay the way out of the lock, and a source without the marker is not
    /// on support at all.
    #[test]
    fn locked_vendor_objects_refuse_every_family_before_staging() {
        use crate::domain::apply::{OperationFamily, OperationRegistry};

        let fixture = ApplySeamFixture::new();
        let ext = fixture.source_dir().join("Ext");
        std::fs::create_dir_all(&ext).unwrap();
        for (directory, kind, name) in [
            ("Roles", "Role", "Reader"),
            ("Subsystems", "Subsystem", "Main"),
            ("XDTOPackages", "XDTOPackage", "Types"),
        ] {
            let directory = fixture.source_dir().join(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join(format!("{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
        // Both fixture documents carry uuid 1111…; the marker locks it.
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            "\u{feff}{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,11111111-1111-4111-8111-111111111111,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,44444444-4444-4444-8444-444444444444,44444444-4444-4444-8444-444444444444,0,0,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111}",
        )
        .unwrap();
        let snapshot = || {
            let root = fixture.source_dir();
            let mut pending = vec![root.clone()];
            let mut entries = std::collections::BTreeMap::new();
            while let Some(directory) = pending.pop() {
                for entry in std::fs::read_dir(directory).unwrap() {
                    let path = entry.unwrap().path();
                    let bytes = if path.is_dir() {
                        pending.push(path.clone());
                        None
                    } else {
                        Some(std::fs::read(&path).unwrap())
                    };
                    entries.insert(path.strip_prefix(&root).unwrap().to_path_buf(), bytes);
                }
            }
            entries
        };
        let cases = [
            (
                "main:Document.First",
                "props.set",
                serde_json::json!({"values": {"Comment": "edited"}}),
            ),
            (
                "main:Document.First",
                "attribute.add",
                serde_json::json!({"items": [{"name": "Extra", "type": {"variants": [{"kind": "string", "length": 10, "allowedLength": "variable"}]}}]}),
            ),
            (
                "main:Document.First",
                "form.add",
                serde_json::json!({"items": [{"name": "Card", "type": "ObjectForm"}]}),
            ),
            (
                "main:Document.First",
                "template.add",
                serde_json::json!({"items": [{"name": "Print", "templateType": "SpreadsheetDocument"}]}),
            ),
            (
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            (
                "main:Role.Reader",
                "right.add",
                serde_json::json!({"items": [{"object": "Document.First", "rights": {"Read": true}}]}),
            ),
            (
                "main:Document.First.Template.Schema.DataSet.Main",
                "field.add",
                serde_json::json!({"items": [{"dataPath": "Total"}]}),
            ),
            (
                "main:Document.First.Template.Print",
                "area.add",
                serde_json::json!({"items": [{"name": "Header", "cells": {"R1C1": "Total"}}]}),
            ),
            (
                "main:XDTOPackage.Types",
                "objectType.add",
                serde_json::json!({"values": {"name": "Entry"}}),
            ),
            (
                "main:Subsystem.Main",
                "content.add",
                serde_json::json!({"items": [{"object": "Document.First"}]}),
            ),
            (
                "main:Subsystem.Main.Interface",
                "groupOrder.set",
                serde_json::json!({"values": {"groups": ["NavigationPanelImportant"]}}),
            ),
            (
                "main:Document.First.Module.Object",
                "code.insert",
                serde_json::json!({"text": "Procedure Added()\nEndProcedure\n"}),
            ),
            (
                "main:Document.First.Module.Object.Event.BeforeWrite",
                "event.implement",
                serde_json::json!({}),
            ),
        ];
        let covered = cases
            .iter()
            .map(|(_, operation, _)| {
                crate::application::v13::apply::dispatch_family(operation).unwrap()
            })
            .collect::<Vec<_>>();
        for descriptor in OperationRegistry::closed().descriptors() {
            assert!(
                descriptor.family() == OperationFamily::Support
                    || covered.contains(&descriptor.family()),
                "support refusal needs a real case for {:?}",
                descriptor.family()
            );
        }
        let admission = fixture.admission();
        let before = snapshot();
        for (at, op, args) in cases {
            let request = seam_request(at, op, args);
            let error = plan_hidden_v13_apply(&request, &fixture.binding, &admission)
                .err()
                .unwrap_or_else(|| panic!("{op} on a locked vendor object must refuse"));
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{op}");
            assert!(
                error.to_string().contains("support rule `locked`"),
                "{op}: {error}"
            );
            assert_eq!(snapshot(), before, "{op} must leave the source tree intact");
        }
        // The support operations are the way out of the lock.
        for (operation, values) in [
            ("supportRule.set", serde_json::json!({"rule": "editable"})),
            (
                "supportCapability.set",
                serde_json::json!({"enabled": true}),
            ),
        ] {
            // Возможность изменения — свойство всей конфигурации, правило —
            // объекта.
            let target = if operation == "supportCapability.set" {
                "main:Configuration"
            } else {
                "main:Document.First"
            };
            let unlock = seam_request(target, operation, serde_json::json!({"values": values}));
            plan_hidden_v13_apply(&unlock, &fixture.binding, &admission)
                .unwrap_or_else(|error| panic!("{operation} must plan on a locked owner: {error}"));
        }
        let mixed = ApplyRequest::parse(
            serde_json::json!({
                "at": "main:Document.First",
                "ops": [
                    {"op": "supportRule.set", "args": {"values": {"rule": "editable"}}},
                    {"op": "props.set", "args": {"values": {"Comment": "edited"}}}
                ],
                "dryRun": true,
            })
            .as_object()
            .unwrap(),
            &["main"],
        )
        .unwrap();
        let error = plan_hidden_v13_apply(&mixed, &fixture.binding, &admission).unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert!(
            error.to_string().contains("support rule `locked`"),
            "{error}"
        );
        assert_eq!(snapshot(), before);

        // A configuration on support with changes disabled refuses even an
        // unlisted object.
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            "\u{feff}{6,1,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,11111111-1111-4111-8111-111111111111,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,44444444-4444-4444-8444-444444444444,44444444-4444-4444-8444-444444444444}",
        )
        .unwrap();
        let admission = fixture.admission();
        let request = seam_request(
            "main:Document.Second",
            "props.set",
            serde_json::json!({"values": {"Comment": "edited"}}),
        );
        let error = plan_hidden_v13_apply(&request, &fixture.binding, &admission).unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert!(error.to_string().contains("changes disabled"), "{error}");

        // Without the marker the source is not on support and edits plan.
        std::fs::remove_file(ext.join("ParentConfigurations.bin")).unwrap();
        let admission = fixture.admission();
        plan_hidden_v13_apply(&request, &fixture.binding, &admission)
            .expect("a source without the support marker is editable");
    }

    /// `object.remove` refuses an object other files still refer to; with
    /// `force` the plan executes and carries a typed warning naming them.
    #[test]
    fn forced_object_removal_keeps_the_referrers_and_says_so() {
        let fixture = ApplySeamFixture::new();
        let rights_dir = fixture.source_dir().join("Roles/Reader/Ext");
        std::fs::create_dir_all(&rights_dir).unwrap();
        std::fs::write(
            rights_dir.join("Rights.xml"),
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"><object><name>Document.First</name></object></Rights>"#,
        )
        .unwrap();
        let admission = fixture.admission();
        let refused = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &admission,
        )
        .unwrap_err();
        assert_eq!(refused.kind(), ApplyPlanErrorKind::InvalidState);
        assert!(
            refused.to_string().contains("still referenced"),
            "{refused}"
        );

        let admission = fixture.admission();
        let (staged, effects) = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({"force": true}),
            ),
            &fixture.binding,
            &admission,
        )
        .expect("a forced removal plans");
        assert!(!staged.planned_changes().is_empty());
        assert_eq!(effects.warnings().len(), 1);
        let warning = &effects.warnings()[0];
        assert_eq!(warning["code"], "references_kept");
        assert_eq!(warning["count"], 1);
        assert!(warning["files"][0]
            .as_str()
            .unwrap()
            .ends_with("Roles/Reader/Ext/Rights.xml"));

        let admission = fixture.admission();
        let error = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({"force": "yes"}),
            ),
            &fixture.binding,
            &admission,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    }

    #[test]
    fn batch_effects_keep_each_event_with_its_own_file() {
        use crate::domain::events::{DomainEvent, DomainEventKind};
        use crate::infrastructure::native_operations::apply::PlannedApplyEffects;

        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let first = std::path::Path::new("Documents/First.xml");
        let second = std::path::Path::new("Documents/Second.xml");
        let first_preimage = staged.read(first).unwrap().unwrap();
        let second_preimage = staged.read(second).unwrap().unwrap();
        let edited = |preimage: &[u8]| {
            String::from_utf8(preimage.to_vec())
                .unwrap()
                .replace("<Comment></Comment>", "<Comment>batch</Comment>")
                .into_bytes()
        };
        staged
            .replace(first, &first_preimage, edited(&first_preimage))
            .unwrap();
        staged
            .replace(second, &second_preimage, edited(&second_preimage))
            .unwrap();

        // The planner names the file behind each event, as the code, XDTO
        // and event planners do.
        let mut effects = PlannedApplyEffects::default();
        effects.append_at(
            DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:Document.First.Module.Object",
            ),
            vec![first.to_path_buf()],
        );
        effects.append_at(
            DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:Document.Second.Module.Object",
            ),
            vec![second.to_path_buf()],
        );
        let provisional = super::batch_effects_as_provisional(&[], &staged, effects, 0);

        // A later operation restores the first file; only the second event
        // may survive request-level reconciliation.
        let first_current = staged.read(first).unwrap().unwrap();
        staged
            .replace(first, &first_current, first_preimage.clone())
            .unwrap();
        let reconciled = reconcile_effects(&staged, provisional);
        assert_eq!(
            reconciled
                .events()
                .iter()
                .map(|event| event.artifact.as_str())
                .collect::<Vec<_>>(),
            ["main:Document.Second.Module.Object"]
        );
    }

    #[test]
    fn request_level_reconciliation_drops_cancelled_effect_before_deduplication() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let first_path = std::path::Path::new("Documents/First.xml");
        let second_path = std::path::Path::new("Documents/Second.xml");
        let first_preimage = staged.read(first_path).unwrap().unwrap();
        let first_postimage = String::from_utf8(first_preimage.clone())
            .unwrap()
            .replace("<Comment></Comment>", "<Comment>cancelled</Comment>")
            .into_bytes();
        staged
            .replace(first_path, &first_preimage, first_postimage.clone())
            .unwrap();
        let second_preimage = staged.read(second_path).unwrap().unwrap();
        let second_postimage = String::from_utf8(second_preimage.clone())
            .unwrap()
            .replace("<Comment></Comment>", "<Comment>survives</Comment>")
            .into_bytes();
        staged
            .replace(second_path, &second_preimage, second_postimage)
            .unwrap();
        staged
            .replace(first_path, &first_postimage, first_preimage)
            .unwrap();

        assert_eq!(
            staged
                .planned_changes()
                .iter()
                .map(|change| change.relative_path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["Documents/Second.xml"]
        );
        let effects = reconcile_effects(
            &staged,
            vec![
                // The surviving duplicate arrives before its earlier sibling.
                ProvisionalApplyEffect::single(
                    second_path,
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    3,
                ),
                ProvisionalApplyEffect::single(
                    first_path,
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    0,
                ),
                ProvisionalApplyEffect::spanning(
                    vec![first_path.to_path_buf(), second_path.to_path_buf()],
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::MetadataChanged,
                        "partly-restored",
                    ),
                    4,
                ),
                ProvisionalApplyEffect::single(
                    second_path,
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    2,
                ),
                ProvisionalApplyEffect::single(
                    second_path,
                    crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::SourceSetChanged,
                        "earlier",
                    ),
                    1,
                ),
            ],
        );
        assert_eq!(
            effects
                .events()
                .iter()
                .map(|event| (event.kind, event.artifact.as_str()))
                .collect::<Vec<_>>(),
            [
                (
                    crate::domain::events::DomainEventKind::SourceSetChanged,
                    "earlier"
                ),
                (
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "shared"
                )
            ]
        );
    }
}
