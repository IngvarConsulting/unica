#[cfg(test)]
use crate::infrastructure::event_sources::PropertyEventSource;
use crate::infrastructure::event_sources::{
    resolve_event_source, resolve_event_source_for_form_evidence, LogicalEventSource,
};
use crate::infrastructure::event_views::{
    project_property_event_implementation,
    resolve_property_event_source as refine_property_event_source,
};
use unica_bindings::events::{
    project_platform_event_implementation, property_call_type, require_plannable_state,
    require_postcondition, EventProjectionError, EventProjectionErrorKind,
};
use unica_bsl::event_text::emit_event_module;
use unica_bsl::profile::PlatformProfile;
use unica_bsl::projection::EventState;
use unica_forms_designer::codec::parse_form_event_evidence_xml;
use unica_forms_designer::event_patch::patch_form_event;
use unica_metadata::event_input::EventImplementArgs;

use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplyStagingError, PlannedApplyEffects,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::event_owner::prove_owner_plan;
use unica_source_designer::owner::owner_evidence_plan;

fn invalid_source(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
}

fn effect(kind: DomainEventKind, artifact: impl Into<String>) -> DomainEvent {
    DomainEvent::new(kind, artifact)
}

pub(crate) fn plan_event_implement_batch(
    staged: &mut ApplyProposal<'_>,
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    operations: &[EventImplementArgs],
    first_index: usize,
) -> Result<PlannedApplyEffects, ApplyPlanError> {
    let mut effects = PlannedApplyEffects::default();
    for (offset, operation) in operations.iter().enumerate() {
        plan_one_event(
            staged,
            &mut effects,
            source_set,
            source_set_kind,
            profile,
            operation,
        )
        .map_err(|error| {
            if error.path().is_some() {
                error
            } else {
                error.at_path(format!("ops[{}].args", first_index + offset))
            }
        })?;
    }
    Ok(effects)
}

fn plan_one_event(
    staged: &mut ApplyProposal<'_>,
    effects: &mut PlannedApplyEffects,
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    args: &EventImplementArgs,
) -> Result<(), ApplyPlanError> {
    if profile.module_prefix_capability(&args.at).is_some() {
        let source = resolve_event_source(source_set, source_set_kind, profile, &args.at)
            .map_err(|error| provider_unavailable(error.to_string()))?;
        let LogicalEventSource::Platform(source) = source else {
            return Err(provider_unavailable(
                "event source did not retain a Platform module",
            ));
        };
        if args.call_type.is_some() {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "Platform events do not accept callType",
            ));
        }
        let owner_plan = owner_evidence_plan(
            source_set_kind,
            &source.module_target,
            &source.descriptor_requirements,
        )
        .map_err(|error| provider_unavailable(error.to_string()))?;
        prove_owner_plan(staged, &owner_plan)?;
        let module_before = staged.read(&source.module_relative).map_err(staging)?;
        let module_text = optional_utf8_module(module_before.as_deref())?;
        let capability = profile
            .module_capability(&source.module_at)
            .ok_or_else(|| provider_unavailable("Platform module capability disappeared"))?;
        let shape = project_platform_event_implementation(
            &source.event_at,
            &source.module_at,
            capability,
            module_text.as_deref(),
        )
        .map_err(projection_error)?;
        require_plannable_state(&shape)?;
        let module_after = {
            let method = &shape;
            emit_event_module(
                module_before.as_deref(),
                &method.handler,
                &method.signature,
                method.method_kind,
                method.directive,
                method.region,
            )
        }?;
        let post_text = utf8_module(&module_after)?;
        let post = project_platform_event_implementation(
            &source.event_at,
            &source.module_at,
            capability,
            Some(post_text),
        )
        .map_err(|error| postcondition(error.to_string()))?;
        require_postcondition(&shape, &post, None)?;
        stage_postimage(
            staged,
            &source.module_relative,
            module_before.as_deref(),
            module_after,
        )?;
        effects.append_at(
            effect(DomainEventKind::ModuleChanged, source.module_at.to_string()),
            vec![source.module_relative.clone()],
        );
        return Ok(());
    }

    let unresolved =
        resolve_event_source_for_form_evidence(source_set, source_set_kind, profile, &args.at)
            .map_err(|error| provider_unavailable(error.to_string()))?;
    prove_owner_plan(staged, &unresolved.owner_evidence)?;
    let form_before = staged
        .read(&unresolved.form_xml_relative)
        .map_err(staging)?
        .ok_or_else(|| ApplyPlanError::new(ApplyPlanErrorKind::NotFound, "Form.xml is absent"))?;
    let form_text = utf8_source(&form_before, "Form.xml")?;
    let source =
        refine_property_event_source(source_set, source_set_kind, profile, &args.at, form_text)
            .map_err(projection_error)?;
    let module_before = staged.read(&source.module_relative).map_err(staging)?;
    let module_text = optional_utf8_module(module_before.as_deref())?;
    let shape = project_property_event_implementation(&source, form_text, module_text.as_deref())
        .map_err(projection_error)?;
    require_plannable_state(&shape)?;
    let evidence = parse_form_event_evidence_xml(form_text).map_err(invalid_source)?;
    let effective_call_type = property_call_type(
        &shape,
        &evidence.context.definition,
        evidence.context.direct_part_writable,
        args.call_type,
    )?;
    let form_after = if shape.projection.state == EventState::Available {
        {
            let source = &source;
            let method = &shape;
            patch_form_event(
                &form_before,
                &source.owner_chain,
                &method.projection.event_id,
                &method.handler,
                effective_call_type,
            )
        }?
    } else {
        form_before.clone()
    };
    let module_after = {
        let method = &shape;
        emit_event_module(
            module_before.as_deref(),
            &method.handler,
            &method.signature,
            method.method_kind,
            method.directive,
            method.region,
        )
    }?;
    let form_after_text = utf8_source(&form_after, "patched Form.xml")?;
    let module_after_text = utf8_module(&module_after)?;
    let post =
        project_property_event_implementation(&source, form_after_text, Some(module_after_text))
            .map_err(|error| postcondition(error.to_string()))?;
    require_postcondition(&shape, &post, effective_call_type)?;

    if form_after != form_before {
        staged
            .replace(&source.form_xml_relative, &form_before, form_after)
            .map_err(staging)?;
    }
    stage_postimage(
        staged,
        &source.module_relative,
        module_before.as_deref(),
        module_after,
    )?;
    if shape.projection.state == EventState::Available {
        effects.append_at(
            effect(DomainEventKind::FormChanged, source.form_at.to_string()),
            vec![source.form_xml_relative.clone()],
        );
    }
    effects.append_at(
        effect(DomainEventKind::ModuleChanged, source.module_at.to_string()),
        vec![source.module_relative.clone()],
    );
    Ok(())
}

