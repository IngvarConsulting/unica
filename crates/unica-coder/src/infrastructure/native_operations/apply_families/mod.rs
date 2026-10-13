use crate::domain::apply::SourceSetWriteSupport;

mod borrow;
pub(crate) mod dcs_mxl;
pub(crate) mod form_resource;
#[cfg(test)]
pub(crate) mod metadata;

use crate::domain::apply::ApplyRequest;
use unica_operations::OperationDescriptor;
use unica_operations::OperationFamily;
use unica_source_contract::proposal::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind, PlannedApplyEffects,
};
use unica_source_contract::proposal::{
    reconcile_effects, IndexedPlanOperation, ProvisionalApplyEffect,
};
use unica_source_contract::SourceSetKind;
use unica_workspace::actor::{ApplyAdmission, ProviderRootBinding};
use unica_workspace::apply::ApplyStagedState;

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
    let mut owner_chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for target in
        std::iter::once(request.at()).chain(request.ops().iter().map(|operation| operation.at()))
    {
        for relative in writable_profile_owner_chain(target, binding.source_kind()) {
            if seen.insert(relative.clone()) {
                owner_chain.push(relative);
            }
        }
    }
    for relative in owner_chain {
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
fn changes_support_state(
    operation: &str,
    at: Option<&unica_address::address::QualifiedAddress>,
    binding: &ProviderRootBinding,
) -> bool {
    resolved_descriptor(operation, at, binding)
        .and_then(OperationDescriptor::operation::<unica_metadata::support::SupportOperation>)
        .is_some_and(|operation| operation.changes_support_state())
}

/// Vendor-support gate of the canonical mutation path. When the actor's
/// support policy is `deny`, a request whose owner is a vendor object with
/// the rule `locked`, or whose configuration is on support with changes
/// disabled, refuses `invalid_state` before any family stages a byte.
/// Removing a vendor object also requires its own rule to be `off-support`:
/// permission to edit it does not permit its removal.
/// The owner is the top-level metadata object of the addressed node, or the
/// configuration root for root-level targets; nested forms, templates and
/// attributes inherit the rule of their owner, as they do in the platform.
fn refuse_targets_on_locked_vendor_objects(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    staged: &mut ApplyStagedState,
) -> Result<(), ApplyPlanError> {
    use unica_metadata_designer::support::{
        parse_support_state_compat_bytes, support_root_uuid_from_bytes,
    };
    use unica_source_contract::proposal::SupportPolicyMode;

    if admission.support_policy_mode() != SupportPolicyMode::Deny {
        return Ok(());
    }
    if request
        .ops()
        .iter()
        .all(|operation| changes_support_state(operation.name(), Some(operation.at()), binding))
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
            "the configuration is on vendor support with changes disabled; enable editing with `supportCapability.set` in a separate apply before ordinary edits",
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
    let support_rule =
        support_root_uuid_from_bytes(&bytes).and_then(|uuid| state.object_rule(&uuid));
    if matches!(support_rule, Some(0 | 1))
        && request.ops().iter().any(|operation| {
            unica_metadata::apply_operations::MetadataOperation::named(operation.name())
                == Some(unica_metadata::apply_operations::MetadataOperation::ObjectRemove)
        })
    {
        let rule_name = if support_rule == Some(0) {
            "locked"
        } else {
            "editable"
        };
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "`{owner}` is a vendor object with the support rule `{rule_name}`; set `off-support` with `supportRule.set` in a separate apply before removal"
            ),
        )
        .at_path(at));
    }
    if support_rule == Some(0) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "`{owner}` is a vendor object with the support rule `locked`; allow editing with `supportRule.set` in a separate apply before ordinary edits"
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
    use unica_address::address::NodeKind;
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_source_designer::layout::metadata_descriptor_relative;

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
    at: &unica_address::address::QualifiedAddress,
    source_kind: SourceSetKind,
) -> Vec<std::path::PathBuf> {
    use unica_address::address::NodeKind;
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_source_designer::layout::{attached_resource_relative, metadata_descriptor_relative};

    let mut chain = Vec::new();
    if matches!(
        source_kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        chain.push(std::path::PathBuf::from("Configuration.xml"));
    }
    let segments = at.segments();
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
    Metadata(IndexedPlanOperation<unica_metadata_designer::decode::MetadataPlanOperation>),
    FormResource(IndexedPlanOperation<form_resource::FormResourcePlanOperation>),
    DcsMxl(IndexedPlanOperation<dcs_mxl::DcsMxlPlanOperation>),
    Code(IndexedPlanOperation<unica_bsl::input::CodePlanOperation>),
    Xdto(IndexedPlanOperation<unica_xdto::operations::XdtoPlanOperation>),
    Event(IndexedPlanOperation<unica_metadata::event_input::EventImplementArgs>),
    Unsupported(usize),
}

