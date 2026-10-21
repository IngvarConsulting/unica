//! Semantic event projection joins over already observed Forms and BSL facts.
use crate::projection::{
    project_form_owner_event_record, project_form_owner_events, project_module,
    project_module_event_record, required_event_directive, EventDirectiveOwner, FormBindingOwner,
    FormEventBindingInput, FormEventOwnerInput, FormMethodFact, ModuleProjectionRequest,
    PlatformEventProjectionRecordError, PlatformEventWriteCapability,
};
use std::fmt;
use unica_address::address::QualifiedAddress;
use unica_bsl::event_text::RequiredEventDirective;
use unica_bsl::profile::{ModuleCapability, PlatformProfile};
use unica_bsl::projection::{EventProjection, EventState, MethodKind, ModuleProjectionSet};
use unica_forms::event_targets::{PropertyEventOwner, PropertyEventOwnerKind};
use unica_forms::events::{
    form_event_handler_signature_8_3_27, FormDefinitionKind, FormElementKind,
};
use unica_forms::facts::{FormEventEvidence, FormInfoElement, FormInfoEvent};
use unica_metadata::event::FormCallType;
use unica_metadata::event_catalog::PlatformEventSpec;
use unica_refusal::RefusalCode;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventProjectionError {
    kind: EventProjectionErrorKind,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventProjectionErrorKind {
    ProviderUnavailable,
    InvalidSource,
}

impl EventProjectionError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: EventProjectionErrorKind::ProviderUnavailable,
            message: message.into(),
        }
    }

    pub fn invalid_source(message: impl Into<String>) -> Self {
        Self {
            kind: EventProjectionErrorKind::InvalidSource,
            message: message.into(),
        }
    }

    pub const fn kind(&self) -> EventProjectionErrorKind {
        self.kind
    }

    pub const fn code(&self) -> RefusalCode {
        RefusalCode::ProviderUnavailable
    }
}

impl fmt::Display for EventProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for EventProjectionError {}

