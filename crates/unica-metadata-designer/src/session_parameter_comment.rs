//! One local read fact without extending the metadata writer algebra.

use roxmltree::Document;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Observes Comment from the same retained bytes used to prove the owner.
/// The caller keeps that identity when this independent fact is unreadable.
pub fn read_comment(bytes: &[u8]) -> Result<Option<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("SessionParameter descriptor has the wrong root".to_string());
    }
    let objects = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [owner] = objects.as_slice() else {
        return Err("SessionParameter descriptor must contain one owner".to_string());
    };
    if !owner.has_tag_name((MD, "SessionParameter")) {
        return Err("SessionParameter owner is absent".to_string());
    }
    let containers = owner
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let properties = match containers.as_slice() {
        [properties] if properties.tag_name().namespace() == Some(MD) => *properties,
        _ => return Err("SessionParameter must contain one direct Properties container in the metadata namespace".to_string()),
    };
    crate::info_projection::observe_collection_item_comment(properties)
        .map_err(|error| error.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(properties: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><SessionParameter><Properties><Name>Probe</Name>{properties}</Properties></SessionParameter></MetaDataObject>")
    }

    #[test]
    fn comment_observation_keeps_absence_empty_and_all_xml_text_chunks() {
        for (raw, expected) in [
            ("", None),
            ("<Comment/>", Some("")),
            ("<Comment>  \n\t</Comment>", Some("  \n\t")),
            (
                "<Comment>Привет &amp; <!-- comment -->мир<![CDATA[!]]>\n</Comment>",
                Some("Привет & мир!\n"),
            ),
        ] {
            assert_eq!(
                read_comment(descriptor(raw).as_bytes()).unwrap().as_deref(),
                expected
            );
        }
    }

    #[test]
    fn comment_observation_rejects_damage_and_ambiguous_containers() {
        for raw in [
            "<Comment><Nested/></Comment>",
            "<Comment>One</Comment><Comment>Two</Comment>",
            "<Comment xmlns=\"urn:foreign\">Foreign</Comment>",
            "<Comment unknown=\"value\">Text</Comment>",
        ] {
            assert!(read_comment(descriptor(raw).as_bytes()).is_err());
        }
        let valid = descriptor("<Comment>Known</Comment>");
        for replacement in ["<Properties/>", "<Properties xmlns=\"urn:foreign\"/>"] {
            let damaged = valid.replace(
                "</SessionParameter>",
                &format!("{replacement}</SessionParameter>"),
            );
            assert!(read_comment(damaged.as_bytes()).is_err());
        }
    }
}