impl ParsedApplyOperation {
    /// Consecutive operations of one family plan as a single batch.
    fn same_family(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
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

/// Политику записи семейства задаёт `SourceSetWriteSupport`; предварительная
/// проверка `apply`, планировщик и словарь `can` спрашивают одного владельца.
/// Запись реестра для вызова: общее имя (`attribute.add` и `command.add`
/// объекта и формы, #1465) выбирает её адрес вызова, как предварительная
/// проверка `apply`.
pub(super) fn resolved_descriptor(
    operation: &str,
    at: Option<&unica_address::address::QualifiedAddress>,
    binding: &ProviderRootBinding,
) -> Option<unica_operations::OperationDescriptor> {
    let registry = crate::domain::apply::operation_registry();
    let traits = crate::domain::apply::source_set_traits_of_source(
        binding.source_kind(),
        binding.source_format(),
        binding.source_profile(),
    );
    at.and_then(|at| crate::domain::apply::operation_site(at, traits))
        .and_then(|site| registry.lookup_at(operation, &site))
        .or_else(|| registry.lookup(operation))
}

fn validate_platform_xml_binding(
    binding: &ProviderRootBinding,
    operation: &str,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let traits = crate::domain::apply::source_set_traits_of_source(
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
    let Some(descriptor) = resolved_descriptor(operation.name(), Some(operation.at()), binding)
    else {
        return Ok(());
    };
    validate_platform_xml_binding(binding, operation.name(), op_index)?;
    let traits = crate::domain::apply::source_set_traits_of_source(
        binding.source_kind(),
        binding.source_format(),
        binding.source_profile(),
    );
    let applies = crate::domain::apply::operation_site(operation.at(), traits)
        .is_some_and(|site| descriptor.applies_at(&site));
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
    let Some(descriptor) = resolved_descriptor(operation.name(), Some(operation.at()), binding)
    else {
        return Err(hidden_apply_family_unimplemented(op_index));
    };
    let args = serde_json::Value::Object(operation.args().clone());
    if descriptor.operation::<unica_metadata::apply_operations::MetadataOperation>()
        == Some(unica_metadata::apply_operations::MetadataOperation::ObjectBorrow)
    {
        return borrow::parse(&args, op_index, binding).map(ParsedApplyOperation::Borrow);
    }
    let parsed = match descriptor.family() {
        OperationFamily::Metadata | OperationFamily::Properties => {
            validate_platform_xml_binding(binding, operation.name(), op_index)?;
            let parsed = unica_metadata_designer::decode::parse_metadata_plan_operation_in(
                Some(descriptor),
                operation.name(),
                &args,
                op_index,
                &unica_metadata_designer::decode::MetadataDecodeContext {
                    source_set_name: binding.source_set_name(),
                    registry: crate::domain::apply::operation_registry(),
                },
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
            let parsed = form_resource::parse_form_resource_plan_operation_in(
                Some(descriptor),
                operation.name(),
                &args,
                op_index,
                binding,
            )?;
            ParsedApplyOperation::FormResource(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Dcs | OperationFamily::Mxl => {
            let parsed = dcs_mxl::parse_dcs_mxl_plan_operation_in(
                Some(descriptor),
                operation.name(),
                &args,
                op_index,
                binding,
            )?;
            ParsedApplyOperation::DcsMxl(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Code => {
            let parsed = unica_bsl::input::parse_code_plan_operation(
                operation.name(),
                &args,
                op_index,
                (binding).source_set_name(),
            )?;
            ParsedApplyOperation::Code(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Xdto => {
            let operation = descriptor
                .operation::<unica_xdto::operations::XdtoOperation>()
                .ok_or_else(|| hidden_apply_family_unimplemented(op_index))?;
            let parsed = unica_xdto_designer::planner::decode_xdto_plan_operation(
                operation,
                &args,
                op_index,
                binding.source_set_name(),
            )?;
            ParsedApplyOperation::Xdto(IndexedPlanOperation::new(op_index, parsed))
        }
        OperationFamily::Event => {
            let operation = descriptor
                .operation::<unica_metadata::event::EventOperation>()
                .ok_or_else(|| hidden_apply_family_unimplemented(op_index))?;
            let parsed = unica_metadata::event_input::parse_event_operation(
                operation,
                args,
                &format!("ops[{op_index}].args"),
            )?;
            ParsedApplyOperation::Event(IndexedPlanOperation::new(op_index, parsed))
        }
    };
    Ok(parsed)
}

/// Разбор семейством без проверки места реестром: семейство выбирает запись
/// реестра для адреса вызова (общее имя `attribute.add` объекта и формы,
/// #1465), но применимость не проверяется. Двусторонняя проверка `can`
/// ищет так адреса, которые семейство приняло бы, а реестр — нет.
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
    at: &unica_address::address::QualifiedAddress,
) -> Option<(unica_address::source_target::MetadataAddress, usize)> {
    use unica_address::address::NodeKind;
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
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
    template: &unica_address::address::QualifiedAddress,
) -> Option<unica_templates::TemplateFormat> {
    let (address, covered) = descriptor_node(template)?;
    if covered != template.segments().len() {
        return None;
    }
    let relative = unica_source_designer::layout::metadata_descriptor_relative(
        &address,
        binding.source_kind(),
    )
    .ok()?;
    let bytes = admission.staged_state().ok()?.read(&relative).ok()??;
    unica_templates_designer::descriptor::template_type_of(&bytes)
        .map(|raw| unica_templates::TemplateFormat::parse(&raw))
}

/// Поиск узла схемы по имени в стадированном макете: владелец — адрес
/// макета (набор данных, в том числе член объединения, #1353) или его
/// варианта настроек (именованный элемент структуры, #1357). Схемы нет —
/// `Missing`.
fn staged_schema_lookup(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    owner: &unica_address::address::QualifiedAddress,
    name: &str,
) -> unica_operations::recovery::NamedLookup {
    use unica_address::address::NodeKind;
    use unica_operations::recovery::NamedLookup;
    let segments = owner.segments();
    let (template, variant) = match segments.last() {
        Some(last) if last.kind() == NodeKind::Template => (owner.clone(), None),
        Some(last) if last.kind() == NodeKind::Setting && segments.len() > 1 => (
            unica_address::address::QualifiedAddress {
                line_range: None,
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
    let Some(bytes) = unica_source_designer::layout::attached_resource_relative(
        &address,
        "Template.xml",
        binding.source_kind(),
    )
    .ok()
    .and_then(|relative| admission.staged_state().ok()?.read(&relative).ok()?) else {
        return NamedLookup::Missing;
    };
    match variant {
        Some(variant) => {
            unica_dcs_designer::recovery::structure_item_path_in_schema(&bytes, variant, name)
        }
        None => unica_dcs_designer::recovery::data_set_path_in_schema(&bytes, name),
    }
}

/// Путь узла с именем `name` под контейнером в стадированном состоянии:
/// набора схемы под макетом, элемента структуры под вариантом настроек или
/// элемента под формой. Им снятый аргумент, называвший узел по имени, ведёт
/// к его настоящему адресу (#1353, #1355, #1357).
pub(crate) fn staged_member_path(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    container: &unica_address::address::QualifiedAddress,
    name: &str,
) -> unica_operations::recovery::NamedLookup {
    match container.segments().last().map(|segment| segment.kind()) {
        Some(unica_address::address::NodeKind::Form) => {
            match staged_form_items(admission, binding, container) {
                Some(items) => unica_forms_designer::planner::form_item_path(&items, name),
                None => unica_operations::recovery::NamedLookup::Missing,
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
    item: &unica_address::address::QualifiedAddress,
) -> Option<unica_forms::FormItemRole> {
    use unica_address::address::NodeKind;
    let segments = item.segments();
    if segments.len() < 3
        || segments[1].kind() != NodeKind::Form
        || !segments[2..]
            .iter()
            .all(|segment| segment.kind() == NodeKind::Item && segment.name().is_some())
    {
        return None;
    }
    let form = unica_address::address::QualifiedAddress {
        line_range: None,
        source_set: item.source_set().to_string(),
        segments: segments[..2].to_vec(),
    };
    let items = staged_form_items(admission, binding, &form)?;
    let chain = segments[2..]
        .iter()
        .filter_map(|segment| segment.name().map(str::to_string))
        .collect::<Vec<_>>();
    unica_forms_designer::planner::form_item_at(&items, &chain)
        .map(|element| unica_forms::FormItemRole::from_tag(&element.tag))
}

/// Дерево элементов стадированной формы `Owner.X.Form.F`, как его читает
/// `view`. `None` — формы или её `Form.xml` нет.
fn staged_form_items(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    form: &unica_address::address::QualifiedAddress,
) -> Option<Vec<unica_forms::facts::FormInfoElement>> {
    let (address, covered) = descriptor_node(form)?;
    if covered != 2 {
        return None;
    }
    let relative = unica_source_designer::layout::attached_resource_relative(
        &address,
        "Form.xml",
        binding.source_kind(),
    )
    .ok()?;
    let bytes = admission.staged_state().ok()?.read(&relative).ok()??;
    unica_forms_designer::planner::form_items_of(std::str::from_utf8(&bytes).ok()?).ok()
}

/// Есть ли узел `at` в стадированном состоянии допуска. Узел с собственным
/// описанием есть, когда есть его файл; член объекта метаданных (реквизит,
/// табличная часть) при существующем описании владельца считается
/// отсутствующим — отказ «не найден» по адресу операции говорит о нём.
/// `None` — по описаниям этого не установить.
pub(crate) fn staged_node_existence(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    at: &unica_address::address::QualifiedAddress,
) -> Option<bool> {
    let (address, covered) = descriptor_node(at)?;
    let relative = unica_source_designer::layout::metadata_descriptor_relative(
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
        crate::domain::source_capabilities::bsl_profile_for_source(binding.source_profile())
            .unwrap_or_else(unica_bsl::profile::PlatformProfile::v8_3_27),
    )?;
    // Запись прав роли и область макета живут в присоединённом файле: их
    // создаёт операция родителя (`right.add`, `area.add`, #1354).
    use unica_address::address::NodeKind;
    use unica_source_contract::route::RouteClass;
    let terminal = at.segments().last()?;
    let resource = |name: &str| -> Option<Vec<u8>> {
        let relative = unica_source_designer::layout::attached_resource_relative(
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
            unica_roles_designer::planner::staged_rights_entry_exists(
                &resource("Rights.xml")?,
                &format!("{kind}.{name}"),
            )
        }
        (RouteClass::Mxl, NodeKind::Area, 1) => {
            unica_templates_designer::planner::spreadsheet_area_exists(
                &resource("Template.xml")?,
                terminal.name()?,
            )
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
            let form = unica_address::address::QualifiedAddress {
                line_range: None,
                source_set: at.source_set().to_string(),
                segments: at.segments()[..covered].to_vec(),
            };
            let items = staged_form_items(admission, binding, &form)?;
            let chain = chain
                .iter()
                .filter_map(|segment| segment.name().map(str::to_string))
                .collect::<Vec<_>>();
            Some(unica_forms_designer::planner::form_item_at(&items, &chain).is_some())
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
    let support_operations = request
        .ops()
        .iter()
        .filter(|op| changes_support_state(op.name(), Some(op.at()), binding))
        .count();
    if support_operations > 0 && support_operations < request.ops().len() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "support changes require a separate apply before ordinary edits",
        )
        .at_path("ops"));
    }
    let mut provisional = Vec::new();
    let mut xdto_validation = unica_xdto_designer::planner::XdtoValidationLedger::default();
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
                let planned = crate::infrastructure::native_operations::code_patch_composition::plan_code_batch(
                    &mut staged.proposal(),
                    &authority,
                    &operations,
                )?;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned, cursor,
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
                    crate::infrastructure::native_operations::event_implementation::plan_event_implement_batch(
                        &mut staged.proposal(),
                        authority.source_set_name(),
                        authority.source_kind(),
                        crate::domain::source_capabilities::bsl_profile_for_source(
                            authority.source_profile(),
                        )
                        .expect(
                            "an issued metadata authority has the exact writable source profile",
                        ),
                        &operations,
                    )
                    ?;
                provisional.extend(batch_effects_as_provisional(
                    &before, &staged, planned, cursor,
                ));
            }
            ParsedApplyOperation::Xdto(_) => {
                let operations = parsed[cursor..end]
                    .iter()
                    .map(|operation| match operation {
                        ParsedApplyOperation::Xdto(operation) => operation.clone(),
                        _ => unreachable!("the selected run is XDTO-only"),
                    })
                    .collect::<Vec<_>>();
                let authority = admission.xdto_planning_authority(binding)?;
                let planned = unica_xdto_designer::planner::plan_xdto_batch(
                    &mut staged.proposal(),
                    &authority,
                    &operations,
                    &mut xdto_validation,
                )?;
                provisional.extend(planned);
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
                let planned =
                    unica_metadata_designer::planner::plan_metadata_batch(&crate::infrastructure::native_operations::meta::template_source::MetadataPlanningReadAdapter::new(&authority),unica_metadata_designer::planner::TemplateSeedEmitter(unica_templates_designer::mxl_root::empty_spreadsheet_document_xml),&mut staged.proposal(), &authority, &operations)?;
                provisional.extend(planned);
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
                let planned = form_resource::plan_form_resource_batch(
                    &mut staged.proposal(),
                    &authority,
                    &operations,
                )?;
                provisional.extend(planned);
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
                let planned =
                    dcs_mxl::plan_dcs_mxl_batch(&mut staged.proposal(), &authority, &operations)?;
                provisional.extend(planned);
            }
            ParsedApplyOperation::Unsupported(_) => unreachable!("unsupported run handled above"),
        }
        cursor = end;
    }

    unica_metadata_designer::planner::validate_final_object_integrity(
        &staged.proposal(),
        binding.source_set_name(),
        binding.source_kind(),
    )?;
    unica_dcs_designer::planner::validate_final_dcs_resources(
        &mut staged.proposal(),
        binding.source_kind(),
        parsed.iter().filter_map(|operation| match operation {
            ParsedApplyOperation::DcsMxl(operation) => operation
                .operation()
                .dcs_plan()
                .map(|edit| (operation.index(), edit)),
            _ => None,
        }),
    )?;
    if !xdto_validation.is_empty() {
        let authority = admission.xdto_planning_authority(binding)?;
        unica_xdto_designer::planner::validate_final_xdto_resources(
            &mut staged.proposal(),
            &authority,
            &xdto_validation,
            parsed.iter().filter_map(|operation| match operation {
                ParsedApplyOperation::Xdto(operation) => Some(operation),
                _ => None,
            }),
        )?;
    }
    let xdto_operations = parsed
        .iter()
        .filter_map(|operation| match operation {
            ParsedApplyOperation::Xdto(operation) => Some(operation.operation().clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let xdto_objects = unica_xdto_designer::planner::final_object_addition_details(
        &mut staged.proposal(),
        binding.source_set_name(),
        binding.source_kind(),
        &xdto_operations,
    )?;
    let mut effects = reconcile_effects(staged.planned_changes(), provisional);
    effects
        .set_details(unica_xdto::results::XDTO_OBJECTS, &xdto_objects)
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, error.to_string()).at_path("ops")
        })?;
    effects.set_borrowing(borrowing);
    Ok((staged, effects))
}

#[cfg(test)]
mod inherited_content_tests;

#[cfg(test)]
mod object_integrity_tests;

#[cfg(test)]
mod tests {
    use super::plan_hidden_v13_apply;
    use crate::domain::apply::ApplyRequest;
    use unica_source_contract::proposal::{reconcile_effects, ProvisionalApplyEffect};
    use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

    use crate::infrastructure::source_workspace::WorkspaceActor;
    use std::sync::Arc;
    use std::time::Duration;
    use unica_source_contract::proposal::ApplyPlanErrorKind;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;
    use unica_workspace::actor::{
        ApplyAdmission, ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput,
    };

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
            let actor = Arc::new(
                crate::infrastructure::source_workspace::new_actor(identity, context).unwrap(),
            );
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
        let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = written.current else {
            panic!("the spreadsheet keeps bytes");
        };
        assert_eq!(
            unica_templates_designer::planner::spreadsheet_area_exists(&bytes, "Header"),
            Some(true)
        );
        assert_eq!(
            unica_templates_designer::planner::spreadsheet_area_exists(&bytes, "Headr"),
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
        let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = written.current else {
            panic!("the spreadsheet keeps bytes");
        };
        assert_eq!(
            unica_templates_designer::planner::spreadsheet_area_exists(&bytes, "Head.er"),
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
                &unica_address::address::QualifiedAddress::parse(raw).unwrap(),
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

    #[test]
    fn support_state_classification_preserves_names_across_targets_and_profiles() {
        use unica_address::address::QualifiedAddress;
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        let workspace = source.parent().unwrap().to_path_buf();
        let mut names = crate::domain::apply::operation_registry().names();
        names.extend(["", "supportCapability.set.extra", "supportRule.unknown"]);
        for (kind, format, profile) in [
            (
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
            (
                SourceSetKind::Extension,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
            (
                SourceSetKind::ExternalProcessor,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
            (
                SourceSetKind::ExternalReport,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
            (
                SourceSetKind::Configuration,
                SourceFormat::Edt,
                SourceProfile::legacy_workspace_service_compatibility(),
            ),
            (
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::legacy_workspace_service_compatibility(),
            ),
            (
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::TestPlatform8_3_28Format2_20,
            ),
            (
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::TestPlatform8_3_27Format2_21,
            ),
        ] {
            let context = WorkspaceContext {
                cwd: workspace.clone(),
                workspace_root: workspace.clone(),
                cache_root: workspace.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main", &source, kind, format, profile,
                )],
                "support-state-classification",
            )
            .unwrap();
            let actor =
                crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
            let binding = actor.bind_provider_root("main", &source).unwrap();
            for raw_target in [
                None,
                Some("main:Configuration"),
                Some("main:Document.First"),
                Some("main:Document.First.Form.Card"),
                Some("main:Role.Reader"),
                Some("other:Configuration"),
                Some("not a logical address"),
                Some("main:Document."),
            ] {
                let target = raw_target.and_then(|raw| QualifiedAddress::parse(raw).ok());
                for name in &names {
                    // The previous name-only gate exempted exactly these two operations.
                    let expected = matches!(*name, "supportCapability.set" | "supportRule.set");
                    assert_eq!(
                        super::changes_support_state(name, target.as_ref(), &binding),
                        expected,
                        "{name} at {raw_target:?}, {kind:?}/{format:?}/{profile:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn support_and_ordinary_edits_require_separate_batches_under_every_policy() {
        for policy in ["deny", "warn", "off"] {
            let fixture = ApplySeamFixture::new();
            std::fs::write(
                fixture
                    .source_dir()
                    .parent()
                    .unwrap()
                    .join(".v8-project.json"),
                serde_json::json!({"editingAllowedCheck": policy}).to_string(),
            )
            .unwrap();
            let request = ApplyRequest::parse(
                serde_json::json!({
                    "at": "main:Document.First.Form.Card",
                    "ops": [
                        {"op": "supportRule.set", "args": {"values": {"rule": "editable"}}},
                        {"op": "item.add", "args": {"items": [{
                            "name": "Field",
                            "type": "InputField",
                            "path": "Object.Field"
                        }]}}
                    ], "dryRun": true
                })
                .as_object()
                .unwrap(),
                &["main"],
            )
            .unwrap();
            let result = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission());
            let error = result.unwrap_err();
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::InvalidState,
                "{policy}: {error}"
            );
            assert!(
                error.to_string().contains("separate apply"),
                "{policy}: {error}"
            );
        }
    }

    #[test]
    fn vendor_object_removal_requires_separate_off_support_apply() {
        use unica_workspace::test_support::tree_snapshot;

        for rule in [1, 0] {
            let fixture = ApplySeamFixture::new();
            let marker = fixture.source_dir().join("Ext/ParentConfigurations.bin");
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(&marker, vendor_removal_support_marker(rule, true)).unwrap();
            let before = tree_snapshot(&fixture.source_dir());
            if rule == 1 {
                let ordinary = seam_request(
                    "main:Document.First",
                    "props.set",
                    serde_json::json!({"values": {"Comment": "editable"}}),
                );
                let (staged, _) =
                    plan_hidden_v13_apply(&ordinary, &fixture.binding, &fixture.admission())
                        .expect("the editable rule still permits ordinary property edits");
                assert!(!staged.planned_changes().is_empty());
            }
            for force in [false, true] {
                let request = seam_request(
                    "main:Document.First",
                    "object.remove",
                    serde_json::json!({"force": force}),
                );
                let error = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission())
                    .expect_err("a vendor object must be off support before removal");
                assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
                assert!(error.to_string().contains("off-support"), "{error}");
                assert!(error.to_string().contains("supportRule.set"), "{error}");
                assert!(error.to_string().contains("separate apply"), "{error}");
                assert_eq!(tree_snapshot(&fixture.source_dir()), before);
            }
        }
    }

    fn vendor_removal_support_marker(rule: u8, editing_enabled: bool) -> String {
        let global_flag = u8::from(!editing_enabled);
        format!(
            "\u{feff}{{6,{global_flag},1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,33333333-3333-4333-8333-333333333333,\"1.0\",\"Vendor\",\"VendorConf\",3,{rule},0,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111}}"
        )
    }

    #[test]
    fn vendor_object_removal_preserves_allowed_support_states() {
        use unica_workspace::test_support::tree_snapshot;

        for (policy, marker) in [
            ("deny", None),
            ("deny", Some(vendor_removal_support_marker(2, true))),
            ("deny", Some("\u{feff}{6,0,0}".to_string())),
            (
                "deny",
                Some(vendor_removal_support_marker(0, true).replace(
                    "11111111-1111-4111-8111-111111111111",
                    "22222222-2222-4222-8222-222222222222",
                )),
            ),
            ("warn", Some(vendor_removal_support_marker(0, false))),
            ("warn", Some(vendor_removal_support_marker(1, true))),
            ("off", Some(vendor_removal_support_marker(0, false))),
            ("off", Some(vendor_removal_support_marker(1, true))),
        ] {
            let fixture = ApplySeamFixture::new();
            std::fs::write(
                fixture
                    .source_dir()
                    .parent()
                    .unwrap()
                    .join(".v8-project.json"),
                serde_json::json!({"editingAllowedCheck": policy}).to_string(),
            )
            .unwrap();
            if let Some(marker) = marker {
                let path = fixture.source_dir().join("Ext/ParentConfigurations.bin");
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, marker).unwrap();
            }
            let before = tree_snapshot(&fixture.source_dir());
            let request = seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            );
            let (mut staged, _) =
                plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission())
                    .unwrap_or_else(|error| panic!("allowed removal under {policy}: {error}"));
            assert!(staged
                .read(std::path::Path::new("Documents/First.xml"))
                .unwrap()
                .is_none());
            assert_eq!(tree_snapshot(&fixture.source_dir()), before);
        }
    }

    #[test]
    fn support_removal_and_object_removal_require_separate_batches_under_every_policy() {
        use unica_workspace::test_support::tree_snapshot;

        for policy in ["deny", "warn", "off"] {
            let fixture = ApplySeamFixture::new();
            std::fs::write(
                fixture
                    .source_dir()
                    .parent()
                    .unwrap()
                    .join(".v8-project.json"),
                serde_json::json!({"editingAllowedCheck": policy}).to_string(),
            )
            .unwrap();
            let marker = fixture.source_dir().join("Ext/ParentConfigurations.bin");
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(marker, vendor_removal_support_marker(1, true)).unwrap();
            let before = tree_snapshot(&fixture.source_dir());
            let request = ApplyRequest::parse(
                serde_json::json!({
                    "at": "main:Document.First",
                    "ops": [
                        {"op": "supportRule.set", "args": {"values": {"rule": "off-support"}}},
                        {"op": "object.remove", "args": {}}
                    ], "dryRun": true
                })
                .as_object()
                .unwrap(),
                &["main"],
            )
            .unwrap();
            let error = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission())
                .expect_err("support removal cannot authorize removal in the same apply");
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
            assert!(
                error.to_string().contains("separate apply"),
                "{policy}: {error}"
            );
            assert_eq!(tree_snapshot(&fixture.source_dir()), before);
        }
    }

    /// The vendor-support gate of the canonical path: a locked vendor object
    /// refuses every family before a byte is staged, the support operations
    /// stay the way out of the lock, and a source without the marker is not
    /// on support at all.
    #[test]
    fn locked_vendor_objects_refuse_every_family_before_staging() {
        use unica_operations::OperationFamily;

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
        for descriptor in crate::domain::apply::operation_registry().descriptors() {
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
        let before = snapshot();
        for (operation, args) in [
            (
                "props.set",
                serde_json::json!({"values": {"Comment": "edited"}}),
            ),
            ("object.remove", serde_json::json!({})),
        ] {
            let request = seam_request("main:Document.Second", operation, args);
            let error = plan_hidden_v13_apply(&request, &fixture.binding, &admission).unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
            assert!(error.to_string().contains("changes disabled"), "{error}");
            assert_eq!(snapshot(), before);
        }

        // Without the marker the source is not on support and edits plan.
        std::fs::remove_file(ext.join("ParentConfigurations.bin")).unwrap();
        let admission = fixture.admission();
        let request = seam_request(
            "main:Document.Second",
            "props.set",
            serde_json::json!({"values": {"Comment": "edited"}}),
        );
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
        use unica_source_contract::events::{DomainEvent, DomainEventKind};
        use unica_source_contract::proposal::PlannedApplyEffects;

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
        let reconciled = reconcile_effects(staged.planned_changes(), provisional);
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
            staged.planned_changes(),
            vec![
                // The surviving duplicate arrives before its earlier sibling.
                ProvisionalApplyEffect::single(
                    second_path,
                    unica_source_contract::events::DomainEvent::new(
                        unica_source_contract::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    3,
                ),
                ProvisionalApplyEffect::single(
                    first_path,
                    unica_source_contract::events::DomainEvent::new(
                        unica_source_contract::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    0,
                ),
                ProvisionalApplyEffect::spanning(
                    vec![first_path.to_path_buf(), second_path.to_path_buf()],
                    unica_source_contract::events::DomainEvent::new(
                        unica_source_contract::events::DomainEventKind::MetadataChanged,
                        "partly-restored",
                    ),
                    4,
                ),
                ProvisionalApplyEffect::single(
                    second_path,
                    unica_source_contract::events::DomainEvent::new(
                        unica_source_contract::events::DomainEventKind::MetadataChanged,
                        "shared",
                    ),
                    2,
                ),
                ProvisionalApplyEffect::single(
                    second_path,
                    unica_source_contract::events::DomainEvent::new(
                        unica_source_contract::events::DomainEventKind::SourceSetChanged,
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
                    unica_source_contract::events::DomainEventKind::SourceSetChanged,
                    "earlier"
                ),
                (
                    unica_source_contract::events::DomainEventKind::MetadataChanged,
                    "shared"
                )
            ]
        );
    }
}
