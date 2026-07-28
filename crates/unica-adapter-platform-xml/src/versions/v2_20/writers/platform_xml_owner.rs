use std::{fs, path::Path};

use crate::PlatformXmlAdapterFactory;
use sha2::{Digest, Sha256};
pub(crate) use unica_format_core::ports::{
    FormatCompatibility, SourceOwnerEvidence as PlatformXmlOwner,
};
use unica_format_core::{
    ports::{
        FormatInspectionMode, FormatInspectionRequest, ObjectKindSelector, OwnerResolutionMode,
        OwnerResolutionRequest, SourceInputEvidence,
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
                    if !transaction.removes_path_or_ancestor(directory)? {
                        transaction.guard_or_verify_directory_membership(
                            directory,
                            DirectoryMembershipSelector::XmlFiles,
                            names.clone(),
                        )?;
                    }
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

pub(crate) fn resolve_platform_xml_owners(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<Vec<PlatformXmlOwner>, PlatformXmlOwnerError> {
    resolve_platform_xml_owners_with_provenance(target, context).map(|resolution| resolution.owners)
}

pub(crate) fn resolve_platform_xml_owners_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve(target, context, OwnerResolutionMode::Existing)
}

pub(crate) fn resolve_existing_platform_xml_owners_for_new_output_with_provenance(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    resolve(target, context, OwnerResolutionMode::ExistingForNewOutput)
}

pub(crate) fn inspect_platform_xml_compatibility(
    target: &Path,
) -> Result<FormatCompatibility, PlatformXmlOwnerError> {
    let target =
        normalize_path_identity(target).map_err(|message| PlatformXmlOwnerError { message })?;
    let result = PlatformXmlAdapterFactory::new()
        .registration()
        .format_inspection
        .inspect(&FormatInspectionRequest {
            source: inspection_source(&target),
            mode: FormatInspectionMode::Versioned,
        })
        .map_err(|error| PlatformXmlOwnerError {
            message: error.message,
        })?;
    result.compatibility.ok_or_else(|| PlatformXmlOwnerError {
        message: "platform XML target has no format-owning root".to_string(),
    })
}

fn resolve(
    target: &Path,
    context: &WorkspaceContext,
    mode: OwnerResolutionMode,
) -> Result<PlatformXmlOwnerResolution, PlatformXmlOwnerError> {
    let (source, source_map_provenance) = operation_source_context(target, context)?;
    let result = PlatformXmlAdapterFactory::new()
        .registration()
        .ownership
        .resolve(&OwnerResolutionRequest { source, mode })
        .map_err(|error| PlatformXmlOwnerError {
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

fn operation_source_context(
    target: &Path,
    context: &WorkspaceContext,
) -> Result<(SourceContext, ProjectSourceMapProvenance), PlatformXmlOwnerError> {
    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|message| PlatformXmlOwnerError { message })?;
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        context.cwd.join(target)
    };
    let target =
        normalize_path_identity(&target).map_err(|message| PlatformXmlOwnerError { message })?;
    let (source_map, source_map_provenance) =
        discover_project_source_map_with_provenance(&context.workspace_root)
            .map_err(|message| PlatformXmlOwnerError { message })?;
    let mut containing = Vec::new();
    for source_set in &source_map.source_sets {
        let source_root =
            normalize_contained_source_root(&context.workspace_root, &source_set.path)
                .map_err(|message| PlatformXmlOwnerError { message })?;
        if target.starts_with(&source_root) {
            containing.push((source_set, source_root));
        }
    }
    let selected = select_unique_deepest_source_set_match(&target, containing)
        .map_err(|message| PlatformXmlOwnerError { message })?;
    let has_explicit_source_map = source_map.config_path.is_some();
    let (configured_source_set, configured_kind, source_root) = match selected {
        Some((source_set, source_root)) => (
            Some(source_set.name.clone()),
            has_explicit_source_map.then(|| configured_kind(source_set.kind)),
            source_root,
        ),
        None if target.starts_with(&workspace_root) && target.is_dir() => {
            (None, None, target.clone())
        }
        None if target.starts_with(&workspace_root) => (
            None,
            None,
            target
                .parent()
                .unwrap_or(workspace_root.as_path())
                .to_path_buf(),
        ),
        None => {
            return Err(PlatformXmlOwnerError {
                message: "platform XML target is outside the workspace containment root"
                    .to_string(),
            });
        }
    };
    let source = SourceContext::new(
        SourceLocation::new(workspace_root, source_root, target.clone()),
        configured_source_set,
        SourceFamily::PlatformXml,
        None,
    )
    .with_configured_source_set_kind(configured_kind);
    Ok((source, source_map_provenance))
}

fn inspection_source(target: &Path) -> SourceContext {
    let source_root = target
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    SourceContext::new(
        SourceLocation::new(source_root.clone(), source_root, target.to_path_buf()),
        None,
        SourceFamily::PlatformXml,
        None,
    )
}

fn configured_kind(kind: SourceSetKind) -> ConfiguredSourceSetKind {
    match kind {
        SourceSetKind::Configuration => ConfiguredSourceSetKind::Configuration,
        SourceSetKind::Extension => ConfiguredSourceSetKind::Extension,
        SourceSetKind::ExternalProcessor => ConfiguredSourceSetKind::ExternalProcessor,
        SourceSetKind::ExternalReport => ConfiguredSourceSetKind::ExternalReport,
    }
}

pub(crate) fn task8_metadata_kind(
    value: &str,
) -> Option<unica_format_core::semantic_ids::SemanticObjectKind> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    registration
        .object_kinds()
        .resolve(&ObjectKindSelector::new(value).ok()?)
}

pub(crate) fn task8_metadata_kind_by_directory(
    value: &str,
) -> Option<unica_format_core::semantic_ids::SemanticObjectKind> {
    task8_metadata_kind(value)
}

pub(crate) fn task8_metadata_kind_tags() -> Vec<&'static str> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    registration
        .object_kinds()
        .ordered_kinds()
        .into_iter()
        .filter_map(|kind| registration.object_kinds().lease(kind))
        .filter_map(|lease| registration.object_kinds().project(&lease))
        .map(|projection| projection.canonical_selector().as_str())
        .collect()
}

pub(crate) fn task8_metadata_kind_index(value: &str) -> Option<usize> {
    let kind = task8_metadata_kind(value)?;
    PlatformXmlAdapterFactory::new()
        .operational_registration()
        .object_kinds()
        .ordered_kinds()
        .iter()
        .position(|candidate| *candidate == kind)
}

pub(crate) fn task8_metadata_kind_tag(
    kind: unica_format_core::semantic_ids::SemanticObjectKind,
) -> Option<&'static str> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    let lease = registration.object_kinds().lease(kind)?;
    registration
        .object_kinds()
        .project(&lease)
        .map(|projection| projection.canonical_selector().as_str())
}

