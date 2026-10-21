//! Local XML changes for the spreadsheet operations `mxl.set` and `area.add`.
//!
//! The writer changes only the addressed nodes of `Template.xml`: the text of
//! a cell, a new cell slot or row, the split of a compressed empty row range,
//! a new named area, the column count, the height and, for a converted
//! parameter or template cell, its format. Every other byte, the byte order
//! mark and the line endings stay as they were; no intermediate model of the
//! whole document is built (#1448, as DCS in #1310).
//!
//! New nodes take the form a platform 8.3.27 export writes: tab indentation by
//! depth, rows contiguous from zero, cell-less runs compressed with
//! `indexTo`, `<i>` only before a column that does not follow the previous
//! one, `<f>0</f>` for a cell without its own format, `<tl/>` for empty text
//! and named areas ordered by name ignoring case.
use crate::mxl_root::MXL_DOCUMENT_NS as NS;
use roxmltree::{Document, Node};
use std::ops::Range;

const CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Root children in the order of a platform export; insertion keeps it.
const ROOT_ORDER: &[&str] = &[
    "languageSettings",
    "columns",
    "rowsItem",
    "drawing",
    "leftHeader",
    "centerHeader",
    "rightHeader",
    "leftFooter",
    "centerFooter",
    "rightFooter",
    "templateMode",
    "defaultFormatIndex",
    "height",
    "vgLevels",
    "vgRows",
    "vg",
    "merge",
    "verticalUnmerge",
    "horizontalUnmerge",
    "namedItem",
    "printSettings",
    "printArea",
    "groupsBackColor",
    "groupsColor",
    "headersBackColor",
    "headersColor",
    "line",
    "font",
    "format",
    "picture",
];

/// Cell children the writer keeps or replaces; anything else in an addressed
/// cell (a value, a control, a picture parameter) is not text it may replace.
const CELL_CHILDREN: &[&str] = &["f", "tl", "parameter", "detailParameter", "note", "d"];

/// One written cell: its `RnCn` key as given, 1-based row and column inside
/// the area, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellText {
    pub key: String,
    pub row: i64,
    pub col: i64,
    pub text: String,
}

/// Why the spreadsheet was not changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MxlEditError {
    /// `mxl.set` names an area the template does not have.
    AreaMissing,
    /// `area.add` names an area the template already has.
    AreaExists(String),
    /// `mxl.set` names an area the template spells in another case.
    AreaSpelling(String),
    /// A written row lies below the area and the area cannot grow.
    RowOutsideArea { key: String, rows: i64 },
    /// The addressed cell cannot take the value: its text cannot be stored
    /// in XML, or the cell lies under a merge, holds a value or control, or
    /// takes a parameter or template fill from its row, column, column-set or document format.
    Cell { key: String, message: String },
    /// The document or the addressed node is not what the writer can change.
    Source(String),
}

fn source(message: impl Into<String>) -> MxlEditError {
    MxlEditError::Source(message.into())
}

/// A refusal of the cell being written; the caller names its key.
fn cell_refusal(message: impl Into<String>) -> MxlEditError {
    MxlEditError::Cell {
        key: String::new(),
        message: message.into(),
    }
}

/// Writes one cell and names it in a cell refusal.
fn write_named_cell(xml: &mut String, begin: i64, cell: &CellText) -> Result<(), MxlEditError> {
    write_cell(xml, begin + cell.row - 1, cell.col - 1, &cell.text).map_err(|error| match error {
        MxlEditError::Cell { message, .. } => MxlEditError::Cell {
            key: cell.key.clone(),
            message,
        },
        other => other,
    })
}

/// Writes the cells of the existing named Rows area `area`.
pub fn set_area_cells(
    xml: &mut String,
    area: &str,
    columns: Option<i64>,
    cells: &[CellText],
) -> Result<(), MxlEditError> {
    check_texts(cells)?;
    let (begin, end, sets, last) = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let named = named_items(root)
            .into_iter()
            .find(|item| text_of(*item, "name").trim() == area)
            .ok_or_else(|| {
                let folded = area.to_lowercase();
                match named_items(root)
                    .into_iter()
                    .map(|item| text_of(item, "name").trim().to_string())
                    .find(|name| name.to_lowercase() == folded)
                {
                    Some(name) => MxlEditError::AreaSpelling(name),
                    None => MxlEditError::AreaMissing,
                }
            })?;
        let bounds = child(named, "area").ok_or_else(|| source("the named area has no bounds"))?;
        let kind = text_of(bounds, "type");
        if kind.trim() != "Rows" {
            return Err(source(format!(
                "area `{area}` is a `{}` area; the cell editor writes Rows areas, edit this one in the Designer",
                kind.trim()
            )));
        }
        let begin = int_of(bounds, "beginRow").ok_or_else(|| source("the area has no beginRow"))?;
        let end = int_of(bounds, "endRow").ok_or_else(|| source("the area has no endRow"))?;
        if begin < 0 || end < begin {
            // The platform keeps an emptied area as `endRow < beginRow`.
            return Err(source(format!(
                "area `{area}` has no rows; the cell editor does not insert rows into an area, edit it in the Designer"
            )));
        }
        let mut sets = Vec::new();
        for row in begin..=end {
            let set = row_item(root, row)
                .and_then(|item| child(item, "row"))
                .and_then(|row| child(row, "columnsID"))
                .map(|id| id.text().unwrap_or("").trim().to_string());
            if !sets.contains(&set) {
                sets.push(set);
            }
        }
        // Nothing lies below the last area: it may grow without moving rows.
        let last = first_free_row(root)? == end + 1;
        (begin, end, sets, last)
    };
    let rows = end - begin + 1;
    let needed = cells.iter().map(|cell| cell.row).max().unwrap_or(0);
    if needed > rows {
        if !last {
            let key = cells
                .iter()
                .find(|cell| cell.row > rows)
                .map(|cell| cell.key.clone())
                .unwrap_or_default();
            return Err(MxlEditError::RowOutsideArea { key, rows });
        }
        grow_area(xml, area, begin + needed - 1)?;
    }
    for cell in cells {
        write_named_cell(xml, begin, cell)?;
    }
    if let Some(columns) = columns {
        for set in &sets {
            grow_columns(xml, set.as_deref(), columns)?;
        }
    }
    Ok(())
}

