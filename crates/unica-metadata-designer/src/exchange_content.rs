//! ExchangePlan Content.xml observations. Unknown or damaged items never become
//! a known empty composition; independent valid entries remain available.
use roxmltree::{Document, Node};
use std::collections::HashSet;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::{
    metadata_name_eq, MetaExchangeAutoRecord as AutoRecord, MetaExchangeContentItem as ContentItem,
};

const NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

/// The document the platform would write for a composition with no items.
/// It never writes it — an empty composition has no Content.xml — but the
/// first added item turns it into the platform's file, header included.
pub const EMPTY_CONTENT: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<ExchangePlanContent xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n</ExchangePlanContent>";

#[derive(Debug, PartialEq, Eq)]
pub struct ContentObservation {
    pub items: Vec<ContentItem>,
    pub errors: Vec<String>,
}

fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    if node.attributes().len() != 0 || node.children().any(|n| n.is_element()) {
        return Err(format!("{} must contain only text", node.tag_name().name()));
    }
    Ok(node
        .children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect())
}

fn read_item(node: Node<'_, '_>) -> Result<ContentItem, String> {
    if !node.has_tag_name((NS, "Item")) || node.attributes().len() != 0 {
        return Err("expected an ExchangePlanContent Item".into());
    }
    let mut metadata = None;
    let mut mode = None;
    for child in node.children() {
        if child.is_text() && child.text().is_some_and(|s| !s.trim().is_empty()) {
            return Err("unexpected text in ExchangePlanContent Item".into());
        }
        if !child.is_element() {
            continue;
        }
        if child.has_tag_name((NS, "Metadata")) && metadata.is_none() {
            metadata = Some(scalar(child)?);
        } else if child.has_tag_name((NS, "AutoRecord")) && mode.is_none() {
            mode = Some(scalar(child)?);
        } else {
            return Err("unknown or repeated field in ExchangePlanContent Item".into());
        }
    }
    let target = metadata.ok_or("ExchangePlanContent Item has no Metadata")?;
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &target)
        .map_err(|_| "ExchangePlanContent Metadata is not a metadata address")?;
    if target.segments().count() != 2 {
        return Err("ExchangePlanContent Metadata must name a top-level object".into());
    }
    let auto_record = match mode.as_deref() {
        Some("Allow") => AutoRecord::Allow,
        Some("Deny") => AutoRecord::Deny,
        _ => return Err("ExchangePlanContent AutoRecord must be Allow or Deny".into()),
    };
    Ok(ContentItem {
        target,
        auto_record,
    })
}

