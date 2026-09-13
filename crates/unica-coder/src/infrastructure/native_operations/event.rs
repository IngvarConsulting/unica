use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::module_projection::EventState;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::project_sources::SourceSetKind;
use crate::infrastructure::bsl_outline::parse_bsl_syntax;
use crate::infrastructure::event_projection::{
    project_platform_event_implementation, project_property_event_implementation,
    resolve_property_event_source as refine_property_event_source, EventImplementationShape,
    EventProjectionError, EventProjectionErrorKind,
};
use crate::infrastructure::logical_event_source::{
    platform_event_owner_evidence, resolve_event_source, resolve_event_source_for_form_evidence,
    EventOwnerDescriptorProof, EventOwnerEvidencePlan, LogicalEventSource, PropertyEventSource,
};
pub(crate) use crate::infrastructure::native_operations::apply::PlannedApplyEffects;
use crate::infrastructure::native_operations::apply::{
    ApplyStagedState, ApplyStagingError, ApplyStagingErrorKind,
};
use crate::infrastructure::native_operations::form::parse_form_event_evidence_xml;
use crate::infrastructure::native_operations::form_event_registry::{
    FormCallType, FormDefinitionKind,
};
use crate::infrastructure::native_operations::text_snapshot::{
    resolve_observed_line_ending, LineEnding, LineEndingProfile, SourceTextSnapshot, Utf8Bom,
};
use crate::infrastructure::platform_xml_owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    PlatformXmlSourceSetOwnerEvidence,
};
use bsl_syntax::ast::{AstNode, FunctionDef, PreRegionDir, ProcedureDef};
use roxmltree::{Document, Node};
use std::fmt;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventImplementArgs {
    pub(crate) at: crate::domain::address::QualifiedAddress,
    pub(crate) call_type: Option<FormCallType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventPlanErrorKind {
    BadValue,
    NotFound,
    ProviderUnavailable,
    InvalidState,
    InvalidSource,
    Staging(ApplyStagingErrorKind),
    Postcondition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventPlanError {
    kind: EventPlanErrorKind,
    path: Option<String>,
    message: String,
}

impl EventPlanError {
    fn new(kind: EventPlanErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: None,
            message: message.into(),
        }
    }

    pub(crate) const fn kind(&self) -> EventPlanErrorKind {
        self.kind
    }

    pub(crate) fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl fmt::Display for EventPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for EventPlanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleInsertionPlan {
    ExistingRegion { offset: usize },
    AppendRegion { offset: usize },
}

impl ModuleInsertionPlan {
    const fn offset(self) -> usize {
        match self {
            Self::ExistingRegion { offset } | Self::AppendRegion { offset } => offset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerInsertionSite {
    SelfClosing { offset: usize },
    Line { offset: usize },
    Inline { offset: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdjacentInsertionSite {
    Line { offset: usize },
    Inline { offset: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerCloseInsertionSite {
    SelfClosing { offset: usize },
    Line { offset: usize },
    Inline { offset: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FormBlock {
    EventsSection {
        section_indent: String,
        event_indent: String,
        event_xml: String,
    },
    Literal(String),
}

impl FormBlock {
    fn render(self, eol: &str) -> String {
        match self {
            Self::EventsSection {
                section_indent,
                event_indent,
                event_xml,
            } => format!(
                "{section_indent}<Events>{eol}{event_indent}{event_xml}{eol}{section_indent}</Events>"
            ),
            Self::Literal(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FormInsertionPlan {
    IntoContainer {
        range: Range<usize>,
        tag: String,
        child: String,
        indent: String,
        child_indent: String,
        site: ContainerInsertionSite,
    },
    AfterElement {
        block: FormBlock,
        indent: String,
        site: AdjacentInsertionSite,
    },
    BeforeElement {
        block: FormBlock,
        child_indent: String,
        site: AdjacentInsertionSite,
    },
    BeforeOwnerClose {
        range: Range<usize>,
        tag: String,
        block: FormBlock,
        indent: String,
        site: OwnerCloseInsertionSite,
    },
}

impl FormInsertionPlan {
    const fn offset(&self) -> usize {
        match self {
            Self::IntoContainer { site, .. } => match site {
                ContainerInsertionSite::SelfClosing { offset }
                | ContainerInsertionSite::Line { offset }
                | ContainerInsertionSite::Inline { offset } => *offset,
            },
            Self::AfterElement { site, .. } | Self::BeforeElement { site, .. } => match site {
                AdjacentInsertionSite::Line { offset }
                | AdjacentInsertionSite::Inline { offset } => *offset,
            },
            Self::BeforeOwnerClose { site, .. } => match site {
                OwnerCloseInsertionSite::SelfClosing { offset }
                | OwnerCloseInsertionSite::Line { offset }
                | OwnerCloseInsertionSite::Inline { offset } => *offset,
            },
        }
    }

    fn apply(self, body: &mut String, eol: &str) {
        match self {
            Self::IntoContainer {
                range,
                tag,
                child,
                indent,
                child_indent,
                site,
            } => match site {
                ContainerInsertionSite::SelfClosing { offset } => {
                    let opening = body[range.start..offset].trim_end().to_string();
                    body.replace_range(
                        range,
                        &format!("{opening}>{eol}{child_indent}{child}{eol}{indent}</{tag}>"),
                    );
                }
                ContainerInsertionSite::Line { offset } => {
                    body.insert_str(offset, &format!("{child_indent}{child}{eol}"));
                }
                ContainerInsertionSite::Inline { offset } => {
                    body.insert_str(offset, &format!("{eol}{child_indent}{child}{eol}{indent}"));
                }
            },
            Self::AfterElement {
                block,
                indent,
                site,
            } => {
                let block = block.render(eol);
                match site {
                    AdjacentInsertionSite::Line { offset } => {
                        body.insert_str(offset, &format!("{block}{eol}"));
                    }
                    AdjacentInsertionSite::Inline { offset } => {
                        body.insert_str(offset, &format!("{eol}{block}{eol}{indent}"));
                    }
                }
            }
            Self::BeforeElement {
                block,
                child_indent,
                site,
            } => {
                let block = block.render(eol);
                match site {
                    AdjacentInsertionSite::Line { offset } => {
                        body.insert_str(offset, &format!("{block}{eol}"));
                    }
                    AdjacentInsertionSite::Inline { offset } => {
                        body.insert_str(offset, &format!("{eol}{block}{eol}{child_indent}"));
                    }
                }
            }
            Self::BeforeOwnerClose {
                range,
                tag,
                block,
                indent,
                site,
            } => {
                let block = block.render(eol);
                match site {
                    OwnerCloseInsertionSite::SelfClosing { offset } => {
                        let opening = body[range.start..offset].trim_end().to_string();
                        body.replace_range(
                            range,
                            &format!("{opening}>{eol}{block}{eol}{indent}</{tag}>"),
                        );
                    }
                    OwnerCloseInsertionSite::Line { offset } => {
                        body.insert_str(offset, &format!("{block}{eol}"));
                    }
                    OwnerCloseInsertionSite::Inline { offset } => {
                        body.insert_str(offset, &format!("{eol}{block}{eol}{indent}"));
                    }
                }
            }
        }
    }
}

pub(crate) fn emit_event_module(
    current: Option<&[u8]>,
    shape: &EventImplementationShape,
) -> Result<Vec<u8>, EventPlanError> {
    let owned_default;
    let bytes = match current {
        Some(bytes) => bytes,
        None => {
            owned_default = Vec::new();
            &owned_default
        }
    };
    let snapshot =
        SourceTextSnapshot::from_bytes(bytes).map_err(|error| invalid_source(error.to_string()))?;
    reject_lone_cr(&snapshot)?;
    let text = snapshot.text();
    let parsed = parse_bsl_syntax(text);
    if !parsed.errors().is_empty() {
        return Err(invalid_source(format!(
            "BSL parser reported {} diagnostic(s)",
            parsed.errors().len()
        )));
    }
    let root = parsed.syntax_node();
    let handler_folded = shape.handler.to_lowercase();
    let matching_methods = root
        .descendants()
        .filter_map(|node| {
            ProcedureDef::cast(node.clone())
                .and_then(|method| method.name_or_keyword())
                .or_else(|| FunctionDef::cast(node).and_then(|method| method.name_or_keyword()))
        })
        .filter(|name| name.text().to_lowercase() == handler_folded)
        .count();
    if matching_methods != 0 {
        return Err(EventPlanError::new(
            EventPlanErrorKind::InvalidState,
            format!(
                "event handler `{}` collides with an existing method",
                shape.handler
            ),
        ));
    }

    let mut markers = root
        .descendants()
        .filter_map(PreRegionDir::cast)
        .collect::<Vec<_>>();
    markers.sort_by_key(|marker| marker.syntax().text_range().start());
    let canonical = shape.region.to_lowercase();
    let mut stack = Vec::<(Option<String>, bool)>::new();
    let mut canonical_closes = Vec::new();
    for marker in markers {
        if marker.is_start() {
            let name = marker.name();
            let is_canonical = name
                .as_deref()
                .is_some_and(|name| name.to_lowercase() == canonical);
            if is_canonical && !stack.is_empty() {
                return Err(invalid_source("canonical handler region is nested"));
            }
            stack.push((name, is_canonical));
        } else if marker.is_end() {
            let Some((_, is_canonical)) = stack.pop() else {
                return Err(invalid_source("BSL region close marker has no opener"));
            };
            if is_canonical {
                canonical_closes.push(usize::from(marker.syntax().text_range().start()));
            }
        }
    }
    if !stack.is_empty() {
        return Err(invalid_source("BSL region is unclosed"));
    }
    if canonical_closes.len() > 1 {
        return Err(invalid_source("canonical handler region is duplicated"));
    }

    let insertion_plan = canonical_closes.first().copied().map_or(
        ModuleInsertionPlan::AppendRegion { offset: text.len() },
        |marker_offset| ModuleInsertionPlan::ExistingRegion {
            offset: line_start(text, marker_offset),
        },
    );
    let eol = local_line_ending(text, insertion_plan.offset())?;
    let eol = resolve_observed_line_ending(&snapshot, eol)
        .map_err(|error| invalid_source(error.to_string()))?
        .as_str();
    let method = render_method(shape, eol);
    let mut body = text.to_string();
    match insertion_plan {
        ModuleInsertionPlan::ExistingRegion { offset } => {
            body.insert_str(offset, &format!("{method}{eol}"));
        }
        ModuleInsertionPlan::AppendRegion { .. } => {
            if !body.is_empty() {
                if !(body.ends_with('\n') || body.ends_with('\r')) {
                    body.push_str(eol);
                }
                body.push_str(eol);
            }
            body.push_str(&format!(
                "#Область {}{eol}{eol}{method}{eol}#КонецОбласти{eol}",
                shape.region
            ));
        }
    }
    let reparsed = parse_bsl_syntax(&body);
    if !reparsed.errors().is_empty() {
        return Err(EventPlanError::new(
            EventPlanErrorKind::Postcondition,
            "emitted BSL failed parser postcondition",
        ));
    }
    let mut output = Vec::with_capacity(body.len() + 3);
    if current.is_none() || snapshot.bom() == Utf8Bom::Present {
        output.extend_from_slice(b"\xef\xbb\xbf");
    }
    output.extend_from_slice(body.as_bytes());
    Ok(output)
}

pub(crate) fn patch_form_event(
    current: &[u8],
    source: &PropertyEventSource,
    shape: &EventImplementationShape,
    call_type: Option<FormCallType>,
) -> Result<Vec<u8>, EventPlanError> {
    const FORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
    let snapshot = SourceTextSnapshot::from_bytes(current)
        .map_err(|error| invalid_source(error.to_string()))?;
    reject_lone_cr(&snapshot)?;
    let text = snapshot.text();
    let document = Document::parse(text)
        .map_err(|error| invalid_source(format!("Form.xml parse failed: {error}")))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(FORM_NS)
        || root.tag_name().name() != "Form"
        || root.attribute("version") != Some("2.20")
    {
        return Err(invalid_source(
            "Form.xml has the wrong root namespace or version",
        ));
    }
    let owner = resolve_form_owner(root, source, FORM_NS)?;
    let insertion_plan = if shape.owner
        == crate::infrastructure::event_projection::EventImplementationOwner::Command
    {
        plan_command_action(text, owner, shape, call_type, FORM_NS)?
    } else {
        plan_property_event(text, owner, shape, call_type, FORM_NS)?
    };
    let eol = local_line_ending(text, insertion_plan.offset())?
        .or_else(|| match snapshot.line_endings() {
            LineEndingProfile::Uniform(value) => Some(value),
            LineEndingProfile::None => Some(LineEnding::Lf),
            LineEndingProfile::Mixed { .. } => None,
        })
        .ok_or_else(|| invalid_source("Form.xml insertion EOL is ambiguous"))?
        .as_str();
    let mut body = text.to_string();
    insertion_plan.apply(&mut body, eol);
    Document::parse(&body).map_err(|error| {
        EventPlanError::new(
            EventPlanErrorKind::Postcondition,
            format!("patched Form.xml is invalid: {error}"),
        )
    })?;
    let mut output = Vec::with_capacity(body.len() + 3);
    if snapshot.bom() == Utf8Bom::Present {
        output.extend_from_slice(b"\xef\xbb\xbf");
    }
    output.extend_from_slice(body.as_bytes());
    Ok(output)
}

fn invalid_source(message: impl Into<String>) -> EventPlanError {
    EventPlanError::new(EventPlanErrorKind::InvalidSource, message)
}

fn reject_lone_cr(snapshot: &SourceTextSnapshot) -> Result<(), EventPlanError> {
    if matches!(
        snapshot.line_endings(),
        LineEndingProfile::Uniform(LineEnding::Cr) | LineEndingProfile::Mixed { cr: 1.., .. }
    ) {
        Err(invalid_source("source contains a lone CR line ending"))
    } else {
        Ok(())
    }
}

fn local_line_ending(text: &str, offset: usize) -> Result<Option<LineEnding>, EventPlanError> {
    let bytes = text.as_bytes();
    let previous = bytes[..offset]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|newline| {
            if newline > 0 && bytes[newline - 1] == b'\r' {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            }
        });
    let next = bytes[offset..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|relative| offset + relative)
        .map(|newline| {
            if newline > 0 && bytes[newline - 1] == b'\r' {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            }
        });
    match (previous, next) {
        (Some(left), Some(right)) if left != right => {
            Err(invalid_source("local insertion EOL is ambiguous"))
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

fn render_method(shape: &EventImplementationShape, eol: &str) -> String {
    let directive = shape
        .directive
        .canonical()
        .map(|directive| format!("{directive}{eol}"))
        .unwrap_or_default();
    let end = match shape.method_kind {
        crate::domain::module_projection::MethodKind::Procedure => "КонецПроцедуры",
        crate::domain::module_projection::MethodKind::Function => "КонецФункции",
    };
    format!("{directive}{}{eol}{eol}{end}{eol}", shape.signature)
}

fn exact_direct_children<'a, 'input>(
    owner: Node<'a, 'input>,
    local: &str,
    namespace: &str,
) -> Result<Vec<Node<'a, 'input>>, EventPlanError> {
    let mut exact = Vec::new();
    for child in owner.children().filter(Node::is_element) {
        if child.tag_name().name() == local {
            if child.tag_name().namespace() != Some(namespace) {
                return Err(invalid_source(format!(
                    "Form.xml contains a namespace lookalike `{local}`"
                )));
            }
            exact.push(child);
        }
    }
    Ok(exact)
}

fn exactly_one_direct<'a, 'input>(
    owner: Node<'a, 'input>,
    local: &str,
    namespace: &str,
) -> Result<Node<'a, 'input>, EventPlanError> {
    match exact_direct_children(owner, local, namespace)?.as_slice() {
        [node] => Ok(*node),
        [] => Err(EventPlanError::new(
            EventPlanErrorKind::NotFound,
            format!("Form.xml direct `{local}` owner is absent"),
        )),
        _ => Err(invalid_source(format!(
            "Form.xml direct `{local}` owner is duplicated"
        ))),
    }
}

fn resolve_form_owner<'a, 'input>(
    root: Node<'a, 'input>,
    source: &PropertyEventSource,
    namespace: &str,
) -> Result<Node<'a, 'input>, EventPlanError> {
    use crate::infrastructure::logical_event_source::PropertyEventOwnerKind;
    let owners = &source.owner_chain;
    if owners.first().map(|owner| owner.kind) != Some(PropertyEventOwnerKind::Form) {
        return Err(invalid_source(
            "typed property owner chain does not start at Form",
        ));
    }
    if owners.len() == 1 {
        return Ok(root);
    }
    if owners.last().map(|owner| owner.kind) == Some(PropertyEventOwnerKind::Command) {
        if owners.len() != 2 {
            return Err(invalid_source(
                "form Command owner chain has an unsupported depth",
            ));
        }
        let commands = exactly_one_direct(root, "Commands", namespace)?;
        let name = owners[1]
            .at
            .segments()
            .last()
            .and_then(crate::domain::address::AddressSegment::name)
            .ok_or_else(|| invalid_source("typed Command owner has no exact name"))?;
        let matches = exact_direct_children(commands, "Command", namespace)?
            .into_iter()
            .filter(|node| node.attribute("name") == Some(name))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [node] => Ok(*node),
            [] => Err(EventPlanError::new(
                EventPlanErrorKind::NotFound,
                format!("form Command `{name}` is absent"),
            )),
            _ => Err(invalid_source(format!(
                "form Command `{name}` is duplicated"
            ))),
        };
    }
    let mut current = root;
    for expected in &owners[1..] {
        let child_items = exactly_one_direct(current, "ChildItems", namespace)?;
        let name = expected
            .at
            .segments()
            .last()
            .and_then(crate::domain::address::AddressSegment::name)
            .ok_or_else(|| invalid_source("typed Item owner has no exact name"))?;
        let named = child_items
            .children()
            .filter(Node::is_element)
            .filter(|node| node.attribute("name") == Some(name))
            .collect::<Vec<_>>();
        if named
            .iter()
            .any(|node| node.tag_name().namespace() != Some(namespace))
        {
            return Err(invalid_source(format!(
                "Form.xml contains a namespace lookalike for Item `{name}`"
            )));
        }
        let matches = named;
        current = match matches.as_slice() {
            [node] => *node,
            [] => {
                return Err(EventPlanError::new(
                    EventPlanErrorKind::NotFound,
                    format!("form Item `{name}` is absent from the exact owner chain"),
                ))
            }
            _ => return Err(invalid_source(format!("form Item `{name}` is duplicated"))),
        };
    }
    Ok(current)
}

fn plan_property_event(
    body: &str,
    owner: Node<'_, '_>,
    shape: &EventImplementationShape,
    call_type: Option<FormCallType>,
    namespace: &str,
) -> Result<FormInsertionPlan, EventPlanError> {
    let events = exact_direct_children(owner, "Events", namespace)?;
    if events.len() > 1 {
        return Err(invalid_source("direct Events section is duplicated"));
    }
    if let Some(events) = events.first().copied() {
        let slots = exact_direct_children(events, "Event", namespace)?;
        let mut slot_names = std::collections::HashSet::new();
        for slot in &slots {
            let name = slot
                .attribute("name")
                .filter(|name| !name.is_empty())
                .ok_or_else(|| invalid_source("direct event slot has no exact name"))?;
            if !slot_names.insert(name) {
                return Err(invalid_source(format!(
                    "direct event slot `{name}` is duplicated"
                )));
            }
        }
        if slots
            .iter()
            .any(|event| event.attribute("name") == Some(shape.projection.event_id.as_str()))
        {
            return Err(invalid_source(
                "direct event slot is duplicated or already present",
            ));
        }
        let event_xml = property_event_xml(shape, call_type);
        return plan_into_container(body, events.range(), "Events", event_xml);
    }
    let indent = line_indent(body, owner.range().start);
    let section_indent = format!("{indent}\t");
    let event_indent = format!("{section_indent}\t");
    let event_xml = property_event_xml(shape, call_type);
    let block = FormBlock::EventsSection {
        section_indent: section_indent.clone(),
        event_indent,
        event_xml,
    };
    let owner_tag = owner.tag_name().name();
    if owner == owner.document().root_element() {
        let auto = exact_direct_children(owner, "AutoCommandBar", namespace)?;
        if auto.len() > 1 {
            return Err(invalid_source("root AutoCommandBar is duplicated"));
        }
        if let Some(auto) = auto.first() {
            return Ok(plan_after_element(body, auto.range(), block, indent));
        }
        for anchor in [
            "ChildItems",
            "Attributes",
            "Parameters",
            "Commands",
            "BaseForm",
        ] {
            let nodes = exact_direct_children(owner, anchor, namespace)?;
            if nodes.len() > 1 {
                return Err(invalid_source(format!("root `{anchor}` is duplicated")));
            }
            if let Some(node) = nodes.first() {
                return Ok(plan_before_element(
                    body,
                    node.range().start,
                    block,
                    section_indent,
                ));
            }
        }
    } else if matches!(owner_tag, "Table" | "Pages") {
        let child_items = exact_direct_children(owner, "ChildItems", namespace)?;
        if child_items.len() > 1 {
            return Err(invalid_source("owner ChildItems section is duplicated"));
        }
        if let Some(child_items) = child_items.first() {
            return Ok(plan_before_element(
                body,
                child_items.range().start,
                block,
                section_indent,
            ));
        }
    }
    plan_before_owner_close(body, owner.range(), owner_tag, block, indent)
}

fn plan_command_action(
    body: &str,
    owner: Node<'_, '_>,
    shape: &EventImplementationShape,
    call_type: Option<FormCallType>,
    namespace: &str,
) -> Result<FormInsertionPlan, EventPlanError> {
    let actions = exact_direct_children(owner, "Action", namespace)?;
    if !actions.is_empty() {
        return Err(invalid_source(
            "form Command direct Action is duplicated or already present",
        ));
    }
    let indent = line_indent(body, owner.range().start);
    let child_indent = format!("{indent}\t");
    let call_type = call_type
        .map(|value| format!(" callType=\"{}\"", value.as_str()))
        .unwrap_or_default();
    let action = format!(
        "{child_indent}<Action{call_type}>{}</Action>",
        escape_xml(&shape.handler)
    );
    let block = FormBlock::Literal(action);
    let following_properties = [
        "Shortcut",
        "Representation",
        "CurrentRowUse",
        "ModifiesData",
        "ModifiesSavedData",
        "ChangedStateSavedData",
        "Use",
        "Mark",
        "ParameterUse",
    ];
    let following = owner.children().filter(Node::is_element).find(|child| {
        child.tag_name().namespace() == Some(namespace)
            && following_properties.contains(&child.tag_name().name())
    });
    if let Some(child) = following {
        return Ok(plan_before_element(
            body,
            child.range().start,
            block,
            child_indent,
        ));
    }
    plan_before_owner_close(body, owner.range(), "Command", block, indent)
}

fn property_event_xml(shape: &EventImplementationShape, call_type: Option<FormCallType>) -> String {
    let call_type = call_type
        .map(|value| format!(" callType=\"{}\"", value.as_str()))
        .unwrap_or_default();
    format!(
        "<Event name=\"{}\"{call_type}>{}</Event>",
        escape_xml(&shape.projection.event_id),
        escape_xml(&shape.handler)
    )
}

fn plan_into_container(
    body: &str,
    range: Range<usize>,
    tag: &str,
    child: String,
) -> Result<FormInsertionPlan, EventPlanError> {
    let section = &body[range.clone()];
    let indent = line_indent(body, range.start);
    let child_indent = format!("{indent}\t");
    let site = if section.trim_end().ends_with("/>") {
        let relative = section
            .rfind("/>")
            .ok_or_else(|| invalid_source(format!("self-closing `{tag}` has no terminator")))?;
        ContainerInsertionSite::SelfClosing {
            offset: range.start + relative,
        }
    } else {
        let close_tag = format!("</{tag}>");
        let relative = section
            .rfind(&close_tag)
            .ok_or_else(|| invalid_source(format!("`{tag}` has no closing tag")))?;
        let close = range.start + relative;
        let start = line_start(body, close);
        if start >= range.start && body[start..close].trim().is_empty() {
            ContainerInsertionSite::Line { offset: start }
        } else {
            ContainerInsertionSite::Inline { offset: close }
        }
    };
    Ok(FormInsertionPlan::IntoContainer {
        range,
        tag: tag.to_string(),
        child,
        indent,
        child_indent,
        site,
    })
}

fn plan_after_element(
    body: &str,
    range: Range<usize>,
    block: FormBlock,
    indent: String,
) -> FormInsertionPlan {
    let suffix = &body[range.end..];
    let site = if suffix.starts_with("\r\n") {
        AdjacentInsertionSite::Line {
            offset: range.end + 2,
        }
    } else if suffix.starts_with('\n') {
        AdjacentInsertionSite::Line {
            offset: range.end + 1,
        }
    } else {
        AdjacentInsertionSite::Inline { offset: range.end }
    };
    FormInsertionPlan::AfterElement {
        block,
        indent,
        site,
    }
}

fn plan_before_element(
    body: &str,
    offset: usize,
    block: FormBlock,
    child_indent: String,
) -> FormInsertionPlan {
    let start = line_start(body, offset);
    let site = if body[start..offset].trim().is_empty() {
        AdjacentInsertionSite::Line { offset: start }
    } else {
        AdjacentInsertionSite::Inline { offset }
    };
    FormInsertionPlan::BeforeElement {
        block,
        child_indent,
        site,
    }
}

fn plan_before_owner_close(
    body: &str,
    range: Range<usize>,
    tag: &str,
    block: FormBlock,
    indent: String,
) -> Result<FormInsertionPlan, EventPlanError> {
    let owner = &body[range.clone()];
    let site = if owner.trim_end().ends_with("/>") {
        let relative = owner
            .rfind("/>")
            .ok_or_else(|| invalid_source("self-closing owner has no terminator"))?;
        OwnerCloseInsertionSite::SelfClosing {
            offset: range.start + relative,
        }
    } else {
        let close_tag = format!("</{tag}>");
        let relative = owner
            .rfind(&close_tag)
            .ok_or_else(|| invalid_source("form owner has no closing tag"))?;
        let close = range.start + relative;
        let start = line_start(body, close);
        if start >= range.start && body[start..close].trim().is_empty() {
            OwnerCloseInsertionSite::Line { offset: start }
        } else {
            OwnerCloseInsertionSite::Inline { offset: close }
        }
    };
    Ok(FormInsertionPlan::BeforeOwnerClose {
        range,
        tag: tag.to_string(),
        block,
        indent,
        site,
    })
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |newline| newline + 1)
}

fn line_indent(text: &str, offset: usize) -> String {
    let start = line_start(text, offset);
    text[start..offset]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t' | '\r'))
        .collect()
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn effect(kind: DomainEventKind, artifact: impl Into<String>) -> DomainEvent {
    DomainEvent::new(kind, artifact)
}

pub(crate) fn parse_event_implement_args(
    value: &serde_json::Value,
    path: &str,
) -> Result<EventImplementArgs, EventPlanError> {
    let object = value.as_object().ok_or_else(|| {
        EventPlanError::new(EventPlanErrorKind::BadValue, "event args must be an object")
            .at_path(path)
    })?;
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "at" | "callType"))
    {
        return Err(EventPlanError::new(
            EventPlanErrorKind::BadValue,
            format!("unknown event argument `{unknown}`"),
        )
        .at_path(format!("{path}.{unknown}")));
    }
    let at_path = format!("{path}.at");
    let at = object
        .get("at")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            EventPlanError::new(EventPlanErrorKind::BadValue, "at must be a string")
                .at_path(&at_path)
        })?;
    let at = crate::domain::address::QualifiedAddress::parse(at).map_err(|error| {
        EventPlanError::new(EventPlanErrorKind::BadValue, error.to_string()).at_path(&at_path)
    })?;
    let call_type = match object.get("callType") {
        None => None,
        Some(value) => {
            let call_path = format!("{path}.callType");
            let raw = value.as_str().ok_or_else(|| {
                EventPlanError::new(EventPlanErrorKind::BadValue, "callType must be a string")
                    .at_path(&call_path)
            })?;
            Some(FormCallType::from_xml(raw).ok_or_else(|| {
                EventPlanError::new(
                    EventPlanErrorKind::BadValue,
                    "callType must be Before, After, or Override",
                )
                .at_path(call_path)
            })?)
        }
    };
    Ok(EventImplementArgs { at, call_type })
}

pub(crate) fn plan_event_implement_batch(
    mut staged: ApplyStagedState,
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    operations: &[EventImplementArgs],
) -> Result<(ApplyStagedState, PlannedApplyEffects), EventPlanError> {
    let mut effects = PlannedApplyEffects::default();
    for operation in operations {
        plan_one_event(
            &mut staged,
            &mut effects,
            source_set,
            source_set_kind,
            profile,
            operation,
        )?;
    }
    Ok((staged, effects))
}

fn plan_one_event(
    staged: &mut ApplyStagedState,
    effects: &mut PlannedApplyEffects,
    source_set: &str,
    source_set_kind: SourceSetKind,
    profile: PlatformProfile,
    args: &EventImplementArgs,
) -> Result<(), EventPlanError> {
    if profile.module_prefix_capability(&args.at).is_some() {
        let source = resolve_event_source(source_set, source_set_kind, profile, &args.at)
            .map_err(|error| provider_unavailable(error.to_string()))?;
        let LogicalEventSource::Platform(source) = source else {
            return Err(provider_unavailable(
                "event source did not retain a Platform module",
            ));
        };
        if args.call_type.is_some() {
            return Err(EventPlanError::new(
                EventPlanErrorKind::BadValue,
                "Platform events do not accept callType",
            ));
        }
        let owner_plan = platform_event_owner_evidence(source_set_kind, &source)
            .map_err(|error| provider_unavailable(error.to_string()))?;
        prove_owner_plan(staged, &owner_plan)?;
        let module_before = staged.read(&source.module_relative).map_err(staging)?;
        let module_text = optional_utf8_module(module_before.as_deref())?;
        let capability = profile
            .module_capability(&source.module_at)
            .ok_or_else(|| provider_unavailable("Platform module capability disappeared"))?;
        let shape =
            project_platform_event_implementation(&source, capability, module_text.as_deref())
                .map_err(projection_error)?;
        require_plannable_state(&shape)?;
        let module_after = emit_event_module(module_before.as_deref(), &shape)?;
        let post_text = utf8_module(&module_after)?;
        let post = project_platform_event_implementation(&source, capability, Some(post_text))
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
        .ok_or_else(|| EventPlanError::new(EventPlanErrorKind::NotFound, "Form.xml is absent"))?;
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
        patch_form_event(&form_before, &source, &shape, effective_call_type)?
    } else {
        form_before.clone()
    };
    let module_after = emit_event_module(module_before.as_deref(), &shape)?;
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

fn prove_owner_plan(
    staged: &mut ApplyStagedState,
    plan: &EventOwnerEvidencePlan,
) -> Result<(), EventPlanError> {
    let mut evidence = Vec::<PlatformXmlSourceSetOwnerEvidence>::new();
    for (index, step) in plan.descriptors.iter().enumerate() {
        if let Some(parent) = step.registered_by {
            let parent = evidence.get(parent).ok_or_else(|| {
                provider_unavailable("owner evidence plan references an unavailable parent")
            })?;
            let name = step.expected_name.as_deref().ok_or_else(|| {
                provider_unavailable("metadata owner expectation has no exact name")
            })?;
            if !parent.registers(&step.expected_kind, name) {
                return Err(EventPlanError::new(
                    EventPlanErrorKind::NotFound,
                    format!(
                        "owner `{}` is not registered by descriptor {}",
                        step.expected_kind,
                        index.saturating_sub(1)
                    ),
                ));
            }
        }
        let bytes = staged
            .read(&step.relative)
            .map_err(staging)?
            .ok_or_else(|| {
                provider_unavailable(format!(
                    "proved owner descriptor `{}` is absent",
                    step.relative.display()
                ))
            })?;
        let actual = match step.proof {
            EventOwnerDescriptorProof::SourceSet => {
                prove_already_read_source_set_owner(&step.relative, &bytes, plan.source_set_kind)
            }
            EventOwnerDescriptorProof::Metadata => {
                prove_already_read_metadata_owner(&step.relative, &bytes)
            }
        }
        .map_err(|error| provider_unavailable(error.message))?;
        if actual.artifact_kind() != step.expected_kind
            || step
                .expected_name
                .as_deref()
                .is_some_and(|name| actual.artifact_name() != Some(name))
            || actual.version() != Some("2.20")
            || (step.proof == EventOwnerDescriptorProof::SourceSet
                && matches!(
                    plan.source_set_kind,
                    SourceSetKind::Configuration | SourceSetKind::Extension
                )
                && actual.is_configuration_extension()
                    != (plan.source_set_kind == SourceSetKind::Extension))
        {
            return Err(provider_unavailable(format!(
                "owner descriptor `{}` has inconsistent identity or format",
                step.relative.display()
            )));
        }
        evidence.push(actual);
    }
    Ok(())
}

fn property_call_type(
    shape: &EventImplementationShape,
    definition: &FormDefinitionKind,
    direct_part_writable: bool,
    requested: Option<FormCallType>,
) -> Result<Option<FormCallType>, EventPlanError> {
    match shape.projection.state {
        EventState::Available if !direct_part_writable => Err(EventPlanError::new(
            EventPlanErrorKind::InvalidState,
            "BaseForm-only Part 1 has no writable direct owner",
        )),
        EventState::Available if *definition == FormDefinitionKind::Extension => {
            requested.map(Some).ok_or_else(|| {
                EventPlanError::new(
                    EventPlanErrorKind::BadValue,
                    "borrowed Property Available requires callType",
                )
            })
        }
        EventState::Available if requested.is_some() => Err(EventPlanError::new(
            EventPlanErrorKind::BadValue,
            "regular Property Available forbids callType",
        )),
        EventState::Available => Ok(None),
        EventState::Missing if requested.is_some() => Err(EventPlanError::new(
            EventPlanErrorKind::BadValue,
            "Property Missing forbids a new callType",
        )),
        EventState::Missing => Ok(shape
            .projection
            .call_type
            .as_deref()
            .and_then(FormCallType::from_xml)),
        EventState::Implemented | EventState::Invalid => Err(EventPlanError::new(
            EventPlanErrorKind::InvalidState,
            "event state is not plannable",
        )),
    }
}

fn require_plannable_state(shape: &EventImplementationShape) -> Result<(), EventPlanError> {
    match shape.projection.state {
        EventState::Available | EventState::Missing => Ok(()),
        EventState::Implemented | EventState::Invalid => Err(EventPlanError::new(
            EventPlanErrorKind::InvalidState,
            format!("event is {:?}", shape.projection.state),
        )),
    }
}

fn require_postcondition(
    before: &EventImplementationShape,
    after: &EventImplementationShape,
    call_type: Option<FormCallType>,
) -> Result<(), EventPlanError> {
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

fn stage_postimage(
    staged: &mut ApplyStagedState,
    relative: &std::path::Path,
    before: Option<&[u8]>,
    after: Vec<u8>,
) -> Result<(), EventPlanError> {
    match before {
        Some(before) => staged.replace(relative, before, after).map_err(staging),
        None => staged.create(relative, after).map_err(staging),
    }
}

fn optional_utf8_module(bytes: Option<&[u8]>) -> Result<Option<String>, EventPlanError> {
    bytes
        .map(utf8_module)
        .transpose()
        .map(|value| value.map(str::to_string))
}

fn utf8_module(bytes: &[u8]) -> Result<&str, EventPlanError> {
    utf8_source(bytes, "BSL module")
}

fn utf8_source<'a>(bytes: &'a [u8], label: &str) -> Result<&'a str, EventPlanError> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    std::str::from_utf8(bytes).map_err(|_| invalid_source(format!("{label} is not valid UTF-8")))
}

fn staging(error: ApplyStagingError) -> EventPlanError {
    EventPlanError::new(EventPlanErrorKind::Staging(error.kind()), error.to_string())
}

fn provider_unavailable(message: impl Into<String>) -> EventPlanError {
    EventPlanError::new(EventPlanErrorKind::ProviderUnavailable, message)
}

fn postcondition(message: impl Into<String>) -> EventPlanError {
    EventPlanError::new(EventPlanErrorKind::Postcondition, message)
}

fn projection_error(error: EventProjectionError) -> EventPlanError {
    match error.kind() {
        EventProjectionErrorKind::ProviderUnavailable => provider_unavailable(error.to_string()),
        EventProjectionErrorKind::InvalidSource => invalid_source(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::address::QualifiedAddress;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::module_projection::{BindingFact, EventProjection, EventState, MethodKind};
    use crate::domain::project_sources::SourceSetKind;
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::bsl_module_projection::RequiredEventDirective;
    use crate::infrastructure::event_projection::{
        EventImplementationOwner, EventImplementationShape,
    };
    use crate::infrastructure::logical_event_source::{
        EventOwnerDescriptorExpectation, EventOwnerDescriptorProof,
    };
    use crate::infrastructure::logical_event_source::{
        EventOwnerEvidencePlan, PropertyEventOwner, PropertyEventOwnerKind,
    };
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn shape(
        owner: EventImplementationOwner,
        handler: &str,
        signature: &str,
    ) -> EventImplementationShape {
        EventImplementationShape {
            projection: EventProjection {
                at: "main:CommonForm.Main.Event.OnOpen".to_string(),
                event_id: "OnOpen".to_string(),
                state: EventState::Available,
                signature: signature.to_string(),
                contexts: vec!["thinClient".to_string()],
                binding: BindingFact::Property,
                handler: handler.to_string(),
                handler_en: String::new(),
                implementation_at: None,
                call_type: None,
                can: Vec::new(),
            },
            owner,
            method_kind: MethodKind::Procedure,
            handler: handler.to_string(),
            implementation_at: format!("main:CommonForm.Main.Module.Form.Method.{handler}"),
            signature: signature.to_string(),
            directive: RequiredEventDirective::Client,
            region: match owner {
                EventImplementationOwner::Form => "ОбработчикиСобытийФормы",
                _ => "ОбработчикиСобытий",
            },
        }
    }

    fn form_source() -> PropertyEventSource {
        let form_at = QualifiedAddress::parse("main:CommonForm.Main").unwrap();
        PropertyEventSource {
            event_at: QualifiedAddress::parse("main:CommonForm.Main.Event.OnOpen").unwrap(),
            form_at: form_at.clone(),
            form_target: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonForm.Main")
                .unwrap(),
            form_xml_relative: PathBuf::from("CommonForms/Main/Ext/Form.xml"),
            module_relative: PathBuf::from("CommonForms/Main/Ext/Form/Module.bsl"),
            module_at: QualifiedAddress::parse("main:CommonForm.Main.Module.Form").unwrap(),
            owner_chain: vec![PropertyEventOwner {
                kind: PropertyEventOwnerKind::Form,
                at: form_at,
            }],
            descriptor_requirements: vec![PathBuf::from("CommonForms/Main.xml")],
            owner_evidence: EventOwnerEvidencePlan {
                source_set_kind: SourceSetKind::Configuration,
                descriptors: Vec::new(),
            },
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-event-b1b-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn staged(root: &std::path::Path) -> ApplyStagedState {
        ApplyStagedState::from_retained_root(
            Arc::new(
                RetainedDirectoryCapability::open(&std::fs::canonicalize(root).unwrap()).unwrap(),
            ),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            CancellationToken::new(),
            crate::infrastructure::workspace_actor::apply_writer_authority_for_test(),
        )
    }

    fn common_form_owner_plan() -> EventOwnerEvidencePlan {
        EventOwnerEvidencePlan {
            source_set_kind: SourceSetKind::Configuration,
            descriptors: vec![
                EventOwnerDescriptorExpectation {
                    relative: PathBuf::from("Configuration.xml"),
                    proof: EventOwnerDescriptorProof::SourceSet,
                    expected_kind: "Configuration".to_string(),
                    expected_name: None,
                    registered_by: None,
                },
                EventOwnerDescriptorExpectation {
                    relative: PathBuf::from("CommonForms/Main.xml"),
                    proof: EventOwnerDescriptorProof::Metadata,
                    expected_kind: "CommonForm".to_string(),
                    expected_name: Some("Main".to_string()),
                    registered_by: Some(0),
                },
            ],
        }
    }

    fn write_catalog_form_fixture(root: &std::path::Path) -> Vec<u8> {
        const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
        std::fs::create_dir_all(root.join("Catalogs/Products/Forms/Main/Ext")).unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration><Properties><Name>Fixture</Name></Properties><ChildObjects><Catalog>Products</Catalog></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Catalogs/Products.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Form>Main</Form></ChildObjects></Catalog></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Catalogs/Products/Forms/Main.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"
            ),
        )
        .unwrap();
        let form = concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
            "\t<ChildItems/>\r\n",
            "\t<Commands/>\r\n",
            "</Form>\r\n"
        )
        .as_bytes()
        .to_vec();
        std::fs::write(
            root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
            &form,
        )
        .unwrap();
        form
    }

    fn write_extension_catalog_form_fixture(root: &std::path::Path, direct_part: bool) -> Vec<u8> {
        const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
        std::fs::create_dir_all(root.join("Catalogs/Products/Forms/Main/Ext")).unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration><Properties><Name>FixtureExtension</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties><ChildObjects><Catalog>Products</Catalog></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Catalogs/Products.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Form>Main</Form></ChildObjects></Catalog></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Catalogs/Products/Forms/Main.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"
            ),
        )
        .unwrap();
        let direct = if direct_part {
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n\t<ChildItems/>\r\n\t<Commands/>\r\n"
        } else {
            ""
        };
        let form = format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n{direct}\t<BaseForm version=\"2.20\"><AutoCommandBar name=\"BaseBar\" id=\"-1\"/></BaseForm>\r\n</Form>\r\n"
        )
        .into_bytes();
        std::fs::write(
            root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
            &form,
        )
        .unwrap();
        form
    }

    fn write_external_fixture(root: &std::path::Path, kind: &str, name: &str) {
        const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
        std::fs::write(
            root.join(format!("{name}.xml")),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><{kind}><Properties><Name>{name}</Name></Properties><ChildObjects><Command>Run</Command></ChildObjects></{kind}></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(root.join(format!("{name}/Commands"))).unwrap();
        std::fs::write(
            root.join(format!("{name}/Commands/Run.xml")),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Command><Properties><Name>Run</Name></Properties></Command></MetaDataObject>"
            ),
        )
        .unwrap();
    }

    #[test]
    fn absent_module_emission_has_exact_platform_bom_lf_and_terminal_newline() {
        let actual = emit_event_module(
            None,
            &shape(
                EventImplementationOwner::Form,
                "ПриОткрытии",
                "Процедура ПриОткрытии(Отказ)",
            ),
        )
        .unwrap();
        assert_eq!(
            actual,
            concat!(
                "\u{feff}#Область ОбработчикиСобытийФормы\n",
                "\n",
                "&НаКлиенте\n",
                "Процедура ПриОткрытии(Отказ)\n",
                "\n",
                "КонецПроцедуры\n",
                "\n",
                "#КонецОбласти\n"
            )
            .as_bytes()
        );
    }

    #[test]
    fn module_emitter_preserves_profiles_untouched_bytes_and_exact_directives() {
        let mut client = shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        let crlf = concat!(
            "\u{feff}// keep\r\n",
            "#Область ОбработчикиСобытийФормы\r\n",
            "// inside\r\n",
            "#КонецОбласти\r\n",
            "// tail"
        );
        let actual = emit_event_module(Some(crlf.as_bytes()), &client).unwrap();
        assert!(actual.starts_with("\u{feff}// keep\r\n".as_bytes()));
        let actual = String::from_utf8(actual).unwrap();
        assert!(actual.contains("&НаКлиенте\r\nПроцедура ПриОткрытии(Отказ)\r\n"));
        assert!(actual.ends_with("#КонецОбласти\r\n// tail"));

        client.directive = RequiredEventDirective::Server;
        client.handler = "ПриСозданииНаСервере".to_string();
        client.signature =
            "Процедура ПриСозданииНаСервере(Отказ, СтандартнаяОбработка)".to_string();
        let no_bom = b"// untouched";
        let server = emit_event_module(Some(no_bom), &client).unwrap();
        assert!(!server.starts_with(b"\xef\xbb\xbf"));
        let server = String::from_utf8(server).unwrap();
        assert!(server.starts_with("// untouched\n\n#Область"));
        assert!(server.contains("&НаСервере\nПроцедура ПриСозданииНаСервере"));

        client.directive = RequiredEventDirective::None;
        client.method_kind = MethodKind::Function;
        client.handler = "Result".to_string();
        client.signature = "Функция Result()".to_string();
        let function = String::from_utf8(emit_event_module(Some(b""), &client).unwrap()).unwrap();
        assert!(!function.contains("&На"));
        assert!(function.contains("Функция Result()\n\nКонецФункции\n"));
    }

    #[test]
    fn module_emitter_fails_closed_on_source_region_and_unicode_method_ambiguity() {
        let shape = shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        for source in [
            "// lone\rcr",
            "Процедура Broken(\n",
            "#Область ОбработчикиСобытийФормы\n#Область Inner\n#КонецОбласти\n",
            "#Область ОбработчикиСобытийФормы\n#КонецОбласти\n#Область обработчикисобытийформы\n#КонецОбласти\n",
            "#Область Outer\n#Область ОбработчикиСобытийФормы\n#КонецОбласти\n#КонецОбласти\n",
        ] {
            let error = emit_event_module(Some(source.as_bytes()), &shape).unwrap_err();
            assert_eq!(error.kind(), EventPlanErrorKind::InvalidSource, "{source:?}");
        }
        for source in [
            "Процедура приоткрытии(Отказ)\nКонецПроцедуры\n",
            "Procedure ПРИОТКРЫТИИ(Cancel)\nEndProcedure\n",
        ] {
            let error = emit_event_module(Some(source.as_bytes()), &shape).unwrap_err();
            assert_eq!(error.kind(), EventPlanErrorKind::InvalidState, "{source:?}");
        }
    }

    #[test]
    fn module_emitter_uses_the_terminal_site_in_mixed_sources_and_rejects_an_ambiguous_site() {
        let shape = shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        let unambiguous_terminal = "// preserved CRLF\r\n// preserved LF\n// terminal";
        let emitted = String::from_utf8(
            emit_event_module(Some(unambiguous_terminal.as_bytes()), &shape)
                .expect("the terminal insertion site has an unambiguous LF profile"),
        )
        .unwrap();
        assert!(emitted.starts_with(unambiguous_terminal));
        assert!(emitted.contains("// terminal\n\n#Область ОбработчикиСобытийФормы\n\n&НаКлиенте\n"));

        let ambiguous_region = concat!(
            "// preserved CRLF\r\n",
            "#Область ОбработчикиСобытийФормы\n",
            "// previous LF\n",
            "#КонецОбласти\r\n",
            "// tail CRLF\r\n"
        );
        let error = emit_event_module(Some(ambiguous_region.as_bytes()), &shape)
            .expect_err("different EOLs adjacent to the real region site are ambiguous");
        assert_eq!(error.kind(), EventPlanErrorKind::InvalidSource);
    }

    #[test]
    fn direct_form_event_patch_changes_only_the_root_owner_range() {
        let before = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
            "\t<ChildItems/>\r\n",
            "</Form>"
        );
        let actual = patch_form_event(
            before.as_bytes(),
            &form_source(),
            &shape(
                EventImplementationOwner::Form,
                "ПриОткрытии",
                "Процедура ПриОткрытии(Отказ)",
            ),
            None,
        )
        .unwrap();
        let text = String::from_utf8(actual).unwrap();
        assert!(text.contains(
            "\r\n\t<Events>\r\n\t\t<Event name=\"OnOpen\">ПриОткрытии</Event>\r\n\t</Events>\r\n"
        ));
        assert!(text.ends_with("\t<ChildItems/>\r\n</Form>"));
    }

    #[test]
    fn form_patcher_uses_the_actual_site_in_mixed_sources_and_rejects_an_ambiguous_site() {
        let unambiguous_site = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
            "\t<ChildItems/>\n",
            "</Form>\n"
        );
        let patched = String::from_utf8(
            patch_form_event(
                unambiguous_site.as_bytes(),
                &form_source(),
                &shape(
                    EventImplementationOwner::Form,
                    "ПриОткрытии",
                    "Процедура ПриОткрытии(Отказ)",
                ),
                None,
            )
            .expect("the actual insertion site is locally LF even though the owner start is not"),
        )
        .unwrap();
        assert!(patched.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Form"));
        assert!(patched.contains(
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n\t<Events>\n\t\t<Event name=\"OnOpen\">ПриОткрытии</Event>\n\t</Events>\n"
        ));

        let ambiguous_site = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
            "\t<ChildItems/>\r\n",
            "</Form>\r\n"
        );
        let error = patch_form_event(
            ambiguous_site.as_bytes(),
            &form_source(),
            &shape(
                EventImplementationOwner::Form,
                "ПриОткрытии",
                "Процедура ПриОткрытии(Отказ)",
            ),
            None,
        )
        .expect_err("different EOLs adjacent to the real insertion site are ambiguous");
        assert_eq!(error.kind(), EventPlanErrorKind::InvalidSource);
    }

    #[test]
    fn form_patcher_uses_complete_item_chain_and_command_action_without_touching_siblings() {
        let xml = concat!(
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
            "\t<ChildItems>\n",
            "\t\t<Table name=\"First\" id=\"1\"><ChildItems><InputField name=\"Value\" id=\"2\"/></ChildItems></Table>\n",
            "\t\t<Table name=\"Second\" id=\"3\"><ChildItems><InputField name=\"Value\" id=\"4\"/></ChildItems></Table>\n",
            "\t</ChildItems>\n",
            "\t<Commands><Command name=\"Заполнить\"><Title>keep</Title></Command></Commands>\n",
            "</Form>\n"
        );
        let form_at = QualifiedAddress::parse("main:Catalog.Products.Form.Main").unwrap();
        let nested_source = PropertyEventSource {
            event_at: QualifiedAddress::parse(
                "main:Catalog.Products.Form.Main.Item.Second.Item.Value.Event.OnChange",
            )
            .unwrap(),
            form_at: form_at.clone(),
            form_target: MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "Catalog.Products.Form.Main",
            )
            .unwrap(),
            form_xml_relative: PathBuf::from("Form.xml"),
            module_relative: PathBuf::from("Module.bsl"),
            module_at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Module.Form")
                .unwrap(),
            owner_chain: vec![
                PropertyEventOwner {
                    kind: PropertyEventOwnerKind::Form,
                    at: form_at.clone(),
                },
                PropertyEventOwner {
                    kind: PropertyEventOwnerKind::Table,
                    at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Item.Second")
                        .unwrap(),
                },
                PropertyEventOwner {
                    kind: PropertyEventOwnerKind::Column,
                    at: QualifiedAddress::parse(
                        "main:Catalog.Products.Form.Main.Item.Second.Item.Value",
                    )
                    .unwrap(),
                },
            ],
            descriptor_requirements: Vec::new(),
            owner_evidence: EventOwnerEvidencePlan {
                source_set_kind: SourceSetKind::Configuration,
                descriptors: Vec::new(),
            },
        };
        let mut nested_shape = shape(
            EventImplementationOwner::Column,
            "ValueПриИзменении",
            "Процедура ValueПриИзменении(Элемент)",
        );
        nested_shape.projection.event_id = "OnChange".to_string();
        let patched = String::from_utf8(
            patch_form_event(xml.as_bytes(), &nested_source, &nested_shape, None).unwrap(),
        )
        .unwrap();
        let first = patched.find("name=\"First\"").unwrap();
        let second = patched.find("name=\"Second\"").unwrap();
        let event = patched.find("ValueПриИзменении").unwrap();
        assert!(first < second && second < event);
        assert_eq!(patched.matches("ValueПриИзменении").count(), 1);
        assert!(patched.contains("<Title>keep</Title>"));

        let command_source = PropertyEventSource {
            event_at: QualifiedAddress::parse(
                "main:Catalog.Products.Form.Main.Command.Заполнить.Event.Execute",
            )
            .unwrap(),
            owner_chain: vec![
                PropertyEventOwner {
                    kind: PropertyEventOwnerKind::Form,
                    at: form_at.clone(),
                },
                PropertyEventOwner {
                    kind: PropertyEventOwnerKind::Command,
                    at: QualifiedAddress::parse(
                        "main:Catalog.Products.Form.Main.Command.Заполнить",
                    )
                    .unwrap(),
                },
            ],
            ..nested_source
        };
        let mut command_shape = shape(
            EventImplementationOwner::Command,
            "ЗаполнитьОбработкаКоманды",
            "Процедура ЗаполнитьОбработкаКоманды(Команда)",
        );
        command_shape.projection.event_id = "Execute".to_string();
        let command = String::from_utf8(
            patch_form_event(
                xml.as_bytes(),
                &command_source,
                &command_shape,
                Some(FormCallType::After),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(command.contains("<Action callType=\"After\">ЗаполнитьОбработкаКоманды</Action>"));
        assert!(!command.contains("<Event name=\"Execute\""));
        assert!(command.find("<Title>keep</Title>").unwrap() < command.find("<Action").unwrap());
    }

    #[test]
    fn form_patcher_rejects_wrong_namespace_owner_lookalikes() {
        let xml = concat!(
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:foreign=\"urn:foreign\" version=\"2.20\">\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
            "\t<ChildItems><foreign:InputField name=\"Field\" id=\"1\"/></ChildItems>\n",
            "</Form>\n"
        );
        let form_at = QualifiedAddress::parse("main:CommonForm.Main").unwrap();
        let source = PropertyEventSource {
            event_at: QualifiedAddress::parse("main:CommonForm.Main.Item.Field.Event.OnChange")
                .unwrap(),
            owner_chain: vec![
                PropertyEventOwner {
                    kind: PropertyEventOwnerKind::Form,
                    at: form_at.clone(),
                },
                PropertyEventOwner {
                    kind: PropertyEventOwnerKind::Element,
                    at: QualifiedAddress::parse("main:CommonForm.Main.Item.Field").unwrap(),
                },
            ],
            ..form_source()
        };
        let error = patch_form_event(
            xml.as_bytes(),
            &source,
            &shape(
                EventImplementationOwner::Element,
                "FieldПриИзменении",
                "Процедура FieldПриИзменении(Элемент)",
            ),
            None,
        )
        .unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::InvalidSource);
    }

    #[test]
    fn form_patcher_rejects_duplicate_existing_event_slots_even_off_target() {
        let xml = concat!(
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
            "\t<Events>\n",
            "\t\t<Event name=\"OnClose\">First</Event>\n",
            "\t\t<Event name=\"OnClose\">Second</Event>\n",
            "\t</Events>\n",
            "</Form>\n"
        );
        let error = patch_form_event(
            xml.as_bytes(),
            &form_source(),
            &shape(
                EventImplementationOwner::Form,
                "ПриОткрытии",
                "Процедура ПриОткрытии(Отказ)",
            ),
            None,
        )
        .unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::InvalidSource);
    }

    #[test]
    fn planned_effects_deduplicate_by_kind_and_artifact_in_first_occurrence_order() {
        let mut effects = PlannedApplyEffects::default();
        effects.append(effect(DomainEventKind::ModuleChanged, "main:Module.Form"));
        effects.append(effect(DomainEventKind::FormChanged, "main:Form.Main"));
        effects.append(effect(DomainEventKind::ModuleChanged, "main:Module.Form"));
        assert_eq!(effects.events().len(), 2);
        assert_eq!(effects.events()[0].kind, DomainEventKind::ModuleChanged);
        assert_eq!(effects.events()[1].kind, DomainEventKind::FormChanged);
    }

    #[test]
    fn call_type_rules_and_argument_paths_are_closed_and_typed() {
        let available = shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        for call_type in [
            FormCallType::Before,
            FormCallType::After,
            FormCallType::Override,
        ] {
            assert_eq!(
                property_call_type(
                    &available,
                    &FormDefinitionKind::Extension,
                    true,
                    Some(call_type),
                )
                .unwrap(),
                Some(call_type)
            );
        }
        assert_eq!(
            property_call_type(&available, &FormDefinitionKind::Extension, true, None,)
                .unwrap_err()
                .kind(),
            EventPlanErrorKind::BadValue
        );
        assert_eq!(
            property_call_type(
                &available,
                &FormDefinitionKind::Regular,
                true,
                Some(FormCallType::After),
            )
            .unwrap_err()
            .kind(),
            EventPlanErrorKind::BadValue
        );
        assert_eq!(
            property_call_type(
                &available,
                &FormDefinitionKind::Extension,
                false,
                Some(FormCallType::After),
            )
            .unwrap_err()
            .kind(),
            EventPlanErrorKind::InvalidState
        );

        let bad = parse_event_implement_args(
            &serde_json::json!({"at": "main:Catalog.Products.Form.Main.Event.OnOpen", "extra": true}),
            "ops[4].args",
        )
        .unwrap_err();
        assert_eq!(bad.kind(), EventPlanErrorKind::BadValue);
        assert_eq!(bad.path(), Some("ops[4].args.extra"));
        let bad = parse_event_implement_args(
            &serde_json::json!({"at": "main:Catalog.Products.Form.Main.Event.OnOpen", "callType": "Later"}),
            "ops[5].args",
        )
        .unwrap_err();
        assert_eq!(bad.path(), Some("ops[5].args.callType"));
    }

    #[test]
    fn owner_proof_distinguishes_unregistered_from_registered_inconsistent_descriptor() {
        const HEADER: &str =
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">";
        let root = temp_root("owner-proof");
        std::fs::create_dir_all(root.join("CommonForms")).unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            format!("{HEADER}<Configuration><ChildObjects/></Configuration></MetaDataObject>"),
        )
        .unwrap();
        std::fs::write(
            root.join("CommonForms/Main.xml"),
            format!("{HEADER}<CommonForm><Properties><Name>Main</Name></Properties></CommonForm></MetaDataObject>"),
        )
        .unwrap();
        let error = prove_owner_plan(&mut staged(&root), &common_form_owner_plan()).unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::NotFound);

        std::fs::write(
            root.join("Configuration.xml"),
            format!("{HEADER}<Configuration><ChildObjects><CommonForm>Main</CommonForm></ChildObjects></Configuration></MetaDataObject>"),
        )
        .unwrap();
        std::fs::write(root.join("CommonForms/Main.xml"), b"not xml").unwrap();
        let error = prove_owner_plan(&mut staged(&root), &common_form_owner_plan()).unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::ProviderUnavailable);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn postprojection_rejects_a_wrong_handler_or_missing_implementation_identity() {
        let before = shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        let mut after = before.clone();
        after.projection.state = EventState::Implemented;
        after.projection.handler = "ЧужойОбработчик".to_string();
        after.projection.implementation_at = None;
        let error = require_postcondition(&before, &after, None).unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::Postcondition);

        let mut wrong_identity = before.clone();
        wrong_identity.projection.state = EventState::Implemented;
        wrong_identity.projection.binding = BindingFact::Platform;
        wrong_identity.projection.implementation_at =
            Some("main:CommonForm.Main.Module.Form.Method.Other".to_string());
        let error = require_postcondition(&before, &wrong_identity, None).unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::Postcondition);
    }

    #[test]
    fn staged_property_planner_composes_two_events_and_poisoned_batch_publishes_nothing() {
        let root = temp_root("staged-planner");
        let original_form = write_catalog_form_fixture(&root);
        let operations = ["OnOpen", "OnClose"].map(|event| EventImplementArgs {
            at: QualifiedAddress::parse(&format!("main:Catalog.Products.Form.Main.Event.{event}"))
                .unwrap(),
            call_type: None,
        });
        let (planned, effects) = plan_event_implement_batch(
            staged(&root),
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &operations,
        )
        .unwrap();
        let changes = planned.planned_changes();
        assert_eq!(changes.len(), 2);
        let form_after = changes
            .iter()
            .find(|change| change.relative_path.ends_with("Form.xml"))
            .and_then(|change| match &change.current {
                crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) => {
                    Some(bytes)
                }
                _ => None,
            })
            .unwrap();
        let form_after = String::from_utf8(form_after.clone()).unwrap();
        assert!(form_after.contains("name=\"OnOpen\""));
        assert!(form_after.contains("name=\"OnClose\""));
        assert_eq!(effects.events().len(), 2, "Form+Module effects deduplicate");
        assert_eq!(effects.events()[0].kind, DomainEventKind::FormChanged);
        assert_eq!(effects.events()[1].kind, DomainEventKind::ModuleChanged);
        assert_eq!(
            std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
            original_form,
            "planning never publishes source bytes"
        );
        assert!(!root
            .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
            .exists());

        let duplicate = [operations[0].clone(), operations[0].clone()];
        let error = plan_event_implement_batch(
            staged(&root),
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &duplicate,
        )
        .unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::InvalidState);
        assert_eq!(
            std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
            original_form
        );
        assert!(!root
            .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
            .exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_planner_extends_compact_existing_events_inside_root_and_arbitrary_nested_owners() {
        struct Case {
            name: &'static str,
            form: &'static str,
            target: &'static str,
            owner_marker: &'static str,
            owner_close: &'static str,
            owner_name: Option<&'static str>,
            existing_event: &'static str,
            target_event: &'static str,
            handler: &'static str,
        }

        let cases = [
            Case {
                name: "compact-root-events",
                form: concat!(
                    "\u{feff}<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
                    "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
                    "\t<Events><Event name=\"OnClose\">ExistingClose</Event></Events>\n",
                    "\t<ChildItems/>\n",
                    "\t<Commands/>\n",
                    "</Form>"
                ),
                target: "main:Catalog.Products.Form.Main.Event.OnOpen",
                owner_marker: "<Form ",
                owner_close: "</Form>",
                owner_name: None,
                existing_event: "OnClose",
                target_event: "OnOpen",
                handler: "ПриОткрытии",
            },
            Case {
                name: "compact-arbitrary-nested-events",
                form: concat!(
                    "\u{feff}<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
                    "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
                    "\t<ChildItems>\n",
                    "\t\t<UsualGroup name=\"Outer\" id=\"1\"><ChildItems><Table name=\"Goods\" id=\"2\"><Events><Event name=\"Selection\">ExistingSelection</Event></Events><ChildItems/></Table></ChildItems></UsualGroup>\n",
                    "\t</ChildItems>\n",
                    "\t<Commands/>\n",
                    "</Form>"
                ),
                target: "main:Catalog.Products.Form.Main.Item.Outer.Item.Goods.Event.BeforeAddRow",
                owner_marker: "<Table name=\"Goods\"",
                owner_close: "</Table>",
                owner_name: Some("Goods"),
                existing_event: "Selection",
                target_event: "BeforeAddRow",
                handler: "GoodsПередНачаломДобавления",
            },
        ];

        for case in cases {
            let root = temp_root(case.name);
            write_catalog_form_fixture(&root);
            let form_path = root.join("Catalogs/Products/Forms/Main/Ext/Form.xml");
            std::fs::write(&form_path, case.form.as_bytes()).unwrap();
            let (planned, _) = plan_event_implement_batch(
                staged(&root),
                "main",
                SourceSetKind::Configuration,
                PlatformProfile::v8_3_27(),
                &[EventImplementArgs {
                    at: QualifiedAddress::parse(case.target).unwrap(),
                    call_type: None,
                }],
            )
            .expect("compact direct Events must remain the exact projected owner");
            let form_after = planned
                .planned_changes()
                .iter()
                .find(|change| change.relative_path.ends_with("Form.xml"))
                .and_then(|change| match &change.current {
                    crate::infrastructure::native_operations::apply::StagedFileState::Bytes(
                        bytes,
                    ) => Some(String::from_utf8(bytes.clone()).unwrap()),
                    _ => None,
                })
                .unwrap();

            let document = Document::parse(form_after.trim_start_matches('\u{feff}')).unwrap();
            let owner = match case.owner_name {
                None => document.root_element(),
                Some(name) => document
                    .descendants()
                    .find(|node| node.is_element() && node.attribute("name") == Some(name))
                    .unwrap(),
            };
            let events = owner
                .children()
                .filter(Node::is_element)
                .filter(|node| node.tag_name().name() == "Events")
                .collect::<Vec<_>>();
            assert_eq!(events.len(), 1, "{}", case.name);
            let direct_names = events[0]
                .children()
                .filter(Node::is_element)
                .filter(|node| node.tag_name().name() == "Event")
                .filter_map(|node| node.attribute("name"))
                .collect::<Vec<_>>();
            assert_eq!(
                direct_names,
                vec![case.existing_event, case.target_event],
                "{}",
                case.name
            );
            assert!(form_after.contains(case.handler), "{}", case.name);

            let before_start = case.form.find(case.owner_marker).unwrap();
            let after_start = form_after.find(case.owner_marker).unwrap();
            assert_eq!(
                &case.form[..before_start],
                &form_after[..after_start],
                "bytes before the exact owner changed in {}",
                case.name
            );
            let before_end = case.form[before_start..]
                .find(case.owner_close)
                .map(|relative| before_start + relative + case.owner_close.len())
                .unwrap();
            let after_end = form_after[after_start..]
                .find(case.owner_close)
                .map(|relative| after_start + relative + case.owner_close.len())
                .unwrap();
            assert_eq!(
                &case.form[before_end..],
                &form_after[after_end..],
                "bytes after the exact owner changed in {}",
                case.name
            );
            assert_eq!(std::fs::read_to_string(&form_path).unwrap(), case.form);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn staged_regular_owner_matrix_composes_exact_form_and_module_postimages() {
        let root = temp_root("regular-owner-matrix");
        write_catalog_form_fixture(&root);
        let form_path = root.join("Catalogs/Products/Forms/Main/Ext/Form.xml");
        let form = concat!(
            "\u{feff}<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
            "\t<ChildItems>\n",
            "\t\t<InputField name=\"Field\" id=\"1\"/>\n",
            "\t\t<Table name=\"Goods\" id=\"2\"><ChildItems><InputField name=\"Quantity\" id=\"3\"/></ChildItems></Table>\n",
            "\t</ChildItems>\n",
            "\t<Commands><Command name=\"Заполнить\"><Title>keep</Title></Command></Commands>\n",
            "</Form>"
        )
        .as_bytes()
        .to_vec();
        std::fs::write(&form_path, &form).unwrap();
        let targets = [
            "main:Catalog.Products.Form.Main.Event.OnOpen",
            "main:Catalog.Products.Form.Main.Item.Field.Event.OnChange",
            "main:Catalog.Products.Form.Main.Item.Goods.Event.BeforeAddRow",
            "main:Catalog.Products.Form.Main.Item.Goods.Item.Quantity.Event.OnChange",
            "main:Catalog.Products.Form.Main.Command.Заполнить.Event.Execute",
        ];
        let operations = targets.map(|target| EventImplementArgs {
            at: QualifiedAddress::parse(target).unwrap(),
            call_type: None,
        });
        let (planned, effects) = plan_event_implement_batch(
            staged(&root),
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &operations,
        )
        .unwrap();
        assert_eq!(planned.planned_changes().len(), 2);
        let form_after = planned
            .planned_changes()
            .iter()
            .find(|change| change.relative_path.ends_with("Form.xml"))
            .and_then(|change| match &change.current {
                crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) => {
                    Some(String::from_utf8(bytes.clone()).unwrap())
                }
                _ => None,
            })
            .unwrap();
        for binding in [
            "<Event name=\"OnOpen\">ПриОткрытии</Event>",
            "<Event name=\"OnChange\">FieldПриИзменении</Event>",
            "<Event name=\"BeforeAddRow\">GoodsПередНачаломДобавления</Event>",
            "<Event name=\"OnChange\">QuantityПриИзменении</Event>",
            "<Action>ЗаполнитьОбработкаКоманды</Action>",
        ] {
            assert!(form_after.contains(binding), "missing binding {binding}");
        }
        assert!(form_after.starts_with('\u{feff}'));
        assert!(!form_after.ends_with('\n') && !form_after.ends_with('\r'));
        let module_after = planned
            .planned_changes()
            .iter()
            .find(|change| change.relative_path.ends_with("Module.bsl"))
            .and_then(|change| match &change.current {
                crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) => {
                    Some(String::from_utf8(bytes.clone()).unwrap())
                }
                _ => None,
            })
            .unwrap();
        assert!(module_after.contains("Процедура ЗаполнитьОбработкаКоманды(Команда)"));
        assert!(!module_after.contains("CommandProcessing"));
        assert_eq!(effects.events().len(), 2);
        assert_eq!(std::fs::read(&form_path).unwrap(), form);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn borrowed_available_accepts_all_call_types_and_base_form_only_stays_byte_exact() {
        for call_type in [
            FormCallType::Before,
            FormCallType::After,
            FormCallType::Override,
        ] {
            let root = temp_root(call_type.as_str());
            let original = write_extension_catalog_form_fixture(&root, true);
            let (planned, _) = plan_event_implement_batch(
                staged(&root),
                "main",
                SourceSetKind::Extension,
                PlatformProfile::v8_3_27(),
                &[EventImplementArgs {
                    at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Event.OnOpen")
                        .unwrap(),
                    call_type: Some(call_type),
                }],
            )
            .unwrap();
            let form_after = planned
                .planned_changes()
                .iter()
                .find(|change| change.relative_path.ends_with("Form.xml"))
                .and_then(|change| match &change.current {
                    crate::infrastructure::native_operations::apply::StagedFileState::Bytes(
                        bytes,
                    ) => Some(String::from_utf8(bytes.clone()).unwrap()),
                    _ => None,
                })
                .unwrap();
            assert!(form_after.starts_with('\u{feff}'));
            assert!(form_after.ends_with("\r\n"));
            assert!(form_after.contains(&format!(
                "<Event name=\"OnOpen\" callType=\"{}\">ПриОткрытии</Event>",
                call_type.as_str()
            )));
            assert_eq!(
                std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
                original
            );
            std::fs::remove_dir_all(root).unwrap();
        }

        let root = temp_root("base-form-only");
        let original = write_extension_catalog_form_fixture(&root, false);
        let error = plan_event_implement_batch(
            staged(&root),
            "main",
            SourceSetKind::Extension,
            PlatformProfile::v8_3_27(),
            &[EventImplementArgs {
                at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Event.OnOpen")
                    .unwrap(),
                call_type: Some(FormCallType::After),
            }],
        )
        .unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::InvalidState);
        assert_eq!(
            std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
            original
        );
        assert!(!root
            .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
            .exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_missing_property_keeps_form_byte_exact_and_platform_creates_only_module() {
        let root = temp_root("missing-and-platform");
        write_catalog_form_fixture(&root);
        let form_path = root.join("Catalogs/Products/Forms/Main/Ext/Form.xml");
        let missing_form = concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
            "\t<Events><Event name=\"OnOpen\">ExistingOpen</Event></Events>\r\n",
            "\t<ChildItems/>\r\n",
            "\t<Commands/>\r\n",
            "</Form>\r\n"
        )
        .as_bytes()
        .to_vec();
        std::fs::write(&form_path, &missing_form).unwrap();
        let missing = [EventImplementArgs {
            at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Event.OnOpen").unwrap(),
            call_type: None,
        }];
        let (planned, effects) = plan_event_implement_batch(
            staged(&root),
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &missing,
        )
        .unwrap();
        let changes = planned.planned_changes();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].relative_path.ends_with("Module.bsl"));
        assert_eq!(effects.events().len(), 1);
        assert_eq!(effects.events()[0].kind, DomainEventKind::ModuleChanged);
        assert_eq!(std::fs::read(&form_path).unwrap(), missing_form);

        let clean_root = temp_root("platform-module");
        write_catalog_form_fixture(&clean_root);
        let platform = [EventImplementArgs {
            at: QualifiedAddress::parse("main:Catalog.Products.Module.Object.Event.BeforeWrite")
                .unwrap(),
            call_type: None,
        }];
        let (planned, effects) = plan_event_implement_batch(
            staged(&clean_root),
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &platform,
        )
        .unwrap();
        let changes = planned.planned_changes();
        assert_eq!(changes.len(), 1);
        let bytes = match &changes[0].current {
            crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) => bytes,
            _ => panic!("Platform module must be created"),
        };
        assert!(bytes.starts_with(b"\xef\xbb\xbf"));
        assert!(String::from_utf8_lossy(bytes).contains("Процедура ПередЗаписью"));
        assert_eq!(effects.events().len(), 1);
        assert_eq!(effects.events()[0].kind, DomainEventKind::ModuleChanged);
        assert!(!clean_root
            .join("Catalogs/Products/Ext/ObjectModule.bsl")
            .exists());
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(clean_root).unwrap();
    }

    #[test]
    fn unsupported_frozen_catalog_event_is_provider_unavailable_not_malformed_source() {
        let root = temp_root("unsupported-platform-event");
        write_catalog_form_fixture(&root);
        let error = plan_event_implement_batch(
            staged(&root),
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &[EventImplementArgs {
                at: QualifiedAddress::parse(
                    "main:Catalog.Products.Module.Object.Event.DoesNotExist",
                )
                .unwrap(),
                call_type: None,
            }],
        )
        .unwrap_err();
        assert_eq!(error.kind(), EventPlanErrorKind::ProviderUnavailable);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_epf_and_erf_platform_owners_plan_only_proved_module_postimages() {
        let epf = temp_root("external-epf");
        write_external_fixture(&epf, "ExternalDataProcessor", "Import");
        let (planned, effects) = plan_event_implement_batch(
            staged(&epf),
            "epf",
            SourceSetKind::ExternalProcessor,
            PlatformProfile::v8_3_27(),
            &[
                EventImplementArgs {
                    at: QualifiedAddress::parse(
                        "epf:ExternalDataProcessor.Import.Module.Object.Event.FillCheckProcessing",
                    )
                    .unwrap(),
                    call_type: None,
                },
                EventImplementArgs {
                    at: QualifiedAddress::parse(
                        "epf:ExternalDataProcessor.Import.Command.Run.Module.Command.Event.CommandProcessing",
                    )
                    .unwrap(),
                    call_type: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(planned.planned_changes().len(), 2);
        assert_eq!(effects.events().len(), 2);
        assert!(!epf.join("Import/Ext/ObjectModule.bsl").exists());
        assert!(!epf
            .join("Import/Commands/Run/Ext/CommandModule.bsl")
            .exists());
        std::fs::remove_dir_all(epf).unwrap();

        let erf = temp_root("external-erf");
        write_external_fixture(&erf, "ExternalReport", "Sales");
        let (planned, effects) = plan_event_implement_batch(
            staged(&erf),
            "erf",
            SourceSetKind::ExternalReport,
            PlatformProfile::v8_3_27(),
            &[EventImplementArgs {
                at: QualifiedAddress::parse(
                    "erf:ExternalReport.Sales.Module.Object.Event.OnComposeResult",
                )
                .unwrap(),
                call_type: None,
            }],
        )
        .unwrap();
        assert_eq!(planned.planned_changes().len(), 1);
        assert_eq!(effects.events().len(), 1);
        assert!(!erf.join("Sales/Ext/ObjectModule.bsl").exists());
        std::fs::remove_dir_all(erf).unwrap();
    }
}