/// Appends the new named Rows area `area` after the used rows and writes its
/// initial cells.
pub fn add_area(
    xml: &mut String,
    area: &str,
    columns: Option<i64>,
    cells: &[CellText],
) -> Result<(), MxlEditError> {
    check_texts(cells)?;
    let rows = cells.iter().map(|cell| cell.row).max().unwrap_or(1);
    let begin = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let folded = area.to_lowercase();
        if let Some(existing) = named_items(root)
            .into_iter()
            .map(|item| text_of(item, "name").trim().to_string())
            .find(|name| name.to_lowercase() == folded)
        {
            return Err(MxlEditError::AreaExists(existing));
        }
        first_free_row(root)?
    };
    let end = begin + rows - 1;
    reserve_rows(xml, end)?;
    insert_named_area(xml, area, begin, end)?;
    raise_height(xml, end + 1)?;
    for cell in cells {
        write_named_cell(xml, begin, cell)?;
    }
    if let Some(columns) = columns {
        grow_columns(xml, None, columns)?;
    }
    Ok(())
}

/// Text XML 1.0 can hold: no control characters other than tab and breaks.
fn check_texts(cells: &[CellText]) -> Result<(), MxlEditError> {
    for cell in cells {
        if let Some(ch) = cell.text.chars().find(|ch| {
            (*ch < ' ' && !matches!(ch, '\t' | '\n' | '\r'))
                || matches!(ch, '\u{FFFE}' | '\u{FFFF}')
        }) {
            return Err(MxlEditError::Cell {
                key: cell.key.clone(),
                message: format!(
                    "the text holds the control character U+{:04X}, which XML cannot store",
                    ch as u32
                ),
            });
        }
    }
    Ok(())
}

/// Moves the end of the last area `area` down to `end`.
fn grow_area(xml: &mut String, area: &str, end: i64) -> Result<(), MxlEditError> {
    reserve_rows(xml, end)?;
    let edit = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let named = named_items(root)
            .into_iter()
            .find(|item| text_of(*item, "name").trim() == area)
            .ok_or(MxlEditError::AreaMissing)?;
        let bounds = child(named, "area").ok_or_else(|| source("the named area has no bounds"))?;
        let text = child(bounds, "endRow")
            .and_then(|node| node.first_child())
            .ok_or_else(|| source("the area has no endRow"))?;
        (text.range(), end.to_string())
    };
    xml.replace_range(edit.0, &edit.1);
    raise_height(xml, end + 1)
}

fn parse(xml: &str) -> Result<Document<'_>, MxlEditError> {
    Document::parse(xml).map_err(|error| {
        source(format!(
            "the spreadsheet template is not well-formed XML: {error}"
        ))
    })
}

fn document_root<'a, 'i>(document: &'a Document<'i>) -> Result<Node<'a, 'i>, MxlEditError> {
    let root = document.root_element();
    crate::mxl_root::require_mxl_document_root(root).map_err(source)?;
    // New elements are written without a prefix, the way the platform does.
    if root
        .lookup_prefix(NS)
        .is_some_and(|prefix| !prefix.is_empty())
        || root.lookup_namespace_uri(None) != Some(NS)
    {
        return Err(source(
            "the spreadsheet namespace is not the default namespace of the document",
        ));
    }
    Ok(root)
}

fn is(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(NS)
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|item| is(*item, name))
}

fn children<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Vec<Node<'a, 'i>> {
    node.children().filter(|item| is(*item, name)).collect()
}

fn text_of(node: Node<'_, '_>, name: &str) -> String {
    child(node, name)
        .and_then(|item| item.text())
        .unwrap_or("")
        .to_string()
}

fn int_of(node: Node<'_, '_>, name: &str) -> Option<i64> {
    child(node, name).and_then(|item| item.text()?.trim().parse().ok())
}

fn named_items<'a, 'i>(root: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    children(root, "namedItem")
}

/// The first and last row a `rowsItem` describes.
fn row_span(item: Node<'_, '_>) -> Option<(i64, i64)> {
    let first = int_of(item, "index")?;
    let last = match child(item, "indexTo") {
        Some(_) => int_of(item, "indexTo")?,
        None => first,
    };
    Some((first, last))
}

fn row_item<'a, 'i>(root: Node<'a, 'i>, row: i64) -> Option<Node<'a, 'i>> {
    children(root, "rowsItem")
        .into_iter()
        .find(|item| row_span(*item).is_some_and(|(first, last)| first <= row && row <= last))
}

/// A row with nothing but `<empty>true</empty>`: the same as no row at all.
fn is_bare_empty_row(item: Node<'_, '_>) -> bool {
    child(item, "row").is_some_and(|row| {
        let parts: Vec<_> = row.children().filter(Node::is_element).collect();
        matches!(parts.as_slice(), [only] if is(*only, "empty"))
    })
}

