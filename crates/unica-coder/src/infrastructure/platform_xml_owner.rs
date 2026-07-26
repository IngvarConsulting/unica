use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
pub(crate) use unica_format_core::ports::{
    FormatCompatibility, SourceArtifactKind as PlatformXmlRootExpectation,
    SourceOwnerEvidence as PlatformXmlOwner, SourceOwnerKind as PlatformXmlOwnerKind,
};
use unica_format_core::{
    ports::{
        FormatInspectionMode, FormatInspectionRequest, OwnerResolutionMode, OwnerResolutionRequest,
        SourceInputEvidence,
    },
    source::{ConfiguredSourceSetKind, SourceContext, SourceFamily, SourceLocation},
};

use crate::{
    domain::{project_sources::SourceSetKind, workspace::WorkspaceContext},
    infrastructure::{
        native_operations::compile_transaction::{CompileTransaction, DirectoryMembershipSelector},
        project_sources::{
            discover_project_source_map_with_provenance, ProjectSourceMapProvenance,
        },
        source_roots::{
            normalize_contained_source_root, normalize_path_identity,
            select_unique_deepest_source_set_match,
        },
    },
};

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwnerError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformXmlOwnerProvenance {
    source_map: ProjectSourceMapProvenance,
    evidence: Vec<SourceInputEvidence>,
}

