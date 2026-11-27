//! Byte-preserving direct Form event slots over exact typed owner chains.
use crate::root::{form_edit_opening_tag_end, require_form_root, FORM_LOGFORM_NS};
use roxmltree::{Document, Node};
use std::ops::Range;
use unica_address::address::{AddressSegment, NodeKind};
use unica_forms::event_targets::{PropertyEventOwner, PropertyEventOwnerKind};
use unica_forms::events::{validate_event, FormElementKind, FormEventBinding, FormEventTarget};
use unica_metadata::event::FormCallType;
use unica_source_contract::format_identity::PlatformLine;
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
        || !crate::dialect::root_version::is_writable(root.attribute("version"))
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
    let names = owners[1..]
        .iter()
        .map(|owner| {
            owner
                .at
                .segments()
                .last()
                .and_then(AddressSegment::name)
                .ok_or_else(|| invalid_source("typed Item owner has no exact name"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    resolve_item_chain(root, names)
}

fn resolve_item_chain<'a, 'input, 'name>(
    root: Node<'a, 'input>,
    names: impl IntoIterator<Item = &'name str>,
) -> Result<Node<'a, 'input>, ApplyPlanError> {
    let mut owner = root;
    for name in names {
        let sections = exact_direct_children(owner, "ChildItems", FORM_LOGFORM_NS)?;
        if sections.len() > 1 {
            return Err(invalid_source("direct ChildItems section is duplicated"));
        }
        let bars = exact_direct_children(owner, "AutoCommandBar", FORM_LOGFORM_NS)?;
        let mut named = sections
            .iter()
            .flat_map(|section| section.children())
            .chain(bars)
            .filter(|node| node.is_element() && node.attribute("name") == Some(name));
        let item = named.next().ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("form Item '{name}' is absent from the exact owner chain"),
            )
        })?;
        if named.next().is_some() || item.tag_name().namespace() != Some(FORM_LOGFORM_NS) {
            return Err(invalid_source(format!(
                "Form.xml contains an ambiguous Item '{name}'"
            )));
        }
        owner = item;
    }
    Ok(owner)
}

#[derive(Debug, Clone)]
pub enum LocalEventEdit {
    Replace {
        handler: String,
        call_type: Option<FormCallType>,
    },
    Unbind,
}

fn local_event_slot<'a, 'input>(
    owner: Node<'a, 'input>,
    event_id: &str,
) -> Result<(Node<'a, 'input>, Node<'a, 'input>), ApplyPlanError> {
    let events = exactly_one_direct(owner, "Events", FORM_LOGFORM_NS)?;
    let slots = exact_direct_children(events, "Event", FORM_LOGFORM_NS)?;
    let mut matching = slots
        .into_iter()
        .filter(|slot| slot.attribute("name") == Some(event_id));
    let slot = matching.next().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!("the local binding for form event '{event_id}' is absent"),
        )
    })?;
    if matching.next().is_some() {
        return Err(invalid_source(format!(
            "local form event '{event_id}' is duplicated"
        )));
    }
    Ok((events, slot))
}

pub fn has_local_event_binding<'a>(
    xml: &str,
    items: impl IntoIterator<Item = &'a str>,
    event_id: &str,
) -> bool {
    let Ok(document) = Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return false;
    };
    let root = document.root_element();
    require_form_root(root).is_ok()
        && resolve_item_chain(root, items)
            .and_then(|owner| local_event_slot(owner, event_id))
            .is_ok()
}

pub fn edit_local_event(
    xml: &mut String,
    items: &[String],
    event_id: &str,
    edit: &LocalEventEdit,
    metadata_owner: Option<NodeKind>,
    platform_line: Option<PlatformLine>,
) -> Result<(), ApplyPlanError> {
    let document = Document::parse(xml).map_err(|error| invalid_source(error.to_string()))?;
    let root = document.root_element();
    require_form_root(root).map_err(invalid_source)?;
    let owner = resolve_item_chain(root, items.iter().map(String::as_str))?;
    let (events, slot) = local_event_slot(owner, event_id)?;

    match edit {
        LocalEventEdit::Replace { handler, call_type } => {
            let mut context = crate::event_context::context_from_root(root, platform_line);
            context.metadata_owner = metadata_owner;
            let target = if owner == root {
                FormEventTarget::Form
            } else {
                let kind =
                    FormElementKind::from_xml_tag(owner.tag_name().name()).ok_or_else(|| {
                        invalid_source("the event owner has an unsupported element type")
                    })?;
                FormEventTarget::Element(kind)
            };
            let effective_call_type = call_type
                .map(FormCallType::as_str)
                .or_else(|| slot.attribute("callType"));
            let data_path = crate::codec::form_child_text(owner, "DataPath");
            let binding = FormEventBinding {
                name: event_id,
                handler,
                call_type: effective_call_type,
                data_path: data_path.as_deref(),
            };
            crate::codec::validate_event_owner_node(owner, target, &binding)
                .and_then(|_| validate_event(&context, target, &binding))
                .map_err(|error| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string())
                })?;

            let range = slot.range();
            let replacement = replace_event_xml(xml, slot, handler, *call_type)?;
            xml.replace_range(range, &replacement);
        }
        LocalEventEdit::Unbind => {
            // The platform omits an empty Events section. Keep any comments
            // and attributes on the existing section.
            let remove_section = events.attributes().len() == 0
                && events.children().all(|node| {
                    node == slot
                        || node.is_text() && node.text().is_some_and(|text| text.trim().is_empty())
                });
            let target = if remove_section { events } else { slot };
            let range = crate::root::element_with_leading_space(xml, target);
            xml.replace_range(range, "");
        }
    }
    Ok(())
}