/// Line break of the document: the one before `position` when the text there
/// has one, else the first one of the document.
fn line_break(xml: &str, position: usize) -> &'static str {
    let before = &xml[..position];
    let local = before
        .rfind('\n')
        .filter(|index| before[index + 1..].trim().is_empty());
    let crlf = match local {
        Some(index) => index > 0 && before.as_bytes()[index - 1] == b'\r',
        None => xml
            .find('\n')
            .is_some_and(|index| index > 0 && xml.as_bytes()[index - 1] == b'\r'),
    };
    if crlf {
        "\r\n"
    } else {
        "\n"
    }
}

fn tabs(depth: usize) -> String {
    "\t".repeat(depth)
}

/// Text content escaped as the platform writes it: quotes and tabs stay raw,
/// a CR LF or CR line break is stored as LF.
fn escape_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\r', "\n")
}

/// Range of the whitespace text node right before `node`, if any.
fn leading_space(node: Node<'_, '_>) -> Range<usize> {
    match node.prev_sibling() {
        Some(previous)
            if previous.is_text() && previous.text().unwrap_or("x").trim().is_empty() =>
        {
            previous.range()
        }
        _ => node.range().start..node.range().start,
    }
}

/// Inserts `fragment` (one root child at depth 1) among the root children in
/// platform order.
fn insert_root_child(xml: &mut String, name: &str, fragment: &str) -> Result<(), MxlEditError> {
    let (position, before) = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let rank = ROOT_ORDER
            .iter()
            .position(|item| *item == name)
            .expect("known root child");
        let elements: Vec<_> = root.children().filter(Node::is_element).collect();
        let later = elements.iter().find(|node| {
            node.tag_name().namespace() == Some(NS)
                && ROOT_ORDER
                    .iter()
                    .position(|item| *item == node.tag_name().name())
                    .is_some_and(|other| other > rank)
        });
        match (
            later,
            elements
                .iter()
                .rev()
                .find(|node| Some(**node) != later.copied()),
        ) {
            (Some(next), _) => (next.range().start, true),
            (None, Some(last)) => (last.range().end, false),
            (None, None) => {
                return Err(source("the spreadsheet document has no content to extend"))
            }
        }
    };
    let eol = line_break(xml, position);
    let text = if before {
        format!("{fragment}{eol}\t")
    } else {
        format!("{eol}\t{fragment}")
    };
    xml.insert_str(position, &text);
    Ok(())
}

/// The row the new area starts at: after the height and every row something
/// refers to — a named area, a drawing, a merge, a row with content; trailing
/// bare empty rows are free.
fn first_free_row(root: Node<'_, '_>) -> Result<i64, MxlEditError> {
    let mut begin = int_of(root, "height").unwrap_or(0).max(0);
    for node in root.descendants().filter(|node| is(*node, "endRow")) {
        if let Some(end) = node.text().and_then(|text| text.trim().parse::<i64>().ok()) {
            begin = begin.max(end + 1);
        }
    }
    for kind in ["merge", "verticalUnmerge", "horizontalUnmerge"] {
        for node in children(root, kind) {
            if let Some(row) = int_of(node, "r") {
                begin = begin.max(row + int_of(node, "h").unwrap_or(0) + 1);
            }
        }
    }
    for item in children(root, "rowsItem") {
        let (_, last) = row_span(item).ok_or_else(|| source("a row has no valid index"))?;
        if !is_bare_empty_row(item) {
            begin = begin.max(last + 1);
        }
    }
    Ok(begin)
}

/// Makes rows exist up to `last`: the platform writes every row from zero, so
/// rows past the end extend a trailing bare empty range or become a new one.
fn reserve_rows(xml: &mut String, last: i64) -> Result<(), MxlEditError> {
    let edit = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let items = children(root, "rowsItem");
        let Some(tail) = items.last() else {
            let eol = line_break(xml, xml.len());
            let fragment = rows_item_text(0, last, eol);
            drop(document);
            insert_root_child(xml, "rowsItem", &fragment)?;
            return Ok(());
        };
        let (_, end) = row_span(*tail).ok_or_else(|| source("a row has no valid index"))?;
        if end >= last {
            return Ok(());
        }
        if is_bare_empty_row(*tail) {
            match child(*tail, "indexTo") {
                Some(index_to) => {
                    let text = index_to
                        .first_child()
                        .ok_or_else(|| source("indexTo is empty"))?;
                    (text.range(), last.to_string())
                }
                None => {
                    let index =
                        child(*tail, "index").ok_or_else(|| source("a row has no index"))?;
                    let eol = line_break(xml, index.range().start);
                    let indent = &xml[leading_space(index)]
                        .rsplit('\n')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    let at = index.range().end;
                    (at..at, format!("{eol}{indent}<indexTo>{last}</indexTo>"))
                }
            }
        } else {
            let at = tail.range().end;
            let eol = line_break(xml, tail.range().start);
            (
                at..at,
                format!("{eol}\t{}", rows_item_text(end + 1, last, eol)),
            )
        }
    };
    xml.replace_range(edit.0, &edit.1);
    Ok(())
}

