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
mod info;
mod publisher;
mod remove;
mod template_catalog;
mod validation;
mod validation_context;
mod xml_model;

// The split stays private. This is the exact pre-existing meta::* compatibility
// surface required by native-operation callers outside this family.
pub(crate) use edit::{
    prepare_typed_edit, resolve_meta_edit_object_path, resolve_typed_edit_object,
    resolve_typed_metadata_object,
};
pub(crate) use info::{analyze_meta_info_with_data, read_typed_meta_info};
#[cfg(test)]
pub(crate) use info::{with_registrar_processing_hook, RegistrarProcessingPhase};
pub(crate) use publisher::{fresh_metadata_uuid, prepare_meta_add, prepare_meta_remove};
pub(crate) use remove::{
    meta_remove_reference_xml_dependency_paths, meta_remove_subsystem_dependency_paths,
    meta_remove_type_plural, remove_metadata_child_text_with_flag,
    remove_metadata_object_with_data,
};
pub(crate) use template_catalog::metadata_generated_types_8_3_27;
#[cfg(test)]
pub(crate) use template_catalog::{emit_meta_internal_info, minimal_metadata_xml_for_tests};
pub(crate) use validation::{validate_metadata_owner_shape_8_3_27, MetadataValidator};
pub(crate) use xml_model::{
    meta_info_child, meta_info_child_text, meta_info_children, meta_info_inner_text,
};

#[cfg(test)]
mod info_tests;
#[cfg(test)]
mod remove_tests;
#[cfg(test)]
mod template_catalog_tests;
