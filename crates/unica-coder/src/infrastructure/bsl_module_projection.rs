use crate::domain::address::QualifiedAddress;
use crate::domain::apply::OperationRegistry;
use crate::domain::module_projection::{
    BindingFact, BodyLine, CommonModuleProperties, CompilationFacts, CompilationProjection,
    EventProjection, EventState, ExtensionKind, ExtensionProjection, HandleProjection,
    InterfaceKind, InterfaceProjection, MethodKind, MethodParameter, MethodProjection,
    ModuleIdentity, ModuleProjectionSet, ModuleProperties, RegionProjection,
};
use crate::domain::platform_profile::{ModuleCapability, ModuleRole, PlatformProfile};
use crate::domain::refusal::RefusalCode;
use crate::infrastructure::bsl_outline::parse_bsl_syntax;
use crate::infrastructure::native_operations::form_event_registry::{
    foreign_handler_signature_8_3_27, form_event_catalog_8_3_27,
    form_event_handler_signature_8_3_27, form_owner_event_parameters_8_3_27,
    module_event_catalog_8_3_27, validate_property_event_binding, FormElementKind,
    FormEventBinding, FormEventContext, FormEventOwnerKind, FormEventParameterForms,
    FormEventTarget, PlatformEventSpec,
};
use bsl_syntax::ast::{Annotation, AstNode, FunctionDef, PreIfDir, PreRegionDir, ProcedureDef};
use bsl_syntax::{SyntaxKind, SyntaxNode};
use std::cell::OnceCell;
use std::collections::HashMap;

