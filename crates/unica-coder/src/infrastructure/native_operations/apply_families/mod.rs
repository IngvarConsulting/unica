use crate::domain::apply::SourceSetWriteSupport;

mod borrow;
pub(crate) mod dcs_mxl;
pub(crate) mod form_resource;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod metadata;

use crate::domain::apply::ApplyRequest;
use unica_operations::OperationDescriptor;
use unica_operations::OperationFamily;
use unica_source_contract::proposal::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind, PlannedApplyEffects,
    ReadOnlyMode, SupportLock, SupportLockKind,
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
/// The compatibility mode the request sets when all it does is set the
/// set's own mode property on its root (`props.set` at `Configuration`).
fn raised_compatibility_mode(
    request: &ApplyRequest,
    source_kind: SourceSetKind,
) -> Option<unica_source_contract::compatibility::CompatibilityMode> {
    let property = unica_source_designer::extension_mode::compatibility_mode_property(source_kind)?;
    let mut raised = None;
    for operation in request.ops() {
        let at_root = operation.at().segments().len() == 1
            && operation.at().segments()[0].kind()
                == unica_address::address::NodeKind::Configuration;
        let values = operation.args().get("values")?.as_object()?;
        if operation.name() != "props.set" || !at_root || values.len() != 1 {
            return None;
        }
        let mode = unica_source_contract::compatibility::CompatibilityMode::parse(
            values.get(property)?.as_str()?,
        )?;
        raised = Some(mode);
    }
    raised
}

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
            crate::infrastructure::format_guard::classify_staged_platform_xml_root_in_set(
                &relative,
                &bytes,
                binding.source_profile().serialization_format(),
            )
        {
            let set_format = binding.source_profile().serialization_format();
            // An older or mixed export the person may re-export in a writable
            // format (INV.SOURCE.WRITABLE-PROFILE); other findings have no way out.
            let reexport = match (finding.code, finding.actual.clone(), set_format) {
                ("formatMigrationAvailable", Some(actual), _) => {
                    Some(unica_source_contract::proposal::FormatReexportKind::Old { actual })
                }
                ("formatVersionMixed", Some(other), Some(set_format)) => {
                    Some(unica_source_contract::proposal::FormatReexportKind::Mixed {
                        set_format: set_format.to_string(),
                        other,
                    })
                }
                _ => None,
            };
            let error =
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message).at_path(at);
            return Err(match reexport {
                Some(kind) => {
                    error.with_format_reexport(unica_source_contract::proposal::FormatReexport {
                        source_set: binding.source_set_name().to_string(),
                        kind,
                    })
                }
                None => error,
            });
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
/// is silent. Removal follows the same policy: an editable vendor object, or
/// a node of it with its own record, is removed while staying on support, and
/// only a `locked` rule on it, above it or on a node under it with its own
/// record refuses under `deny` or warns under `warn`. The marker is not
/// changed by the removal.
/// The owner is the top-level metadata object of the addressed node, or the
/// configuration root for root-level targets. A node below it with its own
/// record (a form, an attribute, a nested subsystem) closes itself and
/// everything under it when its rule is `locked`, as in the configurator.
/// Every operation target is checked, not only the request's node.
/// What locks a target is decided by [`support_gate_facts`], which `view`
/// also asks, so the editability a node shows is this gate's answer.
fn refuse_targets_on_locked_vendor_objects(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    staged: &mut ApplyStagedState,
) -> Result<Vec<GateWarning>, ApplyPlanError> {
    let policy = admission.support_policy_mode();
    let mut warnings = Vec::new();
    for (target, removes) in gate_targets(request, binding) {
        warnings.extend(refuse_target_on_locked_vendor_object(
            target, removes, policy, binding, staged,
        )?);
    }
    Ok(warnings)
}

/// Every lock of the gate on the targets of `request`, not only the first:
/// the locks a person may be asked about in the current support state.
/// Another refusal of the gate (support that cannot be read) is returned.
fn locks_on_targets(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    staged: &mut ApplyStagedState,
) -> Result<Vec<SupportLock>, ApplyPlanError> {
    let policy = admission.support_policy_mode();
    let mut locks = Vec::new();
    for (target, removes) in gate_targets(request, binding) {
        match refuse_target_on_locked_vendor_object(target, removes, policy, binding, staged) {
            Ok(_) => {}
            Err(error) => match error.support_lock() {
                Some(lock) => locks.push(lock.clone()),
                None => return Err(error),
            },
        }
    }
    Ok(locks)
}

/// The targets the gate checks: every operation answers for its own target,
/// since an `args.at` below the request's node may sit under a node with its
/// own `locked` record; with whether an operation removes it.
fn gate_targets<'a>(
    request: &'a ApplyRequest,
    binding: &ProviderRootBinding,
) -> Vec<(&'a unica_address::address::QualifiedAddress, bool)> {
    let mut targets: Vec<(&unica_address::address::QualifiedAddress, bool)> = Vec::new();
    for operation in request.ops() {
        if changes_support_state(operation.name(), Some(operation.at()), binding) {
            continue;
        }
        let removes = removes_support_node(operation.name(), operation.at());
        match targets.iter_mut().find(|(at, _)| *at == operation.at()) {
            Some((_, target_removes)) => *target_removes |= removes,
            None => targets.push((operation.at(), removes)),
        }
    }
    targets
}

/// Whether the operation `name` removes the node it addresses and that node
/// can carry its own support record: the object itself (`object.remove`) or
/// a node below it (`tabularSection.remove`, `form.remove`,
/// `childSubsystem.remove`…). Such a removal answers for the locked nodes
/// under the removed one, not only for the path to it.
pub(crate) fn removes_support_node(
    name: &str,
    at: &unica_address::address::QualifiedAddress,
) -> bool {
    use unica_metadata::apply_operations::MetadataOperation;
    use unica_operation_contract::OperationSkeleton;

    if MetadataOperation::named(name) == Some(MetadataOperation::ObjectRemove) {
        return true;
    }
    name.ends_with(".remove")
        && unica_metadata::applicability::support_record_path(at.segments())
        && crate::domain::apply::operation_registry()
            .named(name)
            .any(|descriptor| descriptor.skeleton() == OperationSkeleton::Target)
}

