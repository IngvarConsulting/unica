//! Reversible spelling of a metadata descriptor whose MDClasses elements use a
//! namespace prefix.
//!
//! Writers spell MDClasses elements without a prefix, in the default
//! namespace. A descriptor written as `<md:MetaDataObject xmlns:md=…>` is
//! brought into that form before writing and back afterwards, so every writer
//! keeps the owner's spelling without knowing it. The original prefix binding
//! remains available to QName values. The conversion is used only
//! when it restores the original bytes exactly.

/// The MDClasses namespace of 1C metadata descriptors.
pub const MDCLASSES: &str = "http://v8.1c.ru/8.3/MDClasses";

/// How a descriptor spells its MDClasses elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixForm {
    /// Default namespace, or not an MDClasses document: nothing to convert.
    Default,
    /// Every MDClasses element uses this prefix, declared on the root.
    Prefixed(String),
    /// Prefixed elements with an original lexical default namespace value.
    PrefixedWithDefault {
        prefix: String,
        default_value: String,
    },
}

impl PrefixForm {
    /// Normalize element spelling without replacing the original prefix binding.
    pub fn normalize(source: &str) -> Result<(String, PrefixForm), String> {
        let body = source.trim_start_matches('\u{feff}');
        let Ok(document) = roxmltree::Document::parse(body) else {
            return Ok((source.to_string(), PrefixForm::Default));
        };
        let root = document.root_element();
        if root.tag_name().namespace() != Some(MDCLASSES)
            || root.lookup_namespace_uri(None) == Some(MDCLASSES)
        {
            return Ok((source.to_string(), PrefixForm::Default));
        }
        let start = source.len() - body.len() + root.range().start;
        let opening = opening_range(source, start)?;
        let tag = &source[opening];
        let Some((prefix, _)) = tag[1..name_end(tag)].split_once(':') else {
            return Ok((source.to_string(), PrefixForm::Default));
        };
        let refused = || {
            format!("the descriptor spells MDClasses elements with the `{prefix}:` prefix in a form that cannot be preserved; use one `{prefix}:` prefix declared on the root for every MDClasses element")
        };
        if root.lookup_namespace_uri(Some(prefix)) != Some(MDCLASSES)
            || document.descendants().any(|node| {
                node.is_element() && node.tag_name().namespace().is_none()
                    || node.is_element()
                        && node.tag_name().namespace() == Some(MDCLASSES)
                        && !body[node.range()].starts_with(&format!("<{prefix}:"))
            })
        {
            return Err(refused());
        }
        let form = match default_value(tag)? {
            Some((begin, end)) => PrefixForm::PrefixedWithDefault {
                prefix: prefix.to_string(),
                default_value: tag[begin..end].to_string(),
            },
            None => PrefixForm::Prefixed(prefix.to_string()),
        };
        let mut normalized = rename(source, &document, prefix, Direction::Strip);
        set_default(&mut normalized, start, Some(MDCLASSES))?;
        let normalized_document =
            roxmltree::Document::parse(normalized.trim_start_matches('\u{feff}'))
                .map_err(|_| refused())?;
        if !same_expanded_names(&document, &normalized_document)
            || form.restore(&normalized).ok().as_deref() != Some(source)
        {
            return Err(refused());
        }
        Ok((normalized, form))
    }

    /// Restore namespace spelling and verify that expanded XML names survive.
    pub fn restore(&self, written: &str) -> Result<String, String> {
        let (prefix, original_default) = match self {
            PrefixForm::Default => return Ok(written.to_string()),
            PrefixForm::Prefixed(prefix) => (prefix, None),
            PrefixForm::PrefixedWithDefault {
                prefix,
                default_value,
            } => (prefix, Some(default_value.as_str())),
        };
        let body = written.trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(body)
            .map_err(|error| format!("written descriptor is not valid XML: {error}"))?;
        let root = document.root_element();
        if root.lookup_namespace_uri(None) != Some(MDCLASSES)
            || root.lookup_namespace_uri(Some(prefix)) != Some(MDCLASSES)
        {
            return Err("written descriptor lost its MDClasses namespace binding".to_string());
        }
        let start = written.len() - body.len() + root.range().start;
        let mut restored = rename(written, &document, prefix, Direction::Add);
        set_default(&mut restored, start, original_default)?;
        let restored_document = roxmltree::Document::parse(restored.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("restored descriptor is not valid XML: {error}"))?;
        if !same_expanded_names(&document, &restored_document) {
            return Err("restoring the descriptor would change an expanded XML name".to_string());
        }
        Ok(restored)
    }
}

fn name_end(opening: &str) -> usize {
    opening
        .as_bytes()
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, byte)| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
        .map_or(opening.len(), |(index, _)| index)
}

// Only inspect the actual root opening, respecting quoted `>` characters.
fn opening_range(source: &str, start: usize) -> Result<std::ops::Range<usize>, String> {
    let mut quote = None;
    for (offset, byte) in source.as_bytes()[start..].iter().copied().enumerate() {
        match (quote, byte) {
            (Some(current), value) if current == value => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Ok(start..start + offset + 1),
            _ => {}
        }
    }
    Err("the root opening tag is incomplete".to_string())
}