const PLATFORM_CONTEXTS: &[&str] = &[
    "server",
    "externalConnection",
    "thinClient",
    "webClient",
    "thickClientManaged",
    "thickClientOrdinary",
    "mobileClient",
    "mobileAppClient",
    "mobileAppServer",
    "mobileStandaloneServer",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormBindingOwner {
    Form,
    Element(FormElementKind),
    Table,
    Column(FormElementKind),
    Command,
}

impl FormBindingOwner {
    pub(crate) fn parse(raw: &str, element_kind: Option<FormElementKind>) -> Option<Self> {
        match raw {
            "form" => Some(Self::Form),
            "element" => element_kind.map(Self::Element),
            "table" => Some(Self::Table),
            "column" => element_kind.map(Self::Column),
            "command" => Some(Self::Command),
            _ => None,
        }
    }

    pub(crate) fn catalog_owner(self) -> FormEventOwnerKind {
        match self {
            Self::Form => FormEventOwnerKind::Form,
            Self::Element(kind) => FormEventOwnerKind::Element(kind),
            Self::Table => FormEventOwnerKind::Table,
            Self::Column(kind) => FormEventOwnerKind::Column(kind),
            Self::Command => FormEventOwnerKind::Command,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Form => "form",
            Self::Element(_) => "item",
            Self::Table => "formTable",
            Self::Column(_) => "column",
            Self::Command => "command",
        }
    }

    fn validation_target(self) -> FormEventTarget {
        match self {
            Self::Form => FormEventTarget::Form,
            Self::Element(kind) | Self::Column(kind) => FormEventTarget::Element(kind),
            Self::Table => FormEventTarget::Element(FormElementKind::Table),
            // Command bindings share the extension call-type rule. The target
            // is diagnostic-only for this validation seam.
            Self::Command => FormEventTarget::Form,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormEventBindingInput {
    owner: FormBindingOwner,
    at: String,
    event: String,
    handler: String,
    call_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormEventOwnerInput {
    owner: FormBindingOwner,
    at: String,
    data_path: Option<String>,
}

impl FormEventOwnerInput {
    pub(crate) fn new(owner: FormBindingOwner, at: &str) -> Self {
        Self {
            owner,
            at: at.to_string(),
            data_path: None,
        }
    }

    #[must_use]
    pub(crate) fn with_data_path(mut self, data_path: &str) -> Self {
        self.data_path = Some(data_path.to_string());
        self
    }

    pub(crate) const fn owner(&self) -> FormBindingOwner {
        self.owner
    }

    pub(crate) fn at(&self) -> &str {
        &self.at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormMethodFact<'a> {
    pub(crate) name: &'a str,
    pub(crate) method_kind: MethodKind,
    pub(crate) parameters: &'a [MethodParameter],
    pub(crate) directive: Option<&'a str>,
    pub(crate) contexts: Vec<&'a str>,
}

impl<'a> FormMethodFact<'a> {
    pub(crate) fn new(
        name: &'a str,
        method_kind: MethodKind,
        parameters: &'a [MethodParameter],
        contexts: Vec<&'a str>,
    ) -> Self {
        Self {
            name,
            method_kind,
            parameters,
            directive: None,
            contexts,
        }
    }

    #[must_use]
    pub(crate) const fn with_directive(mut self, directive: Option<&'a str>) -> Self {
        self.directive = directive;
        self
    }
}

impl FormEventBindingInput {
    pub(crate) fn property(
        owner: FormBindingOwner,
        at: &str,
        event: &str,
        handler: &str,
        call_type: Option<&str>,
    ) -> Self {
        Self {
            owner,
            at: at.to_string(),
            event: event.to_string(),
            handler: handler.to_string(),
            call_type: call_type.map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeclarativeBinding<'a> {
    pub(crate) owner: &'a str,
    pub(crate) at: &'a str,
    pub(crate) event: &'a str,
    pub(crate) handler: &'a str,
}

pub(crate) struct ModuleProjectionRequest<'a> {
    pub(crate) at: &'a QualifiedAddress,
    pub(crate) capability: ModuleCapability,
    pub(crate) title: String,
    pub(crate) rev: &'a str,
    pub(crate) source: Option<&'a str>,
    pub(crate) common_module: Option<CommonModuleProperties>,
    pub(crate) handles: &'a [FormEventBindingInput],
    pub(crate) declarative_bindings: &'a [DeclarativeBinding<'a>],
    pub(crate) extension_targets: &'a [(&'a str, &'a str)],
    pub(crate) platform_event_write: PlatformEventWriteCapability,
}

pub(crate) struct PlatformEventProjectionRecord {
    pub(crate) projection: EventProjection,
    pub(crate) spec: &'static PlatformEventSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlatformEventProjectionRecordError {
    InvalidSource(String),
    ProviderUnavailable(String),
}

impl std::fmt::Display for PlatformEventProjectionRecordError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSource(message) | Self::ProviderUnavailable(message) => {
                message.fmt(formatter)
            }
        }
    }
}

/// Typed proof status for generating a Platform-bound event capability. An
/// extension source set currently has no retained `PropertyState` evidence to
/// distinguish a New owner from an adopted/extended owner, so it is unproved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformEventWriteCapability {
    Proven,
    Unproved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequiredEventDirective {
    None,
    Client,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectiveRuleError {
    message: String,
}

impl DirectiveRuleError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> RefusalCode {
        RefusalCode::ProviderUnavailable
    }
}

impl std::fmt::Display for DirectiveRuleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for DirectiveRuleError {}

impl RequiredEventDirective {
    pub(crate) const fn canonical(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Client => Some("&НаКлиенте"),
            Self::Server => Some("&НаСервере"),
        }
    }
}

pub(crate) fn project_module(
    request: ModuleProjectionRequest<'_>,
) -> Result<ModuleProjectionSet, String> {
    let specs = module_event_catalog_8_3_27(request.capability);
    project_module_with_specs(request, specs)
}

pub(crate) fn project_module_event_record(
    request: ModuleProjectionRequest<'_>,
    event_at: &str,
) -> Result<PlatformEventProjectionRecord, PlatformEventProjectionRecordError> {
    let specs = module_event_catalog_8_3_27(request.capability);
    let projection = project_module_with_specs(request, specs)
        .map_err(PlatformEventProjectionRecordError::InvalidSource)?;
    let index = projection
        .events()
        .iter()
        .position(|event| event.at == event_at)
        .ok_or_else(|| {
            PlatformEventProjectionRecordError::ProviderUnavailable(format!(
                "event `{event_at}` is not in the frozen module catalog"
            ))
        })?;
    let event = projection.events()[index].clone();
    let spec = specs.get(index).ok_or_else(|| {
        PlatformEventProjectionRecordError::ProviderUnavailable(
            "module event projection lost the selected frozen catalog record".to_string(),
        )
    })?;
    if event.event_id != spec.event_id {
        return Err(PlatformEventProjectionRecordError::ProviderUnavailable(
            "module event projection/catalog ordering diverged".to_string(),
        ));
    }
    Ok(PlatformEventProjectionRecord {
        projection: event,
        spec,
    })
}

fn project_module_with_specs(
    request: ModuleProjectionRequest<'_>,
    possible_events: &'static [PlatformEventSpec],
) -> Result<ModuleProjectionSet, String> {
    if request.capability.role() == ModuleRole::Common && request.common_module.is_none() {
        return Err(
            "common-module flags are required to build the normalized module summary".to_string(),
        );
    }
    let props = request.common_module.clone().map_or_else(
        || ModuleProperties::new(request.capability.owner_kind(), request.capability.role()),
        ModuleProperties::common,
    );
    let identity = ModuleIdentity {
        at: request.at.clone(),
        title: request.title,
        props,
        rev: request.rev.to_string(),
    };

    let Some(source) = request.source else {
        let events = possible_events
            .iter()
            .map(|spec| available_module_event(request.at, spec, request.platform_event_write))
            .collect();
        return Ok(ModuleProjectionSet::new(
            identity,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            events,
            Vec::new(),
            Vec::new(),
        ));
    };

    if source.len() > u32::MAX as usize {
        return Err("BSL module is too large for the analyzer parser".to_string());
    }
    let parsed = parse_bsl_syntax(source);
    if !parsed.errors().is_empty() {
        return Err(format!(
            "BSL module projection failed because the parser reported {} diagnostic(s)",
            parsed.errors().len()
        ));
    }
    let root = parsed.syntax_node();
    let lines = LineIndex::new(source);
    let compilation = compilation_projection(
        &root,
        &lines,
        request.capability,
        request.common_module.as_ref(),
    );
    let base_contexts = base_contexts(request.capability, request.common_module.as_ref());
    let mut methods = method_projections(
        &root,
        source,
        &lines,
        request.at,
        request.capability,
        base_contexts.as_deref(),
        &compilation,
        request.handles,
        request.declarative_bindings,
        request.extension_targets,
    )?;
    let regions = region_projections(&root, &lines, request.at, &methods);
    let interfaces = interface_projections(request.at, &methods, &regions);
    let events = project_module_events(
        request.at,
        request.capability,
        possible_events,
        &mut methods,
        request.platform_event_write,
    );
    let body = source
        .lines()
        .enumerate()
        .map(|(index, text)| BodyLine {
            line: index + 1,
            text: text.to_string(),
        })
        .collect();

    Ok(ModuleProjectionSet::new(
        identity,
        methods,
        regions,
        interfaces,
        events,
        compilation,
        body,
    ))
}

fn available_module_event(
    at: &QualifiedAddress,
    spec: &PlatformEventSpec,
    write: PlatformEventWriteCapability,
) -> EventProjection {
    let event_at = format!("{at}.Event.{}", spec.event_id);
    EventProjection {
        at: event_at.clone(),
        event_id: spec.event_id.to_string(),
        state: EventState::Available,
        signature: spec.signature_ru.clone(),
        contexts: spec.contexts.clone(),
        binding: spec.binding,
        handler: spec.handler_ru.clone(),
        handler_en: spec.handler_en.clone(),
        implementation_at: None,
        call_type: None,
        can: (write == PlatformEventWriteCapability::Proven)
            .then(|| OperationRegistry::closed().event_implementation_skeleton(event_at, None))
            .into_iter()
            .collect(),
    }
}

fn project_module_events(
    at: &QualifiedAddress,
    capability: ModuleCapability,
    specs: &[PlatformEventSpec],
    methods: &mut [MethodProjection],
    write: PlatformEventWriteCapability,
) -> Vec<EventProjection> {
    specs
        .iter()
        .map(|spec| {
            let matching = methods
                .iter()
                .enumerate()
                .filter(|(_, method)| event_handler_name_matches(&method.name, spec))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let Some(&method_index) = matching.first() else {
                return available_module_event(at, spec, write);
            };
            let duplicate = matching.len() > 1;
            let method = &mut methods[method_index];
            let compatible = method_matches_event(
                method.method_kind,
                &method.signature,
                method.compile.contexts.as_deref(),
                method.compile.directive.as_deref(),
                capability,
                spec,
            );
            let event_at = format!("{at}.Event.{}", spec.event_id);
            method.handles.push(HandleProjection {
                owner: "module".to_string(),
                event: spec.event_id.to_string(),
                at: event_at.clone(),
                binding: BindingFact::Platform,
                call_type: None,
            });
            EventProjection {
                at: event_at,
                event_id: spec.event_id.to_string(),
                state: if compatible && !duplicate {
                    EventState::Implemented
                } else {
                    EventState::Invalid
                },
                signature: spec.signature_ru.clone(),
                contexts: spec.contexts.clone(),
                binding: spec.binding,
                handler: spec.handler_ru.clone(),
                handler_en: spec.handler_en.clone(),
                implementation_at: Some(method.at.clone()),
                call_type: None,
                can: Vec::new(),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn method_projections(
    root: &SyntaxNode,
    source: &str,
    lines: &LineIndex,
    at: &QualifiedAddress,
    capability: ModuleCapability,
    base_contexts: Option<&[String]>,
    compilation: &[CompilationProjection],
    handles: &[FormEventBindingInput],
    declarative_bindings: &[DeclarativeBinding<'_>],
    extension_targets: &[(&str, &str)],
) -> Result<Vec<MethodProjection>, String> {
    let mut methods = Vec::new();
    for node in root.descendants() {
        let (name, method_kind, export) = if let Some(procedure) = ProcedureDef::cast(node.clone())
        {
            (
                procedure
                    .name_or_keyword()
                    .map(|token| token.text().to_string())
                    .unwrap_or_default(),
                MethodKind::Procedure,
                procedure.export_keyword().is_some(),
            )
        } else if let Some(function) = FunctionDef::cast(node.clone()) {
            (
                function
                    .name_or_keyword()
                    .map(|token| token.text().to_string())
                    .unwrap_or_default(),
                MethodKind::Function,
                function.export_keyword().is_some(),
            )
        } else {
            continue;
        };
        if name.is_empty() {
            return Err("BSL method has no parser-proven name".to_string());
        }
        let header_token = node
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| {
                matches!(
                    token.kind(),
                    SyntaxKind::KW_PROCEDURE | SyntaxKind::KW_FUNCTION
                )
            })
            .ok_or_else(|| format!("BSL method `{name}` has no declaration token"))?;
        let header_line = lines.line_of(u32::from(header_token.text_range().start()));
        let parameter_list = node
            .children()
            .find(|child| child.kind() == SyntaxKind::PARAM_LIST)
            .ok_or_else(|| format!("BSL method `{name}` has no parser-proven parameter list"))?;
        let parameters = crate::infrastructure::bsl_outline::parameters(&node)?
            .into_iter()
            .map(|parameter| MethodParameter {
                name: parameter.name,
                optional: parameter.default_value.is_some(),
            })
            .collect();
        let declaration_end = node
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == SyntaxKind::KW_EXPORT)
            .map_or(parameter_list.text_range().end(), |token| {
                token.text_range().end()
            });
        let signature = source
            [usize::from(header_token.text_range().start())..usize::from(declaration_end)]
            .trim()
            .to_string();
        let signature_end_line = lines.line_of(u32::from(declaration_end).saturating_sub(1));
        let annotations = node
            .children()
            .filter_map(Annotation::cast)
            .collect::<Vec<_>>();
        let directive = annotations
            .iter()
            .find(|annotation| annotation.syntax().kind() == SyntaxKind::COMPILER_DIRECTIVE)
            .map(|annotation| source_slice(source, annotation.syntax()).trim().to_string());
        let extension = annotations
            .iter()
            .find(|annotation| annotation.syntax().kind() == SyntaxKind::ANNOTATION)
            .and_then(|annotation| extension_projection(annotation, source, extension_targets));
        let guard = accumulated_guard(header_line, compilation);
        let contexts = base_contexts.map(|base| {
            let mut contexts = directive_contexts(directive.as_deref(), base);
            if let Some(guard) = guard.as_deref() {
                contexts.retain(|context| evaluate_guard(guard, context));
            }
            contexts
        });
        let node_from = lines.line_of(u32::from(node.text_range().start()));
        let node_to = lines.line_of(u32::from(node.text_range().end()).saturating_sub(1));
        let closing_line = node
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| {
                matches!(
                    token.kind(),
                    SyntaxKind::KW_END_PROCEDURE | SyntaxKind::KW_END_FUNCTION
                )
            })
            .map(|token| lines.line_of(u32::from(token.text_range().start())))
            .unwrap_or(node_to);
        // Annotations, the compilation directive and `Асинх` belong to the
        // method even when they stand on lines above the declaration keyword.
        let async_line = node
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == SyntaxKind::KW_ASYNC)
            .map(|token| lines.line_of(u32::from(token.text_range().start())));
        let from_line = annotations
            .iter()
            .map(|annotation| lines.line_of(u32::from(annotation.syntax().text_range().start())))
            .chain(async_line)
            .chain([header_line])
            .min()
            .unwrap_or(header_line);
        let body_from_line = signature_end_line.saturating_add(1);
        let body_to_line = closing_line.saturating_sub(1);
        let compilation_count = compilation
            .iter()
            .filter(|range| range.from_line <= node_to && range.to_line >= node_from)
            .count();
        let method_at = format!("{at}.Method.{name}");
        let mut method_handles = handles
            .iter()
            .filter(|binding| binding.handler.to_lowercase() == name.to_lowercase())
            .map(handle_from_form_binding)
            .collect::<Vec<_>>();
        method_handles.extend(
            declarative_bindings
                .iter()
                .filter(|binding| binding.handler.to_lowercase() == name.to_lowercase())
                .map(|binding| HandleProjection {
                    owner: binding.owner.to_string(),
                    event: binding.event.to_string(),
                    at: binding.at.to_string(),
                    binding: BindingFact::Property,
                    call_type: None,
                }),
        );
        let no_context = directive.as_deref().is_some_and(|value| {
            value.eq_ignore_ascii_case("&НаСервереБезКонтекста")
                || value.eq_ignore_ascii_case("&AtServerNoContext")
                || value.eq_ignore_ascii_case("&НаКлиентеНаСервереБезКонтекста")
                || value.eq_ignore_ascii_case("&AtClientAtServerNoContext")
        });
        let form_context = (capability.role() == ModuleRole::Form).then_some(!no_context);
        methods.push((
            node_from,
            MethodProjection {
                at: method_at,
                name,
                signature,
                parameters,
                doc: documentation_block(lines, header_line),
                method_kind,
                export,
                compile: CompilationFacts {
                    directive,
                    guard,
                    contexts,
                    form_context,
                    conditional_body: compilation.iter().any(|range| {
                        range.from_line >= body_from_line && range.from_line <= body_to_line
                    }),
                },
                handles: method_handles,
                extension,
                compilation_count,
                from_line,
                to_line: closing_line,
                body_from_line,
                body_to_line,
            },
        ));
    }
    methods.sort_by_key(|(line, _)| *line);
    Ok(methods.into_iter().map(|(_, method)| method).collect())
}

fn handle_from_form_binding(binding: &FormEventBindingInput) -> HandleProjection {
    HandleProjection {
        owner: binding.owner.as_str().to_string(),
        event: binding.event.to_string(),
        at: binding.at.to_string(),
        binding: BindingFact::Property,
        call_type: binding.call_type.clone(),
    }
}

fn extension_projection(
    annotation: &Annotation,
    source: &str,
    extension_targets: &[(&str, &str)],
) -> Option<ExtensionProjection> {
    let token = annotation.kind_token()?;
    let kind = match token.kind() {
        SyntaxKind::ANN_BEFORE => ExtensionKind::Before,
        SyntaxKind::ANN_AFTER => ExtensionKind::After,
        SyntaxKind::ANN_AROUND => ExtensionKind::Instead,
        SyntaxKind::ANN_CHANGE_AND_VALIDATE => ExtensionKind::ChangeAndValidate,
        _ => return None,
    };
    let directive = source_slice(source, annotation.syntax()).trim().to_string();
    let target = annotation
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::STRING)
        .map(|token| token.text().trim_matches('"').to_string());
    let target_at = target.as_deref().and_then(|target| {
        let matches = extension_targets
            .iter()
            .filter(|(name, _)| name.to_lowercase() == target.to_lowercase())
            .collect::<Vec<_>>();
        matches
            .as_slice()
            .first()
            .filter(|_| matches.len() == 1)
            .map(|(_, at)| (*at).to_string())
    });
    Some(ExtensionProjection {
        kind,
        directive,
        target_at,
    })
}

fn documentation_block(lines: &LineIndex<'_>, header_line: usize) -> Option<String> {
    let mut cursor = header_line.checked_sub(1)?;
    let mut result = Vec::new();
    loop {
        let text = lines.text(cursor)?.trim();
        if let Some(comment) = text.strip_prefix("//") {
            result.push(comment.strip_prefix(' ').unwrap_or(comment).to_string());
        } else if text.starts_with('&') || text.is_empty() {
            if result.is_empty() && cursor > 1 {
                cursor -= 1;
                continue;
            }
            break;
        } else {
            break;
        }
        if cursor == 1 {
            break;
        }
        cursor -= 1;
    }
    if result.is_empty() {
        None
    } else {
        result.reverse();
        Some(result.join("\n"))
    }
}

fn region_projections(
    root: &SyntaxNode,
    lines: &LineIndex<'_>,
    at: &QualifiedAddress,
    methods: &[MethodProjection],
) -> Vec<RegionProjection> {
    #[derive(Debug)]
    struct RawRegion {
        name: Option<String>,
        line: usize,
        end_line: Option<usize>,
        parent: Option<usize>,
    }

    let mut raw = Vec::<RawRegion>::new();
    let mut stack = Vec::<usize>::new();
    let mut markers = root
        .descendants()
        .filter_map(PreRegionDir::cast)
        .collect::<Vec<_>>();
    markers.sort_by_key(|marker| marker.syntax().text_range().start());
    for marker in markers {
        let line = lines.line_of(u32::from(marker.syntax().text_range().start()));
        if marker.is_start() {
            let index = raw.len();
            raw.push(RawRegion {
                name: marker.name(),
                line,
                end_line: None,
                parent: stack.last().copied(),
            });
            stack.push(index);
        } else if marker.is_end() {
            if let Some(index) = stack.pop() {
                raw[index].end_line = Some(line);
            }
        }
    }

    let mut sibling_counts = HashMap::<(Option<usize>, String), usize>::new();
    for region in &raw {
        if let Some(name) = &region.name {
            *sibling_counts
                .entry((region.parent, name.clone()))
                .or_default() += 1;
        }
    }
    let mut addresses = vec![None::<String>; raw.len()];
    for (index, region) in raw.iter().enumerate() {
        let addressable = region.name.as_ref().is_some_and(|name| {
            sibling_counts.get(&(region.parent, name.clone())) == Some(&1)
                && region
                    .parent
                    .is_none_or(|parent| addresses[parent].is_some())
        });
        if addressable {
            let prefix = region
                .parent
                .and_then(|parent| addresses[parent].as_ref().map(|at| format!("{at}.Region")))
                .unwrap_or_else(|| format!("{at}.Region"));
            addresses[index] = Some(format!("{prefix}.{}", region.name.as_deref().unwrap()));
        }
    }

    raw.iter()
        .enumerate()
        .map(|(index, region)| {
            let end = region.end_line.unwrap_or(usize::MAX);
            let method_addresses = methods
                .iter()
                .filter(|method| {
                    let declaration = method.body_from_line.saturating_sub(1);
                    declaration > region.line
                        && declaration < end
                        && !raw.iter().enumerate().any(|(child_index, child)| {
                            child_index != index
                                && child.parent == Some(index)
                                && declaration > child.line
                                && declaration < child.end_line.unwrap_or(usize::MAX)
                        })
                })
                .map(|method| method.at.clone())
                .collect();
            let children = raw
                .iter()
                .enumerate()
                .filter(|(_, child)| child.parent == Some(index))
                .filter_map(|(child_index, _)| addresses[child_index].clone())
                .collect();
            RegionProjection {
                at: addresses[index].clone(),
                name: region.name.clone(),
                addressable: addresses[index].is_some(),
                line: region.line,
                end_line: region.end_line,
                methods: method_addresses,
                children,
                parent: region.parent,
            }
        })
        .collect()
}

fn interface_projections(
    at: &QualifiedAddress,
    methods: &[MethodProjection],
    regions: &[RegionProjection],
) -> Vec<InterfaceProjection> {
    InterfaceKind::ALL
        .iter()
        .map(|interface| {
            let normative = match interface {
                InterfaceKind::Public => &["ПрограммныйИнтерфейс"][..],
                InterfaceKind::Library => {
                    &["ПрограммныйИнтерфейс", "СлужебныйПрограммныйИнтерфейс"][..]
                }
                InterfaceKind::Override => &["ПереопределяемыеПроцедурыИФункции"][..],
            };
            let selected = methods
                .iter()
                .filter(|method| {
                    let belongs = regions.iter().any(|region| {
                        region
                            .name
                            .as_deref()
                            .is_some_and(|name| normative.contains(&name))
                            && method_in_region_tree(method, region, regions)
                    });
                    belongs && (*interface == InterfaceKind::Override || method.export)
                })
                .map(|method| method.at.clone())
                .collect();
            InterfaceProjection {
                at: format!("{at}.Interface.{}", interface.as_str()),
                kind: "Interface",
                interface: *interface,
                methods: selected,
            }
        })
        .collect()
}

fn method_in_region_tree(
    method: &MethodProjection,
    region: &RegionProjection,
    regions: &[RegionProjection],
) -> bool {
    if region.methods.iter().any(|at| at == &method.at) {
        return true;
    }
    region.children.iter().any(|child_at| {
        regions
            .iter()
            .find(|candidate| candidate.at.as_deref() == Some(child_at))
            .is_some_and(|child| method_in_region_tree(method, child, regions))
    })
}

fn compilation_projection(
    root: &SyntaxNode,
    lines: &LineIndex<'_>,
    capability: ModuleCapability,
    common: Option<&CommonModuleProperties>,
) -> Vec<CompilationProjection> {
    let base = base_contexts(capability, common);
    let base = base.as_deref();
    let mut ranges = Vec::<CompilationProjection>::new();
    let mut nodes = root
        .descendants()
        .filter_map(PreIfDir::cast)
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.syntax().text_range().start());
    for node in nodes {
        let mut prior = Vec::<String>::new();
        if let Some(condition) = node.condition() {
            let guard = normalize_guard(condition.syntax());
            if let Some((from_line, to_line)) = node_range(node.then_body_nodes(), lines) {
                push_compilation_range(&mut ranges, from_line, to_line, &guard, base);
            }
            prior.push(guard);
        }
        for clause in node.elsif_clauses() {
            if let Some(condition) = clause.condition() {
                let condition = normalize_guard(condition.syntax());
                let local = format!("Not ({}) And {condition}", prior.join(" Or "));
                if let Some((from_line, to_line)) = node_range(clause.body_nodes(), lines) {
                    push_compilation_range(&mut ranges, from_line, to_line, &local, base);
                }
                prior.push(condition);
            }
        }
        if let Some(clause) = node.else_clause() {
            let local = format!("Not ({})", prior.join(" Or "));
            if let Some((from_line, to_line)) = node_range(clause.body_nodes(), lines) {
                push_compilation_range(&mut ranges, from_line, to_line, &local, base);
            }
        }
    }
    ranges.sort_by_key(|range| (range.from_line, range.to_line));
    let snapshot = ranges.clone();
    for range in &mut ranges {
        let ancestors = snapshot
            .iter()
            .filter(|candidate| {
                candidate.from_line < range.from_line && candidate.to_line >= range.to_line
            })
            .map(|candidate| candidate.guard.clone())
            .collect::<Vec<_>>();
        if !ancestors.is_empty() {
            range.guard = format!("{} And {}", ancestors.join(" And "), range.guard);
            range.contexts = guarded_contexts(base, &range.guard);
        }
    }
    ranges
}

fn push_compilation_range(
    ranges: &mut Vec<CompilationProjection>,
    from_line: usize,
    to_line: usize,
    guard: &str,
    base: Option<&[String]>,
) {
    ranges.push(CompilationProjection {
        from_line,
        to_line,
        guard: guard.to_string(),
        contexts: guarded_contexts(base, guard),
    });
}

fn guarded_contexts(base: Option<&[String]>, guard: &str) -> Option<Vec<String>> {
    base.map(|base| {
        base.iter()
            .filter(|context| evaluate_guard(guard, context))
            .cloned()
            .collect()
    })
}

fn node_range(
    nodes: impl Iterator<Item = SyntaxNode>,
    lines: &LineIndex<'_>,
) -> Option<(usize, usize)> {
    let nodes = nodes.collect::<Vec<_>>();
    let first = nodes.first()?;
    let last = nodes.last()?;
    Some((
        lines.line_of(u32::from(first.text_range().start())),
        lines.line_of(u32::from(last.text_range().end()).saturating_sub(1)),
    ))
}

fn accumulated_guard(line: usize, compilation: &[CompilationProjection]) -> Option<String> {
    compilation
        .iter()
        .filter(|range| line >= range.from_line && line <= range.to_line)
        .min_by_key(|range| range.to_line.saturating_sub(range.from_line))
        .map(|range| range.guard.clone())
}

fn normalize_guard(node: &SyntaxNode) -> String {
    let mut parts = Vec::new();
    for token in node
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
    {
        if token.kind().is_trivia() {
            continue;
        }
        let raw = token.text();
        let normalized = match raw.to_lowercase().as_str() {
            "клиент" | "client" => "Client",
            "сервер" | "server" => "Server",
            "вебклиент" | "webclient" => "WebClient",
            "тонкийклиент" | "thinclient" => "ThinClient",
            "толстыйклиент" | "thickclient" => "ThickClient",
            "внешнеесоединение" | "externalconnection" => "ExternalConnection",
            "мобильныйклиент" | "mobileclient" => "MobileClient",
            "мобильноеприложениеклиент" | "mobileappclient" => {
                "MobileAppClient"
            }
            "мобильноеприложениесервер" | "mobileappserver" => {
                "MobileAppServer"
            }
            "и" | "and" => "And",
            "или" | "or" => "Or",
            "не" | "not" => "Not",
            _ => raw,
        };
        parts.push(normalized.to_string());
    }
    parts.join(" ").replace("( ", "(").replace(" )", ")")
}

fn evaluate_guard(guard: &str, context: &str) -> bool {
    let tokens = guard
        .replace('(', " ( ")
        .replace(')', " ) ")
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut parser = GuardParser {
        tokens: &tokens,
        index: 0,
        context,
    };
    parser.parse_or()
}

struct GuardParser<'a> {
    tokens: &'a [String],
    index: usize,
    context: &'a str,
}