/// A `support_locked` warning of the gate under `warn`, with the source
/// paths it speaks for.
struct GateWarning {
    warning: serde_json::Value,
    /// The paths of a locked node below its owner: the warning stands only
    /// when the plan changes one of them. `None`: any change of the plan.
    scope: Option<Vec<std::path::PathBuf>>,
}

/// The gate's warnings that the plan's changes bear out, one per node: a
/// locked node whose files the plan leaves as they were edited nothing
/// protected.
fn planned_gate_warnings(
    warnings: Vec<GateWarning>,
    changed: &[unica_source_contract::proposal::StagedApplyChange],
) -> Vec<serde_json::Value> {
    let mut kept: Vec<serde_json::Value> = Vec::new();
    for GateWarning { warning, scope } in warnings {
        let touched = match &scope {
            None => !changed.is_empty(),
            Some(scope) => changed.iter().any(|change| {
                scope
                    .iter()
                    .any(|path| change.relative_path.starts_with(path))
            }),
        };
        if touched && !kept.iter().any(|seen| seen["at"] == warning["at"]) {
            kept.push(warning);
        }
    }
    kept
}

/// The support gate for one operation target; `removes` asks about removing
/// the target itself (`removes_support_node`).
fn refuse_target_on_locked_vendor_object(
    target: &unica_address::address::QualifiedAddress,
    removes: bool,
    policy: unica_source_contract::proposal::SupportPolicyMode,
    binding: &ProviderRootBinding,
    staged: &mut ApplyStagedState,
) -> Result<Option<GateWarning>, ApplyPlanError> {
    use unica_source_contract::proposal::SupportPolicyMode;

    // `off` silences locks but not a support state that cannot be read: the
    // gate reads the marker under every policy.
    let deny = policy == SupportPolicyMode::Deny;
    let support_locked = |owner: &str, message: String| {
        serde_json::json!({
            "code": "support_locked",
            "severity": "warning",
            "at": format!("{}:{owner}", target.source_set()),
            "message": message,
        })
    };
    let at = target.to_string();
    let source_set = target.source_set();
    let staged = std::cell::RefCell::new(staged);
    let facts = support_gate_facts(
        target,
        binding.source_kind(),
        // `off` asks only whether the marker can be read: no child listing.
        removes && policy != SupportPolicyMode::Off,
        &mut |relative| staged.borrow_mut().read(relative),
        &mut |relative| {
            staged
                .borrow_mut()
                .enumerate_tree_with_root_absence(relative)
                .map(|(files, _)| files)
        },
    )
    .map_err(|error| ApplyPlanError::staging(error, at.clone()))?;
    // Support that cannot be read is not the absence of support
    // (INV.APP.SUPPORT-STATE): `deny` and `off` refuse before any byte is
    // written, `warn` permits the edit with a warning.
    if let Some(unreadable) = facts.unreadable {
        let reason = unreadable.reason;
        if policy == SupportPolicyMode::Warn {
            let owner = support_owner_relative(target, binding.source_kind())
                .map_or_else(|| "Configuration".to_string(), |(owner, _)| owner);
            return Ok(Some(GateWarning {
                warning: support_locked(
                    &owner,
                    format!("{reason}; the project support policy `warn` permits this edit"),
                ),
                scope: None,
            }));
        }
        if !unreadable.marker {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!(
                    "{reason}; restore the descriptor from version control or export the set again"
                ),
            )
            .at_path(at));
        }
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("{reason}; restore the infobase from a backup and export the set again, or take the configuration off support entirely"),
        )
        .at_path(at)
        .with_support_state_unreadable());
    }
    if policy == SupportPolicyMode::Off {
        return Ok(None);
    }
    let Some(lock) = facts.lock else {
        return Ok(None);
    };
    let support_lock = unica_source_contract::proposal::SupportLock {
        kind: lock.kind,
        source_set: source_set.to_string(),
        owner: lock.owner.clone(),
    };
    let message = match (lock.kind, deny) {
        (SupportLockKind::ConfigurationLocked, false) => {
            let permits = if removes { "its removal" } else { "this edit" };
            format!("{}; the project support policy `warn` permits {permits}", lock.reason)
        }
        (SupportLockKind::ConfigurationLocked, true) => {
            let before = if removes { "removal" } else { "ordinary edits" };
            format!(
                "{}; enable editing with `supportCapability.set` in a separate apply before {before}",
                lock.reason
            )
        }
        (SupportLockKind::RemovalLocked, false) => format!(
            "{}; the project support policy `warn` permits its removal",
            lock.reason
        ),
        (SupportLockKind::RemovalLocked, true) => format!(
            "{}; take the object off support with `supportRule.set` and the rule `off-support` in a separate apply before removal",
            lock.reason
        ),
        (_, false) => format!(
            "{}; the project support policy `warn` permits this edit",
            lock.reason
        ),
        (_, true) => format!(
            "{}; allow editing with `supportRule.set` in a separate apply before ordinary edits",
            lock.reason
        ),
    };
    if !deny {
        return Ok(Some(GateWarning {
            warning: support_locked(&lock.owner, message),
            scope: lock.scope,
        }));
    }
    Err(
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message)
            .at_path(at)
            .with_support_lock(support_lock),
    )
}

/// Reads one source file relative to the source root: `None` when absent.
pub(crate) type GateRead<'a, E> = dyn FnMut(&std::path::Path) -> Result<Option<Vec<u8>>, E> + 'a;
/// Lists the regular files under a relative directory, recursively.
pub(crate) type GateEnumerate<'a, E> =
    dyn FnMut(&std::path::Path) -> Result<Vec<std::path::PathBuf>, E> + 'a;