/// A bare empty `rowsItem` for rows `first..=last` in platform form.
fn rows_item_text(first: i64, last: i64, eol: &str) -> String {
    let index_to = if last > first {
        format!("{eol}\t\t<indexTo>{last}</indexTo>")
    } else {
        String::new()
    };
    format!(
        "<rowsItem>{eol}\t\t<index>{first}</index>{index_to}{eol}\t\t<row>{eol}\t\t\t<empty>true</empty>{eol}\t\t</row>{eol}\t</rowsItem>"
    )
}

/// Leaves `row` in a `rowsItem` of its own: a compressed range is split into
/// the rows before, the row and the rows after, each a copy of the original.
fn isolate_row(xml: &mut String, row: i64) -> Result<(), MxlEditError> {
    reserve_rows(xml, row)?;
    let edit = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let Some(item) = row_item(root, row) else {
            // A gap inside the rows of a hand-made document: a single new
            // row in index order.
            let next = children(root, "rowsItem")
                .into_iter()
                .find(|item| row_span(*item).is_some_and(|(first, _)| first > row))
                .ok_or_else(|| source("rows are not ordered by index"))?;
            let at = next.range().start;
            let eol = line_break(xml, at);
            let text = format!("{}{eol}\t", rows_item_text(row, row, eol));
            drop(document);
            xml.insert_str(at, &text);
            return Ok(());
        };
        let (first, last) = row_span(item).ok_or_else(|| source("a row has no valid index"))?;
        if first == last {
            return Ok(());
        }
        if children(root, "rowsItem").into_iter().any(|other| {
            other != item && row_span(other).is_some_and(|(from, to)| from <= last && first <= to)
        }) {
            return Err(source(format!(
                "rows {first}..{last} overlap other rows of the document; edit this template in the Designer"
            )));
        }
        let range = item.range();
        let base = range.start;
        let original = &xml[range.clone()];
        let index = child(item, "index").ok_or_else(|| source("a row has no index"))?;
        let index_text = index
            .first_child()
            .ok_or_else(|| source("index is empty"))?
            .range();
        let index_to = child(item, "indexTo").ok_or_else(|| source("a row has no indexTo"))?;
        let to_text = index_to
            .first_child()
            .ok_or_else(|| source("indexTo is empty"))?
            .range();
        let to_whole = leading_space(index_to).start..index_to.range().end;
        let piece = |from: i64, to: i64| -> String {
            let local = |range: &Range<usize>| range.start - base..range.end - base;
            let (index_text, to_text, to_whole) =
                (local(&index_text), local(&to_text), local(&to_whole));
            let mut text = String::new();
            text.push_str(&original[..index_text.start]);
            text.push_str(&from.to_string());
            if to > from {
                text.push_str(&original[index_text.end..to_text.start]);
                text.push_str(&to.to_string());
                text.push_str(&original[to_text.end..]);
            } else {
                text.push_str(&original[index_text.end..to_whole.start]);
                text.push_str(&original[to_whole.end..]);
            }
            text
        };
        let separator = &xml[leading_space(item)];
        let separator = if separator.is_empty() {
            String::new()
        } else {
            separator.to_string()
        };
        let mut pieces = Vec::new();
        if row > first {
            pieces.push(piece(first, row - 1));
        }
        pieces.push(piece(row, row));
        if row < last {
            pieces.push(piece(row + 1, last));
        }
        (range, pieces.join(&separator))
    };
    xml.replace_range(edit.0, &edit.1);
    Ok(())
}

/// One cell slot of a row: its 0-based column and the slot element.
fn slots<'a, 'i>(row: Node<'a, 'i>) -> Result<Vec<(i64, Node<'a, 'i>)>, MxlEditError> {
    let mut column = -1;
    let mut result = Vec::new();
    for slot in children(row, "c") {
        column = match child(slot, "i") {
            Some(_) => int_of(slot, "i").ok_or_else(|| source("a cell has an invalid column"))?,
            None => column + 1,
        };
        result.push((column, slot));
    }
    Ok(result)
}

fn default_language(root: Node<'_, '_>) -> String {
    child(root, "languageSettings")
        .map(|settings| text_of(settings, "defaultLanguage").trim().to_string())
        .filter(|language| !language.is_empty())
        .unwrap_or_else(|| "ru".to_string())
}

fn core_prefix(node: Node<'_, '_>) -> Result<String, MxlEditError> {
    match node.lookup_prefix(CORE_NS) {
        Some(prefix) if !prefix.is_empty() => Ok(prefix.to_string()),
        _ => Err(source(
            "the spreadsheet document does not declare a prefix for the core namespace",
        )),
    }
}

/// `<tl>` of `text` at `depth`; empty text is `<tl/>` as the platform writes.
fn text_element(text: &str, lang: &str, prefix: &str, depth: usize, eol: &str) -> String {
    if text.is_empty() {
        return "<tl/>".to_string();
    }
    let (item, part) = (tabs(depth + 1), tabs(depth + 2));
    format!(
        "<tl>{eol}{item}<{prefix}:item>{eol}{part}<{prefix}:lang>{lang}</{prefix}:lang>{eol}{part}<{prefix}:content>{}</{prefix}:content>{eol}{item}</{prefix}:item>{eol}{}</tl>",
        escape_text(text),
        tabs(depth)
    )
}

