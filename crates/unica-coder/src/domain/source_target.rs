use serde::Serialize;
use std::fmt;

pub const PLATFORM_XML_8_3_27_FORMAT_2_20: &str = "platform-xml-8.3.27-format-2.20";

/// XML 1.0 Fifth Edition NCName validation shared by typed metadata addresses
/// and the XML readers that turn QName values back into logical targets.
pub(crate) fn xml_ncname_is_valid(value: &str) -> bool {
    let mut characters = value.chars();
    characters.next().is_some_and(xml_ncname_is_start) && characters.all(xml_ncname_is_character)
}

fn xml_ncname_is_start(character: char) -> bool {
    matches!(
        character,
        'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{00C0}'..='\u{00D6}'
            | '\u{00D8}'..='\u{00F6}'
            | '\u{00F8}'..='\u{02FF}'
            | '\u{0370}'..='\u{037D}'
            | '\u{037F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
            | '\u{10000}'..='\u{EFFFF}'
    )
}

fn xml_ncname_is_character(character: char) -> bool {
    xml_ncname_is_start(character)
        || matches!(
            character,
            '-' | '.' | '0'..='9' | '\u{00B7}' | '\u{0300}'..='\u{036F}' | '\u{203F}'..='\u{2040}'
        )
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTarget {
    pub source_set: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct MetadataAddress(String);

impl MetadataAddress {
    pub fn parse(profile: &str, raw: &str) -> Result<Self, SourceTargetError> {
        AddressProfile::new(profile)?.parse(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Arity decides the kind, not the spelling of the last segment: `parse`
    /// accepts a module terminal only on an odd segment count, so an object
    /// legitimately named `Module` stays a metadata object instead of being
    /// read as the module role of a nameless owner.
    pub fn target_kind(&self) -> TargetKind {
        if self.0.split('.').count() % 2 == 1 {
            TargetKind::Module
        } else {
            TargetKind::MetadataObject
        }
    }

    pub(crate) fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('.')
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MetadataAddressPrefix(String);

impl MetadataAddressPrefix {
    pub(crate) fn parse(profile: &str, raw: &str) -> Result<Self, SourceTargetError> {
        AddressProfile::new(profile)?.parse_prefix(raw)
    }
}

impl fmt::Display for MetadataAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetKind {
    SourceRoot,
    MetadataObject,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTarget {
    pub source_set: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
    pub target_kind: TargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTargetErrorCode {
    SourceSetRequired,
    SourceSetNotFound,
    SourceRootNotAddressable,
    MetadataAddressInvalid,
    MetadataAddressNotFound,
    TargetKindMismatch,
    AddressProfileUnsupported,
    ContainmentDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceTargetErrorReason {
    General,
    SourceFormatUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTargetError {
    pub code: SourceTargetErrorCode,
    pub message: String,
    #[serde(skip)]
    reason: SourceTargetErrorReason,
}

impl SourceTargetError {
    pub(crate) fn new(code: SourceTargetErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            reason: SourceTargetErrorReason::General,
        }
    }

    pub(crate) fn source_format_unsupported(message: impl Into<String>) -> Self {
        Self {
            code: SourceTargetErrorCode::SourceRootNotAddressable,
            message: message.into(),
            reason: SourceTargetErrorReason::SourceFormatUnsupported,
        }
    }

    pub(crate) fn reason(&self) -> SourceTargetErrorReason {
        self.reason
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(SourceTargetErrorCode::MetadataAddressInvalid, message)
    }
}

impl fmt::Display for SourceTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for SourceTargetError {}

#[derive(Debug, Clone, Copy)]
pub struct AddressProfile {
    id: &'static str,
}

impl AddressProfile {
    pub fn new(id: &str) -> Result<Self, SourceTargetError> {
        if id == PLATFORM_XML_8_3_27_FORMAT_2_20 {
            Ok(Self {
                id: PLATFORM_XML_8_3_27_FORMAT_2_20,
            })
        } else {
            Err(SourceTargetError::new(
                SourceTargetErrorCode::AddressProfileUnsupported,
                format!("unsupported metadata address profile `{id}`"),
            ))
        }
    }

    pub fn id(self) -> &'static str {
        self.id
    }

    pub fn parse(self, raw: &str) -> Result<MetadataAddress, SourceTargetError> {
        let parts = raw.split('.').collect::<Vec<_>>();
        if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
            return Err(SourceTargetError::invalid(
                "metadata address contains an empty segment",
            ));
        }

        if parts.len() == 1 {
            return if is_root_module_terminal(parts[0]) {
                Ok(MetadataAddress(parts[0].to_string()))
            } else {
                Err(SourceTargetError::invalid(
                    "metadata address must contain a kind and application name",
                ))
            };
        }

        if matches!(parts.len(), 3 | 5)
            && !is_module_terminal(parts.last().copied().unwrap_or_default())
        {
            return Err(SourceTargetError::invalid(format!(
                "unsupported metadata address terminal `{}`",
                parts.last().copied().unwrap_or_default()
            )));
        }
        if !matches!(parts.len(), 2..=5) {
            return Err(SourceTargetError::invalid(
                "metadata address has an unsupported segment count",
            ));
        }

        let mut canonical = Vec::with_capacity(parts.len());
        let owner_kind = canonical_kind(parts[0])?;
        canonical.push(owner_kind);
        canonical.push(parts[1]);
        if parts.len() >= 4 {
            let child_kind = canonical_nested_kind_for_owner(owner_kind, parts[2])?;
            if child_kind == "Recalculation" && parts.len() != 4 {
                return Err(SourceTargetError::invalid(
                    "nested Recalculation metadata has no module terminal",
                ));
            }
            canonical.push(child_kind);
            canonical.push(parts[3]);
        }
        if parts.len() % 2 == 1 {
            canonical.push(parts.last().copied().unwrap_or_default());
        }
        Ok(MetadataAddress(canonical.join(".")))
    }

    fn parse_prefix(self, raw: &str) -> Result<MetadataAddressPrefix, SourceTargetError> {
        let parts = raw.split('.').collect::<Vec<_>>();
        if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
            return Err(SourceTargetError::invalid(
                "metadata address prefix contains an empty segment",
            ));
        }
        if parts.len() > 5 {
            return Err(SourceTargetError::invalid(
                "metadata address prefix has an unsupported segment count",
            ));
        }

        if parts.len() == 1 {
            if let Ok(kind) = canonical_kind_or_collection(parts[0]) {
                return Ok(MetadataAddressPrefix(kind.to_string()));
            }
            if is_root_module_terminal(parts[0]) {
                return Ok(MetadataAddressPrefix(parts[0].to_string()));
            }
            if is_canonical_token_prefix(parts[0]) {
                return Ok(MetadataAddressPrefix(parts[0].to_string()));
            }
            if is_alias_prefix(parts[0]) {
                return Err(SourceTargetError::invalid(format!(
                    "metadata address prefix `{}` matches only a partial alias; use the complete kind token or its canonical English prefix",
                    parts[0]
                )));
            }
            return Err(SourceTargetError::invalid(format!(
                "unknown metadata address prefix root `{}`",
                parts[0]
            )));
        }

        let mut canonical = Vec::with_capacity(parts.len());
        let owner_kind = canonical_kind_or_collection(parts[0])?;
        canonical.push(owner_kind);
        canonical.push(parts[1]);
        match parts.len() {
            2 => {}
            3 => {
                if let Ok(child_kind) = canonical_nested_kind_for_owner(owner_kind, parts[2]) {
                    canonical.push(child_kind);
                } else if MODULE_TERMINALS
                    .iter()
                    .any(|terminal| terminal.starts_with(parts[2]))
                {
                    canonical.push(parts[2]);
                } else {
                    return Err(SourceTargetError::invalid(format!(
                        "unknown metadata address prefix transition `{}`",
                        parts[2]
                    )));
                }
            }
            4 => {
                canonical.push(canonical_nested_kind_for_owner(owner_kind, parts[2])?);
                canonical.push(parts[3]);
            }
            5 => {
                let child_kind = canonical_nested_kind_for_owner(owner_kind, parts[2])?;
                if child_kind == "Recalculation" {
                    return Err(SourceTargetError::invalid(
                        "nested Recalculation metadata has no module terminal",
                    ));
                }
                canonical.push(child_kind);
                canonical.push(parts[3]);
                let terminal = match child_kind {
                    "Form" => "FormModule",
                    "Command" => "CommandModule",
                    "Template" => {
                        return Err(SourceTargetError::invalid(
                            "nested Template metadata has no module terminal",
                        ))
                    }
                    _ => unreachable!("nested kind parser is closed"),
                };
                if !terminal.starts_with(parts[4]) {
                    return Err(SourceTargetError::invalid(format!(
                        "metadata address prefix terminal `{}` is invalid after `{child_kind}`",
                        parts[4]
                    )));
                }
                canonical.push(parts[4]);
            }
            _ => unreachable!("prefix segment count was bounded"),
        }
        Ok(MetadataAddressPrefix(canonical.join(".")))
    }
}

#[derive(Clone, Copy)]
struct AddressKind {
    canonical: &'static str,
    russian_aliases: &'static [&'static str],
    collection_aliases: &'static [&'static str],
}

const ADDRESS_KINDS: &[AddressKind] = &[
    kind("Language", &["Язык"], &["Languages", "Языки"]),
    kind("Subsystem", &["Подсистема"], &["Subsystems", "Подсистемы"]),
    kind(
        "StyleItem",
        &["ЭлементСтиля"],
        &["StyleItems", "ЭлементыСтиля"],
    ),
    kind("Style", &["Стиль"], &["Styles", "Стили"]),
    kind(
        "CommonPicture",
        &["ОбщаяКартинка"],
        &["CommonPictures", "ОбщиеКартинки"],
    ),
    kind(
        "SessionParameter",
        &["ПараметрСеанса"],
        &["SessionParameters", "ПараметрыСеанса"],
    ),
    kind("Role", &["Роль"], &["Roles", "Роли"]),
    kind(
        "CommonTemplate",
        &["ОбщийМакет"],
        &["CommonTemplates", "ОбщиеМакеты"],
    ),
    kind(
        "FilterCriterion",
        &["КритерийОтбора"],
        &["FilterCriteria", "КритерииОтбора"],
    ),
    kind(
        "CommonModule",
        &["ОбщийМодуль"],
        &["CommonModules", "ОбщиеМодули"],
    ),
    kind("Bot", &["Бот"], &["Bots", "Боты"]),
    kind(
        "CommonAttribute",
        &["ОбщийРеквизит"],
        &["CommonAttributes", "ОбщиеРеквизиты"],
    ),
    kind(
        "ExchangePlan",
        &["ПланОбмена"],
        &["ExchangePlans", "ПланыОбмена"],
    ),
    kind(
        "XDTOPackage",
        &["ПакетXDTO"],
        &["XDTOPackages", "ПакетыXDTO"],
    ),
    kind("WebService", &["ВебСервис"], &["WebServices", "ВебСервисы"]),
    kind(
        "HTTPService",
        &["HTTPСервис"],
        &["HTTPServices", "HTTPСервисы"],
    ),
    kind("WSReference", &["WSСсылка"], &["WSReferences", "WSСсылки"]),
    kind(
        "EventSubscription",
        &["ПодпискаНаСобытие"],
        &["EventSubscriptions", "ПодпискиНаСобытия"],
    ),
    kind(
        "ScheduledJob",
        &["РегламентноеЗадание"],
        &["ScheduledJobs", "РегламентныеЗадания"],
    ),
    kind(
        "SettingsStorage",
        &["ХранилищеНастроек"],
        &["SettingsStorages", "ХранилищаНастроек"],
    ),
    kind(
        "FunctionalOption",
        &["ФункциональнаяОпция"],
        &["FunctionalOptions", "ФункциональныеОпции"],
    ),
    kind(
        "FunctionalOptionsParameter",
        &["ПараметрФункциональнойОпции"],
        &[
            "FunctionalOptionsParameters",
            "ПараметрыФункциональныхОпций",
        ],
    ),
    kind(
        "DefinedType",
        &["ОпределяемыйТип"],
        &["DefinedTypes", "ОпределяемыеТипы"],
    ),
    kind(
        "CommonCommand",
        &["ОбщаяКоманда"],
        &["CommonCommands", "ОбщиеКоманды"],
    ),
    kind(
        "CommandGroup",
        &["ГруппаКоманд"],
        &["CommandGroups", "ГруппыКоманд"],
    ),
    kind("Constant", &["Константа"], &["Constants", "Константы"]),
    kind(
        "CommonForm",
        &["ОбщаяФорма"],
        &["CommonForms", "ОбщиеФормы"],
    ),
    kind("Catalog", &["Справочник"], &["Catalogs", "Справочники"]),
    kind("Document", &["Документ"], &["Documents", "Документы"]),
    kind(
        "DocumentNumerator",
        &["НумераторДокументов"],
        &["DocumentNumerators", "НумераторыДокументов"],
    ),
    kind(
        "Sequence",
        &["Последовательность"],
        &["Sequences", "Последовательности"],
    ),
    kind(
        "DocumentJournal",
        &["ЖурналДокументов"],
        &["DocumentJournals", "ЖурналыДокументов"],
    ),
    kind("Enum", &["Перечисление"], &["Enums", "Перечисления"]),
    kind(
        "Report",
        &["Отчет", "Отчёт"],
        &["Reports", "Отчеты", "Отчёты"],
    ),
    kind(
        "DataProcessor",
        &["Обработка"],
        &["DataProcessors", "Обработки"],
    ),
    kind(
        "InformationRegister",
        &["РегистрСведений"],
        &["InformationRegisters", "РегистрыСведений"],
    ),
    kind(
        "AccumulationRegister",
        &["РегистрНакопления"],
        &["AccumulationRegisters", "РегистрыНакопления"],
    ),
    kind(
        "ChartOfCharacteristicTypes",
        &["ПланВидовХарактеристик"],
        &["ChartsOfCharacteristicTypes", "ПланыВидовХарактеристик"],
    ),
    kind(
        "ChartOfAccounts",
        &["ПланСчетов"],
        &["ChartsOfAccounts", "ПланыСчетов"],
    ),
    kind(
        "AccountingRegister",
        &["РегистрБухгалтерии"],
        &["AccountingRegisters", "РегистрыБухгалтерии"],
    ),
    kind(
        "ChartOfCalculationTypes",
        &["ПланВидовРасчета", "ПланВидовРасчёта"],
        &[
            "ChartsOfCalculationTypes",
            "ПланыВидовРасчета",
            "ПланыВидовРасчёта",
        ],
    ),
    kind(
        "CalculationRegister",
        &["РегистрРасчета", "РегистрРасчёта"],
        &["CalculationRegisters", "РегистрыРасчета", "РегистрыРасчёта"],
    ),
    kind(
        "BusinessProcess",
        &["БизнесПроцесс"],
        &["BusinessProcesses", "БизнесПроцессы"],
    ),
    kind("Task", &["Задача"], &["Tasks", "Задачи"]),
    kind(
        "IntegrationService",
        &["СервисИнтеграции"],
        &["IntegrationServices", "СервисыИнтеграции"],
    ),
    kind("Form", &["Форма"], &["Forms", "Формы"]),
    kind("Template", &["Макет"], &["Templates", "Макеты"]),
    kind("Command", &["Команда"], &["Commands", "Команды"]),
    kind(
        "ExternalDataProcessor",
        &["ВнешняяОбработка"],
        &["ExternalDataProcessors", "ВнешниеОбработки"],
    ),
    kind(
        "ExternalReport",
        &["ВнешнийОтчет", "ВнешнийОтчёт"],
        &["ExternalReports", "ВнешниеОтчеты", "ВнешниеОтчёты"],
    ),
];

const fn kind(
    canonical: &'static str,
    russian_aliases: &'static [&'static str],
    collection_aliases: &'static [&'static str],
) -> AddressKind {
    AddressKind {
        canonical,
        russian_aliases,
        collection_aliases,
    }
}

fn canonical_kind(raw: &str) -> Result<&'static str, SourceTargetError> {
    if let Some(kind) = ADDRESS_KINDS
        .iter()
        .find(|kind| kind.canonical == raw || kind.russian_aliases.contains(&raw))
    {
        return Ok(kind.canonical);
    }
    if ADDRESS_KINDS
        .iter()
        .any(|kind| kind.collection_aliases.contains(&raw))
    {
        return Err(SourceTargetError::invalid(format!(
            "metadata collection `{raw}` is not an addressable kind"
        )));
    }
    Err(SourceTargetError::invalid(format!(
        "unknown metadata kind `{raw}`"
    )))
}

