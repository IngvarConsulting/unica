use std::path::{Path, PathBuf};
use unica_address::source_target::MetadataAddress;
use unica_source_contract::SourceSetKind;

/// Раскладки видов объявлены в `unica_address::kinds`, по файлу на вид.
pub use unica_address::kinds::MetadataLayout;

/// Виды с раскладкой в порядке, в котором платформа пишет их в `ChildObjects`.
pub const METADATA_KINDS: &[MetadataLayout] = &unica_address::kinds::METADATA_LAYOUTS;

pub const METADATA_KIND_TAGS: &[&str] = &unica_address::kinds::METADATA_LAYOUT_TAGS;

pub fn metadata_kind(tag: &str) -> Option<&'static MetadataLayout> {
    METADATA_KINDS.iter().find(|kind| kind.tag == tag)
}

pub fn metadata_kind_by_directory(directory: &str) -> Option<&'static MetadataLayout> {
    METADATA_KINDS
        .iter()
        .find(|kind| kind.directory.eq_ignore_ascii_case(directory))
}

pub fn metadata_kind_index(tag: &str) -> Option<usize> {
    METADATA_KINDS.iter().position(|kind| kind.tag == tag)
}

/// Root kinds the platform exports that have no layout in the registry:
/// declared kinds without a layout and kinds Unica does not know. They are
/// never created; their place only keeps a new entry of another kind from
/// landing on the wrong side of them.
pub fn unlaid_root_kinds() -> impl Iterator<Item = &'static str> {
    unica_address::kinds::DECLARED_ROOT_KINDS
        .iter()
        .filter(|tag| metadata_kind(tag).is_none())
        .copied()
        .chain(
            unica_address::kinds::UNDECLARED_ROOT_KINDS
                .iter()
                .map(|(tag, _)| *tag),
        )
}

/// Place of a root kind in the configuration's `ChildObjects` as the platform
/// writes it: the registry kinds and the kinds without a layout between them.
pub fn root_child_objects_order(tag: &str) -> Option<usize> {
    unica_address::kinds::root_child_objects_place(tag).map(|place| place as usize)
}

/// Whether platform 8.3.27 requires a root `ChildObjects` container for this
/// physical metadata kind, including when the collection is empty.
///
/// `None` keeps an unknown future kind from being silently treated as
/// childless. This profile covers the complete physical registry used by
/// `cfe.borrow`, which is wider than the typed [`MetadataKind`] surface.
pub fn metadata_kind_requires_child_objects_8_3_27(tag: &str) -> Option<bool> {
    metadata_kind(tag).map(|kind| kind.child_objects_8_3_27)
}

pub fn attached_resource_relative(
    target: &MetadataAddress,
    resource: &str,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    if matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return external_attached_resource_relative(target, resource, source_set_kind);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, rest @ ..] = parts.as_slice() else {
        return Err("typed reader target has no owner identity".to_string());
    };
    let owner = metadata_kind(owner_kind)
        .ok_or_else(|| format!("typed reader owner kind `{owner_kind}` has no platform layout"))?;
    let mut relative = PathBuf::from(owner.directory);
    relative.push(owner_name);
    match rest {
        [] if (*owner_kind == "CommonForm" && resource == "Form.xml")
            || (*owner_kind == "CommonTemplate" && resource == "Template.xml")
            || (*owner_kind == "Role" && resource == "Rights.xml")
            || (*owner_kind == "Subsystem" && resource == "CommandInterface.xml")
            || (*owner_kind == "XDTOPackage" && resource == "Package.bin")
            || (*owner_kind == "WSReference" && resource == "WSDefinition.xml")
            || (*owner_kind == "ScheduledJob" && resource == "Schedule.xml") =>
        {
            relative.push("Ext");
            relative.push(resource);
        }
        [nested_kind, nested_name] => {
            let directory = match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(format!(
                        "unsupported attached resource owner `{nested_kind}`"
                    ))
                }
            };
            relative.push(directory);
            relative.push(nested_name);
            relative.push("Ext");
            relative.push(resource);
        }
        _ => return Err("typed reader target has an unsupported attached-resource depth".into()),
    }
    Ok(relative)
}

pub fn metadata_descriptor_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    if matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return external_metadata_descriptor_relative(target, source_set_kind);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, rest @ ..] = parts.as_slice() else {
        return Err("metadata descriptor target has no owner identity".into());
    };
    let owner = metadata_kind(owner_kind)
        .ok_or_else(|| format!("metadata kind `{owner_kind}` has no platform layout"))?;
    let mut relative = PathBuf::from(owner.directory);
    match rest {
        [] => relative.push(format!("{owner_name}.xml")),
        [nested_kind, nested_name] => {
            relative.push(owner_name);
            relative.push(match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(format!(
                        "unsupported nested descriptor kind `{nested_kind}`"
                    ))
                }
            });
            relative.push(format!("{nested_name}.xml"));
        }
        _ => return Err("metadata descriptor target has unsupported depth".into()),
    }
    Ok(relative)
}

