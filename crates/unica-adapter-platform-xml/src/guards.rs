use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use roxmltree::Document;
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityPort, AuthorabilityRequest, AuthorabilityRequirement, AuthorabilityResult,
        AuthorabilityViolation, CompatibilityIssue, CompatibilityIssueKind, CompatibilityPort,
        CompatibilityRequest, CompatibilityResult, FormatCompatibility, FormatDiagnostic,
        OwnerResolutionRequest, SourceCompatibilityEvidence, SourceCompatibilityPort,
        SourceCompatibilityRequest, SourceCompatibilityResult,
    },
    source::{FormatVersion, SourceAdapterError, SourceAdapterErrorKind, SourceFamily},
};

use crate::versions::v2_20;

pub(crate) struct PlatformXmlGuards;

impl CompatibilityPort for PlatformXmlGuards {
    fn inspect(
        &self,
        request: &CompatibilityRequest,
    ) -> Result<CompatibilityResult, SourceAdapterError> {
        let mut older = None;
        let mut newer = None;
        for target in &request.targets {
            let resolution = match crate::owner::resolve(&OwnerResolutionRequest {
                source: target.source.clone(),
                mode: target.mode,
            }) {
                Ok(resolution) => resolution,
                Err(error) => {
                    return Ok(CompatibilityResult {
                        issue: Some(CompatibilityIssue {
                            kind: CompatibilityIssueKind::Malformed,
                            diagnostic: invalid_diagnostic(
                                &target.source,
                                format!(
                                    "Некорректный корневой файл формата выгрузки: {}",
                                    error.message
                                ),
                            ),
                            actual_format: None,
                            target_format: Some(target_format()?),
                            producer_version: None,
                            source_kind: target.source.configured_source_set_kind(),
                        }),
                    });
                }
            };
            if resolution.owners.is_empty()
                && matches!(
                    target.mode,
                    unica_format_core::ports::OwnerResolutionMode::Existing
                )
                && matches!(
                    target.source.configured_source_set_kind(),
                    Some(
                        unica_format_core::source::ConfiguredSourceSetKind::Configuration
                            | unica_format_core::source::ConfiguredSourceSetKind::Extension
                    )
                )
            {
                return Ok(CompatibilityResult {
                    issue: Some(CompatibilityIssue {
                        kind: CompatibilityIssueKind::Malformed,
                        diagnostic: invalid_diagnostic(
                            &target.source,
                            "Некорректный корневой файл формата выгрузки: Configuration.xml не найден"
                                .to_string(),
                        ),
                        actual_format: None,
                        target_format: Some(target_format()?),
                        producer_version: None,
                        source_kind: target.source.configured_source_set_kind(),
                    }),
                });
            }
            for owner in resolution.owners {
                if owner.configured_source_kind.is_none()
                    && is_versionless_embedded_content(&owner.path)
                {
                    continue;
                }
                match owner.format {
                    FormatCompatibility::Supported { .. } => {}
                    FormatCompatibility::Older { .. } if older.is_none() => {
                        older = Some(owner);
                    }
                    FormatCompatibility::Newer { .. } if newer.is_none() => {
                        newer = Some(owner);
                    }
                    FormatCompatibility::Older { .. } | FormatCompatibility::Newer { .. } => {}
                }
            }
        }
        let Some(owner) = newer.or(older) else {
            return Ok(CompatibilityResult { issue: None });
        };
        let actual = owner.format.actual().clone();
        let target = owner.format.target().clone();
        let (kind, code, message) = match owner.format {
            FormatCompatibility::Older { .. } => (
                CompatibilityIssueKind::Older,
                "formatMigrationAvailable",
                format!(
                    "Формат выгрузки {actual} старше поддерживаемого {target} для платформы 1С {}. Чтобы редактировать исходники, явно перенесите выгрузку средствами платформы 1С 8.3.27: загрузите исходники и повторно выгрузите их. Unica не выполняет эту миграцию автоматически.",
                    v2_20::PLATFORM_LINE
                ),
            ),
            FormatCompatibility::Newer { .. } => (
                CompatibilityIssueKind::Newer,
                "platformVersionUnsupported",
                format!(
                    "Формат выгрузки {actual} новее поддерживаемого {target} для платформы 1С {}. Unica пока не поддерживает работу с этой выгрузкой. Поддержка платформы 1С 8.5 планируется в ближайших версиях.",
                    v2_20::PLATFORM_LINE
                ),
            ),
            FormatCompatibility::Supported { .. } => unreachable!(),
        };
        let mut diagnostic = FormatDiagnostic::new(code, message);
        diagnostic
            .details
            .insert("actualFormat".to_string(), actual.to_string());
        diagnostic
            .details
            .insert("targetFormat".to_string(), target.to_string());
        diagnostic.details.insert(
            "targetPlatform".to_string(),
            v2_20::PLATFORM_LINE.to_string(),
        );
        diagnostic.details.insert(
            "compatibility".to_string(),
            match kind {
                CompatibilityIssueKind::Older => "older",
                CompatibilityIssueKind::Newer => "newer",
                CompatibilityIssueKind::Malformed => "invalid",
            }
            .to_string(),
        );
        diagnostic
            .details
            .insert("root".to_string(), owner.path.display().to_string());
        diagnostic.details.insert(
            "ownerKind".to_string(),
            owner
                .configured_source_kind
                .map(|kind| kind.label())
                .unwrap_or("standalone")
                .to_string(),
        );
        Ok(CompatibilityResult {
            issue: Some(CompatibilityIssue {
                kind,
                diagnostic,
                actual_format: Some(actual),
                target_format: Some(target),
                producer_version: owner.producer_version,
                source_kind: owner.configured_source_kind,
            }),
        })
    }
}

