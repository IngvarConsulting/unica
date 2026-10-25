//! Existing configuration Designer algorithms; publication stays with trusted composition.
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use unica_metadata::configuration::*;
use unica_metadata::support_state::ConfigurationSupportData;
use unica_metadata::MetadataKind;
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_designer::configuration_layout::*;
use unica_source_designer::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use unica_source_designer::layout::metadata_kind;
use unica_source_designer::read::{path_exists, path_is_dir, read_string, read_utf8_sig};
use unica_source_designer::roots::root_version_literal;
pub const CF_HP_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

pub const CF_CAI_NS: &str = "http://v8.1c.ru/8.2/managed-application/core";

pub const CF_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";

pub const CF_XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

pub const CF_MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

pub fn cf_validate_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> String {
    node.children()
        .find(|child| unica_xml_format::names::element_has_name(*child, local_name, namespace))
        .and_then(|child| child.text())
        .unwrap_or("")
        .to_string()
}

pub fn cf_validate_form_ref(
    reader: &dyn ProjectDiscoveryRead,
    config_dir: &Path,
    form_ref: &str,
) -> bool {
    if form_ref.is_empty() || cf_validate_guid(form_ref) {
        return true;
    }
    let parts = form_ref.split('.').collect::<Vec<_>>();
    if parts.len() == 2 && parts[0] == "CommonForm" {
        let direct = config_dir
            .join("CommonForms")
            .join(parts[1])
            .join("Form.xml");
        let ext = config_dir
            .join("CommonForms")
            .join(parts[1])
            .join("Ext")
            .join("Form.xml");
        return reader.path_is_file(&direct) || reader.path_is_file(&ext);
    }
    if parts.len() == 4 && parts[2] == "Form" {
        if let Some(dir_name) = cf_validate_child_type_dir(parts[0]) {
            let direct = config_dir
                .join(dir_name)
                .join(parts[1])
                .join("Forms")
                .join(parts[3])
                .join("Form.xml");
            let ext = config_dir
                .join(dir_name)
                .join(parts[1])
                .join("Forms")
                .join(parts[3])
                .join("Ext")
                .join("Form.xml");
            return reader.path_is_file(&direct) || reader.path_is_file(&ext);
        }
    }
    false
}

pub fn optional(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

pub fn cf_home_page_item_data(items: &[CfHomePageItem]) -> Vec<CfHomePageItemData> {
    items
        .iter()
        .map(|item| CfHomePageItemData {
            form: item.form.clone(),
            height: item.height,
            common: item.common,
            roles: item
                .roles
                .iter()
                .map(|(role, visible)| CfHomePageRoleData {
                    role: role.clone(),
                    visible: *visible,
                })
                .collect(),
        })
        .collect()
}

/// Разбирает корневой `Ext/CommandInterface.xml`.
///
/// Документ несёт единственную секцию — `SubsystemsOrder`, список ссылок вида
/// `Subsystem.Планирование` в порядке, заданном разработчиком. Порядок —
/// содержательный факт: он задаёт, как разделы встают в интерфейсе.
pub fn parse_cf_command_interface_xml(text: &str) -> Result<CfInterfaceData, String> {
    const CI_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("CommandInterface XML parse error: {err}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "CommandInterface" {
        return Err(
            "[ERROR] Not a command-interface XML file (no CommandInterface root)".to_string(),
        );
    }
    let subsystem_order = root
        .children()
        .filter(|node| node.tag_name().name() == "SubsystemsOrder")
        .flat_map(|section| section.children())
        .filter(|node| {
            node.tag_name().name() == "Subsystem"
                && node.tag_name().namespace().is_none_or(|ns| ns == CI_NS)
        })
        .filter_map(|node| node.text())
        .map(|text| text.trim().to_string())
        .filter(|reference| !reference.is_empty())
        .collect();
    Ok(CfInterfaceData { subsystem_order })
}

pub fn parse_cf_info_xml(
    text: &str,
    support: ConfigurationSupportData,
    home_page: Option<CfHomePageData>,
    interface: Option<CfInterfaceData>,
    panels: Option<serde_json::Value>,
) -> Result<ParsedCfInfo, String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("Configuration XML parse error: {err}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(
            "[ERROR] Not a valid 1C metadata XML file (no MetaDataObject root)".to_string(),
        );
    }
    let Some(cfg) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "Configuration", Some(MD_NS))
    }) else {
        return Err("[ERROR] No <Configuration> element found".to_string());
    };
    let Some(props) = cfg
        .children()
        .find(|node| unica_xml_format::names::element_has_name(*node, "Properties", Some(MD_NS)))
    else {
        return Err("[ERROR] No <Configuration>/<Properties> element found".to_string());
    };

    let version = root.attribute("version").unwrap_or("");
    let extension_purpose = optional(cf_prop_text(props, "ConfigurationExtensionPurpose"));
    let counts = cf_child_object_counts(cfg);
    let total_objects = counts.iter().map(|(_, count)| *count).sum::<usize>();
    let registered_objects = cfg
        .children()
        .find(|node| unica_xml_format::names::element_has_name(*node, "ChildObjects", Some(MD_NS)))
        .into_iter()
        .flat_map(|children| children.children().filter(|node| node.is_element()))
        .filter_map(|child| {
            child
                .text()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| CfRegisteredObject {
                    kind: child.tag_name().name().to_string(),
                    name: name.to_string(),
                })
        })
        .collect();
    let data = CfInfoData {
        format: version.to_string(),
        name: cf_prop_text(props, "Name"),
        synonym: optional(cf_prop_ml(props, "Synonym")),
        version: optional(cf_prop_text(props, "Version")),
        vendor: optional(cf_prop_text(props, "Vendor")),
        support,
        extension_purpose,
        properties: CfInfoProperties {
            compatibility_mode: optional(cf_prop_text(props, "CompatibilityMode")),
            default_run_mode: optional(cf_prop_text(props, "DefaultRunMode")),
            script_variant: optional(cf_prop_text(props, "ScriptVariant")),
            default_language: optional(cf_prop_text(props, "DefaultLanguage")),
            data_lock_control_mode: optional(cf_prop_text(props, "DataLockControlMode")),
            modality_use_mode: optional(cf_prop_text(props, "ModalityUseMode")),
            interface_compatibility_mode: optional(cf_prop_text(
                props,
                "InterfaceCompatibilityMode",
            )),
            extension_compatibility_mode: optional(cf_prop_text(
                props,
                "ConfigurationExtensionCompatibilityMode",
            )),
            object_autonumeration_mode: optional(cf_prop_text(props, "ObjectAutonumerationMode")),
            synchronous_call_use_mode: optional(cf_prop_text(
                props,
                "SynchronousPlatformExtensionAndAddInCallUseMode",
            )),
            database_tablespaces_use_mode: optional(cf_prop_text(
                props,
                "DatabaseTablespacesUseMode",
            )),
            main_window_mode: optional(cf_prop_text(props, "MainClientApplicationWindowMode")),
            comment: optional(cf_prop_text(props, "Comment")),
            name_prefix: optional(cf_prop_text(props, "NamePrefix")),
            update_catalog_address: optional(cf_prop_text(props, "UpdateCatalogAddress")),
        },
        child_objects: counts
            .into_iter()
            .map(|(kind, count)| CfChildObjectCount { kind, count })
            .collect(),
        total_objects,
        home_page,
        interface,
        panels,
    };
    Ok(ParsedCfInfo {
        data,
        registered_objects,
    })
}

pub fn cf_prop_text(props: roxmltree::Node<'_, '_>, local_name: &str) -> String {
    unica_xml_format::read::child_text(props, local_name, Some("http://v8.1c.ru/8.3/MDClasses"))
}

pub fn cf_prop_ml(props: roxmltree::Node<'_, '_>, local_name: &str) -> String {
    props
        .children()
        .find(|node| {
            unica_xml_format::names::element_has_name(
                *node,
                local_name,
                Some("http://v8.1c.ru/8.3/MDClasses"),
            )
        })
        .map(unica_xml_format::read::multilang_text)
        .unwrap_or_default()
}

pub fn cf_child_object_counts(cfg: roxmltree::Node<'_, '_>) -> Vec<(String, usize)> {
    let mut counts = Vec::<(String, usize)>::new();
    if let Some(child_objects) = cfg.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "ChildObjects",
            Some("http://v8.1c.ru/8.3/MDClasses"),
        )
    }) {
        for child in child_objects.children().filter(|node| node.is_element()) {
            let type_name = child.tag_name().name().to_string();
            if let Some((_, count)) = counts.iter_mut().find(|(name, _)| name == &type_name) {
                *count += 1;
            } else {
                counts.push((type_name, 1));
            }
        }
    }
    counts
}

