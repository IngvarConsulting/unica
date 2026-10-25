use std::path::{Path, PathBuf};
use unica_address::source_target::MetadataAddress;
use unica_source_contract::SourceSetKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataLayout {
    pub tag: &'static str,
    pub directory: &'static str,
    pub display_name_ru: &'static str,
    // ConfigDumpInfo is not mutated today; optional fields record only established facts.
    #[allow(dead_code)]
    pub config_dump_prefix: Option<&'static str>,
    #[allow(dead_code)]
    pub config_dump_module_suffix: Option<&'static str>,
}

macro_rules! metadata_kind_registry {
    ($(
        $tag:literal => {
            directory: $directory:literal,
            display_name_ru: $display_name_ru:literal,
            config_dump_prefix: $config_dump_prefix:expr,
            config_dump_module_suffix: $config_dump_module_suffix:expr $(,)?
        }
    ),+ $(,)?) => {
        pub const METADATA_KINDS: &[MetadataLayout] = &[
            $(MetadataLayout {
                tag: $tag,
                directory: $directory,
                display_name_ru: $display_name_ru,
                config_dump_prefix: $config_dump_prefix,
                config_dump_module_suffix: $config_dump_module_suffix,
            }),+
        ];

        pub const METADATA_KIND_TAGS: &[&str] = &[$($tag),+];
    };
}