/// What the vendor-support marker says about one target before the project
/// policy is applied: the lock that stands in the way of an edit (or of a
/// removal), and the rule of the target's owner.
pub(crate) struct SupportGateFacts {
    /// The parsed marker, or `None` when the set has none or the
    /// compatible reader cannot read it (the gate then does not lock).
    pub(crate) marker: Option<SupportGateMarker>,
    /// The rule of the owner's own record: 0 `locked`, 1 editable with
    /// support, 2 taken off support; `None` without a record.
    pub(crate) owner_rule: Option<u8>,
    /// The owner's descriptor was read but names no uuid: its rule cannot
    /// be looked up, and its support is unreadable.
    pub(crate) owner_uuid_missing: bool,
    /// The rule of the addressed node's own record when the node is not the
    /// owner itself and has one (a form, an attribute, a nested subsystem).
    pub(crate) node_rule: Option<u8>,
    pub(crate) lock: Option<SupportGateLock>,
    /// Why the support of the target cannot be read. Not the absence of
    /// support (INV.APP.SUPPORT-STATE).
    pub(crate) unreadable: Option<SupportUnreadable>,
}

/// Support that cannot be read: the marker itself, or the owner's
/// descriptor that names no uuid to look its rule up.
pub(crate) struct SupportUnreadable {
    pub(crate) reason: String,
    /// The marker is damaged: taking the configuration off support entirely
    /// repairs it. A descriptor without a uuid is repaired by restoring it.
    pub(crate) marker: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportGateMarker {
    /// The configuration was taken off support as a whole.
    Removed,
    /// On support; whether its changes are enabled.
    Supported { editing_enabled: bool },
}

pub(crate) struct SupportGateLock {
    pub(crate) kind: SupportLockKind,
    /// The owner path as the lock names it (`Catalog.Items`, `Configuration`).
    pub(crate) owner: String,
    pub(crate) reason: String,
    /// The source paths of a locked node below the owner: its own descriptor
    /// and directory, or the descriptor that declares it. `None` for a lock of
    /// the owner or of the configuration.
    pub(crate) scope: Option<Vec<std::path::PathBuf>>,
}

/// The support facts of one target, read through `read` and `enumerate`
/// relative to the source root: the staged state of `apply` or the retained
/// root of `view`. `removes` asks about removing the target itself.
pub(crate) fn support_gate_facts<E>(
    at: &unica_address::address::QualifiedAddress,
    source_kind: SourceSetKind,
    removes: bool,
    read: &mut GateRead<'_, E>,
    enumerate: &mut GateEnumerate<'_, E>,
) -> Result<SupportGateFacts, E> {
    use unica_metadata_designer::support::{
        parse_support_state_strict_bytes, support_root_uuid_from_bytes,
    };

    let free = |marker, owner_rule| SupportGateFacts {
        marker,
        owner_rule,
        owner_uuid_missing: false,
        node_rule: None,
        lock: None,
        unreadable: None,
    };
    let source_set = at.source_set();
    let marker = read(std::path::Path::new("Ext/ParentConfigurations.bin"))?;
    let state = match parse_support_state_strict_bytes(marker.as_deref()) {
        Ok(Some(state)) => state,
        Ok(None) => return Ok(free(None, None)),
        Err(error) => {
            return Ok(SupportGateFacts {
                marker: None,
                owner_rule: None,
                owner_uuid_missing: false,
                node_rule: None,
                lock: None,
                unreadable: Some(SupportUnreadable {
                    reason: format!("the vendor support state `Ext/ParentConfigurations.bin` cannot be read ({error}); Unica does not take an unreadable support state for no support"),
                    marker: true,
                }),
            })
        }
    };
    if state.removed() {
        return Ok(free(Some(SupportGateMarker::Removed), None));
    }
    let editing_enabled = state.global_editing_enabled();
    let marker = Some(SupportGateMarker::Supported { editing_enabled });
    let owner = support_owner_relative(at, source_kind);
    if !editing_enabled {
        // The node still names its own rule, as `view` shows it.
        let mut node_rule = None;
        if let Some((owner, owner_relative)) = &owner {
            if let Some(bytes) = read(owner_relative)? {
                node_rule =
                    support_records_on_path(at, owner, owner_relative, &bytes, &state, read)?
                        .node_rule;
            }
        }
        let owner = owner.map_or_else(|| "Configuration".to_string(), |(owner, _)| owner);
        return Ok(SupportGateFacts {
            marker,
            owner_rule: None,
            owner_uuid_missing: false,
            node_rule,
            lock: Some(SupportGateLock {
                kind: SupportLockKind::ConfigurationLocked,
                owner,
                reason: "the configuration is on vendor support with changes disabled".to_string(),
                scope: None,
            }),
            unreadable: None,
        });
    }
    let Some((owner, owner_relative)) = owner else {
        return Ok(free(marker, None));
    };
    let Some(bytes) = read(&owner_relative)? else {
        return Ok(free(marker, None));
    };
    let owner_uuid = support_root_uuid_from_bytes(&bytes);
    let owner_uuid_missing = owner_uuid.is_none();
    if owner_uuid_missing {
        // An owner whose descriptor names no uuid has a rule nobody can look
        // up: its support cannot be read (INV.APP.SUPPORT-STATE).
        return Ok(SupportGateFacts {
            marker,
            owner_rule: None,
            owner_uuid_missing,
            node_rule: None,
            lock: None,
            unreadable: Some(SupportUnreadable {
                reason: format!("the descriptor of `{source_set}:{owner}` names no uuid, so its vendor support rule cannot be read"),
                marker: false,
            }),
        });
    }
    let support_rule = owner_uuid.and_then(|uuid| state.object_rule(&uuid));
    let free = |marker, owner_rule| SupportGateFacts {
        marker,
        owner_rule,
        owner_uuid_missing,
        node_rule: None,
        lock: None,
        unreadable: None,
    };
    let locked = |kind, reason: String| SupportGateFacts {
        marker,
        owner_rule: support_rule,
        owner_uuid_missing,
        node_rule: None,
        lock: Some(SupportGateLock {
            kind,
            owner: owner.clone(),
            reason,
            scope: None,
        }),
        unreadable: None,
    };
    // The removal of a node below the owner edits the nodes above it and
    // answers for the nodes under it.
    let removes_owner = removes && at.segments().len() == 1;
    if removes_owner {
        // An editable vendor object is removed without leaving support, as
        // the configurator does; only a lock on it or on one of its own
        // child objects stands in the way.
        // An object taken off support no longer answers for its children.
        if support_rule == Some(0) {
            return Ok(locked(
                SupportLockKind::RemovalLocked,
                format!("`{source_set}:{owner}` is a vendor object with the support rule `locked`"),
            ));
        }
        if support_rule != Some(1) {
            return Ok(free(marker, support_rule));
        }
        let children = locked_vendor_descendants(
            &state,
            source_set,
            DescendantRoot {
                base: &owner,
                prefix: "",
                descriptor: &bytes,
                directory: Some(&owner_relative.with_extension("")),
            },
            read,
            enumerate,
        )?;
        if children.is_empty() {
            return Ok(free(marker, support_rule));
        }
        return Ok(locked(
            SupportLockKind::RemovalLocked,
            format!(
                "`{source_set}:{owner}` has vendor child objects with the support rule `locked`: {}",
                children.join(", ")
            ),
        ));
    }
    // A node under the owner with its own record answers for itself too:
    // the first `locked` record on the path closes the target.
    let path = support_records_on_path(at, &owner, &owner_relative, &bytes, &state, read)?;
    if support_rule == Some(0) {
        let mut facts = locked(
            SupportLockKind::ObjectLocked,
            format!("`{source_set}:{owner}` is a vendor object with the support rule `locked`"),
        );
        facts.node_rule = path.node_rule;
        return Ok(facts);
    }
    let reached = path.reached.as_ref().filter(|_| removes);
    if let Some((node, scope)) = path.locked {
        // The removed node's own `locked` rule refuses its removal as the
        // object's own rule does; a lock above it refuses the edit.
        if reached.is_some_and(|reached| reached.node == node) {
            let mut facts = locked(
                SupportLockKind::RemovalLocked,
                format!("`{source_set}:{node}` is a vendor object with the support rule `locked`"),
            );
            facts.node_rule = path.node_rule;
            if let Some(lock) = facts.lock.as_mut() {
                lock.owner = node;
            }
            return Ok(facts);
        }
        return Ok(SupportGateFacts {
            marker,
            owner_rule: support_rule,
            owner_uuid_missing,
            node_rule: path.node_rule,
            lock: Some(SupportGateLock {
                kind: SupportLockKind::ObjectLocked,
                reason: format!(
                    "`{source_set}:{node}` is a vendor object with the support rule `locked`"
                ),
                owner: node,
                scope: Some(scope),
            }),
            unreadable: None,
        });
    }
    // A removed node editable with support answers for the nodes under it
    // with their own `locked` rule; one taken off support no longer does.
    if let Some(reached) = reached.filter(|_| path.node_rule == Some(1)) {
        let children = locked_vendor_descendants(
            &state,
            source_set,
            DescendantRoot {
                base: &reached.base,
                prefix: &reached.prefix,
                descriptor: &reached.descriptor,
                directory: reached.directory.as_deref(),
            },
            read,
            enumerate,
        )?;
        if !children.is_empty() {
            let mut facts = locked(
                SupportLockKind::RemovalLocked,
                format!(
                    "`{source_set}:{}` has vendor child objects with the support rule `locked`: {}",
                    reached.node,
                    children.join(", ")
                ),
            );
            facts.node_rule = path.node_rule;
            if let Some(lock) = facts.lock.as_mut() {
                lock.owner = reached.node.clone();
            }
            return Ok(facts);
        }
    }
    let mut facts = free(marker, support_rule);
    facts.node_rule = path.node_rule;
    Ok(facts)
}

/// The own support records on the path from the owner to `at`.
struct PathRecords {
    /// The first node (owner excluded) whose own record is `locked`, with
    /// the source paths that hold it.
    locked: Option<(String, Vec<std::path::PathBuf>)>,
    /// The rule of `at`'s own record when `at` is below the owner.
    node_rule: Option<u8>,
    /// `at` itself when the walk reaches it below the owner: where the nodes
    /// under it are described.
    reached: Option<ReachedNode>,
}

/// A node below the owner that the walk reached, and the descriptor that
/// declares the nodes under it.
struct ReachedNode {
    /// The node path without the source set (`Catalog.X.TabularSection.T`).
    node: String,
    /// The object `descriptor` describes.
    base: String,
    /// The node's own `.Kind.Name…` chain in `descriptor`; empty when the
    /// descriptor is the node's own.
    prefix: String,
    descriptor: Vec<u8>,
    /// The node's own directory with nested descriptors (a form, a nested
    /// subsystem); `None` for a node declared in another descriptor.
    directory: Option<std::path::PathBuf>,
}

/// Walks `at` below its owner over the nodes that `supportRule.set` can
/// address (`support_record_path`), so a lock found here can be lifted. A
/// segment is a node declared in the current descriptor (an attribute, a
/// tabular section and its attribute, a dimension, a resource) or a node with
/// a descriptor of its own (a form, a template, a command, a nested
/// subsystem); each may carry its own record by uuid. Any other segment (a
/// module, a form item, an enum value) has no record of its own and ends the
/// walk: it follows the nearest node above it.
fn support_records_on_path<E>(
    at: &unica_address::address::QualifiedAddress,
    owner: &str,
    owner_relative: &std::path::Path,
    owner_bytes: &[u8],
    state: &unica_metadata_designer::support::SupportState,
    read: &mut GateRead<'_, E>,
) -> Result<PathRecords, E> {
    use unica_address::address::NodeKind;
    use unica_metadata_designer::support::support_root_uuid_from_bytes;

    let mut records = PathRecords {
        locked: None,
        node_rule: None,
        reached: None,
    };
    let segments = at.segments();
    let Some(first) = segments.first() else {
        return Ok(records);
    };
    if first.kind() == NodeKind::Configuration || !first.kind().is_metadata_kind() {
        return Ok(records);
    }
    let mut node = owner.to_string();
    let mut descriptor = owner_bytes.to_vec();
    let mut descriptor_relative = owner_relative.to_path_buf();
    let mut chain = String::new();
    let mut base = owner.to_string();
    // A nested subsystem lives in `Subsystems/<A>/Subsystems/<B>.xml`.
    let mut directory = owner_relative.with_extension("");
    let last = segments.len().saturating_sub(1);
    for (index, segment) in segments.iter().enumerate().skip(1) {
        if !unica_metadata::applicability::support_record_path(&segments[..=index]) {
            break;
        }
        let Some(name) = segment.name() else {
            break;
        };
        let kind = segment.kind().as_str();
        node.push_str(&format!(".{kind}.{name}"));
        let declared = format!("{chain}.{kind}.{name}");
        let (uuid, scope, own_directory) = if let Some(uuid) = declared_uuid(&descriptor, &declared)
        {
            chain = declared;
            (Some(uuid), vec![descriptor_relative.clone()], false)
        } else if matches!(kind, "Form" | "Template" | "Command" | "Subsystem") && chain.is_empty()
        {
            let folder = match kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => "Subsystems",
            };
            let relative = directory.join(folder).join(format!("{name}.xml"));
            let Some(bytes) = read(&relative)? else {
                break;
            };
            directory = directory.join(folder).join(name);
            let uuid = support_root_uuid_from_bytes(&bytes);
            descriptor = bytes;
            descriptor_relative = relative.clone();
            base = node.clone();
            (uuid, vec![relative, directory.clone()], true)
        } else {
            break;
        };
        let rule = uuid.and_then(|uuid| state.object_rule(&uuid));
        if rule == Some(0) && records.locked.is_none() {
            records.locked = Some((node.clone(), scope));
        }
        if index == last {
            records.node_rule = rule;
            records.reached = Some(ReachedNode {
                node: node.clone(),
                base: base.clone(),
                prefix: chain.clone(),
                descriptor: descriptor.clone(),
                directory: own_directory.then(|| directory.clone()),
            });
        }
    }
    Ok(records)
}

