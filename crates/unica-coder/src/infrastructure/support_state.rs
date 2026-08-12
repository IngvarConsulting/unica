use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_target::{ResolvedTarget, SourceTarget, TargetKind};
use crate::domain::support_state::{
    ConfigurationSupportData, ConfigurationSupportState, ObjectSupportData, ObjectSupportState,
    ResolvedSubsystemTarget, SupportCounts, SupportReadError, SupportReadErrorCode,
    SupportStateReader,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::{
    read_support_state_strict, support_root_uuid_from_bytes, SupportState,
};
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target_in, TargetKindPolicy,
};
use crate::infrastructure::source_roots::{resolve_named_source_set, ResolvedNamedSourceSet};
use crate::infrastructure::subsystem_topology::{
    capture_registered_subsystem_topology, SubsystemTopologyNode,
};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) trait SupportStateReaderFactory: Send + Sync {
    fn create<'a>(&'a self, context: &'a WorkspaceContext) -> Box<dyn SupportStateReader + 'a>;
}

pub(crate) struct WorkspaceSupportStateReaderFactory;

impl SupportStateReaderFactory for WorkspaceSupportStateReaderFactory {
    fn create<'a>(&'a self, context: &'a WorkspaceContext) -> Box<dyn SupportStateReader + 'a> {
        Box::new(WorkspaceSupportStateReader::new(context))
    }
}

pub(crate) struct WorkspaceSupportStateReader<'a> {
    context: &'a WorkspaceContext,
}

impl<'a> WorkspaceSupportStateReader<'a> {
    pub(crate) const fn new(context: &'a WorkspaceContext) -> Self {
        Self { context }
    }

    fn selected_source_set(
        &self,
        target: &ResolvedTarget,
    ) -> Result<ResolvedNamedSourceSet, SupportReadError> {
        self.selected_source_set_by_name(&target.source_set)
    }

    fn selected_source_set_by_name(
        &self,
        source_set: &str,
    ) -> Result<ResolvedNamedSourceSet, SupportReadError> {
        let selected = resolve_named_source_set(self.context, source_set).map_err(|_| {
            SupportReadError::new(
                SupportReadErrorCode::EvidenceUnavailable,
                "support-state source set evidence is unavailable",
            )
        })?;
        if selected.source_set.source_format != SourceFormat::PlatformXml {
            return Err(SupportReadError::new(
                SupportReadErrorCode::ProviderUnavailable,
                "the selected source format has no support-state reader",
            ));
        }
        Ok(selected)
    }

    fn resolve_platform_target(
        &self,
        target: &ResolvedTarget,
        selected: ResolvedNamedSourceSet,
    ) -> Result<
        crate::infrastructure::platform_xml_source_targets::PlatformXmlResolution,
        SupportReadError,
    > {
        let source_target = SourceTarget {
            source_set: target.source_set.clone(),
            metadata_path: target.metadata_path.clone(),
        };
        let resolution = resolve_platform_xml_target_in(
            self.context,
            &source_target,
            TargetKindPolicy::Any,
            selected,
        )
        .map_err(|_| {
            SupportReadError::new(
                SupportReadErrorCode::EvidenceUnavailable,
                "logical support-state target evidence is unavailable",
            )
        })?;
        if resolution.resolved != *target {
            return Err(SupportReadError::new(
                SupportReadErrorCode::EvidenceUnavailable,
                "logical support-state target did not rebind to the same identity",
            ));
        }
        Ok(resolution)
    }

    fn support_state(
        &self,
        evidence: &crate::infrastructure::platform_xml_source_targets::PlatformXmlResourceEvidence,
    ) -> Result<Option<SupportState>, SupportReadError> {
        self.support_state_at(&evidence.source_root)
    }

    fn support_state_at(
        &self,
        source_root: &Path,
    ) -> Result<Option<SupportState>, SupportReadError> {
        read_support_state_strict(&source_root.join("Ext").join("ParentConfigurations.bin"))
    }
}

fn subsystem_node_is_registered(
    nodes: &[SubsystemTopologyNode],
    target: &ResolvedSubsystemTarget,
) -> bool {
    nodes.iter().any(|node| {
        node.address == target.address || subsystem_node_is_registered(&node.children, target)
    })
}

fn subsystem_descriptor_path(target: &ResolvedSubsystemTarget) -> PathBuf {
    let mut names = target.address.as_str().split('.');
    let first = names
        .next()
        .expect("a resolved subsystem address has at least one name");
    let mut path = PathBuf::from("Subsystems").join(first);
    for name in names {
        path = path.join("Subsystems").join(name);
    }
    path.set_extension("xml");
    path
}