metadata_kind_registry! {
    "Language" => { directory: "Languages", display_name_ru: "Языки", config_dump_prefix: None, config_dump_module_suffix: None },
    "Subsystem" => { directory: "Subsystems", display_name_ru: "Подсистемы", config_dump_prefix: None, config_dump_module_suffix: None },
    "StyleItem" => { directory: "StyleItems", display_name_ru: "Элементы стиля", config_dump_prefix: None, config_dump_module_suffix: None },
    "Style" => { directory: "Styles", display_name_ru: "Стили", config_dump_prefix: None, config_dump_module_suffix: None },
    "CommonPicture" => { directory: "CommonPictures", display_name_ru: "Общие картинки", config_dump_prefix: None, config_dump_module_suffix: None },
    "SessionParameter" => { directory: "SessionParameters", display_name_ru: "Параметры сеанса", config_dump_prefix: None, config_dump_module_suffix: None },
    "Role" => { directory: "Roles", display_name_ru: "Роли", config_dump_prefix: None, config_dump_module_suffix: None },
    "CommonTemplate" => { directory: "CommonTemplates", display_name_ru: "Общие макеты", config_dump_prefix: None, config_dump_module_suffix: None },
    "FilterCriterion" => { directory: "FilterCriteria", display_name_ru: "Критерии отбора", config_dump_prefix: None, config_dump_module_suffix: None },
    "CommonModule" => { directory: "CommonModules", display_name_ru: "Общие модули", config_dump_prefix: None, config_dump_module_suffix: None },
    "Bot" => { directory: "Bots", display_name_ru: "Боты", config_dump_prefix: Some("Bot"), config_dump_module_suffix: Some(".Module") },
    "CommonAttribute" => { directory: "CommonAttributes", display_name_ru: "Общие реквизиты", config_dump_prefix: None, config_dump_module_suffix: None },
    "ExchangePlan" => { directory: "ExchangePlans", display_name_ru: "Планы обмена", config_dump_prefix: None, config_dump_module_suffix: None },
    "XDTOPackage" => { directory: "XDTOPackages", display_name_ru: "XDTO-пакеты", config_dump_prefix: None, config_dump_module_suffix: None },
    "WebService" => { directory: "WebServices", display_name_ru: "Веб-сервисы", config_dump_prefix: None, config_dump_module_suffix: None },
    "HTTPService" => { directory: "HTTPServices", display_name_ru: "HTTP-сервисы", config_dump_prefix: None, config_dump_module_suffix: None },
    "WSReference" => { directory: "WSReferences", display_name_ru: "WS-ссылки", config_dump_prefix: None, config_dump_module_suffix: None },
    "EventSubscription" => { directory: "EventSubscriptions", display_name_ru: "Подписки на события", config_dump_prefix: None, config_dump_module_suffix: None },
    "ScheduledJob" => { directory: "ScheduledJobs", display_name_ru: "Регламентные задания", config_dump_prefix: None, config_dump_module_suffix: None },
    "SettingsStorage" => { directory: "SettingsStorages", display_name_ru: "Хранилища настроек", config_dump_prefix: None, config_dump_module_suffix: None },
    "FunctionalOption" => { directory: "FunctionalOptions", display_name_ru: "Функциональные опции", config_dump_prefix: None, config_dump_module_suffix: None },
    "FunctionalOptionsParameter" => { directory: "FunctionalOptionsParameters", display_name_ru: "Параметры ФО", config_dump_prefix: None, config_dump_module_suffix: None },
    "DefinedType" => { directory: "DefinedTypes", display_name_ru: "Определяемые типы", config_dump_prefix: None, config_dump_module_suffix: None },
    "CommonCommand" => { directory: "CommonCommands", display_name_ru: "Общие команды", config_dump_prefix: None, config_dump_module_suffix: None },
    "CommandGroup" => { directory: "CommandGroups", display_name_ru: "Группы команд", config_dump_prefix: None, config_dump_module_suffix: None },
    "Constant" => { directory: "Constants", display_name_ru: "Константы", config_dump_prefix: None, config_dump_module_suffix: None },
    "CommonForm" => { directory: "CommonForms", display_name_ru: "Общие формы", config_dump_prefix: None, config_dump_module_suffix: None },
    "Catalog" => { directory: "Catalogs", display_name_ru: "Справочники", config_dump_prefix: None, config_dump_module_suffix: None },
    "Document" => { directory: "Documents", display_name_ru: "Документы", config_dump_prefix: None, config_dump_module_suffix: None },
    "DocumentNumerator" => { directory: "DocumentNumerators", display_name_ru: "Нумераторы", config_dump_prefix: None, config_dump_module_suffix: None },
    "Sequence" => { directory: "Sequences", display_name_ru: "Последовательности", config_dump_prefix: None, config_dump_module_suffix: None },
    "DocumentJournal" => { directory: "DocumentJournals", display_name_ru: "Журналы документов", config_dump_prefix: None, config_dump_module_suffix: None },
    "Enum" => { directory: "Enums", display_name_ru: "Перечисления", config_dump_prefix: None, config_dump_module_suffix: None },
    "Report" => { directory: "Reports", display_name_ru: "Отчёты", config_dump_prefix: None, config_dump_module_suffix: None },
    "DataProcessor" => { directory: "DataProcessors", display_name_ru: "Обработки", config_dump_prefix: None, config_dump_module_suffix: None },
    "InformationRegister" => { directory: "InformationRegisters", display_name_ru: "Регистры сведений", config_dump_prefix: None, config_dump_module_suffix: None },
    "AccumulationRegister" => { directory: "AccumulationRegisters", display_name_ru: "Регистры накопления", config_dump_prefix: None, config_dump_module_suffix: None },
    "ChartOfCharacteristicTypes" => { directory: "ChartsOfCharacteristicTypes", display_name_ru: "ПВХ", config_dump_prefix: None, config_dump_module_suffix: None },
    "ChartOfAccounts" => { directory: "ChartsOfAccounts", display_name_ru: "Планы счетов", config_dump_prefix: None, config_dump_module_suffix: None },
    "AccountingRegister" => { directory: "AccountingRegisters", display_name_ru: "Регистры бухгалтерии", config_dump_prefix: None, config_dump_module_suffix: None },
    "ChartOfCalculationTypes" => { directory: "ChartsOfCalculationTypes", display_name_ru: "ПВР", config_dump_prefix: None, config_dump_module_suffix: None },
    "CalculationRegister" => { directory: "CalculationRegisters", display_name_ru: "Регистры расчёта", config_dump_prefix: None, config_dump_module_suffix: None },
    "BusinessProcess" => { directory: "BusinessProcesses", display_name_ru: "Бизнес-процессы", config_dump_prefix: None, config_dump_module_suffix: None },
    "Task" => { directory: "Tasks", display_name_ru: "Задачи", config_dump_prefix: None, config_dump_module_suffix: None },
    "IntegrationService" => { directory: "IntegrationServices", display_name_ru: "Сервисы интеграции", config_dump_prefix: None, config_dump_module_suffix: None },
}

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

