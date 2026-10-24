//! Existing configuration semantic values and closed capabilities.
use crate::support_state::ConfigurationSupportData;

/// Typed answer of `unica.cf.info` (ADR-0023). Every declared property is
/// present: an absent value is `null`, never a missing key, so "not set" and
/// "not reported" stay different facts.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfInfoData {
    pub format: String,
    pub name: String,
    pub synonym: Option<String>,
    pub version: Option<String>,
    pub vendor: Option<String>,
    pub extension_purpose: Option<String>,
    pub support: ConfigurationSupportData,
    pub properties: CfInfoProperties,
    pub child_objects: Vec<CfChildObjectCount>,
    pub total_objects: usize,
    pub home_page: Option<CfHomePageData>,
    pub interface: Option<CfInterfaceData>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfInfoProperties {
    pub compatibility_mode: Option<String>,
    pub default_run_mode: Option<String>,
    pub script_variant: Option<String>,
    pub default_language: Option<String>,
    pub data_lock_control_mode: Option<String>,
    pub modality_use_mode: Option<String>,
    pub interface_compatibility_mode: Option<String>,
    pub extension_compatibility_mode: Option<String>,
    pub object_autonumeration_mode: Option<String>,
    pub synchronous_call_use_mode: Option<String>,
    pub database_tablespaces_use_mode: Option<String>,
    pub main_window_mode: Option<String>,
    pub comment: Option<String>,
    pub name_prefix: Option<String>,
    pub update_catalog_address: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfChildObjectCount {
    pub kind: String,
    pub count: usize,
}

/// Командный интерфейс конфигурации.
///
/// В корневом `Ext/CommandInterface.xml` лежит только порядок подсистем
/// верхнего уровня — команд там нет, они у подсистем. Поэтому интерфейс
/// конфигурации это свойство корня, рядом с `homePage`, а не отдельный узел:
/// листать и адресовать поштучно тут нечего.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfInterfaceData {
    /// Ссылки на подсистемы в объявленном порядке, как их пишет платформа:
    /// `Subsystem.Планирование`. Набором исходников их дополняет проекция.
    pub subsystem_order: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfHomePageData {
    pub template: String,
    pub left: Vec<CfHomePageItemData>,
    pub right: Vec<CfHomePageItemData>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfHomePageItemData {
    pub form: String,
    pub height: i64,
    pub common: bool,
    pub roles: Vec<CfHomePageRoleData>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfHomePageRoleData {
    pub role: String,
    pub visible: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfRegisteredObject {
    pub kind: String,
    pub name: String,
}

/// Typed answer of `unica.cf.edit` (ADR-0023). Every requested operation is
/// reported, including the ones that changed nothing: `[WARN] Already exists`
/// becomes `applied: false` with a reason instead of a line to grep for.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfEditData {
    pub configuration: String,
    pub operations: Vec<CfEditOperationData>,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    /// True when `Configuration.xml` itself was rewritten.
    pub config_updated: bool,
    /// The edit commits only through post-validation; this records that the
    /// validator actually ran over the written configuration.
    pub validated: bool,
    pub mutation: unica_execution_contract::mutation::MutationData,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfEditOperationData {
    pub operation: String,
    /// What the operation acted on: an object reference, a role, or a path.
    pub target: Option<String>,
    pub applied: bool,
    /// Why nothing changed; `null` when the operation applied.
    pub reason: Option<String>,
}

/// Typed answer of `unica.cf.init` (ADR-0023): the scaffold that was written.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfInitData {
    pub name: String,
    pub root: String,
    pub mutation: unica_execution_contract::mutation::MutationData,
}

impl CfEditOperationData {
    pub fn applied(operation: &str, target: impl Into<Option<String>>) -> Self {
        Self {
            operation: operation.to_string(),
            target: target.into(),
            applied: true,
            reason: None,
        }
    }

    pub fn skipped(operation: &str, target: impl Into<Option<String>>, reason: &str) -> Self {
        Self {
            operation: operation.to_string(),
            target: target.into(),
            applied: false,
            reason: Some(reason.to_string()),
        }
    }
}

pub fn cf_validate_guid(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    let lengths = [8, 4, 4, 4, 12];
    parts.len() == lengths.len()
        && parts
            .iter()
            .zip(lengths)
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

pub fn cf_validate_class_ids() -> &'static [&'static str] {
    &[
        "9cd510cd-abfc-11d4-9434-004095e12fc7",
        "9fcd25a0-4822-11d4-9414-008048da11f9",
        "e3687481-0a87-462c-a166-9f34594f9bba",
        "9de14907-ec23-4a07-96f0-85521cb6b53b",
        "51f2d5d8-ea4d-4064-8892-82951750031e",
        "e68182ea-4237-4383-967f-90c1e3370bc7",
        "fb282519-d103-4dd3-bc12-cb271d631dfc",
    ]
}

pub fn cf_validate_enum_properties() -> &'static [&'static str] {
    &[
        "ConfigurationExtensionCompatibilityMode",
        "DefaultRunMode",
        "ScriptVariant",
        "DataLockControlMode",
        "ObjectAutonumerationMode",
        "ModalityUseMode",
        "SynchronousPlatformExtensionAndAddInCallUseMode",
        "InterfaceCompatibilityMode",
        "DatabaseTablespacesUseMode",
        "MainClientApplicationWindowMode",
        "CompatibilityMode",
    ]
}

pub fn cf_validate_boolean_properties() -> &'static [&'static str] {
    &[
        "IncludeHelpInContents",
        "UseManagedFormInOrdinaryApplication",
        "UseOrdinaryFormInManagedApplication",
    ]
}

pub fn cf_validate_enum_allowed(property: &str) -> &'static [&'static str] {
    const COMPAT: &[&str] = &[
        "DontUse",
        "Version8_1",
        "Version8_2_13",
        "Version8_2_16",
        "Version8_3_1",
        "Version8_3_2",
        "Version8_3_3",
        "Version8_3_4",
        "Version8_3_5",
        "Version8_3_6",
        "Version8_3_7",
        "Version8_3_8",
        "Version8_3_9",
        "Version8_3_10",
        "Version8_3_11",
        "Version8_3_12",
        "Version8_3_13",
        "Version8_3_14",
        "Version8_3_15",
        "Version8_3_16",
        "Version8_3_17",
        "Version8_3_18",
        "Version8_3_19",
        "Version8_3_20",
        "Version8_3_21",
        "Version8_3_22",
        "Version8_3_23",
        "Version8_3_24",
        "Version8_3_25",
        "Version8_3_26",
        "Version8_3_27",
    ];
    match property {
        "ConfigurationExtensionCompatibilityMode" | "CompatibilityMode" => COMPAT,
        "DefaultRunMode" => &["ManagedApplication", "OrdinaryApplication", "Auto"],
        "ScriptVariant" => &["Russian", "English"],
        "DataLockControlMode" => &["Automatic", "Managed", "AutomaticAndManaged"],
        "ObjectAutonumerationMode" => &["NotAutoFree", "AutoFree"],
        "ModalityUseMode" | "SynchronousPlatformExtensionAndAddInCallUseMode" => {
            &["DontUse", "Use", "UseWithWarnings"]
        }
        "InterfaceCompatibilityMode" => &[
            "Version8_2",
            "Version8_2EnableTaxi",
            "Taxi",
            "TaxiEnableVersion8_2",
        ],
        "DatabaseTablespacesUseMode" => &["DontUse", "Use"],
        // The 8.3.27 XDTO type rejects `Fullscreen`; these are the values it reads.
        "MainClientApplicationWindowMode" => &[
            "Normal",
            "Workplace",
            "EmbeddedWorkplace",
            "FullscreenWorkplace",
            "Kiosk",
        ],
        _ => &[],
    }
}

