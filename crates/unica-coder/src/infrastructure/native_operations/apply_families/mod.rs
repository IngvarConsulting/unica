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

/// Every edit of an owner registers its forms and templates by name (#1370).
/// Before #1325 an owner could carry a form's or template's whole description
/// inline next to its standalone `Forms/<Name>.xml` or `Templates/<Name>.xml`,
/// and 1C 8.3.27 then imports the configuration endlessly. An owner this plan
/// changes loses such inline registrations whose standalone descriptor
/// exists; the plan names them in a `legacy_inline_registration` warning, so
/// the rewrite of bytes the operation did not ask for is not silent.
fn collapse_legacy_inline_registrations(
    staged: &mut ApplyStagedState,
    binding: &ProviderRootBinding,
) -> Result<Vec<serde_json::Value>, ApplyPlanError> {
    let mut warnings = Vec::new();
    for change in staged.planned_changes() {
        let unica_source_contract::proposal::StagedFileState::Bytes(current) = &change.current
        else {
            continue;
        };
        let parts = change
            .relative_path
            .iter()
            .filter_map(std::ffi::OsStr::to_str)
            .collect::<Vec<_>>();
        // An object of a configuration or extension is `<Kind dir>/<Name>.xml`;
        // an external data processor or report is `<Name>.xml` at the root.
        let (kind, owner_dir, file) = match (parts.as_slice(), binding.source_kind()) {
            ([directory, file], _) => {
                let Some(layout) =
                    unica_source_designer::layout::metadata_kind_by_directory(directory)
                else {
                    continue;
                };
                (layout.tag, std::path::PathBuf::from(directory), *file)
            }
            ([file], SourceSetKind::ExternalProcessor) => {
                ("ExternalDataProcessor", std::path::PathBuf::new(), *file)
            }
            ([file], SourceSetKind::ExternalReport) => {
                ("ExternalReport", std::path::PathBuf::new(), *file)
            }
            _ => continue,
        };
        let Some(name) = file.strip_suffix(".xml") else {
            continue;
        };
        let (bom, text) = match current.strip_prefix(b"\xef\xbb\xbf") {
            Some(text) => (&b"\xef\xbb\xbf"[..], text),
            None => (&b""[..], current.as_slice()),
        };
        let Ok(text) = std::str::from_utf8(text) else {
            continue;
        };
        let mut image = text.to_string();
        let owner_dir = owner_dir.join(name);
        let mut described = std::collections::BTreeSet::new();
        for (tag, child) in unica_metadata_designer::edit::inline_standalone_child_names(&image) {
            let collection = if tag == "Form" { "Forms" } else { "Templates" };
            let descriptor = owner_dir.join(collection).join(format!("{child}.xml"));
            if staged
                .read(&descriptor)
                .map_err(|error| ApplyPlanError::staging(error, "ops"))?
                .is_some()
            {
                described.insert((tag, child));
            }
        }
        let collapsed = unica_metadata_designer::edit::collapse_legacy_inline_children(
            &mut image,
            &|tag, child| {
                described
                    .iter()
                    .any(|(known, known_name)| *known == tag && known_name == child)
            },
        );
        if collapsed.is_empty() {
            continue;
        }
        let mut bytes = bom.to_vec();
        bytes.extend_from_slice(image.as_bytes());
        staged
            .replace(&change.relative_path, current, bytes)
            .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
        let names = collapsed
            .iter()
            .map(|(tag, child)| format!("{tag}.{child}"))
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(serde_json::json!({
            "code": "legacy_inline_registration",
            "severity": "warning",
            "at": format!("{}:{kind}.{name}", binding.source_set_name()),
            "message": format!(
                "the owner registers {names} by name now: the description inline in the owner is an older Unica format that 1C 8.3.27 imports endlessly"
            ),
        }));
    }
    Ok(warnings)
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

/// Vendor-support gate of the canonical mutation path. A request whose owner
/// is a vendor object with the rule `locked`, or whose configuration is on
/// support with changes disabled, refuses `invalid_state` before any family
/// stages a byte when the actor's support policy is `deny`; under `warn` it
/// proceeds and returns the `support_locked` warning the plan carries; `off`
/// is silent. Removal follows the same policy: an editable vendor object is
/// removed while staying on support, and only a `locked` rule on the object
/// or on one of its own child objects refuses under `deny` or warns under
/// `warn`. The marker is not changed by the removal.
/// The owner is the top-level metadata object of the addressed node, or the
/// configuration root for root-level targets; nested forms, templates and
/// attributes inherit the rule of their owner, as they do in the platform.
fn refuse_targets_on_locked_vendor_objects(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    staged: &mut ApplyStagedState,
) -> Result<Option<serde_json::Value>, ApplyPlanError> {
    use unica_metadata_designer::support::{
        parse_support_state_compat_bytes, support_root_uuid_from_bytes,
    };
    use unica_source_contract::proposal::SupportPolicyMode;

    let deny = match admission.support_policy_mode() {
        SupportPolicyMode::Deny => true,
        SupportPolicyMode::Warn => false,
        SupportPolicyMode::Off => return Ok(None),
    };
    if request
        .ops()
        .iter()
        .all(|operation| changes_support_state(operation.name(), Some(operation.at()), binding))
    {
        return Ok(None);
    }
    let support_locked = |owner: &str, message: String| {
        serde_json::json!({
            "code": "support_locked",
            "severity": "warning",
            "at": format!("{}:{owner}", request.at().source_set()),
            "message": message,
        })
    };
    let at = request.at().to_string();
    let source_set = request.at().source_set();
    let removes = request.ops().iter().any(|operation| {
        unica_metadata::apply_operations::MetadataOperation::named(operation.name())
            == Some(unica_metadata::apply_operations::MetadataOperation::ObjectRemove)
    });
    let marker_path = std::path::Path::new("Ext/ParentConfigurations.bin");
    let marker = staged
        .read(marker_path)
        .map_err(|error| ApplyPlanError::staging(error, at.clone()))?;
    let Some(state) = parse_support_state_compat_bytes(marker.as_deref()) else {
        return Ok(None);
    };
    if state.removed() {
        return Ok(None);
    }
    if !state.global_editing_enabled() {
        if !deny {
            let owner = support_owner_relative(request, binding.source_kind())
                .map_or_else(|| "Configuration".to_string(), |(owner, _)| owner);
            let permits = if removes { "its removal" } else { "this edit" };
            return Ok(Some(support_locked(
                &owner,
                format!("the configuration is on vendor support with changes disabled; the project support policy `warn` permits {permits}"),
            )));
        }
        let before = if removes { "removal" } else { "ordinary edits" };
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!("the configuration is on vendor support with changes disabled; enable editing with `supportCapability.set` in a separate apply before {before}"),
        )
        .at_path(at));
    }
    let Some((owner, owner_relative)) = support_owner_relative(request, binding.source_kind())
    else {
        return Ok(None);
    };
    let Some(bytes) = staged
        .read(&owner_relative)
        .map_err(|error| ApplyPlanError::staging(error, at.clone()))?
    else {
        return Ok(None);
    };
    let support_rule =
        support_root_uuid_from_bytes(&bytes).and_then(|uuid| state.object_rule(&uuid));
    if removes {
        // An editable vendor object is removed without leaving support, as
        // the configurator does; only a lock on it or on one of its own
        // child objects stands in the way.
        // An object taken off support no longer answers for its children.
        let reason = if support_rule == Some(0) {
            format!("`{source_set}:{owner}` is a vendor object with the support rule `locked`")
        } else if support_rule != Some(1) {
            return Ok(None);
        } else {
            let children = locked_vendor_children(
                &state,
                source_set,
                &owner,
                &owner_relative,
                &bytes,
                staged,
                &at,
            )?;
            if children.is_empty() {
                return Ok(None);
            }
            format!(
                "`{source_set}:{owner}` has vendor child objects with the support rule `locked`: {}",
                children.join(", ")
            )
        };
        if !deny {
            return Ok(Some(support_locked(
                &owner,
                format!("{reason}; the project support policy `warn` permits its removal"),
            )));
        }
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "{reason}; take the object off support with `supportRule.set` and the rule `off-support` in a separate apply before removal"
            ),
        )
        .at_path(at));
    }
    if support_rule == Some(0) {
        if !deny {
            return Ok(Some(support_locked(
                &owner,
                format!("`{source_set}:{owner}` is a vendor object with the support rule `locked`; the project support policy `warn` permits this edit"),
            )));
        }
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "`{source_set}:{owner}` is a vendor object with the support rule `locked`; allow editing with `supportRule.set` in a separate apply before ordinary edits"
            ),
        )
        .at_path(at));
    }
    Ok(None)
}

