//! Профиль набора XML-выгрузки по версии его корня (INV.SOURCE.WRITABLE-PROFILE).
//!
//! Допуск демона и проекция актора выбирают профиль одним способом, иначе
//! набор 2.21 расходится с актором, который его держит.

use std::path::Path;
use unica_source_contract::{SourceProfile, SourceSetKind};

// resource-limit: workspace.root-descriptor-prefix-bytes
// class: format
// unit: bytes
// scope: file
// value: 65536
// resource: Начало корня набора Configuration.xml, в котором ищется открывающий тег корня с атрибутом version.
// status: unproven
// origin: #2730
// problem: неизвестно
// on-reach: набор получает профиль 2.20; чтение сообщает о версии корня само
const ROOT_DESCRIPTOR_PREFIX_BYTES: usize = 64 * 1024;

/// Профиль набора по версии корня `Configuration.xml`: 2.20 или выпуск со
/// своими диалектами (2.21, 2.22). Набор, чей корень не прочитан или записан
/// в другой версии, получает профиль 2.20: чтение сообщает о версии само, а
/// правку отклоняет охрана формата.
pub fn platform_xml_source_profile(kind: SourceSetKind, root: &Path) -> SourceProfile {
    let fallback = SourceProfile::platform_xml_8_3_27_format_2_20();
    if !matches!(
        kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        return fallback;
    }
    crate::read::open_source_directory(root)
        .ok()
        .and_then(|directory| exact_platform_xml_source_profile(&directory))
        .unwrap_or(fallback)
}

/// Read the exact profile from the retained source root, without the
/// classification fallback used to keep read-only views available.
pub fn exact_platform_xml_source_profile(
    directory: &unica_source_contract::read::SourceDirectory,
) -> Option<SourceProfile> {
    exact_platform_xml_descriptor_profile(directory, Path::new("Configuration.xml"))
}

/// Profile proved by the literal root of a descriptor in the retained set.
pub fn exact_platform_xml_descriptor_profile(
    directory: &unica_source_contract::read::SourceDirectory,
    relative: &Path,
) -> Option<SourceProfile> {
    let prefix = directory
        .read_relative_regular_prefix(relative, ROOT_DESCRIPTOR_PREFIX_BYTES)
        .ok()?;
    SourceProfile::platform_xml_for_export_format(&root_version_literal(&prefix)?)
}

/// The `version` attribute of the root start tag as written, before entity
/// decoding; `None` when the prefix holds no complete root start tag.
fn root_version_literal(prefix: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(prefix);
    let mut rest = text.trim_start_matches('\u{feff}');
    // Skip the declaration, processing instructions, comments and doctype.
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("<?") {
            rest = &after[after.find("?>")? + 2..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->")? + 3..];
        } else if let Some(after) = rest.strip_prefix("<!") {
            rest = &after[after.find('>')? + 1..];
        } else {
            break;
        }
    }
    let mut tag = rest.strip_prefix('<')?;
    tag = tag.trim_start_matches(|c: char| !c.is_whitespace() && c != '>' && c != '/');
    loop {
        tag = tag.trim_start();
        if tag.starts_with('>') || tag.starts_with("/>") {
            return None;
        }
        let equals = tag.find('=')?;
        let name = tag[..equals].trim();
        let value = tag[equals + 1..].trim_start();
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let end = value[1..].find(quote)?;
        if name == "version" {
            return Some(value[1..1 + end].to_string());
        }
        tag = &value[end + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::{platform_xml_source_profile, root_version_literal};
    use unica_source_contract::{SourceProfile, SourceSetKind};

    #[test]
    fn the_root_version_is_read_from_the_root_start_tag_as_written() {
        for (text, expected) in [
            (r#"<MetaDataObject version="2.21"/>"#, Some("2.21")),
            (
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!-- c -->\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8='urn:x' version='2.22'>",
                Some("2.22"),
            ),
            (r#"<MetaDataObject version="2.&#50;0">"#, Some("2.&#50;0")),
            (r#"<MetaDataObject xmlns="urn:x">"#, None),
            (r#"<MetaDataObject version="2.2"#, None),
            ("not xml", None),
        ] {
            assert_eq!(
                root_version_literal(text.as_bytes()).as_deref(),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn a_source_set_takes_the_profile_of_its_root_version() {
        let temporary = tempfile::tempdir().unwrap();
        // A source root is physical: the retained read refuses a path through
        // a link, as macOS's /var -> /private/var temporary directory is.
        let root = crate::paths::normalize_path_identity(temporary.path()).unwrap();
        for (version, expected) in [
            ("2.20", "platform-xml-8.3.27-format-2.20"),
            ("2.21", "platform-xml-8.5.1-format-2.21"),
            ("2.22", "platform-xml-8.5.4-format-2.22"),
            ("2.23", "platform-xml-8.3.27-format-2.20"),
            ("2.21.0", "platform-xml-8.3.27-format-2.20"),
        ] {
            std::fs::write(
                root.as_path().join("Configuration.xml"),
                format!("\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"{version}\"><Configuration/></MetaDataObject>"),
            )
            .unwrap();
            for kind in [SourceSetKind::Configuration, SourceSetKind::Extension] {
                assert_eq!(
                    platform_xml_source_profile(kind, root.as_path()).canonical_id(),
                    expected,
                    "{version} {kind:?}"
                );
            }
        }
        std::fs::remove_file(root.as_path().join("Configuration.xml")).unwrap();
        assert_eq!(
            platform_xml_source_profile(SourceSetKind::Configuration, root.as_path()),
            SourceProfile::platform_xml_8_3_27_format_2_20()
        );
    }
}