/// Non-allocating twin of `metadata_address_kind_spellings` for the hot
/// address-parsing path: it answers whether `raw` is the canonical spelling
/// or a Russian alias of `canonical`.
pub(crate) fn metadata_address_kind_matches(canonical: &str, raw: &str) -> bool {
    ADDRESS_KINDS
        .iter()
        .find(|kind| kind.canonical == canonical)
        .is_some_and(|kind| kind.canonical == raw || kind.russian_aliases.contains(&raw))
}

pub(crate) fn metadata_address_kind_spellings(canonical: &str) -> Option<Vec<&'static str>> {
    ADDRESS_KINDS
        .iter()
        .find(|kind| kind.canonical == canonical)
        .map(|kind| {
            std::iter::once(kind.canonical)
                .chain(kind.russian_aliases.iter().copied())
                .collect()
        })
}

fn canonical_kind_or_collection(raw: &str) -> Result<&'static str, SourceTargetError> {
    ADDRESS_KINDS
        .iter()
        .find(|kind| {
            kind.canonical == raw
                || kind.russian_aliases.contains(&raw)
                || kind.collection_aliases.contains(&raw)
        })
        .map(|kind| kind.canonical)
        .ok_or_else(|| SourceTargetError::invalid(format!("unknown metadata kind `{raw}`")))
}