impl SourceCompatibilityPort for PlatformXmlGuards {
    fn inspect_source(
        &self,
        request: &SourceCompatibilityRequest,
    ) -> Result<SourceCompatibilityResult, SourceAdapterError> {
        let message = match &request.evidence {
            SourceCompatibilityEvidence::Detected {
                family: Some(SourceFamily::PlatformXml) | None,
                invalid: false,
                ..
            } => None,
            SourceCompatibilityEvidence::Detected {
                source_set_name,
                family: Some(SourceFamily::Edt),
                invalid: false,
            } => Some(format!(
                "{} targets source-set `{source_set_name}` with sourceFormat=edt; native platform XML tools require sourceFormat=platform_xml",
                request.operation_name
            )),
            SourceCompatibilityEvidence::Detected {
                source_set_name,
                invalid: true,
                ..
            }
            | SourceCompatibilityEvidence::Detected {
                source_set_name,
                family: Some(SourceFamily::Cf | SourceFamily::FileDatabase),
                invalid: false,
            } => Some(format!(
                "{} targets source-set `{source_set_name}` with invalid/ambiguous format; native platform XML tools require sourceFormat=platform_xml",
                request.operation_name
            )),
            SourceCompatibilityEvidence::DeclaredProjectFormat { value: None } => None,
            SourceCompatibilityEvidence::DeclaredProjectFormat { value: Some(value) }
                if value == "DESIGNER" =>
            {
                None
            }
            SourceCompatibilityEvidence::DeclaredProjectFormat { value: Some(value) }
                if value == "EDT" =>
            {
                Some(format!(
                    "{} requires v8project.yaml format=DESIGNER; format=EDT uses a different external-project layout",
                    request.operation_name
                ))
            }
            SourceCompatibilityEvidence::DeclaredProjectFormat { value: Some(value) } => {
                Some(format!(
                    "{} requires v8project.yaml format to be exact `DESIGNER` (or omitted for the Designer default); got {value:?}",
                    request.operation_name
                ))
            }
        };
        Ok(SourceCompatibilityResult {
            diagnostic: message
                .map(|message| FormatDiagnostic::new("sourceFamilyIncompatible", message)),
        })
    }
}