pub fn cf_append_counts(lines: &mut Vec<String>, counts: &[(String, usize)], total_objects: usize) {
    lines.push(format!("--- Состав ({total_objects} объектов) ---"));
    lines.push(String::new());
    let max_type_len = cf_type_order()
        .iter()
        .filter(|type_name| counts.iter().any(|(name, _)| name == *type_name))
        .map(|type_name| cf_type_ru_name(type_name).chars().count())
        .max()
        .unwrap_or(0)
        .max(10);
    for type_name in cf_type_order() {
        if let Some((_, count)) = counts.iter().find(|(name, _)| name == type_name) {
            let ru_name = cf_type_ru_name(type_name);
            lines.push(format!("  {ru_name:<max_type_len$}  {count}"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn cf_append_full_info(
    reader: &dyn ProjectDiscoveryRead,
    lines: &mut Vec<String>,
    cfg: roxmltree::Node<'_, '_>,
    props: roxmltree::Node<'_, '_>,
    version: &str,
    cfg_name: &str,
    cfg_synonym: &str,
    cfg_version: &str,
    cfg_vendor: &str,
    cfg_compat: &str,
    cfg_ext_compat: &str,
    cfg_default_run: &str,
    cfg_script: &str,
    cfg_default_lang: &str,
    cfg_data_lock: &str,
    cfg_modality: &str,
    cfg_intf_compat: &str,
    cfg_auto_num: &str,
    cfg_sync_calls: &str,
    cfg_db_spaces: &str,
    cfg_window_mode: &str,
    cfg_comment: &str,
    cfg_prefix: &str,
    cfg_update_addr: &str,
    config_dir: &Path,
    support_lines: &[String],
    counts: &[(String, usize)],
    total_objects: usize,
) {
    let syn_part = if cfg_synonym.is_empty() {
        String::new()
    } else {
        format!(" — \"{cfg_synonym}\"")
    };
    let ver_part = if cfg_version.is_empty() {
        String::new()
    } else {
        format!(" v{cfg_version}")
    };
    lines.push(format!(
        "=== Конфигурация: {cfg_name}{syn_part}{ver_part} ==="
    ));
    lines.push(String::new());
    lines.push("--- Идентификация ---".to_string());
    lines.push(format!(
        "UUID:           {}",
        cfg.attribute("uuid").unwrap_or("")
    ));
    lines.push(format!("Имя:            {cfg_name}"));
    if !cfg_synonym.is_empty() {
        lines.push(format!("Синоним:        {cfg_synonym}"));
    }
    if !cfg_comment.is_empty() {
        lines.push(format!("Комментарий:    {cfg_comment}"));
    }
    if !cfg_prefix.is_empty() {
        lines.push(format!("Префикс:        {cfg_prefix}"));
    }
    if !cfg_vendor.is_empty() {
        lines.push(format!("Поставщик:      {cfg_vendor}"));
    }
    if !cfg_version.is_empty() {
        lines.push(format!("Версия:         {cfg_version}"));
    }
    lines.extend(support_lines.iter().cloned());
    if !cfg_update_addr.is_empty() {
        lines.push(format!("Каталог обн.:   {cfg_update_addr}"));
    }
    lines.push(String::new());
    lines.push("--- Режимы работы ---".to_string());
    lines.push(format!("Формат:              {version}"));
    lines.push(format!("Совместимость:       {cfg_compat}"));
    lines.push(format!("Совм. расширений:    {cfg_ext_compat}"));
    lines.push(format!("Режим запуска:       {cfg_default_run}"));
    lines.push(format!("Язык скриптов:       {cfg_script}"));
    lines.push(format!("Блокировки:          {cfg_data_lock}"));
    lines.push(format!("Автонумерация:       {cfg_auto_num}"));
    lines.push(format!("Модальность:         {cfg_modality}"));
    lines.push(format!("Синхр. вызовы:       {cfg_sync_calls}"));
    lines.push(format!("Интерфейс:           {cfg_intf_compat}"));
    lines.push(format!("Табл. пространства:  {cfg_db_spaces}"));
    lines.push(format!("Режим окна:          {cfg_window_mode}"));
    lines.push(String::new());
    lines.push("--- Назначение ---".to_string());
    lines.push(format!("Язык по умолч.:  {cfg_default_lang}"));
    cf_append_full_purpose_info(lines, props);
    cf_append_full_panel_layout(reader, lines, config_dir);
    cf_append_full_home_page_summary(reader, lines, config_dir);
    cf_append_full_storages_and_forms(lines, props);
    cf_append_full_multilang_info(lines, props);
    cf_append_full_mobile_functionalities(lines, props);
    cf_append_full_internal_info(lines, cfg);
    cf_append_full_child_objects(lines, cfg, counts, total_objects);
}

pub fn cf_append_full_purpose_info(lines: &mut Vec<String>, props: roxmltree::Node<'_, '_>) {
    if let Some(purpose_node) = props.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "UsePurposes", Some(CF_MD_NS))
    }) {
        let purposes = purpose_node
            .children()
            .filter(|node| {
                unica_xml_format::names::element_has_name(*node, "Value", Some(CF_V8_NS))
            })
            .filter_map(|node| node.text())
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !purposes.is_empty() {
            lines.push(format!("Назначения:      {}", purposes.join(", ")));
        }
    }

    if let Some(roles_node) = props.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "DefaultRoles", Some(CF_MD_NS))
    }) {
        let roles = roles_node
            .children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, "Item", Some(CF_XR_NS)))
            .filter_map(|node| node.text())
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !roles.is_empty() {
            lines.push(format!("Роли по умолч.:  {}", roles.len()));
            for role in roles {
                lines.push(format!("  - {role}"));
            }
        }
    }

    lines.push(format!(
        "Управл.формы в обычн.: {}",
        cf_prop_text(props, "UseManagedFormInOrdinaryApplication")
    ));
    lines.push(format!(
        "Обычн.формы в управл.: {}",
        cf_prop_text(props, "UseOrdinaryFormInManagedApplication")
    ));
    lines.push(String::new());
}

pub fn cf_panel_name(uuid: &str) -> String {
    match uuid {
        "cbab57f2-a0f3-4f0a-89ea-4cb19570ab75" => "Открытых".to_string(),
        "b553047f-c9aa-4157-978d-448ecad24248" => "Разделов".to_string(),
        "13322b22-3960-4d68-93a6-fe2dd7f28ca3" => "Избранного".to_string(),
        "c933ac92-92cd-459d-81cc-e0c8a83ced99" => "История".to_string(),
        "b2735bd3-d822-4430-ba59-c9e869693b24" => "Функций".to_string(),
        other => format!("?{other}"),
    }
}

pub fn cf_read_panel_layout(
    reader: &dyn ProjectDiscoveryRead,
    config_dir: &Path,
) -> Option<CfPanelLayout> {
    let path = config_dir
        .join("Ext")
        .join("ClientApplicationInterface.xml");
    let text = read_utf8_sig(reader, &path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    let side_slots = |side: &str| {
        root.children()
            .filter(|node| unica_xml_format::names::element_has_name(*node, side, Some(CF_CAI_NS)))
            .filter_map(|side_el| {
                let slot = side_el
                    .descendants()
                    .filter(|node| {
                        unica_xml_format::names::element_has_name(*node, "uuid", Some(CF_CAI_NS))
                    })
                    .filter_map(|node| node.text())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(cf_panel_name)
                    .collect::<Vec<_>>();
                if slot.is_empty() {
                    None
                } else {
                    Some(slot)
                }
            })
            .collect::<Vec<_>>()
    };
    let declared = root
        .children()
        .filter(|node| {
            unica_xml_format::names::element_has_name(*node, "panelDef", Some(CF_CAI_NS))
        })
        .map(|node| cf_panel_name(node.attribute("id").unwrap_or("")))
        .collect::<Vec<_>>();
    Some(CfPanelLayout {
        top: side_slots("top"),
        left: side_slots("left"),
        right: side_slots("right"),
        bottom: side_slots("bottom"),
        declared,
    })
}

pub fn cf_format_layout_slots(slots: &[Vec<String>]) -> String {
    slots
        .iter()
        .map(|slot| {
            if slot.len() == 1 {
                slot[0].clone()
            } else {
                format!("Стек({})", slot.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub fn cf_append_full_panel_layout(
    reader: &dyn ProjectDiscoveryRead,
    lines: &mut Vec<String>,
    config_dir: &Path,
) {
    let Some(layout) = cf_read_panel_layout(reader, config_dir) else {
        return;
    };
    lines.push("--- Раскладка панелей ---".to_string());
    for (side, slots) in [
        ("top", &layout.top),
        ("left", &layout.left),
        ("right", &layout.right),
        ("bottom", &layout.bottom),
    ] {
        if slots.is_empty() {
            lines.push(format!("  {:<7} —", side));
        } else {
            lines.push(format!("  {:<7} {}", side, cf_format_layout_slots(slots)));
        }
    }
    if !layout.declared.is_empty() {
        lines.push(format!("  объявлено: {}", layout.declared.join(", ")));
    }
    lines.push(String::new());
}

pub fn cf_read_home_page(
    reader: &dyn ProjectDiscoveryRead,
    config_dir: &Path,
) -> Option<CfHomePageLayout> {
    let path = config_dir.join("Ext").join("HomePageWorkArea.xml");
    let text = read_utf8_sig(reader, &path).ok()?;
    parse_cf_home_page_xml(&text)
}

pub fn parse_cf_home_page_xml(text: &str) -> Option<CfHomePageLayout> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    Some(cf_home_page_layout(doc.root_element()))
}

pub fn parse_cf_home_page_xml_strict(text: &str) -> Result<CfHomePageLayout, String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("HomePageWorkArea.xml is malformed: {error}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "HomePageWorkArea" || root.tag_name().namespace() != Some(CF_HP_NS)
    {
        return Err("HomePageWorkArea.xml has a wrong root element".to_string());
    }
    Ok(cf_home_page_layout(root))
}

pub fn cf_home_page_layout(root: roxmltree::Node<'_, '_>) -> CfHomePageLayout {
    let template = unica_xml_format::read::child_text(root, "WorkingAreaTemplate", Some(CF_HP_NS))
        .trim()
        .to_string();
    let columns = root
        .children()
        .filter(|node| unica_xml_format::names::element_has_name(*node, "Column", Some(CF_HP_NS)))
        .collect::<Vec<_>>();
    let (left, right) = if columns.is_empty() {
        // Read-only inspection still supports pre-2.20 dumps. Mutations are guarded
        // separately and always rewrite the active 2.20 Column sequence.
        (
            cf_home_page_named_column(root, "LeftColumn"),
            cf_home_page_named_column(root, "RightColumn"),
        )
    } else {
        (
            cf_home_page_items(columns.first().copied()),
            cf_home_page_items(columns.get(1).copied()),
        )
    };
    CfHomePageLayout {
        template,
        left,
        right,
    }
}

pub fn cf_home_page_named_column(
    root: roxmltree::Node<'_, '_>,
    column_name: &str,
) -> Vec<CfHomePageItem> {
    let column = root
        .children()
        .find(|node| unica_xml_format::names::element_has_name(*node, column_name, Some(CF_HP_NS)));
    cf_home_page_items(column)
}

pub fn cf_home_page_items(column: Option<roxmltree::Node<'_, '_>>) -> Vec<CfHomePageItem> {
    let Some(column) = column else {
        return Vec::new();
    };
    column
        .children()
        .filter(|node| unica_xml_format::names::element_has_name(*node, "Item", Some(CF_HP_NS)))
        .map(|item| {
            let form = unica_xml_format::read::child_text(item, "Form", Some(CF_HP_NS))
                .trim()
                .to_string();
            let height = unica_xml_format::read::child_text(item, "Height", Some(CF_HP_NS))
                .trim()
                .parse::<i64>()
                .unwrap_or(10);
            let mut common = true;
            let mut roles = Vec::<(String, bool)>::new();
            if let Some(visibility) = item.children().find(|node| {
                unica_xml_format::names::element_has_name(*node, "Visibility", Some(CF_HP_NS))
            }) {
                let common_text =
                    unica_xml_format::read::child_text(visibility, "Common", Some(CF_XR_NS));
                if !common_text.trim().is_empty() {
                    common = common_text.trim() == "true";
                }
                roles = visibility
                    .children()
                    .filter(|node| {
                        unica_xml_format::names::element_has_name(*node, "Value", Some(CF_XR_NS))
                    })
                    .map(|node| {
                        (
                            node.attribute("name").unwrap_or("").to_string(),
                            node.text().unwrap_or("").trim() == "true",
                        )
                    })
                    .collect::<Vec<_>>();
            }
            CfHomePageItem {
                form,
                height,
                common,
                roles,
            }
        })
        .collect::<Vec<_>>()
}

pub fn cf_append_full_home_page_summary(
    reader: &dyn ProjectDiscoveryRead,
    lines: &mut Vec<String>,
    config_dir: &Path,
) {
    let Some(home_page) = cf_read_home_page(reader, config_dir) else {
        return;
    };
    lines.push("--- Начальная страница ---".to_string());
    lines.push(format!("  Шаблон: {}", home_page.template));
    lines.push(format!(
        "  LeftColumn: {}, RightColumn: {}  (детали: -Section home-page)",
        home_page.left.len(),
        home_page.right.len()
    ));
    lines.push(String::new());
}

pub fn cf_append_home_page_section(
    reader: &dyn ProjectDiscoveryRead,
    lines: &mut Vec<String>,
    config_dir: &Path,
    cfg_name: &str,
) {
    let Some(home_page) = cf_read_home_page(reader, config_dir) else {
        lines.push("Файл Ext/HomePageWorkArea.xml не найден".to_string());
        return;
    };
    lines.push(format!("=== Начальная страница: {cfg_name} ==="));
    lines.push(String::new());
    lines.push(format!("Шаблон: {}", home_page.template));
    lines.push(String::new());
    for (label, items) in [
        ("LeftColumn", &home_page.left),
        ("RightColumn", &home_page.right),
    ] {
        if items.is_empty() {
            lines.push(format!("{label}: —"));
            lines.push(String::new());
            continue;
        }
        lines.push(format!("{label} ({}):", items.len()));
        for item in items {
            lines.push(cf_format_home_page_item(item, true));
            for (role, value) in &item.roles {
                lines.push(format!("      {role}: {value}"));
            }
        }
        lines.push(String::new());
    }
}

pub fn cf_format_home_page_item(item: &CfHomePageItem, detailed: bool) -> String {
    let mut badges = vec![format!("h={}", item.height)];
    if !item.common {
        badges.push("скрыта".to_string());
    }
    if !item.roles.is_empty() {
        if detailed {
            badges.push(format!("роли: {}", item.roles.len()));
        } else {
            badges.push(format!("+{} ролей", item.roles.len()));
        }
    }
    let tail = if badges.is_empty() {
        String::new()
    } else {
        format!(" ({})", badges.join(", "))
    };
    format!("    {}{tail}", item.form)
}

pub fn cf_append_full_storages_and_forms(lines: &mut Vec<String>, props: roxmltree::Node<'_, '_>) {
    lines.push("--- Хранилища и формы по умолчанию ---".to_string());
    for property in [
        "CommonSettingsStorage",
        "ReportsUserSettingsStorage",
        "ReportsVariantsStorage",
        "FormDataSettingsStorage",
        "DynamicListsUserSettingsStorage",
        "URLExternalDataStorage",
        "DefaultReportForm",
        "DefaultReportVariantForm",
        "DefaultReportSettingsForm",
        "DefaultReportAppearanceTemplate",
        "DefaultDynamicListSettingsForm",
        "DefaultSearchForm",
        "DefaultDataHistoryChangeHistoryForm",
        "DefaultDataHistoryVersionDataForm",
        "DefaultDataHistoryVersionDifferencesForm",
        "DefaultCollaborationSystemUsersChoiceForm",
        "DefaultConstantsForm",
        "DefaultInterface",
        "DefaultStyle",
    ] {
        let value = cf_prop_text(props, property);
        if !value.is_empty() {
            lines.push(format!("  {property}: {value}"));
        }
    }
    lines.push(String::new());
}

pub fn cf_append_full_multilang_info(lines: &mut Vec<String>, props: roxmltree::Node<'_, '_>) {
    let cfg_brief = cf_prop_ml(props, "BriefInformation");
    let cfg_detail = cf_prop_ml(props, "DetailedInformation");
    let cfg_copyright = cf_prop_ml(props, "Copyright");
    let cfg_vendor_addr = cf_prop_ml(props, "VendorInformationAddress");
    let cfg_info_addr = cf_prop_ml(props, "ConfigurationInformationAddress");
    if cfg_brief.is_empty()
        && cfg_detail.is_empty()
        && cfg_copyright.is_empty()
        && cfg_vendor_addr.is_empty()
        && cfg_info_addr.is_empty()
    {
        return;
    }

    lines.push("--- Информация ---".to_string());
    if !cfg_brief.is_empty() {
        lines.push(format!("Краткая:         {cfg_brief}"));
    }
    if !cfg_detail.is_empty() {
        lines.push(format!("Подробная:       {cfg_detail}"));
    }
    if !cfg_copyright.is_empty() {
        lines.push(format!("Copyright:       {cfg_copyright}"));
    }
    if !cfg_vendor_addr.is_empty() {
        lines.push(format!("Сайт поставщика: {cfg_vendor_addr}"));
    }
    if !cfg_info_addr.is_empty() {
        lines.push(format!("Адрес информ.:   {cfg_info_addr}"));
    }
    lines.push(String::new());
}

pub fn cf_append_full_mobile_functionalities(
    lines: &mut Vec<String>,
    props: roxmltree::Node<'_, '_>,
) {
    let Some(mobile_func) = props.children().find(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "UsedMobileApplicationFunctionalities",
            Some(CF_MD_NS),
        )
    }) else {
        return;
    };

    let mut enabled = Vec::<String>::new();
    let mut disabled = Vec::<String>::new();
    for func in mobile_func
        .children()
        .filter(|node| unica_xml_format::names::element_has_name(*node, "functionality", None))
    {
        let name = unica_xml_format::read::child_text(func, "functionality", None);
        let use_flag = unica_xml_format::read::child_text(func, "use", None);
        if use_flag == "true" {
            enabled.push(name);
        } else {
            disabled.push(name);
        }
    }

    let total = enabled.len() + disabled.len();
    lines.push(format!(
        "--- Мобильные функциональности ({total}, включено: {}) ---",
        enabled.len()
    ));
    for name in enabled {
        lines.push(format!("  [+] {name}"));
    }
    for name in disabled {
        lines.push(format!("  [-] {name}"));
    }
    lines.push(String::new());
}

pub fn cf_append_full_internal_info(lines: &mut Vec<String>, cfg: roxmltree::Node<'_, '_>) {
    let Some(internal_info) = cfg.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "InternalInfo", Some(CF_MD_NS))
    }) else {
        return;
    };
    let contained = internal_info
        .children()
        .filter(|node| {
            unica_xml_format::names::element_has_name(*node, "ContainedObject", Some(CF_XR_NS))
        })
        .collect::<Vec<_>>();
    lines.push(format!(
        "--- InternalInfo ({} ContainedObject) ---",
        contained.len()
    ));
    for co in contained {
        let class_id = unica_xml_format::read::child_text(co, "ClassId", Some(CF_XR_NS));
        let object_id = unica_xml_format::read::child_text(co, "ObjectId", Some(CF_XR_NS));
        lines.push(format!("  {class_id} -> {object_id}"));
    }
    lines.push(String::new());
}