/// The uuid of the node declared in `descriptor` under the chain
/// `.Kind.Name…` below the described object, as `declared_chain` spells it.
fn declared_uuid(descriptor: &[u8], chain: &str) -> Option<String> {
    use unica_metadata_designer::remove::{declared_chain, declared_name};

    let text = std::str::from_utf8(descriptor).ok()?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    document
        .descendants()
        .filter(|node| node.is_element() && node.has_attribute("uuid"))
        .filter(|node| declared_name(*node).is_some())
        .find(|node| declared_chain(*node).concat() == chain)
        .and_then(|node| node.attribute("uuid"))
        .map(str::to_ascii_lowercase)
}

/// Where the nodes under a removed node are described.
struct DescendantRoot<'a> {
    /// The object `descriptor` describes.
    base: &'a str,
    /// The removed node's `.Kind.Name…` chain in `descriptor`; empty when
    /// it is the described object itself.
    prefix: &'a str,
    descriptor: &'a [u8],
    /// The directory of nested descriptors under the removed node.
    directory: Option<&'a std::path::Path>,
}

/// The nodes under a removed vendor node that carry their own `locked` rule:
/// attributes, tabular sections and other nodes declared under it in its
/// descriptor, and forms, templates, commands and nested subsystems described
/// in their own descriptors under its directory.
fn locked_vendor_descendants<E>(
    state: &unica_metadata_designer::support::SupportState,
    source_set: &str,
    root: DescendantRoot<'_>,
    read: &mut GateRead<'_, E>,
    enumerate: &mut GateEnumerate<'_, E>,
) -> Result<Vec<String>, E> {
    use unica_metadata_designer::remove::{declared_chain, declared_name};

    let mut locked = Vec::new();
    // `prefix` of `None`: every declared node of the descriptor, its own
    // root included, lies under the removed node.
    let mut collect = |base: &str, bytes: &[u8], prefix: Option<&str>| {
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
            let chain = declared_chain(node).concat();
            // The removed node's own rule is checked by the caller.
            if let Some(prefix) = prefix {
                let below = chain
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('.'));
                if !below {
                    continue;
                }
            }
            locked.push(format!("`{source_set}:{base}{chain}`"));
        }
    };
    collect(root.base, root.descriptor, Some(root.prefix));
    let Some(directory) = root.directory else {
        return Ok(locked);
    };
    let files = enumerate(directory)?;
    for relative in files {
        let (base, descriptor) = unica_metadata_designer::remove::meta_remove_referrer(&relative);
        let is_xml = relative
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"));
        if !descriptor || !is_xml {
            continue;
        }
        if let Some(bytes) = read(&relative)? {
            collect(&base, &bytes, None);
        }
    }
    Ok(locked)
}

