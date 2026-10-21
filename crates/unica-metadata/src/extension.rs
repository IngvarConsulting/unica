//! Existing extension semantic values and closed capabilities.
use crate::configuration::cf_validate_enum_allowed;

/// Typed answer of `unica.cfe.borrow` (ADR-0023). The prose narrated each step;
/// the data keeps only the facts a caller cannot recover elsewhere: what was
/// borrowed, what the borrow pulled in on its own, and what it left alone.
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeBorrowData {
    pub name_prefix: String,
    pub extension: String,
    pub configuration: String,
    /// Objects and forms named in the request.
    pub borrowed: Vec<CfeBorrowedItem>,
    /// Objects the borrow had to pull in to satisfy a reference or a deep path.
    pub auto_borrowed: Vec<String>,
    /// Files deliberately left as they were, with the reason.
    pub skipped: Vec<CfeBorrowSkip>,
    pub mutation: unica_execution_contract::mutation::MutationData,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeBorrowedItem {
    pub kind: String,
    pub name: String,
    /// The form that was borrowed; `null` when the whole object was.
    pub form: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeBorrowSkip {
    pub target: String,
    pub reason: String,
}

/// Collects the borrow facts the helpers used to print. Replaces the `&mut
/// String` that was threaded through them.
#[derive(Debug, Default)]
pub struct CfeBorrowLog {
    pub auto_borrowed: Vec<String>,
    pub skipped: Vec<CfeBorrowSkip>,
    pub warnings: Vec<String>,
}

pub struct CfeBorrowSpec {
    pub type_name: String,
    pub object_name: String,
    pub form_name: Option<String>,
}

/// Typed answer of `unica.cfe.diff` (ADR-0023). Mode A described what the
/// extension contains and Mode B whether its insertions reached the
/// configuration; both describe one extension, so both are reported at once.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeDiffData {
    pub name: String,
    pub purpose: String,
    pub name_prefix: String,
    pub objects: Vec<CfeDiffObjectData>,
    pub totals: CfeDiffTotals,
    pub transfer: Vec<CfeDiffTransferData>,
    pub transfer_totals: CfeDiffTransferTotals,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeDiffObjectData {
    pub kind: String,
    pub name: String,
    /// `borrowed`, `own`, `missing` or `unknownKind`.
    pub status: String,
    pub attributes: usize,
    pub forms: usize,
    pub tabular_sections: usize,
    pub borrowed_items: usize,
    pub form_names: Vec<String>,
    pub modules: Vec<CfeDiffModuleData>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeDiffModuleData {
    pub path: String,
    pub interceptors: Vec<CfeDiffInterceptorData>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeDiffInterceptorData {
    pub method: String,
    pub kind: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeDiffTotals {
    pub borrowed: usize,
    pub own: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeDiffTransferData {
    pub kind: String,
    pub name: String,
    pub method: String,
    /// `transferred`, `notTransferred` or `needsReview`.
    pub status: String,
    pub blocks: usize,
    /// Why a case needs review, `null` when it does not.
    pub reason: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeDiffTransferTotals {
    pub transferred: usize,
    pub not_transferred: usize,
    pub needs_review: usize,
}

/// Typed answer of `unica.cfe.patch_method` (ADR-0023): the interceptor that
/// was written and where it landed.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfePatchMethodData {
    pub module: String,
    /// True when the module file did not exist and was created by this call.
    pub module_created: bool,
    pub decorator: String,
    pub method: String,
    pub procedure: String,
    /// The compilation directive on the generated procedure; `null` when it
    /// carries none.
    pub compilation_directive: Option<String>,
    /// The descriptor switched to `Extended` for this method; `null` when the
    /// descriptor already carried the right state.
    pub descriptor: Option<CfePatchDescriptorData>,
    pub mutation: unica_execution_contract::mutation::MutationData,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfePatchDescriptorData {
    pub path: String,
    pub property: String,
}

/// Typed answer of `unica.cfe.init` (ADR-0023). The prose mixed `[INFO]` lines
/// about values read from the base configuration with `[WARN]` lines about
/// values that had to be defaulted; the first become `derived`, the second
/// become envelope warnings.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeInitData {
    pub name: String,
    pub purpose: String,
    pub name_prefix: String,
    pub compatibility_mode: String,
    pub interface_compatibility_mode: String,
    /// The base configuration the properties were read from; `null` when the
    /// scaffold was written without one.
    pub base_config: Option<String>,
    pub root: String,
    pub derived: Vec<CfeInitDerivedProperty>,
    pub mutation: unica_execution_contract::mutation::MutationData,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfeInitDerivedProperty {
    pub property: String,
    pub value: String,
    /// `baseConfig` when the value was read from the base configuration,
    /// `default` when the base configuration did not carry it.
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfePatchCommonModuleCapabilities {
    pub client_managed_application: bool,
    pub server: bool,
    pub external_connection: bool,
    pub client_ordinary_application: bool,
}

impl CfeBorrowLog {
    pub fn auto_borrowed(&mut self, reference: String) {
        if !self.auto_borrowed.contains(&reference) {
            self.auto_borrowed.push(reference);
        }
    }

    pub fn skipped(&mut self, target: String, reason: &str) {
        self.skipped.push(CfeBorrowSkip {
            target,
            reason: reason.to_string(),
        });
    }

    pub fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }
}

impl CfeInitDerivedProperty {
    pub fn from_base(property: &str, value: &str) -> Self {
        Self {
            property: property.to_string(),
            value: value.to_string(),
            source: "baseConfig".to_string(),
        }
    }

    pub fn defaulted(property: &str, value: &str) -> Self {
        Self {
            property: property.to_string(),
            value: value.to_string(),
            source: "default".to_string(),
        }
    }
}

impl CfePatchCommonModuleCapabilities {
    pub fn client(self) -> bool {
        self.client_managed_application || self.client_ordinary_application
    }

    pub fn any_execution_context(self) -> bool {
        self.client() || self.server || self.external_connection
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfePatchModuleRole {
    CommonModule,
    ObjectModule,
    ManagerModule,
    RecordSetModule,
    ValueManagerModule,
    Form,
}

impl CfePatchModuleRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CommonModule => "CommonModule",
            Self::ObjectModule => "ObjectModule",
            Self::ManagerModule => "ManagerModule",
            Self::RecordSetModule => "RecordSetModule",
            Self::ValueManagerModule => "ValueManagerModule",
            Self::Form => "Form",
        }
    }

    pub fn extended_property(self) -> &'static str {
        match self {
            Self::CommonModule => "Module",
            Self::ObjectModule => "ObjectModule",
            Self::ManagerModule => "ManagerModule",
            Self::RecordSetModule => "RecordSetModule",
            Self::ValueManagerModule => "ValueManagerModule",
            Self::Form => "Form",
        }
    }
}

pub fn cfe_validate_enum_properties_list() -> &'static [&'static str] {
    &[
        "ConfigurationExtensionCompatibilityMode",
        "DefaultRunMode",
        "ScriptVariant",
        "InterfaceCompatibilityMode",
    ]
}

pub fn cfe_validate_enum_allowed(property: &str) -> &'static [&'static str] {
    match property {
        "ConfigurationExtensionCompatibilityMode" => {
            cf_validate_enum_allowed("ConfigurationExtensionCompatibilityMode")
        }
        "DefaultRunMode" => &["ManagedApplication", "OrdinaryApplication", "Auto"],
        "ScriptVariant" => &["Russian", "English"],
        "InterfaceCompatibilityMode" => &[
            "Version8_2",
            "Version8_2EnableTaxi",
            "Taxi",
            "TaxiEnableVersion8_2",
        ],
        _ => &[],
    }
}

pub fn cfe_init_validate_enum(property: &str, value: &str) -> Result<(), String> {
    let allowed = cfe_validate_enum_allowed(property);
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{property} value {value:?} is not valid for 8.3.27; expected one of: {}",
            allowed.join(", ")
        ))
    }
}

