use unica_bsl::event_text::RequiredEventDirective;
use unica_bsl::projection::{
    BodyLine, CommonModuleProperties, CompilationFacts, CompilationProjection, EventProjection,
    EventState, ExtensionKind, ExtensionProjection, HandleProjection, InterfaceKind,
    InterfaceProjection, MethodArea, MethodKind, MethodParameter, MethodProjection, ModuleIdentity,
    ModuleProjectionSet, ModuleProperties, RegionProjection,
};
use unica_metadata::event::BindingFact;

use std::cell::OnceCell;
use std::collections::HashMap;
use unica_address::address::QualifiedAddress;
use unica_address::module_routes::ModuleRole;
use unica_bsl::event_catalog::module_event_catalog_8_3_27;
use unica_bsl::profile::ModuleCapability;
use unica_bsl_parser::{
    parse as parse_bsl_syntax, AnnotationFact, CodeOutlineMethodKind, ExtensionAnnotationKind,
    ParsedModule,
};
use unica_forms::events::{
    foreign_handler_signature_8_3_27, form_event_catalog_8_3_27,
    form_event_handler_signature_8_3_27, form_owner_event_parameters_8_3_27,
    validate_property_event_binding, FormElementKind, FormEventBinding, FormEventContext,
    FormEventOwnerKind, FormEventParameterForms, FormEventTarget,
};
use unica_metadata::event_catalog::PlatformEventSpec;
use unica_refusal::RefusalCode;

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
pub enum FormBindingOwner {
    Form,
    Element(FormElementKind),
    Table,
    Column(FormElementKind),
    Command,
}

impl FormBindingOwner {
    pub fn parse(raw: &str, element_kind: Option<FormElementKind>) -> Option<Self> {
        match raw {
            "form" => Some(Self::Form),
            "element" => element_kind.map(Self::Element),
            "table" => Some(Self::Table),
            "column" => element_kind.map(Self::Column),
            "command" => Some(Self::Command),
            _ => None,
        }
    }

