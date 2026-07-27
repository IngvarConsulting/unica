use std::path::{Path, PathBuf};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_application::OperationalPolicyService;
use unica_format_core::ports::{ValidationContextRequest, ValidationOwnerKind};

use crate::{
    domain::workspace::WorkspaceContext,
    infrastructure::platform_xml_owner::validation_source_context,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaValidationOwnerKind {
    Configuration,
    Extension,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaValidationOwnerContext {
    pub owner_kind: MetaValidationOwnerKind,
    pub owner_path: PathBuf,
    pub language_codes: Vec<String>,
    pub registrar_present: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct MetaValidationReadInspection {
    pub paths: Vec<PathBuf>,
    pub context: Result<MetaValidationOwnerContext, String>,
}

pub(crate) fn meta_validate_types_with_list_presentation() -> &'static [&'static str] {
    PlatformXmlAdapterFactory::validation_types_with_list_presentation()
}

pub(crate) fn inspect_meta_validation_reads(
    object_path: &Path,
    workspace: &WorkspaceContext,
) -> MetaValidationReadInspection {
    let source = match validation_source_context(object_path, workspace) {
        Ok(source) => source,
        Err(error) => {
            return MetaValidationReadInspection {
                paths: vec![object_path.to_path_buf()],
                context: Err(error.message),
            }
        }
    };
    let port = PlatformXmlAdapterFactory::new().validation_context_port();
    match OperationalPolicyService::validation_context(
        port.as_ref(),
        &ValidationContextRequest { source },
    ) {
        Ok(result) => {
            let context = result
                .context
                .map(|context| MetaValidationOwnerContext {
                    owner_kind: match context.owner_kind {
                        ValidationOwnerKind::Aggregate => MetaValidationOwnerKind::Configuration,
                        ValidationOwnerKind::Extension => MetaValidationOwnerKind::Extension,
                        ValidationOwnerKind::Standalone => MetaValidationOwnerKind::External,
                    },
                    owner_path: context.owner_root,
                    language_codes: context.language_codes,
                    registrar_present: context.registrar_present,
                })
                .ok_or_else(|| {
                    result
                        .diagnostics
                        .first()
                        .map(|diagnostic| diagnostic.message.clone())
                        .unwrap_or_else(|| "validation context is unavailable".to_string())
                });
            MetaValidationReadInspection {
                paths: result.dependencies,
                context,
            }
        }
        Err(error) => MetaValidationReadInspection {
            paths: vec![object_path.to_path_buf()],
            context: Err(error.message),
        },
    }
}

pub(crate) fn meta_validate_registrar_document_scan(
    documents_dir: &Path,
    register_reference: &str,
) -> Result<(Vec<PathBuf>, bool), String> {
    PlatformXmlAdapterFactory::scan_validation_registrars(documents_dir, register_reference)
}