pub struct FormSemanticInputs {
    pub owners: Vec<FormEventOwnerInput>,
    pub bindings: Vec<FormEventBindingInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventImplementationOwner {
    Platform,
    Form,
    Element,
    Table,
    Column,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventImplementationShape {
    pub projection: EventProjection,
    pub owner: EventImplementationOwner,
    pub method_kind: MethodKind,
    pub handler: String,
    pub implementation_at: String,
    pub signature: String,
    pub directive: RequiredEventDirective,
    pub region: &'static str,
}

pub fn project_platform_event_implementation(
    event_at: &QualifiedAddress,
    module_at: &QualifiedAddress,
    capability: ModuleCapability,
    module_text: Option<&str>,
) -> Result<EventImplementationShape, EventProjectionError> {
    let record = project_module_event_record(
        ModuleProjectionRequest {
            at: module_at,
            capability,
            title: String::new(),
            rev: "",
            source: module_text,
            common_module: None,
            handles: &[],
            declarative_bindings: &[],
            extension_targets: &[],
            platform_event_write: PlatformEventWriteCapability::Proven,
        },
        &event_at.to_string(),
    )
    .map_err(|error| match error {
        PlatformEventProjectionRecordError::InvalidSource(message) => {
            EventProjectionError::invalid_source(message)
        }
        PlatformEventProjectionRecordError::ProviderUnavailable(message) => {
            EventProjectionError::unavailable(message)
        }
    })?;
    shape_from_spec(
        record.projection,
        EventImplementationOwner::Platform,
        None,
        module_at,
        EventDirectiveOwner::Platform(capability),
        record.spec,
    )
}

pub fn project_property_event_implementation(
    event_at: &QualifiedAddress,
    form_at: &QualifiedAddress,
    module_at: &QualifiedAddress,
    owner_chain: &[PropertyEventOwner],
    mut evidence: FormEventEvidence,
    module_text: Option<&str>,
) -> Result<EventImplementationShape, EventProjectionError> {
    evidence.context.metadata_owner = form_at
        .segments()
        .first()
        .map(unica_address::address::AddressSegment::kind);
    let inputs = form_semantic_inputs(&form_at.to_string(), &evidence);
    validate_owner_chain(owner_chain, &inputs)?;
    let capability = PlatformProfile::v8_3_27()
        .module_capability(module_at)
        .ok_or_else(|| EventProjectionError::unavailable("form module capability is absent"))?;
    let module = project_module(ModuleProjectionRequest {
        at: module_at,
        capability,
        title: String::new(),
        rev: "",
        source: module_text,
        common_module: None,
        handles: &inputs.bindings,
        declarative_bindings: &[],
        extension_targets: &[],
        platform_event_write: PlatformEventWriteCapability::Proven,
    })
    .map_err(EventProjectionError::invalid_source)?;
    let methods = form_method_facts(&module);
    let owner_at = owner_chain
        .last()
        .ok_or_else(|| EventProjectionError::unavailable("property event has no exact owner"))?
        .at
        .to_string();
    let owner = inputs
        .owners
        .iter()
        .find(|candidate| candidate.at() == owner_at)
        .ok_or_else(|| {
            EventProjectionError::unavailable("property event owner is absent from Form evidence")
        })?;
    let record = project_form_owner_event_record(
        &evidence.context,
        owner,
        &inputs.bindings,
        &methods,
        &event_at.to_string(),
    )
    .ok_or_else(|| {
        EventProjectionError::unavailable("projected Property event lost its catalog record")
    })?;
    let implementation_owner = match record.owner {
        FormBindingOwner::Form => EventImplementationOwner::Form,
        FormBindingOwner::Element(_) => EventImplementationOwner::Element,
        FormBindingOwner::Table => EventImplementationOwner::Table,
        FormBindingOwner::Column(_) => EventImplementationOwner::Column,
        FormBindingOwner::Command => EventImplementationOwner::Command,
    };
    let canonical_handler = match (record.projection.state, implementation_owner) {
        (EventState::Missing | EventState::Implemented | EventState::Invalid, _) => {
            record.projection.handler.clone()
        }
        (_, EventImplementationOwner::Form) => record.spec.handler_ru.clone(),
        (
            _,
            EventImplementationOwner::Element
            | EventImplementationOwner::Table
            | EventImplementationOwner::Column
            | EventImplementationOwner::Command,
        ) => {
            let leaf = owner_chain
                .last()
                .and_then(|owner| owner.at.segments().last())
                .and_then(unica_address::address::AddressSegment::name)
                .ok_or_else(|| {
                    EventProjectionError::unavailable("property owner has no exact leaf name")
                })?;
            format!("{leaf}{}", record.spec.handler_ru)
        }
        (_, EventImplementationOwner::Platform) => unreachable!(),
    };
    shape_from_spec(
        record.projection,
        implementation_owner,
        Some(canonical_handler),
        module_at,
        EventDirectiveOwner::Property(record.owner),
        record.spec,
    )
}

fn form_method_facts(module: &ModuleProjectionSet) -> Vec<FormMethodFact<'_>> {
    module
        .methods()
        .iter()
        .map(|method| {
            FormMethodFact::new(
                &method.name,
                method.method_kind,
                &method.parameters,
                // Формы и владельцы событий всегда знают контексты; `None`
                // бывает только у заимствованного общего модуля без событий.
                method
                    .compile
                    .contexts
                    .iter()
                    .flatten()
                    .map(String::as_str)
                    .collect(),
            )
            .with_directive(method.compile.directive.as_deref())
        })
        .collect()
}

fn shape_from_spec(
    projection: EventProjection,
    owner: EventImplementationOwner,
    handler: Option<String>,
    module_at: &QualifiedAddress,
    directive_owner: EventDirectiveOwner,
    spec: &PlatformEventSpec,
) -> Result<EventImplementationShape, EventProjectionError> {
    let method_kind = match spec.method_kind.as_str() {
        "procedure" => MethodKind::Procedure,
        "function" => MethodKind::Function,
        _ => {
            return Err(EventProjectionError::unavailable(
                "event catalog has an unsupported method kind",
            ))
        }
    };
    let handler = handler.unwrap_or_else(|| spec.handler_ru.clone());
    let implementation_at = format!("{module_at}.Method.{handler}");
    // A form event handler gets the current signature, with `Элемент` first
    // for an item, table or column event (#1390); the help omits it.
    let catalog_signature = match directive_owner {
        EventDirectiveOwner::Platform(_) => spec.signature_ru.clone(),
        EventDirectiveOwner::Property(owner) => {
            form_event_handler_signature_8_3_27(owner.catalog_owner(), spec)
        }
    };
    let signature = substitute_catalog_handler(&catalog_signature, &spec.handler_ru, &handler)?;
    let directive = required_event_directive(directive_owner, spec)
        .map_err(|error| EventProjectionError::unavailable(error.to_string()))?;
    let region = match owner {
        EventImplementationOwner::Platform => "ОбработчикиСобытий",
        EventImplementationOwner::Form => "ОбработчикиСобытийФормы",
        EventImplementationOwner::Element
        | EventImplementationOwner::Table
        | EventImplementationOwner::Column => "ОбработчикиСобытийЭлементовФормы",
        EventImplementationOwner::Command => "ОбработчикиКомандФормы",
    };
    Ok(EventImplementationShape {
        projection,
        owner,
        method_kind,
        handler,
        implementation_at,
        signature,
        directive,
        region,
    })
}

fn substitute_catalog_handler(
    signature: &str,
    catalog_handler: &str,
    handler: &str,
) -> Result<String, EventProjectionError> {
    let (prefix, tail) = signature.split_once(catalog_handler).ok_or_else(|| {
        EventProjectionError::unavailable("event catalog signature does not contain its handler")
    })?;
    if prefix.contains('(') || tail.matches(catalog_handler).count() != 0 {
        return Err(EventProjectionError::unavailable(
            "event catalog signature has an ambiguous handler slot",
        ));
    }
    Ok(format!("{prefix}{handler}{tail}"))
}

pub fn project_platform_event(
    event_at: &QualifiedAddress,
    module_at: &QualifiedAddress,
    capability: ModuleCapability,
    module_text: Option<&str>,
) -> Result<EventProjection, EventProjectionError> {
    let projection = project_module(ModuleProjectionRequest {
        at: module_at,
        capability,
        title: String::new(),
        rev: "",
        source: module_text,
        common_module: None,
        handles: &[],
        declarative_bindings: &[],
        extension_targets: &[],
        platform_event_write: PlatformEventWriteCapability::Proven,
    })
    .map_err(EventProjectionError::invalid_source)?;
    project_platform_event_from_projection(event_at, &projection)
}

pub fn project_platform_event_from_projection(
    event_at: &QualifiedAddress,
    projection: &ModuleProjectionSet,
) -> Result<EventProjection, EventProjectionError> {
    projection
        .events()
        .iter()
        .find(|event| event.at == event_at.to_string())
        .cloned()
        .ok_or_else(|| {
            EventProjectionError::unavailable(format!(
                "event `{}` is not in the frozen module catalog",
                event_at
            ))
        })
}

pub fn project_property_events(
    form_at: &QualifiedAddress,
    evidence: &FormEventEvidence,
    module_text: Option<&str>,
) -> Result<Vec<EventProjection>, EventProjectionError> {
    let module_at = QualifiedAddress::parse(&format!("{form_at}.Module.Form"))
        .map_err(|error| EventProjectionError::unavailable(error.to_string()))?;
    let capability = PlatformProfile::v8_3_27()
        .module_capability(&module_at)
        .ok_or_else(|| EventProjectionError::unavailable("form module capability is absent"))?;
    let inputs = form_semantic_inputs(&form_at.to_string(), evidence);
    let module = project_module(ModuleProjectionRequest {
        at: &module_at,
        capability,
        title: String::new(),
        rev: "",
        source: module_text,
        common_module: None,
        handles: &inputs.bindings,
        declarative_bindings: &[],
        extension_targets: &[],
        platform_event_write: PlatformEventWriteCapability::Proven,
    })
    .map_err(EventProjectionError::invalid_source)?;
    project_property_events_from_projection(evidence, &inputs, &module)
}

pub fn project_property_events_from_projection(
    evidence: &FormEventEvidence,
    inputs: &FormSemanticInputs,
    module: &ModuleProjectionSet,
) -> Result<Vec<EventProjection>, EventProjectionError> {
    let methods = form_method_facts(module);
    Ok(project_form_owner_events(
        &evidence.context,
        &inputs.owners,
        &inputs.bindings,
        &methods,
    ))
}

pub fn form_semantic_inputs(form_at: &str, evidence: &FormEventEvidence) -> FormSemanticInputs {
    let mut inputs = FormSemanticInputs {
        owners: vec![FormEventOwnerInput::new(FormBindingOwner::Form, form_at)],
        bindings: evidence
            .events
            .iter()
            .map(|event| form_event_binding(FormBindingOwner::Form, form_at, event))
            .collect(),
    };
    collect_element_semantics(form_at, &evidence.elements, false, &mut inputs);
    for command in &evidence.commands {
        let at = format!("{form_at}.Command.{}", command.name);
        inputs
            .owners
            .push(FormEventOwnerInput::new(FormBindingOwner::Command, &at));
        inputs.bindings.extend(
            command
                .actions
                .iter()
                .map(|event| form_event_binding(FormBindingOwner::Command, &at, event)),
        );
    }
    inputs
}

fn form_event_binding(
    owner: FormBindingOwner,
    at: &str,
    event: &FormInfoEvent,
) -> FormEventBindingInput {
    FormEventBindingInput::property(
        owner,
        at,
        &event.name,
        &event.handler,
        event.call_type.as_deref(),
    )
}

fn collect_element_semantics(
    parent_at: &str,
    elements: &[FormInfoElement],
    parent_is_table: bool,
    inputs: &mut FormSemanticInputs,
) {
    for element in elements {
        let at = format!("{parent_at}.Item.{}", element.name);
        let is_table = element.event_kind == Some(FormElementKind::Table);
        if let Some(kind) = element.event_kind {
            // Командная панель таблицы — её часть, а не колонка.
            let owner = if parent_is_table && kind != FormElementKind::CommandBar {
                FormBindingOwner::Column(kind)
            } else if is_table {
                FormBindingOwner::Table
            } else {
                FormBindingOwner::Element(kind)
            };
            let mut semantic_owner = FormEventOwnerInput::new(owner, &at);
            if is_table {
                if let Some(data_path) = element
                    .binding
                    .as_ref()
                    .filter(|binding| binding.kind == "dataPath")
                    .map(|binding| binding.target.as_str())
                {
                    semantic_owner = semantic_owner.with_data_path(data_path);
                }
            }
            inputs.owners.push(semantic_owner);
            inputs.bindings.extend(
                element
                    .events
                    .iter()
                    .map(|event| form_event_binding(owner, &at, event)),
            );
        }
        collect_element_semantics(&at, &element.children, is_table, inputs);
    }
}

pub fn validate_owner_chain(
    owner_chain: &[PropertyEventOwner],
    inputs: &FormSemanticInputs,
) -> Result<(), EventProjectionError> {
    if owner_chain.is_empty() {
        return Err(EventProjectionError::unavailable(
            "property event source has no owner chain",
        ));
    }
    for expected in owner_chain {
        let actual = inputs
            .owners
            .iter()
            .find(|owner| owner.at() == expected.at.to_string())
            .ok_or_else(|| {
                EventProjectionError::unavailable(format!(
                    "form evidence does not contain owner `{}`",
                    expected.at
                ))
            })?;
        let matches = match (expected.kind, actual.owner()) {
            (PropertyEventOwnerKind::Form, FormBindingOwner::Form)
            | (PropertyEventOwnerKind::Element, FormBindingOwner::Element(_))
            | (PropertyEventOwnerKind::Table, FormBindingOwner::Table)
            | (PropertyEventOwnerKind::Column, FormBindingOwner::Column(_))
            | (PropertyEventOwnerKind::Command, FormBindingOwner::Command) => true,
            (PropertyEventOwnerKind::UnresolvedItem, _) => false,
            _ => false,
        };
        if !matches {
            return Err(EventProjectionError::unavailable(format!(
                "form evidence disagrees with the typed owner `{}`",
                expected.at
            )));
        }
    }
    Ok(())
}

pub fn refine_property_owner_chain(
    owner_chain: &mut [PropertyEventOwner],
    inputs: &FormSemanticInputs,
) -> Result<(), EventProjectionError> {
    for expected in owner_chain.iter_mut() {
        if expected.kind != PropertyEventOwnerKind::UnresolvedItem {
            continue;
        }
        let actual = inputs
            .owners
            .iter()
            .find(|owner| owner.at() == expected.at.to_string())
            .ok_or_else(|| {
                EventProjectionError::unavailable(format!(
                    "form evidence does not contain owner `{}`",
                    expected.at
                ))
            })?;
        expected.kind = match actual.owner() {
            FormBindingOwner::Element(_) => PropertyEventOwnerKind::Element,
            FormBindingOwner::Table => PropertyEventOwnerKind::Table,
            FormBindingOwner::Column(_) => PropertyEventOwnerKind::Column,
            _ => {
                return Err(EventProjectionError::unavailable(format!(
                    "Item `{}` has no exact form element evidence",
                    expected.at
                )))
            }
        };
    }
    validate_owner_chain(owner_chain, inputs)?;
    Ok(())
}

pub fn property_call_type(
    shape: &EventImplementationShape,
    definition: &FormDefinitionKind,
    direct_part_writable: bool,
    requested: Option<FormCallType>,
) -> Result<Option<FormCallType>, ApplyPlanError> {
    match shape.projection.state {
        EventState::Available if !direct_part_writable => Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "BaseForm-only Part 1 has no writable direct owner",
        )),
        EventState::Available if *definition == FormDefinitionKind::Extension => {
            requested.map(Some).ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    "borrowed Property Available requires callType",
                )
            })
        }
        EventState::Available if requested.is_some() => Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "regular Property Available forbids callType",
        )),
        EventState::Available => Ok(None),
        EventState::Missing if requested.is_some() => Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "Property Missing forbids a new callType",
        )),
        EventState::Missing => Ok(shape
            .projection
            .call_type
            .as_deref()
            .and_then(FormCallType::from_xml)),
        EventState::Implemented | EventState::Invalid => Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "event state is not plannable",
        )),
    }
}

