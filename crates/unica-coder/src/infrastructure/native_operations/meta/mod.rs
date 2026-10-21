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

mod info;

pub(crate) mod predefined;
mod publisher;

pub(crate) mod remove;
pub(crate) mod template_source;
mod validation;
#[cfg(test)]
mod validation_context;

pub(crate) use edit::{
    prepare_typed_edit, resolve_typed_edit_object, resolve_typed_metadata_object,
};

pub(crate) use info::read_typed_meta_info;
#[cfg(test)]
pub(crate) use info::{
    with_meta_info_descriptor_image_hook, with_registrar_processing_hook,
    with_subsystem_evidence_processing_hook,
};

pub(crate) use publisher::prepare_meta_add;
#[cfg(test)]
pub(crate) use publisher::{
    with_meta_add_after_authorization_hook, with_meta_edit_before_reauthorization_hook,
};

pub(crate) use validation::validate_metadata_owner_shape_8_3_27;

/// The hidden v0.13 tree delegates metadata branches to the current typed
/// metadata reader. This predicate describes only reader ownership; it never
/// resolves a path or treats a physical resource as logical identity.
pub(crate) fn logical_metadata_reader_target(
    address: &unica_address::address::QualifiedAddress,
) -> Option<unica_address::source_target::MetadataAddress> {
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
    address: &unica_address::address::QualifiedAddress,
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
    unica_address::source_target::MetadataAddressPrefix::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
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