fn stage_postimage(
    staged: &mut ApplyProposal<'_>,
    relative: &std::path::Path,
    before: Option<&[u8]>,
    after: Vec<u8>,
) -> Result<(), ApplyPlanError> {
    match before {
        Some(before) => staged.replace(relative, before, after).map_err(staging),
        None => staged.create(relative, after).map_err(staging),
    }
}

fn optional_utf8_module(bytes: Option<&[u8]>) -> Result<Option<String>, ApplyPlanError> {
    bytes
        .map(utf8_module)
        .transpose()
        .map(|value| value.map(str::to_string))
}

fn utf8_module(bytes: &[u8]) -> Result<&str, ApplyPlanError> {
    utf8_source(bytes, "BSL module")
}

fn utf8_source<'a>(bytes: &'a [u8], label: &str) -> Result<&'a str, ApplyPlanError> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    std::str::from_utf8(bytes).map_err(|_| invalid_source(format!("{label} is not valid UTF-8")))
}

fn staging(error: ApplyStagingError) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::Staging(error.kind()), error.to_string())
}

fn provider_unavailable(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::ProviderUnavailable, message)
}

fn postcondition(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message)
}

fn projection_error(error: EventProjectionError) -> ApplyPlanError {
    match error.kind() {
        EventProjectionErrorKind::ProviderUnavailable => provider_unavailable(error.to_string()),
        EventProjectionErrorKind::InvalidSource => invalid_source(error.to_string()),
    }
}

#[cfg(test)]
pub(crate) fn plan_event_implement_batch_owned_for_test(
    mut staged: unica_workspace::apply::ApplyStagedState,
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    operations: &[EventImplementArgs],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        PlannedApplyEffects,
    ),
    ApplyPlanError,
> {
    let effects = plan_event_implement_batch(
        &mut staged.proposal(),
        source_set,
        source_set_kind,
        profile,
        operations,
        0,
    )?;
    Ok((staged, effects))
}

#[cfg(test)]
#[path = "../../../tests/platform/event_implementation.rs"]
pub(super) mod tests;