fn default_value(opening: &str) -> Result<Option<(usize, usize)>, String> {
    crate::text::cfe_borrow_xml_attribute_value_range(opening, "xmlns")
        .map_err(|_| "the default namespace declaration is malformed".to_string())
}

fn set_default(source: &mut String, start: usize, value: Option<&str>) -> Result<(), String> {
    let opening = opening_range(source, start)?;
    let tag = &source[opening];
    match (default_value(tag)?, value) {
        (Some((begin, end)), Some(value)) => {
            source.replace_range(start + begin..start + end, value)
        }
        (None, Some(value)) => {
            let at = start + name_end(tag);
            source.insert_str(at, &format!(" xmlns=\"{value}\""));
        }
        (Some((begin, end)), None) => {
            // Remove the real assignment identified by the existing lexer,
            // including the whitespace owned by the inserted declaration.
            let mut at = begin - 1;
            let bytes = tag.as_bytes();
            while at > 0 && bytes[at - 1].is_ascii_whitespace() {
                at -= 1;
            }
            if at == 0 || bytes[at - 1] != b'=' {
                return Err("invalid default namespace assignment".to_string());
            }
            at -= 1;
            while at > 0 && bytes[at - 1].is_ascii_whitespace() {
                at -= 1;
            }
            if at < 5 || &tag[at - 5..at] != "xmlns" {
                return Err("invalid default namespace name".to_string());
            }
            at -= 5;
            while at > 0 && bytes[at - 1].is_ascii_whitespace() {
                at -= 1;
            }
            source.replace_range(start + at..start + end + 1, "");
        }
        (None, None) => {}
    }
    Ok(())
}