pub fn require_plannable_state(shape: &EventImplementationShape) -> Result<(), ApplyPlanError> {
    match shape.projection.state {
        EventState::Available | EventState::Missing => Ok(()),
        EventState::Implemented | EventState::Invalid => Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!("event is {:?}", shape.projection.state),
        )),
    }
}

pub fn require_postcondition(
    before: &EventImplementationShape,
    after: &EventImplementationShape,
    call_type: Option<FormCallType>,
) -> Result<(), ApplyPlanError> {
    let expected_call_type = call_type.map(FormCallType::as_str);
    if after.projection.state != EventState::Implemented
        || after.projection.at != before.projection.at
        || after.projection.binding != before.projection.binding
        || after.projection.handler != before.handler
        || after.projection.call_type.as_deref() != expected_call_type
        || after.projection.implementation_at.as_deref() != Some(before.implementation_at.as_str())
    {
        return Err(postcondition(format!(
            "event postimage did not reproject to the exact Implemented identity: expected at={} handler={} callType={expected_call_type:?}; actual state={:?} at={} handler={} callType={:?} implementationAt={:?}",
            before.projection.at,
            before.handler,
            after.projection.state,
            after.projection.at,
            after.projection.handler,
            after.projection.call_type,
            after.projection.implementation_at,
        )));
    }
    Ok(())
}

