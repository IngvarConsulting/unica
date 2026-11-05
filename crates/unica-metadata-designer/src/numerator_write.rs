//! Validate the numerator's own scalar image before publication.
use roxmltree::{Document, Node};
use unica_metadata::document_numerator::{
    NUMBER_ALLOWED_LENGTHS, NUMBER_PERIODICITIES, NUMBER_TYPES,
};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn properties<'a, 'input>(
    document: &'a Document<'input>,
    name: &str,
) -> Result<Node<'a, 'input>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) || root.attribute("version") != Some("2.20") {
        return Err("DocumentNumerator requires a Platform XML 2.20 descriptor".into());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("DocumentNumerator must have exactly one owner".into());
    };
    if !owner.has_tag_name((MD, "DocumentNumerator")) {
        return Err("DocumentNumerator descriptor has a different owner kind".into());
    }
    let containers = owner
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let [properties] = containers.as_slice() else {
        return Err("DocumentNumerator must have exactly one Properties container".into());
    };
    if !properties.has_tag_name((MD, "Properties")) || scalar(*properties, "Name")? != name {
        return Err("DocumentNumerator descriptor identity differs from its address".into());
    }
    for (field, allowed) in [
        ("NumberType", NUMBER_TYPES),
        ("NumberAllowedLength", NUMBER_ALLOWED_LENGTHS),
        ("NumberPeriodicity", NUMBER_PERIODICITIES),
        ("CheckUnique", &["true", "false"][..]),
    ] {
        if !allowed.contains(&scalar(*properties, field)?.as_str()) {
            return Err(format!("DocumentNumerator has an invalid {field}"));
        }
    }
    scalar(*properties, "NumberLength")?
        .parse::<u32>()
        .map_err(|_| {
            "DocumentNumerator NumberLength must be an unsigned 32-bit integer".to_string()
        })?;
    Ok(*properties)
}

pub(crate) fn scalar(properties: Node<'_, '_>, name: &str) -> Result<String, String> {
    let nodes = properties
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .collect::<Vec<_>>();
    let [node] = nodes.as_slice() else {
        return Err(format!(
            "DocumentNumerator requires exactly one direct {name}"
        ));
    };
    if !node.has_tag_name((MD, name))
        || node.children().any(|n| n.is_element())
        || node.attributes().len() != 0
    {
        return Err(format!("DocumentNumerator has a malformed {name}"));
    }
    Ok(node
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect())
}

pub(crate) fn validate(bytes: &[u8], name: &str) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let properties = properties(&document, name)?;
    let length = scalar(properties, "NumberLength")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    if scalar(properties, "NumberType")? == "Number" && length > 38 {
        return Err("DocumentNumerator NumberLength for Number must not exceed 38 digits".into());
    }
    Ok(())
}

pub(crate) fn patch(
    source: &str,
    name: &str,
    changes: &unica_metadata::document_numerator::DocumentNumeratorChanges,
) -> Result<String, String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let properties = properties(&document, name)?;
    let mut replacements = Vec::new();
    for (tag, value) in changes.values() {
        let observed = scalar(properties, tag)?;
        let equivalent = if tag == "NumberLength" {
            observed.parse::<u32>().ok() == value.parse::<u32>().ok()
        } else {
            observed == value
        };
        if equivalent {
            continue;
        }
        // The strict scalar proof above identifies this direct element. Only
        // its text changes; prefixes, unknown siblings and outer bytes stay.
        let node = properties
            .children()
            .find(|n| n.has_tag_name((MD, tag)))
            .expect("validated direct scalar");
        let range = node.range();
        let element = &source[range.clone()];
        let start = element.find('>').ok_or("scalar opening tag is absent")? + range.start + 1;
        let end = element.rfind("</").ok_or("scalar closing tag is absent")? + range.start;
        replacements.push((start..end, value));
    }
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut text = source.to_string();
    for (range, value) in replacements {
        text.replace_range(range, &value);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_metadata::document_numerator::DocumentNumeratorChanges;

    const IMAGE: &str = concat!(
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><DocumentNumerator uuid=\"16760000-0000-4000-8000-000000000001\">",
        "<Future><Properties><NumberType>do not touch</NumberType></Properties></Future>",
        "<Properties><Name>Sales</Name><NumberType>String</NumberType><NumberLength>00051</NumberLength>",
        "<NumberAllowedLength>Fixed</NumberAllowedLength><NumberPeriodicity>Year</NumberPeriodicity><CheckUnique>true</CheckUnique>",
        "</Properties></DocumentNumerator></MetaDataObject>"
    );

    #[test]
    fn numerator_patch_uses_the_direct_owner_and_preserves_equivalent_integer_bytes() {
        let unchanged = patch(
            IMAGE,
            "Sales",
            &DocumentNumeratorChanges {
                number_length: Some(51),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(unchanged, IMAGE);
        let changed = patch(
            IMAGE,
            "Sales",
            &DocumentNumeratorChanges {
                number_type: Some("Number".into()),
                number_length: Some(38),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            changed,
            IMAGE
                .replace(
                    "<NumberType>String</NumberType>",
                    "<NumberType>Number</NumberType>"
                )
                .replace(
                    "<NumberLength>00051</NumberLength>",
                    "<NumberLength>38</NumberLength>"
                )
        );
        validate(changed.as_bytes(), "Sales").unwrap();
        assert!(validate(
            IMAGE
                .replace(
                    "<NumberType>String</NumberType>",
                    "<NumberType>Number</NumberType>"
                )
                .as_bytes(),
            "Sales"
        )
        .is_err());
    }

    #[test]
    fn numerator_write_rejects_incomplete_foreign_duplicate_and_nested_scalar_images() {
        for (old, new) in [
            ("<CheckUnique>true</CheckUnique>", ""),
            ("<NumberType>String</NumberType>", "<NumberType xmlns=\"urn:foreign\">String</NumberType>"),
            ("<NumberLength>00051</NumberLength>", "<NumberLength>1</NumberLength><NumberLength>2</NumberLength>"),
            ("<NumberPeriodicity>Year</NumberPeriodicity>", "<NumberPeriodicity><Value>Year</Value></NumberPeriodicity>"),
            ("<NumberAllowedLength>Fixed</NumberAllowedLength>", "<NumberAllowedLength>Unknown</NumberAllowedLength>"),
            ("<Name>Sales</Name>", "<Name>Another</Name>"),
            ("<CheckUnique>true</CheckUnique>", "<CheckUnique xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\">true</CheckUnique>"),
        ] {
            let source = IMAGE.replace(old, new);
            assert!(patch(&source, "Sales", &DocumentNumeratorChanges { check_unique: Some(false), ..Default::default() }).is_err(), "{source}");
        }
    }
}
