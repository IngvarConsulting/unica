//! Lossless relocation of existing managed-form subtrees. The public apply
//! planner owns argument validation, sequencing and transactional publication.

use roxmltree::{Document, Node};
use std::ops::Range;

use crate::root::FORM_LOGFORM_NS as NS;

#[derive(Debug, Clone)]
pub enum FormMoveDestination {
    Root,
    Named(String),
}

#[derive(Debug, Clone)]
pub struct FormItemMove {
    pub name: String,
    pub source_path: Option<Vec<String>>,
    pub into: FormMoveDestination,
    pub after: Option<String>,
}

#[derive(Debug)]
pub struct MoveError {
    pub field: &'static str,
    pub message: String,
}

fn error(field: &'static str, message: impl Into<String>) -> MoveError {
    MoveError {
        field,
        message: message.into(),
    }
}

fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|node| node.has_tag_name((NS, name)))
}

fn working(node: Node<'_, '_>) -> bool {
    !node
        .ancestors()
        .any(|node| node.has_tag_name((NS, "BaseForm")))
        && (node
            .ancestors()
            .any(|node| node.has_tag_name((NS, "ChildItems")))
            || node.has_tag_name((NS, "AutoCommandBar")))
}

fn named<'a>(
    root: Node<'a, 'a>,
    name: &str,
    field: &'static str,
) -> Result<Node<'a, 'a>, MoveError> {
    let mut found = root.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(NS)
            && node.attribute("name") == Some(name)
            && working(*node)
            && (node
                .parent_element()
                .is_some_and(|p| p.has_tag_name((NS, "ChildItems")))
                || matches!(
                    node.tag_name().name(),
                    "AutoCommandBar"
                        | "ContextMenu"
                        | "ExtendedTooltip"
                        | "SearchStringAddition"
                        | "ViewStatusAddition"
                        | "SearchControlAddition"
                ))
    });
    let node = found.next().ok_or_else(|| {
        error(
            field,
            format!("form element `{name}` was not found in the working tree"),
        )
    })?;
    if found.next().is_some() {
        return Err(error(field, format!("form element `{name}` is ambiguous")));
    }
    Ok(node)
}

fn opening_end(xml: &str, node: Node<'_, '_>) -> usize {
    crate::root::form_edit_opening_tag_end(xml, node.range().start)
        .expect("parsed XML has a complete opening tag")
}

/// Read attribute names lexically, including xmlns (roxmltree excludes them
/// from attributes). Quoted values can themselves contain `xmlns` or `>`.
fn declared_namespace(opening: &str, prefix: Option<&str>) -> bool {
    let wanted = prefix.map_or_else(|| "xmlns".to_string(), |p| format!("xmlns:{p}"));
    let bytes = opening.as_bytes();
    let mut i = 1;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'>' | b'/') {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == bytes.len() || matches!(bytes[i], b'>' | b'/') {
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let name = &opening[start..i];
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'=') {
            i += 1;
        }
        if name == wanted {
            return true;
        }
        let Some(&quote) = bytes.get(i) else {
            break;
        };
        i += 1;
        while bytes.get(i).is_some_and(|byte| *byte != quote) {
            i += 1;
        }
        i += 1;
    }
    false
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
        .replace('\t', "&#9;")
}

/// Меняем только форматирующие whitespace-узлы между элементами. Значимый
/// текст, комментарии и CRLF остаются нетронутыми, а глубина поддерева
/// становится такой же, как у платформенной выгрузки в новом контейнере.
fn reindent_fragment(xml: &str, source: Node<'_, '_>, target_prefix: &str) -> String {
    let source_range = source.range();
    let source_indent = line_indent(xml, source_range.start);
    let target_indent = target_prefix.rsplit('\n').next().unwrap_or("");
    if source_indent == target_indent {
        return xml[source_range].to_string();
    }
    let from = format!("\n{source_indent}");
    let to = format!("\n{target_indent}");
    let mut edits = source
        .descendants()
        .filter(|node| {
            node.is_text()
                && node.text().is_some_and(|text| {
                    text.contains('\n') && text.bytes().all(|byte| byte.is_ascii_whitespace())
                })
                && node
                    .parent()
                    .is_some_and(|parent| parent.children().any(|child| child.is_element()))
        })
        .filter_map(|node| {
            let range = node.range();
            let rewritten = xml[range.clone()].replace(&from, &to);
            (rewritten != xml[range.clone()]).then_some((
                range.start - source_range.start..range.end - source_range.start,
                rewritten,
            ))
        })
        .collect::<Vec<_>>();
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut fragment = xml[source_range].to_string();
    for (range, replacement) in edits {
        fragment.replace_range(range, &replacement);
    }
    fragment
}