pub fn cf_append_full_child_objects(
    lines: &mut Vec<String>,
    cfg: roxmltree::Node<'_, '_>,
    counts: &[(String, usize)],
    total_objects: usize,
) {
    lines.push(format!("--- Состав ({total_objects} объектов) ---"));
    lines.push(String::new());
    let child_objects = cfg.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "ChildObjects", Some(CF_MD_NS))
    });

    for type_name in cf_type_order() {
        let Some((_, count)) = counts.iter().find(|(name, _)| name == type_name) else {
            continue;
        };
        lines.push(format!(
            "  {} ({type_name}): {count}",
            cf_type_ru_name(type_name)
        ));
        if let Some(child_objects) = child_objects {
            for child in child_objects.children().filter(|node| {
                unica_xml_format::names::element_has_name(*node, type_name, Some(CF_MD_NS))
            }) {
                lines.push(format!("    {}", child.text().unwrap_or("")));
            }
        }
    }
}

pub fn cf_paginate(lines: Vec<String>, args: &Map<String, Value>) -> String {
    let total = lines.len();
    let limit = unica_operation_contract::input::int_arg(args, &["limit", "Limit"])
        .unwrap_or(150)
        .max(0) as usize;
    let offset = unica_operation_contract::input::int_arg(args, &["offset", "Offset"])
        .unwrap_or(0)
        .max(0) as usize;
    if offset > 0 || limit < total {
        let start = offset.min(total);
        let end = (start + limit).min(total);
        let mut result = lines[start..end].join("\n");
        if end < total {
            result.push_str(&format!(
                "\n\n... ({end} of {total} lines, use -Offset {end} to continue)"
            ));
        }
        result
    } else {
        lines.join("\n")
    }
}

pub fn cf_edit_operations(
    reader: &dyn ProjectDiscoveryRead,
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = unica_operation_contract::input::absolutize(definition_file, cwd);
        let text = read_string(reader, &definition_file)
            .map_err(|err| format!("failed to read {}: {err}", definition_file.display()))?;
        let parsed: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("failed to parse {}: {err}", definition_file.display()))?;
        Ok(cf_edit_operations_from_value(parsed, operation))
    } else {
        Ok(vec![(
            operation.unwrap_or("").to_string(),
            Value::String(
                unica_operation_contract::input::string_arg(args, &["value", "Value"])
                    .unwrap_or_default()
                    .to_string(),
            ),
        )])
    }
}

pub fn cf_edit_operations_from_value(
    parsed: Value,
    operation: Option<&str>,
) -> Vec<(String, Value)> {
    let items = match parsed {
        Value::Array(items) => items,
        other => vec![other],
    };
    items
        .into_iter()
        .map(|item| {
            let op_name = item
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or(operation.unwrap_or(""))
                .to_string();
            let value = item
                .get("value")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            (op_name, value)
        })
        .collect()
}