pub fn external_owner_parts(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<(&str, &str, Vec<&str>), String> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [kind, name, rest @ ..] = parts.as_slice() else {
        return Err("external target has no canonical owner identity".into());
    };
    let expected = match source_set_kind {
        SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
        SourceSetKind::ExternalReport => "ExternalReport",
        SourceSetKind::Configuration | SourceSetKind::Extension => {
            return Err("external path requested for configuration source set".into())
        }
    };
    if *kind != expected {
        return Err(format!("external source set has no `{kind}` owner family"));
    }
    Ok((kind, name, rest.to_vec()))
}

pub fn external_metadata_descriptor_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    let (_, owner_name, rest) = external_owner_parts(target, source_set_kind)?;
    match rest.as_slice() {
        [] => Ok(PathBuf::from(format!("{owner_name}.xml"))),
        [nested_kind, nested_name] => {
            let directory = match *nested_kind {
                "Form" => "Forms",
                "Template" => "Templates",
                "Command" => "Commands",
                _ => {
                    return Err(format!(
                        "unsupported external nested descriptor kind `{nested_kind}`"
                    ))
                }
            };
            Ok(PathBuf::from(owner_name)
                .join(directory)
                .join(format!("{nested_name}.xml")))
        }
        _ => Err("external descriptor target has unsupported depth".into()),
    }
}

pub fn external_attached_resource_relative(
    target: &MetadataAddress,
    resource: &str,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    let (_, owner_name, rest) = external_owner_parts(target, source_set_kind)?;
    let [nested_kind, nested_name] = rest.as_slice() else {
        return Err("external attached resource requires a nested owner".into());
    };
    let directory = match *nested_kind {
        "Form" => "Forms",
        "Template" => "Templates",
        "Command" => "Commands",
        _ => {
            return Err(format!(
                "unsupported external attached resource owner `{nested_kind}`"
            ))
        }
    };
    Ok(PathBuf::from(owner_name)
        .join(directory)
        .join(nested_name)
        .join("Ext")
        .join(resource))
}

/// The namespace declarations of an MDClasses root in `export_format`.
pub fn full_md_namespace_declarations(export_format: &str) -> String {
    format_namespace_declarations(FULL_MD_NAMESPACE_DECLARATIONS, export_format)
}

const FULL_MD_NAMESPACE_DECLARATIONS: &str =
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"";

const PALETTE_DECLARATION: &str = "xmlns:pal=\"http://v8.1c.ru/8.1/data/ui/colors/palette\"";

/// `document` whose root declares the namespaces its export format declares
/// (see [`format_namespace_declarations`]).
pub fn declare_format_namespaces(mut document: String, export_format: &str) -> String {
    let Some((start, end)) = root_start_tag(&document) else {
        return document;
    };
    if let Some((at, text)) = palette_insertion(&document[start..end], export_format) {
        document.insert_str(start + at, &text);
    }
    document
}

/// `document` whose every `element` start tag that declares namespaces
/// declares those of its export format, as a root does (DCS settings carry
/// their own declarations inside a template).
pub fn declare_element_format_namespaces(
    mut document: String,
    element: &str,
    export_format: &str,
) -> String {
    let opening = format!("<{element}");
    let mut insertions = Vec::new();
    for (start, _) in document.match_indices(&opening) {
        let name_end = start + opening.len();
        if !document[name_end..].starts_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let Some(close) = document[name_end..].find('>') else {
            continue;
        };
        if let Some((at, text)) =
            palette_insertion(&document[name_end..name_end + close], export_format)
        {
            insertions.push((name_end + at, text));
        }
    }
    for (at, text) in insertions.into_iter().rev() {
        document.insert_str(at, &text);
    }
    document
}

/// Root namespace `declarations` as `export_format` writes them. From 2.21
/// the palette namespace `pal` joins the prefixed declarations in prefix
/// order, as the platform writes them, with the separator of the declaration
/// it precedes. Declarations that have it already, that have no prefixed
/// declaration, and those of an older format are returned as they are.
pub fn format_namespace_declarations(declarations: &str, export_format: &str) -> String {
    let mut text = declarations.to_owned();
    if let Some((at, insertion)) = palette_insertion(declarations, export_format) {
        text.insert_str(at, &insertion);
    }
    text
}

