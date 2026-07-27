//! Platform XML source-family adapter.
//!
//! The public boundary is a factory and registration composed from
//! format-neutral core ports.
//!
//! ```
//! use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
//! use unica_format_core::source::SourceFamily;
//!
//! let registration = PlatformXmlAdapterFactory::new().registration();
//! assert_eq!(registration.manifest.source_family, SourceFamily::PlatformXml);
//! ```
//!
//! Version modules and native parser types are deliberately private.
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::v2_20;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions::v2_20::native_model::PlatformXmlNativeSnapshot;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions::v2_20::projector::project;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions::v2_20::schema::MetadataClassProfile;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::owner::Owner;
//! ```

mod artifact_access;
mod factory;
mod guards;
mod operations;
mod owner;
mod platform_handle;
mod publication;
mod safe_root;
mod validation;
mod versions;

pub use factory::PlatformXmlAdapterFactory;

mod application {
    pub(crate) use crate::operations::operation_descriptors::SupportGuardRequirement;
    pub(crate) use crate::operations::AdapterOutcome;
    #[cfg(test)]
    #[derive(Debug)]
    pub(crate) struct CacheReport {
        pub(crate) mode: &'static str,
    }

    #[cfg(test)]
    #[derive(Debug)]
    pub(crate) struct OperationResult {
        pub(crate) ok: bool,
        pub(crate) summary: String,
        pub(crate) changes: Vec<String>,
        pub(crate) warnings: Vec<String>,
        pub(crate) errors: Vec<String>,
        pub(crate) artifacts: Vec<String>,
        pub(crate) cache: CacheReport,
        pub(crate) stdout: Option<String>,
        pub(crate) stderr: Option<String>,
        pub(crate) command: Option<Vec<String>>,
        pub(crate) diagnostics: Option<serde_json::Value>,
    }

    pub(crate) mod operation_descriptors {
        pub(crate) use crate::operations::operation_descriptors::*;
    }

    #[cfg(test)]
    pub(crate) struct UnicaApplication;

    #[cfg(test)]
    impl UnicaApplication {
        pub(crate) const fn new() -> Self {
            Self
        }

        pub(crate) fn call_tool(
            &self,
            tool_name: &str,
            args: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<OperationResult, String> {
            let operation = match tool_name {
                "unica.cf.init" => "cf-init",
                "unica.cf.edit" => "cf-edit",
                "unica.cfe.init" => "cfe-init",
                "unica.cfe.borrow" => "cfe-borrow",
                "unica.cfe.patch-method" => "cfe-patch-method",
                "unica.epf.init" => "epf-init",
                "unica.erf.init" => "erf-init",
                "unica.meta.compile" => "meta-compile",
                "unica.meta.edit" => "meta-edit",
                "unica.meta.remove" => "meta-remove",
                "unica.form.add" => "form-add",
                "unica.form.compile" => "form-compile",
                "unica.form.edit" => "form-edit",
                "unica.form.remove" => "form-remove",
                "unica.template.add" => "template-add",
                "unica.template.remove" => "template-remove",
                "unica.help.add" => "help-add",
                "unica.interface.edit" => "interface-edit",
                "unica.role.compile" => "role-compile",
                "unica.subsystem.compile" => "subsystem-compile",
                "unica.subsystem.edit" => "subsystem-edit",
                "unica.support.edit" => "support-edit",
                "unica.dcs.compile" => "dcs-compile",
                "unica.dcs.edit" => "dcs-edit",
                "unica.mxl.compile" => "mxl-compile",
                _ => return Err(format!("unknown test tool: {tool_name}")),
            };
            let context = test_workspace_context(args)?;
            let dry_run = args
                .get("dryRun")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let mut outcome = crate::operations::NativeOperationAdapter::invoke(
                operation, tool_name, args, &context, dry_run, true,
            )?;
            let diagnostic_text = outcome
                .errors
                .iter()
                .chain(outcome.warnings.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let diagnostics = (!outcome.ok
                && (diagnostic_text.contains("2.21")
                    || diagnostic_text.contains("newer than supported 2.20")))
            .then(|| {
                if !outcome
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("1С 8.5"))
                {
                    outcome.warnings.push(
                        "Формат 2.21 требует платформу 1С 8.5 и не поддерживается.".to_string(),
                    );
                }
                serde_json::json!({
                    "formatCompatibility": {
                        "code": "platformVersionUnsupported",
                        "actualFormat": "2.21"
                    }
                })
            });
            Ok(OperationResult {
                ok: outcome.ok,
                summary: outcome.summary,
                changes: outcome.changes,
                warnings: outcome.warnings,
                errors: outcome.errors,
                artifacts: outcome.artifacts,
                cache: CacheReport {
                    mode: if dry_run { "dry-run" } else { "applied" },
                },
                stdout: outcome.stdout,
                stderr: outcome.stderr,
                command: outcome.command,
                diagnostics,
            })
        }
    }