/// A partial prefix segment is matched against canonical English spelling only.
/// A partial alias has no single canonical form to expand into, so it is
/// reported instead of being silently passed through unnormalized.
fn is_canonical_token_prefix(raw: &str) -> bool {
    ADDRESS_KINDS
        .iter()
        .any(|kind| kind.canonical.starts_with(raw))
        || ROOT_MODULE_TERMINALS
            .iter()
            .any(|terminal| terminal.starts_with(raw))
}

fn is_alias_prefix(raw: &str) -> bool {
    ADDRESS_KINDS.iter().any(|kind| {
        kind.russian_aliases
            .iter()
            .chain(kind.collection_aliases.iter())
            .any(|alias| alias.starts_with(raw))
    })
}

fn canonical_nested_kind(raw: &str) -> Result<&'static str, SourceTargetError> {
    let canonical = canonical_kind_or_collection(raw)?;
    if matches!(canonical, "Form" | "Template" | "Command") {
        Ok(canonical)
    } else {
        Err(SourceTargetError::invalid(format!(
            "unsupported nested metadata kind `{raw}`"
        )))
    }
}

fn canonical_nested_kind_for_owner(
    owner_kind: &str,
    raw: &str,
) -> Result<&'static str, SourceTargetError> {
    if owner_kind == "CalculationRegister" && RECALCULATION_KIND_SPELLINGS.contains(&raw) {
        return Ok("Recalculation");
    }
    canonical_nested_kind(raw)
}