/// The child objects of a removed vendor object that carry their own
/// `locked` rule: attributes, tabular sections and other nodes declared in
/// its descriptor, and forms, templates and commands described in their own
/// descriptors under its directory.
fn locked_vendor_children(
    state: &unica_metadata_designer::support::SupportState,
    source_set: &str,
    owner: &str,
    owner_relative: &std::path::Path,
    owner_bytes: &[u8],
    staged: &mut ApplyStagedState,
    at: &str,
) -> Result<Vec<String>, ApplyPlanError> {
    use unica_metadata_designer::remove::{declared_chain, declared_name};

    let mut locked = Vec::new();
    let mut collect = |base: &str, bytes: &[u8]| {
        let Some(document) = std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).ok())
        else {
            return;
        };
        for node in document.descendants().filter(|node| node.is_element()) {
            let Some(uuid) = node.attribute("uuid") else {
                continue;
            };
            if state.object_rule(uuid) != Some(0) || declared_name(node).is_none() {
                continue;
            }
            let chain = declared_chain(node);
            // The owner's own rule is checked by the caller.
            if chain.is_empty() && base == owner {
                continue;
            }
            locked.push(format!("`{source_set}:{base}{}`", chain.concat()));
        }
    };
    collect(owner, owner_bytes);
    let directory = owner_relative.with_extension("");
    let (files, _) = staged
        .enumerate_tree_with_root_absence(&directory)
        .map_err(|error| ApplyPlanError::staging(error, at.to_string()))?;
    for relative in files {
        let (base, descriptor) = unica_metadata_designer::remove::meta_remove_referrer(&relative);
        let is_xml = relative
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"));
        if !descriptor || !is_xml {
            continue;
        }
        if let Some(bytes) = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, at.to_string()))?
        {
            collect(&base, &bytes);
        }
    }
    Ok(locked)
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
                    source_kind: binding.source_kind(),
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
    let Ok(mut staged) = admission.staged_state() else {
        return NamedLookup::Missing;
    };
    let member = match variant {
        Some(variant) => unica_dcs::codec::DcsMember::StructureItem { variant, name },
        None => unica_dcs::codec::DcsMember::DataSet(name),
    };
    crate::infrastructure::family_codecs::dcs_codec().member_path(
        &mut staged.proposal(),
        binding.source_kind(),
        &address,
        member,
    )
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
    let metadata_creation_overrides =
        unica_metadata_designer::planner::MetadataCreationOverrides::from_operations(
            parsed.iter().filter_map(|operation| match operation {
                ParsedApplyOperation::Metadata(operation) => Some(operation),
                _ => None,
            }),
        );

    let mut staged = admission
        .staged_state()
        .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
    refuse_targets_outside_writable_profile(request, binding, &mut staged)?;
    let support_warning =
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
                    cursor,
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
                    unica_metadata_designer::planner::plan_metadata_batch_with_context(&crate::infrastructure::native_operations::meta::template_source::MetadataPlanningReadAdapter::new(&authority),unica_metadata_designer::planner::TemplateSeedEmitter(unica_templates_designer::mxl_root::empty_spreadsheet_document_xml),&mut staged.proposal(), &authority, &operations, &metadata_creation_overrides)?;
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
        &mut staged.proposal(),
        binding.source_set_name(),
        binding.source_kind(),
    )?;
    let dcs_edits = parsed
        .iter()
        .filter_map(|operation| match operation {
            ParsedApplyOperation::DcsMxl(operation) => operation
                .operation()
                .dcs_plan()
                .map(|edit| (operation.index(), edit)),
            _ => None,
        })
        .collect::<Vec<_>>();
    crate::infrastructure::family_codecs::dcs_codec().check_staged(
        &mut staged.proposal(),
        binding.source_kind(),
        &dcs_edits,
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
    let exchange_content = unica_metadata_designer::planner::final_exchange_content(
        &mut staged.proposal(),
        binding.source_set_name(),
        binding.source_kind(),
        parsed.iter().filter_map(|operation| match operation {
            ParsedApplyOperation::Metadata(operation) => {
                operation.operation().exchange_content_owner().cloned()
            }
            _ => None,
        }),
    )?;
    let legacy_inline = collapse_legacy_inline_registrations(&mut staged, binding)?;
    let mut effects = reconcile_effects(staged.planned_changes(), provisional);
    // A plan whose operations cancel out edits nothing protected.
    if let Some(warning) = support_warning.filter(|_| !staged.planned_changes().is_empty()) {
        effects.push_warning(warning);
    }
    for warning in legacy_inline {
        effects.push_warning(warning);
    }
    let detail_error = |error: serde_json::Error| {
        ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, error.to_string()).at_path("ops")
    };
    effects
        .set_details(unica_xdto::results::XDTO_OBJECTS, &xdto_objects)
        .map_err(detail_error)?;
    effects
        .set_details(unica_metadata::EXCHANGE_CONTENT, &exchange_content)
        .map_err(detail_error)?;
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

    /// Plans the removal of `Document.First` under `policy` over `marker`
    /// and returns the outcome with the source tree before and after.
    fn plan_vendor_removal(
        policy: &str,
        marker: &str,
        prepare: impl FnOnce(&std::path::Path),
    ) -> (
        Result<Vec<serde_json::Value>, unica_source_contract::proposal::ApplyPlanError>,
        bool,
        std::path::PathBuf,
    ) {
        use unica_workspace::test_support::tree_snapshot;

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
        let path = fixture.source_dir().join("Ext/ParentConfigurations.bin");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, marker).unwrap();
        prepare(&fixture.source_dir());
        let before = tree_snapshot(&fixture.source_dir());
        let request = seam_request(
            "main:Document.First",
            "object.remove",
            serde_json::json!({}),
        );
        let outcome = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission()).map(
            |(mut staged, effects)| {
                assert!(staged
                    .read(std::path::Path::new("Documents/First.xml"))
                    .unwrap()
                    .is_none());
                // The removal leaves the support marker as it is.
                assert_eq!(
                    staged
                        .read(std::path::Path::new("Ext/ParentConfigurations.bin"))
                        .unwrap()
                        .as_deref(),
                    Some(marker.as_bytes())
                );
                effects.warnings().to_vec()
            },
        );
        let unchanged = tree_snapshot(&fixture.source_dir()) == before;
        (outcome, unchanged, fixture.source_dir())
    }

    /// An editable vendor object is removed while staying on support, as the
    /// configurator removes it, under every support policy and silently.
    #[test]
    fn vendor_object_removal_keeps_editable_objects_on_support_under_every_policy() {
        for policy in ["deny", "warn", "off"] {
            let (outcome, unchanged, _) =
                plan_vendor_removal(policy, &vendor_removal_support_marker(1, true), |_| {});
            let warnings =
                outcome.unwrap_or_else(|error| panic!("editable removal under {policy}: {error}"));
            assert!(warnings.is_empty(), "{policy}: {warnings:?}");
            assert!(unchanged, "{policy}");
        }
    }

    /// A locked vendor object refuses removal under `deny` with the way out,
    /// is removed with `support_locked` under `warn` and silently under `off`.
    #[test]
    fn vendor_object_removal_of_locked_objects_follows_the_support_policy() {
        let marker = vendor_removal_support_marker(0, true);
        let (outcome, unchanged, _) = plan_vendor_removal("deny", &marker, |_| {});
        let error = outcome.expect_err("a locked vendor object is not removed under deny");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        let message = error.to_string();
        assert!(message.contains("`main:Document.First`"), "{message}");
        assert!(message.contains("support rule `locked`"), "{message}");
        assert!(message.contains("supportRule.set"), "{message}");
        assert!(message.contains("off-support"), "{message}");
        assert!(message.contains("separate apply"), "{message}");
        assert!(unchanged);

        let (outcome, unchanged, _) = plan_vendor_removal("warn", &marker, |_| {});
        let warnings = outcome.expect("warn removes a locked vendor object");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0]["code"], "support_locked");
        assert_eq!(warnings[0]["at"], "main:Document.First");
        assert!(unchanged);

        let (outcome, unchanged, _) = plan_vendor_removal("off", &marker, |_| {});
        let warnings = outcome.expect("off removes a locked vendor object");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(unchanged);
    }

    /// A common template on support follows the same rules as any vendor
    /// object: locked, it refuses under `deny` and warns under `warn`.
    #[test]
    fn vendor_common_template_removal_follows_its_support_rule() {
        use unica_workspace::test_support::tree_snapshot;

        let template = "44444444-4444-4444-8444-444444444444";
        for (rule, policy) in [(0, "deny"), (0, "warn"), (1, "deny")] {
            let fixture = ApplySeamFixture::new();
            let source = fixture.source_dir();
            std::fs::write(
                source.parent().unwrap().join(".v8-project.json"),
                serde_json::json!({"editingAllowedCheck": policy}).to_string(),
            )
            .unwrap();
            std::fs::create_dir_all(source.join("Ext")).unwrap();
            std::fs::write(
                source.join("Ext/ParentConfigurations.bin"),
                format!(
                    "\u{feff}{{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,33333333-3333-4333-8333-333333333333,\"1.0\",\"Vendor\",\"VendorConf\",3,{rule},0,{template},{template}}}"
                ),
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>First</Document><Document>Second</Document><CommonTemplate>Print</CommonTemplate></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            std::fs::create_dir_all(source.join("CommonTemplates/Print/Ext")).unwrap();
            std::fs::write(
                source.join("CommonTemplates/Print.xml"),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonTemplate uuid="{template}"><Properties><Name>Print</Name><Synonym/><Comment/><TemplateType>SpreadsheetDocument</TemplateType></Properties></CommonTemplate></MetaDataObject>"#
                ),
            )
            .unwrap();
            std::fs::write(
                source.join("CommonTemplates/Print/Ext/Template.xml"),
                r#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
            )
            .unwrap();
            let before = tree_snapshot(&source);
            let request = seam_request(
                "main:CommonTemplate.Print",
                "object.remove",
                serde_json::json!({}),
            );
            let outcome = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission())
                .map(|(mut staged, effects)| {
                    assert!(staged
                        .read(std::path::Path::new("CommonTemplates/Print.xml"))
                        .unwrap()
                        .is_none());
                    effects.warnings().to_vec()
                });
            assert_eq!(tree_snapshot(&source), before);
            match (rule, policy) {
                (0, "deny") => {
                    let error = outcome.expect_err("a locked common template stays under deny");
                    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
                    let message = error.to_string();
                    assert!(message.contains("`main:CommonTemplate.Print`"));
                    assert!(message.contains("support rule `locked`"));
                }
                (0, _) => {
                    let warnings = outcome.expect("warn removes a locked common template");
                    assert_eq!(warnings.len(), 1);
                    assert_eq!(warnings[0]["code"], "support_locked");
                    assert_eq!(warnings[0]["at"], "main:CommonTemplate.Print");
                }
                _ => {
                    let warnings = outcome.expect("an editable common template is removed");
                    assert!(warnings.is_empty());
                }
            }
        }
    }

    /// With changes disabled for the whole configuration a removal is told
    /// so in its own words: `deny` refuses before removal, `warn` permits it.
    #[test]
    fn vendor_object_removal_with_changes_disabled_names_the_removal() {
        let marker = vendor_removal_support_marker(1, false);
        let (outcome, unchanged, _) = plan_vendor_removal("deny", &marker, |_| {});
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "the configuration is on vendor support with changes disabled; enable editing with `supportCapability.set` in a separate apply before removal"
        );
        assert!(unchanged);
        let (outcome, _, _) = plan_vendor_removal("warn", &marker, |_| {});
        let warnings = outcome.expect("warn removes with changes disabled");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            warnings[0]["message"],
            "the configuration is on vendor support with changes disabled; the project support policy `warn` permits its removal"
        );
        assert_eq!(warnings[0]["code"], "support_locked");
    }

    /// An editable vendor object with a locked child of its own — an
    /// attribute in its descriptor or a form described next to it — follows
    /// the policy of a locked object and names that child.
    #[test]
    fn vendor_object_removal_with_locked_children_follows_the_support_policy() {
        // The fixture attribute `Total` carries the uuid 2222….
        let attribute = vendor_removal_support_marker(1, true).replace(
            "}",
            ",0,0,22222222-2222-4222-8222-222222222222,22222222-2222-4222-8222-222222222222}",
        );
        let form = vendor_removal_support_marker(1, true).replace(
            "}",
            ",0,0,77777777-7777-4777-8777-777777777777,77777777-7777-4777-8777-777777777777}",
        );
        let with_form = |source: &std::path::Path| {
            std::fs::create_dir_all(source.join("Documents/First/Forms")).unwrap();
            std::fs::write(
                source.join("Documents/First/Forms/Card.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Card</Name></Properties></Form></MetaDataObject>"#,
            )
            .unwrap();
        };
        for (marker, child) in [
            (attribute.as_str(), "`main:Document.First.Attribute.Total`"),
            (form.as_str(), "`main:Document.First.Form.Card`"),
        ] {
            let (outcome, unchanged, _) = plan_vendor_removal("deny", marker, with_form);
            let error = outcome.expect_err("a locked child keeps its owner under deny");
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
            let message = error.to_string();
            assert!(message.contains(child), "{message}");
            assert!(message.contains("off-support"), "{message}");
            assert!(unchanged);

            let (outcome, unchanged, _) = plan_vendor_removal("warn", marker, with_form);
            let warnings = outcome.expect("warn removes an object with a locked child");
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert_eq!(warnings[0]["code"], "support_locked");
            assert!(
                warnings[0]["message"].as_str().unwrap().contains(child),
                "{warnings:?}"
            );
            assert!(unchanged);

            let (outcome, _, _) = plan_vendor_removal("off", marker, with_form);
            assert!(outcome.expect("off removes silently").is_empty());
        }
        // Once the object itself is off support its locked children no
        // longer keep it: this is what the refusal tells the caller to do.
        let off_support = attribute.replacen(",3,1,0,", ",3,2,0,", 1);
        assert_ne!(off_support, attribute);
        let (outcome, _, _) = plan_vendor_removal("deny", &off_support, with_form);
        assert!(outcome
            .expect("an object off support is removed")
            .is_empty());
        // Without a locked child the same form changes nothing.
        let (outcome, _, _) =
            plan_vendor_removal("deny", &vendor_removal_support_marker(1, true), with_form);
        assert!(outcome.expect("editable children do not block").is_empty());
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
            // A locked object the removed one does not contain does not keep it.
            (
                "deny",
                Some(vendor_removal_support_marker(0, true).replace(
                    "11111111-1111-4111-8111-111111111111",
                    "99999999-9999-4999-8999-999999999999",
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
    fn warn_policy_reports_support_locked_only_for_a_plan_that_changes_the_object() {
        let fixture = ApplySeamFixture::new();
        std::fs::write(
            fixture
                .source_dir()
                .parent()
                .unwrap()
                .join(".v8-project.json"),
            serde_json::json!({"editingAllowedCheck": "warn"}).to_string(),
        )
        .unwrap();
        let marker = fixture.source_dir().join("Ext/ParentConfigurations.bin");
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(marker, vendor_removal_support_marker(0, true)).unwrap();

        let edit = seam_request(
            "main:Document.First",
            "props.set",
            serde_json::json!({"values": {"Comment": "vendor edit"}}),
        );
        let (staged, effects) =
            plan_hidden_v13_apply(&edit, &fixture.binding, &fixture.admission())
                .expect("warn permits editing a locked vendor object");
        assert!(!staged.planned_changes().is_empty());
        let codes = effects
            .warnings()
            .iter()
            .map(|warning| warning["code"].clone())
            .collect::<Vec<_>>();
        assert_eq!(codes, vec![serde_json::json!("support_locked")]);
        assert_eq!(effects.warnings()[0]["at"], "main:Document.First");

        // Restating the stored comment changes nothing.
        let descriptor = fixture.source_dir().join("Documents/First.xml");
        let text = std::fs::read_to_string(&descriptor).unwrap();
        std::fs::write(
            &descriptor,
            text.replacen("<Comment></Comment>", "<Comment>kept</Comment>", 1),
        )
        .unwrap();
        let unchanged = seam_request(
            "main:Document.First",
            "props.set",
            serde_json::json!({"values": {"Comment": "kept"}}),
        );
        let (staged, effects) =
            plan_hidden_v13_apply(&unchanged, &fixture.binding, &fixture.admission())
                .expect("a no-op plan is still planned");
        assert!(staged.planned_changes().is_empty());
        assert_eq!(effects.warnings().len(), 0);
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
        // Both fixture documents carry uuid 1111…; the marker locks it. The
        // vendor marks it "changes not recommended" (`-1`), as 1C deliveries
        // do: the lock still holds (#1976).
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            "\u{feff}{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,11111111-1111-4111-8111-111111111111,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,44444444-4444-4444-8444-444444444444,44444444-4444-4444-8444-444444444444,0,-1,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111}",
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

    /// `object.remove` refuses an object other metadata still refers to
    /// before staging a byte, and names each referrer by its logical address
    /// and the property holding the reference, not by its file.
    #[test]
    fn object_removal_refuses_metadata_references_by_address_and_property() {
        use unica_workspace::test_support::tree_snapshot;

        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        std::fs::create_dir_all(source.join("Roles/Reader/Ext")).unwrap();
        std::fs::write(
            source.join("Roles/Reader/Ext/Rights.xml"),
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"><object><name>Document.First</name></object></Rights>"#,
        )
        .unwrap();
        std::fs::create_dir_all(source.join("EventSubscriptions")).unwrap();
        std::fs::write(
            source.join("EventSubscriptions/OnWrite.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><EventSubscription uuid="44444444-4444-4444-8444-444444444444"><Properties><Name>OnWrite</Name><Source><v8:Type>cfg:DocumentObject.First</v8:Type></Source><Event>OnWrite</Event></Properties></EventSubscription></MetaDataObject>"#,
        )
        .unwrap();
        let second = source.join("Documents/Second.xml");
        let text = std::fs::read_to_string(&second).unwrap().replace(
            "<Name>Total</Name>",
            "<Name>Total</Name><Type><v8:Type>cfg:DocumentRef.First</v8:Type></Type>",
        );
        std::fs::write(&second, text).unwrap();
        // A longer name that merely starts with the removed one is not a reference.
        std::fs::create_dir_all(source.join("Constants")).unwrap();
        std::fs::write(
            source.join("Constants/Other.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Constant uuid="55555555-5555-4555-8555-555555555555"><Properties><Name>Other</Name><Type><v8:Type>cfg:DocumentRef.FirstArchive</v8:Type></Type></Properties></Constant></MetaDataObject>"#,
        )
        .unwrap();
        let before = tree_snapshot(&source);
        let refused = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .unwrap_err();
        assert_eq!(refused.kind(), ApplyPlanErrorKind::InvalidState);
        let message = refused.to_string();
        for referrer in [
            "`main:Document.Second.Attribute.Total` → Type",
            "`main:EventSubscription.OnWrite` → Source",
        ] {
            assert!(message.contains(referrer), "{referrer}: {message}");
        }
        // Role rights are a composition cleaned with the object, not a reference.
        assert!(!message.contains("Role.Reader"), "{message}");
        assert!(!message.contains("Constant.Other"), "{message}");
        assert!(!message.contains(".xml"), "{message}");
        assert_eq!(tree_snapshot(&source), before);
    }

    /// Plans the removal of `Document.First` with `composition` written at
    /// `relative`, and checks that the plan rewrites exactly that file into
    /// `expected` and names the cleaned referrer.
    fn assert_removal_cleans_composition(
        relative: &str,
        composition: &str,
        expected: &str,
        referrer: &str,
        property: &str,
    ) {
        let fixture = ApplySeamFixture::new();
        let path = fixture.source_dir().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("\u{feff}{composition}")).unwrap();
        let (mut staged, effects) = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .unwrap_or_else(|error| panic!("{relative}: {error}"));
        assert_eq!(
            String::from_utf8(
                staged
                    .read(std::path::Path::new(relative))
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            format!("\u{feff}{expected}"),
            "{relative}"
        );
        let cleaned = effects
            .warnings()
            .iter()
            .find(|warning| warning["code"] == "composition_cleaned")
            .unwrap_or_else(|| panic!("{relative}: {:?}", effects.warnings()));
        assert_eq!(
            cleaned["entries"],
            serde_json::json!([{"at": referrer, "property": property}]),
            "{relative}"
        );
        let message = cleaned["message"].as_str().unwrap();
        assert!(
            message.contains(&format!("`{referrer}` → {property}")),
            "{message}"
        );
        assert!(!message.contains(".xml"), "{message}");
    }

    /// Role rights to the removed object and its children go with it; the
    /// rights to other objects stay byte for byte.
    #[test]
    fn object_removal_cleans_role_rights() {
        assert_removal_cleans_composition(
            "Roles/Reader/Ext/Rights.xml",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" version=\"2.20\">\n\t<object>\n\t\t<name>Document.First</name>\n\t\t<right><name>Read</name><value>true</value></right>\n\t</object>\n\t<object>\n\t\t<name>Document.FirstArchive</name>\n\t</object>\n\t<object>\n\t\t<name>Document.First.Attribute.Total</name>\n\t\t<right><name>View</name><value>true</value></right>\n\t</object>\n\t<object>\n\t\t<name>Document.Second</name>\n\t</object>\n</Rights>\n",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" version=\"2.20\">\n\t<object>\n\t\t<name>Document.FirstArchive</name>\n\t</object>\n\t<object>\n\t\t<name>Document.Second</name>\n\t</object>\n</Rights>\n",
            "main:Role.Reader",
            "Rights",
        );
    }

    /// A command interface loses the commands of the removed object and
    /// keeps the rest, including the visibility of other commands.
    #[test]
    fn object_removal_cleans_command_interfaces() {
        let before = "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\r\n\t<CommandsVisibility>\r\n\t\t<Command name=\"Document.First.StandardCommand.OpenList\">\r\n\t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>true</xr:Common>\r\n\t\t\t</Visibility>\r\n\t\t</Command>\r\n\t\t<Command name=\"Document.Second.StandardCommand.OpenList\">\r\n\t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>false</xr:Common>\r\n\t\t\t</Visibility>\r\n\t\t</Command>\r\n\t</CommandsVisibility>\r\n\t<CommandsOrder>\r\n\t\t<Command name=\"Document.First.StandardCommand.OpenList\">\r\n\t\t\t<CommandGroup>NavigationPanelOrdinary</CommandGroup>\r\n\t\t</Command>\r\n\t</CommandsOrder>\r\n</CommandInterface>\r\n";
        let after = "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\r\n\t<CommandsVisibility>\r\n\t\t<Command name=\"Document.Second.StandardCommand.OpenList\">\r\n\t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>false</xr:Common>\r\n\t\t\t</Visibility>\r\n\t\t</Command>\r\n\t</CommandsVisibility>\r\n\t<CommandsOrder>\r\n\t</CommandsOrder>\r\n</CommandInterface>\r\n";
        assert_removal_cleans_composition(
            "Subsystems/Sales/Ext/CommandInterface.xml",
            before,
            after,
            "main:Subsystem.Sales",
            "CommandInterface",
        );
        assert_removal_cleans_composition(
            "Ext/CommandInterface.xml",
            before,
            after,
            "main:Configuration",
            "CommandInterface",
        );
    }

    /// The command interface of a subsystem is searched like any other source
    /// once its composition is cleaned: a mention its rules do not clean
    /// refuses the removal and names the subsystem, instead of being left
    /// dangling.
    #[test]
    fn object_removal_refuses_what_a_subsystem_command_interface_keeps() {
        let fixture = ApplySeamFixture::new();
        let path = fixture
            .source_dir()
            .join("Subsystems/Sales/Subsystems/Retail/Ext/CommandInterface.xml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" version=\"2.20\"><CommandsVisibility><Command name=\"Document.First.StandardCommand.OpenList\"/></CommandsVisibility><Unknown>Document.First</Unknown></CommandInterface>",
        )
        .unwrap();
        let message = refused_removal(&fixture, "main:Document.First");
        assert!(
            message.contains("`main:Subsystem.Sales.Subsystem.Retail` → CommandInterface"),
            "a mention left in the subsystem interface refuses the removal"
        );
    }

    /// The main section command interface loses the commands of the removed object.
    #[test]
    fn object_removal_cleans_the_main_section_command_interface() {
        assert_removal_cleans_composition(
            "Ext/MainSectionCommandInterface.xml",
            "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\n\t<CommandsVisibility>\n\t\t<Command name=\"Document.Second.StandardCommand.OpenList\">\n\t\t\t<Visibility>\n\t\t\t\t<xr:Common>true</xr:Common>\n\t\t\t</Visibility>\n\t\t</Command>\n\t\t<Command name=\"Document.First.StandardCommand.OpenList\">\n\t\t\t<Visibility>\n\t\t\t\t<xr:Common>true</xr:Common>\n\t\t\t</Visibility>\n\t\t</Command>\n\t</CommandsVisibility>\n</CommandInterface>\n",
            "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\n\t<CommandsVisibility>\n\t\t<Command name=\"Document.Second.StandardCommand.OpenList\">\n\t\t\t<Visibility>\n\t\t\t\t<xr:Common>true</xr:Common>\n\t\t\t</Visibility>\n\t\t</Command>\n\t</CommandsVisibility>\n</CommandInterface>\n",
            "main:Configuration",
            "MainSectionCommandInterface",
        );
    }

    /// Registers a common module `Tools` next to the fixture documents, so it
    /// can be removed.
    fn register_common_module(source: &std::path::Path) {
        let configuration = source.join("Configuration.xml");
        let text = std::fs::read_to_string(&configuration).unwrap().replace(
            "<Document>Second</Document>",
            "<Document>Second</Document><CommonModule>Tools</CommonModule>",
        );
        std::fs::write(&configuration, text).unwrap();
        std::fs::create_dir_all(source.join("CommonModules")).unwrap();
        std::fs::write(
            source.join("CommonModules/Tools.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Tools</Name><Server>true</Server></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
    }

    /// A reference held in an attribute value counts like one in text: the
    /// characteristics of another object that take their types from the
    /// removed one, `<xr:CharacteristicTypes from="Document.First…">`, refuse
    /// the removal and name that object and property.
    #[test]
    fn object_removal_refuses_references_in_attribute_values() {
        use unica_workspace::test_support::tree_snapshot;

        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        let second = source.join("Documents/Second.xml");
        let text = std::fs::read_to_string(&second).unwrap().replace(
            "<Synonym/>",
            r#"<Synonym/><Characteristics xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><xr:Characteristic><xr:CharacteristicTypes from="Document.First.TabularSection.Kinds"/></xr:Characteristic></Characteristics>"#,
        );
        std::fs::write(&second, text).unwrap();
        let before = tree_snapshot(&source);
        let error = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{error}");
        let message = error.to_string();
        assert!(
            message.contains("`main:Document.Second` → Characteristics"),
            "{message}"
        );
        assert_eq!(tree_snapshot(&source), before);
    }

    /// The bare `Tools.` of a common module is a call only in code: a
    /// synonym starting with the module name in metadata does not block its
    /// removal, while `CommonModule.Tools` in metadata still does.
    #[test]
    fn common_module_removal_ignores_its_bare_name_in_metadata_text() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        register_common_module(&source);
        let second = source.join("Documents/Second.xml");
        let text = std::fs::read_to_string(&second)
            .unwrap()
            .replace("<Synonym/>", "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Tools. Печать</v8:content></v8:item></Synonym>");
        std::fs::write(&second, text).unwrap();
        let request = seam_request(
            "main:CommonModule.Tools",
            "object.remove",
            serde_json::json!({}),
        );
        let (staged, _) = plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission())
            .expect("a synonym is not a reference to the module");
        assert!(!staged.planned_changes().is_empty());

        std::fs::create_dir_all(source.join("EventSubscriptions")).unwrap();
        std::fs::write(
            source.join("EventSubscriptions/OnWrite.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><EventSubscription uuid="44444444-4444-4444-8444-444444444444"><Properties><Name>OnWrite</Name><Handler>CommonModule.Tools.OnWrite</Handler></Properties></EventSubscription></MetaDataObject>"#,
        )
        .unwrap();
        let error =
            plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`main:EventSubscription.OnWrite` → Handler"),
            "{error}"
        );
    }

    /// The home page loses the forms of the removed object.
    #[test]
    fn object_removal_cleans_home_page_forms() {
        assert_removal_cleans_composition(
            "Ext/HomePageWorkArea.xml",
            "<HomePageWorkArea xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" version=\"2.20\">\n\t<LeftColumn>\n\t\t<Item>\n\t\t\t<Form>Document.First.Form.Card</Form>\n\t\t\t<Height>10</Height>\n\t\t</Item>\n\t\t<Item>\n\t\t\t<Form>Document.Second.Form.Card</Form>\n\t\t\t<Height>10</Height>\n\t\t</Item>\n\t</LeftColumn>\n</HomePageWorkArea>\n",
            "<HomePageWorkArea xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" version=\"2.20\">\n\t<LeftColumn>\n\t\t<Item>\n\t\t\t<Form>Document.Second.Form.Card</Form>\n\t\t\t<Height>10</Height>\n\t\t</Item>\n\t</LeftColumn>\n</HomePageWorkArea>\n",
            "main:Configuration",
            "HomePageWorkArea",
        );
    }

    /// The content of an exchange plan loses the removed object only.
    #[test]
    fn object_removal_cleans_exchange_plan_content() {
        assert_removal_cleans_composition(
            "ExchangePlans/Sync/Ext/Content.xml",
            "<ExchangePlanContent xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" version=\"2.20\">\n\t<Item>\n\t\t<Metadata>Document.Second</Metadata>\n\t\t<AutoRecord>Allow</AutoRecord>\n\t</Item>\n\t<Item>\n\t\t<Metadata>Document.First</Metadata>\n\t\t<AutoRecord>Deny</AutoRecord>\n\t</Item>\n</ExchangePlanContent>\n",
            "<ExchangePlanContent xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" version=\"2.20\">\n\t<Item>\n\t\t<Metadata>Document.Second</Metadata>\n\t\t<AutoRecord>Allow</AutoRecord>\n\t</Item>\n</ExchangePlanContent>\n",
            "main:ExchangePlan.Sync",
            "Content",
        );
    }

    /// The content of a functional option loses the removed object and its
    /// children; its other properties and entries stay.
    #[test]
    fn object_removal_cleans_functional_option_content() {
        assert_removal_cleans_composition(
            "FunctionalOptions/UseFirst.xml",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\n\t<FunctionalOption uuid=\"88888888-8888-4888-8888-888888888888\">\n\t\t<Properties>\n\t\t\t<Name>UseFirst</Name>\n\t\t\t<Content>\n\t\t\t\t<xr:Object>Document.First</xr:Object>\n\t\t\t\t<xr:Object>Document.Second</xr:Object>\n\t\t\t\t<xr:Object>Document.First.Attribute.Total</xr:Object>\n\t\t\t</Content>\n\t\t</Properties>\n\t</FunctionalOption>\n</MetaDataObject>\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\n\t<FunctionalOption uuid=\"88888888-8888-4888-8888-888888888888\">\n\t\t<Properties>\n\t\t\t<Name>UseFirst</Name>\n\t\t\t<Content>\n\t\t\t\t<xr:Object>Document.Second</xr:Object>\n\t\t\t</Content>\n\t\t</Properties>\n\t</FunctionalOption>\n</MetaDataObject>\n",
            "main:FunctionalOption.UseFirst",
            "Content",
        );
    }

    /// References in module code — a manager call, a query text in a
    /// string, a Russian type name — do not block the removal: the plan lists
    /// each place by module address and line, and stays exact about whole names.
    #[test]
    fn object_removal_lists_code_only_references_as_a_plan_warning() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        std::fs::create_dir_all(source.join("CommonModules/Tools/Ext")).unwrap();
        std::fs::write(
            source.join("CommonModules/Tools.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="66666666-6666-4666-8666-666666666666"><Properties><Name>Tools</Name><Server>true</Server></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("CommonModules/Tools/Ext/Module.bsl"),
            "Процедура Сделать() Экспорт\n\tДокументы.FirstArchive.СоздатьДокумент();\n\tДокумент = Документы.First.СоздатьДокумент();\n\tЗапрос.Текст = \"ВЫБРАТЬ * ИЗ Документ.First\";\n\tТипы = Новый ОписаниеТипов(\"ДокументСсылка.First, ДокументСсылка.FirstArchive\");\n\tДругой = Тип(\"ДокументСсылка.FirstArchive\");\nКонецПроцедуры\n",
        )
        .unwrap();
        let (staged, effects) = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("references in module code do not block the removal");
        assert!(!staged.planned_changes().is_empty());
        assert_eq!(effects.warnings().len(), 1);
        let warning = &effects.warnings()[0];
        assert_eq!(warning["code"], "code_references");
        assert_eq!(warning["count"], 3);
        // The manager, a query text in a string and a Russian type name are
        // all code references; `FirstArchive` is another object.
        assert_eq!(
            warning["references"],
            serde_json::json!([
                {"at": "main:CommonModule.Tools", "line": 3},
                {"at": "main:CommonModule.Tools", "line": 4},
                {"at": "main:CommonModule.Tools", "line": 5}
            ])
        );
        assert_eq!(
            warning["message"],
            "module code or query text still refers to `main:Document.First`: `main:CommonModule.Tools` line 3, `main:CommonModule.Tools` line 4, `main:CommonModule.Tools` line 5; the removal leaves this text as it is"
        );
    }

    /// Lays out two list forms and a data composition template of
    /// `Document.Second` whose query texts name `Document.First` in Russian
    /// and in English; `main_table` adds the dynamic list's main table.
    fn write_query_texts(source: &std::path::Path, main_table: bool) {
        let form = |query: &str| {
            let table = if main_table {
                "<MainTable>Document.First</MainTable>"
            } else {
                ""
            };
            format!(
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\"><Attributes><Attribute name=\"List\"><Settings xsi:type=\"DynamicList\"><ManualQuery>true</ManualQuery>{table}<QueryText>{query}</QueryText></Settings></Attribute></Attributes></Form>"
            )
        };
        for (name, query) in [
            (
                "List",
                "ВЫБРАТЬ\n\tТ.Ссылка\nИЗ\n\tДокумент.First КАК Т\n\tЛЕВОЕ СОЕДИНЕНИЕ Документ.FirstArchive КАК А",
            ),
            ("ListEn", "SELECT\n\tT.Ref FROM Document.First AS T"),
        ] {
            let directory = source.join(format!("Documents/Second/Forms/{name}/Ext"));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("Form.xml"), form(query)).unwrap();
        }
        let directory = source.join("Documents/Second/Templates/Data/Ext");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("Template.xml"),
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\"><dataSet><name>Set</name><query>ВЫБРАТЬ\n\t*\nИЗ\n\tДокумент.First</query></dataSet></DataCompositionSchema>",
        )
        .unwrap();
    }

    /// A query text names a table like code does: a list form or a data
    /// composition template that selects from the removed object lets the
    /// removal plan and names the form or template and the line inside the
    /// query, in either language, leaving the text as it is.
    #[test]
    fn object_removal_lists_query_text_references_as_code() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        write_query_texts(&source, false);
        let (mut staged, effects) = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("a query text does not block the removal");
        let warning = effects
            .warnings()
            .iter()
            .find(|warning| warning["code"] == "code_references")
            .expect("the plan carries the expected warning");
        assert_eq!(
            warning["references"],
            serde_json::json!([
                {"at": "main:Document.Second.Form.List.Attribute.List", "line": 4},
                {"at": "main:Document.Second.Form.ListEn.Attribute.List", "line": 2},
                {"at": "main:Document.Second.Template.Data.DataSet.Set", "line": 4}
            ])
        );
        let message = warning["message"].as_str().unwrap();
        assert!(
            message.contains("`main:Document.Second.Template.Data.DataSet.Set` line 4"),
            "{message}"
        );
        assert!(!message.contains(".xml"), "{message}");
        for relative in [
            "Documents/Second/Forms/List/Ext/Form.xml",
            "Documents/Second/Templates/Data/Ext/Template.xml",
        ] {
            assert_eq!(
                staged
                    .read(std::path::Path::new(relative))
                    .unwrap()
                    .unwrap(),
                std::fs::read(source.join(relative)).unwrap(),
                "{relative} stays as it is"
            );
        }
    }

    /// Access restriction conditions of another role are query text: they
    /// name the removed object in either language without blocking it and
    /// stay as they are, while the role's rights to the object itself go.
    #[test]
    fn object_removal_lists_rights_conditions_as_code() {
        let fixture = ApplySeamFixture::new();
        let rights = "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" version=\"2.20\">\n\t<object>\n\t\t<name>Document.First</name>\n\t\t<right><name>Read</name><value>true</value></right>\n\t</object>\n\t<object>\n\t\t<name>Document.Second</name>\n\t\t<right>\n\t\t\t<name>Read</name>\n\t\t\t<value>true</value>\n\t\t\t<restrictionByCondition>\n\t\t\t\t<condition>ГДЕ Т.Основание ССЫЛКА Документ.First</condition>\n\t\t\t</restrictionByCondition>\n\t\t</right>\n\t</object>\n\t<object>\n\t\t<name>Document.Second.Attribute.Total</name>\n\t\t<right>\n\t\t\t<name>View</name>\n\t\t\t<value>true</value>\n\t\t\t<restrictionByCondition>\n\t\t\t\t<condition>WHERE T.Base REFS Document.First</condition>\n\t\t\t</restrictionByCondition>\n\t\t</right>\n\t</object>\n\t<restrictionTemplate>\n\t\t<name>ByBase</name>\n\t\t<condition>ГДЕ\n\t#Параметр(1) = Документ.First</condition>\n\t</restrictionTemplate>\n</Rights>\n";
        let path = fixture.source_dir().join("Roles/Other/Ext/Rights.xml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rights).unwrap();
        let (mut staged, effects) = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("a restriction condition does not block the removal");
        let code = effects
            .warnings()
            .iter()
            .find(|warning| warning["code"] == "code_references")
            .expect("the plan carries the expected warning");
        // Two conditions of different rights on the same line are two places.
        assert_eq!(
            code["references"],
            serde_json::json!([
                {"at": "main:Role.Other.Right.Document_Second", "line": 1},
                {"at": r"main:Role.Other.Right.Document_Second\.Attribute\.Total", "line": 1},
                {"at": "main:Role.Other", "line": 2}
            ])
        );
        assert_eq!(code["count"], 3);
        assert!(effects
            .warnings()
            .iter()
            .any(|warning| warning["code"] == "composition_cleaned"));
        let after = String::from_utf8(
            staged
                .read(std::path::Path::new("Roles/Other/Ext/Rights.xml"))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            after,
            rights.replace(
                "\t<object>\n\t\t<name>Document.First</name>\n\t\t<right><name>Read</name><value>true</value></right>\n\t</object>\n",
                ""
            )
        );
    }

    /// The expressions of a data composition schema are query text too:
    /// `ЗНАЧЕНИЕ(Перечисление.Kinds.Main)` in a calculated field names the
    /// enumeration without blocking its removal.
    #[test]
    fn object_removal_lists_data_composition_expressions_as_code() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        let configuration = source.join("Configuration.xml");
        let text = std::fs::read_to_string(&configuration).unwrap().replace(
            "<Document>First</Document>",
            "<Document>First</Document><Enum>Kinds</Enum>",
        );
        std::fs::write(&configuration, text).unwrap();
        std::fs::create_dir_all(source.join("Enums")).unwrap();
        std::fs::write(
            source.join("Enums/Kinds.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Enum uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Kinds</Name></Properties></Enum></MetaDataObject>"#,
        )
        .unwrap();
        let directory = source.join("Documents/Second/Templates/Data/Ext");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("Template.xml"),
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\"><dataSet><name>Set</name><field><dataPath>Контрагент.ИНН</dataPath><field>Контрагент.ИНН</field><presentationExpression>ВЫБОР КОГДА Вид = ЗНАЧЕНИЕ(Перечисление.Kinds.Main) ТОГДА 1 КОНЕЦ</presentationExpression></field></dataSet><calculatedField><dataPath>Main</dataPath><expression>ВЫБОР КОГДА Вид = ЗНАЧЕНИЕ(Перечисление.Kinds.Main) ТОГДА 1 КОНЕЦ</expression></calculatedField><totalField><dataPath>Main</dataPath><expression>Сумма(Main)</expression></totalField><parameter><name>Kind</name><expression>ЗНАЧЕНИЕ(Enum.Kinds.Main)</expression></parameter></DataCompositionSchema>",
        )
        .unwrap();
        let (_, effects) = plan_hidden_v13_apply(
            &seam_request("main:Enum.Kinds", "object.remove", serde_json::json!({})),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("an expression does not block the removal");
        let code = effects
            .warnings()
            .iter()
            .find(|warning| warning["code"] == "code_references")
            .expect("the plan carries the expected warning");
        assert_eq!(
            code["references"],
            serde_json::json!([
                {"at": r"main:Document.Second.Template.Data.DataSet.Set.Field.Контрагент\.ИНН", "line": 1},
                {"at": "main:Document.Second.Template.Data.CalculatedField.Main", "line": 1},
                {"at": "main:Document.Second.Template.Data.Parameter.Kind", "line": 1}
            ])
        );
    }

    /// Every expression of data composition is code, not only those of fields
    /// and parameters: the expressions of a data set link, the total and
    /// detail expressions of a user field in a settings variant, the
    /// expression of an output template parameter. They are listed by the
    /// template or the variant and the line, in either language.
    #[test]
    fn object_removal_lists_every_data_composition_expression_as_code() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        add_removable_object(&source, "Enum", "Enums", "Kinds");
        let directory = source.join("Documents/Second/Templates/Data/Ext");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("Template.xml"),
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:dcsat=\"http://v8.1c.ru/8.1/data-composition-system/area-template\"><dataSetLink><sourceDataSet>A</sourceDataSet><destinationDataSet>B</destinationDataSet><sourceExpression>Вид</sourceExpression><destinationExpression>ЗНАЧЕНИЕ(Enum.Kinds.Main)</destinationExpression></dataSetLink><template><name>Out</name><template xsi:type=\"dcsat:AreaTemplate\"/><parameter xsi:type=\"dcsat:ExpressionAreaTemplateParameter\"><dcsat:name>Kind</dcsat:name><dcsat:expression>Вид\n= ЗНАЧЕНИЕ(Перечисление.Kinds.Main)</dcsat:expression></parameter></template><settingsVariant><dcsset:name>Основной</dcsset:name><dcsset:settings><dcsset:userFields><dcsset:item xsi:type=\"dcsset:UserFieldExpression\"><dcsset:dataPath>ПользовательскиеПоля.Поле1</dcsset:dataPath><dcsset:detailExpression>Вид</dcsset:detailExpression><dcsset:totalExpression>Сумма(\nВЫБОР КОГДА Вид = ЗНАЧЕНИЕ(Перечисление.Kinds.Main) ТОГДА 1 КОНЕЦ)</dcsset:totalExpression></dcsset:item></dcsset:userFields></dcsset:settings></settingsVariant></DataCompositionSchema>",
        )
        .unwrap();
        let (_, effects) = plan_hidden_v13_apply(
            &seam_request("main:Enum.Kinds", "object.remove", serde_json::json!({})),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("expressions of data composition do not block the removal");
        let code = effects
            .warnings()
            .iter()
            .find(|warning| warning["code"] == "code_references")
            .expect("the plan carries the expected warning");
        assert_eq!(
            code["references"],
            serde_json::json!([
                {"at": "main:Document.Second.Template.Data", "line": 1},
                {"at": "main:Document.Second.Template.Data", "line": 2},
                {"at": "main:Document.Second.Template.Data.Setting.Основной", "line": 2}
            ])
        );
    }

    /// Only an element of a data composition namespace is an expression: an
    /// `expression` of another namespace in a template, or the `field` next to
    /// the expression inside `dcsat:fieldExpression`, is a property and
    /// refuses the removal.
    #[test]
    fn object_removal_refuses_references_next_to_data_composition_expressions() {
        for template in [
            "<Root xmlns=\"urn:example:other\"><expression>Enum.Kinds.Main</expression></Root>",
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\" xmlns:dcsat=\"http://v8.1c.ru/8.1/data-composition-system/area-template\"><dataSet><name>Set</name><field><dataPath>Kind</dataPath><orderExpression><dcscom:expression>ЗНАЧЕНИЕ(Перечисление.Kinds.Main)</dcscom:expression><dcscom:orderType>Asc</dcscom:orderType></orderExpression></field></dataSet><template><name>Out</name><parameter xsi:type=\"dcsat:DetailsAreaTemplateParameter\"><dcsat:name>Details</dcsat:name><dcsat:fieldExpression><dcsat:field>Enum.Kinds.Main</dcsat:field><dcsat:expression>Вид</dcsat:expression></dcsat:fieldExpression></parameter></template></DataCompositionSchema>",
        ] {
            let fixture = ApplySeamFixture::new();
            let source = fixture.source_dir();
            add_removable_object(&source, "Enum", "Enums", "Kinds");
            let directory = source.join("Documents/Second/Templates/Data/Ext");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("Template.xml"), template).unwrap();
            let message = refused_removal(&fixture, "main:Enum.Kinds");
            assert!(
                message.contains("`main:Document.Second.Template.Data` → Template"),
                "a reference outside an expression refuses the removal"
            );
        }
    }

    /// A calculated field of a form's dynamic list is code too: it is listed
    /// by the list attribute and the line, while a filter value of the same
    /// list still refuses the removal.
    #[test]
    fn object_removal_lists_dynamic_list_calculated_fields_as_code() {
        let form = |settings: &str| {
            format!("<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcssch=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" version=\"2.20\"><Attributes><Attribute name=\"List\"><Settings xsi:type=\"DynamicList\"><CalculatedField><dcssch:dataPath>Main</dcssch:dataPath><dcssch:expression>ВЫБОР КОГДА Вид = ЗНАЧЕНИЕ(Перечисление.Kinds.Main) ТОГДА 1 КОНЕЦ</dcssch:expression><dcssch:presentationExpression>\nВЫБОР КОГДА Вид = ЗНАЧЕНИЕ(Enum.Kinds.Main) ТОГДА \"\" КОНЕЦ</dcssch:presentationExpression></CalculatedField>{settings}</Settings></Attribute></Attributes></Form>")
        };
        let write = |fixture: &ApplySeamFixture, settings: &str| {
            let source = fixture.source_dir();
            add_removable_object(&source, "Enum", "Enums", "Kinds");
            let directory = source.join("Documents/Second/Forms/List/Ext");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("Form.xml"), form(settings)).unwrap();
        };
        let fixture = ApplySeamFixture::new();
        write(&fixture, "");
        let (_, effects) = plan_hidden_v13_apply(
            &seam_request("main:Enum.Kinds", "object.remove", serde_json::json!({})),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("a calculated field does not block the removal");
        let code = effects
            .warnings()
            .iter()
            .find(|warning| warning["code"] == "code_references")
            .expect("the plan carries the expected warning");
        assert_eq!(
            code["references"],
            serde_json::json!([
                {"at": "main:Document.Second.Form.List.Attribute.List", "line": 1},
                {"at": "main:Document.Second.Form.List.Attribute.List", "line": 2}
            ])
        );

        let fixture = ApplySeamFixture::new();
        write(
            &fixture,
            &format!(
                "<ListSettings>{}</ListSettings>",
                design_time_filter("Перечисление.Kinds.Main")
            ),
        );
        let message = refused_removal(&fixture, "main:Enum.Kinds");
        assert!(
            message.contains("`main:Document.Second.Form.List.Attribute.List` → Filter"),
            "the filter value next to the calculated field refuses the removal"
        );
    }

    /// A type the platform derives from the removed object — the record set
    /// of a register, the characteristic of a chart of characteristic types —
    /// in another object's attribute type refers to it in metadata: the
    /// removal refuses naming the attribute and the property. The same type
    /// of an object with a longer name is not a reference.
    #[test]
    fn object_removal_refuses_derived_types_in_attribute_types() {
        for (kind, directory, name, derived) in [
            (
                "InformationRegister",
                "InformationRegisters",
                "Prices",
                "InformationRegisterRecordSet",
            ),
            (
                "ChartOfCharacteristicTypes",
                "ChartsOfCharacteristicTypes",
                "Props",
                "Characteristic",
            ),
        ] {
            let fixture = ApplySeamFixture::new();
            let source = fixture.source_dir();
            add_removable_object(&source, kind, directory, name);
            let second = source.join("Documents/Second.xml");
            let text = std::fs::read_to_string(&second).unwrap().replace(
                "<Name>Total</Name>",
                &format!("<Name>Total</Name><Type><v8:Type>cfg:{derived}.{name}</v8:Type></Type>"),
            );
            std::fs::write(&second, text).unwrap();
            let message = refused_removal(&fixture, &format!("main:{kind}.{name}"));
            assert!(
                message.contains("`main:Document.Second.Attribute.Total` → Type"),
                "the derived type in the attribute type refuses the removal"
            );

            let fixture = ApplySeamFixture::new();
            let source = fixture.source_dir();
            add_removable_object(&source, kind, directory, name);
            let second = source.join("Documents/Second.xml");
            let text = std::fs::read_to_string(&second).unwrap().replace(
                "<Name>Total</Name>",
                &format!(
                    "<Name>Total</Name><Type><v8:Type>cfg:{derived}.{name}Archive</v8:Type></Type>"
                ),
            );
            std::fs::write(&second, text).unwrap();
            plan_hidden_v13_apply(
                &seam_request(
                    &format!("main:{kind}.{name}"),
                    "object.remove",
                    serde_json::json!({}),
                ),
                &fixture.binding,
                &fixture.admission(),
            )
            .expect("the derived type of an object with a longer name is not a reference");
        }
    }

    /// Module code names derived types in Russian too:
    /// `РегистрСведенийНаборЗаписей.Prices` and `Характеристика.Props` are
    /// code references, the same types of `PricesArchive` are not.
    #[test]
    fn object_removal_lists_russian_derived_types_in_code() {
        for (kind, directory, name, line) in [
            (
                "InformationRegister",
                "InformationRegisters",
                "Prices",
                "\tНабор = Новый ОписаниеТипов(\"РегистрСведенийНаборЗаписей.Prices\");",
            ),
            (
                "ChartOfCharacteristicTypes",
                "ChartsOfCharacteristicTypes",
                "Props",
                "\t// Значение - Характеристика.Props - значение свойства.",
            ),
        ] {
            let fixture = ApplySeamFixture::new();
            let source = fixture.source_dir();
            add_removable_object(&source, kind, directory, name);
            std::fs::create_dir_all(source.join("CommonModules/Tools/Ext")).unwrap();
            std::fs::write(
                source.join("CommonModules/Tools.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="66666666-6666-4666-8666-666666666666"><Properties><Name>Tools</Name><Server>true</Server></Properties></CommonModule></MetaDataObject>"#,
            )
            .unwrap();
            std::fs::write(
                source.join("CommonModules/Tools/Ext/Module.bsl"),
                format!(
                    "Процедура Сделать() Экспорт\n{}\n{line}\nКонецПроцедуры\n",
                    line.replace(&format!(".{name}"), &format!(".{name}Archive"))
                ),
            )
            .unwrap();
            let (_, effects) = plan_hidden_v13_apply(
                &seam_request(
                    &format!("main:{kind}.{name}"),
                    "object.remove",
                    serde_json::json!({}),
                ),
                &fixture.binding,
                &fixture.admission(),
            )
            .expect("references in module code do not block the removal");
            let code = effects
                .warnings()
                .iter()
                .find(|warning| warning["code"] == "code_references")
                .expect("the plan carries the expected warning");
            assert_eq!(
                code["references"],
                serde_json::json!([{"at": "main:CommonModule.Tools", "line": 3}])
            );
        }
    }

    /// Next to an expression, a reference held in a property of the schema —
    /// the value type of a parameter — still refuses the removal.
    #[test]
    fn object_removal_refuses_a_parameter_type_next_to_an_expression() {
        let fixture = ApplySeamFixture::new();
        let directory = fixture
            .source_dir()
            .join("Documents/Second/Templates/Data/Ext");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("Template.xml"),
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><parameter><name>Base</name><valueType><v8:Type>cfg:DocumentRef.First</v8:Type></valueType><expression>ЗНАЧЕНИЕ(Документ.First.ПустаяСсылка)</expression></parameter></DataCompositionSchema>",
        )
        .unwrap();
        let error = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{error}");
        assert!(
            error
                .to_string()
                .contains("`main:Document.Second.Template.Data` → Template"),
            "{error}"
        );
    }

    /// Next to a query text a reference held in a property still refuses:
    /// the dynamic list's main table is metadata, not code.
    #[test]
    fn object_removal_refuses_a_main_table_next_to_a_query_text() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        write_query_texts(&source, true);
        let error = plan_hidden_v13_apply(
            &seam_request(
                "main:Document.First",
                "object.remove",
                serde_json::json!({}),
            ),
            &fixture.binding,
            &fixture.admission(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{error}");
        let message = error.to_string();
        assert!(
            message.contains("`main:Document.Second.Form.List` → Form"),
            "{message}"
        );
        assert!(!message.contains("Template.Data"), "{message}");
    }

    /// Registers `<Kind>` `name` in the fixture configuration and writes its
    /// descriptor, so that the object can be removed.
    fn add_removable_object(source: &std::path::Path, kind: &str, directory: &str, name: &str) {
        let configuration = source.join("Configuration.xml");
        let text = std::fs::read_to_string(&configuration).unwrap().replace(
            "<Document>First</Document>",
            &format!("<Document>First</Document><{kind}>{name}</{kind}>"),
        );
        std::fs::write(&configuration, text).unwrap();
        std::fs::create_dir_all(source.join(directory)).unwrap();
        std::fs::write(
            source.join(format!("{directory}/{name}.xml")),
            format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><{kind} uuid=\"cccccccc-cccc-4ccc-8ccc-cccccccccccc\"><Properties><Name>{name}</Name></Properties></{kind}></MetaDataObject>"),
        )
        .unwrap();
    }

    /// A data composition filter whose right value is `dcscor:DesignTimeValue`.
    fn design_time_filter(value: &str) -> String {
        format!("<dcsset:filter><dcsset:item xsi:type=\"dcsset:FilterItemComparison\"><dcsset:left xsi:type=\"dcscor:Field\">Kind</dcsset:left><dcsset:comparisonType>Equal</dcsset:comparisonType><dcsset:right xsi:type=\"dcscor:DesignTimeValue\">{value}</dcsset:right></dcsset:item></dcsset:filter>")
    }

    /// Plans the removal of `at` and returns the refusal text.
    fn refused_removal(fixture: &ApplySeamFixture, at: &str) -> String {
        let error = plan_hidden_v13_apply(
            &seam_request(at, "object.remove", serde_json::json!({})),
            &fixture.binding,
            &fixture.admission(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        error.to_string()
    }

    /// A dynamic list of another object's form filtering by a predefined
    /// item refers to the catalog in metadata: 8.3.27 writes the filter value
    /// in Russian, `Справочник.Units.Piece`, and the removal refuses naming
    /// the form attribute and the filter.
    #[test]
    fn object_removal_refuses_a_russian_filter_value_in_list_settings() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        add_removable_object(&source, "Catalog", "Catalogs", "Units");
        let directory = source.join("Documents/Second/Forms/List/Ext");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("Form.xml"),
            format!(
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" version=\"2.20\"><Attributes><Attribute name=\"List\"><Settings xsi:type=\"DynamicList\"><ListSettings>{}</ListSettings></Settings></Attribute></Attributes></Form>",
                design_time_filter("Справочник.Units.Piece")
            ),
        )
        .unwrap();
        let message = refused_removal(&fixture, "main:Catalog.Units");
        assert!(
            message.contains("`main:Document.Second.Form.List.Attribute.List` → Filter"),
            "the refusal names the form attribute and the filter"
        );
        assert!(
            !message.contains("→ Form"),
            "the filter is not reported as the whole form"
        );
    }

    /// A settings variant of a data composition template filtering by an
    /// enum value refers to the enumeration, written in Russian or in English:
    /// the refusal names the variant or the parameter and the property.
    #[test]
    fn object_removal_refuses_filter_values_in_a_settings_variant() {
        for (value, referrer) in [
            (
                "Перечисление.Kinds.Main",
                "`main:Document.Second.Template.Data.Setting.Основной` → Filter",
            ),
            (
                "Enum.Kinds.Main",
                "`main:Document.Second.Template.Data.Setting.Основной` → Filter",
            ),
        ] {
            let fixture = ApplySeamFixture::new();
            let source = fixture.source_dir();
            add_removable_object(&source, "Enum", "Enums", "Kinds");
            let directory = source.join("Documents/Second/Templates/Data/Ext");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join("Template.xml"),
                format!(
                    "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\"><parameter><name>Kind</name><value xsi:type=\"dcscor:DesignTimeValue\">Перечисление.Kinds.Other</value></parameter><settingsVariant><dcsset:name>Основной</dcsset:name><dcsset:settings>{}</dcsset:settings></settingsVariant></DataCompositionSchema>",
                    design_time_filter(value)
                ),
            )
            .unwrap();
            let message = refused_removal(&fixture, "main:Enum.Kinds");
            for referrer in [
                referrer,
                "`main:Document.Second.Template.Data.Parameter.Kind` → Value",
            ] {
                assert!(
                    message.contains(referrer),
                    "the refusal names the variant filter and the parameter value"
                );
            }
        }
    }

    /// The conditional appearance of a whole form — `ConditionalAppearance`
    /// outside any dynamic list — filtering by a predefined item refers to the
    /// catalog: the refusal names the form and the conditional appearance.
    #[test]
    fn object_removal_refuses_a_filter_value_in_form_conditional_appearance() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        add_removable_object(&source, "Catalog", "Catalogs", "Units");
        let directory = source.join("Documents/Second/Forms/List/Ext");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("Form.xml"),
            format!(
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" version=\"2.20\"><Attributes><ConditionalAppearance><dcsset:item>{}</dcsset:item></ConditionalAppearance></Attributes></Form>",
                design_time_filter("Справочник.Units.Piece")
            ),
        )
        .unwrap();
        let message = refused_removal(&fixture, "main:Catalog.Units");
        assert!(
            message.contains("`main:Document.Second.Form.List` → ConditionalAppearance"),
            "the refusal names the form and its conditional appearance"
        );
    }

    /// The address grammar has no node for a nested schema: a design-time
    /// value or a query inside one is addressed by the template, not by a
    /// parameter, variant or data set of the outer schema.
    #[test]
    fn object_removal_addresses_a_nested_schema_by_its_template() {
        let template = |body: &str| {
            format!("<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\"><nestedSchema><name>Inner</name><schema>{body}</schema></nestedSchema></DataCompositionSchema>")
        };
        let write = |fixture: &ApplySeamFixture, body: &str| {
            let source = fixture.source_dir();
            add_removable_object(&source, "Enum", "Enums", "Kinds");
            let directory = source.join("Documents/Second/Templates/Data/Ext");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("Template.xml"), template(body)).unwrap();
        };
        let fixture = ApplySeamFixture::new();
        write(
            &fixture,
            &format!(
                "<parameter><name>Kind</name><value xsi:type=\"dcscor:DesignTimeValue\">Перечисление.Kinds.Main</value></parameter><settingsVariant><dcsset:name>Основной</dcsset:name><dcsset:settings>{}</dcsset:settings></settingsVariant>",
                design_time_filter("Перечисление.Kinds.Main")
            ),
        );
        let message = refused_removal(&fixture, "main:Enum.Kinds");
        for referrer in [
            "`main:Document.Second.Template.Data` → Value",
            "`main:Document.Second.Template.Data` → Filter",
        ] {
            assert!(
                message.contains(referrer),
                "the refusal names the template holding the nested schema"
            );
        }
        assert!(
            !message.contains("Parameter.Kind") && !message.contains("Setting."),
            "a nested node is not addressed as a node of the outer schema"
        );

        let fixture = ApplySeamFixture::new();
        write(
            &fixture,
            "<dataSet><name>Set</name><query>ВЫБРАТЬ * ИЗ Перечисление.Kinds</query></dataSet>",
        );
        let (_, effects) = plan_hidden_v13_apply(
            &seam_request("main:Enum.Kinds", "object.remove", serde_json::json!({})),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("a query in a nested schema does not block the removal");
        let code = effects
            .warnings()
            .iter()
            .find(|warning| warning["code"] == "code_references")
            .expect("the plan lists the query of the nested schema");
        assert_eq!(
            code["references"],
            serde_json::json!([{"at": "main:Document.Second.Template.Data", "line": 1}])
        );
    }

    /// Russian kind names are searched only in design-time values: a synonym
    /// or a title reading «Справочник.Units» is text, not a reference.
    #[test]
    fn object_removal_ignores_russian_kind_names_in_synonyms() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        add_removable_object(&source, "Catalog", "Catalogs", "Units");
        let second = source.join("Documents/Second.xml");
        let text = std::fs::read_to_string(&second).unwrap().replace(
            "<Name>Total</Name>",
            "<Name>Total</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Справочник.Units</v8:content></v8:item></Synonym>",
        );
        std::fs::write(&second, text).unwrap();
        let directory = source.join("Documents/Second/Forms/List/Ext");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("Form.xml"),
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\"><Title><v8:item><v8:lang>ru</v8:lang><v8:content>Справочник.Units.Piece</v8:content></v8:item></Title></Form>",
        )
        .unwrap();
        plan_hidden_v13_apply(
            &seam_request("main:Catalog.Units", "object.remove", serde_json::json!({})),
            &fixture.binding,
            &fixture.admission(),
        )
        .expect("a synonym naming the kind in Russian does not block the removal");
    }

    /// `force` is no longer part of the `object.remove` contract: passing it
    /// refuses `bad_value` and explains that the references go first.
    #[test]
    fn object_removal_refuses_force_as_bad_value() {
        let fixture = ApplySeamFixture::new();
        for force in [
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!("yes"),
        ] {
            let error = plan_hidden_v13_apply(
                &seam_request(
                    "main:Document.First",
                    "object.remove",
                    serde_json::json!({"force": force}),
                ),
                &fixture.binding,
                &fixture.admission(),
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{force}");
            assert_eq!(error.path(), Some("ops[0].args.force"));
            assert!(
                error.to_string().contains("remove the references"),
                "{error}"
            );
        }
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
