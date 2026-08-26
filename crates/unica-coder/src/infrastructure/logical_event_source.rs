use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::platform_profile::{
    ModuleCapability, ModuleRole, ModuleSourceLayout, PlatformProfile,
};
use crate::domain::project_sources::SourceSetKind;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::metadata_kinds::metadata_kind;
use crate::infrastructure::platform_xml_source_targets::platform_xml_module_relative;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertyEventOwnerKind {
    Form,
    Element,
    Table,
    Column,
    Command,
    /// A direct logical `Item` has no element-type discriminator. This value
    /// never escapes the strict resolver; Form evidence must refine it first.
    UnresolvedItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PropertyEventOwner {
    pub(crate) kind: PropertyEventOwnerKind,
    pub(crate) at: QualifiedAddress,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventOwnerDescriptorProof {
    SourceSet,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventOwnerDescriptorExpectation {
    pub(crate) relative: PathBuf,
    pub(crate) proof: EventOwnerDescriptorProof,
    pub(crate) expected_kind: String,
    pub(crate) expected_name: Option<String>,
    pub(crate) registered_by: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventOwnerEvidencePlan {
    pub(crate) source_set_kind: SourceSetKind,
    pub(crate) descriptors: Vec<EventOwnerDescriptorExpectation>,
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

    pub(crate) const fn code(&self) -> &'static str {
        "provider_unavailable"
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
        let module_target =
            module_source_address(&module_at, capability).map_err(EventSourceError::unavailable)?;
        let module_relative = module_relative(&module_target, source_set_kind)
            .map_err(EventSourceError::unavailable)?;
        let descriptor_requirements =
            module_descriptor_requirements(&module_target, capability, source_set_kind)?;
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
    let module_target = form_module_target(&form_target)?;
    let module_relative =
        module_relative(&module_target, source_set_kind).map_err(EventSourceError::unavailable)?;
    let descriptor_requirements = form_descriptor_requirements(&form_target, source_set_kind)?;
    let owner_evidence =
        owner_evidence_plan(source_set_kind, &form_target, &descriptor_requirements)?;
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

pub(crate) fn platform_event_owner_evidence(
    source_set_kind: SourceSetKind,
    source: &PlatformEventSource,
) -> Result<EventOwnerEvidencePlan, EventSourceError> {
    owner_evidence_plan(
        source_set_kind,
        &source.module_target,
        &source.descriptor_requirements,
    )
}

fn owner_evidence_plan(
    source_set_kind: SourceSetKind,
    target: &MetadataAddress,
    descriptor_requirements: &[PathBuf],
) -> Result<EventOwnerEvidencePlan, EventSourceError> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let external = matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    );
    let mut descriptors = Vec::new();
    if external {
        let [owner_kind, owner_name, ..] = parts.as_slice() else {
            return Err(EventSourceError::unavailable(
                "external event target has no typed owner identity",
            ));
        };
        let root_target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{owner_kind}.{owner_name}"),
        )
        .map_err(|error| EventSourceError::unavailable(error.to_string()))?;
        descriptors.push(EventOwnerDescriptorExpectation {
            relative: metadata_descriptor_relative(&root_target, source_set_kind)
                .map_err(EventSourceError::unavailable)?,
            proof: EventOwnerDescriptorProof::SourceSet,
            expected_kind: (*owner_kind).to_string(),
            expected_name: Some((*owner_name).to_string()),
            registered_by: None,
        });
    } else {
        descriptors.push(EventOwnerDescriptorExpectation {
            relative: PathBuf::from("Configuration.xml"),
            proof: EventOwnerDescriptorProof::SourceSet,
            expected_kind: match source_set_kind {
                SourceSetKind::Configuration => "Configuration",
                SourceSetKind::Extension => "Configuration",
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => unreachable!(),
            }
            .to_string(),
            expected_name: None,
            registered_by: None,
        });
    }

    for relative in descriptor_requirements {
        if descriptors.iter().any(|step| step.relative == *relative) {
            continue;
        }
        let index = descriptors.len();
        let identity_start = if external { 2 * index } else { 2 * (index - 1) };
        let identity = parts
            .get(identity_start..identity_start + 2)
            .ok_or_else(|| {
                EventSourceError::unavailable("owner descriptor depth is inconsistent")
            })?;
        descriptors.push(EventOwnerDescriptorExpectation {
            relative: relative.clone(),
            proof: EventOwnerDescriptorProof::Metadata,
            expected_kind: identity[0].to_string(),
            expected_name: Some(identity[1].to_string()),
            registered_by: Some(index - 1),
        });
    }
    Ok(EventOwnerEvidencePlan {
        source_set_kind,
        descriptors,
    })
}

fn form_module_target(form: &MetadataAddress) -> Result<MetadataAddress, EventSourceError> {
    MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{}.FormModule", form.as_str()),
    )
    .map_err(|error| EventSourceError::unavailable(error.to_string()))
}

fn form_descriptor_requirements(
    form: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<Vec<PathBuf>, EventSourceError> {
    let parts = form.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, "Form", form_name] = parts.as_slice() else {
        if matches!(parts.as_slice(), ["CommonForm", _]) {
            return metadata_descriptor_relative(form, source_set_kind)
                .map(|path| vec![path])
                .map_err(EventSourceError::unavailable);
        }
        return Err(EventSourceError::unavailable(
            "form target does not retain a typed metadata owner chain",
        ));
    };
    let owner = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner_kind}.{owner_name}"),
    )
    .map_err(|error| EventSourceError::unavailable(error.to_string()))?;
    let nested = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner_kind}.{owner_name}.Form.{form_name}"),
    )
    .map_err(|error| EventSourceError::unavailable(error.to_string()))?;
    Ok(vec![
        metadata_descriptor_relative(&owner, source_set_kind)
            .map_err(EventSourceError::unavailable)?,
        metadata_descriptor_relative(&nested, source_set_kind)
            .map_err(EventSourceError::unavailable)?,
    ])
}