impl GuardParser<'_> {
    fn parse_or(&mut self) -> bool {
        let mut value = self.parse_and();
        while self.peek("Or") {
            self.index += 1;
            value |= self.parse_and();
        }
        value
    }

    fn parse_and(&mut self) -> bool {
        let mut value = self.parse_not();
        while self.peek("And") {
            self.index += 1;
            value &= self.parse_not();
        }
        value
    }

    fn parse_not(&mut self) -> bool {
        if self.peek("Not") {
            self.index += 1;
            return !self.parse_not();
        }
        if self.peek("(") {
            self.index += 1;
            let value = self.parse_or();
            if self.peek(")") {
                self.index += 1;
            }
            return value;
        }
        let symbol = self
            .tokens
            .get(self.index)
            .map(String::as_str)
            .unwrap_or("");
        self.index += usize::from(self.index < self.tokens.len());
        context_matches_symbol(self.context, symbol)
    }

    fn peek(&self, token: &str) -> bool {
        self.tokens
            .get(self.index)
            .is_some_and(|value| value == token)
    }
}

fn context_matches_symbol(context: &str, symbol: &str) -> bool {
    match symbol {
        "Client" => matches!(
            context,
            "thinClient"
                | "webClient"
                | "thickClientManaged"
                | "thickClientOrdinary"
                | "mobileClient"
                | "mobileAppClient"
        ),
        "Server" => matches!(
            context,
            "server" | "externalConnection" | "mobileAppServer" | "mobileStandaloneServer"
        ),
        "WebClient" => context == "webClient",
        "ThinClient" => context == "thinClient",
        "ThickClient" => matches!(context, "thickClientManaged" | "thickClientOrdinary"),
        "ExternalConnection" => context == "externalConnection",
        "MobileClient" => context == "mobileClient",
        "MobileAppClient" => context == "mobileAppClient",
        "MobileAppServer" => context == "mobileAppServer",
        _ => false,
    }
}

/// `None` — контексты неизвестны: у заимствованного общего модуля
/// не записан хотя бы один флаг среды, а значение родителя здесь не читается.
fn base_contexts(
    capability: ModuleCapability,
    common: Option<&CommonModuleProperties>,
) -> Option<Vec<String>> {
    let raw: Vec<&str> = match capability.role() {
        ModuleRole::Common => {
            let Some(common) = common else {
                return Some(
                    PLATFORM_CONTEXTS
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                );
            };
            if !common.execution_environment_known() {
                return None;
            }
            let mut contexts = Vec::new();
            if common.client_managed_application == Some(true) {
                contexts.extend([
                    "thinClient",
                    "webClient",
                    "thickClientManaged",
                    "mobileClient",
                    "mobileAppClient",
                ]);
            }
            if common.client_ordinary_application == Some(true) {
                contexts.push("thickClientOrdinary");
            }
            if common.server == Some(true) {
                contexts.extend(["server", "mobileAppServer", "mobileStandaloneServer"]);
            }
            if common.external_connection == Some(true) {
                contexts.push("externalConnection");
            }
            return Some(contexts.into_iter().map(str::to_string).collect());
        }
        ModuleRole::Form => PLATFORM_CONTEXTS.to_vec(),
        ModuleRole::Command => vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "thickClientOrdinary",
            "mobileClient",
            "mobileAppClient",
        ],
        ModuleRole::ManagedApplication => vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ],
        ModuleRole::OrdinaryApplication => vec!["thickClientOrdinary"],
        ModuleRole::Session => vec!["server", "externalConnection"],
        ModuleRole::ExternalConnection => vec!["externalConnection"],
        ModuleRole::Object
        | ModuleRole::Manager
        | ModuleRole::RecordSet
        | ModuleRole::ValueManager => vec![
            "server",
            "externalConnection",
            "thickClientOrdinary",
            "mobileAppServer",
            "mobileStandaloneServer",
        ],
        ModuleRole::HttpService
        | ModuleRole::WebService
        | ModuleRole::IntegrationService
        | ModuleRole::Bot
        | ModuleRole::WebSocketClient => vec!["server"],
    };
    Some(raw.into_iter().map(str::to_string).collect())
}

fn directive_contexts(directive: Option<&str>, base: &[String]) -> Vec<String> {
    let Some(directive) = directive else {
        return base.to_vec();
    };
    let mode = directive.to_lowercase();
    base.iter()
        .filter(|context| {
            if matches!(mode.as_str(), "&наклиенте" | "&atclient") {
                context_matches_symbol(context, "Client")
            } else if matches!(
                mode.as_str(),
                "&насервере" | "&atserver" | "&насерверебезконтекста" | "&atservernocontext"
            ) {
                context_matches_symbol(context, "Server")
            } else if matches!(
                mode.as_str(),
                "&наклиентенасервере"
                    | "&atclientatserver"
                    | "&наклиентенасерверебезконтекста"
                    | "&atclientatservernocontext"
            ) {
                context_matches_symbol(context, "Client")
                    || context_matches_symbol(context, "Server")
            } else {
                true
            }
        })
        .cloned()
        .collect()
}

fn source_slice<'a>(source: &'a str, node: &SyntaxNode) -> &'a str {
    let range = node.text_range();
    &source[usize::from(range.start())..usize::from(range.end())]
}