pub fn cf_validate_form_properties() -> &'static [&'static str] {
    &[
        "DefaultReportForm",
        "DefaultReportVariantForm",
        "DefaultReportSettingsForm",
        "DefaultDynamicListSettingsForm",
        "DefaultSearchForm",
        "DefaultDataHistoryChangeHistoryForm",
        "DefaultDataHistoryVersionDataForm",
        "DefaultDataHistoryVersionDifferencesForm",
        "DefaultCollaborationSystemUsersChoiceForm",
        "DefaultConstantsForm",
    ]
}

pub fn cf_edit_validate_property_value(prop_name: &str, value: &str) -> Result<(), String> {
    let allowed: &[&str] = if cf_validate_boolean_properties().contains(&prop_name) {
        &["true", "false"]
    } else if cf_validate_enum_properties().contains(&prop_name) {
        cf_validate_enum_allowed(prop_name)
    } else {
        return Ok(());
    };
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "Property '{prop_name}' value '{value}' is not valid for 8.3.27; expected one of: {}",
            allowed.join(", ")
        ))
    }
}

pub fn cf_edit_ml_properties() -> &'static [&'static str] {
    &[
        "Synonym",
        "BriefInformation",
        "DetailedInformation",
        "Copyright",
        "VendorInformationAddress",
        "ConfigurationInformationAddress",
    ]
}

pub fn cf_edit_panel_alias(alias: &str) -> &str {
    match alias.to_lowercase().as_str() {
        "разделов" | "разделы" => "sections",
        "открытых" | "открытые" => "open",
        "избранного" | "избранное" => "favorites",
        "истории" | "история" => "history",
        "функций" | "функции" => "functions",
        _ => alias,
    }
}

pub fn cf_edit_panel_uuid(alias: &str) -> Option<&'static str> {
    match alias {
        "sections" => Some("b553047f-c9aa-4157-978d-448ecad24248"),
        "open" => Some("cbab57f2-a0f3-4f0a-89ea-4cb19570ab75"),
        "favorites" => Some("13322b22-3960-4d68-93a6-fe2dd7f28ca3"),
        "history" => Some("c933ac92-92cd-459d-81cc-e0c8a83ced99"),
        "functions" => Some("b2735bd3-d822-4430-ba59-c9e869693b24"),
        _ => None,
    }
}

pub fn cf_edit_uuid_like(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    [8usize, 4, 4, 4, 12].iter().zip(parts.iter()).count() == 5
        && parts.len() == 5
        && parts
            .iter()
            .zip([8usize, 4, 4, 4, 12])
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}