pub fn cf_edit_format_dependency_paths_for_operations(
    config_path: &Path,
    operations: &[(String, Value)],
) -> Result<Vec<PathBuf>, String> {
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    // Every cf.edit run performs validate_cf after publication. That validator
    // reads HomePageWorkArea.xml when present, so it is a real dependency even
    // for operations that do not write the home-page layout themselves.
    let mut paths = vec![
        config_path.to_path_buf(),
        config_dir.join("Ext").join("HomePageWorkArea.xml"),
    ];
    for (operation, value) in operations {
        if operation == "add-childObject" {
            for item in cf_edit_batch_value(value) {
                let (type_name, object_name) = cf_edit_parse_child_object(&item)?;
                let type_dir = cf_validate_child_type_dir(type_name)
                    .ok_or_else(|| format!("Unknown type '{type_name}'"))?;
                paths.push(
                    config_dir
                        .join(type_dir)
                        .join(object_name)
                        .with_extension("xml"),
                );
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub fn cf_edit_validate_child_object_kinds(operations: &[(String, Value)]) -> Result<(), String> {
    for (op_name, op_value) in operations {
        if !matches!(op_name.as_str(), "add-childObject" | "remove-childObject") {
            continue;
        }
        for item in cf_edit_batch_value(op_value) {
            let (type_name, _) = cf_edit_parse_child_object(&item)?;
            if unica_source_designer::registration::cf_validate_child_object_type_index(type_name)
                .is_none()
            {
                return Err(format!("Unknown type '{type_name}'"));
            }
        }
    }
    Ok(())
}

pub fn cf_edit_parse_child_object(item: &str) -> Result<(&str, &str), String> {
    let Some((type_name, object_name)) = item.split_once('.') else {
        return Err(format!("Invalid format '{item}', expected 'Type.Name'"));
    };
    if type_name.is_empty() || object_name.is_empty() {
        return Err(format!("Invalid format '{item}', expected 'Type.Name'"));
    }
    let mut components = Path::new(object_name).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(std::path::Component::Normal(component))
            if component == std::ffi::OsStr::new(object_name)
    ) && components.next().is_none();
    if !unica_xml_format::names::xml_ncname_is_valid(object_name) || !is_single_path_component {
        return Err(format!(
            "object name must be a valid Unicode XML NCName and a single path component: {object_name:?}"
        ));
    }
    Ok((type_name, object_name))
}

pub fn cf_edit_batch_value(value: &Value) -> Vec<String> {
    let text = match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    };
    text.split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn cf_edit_config_name(text: &str) -> Result<String, String> {
    let (props_start, props_end) = cf_edit_properties_body_range(text)?;
    let body = &text[props_start..props_end];
    if let Some((_, _, Some((name_start, name_end)))) =
        cf_edit_element_range(body, "Name").map(|(start, end, body)| {
            (
                props_start + start,
                props_start + end,
                body.map(|(start, end)| (props_start + start, props_start + end)),
            )
        })
    {
        return Ok(unica_xml_format::text::unescape_xml(
            text[name_start..name_end].trim(),
        ));
    }
    Ok(String::new())
}

pub fn cf_edit_ml_property_xml(prop_name: &str, value: &str) -> String {
    if value.is_empty() {
        return format!("<{prop_name}/>");
    }
    format!(
        "<{prop_name}>\r\n\
         \t\t\t\t<v8:item>\r\n\
         \t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\
         \t\t\t\t\t<v8:content>{}</v8:content>\r\n\
         \t\t\t\t</v8:item>\r\n\
         \t\t\t</{prop_name}>",
        unica_xml_format::text::escape_xml(value)
    )
}

pub fn cf_edit_scalar_property_xml(prop_name: &str, value: &str) -> String {
    if value.is_empty() {
        format!("<{prop_name}/>")
    } else {
        format!(
            "<{prop_name}>{}</{prop_name}>",
            unica_xml_format::text::escape_xml(value)
        )
    }
}

pub fn cf_edit_replace_property(
    text: &str,
    prop_name: &str,
    replacement: &str,
) -> Result<String, String> {
    let (props_start, props_end) = cf_edit_properties_body_range(text)?;
    let body = &text[props_start..props_end];
    let Some((start, end, _)) = cf_edit_element_range(body, prop_name) else {
        return Err(format!("Property '{prop_name}' not found in Properties"));
    };
    let abs_start = props_start + start;
    let abs_end = props_start + end;
    Ok(format!(
        "{}{}{}",
        &text[..abs_start],
        replacement,
        &text[abs_end..]
    ))
}

pub fn cf_edit_default_roles(text: &str) -> Result<Vec<String>, String> {
    let (props_start, props_end) = cf_edit_properties_body_range(text)?;
    let body = &text[props_start..props_end];
    let Some((_, _, body_range)) = cf_edit_element_range(body, "DefaultRoles") else {
        return Err("No <DefaultRoles> element found in Properties".to_string());
    };
    let Some((start, end)) = body_range else {
        return Ok(Vec::new());
    };
    let roles_body = &body[start..end];
    let mut roles = Vec::new();
    let mut offset = 0usize;
    while let Some(rel_start) = roles_body[offset..].find("<xr:Item") {
        let item_start = offset + rel_start;
        let Some(gt_rel) = roles_body[item_start..].find('>') else {
            break;
        };
        let value_start = item_start + gt_rel + 1;
        let Some(end_rel) = roles_body[value_start..].find("</xr:Item>") else {
            break;
        };
        let value_end = value_start + end_rel;
        roles.push(unica_xml_format::text::unescape_xml(
            roles_body[value_start..value_end].trim(),
        ));
        offset = value_end + "</xr:Item>".len();
    }
    Ok(roles)
}

pub fn cf_edit_replace_default_roles(text: &str, roles: &[String]) -> Result<String, String> {
    cf_edit_replace_property(text, "DefaultRoles", &cf_edit_default_roles_xml(roles))
}

pub fn cf_edit_default_roles_xml(roles: &[String]) -> String {
    if roles.is_empty() {
        return "<DefaultRoles/>".to_string();
    }
    let body = roles
        .iter()
        .map(|role| {
            format!(
                "\r\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                unica_xml_format::text::escape_xml(role)
            )
        })
        .collect::<String>();
    format!("<DefaultRoles>{body}\r\n\t\t\t</DefaultRoles>")
}

pub fn cf_edit_role_ref(role: &str) -> String {
    if role.starts_with("Role.") {
        role.to_string()
    } else {
        format!("Role.{role}")
    }
}

pub fn cf_edit_serialized_text(text: &str, source_text: &str) -> String {
    let mut output =
        unica_xml_format::text::lxml_tree_serialized_text_like_source(text, source_text);
    let source_has_trailing_newline = source_text.ends_with('\n') || source_text.ends_with('\r');
    if !source_has_trailing_newline {
        if output.ends_with("\r\n") {
            output.truncate(output.len() - 2);
        } else if output.ends_with('\n') {
            output.pop();
        }
    }
    output
}

pub fn cf_edit_child_objects(text: &str) -> Result<Vec<(String, String)>, String> {
    let Some((_, _, body_range)) = cf_edit_element_range(text, "ChildObjects") else {
        return Err("No <ChildObjects> element found".to_string());
    };
    let Some((start, end)) = body_range else {
        return Ok(Vec::new());
    };
    let body = &text[start..end];
    let mut result = Vec::new();
    let mut offset = 0usize;
    while let Some(rel_start) = body[offset..].find('<') {
        let tag_start = offset + rel_start;
        if body[tag_start + 1..].starts_with('/') {
            offset = tag_start + 1;
            continue;
        }
        let Some(gt_rel) = body[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + gt_rel;
        let tag_name = body[tag_start + 1..tag_end].trim();
        if tag_name.is_empty() || tag_name.contains(' ') || tag_name.ends_with('/') {
            offset = tag_end + 1;
            continue;
        }
        let close = format!("</{tag_name}>");
        let value_start = tag_end + 1;
        let Some(close_rel) = body[value_start..].find(&close) else {
            break;
        };
        let value_end = value_start + close_rel;
        result.push((
            tag_name.to_string(),
            unica_xml_format::text::unescape_xml(body[value_start..value_end].trim()),
        ));
        offset = value_end + close.len();
    }
    Ok(result)
}

pub fn cf_edit_remove_child_object_text(
    text: &mut String,
    type_name: &str,
    object_name: &str,
) -> Result<bool, String> {
    if metadata_kind(type_name).is_none() {
        return Err(format!("Unknown type '{type_name}'"));
    }
    let entries = unica_source_designer::registration::cf_edit_child_object_entries(text)?;
    let Some(entry) = entries
        .into_iter()
        .find(|entry| entry.type_name == type_name && entry.object_name == object_name)
    else {
        return Ok(false);
    };
    text.replace_range(entry.line_range, "");
    Ok(true)
}

pub fn cf_edit_replace_child_objects(
    text: &str,
    children: &[(String, String)],
) -> Result<String, String> {
    let Some((start, end, _)) = cf_edit_element_range(text, "ChildObjects") else {
        return Err("No <ChildObjects> element found".to_string());
    };
    let replacement = cf_edit_child_objects_xml(children);
    Ok(format!("{}{}{}", &text[..start], replacement, &text[end..]))
}

pub fn cf_edit_child_objects_xml(children: &[(String, String)]) -> String {
    if children.is_empty() {
        return "<ChildObjects/>".to_string();
    }
    let mut body = String::from("<ChildObjects>\n");
    for (index, (type_name, obj_name)) in children.iter().enumerate() {
        body.push_str(&format!(
            "\t\t\t<{type_name}>{}</{type_name}>",
            unica_xml_format::text::escape_xml(obj_name)
        ));
        if index + 1 == children.len() {
            body.push('\n');
        } else {
            body.push_str("\r\n");
        }
    }
    body.push_str("\t\t</ChildObjects>");
    body
}

pub fn cf_edit_properties_body_range(text: &str) -> Result<(usize, usize), String> {
    let Some((_, _, Some(body))) = cf_edit_element_range(text, "Properties") else {
        return Err("No <Properties> element found".to_string());
    };
    Ok(body)
}

pub fn cf_edit_element_range(
    text: &str,
    tag: &str,
) -> Option<unica_source_designer::registration::CfEditElementRange> {
    let needle = format!("<{tag}");
    let mut offset = 0usize;
    while let Some(rel_start) = text[offset..].find(&needle) {
        let start = offset + rel_start;
        let boundary_index = start + needle.len();
        let boundary = text[boundary_index..].chars().next();
        let is_boundary = match boundary {
            Some('>') | Some('/') => true,
            Some(ch) => ch.is_whitespace(),
            None => false,
        };
        if !is_boundary {
            offset = boundary_index;
            continue;
        }
        let gt = start + text[start..].find('>')?;
        let open_tag = &text[start..=gt];
        if open_tag.trim_end().ends_with("/>") {
            return Some((start, gt + 1, None));
        }
        let close = format!("</{tag}>");
        let body_start = gt + 1;
        let close_start = body_start + text[body_start..].find(&close)?;
        let end = close_start + close.len();
        return Some((start, end, Some((body_start, close_start))));
    }
    None
}

pub fn cf_edit_set_panels(
    value: &Value,
    config_dir: &Path,
    fresh_uuid: fn() -> String,
) -> Result<CfEditExternalFilePlan, String> {
    let layout = cf_edit_json_object(value, "set-panels value must be valid JSON object")?;
    if layout.is_empty() {
        return Err("set-panels value must be non-empty object".to_string());
    }
    let sides = ["top", "left", "right", "bottom"];
    for key in layout.keys() {
        if !sides.contains(&key.as_str()) {
            return Err(format!(
                "Unknown side '{key}'. Allowed: {}",
                sides.join(", ")
            ));
        }
    }
    let mut body_parts = Vec::new();
    for side in sides {
        let Some(entries) = layout.get(side) else {
            continue;
        };
        let entry_values = if let Some(items) = entries.as_array() {
            items.clone()
        } else {
            vec![entries.clone()]
        };
        for entry in entry_values {
            let entry_xml = cf_edit_panel_entry_xml(&entry, "\t\t", fresh_uuid)?;
            body_parts.push(format!("\t<{side}>\r\n{entry_xml}\r\n\t</{side}>"));
        }
    }
    let body = body_parts.join("\r\n");
    let body_block = if body.is_empty() {
        String::new()
    } else {
        format!("{body}\r\n")
    };
    let declarations = concat!(
        "\t<panelDef id=\"b553047f-c9aa-4157-978d-448ecad24248\"/>\r\n",
        "\t<panelDef id=\"13322b22-3960-4d68-93a6-fe2dd7f28ca3\"/>\r\n",
        "\t<panelDef id=\"c933ac92-92cd-459d-81cc-e0c8a83ced99\"/>\r\n",
        "\t<panelDef id=\"cbab57f2-a0f3-4f0a-89ea-4cb19570ab75\"/>\r\n",
        "\t<panelDef id=\"b2735bd3-d822-4430-ba59-c9e869693b24\"/>",
    );
    let cai_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
         <ClientApplicationInterface xmlns=\"http://v8.1c.ru/8.2/managed-application/core\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:type=\"InterfaceLayouter\">\r\n\
         {body_block}{declarations}\r\n\
         </ClientApplicationInterface>"
    );
    Ok(CfEditExternalFilePlan {
        path: config_dir.join("Ext/ClientApplicationInterface.xml"),
        bytes: unica_xml_format::text::utf8_bom_bytes(&cai_xml),
    })
}

pub fn cf_edit_panel_entry_xml(
    entry: &Value,
    indent: &str,
    fresh_uuid: fn() -> String,
) -> Result<String, String> {
    if let Some(alias) = entry.as_str() {
        let key = cf_edit_panel_alias(alias);
        let Some(uuid) = cf_edit_panel_uuid(key) else {
            return Err(format!(
                "Unknown panel alias '{alias}'. Allowed: favorites, functions, history, open, sections"
            ));
        };
        let inst = fresh_uuid();
        return Ok(format!(
            "{indent}<panel id=\"{inst}\">\r\n{indent}\t<uuid>{uuid}</uuid>\r\n{indent}</panel>"
        ));
    }
    if let Some(group) = entry.as_object().and_then(|obj| obj.get("group")) {
        let Some(children) = group.as_array() else {
            return Err("group must contain at least one entry".to_string());
        };
        if children.is_empty() {
            return Err("group must contain at least one entry".to_string());
        }
        let gid = fresh_uuid();
        let mut inner = String::new();
        for child in children {
            let child_xml = cf_edit_panel_entry_xml(child, &format!("{indent}\t\t"), fresh_uuid)?;
            inner.push_str(&format!(
                "{indent}\t<group>\r\n{child_xml}\r\n{indent}\t</group>\r\n"
            ));
        }
        return Ok(format!(
            "{indent}<group id=\"{gid}\">\r\n{inner}{indent}</group>"
        ));
    }
    Err(format!(
        "Panel entry must be string alias or {{group:[...]}}, got: {entry}"
    ))
}

pub fn cf_edit_set_home_page(
    value: &Value,
    config_dir: &Path,
) -> Result<CfEditExternalFilePlan, String> {
    let layout = cf_edit_json_object(value, "set-home-page value must be valid JSON object")?;
    if layout.is_empty() {
        return Err("set-home-page value must be non-empty object".to_string());
    }
    for key in layout.keys() {
        if !matches!(
            key.as_str(),
            "template" | "WorkingAreaTemplate" | "left" | "LeftColumn" | "right" | "RightColumn"
        ) {
            return Err(format!(
                "Unknown key '{key}'. Allowed: template, left, right"
            ));
        }
    }
    let template = cf_edit_object_field(&layout, &["template", "WorkingAreaTemplate"])
        .and_then(Value::as_str)
        .unwrap_or("TwoColumnsEqualWidth");
    if !matches!(
        template,
        "OneColumn" | "TwoColumnsEqualWidth" | "TwoColumnsVariableWidth"
    ) {
        return Err(format!(
            "Unknown template '{template}'. Allowed: OneColumn, TwoColumnsEqualWidth, TwoColumnsVariableWidth"
        ));
    }
    let left_items = cf_edit_object_field(&layout, &["left", "LeftColumn"]);
    let right_items = cf_edit_object_field(&layout, &["right", "RightColumn"]);
    if template == "OneColumn" && right_items.is_some_and(cf_edit_truthy_value) {
        return Err("Template 'OneColumn' cannot have items in 'right' column".to_string());
    }
    let columns_xml = if template == "OneColumn" {
        cf_edit_home_page_column_xml("Column", left_items)?
    } else {
        let left_xml = cf_edit_home_page_column_xml("LeftColumn", left_items)?;
        let right_xml = cf_edit_home_page_column_xml("RightColumn", right_items)?;
        format!("{left_xml}\r\n{right_xml}")
    };
    let hp_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
         <HomePageWorkArea xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" \
         xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">\r\n\
         \t<WorkingAreaTemplate>{template}</WorkingAreaTemplate>\r\n\
         {columns_xml}\r\n\
         </HomePageWorkArea>",
        format_version = ACTIVE_FORMAT_PROFILE.export_format
    );
    Ok(CfEditExternalFilePlan {
        path: config_dir.join("Ext/HomePageWorkArea.xml"),
        bytes: unica_xml_format::text::utf8_bom_bytes(&hp_xml),
    })
}

pub fn cf_edit_home_page_column_xml(tag: &str, items: Option<&Value>) -> Result<String, String> {
    let Some(items) = items.filter(|value| cf_edit_truthy_value(value)) else {
        return Ok(format!("\t<{tag}/>"));
    };
    let values = if let Some(array) = items.as_array() {
        if array.is_empty() {
            return Ok(format!("\t<{tag}/>"));
        }
        array.clone()
    } else {
        vec![items.clone()]
    };
    let blocks = values
        .iter()
        .map(|item| cf_edit_home_page_item_xml(item, "\t\t"))
        .collect::<Result<Vec<_>, _>>()?
        .join("\r\n");
    Ok(format!("\t<{tag}>\r\n{blocks}\r\n\t</{tag}>"))
}

pub fn cf_edit_home_page_item_xml(entry: &Value, indent: &str) -> Result<String, String> {
    let (form_ref, height, common, roles) = if let Some(form_ref) = entry.as_str() {
        (cf_edit_normalize_form_ref(form_ref), 10i64, true, None)
    } else if let Some(obj) = entry.as_object() {
        let form_raw = cf_edit_object_field(obj, &["form", "Form"])
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Home page item: 'form' is required, got: {entry}"))?;
        let height = match cf_edit_object_field(obj, &["height", "Height"]) {
            None => 10,
            Some(Value::Number(value)) => value.as_i64().ok_or_else(|| {
                format!("Home page item height must be a JSON integer when provided, got: {value}")
            })?,
            Some(value) => {
                return Err(format!(
                    "Home page item height must be a JSON integer when provided, got: {value}"
                ));
            }
        };
        let common = match cf_edit_object_field(obj, &["visibility", "Visibility"]) {
            None => true,
            Some(Value::Bool(value)) => *value,
            Some(value) => {
                return Err(format!(
                    "Home page item visibility must be a JSON boolean true or false, got: {value}"
                ));
            }
        };
        let roles = match obj.get("roles") {
            None => None,
            Some(Value::Object(roles)) => Some(roles),
            Some(value) => {
                return Err(format!(
                    "Home page item roles must be a JSON object of role names to boolean values, got: {value}"
                ));
            }
        };
        (cf_edit_normalize_form_ref(form_raw), height, common, roles)
    } else {
        return Err(format!(
            "Home page item must be string or object, got: {entry}"
        ));
    };

    let mut vis_parts = vec![format!("{indent}\t\t<xr:Common>{common}</xr:Common>")];
    if let Some(roles) = roles {
        for (role_name, value) in roles {
            let value = value.as_bool().ok_or_else(|| {
                format!(
                    "Home page item roles.{role_name} must be a JSON boolean true or false, got: {value}"
                )
            })?;
            let role_name = if role_name.starts_with("Role.") || cf_edit_uuid_like(role_name) {
                role_name.clone()
            } else {
                format!("Role.{role_name}")
            };
            vis_parts.push(format!(
                "{indent}\t\t<xr:Value name=\"{}\">{}</xr:Value>",
                unica_xml_format::text::escape_xml(&role_name),
                value
            ));
        }
    }
    let vis_block = vis_parts.join("\r\n");
    Ok(format!(
        "{indent}<Item>\r\n\
         {indent}\t<Form>{}</Form>\r\n\
         {indent}\t<Height>{height}</Height>\r\n\
         {indent}\t<Visibility>\r\n\
         {vis_block}\r\n\
         {indent}\t</Visibility>\r\n\
         {indent}</Item>",
        unica_xml_format::text::escape_xml(&form_ref)
    ))
}

pub fn cf_edit_normalize_form_ref(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || cf_edit_uuid_like(trimmed) {
        return trimmed.to_string();
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        let mut parts = trimmed
            .replace('\\', "/")
            .split('/')
            .filter(|part| !part.is_empty() && part.to_lowercase() != "ext")
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if parts
            .last()
            .is_some_and(|part| part.to_lowercase() == "form.xml")
        {
            parts.pop();
        }
        if parts.len() >= 2 {
            if let Some(type_singular) = cf_edit_dir_to_type(&parts[0]) {
                if type_singular == "CommonForm" {
                    return format!("CommonForm.{}", parts[1]);
                }
                if parts.len() >= 4 && parts[2].eq_ignore_ascii_case("Forms") {
                    return format!("{}.{}.Form.{}", type_singular, parts[1], parts[3]);
                }
            }
        }
        return trimmed.to_string();
    }
    let mut parts = trimmed
        .split('.')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if let Some(head) = parts.first_mut() {
        if let Some(normalized) = cf_edit_ru_type(head) {
            *head = normalized.to_string();
        }
    }
    for part in parts.iter_mut().skip(1) {
        if part == "Форма" {
            *part = "Form".to_string();
        }
    }
    if parts.len() == 3
        && parts[0] != "CommonForm"
        && unica_source_designer::registration::cf_validate_child_object_type_index(&parts[0])
            .is_some()
    {
        parts.insert(2, "Form".to_string());
    }
    parts.join(".")
}

pub fn cf_edit_json_object(
    value: &Value,
    string_parse_error: &str,
) -> Result<Map<String, Value>, String> {
    if let Value::String(text) = value {
        let parsed: Value =
            serde_json::from_str(text).map_err(|_| string_parse_error.to_string())?;
        return Ok(parsed
            .as_object()
            .ok_or_else(|| string_parse_error.to_string())?
            .clone());
    }
    value
        .as_object()
        .cloned()
        .ok_or_else(|| string_parse_error.to_string())
}

pub fn cf_edit_object_field<'a>(
    object: &'a Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

pub fn cf_edit_truthy_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_i64().is_some_and(|number| number != 0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

pub fn cf_edit_i64_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

pub fn cf_init_post_validation_dependency_paths(planned: &CfInitPlannedXml) -> Vec<PathBuf> {
    vec![planned.output_dir.join("Ext/HomePageWorkArea.xml")]
}

pub fn mobile_functionality_xml() -> String {
    const MOBILE_FUNCTIONS: &[(&str, &str)] = &[
        ("Biometrics", "true"),
        ("Location", "false"),
        ("BackgroundLocation", "false"),
        ("BluetoothPrinters", "false"),
        ("WiFiPrinters", "false"),
        ("Contacts", "false"),
        ("Calendars", "false"),
        ("PushNotifications", "false"),
        ("LocalNotifications", "false"),
        ("InAppPurchases", "false"),
        ("PersonalComputerFileExchange", "false"),
        ("Ads", "false"),
        ("NumberDialing", "false"),
        ("CallProcessing", "false"),
        ("CallLog", "false"),
        ("AutoSendSMS", "false"),
        ("ReceiveSMS", "false"),
        ("SMSLog", "false"),
        ("Camera", "false"),
        ("Microphone", "false"),
        ("MusicLibrary", "false"),
        ("PictureAndVideoLibraries", "false"),
        ("AudioPlaybackAndVibration", "false"),
        ("BackgroundAudioPlaybackAndVibration", "false"),
        ("InstallPackages", "false"),
        ("OSBackup", "true"),
        ("ApplicationUsageStatistics", "false"),
        ("BarcodeScanning", "false"),
        ("BackgroundAudioRecording", "false"),
        ("AllFilesAccess", "false"),
        ("Videoconferences", "false"),
        ("NFC", "false"),
        ("DocumentScanning", "false"),
        ("SpeechToText", "false"),
        ("Geofences", "false"),
        ("IncomingShareRequests", "false"),
        ("AllIncomingShareRequestsTypesProcessing", "false"),
        ("TextToSpeech", "false"),
    ];

    let mut xml = String::new();
    for (name, enabled) in MOBILE_FUNCTIONS {
        xml.push_str(&format!(
            "\r\n\t\t\t\t<app:functionality>\r\n\t\t\t\t\t<app:functionality>{name}</app:functionality>\r\n\t\t\t\t\t<app:use>{enabled}</app:use>\r\n\t\t\t\t</app:functionality>"
        ));
    }
    xml
}

pub fn contained_objects_xml(object_ids: &[String]) -> String {
    const CLASS_IDS: &[&str] = &[
        "9cd510cd-abfc-11d4-9434-004095e12fc7",
        "9fcd25a0-4822-11d4-9414-008048da11f9",
        "e3687481-0a87-462c-a166-9f34594f9bba",
        "9de14907-ec23-4a07-96f0-85521cb6b53b",
        "51f2d5d8-ea4d-4064-8892-82951750031e",
        "e68182ea-4237-4383-967f-90c1e3370bc7",
        "fb282519-d103-4dd3-bc12-cb271d631dfc",
    ];

    let mut xml = String::new();
    for (class_id, object_id) in CLASS_IDS.iter().zip(object_ids) {
        xml.push_str(&format!(
            "\t\t\t<xr:ContainedObject>\n\t\t\t\t<xr:ClassId>{class_id}</xr:ClassId>\n\t\t\t\t<xr:ObjectId>{object_id}</xr:ObjectId>\n\t\t\t</xr:ContainedObject>\n"
        ));
    }
    xml
}

pub struct CfValidationReporter {
    pub errors: usize,
    pub warnings: usize,
    pub ok_count: usize,
    pub stopped: bool,
    pub max_errors: usize,
    pub detailed: bool,
    pub lines: Vec<String>,
    pub obj_name: String,
}

pub struct CfValidationRun {
    pub ok: bool,
    pub stdout: String,
    pub artifact: PathBuf,
    pub errors: Vec<String>,
}

pub struct ParsedCfInfo {
    pub data: CfInfoData,
    pub registered_objects: Vec<CfRegisteredObject>,
}

pub struct CfPanelLayout {
    pub top: Vec<Vec<String>>,
    pub left: Vec<Vec<String>>,
    pub right: Vec<Vec<String>>,
    pub bottom: Vec<Vec<String>>,
    pub declared: Vec<String>,
}

pub struct CfHomePageItem {
    pub form: String,
    pub height: i64,
    pub common: bool,
    pub roles: Vec<(String, bool)>,
}

pub struct CfHomePageLayout {
    pub template: String,
    pub left: Vec<CfHomePageItem>,
    pub right: Vec<CfHomePageItem>,
}

pub struct CfEditExternalFilePlan {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CfInitPlannedXml {
    pub output_dir: PathBuf,
    pub configuration: PathBuf,
    pub language: PathBuf,
    pub client_application_interface: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfValidationScope {
    Full,
    OwnerShape,
}

impl CfValidationReporter {
    pub fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: vec![String::new()],
            obj_name: "(unknown)".to_string(),
        }
    }

    pub fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub fn finalize(mut self) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!(
                    "=== Validation OK: Configuration.{} ({checks} checks) ===",
                    self.obj_name
                ),
                Vec::new(),
            );
        }
        self.lines.insert(
            0,
            format!("=== Validation: Configuration.{} ===", self.obj_name),
        );
        self.lines.push(String::new());
        self.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            self.errors, self.warnings
        ));
        let errors = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[ERROR]"))
            .cloned()
            .collect::<Vec<_>>();
        (ok, self.lines.join("\r\n") + "\r\n", errors)
    }
}