fn normalized_declaration(signature: &str) -> String {
    signature
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn event_handler_name_matches(name: &str, spec: &PlatformEventSpec) -> bool {
    let name = name.to_lowercase();
    name == spec.handler_ru.to_lowercase() || name == spec.handler_en.to_lowercase()
}

fn method_matches_event(
    method_kind: MethodKind,
    signature: &str,
    contexts: Option<&[String]>,
    directive: Option<&str>,
    capability: ModuleCapability,
    spec: &PlatformEventSpec,
) -> bool {
    // Контексты неизвестны только у заимствованного общего модуля, а у него
    // нет событий платформы: ветка не исполняется и лишь не выдаёт
    // совместимость без доказательства.
    let Some(contexts) = contexts else {
        return false;
    };
    let kind = match method_kind {
        MethodKind::Procedure => "procedure",
        MethodKind::Function => "function",
    };
    let actual = normalized_declaration(signature);
    kind == spec.method_kind
        && (actual == normalized_declaration(&spec.signature_ru)
            || actual == normalized_declaration(&spec.signature_en))
        && spec
            .contexts
            .iter()
            .all(|expected| contexts.contains(expected))
        && required_event_directive(EventDirectiveOwner::Platform(capability), spec)
            .is_ok_and(|required| event_directive_matches(directive, required))
}

/// A form handler is compatible with its event when the method kind, the
/// contexts and the directive fit, and the parameters give no finding of the
/// form check (#791): a shorter earlier signature and renamed parameters are
/// compatible, a signature of another event of the owner or more required
/// parameters than the platform passes are not. The parameters are compared
/// with the current signature, `Элемент` included (#1390).
fn form_method_matches_event(
    method: &FormMethodFact<'_>,
    spec: &PlatformEventSpec,
    owner: &FormEventOwnerInput,
    owner_events: &[FormEventParameterForms],
) -> bool {
    let kind = match method.method_kind {
        MethodKind::Procedure => "procedure",
        MethodKind::Function => "function",
    };
    kind == spec.method_kind
        && foreign_handler_signature_8_3_27(owner_events, &spec.event_id, method.parameters)
            .is_none()
        && spec
            .contexts
            .iter()
            .all(|expected| method.contexts.contains(&expected.as_str()))
        && required_event_directive(EventDirectiveOwner::Property(owner.owner), spec)
            .is_ok_and(|required| event_directive_matches(method.directive, required))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventDirectiveOwner {
    Platform(ModuleCapability),
    Property(FormBindingOwner),
}

pub(crate) fn required_event_directive(
    owner: EventDirectiveOwner,
    spec: &PlatformEventSpec,
) -> Result<RequiredEventDirective, DirectiveRuleError> {
    match owner {
        EventDirectiveOwner::Platform(capability) => {
            return Ok(if capability.role() == ModuleRole::Command {
                RequiredEventDirective::Client
            } else {
                RequiredEventDirective::None
            });
        }
        EventDirectiveOwner::Property(FormBindingOwner::Command) if spec.event_id == "Execute" => {
            return Ok(RequiredEventDirective::Client);
        }
        EventDirectiveOwner::Property(_) => {}
    }
    const CLIENT_CONTEXTS: &[&str] = &[
        "client",
        "thinClient",
        "webClient",
        "thickClientManaged",
        "thickClientOrdinary",
        "mobileClient",
        "mobileAppClient",
    ];
    if spec.contexts.is_empty() {
        return Err(DirectiveRuleError::unavailable(format!(
            "event `{}` has no classified directive contexts",
            spec.event_id
        )));
    }
    if spec.contexts.iter().all(|context| context == "server") {
        return Ok(RequiredEventDirective::Server);
    }
    if spec
        .contexts
        .iter()
        .all(|context| CLIENT_CONTEXTS.contains(&context.as_str()))
    {
        return Ok(RequiredEventDirective::Client);
    }
    Err(DirectiveRuleError::unavailable(format!(
        "event `{}` mixes or uses unclassified directive contexts",
        spec.event_id
    )))
}

fn event_directive_matches(directive: Option<&str>, required: RequiredEventDirective) -> bool {
    let normalized = directive.map(str::trim).map(str::to_lowercase);
    match required {
        RequiredEventDirective::None => normalized.is_none(),
        RequiredEventDirective::Client => normalized
            .as_deref()
            .is_some_and(|value| matches!(value, "&наклиенте" | "&atclient")),
        RequiredEventDirective::Server => normalized
            .as_deref()
            .is_some_and(|value| matches!(value, "&насервере" | "&atserver")),
    }
}

pub(crate) fn project_form_events(
    context: &FormEventContext,
    bindings: &[FormEventBindingInput],
    methods: &[FormMethodFact<'_>],
) -> Vec<EventProjection> {
    let mut owners = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for binding in bindings {
        let owner_at = binding
            .at
            .split(".Event.")
            .next()
            .unwrap_or(binding.at.as_str());
        if seen.insert((owner_at.to_string(), binding.owner.catalog_owner())) {
            owners.push(FormEventOwnerInput::new(binding.owner, owner_at));
        }
    }
    project_form_owner_events(context, &owners, bindings, methods)
}

pub(crate) struct FormEventProjectionRecord {
    pub(crate) projection: EventProjection,
    pub(crate) spec: &'static PlatformEventSpec,
    pub(crate) owner: FormBindingOwner,
}

pub(crate) fn project_form_owner_event_record(
    context: &FormEventContext,
    owner: &FormEventOwnerInput,
    bindings: &[FormEventBindingInput],
    methods: &[FormMethodFact<'_>],
    event_at: &str,
) -> Option<FormEventProjectionRecord> {
    let catalog = form_event_catalog_8_3_27(
        context,
        owner.owner.catalog_owner(),
        owner.data_path.as_deref(),
    );
    catalog.into_iter().find_map(|spec| {
        let at = format!("{}.Event.{}", owner.at, spec.event_id);
        (at == event_at).then(|| FormEventProjectionRecord {
            projection: project_known_form_event(
                context,
                owner,
                bindings,
                methods,
                spec,
                &OnceCell::new(),
            ),
            spec,
            owner: owner.owner,
        })
    })
}

pub(crate) fn project_form_owner_events(
    context: &FormEventContext,
    owners: &[FormEventOwnerInput],
    bindings: &[FormEventBindingInput],
    methods: &[FormMethodFact<'_>],
) -> Vec<EventProjection> {
    let mut events = Vec::new();
    let mut projected_owners = std::collections::HashSet::new();
    for owner in owners {
        let owner_key = (owner.at.clone(), owner.owner.catalog_owner());
        if !projected_owners.insert(owner_key) {
            continue;
        }
        let catalog = form_event_catalog_8_3_27(
            context,
            owner.owner.catalog_owner(),
            owner.data_path.as_deref(),
        );
        // Parameter lists of the owner are built once, and only when a bound
        // handler has to be compared with them.
        let owner_events = OnceCell::new();
        for spec in &catalog {
            events.push(project_known_form_event(
                context,
                owner,
                bindings,
                methods,
                spec,
                &owner_events,
            ));
        }
        let known = catalog
            .iter()
            .map(|spec| spec.event_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let mut unknown = std::collections::BTreeMap::<&str, Vec<&FormEventBindingInput>>::new();
        for binding in bindings.iter().filter(|candidate| {
            candidate.owner.catalog_owner() == owner.owner.catalog_owner()
                && candidate
                    .at
                    .split(".Event.")
                    .next()
                    .unwrap_or(candidate.at.as_str())
                    == owner.at.as_str()
                && !known.contains(candidate.event.as_str())
        }) {
            unknown
                .entry(binding.event.as_str())
                .or_default()
                .push(binding);
        }
        for (event_id, actuals) in unknown {
            let actual = actuals[0];
            let method = methods
                .iter()
                .find(|method| method.name.to_lowercase() == actual.handler.to_lowercase());
            let at = format!("{}.Event.{event_id}", owner.at);
            events.push(EventProjection {
                at,
                event_id: event_id.to_string(),
                state: EventState::Invalid,
                signature: String::new(),
                contexts: Vec::new(),
                binding: BindingFact::Property,
                handler: actual.handler.clone(),
                handler_en: String::new(),
                implementation_at: method
                    .map(|method| form_method_at(actual.at.as_str(), method.name)),
                call_type: actual.call_type.clone(),
                can: Vec::new(),
            });
        }
    }
    events
}

fn project_known_form_event(
    context: &FormEventContext,
    owner: &FormEventOwnerInput,
    bindings: &[FormEventBindingInput],
    methods: &[FormMethodFact<'_>],
    spec: &PlatformEventSpec,
    owner_events: &OnceCell<Vec<FormEventParameterForms>>,
) -> EventProjection {
    let owner_prefix = owner.at.as_str();
    let at = format!("{owner_prefix}.Event.{}", spec.event_id);
    let actuals = bindings
        .iter()
        .filter(|candidate| {
            candidate.owner.catalog_owner() == owner.owner.catalog_owner()
                && candidate
                    .at
                    .split(".Event.")
                    .next()
                    .unwrap_or(candidate.at.as_str())
                    == owner_prefix
                && spec.event_id == candidate.event
        })
        .collect::<Vec<_>>();
    let duplicate = actuals.len() > 1;
    let actual = (actuals.len() == 1).then(|| actuals[0]);
    let matching_methods = actual
        .map(|binding| {
            methods
                .iter()
                .filter(|method| method.name.to_lowercase() == binding.handler.to_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let duplicate_method = matching_methods.len() > 1;
    let method = (matching_methods.len() == 1).then(|| matching_methods[0]);
    let binding_is_valid = actual.is_none_or(|binding| {
        validate_property_event_binding(
            context,
            binding.owner.validation_target(),
            &FormEventBinding {
                name: binding.event.as_str(),
                handler: binding.handler.as_str(),
                call_type: binding.call_type.as_deref(),
            },
        )
        .is_ok()
    });
    let state = match (
        duplicate || duplicate_method,
        actual,
        method,
        binding_is_valid,
    ) {
        (true, _, _, _) => EventState::Invalid,
        (false, None, _, _) => EventState::Available,
        (false, Some(_), _, false) => EventState::Invalid,
        (false, Some(_), None, true) => EventState::Missing,
        (false, Some(_), Some(method), true)
            if form_method_matches_event(
                method,
                spec,
                owner,
                owner_events.get_or_init(|| {
                    form_owner_event_parameters_8_3_27(
                        context,
                        owner.owner.catalog_owner(),
                        owner.data_path.as_deref(),
                    )
                }),
            ) =>
        {
            EventState::Implemented
        }
        (false, Some(_), Some(_), true) => EventState::Invalid,
    };
    let signature = if owner.owner == FormBindingOwner::Command {
        actual.map_or_else(
            || spec.signature_ru.clone(),
            |binding| format!("Процедура {}(Команда)", binding.handler),
        )
    } else {
        form_event_handler_signature_8_3_27(owner.owner.catalog_owner(), spec)
    };
    let can = match state {
        EventState::Available if context.direct_part_writable => {
            vec![OperationRegistry::closed().event_implementation_skeleton(
                at.clone(),
                (context.definition == crate::infrastructure::native_operations::form_event_registry::FormDefinitionKind::Extension)
                    .then_some("After"),
            )]
        }
        EventState::Missing => {
            vec![OperationRegistry::closed().event_implementation_skeleton(at.clone(), None)]
        }
        EventState::Available | EventState::Implemented | EventState::Invalid => Vec::new(),
    };
    EventProjection {
        at,
        event_id: spec.event_id.to_string(),
        state,
        signature,
        contexts: spec.contexts.clone(),
        binding: BindingFact::Property,
        handler: actual
            .map_or(spec.handler_ru.as_str(), |binding| binding.handler.as_str())
            .to_string(),
        handler_en: spec.handler_en.clone(),
        implementation_at: actual.and_then(|binding| {
            method.map(|method| form_method_at(binding.at.as_str(), method.name))
        }),
        call_type: actual.and_then(|binding| binding.call_type.clone()),
        can,
    }
}

fn form_method_at(event_at: &str, handler: &str) -> String {
    let owner_at = event_at.split(".Event.").next().unwrap_or(event_at);
    let item = owner_at.find(".Item.");
    let command = owner_at.find(".Command.");
    let boundary = match (item, command) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(index), None) | (None, Some(index)) => Some(index),
        (None, None) => None,
    };
    let form_at = boundary.map_or(owner_at, |index| &owner_at[..index]);
    format!("{form_at}.Module.Form.Method.{handler}")
}

pub(crate) fn project_declarative_binding(
    binding: DeclarativeBinding<'_>,
    source: &str,
) -> Result<ModuleProjectionSet, String> {
    let module_at = match binding.owner {
        "httpMethod" => "main:HTTPService.API.Module.HTTPService",
        "webServiceOperation" => "main:WebService.Обмен.Module.WebService",
        "integrationServiceChannel" => "main:IntegrationService.Шина.Module.IntegrationService",
        other => return Err(format!("unknown declarative service owner `{other}`")),
    };
    let at = QualifiedAddress::parse(module_at).map_err(|error| error.to_string())?;
    let capability = PlatformProfile::v8_3_27()
        .module_capability(&at)
        .ok_or_else(|| format!("service module `{module_at}` is not in profile 8.3.27"))?;
    project_module(ModuleProjectionRequest {
        at: &at,
        capability,
        title: format!("{} module", capability.role().as_str()),
        rev: "fixture-rev",
        source: Some(source),
        common_module: None,
        handles: &[],
        declarative_bindings: std::slice::from_ref(&binding),
        extension_targets: &[],
        platform_event_write: PlatformEventWriteCapability::Proven,
    })
}

struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        Self { text, starts }
    }

    fn line_of(&self, offset: u32) -> usize {
        match self.starts.binary_search(&(offset as usize)) {
            Ok(index) => index + 1,
            Err(index) => index.max(1),
        }
    }

    fn text(&self, line: usize) -> Option<&'a str> {
        let start = *self.starts.get(line.checked_sub(1)?)?;
        let end = self
            .starts
            .get(line)
            .copied()
            .unwrap_or(self.text.len())
            .saturating_sub(usize::from(line < self.starts.len()));
        self.text.get(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_guard, normalize_guard, project_declarative_binding,
        project_form_events as project_form_events_with_context,
        project_form_owner_events as project_form_owner_events_with_context, project_module,
        DeclarativeBinding, FormBindingOwner, FormEventBindingInput, FormEventOwnerInput,
        FormMethodFact, ModuleProjectionRequest, PlatformEventWriteCapability,
    };
    use crate::domain::address::{NodeKind, QualifiedAddress};
    use crate::domain::module_projection::{BindingFact, EventState, ExtensionKind, InterfaceKind};
    use crate::domain::platform_profile::PlatformProfile;
    use crate::infrastructure::bsl_outline::parse_bsl_syntax;
    use crate::infrastructure::native_operations::form_event_registry::{
        context_from_root, FormDefinitionKind, FormElementKind,
    };
    use bsl_syntax::ast::{AstNode, PreIfDir};
    use serde::Deserialize;
    use serde_json::json;

    /// Declared parameters of a test handler, all required. The slice lives
    /// for the test process, like the handler facts that borrow it.
    fn params(names: &[&str]) -> &'static [crate::domain::module_projection::MethodParameter] {
        names
            .iter()
            .map(|name| crate::domain::module_projection::MethodParameter {
                name: (*name).to_string(),
                optional: false,
            })
            .collect::<Vec<_>>()
            .leak()
    }

    const COMPLEX: &str = include_str!("../../../../tests/fixtures/v013/modules/complex.bsl");
    const EXTENSION: &str = include_str!("../../../../tests/fixtures/v013/modules/extension.bsl");
    const SYNTAX_BOUNDARIES: &str =
        include_str!("../../../../tests/fixtures/v013/modules/syntax-boundaries.bsl");
    const NESTED_REGION_ADDRESSES: &str =
        include_str!("../../../../tests/fixtures/v013/modules/nested-region-addresses.bsl");
    const DECLARATIVE_HTTP: &str =
        include_str!("../../../../tests/fixtures/v013/modules/declarative-http.bsl");
    const DECLARATIVE_SOAP: &str =
        include_str!("../../../../tests/fixtures/v013/modules/declarative-soap.bsl");
    const DECLARATIVE_INTEGRATION: &str =
        include_str!("../../../../tests/fixtures/v013/modules/declarative-integration.bsl");

    #[derive(Debug, Deserialize)]
    struct ModuleFixture {
        profile: String,
        cases: Vec<ModuleCase>,
    }

    #[derive(Debug, Deserialize)]
    struct ModuleCase {
        case: String,
        at: String,
        role: String,
        source: bool,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FormFixture {
        bindings: Vec<FormBindingCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FormBindingCase {
        case: String,
        owner: String,
        owner_at: String,
        event: String,
        handler: String,
        call_type: Option<String>,
        element_kind: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct DeclarativeFixture {
        handlers: Vec<DeclarativeCase>,
    }

    #[derive(Debug, Deserialize)]
    struct DeclarativeCase {
        owner: String,
        at: String,
        event: String,
        handler: String,
    }

    fn module_fixture() -> ModuleFixture {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v013/modules/module-cases.json"
        ))
        .unwrap()
    }

    fn form_fixture() -> FormFixture {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v013/modules/form-bindings.json"
        ))
        .unwrap()
    }

    fn declarative_fixture() -> DeclarativeFixture {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v013/modules/declarative-handlers.json"
        ))
        .unwrap()
    }

    fn declarative_source(owner: &str) -> &'static str {
        match owner {
            "httpMethod" => DECLARATIVE_HTTP,
            "webServiceOperation" => DECLARATIVE_SOAP,
            "integrationServiceChannel" => DECLARATIVE_INTEGRATION,
            other => panic!("unknown declarative fixture owner `{other}`"),
        }
    }

    fn form_context(
        xml: &str,
        owner_kind: NodeKind,
    ) -> crate::infrastructure::native_operations::form_event_registry::FormEventContext {
        let document = roxmltree::Document::parse(xml).unwrap();
        let mut context = context_from_root(document.root_element());
        context.metadata_owner = Some(owner_kind);
        context
    }

    fn default_form_context(
    ) -> crate::infrastructure::native_operations::form_event_registry::FormEventContext {
        form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
            NodeKind::CommonForm,
        )
    }

    fn project_form_events(
        bindings: &[FormEventBindingInput],
        methods: &[FormMethodFact<'_>],
    ) -> Vec<crate::domain::module_projection::EventProjection> {
        project_form_events_with_context(&default_form_context(), bindings, methods)
    }

    fn project_form_owner_events(
        owners: &[FormEventOwnerInput],
        bindings: &[FormEventBindingInput],
        methods: &[FormMethodFact<'_>],
    ) -> Vec<crate::domain::module_projection::EventProjection> {
        project_form_owner_events_with_context(&default_form_context(), owners, bindings, methods)
    }

    fn project(
        at: &str,
        source: Option<&str>,
    ) -> crate::domain::module_projection::ModuleProjectionSet {
        let at = QualifiedAddress::parse(at).unwrap();
        let profile = PlatformProfile::v8_3_27();
        let capability = profile.module_capability(&at).unwrap();
        let common_module =
            (capability.role() == crate::domain::platform_profile::ModuleRole::Common).then(|| {
                crate::domain::module_projection::CommonModuleProperties {
                    global: Some(false),
                    client_managed_application: Some(true),
                    server: Some(true),
                    external_connection: Some(true),
                    client_ordinary_application: Some(false),
                    server_call: Some(false),
                    privileged: Some(false),
                    return_values_reuse: Some("DuringRequest".to_string()),
                }
            });
        project_module(ModuleProjectionRequest {
            at: &at,
            capability,
            title: format!("{} module", capability.role().as_str()),
            rev: "fixture-rev",
            source,
            common_module,
            handles: &[],
            declarative_bindings: &[],
            extension_targets: &[],
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap()
    }

    #[test]
    fn every_approved_module_role_projects_without_a_parallel_role_registry() {
        let fixture = module_fixture();
        assert_eq!(fixture.profile, "8.3.27");
        let actual_cases = fixture
            .cases
            .iter()
            .map(|case| (case.case.as_str(), case.at.as_str(), case.role.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            actual_cases,
            vec![
                ("object", "main:Document.Заказ.Module.Object", "Object"),
                ("manager", "main:Document.Заказ.Module.Manager", "Manager"),
                (
                    "record-set",
                    "main:InformationRegister.Цены.Module.RecordSet",
                    "RecordSet"
                ),
                (
                    "value-manager",
                    "main:Constant.ОсновнаяВалюта.Module.ValueManager",
                    "ValueManager"
                ),
                ("common", "main:CommonModule.ЗаказыСервер", "Common"),
                (
                    "form",
                    "main:Document.Заказ.Form.ФормаДокумента.Module.Form",
                    "Form"
                ),
                (
                    "command",
                    "main:Document.Заказ.Command.ПровестиИЗакрыть.Module.Command",
                    "Command"
                ),
                (
                    "managed-application",
                    "main:Module.ManagedApplication",
                    "ManagedApplication"
                ),
                (
                    "ordinary-application",
                    "main:Module.OrdinaryApplication",
                    "OrdinaryApplication"
                ),
                (
                    "external-connection",
                    "main:Module.ExternalConnection",
                    "ExternalConnection"
                ),
                ("session", "main:Module.Session", "Session"),
                (
                    "http",
                    "main:HTTPService.API.Module.HTTPService",
                    "HTTPService"
                ),
                (
                    "soap",
                    "main:WebService.Обмен.Module.WebService",
                    "WebService"
                ),
                (
                    "integration-service",
                    "main:IntegrationService.Шина.Module.IntegrationService",
                    "IntegrationService"
                ),
                ("bot", "main:Bot.Помощник.Module.Bot", "Bot"),
                (
                    "websocket-client",
                    "main:WebSocketClient.Телефония.Module.WebSocketClient",
                    "WebSocketClient"
                ),
                (
                    "epf-object",
                    "epf:ExternalDataProcessor.Импорт.Module.Object",
                    "Object"
                ),
                (
                    "epf-form",
                    "epf:ExternalDataProcessor.Импорт.Form.Основная.Module.Form",
                    "Form"
                ),
                (
                    "epf-command",
                    "epf:ExternalDataProcessor.Импорт.Command.Выполнить.Module.Command",
                    "Command"
                ),
                (
                    "erf-object",
                    "erf:ExternalReport.Продажи.Module.Object",
                    "Object"
                ),
                (
                    "erf-form",
                    "erf:ExternalReport.Продажи.Form.Основная.Module.Form",
                    "Form"
                ),
                (
                    "erf-command",
                    "erf:ExternalReport.Продажи.Command.Сформировать.Module.Command",
                    "Command"
                ),
                (
                    "extension-object",
                    "sales:Document.Заказ.Module.Object",
                    "Object"
                ),
                (
                    "valid-missing-file",
                    "main:Document.Пустой.Module.Object",
                    "Object"
                ),
            ]
        );
        for case in fixture.cases {
            let projection = project(&case.at, case.source.then_some(""));
            assert_eq!(projection.summary().props.role, case.role, "{}", case.case);
            assert_ne!(projection.summary().props.role, "Service", "{}", case.case);
            assert_eq!(
                projection.summary().branches[3].count,
                projection.events().len(),
                "{}",
                case.case
            );
            for (branch, actual) in projection.summary().branches.iter().zip([
                projection.methods().len(),
                projection.regions().len(),
                projection.interfaces().len(),
                projection.events().len(),
                projection.compilation().len(),
                projection.body().len(),
            ]) {
                assert_eq!(branch.count, actual, "{} {:?}", case.case, branch.kind);
            }
        }
    }

    #[test]
    fn multiline_signature_and_real_empty_body_boundaries_come_from_the_projector() {
        let projection = project("main:CommonModule.ЗаказыСервер", Some(SYNTAX_BOUNDARIES));
        let multiline = projection
            .methods()
            .iter()
            .find(|method| method.name == "Многострочная")
            .unwrap();
        assert_eq!(
            multiline.signature,
            "Процедура Многострочная(\n    Первый,\n    Знач Второй = Неопределено\n) Экспорт"
        );
        assert_eq!(
            multiline.doc.as_deref(),
            Some("Exact multiline declaration.\nThe body starts after the complete signature.")
        );
        let body = projection
            .method_body(&multiline.at, None, 20, None)
            .unwrap();
        assert_eq!(
            body.lines
                .iter()
                .map(|line| (line.line, line.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(7, "    Сообщить(\"body\");")]
        );

        let empty = projection
            .methods()
            .iter()
            .find(|method| method.name == "Пустая")
            .unwrap();
        assert!(projection
            .method_body(&empty.at, None, 20, None)
            .unwrap()
            .lines
            .is_empty());
        let serialized = serde_json::to_value(empty).unwrap();
        assert_eq!(serialized["branches"][1]["count"], 0);
    }

    #[test]
    fn method_compilation_count_and_nested_guards_match_actual_ranges() {
        let projection = project("main:CommonModule.ЗаказыСервер", Some(SYNTAX_BOUNDARIES));
        let method = projection
            .methods()
            .iter()
            .find(|method| method.name == "Контекстная")
            .unwrap();
        let serialized = serde_json::to_value(method).unwrap();
        assert_eq!(serialized["branches"][0]["count"], 3);
        assert_eq!(method.compile.guard.as_deref(), Some("Client"));
        assert!(method.compile.conditional_body);
        let ranges = projection
            .compilation()
            .iter()
            .filter(|range| range.from_line >= 15 && range.to_line <= 22)
            .map(|range| (range.from_line, range.to_line, range.guard.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            ranges,
            vec![
                (15, 21, "Client"),
                (17, 17, "Client And WebClient"),
                (19, 19, "Client And Not (WebClient)"),
            ]
        );
    }

    #[test]
    fn crlf_projection_preserves_source_signature_but_body_lines_drop_record_terminators() {
        let source = SYNTAX_BOUNDARIES.replace('\n', "\r\n");
        let projection = project("main:CommonModule.ЗаказыСервер", Some(&source));
        let multiline = projection
            .methods()
            .iter()
            .find(|method| method.name == "Многострочная")
            .unwrap();
        assert!(
            multiline.signature.contains("\r\n"),
            "source-proven signatures retain the source EOL profile"
        );
        let body = projection
            .method_body(&multiline.at, None, 100, None)
            .unwrap();
        assert!(
            body.lines
                .iter()
                .all(|line| !line.text.contains(['\r', '\n'])),
            "BodyLine.text must not expose record terminators: {:?}",
            body.lines
        );
    }

    #[test]
    fn form_catalog_is_emitted_once_for_two_bindings_on_one_owner() {
        let bindings = [
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Тест.Item.Поле.Event.OnChange",
                "OnChange",
                "ПолеПриИзменении",
                None,
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Тест.Item.Поле.Event.StartChoice",
                "StartChoice",
                "ПолеНачалоВыбора",
                None,
            ),
        ];
        let methods = [
            FormMethodFact::new(
                "ПолеПриИзменении",
                crate::domain::module_projection::MethodKind::Procedure,
                params(&[]),
                vec![
                    "thinClient",
                    "webClient",
                    "thickClientManaged",
                    "mobileClient",
                    "mobileAppClient",
                ],
            )
            .with_directive(Some("&НаКлиенте")),
            FormMethodFact::new(
                "ПолеНачалоВыбора",
                crate::domain::module_projection::MethodKind::Procedure,
                params(&["ДанныеВыбора", "ВыборДобавлением", "СтандартнаяОбработка"]),
                vec![
                    "thinClient",
                    "webClient",
                    "thickClientManaged",
                    "mobileClient",
                    "mobileAppClient",
                ],
            )
            .with_directive(Some("&НаКлиенте")),
        ];
        let events = project_form_events(&bindings, &methods);
        assert_eq!(events.len(), 16);
        let unique = events
            .iter()
            .map(|event| event.at.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), events.len());
        assert_eq!(
            events
                .iter()
                .filter(|event| event.state == EventState::Implemented)
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["OnChange", "StartChoice"]
        );
    }

    #[test]
    fn form_event_state_requires_the_method_kind_and_accepts_an_earlier_shorter_signature() {
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Тест.Item.Поле.Event.OnChange",
            "OnChange",
            "ПолеПриИзменении",
            None,
        );
        let contexts = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        let procedure = [FormMethodFact::new(
            "ПолеПриИзменении",
            crate::domain::module_projection::MethodKind::Procedure,
            params(&[]),
            contexts.clone(),
        )
        .with_directive(Some("&НаКлиенте"))];
        assert_eq!(
            project_form_events(std::slice::from_ref(&binding), &procedure)
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .state,
            EventState::Implemented
        );
        let function = [FormMethodFact::new(
            "ПолеПриИзменении",
            crate::domain::module_projection::MethodKind::Function,
            params(&[]),
            contexts,
        )];
        assert_eq!(
            project_form_events(&[binding], &function)
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn method_parameters_keep_names_without_by_value_and_mark_defaults() {
        let module = project(
            "main:Catalog.Products.Form.Main.Module.Form",
            Some(
                "&НаКлиенте\nПроцедура Поле(Знач Элемент, Режим = Неопределено)\nКонецПроцедуры\n",
            ),
        );
        let parameters = &module.methods()[0].parameters;
        assert_eq!(
            parameters
                .iter()
                .map(|parameter| (parameter.name.as_str(), parameter.optional))
                .collect::<Vec<_>>(),
            [("Элемент", false), ("Режим", true)]
        );
    }

    #[test]
    fn form_handler_parameters_are_compared_with_the_item_and_the_form_check_rule() {
        let client = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        let input = FormBindingOwner::Element(FormElementKind::InputField);
        let item_event = |event: &str, handler: &str, parameters: &[&str]| {
            let at = format!("main:CommonForm.Тест.Item.Поле.Event.{event}");
            let binding = FormEventBindingInput::property(input, &at, event, handler, None);
            let methods = [FormMethodFact::new(
                handler,
                crate::domain::module_projection::MethodKind::Procedure,
                params(parameters),
                client.clone(),
            )
            .with_directive(Some("&НаКлиенте"))];
            project_form_events(&[binding], &methods)
                .into_iter()
                .find(|projected| projected.at == at)
                .unwrap()
        };
        // The current signature with the item, a renamed item and an
        // earlier shorter signature are all handlers the platform calls.
        for parameters in [&["Элемент"][..], &["Поле"][..], &[][..]] {
            assert_eq!(
                item_event("OnChange", "ПолеПриИзменении", parameters).state,
                EventState::Implemented,
                "{parameters:?}"
            );
        }
        let on_change = item_event("OnChange", "ПолеПриИзменении", &["Элемент"]);
        assert_eq!(on_change.signature, "Процедура ПриИзменении(Элемент)");
        assert_eq!(
            item_event(
                "StartChoice",
                "ПолеНачалоВыбора",
                &[
                    "Элемент",
                    "ДанныеВыбора",
                    "ВыборДобавлением",
                    "СтандартнаяОбработка"
                ]
            )
            .state,
            EventState::Implemented
        );
        // The signature of another event of the item and more required
        // parameters than the platform passes are not this event's handler.
        for parameters in [
            &["Элемент", "Текст", "СтандартнаяОбработка"][..],
            &["Элемент", "Лишний"][..],
        ] {
            assert_eq!(
                item_event("OnChange", "ПолеПриИзменении", parameters).state,
                EventState::Invalid,
                "{parameters:?}"
            );
        }
        // A form event handler does not receive the item.
        let open_at = "main:CommonForm.Тест.Event.OnOpen";
        let open = FormEventBindingInput::property(
            FormBindingOwner::Form,
            open_at,
            "OnOpen",
            "ПриОткрытии",
            None,
        );
        let open_event = |parameters: &[&str]| {
            let methods = [FormMethodFact::new(
                "ПриОткрытии",
                crate::domain::module_projection::MethodKind::Procedure,
                params(parameters),
                client.clone(),
            )
            .with_directive(Some("&НаКлиенте"))];
            project_form_events(std::slice::from_ref(&open), &methods)
                .into_iter()
                .find(|projected| projected.at == open_at)
                .unwrap()
        };
        let implemented = open_event(&["Отказ"]);
        assert_eq!(implemented.state, EventState::Implemented);
        assert_eq!(implemented.signature, "Процедура ПриОткрытии(Отказ)");
        assert_eq!(open_event(&["Элемент", "Отказ"]).state, EventState::Invalid);
        // A command action follows the same rule with its own `(Команда)`.
        let action_at = "main:CommonForm.Тест.Command.Обновить.Event.Execute";
        let action = FormEventBindingInput::property(
            FormBindingOwner::Command,
            action_at,
            "Execute",
            "Обновить",
            None,
        );
        let action_state = |parameters: &[&str]| {
            let methods = [FormMethodFact::new(
                "Обновить",
                crate::domain::module_projection::MethodKind::Procedure,
                params(parameters),
                client.clone(),
            )
            .with_directive(Some("&НаКлиенте"))];
            project_form_events(std::slice::from_ref(&action), &methods)
                .into_iter()
                .find(|projected| projected.at == action_at)
                .unwrap()
                .state
        };
        assert_eq!(action_state(&["Команда"]), EventState::Implemented);
        assert_eq!(action_state(&[]), EventState::Implemented);
        assert_eq!(action_state(&["Команда", "Лишний"]), EventState::Invalid);
    }

    #[test]
    fn duplicate_case_insensitive_event_methods_are_invalid_for_platform_and_property_sources() {
        let platform = project(
            "main:Catalog.Products.Module.Object",
            Some(
                "Процедура ПередЗаписью(Отказ)\nКонецПроцедуры\n\nПроцедура передзаписью(Отказ)\nКонецПроцедуры\n",
            ),
        );
        assert_eq!(
            platform
                .events()
                .iter()
                .find(|event| event.event_id == "BeforeWrite")
                .unwrap()
                .state,
            EventState::Invalid
        );

        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Test.Item.Field",
        );
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Test.Item.Field.Event.OnChange",
            "OnChange",
            "FieldOnChange",
            None,
        );
        let contexts = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        let methods = [
            FormMethodFact::new(
                "FieldOnChange",
                crate::domain::module_projection::MethodKind::Procedure,
                params(&[]),
                contexts.clone(),
            )
            .with_directive(Some("&AtClient")),
            FormMethodFact::new(
                "fieldonchange",
                crate::domain::module_projection::MethodKind::Procedure,
                params(&[]),
                contexts,
            )
            .with_directive(Some("&AtClient")),
        ];
        let property = project_form_owner_events(&[owner], &[binding], &methods);
        assert_eq!(
            property
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn form_binding_retains_actual_element_and_nested_column_kinds() {
        let label = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::LabelField),
            "main:CommonForm.Тест.Item.Метка.Event.OnChange",
            "OnChange",
            "МеткаПриИзменении",
            None,
        );
        let events = project_form_events(&[label], &[]);
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            ["Click", "OnChange", "URLProcessing"]
        );

        let column = FormEventBindingInput::property(
            FormBindingOwner::Column(FormElementKind::CheckBoxField),
            "main:CommonForm.Тест.Item.Таблица.Item.Пометка.Event.OnChange",
            "OnChange",
            "ПометкаПриИзменении",
            None,
        );
        let events = project_form_events(&[column], &[]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "OnChange");
    }

    #[test]
    fn form_owner_without_bindings_still_projects_its_closed_available_catalog() {
        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Element(FormElementKind::LabelField),
            "main:CommonForm.Тест.Item.Метка",
        );
        let events = project_form_owner_events(&[owner], &[], &[]);
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            ["Click", "OnChange", "URLProcessing"]
        );
        assert!(events
            .iter()
            .all(|event| event.state == EventState::Available));
        assert_eq!(
            events
                .iter()
                .map(|event| event.at.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            events.len()
        );
    }

    #[test]
    fn borrowed_available_form_event_skeleton_defaults_call_type_after() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Events/><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");

        let event = project_form_owner_events_with_context(&context, &[owner], &[], &[])
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();

        assert_eq!(event.state, EventState::Available);
        assert_eq!(
            serde_json::to_value(&event.can[0]).unwrap(),
            json!({
                "op": "event.implement",
                "args": {
                    "at": "main:CommonForm.Test.Event.OnOpen",
                    "callType": "After"
                }
            })
        );
    }

    #[test]
    fn base_form_only_available_event_is_not_advertised_as_directly_writable() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");

        let event = project_form_owner_events_with_context(&context, &[owner], &[], &[])
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();

        assert_eq!(event.state, EventState::Available);
        assert!(event.can.is_empty(), "{event:?}");
    }

    #[test]
    fn borrowed_missing_event_preserves_binding_and_offers_no_new_call_type() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Events/><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            Some("Before"),
        );

        let event = project_form_owner_events_with_context(
            &context,
            &[owner],
            std::slice::from_ref(&binding),
            &[],
        )
        .into_iter()
        .find(|event| event.event_id == "OnOpen")
        .unwrap();

        assert_eq!(event.state, EventState::Missing);
        assert_eq!(event.call_type.as_deref(), Some("Before"));
        assert_eq!(
            serde_json::to_value(&event.can[0]).unwrap(),
            json!({
                "op": "event.implement",
                "args": {"at": "main:CommonForm.Test.Event.OnOpen"}
            })
        );
    }

    #[test]
    fn valid_missing_form_event_keeps_its_direct_implementation_capability() {
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Test.Item.Value.Event.OnChange",
            "OnChange",
            "ValueOnChange",
            None,
        );

        let event = project_form_events(std::slice::from_ref(&binding), &[])
            .into_iter()
            .find(|event| event.event_id == "OnChange")
            .unwrap();

        assert_eq!(event.state, EventState::Missing);
        assert_eq!(
            serde_json::to_value(&event.can[0]).unwrap(),
            json!({
                "op": "event.implement",
                "args": {"at": "main:CommonForm.Test.Item.Value.Event.OnChange"}
            })
        );
    }

    #[test]
    fn duplicate_property_bindings_are_invalid_and_never_advertise_implementation() {
        let bindings = [
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Test.Item.Value.Event.OnChange",
                "OnChange",
                "FirstHandler",
                None,
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Test.Item.Value.Event.OnChange",
                "OnChange",
                "SecondHandler",
                None,
            ),
        ];

        let event = project_form_events(&bindings, &[])
            .into_iter()
            .find(|event| event.event_id == "OnChange")
            .unwrap();

        assert_eq!(event.state, EventState::Invalid);
        assert!(event.can.is_empty());
        assert!(event.implementation_at.is_none());
    }

    #[test]
    fn form_command_execute_uses_action_handler_and_form_signature() {
        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Command,
            "main:CommonForm.Test.Command.Refresh",
        );
        let available = project_form_owner_events(std::slice::from_ref(&owner), &[], &[]);
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].event_id, "Execute");
        assert_eq!(available[0].state, EventState::Available);
        assert_eq!(
            available[0].signature,
            "Процедура ОбработкаКоманды(Команда)"
        );
        assert_eq!(
            available[0].contexts,
            [
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ]
        );

        let action = FormEventBindingInput::property(
            FormBindingOwner::Command,
            "main:CommonForm.Test.Command.Refresh.Event.Execute",
            "Execute",
            "RefreshAction",
            None,
        );
        let missing = project_form_owner_events(
            std::slice::from_ref(&owner),
            std::slice::from_ref(&action),
            &[],
        );
        assert_eq!(missing[0].state, EventState::Missing);
        assert_eq!(missing[0].handler, "RefreshAction");
        assert_eq!(missing[0].signature, "Процедура RefreshAction(Команда)");

        let method = [FormMethodFact::new(
            "RefreshAction",
            crate::domain::module_projection::MethodKind::Procedure,
            params(&["Команда"]),
            vec![
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ],
        )
        .with_directive(Some("&НаКлиенте"))];
        let implemented = project_form_owner_events(
            std::slice::from_ref(&owner),
            std::slice::from_ref(&action),
            &method,
        );
        assert_eq!(implemented[0].state, EventState::Implemented);

        let duplicate = project_form_owner_events(&[owner], &[action.clone(), action], &method);
        assert_eq!(duplicate[0].state, EventState::Invalid);
        assert!(duplicate[0].can.is_empty());
    }

    #[test]
    fn every_form_owner_requires_the_exact_versioned_directive() {
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            None,
        );
        let client_contexts = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        for directive in [None, Some("&НаСервере"), Some("&AtClientAtServer")] {
            let methods = [FormMethodFact::new(
                "FormOnOpen",
                crate::domain::module_projection::MethodKind::Procedure,
                params(&["Cancel"]),
                client_contexts.clone(),
            )
            .with_directive(directive)];
            let event = project_form_owner_events(
                std::slice::from_ref(&owner),
                std::slice::from_ref(&binding),
                &methods,
            )
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();
            assert_eq!(event.state, EventState::Invalid, "{directive:?}");
            assert!(event.can.is_empty());
            assert!(event.implementation_at.is_some());
        }
        for directive in ["&НаКлИеНтЕ", "  &aTcLiEnT\r\n"] {
            let methods = [FormMethodFact::new(
                "FormOnOpen",
                crate::domain::module_projection::MethodKind::Procedure,
                params(&["Cancel"]),
                client_contexts.clone(),
            )
            .with_directive(Some(directive))];
            let event = project_form_owner_events(
                std::slice::from_ref(&owner),
                std::slice::from_ref(&binding),
                &methods,
            )
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();
            assert_eq!(event.state, EventState::Implemented, "{directive:?}");
        }

        let server_binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnCreateAtServer",
            "OnCreateAtServer",
            "OnCreateAtServer",
            None,
        );
        let server = [FormMethodFact::new(
            "OnCreateAtServer",
            crate::domain::module_projection::MethodKind::Procedure,
            params(&["Cancel", "StandardProcessing"]),
            vec!["server"],
        )
        .with_directive(Some(" &AtServer "))];
        let event = project_form_owner_events(&[owner], &[server_binding], &server)
            .into_iter()
            .find(|event| event.event_id == "OnCreateAtServer")
            .unwrap();
        assert_eq!(event.state, EventState::Implemented);
    }

    #[test]
    fn metadata_command_platform_event_requires_the_client_directive() {
        let at = "main:Document.Заказ.Command.Провести.Module.Command";
        let no_directive = project(
            at,
            Some(
                "Процедура ОбработкаКоманды(ПараметрКоманды, ПараметрыВыполненияКоманды)\nКонецПроцедуры",
            ),
        );
        assert_eq!(
            no_directive
                .events()
                .iter()
                .find(|event| event.event_id == "CommandProcessing")
                .unwrap()
                .state,
            EventState::Invalid
        );
        let client = project(
            at,
            Some(
                "&AtClient\r\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure",
            ),
        );
        assert_eq!(
            client
                .events()
                .iter()
                .find(|event| event.event_id == "CommandProcessing")
                .unwrap()
                .state,
            EventState::Implemented
        );
    }

    #[test]
    fn borrowed_form_command_with_multiple_call_type_actions_fails_closed() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Commands/><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Command,
            "main:CommonForm.Test.Command.Refresh",
        );
        let bindings = [
            FormEventBindingInput::property(
                FormBindingOwner::Command,
                "main:CommonForm.Test.Command.Refresh.Event.Execute",
                "Execute",
                "BeforeRefresh",
                Some("Before"),
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Command,
                "main:CommonForm.Test.Command.Refresh.Event.Execute",
                "Execute",
                "AfterRefresh",
                Some("After"),
            ),
        ];

        let event = project_form_owner_events_with_context(&context, &[owner], &bindings, &[])
            .into_iter()
            .next()
            .unwrap();

        assert_eq!(event.event_id, "Execute");
        assert_eq!(event.state, EventState::Invalid);
        assert!(event.can.is_empty());
    }

    #[test]
    fn form_possible_events_are_exact_for_document_and_dynamic_list_sources_without_bsl() {
        let document_context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
                    xmlns:v8="http://v8.1c.ru/8.1/data/core">
                <Attributes><Attribute name="Object" id="1">
                    <Type><v8:Type>cfg:DocumentObject.Order</v8:Type></Type>
                    <MainAttribute>true</MainAttribute>
                </Attribute></Attributes>
            </Form>"#,
            NodeKind::Document,
        );
        assert_eq!(document_context.definition, FormDefinitionKind::Regular);
        let form =
            FormEventOwnerInput::new(FormBindingOwner::Form, "main:Document.Order.Form.Main");
        let document_events =
            project_form_owner_events_with_context(&document_context, &[form], &[], &[]);
        assert_eq!(
            document_events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            [
                "ActivationProcessing",
                "AddInDetachmentOnError",
                "AfterWrite",
                "AfterWriteAtServer",
                "BeforeClose",
                "BeforeLoadDataFromSettingsAtServer",
                "BeforeReopenFromOtherServer",
                "BeforeWrite",
                "BeforeWriteAtServer",
                "ChoiceProcessing",
                "CollaborationSystemUsersAutoComplete",
                "CollaborationSystemUsersChoiceFormGetProcessing",
                "ExternalEvent",
                "FillCheckProcessingAtServer",
                "NavigationProcessing",
                "NewWriteProcessing",
                "NotificationProcessing",
                "OnChangeDisplaySettings",
                "OnClose",
                "OnCreateAtServer",
                "OnLoadDataFromSettingsAtServer",
                "OnMainServerAvailabilityChange",
                "OnOpen",
                "OnPasteFromClipboard",
                "OnReadAtServer",
                "OnReopen",
                "OnReopenFromOtherServer",
                "OnSaveDataInSettingsAtServer",
                "OnWriteAtServer",
                "URLGetProcessing",
                "URLListGetProcessing",
                "URLProcessing",
                "ValueChoice",
            ]
        );
        assert!(document_events
            .iter()
            .all(|event| event.state == EventState::Available));

        let dynamic_context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
                    xmlns:v8="http://v8.1c.ru/8.1/data/core">
                <Attributes><Attribute name="List" id="1">
                    <Type><v8:Type>cfg:DynamicList</v8:Type></Type>
                    <MainAttribute>true</MainAttribute>
                </Attribute></Attributes>
            </Form>"#,
            NodeKind::Catalog,
        );
        let table = FormEventOwnerInput::new(
            FormBindingOwner::Table,
            "main:Catalog.Items.Form.List.Item.List",
        )
        .with_data_path("List");
        let table_events =
            project_form_owner_events_with_context(&dynamic_context, &[table], &[], &[]);
        assert_eq!(
            table_events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            [
                "AfterDeleteRow",
                "BeforeAddRow",
                "BeforeCollapse",
                "BeforeDeleteRow",
                "BeforeEditEnd",
                "BeforeExpand",
                "BeforeLoadUserSettingsAtServer",
                "BeforeRowChange",
                "ChoiceProcessing",
                "Drag",
                "DragCheck",
                "DragEnd",
                "DragStart",
                "NewWriteProcessing",
                "OnActivateCell",
                "OnActivateField",
                "OnActivateRow",
                "OnChange",
                "OnCurrentParentChange",
                "OnEditEnd",
                "OnGetDataAtServer",
                "OnLoadUserSettingsAtServer",
                "OnSaveUserSettingsAtServer",
                "OnStartEdit",
                "OnUpdateUserSettingSetAtServer",
                "RefreshRequestProcessing",
                "Selection",
                "URLGetProcessing",
                "URLListGetProcessing",
                "ValueChoice",
            ]
        );
    }

    #[test]
    fn form_event_implementation_requires_call_type_valid_for_the_form_definition() {
        let regular = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
            NodeKind::CommonForm,
        );
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            Some("After"),
        );
        let methods = [FormMethodFact::new(
            "FormOnOpen",
            crate::domain::module_projection::MethodKind::Procedure,
            params(&["Cancel"]),
            vec![
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ],
        )
        .with_directive(Some("&AtClient"))];
        assert_eq!(
            project_form_events_with_context(&regular, std::slice::from_ref(&binding), &methods)
                .iter()
                .find(|event| event.event_id == "OnOpen")
                .unwrap()
                .state,
            EventState::Invalid
        );

        let extension = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        assert_eq!(extension.definition, FormDefinitionKind::Extension);
        assert_eq!(
            project_form_events_with_context(&extension, std::slice::from_ref(&binding), &methods,)
                .iter()
                .find(|event| event.event_id == "OnOpen")
                .unwrap()
                .state,
            EventState::Implemented
        );

        let unknown = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            Some("Instead"),
        );
        assert_eq!(
            project_form_events_with_context(&extension, &[unknown], &methods)
                .iter()
                .find(|event| event.event_id == "OnOpen")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn equal_nested_region_names_are_resolved_by_full_logical_address() {
        let projection = project(
            "main:CommonModule.ЗаказыСервер",
            Some(NESTED_REGION_ADDRESSES),
        );
        let first = "main:CommonModule.ЗаказыСервер.Region.Первый.Region.Общая";
        let second = "main:CommonModule.ЗаказыСервер.Region.Второй.Region.Общая";
        assert_eq!(projection.region(first).unwrap().at.as_deref(), Some(first));
        assert_eq!(
            projection.region(second).unwrap().at.as_deref(),
            Some(second)
        );
    }

    #[test]
    fn declarative_service_projection_must_use_real_fixture_syntax() {
        let case = &declarative_fixture().handlers[0];
        let projection = project_declarative_binding(
            DeclarativeBinding {
                owner: &case.owner,
                at: &case.at,
                event: &case.event,
                handler: &case.handler,
            },
            declarative_source(&case.owner),
        )
        .unwrap();
        let method = projection
            .methods()
            .iter()
            .find(|method| method.name == case.handler)
            .unwrap();
        assert_eq!(method.signature, "Процедура ОбработатьGET(Запрос, Ответ)");
        assert_eq!(
            projection
                .method_body(&method.at, None, 20, None)
                .unwrap()
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["    Ответ.КодСостояния = 200;"]
        );
        assert!(declarative_source(&case.owner).contains(&method.signature));
    }

    #[test]
    fn valid_missing_physical_file_keeps_possible_events_but_no_source_projection() {
        let projection = project("main:Document.Пустой.Module.Object", None);
        assert!(projection.methods().is_empty());
        assert!(projection.regions().is_empty());
        assert!(projection.interfaces().is_empty());
        assert!(projection.compilation().is_empty());
        assert!(projection.body().is_empty());
        assert!(!projection.events().is_empty());
        assert!(projection.events().iter().all(|event| {
            event.state == EventState::Available
                && event
                    .can
                    .iter()
                    .any(|capability| capability.op() == "event.implement")
        }));
        let json = serde_json::to_string(projection.summary()).unwrap();
        for forbidden in [
            "sourceState",
            "fileExists",
            "\"set\"",
            "\"role\":\"Service\"",
        ] {
            assert!(!json.contains(forbidden), "{forbidden}: {json}");
        }
    }

    #[test]
    fn platform_event_state_requires_exact_kind_parameter_shape_and_effective_contexts() {
        let exact = project(
            "main:Document.Заказ.Module.Object",
            Some("Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецПроцедуры"),
        );
        let event = exact
            .events()
            .iter()
            .find(|event| event.event_id == "BeforeWrite")
            .unwrap();
        assert_eq!(event.state, EventState::Implemented);
        assert_eq!(
            event.contexts,
            [
                "server",
                "externalConnection",
                "thickClientOrdinary",
                "mobileAppServer",
                "mobileStandaloneServer",
            ]
        );

        for source in [
            "Процедура ПередЗаписью(РежимЗаписи, Отказ, РежимПроведения)\nКонецПроцедуры",
            "Функция ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецФункции",
        ] {
            let projection = project("main:Document.Заказ.Module.Object", Some(source));
            assert_eq!(
                projection
                    .events()
                    .iter()
                    .find(|event| event.event_id == "BeforeWrite")
                    .unwrap()
                    .state,
                EventState::Invalid,
                "{source}"
            );
        }
    }

    /// #1381 (по PR #1364): без любого из четырёх флагов среды контексты
    /// методов и областей неизвестны, тело без фильтра доступно; все флаги
    /// `false` дают известный пустой список.
    #[test]
    fn unknown_execution_flags_do_not_become_empty_or_universal_contexts() {
        use crate::domain::module_projection::CommonModuleProperties;
        let known = CommonModuleProperties {
            global: None,
            server_call: None,
            privileged: None,
            return_values_reuse: None,
            client_managed_application: Some(false),
            client_ordinary_application: Some(false),
            server: Some(true),
            external_connection: Some(false),
        };
        let at = QualifiedAddress::parse("ext:CommonModule.Probe").unwrap();
        let capability = PlatformProfile::v8_3_27().module_capability(&at).unwrap();
        for missing in 0..5 {
            let mut props = known.clone();
            match missing {
                0 => props.server = None,
                1 => props.client_managed_application = None,
                2 => props.client_ordinary_application = None,
                3 => props.external_connection = None,
                _ => {}
            }
            let projection = project_module(ModuleProjectionRequest {
                at: &at, capability, title: "Probe".into(), rev: "fixture", source: Some("#Если Сервер Тогда\nПроцедура Test() Экспорт\nСообщить(1);\nКонецПроцедуры\n#КонецЕсли"),
                common_module: Some(props), handles: &[], declarative_bindings: &[], extension_targets: &[], platform_event_write: PlatformEventWriteCapability::Unproved,
            }).unwrap();
            let method = &projection.methods()[0];
            assert_eq!(method.compile.contexts.is_none(), missing < 4);
            assert_eq!(projection.compilation()[0].contexts.is_none(), missing < 4);
            assert!(!projection
                .method_body(&method.at, None, 100, None)
                .unwrap()
                .lines
                .is_empty());
        }
        let mut no_execution = known;
        no_execution.server = Some(false);
        assert_eq!(
            super::base_contexts(capability, Some(&no_execution)),
            Some(Vec::new())
        );
    }

    #[test]
    fn common_module_requires_all_normalized_flags() {
        let at = QualifiedAddress::parse("main:CommonModule.ЗаказыСервер").unwrap();
        let profile = PlatformProfile::v8_3_27();
        let error = project_module(ModuleProjectionRequest {
            at: &at,
            capability: profile.module_capability(&at).unwrap(),
            title: "Общий модуль".to_string(),
            rev: "fixture-rev",
            source: Some(""),
            common_module: None,
            handles: &[],
            declarative_bindings: &[],
            extension_targets: &[],
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap_err();
        assert!(error.contains("common-module flags"), "{error}");
    }

    #[test]
    fn method_projection_uses_ast_and_omits_body_text() {
        let handles = [
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:Document.Заказ.Form.ФормаДокумента.Item.Склад.Event.OnChange",
                "OnChange",
                "ОбщийОбработчик",
                None,
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Column(FormElementKind::InputField),
                "main:Document.Заказ.Form.ФормаДокумента.Item.Товары.Item.Количество.Event.OnChange",
                "OnChange",
                "ОбщийОбработчик",
                Some("After"),
            ),
        ];
        let at =
            QualifiedAddress::parse("main:Document.Заказ.Form.ФормаДокумента.Module.Form").unwrap();
        let profile = PlatformProfile::v8_3_27();
        let projection = project_module(ModuleProjectionRequest {
            at: &at,
            capability: profile.module_capability(&at).unwrap(),
            title: "Модуль формы".to_string(),
            rev: "fixture-rev",
            source: Some(COMPLEX),
            common_module: None,
            handles: &handles,
            declarative_bindings: &[],
            extension_targets: &[],
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap();

        let method = projection
            .methods()
            .iter()
            .find(|method| method.name == "ОбщийОбработчик")
            .unwrap();
        assert_eq!(
            method.signature,
            "Процедура ОбщийОбработчик(Элемент, Отказ = Ложь) Экспорт"
        );
        assert_eq!(
            method.doc.as_deref(),
            Some("Пересчитывает сумму строки.\nНе загружает данные повторно.")
        );
        assert_eq!(method.method_kind.as_str(), "procedure");
        assert!(method.export);
        assert_eq!(method.compile.directive.as_deref(), Some("&НаКлиенте"));
        assert_eq!(method.compile.guard.as_deref(), Some("Client"));
        assert!(method
            .compile
            .contexts
            .as_ref()
            .unwrap()
            .contains(&"thinClient".to_string()));
        assert_eq!(method.handles.len(), 2);
        assert_eq!(method.handles[1].call_type.as_deref(), Some("After"));
        assert!(method.extension.is_none());
        let json = serde_json::to_string(method).unwrap();
        assert!(!json.contains("Сообщить"));
        assert!(!json.contains("source"));
    }

    #[test]
    fn explicit_body_preserves_lines_paginates_and_filters_without_method_duplication() {
        let projection = project(
            "main:Document.Заказ.Form.ФормаДокумента.Module.Form",
            Some(COMPLEX),
        );
        let method = projection
            .methods()
            .iter()
            .find(|method| method.name == "ОбщийОбработчик")
            .unwrap();
        let all = projection.method_body(&method.at, None, 2, None).unwrap();
        assert_eq!(all.lines.len(), 2);
        assert!(all.next.is_some());
        let next = projection
            .method_body(&method.at, None, 100, all.next.as_deref())
            .unwrap();
        assert!(next.lines.first().unwrap().line > all.lines.last().unwrap().line);
        let client = projection
            .method_body(&method.at, Some("webClient"), 100, None)
            .unwrap();
        let server = projection
            .method_body(&method.at, Some("server"), 100, None)
            .unwrap();
        assert!(client.lines.iter().any(|line| line.text.contains("web")));
        assert!(!client
            .lines
            .iter()
            .any(|line| line.text.contains("other client")));
        assert!(server.lines.is_empty());
        assert_eq!(
            projection
                .methods()
                .iter()
                .filter(|candidate| candidate.name == method.name)
                .count(),
            1
        );
    }

    #[test]
    fn nested_regions_interface_projections_and_ambiguity_are_exact() {
        let projection = project("main:CommonModule.ЗаказыСервер", Some(COMPLEX));
        let public = projection
            .interfaces()
            .iter()
            .find(|interface| interface.interface == InterfaceKind::Public)
            .unwrap();
        let library = projection
            .interfaces()
            .iter()
            .find(|interface| interface.interface == InterfaceKind::Library)
            .unwrap();
        let overrides = projection
            .interfaces()
            .iter()
            .find(|interface| interface.interface == InterfaceKind::Override)
            .unwrap();
        assert_eq!(public.methods.len(), 2);
        assert_eq!(library.methods.len(), 3);
        assert_eq!(overrides.methods.len(), 1);
        assert!(!library
            .methods
            .iter()
            .any(|method| method.ends_with("НеклассифицированныйЭкспорт")));
        let nested = projection
            .regions()
            .iter()
            .find(|region| region.name.as_deref() == Some("Внутренние"))
            .unwrap();
        assert!(nested
            .at
            .as_ref()
            .unwrap()
            .contains("ПрограммныйИнтерфейс.Region.Внутренние"));
        let duplicates = projection
            .regions()
            .iter()
            .filter(|region| region.name.as_deref() == Some("Повтор"))
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 2);
        assert!(duplicates
            .iter()
            .all(|region| !region.addressable && region.at.is_none()));
        assert!(projection.region("Повтор").unwrap_err().is_ambiguous());
    }

    #[test]
    fn extension_annotations_resolve_independently_from_compilation_directives() {
        let targets = [
            (
                "Проверить",
                "main:Document.Заказ.Module.Object.Method.Проверить",
            ),
            (
                "Записать",
                "main:Document.Заказ.Module.Object.Method.Записать",
            ),
            (
                "Рассчитать",
                "main:Document.Заказ.Module.Object.Method.Рассчитать",
            ),
        ];
        let at = QualifiedAddress::parse("sales:Document.Заказ.Module.Object").unwrap();
        let profile = PlatformProfile::v8_3_27();
        let projection = project_module(ModuleProjectionRequest {
            at: &at,
            capability: profile.module_capability(&at).unwrap(),
            title: "Расширение".to_string(),
            rev: "fixture-rev",
            source: Some(EXTENSION),
            common_module: None,
            handles: &[],
            declarative_bindings: &[],
            extension_targets: &targets,
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap();
        let kinds = projection
            .methods()
            .iter()
            .map(|method| method.extension.as_ref().unwrap().kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                ExtensionKind::Instead,
                ExtensionKind::Before,
                ExtensionKind::After,
                ExtensionKind::ChangeAndValidate,
            ]
        );
        let instead = &projection.methods()[0];
        assert_eq!(instead.compile.directive.as_deref(), Some("&НаСервере"));
        assert_eq!(
            instead.extension.as_ref().unwrap().directive,
            "&Вместо(\"Проверить\")"
        );
        assert_eq!(
            instead.extension.as_ref().unwrap().target_at.as_deref(),
            Some("main:Document.Заказ.Module.Object.Method.Проверить")
        );
    }

    #[test]
    fn epf_erf_and_extension_sources_project_end_to_end() {
        let epf = project(
            "epf:ExternalDataProcessor.Импорт.Module.Object",
            Some(
                "Процедура ОбработкаПроверкиЗаполнения(Отказ, ПроверяемыеРеквизиты)\nКонецПроцедуры",
            ),
        );
        assert_eq!(
            epf.events()
                .iter()
                .find(|event| event.event_id == "FillCheckProcessing")
                .unwrap()
                .state,
            EventState::Implemented
        );
        assert_eq!(
            epf.methods()[0].at,
            "epf:ExternalDataProcessor.Импорт.Module.Object.Method.ОбработкаПроверкиЗаполнения"
        );

        let erf = project(
            "erf:ExternalReport.Продажи.Module.Object",
            Some("Процедура ПриКомпоновкеРезультата()\nКонецПроцедуры"),
        );
        assert_eq!(
            erf.events()
                .iter()
                .find(|event| event.event_id == "OnComposeResult")
                .unwrap()
                .state,
            EventState::Implemented
        );
        assert_eq!(
            erf.methods()[0].at,
            "erf:ExternalReport.Продажи.Module.Object.Method.ПриКомпоновкеРезультата"
        );

        extension_annotations_resolve_independently_from_compilation_directives();
    }

    #[test]
    fn russian_mobile_application_client_guard_normalizes_and_evaluates() {
        let parsed = parse_bsl_syntax(
            "#Если МобильноеПриложениеКлиент Тогда\nСообщить(\"mobile\");\n#КонецЕсли",
        );
        let directive = parsed
            .syntax_node()
            .descendants()
            .find_map(PreIfDir::cast)
            .unwrap();
        let guard = normalize_guard(directive.condition().unwrap().syntax());

        assert_eq!(guard, "MobileAppClient");
        assert!(evaluate_guard(&guard, "mobileAppClient"));
        assert!(!evaluate_guard(&guard, "server"));
    }

    #[test]
    fn form_binding_owners_and_all_four_event_states_are_projected() {
        let fixture = form_fixture();
        let bindings = fixture
            .bindings
            .iter()
            .map(|case| {
                FormEventBindingInput::property(
                    FormBindingOwner::parse(
                        &case.owner,
                        case.element_kind
                            .as_deref()
                            .and_then(FormElementKind::from_dsl_key),
                    )
                    .unwrap(),
                    &format!("{}.Event.{}", case.owner_at, case.event),
                    &case.event,
                    &case.handler,
                    case.call_type.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        let methods = [
            FormMethodFact::new(
                "ОбщийОбработчик",
                crate::domain::module_projection::MethodKind::Procedure,
                params(&[]),
                vec![
                    "thinClient",
                    "webClient",
                    "thickClientManaged",
                    "mobileClient",
                    "mobileAppClient",
                    "server",
                ],
            )
            .with_directive(Some("&НаКлиенте")),
            FormMethodFact::new(
                "Неверный",
                crate::domain::module_projection::MethodKind::Function,
                params(&["Элемент"]),
                vec!["server"],
            ),
        ];
        let events = project_form_events(&bindings, &methods);
        for case in fixture.bindings {
            let at = format!("{}.Event.{}", case.owner_at, case.event);
            let event = events.iter().find(|event| event.at == at).unwrap();
            assert_eq!(
                event.implementation_at.as_deref(),
                Some("main:Document.Заказ.Form.ФормаДокумента.Module.Form.Method.ОбщийОбработчик"),
                "{}",
                case.case
            );
            if case.case == "form" {
                assert_eq!(event.call_type, None);
            }
        }
        let states = events.iter().map(|event| event.state).collect::<Vec<_>>();
        assert!(states.contains(&EventState::Implemented));
        assert!(states.contains(&EventState::Available));

        let missing = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Тест.Item.Поле.Event.OnChange",
            "OnChange",
            "Отсутствует",
            None,
        );
        let invalid = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Тест.Item.Другое.Event.OnChange",
            "OnChange",
            "Неверный",
            None,
        );
        let broken = project_form_events(&[missing, invalid], &methods);
        assert_eq!(
            broken
                .iter()
                .find(|event| event.at == "main:CommonForm.Тест.Item.Поле.Event.OnChange")
                .unwrap()
                .state,
            EventState::Missing
        );
        assert_eq!(
            broken
                .iter()
                .find(|event| event.at == "main:CommonForm.Тест.Item.Другое.Event.OnChange")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn declarative_service_handlers_remain_on_exact_owners_without_synthetic_events() {
        let fixture = declarative_fixture();
        for case in fixture.handlers {
            let binding = DeclarativeBinding {
                owner: &case.owner,
                at: &case.at,
                event: &case.event,
                handler: &case.handler,
            };
            let projection =
                project_declarative_binding(binding, declarative_source(&case.owner)).unwrap();
            assert!(projection.events().is_empty(), "{}", case.at);
            let method = projection
                .methods()
                .iter()
                .find(|method| method.name == case.handler)
                .unwrap();
            assert_eq!(method.handles.len(), 1);
            assert_eq!(method.handles[0].owner, case.owner);
            assert_eq!(method.handles[0].binding, BindingFact::Property);
        }
    }
}
