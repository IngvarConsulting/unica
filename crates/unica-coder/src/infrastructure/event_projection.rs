use crate::domain::address::QualifiedAddress;
use crate::domain::module_projection::{
    EventProjection, EventState, MethodKind, ModuleProjectionSet,
};
use crate::domain::platform_profile::{ModuleCapability, PlatformProfile};
use crate::domain::project_sources::SourceSetKind;
use crate::infrastructure::bsl_module_projection::{
    project_form_owner_event_record, project_form_owner_events, project_module,
    project_module_event_record, required_event_directive, EventDirectiveOwner, FormBindingOwner,
    FormEventBindingInput, FormEventOwnerInput, FormMethodFact, ModuleProjectionRequest,
    PlatformEventProjectionRecordError, PlatformEventWriteCapability, RequiredEventDirective,
};
use crate::infrastructure::logical_event_source::{
    resolve_event_source_for_form_evidence, PlatformEventSource, PropertyEventOwnerKind,
    PropertyEventSource,
};
use crate::infrastructure::native_operations::form::{
    parse_form_event_evidence_xml, FormEventEvidence, FormInfoElement, FormInfoEvent,
};
use crate::infrastructure::native_operations::form_event_registry::{
    FormElementKind, PlatformEventSpec,
};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventProjectionError {
    kind: EventProjectionErrorKind,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventProjectionErrorKind {
    ProviderUnavailable,
    InvalidSource,
}

impl EventProjectionError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: EventProjectionErrorKind::ProviderUnavailable,
            message: message.into(),
        }
    }

    fn invalid_source(message: impl Into<String>) -> Self {
        Self {
            kind: EventProjectionErrorKind::InvalidSource,
            message: message.into(),
        }
    }

    pub(crate) const fn kind(&self) -> EventProjectionErrorKind {
        self.kind
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventImplementationOwner {
    Platform,
    Form,
    Element,
    Table,
    Column,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventImplementationShape {
    pub(crate) projection: EventProjection,
    pub(crate) owner: EventImplementationOwner,
    pub(crate) method_kind: MethodKind,
    pub(crate) handler: String,
    pub(crate) implementation_at: String,
    pub(crate) signature: String,
    pub(crate) directive: RequiredEventDirective,
    pub(crate) region: &'static str,
}

pub(crate) fn project_platform_event_implementation(
    source: &PlatformEventSource,
    capability: ModuleCapability,
    module_text: Option<&str>,
) -> Result<EventImplementationShape, EventProjectionError> {
    let record = project_module_event_record(
        ModuleProjectionRequest {
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
        },
        &source.event_at.to_string(),
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
        &source.module_at,
        EventDirectiveOwner::Platform(capability),
        record.spec,
    )
}

pub(crate) fn project_property_event_implementation(
    source: &PropertyEventSource,
    form_xml: &str,
    module_text: Option<&str>,
) -> Result<EventImplementationShape, EventProjectionError> {
    let mut evidence =
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::invalid_source)?;
    evidence.context.metadata_owner = source
        .form_at
        .segments()
        .first()
        .map(crate::domain::address::AddressSegment::kind);
    let inputs = form_semantic_inputs(&source.form_at.to_string(), &evidence);
    validate_owner_chain(source, &inputs)?;
    let capability = PlatformProfile::v8_3_27()
        .module_capability(&source.module_at)
        .ok_or_else(|| EventProjectionError::unavailable("form module capability is absent"))?;
    let module = project_module(ModuleProjectionRequest {
        at: &source.module_at,
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
    let owner_at = source
        .owner_chain
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
        &source.event_at.to_string(),
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
            let leaf = source
                .owner_chain
                .last()
                .and_then(|owner| owner.at.segments().last())
                .and_then(crate::domain::address::AddressSegment::name)
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
        &source.module_at,
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
                &method.signature,
                method.compile.contexts.iter().map(String::as_str).collect(),
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
    let signature = substitute_catalog_handler(&spec.signature_ru, &spec.handler_ru, &handler)?;
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
    .map_err(EventProjectionError::invalid_source)?;
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
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::invalid_source)?;
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
        parse_form_event_evidence_xml(form_xml).map_err(EventProjectionError::invalid_source)?;
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
            FormBindingOwner::Column(_) => PropertyEventOwnerKind::Column,
            _ => {
                return Err(EventProjectionError::unavailable(format!(
                    "Item `{}` has no exact form element evidence",
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
    .map_err(EventProjectionError::invalid_source)?;
    project_property_events_from_projection(evidence, &inputs, &module)
}

pub(crate) fn project_property_events_from_projection(
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
    if source.owner_chain.is_empty() {
        return Err(EventProjectionError::unavailable(
            "property event source has no owner chain",
        ));
    }
    for expected in &source.owner_chain {
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

    #[test]
    fn implementation_shape_uses_typed_owner_leaf_and_the_selected_catalog_record() {
        let form_xml = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
            <AutoCommandBar name="Bar" id="-1"/>
            <ChildItems>
                <InputField name="Field" id="1"/>
                <Table name="Goods" id="2"><ChildItems>
                    <InputField name="Quantity" id="3"/>
                </ChildItems></Table>
            </ChildItems>
            <Commands><Command name="Заполнить"/></Commands>
        </Form>"#;
        let cases = [
            (
                "main:Catalog.Products.Form.Main.Event.OnOpen",
                EventImplementationOwner::Form,
                "ПриОткрытии",
                "ОбработчикиСобытийФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Item.Field.Event.OnChange",
                EventImplementationOwner::Element,
                "FieldПриИзменении",
                "ОбработчикиСобытийЭлементовФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Item.Goods.Event.BeforeAddRow",
                EventImplementationOwner::Table,
                "GoodsПередНачаломДобавления",
                "ОбработчикиСобытийЭлементовФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Item.Goods.Item.Quantity.Event.OnChange",
                EventImplementationOwner::Column,
                "QuantityПриИзменении",
                "ОбработчикиСобытийЭлементовФормы",
            ),
            (
                "main:Catalog.Products.Form.Main.Command.Заполнить.Event.Execute",
                EventImplementationOwner::Command,
                "ЗаполнитьОбработкаКоманды",
                "ОбработчикиКомандФормы",
            ),
        ];
        for (at, owner, handler, region) in cases {
            let at = QualifiedAddress::parse(at).unwrap();
            let source = resolve_property_event_source(
                "main",
                SourceSetKind::Configuration,
                PlatformProfile::v8_3_27(),
                &at,
                form_xml,
            )
            .unwrap();
            let shape = project_property_event_implementation(&source, form_xml, None).unwrap();
            assert_eq!(shape.owner, owner, "{at}");
            assert_eq!(shape.handler, handler, "{at}");
            assert_eq!(
                shape.implementation_at,
                format!("{}.Method.{handler}", source.module_at),
                "{at}"
            );
            assert!(
                shape.signature.contains(handler),
                "{at}: {}",
                shape.signature
            );
            assert_eq!(shape.region, region, "{at}");
            assert_eq!(shape.method_kind, MethodKind::Procedure, "{at}");
        }
    }
}