pub fn validate_cf_source(
    reader: &dyn ProjectDiscoveryRead,
    args: &Map<String, Value>,
    resolved_path: PathBuf,
    cwd: &Path,
    scope: CfValidationScope,
) -> Result<CfValidationRun, String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    const XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
    const HP_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

    let config_dir = resolved_path.parent().unwrap_or(cwd);
    let detailed = unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]);
    let owner_shape_only = scope == CfValidationScope::OwnerShape;
    let max_errors = unica_operation_contract::input::int_arg(args, &["maxErrors", "MaxErrors"])
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(30);

    let text = read_utf8_sig(reader, &resolved_path)?;
    let source = text.trim_start_matches('\u{feff}');
    let doc = match Document::parse(source) {
        Ok(doc) => doc,
        Err(err) => {
            let mut report = CfValidationReporter::new(max_errors, detailed);
            report.obj_name = "(parse failed)".to_string();
            report.error(format!("1. XML parse failed: {err}"));
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfValidationRun {
                ok,
                stdout,
                artifact: resolved_path,
                errors,
            });
        }
    };

    let mut report = CfValidationReporter::new(max_errors, detailed);
    let root = doc.root_element();
    let mut check1_ok = true;
    let root_local = root.tag_name().name();
    let root_ns = root.tag_name().namespace().unwrap_or("");
    if root_local != "MetaDataObject" {
        report.error(format!(
            "1. Root element is '{root_local}', expected 'MetaDataObject'"
        ));
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    }
    if root_ns != MD_NS {
        report.error(format!(
            "1. Root namespace is '{root_ns}', expected '{MD_NS}'"
        ));
        check1_ok = false;
    }
    let version_literal = root_version_literal(source, root);
    match classify_root_version(version_literal.as_deref()) {
        Ok(FormatCompatibility::Supported { .. }) => report.ok("Export format: 2.20"),
        Ok(compatibility) => report.warn(
            unica_source_designer::format_profile::format_compatibility_warning(&compatibility),
        ),
        Err(error) => report.error(error.to_string()),
    }
    let version = version_literal.as_deref().unwrap_or("");

    let Some(cfg_node) = root.children().find(|node| {
        unica_xml_format::names::element_has_name(*node, "Configuration", Some(MD_NS))
    }) else {
        report.error("1. No <Configuration> element found inside MetaDataObject");
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    };

    let cfg_uuid = cfg_node.attribute("uuid").unwrap_or("");
    if cfg_uuid.is_empty() {
        report.error("1. Missing uuid on <Configuration>");
        check1_ok = false;
    } else if !cf_validate_guid(cfg_uuid) {
        report.error(format!("1. Invalid uuid '{cfg_uuid}' on <Configuration>"));
        check1_ok = false;
    }

    let props_node = cfg_node
        .children()
        .find(|node| unica_xml_format::names::element_has_name(*node, "Properties", Some(MD_NS)));
    let name_node = props_node.and_then(|props| {
        props
            .children()
            .find(|node| unica_xml_format::names::element_has_name(*node, "Name", Some(MD_NS)))
    });
    let obj_name = name_node
        .and_then(|node| node.text())
        .filter(|value| !value.is_empty())
        .unwrap_or("(unknown)")
        .to_string();
    report.obj_name = obj_name.clone();

    if check1_ok {
        report.ok(format!(
            "1. Root structure: MetaDataObject/Configuration, version {version}"
        ));
    }
    if report.stopped {
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    }

    let internal_info = cfg_node
        .children()
        .find(|node| unica_xml_format::names::element_has_name(*node, "InternalInfo", Some(MD_NS)));
    if let Some(internal_info) = internal_info {
        let contained = internal_info
            .children()
            .filter(|node| {
                unica_xml_format::names::element_has_name(*node, "ContainedObject", Some(XR_NS))
            })
            .collect::<Vec<_>>();
        let mut check2_ok = true;
        if contained.len() != 7 {
            report.warn(format!(
                "2. InternalInfo: expected 7 ContainedObject, found {}",
                contained.len()
            ));
        }
        let mut found_class_ids = HashSet::new();
        for co in &contained {
            let class_id = cf_validate_child_text(*co, "ClassId", Some(XR_NS));
            let object_id = cf_validate_child_text(*co, "ObjectId", Some(XR_NS));
            if class_id.is_empty() {
                report.error("2. ContainedObject missing ClassId");
                check2_ok = false;
                continue;
            }
            if !cf_validate_class_ids().contains(&class_id.as_str()) {
                report.error(format!("2. Unknown ClassId: {class_id}"));
                check2_ok = false;
            }
            if !found_class_ids.insert(class_id.clone()) {
                report.error(format!("2. Duplicate ClassId: {class_id}"));
                check2_ok = false;
            }
            if object_id.is_empty() {
                report.error(format!(
                    "2. ContainedObject missing ObjectId for ClassId {class_id}"
                ));
                check2_ok = false;
            } else if !cf_validate_guid(&object_id) {
                report.error(format!(
                    "2. Invalid ObjectId '{object_id}' for ClassId {class_id}"
                ));
                check2_ok = false;
            }
        }
        let missing_ids = cf_validate_class_ids()
            .iter()
            .filter(|class_id| !found_class_ids.contains(**class_id))
            .count();
        if missing_ids > 0 {
            report.warn(format!("2. Missing ClassIds: {missing_ids} of 7"));
        }
        if check2_ok {
            report.ok(format!(
                "2. InternalInfo: {} ContainedObject, all ClassIds valid",
                contained.len()
            ));
        }
    } else {
        report.error("2. InternalInfo: missing");
    }
    if report.stopped {
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    }

    let mut def_lang = String::new();
    if let Some(props_node) = props_node {
        let mut check3_ok = true;
        if obj_name == "(unknown)" {
            report.error("3. Properties: Name is missing or empty");
            check3_ok = false;
        } else if !unica_metadata::identifier::is_1c_identifier(&obj_name) {
            report.error(format!(
                "3. Properties: Name '{obj_name}' is not a valid 1C identifier"
            ));
            check3_ok = false;
        }

        let syn_present = props_node
            .children()
            .find(|node| unica_xml_format::names::element_has_name(*node, "Synonym", Some(MD_NS)))
            .map(|syn_node| !unica_xml_format::read::multilang_text(syn_node).is_empty())
            .unwrap_or(false);
        def_lang = cf_validate_child_text(props_node, "DefaultLanguage", Some(MD_NS));
        if def_lang.is_empty() {
            report.error("3. Properties: DefaultLanguage is missing or empty");
            check3_ok = false;
        }
        let default_run = cf_validate_child_text(props_node, "DefaultRunMode", Some(MD_NS));
        if default_run.is_empty() {
            report.warn("3. Properties: DefaultRunMode is missing or empty");
        }
        if check3_ok {
            let syn_info = if syn_present {
                "Synonym present"
            } else {
                "no Synonym"
            };
            report.ok(format!(
                "3. Properties: Name=\"{obj_name}\", {syn_info}, DefaultLanguage={def_lang}"
            ));
        }

        let mut enum_checked = 0usize;
        let mut check4_ok = true;
        for property in cf_validate_enum_properties() {
            let value = cf_validate_child_text(props_node, property, Some(MD_NS));
            if !value.is_empty() {
                let allowed = cf_validate_enum_allowed(property);
                if !allowed.contains(&value.as_str()) {
                    report.error(format!(
                        "4. Property '{property}' has invalid value '{value}'"
                    ));
                    check4_ok = false;
                }
                enum_checked += 1;
            }
        }
        let mut boolean_checked = 0usize;
        for property in cf_validate_boolean_properties() {
            let Some(property_node) = props_node.children().find(|node| {
                unica_xml_format::names::element_has_name(*node, property, Some(MD_NS))
            }) else {
                continue;
            };
            let value = property_node.text().unwrap_or("");
            if !matches!(value, "true" | "false") {
                report.error(format!(
                        "4. Property '{property}' has invalid boolean value '{value}'; expected one of: true, false"
                    ));
                check4_ok = false;
            }
            boolean_checked += 1;
        }
        if check4_ok {
            report.ok(format!(
                    "4. Property values: {enum_checked} enum and {boolean_checked} boolean properties checked"
                ));
        }
    } else {
        report.error("3. Properties block missing");
        report.warn("4. No Properties block to check");
    }
    if report.stopped {
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    }

    let child_obj_node = cfg_node
        .children()
        .find(|node| unica_xml_format::names::element_has_name(*node, "ChildObjects", Some(MD_NS)));
    if let Some(child_obj_node) = child_obj_node {
        let mut check5_ok = true;
        let mut total_count = 0usize;
        let mut type_counts: Vec<(String, HashSet<String>)> = Vec::new();
        let mut type_first_index = HashSet::new();
        let mut last_type_order = -1isize;
        let mut order_ok = true;
        for child in child_obj_node.children().filter(|node| node.is_element()) {
            let type_name = child.tag_name().name().to_string();
            let object_name = child.text().unwrap_or("").to_string();
            if let Some(type_index) =
                unica_source_designer::registration::cf_validate_child_object_type_index(&type_name)
            {
                if type_first_index.insert(type_name.clone()) {
                    if (type_index as isize) < last_type_order {
                        report.warn(format!(
                                "5. Type '{type_name}' is out of canonical order (after type at position {last_type_order})"
                            ));
                        order_ok = false;
                    }
                    last_type_order = type_index as isize;
                }
            } else {
                report.error(format!("5. Unknown type '{type_name}' in ChildObjects"));
                check5_ok = false;
            }

            let existing = type_counts
                .iter_mut()
                .find(|(name, _)| name == &type_name)
                .map(|(_, names)| names);
            if let Some(names) = existing {
                if !names.insert(object_name.clone()) {
                    report.error(format!("5. Duplicate: {type_name}.{object_name}"));
                    check5_ok = false;
                }
            } else {
                let mut names = HashSet::new();
                names.insert(object_name);
                type_counts.push((type_name, names));
            }
            total_count += 1;
        }
        if check5_ok {
            let order_info = if order_ok { ", order correct" } else { "" };
            report.ok(format!(
                "5. ChildObjects: {} types, {total_count} objects{order_info}",
                type_counts.len()
            ));
        }

        if !def_lang.is_empty() {
            let lang_name = def_lang.strip_prefix("Language.").unwrap_or(&def_lang);
            let found = child_obj_node.children().any(|child| {
                unica_xml_format::names::element_has_name(child, "Language", Some(MD_NS))
                    && child.text().unwrap_or("") == lang_name
            });
            if found {
                report.ok(format!(
                    "6. DefaultLanguage \"{def_lang}\" found in ChildObjects"
                ));
            } else {
                report.error(format!(
                    "6. DefaultLanguage \"{def_lang}\" not found in ChildObjects"
                ));
            }
        } else {
            report.warn("6. Cannot check DefaultLanguage (empty)");
        }

        let lang_names = child_obj_node
            .children()
            .filter(|child| {
                unica_xml_format::names::element_has_name(*child, "Language", Some(MD_NS))
            })
            .map(|child| child.text().unwrap_or("").to_string())
            .collect::<Vec<_>>();
        if lang_names.is_empty() {
            report.warn("7. No Language entries in ChildObjects");
        } else {
            let mut exist_count = 0usize;
            for lang_name in &lang_names {
                let lang_file = config_dir
                    .join("Languages")
                    .join(format!("{lang_name}.xml"));
                if path_exists(reader, &lang_file) {
                    exist_count += 1;
                } else {
                    report.warn(format!(
                        "7. Language file missing: Languages/{lang_name}.xml"
                    ));
                }
            }
            if exist_count == lang_names.len() {
                report.ok(format!(
                    "7. Language files: {exist_count}/{} exist",
                    lang_names.len()
                ));
            }
        }

        let mut dirs_to_check = Vec::<(String, usize)>::new();
        for child in child_obj_node.children().filter(|node| node.is_element()) {
            let type_name = child.tag_name().name();
            if type_name == "Language" {
                continue;
            }
            if let Some(dir_name) = cf_validate_child_type_dir(type_name) {
                if let Some((_, count)) = dirs_to_check
                    .iter_mut()
                    .find(|(existing, _)| existing == dir_name)
                {
                    *count += 1;
                } else {
                    dirs_to_check.push((dir_name.to_string(), 1));
                }
            }
        }
        let missing_dirs = dirs_to_check
            .iter()
            .filter(|(dir_name, _)| !path_is_dir(reader, &config_dir.join(dir_name)))
            .map(|(dir_name, count)| format!("{dir_name} ({count} objects)"))
            .collect::<Vec<_>>();
        if missing_dirs.is_empty() {
            report.ok(format!(
                "8. Object directories: {} directories, all exist",
                dirs_to_check.len()
            ));
        } else {
            for missing in missing_dirs {
                report.warn(format!("8. Missing directory: {missing}"));
            }
        }
    } else {
        report.error("5. ChildObjects block missing");
        if def_lang.is_empty() {
            report.warn("6. Cannot check DefaultLanguage (empty)");
        } else {
            report.warn("6. Cannot check DefaultLanguage (no ChildObjects)");
        }
        report.warn("7. Cannot check language files (no ChildObjects)");
    }
    if report.stopped {
        let (ok, stdout, errors) = report.finalize();
        return Ok(CfValidationRun {
            ok,
            stdout,
            artifact: resolved_path,
            errors,
        });
    }

    let mut form_refs_checked = 0usize;
    let mut form_ref_errors = Vec::new();
    let home_page = config_dir.join("Ext").join("HomePageWorkArea.xml");
    if !owner_shape_only && reader.path_is_file(&home_page) {
        match read_utf8_sig(reader, &home_page) {
            Ok(home_page_text) => {
                match Document::parse(home_page_text.trim_start_matches('\u{feff}')) {
                    Ok(hp_doc) => {
                        for form_node in hp_doc.descendants().filter(|node| {
                            unica_xml_format::names::element_has_name(*node, "Form", Some(HP_NS))
                        }) {
                            let form_ref = form_node.text().unwrap_or("").trim();
                            if form_ref.is_empty() {
                                continue;
                            }
                            form_refs_checked += 1;
                            if !cf_validate_form_ref(reader, config_dir, form_ref) {
                                form_ref_errors.push(format!(
                                    "HomePageWorkArea.Form '{form_ref}' — file not found"
                                ));
                            }
                        }
                    }
                    Err(err) => {
                        form_ref_errors.push(format!("HomePageWorkArea.xml: parse error — {err}"))
                    }
                }
            }
            Err(err) => {
                form_ref_errors.push(format!("HomePageWorkArea.xml: parse error — {err}"));
            }
        }
    }
    if let Some(props_node) = props_node {
        for property in cf_validate_form_properties() {
            let form_ref = cf_validate_child_text(props_node, property, Some(MD_NS));
            if form_ref.trim().is_empty() {
                continue;
            }
            form_refs_checked += 1;
            if !cf_validate_form_ref(reader, config_dir, form_ref.trim()) {
                form_ref_errors.push(format!(
                    "Properties.{property} '{}' — form not found",
                    form_ref.trim()
                ));
            }
        }
    }
    if form_refs_checked == 0 {
        report.ok("9. Form references: none to check");
    } else if form_ref_errors.is_empty() {
        report.ok(format!("9. Form references: {form_refs_checked} verified"));
    } else {
        for error in form_ref_errors {
            report.error(format!("9. {error}"));
        }
    }

    let (ok, stdout, errors) = report.finalize();
    Ok(CfValidationRun {
        ok,
        stdout,
        artifact: resolved_path,
        errors,
    })
}