/// The descriptor that carries the support rule of the addressed node: the
/// top-level metadata object, or the configuration root for root targets.
pub(crate) fn support_owner_relative(
    at: &unica_address::address::QualifiedAddress,
    source_kind: SourceSetKind,
) -> Option<(String, std::path::PathBuf)> {
    use unica_address::address::NodeKind;
    use unica_address::source_target::MetadataAddress;
    use unica_source_designer::layout::metadata_descriptor_relative;

    let segments = at.segments();
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
    let owner_address = MetadataAddress::parse_logical(&owner_path).ok()?;
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
    use unica_address::source_target::MetadataAddress;
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
    let Ok(owner_address) = MetadataAddress::parse_logical(&owner_path) else {
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
            if let Ok(nested_address) = MetadataAddress::parse_logical(&nested_path) {
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
#[cfg(any(test, feature = "test-support"))]
pub fn parse_family_operation_for_test(
    op_index: usize,
    operation: &crate::domain::apply::ApplyOp,
    binding: &ProviderRootBinding,
) -> Result<(), ApplyPlanError> {
    parse_family_operation(op_index, operation, binding).map(|_| ())
}

#[cfg(any(test, feature = "test-support"))]
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
    use unica_address::source_target::MetadataAddress;
    let segments = at.segments();
    let owner = segments.first()?;
    let owner_path = format!("{}.{}", owner.kind().as_str(), owner.name()?);
    match segments.get(1) {
        Some(nested)
            if matches!(nested.kind(), NodeKind::Form | NodeKind::Template)
                && nested.name().is_some() =>
        {
            let path = format!("{owner_path}.{}.{}", nested.kind().as_str(), nested.name()?);
            Some((MetadataAddress::parse_logical(&path).ok()?, 2))
        }
        _ => Some((MetadataAddress::parse_logical(&owner_path).ok()?, 1)),
    }
}

/// Recovery hints may omit evidence; productive admission uses the Result below.
pub fn staged_style_item_properties_hint(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    at: &unica_address::address::QualifiedAddress,
) -> Option<unica_metadata::style_color::StyleItemPropertiesObservation> {
    staged_style_item_properties(admission, binding, at, None)
        .ok()
        .flatten()
}

pub fn staged_style_item_properties(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    at: &unica_address::address::QualifiedAddress,
    intent: Option<&unica_metadata::property_input::PropertiesIntent>,
) -> Result<Option<unica_metadata::style_color::StyleItemPropertiesObservation>, ApplyPlanError> {
    let [owner] = at.segments() else {
        return Ok(None);
    };
    if owner.kind() != unica_address::address::NodeKind::StyleItem
        || at.source_set() != binding.source_set_name()
    {
        return Ok(None);
    }
    let Some(name) = owner.name() else {
        return Ok(None);
    };
    let path = at.to_string();
    let invalid = |message| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(path.clone())
    };
    let (address, covered) = descriptor_node(at)
        .ok_or_else(|| invalid("StyleItem descriptor route is unavailable".to_string()))?;
    if covered != 1 {
        return Ok(None);
    }
    let relative = unica_source_designer::layout::metadata_descriptor_relative(
        &address,
        binding.source_kind(),
    )
    .map_err(invalid)?;
    let mut staged = admission
        .staged_state()
        .map_err(|error| ApplyPlanError::staging(error, path.clone()))?;
    let parent = staged
        .read(std::path::Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(ApplyPlanErrorKind::NotFound, "source-set owner is absent")
                .at_path(path.clone())
        })?;
    let raw = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "StyleItem descriptor is absent",
            )
            .at_path(path.clone())
        })?;
    let text = std::str::from_utf8(&raw)
        .map_err(|error| invalid(error.to_string()))?
        .trim_start_matches('\u{feff}');
    unica_metadata_designer::style_color::observe_registered_properties(
        text,
        name,
        &relative,
        &parent,
        binding.source_kind(),
        intent,
    )
    .map(Some)
    .map_err(|error| {
        let kind = match &error {
            unica_metadata_designer::style_color::ColorIssue::NotFound(_) => {
                ApplyPlanErrorKind::NotFound
            }
            unica_metadata_designer::style_color::ColorIssue::Invalid(_) => {
                ApplyPlanErrorKind::InvalidSource
            }
            unica_metadata_designer::style_color::ColorIssue::ReadOnly(_) => {
                ApplyPlanErrorKind::InvalidState
            }
        };
        ApplyPlanError::new(kind, error.to_string()).at_path(path)
    })
}