impl AuthorabilityPort for PlatformXmlGuards {
    fn inspect(
        &self,
        request: &AuthorabilityRequest,
    ) -> Result<AuthorabilityResult, SourceAdapterError> {
        let target = crate::factory::authorized_target(&request.source)?;
        let source_root =
            std::fs::canonicalize(request.source.location().source_root()).map_err(|error| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::SourceUnavailable,
                    format!("failed to resolve authorized source root: {error}"),
                )
            })?;
        let Some(config_dir) = find_support_config_dir(&target, &source_root) else {
            return Ok(AuthorabilityResult {
                authorability: Authorability::Authorable,
                violation: None,
            });
        };
        let object_uuid = support_object_uuid_for_path(&target, &source_root)
            .or_else(|| support_root_uuid(&config_dir.join("Configuration.xml")))
            .unwrap_or_default();
        let facts = v2_20::support::read_support_facts(
            &config_dir.join("Ext").join("ParentConfigurations.bin"),
        );
        let effective = facts.effective_rule_for(&object_uuid);
        let authorability = facts.authorability_for(&object_uuid);
        let violation = if let v2_20::support::SupportSourceState::Unreadable { error } =
            &facts.source
        {
            let location = error
                .offset
                .map(|offset| format!(" at byte {offset}"))
                .unwrap_or_default();
            Some(violation(
                "support-state-unreadable",
                format!(
                    "не удалось прочитать состояние поддержки (ParentConfigurations.bin): {}{}; безопасность правки не подтверждена",
                    error.context, location
                ),
                &target,
                &config_dir,
            ))
        } else if request.requirement == AuthorabilityRequirement::Removed {
            match effective {
                v2_20::support::EffectiveSupportRule::Removed => None,
                v2_20::support::EffectiveSupportRule::ConfigurationReadOnly => Some(violation(
                    "capability-off",
                    "возможность изменения конфигурации выключена (вся конфигурация read-only)",
                    &target,
                    &config_dir,
                )),
                _ => Some(violation(
                    "not-removed",
                    "объект не снят с поддержки; удаление сломает обновления",
                    &target,
                    &config_dir,
                )),
            }
        } else {
            match authorability {
                Authorability::Authorable => None,
                Authorability::ConfigurationReadOnly => Some(violation(
                    "capability-off",
                    "возможность изменения конфигурации выключена (вся конфигурация read-only)",
                    &target,
                    &config_dir,
                )),
                Authorability::SupportLocked => Some(violation(
                    "locked",
                    "объект или конфигурация на замке — редактирование сломает обновления",
                    &target,
                    &config_dir,
                )),
                Authorability::UnknownSupportState
                | Authorability::UnknownReadOnly
                | Authorability::DerivedReadOnly => Some(violation(
                    "support-state-unreadable",
                    "состояние поддержки объекта или конфигурации неизвестно; безопасность правки не подтверждена",
                    &target,
                    &config_dir,
                )),
            }
        };
        Ok(AuthorabilityResult {
            authorability,
            violation,
        })
    }
}

fn target_format() -> Result<FormatVersion, SourceAdapterError> {
    FormatVersion::parse(v2_20::EXPORT_FORMAT)
}

fn violation(
    code: &str,
    message: impl Into<String>,
    target: &Path,
    source_root: &Path,
) -> AuthorabilityViolation {
    AuthorabilityViolation {
        diagnostic: {
            let mut diagnostic = FormatDiagnostic::new(code, message);
            diagnostic
                .details
                .insert("target".to_string(), target.display().to_string());
            diagnostic
                .details
                .insert("sourceRoot".to_string(), source_root.display().to_string());
            diagnostic.details.insert(
                "supportStatePath".to_string(),
                source_root
                    .join("Ext")
                    .join("ParentConfigurations.bin")
                    .display()
                    .to_string(),
            );
            diagnostic
        },
        target: target.to_path_buf(),
        source_root: source_root.to_path_buf(),
    }
}

fn invalid_diagnostic(
    source: &unica_format_core::source::SourceContext,
    message: String,
) -> FormatDiagnostic {
    let mut diagnostic = FormatDiagnostic::new("formatVersionInvalid", message);
    diagnostic
        .details
        .insert("actualFormat".to_string(), "null".to_string());
    diagnostic
        .details
        .insert("targetFormat".to_string(), v2_20::EXPORT_FORMAT.to_string());
    diagnostic.details.insert(
        "targetPlatform".to_string(),
        v2_20::PLATFORM_LINE.to_string(),
    );
    diagnostic
        .details
        .insert("compatibility".to_string(), "invalid".to_string());
    diagnostic.details.insert(
        "root".to_string(),
        diagnostic_root(source).display().to_string(),
    );
    diagnostic
}

fn diagnostic_root(source: &unica_format_core::source::SourceContext) -> PathBuf {
    let configuration_owner = source.location().source_root().join("Configuration.xml");
    if configuration_owner.exists() {
        return configuration_owner;
    }
    match source.configured_source_set_kind() {
        Some(
            unica_format_core::source::ConfiguredSourceSetKind::Configuration
            | unica_format_core::source::ConfiguredSourceSetKind::Extension,
        ) => source.location().source_root().join("Configuration.xml"),
        _ => source.location().target().to_path_buf(),
    }
}

fn is_versionless_embedded_content(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok((_, document)) = v2_20::xml::parse_bounded_xml_document(&bytes) else {
        return false;
    };
    let root = document.root_element();
    matches!(
        (root.tag_name().namespace(), root.tag_name().name()),
        (Some(v2_20::xml::SPREADSHEET_DOCUMENT_NS), "document")
            | (
                Some(v2_20::xml::DATA_COMPOSITION_SCHEMA_NS),
                "DataCompositionSchema"
            )
    ) && root
        .attributes()
        .all(|attribute| attribute.namespace().is_some() || attribute.name() != "version")
}