    pub fn catalog_owner(self) -> FormEventOwnerKind {
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
pub struct FormEventBindingInput {
    owner: FormBindingOwner,
    at: String,
    event: String,
    handler: String,
    call_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormEventOwnerInput {
    owner: FormBindingOwner,
    at: String,
    data_path: Option<String>,
}

impl FormEventOwnerInput {
    pub fn new(owner: FormBindingOwner, at: &str) -> Self {
        Self {
            owner,
            at: at.to_string(),
            data_path: None,
        }
    }

    #[must_use]
    pub fn with_data_path(mut self, data_path: &str) -> Self {
        self.data_path = Some(data_path.to_string());
        self
    }

    pub const fn owner(&self) -> FormBindingOwner {
        self.owner
    }

    pub fn at(&self) -> &str {
        &self.at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormMethodFact<'a> {
    pub name: &'a str,
    pub method_kind: MethodKind,
    pub parameters: &'a [MethodParameter],
    pub directive: Option<&'a str>,
    pub contexts: Vec<&'a str>,
}

impl<'a> FormMethodFact<'a> {
    pub fn new(
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
    pub const fn with_directive(mut self, directive: Option<&'a str>) -> Self {
        self.directive = directive;
        self
    }
}

impl FormEventBindingInput {
    pub fn property(
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
pub struct DeclarativeBinding<'a> {
    pub owner: &'a str,
    pub at: &'a str,
    pub event: &'a str,
    pub handler: &'a str,
}

pub struct ModuleProjectionRequest<'a> {
    pub at: &'a QualifiedAddress,
    pub capability: ModuleCapability,
    pub title: String,
    pub rev: &'a str,
    pub source: Option<&'a str>,
    pub common_module: Option<CommonModuleProperties>,
    pub handles: &'a [FormEventBindingInput],
    pub declarative_bindings: &'a [DeclarativeBinding<'a>],
    pub extension_targets: &'a [(&'a str, &'a str)],
    pub platform_event_write: PlatformEventWriteCapability,
}

pub struct PlatformEventProjectionRecord {
    pub projection: EventProjection,
    pub spec: &'static PlatformEventSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEventProjectionRecordError {
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
pub enum PlatformEventWriteCapability {
    Proven,
    Unproved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveRuleError {
    message: String,
}

impl DirectiveRuleError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub const fn code(&self) -> RefusalCode {
        RefusalCode::ProviderUnavailable
    }
}

impl std::fmt::Display for DirectiveRuleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for DirectiveRuleError {}

pub fn project_module(request: ModuleProjectionRequest<'_>) -> Result<ModuleProjectionSet, String> {
    let specs = module_event_catalog_8_3_27(request.capability);
    project_module_with_specs(request, specs)
}

pub fn project_module_event_record(
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
    let root = parsed;
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
    assign_method_areas(&mut methods, &regions);
    let interfaces = interface_projections(&methods, &regions);
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
    )
    .with_source(source.to_string()))
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
        handlers: Vec::new(),
        can: (write == PlatformEventWriteCapability::Proven)
            .then(|| unica_metadata::event::EventOperation::implementation_skeleton(event_at, None))
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
                handlers: Vec::new(),
                can: Vec::new(),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn method_projections(
    root: &ParsedModule<'_>,
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
    for node in root.methods() {
        let name = node.name.clone().unwrap_or_default();
        let method_kind = match node.kind {
            CodeOutlineMethodKind::Procedure => MethodKind::Procedure,
            CodeOutlineMethodKind::Function => MethodKind::Function,
        };
        let export = node.is_export;
        if name.is_empty() {
            return Err("BSL method has no parser-proven name".to_string());
        }
        let header_token = node
            .header_keyword
            .clone()
            .ok_or_else(|| format!("BSL method `{name}` has no declaration token"))?;
        let header_line = lines.line_of(header_token.start as u32);
        let parameter_list = node
            .parameter_list
            .clone()
            .ok_or_else(|| format!("BSL method `{name}` has no parser-proven parameter list"))?;
        let parameters = node
            .parameters
            .clone()?
            .into_iter()
            .map(|parameter| MethodParameter {
                name: parameter.name,
                optional: parameter.default_value.is_some(),
            })
            .collect();
        let declaration_end = node.declaration_end.unwrap_or(parameter_list.end);
        let signature = source[header_token.start..declaration_end]
            .trim()
            .to_string();
        let signature_end_line = lines.line_of((declaration_end as u32).saturating_sub(1));
        let annotations = &node.annotations;
        let directive = annotations
            .iter()
            .find(|annotation| annotation.is_compiler_directive)
            .map(|annotation| annotation.text.clone());
        let extension = annotations
            .iter()
            .find(|annotation| annotation.is_annotation)
            .and_then(|annotation| extension_projection(annotation, extension_targets));
        let guard = accumulated_guard(header_line, compilation);
        let contexts = base_contexts.map(|base| {
            let mut contexts = directive_contexts(directive.as_deref(), base);
            if let Some(guard) = guard.as_deref() {
                contexts.retain(|context| evaluate_guard(guard, context));
            }
            contexts
        });
        let node_from = lines.line_of(node.range.start as u32);
        let node_to = lines.line_of((node.range.end as u32).saturating_sub(1));
        let closing_line = node
            .closing_keyword
            .as_ref()
            .map(|range| lines.line_of(range.start as u32))
            .unwrap_or(node_to);
        // Annotations, the compilation directive and `Асинх` belong to the
        // method even when they stand on lines above the declaration keyword.
        let async_line = node.async_start.map(|offset| lines.line_of(offset as u32));
        let from_line = annotations
            .iter()
            .map(|annotation| lines.line_of(annotation.range.start as u32))
            .chain(async_line)
            .chain([header_line])
            .min()
            .unwrap_or(header_line);
        let text_from_line = documentation_comment_start(lines, from_line);
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
                text_from_line,
                body_from_line,
                body_to_line,
                area: None,
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
    annotation: &AnnotationFact,
    extension_targets: &[(&str, &str)],
) -> Option<ExtensionProjection> {
    let kind = match annotation.extension_kind? {
        ExtensionAnnotationKind::Before => ExtensionKind::Before,
        ExtensionAnnotationKind::After => ExtensionKind::After,
        ExtensionAnnotationKind::Instead => ExtensionKind::Instead,
        ExtensionAnnotationKind::ChangeAndValidate => ExtensionKind::ChangeAndValidate,
    };
    let directive = annotation.text.clone();
    let target = annotation.target.clone();
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

/// Первая строка документирующего комментария метода для его `Body`:
/// подряд идущие строки `//` непосредственно над первой строкой метода
/// (директивой, аннотацией или объявлением). Пустая строка и инструкция
/// препроцессора обрывают комментарий; без комментария текст начинается
/// с самого метода.
fn documentation_comment_start(lines: &LineIndex<'_>, from_line: usize) -> usize {
    let mut start = from_line;
    while start > 1 {
        match lines.text(start - 1) {
            Some(text) if text.trim_start().starts_with("//") => start -= 1,
            _ => break,
        }
    }
    start
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
    root: &ParsedModule<'_>,
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
    let markers = root.region_markers();
    for marker in markers {
        let line = lines.line_of(marker.range.start as u32);
        if marker.is_start {
            let index = raw.len();
            raw.push(RawRegion {
                name: marker.name,
                line,
                end_line: None,
                parent: stack.last().copied(),
            });
            stack.push(index);
        } else if marker.is_end {
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

/// Ближайшая нормативная область метода (INV.SOURCE.MODULE-REGION-INTERFACE):
/// от области, прямо содержащей метод, вверх по родителям до первой области
/// с нормативным именем.
fn assign_method_areas(methods: &mut [MethodProjection], regions: &[RegionProjection]) {
    for method in methods {
        let mut current = regions
            .iter()
            .position(|region| region.methods.contains(&method.at));
        method.area = None;
        while let Some(index) = current {
            let region = &regions[index];
            if let Some(area) = region
                .name
                .as_deref()
                .and_then(MethodArea::from_region_name)
            {
                method.area = Some(area);
                break;
            }
            current = region.parent;
        }
    }
}

fn interface_projections(
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
    root: &ParsedModule<'_>,
    lines: &LineIndex<'_>,
    capability: ModuleCapability,
    common: Option<&CommonModuleProperties>,
) -> Vec<CompilationProjection> {
    let base = base_contexts(capability, common);
    let base = base.as_deref();
    let mut ranges = Vec::<CompilationProjection>::new();
    for branch in root.conditional_branches() {
        let from_line = lines.line_of(branch.range.start as u32);
        let to_line = lines.line_of((branch.range.end as u32).saturating_sub(1));
        push_compilation_range(&mut ranges, from_line, to_line, &branch.guard, base);
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

fn accumulated_guard(line: usize, compilation: &[CompilationProjection]) -> Option<String> {
    compilation
        .iter()
        .filter(|range| line >= range.from_line && line <= range.to_line)
        .min_by_key(|range| range.to_line.saturating_sub(range.from_line))
        .map(|range| range.guard.clone())
}

pub(crate) fn evaluate_guard(guard: &str, context: &str) -> bool {
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
pub(crate) fn base_contexts(
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
        && foreign_handler_signature_8_3_27(
            owner_events,
            &spec.event_id,
            &method
                .parameters
                .iter()
                .map(|parameter| (parameter.name.as_str(), parameter.optional))
                .collect::<Vec<_>>(),
        )
        .is_none()
        && spec
            .contexts
            .iter()
            .all(|expected| method.contexts.contains(&expected.as_str()))
        && required_event_directive(EventDirectiveOwner::Property(owner.owner), spec)
            .is_ok_and(|required| event_directive_matches(method.directive, required))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDirectiveOwner {
    Platform(ModuleCapability),
    Property(FormBindingOwner),
}

pub fn required_event_directive(
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
        // This dynamic-list callback has no form context. Preserve the
        // previously accepted AtServer binding; generate NoContext methods.
        return Ok(
            if matches!(
                owner,
                EventDirectiveOwner::Property(FormBindingOwner::Table)
            ) && spec.event_id == "OnGetDataAtServer"
            {
                RequiredEventDirective::ServerOrServerNoContext
            } else {
                RequiredEventDirective::Server
            },
        );
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
        RequiredEventDirective::ServerOrServerNoContext => {
            normalized.as_deref().is_some_and(|value| {
                matches!(
                    value,
                    "&насервере" | "&atserver" | "&насерверебезконтекста" | "&atservernocontext"
                )
            })
        }
    }
}

pub fn project_form_events(
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

pub struct FormEventProjectionRecord {
    pub projection: EventProjection,
    pub spec: &'static PlatformEventSpec,
    pub owner: FormBindingOwner,
}

pub fn project_form_owner_event_record(
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

pub fn project_form_owner_events(
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
            let duplicate = actuals.len() > 1;
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
                handler: if duplicate {
                    String::new()
                } else {
                    actual.handler.clone()
                },
                handler_en: String::new(),
                implementation_at: method
                    .filter(|_| !duplicate)
                    .map(|method| form_method_at(actual.at.as_str(), method.name)),
                call_type: actual.call_type.clone().filter(|_| !duplicate),
                handlers: if duplicate {
                    event_handler_bindings(&actuals, methods)
                } else {
                    Vec::new()
                },
                can: Vec::new(),
            });
        }
    }
    events
}

/// Each actual binding of a conflicting event, with its implementation when
/// exactly one method of the form carries that name.
fn event_handler_bindings(
    actuals: &[&FormEventBindingInput],
    methods: &[FormMethodFact<'_>],
) -> Vec<unica_bsl::projection::EventHandlerBinding> {
    actuals
        .iter()
        .map(|binding| {
            let mut named = methods
                .iter()
                .filter(|method| method.name.to_lowercase() == binding.handler.to_lowercase());
            let method = named.next().filter(|_| named.next().is_none());
            unica_bsl::projection::EventHandlerBinding {
                handler: binding.handler.clone(),
                call_type: binding.call_type.clone(),
                implementation_at: method
                    .map(|method| form_method_at(binding.at.as_str(), method.name)),
            }
        })
        .collect()
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
                data_path: None,
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
    // A conflict names no handler, so no catalog signature stands in for one.
    let signature = if duplicate {
        String::new()
    } else if owner.owner == FormBindingOwner::Command {
        actual.map_or_else(
            || spec.signature_ru.clone(),
            |binding| format!("Процедура {}(Команда)", binding.handler),
        )
    } else {
        form_event_handler_signature_8_3_27(owner.owner.catalog_owner(), spec)
    };
    let can = match state {
        EventState::Available if context.direct_part_writable => {
            vec![
                unica_metadata::event::EventOperation::implementation_skeleton(
                    at.clone(),
                    (context.definition == unica_forms::events::FormDefinitionKind::Extension)
                        .then_some("After"),
                ),
            ]
        }
        EventState::Missing => {
            vec![unica_metadata::event::EventOperation::implementation_skeleton(at.clone(), None)]
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
        // Several bindings conflict: none is the handler, and the catalog
        // name would read as one that the source states.
        handler: if duplicate {
            String::new()
        } else {
            actual
                .map_or(spec.handler_ru.as_str(), |binding| binding.handler.as_str())
                .to_string()
        },
        handler_en: if duplicate {
            String::new()
        } else {
            spec.handler_en.clone()
        },
        implementation_at: actual.and_then(|binding| {
            method.map(|method| form_method_at(binding.at.as_str(), method.name))
        }),
        call_type: actual.and_then(|binding| binding.call_type.clone()),
        handlers: if duplicate {
            event_handler_bindings(&actuals, methods)
        } else {
            Vec::new()
        },
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