pub(crate) const RECALCULATION_KIND_SPELLINGS: &[&str] = &[
    "Recalculation",
    "Recalculations",
    "Перерасчет",
    "Перерасчёт",
    "Перерасчеты",
    "Перерасчёты",
];

const MODULE_TERMINALS: &[&str] = &[
    "Module",
    "ObjectModule",
    "ManagerModule",
    "RecordSetModule",
    "ValueManagerModule",
    "FormModule",
    "CommandModule",
    "ManagedApplicationModule",
    "OrdinaryApplicationModule",
    "SessionModule",
    "ExternalConnectionModule",
];

const ROOT_MODULE_TERMINALS: &[&str] = &[
    "ManagedApplicationModule",
    "OrdinaryApplicationModule",
    "SessionModule",
    "ExternalConnectionModule",
];

pub(crate) fn is_module_terminal(value: &str) -> bool {
    MODULE_TERMINALS.contains(&value)
}

fn is_root_module_terminal(value: &str) -> bool {
    ROOT_MODULE_TERMINALS.contains(&value)
}

#[cfg(test)]
mod tests {
    use super::{
        xml_ncname_is_valid, AddressProfile, MetadataAddress, MetadataAddressPrefix,
        SourceTargetErrorCode, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };

    #[test]
    fn source_target_profile_emits_canonical_english_kind_tokens() {
        let address = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Catalog.Counterparties.ObjectModule",
        )
        .unwrap();

