use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::source::{SourceContext, SourceFamily, SourceLocation};

use crate::{
    domain::workspace::WorkspaceContext,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlOwnerKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
    Standalone,
}

impl PlatformXmlOwnerKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Extension => "extension",
            Self::ExternalProcessor => "external_processor",
            Self::ExternalReport => "external_report",
            Self::Standalone => "standalone",
        }
    }

    fn parse(label: &str) -> Option<Self> {
        match label {
            "configuration" => Some(Self::Configuration),
            "extension" => Some(Self::Extension),
            "external_processor" => Some(Self::ExternalProcessor),
            "external_report" => Some(Self::ExternalReport),
            "standalone" => Some(Self::Standalone),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwner {
    pub kind: PlatformXmlOwnerKind,
    pub path: PathBuf,
    pub version: Option<String>,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwnerError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlatformXmlOwnerCandidateInput {
    ExactFile(Vec<u8>),
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformXmlOwnerProvenance {
    source_map: ProjectSourceMapProvenance,
    candidates: BTreeMap<PathBuf, PlatformXmlOwnerCandidateInput>,
    directory_memberships: BTreeMap<PathBuf, Vec<OsString>>,
}

impl PlatformXmlOwnerProvenance {
    pub(crate) fn bind_to(&self, transaction: &mut CompileTransaction) -> Result<(), String> {
        self.source_map.bind_to(transaction)?;
        for (path, input) in &self.candidates {
            if transaction.protects_path(path)? {
                continue;
            }
            match input {
                PlatformXmlOwnerCandidateInput::ExactFile(raw) => {
                    transaction.guard_or_verify_exact_preimage(path, raw)?;
                }
                PlatformXmlOwnerCandidateInput::Absent => transaction.guard_path_absent(path)?,
            }
        }
        for (directory, expected_names) in &self.directory_memberships {
            transaction.guard_or_verify_directory_membership(
                directory,
                DirectoryMembershipSelector::XmlFiles,
                expected_names.clone(),
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformXmlOwnerResolution {
    pub owners: Vec<PlatformXmlOwner>,
    pub provenance: PlatformXmlOwnerProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformXmlRootExpectation {
    pub(crate) namespace: &'static str,
    pub(crate) local_name: &'static str,
}

impl PlatformXmlRootExpectation {
    pub(crate) const fn new(namespace: &'static str, local_name: &'static str) -> Self {
        Self {
            namespace,
            local_name,
        }
    }
}

pub(crate) const MANAGED_FORM_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::new("http://v8.1c.ru/8.3/xcf/logform", "Form");
pub(crate) const DCS_ROOT: PlatformXmlRootExpectation = PlatformXmlRootExpectation::new(
    "http://v8.1c.ru/8.1/data-composition-system/schema",
    "DataCompositionSchema",
);
pub(crate) const MXL_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::new("http://v8.1c.ru/8.2/data/spreadsheet", "document");

pub(crate) fn root_version_literal(source: &str, root: roxmltree::Node<'_, '_>) -> Option<String> {
    root.attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == "version")
        .and_then(|attribute| source.get(attribute.range_value()))
        .map(str::to_owned)
}

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
    resolve(target, context, None, false)
}

pub(crate) fn resolve_platform_xml_owners_for_exact_root_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: PlatformXmlRootExpectation,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve(target, context, Some(expected_root), false)
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
    resolve(target, context, None, true)
}

fn resolve(
    target: &Path,
    context: &WorkspaceContext,
    expected_root: Option<PlatformXmlRootExpectation>,
    existing_only: bool,
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
    let (configured_source_set, source_root) = match selected {
        Some((source_set, source_root)) => (Some(source_set.name.clone()), source_root),
        None if target.is_dir() => (None, target.clone()),
        None => (
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
    );
    let owners = RefCell::new(Vec::new());
    let candidates = RefCell::new(BTreeMap::new());
    let memberships = RefCell::new(BTreeMap::new());
    PlatformXmlAdapterFactory::resolve_owners(
        &source,
        expected_root.map(|root| (root.namespace, root.local_name)),
        existing_only,
        |kind, path, version, raw| {
            let kind = PlatformXmlOwnerKind::parse(kind)
                .expect("adapter owner kind labels are a closed contract");
            owners.borrow_mut().push(PlatformXmlOwner {
                kind,
                path: path.to_path_buf(),
                version: version.map(str::to_string),
                raw: raw.to_vec(),
            });
        },
        |path, raw| {
            candidates.borrow_mut().insert(
                path.to_path_buf(),
                PlatformXmlOwnerCandidateInput::ExactFile(raw.to_vec()),
            );
        },
        |path| {
            candidates
                .borrow_mut()
                .insert(path.to_path_buf(), PlatformXmlOwnerCandidateInput::Absent);
        },
        |directory, names| {
            memberships
                .borrow_mut()
                .insert(directory.to_path_buf(), names.to_vec());
        },
    )
    .map_err(|error| PlatformXmlOwnerError {
        path: target,
        message: error.message,
    })?;
    Ok(PlatformXmlOwnerResolution {
        owners: owners.into_inner(),
        provenance: PlatformXmlOwnerProvenance {
            source_map: source_map_provenance,
            candidates: candidates.into_inner(),
            directory_memberships: memberships.into_inner(),
        },
    })
}
