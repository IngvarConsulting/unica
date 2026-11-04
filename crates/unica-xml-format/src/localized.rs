//! Lossless existing multilingual XML item replacement.

pub const DATA_CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";

/// The replacement of a multilingual `property` of `source` (for its
/// `range()`) that sets the `lang` value to `text`.
///
/// Only the `v8:content` of the `lang` item changes; the other `v8:item`s
/// keep their bytes. A missing `lang` item is appended after the last one.
/// An empty `text` removes the `lang` item, and a property left without items
/// becomes empty when no other content remains. Opening tags and local
/// namespace declarations survive; new children bind their prefixes in scope.
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
    // Locate the opening delimiter without mistaking an attribute's `>` for it.
    fn opening_end(xml: &str) -> usize {
        let mut quote = None;
        for (offset, ch) in xml.char_indices() {
            match (quote, ch) {
                (Some(current), ch) if current == ch => quote = None,
                (None, '\'' | '"') => quote = Some(ch),
                (None, '>') => return offset + 1,
                _ => {}
            }
        }
        unreachable!("a parsed element has an opening delimiter")
    }
    fn prefix_of(name: &str) -> Option<&str> {
        name.split_once(':').map(|(prefix, _)| prefix)
    }
    // A sibling's declaration is not in scope on the new child. Bind the
    // preferred spelling on that child instead of rebinding its parent.
    fn core_name(parent: roxmltree::Node<'_, '_>, prefix: Option<&str>) -> (String, String) {
        let name = prefix.map_or_else(String::new, |prefix| format!("{prefix}:"));
        let declaration = if parent.lookup_namespace_uri(prefix) == Some(DATA_CORE_NS) {
            String::new()
        } else {
            let attribute =
                prefix.map_or_else(|| "xmlns".to_string(), |prefix| format!("xmlns:{prefix}"));
            format!(" {attribute}=\"{DATA_CORE_NS}\"")
        };
        (name, declaration)
    }
    // Replace only the inner image, preserving the original opening tag,
    // including locally declared namespaces and attributes.
    fn with_inner(xml: &str, name: &str, inner: &str) -> String {
        let end = opening_end(xml);
        if xml[..end].ends_with("/>") {
            format!("{}>{inner}</{name}>", &xml[..end - 2])
        } else {
            let close = xml
                .rfind("</")
                .expect("a nonempty element has a closing tag");
            format!("{}{inner}{}", &xml[..end], &xml[close..])
        }
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
            return existing.to_string();
        }
        let prefix = if property.lookup_namespace_uri(None) == Some(DATA_CORE_NS) {
            None
        } else {
            Some(property.lookup_prefix(DATA_CORE_NS).unwrap_or("v8"))
        };
        let (core, declaration) = core_name(property, prefix);
        let end = opening_end(existing);
        let inner = if existing[..end].ends_with("/>") {
            ""
        } else {
            &existing[end..existing.rfind("</").expect("a closing tag")]
        };
        // Keep existing nonstructural content byte for byte. A new formatted
        // item follows the local line ending when one is available.
        let newline = existing
            .find(['\r', '\n'])
            .map(|offset| range.start + offset)
            .or_else(|| source[..range.start].rfind(['\r', '\n']))
            .or_else(|| {
                source[range.end..]
                    .find(['\r', '\n'])
                    .map(|offset| range.end + offset)
            });
        let eol = match newline.map(|offset| (offset, source.as_bytes()[offset])) {
            Some((offset, b'\r')) if source.as_bytes().get(offset + 1) == Some(&b'\n') => "\r\n",
            Some((offset, b'\n')) if offset > 0 && source.as_bytes()[offset - 1] == b'\r' => "\r\n",
            Some((_, b'\r')) => "\r",
            _ => "\n",
        };
        let item = format!(
            "{eol}{indent}\t<{core}item{declaration}>{eol}{indent}\t\t<{core}lang>{lang_text}</{core}lang>{eol}{indent}\t\t<{core}content>{escaped}</{core}content>{eol}{indent}\t</{core}item>{eol}{indent}"
        );
        return with_inner(existing, qname, &format!("{inner}{item}"));
    }
    let target = items.iter().copied().find(|item| {
        core_child(*item, "lang").is_some_and(|node| crate::read::direct_text(node).trim() == lang)
    });
    let mut updated = existing.to_string();
    match target {
        Some(item) if !text.is_empty() => {
            if let Some(content) = core_child(item, "content") {
                if !content.children().any(|node| node.is_element())
                    && crate::read::direct_text(content) == text
                {
                    return updated;
                }
                let content_name = qualified_name(source, content);
                updated.replace_range(
                    local(content.range()),
                    &with_inner(&source[content.range()], content_name, &escaped),
                );
            } else {
                let language = core_child(item, "lang").expect("the item was found by its lang");
                let (core, declaration) =
                    core_name(item, prefix_of(qualified_name(source, language)));
                updated.insert_str(
                    local(language.range()).end,
                    &format!("<{core}content{declaration}>{escaped}</{core}content>"),
                );
            }
        }
        Some(item) => {
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
            let end = opening_end(&updated);
            let close = updated
                .rfind("</")
                .expect("a property containing an item has a closing tag");
            if updated[end..close]
                .chars()
                .all(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n'))
            {
                return format!("{}/>", &updated[..end - 1]);
            }
        }
        None if text.is_empty() => {}
        None => {
            let last = *items.last().expect("items are not empty");
            let (core, declaration) = core_name(property, prefix_of(qualified_name(source, last)));
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
                format!("{eol}{item_indent}<{core}item{declaration}>{eol}{item_indent}\t<{core}lang>{lang_text}</{core}lang>{eol}{item_indent}\t<{core}content>{escaped}</{core}content>{eol}{item_indent}</{core}item>")
            } else {
                format!("<{core}item{declaration}><{core}lang>{lang_text}</{core}lang><{core}content>{escaped}</{core}content></{core}item>")
            };
            updated.insert_str(last.end, &item);
        }
    }
    updated
}

#[cfg(test)]
mod tests;
