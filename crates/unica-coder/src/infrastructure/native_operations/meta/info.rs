//! Trusted read/session and existing test-observation binding for typed metadata info.
use super::template_source::MetadataDependencyReadAdapter;
use crate::infrastructure::platform_xml_source_targets::ClosedPlatformXmlTarget;
use std::path::Path;
use unica_address::source_target::MetadataAddress;
#[cfg(test)]
use unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20;
use unica_metadata::support_state::SupportStateReader;
#[cfg(test)]
use unica_metadata::MetaElementData;
use unica_metadata_designer::info_read::{
    RegistrarProcessingPhase, SubsystemEvidenceProcessingPhase,
};
use unica_metadata_designer::resource_plan::ResolvedMetadataObject;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

#[cfg(test)]
type MetaInfoDescriptorImageHook = Box<dyn FnOnce(&[u8]) -> Vec<u8>>;

#[cfg(test)]
thread_local! {
    static META_INFO_DESCRIPTOR_IMAGE_HOOK:
        std::cell::RefCell<Option<MetaInfoDescriptorImageHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_meta_info_descriptor_image_hook<T>(
    hook: impl FnOnce(&[u8]) -> Vec<u8> + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaInfoDescriptorImageHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_INFO_DESCRIPTOR_IMAGE_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous = META_INFO_DESCRIPTOR_IMAGE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn meta_info_descriptor_image_for_test(bytes: &[u8]) -> Option<Vec<u8>> {
    META_INFO_DESCRIPTOR_IMAGE_HOOK.with(|slot| slot.borrow_mut().take().map(|hook| hook(bytes)))
}

#[cfg(test)]
type RegistrarProcessingHook = Box<dyn FnMut(&RegistrarProcessingPhase)>;

#[cfg(test)]
thread_local! {
    static REGISTRAR_PROCESSING_HOOK:
        std::cell::RefCell<Option<RegistrarProcessingHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_registrar_processing_hook<T>(
    hook: impl FnMut(&RegistrarProcessingPhase) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RegistrarProcessingHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            REGISTRAR_PROCESSING_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous = REGISTRAR_PROCESSING_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

pub(super) fn emit_registrar_processing_phase(phase: RegistrarProcessingPhase) {
    #[cfg(test)]
    REGISTRAR_PROCESSING_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(&phase);
        }
    });
    #[cfg(not(test))]
    let _ = phase;
}

#[cfg(test)]
type SubsystemEvidenceProcessingHook = Box<dyn FnMut(&SubsystemEvidenceProcessingPhase)>;

#[cfg(test)]
thread_local! {
    static SUBSYSTEM_EVIDENCE_PROCESSING_HOOK:
        std::cell::RefCell<Option<SubsystemEvidenceProcessingHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_subsystem_evidence_processing_hook<T>(
    hook: impl FnMut(&SubsystemEvidenceProcessingPhase) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<SubsystemEvidenceProcessingHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            SUBSYSTEM_EVIDENCE_PROCESSING_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous =
        SUBSYSTEM_EVIDENCE_PROCESSING_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

pub(super) fn emit_subsystem_evidence_processing_phase(phase: SubsystemEvidenceProcessingPhase) {
    #[cfg(test)]
    SUBSYSTEM_EVIDENCE_PROCESSING_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(&phase);
        }
    });
    #[cfg(not(test))]
    let _ = phase;
}

#[cfg(test)]
pub(super) fn typed_elements(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
    nested_attributes: bool,
) -> Vec<MetaElementData> {
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Test").unwrap();
    unica_metadata_designer::info::typed_elements_with_diagnostics(
        xml,
        parent,
        tag,
        nested_attributes,
        "collections.test",
        &target,
        &mut Vec::new(),
    )
}

pub(crate) fn read_typed_meta_info(
    resolved: &ResolvedMetadataObject<ClosedPlatformXmlTarget>,
    source_set: &str,
    target: &MetadataAddress,
    context: &WorkspaceContext,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    support_reader: &dyn SupportStateReader,
) -> Result<
    (
        unica_metadata::validation_subject::MetaLocalInfo,
        unica_metadata::validation_subject::MetadataValidationSubject,
    ),
    unica_metadata::request::MetaFailure,
> {
    #[cfg(test)]
    let test_descriptor_image = meta_info_descriptor_image_for_test(&resolved.descriptor_preimage);
    #[cfg(test)]
    let descriptor_preimage = test_descriptor_image
        .as_deref()
        .unwrap_or(&resolved.descriptor_preimage);
    #[cfg(not(test))]
    let descriptor_preimage = resolved.descriptor_preimage.as_slice();
    unica_metadata_designer::info_read::read_typed_meta_info(
        &MetadataDependencyReadAdapter::new(context),
        resolved,
        descriptor_preimage,
        unica_metadata_designer::info_read::MetadataInfoSourceSet {
            name: source_set,
            kind: resolved.handle.source_set_kind(),
        },
        target,
        unica_metadata_designer::info_read::MetadataInfoControl {
            deadline,
            cancellation,
        },
        support_reader,
    )
}
impl unica_metadata_designer::info_read::MetadataInfoRead for MetadataDependencyReadAdapter<'_> {
    fn visit_regular_files(
        &self,
        root: &Path,
        start: &Path,
        limits: unica_source_contract::read::SecureTreeCaptureLimits,
        descend: fn(&Path) -> bool,
        select: fn(&Path) -> bool,
        visit: &mut dyn FnMut(
            unica_source_contract::read::SecureFileSnapshotEntry,
        ) -> std::io::Result<()>,
        checkpoint: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<bool> {
        unica_workspace::read::visit_root_relative_regular_files(
            root, start, limits, descend, select, visit, checkpoint,
        )
        .map(|visited| visited.start_missing)
    }
    fn open_retained_read(
        &self,
        root: &Path,
        limits: unica_source_contract::read::SecureTreeCaptureLimits,
        checkpoint: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<Box<dyn unica_source_contract::read::RetainedSourceRead>> {
        unica_workspace::read::RetainedRootSecureRead::open(root, limits, checkpoint)
            .map(|read| Box::new(read) as Box<dyn unica_source_contract::read::RetainedSourceRead>)
    }
    fn registrar_processing_phase(&self, phase: RegistrarProcessingPhase) {
        emit_registrar_processing_phase(phase)
    }
    fn subsystem_processing_phase(&self, phase: SubsystemEvidenceProcessingPhase) {
        emit_subsystem_evidence_processing_phase(phase)
    }
}
#[cfg(test)]
pub(super) fn typed_registrar_document_images(
    source_root: &Path,
    kind: unica_metadata::MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    target: &MetadataAddress,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> (
    Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    unica_metadata::validation_subject::MetadataEvidenceAvailability,
) {
    let context = WorkspaceContext {
        cwd: source_root.to_path_buf(),
        workspace_root: source_root.to_path_buf(),
        cache_root: source_root.join(".unica"),
        workspace_epoch: 0,
    };
    unica_metadata_designer::info_read::typed_registrar_document_images(
        &MetadataDependencyReadAdapter::new(&context),
        source_root,
        kind,
        properties,
        target,
        deadline,
        cancellation,
    )
}
