use super::super::common::read_utf8_sig;
use super::validation::{
    document_registers, meta_validate_valid_types, subsystem_shows_in_command_interface,
};
use super::xml_model::parse_metadata_image;
use super::{meta_info_child, meta_info_inner_text};
use std::fs;
use std::path::{Path, PathBuf};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Предельная вложенность подсистем, которую обходят оба пути чтения состава.
/// Владелец бюджета один, чтобы обход по каталогу и типизированный сбор
/// доказательств не расходились в том, что считают доказанным.
pub(crate) const SUBSYSTEM_SCAN_MAX_NESTING: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaValidationImageIdentity {
    pub(crate) object_type: String,
    pub(crate) object_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaValidationRegistrationImage {
    pub(crate) registrations: Vec<(String, String)>,
    pub(crate) registered_languages: Vec<String>,
}

pub(crate) fn inspect_metadata_image_identity(
    bytes: &[u8],
) -> Result<MetaValidationImageIdentity, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err("image is not an MDClasses MetaDataObject".to_string());
    }
    let artifacts = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        .collect::<Vec<_>>();
    let [artifact] = artifacts.as_slice() else {
        return Err("image must contain exactly one metadata descriptor".to_string());
    };
    let object_type = artifact.tag_name().name();
    if !meta_validate_valid_types().contains(&object_type) {
        return Err(format!("unrecognized metadata type: {object_type}"));
    }
    let object_name = meta_info_child(*artifact, "Properties")
        .and_then(|properties| meta_info_child(properties, "Name"))
        .map(meta_info_inner_text)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{object_type} Name is missing"))?;
    Ok(MetaValidationImageIdentity {
        object_type: object_type.to_string(),
        object_name,
    })
}

pub(crate) fn inspect_metadata_registration_image(
    bytes: &[u8],
) -> Result<MetaValidationRegistrationImage, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err("registration image is not an MDClasses MetaDataObject".to_string());
    }
    let artifacts = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        .collect::<Vec<_>>();
    let [configuration] = artifacts.as_slice() else {
        return Err("registration image must contain exactly one Configuration".to_string());
    };
    if configuration.tag_name().name() != "Configuration" {
        return Err("registration image does not contain Configuration".to_string());
    }

    let mut registrations = Vec::new();
    let mut registered_languages = Vec::new();
    if let Some(children) = meta_info_child(*configuration, "ChildObjects") {
        for child in children.children().filter(roxmltree::Node::is_element) {
            if child.tag_name().namespace() != Some(MD_CLASSES_NS) {
                continue;
            }
            let object_type = child.tag_name().name();
            let object_name = meta_info_inner_text(child).trim().to_string();
            if object_name.is_empty() {
                continue;
            }
            if object_type == "Language" {
                registered_languages.push(object_name);
            } else {
                registrations.push((object_type.to_string(), object_name));
            }
        }
    }
    Ok(MetaValidationRegistrationImage {
        registrations,
        registered_languages,
    })
}

pub(crate) fn inspect_metadata_language_image(
    bytes: &[u8],
) -> Result<Option<(String, String)>, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let root = document.root_element();
    let language = root.children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "Language"
    });
    let Some(language) = language else {
        return Ok(None);
    };
    let properties = meta_info_child(language, "Properties");
    let name = properties
        .and_then(|properties| meta_info_child(properties, "Name"))
        .map(meta_info_inner_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Language Name is missing".to_string())?;
    let code = properties
        .and_then(|properties| meta_info_child(properties, "LanguageCode"))
        .map(meta_info_inner_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Language {name} has empty LanguageCode"))?;
    Ok(Some((name, code)))
}

pub(crate) fn meta_validate_types_with_list_presentation() -> &'static [&'static str] {
    &[
        "ExchangePlan",
        "Catalog",
        "Document",
        "DocumentJournal",
        "Enum",
        "ChartOfCharacteristicTypes",
        "ChartOfAccounts",
        "ChartOfCalculationTypes",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "BusinessProcess",
        "Task",
    ]
}

pub(crate) fn meta_validate_registrar_document_scan(
    documents_dir: &Path,
    register_reference: &str,
) -> Result<(Vec<PathBuf>, bool), String> {
    let mut entries = fs::read_dir(documents_dir)
        .map_err(|error| format!("failed to read {}: {error}", documents_dir.display()))?;
    let mut entries = entries
        .by_ref()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to inspect a registrar candidate in {}: {error}",
                documents_dir.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut dependencies = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("xml")
            || !path.is_file()
        {
            continue;
        }
        dependencies.push(path.clone());
        let content = read_utf8_sig(&path)?;
        if document_registers(content.as_bytes(), register_reference) {
            return Ok((dependencies, true));
        }
    }
    Ok((dependencies, false))
}

