use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::OnceLock;

// Публикуемые схемы читает ECMA-262: там `\p{...}` без флага `u` — это
// литеральная `p`, а не класс символов. Поэтому наружу уходит сегментная форма
// соседних `meta.*`-схем, а строгую проверку идентификатора 1С держит парсер
// (`is_1c_identifier`) на Rust-regex, где Unicode-классы работают.
pub const ROLE_METADATA_PATH_PATTERN: &str = r"^Role\.[^.]+$";
pub const ROLE_OBJECT_NAME_PATTERN: &str = r"^[^.]+\.[^.]+(?:\.[^.]+\.[^.]+)*$";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleEditRequest {
    pub source_set: String,
    pub metadata_path: MetadataAddress,
    pub operations: Vec<RoleEditOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleEditOperation {
    pub object_name: String,
    pub right: String,
    pub value: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleEditEffectAction {
    SetRight,
    RemoveObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEditEffect {
    pub operation_index: usize,
    pub operation: &'static str,
    pub object_name: String,
    pub right: String,
    pub before: Option<bool>,
    pub after: bool,
    pub action: RoleEditEffectAction,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleEditDiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEditDiagnostic {
    pub code: String,
    pub severity: RoleEditDiagnosticSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleEditValidationStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEditValidation {
    pub status: RoleEditValidationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEditData {
    pub metadata_path: String,
    pub changed: bool,
    pub effects: Vec<RoleEditEffect>,
    pub validation: RoleEditValidation,
    pub diagnostics: Vec<RoleEditDiagnostic>,
}

impl RoleEditData {
    pub fn passed(
        metadata_path: impl Into<String>,
        changed: bool,
        effects: Vec<RoleEditEffect>,
    ) -> Self {
        Self {
            metadata_path: metadata_path.into(),
            changed,
            effects,
            validation: RoleEditValidation {
                status: RoleEditValidationStatus::Passed,
            },
            diagnostics: Vec::new(),
        }
    }

    pub fn failed(
        metadata_path: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        operation_index: Option<usize>,
    ) -> Self {
        Self {
            metadata_path: metadata_path.into(),
            changed: false,
            effects: Vec::new(),
            validation: RoleEditValidation {
                status: RoleEditValidationStatus::Failed,
            },
            diagnostics: vec![RoleEditDiagnostic {
                code: code.into(),
                severity: RoleEditDiagnosticSeverity::Error,
                message: message.into(),
                operation_index,
            }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleEditRequestError {
    pub code: &'static str,
    pub message: String,
    pub operation_index: Option<usize>,
}

impl std::fmt::Display for RoleEditRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RoleEditRequestError {}

fn request_error(code: &'static str, message: impl Into<String>) -> RoleEditRequestError {
    RoleEditRequestError {
        code,
        message: message.into(),
        operation_index: None,
    }
}

fn operation_error(
    index: usize,
    code: &'static str,
    message: impl Into<String>,
) -> RoleEditRequestError {
    RoleEditRequestError {
        code,
        message: message.into(),
        operation_index: Some(index),
    }
}

pub fn parse_role_edit_request(
    args: &Map<String, Value>,
) -> Result<RoleEditRequest, RoleEditRequestError> {
    let accepted = ["sourceSet", "metadataPath", "operations", "dryRun"];
    if let Some(unknown) = args.keys().find(|key| !accepted.contains(&key.as_str())) {
        return Err(request_error(
            "argument_invalid",
            format!("role.edit does not accept top-level argument `{unknown}`"),
        ));
    }
    if args.get("dryRun").is_some_and(|value| !value.is_boolean()) {
        return Err(request_error(
            "argument_invalid",
            "`dryRun` must be a JSON boolean",
        ));
    }
    let source_set = required_trimmed_string(args, "sourceSet")?;
    let raw_metadata_path = required_trimmed_string(args, "metadataPath")?;
    let role_name = raw_metadata_path.strip_prefix("Role.").ok_or_else(|| {
        request_error(
            "metadata_address_invalid",
            "metadataPath must use the canonical logical form Role.<name>",
        )
    })?;
    if !is_1c_identifier(role_name) {
        return Err(request_error(
            "metadata_address_invalid",
            "metadataPath role name must be a valid 1C identifier",
        ));
    }
    let metadata_path = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw_metadata_path)
        .map_err(|error| request_error("metadata_address_invalid", error.to_string()))?;

    let raw_operations = args
        .get("operations")
        .and_then(Value::as_array)
        .filter(|operations| !operations.is_empty())
        .ok_or_else(|| {
            request_error(
                "operations_required",
                "operations must be a non-empty array",
            )
        })?;
    let mut operations = Vec::with_capacity(raw_operations.len());
    for (index, raw_operation) in raw_operations.iter().enumerate() {
        let object = raw_operation.as_object().ok_or_else(|| {
            operation_error(index, "operation_invalid", "operation must be an object")
        })?;
        let accepted = ["op", "objectName", "right", "value"];
        if let Some(unknown) = object.keys().find(|key| !accepted.contains(&key.as_str())) {
            return Err(operation_error(
                index,
                "operation_invalid",
                format!("operation does not accept field `{unknown}`"),
            ));
        }
        if object.get("op").and_then(Value::as_str) != Some("setRight") {
            return Err(operation_error(
                index,
                "unsupported_operation",
                "operation `op` must be `setRight`",
            ));
        }
        let object_name = operation_string(object, index, "objectName")?;
        let right = operation_string(object, index, "right")?;
        let value = object
            .get("value")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                operation_error(
                    index,
                    "operation_invalid",
                    "operation `value` must be a JSON boolean",
                )
            })?;
        validate_object_name(object_name)
            .map_err(|message| operation_error(index, "object_name_invalid", message))?;
        if !all_role_right_names().contains(&right) {
            return Err(operation_error(
                index,
                "unsupported_right",
                format!("right `{right}` is not part of the closed role right contract"),
            ));
        }
        let object_parts = object_name.split('.').collect::<Vec<_>>();
        if object_parts.len() == 2 {
            let allowed = known_role_rights(object_parts[0]);
            if allowed.is_empty() || !allowed.contains(&right) {
                return Err(operation_error(
                    index,
                    "unsupported_right",
                    format!("right `{right}` is not valid for object `{object_name}`"),
                ));
            }
        } else {
            validate_nested_right(object_name, right)
                .map_err(|message| operation_error(index, "unsupported_right", message))?;
        }
        operations.push(RoleEditOperation {
            object_name: object_name.to_string(),
            right: right.to_string(),
            value,
        });
    }

    Ok(RoleEditRequest {
        source_set: source_set.to_string(),
        metadata_path,
        operations,
    })
}

fn required_trimmed_string<'a>(
    args: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, RoleEditRequestError> {
    let value = args
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| request_error("argument_required", format!("`{name}` is required")))?;
    if value != value.trim() {
        return Err(request_error(
            "argument_invalid",
            format!("`{name}` must not have surrounding whitespace"),
        ));
    }
    Ok(value)
}

fn operation_string<'a>(
    operation: &'a Map<String, Value>,
    index: usize,
    name: &str,
) -> Result<&'a str, RoleEditRequestError> {
    let value = operation
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            operation_error(
                index,
                "operation_invalid",
                format!("operation `{name}` must be a non-empty string"),
            )
        })?;
    if value != value.trim() {
        return Err(operation_error(
            index,
            "operation_invalid",
            format!("operation `{name}` must not have surrounding whitespace"),
        ));
    }
    Ok(value)
}

fn validate_object_name(value: &str) -> Result<(), String> {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() < 2 || parts.len() % 2 != 0 || parts.iter().any(|part| !is_1c_identifier(part)) {
        return Err(
            "objectName must be a canonical dot-separated metadata object or nested object"
                .to_string(),
        );
    }
    Ok(())
}

fn is_1c_identifier(value: &str) -> bool {
    static IDENTIFIER: OnceLock<regex::Regex> = OnceLock::new();
    IDENTIFIER
        .get_or_init(|| {
            regex::Regex::new(r"^[\p{L}_][\p{L}\p{N}_]*$")
                .expect("role identifier pattern is static and valid")
        })
        .is_match(value)
}

/// Входит ли раскладка вложенного адреса в закрытую модель прав.
///
/// Наблюдение и мутация относятся к незнакомой раскладке по-разному.
/// `unica.role.edit` пишет только то, что моделирует, поэтому незнакомое
/// сочетание для него — отказ. `unica.role.validate` лишь читает чужой файл и
/// об отсутствующей в модели раскладке не вправе утверждать, что она неверна:
/// платформа знает больше видов вложенных объектов, чем закрытый набор writer-а
/// (например, колонки журналов документов и перерасчёты). Верхний уровень той
/// же проверки уже молчит о неизвестном виде объекта — здесь та же политика.
pub fn nested_right_shape_is_modelled(object_name: &str) -> bool {
    let parts = object_name.split('.').collect::<Vec<_>>();
    if parts.len() >= 4
        && parts[0] == "Subsystem"
        && parts[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .all(|p| p[0] == "Subsystem")
    {
        return true;
    }
    match parts.as_slice() {
        [parent, _, "Command", _] => supports_commands(parent),
        ["WebService", _, "Operation", _]
        | ["HTTPService", _, "URLTemplate", _, "Method", _]
        | ["IntegrationService", _, "IntegrationServiceChannel", _]
        | ["Task", _, "AddressingAttribute", _] => true,
        [parent, _, "StandardAttribute", _] => supports_standard_attributes(parent),
        [parent, _, "Attribute", _] => supports_attributes(parent),
        [parent, _, "TabularSection", _]
        | [parent, _, "TabularSection", _, "Attribute", _]
        | [parent, _, "TabularSection", _, "StandardAttribute", _] => {
            supports_tabular_sections(parent)
        }
        [parent, _, "Dimension", _] | [parent, _, "Resource", _] => {
            supports_register_fields(parent)
        }
        _ => false,
    }
}

#[cfg(test)]
mod nested_shape_tests {
    use super::*;

    #[test]
    fn observation_stays_silent_about_shapes_outside_the_closed_model() {
        // Раскладки, которых закрытая модель не знает: платформа их пишет
        // (колонка журнала документов, перерасчёт регистра расчёта), а writer
        // не моделирует. Наблюдение не должно объявлять их неверными.
        for unmodelled in [
            "DocumentJournal.Orders.Column.Number",
            "CalculationRegister.Payroll.Recalculation.Basis",
        ] {
            assert!(!nested_right_shape_is_modelled(unmodelled), "{unmodelled}");
            // Мутация закрыта по-прежнему: писать неизвестное нельзя.
            assert!(validate_nested_right(unmodelled, "View").is_err());
        }

        // Известная раскладка остаётся под строгой проверкой права.
        assert!(nested_right_shape_is_modelled(
            "Catalog.Goods.Attribute.Price"
        ));
        assert!(validate_nested_right("Catalog.Goods.Attribute.Price", "View").is_ok());
        assert!(validate_nested_right("Catalog.Goods.Attribute.Price", "Delete").is_err());
    }
}

pub fn validate_nested_right(object_name: &str, right: &str) -> Result<(), String> {
    let parts = object_name.split('.').collect::<Vec<_>>();
    let nested_subsystem = parts.len() >= 4
        && parts[0] == "Subsystem"
        && parts[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| pair[0] == "Subsystem");
    let valid = if nested_subsystem {
        right == "View"
    } else {
        match parts.as_slice() {
            [parent, _, "Command", _] if supports_commands(parent) => right == "View",
            ["WebService", _, "Operation", _] => right == "Use",
            ["HTTPService", _, "URLTemplate", _, "Method", _] => right == "Use",
            ["IntegrationService", _, "IntegrationServiceChannel", _] => right == "Use",
            [parent, _, "StandardAttribute", _] if supports_standard_attributes(parent) => {
                matches!(right, "View" | "Edit")
            }
            [parent, _, "Attribute", _] if supports_attributes(parent) => {
                matches!(right, "View" | "Edit")
            }
            [parent, _, "TabularSection", _] if supports_tabular_sections(parent) => {
                matches!(right, "View" | "Edit")
            }
            [parent, _, "TabularSection", _, "Attribute", _]
                if supports_tabular_sections(parent) =>
            {
                matches!(right, "View" | "Edit")
            }
            [parent, _, "TabularSection", _, "StandardAttribute", _]
                if supports_tabular_sections(parent) =>
            {
                matches!(right, "View" | "Edit")
            }
            [parent, _, "Dimension", _] if supports_register_fields(parent) => {
                matches!(right, "View" | "Edit")
            }
            [parent, _, "Resource", _] if supports_register_fields(parent) => {
                matches!(right, "View" | "Edit")
            }
            ["Task", _, "AddressingAttribute", _] => {
                matches!(right, "View" | "Edit")
            }
            _ => false,
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "right `{right}` is not valid for nested object `{object_name}`"
        ))
    }
}

fn supports_commands(parent: &str) -> bool {
    matches!(
        parent,
        "Catalog"
            | "Document"
            | "DataProcessor"
            | "Report"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "DocumentJournal"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
    )
}

fn supports_standard_attributes(parent: &str) -> bool {
    matches!(
        parent,
        "Catalog"
            | "Document"
            | "ExchangePlan"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "DocumentJournal"
    )
}

fn supports_attributes(parent: &str) -> bool {
    matches!(
        parent,
        "Catalog"
            | "Document"
            | "ExchangePlan"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task"
            | "DataProcessor"
            | "Report"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
    )
}

fn supports_tabular_sections(parent: &str) -> bool {
    matches!(
        parent,
        "Catalog"
            | "Document"
            | "ExchangePlan"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task"
            | "DataProcessor"
            | "Report"
    )
}

fn supports_register_fields(parent: &str) -> bool {
    matches!(
        parent,
        "InformationRegister" | "AccumulationRegister" | "AccountingRegister"
    )
}

pub fn all_role_right_names() -> BTreeSet<&'static str> {
    KNOWN_ROLE_OBJECT_TYPES
        .iter()
        .flat_map(|object_type| known_role_rights(object_type).iter().copied())
        .collect()
}

const KNOWN_ROLE_OBJECT_TYPES: &[&str] = &[
    "Configuration",
    "Catalog",
    "Document",
    "InformationRegister",
    "AccumulationRegister",
    "AccountingRegister",
    "CalculationRegister",
    "Constant",
    "ChartOfAccounts",
    "ChartOfCharacteristicTypes",
    "ChartOfCalculationTypes",
    "ExchangePlan",
    "BusinessProcess",
    "Task",
    "DataProcessor",
    "Report",
    "CommonForm",
    "CommonCommand",
    "Subsystem",
    "FilterCriterion",
    "DocumentJournal",
    "Sequence",
    "WebService",
    "HTTPService",
    "IntegrationService",
    "SessionParameter",
    "CommonAttribute",
];

pub fn known_role_rights(object_type: &str) -> &'static [&'static str] {
    match object_type {
        "Configuration" => &[
            "Administration",
            "DataAdministration",
            "UpdateDataBaseConfiguration",
            "ConfigurationExtensionsAdministration",
            "ActiveUsers",
            "EventLog",
            "ExclusiveMode",
            "ThinClient",
            "ThickClient",
            "WebClient",
            "MobileClient",
            "ExternalConnection",
            "Automation",
            "Output",
            "SaveUserData",
            "TechnicalSpecialistMode",
            "InteractiveOpenExtDataProcessors",
            "InteractiveOpenExtReports",
            "AnalyticsSystemClient",
            "CollaborationSystemInfoBaseRegistration",
            "MainWindowModeNormal",
            "MainWindowModeWorkplace",
            "MainWindowModeEmbeddedWorkplace",
            "MainWindowModeFullscreenWorkplace",
            "MainWindowModeKiosk",
        ],
        "Catalog" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "Document" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "Posting",
            "UndoPosting",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "InteractivePosting",
            "InteractivePostingRegular",
            "InteractiveUndoPosting",
            "InteractiveChangeOfPosted",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "InformationRegister" => &[
            "Read",
            "Update",
            "View",
            "Edit",
            "TotalsControl",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "AccumulationRegister" | "AccountingRegister" => {
            &["Read", "Update", "View", "Edit", "TotalsControl"]
        }
        "CalculationRegister" => &["Read", "View"],
        "Constant" => &[
            "Read",
            "Update",
            "View",
            "Edit",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "ChartOfAccounts" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
            "ReadDataHistory",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistory",
            "UpdateDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
        ],
        "ChartOfCharacteristicTypes" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "ChartOfCalculationTypes" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveDeletePredefinedData",
            "InteractiveSetDeletionMarkPredefinedData",
            "InteractiveClearDeletionMarkPredefinedData",
            "InteractiveDeleteMarkedPredefinedData",
        ],
        "ExchangePlan" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveDeleteMarked",
            "ReadDataHistory",
            "ViewDataHistory",
            "UpdateDataHistory",
            "ReadDataHistoryOfMissingData",
            "UpdateDataHistoryOfMissingData",
            "UpdateDataHistorySettings",
            "UpdateDataHistoryVersionComment",
            "EditDataHistoryVersionComment",
            "SwitchToDataHistoryVersion",
        ],
        "BusinessProcess" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "Start",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveActivate",
            "InteractiveStart",
        ],
        "Task" => &[
            "Read",
            "Insert",
            "Update",
            "Delete",
            "View",
            "Edit",
            "InputByString",
            "Execute",
            "InteractiveInsert",
            "InteractiveSetDeletionMark",
            "InteractiveClearDeletionMark",
            "InteractiveDelete",
            "InteractiveActivate",
            "InteractiveExecute",
        ],
        "DataProcessor" | "Report" => &["Use", "View"],
        "CommonForm" | "CommonCommand" | "Subsystem" | "FilterCriterion" => &["View"],
        "DocumentJournal" => &["Read", "View"],
        "Sequence" => &["Read", "Update"],
        "WebService" | "HTTPService" | "IntegrationService" => &["Use"],
        "SessionParameter" => &["Get", "Set"],
        "CommonAttribute" => &["View", "Edit"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parser_accepts_only_closed_logical_contract() {
        let args = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Demo",
            "operations": [{
                "op": "setRight",
                "objectName": "Catalog.Demo",
                "right": "Delete",
                "value": false
            }]
        });
        let parsed = parse_role_edit_request(args.as_object().unwrap()).unwrap();
        assert_eq!(parsed.metadata_path.as_str(), "Role.Demo");
        assert_eq!(parsed.operations.len(), 1);

        for legacy in ["RightsPath", "Path", "ObjectName", "Name", "Value"] {
            let mut invalid = args.clone();
            invalid[legacy] = json!("legacy");
            let error = parse_role_edit_request(invalid.as_object().unwrap()).unwrap_err();
            assert_eq!(error.code, "argument_invalid");
            assert!(error.message.contains(legacy), "{error}");
        }
    }

    #[test]
    fn parser_publishes_stable_argument_gate_codes() {
        let valid = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Demo",
            "operations": [{
                "op": "setRight",
                "objectName": "Catalog.Demo",
                "right": "Delete",
                "value": false
            }]
        });
        let cases = [
            (
                "non-role metadata path",
                json!({
                    "sourceSet": "main",
                    "metadataPath": "Catalog.Demo",
                    "operations": valid["operations"].clone()
                }),
                "metadata_address_invalid",
            ),
            (
                "invalid role name",
                json!({
                    "sourceSet": "main",
                    "metadataPath": "Role.Not-Valid",
                    "operations": valid["operations"].clone()
                }),
                "metadata_address_invalid",
            ),
            (
                "missing operations",
                json!({
                    "sourceSet": "main",
                    "metadataPath": "Role.Demo"
                }),
                "operations_required",
            ),
            (
                "empty operations",
                json!({
                    "sourceSet": "main",
                    "metadataPath": "Role.Demo",
                    "operations": []
                }),
                "operations_required",
            ),
            (
                "non-boolean dryRun",
                json!({
                    "sourceSet": "main",
                    "metadataPath": "Role.Demo",
                    "operations": valid["operations"].clone(),
                    "dryRun": "false"
                }),
                "argument_invalid",
            ),
        ];

        for (case, args, expected_code) in cases {
            let error = parse_role_edit_request(args.as_object().unwrap()).unwrap_err();
            assert_eq!(error.code, expected_code, "{case}: {error}");
        }
    }

    #[test]
    fn nested_right_matrix_is_structural() {
        for (object, right) in [
            ("Catalog.Demo.Command.Open", "View"),
            ("CalculationRegister.Payroll.Command.Fill", "View"),
            ("DocumentJournal.Main.Command.Open", "View"),
            (
                "InformationRegister.Prices.StandardAttribute.Period",
                "Edit",
            ),
            ("DocumentJournal.Main.StandardAttribute.Posted", "View"),
            ("CalculationRegister.Payroll.Attribute.Comment", "Edit"),
            (
                "Catalog.Demo.TabularSection.Lines.Attribute.Quantity",
                "Edit",
            ),
            (
                "Catalog.Demo.TabularSection.Lines.StandardAttribute.LineNumber",
                "View",
            ),
            ("InformationRegister.Prices.Dimension.Product", "View"),
            ("AccumulationRegister.Stock.Resource.Quantity", "Edit"),
            ("AccountingRegister.Ledger.Dimension.Account", "View"),
            ("Task.Job.AddressingAttribute.Assignee", "Edit"),
            (
                "IntegrationService.Api.IntegrationServiceChannel.Main",
                "Use",
            ),
            ("WebService.Api.Operation.GetData", "Use"),
            ("HTTPService.Api.URLTemplate.Items.Method.GET", "Use"),
            ("Subsystem.Admin.Subsystem.Users", "View"),
            ("Subsystem.Admin.Subsystem.Users.Subsystem.Security", "View"),
        ] {
            assert!(
                validate_nested_right(object, right).is_ok(),
                "expected {object} / {right} to be valid"
            );
        }

        for (object, right) in [
            ("Catalog.Demo.Command.Open", "Use"),
            ("DataProcessor.Demo.Command.Open", "Use"),
            ("CommonForm.Main.Command.Refresh", "View"),
            ("DocumentJournal.Main.Attribute.Comment", "Edit"),
            ("CalculationRegister.Payroll.Dimension.Employee", "View"),
            ("CalculationRegister.Payroll.Resource.Amount", "Edit"),
            ("BusinessProcess.Flow.AddressingAttribute.Assignee", "Edit"),
            (
                "InformationRegister.Prices.TabularSection.Lines.StandardAttribute.LineNumber",
                "View",
            ),
            (
                "IntegrationService.Api.IntegrationServiceChannel.Main",
                "View",
            ),
            ("WebService.Api.IntegrationServiceChannel.Main", "Use"),
            ("WebService.Api.Operation.GetData", "View"),
            ("HTTPService.Api.URLTemplate.Items.Method.GET", "View"),
            ("Catalog.Demo.Subsystem.Users", "View"),
            ("Subsystem.Admin.Subsystem.Users.Subsystem.Security", "Edit"),
            ("Catalog.Demo.Dimension.Fake", "View"),
        ] {
            assert!(
                validate_nested_right(object, right).is_err(),
                "expected {object} / {right} to be rejected"
            );
        }
    }

    #[test]
    fn top_level_rights_are_checked_for_the_selected_object_kind() {
        let invalid = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Demo",
            "operations": [{
                "op": "setRight",
                "objectName": "DataProcessor.Demo",
                "right": "Delete",
                "value": false
            }]
        });
        let error = parse_role_edit_request(invalid.as_object().unwrap()).unwrap_err();
        assert_eq!(error.code, "unsupported_right");
    }
}
