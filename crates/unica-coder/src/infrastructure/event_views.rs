use crate::infrastructure::event_sources::{
    resolve_event_source_for_form_evidence, PropertyEventSource,
};
use unica_address::address::QualifiedAddress;
#[cfg(test)]
use unica_bindings::events::EventImplementationOwner;
use unica_bindings::events::{
    form_semantic_inputs, refine_property_owner_chain, EventImplementationShape,
    EventProjectionError,
};
use unica_bsl::profile::PlatformProfile;
use unica_bsl::projection::EventProjection;
#[cfg(test)]
use unica_bsl::projection::MethodKind;
use unica_forms_designer::codec::parse_form_event_evidence_xml;
use unica_source_contract::SourceSetKind;

pub(crate) fn project_property_event(
    source: &PropertyEventSource,
    form_xml: &str,
    module_text: Option<&str>,
) -> Result<EventProjection, EventProjectionError> {
    let evidence =
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::invalid_source)?;
    unica_bindings::events::project_property_event(
        &source.event_at,
        &source.form_at,
        &source.owner_chain,
        evidence,
        module_text,
    )
}

pub(crate) fn resolve_property_event_source(
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    event_at: &QualifiedAddress,
    form_xml: &str,
) -> Result<PropertyEventSource, EventProjectionError> {
    let mut source =
        resolve_event_source_for_form_evidence(source_set, source_set_kind, profile, event_at)
            .map_err(|error| EventProjectionError::unavailable(error.to_string()))?;
    let mut evidence =
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::invalid_source)?;
    evidence.context.metadata_owner = source
        .form_at
        .segments()
        .first()
        .map(unica_address::address::AddressSegment::kind);
    let inputs = form_semantic_inputs(&source.form_at.to_string(), &evidence);
    refine_property_owner_chain(&mut source.owner_chain, &inputs)?;
    Ok(source)
}

pub(crate) fn project_property_event_implementation(
    source: &PropertyEventSource,
    form_xml: &str,
    module_text: Option<&str>,
) -> Result<EventImplementationShape, EventProjectionError> {
    let evidence =
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::invalid_source)?;
    unica_bindings::events::project_property_event_implementation(
        &source.event_at,
        &source.form_at,
        &source.module_at,
        &source.owner_chain,
        evidence,
        module_text,
    )
}

#[cfg(test)]
#[path = "../../tests/platform/event_views.rs"]
mod tests;
