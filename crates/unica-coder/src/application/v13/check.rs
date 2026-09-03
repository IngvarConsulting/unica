use crate::domain::address::QualifiedAddress;
use serde::Serialize;
use std::fmt;

/// The native validators `unica.check` can run. The list is closed: a node
/// kind owns its validators, and the caller never names one on the wire.
pub(crate) const NATIVE_VALIDATORS: &[CheckValidator] = &[
    CheckValidator::Cf,
    CheckValidator::Cfe,
    CheckValidator::Form,
    CheckValidator::Dcs,
    CheckValidator::Mxl,
    CheckValidator::Role,
    CheckValidator::Subsystem,
    CheckValidator::Interface,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckValidator {
    Cf,
    Cfe,
    Form,
    Dcs,
    Mxl,
    Role,
    Subsystem,
    Interface,
}

impl CheckValidator {
    /// The native validator this step runs, by its operation name. The
    /// read-only format guard of the retired `*.validate` tools is keyed by
    /// the same names, so the canonical check inherits it unchanged.
    pub(crate) const fn native_operation(self) -> &'static str {
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

    pub(crate) fn supported() -> &'static [Self] {
        NATIVE_VALIDATORS
    }

    /// The validator that owns an address before the node is read: the
    /// export-format guard names it when the read port cannot open the target.
    pub(crate) fn for_unread_address(at: &QualifiedAddress, extension: bool) -> Option<Self> {
        let kind = at
            .segments()
            .last()
            .map(|segment| segment.kind().as_str())
            .unwrap_or_default();
        match kind {
            "Configuration" if extension => Some(Self::Cfe),
            "Configuration" => Some(Self::Cf),
            "Form" => Some(Self::Form),
            "Role" => Some(Self::Role),
            "Subsystem" => Some(Self::Subsystem),
            "Interface" | "CommandInterface" => Some(Self::Interface),
            _ => None,
        }
    }
}

/// What `check` knows about a readable node before choosing its validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct NodeFacts {
    /// The node lives in a source set of kind `EXTENSION`.
    pub(crate) extension: bool,
    /// A template node's flavour, when the projection states it.
    pub(crate) template: Option<TemplateFlavour>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemplateFlavour {
    DataCompositionSchema,
    SpreadsheetDocument,
}

/// One validation step of a check plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckStep {
    Native(CheckValidator),
    /// The typed metadata validator of one object descriptor.
    Meta,
}

impl CheckStep {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Native(validator) => validator.name(),
            Self::Meta => "meta",
        }
    }
}