/// Writes `text` into column `column` (0-based) of row `row` (0-based).
fn write_cell(xml: &mut String, row: i64, column: i64, text: &str) -> Result<(), MxlEditError> {
    // The platform stores a CR LF or CR break inside text as LF.
    let text = &text.replace("\r\n", "\n").replace('\r', "\n");
    {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        refuse_covered_cell(root, row, column)?;
        // Empty text in a cell that does not exist writes no cell; the
        // written column still counts for the width.
        let row_node = row_item(root, row).and_then(|item| child(item, "row"));
        let slot = row_node
            .map(slots)
            .transpose()?
            .and_then(|slots| slots.into_iter().find(|(index, _)| *index == column))
            .map(|(_, slot)| slot);
        // A cell that already holds the text is left alone, even inside a
        // range of rows, and so is a missing cell given empty text.
        let unchanged = match slot {
            Some(slot) => cell_text_edit(
                xml,
                slot,
                text,
                &default_language(root),
                &core_prefix(root)?,
            )?
            .is_none(),
            None => text.is_empty(),
        };
        if unchanged {
            let set = row_node
                .and_then(|row| child(row, "columnsID"))
                .map(|id| id.text().unwrap_or("").trim().to_string());
            drop(document);
            return grow_columns(xml, set.as_deref(), column + 1);
        }
    }
    isolate_row(xml, row)?;
    let (edit, set) = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let item = row_item(root, row).ok_or_else(|| source("the row was not created"))?;
        let row_node = child(item, "row").ok_or_else(|| source("a row item has no row"))?;
        let set = child(row_node, "columnsID").map(|id| id.text().unwrap_or("").trim().to_string());
        let lang = default_language(root);
        let prefix = core_prefix(root)?;
        let slots = slots(row_node)?;
        let own = slots
            .iter()
            .find(|(index, _)| *index == column)
            .and_then(|(_, slot)| child(*slot, "c"));
        if !own.is_some_and(has_own_fill) {
            if let Some(fill) = inherited_fill(root, row_node, set.as_deref(), column) {
                return Err(cell_refusal(format!(
                    "the cell takes the `{fill}` fill of its row, column, column-set or document format; edit it in the Designer"
                )));
            }
        }
        let edit = match slots.iter().find(|(index, _)| *index == column) {
            Some((_, slot)) => cell_text_edit(xml, *slot, text, &lang, &prefix)?,
            None => Some(new_slot_edit(
                xml, row_node, &slots, column, text, &lang, &prefix,
            )?),
        };
        (edit, set)
    };
    let converted = match edit {
        Some(CellEdit {
            replacements,
            convert,
        }) => {
            apply(xml, replacements);
            convert
        }
        None => false,
    };
    if converted {
        convert_cell_format(xml, row, column)?;
    }
    grow_columns(xml, set.as_deref(), column + 1)
}

struct CellEdit {
    replacements: Vec<(Range<usize>, String)>,
    /// The cell held a parameter or template: its format may need a change.
    convert: bool,
}

fn apply(xml: &mut String, mut replacements: Vec<(Range<usize>, String)>) {
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, text) in replacements {
        xml.replace_range(range, &text);
    }
}

fn new_slot_edit(
    xml: &str,
    row: Node<'_, '_>,
    slots: &[(i64, Node<'_, '_>)],
    column: i64,
    text: &str,
    lang: &str,
    prefix: &str,
) -> Result<CellEdit, MxlEditError> {
    let previous = slots
        .iter()
        .rev()
        .find(|(index, _)| *index < column)
        .map_or(-1, |(index, _)| *index);
    let next = slots.iter().find(|(index, _)| *index > column);
    let anchor = next
        .map(|(_, slot)| slot.range().start)
        .or_else(|| child(row, "empty").map(|empty| empty.range().start))
        .or_else(|| slots.last().map(|(_, slot)| slot.range().end))
        .or_else(|| {
            row.children()
                .rev()
                .find(Node::is_element)
                .map(|node| node.range().end)
        })
        .ok_or_else(|| source("a row has no content to extend"))?;
    let eol = line_break(xml, anchor);
    let index = if column != previous + 1 {
        format!("{eol}{}<i>{column}</i>", tabs(4))
    } else {
        String::new()
    };
    let slot = format!(
        "<c>{index}{eol}{c}<c>{eol}{f}<f>0</f>{eol}{f}{tl}{eol}{c}</c>{eol}{s}</c>",
        c = tabs(4),
        f = tabs(5),
        s = tabs(3),
        tl = text_element(text, lang, prefix, 5, eol),
    );
    let mut replacements = Vec::new();
    if let Some(empty) = child(row, "empty") {
        replacements.push((empty.range(), slot));
    } else if let Some((next_column, next_slot)) = next {
        replacements.push((anchor..anchor, format!("{slot}{eol}{}", tabs(3))));
        // The next slot loses its `<i>` when it now follows the new column.
        if *next_column == column + 1 {
            if let Some(index) = child(*next_slot, "i") {
                replacements.push((leading_space(index).start..index.range().end, String::new()));
            }
        }
    } else {
        replacements.push((anchor..anchor, format!("{eol}{}{slot}", tabs(3))));
    }
    Ok(CellEdit {
        replacements,
        convert: false,
    })
}