pub struct CfEditImagePlan {
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub stdout: String,
    pub operations: Vec<CfEditOperationData>,
    pub artifacts: Vec<PathBuf>,
    pub external_files: BTreeMap<PathBuf, Vec<u8>>,
    pub config_changed: bool,
}
pub fn cf_edit_plan_image(
    reader: &dyn ProjectDiscoveryRead,
    mut text: String,
    config_path: &Path,
    config_dir: &Path,
    operations: Vec<(String, Value)>,
    obj_name: &str,
    fresh_uuid: fn() -> String,
) -> Result<CfEditImagePlan, String> {
    let mut add_count = 0usize;
    let mut remove_count = 0usize;
    let mut modify_count = 0usize;
    let stdout = format!("[INFO] Configuration: {obj_name}\n");
    let mut operation_log: Vec<CfEditOperationData> = Vec::new();
    let mut artifacts = vec![config_path.to_path_buf()];
    let mut external_files = BTreeMap::<PathBuf, Vec<u8>>::new();
    let mut config_changed = false;

    for (op_name, op_value) in operations {
        match op_name.as_str() {
            "modify-property" => {
                for item in cf_edit_batch_value(&op_value) {
                    let Some(eq_idx) = item.find('=') else {
                        return Err(format!(
                            "Invalid property format '{item}', expected 'Key=Value'"
                        ));
                    };
                    if eq_idx < 1 {
                        return Err(format!(
                            "Invalid property format '{item}', expected 'Key=Value'"
                        ));
                    }
                    let prop_name = item[..eq_idx].trim();
                    let prop_value = item[eq_idx + 1..].trim();
                    cf_edit_validate_property_value(prop_name, prop_value)?;
                    let replacement = if cf_edit_ml_properties().contains(&prop_name) {
                        cf_edit_ml_property_xml(prop_name, prop_value)
                    } else {
                        cf_edit_scalar_property_xml(prop_name, prop_value)
                    };
                    text = cf_edit_replace_property(&text, prop_name, &replacement)?;
                    config_changed = true;
                    modify_count += 1;
                    operation_log.push(CfEditOperationData::applied(
                        "set-property",
                        format!("{prop_name}={prop_value}"),
                    ));
                }
            }
            "remove-childObject" => {
                for item in cf_edit_batch_value(&op_value) {
                    let (type_name, obj_name_val) = cf_edit_parse_child_object(&item)?;
                    if cf_edit_remove_child_object_text(&mut text, type_name, obj_name_val)? {
                        remove_count += 1;
                        config_changed = true;
                        operation_log.push(CfEditOperationData::applied(
                            "remove-childObject",
                            format!("{type_name}.{obj_name_val}"),
                        ));
                    } else {
                        operation_log.push(CfEditOperationData::skipped(
                            "remove-childObject",
                            format!("{type_name}.{obj_name_val}"),
                            "не найден в составе конфигурации",
                        ));
                    }
                }
            }
            "add-childObject" => {
                for item in cf_edit_batch_value(&op_value) {
                    let (type_name, obj_name_val) = cf_edit_parse_child_object(&item)?;
                    if unica_source_designer::registration::cf_validate_child_object_type_index(
                        type_name,
                    )
                    .is_none()
                    {
                        return Err(format!("Unknown type '{type_name}'"));
                    }
                    let type_dir = cf_validate_child_type_dir(type_name)
                        .ok_or_else(|| format!("Unknown type '{type_name}'"))?;
                    let object_file = config_dir
                        .join(type_dir)
                        .join(format!("{obj_name_val}.xml"));
                    if !path_exists(reader, &object_file) {
                        let creation_hint = match type_name {
                                "Subsystem" | "Role" => {
                                    let op = if type_name == "Subsystem" {
                                        "subsystem.create"
                                    } else {
                                        "role.create"
                                    };
                                    format!(
                                        "To create a new {type_name}, preview MCP unica.apply \
                                         (/unica:{skill}) with:\n  \
                                         {{\"at\":\"<sourceSet>:Configuration\",\"ops\":[{{\"op\":\"{op}\",\"args\":{{\"at\":\"<sourceSet>:Configuration\",\"values\":{{\"name\":\"{obj_name_val}\"}}}}}}]}}",
                                        skill = if type_name == "Subsystem" {
                                            "subsystem-edit"
                                        } else {
                                            "role-edit"
                                        },
                                    )
                                }
                                _ if MetadataKind::parse(type_name).is_ok() => format!(
                                    "To create a new {type_name}, call MCP unica.meta.add \
                                     (or /unica:meta-add) with:\n  \
                                     {{\"sourceSet\":\"<sourceSet>\",\"kind\":\"{type_name}\",\"name\":\"{obj_name_val}\",\"dryRun\":true}}"
                                ),
                                _ => format!(
                                    "Unica has no typed creation operation for {type_name}; \
                                     create the {type_name} metadata with platform tooling, then retry."
                                ),
                            };
                        return Err(format!(
                                "Object file not found: {type_dir}/{obj_name_val}.xml\n\
                                 cf-edit add-childObject only references objects that already exist on disk.\n\
                                 {creation_hint}"
                            ));
                    }
                    if unica_source_designer::registration::cf_edit_add_child_object_text(
                        &mut text,
                        type_name,
                        obj_name_val,
                    )? {
                        add_count += 1;
                        config_changed = true;
                        operation_log.push(CfEditOperationData::applied(
                            "add-childObject",
                            format!("{type_name}.{obj_name_val}"),
                        ));
                    } else {
                        operation_log.push(CfEditOperationData::skipped(
                            "add-childObject",
                            format!("{type_name}.{obj_name_val}"),
                            "уже присутствует в составе конфигурации",
                        ));
                    }
                }
            }
            "set-defaultRoles" => {
                let roles = cf_edit_batch_value(&op_value)
                    .into_iter()
                    .map(|role| cf_edit_role_ref(&role))
                    .collect::<Vec<_>>();
                text = cf_edit_replace_default_roles(&text, &roles)?;
                config_changed = true;
                modify_count += 1;
                operation_log.push(CfEditOperationData::applied(
                    "set-defaultRoles",
                    (!roles.is_empty()).then(|| roles.join(", ")),
                ));
            }
            "add-defaultRole" => {
                let mut roles = cf_edit_default_roles(&text)?;
                for role in cf_edit_batch_value(&op_value) {
                    let role_name = cf_edit_role_ref(&role);
                    if roles.contains(&role_name) {
                        operation_log.push(CfEditOperationData::skipped(
                            "add-defaultRole",
                            role_name.clone(),
                            "уже в списке ролей по умолчанию",
                        ));
                    } else {
                        roles.push(role_name.clone());
                        add_count += 1;
                        operation_log.push(CfEditOperationData::applied(
                            "add-defaultRole",
                            role_name.clone(),
                        ));
                    }
                }
                text = cf_edit_replace_default_roles(&text, &roles)?;
                config_changed = true;
            }
            "remove-defaultRole" => {
                let mut roles = cf_edit_default_roles(&text)?;
                for role in cf_edit_batch_value(&op_value) {
                    let role_name = cf_edit_role_ref(&role);
                    if let Some(index) = roles.iter().position(|existing| existing == &role_name) {
                        roles.remove(index);
                        remove_count += 1;
                        operation_log.push(CfEditOperationData::applied(
                            "remove-defaultRole",
                            role_name.clone(),
                        ));
                    } else {
                        operation_log.push(CfEditOperationData::skipped(
                            "remove-defaultRole",
                            role_name.clone(),
                            "не найдена в списке ролей по умолчанию",
                        ));
                    }
                }
                text = cf_edit_replace_default_roles(&text, &roles)?;
                config_changed = true;
            }
            "set-panels" => {
                let plan = cf_edit_set_panels(&op_value, config_dir, fresh_uuid)?;
                let path = plan.path.clone();
                external_files.insert(plan.path, plan.bytes);
                modify_count += 1;
                operation_log.push(CfEditOperationData::applied(
                    "set-panels",
                    path.display().to_string(),
                ));
                artifacts.push(path);
            }
            "set-home-page" => {
                let plan = cf_edit_set_home_page(&op_value, config_dir)?;
                let path = plan.path.clone();
                external_files.insert(plan.path, plan.bytes);
                modify_count += 1;
                operation_log.push(CfEditOperationData::applied(
                    "set-home-page",
                    path.display().to_string(),
                ));
                artifacts.push(path);
            }
            _ => return Err(format!("Unknown operation: {op_name}")),
        }
    }

    Ok(CfEditImagePlan {
        text,
        added: add_count,
        removed: remove_count,
        modified: modify_count,
        stdout,
        operations: operation_log,
        artifacts,
        external_files,
        config_changed,
    })
}