fn postcondition(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message)
}

pub fn project_property_event(
    event_at: &QualifiedAddress,
    form_at: &QualifiedAddress,
    owner_chain: &[PropertyEventOwner],
    mut evidence: FormEventEvidence,
    module_text: Option<&str>,
) -> Result<EventProjection, EventProjectionError> {
    evidence.context.metadata_owner = form_at
        .segments()
        .first()
        .map(unica_address::address::AddressSegment::kind);
    let inputs = form_semantic_inputs(&form_at.to_string(), &evidence);
    validate_owner_chain(owner_chain, &inputs)?;
    project_property_events(form_at, &evidence, module_text)?
        .into_iter()
        .find(|event| event.at == event_at.to_string())
        .ok_or_else(|| {
            EventProjectionError::unavailable(format!(
                "event `{}` is not applicable in the frozen form catalog",
                event_at
            ))
        })
}

#[cfg(test)]
mod directive_tests {
    use super::*;
    fn spec(contexts: &[&str]) -> PlatformEventSpec {
        PlatformEventSpec {
            event_id: "Fixture".to_string(),
            handler_ru: "Fixture".to_string(),
            handler_en: "Fixture".to_string(),
            method_kind: "procedure".to_string(),
            signature_ru: "Процедура Fixture()".to_string(),
            signature_en: "Procedure Fixture()".to_string(),
            contexts: contexts.iter().map(|value| (*value).to_string()).collect(),
            source_page_id: "fixture".to_string(),
            binding: unica_metadata::event::BindingFact::Property,
        }
    }

