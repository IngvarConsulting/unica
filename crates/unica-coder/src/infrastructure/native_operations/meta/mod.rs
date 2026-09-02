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
pub(crate) mod remove;
pub(crate) mod template_catalog;
mod usage_scan;
mod validation;
mod validation_context;
mod xml_model;

pub(crate) use edit::{
    meta_edit_object_identity, prepare_typed_edit, resolve_typed_edit_object,
    resolve_typed_metadata_object,
};
pub(crate) fn apply_typed_operations_to_image_with_seed(
    xml_text: &mut String,
    operations: &[crate::domain::metadata::MetaEditOperation],
    seed: &[u8],
) -> Result<(), crate::application::metadata::MetaFailure> {
    use sha2::{Digest, Sha256};

    let mut counter = 0_u64;
    let mut next_uuid = || {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-metadata-uuid-v1\0");
        hasher.update(seed);
        hasher.update(counter.to_be_bytes());
        counter = counter.saturating_add(1);
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        uuid::Uuid::from_bytes(bytes).to_string()
    };
    edit::apply_typed_operations_with_uuid(xml_text, operations, &mut next_uuid).map(|_| ())
}
pub(crate) use info::{parse_typed_meta_local_info, read_typed_meta_info};
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
pub(crate) use validation::parse_child_profile_from_bytes;
pub(crate) use validation::{
    service_child_semantics, validate_metadata_owner_shape_8_3_27, MetadataValidator,
};
pub(crate) use xml_model::{
    meta_info_child, meta_info_child_text, meta_info_children, meta_info_inner_text,
};

/// The hidden v0.13 tree delegates metadata branches to the current typed
/// metadata reader. This predicate describes only reader ownership; it never
/// resolves a path or treats a physical resource as logical identity.
pub(crate) fn logical_metadata_reader_target(
    address: &crate::domain::address::QualifiedAddress,
) -> Option<crate::domain::source_target::MetadataAddress> {
    let owner = address.segments().first()?;
    if !owner.kind().is_metadata_kind() || owner.name().is_none() {
        return None;
    }
    crate::infrastructure::native_operations::logical_selector::typed_reader_metadata_target(
        address,
        &[owner.kind().as_str()],
    )
}

pub(crate) fn accepts_logical_metadata_address(
    address: &crate::domain::address::QualifiedAddress,
) -> bool {
    let Some(owner) = address.segments().first() else {
        return false;
    };
    if !owner.kind().is_metadata_kind() {
        return false;
    }
    if owner.name().is_some() {
        return logical_metadata_reader_target(address).is_some();
    }
    crate::domain::source_target::MetadataAddressPrefix::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        owner.kind().as_str(),
    )
    .is_ok()
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

/// One staged file change of the embedded help facet: `(path, preimage,
/// postimage)` with absolute paths under the source root.
pub(crate) type HelpFacetFileChange = (std::path::PathBuf, Option<Vec<u8>>, Option<Vec<u8>>);

/// Plans the create-only embedded help facet for one owner without a
/// transaction: `Ext/Help.xml`, `Ext/Help/<lang>.html` and the owner's forms
/// gaining `IncludeHelpInContents`.
pub(crate) fn plan_help_facet_files(
    descriptor_path: &std::path::Path,
    owner: &crate::domain::source_target::MetadataAddress,
    object_name: &str,
    lang: &str,
) -> Result<Vec<HelpFacetFileChange>, crate::application::metadata::MetaFailure> {
    let operations = [crate::domain::metadata::MetaEditOperation::AddHelp {
        lang: lang.to_string(),
    }];
    let plan = help_facet::plan_help_resource_after_descriptor_edit(
        descriptor_path,
        owner,
        object_name,
        &operations,
    )?;
    Ok(plan
        .file_mutations
        .into_iter()
        .map(|mutation| (mutation.path, mutation.pre_image, mutation.post_image))
        .collect())
}
