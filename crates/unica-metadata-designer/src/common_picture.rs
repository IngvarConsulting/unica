//! Availability flags of an existing picture, without rewriting its resources.

use roxmltree::{Document, Node};
use unica_metadata::property_input::CommonPicturePropertyChanges;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
pub const FLAGS: [&str; 2] = ["AvailabilityForChoice", "AvailabilityForAppearance"];

fn child<'a>(parent: Node<'a, 'a>, name: &str) -> Result<Option<Node<'a, 'a>>, String> {
    let mut nodes = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == name);
    match (nodes.next(), nodes.next()) {
        (None, None) => Ok(None),
        (Some(node), None) if node.has_tag_name((MD, name)) => Ok(Some(node)),
        _ => Err(format!("{name} must be one direct MDClasses element")),
    }
}

fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    if node.attributes().next().is_some() || node.children().any(|node| node.is_element()) {
        return Err(format!("{} must be a plain scalar", node.tag_name().name()));
    }
    Ok(unica_xml_format::read::direct_text(node))
}

fn properties<'a>(document: &'a Document<'a>, name: &str) -> Result<Node<'a, 'a>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("CommonPicture requires a MetaDataObject root".into());
    }
    let mut owners = root.children().filter(Node::is_element);
    let owner = match (owners.next(), owners.next()) {
        (Some(owner), None) if owner.has_tag_name((MD, "CommonPicture")) => owner,
        _ => return Err("expected one CommonPicture owner".into()),
    };
    let properties = child(owner, "Properties")?.ok_or("missing Properties")?;
    if scalar(child(properties, "Name")?.ok_or("missing Name")?)? != name {
        return Err("CommonPicture Name does not match its address".into());
    }
    Ok(properties)
}

fn flag(properties: Node<'_, '_>, key: &str) -> Result<Option<bool>, String> {
    child(properties, key)?
        .map(
            |node| match scalar(node)?.trim_matches([' ', '\t', '\r', '\n']) {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                _ => Err(format!("{key} must be an XML boolean")),
            },
        )
        .transpose()
}

pub fn read(source: &str, name: &str) -> Result<[Option<bool>; 2], String> {
    let document = Document::parse(source.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())?;
    let properties = properties(&document, name)?;
    Ok([flag(properties, FLAGS[0])?, flag(properties, FLAGS[1])?])
}

/// Keep comments, processing instructions, CDATA markers and whitespace in place.
fn value_spans(source: &str, node: Node<'_, '_>) -> Vec<std::ops::Range<usize>> {
    let range = node.range();
    let raw = &source[range.clone()];
    let mut quote = None;
    let opening_end = raw
        .bytes()
        .position(|byte| match (quote, byte) {
            (Some(delimiter), byte) if byte == delimiter => {
                quote = None;
                false
            }
            (None, b'\'' | b'"') => {
                quote = Some(byte);
                false
            }
            (None, b'>') => true,
            _ => false,
        })
        .expect("parsed opening tag");
    let mut cursor = range.start + opening_end + 1;
    let end = range.start + raw.rfind("</").expect("nonempty boolean closing tag");
    let mut spans = Vec::new();
    while cursor < end {
        let tail = &source[cursor..end];
        if let Some(tail) = tail.strip_prefix("<!--") {
            cursor += 4 + tail.find("-->").expect("parsed comment") + 3;
        } else if let Some(tail) = tail.strip_prefix("<?") {
            cursor += 2 + tail.find("?>").expect("parsed instruction") + 2;
        } else if let Some(tail) = tail.strip_prefix("<![CDATA[") {
            let stop = cursor + 9 + tail.find("]]>").expect("parsed CDATA");
            spans.push(cursor + 9..stop);
            cursor = stop + 3;
        } else {
            let stop = cursor + tail.find('<').unwrap_or(tail.len());
            spans.push(cursor..stop);
            cursor = stop;
        }
    }
    spans
}