fn palette_insertion(tag: &str, export_format: &str) -> Option<(usize, String)> {
    if !crate::format_profile::declares_palette_namespace(export_format)
        || tag.contains("xmlns:pal=")
    {
        return None;
    }
    let declarations: Vec<(usize, &str)> = tag
        .match_indices("xmlns:")
        .map(|(at, _)| {
            let prefix = &tag[at + "xmlns:".len()..];
            (at, &prefix[..prefix.find('=').unwrap_or(prefix.len())])
        })
        .collect();
    let &(last, _) = declarations.last()?;
    match declarations.iter().find(|(_, prefix)| *prefix > "pal") {
        Some(&(at, _)) => {
            let separator = match &tag[tag[..at].trim_end().len()..at] {
                "" => " ",
                separator => separator,
            };
            Some((at, format!("{PALETTE_DECLARATION}{separator}")))
        }
        None => {
            let separator = match &tag[tag[..last].trim_end().len()..last] {
                "" => " ",
                separator => separator,
            };
            let value = &tag[last..];
            let quote_at = value.find(['"', '\''])?;
            let quote = &value[quote_at..=quote_at];
            let value_end = last + quote_at + 1 + value[quote_at + 1..].find(quote)? + 1;
            Some((value_end, format!("{separator}{PALETTE_DECLARATION}")))
        }
    }
}

/// The byte range of the root element's start tag, without `<` and `>`.
fn root_start_tag(document: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    loop {
        let open = from + document[from..].find('<')?;
        let next = document[open + 1..].chars().next()?;
        if next == '?' || next == '!' {
            from = open + 1 + document[open + 1..].find('>')? + 1;
            continue;
        }
        let close = open + document[open..].find('>')?;
        return Some((open + 1, close));
    }
}

pub fn template_add_object_type_folders() -> &'static [&'static str] {
    &[
        "Reports",
        "DataProcessors",
        "Documents",
        "Catalogs",
        "InformationRegisters",
        "AccumulationRegisters",
        "ChartsOfCharacteristicTypes",
        "ChartsOfAccounts",
        "ChartsOfCalculationTypes",
        "BusinessProcesses",
        "Tasks",
        "ExchangePlans",
    ]
}

pub struct RoleReadLayout {
    pub role_dir_name: String,
    pub metadata_path: PathBuf,
    pub configuration_path: PathBuf,
}
pub fn role_read_layout(rights_path: &Path) -> RoleReadLayout {
    let ext_dir = rights_path.parent().unwrap_or_else(|| Path::new(""));
    let role_dir = ext_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let roles_dir = role_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let role_dir_name = role_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let metadata_path = roles_dir.join(format!("{role_dir_name}.xml"));
    let configuration_path = roles_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("Configuration.xml");
    RoleReadLayout {
        role_dir_name,
        metadata_path,
        configuration_path,
    }
}

#[cfg(test)]
mod format_namespace_tests {
    use super::declare_format_namespaces;

