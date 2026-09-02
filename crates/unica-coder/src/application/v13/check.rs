use crate::domain::address::QualifiedAddress;
use serde::Serialize;
use serde_json::Value;
use std::fmt;

const SUPPORTED_PROFILES: &[CheckProfile] = &[
    CheckProfile::Cf,
    CheckProfile::Cfe,
    CheckProfile::Form,
    CheckProfile::Dcs,
    CheckProfile::Mxl,
    CheckProfile::Role,
    CheckProfile::Subsystem,
    CheckProfile::Interface,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckProfile {
    Cf,
    Cfe,
    Form,
    Dcs,
    Mxl,
    Role,
    Subsystem,
    Interface,
}

impl CheckProfile {
    pub(crate) fn parse(value: &str) -> Result<Self, CheckError> {
        let profile = match value {
            "cf" => Self::Cf,
            "cfe" => Self::Cfe,
            "form" => Self::Form,
            "dcs" => Self::Dcs,
            "mxl" => Self::Mxl,
            "role" => Self::Role,
            "subsystem" => Self::Subsystem,
            "interface" => Self::Interface,
            _ => {
                return Err(CheckError::UnsupportedFilter {
                    field: "validation.profile".to_string(),
                    message: format!("unsupported validation profile `{value}`"),
                })
            }
        };
        Ok(profile)
    }

    fn for_address(at: &QualifiedAddress) -> Result<Self, CheckError> {
        let kind = at
            .segments()
            .last()
            .map(|segment| segment.kind().as_str())
            .unwrap_or_default();
        match kind {
            "Configuration" => Ok(Self::Cf),
            "Subsystem" => Ok(Self::Subsystem),
            "Role" => Ok(Self::Role),
            "Form" => Ok(Self::Form),
            "Interface" => Ok(Self::Interface),
            _ => Err(CheckError::UnsupportedFilter {
                field: "filter".to_string(),
                message: format!(
                    "no default validator is registered for logical kind `{kind}`; provide validation.profile"
                ),
            }),
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Cf => "cf",
            Self::Cfe => "cfe",
            Self::Form => "form",
            Self::Dcs => "dcs",
            Self::Mxl => "mxl",
            Self::Role => "role",
            Self::Subsystem => "subsystem",
            Self::Interface => "interface",
        }
    }

    pub(crate) const fn operation(self) -> &'static str {
        match self {
            Self::Cf => "cf-validate",
            Self::Cfe => "cfe-validate",
            Self::Form => "form-validate",
            Self::Dcs => "dcs-validate",
            Self::Mxl => "mxl-validate",
            Self::Role => "role-validate",
            Self::Subsystem => "subsystem-validate",
            Self::Interface => "interface-validate",
        }
    }

    pub(crate) fn supported() -> &'static [Self] {
        SUPPORTED_PROFILES
    }

    /// The logical node kinds a profile validates. An explicit profile that
    /// names a node of another kind is a caller mistake, not a validator run.
    pub(crate) fn accepts_kind(self, kind: &str) -> bool {
        match self {
            Self::Cf | Self::Cfe => kind == "Configuration",
            Self::Form => kind == "Form",
            Self::Dcs | Self::Mxl => kind == "Template",
            Self::Role => kind == "Role",
            Self::Subsystem => kind == "Subsystem",
            Self::Interface => matches!(kind, "Interface" | "CommandInterface"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckRequest {
    at: QualifiedAddress,
    profile: CheckProfile,
}

impl CheckRequest {
    pub(crate) fn new(at: &str, filter: Option<&Value>) -> Result<Self, CheckError> {
        let at = QualifiedAddress::parse(at).map_err(|error| CheckError::BadValue {
            field: "at".to_string(),
            message: error.to_string(),
        })?;
        let profile = match filter {
            Some(filter) => parse_filter(filter)?,
            None => CheckProfile::for_address(&at)?,
        };
        Ok(Self { at, profile })
    }

    pub(crate) fn at(&self) -> &QualifiedAddress {
        &self.at
    }

    pub(crate) const fn profile(&self) -> CheckProfile {
        self.profile
    }
}

fn parse_filter(filter: &Value) -> Result<CheckProfile, CheckError> {
    let filter = filter.as_object().ok_or_else(|| CheckError::BadValue {
        field: "filter".to_string(),
        message: "check filter must be an object".to_string(),
    })?;
    if filter.len() != 1 || !filter.contains_key("validation") {
        return Err(CheckError::UnsupportedFilter {
            field: "filter".to_string(),
            message: "check filter supports only validation.profile".to_string(),
        });
    }
    let validation = filter["validation"]
        .as_object()
        .ok_or_else(|| CheckError::BadValue {
            field: "validation".to_string(),
            message: "check validation filter must be an object".to_string(),
        })?;
    if validation.len() != 1 || !validation.contains_key("profile") {
        return Err(CheckError::UnsupportedFilter {
            field: "validation".to_string(),
            message: "check validation filter supports only profile".to_string(),
        });
    }
    let profile = validation["profile"]
        .as_str()
        .ok_or_else(|| CheckError::BadValue {
            field: "validation.profile".to_string(),
            message: "validation profile must be a string".to_string(),
        })?;
    CheckProfile::parse(profile)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckError {
    BadValue { field: String, message: String },
    UnsupportedFilter { field: String, message: String },
    DependencyUnavailable,
}

impl CheckError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::BadValue { .. } => "bad_value",
            Self::UnsupportedFilter { .. } => "unsupported_filter",
            Self::DependencyUnavailable => "dependency_unavailable",
        }
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadValue { field, message } | Self::UnsupportedFilter { field, message } => {
                write!(formatter, "{field}: {message}")
            }
            Self::DependencyUnavailable => {
                formatter.write_str("the selected validator dependency is unavailable")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckDiagnostic {
    severity: String,
    code: String,
    message: String,
}

impl CheckDiagnostic {
    pub(crate) fn severity(&self) -> &str {
        &self.severity
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeCheckOutcome {
    ok: bool,
    diagnostics: Vec<CheckDiagnostic>,
    unavailable: bool,
}

impl NativeCheckOutcome {
    pub(crate) fn passed() -> Self {
        Self {
            ok: true,
            diagnostics: Vec::new(),
            unavailable: false,
        }
    }

    pub(crate) fn failed(
        diagnostics: impl IntoIterator<Item = (&'static str, &'static str, &'static str)>,
    ) -> Self {
        Self {
            ok: false,
            diagnostics: diagnostics
                .into_iter()
                .map(|(severity, code, message)| CheckDiagnostic {
                    severity: severity.to_string(),
                    code: code.to_string(),
                    message: sanitize_message(message),
                })
                .collect(),
            unavailable: false,
        }
    }

    pub(crate) fn unavailable(_detail: &str) -> Self {
        Self {
            ok: false,
            diagnostics: Vec::new(),
            unavailable: true,
        }
    }

    pub(crate) fn from_adapter(outcome: &crate::application::AdapterOutcome) -> Self {
        let mut diagnostics = outcome
            .errors
            .iter()
            .map(|message| CheckDiagnostic {
                severity: "error".to_string(),
                code: "native_validation_error".to_string(),
                message: sanitize_message(message),
            })
            .collect::<Vec<_>>();
        diagnostics.extend(outcome.warnings.iter().map(|message| CheckDiagnostic {
            severity: "warning".to_string(),
            code: "native_validation_warning".to_string(),
            message: sanitize_message(message),
        }));
        Self {
            ok: outcome.ok,
            diagnostics,
            unavailable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckResult {
    at: String,
    kind: String,
    profile: String,
    ok: bool,
    diagnostics: Vec<CheckDiagnostic>,
}

impl CheckResult {
    pub(crate) fn ok(&self) -> bool {
        self.ok
    }

    pub(crate) fn diagnostics(&self) -> &[CheckDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn raw_stream(&self) -> Option<&str> {
        None
    }
}

pub(crate) fn normalize_native_outcome(
    request: &CheckRequest,
    kind: &str,
    native: NativeCheckOutcome,
) -> Result<CheckResult, CheckError> {
    if native.unavailable {
        return Err(CheckError::DependencyUnavailable);
    }
    Ok(CheckResult {
        at: request.at.to_string(),
        kind: kind.to_string(),
        profile: request.profile.name().to_string(),
        ok: native.ok,
        diagnostics: native.diagnostics,
    })
}

fn sanitize_message(message: &str) -> String {
    message
        .split_whitespace()
        .filter(|token| !token.contains('/') && !token.contains('\\'))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_native_outcome, CheckError, CheckProfile, CheckRequest, NativeCheckOutcome,
    };
    use serde_json::json;

    #[test]
    fn explicit_profiles_accept_only_the_node_kinds_they_validate() {
        assert!(CheckProfile::Form.accepts_kind("Form"));
        assert!(!CheckProfile::Form.accepts_kind("Catalog"));
        assert!(CheckProfile::Dcs.accepts_kind("Template"));
        assert!(CheckProfile::Mxl.accepts_kind("Template"));
        assert!(CheckProfile::Cf.accepts_kind("Configuration"));
        assert!(!CheckProfile::Cf.accepts_kind("Subsystem"));
        assert!(CheckProfile::Role.accepts_kind("Role"));
        assert!(CheckProfile::Subsystem.accepts_kind("Subsystem"));
        assert!(CheckProfile::Interface.accepts_kind("CommandInterface"));
    }

    #[test]
    fn check_registry_accepts_only_proven_validator_profiles() {
        assert_eq!(
            CheckProfile::parse("cf").unwrap().operation(),
            "cf-validate"
        );
        assert_eq!(
            CheckProfile::parse("form").unwrap().operation(),
            "form-validate"
        );
        assert!(matches!(
            CheckProfile::parse("meta"),
            Err(CheckError::UnsupportedFilter { .. })
        ));
        assert!(matches!(
            CheckProfile::parse("shell"),
            Err(CheckError::UnsupportedFilter { .. })
        ));
    }

    #[test]
    fn check_request_rejects_unknown_filter_shape_before_native_dispatch() {
        let error = CheckRequest::new(
            "main:Configuration",
            Some(&json!({"validation": {"profile": "cf"}, "paths": []})),
        )
        .unwrap_err();
        assert!(matches!(error, CheckError::UnsupportedFilter { .. }));
    }

    #[test]
    fn check_request_selects_a_closed_default_for_unfiltered_configuration() {
        let request = CheckRequest::new("main:Configuration", None).unwrap();
        assert_eq!(request.profile(), CheckProfile::Cf);
    }

    #[test]
    fn check_result_normalizes_diagnostics_without_native_stream_or_path() {
        let request = CheckRequest::new(
            "main:Configuration",
            Some(&json!({"validation": {"profile": "cf"}})),
        )
        .unwrap();
        let native = NativeCheckOutcome::failed(vec![
            (
                "error",
                "invalid_root",
                "XML parse failed in /private/workspace/Configuration.xml",
            ),
            (
                "warning",
                "format_warning",
                "provider /usr/local/bin/engine reported a warning",
            ),
        ]);
        let result = normalize_native_outcome(&request, "Configuration", native).unwrap();
        assert!(!result.ok());
        assert_eq!(result.diagnostics().len(), 2);
        assert_eq!(result.diagnostics()[0].code(), "invalid_root");
        assert!(!result.diagnostics()[0].message().contains("/private/"));
        assert!(!result.diagnostics()[1].message().contains("/usr/local/"));
        assert!(result.raw_stream().is_none());
    }

    #[test]
    fn unavailable_validator_is_typed_dependency_failure() {
        let request = CheckRequest::new(
            "main:Configuration",
            Some(&json!({"validation": {"profile": "cf"}})),
        )
        .unwrap();
        let error = normalize_native_outcome(
            &request,
            "Configuration",
            NativeCheckOutcome::unavailable("validator engine is not installed"),
        )
        .unwrap_err();
        assert_eq!(error.code(), "dependency_unavailable");
        assert!(!error.to_string().contains("engine"));
    }
}