fn module_descriptor_requirements(
    target: &MetadataAddress,
    capability: ModuleCapability,
    source_set_kind: SourceSetKind,
) -> Result<Vec<PathBuf>, EventSourceError> {
    if capability.source_layout() == ModuleSourceLayout::Root {
        return Ok(vec![PathBuf::from("Configuration.xml")]);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let descriptor_raw = match capability.source_layout() {
        ModuleSourceLayout::Direct
        | ModuleSourceLayout::Common
        | ModuleSourceLayout::CommonForm
        | ModuleSourceLayout::CommonCommand
        | ModuleSourceLayout::Service
        | ModuleSourceLayout::Bot => parts[..2].join("."),
        ModuleSourceLayout::NestedForm | ModuleSourceLayout::NestedCommand => parts[..4].join("."),
        ModuleSourceLayout::Root | ModuleSourceLayout::WebSocketClient => {
            return Err(EventSourceError::unavailable(
                "module descriptor requirements are unavailable for this layout",
            ))
        }
    };
    let descriptor = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &descriptor_raw)
        .map_err(|error| EventSourceError::unavailable(error.to_string()))?;
    let nested = metadata_descriptor_relative(&descriptor, source_set_kind)
        .map_err(EventSourceError::unavailable)?;
    if matches!(
        capability.source_layout(),
        ModuleSourceLayout::NestedForm | ModuleSourceLayout::NestedCommand
    ) && !matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &parts[..2].join("."))
            .map_err(|error| EventSourceError::unavailable(error.to_string()))?;
        return Ok(vec![
            metadata_descriptor_relative(&owner, source_set_kind)
                .map_err(EventSourceError::unavailable)?,
            nested,
        ]);
    }
    Ok(vec![nested])
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

