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
    replace_language_value_impl(source, property, indent, lang, text, false)
}

/// Replace text while retaining local namespace declarations. New metadata
/// input settings use this entry point; existing consumers retain their API.
pub fn replace_language_value_preserving_namespaces(
    source: &str,
    property: roxmltree::Node<'_, '_>,
    indent: &str,
    lang: &str,
    text: &str,
) -> String {
    replace_language_value_impl(source, property, indent, lang, text, true)
}

fn replace_language_value_impl(
    source: &str,
    property: roxmltree::Node<'_, '_>,
    indent: &str,
    lang: &str,
    text: &str,
    preserve_namespaces: bool,
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
    fn opening_end(raw: &str) -> usize {
        let mut quote = None;
        raw.bytes()
            .enumerate()
            .find_map(|(i, b)| {
                if let Some(q) = quote {
                    if b == q {
                        quote = None;
                    }
                } else if matches!(b, b'\'' | b'"') {
                    quote = Some(b);
                } else if b == b'>' {
                    return Some(i + 1);
                }
                None
            })
            .expect("parsed XML opening element")
    }
    fn wrap(raw: &str, name: &str, content: &str) -> String {
        let open = &raw[..opening_end(raw)];
        let open = open
            .strip_suffix("/>")
            .map_or_else(|| open.to_owned(), |s| format!("{s}>"));
        format!("{open}{content}</{name}>")
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
            return if preserve_namespaces {
                wrap(existing, qname, "")
            } else {
                format!("<{qname}/>")
            };
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
            None => {
                let mut prefix = "v8".to_owned();
                while preserve_namespaces && property.lookup_namespace_uri(Some(&prefix)).is_some()
                {
                    prefix.push('_');
                }
                (
                    format!("{prefix}:"),
                    format!(" xmlns:{prefix}=\"{DATA_CORE_NS}\""),
                )
            }
        };
        if !preserve_namespaces {
            return format!("<{qname}{declaration}>{eol}{indent}\t<{core}item>{eol}{indent}\t\t<{core}lang>{lang_text}</{core}lang>{eol}{indent}\t\t<{core}content>{escaped}</{core}content>{eol}{indent}\t</{core}item>{eol}{indent}</{qname}>");
        }
        let content = format!("{eol}{indent}\t<{core}item{declaration}>{eol}{indent}\t\t<{core}lang>{lang_text}</{core}lang>{eol}{indent}\t\t<{core}content>{escaped}</{core}content>{eol}{indent}\t</{core}item>{eol}{indent}");
        return wrap(existing, qname, &content);
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
                    &if preserve_namespaces {
                        wrap(&source[content.range()], content_name, &escaped)
                    } else {
                        format!("<{content_name}>{escaped}</{content_name}>")
                    },
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
                return if preserve_namespaces {
                    wrap(existing, qname, "")
                } else {
                    format!("<{qname}/>")
                };
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
            // The last item may declare its core prefix locally. A new sibling
            // must carry that binding itself instead of relying on the sibling.
            let prefix = core.strip_suffix(':');
            let declaration = if !preserve_namespaces
                || property.lookup_namespace_uri(prefix) == Some(DATA_CORE_NS)
            {
                String::new()
            } else {
                let name = prefix.map_or_else(|| "xmlns".to_owned(), |p| format!("xmlns:{p}"));
                format!(" {name}=\"{DATA_CORE_NS}\"")
            };
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
                    "{eol}{item_indent}<{core}item{declaration}>{eol}{item_indent}\t<{core}lang>{lang_text}</{core}lang>{eol}{item_indent}\t<{core}content>{escaped}</{core}content>{eol}{item_indent}</{core}item>"
                )
            } else {
                format!(
                    "<{core}item{declaration}><{core}lang>{lang_text}</{core}lang><{core}content>{escaped}</{core}content></{core}item>"
                )
            };
            updated.insert_str(last.end, &item);
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn localized_replacement_preserves_local_namespace_bindings() {
        let md = "http://v8.1c.ru/8.3/MDClasses";
        for source in [
            format!(r#"<v8:Format xmlns:v8="{md}"/>"#),
            format!(
                r#"<v8:Format xmlns:v8="{md}"><c:item xmlns:c="{DATA_CORE_NS}"><c:lang>en</c:lang><c:content>ND=1</c:content></c:item></v8:Format>"#
            ),
            format!(
                r#"<Format xmlns="{md}"><item xmlns="{DATA_CORE_NS}"><lang>en</lang><content>ND=1</content></item></Format>"#
            ),
            format!(
                r#"<Format xmlns="{md}" xmlns:d="{DATA_CORE_NS}"><d:item><d:lang>ru</d:lang><c:content xmlns:c="{DATA_CORE_NS}" xmlns:note="urn:a>b">old</c:content></d:item></Format>"#
            ),
        ] {
            let parsed = roxmltree::Document::parse(&source).unwrap();
            let changed = replace_language_value_preserving_namespaces(
                &source,
                parsed.root_element(),
                "",
                "ru",
                "ЧДЦ=2",
            );
            let result =
                roxmltree::Document::parse(&changed).unwrap_or_else(|e| panic!("{e}: {changed}"));
            assert_eq!(result.root_element().tag_name().namespace(), Some(md));
            let item = result
                .root_element()
                .children()
                .find(|n| {
                    n.is_element()
                        && n.children()
                            .any(|c| c.tag_name().name() == "lang" && c.text() == Some("ru"))
                })
                .unwrap();
            for node in item.descendants().filter(|n| n.is_element()) {
                assert_eq!(node.tag_name().namespace(), Some(DATA_CORE_NS), "{changed}");
            }
            assert_eq!(
                item.children()
                    .find(|n| n.tag_name().name() == "content")
                    .unwrap()
                    .text(),
                Some("ЧДЦ=2")
            );
            if source.contains("ND=1") {
                let en = parsed
                    .root_element()
                    .children()
                    .find(|n| n.is_element())
                    .unwrap();
                assert!(changed.contains(&source[en.range()]));
            }
        }
    }
}
