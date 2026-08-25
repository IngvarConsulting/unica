use crate::domain::address::QualifiedAddress;
use crate::domain::module_projection::{EventProjection, ModuleProjectionSet};
use crate::domain::platform_profile::{ModuleCapability, PlatformProfile};
use crate::domain::project_sources::SourceSetKind;
use crate::infrastructure::bsl_module_projection::{
    project_form_owner_events, project_module, FormBindingOwner, FormEventBindingInput,
    FormEventOwnerInput, FormMethodFact, ModuleProjectionRequest, PlatformEventWriteCapability,
};
use crate::infrastructure::logical_event_source::{
    resolve_event_source_for_form_evidence, PlatformEventSource, PropertyEventOwnerKind,
    PropertyEventSource,
};
use crate::infrastructure::native_operations::form::{
    parse_form_event_evidence_xml, FormEventEvidence, FormInfoElement, FormInfoEvent,
};
use crate::infrastructure::native_operations::form_event_registry::FormElementKind;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventProjectionError {
    message: String,
}

impl EventProjectionError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> &'static str {
        "provider_unavailable"
    }
}

impl fmt::Display for EventProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for EventProjectionError {}

pub(crate) struct FormSemanticInputs {
    pub(crate) owners: Vec<FormEventOwnerInput>,
    pub(crate) bindings: Vec<FormEventBindingInput>,
}

pub(crate) fn project_platform_event(
    source: &PlatformEventSource,
    capability: ModuleCapability,
    module_text: Option<&str>,
) -> Result<EventProjection, EventProjectionError> {
    let projection = project_module(ModuleProjectionRequest {
        at: &source.module_at,
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
    .map_err(EventProjectionError::unavailable)?;
    project_platform_event_from_projection(source, &projection)
}

pub(crate) fn project_platform_event_from_projection(
    source: &PlatformEventSource,
    projection: &ModuleProjectionSet,
) -> Result<EventProjection, EventProjectionError> {
    projection
        .events()
        .iter()
        .find(|event| event.at == source.event_at.to_string())
        .cloned()
        .ok_or_else(|| {
            EventProjectionError::unavailable(format!(
                "event `{}` is not in the frozen module catalog",
                source.event_at
            ))
        })
}

pub(crate) fn project_property_event(
    source: &PropertyEventSource,
    form_xml: &str,
    module_text: Option<&str>,
) -> Result<EventProjection, EventProjectionError> {
    let mut evidence =
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::unavailable)?;
    evidence.context.metadata_owner = source
        .form_at
        .segments()
        .first()
        .map(crate::domain::address::AddressSegment::kind);
    let inputs = form_semantic_inputs(&source.form_at.to_string(), &evidence);
    validate_owner_chain(source, &inputs)?;
    project_property_events(&source.form_at, &evidence, module_text)?
        .into_iter()
        .find(|event| event.at == source.event_at.to_string())
        .ok_or_else(|| {
            EventProjectionError::unavailable(format!(
                "event `{}` is not applicable in the frozen form catalog",
                source.event_at
            ))
        })
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
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::unavailable)?;
    evidence.context.metadata_owner = source
        .form_at
        .segments()
        .first()
        .map(crate::domain::address::AddressSegment::kind);
    let inputs = form_semantic_inputs(&source.form_at.to_string(), &evidence);
    for expected in &mut source.owner_chain {
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
            _ => {
                return Err(EventProjectionError::unavailable(format!(
                    "direct Item `{}` is neither an element nor a table",
                    expected.at
                )))
            }
        };
    }
    validate_owner_chain(&source, &inputs)?;
    Ok(source)
}

pub(crate) fn project_property_events(
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
    .map_err(EventProjectionError::unavailable)?;
    project_property_events_from_projection(evidence, &inputs, &module)
}

pub(crate) fn project_property_events_from_projection(
    evidence: &FormEventEvidence,
    inputs: &FormSemanticInputs,
    module: &ModuleProjectionSet,
) -> Result<Vec<EventProjection>, EventProjectionError> {
    let methods = module
        .methods()
        .iter()
        .map(|method| {
            FormMethodFact::new(
                &method.name,
                method.method_kind,
                &method.signature,
                method.compile.contexts.iter().map(String::as_str).collect(),
            )
            .with_directive(method.compile.directive.as_deref())
        })
        .collect::<Vec<_>>();
    Ok(project_form_owner_events(
        &evidence.context,
        &inputs.owners,
        &inputs.bindings,
        &methods,
    ))
}

pub(crate) fn form_semantic_inputs(
    form_at: &str,
    evidence: &FormEventEvidence,
) -> FormSemanticInputs {
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
            let owner = if parent_is_table {
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

fn validate_owner_chain(
    source: &PropertyEventSource,
    inputs: &FormSemanticInputs,
) -> Result<(), EventProjectionError> {
    let expected = source.owner_chain.last().ok_or_else(|| {
        EventProjectionError::unavailable("property event source has no owner chain")
    })?;
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
    matches.then_some(()).ok_or_else(|| {
        EventProjectionError::unavailable(format!(
            "form evidence disagrees with the typed owner `{}`",
            expected.at
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::bsl_module_projection::{
        required_event_directive, EventDirectiveOwner, RequiredEventDirective,
    };
    use crate::infrastructure::native_operations::form_event_registry::PlatformEventSpec;

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
            binding: crate::domain::module_projection::BindingFact::Property,
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
            assert_eq!(error.code(), "provider_unavailable");
        }
        assert_eq!(
            required_event_directive(
                EventDirectiveOwner::Property(FormBindingOwner::Form),
                &spec(&["server"]),
            )
            .unwrap(),
            RequiredEventDirective::Server
        );
    }
}
