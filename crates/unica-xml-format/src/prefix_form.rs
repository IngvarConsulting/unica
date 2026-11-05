//! Reversible spelling of a metadata descriptor whose MDClasses elements use a
//! namespace prefix.
//!
//! Writers spell MDClasses elements without a prefix, in the default
//! namespace. A descriptor written as `<md:MetaDataObject xmlns:md=…>` is
//! brought into that form before writing and back afterwards, so every writer
//! keeps the owner's spelling without knowing it. The conversion is used only
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
}

impl PrefixForm {
    /// Bring `source` into the default-namespace spelling. Returns the text the
    /// writers work on and the form that restores the original spelling.
    pub fn normalize(source: &str) -> Result<(String, PrefixForm), String> {
        let body = source.trim_start_matches('\u{feff}');
        let Ok(document) = roxmltree::Document::parse(body) else {
            // Writers report the invalid XML themselves.
            return Ok((source.to_string(), PrefixForm::Default));
        };
        let root = document.root_element();
        if root.tag_name().namespace() != Some(MDCLASSES)
            || root.lookup_namespace_uri(None) == Some(MDCLASSES)
        {
            return Ok((source.to_string(), PrefixForm::Default));
        }
        let Some(prefix) = root
            .namespaces()
            .find(|namespace| namespace.uri() == MDCLASSES)
            .and_then(|namespace| namespace.name())
            .map(str::to_string)
        else {
            return Ok((source.to_string(), PrefixForm::Default));
        };
        let refused = || {
            format!(
                "the descriptor spells MDClasses elements with the `{prefix}:` prefix \
                 in a form that cannot be preserved; use one `{prefix}:` prefix declared \
                 on the root for every MDClasses element"
            )
        };
        // Unprefixed elements would join MDClasses once it becomes the default.
        if document.descendants().any(|node| {
            node.is_element() && node.tag_name().namespace().is_none()
                || node.is_element()
                    && node.tag_name().namespace() == Some(MDCLASSES)
                    && !body[node.range()].starts_with(&format!("<{prefix}:"))
        }) {
            return Err(refused());
        }
        let normalized = rename(source, &document, &prefix, Direction::Strip);
        let form = PrefixForm::Prefixed(prefix.clone());
        if form.restore(&normalized)? != source {
            return Err(refused());
        }
        Ok((normalized, form))
    }

    /// Restore the original spelling on text written in the default form.
    pub fn restore(&self, written: &str) -> Result<String, String> {
        let PrefixForm::Prefixed(prefix) = self else {
            return Ok(written.to_string());
        };
        let body = written.trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(body)
            .map_err(|error| format!("written descriptor is not valid XML: {error}"))?;
        if document.root_element().lookup_namespace_uri(None) != Some(MDCLASSES) {
            return Err("written descriptor lost its MDClasses namespace".to_string());
        }
        Ok(rename(written, &document, prefix, Direction::Add))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Strip,
    Add,
}

/// Remove or add `prefix:` on every MDClasses element and swap the root's
/// MDClasses declaration between `xmlns:prefix` and `xmlns`.
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
    let (from, to) = match direction {
        Direction::Strip => (
            format!("xmlns:{prefix}=\"{MDCLASSES}\""),
            format!("xmlns=\"{MDCLASSES}\""),
        ),
        Direction::Add => (
            format!("xmlns=\"{MDCLASSES}\""),
            format!("xmlns:{prefix}=\"{MDCLASSES}\""),
        ),
    };
    text.replacen(&from, &to, 1)
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
}