pub struct CfInitImages {
    pub configuration: String,
    pub language: String,
    pub client_application_interface: String,
}
pub fn cf_init_images(
    args: &Map<String, Value>,
    name: &str,
    synonym: &str,
    compatibility: &str,
    fresh_uuid: fn() -> String,
) -> CfInitImages {
    let uuid_cfg = fresh_uuid();
    let uuid_lang = unica_xml_format::names::stable_uuid(1);
    let contained_object_ids = (2..9)
        .map(unica_xml_format::names::stable_uuid)
        .collect::<Vec<_>>();
    let open_panel_inst = unica_xml_format::names::stable_uuid(9);
    let sections_panel_inst = unica_xml_format::names::stable_uuid(10);
    let compatibility_xml = unica_xml_format::text::escape_xml(compatibility);
    let vendor_xml = unica_operation_contract::input::string_arg(args, &["vendor", "Vendor"])
        .map(unica_xml_format::text::escape_xml)
        .unwrap_or_default();
    let version_xml = unica_operation_contract::input::string_arg(args, &["version", "Version"])
        .map(unica_xml_format::text::escape_xml)
        .unwrap_or_default();
    let synonym_xml = format!(
            "\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content>{}</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t",
            unica_xml_format::text::escape_xml(synonym)
        );
    let mobile_xml = mobile_functionality_xml();
    let contained_objects = contained_objects_xml(&contained_object_ids);
    let format_version = ACTIVE_FORMAT_PROFILE.export_format;

    let config_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version}">
	<Configuration uuid="{uuid_cfg}">
		<InternalInfo>
{contained_objects}		</InternalInfo>
		<Properties>
			<Name>{name}</Name>
			<Synonym>{synonym_xml}</Synonym>
			<Comment/>
			<NamePrefix/>
			<ConfigurationExtensionCompatibilityMode>{compatibility_xml}</ConfigurationExtensionCompatibilityMode>
			<DefaultRunMode>ManagedApplication</DefaultRunMode>
			<UsePurposes>
				<v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value>
			</UsePurposes>
			<ScriptVariant>Russian</ScriptVariant>
			<DefaultRoles/>
			<Vendor>{vendor_xml}</Vendor>
			<Version>{version_xml}</Version>
			<UpdateCatalogAddress/>
			<IncludeHelpInContents>false</IncludeHelpInContents>
			<UseManagedFormInOrdinaryApplication>false</UseManagedFormInOrdinaryApplication>
			<UseOrdinaryFormInManagedApplication>false</UseOrdinaryFormInManagedApplication>
			<AdditionalFullTextSearchDictionaries/>
			<CommonSettingsStorage/>
			<ReportsUserSettingsStorage/>
			<ReportsVariantsStorage/>
			<FormDataSettingsStorage/>
			<DynamicListsUserSettingsStorage/>
			<URLExternalDataStorage/>
			<Content/>
			<DefaultReportForm/>
			<DefaultReportVariantForm/>
			<DefaultReportSettingsForm/>
			<DefaultReportAppearanceTemplate/>
			<DefaultDynamicListSettingsForm/>
			<DefaultSearchForm/>
			<DefaultDataHistoryChangeHistoryForm/>
			<DefaultDataHistoryVersionDataForm/>
			<DefaultDataHistoryVersionDifferencesForm/>
			<DefaultCollaborationSystemUsersChoiceForm/>
			<RequiredMobileApplicationPermissions/>
			<UsedMobileApplicationFunctionalities>{mobile_xml}
			</UsedMobileApplicationFunctionalities>
			<StandaloneConfigurationRestrictionRoles/>
			<MobileApplicationURLs/>
			<AllowedIncomingShareRequestTypes/>
			<MainClientApplicationWindowMode>Normal</MainClientApplicationWindowMode>
			<DefaultInterface/>
			<DefaultStyle/>
			<DefaultLanguage>Language.Русский</DefaultLanguage>
			<BriefInformation/>
			<DetailedInformation/>
			<Copyright/>
			<VendorInformationAddress/>
			<ConfigurationInformationAddress/>
			<DataLockControlMode>Managed</DataLockControlMode>
			<ObjectAutonumerationMode>NotAutoFree</ObjectAutonumerationMode>
			<ModalityUseMode>DontUse</ModalityUseMode>
			<SynchronousPlatformExtensionAndAddInCallUseMode>DontUse</SynchronousPlatformExtensionAndAddInCallUseMode>
			<InterfaceCompatibilityMode>TaxiEnableVersion8_2</InterfaceCompatibilityMode>
			<DatabaseTablespacesUseMode>DontUse</DatabaseTablespacesUseMode>
			<CompatibilityMode>{compatibility_xml}</CompatibilityMode>
			<DefaultConstantsForm/>
		</Properties>
		<ChildObjects>
			<Language>Русский</Language>
		</ChildObjects>
	</Configuration>