/// The change of an existing cell to `text`, or `None` when it already holds it.
fn cell_text_edit(
    xml: &str,
    slot: Node<'_, '_>,
    text: &str,
    lang: &str,
    prefix: &str,
) -> Result<Option<CellEdit>, MxlEditError> {
    let cell = child(slot, "c").ok_or_else(|| source("a cell slot has no cell"))?;
    if let Some(other) = cell.children().find(|node| {
        node.is_element()
            && !(node.tag_name().namespace() == Some(NS)
                && CELL_CHILDREN.contains(&node.tag_name().name()))
    }) {
        return Err(cell_refusal(format!(
            "the cell holds `{}` content the cell editor cannot replace with text; edit it in the Designer",
            other.tag_name().name()
        )));
    }
    let parameter = child(cell, "parameter");
    let convert = parameter.is_some() || format_fill_is_not_text(cell);
    let tl = child(cell, "tl");
    let depth = 5;
    let eol_at = tl
        .or(parameter)
        .or_else(|| child(cell, "f"))
        .map_or(cell.range().start, |node| node.range().start);
    let eol = line_break(xml, eol_at);
    let mut replacements = Vec::new();
    match (tl, parameter) {
        (Some(tl), _) => {
            if let Some(change) = tl_change(xml, tl, text, lang, prefix, depth, eol) {
                replacements.push(change);
            }
            if let Some(parameter) = parameter {
                replacements.push((
                    leading_space(parameter).start..parameter.range().end,
                    String::new(),
                ));
            }
        }
        (None, Some(parameter)) => replacements.push((
            parameter.range(),
            text_element(text, lang, prefix, depth, eol),
        )),
        // A cell without text already holds empty text.
        (None, None) if text.is_empty() => {}
        (None, None) => {
            let fresh = text_element(text, lang, prefix, depth, eol);
            match child(cell, "f") {
                Some(format) => {
                    let at = format.range().end;
                    replacements.push((at..at, format!("{eol}{}{fresh}", tabs(depth))));
                }
                None => {
                    let at = start_content(xml, cell)?;
                    replacements.push((at..at, format!("{eol}{}{fresh}", tabs(depth))));
                }
            }
        }
    }
    if replacements.is_empty() && !convert {
        return Ok(None);
    }
    Ok(Some(CellEdit {
        replacements,
        convert,
    }))
}

/// Offset right after the start tag of `node`.
fn start_content(xml: &str, node: Node<'_, '_>) -> Result<usize, MxlEditError> {
    let range = node.range();
    let mut quote = None;
    for (offset, ch) in xml[range.clone()].char_indices() {
        match (quote, ch) {
            (Some(current), ch) if current == ch => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => {
                if xml[..range.start + offset].ends_with('/') {
                    return Err(source("an empty cell element cannot be extended"));
                }
                return Ok(range.start + offset + 1);
            }
            _ => {}
        }
    }
    Err(source("malformed XML element"))
}

/// The change of `<tl>` to `text`: only the default-language item changes and
/// other translations stay.
fn tl_change(
    xml: &str,
    tl: Node<'_, '_>,
    text: &str,
    lang: &str,
    prefix: &str,
    depth: usize,
    eol: &str,
) -> Option<(Range<usize>, String)> {
    let items: Vec<_> = tl
        .children()
        .filter(|node| node.is_element() && node.has_tag_name((CORE_NS, "item")))
        .collect();
    fn part<'a, 'i>(item: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
        item.children()
            .find(|node| node.is_element() && node.has_tag_name((CORE_NS, name)))
    }
    let own = items.iter().copied().find(|item| {
        part(*item, "lang")
            .and_then(|node| node.text())
            .map(str::trim)
            == Some(lang)
    });
    let others = items.len() - usize::from(own.is_some());
    let current = own
        .and_then(|item| part(item, "content"))
        .and_then(|node| node.text())
        .unwrap_or("");
    if own.is_some() && current == text || own.is_none() && text.is_empty() {
        return None;
    }
    // Empty text without other translations, or the first text of an empty
    // `<tl>`: the whole element in platform form.
    if others == 0 && (text.is_empty() || own.is_none()) {
        return Some((tl.range(), text_element(text, lang, prefix, depth, eol)));
    }
    match (own, text.is_empty()) {
        (Some(item), true) => Some((leading_space(item).start..item.range().end, String::new())),
        (Some(item), false) => {
            let content = part(item, "content")?;
            let name = &xml[content.range()]
                .trim_start_matches('<')
                .split(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '>'))
                .next()?
                .to_string();
            Some((
                content.range(),
                format!("<{name}>{}</{name}>", escape_text(text)),
            ))
        }
        (None, false) => {
            let last = items.last()?;
            let indent = tabs(depth + 1);
            let part_indent = tabs(depth + 2);
            let at = last.range().end;
            Some((
                at..at,
                format!(
                    "{eol}{indent}<{prefix}:item>{eol}{part_indent}<{prefix}:lang>{lang}</{prefix}:lang>{eol}{part_indent}<{prefix}:content>{}</{prefix}:content>{eol}{indent}</{prefix}:item>",
                    escape_text(text)
                ),
            ))
        }
        (None, true) => None,
    }
}

fn format_nodes<'a, 'i>(root: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    children(root, "format")
}

/// The format with 1-based `index`, if the document has it.
fn format_at<'a, 'i>(root: Node<'a, 'i>, index: i64) -> Option<Node<'a, 'i>> {
    usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| format_nodes(root).get(index).copied())
}

fn fill_of(format: Node<'_, '_>) -> Option<String> {
    child(format, "fillType").map(|fill| fill.text().unwrap_or("").trim().to_string())
}

/// Whether the cell's own format sets a fill type.
fn has_own_fill(cell: Node<'_, '_>) -> bool {
    let Some(root) = cell.ancestors().find(|node| is(*node, "document")) else {
        return false;
    };
    int_of(cell, "f")
        .and_then(|index| format_at(root, index))
        .and_then(fill_of)
        .is_some()
}