    #[test]
    fn required_directive_fails_closed_for_empty_mixed_and_unknown_contexts() {
        for contexts in [
            Vec::<&str>::new(),
            vec!["thinClient", "server"],
            vec!["unclassified"],
        ] {
            let error = required_event_directive(
                EventDirectiveOwner::Property(FormBindingOwner::Form),
                &spec(&contexts),
            )
            .unwrap_err();
            assert_eq!(error.code().as_str(), "provider_unavailable");
            let mut dynamic = spec(&contexts);
            dynamic.event_id = "OnGetDataAtServer".into();
            assert!(required_event_directive(
                EventDirectiveOwner::Property(FormBindingOwner::Table),
                &dynamic
            )
            .is_err());
        }
        assert_eq!(
            required_event_directive(
                EventDirectiveOwner::Property(FormBindingOwner::Form),
                &spec(&["server"]),
            )
            .unwrap(),
            RequiredEventDirective::Server
        );
        let mut dynamic = spec(&["server"]);
        dynamic.event_id = "OnGetDataAtServer".into();
        assert_eq!(
            required_event_directive(
                EventDirectiveOwner::Property(FormBindingOwner::Table),
                &dynamic,
            )
            .unwrap(),
            RequiredEventDirective::ServerOrServerNoContext
        );
        assert_eq!(
            required_event_directive(
                EventDirectiveOwner::Property(FormBindingOwner::Form),
                &dynamic,
            )
            .unwrap(),
            RequiredEventDirective::Server
        );
    }
}