</MetaDataObject>"#,
        name = unica_xml_format::text::escape_xml(name),
    );
    let language_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version}">
	<Language uuid="{uuid_lang}">
		<Properties>
			<Name>Русский</Name>
			<Synonym>
				<v8:item>
					<v8:lang>ru</v8:lang>
					<v8:content>Русский</v8:content>
				</v8:item>
			</Synonym>
			<Comment/>
			<LanguageCode>ru</LanguageCode>
		</Properties>
	</Language>
</MetaDataObject>"#
    );
    let cai_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="InterfaceLayouter">
	<top>
		<panel id="{open_panel_inst}">
			<uuid>cbab57f2-a0f3-4f0a-89ea-4cb19570ab75</uuid>
		</panel>
	</top>
	<left>
		<panel id="{sections_panel_inst}">
			<uuid>b553047f-c9aa-4157-978d-448ecad24248</uuid>
		</panel>
	</left>
	<panelDef id="b553047f-c9aa-4157-978d-448ecad24248"/>
	<panelDef id="13322b22-3960-4d68-93a6-fe2dd7f28ca3"/>
	<panelDef id="c933ac92-92cd-459d-81cc-e0c8a83ced99"/>
	<panelDef id="cbab57f2-a0f3-4f0a-89ea-4cb19570ab75"/>
	<panelDef id="b2735bd3-d822-4430-ba59-c9e869693b24"/>
</ClientApplicationInterface>"#
    );

    CfInitImages {
        configuration: config_xml,
        language: language_xml,
        client_application_interface: cai_xml,
    }
}

#[cfg(test)]
#[path = "../tests/platform/configuration_codec.rs"]
mod tests;