pub(crate) fn meta_validate_subsystem_command_interface_scan(
    subsystems_dir: &Path,
    object_reference: &str,
) -> Result<(Vec<PathBuf>, bool), String> {
    let mut dependencies = Vec::new();
    let found = subsystem_command_interface_scan(
        subsystems_dir,
        object_reference,
        &mut dependencies,
        SUBSYSTEM_SCAN_MAX_NESTING,
    )?;
    Ok((dependencies, found))
}

fn subsystem_command_interface_scan(
    subsystems_dir: &Path,
    object_reference: &str,
    dependencies: &mut Vec<PathBuf>,
    remaining_nesting: usize,
) -> Result<bool, String> {
    let mut entries = fs::read_dir(subsystems_dir)
        .map_err(|error| format!("failed to read {}: {error}", subsystems_dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to inspect a subsystem candidate in {}: {error}",
                subsystems_dir.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut nested = Vec::new();
    for entry in entries {
        let path = entry.path();
        // The entry type comes from the directory read, so a symlink is never
        // followed: a subsystem tree holds no links, and following one could
        // leave the scanned tree.
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            // Subsystems nest as Subsystems/<Parent>/Subsystems/<Child>.xml.
            let child_dir = path.join("Subsystems");
            if child_dir.is_dir() {
                nested.push(child_dir);
            }
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("xml") {
            continue;
        }
        dependencies.push(path.clone());
        let content = read_utf8_sig(&path)?;
        if subsystem_shows_in_command_interface(content.as_bytes(), object_reference) {
            return Ok(true);
        }
    }
    if !nested.is_empty() && remaining_nesting == 0 {
        // Absence cannot be proved past the budget, so the scan says so instead
        // of reporting a listing it never reached.
        return Err(format!(
            "subsystem nesting under {} exceeds {SUBSYSTEM_SCAN_MAX_NESTING} levels",
            subsystems_dir.display()
        ));
    }
    for child_dir in nested {
        if subsystem_command_interface_scan(
            &child_dir,
            object_reference,
            dependencies,
            remaining_nesting - 1,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_documents(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-registrar-scan-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn registrar_scan_matches_only_register_records_items() {
        let root = temp_documents("typed-path");
        let reference = "InformationRegister.Ledger";
        fs::write(
            root.join("CommentOnly.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Document><Properties><Name>CommentOnly</Name><Comment>{reference}</Comment><RegisterRecords/></Properties></Document></MetaDataObject>"#
            ),
        )
        .unwrap();

        let (_, found) = meta_validate_registrar_document_scan(&root, reference).unwrap();
        assert!(
            !found,
            "an unrelated property must not count as registrar evidence"
        );

        fs::write(
            root.join("Registrar.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Document><Properties><Name>Registrar</Name><RegisterRecords><Item>{reference}</Item></RegisterRecords></Properties></Document></MetaDataObject>"#
            ),
        )
        .unwrap();
        let (_, found) = meta_validate_registrar_document_scan(&root, reference).unwrap();
        assert!(found);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registrar_scan_propagates_unreadable_document_evidence() {
        let root = temp_documents("invalid-utf8");
        fs::write(root.join("Broken.xml"), [0xff, 0xfe]).unwrap();

        let error = meta_validate_registrar_document_scan(&root, "InformationRegister.Ledger")
            .expect_err("unreadable registrar evidence must stay unavailable");

        assert!(error.contains("UTF-8"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_subsystems(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-subsystem-scan-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_subsystem(dir: &Path, name: &str, include: Option<&str>, content: &str) {
        fs::create_dir_all(dir).unwrap();
        let include = include
            .map(|value| format!("<IncludeInCommandInterface>{value}</IncludeInCommandInterface>"))
            .unwrap_or_default();
        fs::write(
            dir.join(format!("{name}.xml")),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><Subsystem><Properties><Name>{name}</Name>{include}<Content>{content}</Content></Properties></Subsystem></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn content_item(reference: &str) -> String {
        format!(r#"<xr:Item xsi:type="xr:MDObjectRef">{reference}</xr:Item>"#)
    }

    #[test]
    fn subsystem_scan_treats_an_absent_include_flag_as_the_platform_default() {
        let root = temp_subsystems("default-include");
        let reference = "InformationRegister.Ledger";
        write_subsystem(&root, "Sales", None, &content_item(reference));

        let (_, found) = meta_validate_subsystem_command_interface_scan(&root, reference).unwrap();

        assert!(
            found,
            "an absent IncludeInCommandInterface means the platform default: included"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn subsystem_scan_skips_a_subsystem_excluded_from_the_command_interface() {
        let root = temp_subsystems("excluded");
        let reference = "InformationRegister.Ledger";
        write_subsystem(&root, "Service", Some("false"), &content_item(reference));

        let (_, found) = meta_validate_subsystem_command_interface_scan(&root, reference).unwrap();

        assert!(!found, "an excluded subsystem cannot show the register");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn subsystem_scan_reaches_a_nested_subsystem_and_ignores_ext() {
        let root = temp_subsystems("nested");
        let reference = "InformationRegister.Ledger";
        write_subsystem(&root, "Parent", Some("true"), "");
        // Ext holds command interface files, not subsystem descriptors.
        fs::create_dir_all(root.join("Parent/Ext")).unwrap();
        fs::write(root.join("Parent/Ext/CommandInterface.xml"), "<broken").unwrap();
        write_subsystem(
            &root.join("Parent/Subsystems"),
            "Child",
            Some("true"),
            &content_item(reference),
        );

        let (_, found) = meta_validate_subsystem_command_interface_scan(&root, reference).unwrap();

        assert!(found, "the scan must recurse into nested subsystems");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn subsystem_scan_does_not_follow_a_symlinked_directory() {
        let root = temp_subsystems("symlink-loop");
        write_subsystem(&root, "Sales", Some("true"), "");
        fs::create_dir_all(root.join("Sales/Subsystems")).unwrap();
        // Sales/Subsystems/Loop -> .. resolves back to Sales, so a scan that
        // followed the link would keep re-entering the same directory. Windows
        // agents without the privilege report None instead of failing.
        let Some(symlink_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                "..",
                root.join("Sales/Subsystems/Loop"),
            )
        else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        symlink_result.unwrap();

        let (visited, found) =
            meta_validate_subsystem_command_interface_scan(&root, "InformationRegister.Ledger")
                .unwrap();

        assert!(!found, "a symlinked loop holds no listing");
        assert!(
            visited
                .iter()
                .all(|path| !path.to_string_lossy().contains("Loop")),
            "{visited:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn nest_subsystems(root: &Path, levels: usize, reference: &str) {
        let mut dir = root.to_path_buf();
        for level in 0..levels {
            write_subsystem(&dir, &format!("S{level}"), Some("true"), "");
            dir = dir.join(format!("S{level}")).join("Subsystems");
        }
        fs::create_dir_all(&dir).unwrap();
        write_subsystem(&dir, "Deepest", Some("true"), &content_item(reference));
    }

    #[test]
    fn subsystem_scan_reaches_the_deepest_level_within_the_budget() {
        let root = temp_subsystems("within-budget");
        let reference = "InformationRegister.Ledger";
        nest_subsystems(&root, SUBSYSTEM_SCAN_MAX_NESTING, reference);

        let (_, found) = meta_validate_subsystem_command_interface_scan(&root, reference).unwrap();

        assert!(found, "the budget must cover its own nesting depth");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn subsystem_scan_refuses_to_prove_absence_past_the_nesting_budget() {
        let root = temp_subsystems("past-budget");
        let reference = "InformationRegister.Ledger";
        nest_subsystems(&root, SUBSYSTEM_SCAN_MAX_NESTING + 1, reference);

        let error = meta_validate_subsystem_command_interface_scan(&root, reference)
            .expect_err("a tree past the budget is not scannable to the bottom");

        assert!(error.contains("exceeds"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn subsystem_scan_does_not_match_an_unrelated_content_item() {
        let root = temp_subsystems("unrelated");
        write_subsystem(
            &root,
            "Sales",
            Some("true"),
            &content_item("Catalog.Products"),
        );

        let (_, found) =
            meta_validate_subsystem_command_interface_scan(&root, "InformationRegister.Ledger")
                .unwrap();

        assert!(!found, "an unrelated content item must not count");
        fs::remove_dir_all(root).unwrap();
    }
}
