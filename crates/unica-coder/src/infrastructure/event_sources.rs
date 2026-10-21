use std::fmt;
use std::path::{Path, PathBuf};
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_address::module_routes::ModuleRole;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_bsl::profile::{ModuleCapability, PlatformProfile};
use unica_forms::event_targets::{PropertyEventOwner, PropertyEventOwnerKind};
use unica_refusal::RefusalCode;
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::attached_resource_relative;
use unica_source_designer::module_layout::{
    external_module_metadata_address, form_descriptor_requirements, form_module_target,
    logical_module_from_metadata, module_descriptor_requirements, module_relative,
    module_source_address, platform_xml_module_identity,
};
use unica_source_designer::owner::{owner_evidence_plan, EventOwnerEvidencePlan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformEventSource {
    pub(crate) event_at: QualifiedAddress,
    pub(crate) module_at: QualifiedAddress,
    pub(crate) module_target: MetadataAddress,
    pub(crate) module_relative: PathBuf,
    pub(crate) descriptor_requirements: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PropertyEventSource {
    pub(crate) event_at: QualifiedAddress,
    pub(crate) form_at: QualifiedAddress,
    pub(crate) form_target: MetadataAddress,
    pub(crate) form_xml_relative: PathBuf,
    pub(crate) module_relative: PathBuf,
    pub(crate) module_at: QualifiedAddress,
    pub(crate) owner_chain: Vec<PropertyEventOwner>,
    pub(crate) descriptor_requirements: Vec<PathBuf>,
    pub(crate) owner_evidence: EventOwnerEvidencePlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogicalEventSource {
    Platform(PlatformEventSource),
    Property(PropertyEventSource),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventSourceError {
    message: String,
}

impl EventSourceError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> RefusalCode {
        RefusalCode::ProviderUnavailable
    }
}

impl fmt::Display for EventSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for EventSourceError {}

pub(crate) fn resolve_event_source(
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    event_at: &QualifiedAddress,
) -> Result<LogicalEventSource, EventSourceError> {
    if event_at.source_set() != source_set {
        return Err(EventSourceError::unavailable(
            "event address belongs to another admitted source set",
        ));
    }
    let event = event_at
        .segments()
        .last()
        .ok_or_else(|| EventSourceError::unavailable("event address has no logical segments"))?;
    if event.kind() != NodeKind::Event || event.name().is_none() {
        return Err(EventSourceError::unavailable(
            "logical event source requires one exact Event leaf",
        ));
    }

    if let Some(capability) = profile.module_prefix_capability(event_at) {
        if source_set_kind == SourceSetKind::Extension {
            return Err(EventSourceError::unavailable(
                "extension Platform-event interception is not proved",
            ));
        }
        if matches!(
            capability.role(),
            ModuleRole::WebSocketClient
                | ModuleRole::Common
                | ModuleRole::Form
                | ModuleRole::HttpService
                | ModuleRole::WebService
                | ModuleRole::IntegrationService
        ) {
            return Err(EventSourceError::unavailable(format!(
                "event source layout is unavailable for {}.{}",
                capability.owner_kind().as_str(),
                capability.role().as_str()
            )));
        }
        let (module_at, _) =
            module_prefix(event_at, profile, capability).map_err(EventSourceError::unavailable)?;
        let module_target = module_source_address(
            &module_at,
            capability.owner_kind(),
            capability.role(),
            capability.route_kind(),
        )
        .map_err(EventSourceError::unavailable)?;
        let module_relative = module_relative(&module_target, source_set_kind)
            .map_err(EventSourceError::unavailable)?;
        let descriptor_requirements = module_descriptor_requirements(
            &module_target,
            capability.route_kind(),
            source_set_kind,
        )
        .map_err(EventSourceError::unavailable)?;
        return Ok(LogicalEventSource::Platform(PlatformEventSource {
            event_at: event_at.clone(),
            module_at,
            module_target,
            module_relative,
            descriptor_requirements,
        }));
    }

    let source = resolve_property_event_source(source_set_kind, event_at)?;
    if source
        .owner_chain
        .iter()
        .any(|owner| owner.kind == PropertyEventOwnerKind::UnresolvedItem)
    {
        return Err(EventSourceError::unavailable(
            "direct form Item owner is ambiguous without retained Form.xml evidence",
        ));
    }
    Ok(LogicalEventSource::Property(source))
}

pub(in crate::infrastructure) fn resolve_event_source_for_form_evidence(
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    event_at: &QualifiedAddress,
) -> Result<PropertyEventSource, EventSourceError> {
    if event_at.source_set() != source_set {
        return Err(EventSourceError::unavailable(
            "event address belongs to another admitted source set",
        ));
    }
    if profile.module_prefix_capability(event_at).is_some() {
        return Err(EventSourceError::unavailable(
            "property evidence resolver does not accept Platform events",
        ));
    }
    resolve_property_event_source(source_set_kind, event_at)
}

fn resolve_property_event_source(
    source_set_kind: SourceSetKind,
    event_at: &QualifiedAddress,
) -> Result<PropertyEventSource, EventSourceError> {
    let segments = event_at.segments();
    let form_index = segments
        .iter()
        .position(|segment| segment.kind() == NodeKind::Form && segment.name().is_some())
        .ok_or_else(|| {
            EventSourceError::unavailable(
                "event is neither a proved Platform module event nor a Form property event",
            )
        })?;
    let form_at =
        qualified_prefix(event_at, form_index + 1).map_err(EventSourceError::unavailable)?;
    let form_target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &form_at.logical_path())
            .map_err(|error| EventSourceError::unavailable(error.to_string()))?;
    let tail = &segments[form_index + 1..segments.len() - 1];
    let mut owner_chain = vec![PropertyEventOwner {
        kind: PropertyEventOwnerKind::Form,
        at: form_at.clone(),
    }];
    match tail {
        [] => {}
        items
            if items
                .iter()
                .all(|item| item.kind() == NodeKind::Item && item.name().is_some()) =>
        {
            for index in 0..items.len() {
                owner_chain.push(PropertyEventOwner {
                    kind: PropertyEventOwnerKind::UnresolvedItem,
                    at: qualified_prefix(event_at, form_index + index + 2)
                        .map_err(EventSourceError::unavailable)?,
                });
            }
        }
        [command] if command.kind() == NodeKind::Command && command.name().is_some() => {
            owner_chain.push(PropertyEventOwner {
                kind: PropertyEventOwnerKind::Command,
                at: qualified_prefix(event_at, form_index + 2)
                    .map_err(EventSourceError::unavailable)?,
            });
        }
        _ => {
            return Err(EventSourceError::unavailable(
                "form event owner chain mixes unsupported logical owner kinds",
            ))
        }
    }
    let form_xml_relative = attached_resource_relative(&form_target, "Form.xml", source_set_kind)
        .map_err(EventSourceError::unavailable)?;
    let module_target = form_module_target(&form_target).map_err(EventSourceError::unavailable)?;
    let module_relative =
        module_relative(&module_target, source_set_kind).map_err(EventSourceError::unavailable)?;
    let descriptor_requirements = form_descriptor_requirements(&form_target, source_set_kind)
        .map_err(EventSourceError::unavailable)?;
    let owner_evidence =
        owner_evidence_plan(source_set_kind, &form_target, &descriptor_requirements)
            .map_err(EventSourceError::unavailable)?;
    let module_at = QualifiedAddress::parse(&format!("{form_at}.Module.Form"))
        .map_err(|error| EventSourceError::unavailable(error.to_string()))?;
    Ok(PropertyEventSource {
        event_at: event_at.clone(),
        form_at,
        form_target,
        form_xml_relative,
        module_relative,
        module_at,
        owner_chain,
        descriptor_requirements,
        owner_evidence,
    })
}

pub(crate) fn module_prefix(
    address: &QualifiedAddress,
    profile: PlatformProfile,
    capability: ModuleCapability,
) -> Result<(QualifiedAddress, usize), String> {
    for length in 1..=address.segments().len() {
        let prefix = qualified_prefix(address, length)?;
        if profile.module_capability(&prefix) == Some(capability) {
            return Ok((prefix, length));
        }
    }
    Err("module prefix could not be reconstructed from the platform profile".to_string())
}

fn qualified_prefix(address: &QualifiedAddress, length: usize) -> Result<QualifiedAddress, String> {
    let logical = render_segments(&address.segments()[..length]);
    QualifiedAddress::parse(&format!("{}:{logical}", address.source_set()))
        .map_err(|error| error.to_string())
}

fn render_segments(segments: &[AddressSegment]) -> String {
    let mut values = Vec::with_capacity(segments.len() * 2);
    for segment in segments {
        values.push(segment.kind().as_str());
        if let Some(name) = segment.name() {
            values.push(name);
        }
    }
    values.join(".")
}

/// Reverse of [`module_relative`]: the logical module address whose source
/// layout places exactly `relative` inside a source set.
///
/// The candidate comes from the same layout tables the readers use, and is
/// accepted only when the forward mapping for that address renders the very
/// same relative path. A file outside the module layout (a stray `.bsl`, a
/// WebSocketClient module, a mismatched name) has no address rather than a
/// guessed one. Registration of the owner is proved later by `view`; this
/// mapping reads nothing.
pub(crate) fn module_address_for_relative(
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    relative: &Path,
) -> Option<QualifiedAddress> {
    let metadata = if matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        external_module_metadata_address(relative, source_set_kind)?
    } else {
        platform_xml_module_identity(relative)
            .ok()?
            .address
            .as_str()
            .to_owned()
    };
    let logical = logical_module_from_metadata(&metadata)?;
    let at = QualifiedAddress::parse(&format!("{source_set}:{logical}")).ok()?;
    let capability = profile.module_capability(&at)?;
    let target = module_source_address(
        &at,
        capability.owner_kind(),
        capability.role(),
        capability.route_kind(),
    )
    .ok()?;
    (module_relative(&target, source_set_kind).ok()? == relative).then_some(at)
}

#[cfg(test)]
#[path = "../../tests/platform/event_sources.rs"]
mod tests;