pub fn read_content(xml: &str) -> Result<ContentObservation, String> {
    let doc = Document::parse(xml).map_err(|e| format!("invalid ExchangePlanContent XML: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name((NS, "ExchangePlanContent")) {
        return Err("expected ExchangePlanContent in the platform namespace".into());
    }
    if root
        .attributes()
        .any(|a| a.namespace().is_some() || a.name() != "version")
    {
        return Err("unknown ExchangePlanContent root attribute".into());
    }
    if root.attribute("version") != Some("2.20") {
        return Err("ExchangePlanContent requires format version 2.20".into());
    }
    let mut result = ContentObservation {
        items: Vec::new(),
        errors: Vec::new(),
    };
    let mut seen = HashSet::new();
    for node in root.children() {
        if node.is_text() && node.text().is_some_and(|s| !s.trim().is_empty()) {
            result
                .errors
                .push("unexpected text in ExchangePlanContent".into());
        }
        if !node.is_element() {
            continue;
        }
        match read_item(node) {
            Ok(item) => {
                if seen.insert(item.target.as_str().to_lowercase()) {
                    result.items.push(item);
                } else {
                    result.errors.push(format!(
                        "repeated ExchangePlanContent target {}",
                        item.target.as_str()
                    ));
                }
            }
            Err(error) => result.errors.push(error),
        }
    }
    Ok(result)
}

/// Upsert only the named targets. Callers resolve target existence and retain
/// those descriptors before publishing this image through the saved plan.
pub fn update_content(xml: &str, updates: &[ContentItem]) -> Result<String, String> {
    if updates.is_empty() {
        return Err("ExchangePlanContent update must contain at least one item".into());
    }
    let observed = read_content(xml)?;
    if !observed.errors.is_empty() {
        return Err(observed.errors.join("; "));
    }
    let mut targets = HashSet::new();
    for item in updates {
        if item.target.segments().count() != 2
            || !targets.insert(item.target.as_str().to_lowercase())
        {
            return Err("ExchangePlanContent update contains a nested or repeated target".into());
        }
    }
    let doc = Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let mut edits = Vec::new();
    let mut additions = String::new();
    let prefix = root
        .lookup_prefix(NS)
        .map(|p| format!("{p}:"))
        .unwrap_or_default();
    let eol = if xml.contains("\r\n") { "\r\n" } else { "\n" };
    for update in updates {
        let mode = match update.auto_record {
            AutoRecord::Allow => "Allow",
            AutoRecord::Deny => "Deny",
        };
        let existing = root.children().filter(|n| n.is_element()).find(|n| {
            read_item(*n)
                .is_ok_and(|item| metadata_name_eq(item.target.as_str(), update.target.as_str()))
        });
        if let Some(existing) = existing {
            let item = read_item(existing)?;
            if item.auto_record == update.auto_record {
                continue;
            }
            let mode_node = existing
                .children()
                .find(|n| n.has_tag_name((NS, "AutoRecord")))
                .ok_or("ExchangePlanContent Item has no AutoRecord")?;
            let range = mode_node.range();
            let fragment = &xml[range.clone()];
            let start = fragment.find('>').ok_or("invalid AutoRecord element")? + 1;
            let end = fragment.rfind('<').ok_or("invalid AutoRecord element")?;
            edits.push((range.start + start..range.start + end, mode.to_owned()));
        } else {
            let target = unica_xml_format::text::escape_xml(update.target.as_str());
            // The platform writes every field on its own tab-indented line.
            additions.push_str(&format!(
                "\t<{prefix}Item>{eol}\t\t<{prefix}Metadata>{target}</{prefix}Metadata>{eol}\t\t<{prefix}AutoRecord>{mode}</{prefix}AutoRecord>{eol}\t</{prefix}Item>{eol}"
            ));
        }
    }
    if !additions.is_empty() {
        let range = root.range();
        let fragment = &xml[range.clone()];
        if fragment.ends_with("/>") {
            let root_qname = fragment[1..]
                .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .next()
                .ok_or("missing ExchangePlanContent root name")?;
            let start = range.end - 2;
            edits.push((
                start..range.end,
                format!(">{eol}{additions}</{root_qname}>"),
            ));
        } else {
            let end = range.start
                + fragment
                    .rfind("</")
                    .ok_or("missing ExchangePlanContent closing tag")?;
            // New items go on their own lines before the closing tag's line,
            // which keeps its indentation; an inline closing tag moves down.
            let line = xml[..end].trim_end_matches([' ', '\t']);
            if line.ends_with('\n') {
                let at = line.len();
                edits.push((at..at, additions));
            } else {
                edits.push((end..end, format!("{eol}{additions}")));
            }
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut result = xml.to_owned();
    for (range, value) in edits.into_iter().rev() {
        result.replace_range(range, &value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wrap(items: &str) -> String {
        format!(r#"<ExchangePlanContent xmlns="{NS}" version="2.20">{items}</ExchangePlanContent>"#)
    }
    fn item(target: &str, mode: &str) -> String {
        format!("<Item><Metadata>{target}</Metadata><AutoRecord>{mode}</AutoRecord></Item>")
    }
    #[test]
    fn content_reader_keeps_order_and_modes_without_mutating_input() {
        let xml = wrap(&(item("Catalog.Прежний", "Deny") + &item("Catalog.Пример", "Allow")));
        let before = xml.clone();
        let result = read_content(&xml).unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(
            result
                .items
                .iter()
                .map(|i| (i.target.as_str(), i.auto_record))
                .collect::<Vec<_>>(),
            [
                ("Catalog.Прежний", AutoRecord::Deny),
                ("Catalog.Пример", AutoRecord::Allow)
            ]
        );
        assert_eq!(xml, before);
        assert_eq!(
            read_content(&wrap("")).unwrap(),
            ContentObservation {
                items: vec![],
                errors: vec![]
            }
        );
    }
    #[test]
    fn content_reader_preserves_independent_entries_and_marks_damage() {
        for broken in [
            item("Catalog.Bad", "Sometimes"),
            item("Catalog.Bad.Attribute.Nested", "Allow"),
            "<Item><Metadata>Catalog.Bad</Metadata></Item>".into(),
            "<Item><Metadata>Catalog.Bad</Metadata><AutoRecord>Allow</AutoRecord><AutoRecord>Deny</AutoRecord></Item>".into(),
            "<Item xmlns='foreign'><Metadata>Catalog.Bad</Metadata><AutoRecord>Allow</AutoRecord></Item>".into(),
            "<Unknown/>".into(),
        ] {
            let result = read_content(&wrap(&(broken + &item("Catalog.Good", "Deny")))).unwrap();
            assert_eq!(result.items.len(), 1);
            assert_eq!(result.items[0].target.as_str(), "Catalog.Good");
            assert!(!result.errors.is_empty());
        }
        let repeated = item("Catalog.Good", "Allow") + &item("Catalog.Good", "Deny");
        let result = read_content(&wrap(&repeated)).unwrap();
        assert!(!result.errors.is_empty());
        assert!(read_content("").is_err());
        assert!(read_content(&wrap("").replace(NS, "foreign")).is_err());
    }
    #[test]
    fn content_reader_resolves_namespace_aliases_and_entities() {
        let xml = format!(
            r#"<x:ExchangePlanContent xmlns:x="{NS}" version="2.20"><x:Item><x:Metadata>Catalog.&#1055;ример</x:Metadata><x:AutoRecord>Allow</x:AutoRecord></x:Item></x:ExchangePlanContent>"#
        );
        let result = read_content(&xml).unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(result.items[0].target.as_str(), "Catalog.Пример");
    }
    fn update(target: &str, mode: AutoRecord) -> ContentItem {
        ContentItem {
            target: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, target).unwrap(),
            auto_record: mode,
        }
    }
    #[test]
    fn content_upsert_preserves_old_bytes_changes_only_requested_mode_and_repeats_as_noop() {
        let old = "<Item><!--keep--><Metadata>Catalog.Прежний</Metadata><AutoRecord>Deny</AutoRecord></Item>";
        let xml = format!("\u{feff}{}\r\n", wrap(old));
        let added = update_content(&xml, &[update("Catalog.Пример", AutoRecord::Allow)]).unwrap();
        assert!(added.contains(old));
        assert!(added.starts_with('\u{feff}'));
        let observed = read_content(&added).unwrap();
        assert_eq!(observed.items.len(), 2);
        assert_eq!(observed.items[0].auto_record, AutoRecord::Deny);
        assert_eq!(observed.items[1].auto_record, AutoRecord::Allow);
        let denied = update_content(&added, &[update("Catalog.Пример", AutoRecord::Deny)]).unwrap();
        assert_eq!(
            denied,
            added.replacen(
                "<AutoRecord>Allow</AutoRecord>",
                "<AutoRecord>Deny</AutoRecord>",
                1
            )
        );
        assert_eq!(
            update_content(&denied, &[update("Catalog.Пример", AutoRecord::Deny)]).unwrap(),
            denied
        );
        assert_eq!(xml, format!("\u{feff}{}\r\n", wrap(old)));
    }
    #[test]
    fn content_upsert_appends_items_in_the_platform_layout() {
        let platform_item = |target: &str, mode: &str| {
            format!("\t<Item>\r\n\t\t<Metadata>{target}</Metadata>\r\n\t\t<AutoRecord>{mode}</AutoRecord>\r\n\t</Item>\r\n")
        };
        let document = |items: &str| {
            format!("\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<ExchangePlanContent xmlns=\"{NS}\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\r\n{items}</ExchangePlanContent>")
        };
        let xml = document(&platform_item("Catalog.Прежний", "Deny"));
        let added = update_content(&xml, &[update("Catalog.Пример", AutoRecord::Allow)]).unwrap();
        assert_eq!(
            added,
            document(
                &(platform_item("Catalog.Прежний", "Deny")
                    + &platform_item("Catalog.Пример", "Allow"))
            )
        );
        let empty = format!("<ExchangePlanContent xmlns=\"{NS}\" version=\"2.20\"/>");
        assert_eq!(
            update_content(&empty, &[update("Catalog.Пример", AutoRecord::Allow)]).unwrap(),
            format!(
                "<ExchangePlanContent xmlns=\"{NS}\" version=\"2.20\">\n\t<Item>\n\t\t<Metadata>Catalog.Пример</Metadata>\n\t\t<AutoRecord>Allow</AutoRecord>\n\t</Item>\n</ExchangePlanContent>"
            )
        );
    }
    #[test]
    fn content_upsert_preserves_lexical_root_with_multiple_namespace_aliases() {
        for declarations in [
            format!(r#"xmlns="{NS}" xmlns:x="{NS}""#),
            format!(r#"xmlns:a="{NS}" xmlns:x="{NS}""#),
        ] {
            let xml = format!(r#"<x:ExchangePlanContent {declarations} version="2.20"/>"#);
            let added = update_content(&xml, &[update("Catalog.New", AutoRecord::Allow)]).unwrap();
            assert!(added.ends_with("</x:ExchangePlanContent>"), "{added}");
            assert_eq!(
                read_content(&added).unwrap().items,
                [update("Catalog.New", AutoRecord::Allow)]
            );
        }
    }
    #[test]
    fn content_upsert_handles_empty_prefixed_root_and_refuses_ambiguous_input() {
        let xml = format!(r#"<x:ExchangePlanContent xmlns:x="{NS}" version="2.20"/>"#);
        let added = update_content(&xml, &[update("Catalog.New", AutoRecord::Allow)]).unwrap();
        assert!(added.contains("<x:Item>"));
        assert_eq!(
            read_content(&added).unwrap().items,
            [update("Catalog.New", AutoRecord::Allow)]
        );
        assert!(update_content(&xml, &[]).is_err());
        assert!(update_content(
            &xml,
            &[
                update("Catalog.New", AutoRecord::Allow),
                update("Catalog.New", AutoRecord::Deny)
            ]
        )
        .is_err());
        assert_eq!(
            update_content(&added, &[update("Catalog.new", AutoRecord::Allow)]).unwrap(),
            added
        );
        assert!(update_content(
            &xml,
            &[
                update("Catalog.New", AutoRecord::Allow),
                update("Catalog.new", AutoRecord::Deny)
            ]
        )
        .is_err());
        let damaged = wrap(&(item("Catalog.Old", "Unknown") + &item("Catalog.Good", "Deny")));
        assert!(update_content(&damaged, &[update("Catalog.New", AutoRecord::Allow)]).is_err());
    }
}