fn replace_event_xml(
    xml: &str,
    slot: Node<'_, '_>,
    handler: &str,
    call_type: Option<FormCallType>,
) -> Result<String, ApplyPlanError> {
    let range = slot.range();
    let opening_end = form_edit_opening_tag_end(xml, range.start)
        .ok_or_else(|| invalid_source("form event has no complete opening tag"))?;
    let mut edited = xml[range.clone()].to_string();
    let relative_end = opening_end - range.start;
    let self_closing = edited.ends_with("/>");
    if slot.text().unwrap_or("") != handler {
        if slot.children().any(|node| !node.is_text()) {
            return Err(invalid_source("form event handler must be plain text"));
        }
        if self_closing {
            let tag = edited[1..]
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .expect("parsed event has a qualified tag name");
            let closing = format!(">{}</{tag}>", escape_xml(handler));
            edited.replace_range(relative_end - 1.., &closing);
        } else {
            let closing_start = edited
                .rfind("</")
                .ok_or_else(|| invalid_source("form event has no closing tag"))?;
            edited.replace_range(relative_end + 1..closing_start, &escape_xml(handler));
        }
    }
    if let Some(desired) = call_type {
        match slot.attribute_node("callType") {
            Some(attribute) if attribute.value() != desired.as_str() => {
                let attr = attribute.range_value();
                edited.replace_range(
                    attr.start - range.start..attr.end - range.start,
                    desired.as_str(),
                );
            }
            None => {
                let insertion = relative_end - usize::from(self_closing);
                edited.insert_str(insertion, &format!(" callType=\"{}\"", desired.as_str()));
            }
            _ => {}
        }
    }
    Ok(edited)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_call_type_changes_only_the_target_binding() {
        let before = concat!(
            "<Form xmlns='http://v8.1c.ru/8.3/xcf/logform' version='2.20'>\r\n",
            "\t<Events>\r\n",
            "\t\t<Event callType = 'After' name='OnCreateAtServer'> Old </Event>\r\n",
            "\t\t<Event name='OnOpen' callType='After'>Other</Event>\r\n",
            "\t</Events>\r\n",
            "\t<BaseForm version='2.20'><Events>",
            "<Event name='OnCreateAtServer'>Regular</Event>",
            "</Events></BaseForm>\r\n",
            "</Form>",
        );
        let mut xml = before.to_string();
        edit_local_event(
            &mut xml,
            &[],
            "OnCreateAtServer",
            &LocalEventEdit::Replace {
                handler: "Old".to_string(),
                call_type: Some(FormCallType::Before),
            },
            Some(NodeKind::Catalog),
            None,
        )
        .unwrap();

        assert_eq!(
            xml,
            before
                .replacen("callType = 'After'", "callType = 'Before'", 1)
                .replacen("> Old <", ">Old<", 1)
        );
    }

    #[test]
    fn unbind_reaches_a_nested_item_and_keeps_the_base_form() {
        let target = "<Events><Event name='OnChange' callType='After'>Local</Event></Events>";
        let items = format!(
            "<ChildItems><InputField name='Field' id='4'>{target}</InputField></ChildItems>"
        );
        let base = "<BaseForm version='2.20'><ChildItems><InputField name='Field' id='4'>\
                    <Events><Event name='OnChange'>Base</Event></Events>\
                    </InputField></ChildItems></BaseForm>";
        let before = format!(
            "<Form xmlns='{FORM_LOGFORM_NS}' version='2.20'><ChildItems>\
             <UsualGroup name='Group' id='3'>{items}</UsualGroup></ChildItems>{base}</Form>"
        );
        let mut xml = before.clone();
        assert!(has_local_event_binding(
            &xml,
            ["Group", "Field"],
            "OnChange"
        ));

        edit_local_event(
            &mut xml,
            &["Group".to_string(), "Field".to_string()],
            "OnChange",
            &LocalEventEdit::Unbind,
            None,
            None,
        )
        .unwrap();

        assert_eq!(xml, before.replace(target, ""));
        assert!(!has_local_event_binding(
            &xml,
            ["Group", "Field"],
            "OnChange"
        ));
        assert!(!has_local_event_binding(&xml, ["Field"], "OnChange"));
    }
    #[test]
    fn ambiguous_or_foreign_slots_are_neither_observed_nor_edited() {
        for sections in [
            "<Events/><Events><Event name='OnOpen'>Handler</Event></Events>",
            "<Events><foreign:Event name='OnOpen'>Handler</foreign:Event></Events>",
            "<Events><Event name='OnOpen'>First</Event><Event name='OnOpen'>Second</Event></Events>",
        ] {
            let before = format!(
                "<Form xmlns='{FORM_LOGFORM_NS}' xmlns:foreign='urn:foreign' version='2.20'>{sections}</Form>"
            );
            let mut xml = before.clone();
            assert!(!has_local_event_binding(&xml, [], "OnOpen"));

            let error = edit_local_event(
                &mut xml,
                &[],
                "OnOpen",
                &LocalEventEdit::Unbind,
                None,
                None,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
            assert_eq!(xml, before);
        }
    }
}