fn same_expanded_names(left: &roxmltree::Document<'_>, right: &roxmltree::Document<'_>) -> bool {
    let left = left.descendants().filter(|node| node.is_element());
    let right = right.descendants().filter(|node| node.is_element());
    left.map(|node| {
        (
            node.tag_name(),
            node.attributes()
                .map(|attribute| (attribute.namespace(), attribute.name()))
                .collect::<Vec<_>>(),
        )
    })
    .eq(right.map(|node| {
        (
            node.tag_name(),
            node.attributes()
                .map(|attribute| (attribute.namespace(), attribute.name()))
                .collect::<Vec<_>>(),
        )
    }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Strip,
    Add,
}

/// Remove or add `prefix:` on MDClasses element names; bindings stay in place.
fn rename(
    source: &str,
    document: &roxmltree::Document<'_>,
    prefix: &str,
    direction: Direction,
) -> String {
    let body = source.trim_start_matches('\u{feff}');
    let offset = source.len() - body.len();
    let qualified = format!("{prefix}:");
    // (position, remove `prefix:` there or insert it).
    let mut edits = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MDCLASSES))
    {
        let range = node.range();
        let start = offset + range.start + 1;
        let close = body[range.clone()]
            .rfind("</")
            .map(|close| offset + range.start + close + 2);
        edits.push(start);
        edits.extend(close);
    }
    edits.sort_unstable_by(|left, right| right.cmp(left));
    let mut text = source.to_string();
    for at in edits {
        match direction {
            Direction::Strip => {
                debug_assert!(text[at..].starts_with(&qualified));
                text.replace_range(at..at + qualified.len(), "");
            }
            Direction::Add => text.insert_str(at, &qualified),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIXED: &str = concat!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<md:MetaDataObject xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\">\r\n",
        "\t<md:Catalog uuid=\"1\">\r\n\t\t<md:Properties>\r\n\t\t\t<md:Name>A</md:Name>\r\n",
        "\t\t\t<md:Synonym><v8:item><v8:lang>ru</v8:lang></v8:item></md:Synonym>\r\n",
        "\t\t\t<md:Comment/>\r\n\t\t</md:Properties>\r\n\t</md:Catalog>\r\n</md:MetaDataObject>"
    );

    #[test]
    fn prefixed_descriptor_round_trips_byte_for_byte() {
        let (normalized, form) = PrefixForm::normalize(PREFIXED).unwrap();
        assert_eq!(form, PrefixForm::Prefixed("md".into()));
        assert!(normalized.contains("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\""));
        assert!(normalized.contains("<Comment/>") && normalized.contains("<v8:item>"));
        assert_eq!(form.restore(&normalized).unwrap(), PREFIXED);
        // A written element in the default form gets the owner's prefix.
        let written = normalized.replace("<Comment/>", "<Comment>Note</Comment>");
        assert_eq!(
            form.restore(&written).unwrap(),
            PREFIXED.replace("<md:Comment/>", "<md:Comment>Note</md:Comment>")
        );
    }

    #[test]
    fn default_namespace_and_foreign_documents_are_untouched() {
        let default = PREFIXED.replace("md:", "").replace("xmlns:md=", "xmlns=");
        let (normalized, form) = PrefixForm::normalize(&default).unwrap();
        assert_eq!(
            (normalized.as_str(), &form),
            (default.as_str(), &PrefixForm::Default)
        );
        let form_xml = "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\"/>";
        assert_eq!(
            PrefixForm::normalize(form_xml).unwrap().1,
            PrefixForm::Default
        );
    }

    #[test]
    fn mixed_or_irreversible_spellings_are_refused() {
        // An unprefixed element would change its namespace.
        let unprefixed = PREFIXED.replace("<md:Comment/>", "<Comment/>");
        assert!(PrefixForm::normalize(&unprefixed).is_err());
        // A second prefix for MDClasses cannot be restored as one prefix.
        let mixed = PREFIXED.replace(
            "<md:Comment/>",
            "<m:Comment xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\"/>",
        );
        assert!(PrefixForm::normalize(&mixed).is_err());
    }
    #[test]
    fn foreign_default_namespace_round_trips_without_duplicate_declarations() {
        for declarations in [
            r#"xmlns:md="http://v8.1c.ru/8.3/MDClasses" xmlns="urn:unrelated""#,
            "xmlns = 'urn:unrelated' xmlns:md = 'http://v8.1c.ru/8.3/MDClasses'",
        ] {
            let source = format!("\u{feff}<?xml version=\"1.0\"?>\r\n<md:MetaDataObject {declarations}><md:Catalog><md:Properties><md:Name>A</md:Name><md:Comment/></md:Properties></md:Catalog></md:MetaDataObject>");
            let (normalized, form) = PrefixForm::normalize(&source).unwrap();
            let document =
                roxmltree::Document::parse(normalized.trim_start_matches('\u{feff}')).unwrap();
            assert_eq!(
                document.root_element().lookup_namespace_uri(Some("md")),
                Some(MDCLASSES)
            );
            assert_eq!(form.restore(&normalized).unwrap(), source);
            let written = normalized.replace("<Comment/>", "<Comment>Note</Comment>");
            assert_eq!(
                form.restore(&written).unwrap(),
                source.replace("<md:Comment/>", "<md:Comment>Note</md:Comment>")
            );
        }
    }

    #[test]
    fn namespace_spelling_outside_the_root_declaration_is_never_rewritten() {
        let source = concat!(
            "<!-- xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\" -->",
            "<?probe xmlns=\"not-a-binding\"?>",
            "<md:MetaDataObject note='xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\" >' xmlns:md = 'http://v8.1c.ru/8.3/MDClasses' xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">",
            "<md:Catalog xsi:type=\"md:Catalog\"><md:Properties><md:Name>A</md:Name><md:Comment>md:Catalog</md:Comment></md:Properties></md:Catalog></md:MetaDataObject>"
        );
        let (normalized, form) = PrefixForm::normalize(source).unwrap();
        let document = roxmltree::Document::parse(&normalized).unwrap();
        assert_eq!(
            document.root_element().lookup_namespace_uri(Some("md")),
            Some(MDCLASSES)
        );
        assert!(normalized.contains("xsi:type=\"md:Catalog\""));
        assert!(normalized.contains("<Comment>md:Catalog</Comment>"));
        assert_eq!(form.restore(&normalized).unwrap(), source);
    }

    #[test]
    fn foreign_elements_keep_their_expanded_names_or_are_refused() {
        let prefix = r#"<md:MetaDataObject xmlns:md="http://v8.1c.ru/8.3/MDClasses" xmlns="urn:foreign" xmlns:f="urn:foreign">"#;
        let foreign = format!("{prefix}<f:Foreign/><md:Catalog/></md:MetaDataObject>");
        let (normalized, form) = PrefixForm::normalize(&foreign).unwrap();
        assert_eq!(form.restore(&normalized).unwrap(), foreign);
        let unprefixed = foreign.replace("<f:Foreign/>", "<Foreign/>");
        assert!(PrefixForm::normalize(&unprefixed).is_err());
    }
    #[test]
    fn an_existing_mdclasses_default_keeps_the_previous_unchanged_route() {
        let source = PREFIXED
            .replace("xmlns:md=", &format!("xmlns=\"{MDCLASSES}\" xmlns:md="))
            .replace("<md:Comment/>", "<Comment/>");
        assert_eq!(
            PrefixForm::normalize(&source).unwrap(),
            (source, PrefixForm::Default)
        );
    }

    #[test]
    fn restore_checks_new_elements_and_namespace_binding_changes() {
        let (normalized, form) = PrefixForm::normalize(PREFIXED).unwrap();
        let written = normalized.replace("<Comment/>", "<Comment/><Added/>");
        assert_eq!(
            form.restore(&written).unwrap(),
            PREFIXED.replace("<md:Comment/>", "<md:Comment/><md:Added/>")
        );
        let changed_binding = normalized.replace(
            &format!("xmlns:md=\"{MDCLASSES}\""),
            "xmlns:md=\"urn:wrong\"",
        );
        assert!(form.restore(&changed_binding).is_err());
        let rebound = normalized.replace(
            "<Comment/>",
            "<Container xmlns:md=\"urn:wrong\"><Added/></Container>",
        );
        assert!(form.restore(&rebound).is_err());
    }
}