/// The validators a readable node owns, in the order they run. An empty plan
/// means the node has no validator and `check` reports readability only.
pub(crate) fn plan_for_node(kind: &str, facts: NodeFacts) -> Vec<CheckStep> {
    match kind {
        "Configuration" if facts.extension => vec![CheckStep::Native(CheckValidator::Cfe)],
        "Configuration" => vec![CheckStep::Native(CheckValidator::Cf)],
        "Form" => vec![CheckStep::Native(CheckValidator::Form)],
        "Template" => match facts.template {
            Some(TemplateFlavour::DataCompositionSchema) => {
                vec![CheckStep::Native(CheckValidator::Dcs)]
            }
            Some(TemplateFlavour::SpreadsheetDocument) => {
                vec![CheckStep::Native(CheckValidator::Mxl)]
            }
            None => Vec::new(),
        },
        // The typed metadata validator reads object descriptors only; roles
        // and subsystems keep their own validators.
        "Role" => vec![CheckStep::Native(CheckValidator::Role)],
        "Subsystem" => vec![CheckStep::Native(CheckValidator::Subsystem)],
        "Interface" | "CommandInterface" => vec![CheckStep::Native(CheckValidator::Interface)],
        other if crate::domain::metadata::MetadataKind::parse(other).is_ok() => {
            vec![CheckStep::Meta]
        }
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckError {
    BadValue { field: String, message: String },
    DependencyUnavailable,
}

impl CheckError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::BadValue { .. } => "bad_value",
            Self::DependencyUnavailable => "dependency_unavailable",
        }
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadValue { field, message } => {
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
    /// A warning that keeps the validator verdict: the closed format codes of
    /// the export-format guard travel through here.
    pub(crate) fn warning(code: impl Into<String>, message: &str) -> Self {
        Self {
            severity: "warning".to_string(),
            code: code.into(),
            message: sanitize_message(message),
        }
    }

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

    /// Prepends one diagnostic without touching the verdict: a read-only
    /// format warning is reported first, the validator findings follow.
    pub(crate) fn with_leading_diagnostic(mut self, diagnostic: CheckDiagnostic) -> Self {
        self.diagnostics.insert(0, diagnostic);
        self
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
    validator: String,
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

    pub(crate) fn validator(&self) -> &str {
        &self.validator
    }

    pub(crate) fn raw_stream(&self) -> Option<&str> {
        None
    }
}

pub(crate) fn normalize_native_outcome(
    at: &QualifiedAddress,
    kind: &str,
    validator: CheckValidator,
    native: NativeCheckOutcome,
) -> Result<CheckResult, CheckError> {
    if native.unavailable {
        return Err(CheckError::DependencyUnavailable);
    }
    Ok(CheckResult {
        at: at.to_string(),
        kind: kind.to_string(),
        validator: validator.name().to_string(),
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
        normalize_native_outcome, plan_for_node, CheckStep, CheckValidator, NativeCheckOutcome,
        NodeFacts, TemplateFlavour,
    };
    use crate::domain::address::QualifiedAddress;

    #[test]
    fn every_node_kind_owns_its_validators_without_a_caller_choice() {
        let plain = NodeFacts::default();
        assert_eq!(
            plan_for_node("Configuration", plain),
            [CheckStep::Native(CheckValidator::Cf)]
        );
        assert_eq!(
            plan_for_node(
                "Configuration",
                NodeFacts {
                    extension: true,
                    ..plain
                }
            ),
            [CheckStep::Native(CheckValidator::Cfe)]
        );
        assert_eq!(
            plan_for_node("Form", plain),
            [CheckStep::Native(CheckValidator::Form)]
        );
        assert_eq!(
            plan_for_node(
                "Template",
                NodeFacts {
                    template: Some(TemplateFlavour::DataCompositionSchema),
                    ..plain
                }
            ),
            [CheckStep::Native(CheckValidator::Dcs)]
        );
        assert_eq!(
            plan_for_node(
                "Template",
                NodeFacts {
                    template: Some(TemplateFlavour::SpreadsheetDocument),
                    ..plain
                }
            ),
            [CheckStep::Native(CheckValidator::Mxl)]
        );
        assert!(plan_for_node("Template", plain).is_empty());
        assert_eq!(
            plan_for_node("Role", plain),
            [CheckStep::Native(CheckValidator::Role)]
        );
        assert_eq!(
            plan_for_node("Subsystem", plain),
            [CheckStep::Native(CheckValidator::Subsystem)]
        );
        assert_eq!(
            plan_for_node("Interface", plain),
            [CheckStep::Native(CheckValidator::Interface)]
        );
        assert_eq!(plan_for_node("Catalog", plain), [CheckStep::Meta]);
        assert!(plan_for_node("Module", plain).is_empty());
        assert!(plan_for_node("Method", plain).is_empty());
    }

    #[test]
    fn the_validator_registry_maps_every_step_to_its_native_operation() {
        for validator in CheckValidator::supported() {
            assert!(validator.native_operation().ends_with("-validate"));
            assert!(!validator.name().is_empty());
        }
        assert_eq!(CheckValidator::Cf.native_operation(), "cf-validate");
        assert_eq!(CheckValidator::Form.native_operation(), "form-validate");
        assert_eq!(CheckStep::Meta.name(), "meta");
    }

    #[test]
    fn an_unread_address_still_names_the_validator_that_guards_its_format() {
        let root = QualifiedAddress::parse("main:Configuration").unwrap();
        assert_eq!(
            CheckValidator::for_unread_address(&root, false),
            Some(CheckValidator::Cf)
        );
        assert_eq!(
            CheckValidator::for_unread_address(&root, true),
            Some(CheckValidator::Cfe)
        );
        let form = QualifiedAddress::parse("main:Catalog.Items.Form.List").unwrap();
        assert_eq!(
            CheckValidator::for_unread_address(&form, false),
            Some(CheckValidator::Form)
        );
        let module = QualifiedAddress::parse("main:CommonModule.Common").unwrap();
        assert_eq!(CheckValidator::for_unread_address(&module, false), None);
    }

    #[test]
    fn check_result_normalizes_diagnostics_without_native_stream_or_path() {
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
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
        let result =
            normalize_native_outcome(&at, "Configuration", CheckValidator::Cf, native).unwrap();
        assert!(!result.ok());
        assert_eq!(result.validator(), "cf");
        assert_eq!(result.diagnostics().len(), 2);
        assert_eq!(result.diagnostics()[0].code(), "invalid_root");
        assert!(!result.diagnostics()[0].message().contains("/private/"));
        assert!(!result.diagnostics()[1].message().contains("/usr/local/"));
        assert!(result.raw_stream().is_none());
    }

    #[test]
    fn unavailable_validator_is_typed_dependency_failure() {
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
        let error = normalize_native_outcome(
            &at,
            "Configuration",
            CheckValidator::Cf,
            NativeCheckOutcome::unavailable("validator engine is not installed"),
        )
        .unwrap_err();
        assert_eq!(error.code(), "dependency_unavailable");
        assert!(!error.to_string().contains("engine"));
    }
}