impl SupportStateReader for WorkspaceSupportStateReader<'_> {
    fn configuration_support(
        &self,
        target: &ResolvedTarget,
    ) -> Result<ConfigurationSupportData, SupportReadError> {
        if target.target_kind != TargetKind::SourceRoot {
            return Err(SupportReadError::new(
                SupportReadErrorCode::TargetUnsupported,
                "configuration support requires a source-root target",
            ));
        }
        let selected = self.selected_source_set(target)?;
        let source_set_kind = selected.source_set.kind;
        let resolution = self.resolve_platform_target(target, selected)?;
        let evidence =
            platform_xml_resource_evidence(self.context, &resolution.handle).map_err(|_| {
                SupportReadError::new(
                    SupportReadErrorCode::EvidenceUnavailable,
                    "configuration support-state evidence is unavailable",
                )
            })?;
        let Some(state) = self.support_state(&evidence)? else {
            return Ok(ConfigurationSupportData {
                state: if source_set_kind == SourceSetKind::Extension {
                    ConfigurationSupportState::Extension
                } else {
                    ConfigurationSupportState::NotSupported
                },
                editing_enabled: None,
                objects: None,
            });
        };
        if state.removed() {
            return Ok(ConfigurationSupportData {
                state: ConfigurationSupportState::Removed,
                editing_enabled: None,
                objects: None,
            });
        }
        let counts = state.counts();
        Ok(ConfigurationSupportData {
            state: ConfigurationSupportState::Supported,
            editing_enabled: Some(state.global_editing_enabled()),
            objects: Some(SupportCounts {
                locked: counts[0] as u64,
                editable: counts[1] as u64,
                removed: counts[2] as u64,
            }),
        })
    }

    fn object_support(
        &self,
        target: &ResolvedTarget,
    ) -> Result<ObjectSupportData, SupportReadError> {
        if target.target_kind != TargetKind::MetadataObject {
            return Err(SupportReadError::new(
                SupportReadErrorCode::TargetUnsupported,
                "object support requires a metadata-object target",
            ));
        }
        let selected = self.selected_source_set(target)?;
        let resolution = self.resolve_platform_target(target, selected)?;
        let evidence =
            platform_xml_resource_evidence(self.context, &resolution.handle).map_err(|_| {
                SupportReadError::new(
                    SupportReadErrorCode::EvidenceUnavailable,
                    "object support-state evidence is unavailable",
                )
            })?;
        let data = |state, direct_edit_safe| ObjectSupportData {
            state,
            direct_edit_safe,
        };
        let Some(state) = self.support_state(&evidence)? else {
            return Ok(data(ObjectSupportState::NotSupported, None));
        };
        if state.removed() {
            return Ok(data(ObjectSupportState::RemovedFromSupport, Some(true)));
        }
        if !state.global_editing_enabled() {
            return Ok(data(ObjectSupportState::ConfigurationReadOnly, Some(false)));
        }
        let descriptor = fs::read(&evidence.target_path).map_err(|_| {
            SupportReadError::new(
                SupportReadErrorCode::EvidenceUnavailable,
                "metadata descriptor evidence is unreadable",
            )
        })?;
        let object_uuid = support_root_uuid_from_bytes(&descriptor).ok_or_else(|| {
            SupportReadError::new(
                SupportReadErrorCode::EvidenceUnavailable,
                "metadata descriptor has no support-state UUID evidence",
            )
        })?;
        Ok(match state.object_rule(&object_uuid) {
            Some(0) => data(ObjectSupportState::Locked, Some(false)),
            Some(1) => data(ObjectSupportState::EditableWithSupport, Some(true)),
            Some(2) => data(ObjectSupportState::RemovedFromSupport, Some(true)),
            _ => data(ObjectSupportState::NotSupported, None),
        })
    }

    fn subsystem_support(
        &self,
        target: &ResolvedSubsystemTarget,
    ) -> Result<ObjectSupportData, SupportReadError> {
        let selected = self.selected_source_set_by_name(&target.source_set)?;
        let topology =
            capture_registered_subsystem_topology(&selected.path, || Ok(())).map_err(|_| {
                SupportReadError::new(
                    SupportReadErrorCode::EvidenceUnavailable,
                    "registered subsystem support-state evidence is unavailable",
                )
            })?;
        if !subsystem_node_is_registered(&topology.roots, target) {
            return Err(SupportReadError::new(
                SupportReadErrorCode::EvidenceUnavailable,
                "the subsystem support-state target is not registered",
            ));
        }
        let descriptor_path = subsystem_descriptor_path(target);
        let descriptor = topology
            .dependency_documents()
            .iter()
            .find(|document| document.path == descriptor_path)
            .ok_or_else(|| {
                SupportReadError::new(
                    SupportReadErrorCode::EvidenceUnavailable,
                    "registered subsystem descriptor evidence is unavailable",
                )
            })?;
        let data = |state, direct_edit_safe| ObjectSupportData {
            state,
            direct_edit_safe,
        };
        let Some(state) = self.support_state_at(&selected.path)? else {
            return Ok(data(ObjectSupportState::NotSupported, None));
        };
        if state.removed() {
            return Ok(data(ObjectSupportState::RemovedFromSupport, Some(true)));
        }
        if !state.global_editing_enabled() {
            return Ok(data(ObjectSupportState::ConfigurationReadOnly, Some(false)));
        }
        let object_uuid = support_root_uuid_from_bytes(&descriptor.bytes).ok_or_else(|| {
            SupportReadError::new(
                SupportReadErrorCode::EvidenceUnavailable,
                "registered subsystem descriptor has no support-state UUID evidence",
            )
        })?;
        Ok(match state.object_rule(&object_uuid) {
            Some(0) => data(ObjectSupportState::Locked, Some(false)),
            Some(1) => data(ObjectSupportState::EditableWithSupport, Some(true)),
            Some(2) => data(ObjectSupportState::RemovedFromSupport, Some(true)),
            _ => data(ObjectSupportState::NotSupported, None),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source_target::{
        MetadataAddress, ResolvedTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::support_state::{
        ConfigurationSupportState, ObjectSupportState, SupportReadErrorCode, SupportStateReader,
    };
    use crate::domain::workspace::WorkspaceContext;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);
    const ROLE_UUID: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

    fn temp_root(name: &str) -> PathBuf {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-support-state-{name}-{}-{nanos}-{nonce}",
            std::process::id()
        ))
    }

    fn fixture(name: &str, format: &str, source_set_type: &str) -> WorkspaceContext {
        let root = temp_root(name);
        let source = root.join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            format!(
                "format: {format}\nsource-set:\n  - name: main\n    type: {source_set_type}\n    path: src\n"
            ),
        )
        .unwrap();
        if format == "EDT" {
            fs::create_dir_all(source.join("Configuration")).unwrap();
            fs::write(source.join(".project"), "<projectDescription/>").unwrap();
            fs::write(
                source.join("Configuration/Configuration.mdo"),
                "<mdclass:Configuration/>",
            )
            .unwrap();
        } else {
            fs::create_dir_all(source.join("Roles")).unwrap();
            fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(
                source.join("Roles/Reader.xml"),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="{ROLE_UUID}"><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn root_target() -> ResolvedTarget {
        ResolvedTarget {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: TargetKind::SourceRoot,
        }
    }

    fn object_target() -> ResolvedTarget {
        ResolvedTarget {
            source_set: "main".to_string(),
            metadata_path: Some(
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Role.Reader").unwrap(),
            ),
            target_kind: TargetKind::MetadataObject,
        }
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn support_state_reader_rejects_edt_instead_of_claiming_not_supported() {
        let context = fixture("edt", "EDT", "CONFIGURATION");
        let reader = WorkspaceSupportStateReader::new(&context);

        for error in [
            reader
                .configuration_support(&root_target())
                .expect_err("EDT cannot answer configuration support in this slice"),
            reader
                .object_support(&object_target())
                .expect_err("EDT cannot answer object support in this slice"),
        ] {
            assert_eq!(error.code, SupportReadErrorCode::ProviderUnavailable);
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_configuration_support_distinguishes_absent_unreadable_and_invalid_marker() {
        let context = fixture("marker-matrix", "DESIGNER", "CONFIGURATION");
        let reader = WorkspaceSupportStateReader::new(&context);
        let marker = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");

        let absent = reader.configuration_support(&root_target()).unwrap();
        assert_eq!(absent.state, ConfigurationSupportState::NotSupported);

        fs::create_dir_all(&marker).unwrap();
        let unreadable = reader
            .configuration_support(&root_target())
            .expect_err("a directory is not a readable support marker");
        assert_eq!(unreadable.code, SupportReadErrorCode::StateUnreadable);

        fs::remove_dir_all(&marker).unwrap();
        fs::write(&marker, b"{9,0,1}, invalid support header").unwrap();
        let invalid = reader
            .configuration_support(&root_target())
            .expect_err("an unknown non-empty marker is not removed support");
        assert_eq!(invalid.code, SupportReadErrorCode::StateInvalid);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_object_support_uses_the_resolved_descriptor_uuid() {
        let context = fixture("object-rule", "DESIGNER", "CONFIGURATION");
        let marker = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(
            &marker,
            format!("{{6,0,1,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,0,0,0,{ROLE_UUID}}}"),
        )
        .unwrap();

        let support = WorkspaceSupportStateReader::new(&context)
            .object_support(&object_target())
            .unwrap();

        assert_eq!(support.state, ObjectSupportState::Locked);
        assert_eq!(support.direct_edit_safe, Some(false));
        cleanup(&context);
    }

    #[test]
    fn support_reader_rejects_the_wrong_target_kind() {
        let context = fixture("wrong-kind", "DESIGNER", "CONFIGURATION");
        let reader = WorkspaceSupportStateReader::new(&context);

        assert_eq!(
            reader
                .configuration_support(&object_target())
                .expect_err("configuration support requires a root")
                .code,
            SupportReadErrorCode::TargetUnsupported
        );
        assert_eq!(
            reader
                .object_support(&root_target())
                .expect_err("object support requires an object")
                .code,
            SupportReadErrorCode::TargetUnsupported
        );
        cleanup(&context);
    }
}
