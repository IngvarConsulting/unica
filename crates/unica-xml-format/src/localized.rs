//! Lossless existing multilingual XML item replacement.

pub const DATA_CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";

/// The replacement of a multilingual `property` of `source` (for its
/// `range()`) that sets the `lang` value to `text`.
///
/// Only the `v8:content` of the `lang` item changes; the other `v8:item`s
/// keep their bytes. A missing `lang` item is appended after the last one.
/// An empty `text` removes the `lang` item, and a property left without items
/// becomes empty. Element names keep the prefixes the source uses.
pub fn replace_language_value(
    source: &str,
    property: roxmltree::Node<'_, '_>,
    indent: &str,
    lang: &str,
    text: &str,
) -> String {
    fn core_child<'a, 'input>(
        node: roxmltree::Node<'a, 'input>,
        name: &str,
    ) -> Option<roxmltree::Node<'a, 'input>> {
        node.children().find(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(DATA_CORE_NS)
                && child.tag_name().name() == name
        })
    }
    // The qualified name an element is spelled with in the source.
    fn qualified_name<'s>(source: &'s str, node: roxmltree::Node<'_, '_>) -> &'s str {
        let open = &source[node.range().start + 1..];
        let end = open
            .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
            .unwrap_or(open.len());
        &open[..end]
    }
    let range = property.range();
    let existing = &source[range.clone()];
    let local = |inner: std::ops::Range<usize>| inner.start - range.start..inner.end - range.start;
    let qname = qualified_name(source, property);
    let escaped = crate::text::escape_xml(text);
    let lang_text = crate::text::escape_xml(lang);
    let items = property
        .children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(DATA_CORE_NS)
                && child.tag_name().name() == "item"
        })
        .collect::<Vec<_>>();
    debug_assert!(!lang.is_empty(), "a multilingual value needs a language");
    if items.is_empty() {
        if text.is_empty() {
            return format!("<{qname}/>");
        }
        // New lines follow the line endings the descriptor already uses.
        let eol = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let (core, declaration) = match property.lookup_prefix(DATA_CORE_NS) {
            Some("") => (String::new(), String::new()),
            Some(prefix) => (format!("{prefix}:"), String::new()),
            None => ("v8:".to_string(), format!(" xmlns:v8=\"{DATA_CORE_NS}\"")),
        };
        return format!(
            "<{qname}{declaration}>{eol}{indent}\t<{core}item>{eol}{indent}\t\t<{core}lang>{lang_text}</{core}lang>{eol}{indent}\t\t<{core}content>{escaped}</{core}content>{eol}{indent}\t</{core}item>{eol}{indent}</{qname}>"
        );
    }
    let target = items.iter().copied().find(|item| {
        core_child(*item, "lang").is_some_and(|node| crate::read::direct_text(node).trim() == lang)
    });
    let mut updated = existing.to_string();
    match target {
        Some(item) if !text.is_empty() => {
            if let Some(content) = core_child(item, "content") {
                let content_name = qualified_name(source, content);
                updated.replace_range(
                    local(content.range()),
                    &format!("<{content_name}>{escaped}</{content_name}>"),
                );
            } else {
                let language = core_child(item, "lang").expect("the item was found by its lang");
                let core = qualified_name(source, language)
                    .rsplit_once(':')
                    .map_or(String::new(), |(prefix, _)| format!("{prefix}:"));
                updated.insert_str(
                    local(language.range()).end,
                    &format!("<{core}content>{escaped}</{core}content>"),
                );
            }
        }
        Some(item) => {
            if items.len() == 1 {
                return format!("<{qname}/>");
            }
            let item_range = local(item.range());
            let line_start = existing[..item_range.start]
                .rfind('\n')
                .map(|newline| newline + 1)
                .filter(|start| {
                    existing[*start..item_range.start]
                        .chars()
                        .all(|ch| ch == '\t' || ch == ' ')
                });
            let removed = match line_start {
                // The item owns its line: drop the line with its terminator.
                Some(start) => {
                    let rest = &existing[item_range.end..];
                    let eol = if rest.starts_with("\r\n") {
                        2
                    } else {
                        usize::from(rest.starts_with('\n'))
                    };
                    start..item_range.end + eol
                }
                None => item_range,
            };
            updated.replace_range(removed, "");
        }
        None if text.is_empty() => {}
        None => {
            let last = *items.last().expect("items are not empty");
            let core = qualified_name(source, last)
                .rsplit_once(':')
                .map_or(String::new(), |(prefix, _)| format!("{prefix}:"));
            let last = local(last.range());
            let line_start = existing[..last.start]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            let item_indent = &existing[line_start..last.start];
            let item = if line_start > 0 && item_indent.chars().all(|ch| ch == '\t' || ch == ' ') {
                let eol = if existing[..line_start].ends_with("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                };
                format!(
                    "{eol}{item_indent}<{core}item>{eol}{item_indent}\t<{core}lang>{lang_text}</{core}lang>{eol}{item_indent}\t<{core}content>{escaped}</{core}content>{eol}{item_indent}</{core}item>"
                )
            } else {
                format!(
                    "<{core}item><{core}lang>{lang_text}</{core}lang><{core}content>{escaped}</{core}content></{core}item>"
                )
            };
            updated.insert_str(last.end, &item);
        }
    }
    updated
}