pub fn patch(
    source: &str,
    name: &str,
    changes: &CommonPicturePropertyChanges,
) -> Result<String, String> {
    let observed = read(source, name)?;
    let prefix = if source.starts_with('\u{feff}') {
        "\u{feff}"
    } else {
        ""
    };
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let properties = properties(&document, name)?;
    let mut replacements = Vec::new();
    for (index, requested) in [
        changes.availability_for_choice,
        changes.availability_for_appearance,
    ]
    .into_iter()
    .enumerate()
    {
        let Some(requested) = requested else { continue };
        let old = observed[index]
            .ok_or_else(|| format!("missing {}: only existing flags can change", FLAGS[index]))?;
        if requested == old {
            continue;
        }
        let node = child(properties, FLAGS[index])?.expect("observed flag");
        let mut first = true;
        for range in value_spans(source, node) {
            let raw = &source[range.clone()];
            let trimmed = raw.trim_matches([' ', '\t', '\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            let start =
                range.start + raw.len() - raw.trim_start_matches([' ', '\t', '\r', '\n']).len();
            let end = range.end - (raw.len() - raw.trim_end_matches([' ', '\t', '\r', '\n']).len());
            replacements.push((
                start..end,
                if first {
                    requested.to_string()
                } else {
                    String::new()
                },
            ));
            first = false;
        }
    }
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut updated = source.to_string();
    for (range, text) in replacements {
        updated.replace_range(range, &text);
    }
    let actual = read(&updated, name)?;
    for (index, requested) in [
        changes.availability_for_choice,
        changes.availability_for_appearance,
    ]
    .into_iter()
    .enumerate()
    {
        if actual[index] != requested.or(observed[index]) {
            return Err("CommonPicture postcondition failed".into());
        }
    }
    Ok(format!("{prefix}{updated}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(choice: &str, appearance: &str) -> String {
        format!("\u{feff}<MetaDataObject xmlns=\"{MD}\" version=\"2.20\">\r\n<CommonPicture uuid=\"16820000-0000-4000-8000-000000000001\"><Properties><Name>Probe</Name><Comment>Keep &amp; verify</Comment>{choice}{appearance}</Properties></CommonPicture></MetaDataObject>\r\n")
    }
    #[test]
    fn independent_flags_roundtrip_and_noop_preserve_every_other_byte() {
        for (raw, value) in [
            ("false", false),
            ("0", false),
            ("true", true),
            ("1", true),
            (" &#49; ", true),
            (" <![CDATA[fa]]><!--keep--><?keep yes?>lse ", false),
        ] {
            let before = source(
                &format!(
                    "<m:AvailabilityForChoice xmlns:m=\"{MD}\">{raw}</m:AvailabilityForChoice>"
                ),
                "<AvailabilityForAppearance>false</AvailabilityForAppearance>",
            );
            let same = CommonPicturePropertyChanges {
                availability_for_choice: Some(value),
                availability_for_appearance: None,
            };
            assert_eq!(read(&before, "Probe").unwrap(), [Some(value), Some(false)]);
            assert_eq!(patch(&before, "Probe", &same).unwrap(), before);
            let changed = patch(
                &before,
                "Probe",
                &CommonPicturePropertyChanges {
                    availability_for_choice: Some(!value),
                    ..same
                },
            )
            .unwrap();
            assert_eq!(
                read(&changed, "Probe").unwrap(),
                [Some(!value), Some(false)]
            );
            for kept in [
                "\u{feff}",
                "\r\n",
                "<Comment>Keep &amp; verify</Comment>",
                "<AvailabilityForAppearance>false</AvailabilityForAppearance>",
            ] {
                assert!(changed.contains(kept));
            }
            if raw.contains("<!--keep-->") {
                assert!(changed.contains("<!--keep--><?keep yes?>"));
                assert!(changed.contains("<![CDATA["));
            }
        }
    }
    #[test]
    fn absent_and_malformed_flags_never_become_false() {
        let absent = source(
            "",
            "<AvailabilityForAppearance>true</AvailabilityForAppearance>",
        );
        assert_eq!(read(&absent, "Probe").unwrap(), [None, Some(true)]);
        let change = CommonPicturePropertyChanges {
            availability_for_choice: Some(false),
            availability_for_appearance: None,
        };
        assert!(patch(&absent, "Probe", &change).is_err());
        for raw in ["<AvailabilityForChoice/>", "<AvailabilityForChoice>False</AvailabilityForChoice>", "<AvailabilityForChoice>&#160;false</AvailabilityForChoice>", "<AvailabilityForChoice>true</AvailabilityForChoice><AvailabilityForChoice>false</AvailabilityForChoice>", "<AvailabilityForChoice xmlns=\"urn:foreign\">false</AvailabilityForChoice>", "<AvailabilityForChoice extra=\"x\">false</AvailabilityForChoice>", "<AvailabilityForChoice><Nested/></AvailabilityForChoice>"] {
            let damaged = source(raw,"<AvailabilityForAppearance>true</AvailabilityForAppearance>");
            assert!(read(&damaged,"Probe").is_err(), "{raw}");
            assert!(patch(&damaged,"Probe",&change).is_err(), "{raw}");
        }
    }

    #[test]
    fn scalar_edits_preserve_namespace_declarations_and_embedded_markup_exactly() {
        let changes = CommonPicturePropertyChanges {
            availability_for_choice: Some(true),
            availability_for_appearance: None,
        };
        for (raw, expected) in [
            ("<AvailabilityForChoice xmlns:x=\"urn:a>b\">false</AvailabilityForChoice>", "<AvailabilityForChoice xmlns:x=\"urn:a>b\">true</AvailabilityForChoice>"),
            ("<AvailabilityForChoice xmlns:x='urn:a>b'>false<!--keep--></AvailabilityForChoice>", "<AvailabilityForChoice xmlns:x='urn:a>b'>true<!--keep--></AvailabilityForChoice>"),
            ("<AvailabilityForChoice> <![CDATA[fa]]><!--keep--><?keep yes?>lse </AvailabilityForChoice>", "<AvailabilityForChoice> <![CDATA[true]]><!--keep--><?keep yes?> </AvailabilityForChoice>"),
        ] {
            let before = source(raw, "<AvailabilityForAppearance>false</AvailabilityForAppearance>");
            assert_eq!(patch(&before, "Probe", &changes).unwrap(), before.replace(raw, expected));
        }
        let absent = source(
            "",
            "<AvailabilityForAppearance>true</AvailabilityForAppearance>",
        );
        let appearance_only = CommonPicturePropertyChanges {
            availability_for_choice: None,
            availability_for_appearance: Some(false),
        };
        let after = patch(&absent, "Probe", &appearance_only).unwrap();
        assert_eq!(
            after,
            absent.replace(
                "<AvailabilityForAppearance>true",
                "<AvailabilityForAppearance>false"
            )
        );
        assert_eq!(read(&after, "Probe").unwrap(), [None, Some(false)]);
    }
}