fn find_support_config_dir(target: &Path, source_root: &Path) -> Option<PathBuf> {
    let mut current = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent()?.to_path_buf()
    };
    loop {
        if !current.starts_with(source_root) {
            return None;
        }
        if current
            .join("Ext")
            .join("ParentConfigurations.bin")
            .exists()
            || current.join("Configuration.xml").exists()
        {
            return Some(current);
        }
        if current == source_root {
            return None;
        }
        current = current.parent()?.to_path_buf();
    }
}

fn support_object_uuid_for_path(target: &Path, source_root: &Path) -> Option<String> {
    if target.is_file() {
        if let Some(uuid) = support_root_uuid(target) {
            return Some(uuid);
        }
    }
    let mut current = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent()?.to_path_buf()
    };
    loop {
        if !current.starts_with(source_root) {
            return None;
        }
        let candidate = current.with_extension("xml");
        if candidate.is_file() {
            if let Some(uuid) = support_root_uuid(&candidate) {
                return Some(uuid);
            }
        }
        if current == source_root {
            return None;
        }
        current = current.parent()?.to_path_buf();
    }
}

fn support_root_uuid(path: &Path) -> Option<String> {
    let raw = std::fs::read(path).ok()?;
    let text = std::str::from_utf8(&raw).ok()?;
    let document = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = document.root_element();
    root.attribute("uuid")
        .or_else(|| {
            root.children()
                .find(|node| node.is_element() && node.attribute("uuid").is_some())
                .and_then(|node| node.attribute("uuid"))
        })
        .map(str::to_ascii_lowercase)
}

pub(crate) fn normalize_metadata_category(value: &str) -> Option<&'static str> {
    if let Some(profile) = v2_20::schema::metadata_class_profile(value) {
        return Some(
            v2_20::metadata_classes()
                .iter()
                .copied()
                .find(|class| *class == profile.class_name)
                .expect("runtime metadata class profile is authoritative"),
        );
    }
    legacy_metadata_kind_by_directory(value).map(|kind| kind.tag)
}

pub(crate) fn registered_subsystem_names(path: &Path) -> Result<HashSet<String>, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read format owner {}: {error}", path.display()))?;
    let (_, document) = v2_20::xml::parse_bounded_xml_document(&bytes)
        .map_err(|_| format!("failed to parse format owner {}", path.display()))?;
    Ok(document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(v2_20::xml::MD_CLASSES_NS)
                && node.tag_name().name() == "ChildObjects"
        })
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(v2_20::xml::MD_CLASSES_NS)
                && node.tag_name().name() == "Subsystem"
        })
        .filter_map(|node| node.text())
        .map(str::to_string)
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMetadataKind {
    pub tag: &'static str,
    pub directory: &'static str,
    pub display_name_ru: &'static str,
    pub config_dump_prefix: Option<&'static str>,
    pub config_dump_module_suffix: Option<&'static str>,
}

pub fn legacy_metadata_kinds() -> &'static [LegacyMetadataKind] {
    debug_assert_eq!(LEGACY_METADATA_KINDS.len(), v2_20::metadata_classes().len());
    debug_assert!(LEGACY_METADATA_KINDS
        .iter()
        .all(|kind| { v2_20::schema::metadata_class_profile(kind.tag).is_some() }));
    LEGACY_METADATA_KINDS
}

pub fn legacy_metadata_kind(tag: &str) -> Option<&'static LegacyMetadataKind> {
    v2_20::schema::metadata_class_profile(tag)?;
    legacy_metadata_kinds().iter().find(|kind| kind.tag == tag)
}

pub fn legacy_metadata_kind_by_directory(directory: &str) -> Option<&'static LegacyMetadataKind> {
    let kind = legacy_metadata_kinds()
        .iter()
        .find(|kind| kind.directory.eq_ignore_ascii_case(directory))?;
    v2_20::schema::metadata_class_profile(kind.tag)?;
    Some(kind)
}

pub fn legacy_metadata_kind_index(tag: &str) -> Option<usize> {
    v2_20::metadata_classes()
        .iter()
        .position(|candidate| *candidate == tag)
}

pub fn legacy_metadata_kind_tags() -> &'static [&'static str] {
    v2_20::metadata_classes()
}