pub fn staged_template_format(
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

/// Form facts for the same admission predicate used by the published dictionary.
pub fn staged_form_observation(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    at: &unica_address::address::QualifiedAddress,
) -> Option<unica_forms::FormObservation> {
    if let Some(role) = staged_form_item_role(admission, binding, at) {
        return Some(unica_forms::FormObservation::Item(role));
    }
    let column = unica_forms::applicability::is_form_column(at.segments());
    if !column && !unica_forms::applicability::is_form_attribute(at.segments()) {
        return None;
    }
    let form = unica_address::address::QualifiedAddress {
        line_range: None,
        source_set: at.source_set().to_string(),
        segments: at.segments()[..2].to_vec(),
    };
    let xml = staged_form_xml(admission, binding, &form)?;
    let attributes = unica_forms_designer::codec::parse_form_attributes_xml(&xml).ok()?;
    let name = at.segments()[2].name()?;
    let mut found = attributes.iter().filter(|attribute| attribute.name == name);
    let attribute = found.next()?;
    if found.next().is_some() {
        return None;
    }
    if !column {
        return unica_forms::FormObservation::from_attribute_type(attribute.type_name.as_deref()?);
    }
    let mut columns = attribute
        .columns
        .iter()
        .filter(|column| Some(column.name.as_str()) == at.segments()[3].name());
    let column = columns.next()?;
    if columns.next().is_some() {
        return None;
    }
    unica_forms::FormObservation::from_column_parent_type(column.parent_type.as_deref()?)
}

/// Дерево элементов стадированной формы `Owner.X.Form.F`, как его читает
/// `view`. `None` — формы или её `Form.xml` нет.
fn staged_form_items(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    form: &unica_address::address::QualifiedAddress,
) -> Option<Vec<unica_forms::facts::FormInfoElement>> {
    let xml = staged_form_xml(admission, binding, form)?;
    unica_forms_designer::planner::form_items_of(&xml).ok()
}

fn staged_form_xml(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    form: &unica_address::address::QualifiedAddress,
) -> Option<String> {
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
    std::str::from_utf8(&bytes).ok().map(str::to_owned)
}

/// Есть ли узел `at` в стадированном состоянии допуска. Узел с собственным
/// описанием есть, когда есть его файл; член объекта метаданных и узел СКД
/// проверяются штатным читателем представления и публичной проекцией.
/// `None` — по описаниям этого не установить.
pub fn staged_node_existence(
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    at: &unica_address::address::QualifiedAddress,
) -> Option<bool> {
    if matches!(at.segments(), [root] if root.kind() == unica_address::address::NodeKind::Configuration)
    {
        return Some(
            admission
                .staged_state()
                .ok()?
                .read(std::path::Path::new("Configuration.xml"))
                .ok()?
                .is_some(),
        );
    }
    let (address, covered) = descriptor_node(at)?;
    let relative = unica_source_designer::layout::metadata_descriptor_relative(
        &address,
        binding.source_kind(),
    )
    .ok()?;
    let Some(descriptor) = admission.staged_state().ok()?.read(&relative).ok()? else {
        return Some(false);
    };
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
        (RouteClass::Metadata, _, _) => {
            let tail = &at.segments()[covered..];
            let (member, scope) = match tail {
                [member] => (member, None),
                [section, member]
                    if section.kind() == NodeKind::TabularSection
                        && member.kind() == NodeKind::Attribute =>
                {
                    (member, section.name())
                }
                _ => return None,
            };
            let text = std::str::from_utf8(&descriptor).ok()?;
            match unica_metadata_designer::edit::find_typed_element_identity(
                text,
                member.kind().as_str(),
                scope,
                member.name()?,
            ) {
                Ok(_) => Some(true),
                Err(error) if error.code == unica_metadata::MetaDiagnosticCode::TargetNotFound => {
                    Some(false)
                }
                Err(_) => None,
            }
        }
        (RouteClass::Dcs, _, _) => {
            let bytes = resource("Template.xml")?;
            let text = std::str::from_utf8(&bytes).ok()?;
            // Membership does not consume or expose support properties.
            let facts = crate::infrastructure::family_codecs::dcs_codec()
                .read(
                    text,
                    unica_metadata::support_state::ObjectSupportData {
                        state: unica_metadata::support_state::ObjectSupportState::NotSupported,
                        direct_edit_safe: None,
                    },
                )
                .ok()?;
            let profile = crate::domain::source_capabilities::bsl_profile_for_source(
                binding.source_profile(),
            )?;
            let logical =
                crate::infrastructure::logical_tree::route_logical_address(at, profile).ok()?;
            match crate::infrastructure::v13_read_projection::project_typed_payload_for_source(
                &logical,
                serde_json::to_value(facts).ok()?,
                binding.source_kind(),
            ) {
                Ok(_) => Some(true),
                Err(error) if error.code() == unica_refusal::RefusalCode::NotFound => Some(false),
                Err(_) => None,
            }
        }
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

pub fn plan_hidden_v13_apply(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    plan_apply(
        request,
        binding,
        admission,
        PersonChosenChanges {
            mode_raise: None,
            support: Vec::new(),
            remove_support: false,
        },
    )
}

/// The change of vendor support a person chose in the question about a
/// `deny` lock: the support operations that lift exactly `lock`, planned
/// before the edits of the same request (INV.SOURCE.SUPPORT-SEPARATE-APPLY).
pub struct PersonChosenSupportChange<'a> {
    pub lock: &'a SupportLock,
    pub operations: &'a [crate::domain::apply::ApplyOp],
}

/// Plans `request` together with the support change a person chose: both
/// go into one plan and publish together. The change lifts only the lock
/// the person was asked about; the targets are then checked against the
/// support it leaves, and another lock, or none, refuses as before.
pub fn plan_hidden_v13_apply_with_person_chosen_support(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    change: PersonChosenSupportChange<'_>,
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    plan_apply(
        request,
        binding,
        admission,
        PersonChosenChanges {
            mode_raise: None,
            support: vec![change],
            remove_support: false,
        },
    )
}

/// The raise of a read-only compatibility mode a person chose in the
/// question about it (INV.SOURCE.WRITABLE-PROFILE): `request` sets the
/// set's own mode property on its root and nothing else.
pub struct PersonChosenModeRaise<'a> {
    pub read_only: &'a ReadOnlyMode,
    pub request: &'a ApplyRequest,
}