pub(crate) fn module_source_address(
    module_at: &QualifiedAddress,
    capability: ModuleCapability,
) -> Result<MetadataAddress, String> {
    let segments = module_at.segments();
    let logical = match capability.source_layout() {
        ModuleSourceLayout::Root => match capability.role() {
            ModuleRole::ManagedApplication => "ManagedApplicationModule".to_string(),
            ModuleRole::OrdinaryApplication => "OrdinaryApplicationModule".to_string(),
            ModuleRole::Session => "SessionModule".to_string(),
            ModuleRole::ExternalConnection => "ExternalConnectionModule".to_string(),
            _ => return Err(unsupported_module_layout(capability)),
        },
        ModuleSourceLayout::Common => format!(
            "CommonModule.{}.Module",
            required_segment_name(segments.first(), capability)?
        ),
        ModuleSourceLayout::Direct => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_module_layout(capability))?;
            let role = match capability.role() {
                ModuleRole::Object => "ObjectModule",
                ModuleRole::Manager => "ManagerModule",
                ModuleRole::RecordSet => "RecordSetModule",
                ModuleRole::ValueManager => "ValueManagerModule",
                _ => return Err(unsupported_module_layout(capability)),
            };
            format!(
                "{}.{}.{role}",
                owner.kind().as_str(),
                required_segment_name(Some(owner), capability)?
            )
        }
        ModuleSourceLayout::CommonForm => format!(
            "CommonForm.{}.FormModule",
            required_segment_name(segments.first(), capability)?
        ),
        ModuleSourceLayout::CommonCommand => format!(
            "CommonCommand.{}.CommandModule",
            required_segment_name(segments.first(), capability)?
        ),
        ModuleSourceLayout::NestedForm | ModuleSourceLayout::NestedCommand => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_module_layout(capability))?;
            let child = segments
                .get(1)
                .ok_or_else(|| unsupported_module_layout(capability))?;
            let terminal = if capability.source_layout() == ModuleSourceLayout::NestedForm {
                "FormModule"
            } else {
                "CommandModule"
            };
            format!(
                "{}.{}.{}.{}.{terminal}",
                owner.kind().as_str(),
                required_segment_name(Some(owner), capability)?,
                child.kind().as_str(),
                required_segment_name(Some(child), capability)?,
            )
        }
        ModuleSourceLayout::Service | ModuleSourceLayout::Bot => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_module_layout(capability))?;
            format!(
                "{}.{}.Module",
                owner.kind().as_str(),
                required_segment_name(Some(owner), capability)?
            )
        }
        ModuleSourceLayout::WebSocketClient => {
            return Err(
                "WebSocketClient source layout is not specified for platform profile 8.3.27"
                    .to_string(),
            )
        }
    };
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &logical)
        .map_err(|error| error.to_string())
}

fn required_segment_name(
    segment: Option<&AddressSegment>,
    capability: ModuleCapability,
) -> Result<&str, String> {
    segment
        .and_then(AddressSegment::name)
        .ok_or_else(|| unsupported_module_layout(capability))
}

fn unsupported_module_layout(capability: ModuleCapability) -> String {
    format!(
        "module source layout is unavailable for {}.{}",
        capability.owner_kind().as_str(),
        capability.role().as_str()
    )
}

pub(crate) fn module_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    if matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        external_module_relative(target, source_set_kind)
    } else {
        platform_xml_module_relative(target)
    }
}

pub(crate) fn attached_resource_relative(
    target: &MetadataAddress,
    resource: &str,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    if matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return external_attached_resource_relative(target, resource, source_set_kind);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, rest @ ..] = parts.as_slice() else {
        return Err("typed reader target has no owner identity".to_string());
    };
    let owner = metadata_kind(owner_kind)
        .ok_or_else(|| format!("typed reader owner kind `{owner_kind}` has no platform layout"))?;
    let mut relative = PathBuf::from(owner.directory);
    relative.push(owner_name);
    match rest {
        [] if (*owner_kind == "CommonForm" && resource == "Form.xml")
            || (*owner_kind == "Role" && resource == "Rights.xml")
            || (*owner_kind == "Subsystem" && resource == "CommandInterface.xml")
            || (*owner_kind == "XDTOPackage" && resource == "Package.bin") =>
        {
            relative.push("Ext");
            relative.push(resource);
        }
        [nested_kind, nested_name] => {
            let directory = match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(format!(
                        "unsupported attached resource owner `{nested_kind}`"
                    ))
                }
            };
            relative.push(directory);
            relative.push(nested_name);
            relative.push("Ext");
            relative.push(resource);
        }
        _ => return Err("typed reader target has an unsupported attached-resource depth".into()),
    }
    Ok(relative)
}

