#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
type MetaRemoveSubsystemChildInspectionHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static META_REMOVE_FORCED_REPARSE_PATHS:
        std::cell::RefCell<HashSet<PathBuf>> =
        std::cell::RefCell::new(HashSet::new());
    static META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK:
        std::cell::RefCell<Option<MetaRemoveSubsystemChildInspectionHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_meta_remove_forced_reparse_paths<T>(
    paths: impl IntoIterator<Item = PathBuf>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(HashSet<PathBuf>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| {
                slot.replace(std::mem::take(&mut self.0));
            });
        }
    }

    let paths = paths.into_iter().collect();
    let previous = META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| slot.replace(paths));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn force_meta_remove_reparse_path(path: impl Into<PathBuf>) {
    META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| {
        slot.borrow_mut().insert(path.into());
    });
}

#[cfg(test)]
fn with_before_meta_remove_subsystem_child_inspection_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaRemoveSubsystemChildInspectionHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_before_meta_remove_subsystem_child_inspection_hook(path: &Path) {
    if let Some(hook) =
        META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(path);
    }
}

mod edit;
mod format_contract;
mod help_facet;
mod info;
mod info_projection;
mod integrity_check;
mod predefined;
mod publisher;

#[cfg(test)]
pub(crate) fn typed_resource_noop_and_identity_contract_is_complete() {
    edit::tests::typed_exact_noop_form_update_preserves_resource_bytes_and_identities();
}

#[cfg(test)]
pub(crate) fn typed_resource_preimage_contract_is_complete() {
    publisher::typed_add_publication_tests::meta_add_detects_concurrent_owner_change_without_overwriting_it();
}
mod remove;
mod template_catalog;
mod usage_scan;
mod validation;
mod validation_context;
mod xml_model;

pub(crate) use edit::{
    prepare_typed_edit, resolve_typed_edit_object, resolve_typed_metadata_object,
};
pub(crate) use info::read_typed_meta_info;
#[cfg(test)]
pub(crate) use info::{
    with_meta_info_descriptor_image_hook, with_registrar_processing_hook,
    with_subsystem_evidence_processing_hook, RegistrarProcessingPhase,
    SubsystemEvidenceProcessingPhase,
};
pub(crate) use publisher::{fresh_metadata_uuid, prepare_meta_add, prepare_meta_remove};
#[cfg(test)]
pub(crate) use publisher::{
    with_meta_add_after_authorization_hook, with_meta_edit_before_reauthorization_hook,
};
pub(crate) use remove::remove_metadata_child_text_with_flag;
#[cfg(test)]
pub(crate) use remove::with_meta_remove_before_reauthorization_hook;
#[cfg(test)]
pub(crate) use template_catalog::emit_meta_internal_info;
pub(crate) use template_catalog::metadata_generated_types_8_3_27;
pub(crate) use usage_scan::{scan_local_enrichment, LocalEnrichment, LocalSection};
pub(crate) use validation::{
    service_child_semantics, validate_metadata_owner_shape_8_3_27, MetadataValidator,
};
pub(crate) use xml_model::{
    meta_info_child, meta_info_child_text, meta_info_children, meta_info_inner_text,
};

/// The hidden v0.13 tree delegates metadata branches to the current typed
/// metadata reader. This predicate describes only reader ownership; it never
/// resolves a path or treats a physical resource as logical identity.
pub(crate) fn accepts_logical_metadata_address(
    address: &crate::domain::address::QualifiedAddress,
) -> bool {
    address
        .segments()
        .first()
        .is_some_and(|segment| segment.kind().is_metadata_kind())
}

#[cfg(test)]
mod info_projection_tests;
#[cfg(test)]
mod info_tests;
#[cfg(test)]
mod remove_tests;
#[cfg(test)]
mod template_catalog_tests;
#[cfg(test)]
mod usage_scan_tests;
