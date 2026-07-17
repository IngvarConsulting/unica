//! Versioned platform identity registry shared by discovery and infrastructure.
//!
//! Canonical artifact parsing and Platform XML providers must use this single
//! registry so they cannot silently disagree about object or module kinds.

pub(crate) const DISCOVERY_REGISTRY_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataKind {
    pub(crate) tag: &'static str,
    pub(crate) directory: &'static str,
    pub(crate) display_name_ru: &'static str,
    // ConfigDumpInfo is not mutated today; optional fields record only established facts.
    #[allow(dead_code)]
    pub(crate) config_dump_prefix: Option<&'static str>,
    #[allow(dead_code)]
    pub(crate) config_dump_module_suffix: Option<&'static str>,
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
        pub(crate) const METADATA_KINDS: &[MetadataKind] = &[
            $(MetadataKind {
                tag: $tag,
                directory: $directory,
                display_name_ru: $display_name_ru,
                config_dump_prefix: $config_dump_prefix,
                config_dump_module_suffix: $config_dump_module_suffix,
            }),+
        ];

        pub(crate) const METADATA_KIND_TAGS: &[&str] = &[$($tag),+];
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

pub(crate) const MODULE_KIND_TAGS: &[&str] = &[
    "Module",
    "ObjectModule",
    "ManagerModule",
    "RecordSetModule",
    "ValueManagerModule",
    "CommandModule",
];

pub(crate) fn metadata_kind(tag: &str) -> Option<&'static MetadataKind> {
    METADATA_KINDS.iter().find(|kind| kind.tag == tag)
}

pub(crate) fn metadata_kind_by_directory(directory: &str) -> Option<&'static MetadataKind> {
    METADATA_KINDS
        .iter()
        .find(|kind| kind.directory.eq_ignore_ascii_case(directory))
}

pub(crate) fn metadata_kind_index(tag: &str) -> Option<usize> {
    METADATA_KINDS.iter().position(|kind| kind.tag == tag)
}

pub(crate) fn module_kind(tag: &str) -> Option<&'static str> {
    MODULE_KIND_TAGS.iter().copied().find(|kind| *kind == tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_has_unique_canonical_tags_and_directories() {
        assert_eq!(DISCOVERY_REGISTRY_VERSION, 1);
        assert_eq!(METADATA_KINDS.len(), 45);
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
        assert_eq!(
            MODULE_KIND_TAGS
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len(),
            MODULE_KIND_TAGS.len()
        );
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
        assert_eq!(module_kind("ObjectModule"), Some("ObjectModule"));
        assert_eq!(module_kind("SyntheticModule"), None);
    }
}