/// Everything a person chose for one call, planned before its edits: the
/// vendor support changes first, then the raise of the set's mode.
pub struct PersonChosenChanges<'a> {
    pub mode_raise: Option<PersonChosenModeRaise<'a>>,
    /// The support changes in the order the person was asked about them:
    /// each lifts the lock the gate showed after the changes before it.
    pub support: Vec<PersonChosenSupportChange<'a>>,
    /// The person took the configuration off vendor support entirely: its
    /// unreadable marker is replaced by the platform's marker of removed
    /// support before the edits (INV.SAFETY.APPLY-SUPPORT-GATE).
    pub remove_support: bool,
}

/// Plans `request` together with the changes a person chose: all of them go
/// into one plan and publish together, or none does.
pub fn plan_hidden_v13_apply_with_person_choices(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    chosen: PersonChosenChanges<'_>,
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    plan_apply(request, binding, admission, chosen)
}

fn plan_apply(
    request: &ApplyRequest,
    binding: &ProviderRootBinding,
    admission: &ApplyAdmission,
    chosen: PersonChosenChanges<'_>,
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    let PersonChosenChanges {
        mode_raise,
        support: chosen_support,
        remove_support,
    } = chosen;
    let mut parsed = request
        .ops()
        .iter()
        .enumerate()
        .map(|(op_index, operation)| parse_apply_operation(op_index, operation, binding))
        .collect::<Result<Vec<_>, ApplyPlanError>>()?;

    let mut staged = admission
        .staged_state()
        .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
    // A set older than the oldest editable compatibility mode is read-only.
    let configuration = staged
        .read(std::path::Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, request.at().to_string()))?;
    let mode_check = unica_source_designer::extension_mode::refuse_edit_below_minimum_mode(
        configuration.as_deref(),
        binding.source_kind(),
        raised_compatibility_mode(request, binding.source_kind()),
        &request.at().to_string(),
    );
    match (&mode_raise, mode_check) {
        (None, check) => check?,
        // The person raises exactly the mode they were asked about.
        (Some(raise), Err(error)) if error.read_only_mode() == Some(raise.read_only) => {
            if raise.request.at().to_string() != format!("{}:Configuration", binding.source_set_name())
                || raised_compatibility_mode(raise.request, binding.source_kind())
                    .is_none_or(|mode| !mode.editable())
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "a person-chosen mode raise sets only the set's own compatibility mode",
                )
                .at_path("ops"));
            }
        }
        (Some(_), Err(error)) => return Err(error),
        (Some(_), Ok(())) => {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "the compatibility mode changed after the person's answer; repeat the apply to be asked again",
            )
            .at_path(request.at().to_string()))
        }
    }
    refuse_targets_outside_writable_profile(request, binding, &mut staged)?;
    let gate = refuse_targets_on_locked_vendor_objects(request, binding, admission, &mut staged);
    let gate = match gate {
        // The person takes the configuration off support: its unreadable
        // marker no longer stands in the way.
        Err(error) if remove_support && error.support_state_unreadable() => Ok(Vec::new()),
        Err(error) => Err(error),
        Ok(_) if remove_support => {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "the vendor support state changed after the person's answer; repeat the apply to be asked again",
            )
            .at_path(request.at().to_string()))
        }
        Ok(warning) => Ok(warning),
    };
    if remove_support {
        let marker = std::path::Path::new("Ext/ParentConfigurations.bin");
        let current = staged
            .read(marker)
            .map_err(|error| ApplyPlanError::staging(error, request.at().to_string()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "the vendor support state changed after the person's answer; repeat the apply to be asked again",
                )
                .at_path(request.at().to_string())
            })?;
        staged
            .replace(
                marker,
                &current,
                unica_metadata_designer::support::REMOVED_SUPPORT_MARKER.to_vec(),
            )
            .map_err(|error| ApplyPlanError::staging(error, request.at().to_string()))?;
    }
    let mut support_warning = match gate {
        Ok(warnings) => warnings,
        // A lock the person answered is checked again below, against the
        // support their answers leave.
        Err(error) if !chosen_support.is_empty() && error.support_lock().is_some() => Vec::new(),
        Err(error) => return Err(error),
    };
    // The request the model composed never mixes support and ordinary edits.
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
    if let Some(raise) = &mode_raise {
        // The raise edits the configuration root: vendor support guards it
        // as any edit. Only the lock of the whole configuration the person
        // also lifted lets it pass.
        match refuse_targets_on_locked_vendor_objects(
            raise.request,
            binding,
            admission,
            &mut staged,
        ) {
            Ok(warnings) => support_warning.extend(warnings),
            Err(error)
                if error.support_lock().is_some_and(|lock| {
                    lock.kind == SupportLockKind::ConfigurationLocked
                        && chosen_support.iter().any(|chosen| {
                            chosen.lock.kind == SupportLockKind::ConfigurationLocked
                                && chosen.lock.source_set == lock.source_set
                        })
                }) => {}
            Err(error) => return Err(error),
        }
        let mut raised = Vec::with_capacity(raise.request.ops().len() + parsed.len());
        for operation in raise.request.ops() {
            raised.push(parse_apply_operation(0, operation, binding)?);
        }
        raised.append(&mut parsed);
        parsed = raised;
    }
    // The chosen support changes are planned first, each against the
    // support the ones before it leave: a change lifts the lock it was asked
    // about only while the gate still shows that lock. Then every target is
    // checked against the support the answers leave, so a lock the answers
    // did not lift refuses and asks its own question.
    let mut chosen_planned = Vec::new();
    for chosen in &chosen_support {
        let lock = chosen.lock;
        if !locks_on_targets(request, binding, admission, &mut staged)?.contains(lock) {
            // The answer does not lift a lock the gate shows: the gate's own
            // refusal stands, and without one the support changed.
            refuse_targets_on_locked_vendor_objects(request, binding, admission, &mut staged)?;
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "the vendor support state changed after the person's answer; repeat the apply to be asked again",
            )
            .at_path(request.at().to_string()));
        }
        let owner = format!("{}:{}", lock.source_set, lock.owner);
        let root = format!("{}:Configuration", lock.source_set);
        let mut support = Vec::with_capacity(chosen.operations.len());
        for operation in chosen.operations {
            let target = operation.at().to_string();
            let lifts_this_lock = target == owner
                || (target == root && lock.kind == SupportLockKind::ConfigurationLocked);
            if !lifts_this_lock
                || !changes_support_state(operation.name(), Some(operation.at()), binding)
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "a person-chosen support change carries only support operations",
                )
                .at_path("ops"));
            }
            match parse_apply_operation(0, operation, binding)? {
                ParsedApplyOperation::FormResource(operation) => support.push(operation),
                _ => {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidState,
                        "a person-chosen support change carries only support operations",
                    )
                    .at_path("ops"))
                }
            }
        }
        let authority = admission.form_resource_planning_authority(binding)?;
        chosen_planned.extend(form_resource::plan_form_resource_batch(
            &mut staged.proposal(),
            &authority,
            &support,
        )?);
    }
    if !chosen_support.is_empty() {
        support_warning.extend(refuse_targets_on_locked_vendor_objects(
            request,
            binding,
            admission,
            &mut staged,
        )?);
    }
    let code_type_plan = unica_metadata_designer::planner::PredefinedCodeTypePlan::from_operations(
        parsed.iter().filter_map(|operation| match operation {
            ParsedApplyOperation::Metadata(operation) => Some(operation),
            _ => None,
        }),
    );
    let mut provisional = chosen_planned;
    let mut code_support_warnings = Vec::new();
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
                code_support_warnings.extend(planned.support_warnings);
                provisional.extend(batch_effects_as_provisional(
                    &before,
                    &staged,
                    planned.effects,
                    cursor,
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
                    crate::infrastructure::family_codecs::SUPPORT_MARKER_READER,
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
                    unica_metadata_designer::planner::plan_metadata_batch(&crate::infrastructure::native_operations::meta::template_source::MetadataPlanningReadAdapter::new(&authority),unica_metadata_designer::planner::TemplateSeedEmitter(unica_templates_designer::mxl_root::empty_spreadsheet_document_xml),&mut staged.proposal(), &authority, &operations, &code_type_plan)?;
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

    code_type_plan.validate_final(&mut staged.proposal(), binding.source_kind())?;
    unica_metadata_designer::planner::validate_final_numerators(
        &mut staged.proposal(),
        parsed.iter().filter_map(|operation| match operation {
            ParsedApplyOperation::Metadata(operation) => {
                Some((operation.index(), operation.operation()))
            }
            _ => None,
        }),
    )?;
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
            crate::infrastructure::family_codecs::SUPPORT_MARKER_READER,
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
    // Values the compatibility mode of the set forbids are refused before the
    // apply publishes; the platform would refuse them on import.
    unica_metadata_designer::mode_checks::check_changed_descriptors(
        &mut staged.proposal(),
        binding.source_kind(),
    )?;
    let legacy_inline = collapse_legacy_inline_registrations(&mut staged, binding)?;
    let mut effects = reconcile_effects(staged.planned_changes(), provisional);
    // A plan whose operations cancel out edits nothing protected. The gate's
    // warnings go first: a module whose own owner is the locked node then
    // reports it once.
    for warning in planned_gate_warnings(support_warning, &staged.planned_changes()) {
        effects.push_warning(warning);
    }
    for warning in code_support_warnings {
        warning.append_if_changed(&staged.planned_changes(), &mut effects);
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
mod calculation_code_type_tests;

#[cfg(test)]
mod mode_rule_tests;

#[cfg(test)]
mod inherited_content_tests;

#[cfg(test)]
mod object_integrity_tests;

#[cfg(test)]
mod tests;