    fn root_start_tag(document: &str) -> &str {
        let (start, end) = super::root_start_tag(document).expect("a root");
        &document[start..end]
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_root_declares_the_namespaces_of_its_export_format_as_the_corpus_does() {
        // One root of each kind the 2.21 corpus extends: MDClasses, form,
        // spreadsheet and graphical schema. The 2.20 root with the 2.21
        // version is what a writer of 2.21 starts from.
        for (older, newer) in [
            (
                include_str!("../../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/Catalogs/Валюты.xml"),
                include_str!("../../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/Catalogs/Валюты.xml"),
            ),
            (
                include_str!("../../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml"),
                include_str!("../../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml"),
            ),
            (
                include_str!("../../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/DataProcessors/Обработка/Templates/Макет/Ext/Template.xml"),
                include_str!("../../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/DataProcessors/Обработка/Templates/Макет/Ext/Template.xml"),
            ),
            (
                include_str!("../../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/BusinessProcesses/Approval/Ext/Flowchart.xml"),
                include_str!("../../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/BusinessProcesses/Approval/Ext/Flowchart.xml"),
            ),
        ] {
            let seed = older.replace("version=\"2.20\"", "version=\"2.21\"");
            let written = declare_format_namespaces(seed.clone(), "2.21");
            assert_eq!(root_start_tag(&written), root_start_tag(newer));
            assert_eq!(declare_format_namespaces(written.clone(), "2.22"), written);
            assert_eq!(declare_format_namespaces(seed.clone(), "2.20"), seed);
        }
    }

    #[test]
    fn the_palette_declaration_keeps_the_separator_of_its_neighbour() {
        let lines = "<?xml version=\"1.0\"?>\n<Root xmlns=\"urn:d\"\n    xmlns:lf=\"urn:lf\"\n    xmlns:style=\"urn:s\"\n    version=\"2.22\">";
        let written = declare_format_namespaces(lines.to_owned(), "2.22");
        assert!(
            written.contains("xmlns:lf=\"urn:lf\"\n    xmlns:pal=\"http://v8.1c.ru/8.1/data/ui/colors/palette\"\n    xmlns:style="),
            "{written}"
        );
        let last = declare_format_namespaces("<Root xmlns:a=\"urn:a\"/>".to_owned(), "2.21");
        assert_eq!(
            last,
            "<Root xmlns:a=\"urn:a\" xmlns:pal=\"http://v8.1c.ru/8.1/data/ui/colors/palette\"/>"
        );
        let single =
            declare_format_namespaces("<Root xmlns:a='urn:a'\n  x=\"1\"/>".to_owned(), "2.21");
        assert_eq!(
            single,
            "<Root xmlns:a='urn:a' xmlns:pal=\"http://v8.1c.ru/8.1/data/ui/colors/palette\"\n  x=\"1\"/>"
        );
        let first = super::format_namespace_declarations("xmlns:v8=\"urn:v8\"", "2.21");
        assert_eq!(
            first,
            "xmlns:pal=\"http://v8.1c.ru/8.1/data/ui/colors/palette\" xmlns:v8=\"urn:v8\""
        );
        let bare = "<Root version=\"2.21\"/>".to_owned();
        assert_eq!(declare_format_namespaces(bare.clone(), "2.21"), bare);
    }

    #[test]
    fn an_element_declares_the_palette_only_where_it_declares_prefixes() {
        let pal = "xmlns:pal=\"http://v8.1c.ru/8.1/data/ui/colors/palette\"";
        let schema = concat!(
            "<s xmlns:v8=\"urn:v8\">",
            "<dcsset:settingsVariant xmlns:style=\"urn:s\"/>",
            "<dcsset:settings xmlns:style=\"urn:s\" xmlns:win=\"urn:w\">",
            "<dcsset:settings/><dcsset:settings a=\"1\"/>",
            "</dcsset:settings>",
            "<dcsset:settings xmlns:sys=\"urn:y\"/>",
            "</s>"
        );
        let written =
            super::declare_element_format_namespaces(schema.to_owned(), "dcsset:settings", "2.21");
        assert_eq!(
            written,
            format!(
                concat!(
                    "<s xmlns:v8=\"urn:v8\">",
                    "<dcsset:settingsVariant xmlns:style=\"urn:s\"/>",
                    "<dcsset:settings {pal} xmlns:style=\"urn:s\" xmlns:win=\"urn:w\">",
                    "<dcsset:settings/><dcsset:settings a=\"1\"/>",
                    "</dcsset:settings>",
                    "<dcsset:settings {pal} xmlns:sys=\"urn:y\"/>",
                    "</s>"
                ),
                pal = pal
            )
        );
        assert_eq!(
            super::declare_element_format_namespaces(schema.to_owned(), "dcsset:settings", "2.20"),
            schema
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn registry_has_unique_canonical_tags_and_directories() {
        // Виды объявлены в `unica_address::kinds`, по файлу на вид: список здесь
        // не повторяется. Порядок раскладки сверяет с выгрузками платформы
        // `tests/platform/configuration_layout.rs`.
        for tag in [
            "Catalog",
            "Bot",
            "PaletteColor",
            "Queue",
            "IntegrationService",
        ] {
            assert!(METADATA_KIND_TAGS.contains(&tag), "{tag} has no layout");
        }
        assert_eq!(METADATA_KIND_TAGS.len(), METADATA_KINDS.len());
        assert_eq!(
            METADATA_KIND_TAGS,
            METADATA_KINDS
                .iter()
                .map(|kind| kind.tag)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            METADATA_KINDS
                .iter()
                .map(|kind| kind.tag)
                .collect::<HashSet<_>>()
                .len(),
            METADATA_KINDS.len()
        );
        assert_eq!(
            METADATA_KINDS
                .iter()
                .map(|kind| kind.directory.to_ascii_lowercase())
                .collect::<HashSet<_>>()
                .len(),
            METADATA_KINDS.len()
        );
    }

    #[test]
    fn every_physical_kind_has_a_child_objects_profile() {
        for kind in METADATA_KINDS {
            assert!(
                metadata_kind_requires_child_objects_8_3_27(kind.tag).is_some(),
                "{} has no 8.3.27 ChildObjects profile",
                kind.tag
            );
        }
    }

    #[test]
    fn bot_registry_entry_models_known_platform_facts() {
        let bot = metadata_kind("Bot").expect("Bot must be registered");
        assert_eq!(bot.directory, "Bots");
        assert_eq!(bot.display_name_ru, "Боты");
        assert_eq!(bot.config_dump_prefix, Some("Bot"));
        assert_eq!(bot.config_dump_module_suffix, Some(".Module"));
        assert_eq!(
            metadata_kind_index("Bot"),
            metadata_kind_index("DefinedType").map(|index| index + 1)
        );
        assert_eq!(metadata_kind("SyntheticMetadata"), None);
    }
}