pub(crate) fn metadata_descriptor_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    if matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return external_metadata_descriptor_relative(target, source_set_kind);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, rest @ ..] = parts.as_slice() else {
        return Err("metadata descriptor target has no owner identity".into());
    };
    let owner = metadata_kind(owner_kind)
        .ok_or_else(|| format!("metadata kind `{owner_kind}` has no platform layout"))?;
    let mut relative = PathBuf::from(owner.directory);
    match rest {
        [] => relative.push(format!("{owner_name}.xml")),
        [nested_kind, nested_name] => {
            relative.push(owner_name);
            relative.push(match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(format!(
                        "unsupported nested descriptor kind `{nested_kind}`"
                    ))
                }
            });
            relative.push(format!("{nested_name}.xml"));
        }
        _ => return Err("metadata descriptor target has unsupported depth".into()),
    }
    Ok(relative)
}

fn external_owner_parts(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<(&str, &str, Vec<&str>), String> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [kind, name, rest @ ..] = parts.as_slice() else {
        return Err("external target has no canonical owner identity".into());
    };
    let expected = match source_set_kind {
        SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
        SourceSetKind::ExternalReport => "ExternalReport",
        SourceSetKind::Configuration | SourceSetKind::Extension => {
            return Err("external path requested for configuration source set".into())
        }
    };
    if *kind != expected {
        return Err(format!("external source set has no `{kind}` owner family"));
    }
    Ok((kind, name, rest.to_vec()))
}

fn external_metadata_descriptor_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    let (_, owner_name, rest) = external_owner_parts(target, source_set_kind)?;
    match rest.as_slice() {
        [] => Ok(PathBuf::from(format!("{owner_name}.xml"))),
        [nested_kind, nested_name] => {
            let directory = match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(format!(
                        "unsupported external nested descriptor kind `{nested_kind}`"
                    ))
                }
            };
            Ok(PathBuf::from(owner_name)
                .join(directory)
                .join(format!("{nested_name}.xml")))
        }
        _ => Err("external descriptor target has unsupported depth".into()),
    }
}

fn external_attached_resource_relative(
    target: &MetadataAddress,
    resource: &str,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    let (_, owner_name, rest) = external_owner_parts(target, source_set_kind)?;
    let [nested_kind, nested_name] = rest.as_slice() else {
        return Err("external attached resource requires a nested owner".into());
    };
    let directory = match *nested_kind {
        "Form" => "Forms",
        "Template" => "Templates",
        "Command" => "Commands",
        _ => {
            return Err(format!(
                "unsupported external attached resource owner `{nested_kind}`"
            ))
        }
    };
    Ok(PathBuf::from(owner_name)
        .join(directory)
        .join(nested_name)
        .join("Ext")
        .join(resource))
}

fn external_module_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    let (_, owner_name, rest) = external_owner_parts(target, source_set_kind)?;
    match rest.as_slice() {
        [terminal] if *terminal == "ObjectModule" => Ok(PathBuf::from(owner_name)
            .join("Ext")
            .join("ObjectModule.bsl")),
        [nested_kind, nested_name, terminal] => {
            let (directory, file) = match (*nested_kind, *terminal) {
                ("Form", "FormModule") => ("Forms", PathBuf::from("Form/Module.bsl")),
                ("Command", "CommandModule") => ("Commands", PathBuf::from("CommandModule.bsl")),
                _ => return Err("external nested module target has unsupported layout".into()),
            };
            Ok(PathBuf::from(owner_name)
                .join(directory)
                .join(nested_name)
                .join("Ext")
                .join(file))
        }
        _ => Err("external module target has unsupported layout".into()),
    }
}