        assert_eq!(address.as_str(), "Catalog.Counterparties.ObjectModule");
        assert_eq!(address.target_kind(), TargetKind::Module);
    }

    #[test]
    fn source_target_profile_normalizes_only_registered_exact_russian_kind_aliases() {
        let address = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Справочник.Контрагенты.Форма.Списка.FormModule",
        )
        .unwrap();

        assert_eq!(
            address.as_str(),
            "Catalog.Контрагенты.Form.Списка.FormModule"
        );
        let error = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "справочник.Контрагенты.ObjectModule",
        )
        .unwrap_err();
        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressInvalid);
    }

    #[test]
    fn source_target_profile_preserves_application_name_case() {
        let address = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "CommonModule.eBayHTTP.Module",
        )
        .unwrap();

        assert_eq!(address.as_str(), "CommonModule.eBayHTTP.Module");
    }

    #[test]
    fn source_target_profile_rejects_unsupported_terminal() {
        let error = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Catalog.Items.UnknownModule",
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressInvalid);
        assert!(error.message.contains("terminal"));
    }

    #[test]
    fn source_target_profile_rejects_empty_segments_and_collections() {
        for raw in ["Catalog..ObjectModule", "Catalogs.Items"] {
            let error = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap_err();
            assert_eq!(
                error.code,
                SourceTargetErrorCode::MetadataAddressInvalid,
                "{raw}"
            );
        }
    }

    #[test]
    fn source_target_profile_rejects_unknown_kind() {
        let error = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "UnknownKind.Items.ObjectModule",
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressInvalid);
        assert!(error.message.contains("kind"));
    }

    #[test]
    fn source_target_prefix_profile_rejects_unknown_transitions_and_malformed_forms() {
        for raw in [
            "",
            ".",
            "Catalog..Man",
            "Unknown.Items",
            "Catalog.Items.Unknown",
            "Catalog.Items.Form.Order.Unknown",
            "Catalog.Items.ManagerModule.Extra",
            "Catalog.Items.Command.Print.FormM",
            "Catalog.Items.Template.Print.FormModule",
        ] {
            let error =
                MetadataAddressPrefix::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap_err();
            assert_eq!(
                error.code,
                SourceTargetErrorCode::MetadataAddressInvalid,
                "{raw}"
            );
        }
    }

    #[test]
    fn source_target_profile_classifies_by_arity_not_by_terminal_spelling() {
        for raw in [
            "CommonModule.Module",
            "Catalog.SessionModule",
            "Document.ObjectModule",
            "Catalog.Items.Form.FormModule",
        ] {
            let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap();
            assert_eq!(
                address.target_kind(),
                TargetKind::MetadataObject,
                "{raw} names an object, its terminal segment is an application name"
            );
        }
        for raw in [
            "SessionModule",
            "CommonModule.Shared.Module",
            "Catalog.Items.Form.Order.FormModule",
        ] {
            let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap();
            assert_eq!(address.target_kind(), TargetKind::Module, "{raw}");
        }
    }

    #[test]
    fn source_target_prefix_profile_names_partial_aliases_apart_from_unknown_roots() {
        let alias =
            MetadataAddressPrefix::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Док").unwrap_err();
        assert_eq!(alias.code, SourceTargetErrorCode::MetadataAddressInvalid);
        assert!(alias.message.contains("partial alias"), "{}", alias.message);

        let unknown =
            MetadataAddressPrefix::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Zz").unwrap_err();
        assert!(
            unknown
                .message
                .contains("unknown metadata address prefix root"),
            "{}",
            unknown.message
        );
    }

    #[test]
    fn source_target_profile_rejects_unsupported_profile() {
        let error =
            MetadataAddress::parse("platform-xml-8.3.26-format-2.19", "Catalog.Items").unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::AddressProfileUnsupported);
    }

    #[test]
    fn xml_ncname_validation_covers_unicode_and_rejects_qname_or_path_syntax() {
        for valid in ["Name", "_Имя", "Имя-2", "A.B", "\u{10000}Name"] {
            assert!(xml_ncname_is_valid(valid), "{valid:?}");
        }
        for invalid in ["", "1Name", "Bad Name", "cfg:Name", ".Name"] {
            assert!(!xml_ncname_is_valid(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn address_profile_accepts_proven_root_module_terminals() {
        let profile = AddressProfile::new(PLATFORM_XML_8_3_27_FORMAT_2_20).unwrap();
        let address = profile.parse("ManagedApplicationModule").unwrap();

        assert_eq!(address.as_str(), "ManagedApplicationModule");
        assert_eq!(address.target_kind(), TargetKind::Module);
    }

    #[test]
    fn source_target_and_resolved_target_serialize_only_logical_identity() {
        let address =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Справочник.Items").unwrap();
        let target = super::SourceTarget {
            source_set: "addOn".to_string(),
            metadata_path: Some(address.clone()),
        };
        let resolved = super::ResolvedTarget {
            source_set: "addOn".to_string(),
            metadata_path: Some(address),
            target_kind: TargetKind::MetadataObject,
        };

        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({
                "sourceSet": "addOn",
                "metadataPath": "Catalog.Items"
            })
        );
        assert_eq!(
            serde_json::to_value(resolved).unwrap(),
            serde_json::json!({
                "sourceSet": "addOn",
                "metadataPath": "Catalog.Items",
                "targetKind": "metadataObject"
            })
        );
    }

    #[test]
    fn logical_target_identity_contract_is_complete() {
        source_target_profile_emits_canonical_english_kind_tokens();
        source_target_profile_normalizes_only_registered_exact_russian_kind_aliases();
        source_target_profile_preserves_application_name_case();
        source_target_and_resolved_target_serialize_only_logical_identity();
    }

    #[test]
    fn nested_subsystem_paths_do_not_extend_the_metadata_address_grammar() {
        let profile = AddressProfile::new(PLATFORM_XML_8_3_27_FORMAT_2_20).unwrap();

        assert!(profile.parse("Subsystem.A.B").is_err());
        assert!(
            MetadataAddressPrefix::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Subsystem.A.B").is_err()
        );

        let top_level = profile.parse("Subsystem.A").unwrap();
        assert_eq!(top_level.target_kind(), TargetKind::MetadataObject);
    }
}
