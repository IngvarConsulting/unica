//! Byte-preserving direct Form event slots over exact typed owner chains.
use roxmltree::{Document, Node};
use std::ops::Range;
use unica_forms::event_targets::{PropertyEventOwner, PropertyEventOwnerKind};
use unica_metadata::event::FormCallType;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
use unica_xml_format::text::{
    event_line_start, event_local_line_ending, reject_event_lone_cr, LineEnding, LineEndingProfile,
    SourceTextSnapshot, Utf8Bom,
};
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

pub fn patch_form_event(
    current: &[u8],
    owners: &[PropertyEventOwner],
    event_id: &str,
    handler: &str,
    call_type: Option<FormCallType>,
) -> Result<Vec<u8>, ApplyPlanError> {
    const FORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
    let snapshot = SourceTextSnapshot::from_bytes(current)
        .map_err(|error| invalid_source(error.to_string()))?;
    reject_event_lone_cr(&snapshot).map_err(invalid_source)?;
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
    let owner = resolve_form_owner(root, owners, FORM_NS)?;
    let insertion_plan = if owners
        .last()
        .is_some_and(|owner| owner.kind == PropertyEventOwnerKind::Command)
    {
        plan_command_action(text, owner, event_id, handler, call_type, FORM_NS)?
    } else {
        plan_property_event(text, owner, event_id, handler, call_type, FORM_NS)?
    };
    let eol = event_local_line_ending(text, insertion_plan.offset())
        .map_err(invalid_source)?
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
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
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

fn exact_direct_children<'a, 'input>(
    owner: Node<'a, 'input>,
    local: &str,
    namespace: &str,
) -> Result<Vec<Node<'a, 'input>>, ApplyPlanError> {
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
) -> Result<Node<'a, 'input>, ApplyPlanError> {
    match exact_direct_children(owner, local, namespace)?.as_slice() {
        [node] => Ok(*node),
        [] => Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!("Form.xml direct `{local}` owner is absent"),
        )),
        _ => Err(invalid_source(format!(
            "Form.xml direct `{local}` owner is duplicated"
        ))),
    }
}

fn resolve_form_owner<'a, 'input>(
    root: Node<'a, 'input>,
    owners: &[PropertyEventOwner],
    namespace: &str,
) -> Result<Node<'a, 'input>, ApplyPlanError> {
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
            .and_then(unica_address::address::AddressSegment::name)
            .ok_or_else(|| invalid_source("typed Command owner has no exact name"))?;
        let matches = exact_direct_children(commands, "Command", namespace)?
            .into_iter()
            .filter(|node| node.attribute("name") == Some(name))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [node] => Ok(*node),
            [] => Err(ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
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
            .and_then(unica_address::address::AddressSegment::name)
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
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
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
    event_id: &str,
    handler: &str,
    call_type: Option<FormCallType>,
    namespace: &str,
) -> Result<FormInsertionPlan, ApplyPlanError> {
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
            .any(|event| event.attribute("name") == Some(event_id))
        {
            return Err(invalid_source(
                "direct event slot is duplicated or already present",
            ));
        }
        let event_xml = property_event_xml(event_id, handler, call_type);
        return plan_into_container(body, events.range(), "Events", event_xml);
    }
    let indent = line_indent(body, owner.range().start);
    let section_indent = format!("{indent}\t");
    let event_indent = format!("{section_indent}\t");
    let event_xml = property_event_xml(event_id, handler, call_type);
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
    _event_id: &str,
    handler: &str,
    call_type: Option<FormCallType>,
    namespace: &str,
) -> Result<FormInsertionPlan, ApplyPlanError> {
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
        escape_xml(handler)
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

fn property_event_xml(event_id: &str, handler: &str, call_type: Option<FormCallType>) -> String {
    let call_type = call_type
        .map(|value| format!(" callType=\"{}\"", value.as_str()))
        .unwrap_or_default();
    format!(
        "<Event name=\"{}\"{call_type}>{}</Event>",
        escape_xml(event_id),
        escape_xml(handler)
    )
}

fn plan_into_container(
    body: &str,
    range: Range<usize>,
    tag: &str,
    child: String,
) -> Result<FormInsertionPlan, ApplyPlanError> {
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
        let start = event_line_start(body, close);
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
    let start = event_line_start(body, offset);
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
) -> Result<FormInsertionPlan, ApplyPlanError> {
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
        let start = event_line_start(body, close);
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

fn line_indent(text: &str, offset: usize) -> String {
    let start = event_line_start(text, offset);
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

fn invalid_source(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
}