fn portable_fragment(
    xml: &str,
    source: Node<'_, '_>,
    destination: Node<'_, '_>,
    target_prefix: &str,
) -> String {
    let range = source.range();
    let open_end = opening_end(xml, source);
    let opening = &xml[range.start..=open_end];
    let mut declarations = String::new();
    let source_default = source.lookup_namespace_uri(None).unwrap_or("");
    let destination_default = destination.lookup_namespace_uri(None).unwrap_or("");
    if source_default != destination_default && !declared_namespace(opening, None) {
        declarations.push_str(&format!(" xmlns=\"{}\"", escape_attribute(source_default)));
    }
    for namespace in source.namespaces() {
        let prefix = namespace.name();
        if prefix.is_none()
            || destination.lookup_namespace_uri(prefix) == Some(namespace.uri())
            || declared_namespace(opening, prefix)
        {
            continue;
        }
        let key = prefix.map_or_else(|| "xmlns".to_string(), |prefix| format!("xmlns:{prefix}"));
        declarations.push_str(&format!(" {key}=\"{}\"", escape_attribute(namespace.uri())));
    }
    let mut fragment = reindent_fragment(xml, source, target_prefix);
    let insertion = open_end - range.start - usize::from(xml.as_bytes()[open_end - 1] == b'/');
    fragment.insert_str(insertion, &declarations);
    fragment
}

fn cut_range(xml: &str, node: Node<'_, '_>) -> Range<usize> {
    let mut range = node.range();
    if let Some(previous) = node.prev_sibling().filter(|node| node.is_text()) {
        let preceding = &xml[previous.range()];
        // Formatting belongs to the following element; meaningful text and
        // comments outside the subtree stay in their original container.
        if preceding.bytes().all(|b| b.is_ascii_whitespace()) {
            range.start = previous.range().start;
        }
    }
    range
}

/// Последний перенесённый элемент не оставляет секцию, которую платформа
/// сама удаляет. Если `ChildItems` был единственным содержимым владельца,
/// удаляем и оба пробельных узла: иначе отступ закрывающего тега переживёт
/// последующий перенос владельца.
fn emptied_child_items_range(
    child_items: Node<'_, '_>,
    source: Node<'_, '_>,
) -> Option<Range<usize>> {
    let becomes_empty = child_items.children().all(|node| {
        node == source
            || node.is_text()
                && node
                    .text()
                    .is_some_and(|text| text.bytes().all(|byte| byte.is_ascii_whitespace()))
    });
    if !becomes_empty {
        return None;
    }
    let mut range = child_items.range();
    if let Some(previous) = child_items.prev_sibling().filter(|node| {
        node.is_text()
            && node
                .text()
                .is_some_and(|text| text.bytes().all(|byte| byte.is_ascii_whitespace()))
    }) {
        range.start = previous.range().start;
    }
    let owner_has_other_elements = child_items.parent_element().is_some_and(|owner| {
        owner
            .children()
            .any(|node| node.is_element() && node != child_items)
    });
    if !owner_has_other_elements {
        if let Some(next) = child_items.next_sibling().filter(|node| {
            node.is_text()
                && node
                    .text()
                    .is_some_and(|text| text.bytes().all(|byte| byte.is_ascii_whitespace()))
        }) {
            range.end = next.range().end;
        }
    }
    Some(range)
}

fn line_ending(xml: &str) -> &'static str {
    if xml.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn line_indent(xml: &str, position: usize) -> &str {
    let line_start = xml[..position].rfind('\n').map_or(0, |index| index + 1);
    let indent = &xml[line_start..position];
    if indent.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
        indent
    } else {
        ""
    }
}

/// Префикс прямого потомка в стиле текущего контейнера. Если потомки уже
/// есть, берём их фактический отступ; для пустого платформенного узла строим
/// следующий табличный уровень и сохраняем CRLF/LF документа.
fn child_prefix(xml: &str, parent: Node<'_, '_>) -> String {
    if !xml.contains('\n') {
        return String::new();
    }
    if let Some(prefix) = parent
        .children()
        .find(|node| !node.is_text())
        .and_then(|node| node.prev_sibling())
        .filter(Node::is_text)
        .map(|node| &xml[node.range()])
        .filter(|text| text.bytes().all(|byte| byte.is_ascii_whitespace()))
    {
        return prefix.to_string();
    }
    format!(
        "{}{}\t",
        line_ending(xml),
        line_indent(xml, parent.range().start)
    )
}

fn nested_prefix(prefix: &str) -> String {
    if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}\t")
    }
}