    #[cfg(test)]
    fn test_workspace_context(
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<crate::operations::WorkspaceContext, String> {
        let mut paths = args
            .values()
            .filter_map(serde_json::Value::as_str)
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| {
                if path.is_dir() {
                    path
                } else {
                    path.parent()
                        .unwrap_or_else(|| std::path::Path::new(""))
                        .to_path_buf()
                }
            })
            .collect::<Vec<_>>();
        paths.sort();
        let mut root = paths
            .first()
            .cloned()
            .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?);
        for path in paths.iter().skip(1) {
            while !path.starts_with(&root) && root.pop() {}
        }
        for candidate in root.ancestors() {
            if candidate.join("v8project.yaml").is_file() {
                root = candidate.to_path_buf();
                break;
            }
        }
        Ok(crate::operations::WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        })
    }
}

mod domain {
    pub(crate) use unica_format_core::limits as navigation_limits;
    pub(crate) use unica_format_core::navigation;
    pub(crate) use unica_format_core::source as source_adapters;

    pub(crate) mod form_edit {
        pub(crate) use crate::operations::form_edit::*;
    }

    pub(crate) mod format_profile {
        pub(crate) use crate::operations::{FormatProfile, ACTIVE_FORMAT_PROFILE};
        pub(crate) use unica_format_core::ports::FormatCompatibility;
    }

    pub(crate) mod identifiers {
        pub(crate) use crate::operations::{
            is_1c_identifier, is_1c_identifier_part, is_1c_identifier_start,
        };
    }

    pub(crate) mod project_sources {
        pub(crate) use crate::operations::project_source_types::*;
    }

    pub(crate) mod source_roots {
        pub(crate) use crate::operations::source_root_types::*;
    }

    pub(crate) mod workspace {
        pub(crate) use crate::operations::WorkspaceContext;
    }
}

mod infrastructure {
    pub(crate) mod native_operations {
        pub(crate) use crate::operations::{
            cf, compile_transaction, external, single_file_publisher, NativeOperationAdapter,
        };
    }

    pub(crate) mod platform {
        pub(crate) mod filesystem {
            pub(crate) use crate::operations::filesystem::*;
        }

        #[cfg(test)]
        pub(crate) mod testing {
            pub(crate) use crate::operations::testing::*;
        }
    }

    pub(crate) mod platform_xml_owner {
        pub(crate) use crate::operations::platform_xml_owner::*;
    }

    pub(crate) mod project_sources {
        pub(crate) use crate::operations::project_sources::*;
    }

    pub(crate) mod source_roots {
        pub(crate) use crate::operations::source_roots::*;
    }

    #[cfg(test)]
    pub(crate) mod workspace {
        pub(crate) fn discover_workspace(
            root: Option<std::path::PathBuf>,
        ) -> Result<crate::operations::WorkspaceContext, String> {
            let workspace_root =
                root.unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?);
            Ok(crate::operations::WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            })
        }
    }

    #[cfg(test)]
    pub(crate) mod support_guard {
        use std::path::Path;
        use unica_format_core::ports::{
            AuthorabilityRequest, AuthorabilityRequirement, AuthorabilityResult,
            FormatDiagnosticCode, OwnerResolutionMode,
        };

        #[derive(Debug, Clone, PartialEq, Eq)]
        pub(crate) struct SupportGuardViolation {
            pub(crate) code: &'static str,
            pub(crate) reason: String,
        }

        pub(crate) fn support_guard_violation(
            target: &Path,
            requirement: crate::operations::operation_descriptors::SupportGuardRequirement,
        ) -> Option<SupportGuardViolation> {
            let root = target
                .parent()
                .and_then(Path::parent)
                .unwrap_or_else(|| Path::new(""));
            let factory = crate::PlatformXmlAdapterFactory::new();
            let session =
                factory.capture_unscoped_source(target, root, OwnerResolutionMode::Existing);
            let request = AuthorabilityRequest::new(
                session,
                match requirement {
                    crate::operations::operation_descriptors::SupportGuardRequirement::Editable => {
                        AuthorabilityRequirement::Editable
                    }
                    crate::operations::operation_descriptors::SupportGuardRequirement::Removed => {
                        AuthorabilityRequirement::Removed
                    }
                },
            );
            match factory.authorability_port().inspect(&request) {
                Ok(AuthorabilityResult::Allowed(_)) => None,
                Ok(AuthorabilityResult::Denied(denial)) => {
                    let (code, reason) = match denial.diagnostic().code() {
                        FormatDiagnosticCode::SupportCapabilityDisabled => (
                            "capability-off",
                            "Source editing is disabled by support policy.",
                        ),
                        FormatDiagnosticCode::SupportRemovalRequired => (
                            "not-removed",
                            "The object must be removed from support before this operation.",
                        ),
                        FormatDiagnosticCode::SupportLocked => {
                            ("locked", "The object is locked by support policy.")
                        }
                        _ => (
                            "support-state-unreadable",
                            "состояние поддержки не удалось прочитать — правки не подтверждены",
                        ),
                    };
                    Some(SupportGuardViolation {
                        code,
                        reason: reason.to_string(),
                    })
                }
                Err(_) => Some(SupportGuardViolation {
                    code: "support-state-unreadable",
                    reason: "состояние поддержки не удалось прочитать — правки не подтверждены"
                        .to_string(),
                }),
            }
        }
    }

    #[cfg(test)]
    pub(crate) mod source_adapters {
        pub(crate) mod platform_xml {
            pub(crate) use crate::versions::v2_20::*;
        }
    }
}

#[cfg(test)]
mod certification;