pub(crate) fn task8_metadata_kind_directory(value: &str) -> Option<&'static str> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    let kind = registration
        .object_kinds()
        .resolve(&ObjectKindSelector::new(value).ok()?)?;
    let lease = registration.object_kinds().lease(kind)?;
    registration
        .object_kinds()
        .project(&lease)
        .map(|projection| projection.collection_selector().as_str())
}

pub(crate) fn task8_metadata_kind_display_name_ru(value: &str) -> Option<&'static str> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    let kind = registration
        .object_kinds()
        .resolve(&ObjectKindSelector::new(value).ok()?)?;
    let lease = registration.object_kinds().lease(kind)?;
    registration
        .object_kinds()
        .project(&lease)
        .map(|projection| projection.display_label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn equal_depth_source_sets_are_ambiguous_in_both_orders_and_modes() {
        let orders = [
            "  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
            "  - name: configuration\n    type: CONFIGURATION\n    path: src\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n",
        ];
        for (index, source_sets) in orders.iter().enumerate() {
            let context = temp_context(&format!("ambiguous-{index}"));
            fs::write(
                context.cwd.join("v8project.yaml"),
                format!("format: DESIGNER\nsource-set:\n{source_sets}"),
            )
            .unwrap();
            let target = context.cwd.join("src/Demo/Ext/ObjectModule.bsl");
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            for mode in [
                OwnerResolutionMode::Existing,
                OwnerResolutionMode::ExistingForNewOutput,
            ] {
                let error = resolve(&target, &context, mode).unwrap_err();
                assert!(error.message.contains("ambiguous source-set"), "{error:?}");
                assert!(error.message.contains("external"), "{error:?}");
                assert!(error.message.contains("configuration"), "{error:?}");
            }
            fs::remove_dir_all(context.cwd).unwrap();
        }
    }

    #[test]
    fn deepest_source_set_boundary_excludes_the_outer_owner() {
        let context = temp_context("deepest-boundary");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: outer\n    type: CONFIGURATION\n    path: src\n  - name: nested\n    type: CONFIGURATION\n    path: src/new\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/new")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let target = context.cwd.join("src/new/Missing.xml");
        assert!(
            resolve_existing_platform_xml_owners_for_new_output_with_provenance(&target, &context)
                .unwrap()
                .owners
                .is_empty()
        );

        let nested = context.cwd.join("src/new/Configuration.xml");
        fs::write(
            &nested,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let owners =
            resolve_existing_platform_xml_owners_for_new_output_with_provenance(&target, &context)
                .unwrap()
                .owners;
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].path, fs::canonicalize(nested).unwrap());
        fs::remove_dir_all(context.cwd).unwrap();
    }

    #[test]
    fn owner_provenance_rejects_project_map_remap_before_binding() {
        let context = temp_context("project-map-race");
        let project_map = context.cwd.join("v8project.yaml");
        fs::write(
            &project_map,
            "format: DESIGNER\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let target = context.cwd.join("src/Demo/Ext/ObjectModule.bsl");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let resolution = resolve_platform_xml_owners_with_provenance(&target, &context).unwrap();
        fs::write(
            &project_map,
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n",
        )
        .unwrap();

        let error = resolution
            .provenance
            .bind_to(&mut CompileTransaction::new())
            .unwrap_err();

        assert!(error.contains("v8project.yaml"), "{error}");
        assert!(error.contains("changed"), "{error}");
        fs::remove_dir_all(context.cwd).unwrap();
    }

    #[test]
    fn owner_provenance_rejects_a_wrapper_created_after_resolution() {
        let context = temp_context("late-wrapper-race");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let wrapper = context.cwd.join("src/Reports/Sales/Templates/Planned.xml");
        let content = context
            .cwd
            .join("src/Reports/Sales/Templates/Planned/Ext/Template.xml");
        fs::create_dir_all(content.parent().unwrap()).unwrap();
        fs::write(
            &content,
            r#"<Template xmlns="http://v8.1c.ru/8.3/xcf/data"/>"#,
        )
        .unwrap();
        let resolution = resolve_platform_xml_owners_with_provenance(&content, &context).unwrap();
        fs::write(
            &wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template/></MetaDataObject>"#,
        )
        .unwrap();

        let error = resolution
            .provenance
            .bind_to(&mut CompileTransaction::new())
            .unwrap_err();

        assert!(error.contains("Planned.xml"), "{error}");
        assert!(error.contains("absence guard"), "{error}");
        fs::remove_dir_all(context.cwd).unwrap();
    }

    #[test]
    fn owner_provenance_rejects_external_membership_growth() {
        let context = temp_context("external-membership-race");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        let source_root = context.cwd.join("external");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(
            source_root.join("Existing.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();
        let resolution = resolve_existing_platform_xml_owners_for_new_output_with_provenance(
            &source_root,
            &context,
        )
        .unwrap();
        fs::write(
            source_root.join("Late.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();

        let error = resolution
            .provenance
            .bind_to(&mut CompileTransaction::new())
            .unwrap_err();

        assert!(error.contains("directory membership"), "{error}");
        assert!(error.contains("Late.xml"), "{error}");
        fs::remove_dir_all(context.cwd).unwrap();
    }

    fn temp_context(label: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-platform-owner-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }
}