/// A parameter or template fill the cell would take from the format of its
/// row or column.
fn inherited_fill(
    root: Node<'_, '_>,
    row: Node<'_, '_>,
    set: Option<&str>,
    column: i64,
) -> Option<String> {
    let row_format = int_of(row, "formatIndex");
    let columns = children(root, "columns").into_iter().find(|columns| {
        child(*columns, "id")
            .map(|id| id.text().unwrap_or("").trim().to_string())
            .as_deref()
            == set
    });
    let column_format = columns
        .and_then(|columns| {
            children(columns, "columnsItem")
                .into_iter()
                .find(|item| int_of(*item, "index") == Some(column))
        })
        .and_then(|item| child(item, "column"))
        .and_then(|column| int_of(column, "formatIndex"));
    let set_format = columns.and_then(|columns| int_of(columns, "formatIndex"));
    let document_format = int_of(root, "defaultFormatIndex");
    [row_format, column_format, set_format, document_format]
        .into_iter()
        .flatten()
        .filter_map(|index| format_at(root, index).and_then(fill_of))
        .find(|fill| fill != "Text")
}

/// Refuses a cell that a merge covers but does not start: its text would not
/// be shown.
fn refuse_covered_cell(root: Node<'_, '_>, row: i64, column: i64) -> Result<(), MxlEditError> {
    for merge in children(root, "merge") {
        let (Some(top), Some(left)) = (int_of(merge, "r"), int_of(merge, "c")) else {
            continue;
        };
        let bottom = top + int_of(merge, "h").unwrap_or(0);
        let right = left + int_of(merge, "w").unwrap_or(0);
        if (top..=bottom).contains(&row)
            && (left..=right).contains(&column)
            && (row, column) != (top, left)
        {
            return Err(cell_refusal(format!(
                "the cell lies inside a merged cell that starts at document row {} column {}; write that cell instead",
                top + 1,
                left + 1
            )));
        }
    }
    Ok(())
}

/// Whether the own format of `cell` fills it from a parameter or a template.
fn format_fill_is_not_text(cell: Node<'_, '_>) -> bool {
    let Some(index) = int_of(cell, "f") else {
        return false;
    };
    let Some(root) = cell.ancestors().find(|node| is(*node, "document")) else {
        return false;
    };
    usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| format_nodes(root).get(index).copied())
        .and_then(|format| child(format, "fillType"))
        .is_some_and(|fill| fill.text().map(str::trim) != Some("Text"))
}

/// Gives the converted cell a format without a parameter or template fill:
/// the format itself when only this cell uses it, else an identical existing
/// one, else a new one appended after the last.
fn convert_cell_format(xml: &mut String, row: i64, column: i64) -> Result<(), MxlEditError> {
    let replacements = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let item = row_item(root, row).ok_or_else(|| source("the row disappeared"))?;
        let row_node = child(item, "row").ok_or_else(|| source("a row item has no row"))?;
        let slot = slots(row_node)?
            .into_iter()
            .find(|(index, _)| *index == column)
            .map(|(_, slot)| slot)
            .ok_or_else(|| source("the cell disappeared"))?;
        let cell = child(slot, "c").ok_or_else(|| source("a cell slot has no cell"))?;
        let Some(f) = child(cell, "f") else {
            return Ok(());
        };
        let Some(index) = int_of(cell, "f") else {
            return Err(source("a cell has an invalid format index"));
        };
        let formats = format_nodes(root);
        let Some(format) = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| formats.get(index).copied())
        else {
            return Ok(());
        };
        let Some(fill) = child(format, "fillType") else {
            return Ok(());
        };
        if fill.text().map(str::trim) == Some("Text") {
            return Ok(());
        }
        let fill_whole = leading_space(fill).start..fill.range().end;
        let users = root
            .descendants()
            .filter(|node| {
                (is(*node, "f") || is(*node, "formatIndex") || is(*node, "defaultFormatIndex"))
                    && node.text().map(str::trim) == Some(index.to_string().as_str())
            })
            .count();
        let alone = format.children().filter(Node::is_element).count() == 1;
        if users == 1 && alone {
            // An empty format is written as `<format/>` by the platform.
            vec![(format.range(), "<format/>".to_string())]
        } else if users == 1 {
            vec![(fill_whole, String::new())]
        } else {
            let format_range = format.range();
            let local = fill_whole.start - format_range.start..fill_whole.end - format_range.start;
            let original = &xml[format_range.clone()];
            let target = format!("{}{}", &original[..local.start], &original[local.end..]);
            // The copy keeps the prefixes the document root declares.
            let declarations: String = root
                .namespaces()
                .map(|namespace| match namespace.name() {
                    Some(prefix) => format!(
                        " xmlns:{prefix}=\"{}\"",
                        escape_text(namespace.uri()).replace('"', "&quot;")
                    ),
                    None => format!(
                        " xmlns=\"{}\"",
                        escape_text(namespace.uri()).replace('"', "&quot;")
                    ),
                })
                .collect();
            let wrapped = format!("<wrapper{declarations}>{target}</wrapper>");
            let target_document = Document::parse(&wrapped)
                .map_err(|error| source(format!("a format is not well-formed: {error}")))?;
            let target_node = target_document
                .root_element()
                .first_element_child()
                .ok_or_else(|| source("a format copy has no element"))?;
            let reuse = formats
                .iter()
                .position(|candidate| same_element(*candidate, target_node));
            let f_text = f
                .first_child()
                .ok_or_else(|| source("a cell format is empty"))?;
            match reuse {
                Some(position) => vec![(f_text.range(), (position + 1).to_string())],
                None => {
                    let last = formats.last().expect("the converted format exists");
                    let at = last.range().end;
                    let eol = line_break(xml, last.range().start);
                    vec![
                        (at..at, format!("{eol}\t{target}")),
                        (f_text.range(), (formats.len() + 1).to_string()),
                    ]
                }
            }
        }
    };
    apply(xml, replacements);
    Ok(())
}