fn end_insertion(xml: &str, node: Node<'_, '_>, fragment: &str) -> (Range<usize>, String) {
    let range = node.range();
    let end = opening_end(xml, node);
    if xml.as_bytes()[end - 1] == b'/' {
        let qname_end = xml[range.start + 1..]
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap()
            + range.start
            + 1;
        let qname = &xml[range.start + 1..qname_end];
        let closing_prefix = if fragment.contains('\n') {
            format!(
                "{}{}",
                line_ending(xml),
                line_indent(xml, node.range().start)
            )
        } else {
            String::new()
        };
        return (
            end - 1..end + 1,
            format!(">{fragment}{closing_prefix}</{qname}>"),
        );
    }
    let pos = node
        .last_child()
        .filter(|node| node.is_text() && node.text().is_some_and(|s| s.trim().is_empty()))
        .map(|node| node.range().start)
        .unwrap_or_else(|| {
            range.start + xml[range.clone()].rfind("</").expect("element closing tag")
        });
    (pos..pos, fragment.to_string())
}

pub fn move_item(xml: &str, request: &FormItemMove) -> Result<String, MoveError> {
    let document =
        Document::parse(xml).map_err(|e| error("name", format!("invalid form XML: {e}")))?;
    let root = document.root_element();
    crate::root::require_form_root(root).map_err(|e| error("name", e))?;
    let source = named(root, &request.name, "name")?;
    if let Some(expected) = &request.source_path {
        let mut actual: Vec<_> = source
            .ancestors()
            .take_while(|node| *node != root)
            .filter_map(|node| node.attribute("name"))
            .collect();
        actual.reverse();
        if actual
            .iter()
            .copied()
            .ne(expected.iter().map(String::as_str))
        {
            return Err(error(
                "name",
                "the source element is not at the addressed Item path in the current form",
            ));
        }
    }
    let source_items = source.parent_element().filter(|node| node.has_tag_name((NS, "ChildItems")))
        .ok_or_else(|| error("name", "only an element directly owned by working ChildItems may be moved; companions cannot be separated"))?;
    let destination = match &request.into {
        FormMoveDestination::Named(name) => named(root, name, "into")?,
        FormMoveDestination::Root => root,
    };
    if destination.ancestors().any(|node| node == source) {
        return Err(error(
            "into",
            "an element cannot be moved into itself or its descendant",
        ));
    }
    let target_items = child(destination, "ChildItems");
    let after = request
        .after
        .as_ref()
        .map(|name| named(root, name, "after"))
        .transpose()?;
    if let Some(after) = after {
        if after == source {
            return Err(error("after", "after cannot name the element being moved"));
        }
        if after.parent_element() != target_items {
            return Err(error(
                "after",
                "after must be a direct child of the destination ChildItems",
            ));
        }
    }
    validate_placement(source, destination)?;
    if target_items == Some(source_items) {
        let previous = source.prev_siblings().skip(1).find(Node::is_element);
        let next = source.next_siblings().skip(1).find(Node::is_element);
        if after.is_some() && previous == after || after.is_none() && next.is_none() {
            return Ok(xml.to_string());
        }
    }
    let cut =
        emptied_child_items_range(source_items, source).unwrap_or_else(|| cut_range(xml, source));
    let namespace_parent = target_items.unwrap_or(destination);
    let destination_prefix = target_items
        .map(|items| child_prefix(xml, items))
        .unwrap_or_else(|| nested_prefix(&child_prefix(xml, destination)));
    let portable = portable_fragment(xml, source, namespace_parent, &destination_prefix);
    let (insert, replacement) = if let Some(after) = after {
        let pos = after.range().end;
        (pos..pos, format!("{}{}", destination_prefix, portable))
    } else if let Some(items) = target_items {
        let fragment = format!("{}{}", destination_prefix, portable);
        end_insertion(xml, items, &fragment)
    } else {
        let prefix = if destination.lookup_namespace_uri(None) == Some(NS) {
            String::new()
        } else {
            format!(
                "{}:",
                destination
                    .lookup_prefix(NS)
                    .expect("logform namespace is in scope")
            )
        };
        let section_prefix = child_prefix(xml, destination);
        let section = format!(
            "<{prefix}ChildItems>{destination_prefix}{portable}{section_prefix}</{prefix}ChildItems>"
        );
        if destination == root {
            if let Some(anchor) = root.children().find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(NS)
                    && matches!(
                        node.tag_name().name(),
                        "Attributes" | "Parameters" | "Commands"
                    )
            }) {
                let pos = anchor.range().start;
                (pos..pos, format!("{section}{section_prefix}"))
            } else {
                end_insertion(xml, destination, &format!("{section_prefix}{section}"))
            }
        } else {
            end_insertion(xml, destination, &format!("{section_prefix}{section}"))
        }
    };
    let mut edits = [(cut, String::new()), (insert, replacement)];
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = xml.to_string();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Document::parse(&result)
        .map_err(|e| error("into", format!("move produced invalid XML: {e}")))?;
    Ok(result)
}

