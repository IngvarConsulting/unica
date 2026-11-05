//! Session parameter observation through the shared descriptor comment reader.

#[cfg(test)]
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Observes Comment from the same retained bytes used to prove the owner.
/// The caller keeps that identity when this independent fact is unreadable.
pub fn read_comment(bytes: &[u8]) -> Result<Option<String>, String> {
    crate::descriptor_comment::read_comment(bytes, "SessionParameter")
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