/// Structural equality of two formats: the same child elements with the
/// same attributes and text, whitespace between elements ignored.
fn same_element(left: Node<'_, '_>, right: Node<'_, '_>) -> bool {
    if left.tag_name().name() != right.tag_name().name()
        || left.tag_name().namespace() != right.tag_name().namespace()
    {
        return false;
    }
    let attributes = |node: Node<'_, '_>| {
        let mut list: Vec<_> = node
            .attributes()
            .map(|attribute| {
                (
                    attribute.namespace().map(str::to_string),
                    attribute.name().to_string(),
                    attribute.value().to_string(),
                )
            })
            .collect();
        list.sort();
        list
    };
    if attributes(left) != attributes(right) {
        return false;
    }
    fn elements<'a, 'i>(node: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
        node.children().filter(Node::is_element).collect()
    }
    let (left_elements, right_elements) = (elements(left), elements(right));
    if left_elements.is_empty() && right_elements.is_empty() {
        return left.text().unwrap_or("") == right.text().unwrap_or("");
    }
    left_elements.len() == right_elements.len()
        && left_elements
            .iter()
            .zip(&right_elements)
            .all(|(left, right)| same_element(*left, *right))
}

/// Raises the `size` of a column set to at least `count`; the set is the
/// default one (`None`) or the one with `id`.
fn grow_columns(xml: &mut String, set: Option<&str>, count: i64) -> Result<(), MxlEditError> {
    let edit = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let columns = children(root, "columns")
            .into_iter()
            .find(|columns| {
                child(*columns, "id")
                    .map(|id| id.text().unwrap_or("").trim().to_string())
                    .as_deref()
                    == set
            })
            .ok_or_else(|| match set {
                Some(id) => source(format!("the column set `{id}` of the row does not exist")),
                None => source("the spreadsheet has no default column set"),
            })?;
        let size = child(columns, "size").ok_or_else(|| source("a column set has no size"))?;
        let current =
            int_of(columns, "size").ok_or_else(|| source("a column set has an invalid size"))?;
        if current >= count {
            return Ok(());
        }
        let text = size
            .first_child()
            .ok_or_else(|| source("a column set has an empty size"))?;
        (text.range(), count.to_string())
    };
    xml.replace_range(edit.0, &edit.1);
    Ok(())
}

/// The new named area in platform form, among the others ordered by name
/// ignoring case.
fn insert_named_area(
    xml: &mut String,
    area: &str,
    begin: i64,
    end: i64,
) -> Result<(), MxlEditError> {
    let (next, xsi) = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let xsi = match root.lookup_prefix(XSI_NS) {
            Some(prefix) if !prefix.is_empty() => prefix.to_string(),
            _ => {
                return Err(source(
                    "the spreadsheet document does not declare a prefix for the XML Schema instance namespace",
                ))
            }
        };
        let folded = area.to_lowercase();
        let items = named_items(root);
        let next = items
            .iter()
            .find(|item| text_of(**item, "name").trim().to_lowercase() > folded)
            .map(|item| (item.range().start, true))
            .or_else(|| items.last().map(|item| (item.range().end, false)));
        (next, xsi)
    };
    let eol = line_break(xml, next.map_or(xml.len(), |(at, _)| at));
    let fragment = format!(
        "<namedItem {xsi}:type=\"NamedItemCells\">{eol}\t\t<name>{}</name>{eol}\t\t<area>{eol}\t\t\t<type>Rows</type>{eol}\t\t\t<beginRow>{begin}</beginRow>{eol}\t\t\t<endRow>{end}</endRow>{eol}\t\t\t<beginColumn>-1</beginColumn>{eol}\t\t\t<endColumn>-1</endColumn>{eol}\t\t</area>{eol}\t</namedItem>",
        escape_text(area)
    );
    match next {
        Some((at, true)) => xml.insert_str(at, &format!("{fragment}{eol}\t")),
        Some((at, false)) => xml.insert_str(at, &format!("{eol}\t{fragment}")),
        None => insert_root_child(xml, "namedItem", &fragment)?,
    }
    Ok(())
}

/// Raises the document height to `height`; `vgRows` follows it when it was
/// equal to the height, as in every platform export.
fn raise_height(xml: &mut String, height: i64) -> Result<(), MxlEditError> {
    let (edits, missing) = {
        let document = parse(xml)?;
        let root = document_root(&document)?;
        let current = int_of(root, "height");
        let old = current.unwrap_or(0);
        if old >= height {
            return Ok(());
        }
        let mut edits = Vec::new();
        if let Some(node) = child(root, "height") {
            let text = node
                .first_child()
                .ok_or_else(|| source("height is empty"))?;
            edits.push((text.range(), height.to_string()));
        }
        if let Some(node) = child(root, "vgRows") {
            if int_of(root, "vgRows") == Some(old) {
                let text = node
                    .first_child()
                    .ok_or_else(|| source("vgRows is empty"))?;
                edits.push((text.range(), height.to_string()));
            }
        }
        (edits, current.is_none())
    };
    apply(xml, edits);
    if missing {
        insert_root_child(xml, "height", &format!("<height>{height}</height>"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "mxl_cells/tests.rs"]
mod tests;