impl PlatformXmlOwnerProvenance {
    pub(crate) fn bind_to(&self, transaction: &mut CompileTransaction) -> Result<(), String> {
        self.source_map.bind_to(transaction)?;
        for evidence in &self.evidence {
            match evidence {
                SourceInputEvidence::ExactFileSha256 { path, sha256 } => {
                    if transaction.protects_path(path)? {
                        continue;
                    }
                    let raw = fs::read(path)
                        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
                    let actual = format!("{:x}", Sha256::digest(&raw));
                    if &actual != sha256 {
                        return Err(format!(
                            "platform XML owner changed while binding evidence: {}",
                            path.display()
                        ));
                    }
                    transaction.guard_or_verify_exact_preimage(path, &raw)?;
                }
                SourceInputEvidence::PathAbsent { path } => {
                    if !transaction.protects_path(path)? {
                        transaction.guard_path_absent(path)?;
                    }
                }
                SourceInputEvidence::DirectoryMembership { directory, names } => {
                    transaction.guard_or_verify_directory_membership(
                        directory,
                        DirectoryMembershipSelector::XmlFiles,
                        names.clone(),
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwnerResolution {
    pub owners: Vec<PlatformXmlOwner>,
    pub provenance: PlatformXmlOwnerProvenance,
}

pub(crate) const MANAGED_FORM_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::ManagedForm;
pub(crate) const DCS_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::DataCompositionSchema;
pub(crate) const MXL_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::SpreadsheetDocument;

pub(crate) fn resolve_platform_xml_owners(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    resolve_platform_xml_owners_with_provenance(target, context).map(|resolution| resolution.owners)
}

pub(crate) fn resolve_platform_xml_owners_for_exact_root(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: PlatformXmlRootExpectation,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    resolve_platform_xml_owners_for_exact_root_with_provenance(target, context, expected_root)
        .map(|resolution| resolution.owners)
}

pub(crate) fn resolve_platform_xml_owners_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve(target, context, None, OwnerResolutionMode::Existing)
}

pub(crate) fn resolve_platform_xml_owners_for_exact_root_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: PlatformXmlRootExpectation,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve(
        target,
        context,
        Some(expected_root),
        OwnerResolutionMode::Existing,
    )
}

pub(crate) fn resolve_existing_platform_xml_owners_for_new_output(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    resolve_existing_platform_xml_owners_for_new_output_with_provenance(target, context)
        .map(|resolution| resolution.owners)
}

pub(crate) fn resolve_existing_platform_xml_owners_for_new_output_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve(
        target,
        context,
        None,
        OwnerResolutionMode::ExistingForNewOutput,
    )
}

pub(crate) fn inspect_platform_xml_compatibility(
    target: &Path,
    _expected_artifact: Option<PlatformXmlRootExpectation>,
) -> Result<FormatCompatibility, PlatformXmlOwnerError> {
    let target = normalize_path_identity(target).map_err(|message| PlatformXmlOwnerError {
        path: target.to_path_buf(),
        message,
    })?;
    let result = PlatformXmlAdapterFactory::new()
        .registration()
        .format_inspection
        .inspect(&FormatInspectionRequest {
            path: target.clone(),
            mode: FormatInspectionMode::Versioned,
        })
        .map_err(|error| PlatformXmlOwnerError {
            path: target.clone(),
            message: error.message,
        })?;
    result.compatibility.ok_or_else(|| PlatformXmlOwnerError {
        path: target,
        message: "platform XML target has no format-owning root".to_string(),
    })
}

pub(crate) fn inspect_platform_xml_versionless(target: &Path) -> Result<(), PlatformXmlOwnerError> {
    let target = normalize_path_identity(target).map_err(|message| PlatformXmlOwnerError {
        path: target.to_path_buf(),
        message,
    })?;
    PlatformXmlAdapterFactory::new()
        .registration()
        .format_inspection
        .inspect(&FormatInspectionRequest {
            path: target.clone(),
            mode: FormatInspectionMode::Versionless,
        })
        .map(|_| ())
        .map_err(|error| PlatformXmlOwnerError {
            path: target,
            message: error.message,
        })
}

fn resolve(
    target: &Path,
    context: &WorkspaceContext,
    expected_artifact: Option<PlatformXmlRootExpectation>,
    mode: OwnerResolutionMode,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        context.cwd.join(target)
    };
    let target = normalize_path_identity(&target).map_err(|message| PlatformXmlOwnerError {
        path: target.clone(),
        message,
    })?;
    let (source_map, source_map_provenance) = discover_project_source_map_with_provenance(
        &context.workspace_root,
    )
    .map_err(|message| PlatformXmlOwnerError {
        path: context.workspace_root.clone(),
        message,
    })?;
    let mut containing = Vec::new();
    for source_set in &source_map.source_sets {
        let source_root =
            normalize_contained_source_root(&context.workspace_root, &source_set.path).map_err(
                |message| PlatformXmlOwnerError {
                    path: context.workspace_root.join(&source_set.path),
                    message,
                },
            )?;
        if target.starts_with(&source_root) {
            containing.push((source_set, source_root));
        }
    }
    let selected =
        select_unique_deepest_source_set_match(&target, containing).map_err(|message| {
            PlatformXmlOwnerError {
                path: target.clone(),
                message,
            }
        })?;
    let (configured_source_set, configured_kind, source_root) = match selected {
        Some((source_set, source_root)) => (
            Some(source_set.name.clone()),
            Some(configured_kind(source_set.kind)),
            source_root,
        ),
        None if target.is_dir() => (None, None, target.clone()),
        None => (
            None,
            None,
            target
                .parent()
                .unwrap_or(context.workspace_root.as_path())
                .to_path_buf(),
        ),
    };
    let source = SourceContext::new(
        SourceLocation::new(context.workspace_root.clone(), source_root, target.clone()),
        configured_source_set,
        SourceFamily::PlatformXml,
        None,
    )
    .with_configured_source_set_kind(configured_kind);
    let result = PlatformXmlAdapterFactory::new()
        .registration()
        .ownership
        .resolve(&OwnerResolutionRequest {
            source,
            expected_artifact,
            mode,
        })
        .map_err(|error| PlatformXmlOwnerError {
            path: target,
            message: error.message,
        })?;
    Ok(PlatformXmlOwnerResolution {
        owners: result.owners,
        provenance: PlatformXmlOwnerProvenance {
            source_map: source_map_provenance,
            evidence: result.evidence,
        },
    })
}

fn configured_kind(kind: SourceSetKind) -> ConfiguredSourceSetKind {
    match kind {
        SourceSetKind::Configuration => ConfiguredSourceSetKind::Configuration,
        SourceSetKind::Extension => ConfiguredSourceSetKind::Extension,
        SourceSetKind::ExternalProcessor => ConfiguredSourceSetKind::ExternalProcessor,
        SourceSetKind::ExternalReport => ConfiguredSourceSetKind::ExternalReport,
    }
}
