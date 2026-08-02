#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::OBJECT_PATH;
use crate::application::AdapterOutcome;
use crate::domain::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use crate::domain::source_target::ResolvedTarget;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::metadata_kind;
use crate::infrastructure::platform_xml_owner::{
    resolve_platform_xml_owners_with_provenance, root_version_literal, PlatformXmlOwnerKind,
    PlatformXmlOwnerProvenance,
};
use diffy::{apply, DiffOptions, Patch};
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use self::validation_context::{
    inspect_meta_validation_reads, meta_validate_registrar_document_scan,
    meta_validate_types_with_list_presentation, MetaValidationOwnerKind,
};
use super::common::*;
use super::compile_transaction::{
    CompileTransaction, DirectoryTopologyEntry, DirectoryTopologyEntryKind, RegistrationStatus,
};
use super::{
    cf::*, cfe::*, dcs::*, form::*, interface::*, mxl::*, role::*, subsystem::*, template::*,
};

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

pub(crate) mod edit;
pub(crate) mod info;
pub(crate) mod legacy_dsl;
pub(crate) mod publisher;
pub(crate) mod remove;
pub(crate) mod template_catalog;
pub(crate) mod validation;
pub(crate) mod validation_context;
pub(crate) mod xml_model;

pub(crate) use edit::*;
pub(crate) use info::*;
pub(crate) use legacy_dsl::*;
pub(crate) use publisher::*;
pub(crate) use remove::*;
pub(crate) use template_catalog::*;
pub(crate) use validation::*;
pub(crate) use xml_model::*;

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
mod tests;
