//! Trusted binding of selected source evidence to the metadata template reader.
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    ResolvedSourceSet, TargetKindPolicy,
};
use std::io::Read;
use std::path::{Component, Path};
use unica_address::source_target::SourceTarget;
use unica_metadata_designer::template_catalog::{
    MetadataModuleEvidence, MetadataModuleResolution, MetadataTemplateRead, MetadataTemplateSource,
};
use unica_source_contract::read::SourceDirectory;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::paths::metadata_is_link_or_reparse_point;

pub(crate) struct MetadataTemplateReadAdapter<'a> {
    source: &'a ResolvedSourceSet,
    context: &'a WorkspaceContext,
    retained: Option<&'a SourceDirectory>,
}
impl<'a> MetadataTemplateReadAdapter<'a> {
    pub(crate) fn new(
        source: &'a ResolvedSourceSet,
        context: &'a WorkspaceContext,
        retained: Option<&'a SourceDirectory>,
    ) -> Self {
        Self {
            source,
            context,
            retained,
        }
    }
    pub(crate) fn source(&self) -> MetadataTemplateSource<'_, ClosedPlatformXmlTarget> {
        MetadataTemplateSource {
            source_root: &self.source.source_root,
            owner_path: &self.source.owner_path,
            owner_preimage: &self.source.owner_preimage,
            format_version: &self.source.format_version,
            reader: self,
            fresh_uuid: crate::infrastructure::source_ids::fresh_uuid,
        }
    }
}
impl MetadataTemplateRead for MetadataTemplateReadAdapter<'_> {
    type Guard = ClosedPlatformXmlTarget;
    fn resolve_module(
        &self,
        target: &SourceTarget,
    ) -> Option<MetadataModuleResolution<Self::Guard>> {
        let resolution =
            resolve_platform_xml_target(self.context, target, TargetKindPolicy::ModuleOnly).ok()?;
        Some(MetadataModuleResolution {
            target_kind: resolution.resolved.target_kind,
            handle: resolution.handle,
        })
    }
    fn resource_evidence(&self, handle: &Self::Guard) -> Option<MetadataModuleEvidence> {
        let evidence = platform_xml_resource_evidence(self.context, handle).ok()?;
        Some(MetadataModuleEvidence {
            source_root: evidence.source_root,
            registration_path: evidence.registration_path,
            target_path: evidence.target_path,
        })
    }
    fn read_relative_regular_file(
        &self,
        relative: &Path,
    ) -> Result<Vec<u8>, unica_metadata_designer::resource_plan::MetadataReadFailure> {
        if let Some(root) = self.retained {
            let mut file = root.open_relative_regular_nofollow(relative).map_err(|_| {
                unica_metadata_designer::resource_plan::MetadataReadFailure::UNAVAILABLE
            })?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|_| {
                unica_metadata_designer::resource_plan::MetadataReadFailure::UNAVAILABLE
            })?;
            Ok(bytes)
        } else {
            read_contained_regular_file(&self.source.source_root, relative).map_err(|_| {
                unica_metadata_designer::resource_plan::MetadataReadFailure::UNAVAILABLE
            })
        }
    }
}
/// Read a prerequisite only through the exact selected source-root topology.
/// `fs::read` follows linked ancestors, which would let a registered metadata
/// name redirect template discovery outside its source set before the
/// transaction has a chance to bind the dependency.
fn read_contained_regular_file(source_root: &Path, relative_path: &Path) -> Result<Vec<u8>, ()> {
    if relative_path.is_absolute() {
        return Err(());
    }

    let mut current = source_root.to_path_buf();
    let mut components = relative_path.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(component) = component else {
            return Err(());
        };
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current).map_err(|_| ())?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(());
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return Err(());
            }
        } else if !metadata.is_file() {
            return Err(());
        }
    }

    std::fs::read(current).map_err(|_| ())
}

