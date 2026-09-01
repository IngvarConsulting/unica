use crate::application::v13::check::{CheckProfile, NativeCheckOutcome};
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};

/// Closed native validator registry for the hidden A0 analysis seam. The
/// family validators remain the source of validation truth; this adapter only
/// translates their result into the bounded v0.13 diagnostic model.
pub(crate) fn validate(
    profile: CheckProfile,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> NativeCheckOutcome {
    validate_with_availability(profile, args, context, ValidatorAvailability::Available)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidatorAvailability {
    Available,
    Unavailable,
}

pub(crate) fn validate_with_availability(
    profile: CheckProfile,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    availability: ValidatorAvailability,
) -> NativeCheckOutcome {
    if availability == ValidatorAvailability::Unavailable {
        return unavailable_dependency();
    }
    let outcome = match profile {
        CheckProfile::Cf => super::cf::validate_cf(args, context),
        CheckProfile::Cfe => super::cfe::validate_cfe(args, context),
        CheckProfile::Form => super::form::validate_form(args, context),
        CheckProfile::Dcs => super::dcs::validate_dcs(args, context),
        CheckProfile::Mxl => super::mxl::validate_mxl(args, context),
        CheckProfile::Role => super::role::validate_role(args, context),
        CheckProfile::Subsystem => super::subsystem::validate_subsystem(args, context),
        CheckProfile::Interface => super::interface::validate_interface(args, context),
    };
    NativeCheckOutcome::from_adapter(&outcome)
}

/// A dependency that is not installed is not a validation failure and must be
/// represented as a typed unavailable outcome by the caller.
pub(crate) fn unavailable_dependency() -> NativeCheckOutcome {
    NativeCheckOutcome::unavailable("native validator dependency is unavailable")
}

#[cfg(test)]
mod tests {
    use super::{validate, validate_with_availability, ValidatorAvailability};
    use crate::application::v13::check::{normalize_native_outcome, CheckRequest};
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn context() -> (WorkspaceContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-a0-check-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        (
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            root,
        )
    }

    #[test]
    fn cf_validator_is_real_and_normalized_before_application_projection() {
        let (context, root) = context();
        fs::write(root.join("Configuration.xml"), "<not-configuration />").unwrap();
        let request = CheckRequest::new(
            "main:Configuration",
            Some(&json!({"validation": {"profile": "cf"}})),
        )
        .unwrap();
        let native = validate(
            request.profile(),
            &Map::from_iter([(
                "ConfigPath".to_string(),
                json!(root.join("Configuration.xml").display().to_string()),
            )]),
            &context,
        );
        let result = normalize_native_outcome(&request, "Configuration", native).unwrap();
        assert!(!result.ok());
        assert!(!result.diagnostics().is_empty());
        for diagnostic in result.diagnostics() {
            assert!(!diagnostic.message().contains(root.to_str().unwrap()));
            assert!(!diagnostic.message().contains("stdout"));
            assert!(!diagnostic.message().contains("stderr"));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_dependency_stays_typed_and_does_not_claim_success() {
        let request = CheckRequest::new(
            "main:Configuration",
            Some(&json!({"validation": {"profile": "cf"}})),
        )
        .unwrap();
        let error = normalize_native_outcome(
            &request,
            "Configuration",
            validate_with_availability(
                request.profile(),
                &Map::new(),
                &WorkspaceContext {
                    cwd: std::path::PathBuf::new(),
                    workspace_root: std::path::PathBuf::new(),
                    cache_root: std::path::PathBuf::new(),
                    workspace_epoch: 1,
                },
                ValidatorAvailability::Unavailable,
            ),
        )
        .unwrap_err();
        assert_eq!(error.code(), "dependency_unavailable");
    }
}
