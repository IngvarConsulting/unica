//! Existing configuration physical layout atoms over the sole Designer kind table.
use crate::layout::{metadata_kind, metadata_kind_by_directory, METADATA_KIND_TAGS};
use std::path::{Path, PathBuf};

pub fn cf_validate_child_object_types() -> &'static [&'static str] {
    METADATA_KIND_TAGS
}

pub fn cf_validate_child_type_dir(type_name: &str) -> Option<&'static str> {
    metadata_kind(type_name).map(|kind| kind.directory)
}

pub fn cf_type_order() -> &'static [&'static str] {
    METADATA_KIND_TAGS
}

pub fn cf_type_ru_name(type_name: &str) -> &'static str {
    metadata_kind(type_name).map_or("Unknown", |kind| kind.display_name_ru)
}

pub fn cf_edit_child_object_cmp(
    left: &(String, String),
    right: &(String, String),
) -> std::cmp::Ordering {
    let left_idx =
        crate::registration::cf_validate_child_object_type_index(&left.0).unwrap_or(usize::MAX);
    let right_idx =
        crate::registration::cf_validate_child_object_type_index(&right.0).unwrap_or(usize::MAX);
    left_idx.cmp(&right_idx).then_with(|| left.1.cmp(&right.1))
}

pub fn cf_edit_ru_type(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "справочник" => Some("Catalog"),
        "документ" => Some("Document"),
        "перечисление" => Some("Enum"),
        "отчёт" | "отчет" => Some("Report"),
        "обработка" => Some("DataProcessor"),
        "общаяформа" => Some("CommonForm"),
        "журналдокументов" => Some("DocumentJournal"),
        "планвидовхарактеристик" => Some("ChartOfCharacteristicTypes"),
        "плансчетов" => Some("ChartOfAccounts"),
        "планвидоврасчета" | "планвидоврасчёта" => {
            Some("ChartOfCalculationTypes")
        }
        "регистрсведений" => Some("InformationRegister"),
        "регистрнакопления" => Some("AccumulationRegister"),
        "регистрбухгалтерии" => Some("AccountingRegister"),
        "регистррасчета" | "регистррасчёта" => {
            Some("CalculationRegister")
        }
        "бизнеспроцесс" => Some("BusinessProcess"),
        "задача" => Some("Task"),
        "планобмена" => Some("ExchangePlan"),
        "хранилищенастроек" => Some("SettingsStorage"),
        _ => None,
    }
}

pub fn cf_edit_dir_to_type(value: &str) -> Option<&'static str> {
    metadata_kind_by_directory(value).map(|kind| kind.tag)
}

pub fn cfe_borrow_type_dir(type_name: &str) -> Option<&'static str> {
    cf_validate_child_type_dir(type_name)
}

pub fn cfe_borrow_target_object(ext_dir: &Path, type_name: &str, object_name: &str) -> PathBuf {
    let dir_name = cfe_borrow_type_dir(type_name).unwrap_or(type_name);
    ext_dir.join(dir_name).join(format!("{object_name}.xml"))
}

pub fn cfe_borrow_type_has_child_objects(type_name: &str) -> Result<bool, String> {
    crate::layout::metadata_kind_requires_child_objects_8_3_27(type_name)
        .ok_or_else(|| format!("Type '{type_name}' has no 8.3.27 ChildObjects profile"))
}

#[cfg(test)]
#[path = "../tests/platform/configuration_layout.rs"]
mod tests;

/// Classifies the exact already captured image used by the trusted final gate.
pub fn classify_configuration_xml_image(
    path: &Path,
    raw: &[u8],
) -> Result<crate::format_profile::FormatCompatibility, String> {
    let text = std::str::from_utf8(raw)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    let source = text.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(source)
        .map_err(|error| format!("failed to parse platform XML {}: {error}", path.display()))?;
    let version_literal = crate::roots::root_version_literal(source, document.root_element());
    crate::format_profile::classify_root_version(version_literal.as_deref())
        .map_err(|error| error.to_string())
}