pub fn cfe_patch_canonical_type(type_name: &str) -> Option<&'static str> {
    match type_name {
        "Catalog" => Some("Catalog"),
        "Document" => Some("Document"),
        "Enum" => Some("Enum"),
        "Report" => Some("Report"),
        "DataProcessor" => Some("DataProcessor"),
        "ExchangePlan" => Some("ExchangePlan"),
        "ChartOfAccounts" => Some("ChartOfAccounts"),
        "ChartOfCharacteristicTypes" => Some("ChartOfCharacteristicTypes"),
        "ChartOfCalculationTypes" => Some("ChartOfCalculationTypes"),
        "BusinessProcess" => Some("BusinessProcess"),
        "Task" => Some("Task"),
        "InformationRegister" => Some("InformationRegister"),
        "AccumulationRegister" => Some("AccumulationRegister"),
        "AccountingRegister" => Some("AccountingRegister"),
        "CalculationRegister" => Some("CalculationRegister"),
        "Constant" => Some("Constant"),
        "DocumentJournal" => Some("DocumentJournal"),
        "FilterCriterion" => Some("FilterCriterion"),
        _ => None,
    }
}

pub fn cfe_patch_form_role_is_supported(type_name: &str) -> bool {
    cfe_patch_canonical_type(type_name).is_some() && type_name != "Constant"
}

pub fn cfe_patch_direct_role_is_supported(type_name: &str, role: CfePatchModuleRole) -> bool {
    match role {
        CfePatchModuleRole::ObjectModule => matches!(
            type_name,
            "Catalog"
                | "Document"
                | "ExchangePlan"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "Report"
                | "DataProcessor"
        ),
        CfePatchModuleRole::ManagerModule => matches!(
            type_name,
            "Catalog"
                | "Document"
                | "ExchangePlan"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "Report"
                | "DataProcessor"
                | "Enum"
                | "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
                | "Constant"
                | "DocumentJournal"
                | "FilterCriterion"
        ),
        CfePatchModuleRole::RecordSetModule => matches!(
            type_name,
            "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
        ),
        CfePatchModuleRole::ValueManagerModule => type_name == "Constant",
        CfePatchModuleRole::CommonModule | CfePatchModuleRole::Form => false,
    }
}