// The platform observations and the complete type/context matrix live in
// tests/fixtures/platform_8_3_27/form_move. A successful platform Move alone
// is insufficient: 1C can convert the kind/path or silently keep the parent.
fn is_field(kind: &str) -> bool {
    matches!(
        kind,
        "InputField"
            | "LabelField"
            | "CheckBoxField"
            | "PictureField"
            | "HTMLDocumentField"
            | "CalendarField"
            | "RadioButtonField"
            | "ProgressBarField"
            | "TrackBarField"
            | "SpreadSheetDocumentField"
            | "FormattedDocumentField"
            | "TextDocumentField"
            | "ChartField"
            | "GanttChartField"
            | "GraphicalSchemaField"
            | "PlannerField"
            | "PeriodField"
            | "DendrogramField"
            | "GeographicalSchemaField"
            | "PDFDocumentField"
    )
}

fn property(node: Node<'_, '_>, name: &str) -> Option<String> {
    let value: String = child(node, name)?
        .children()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect();
    (!value.is_empty()).then_some(value)
}

fn table_context<'a>(node: Node<'a, 'a>) -> Option<Node<'a, 'a>> {
    node.ancestors()
        .find(|node| node.has_tag_name((NS, "Table")))
}

fn validate_placement(source: Node<'_, '_>, destination: Node<'_, '_>) -> Result<(), MoveError> {
    let kind = source.tag_name().name();
    let parent = destination.tag_name().name();
    let button_type = property(source, "Type");
    let button_type = button_type.as_deref().unwrap_or("UsualButton");
    let compatible = match parent {
        "Pages" => kind == "Page",
        "Form" | "UsualGroup" | "Page" => {
            matches!(
                kind,
                "UsualGroup"
                    | "Pages"
                    | "Table"
                    | "CommandBar"
                    | "LabelDecoration"
                    | "PictureDecoration"
            ) || is_field(kind) && table_context(source).is_none()
                || kind == "Button" && matches!(button_type, "UsualButton" | "Hyperlink")
        }
        "CommandBar" | "AutoCommandBar" | "ButtonGroup" | "Popup" | "ContextMenu" => {
            matches!(kind, "ButtonGroup" | "Popup")
                || kind == "Button"
                    && matches!(button_type, "CommandBarButton" | "CommandBarHyperlink")
        }
        "Table" | "ColumnGroup" if kind == "ColumnGroup" || is_field(kind) => {
            return validate_column_context(source, destination);
        }
        _ => false,
    };
    if compatible {
        Ok(())
    } else {
        Err(error(
            "into",
            format!(
                "{kind} cannot be placed in {parent}; this destination does not accept that element kind"
            ),
        ))
    }
}

fn validate_column_context(
    source: Node<'_, '_>,
    destination: Node<'_, '_>,
) -> Result<(), MoveError> {
    let Some(target_table) = table_context(destination) else {
        return Err(error(
            "into",
            "a column group destination must belong to a table",
        ));
    };
    let Some(source_table) = table_context(source) else {
        return Err(error(
            "into",
            "moving an ordinary field into a table would require changing its binding",
        ));
    };
    // Reordering inside the same table does not reinterpret even an opaque
    // platform binding; no path parsing or rewriting is necessary.
    if source_table == target_table {
        return Ok(());
    }
    if is_field(source.tag_name().name()) && property(source, "DataPath").is_none() {
        return Err(error("into", "moving a column without DataPath between tables is temporarily unsupported: the platform does not instantiate it, so compatibility has not been verified; moves within the same table remain available"));
    }
    let source_path = property(source_table, "DataPath");
    if source_path.is_some() && source_path == property(target_table, "DataPath") {
        return Ok(());
    }
    // A single empty group is independent of table data. Nested groups are
    // real runtime children even when empty, whereas unbound fields are not.
    let runtime_children = source.descendants().skip(1).any(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(NS)
            && node
                .parent_element()
                .is_some_and(|p| p.has_tag_name((NS, "ChildItems")))
            && !(is_field(node.tag_name().name()) && property(node, "DataPath").is_none())
    });
    if source.has_tag_name((NS, "ColumnGroup")) && !runtime_children {
        return Ok(());
    }
    Err(error("into", "moving these columns between tables with different DataPath would change or invalidate their bindings"))
}

#[cfg(test)]
#[path = "form_move_tests.rs"]
mod tests;
