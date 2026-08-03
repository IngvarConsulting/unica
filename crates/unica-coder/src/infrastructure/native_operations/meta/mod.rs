#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};

#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
type MetaCompileAfterOwnerValidationHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
type MetaCompileAfterFormatPlanHook = Box<dyn FnOnce()>;

#[cfg(test)]
type MetaEditAfterLineNumberLengthPolicyHook = Box<dyn FnOnce()>;

#[cfg(test)]
type MetaRemoveSubsystemChildInspectionHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static META_COMPILE_AFTER_OWNER_VALIDATION_HOOK:
        std::cell::RefCell<Option<MetaCompileAfterOwnerValidationHook>> =
        const { std::cell::RefCell::new(None) };
    static META_COMPILE_AFTER_FORMAT_PLAN_HOOK:
        std::cell::RefCell<Option<MetaCompileAfterFormatPlanHook>> =
        const { std::cell::RefCell::new(None) };
    static META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK:
        std::cell::RefCell<Option<MetaEditAfterLineNumberLengthPolicyHook>> =
        const { std::cell::RefCell::new(None) };
    static META_REMOVE_FORCED_REPARSE_PATHS:
        std::cell::RefCell<HashSet<PathBuf>> =
        std::cell::RefCell::new(HashSet::new());
    static META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK:
        std::cell::RefCell<Option<MetaRemoveSubsystemChildInspectionHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_meta_compile_after_owner_validation_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaCompileAfterOwnerValidationHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_COMPILE_AFTER_OWNER_VALIDATION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        META_COMPILE_AFTER_OWNER_VALIDATION_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_meta_compile_after_owner_validation_hook(path: &Path) {
    if let Some(hook) =
        META_COMPILE_AFTER_OWNER_VALIDATION_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(path);
    }
}

#[cfg(test)]
fn with_meta_compile_after_format_plan_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaCompileAfterFormatPlanHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_COMPILE_AFTER_FORMAT_PLAN_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        META_COMPILE_AFTER_FORMAT_PLAN_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_meta_compile_after_format_plan_hook() {
    if let Some(hook) = META_COMPILE_AFTER_FORMAT_PLAN_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn with_meta_edit_after_line_number_length_policy_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaEditAfterLineNumberLengthPolicyHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_meta_edit_after_line_number_length_policy_hook() {
    if let Some(hook) =
        META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
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
mod legacy_dsl;
mod publisher;
mod remove;
mod template_catalog;
mod validation;
mod validation_context;
mod xml_model;

// The split stays private. This is the exact pre-existing meta::* compatibility
// surface required by native-operation callers outside this family.
pub(crate) use edit::{
    edit_meta_with_data, prepare_typed_edit, preview_meta_edit, preview_meta_edit_with_data,
    resolve_meta_edit_object_path, resolve_typed_edit_object,
};
pub(crate) use info::{analyze_meta_info_with_data, resolve_meta_info_path};
pub(crate) use legacy_dsl::{
    meta_compile_format_dependency_paths, meta_compile_object_xml, preview_meta_compile,
    validate_metadata_owner_shape_8_3_27,
};
pub(crate) use publisher::{fresh_meta_compile_uuid, prepare_meta_add};
pub(crate) use remove::{
    meta_remove_reference_xml_dependency_paths, meta_remove_subsystem_dependency_paths,
    meta_remove_type_plural, remove_metadata_child_text, remove_metadata_child_text_lxml,
    remove_metadata_child_text_with_flag, remove_metadata_object_with_data,
};
pub(crate) use template_catalog::{emit_meta_internal_info, metadata_generated_types_8_3_27};
pub(crate) use validation::{
    meta_validate_format_dependency_paths, validate_meta, MetadataValidator,
};
pub(crate) use xml_model::{
    meta_info_child, meta_info_child_text, meta_info_children, meta_info_inner_text,
};

use edit::edit_meta;
use info::analyze_meta_info;
use legacy_dsl::compile_meta;
use remove::remove_metadata_object;

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "meta-info" => Some(Ok(analyze_meta_info(args, context))),
        "meta-validate" => Some(Ok(validate_meta(args, context))),
        _ => None,
    }
}

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "meta-compile" => Some(compile_meta(args, context)),
        // Typed answer; data reaches the envelope through typed_result.rs.
        "meta-edit" => Some(edit_meta(args, context)),
        "meta-remove" => Some(remove_metadata_object(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod compile_tests;
#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod info_tests;
#[cfg(test)]
mod remove_tests;
#[cfg(test)]
mod template_catalog_tests;