/// Whether platform 8.3.27 requires a root `ChildObjects` container for this
/// physical metadata kind, including when the collection is empty.
///
/// `None` keeps an unknown future kind from being silently treated as
/// childless. This profile covers the complete physical registry used by
/// `cfe.borrow`, which is wider than the typed [`MetadataKind`] surface.
pub fn metadata_kind_requires_child_objects_8_3_27(tag: &str) -> Option<bool> {
    match tag {
        "Subsystem"
        | "FilterCriterion"
        | "ExchangePlan"
        | "WebService"
        | "HTTPService"
        | "SettingsStorage"
        | "Catalog"
        | "Document"
        | "Sequence"
        | "DocumentJournal"
        | "Enum"
        | "Report"
        | "DataProcessor"
        | "InformationRegister"
        | "AccumulationRegister"
        | "ChartOfCharacteristicTypes"
        | "ChartOfAccounts"
        | "AccountingRegister"
        | "ChartOfCalculationTypes"
        | "CalculationRegister"
        | "BusinessProcess"
        | "Task"
        | "IntegrationService" => Some(true),
        "Language"
        | "StyleItem"
        | "Style"
        | "CommonPicture"
        | "SessionParameter"
        | "Role"
        | "CommonTemplate"
        | "CommonModule"
        | "Bot"
        | "CommonAttribute"
        | "XDTOPackage"
        | "WSReference"
        | "EventSubscription"
        | "ScheduledJob"
        | "FunctionalOption"
        | "FunctionalOptionsParameter"
        | "DefinedType"
        | "CommonCommand"
        | "CommandGroup"
        | "Constant"
        | "CommonForm"
        | "DocumentNumerator" => Some(false),
        _ => None,
    }
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
            || (*owner_kind == "XDTOPackage" && resource == "Package.bin") =>
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

pub fn full_md_namespace_declarations() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
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
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn registry_has_unique_canonical_tags_and_directories() {
        const EXPECTED_TAGS: &[&str] = &[
            "Language",
            "Subsystem",
            "StyleItem",
            "Style",
            "CommonPicture",
            "SessionParameter",
            "Role",
            "CommonTemplate",
            "FilterCriterion",
            "CommonModule",
            "Bot",
            "CommonAttribute",
            "ExchangePlan",
            "XDTOPackage",
            "WebService",
            "HTTPService",
            "WSReference",
            "EventSubscription",
            "ScheduledJob",
            "SettingsStorage",
            "FunctionalOption",
            "FunctionalOptionsParameter",
            "DefinedType",
            "CommonCommand",
            "CommandGroup",
            "Constant",
            "CommonForm",
            "Catalog",
            "Document",
            "DocumentNumerator",
            "Sequence",
            "DocumentJournal",
            "Enum",
            "Report",
            "DataProcessor",
            "InformationRegister",
            "AccumulationRegister",
            "ChartOfCharacteristicTypes",
            "ChartOfAccounts",
            "AccountingRegister",
            "ChartOfCalculationTypes",
            "CalculationRegister",
            "BusinessProcess",
            "Task",
            "IntegrationService",
        ];

        assert_eq!(METADATA_KINDS.len(), 45);
        assert_eq!(METADATA_KIND_TAGS, EXPECTED_TAGS);
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
            metadata_kind_index("CommonModule").map(|index| index + 1)
        );
        assert_eq!(metadata_kind("SyntheticMetadata"), None);
    }
}