const LEGACY_METADATA_KINDS: &[LegacyMetadataKind] = &[
    legacy_kind("Language", "Languages", "Языки"),
    legacy_kind("Subsystem", "Subsystems", "Подсистемы"),
    legacy_kind("StyleItem", "StyleItems", "Элементы стиля"),
    legacy_kind("Style", "Styles", "Стили"),
    legacy_kind("CommonPicture", "CommonPictures", "Общие картинки"),
    legacy_kind("SessionParameter", "SessionParameters", "Параметры сеанса"),
    legacy_kind("Role", "Roles", "Роли"),
    legacy_kind("CommonTemplate", "CommonTemplates", "Общие макеты"),
    legacy_kind("FilterCriterion", "FilterCriteria", "Критерии отбора"),
    legacy_kind("CommonModule", "CommonModules", "Общие модули"),
    LegacyMetadataKind {
        tag: "Bot",
        directory: "Bots",
        display_name_ru: "Боты",
        config_dump_prefix: Some("Bot"),
        config_dump_module_suffix: Some(".Module"),
    },
    legacy_kind("CommonAttribute", "CommonAttributes", "Общие реквизиты"),
    legacy_kind("ExchangePlan", "ExchangePlans", "Планы обмена"),
    legacy_kind("XDTOPackage", "XDTOPackages", "XDTO-пакеты"),
    legacy_kind("WebService", "WebServices", "Веб-сервисы"),
    legacy_kind("HTTPService", "HTTPServices", "HTTP-сервисы"),
    legacy_kind("WSReference", "WSReferences", "WS-ссылки"),
    legacy_kind(
        "EventSubscription",
        "EventSubscriptions",
        "Подписки на события",
    ),
    legacy_kind("ScheduledJob", "ScheduledJobs", "Регламентные задания"),
    legacy_kind("SettingsStorage", "SettingsStorages", "Хранилища настроек"),
    legacy_kind(
        "FunctionalOption",
        "FunctionalOptions",
        "Функциональные опции",
    ),
    legacy_kind(
        "FunctionalOptionsParameter",
        "FunctionalOptionsParameters",
        "Параметры ФО",
    ),
    legacy_kind("DefinedType", "DefinedTypes", "Определяемые типы"),
    legacy_kind("CommonCommand", "CommonCommands", "Общие команды"),
    legacy_kind("CommandGroup", "CommandGroups", "Группы команд"),
    legacy_kind("Constant", "Constants", "Константы"),
    legacy_kind("CommonForm", "CommonForms", "Общие формы"),
    legacy_kind("Catalog", "Catalogs", "Справочники"),
    legacy_kind("Document", "Documents", "Документы"),
    legacy_kind("DocumentNumerator", "DocumentNumerators", "Нумераторы"),
    legacy_kind("Sequence", "Sequences", "Последовательности"),
    legacy_kind("DocumentJournal", "DocumentJournals", "Журналы документов"),
    legacy_kind("Enum", "Enums", "Перечисления"),
    legacy_kind("Report", "Reports", "Отчёты"),
    legacy_kind("DataProcessor", "DataProcessors", "Обработки"),
    legacy_kind(
        "InformationRegister",
        "InformationRegisters",
        "Регистры сведений",
    ),
    legacy_kind(
        "AccumulationRegister",
        "AccumulationRegisters",
        "Регистры накопления",
    ),
    legacy_kind(
        "ChartOfCharacteristicTypes",
        "ChartsOfCharacteristicTypes",
        "ПВХ",
    ),
    legacy_kind("ChartOfAccounts", "ChartsOfAccounts", "Планы счетов"),
    legacy_kind(
        "AccountingRegister",
        "AccountingRegisters",
        "Регистры бухгалтерии",
    ),
    legacy_kind("ChartOfCalculationTypes", "ChartsOfCalculationTypes", "ПВР"),
    legacy_kind(
        "CalculationRegister",
        "CalculationRegisters",
        "Регистры расчёта",
    ),
    legacy_kind("BusinessProcess", "BusinessProcesses", "Бизнес-процессы"),
    legacy_kind("Task", "Tasks", "Задачи"),
    legacy_kind(
        "IntegrationService",
        "IntegrationServices",
        "Сервисы интеграции",
    ),
];

const fn legacy_kind(
    tag: &'static str,
    directory: &'static str,
    display_name_ru: &'static str,
) -> LegacyMetadataKind {
    LegacyMetadataKind {
        tag,
        directory,
        display_name_ru,
        config_dump_prefix: None,
        config_dump_module_suffix: None,
    }
}