/// Binds the existing source selection and ordinary diagnostic reads to one
/// canonical metadata planning call. Construction performs no observations.
pub(crate) struct MetadataPlanningReadAdapter<'a> {
    authority: &'a dyn unica_source_contract::proposal::ApplySourceAuthority,
    ordinary: unica_workspace::discovery::ProjectDiscoveryReader,
}
impl<'a> MetadataPlanningReadAdapter<'a> {
    pub(crate) fn new(
        authority: &'a dyn unica_source_contract::proposal::ApplySourceAuthority,
    ) -> Self {
        Self {
            authority,
            ordinary: unica_workspace::discovery::ProjectDiscoveryReader::new(
                &authority.workspace_context().cwd,
            ),
        }
    }
}
impl unica_metadata_designer::planner::MetadataPlanningRead for MetadataPlanningReadAdapter<'_> {
    type Guard = ClosedPlatformXmlTarget;
    fn with_creation_source(
        &self,
        op_index: usize,
        consume: &mut dyn FnMut(
            &MetadataTemplateSource<'_, Self::Guard>,
        )
            -> Result<(), unica_source_contract::proposal::ApplyPlanError>,
    ) -> Result<(), unica_source_contract::proposal::ApplyPlanError> {
        let source =
            crate::infrastructure::platform_xml_source_targets::resolve_metadata_add_source(
                self.authority.workspace_context(),
                self.authority.source_set_name(),
            )
            .map_err(|failure| {
                unica_metadata_designer::planner::meta_failure_to_plan_error(failure, op_index)
            })?;
        let retained = self.authority.source_reader();
        let reader = MetadataTemplateReadAdapter::new(
            &source,
            self.authority.workspace_context(),
            Some(retained.as_ref()),
        );
        consume(&reader.source())
    }
    fn ordinary_read(&self) -> &dyn unica_source_contract::discovery::ProjectDiscoveryRead {
        &self.ordinary
    }
    fn removal_observations(&self) -> unica_metadata_designer::remove::RemoveObservationHooks {
        super::remove::observation_hooks()
    }
}

/// Bind only the original resolver, evidence and ordinary diagnostic reads.
/// Constructing this adapter performs no source observation.
pub(crate) struct MetadataDependencyReadAdapter<'a> {
    pub(super) context: &'a unica_source_contract::workspace::WorkspaceContext,
    pub(super) ordinary: unica_workspace::discovery::ProjectDiscoveryReader,
}
impl<'a> MetadataDependencyReadAdapter<'a> {
    pub(crate) fn new(context: &'a unica_source_contract::workspace::WorkspaceContext) -> Self {
        Self {
            context,
            ordinary: unica_workspace::discovery::ProjectDiscoveryReader::new(&context.cwd),
        }
    }
}
impl unica_metadata_designer::relations::MetadataDependencyRead
    for MetadataDependencyReadAdapter<'_>
{
    type Guard = ClosedPlatformXmlTarget;
    fn resolve_object(
        &self,
        target: &SourceTarget,
    ) -> Result<
        (unica_address::source_target::ResolvedTarget, Self::Guard),
        unica_metadata_designer::resource_plan::MetadataReadFailure,
    > {
        let resolution = resolve_platform_xml_target(self.context, target, TargetKindPolicy::Any)
            .map_err(|_| {
            unica_metadata_designer::resource_plan::MetadataReadFailure::UNAVAILABLE
        })?;
        Ok((resolution.resolved, resolution.handle))
    }
    fn resolve_module(
        &self,
        target: &SourceTarget,
    ) -> Result<
        (unica_address::source_target::ResolvedTarget, Self::Guard),
        unica_metadata_designer::resource_plan::MetadataReadFailure,
    > {
        let resolution =
            resolve_platform_xml_target(self.context, target, TargetKindPolicy::ModuleOnly)
                .map_err(|_| {
                    unica_metadata_designer::resource_plan::MetadataReadFailure::UNAVAILABLE
                })?;
        Ok((resolution.resolved, resolution.handle))
    }
    fn resource_evidence(
        &self,
        guard: &Self::Guard,
    ) -> Result<
        unica_source_designer::owner::PlatformXmlResourceEvidence,
        unica_metadata_designer::resource_plan::MetadataReadFailure,
    > {
        platform_xml_resource_evidence(self.context, guard)
            .map_err(|_| unica_metadata_designer::resource_plan::MetadataReadFailure::UNAVAILABLE)
    }
    fn ordinary_read(&self) -> &dyn unica_source_contract::discovery::ProjectDiscoveryRead {
        &self.ordinary
    }
}

impl unica_metadata_designer::image_plan::MetadataEditRead for MetadataDependencyReadAdapter<'_> {
    fn predefined_source_root(
        &self,
        source_set: &str,
    ) -> Result<std::path::PathBuf, unica_metadata_designer::resource_plan::MetadataReadFailure>
    {
        crate::infrastructure::source_roots::resolve_named_source_set(self.context, source_set)
            .map(|source| source.path)
            .map_err(|_| unica_metadata_designer::resource_plan::MetadataReadFailure::UNAVAILABLE)
    }
    fn predefined_read(&self) -> &dyn unica_metadata_designer::predefined::PredefinedResourceRead {
        &super::predefined::PredefinedResourceReadBinding
    }
}
