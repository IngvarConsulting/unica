#![allow(dead_code, unused_imports)]
use unica_metadata::extension::*;
use unica_metadata_designer::extension::*;
use unica_source_designer::configuration_layout::*;

use unica_metadata_designer::property_states::{
    cfe_patch_direct_md_child, cfe_patch_exact_md_child, cfe_patch_mark_extended_property,
    cfe_patch_precondition_error,
};
use unica_source_designer::module_layout::{
    platform_xml_module_identity, platform_xml_module_relative,
};
use unica_xml_format::text::{
    cfe_borrow_ensure_namespace_declaration, cfe_borrow_find_start_tag, cfe_borrow_is_xml_space,
    cfe_borrow_start_tag_at, cfe_borrow_tag_local_name, cfe_borrow_xml_attribute_value_range,
};

use crate::infrastructure::form_composition::*;
use crate::infrastructure::interface_composition::*;
use crate::infrastructure::subsystem_composition::*;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_forms::facts::*;
use unica_forms_designer::codec::*;
use unica_metadata::interface::*;
use unica_metadata::subsystem::*;
use unica_metadata_designer::interface::*;
use unica_metadata_designer::subsystem::*;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use unica_source_designer::roots::root_version_literal;

use super::common::*;
use super::{cf::*, dcs::*, meta::*, role::*};
use unica_metadata_designer::validation::service_child_semantics;
use unica_workspace::compile_transaction::{CommitReport, CompileTransaction};

/// Enumerate the registered CFE source graph: Configuration.xml, registered
/// languages and object descriptors, plus registered form wrappers and
/// Form.xml payloads.
///
/// This graph is intentionally the shared format-compatibility boundary for
/// validate, diff, and borrow. A particular read-only diff mode may not open
/// every related node, but registered nodes still belong to the selected
/// extension source graph; unregistered neighboring XML does not.
pub(crate) fn cfe_registered_xml_dependency_paths(
    config_path: &Path,
) -> Result<Vec<PathBuf>, String> {
    cfe_registered_xml_dependency_paths_with_reader(config_path, &mut |path| {
        let raw = match fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("failed to read {}: {error}", path.display()));
            }
        };
        let text = std::str::from_utf8(&raw)
            .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
        Ok(Some(text.trim_start_matches('\u{feff}').to_string()))
    })
}

#[derive(Debug)]
struct PreparedCfeBorrow {
    cfg_path: PathBuf,
    ext_path: PathBuf,
    cfg_dir: PathBuf,
    ext_dir: PathBuf,
    write_plan: CfeBorrowWritePlan,
    name_prefix: String,
    log: CfeBorrowLog,
    borrowed: Vec<CfeBorrowedItem>,
    artifacts: Vec<PathBuf>,
    registered_format_dependencies: Vec<PathBuf>,
}

impl PreparedCfeBorrow {
    fn format_dependency_paths(&self) -> Vec<PathBuf> {
        let mut paths = self.write_plan.format_dependency_paths();
        paths.extend(self.registered_format_dependencies.iter().cloned());
        paths.sort();
        paths.dedup();
        paths
    }
}

fn prepare_cfe_borrow(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PreparedCfeBorrow, String> {
    prepare_cfe_borrow_with_trace(args, context, CfeBorrowReadTrace::default())
}

fn prepare_cfe_borrow_with_trace(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    read_trace: CfeBorrowReadTrace,
) -> Result<PreparedCfeBorrow, String> {
    let ext_path = cfe_borrow_resolve_path(
        args,
        context,
        &["extensionPath", "ExtensionPath"],
        "extension",
    )?;
    let cfg_path = cfe_borrow_resolve_path(args, context, &["configPath", "ConfigPath"], "config")?;
    cfe_borrow_validate_extension(&ext_path, context)?;
    let object_spec =
        unica_operation_contract::input::required_string(args, &["object", "Object"], "Object")?;
    let borrow_main_attribute = cfe_borrow_main_attribute_mode(args)?;

    let ext_dir = ext_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| context.cwd.clone());
    let cfg_dir = cfg_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| context.cwd.clone());
    let mut write_plan = CfeBorrowWritePlan::with_read_trace(read_trace);
    write_plan.read_dependency_utf8_sig(&cfg_path)?;
    let ext_text = write_plan.read_utf8_sig(&ext_path)?;
    let mut registered_format_dependencies =
        cfe_registered_xml_dependency_paths_with_reader(&ext_path, &mut |path| {
            write_plan.read_current_or_dependency_utf8_sig(path)
        })?;
    let CfeBorrowImagePlan {
        name_prefix,
        log,
        borrowed,
        artifacts,
    } = cfe_borrow_plan_image(
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.cwd),
        &mut write_plan,
        CfeBorrowImageInput {
            ext_text,
            cfg_dir: &cfg_dir,
            ext_dir: &ext_dir,
            ext_path: &ext_path,
            object_spec,
            borrow_main_attribute,
        },
        crate::infrastructure::source_ids::fresh_uuid,
    )?;
    registered_format_dependencies.extend(cfe_registered_xml_dependency_paths_with_reader(
        &ext_path,
        &mut |path| write_plan.read_current_or_dependency_utf8_sig(path),
    )?);
    registered_format_dependencies.sort();
    registered_format_dependencies.dedup();
    Ok(PreparedCfeBorrow {
        cfg_path,
        ext_path,
        cfg_dir,
        ext_dir,
        write_plan,
        name_prefix,
        log,
        borrowed,
        artifacts,
        registered_format_dependencies,
    })
}

pub(crate) fn cfe_borrow_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    Ok(prepare_cfe_borrow(args, context)?.format_dependency_paths())
}

pub(crate) struct CfeBorrowFormatDependencyInspection {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) planning_error: Option<String>,
}

pub(crate) fn cfe_borrow_format_dependency_inspection(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CfeBorrowFormatDependencyInspection {
    let read_trace = CfeBorrowReadTrace::default();
    match prepare_cfe_borrow_with_trace(args, context, read_trace.clone()) {
        Ok(prepared) => CfeBorrowFormatDependencyInspection {
            paths: prepared.format_dependency_paths(),
            planning_error: None,
        },
        Err(error) => CfeBorrowFormatDependencyInspection {
            paths: read_trace.xml_paths(),
            planning_error: Some(error),
        },
    }
}

pub(crate) struct CfeBorrowExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CfeBorrowData>,
}

/// What one committed borrow produced. `changes` states the effect the
/// transaction reported, so it never disagrees with `data.mutation`.
struct CommittedCfeBorrow {
    data: CfeBorrowData,
    artifacts: Vec<PathBuf>,
    changes: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) fn borrow_cfe(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    borrow_cfe_with_data(args, context).outcome
}

pub(crate) fn borrow_cfe_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CfeBorrowExecution {
    let result = (|| -> Result<CommittedCfeBorrow, String> {
        let PreparedCfeBorrow {
            cfg_path,
            ext_path,
            cfg_dir,
            ext_dir,
            write_plan,
            name_prefix,
            log,
            borrowed,
            mut artifacts,
            registered_format_dependencies,
        } = prepare_cfe_borrow(args, context)?;
        let mut format_owner_targets = vec![cfg_path.as_path(), ext_path.as_path()];
        format_owner_targets.extend(registered_format_dependencies.iter().map(PathBuf::as_path));
        format_owner_targets.sort();
        format_owner_targets.dedup();
        // The commit knows which paths it brought into existence and which it
        // replaced; the plan's own artifact list knows neither, so the typed
        // report is built from the commit and not from the plan.
        let CommitReport {
            mut created,
            mut updated,
            cleanup_warnings,
            retained_apply_cleanup_diagnostics: _,
        } = write_plan.commit_with_post_validation(&format_owner_targets, context, || {
            cfe_borrow_validate_extension(&ext_path, context)
        })?;
        created.sort();
        created.dedup();
        updated.sort();
        updated.dedup();
        let mut mutation = unica_execution_contract::mutation::MutationData::new(true);
        for path in &created {
            mutation = mutation.created(path);
        }
        for path in &updated {
            mutation = mutation.updated(path);
        }
        let changes = created
            .iter()
            .map(|path| format!("created {}", path.display()))
            .chain(
                updated
                    .iter()
                    .map(|path| format!("updated {}", path.display())),
            )
            .collect::<Vec<_>>();
        let CfeBorrowLog {
            auto_borrowed,
            skipped,
            warnings: borrow_warnings,
        } = log;
        let data = CfeBorrowData {
            name_prefix,
            extension: ext_dir.display().to_string(),
            configuration: cfg_dir.display().to_string(),
            borrowed,
            auto_borrowed,
            skipped,
            mutation,
        };
        artifacts.push(ext_path);
        let mut warnings = borrow_warnings;
        warnings.extend(cleanup_warnings);
        Ok(CommittedCfeBorrow {
            data,
            artifacts,
            changes,
            warnings,
        })
    })();

    match result {
        Ok(CommittedCfeBorrow {
            data,
            artifacts,
            changes,
            warnings,
        }) => CfeBorrowExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.cfe.borrow borrowed {} item(s) into {}",
                    data.borrowed.len(),
                    data.extension
                ),
                changes,
                warnings,
                errors: Vec::new(),
                artifacts: artifacts
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(data),
        },
        Err(error) => CfeBorrowExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.cfe.borrow failed in native extension borrower".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

fn cfe_borrow_validate_extension(
    extension_path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let validation_args = Map::from_iter([(
        "ExtensionPath".to_string(),
        Value::String(extension_path.display().to_string()),
    )]);
    let outcome = validate_cfe(&validation_args, context);
    if outcome.ok {
        return Ok(());
    }
    let detail = if outcome.errors.is_empty() {
        outcome
            .stdout
            .unwrap_or_else(|| "validation returned no diagnostics".to_string())
    } else {
        outcome.errors.join("; ")
    };
    Err(format!("cfe validation failed: {detail}"))
}

#[derive(Debug, Default)]
pub(crate) struct CfeBorrowWritePlan {
    files: BTreeMap<PathBuf, CfeBorrowPlannedFile>,
    dependencies: BTreeMap<PathBuf, Vec<u8>>,
    read_trace: CfeBorrowReadTrace,
}

#[derive(Debug)]
struct CfeBorrowPlannedFile {
    original: Option<Vec<u8>>,
    updated: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
struct CfeBorrowReadTrace(Rc<RefCell<Vec<PathBuf>>>);

impl CfeBorrowReadTrace {
    fn record(&self, path: &Path) {
        self.0.borrow_mut().push(path.to_path_buf());
    }

    fn xml_paths(&self) -> Vec<PathBuf> {
        let mut paths = self
            .0
            .borrow()
            .iter()
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            })
            .cloned()
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }
}

impl CfeBorrowWritePlan {
    fn with_read_trace(read_trace: CfeBorrowReadTrace) -> Self {
        Self {
            read_trace,
            ..Self::default()
        }
    }

    fn format_dependency_paths(&self) -> Vec<PathBuf> {
        let mut paths = self
            .dependencies
            .keys()
            .chain(self.files.keys())
            .cloned()
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }

    fn is_planned(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path) || path.exists()
    }

    fn read_utf8_sig(&mut self, path: &Path) -> Result<String, String> {
        if !self.files.contains_key(path) {
            let original = match self.dependencies.remove(path) {
                Some(original) => original,
                None => {
                    let original = fs::read(path)
                        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
                    self.read_trace.record(path);
                    original
                }
            };
            self.files.insert(
                path.to_path_buf(),
                CfeBorrowPlannedFile {
                    original: Some(original.clone()),
                    updated: original,
                },
            );
        }
        let bytes = &self
            .files
            .get(path)
            .expect("tracked path was inserted")
            .updated;
        let text = std::str::from_utf8(bytes)
            .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
        Ok(text.trim_start_matches('\u{feff}').to_string())
    }

    fn read_dependency_utf8_sig(&mut self, path: &Path) -> Result<String, String> {
        if !self.dependencies.contains_key(path) {
            let raw = fs::read(path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            self.read_trace.record(path);
            self.dependencies.insert(path.to_path_buf(), raw);
        }
        let raw = self
            .dependencies
            .get(path)
            .expect("tracked dependency was inserted");
        let text = std::str::from_utf8(raw)
            .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
        Ok(text.trim_start_matches('\u{feff}').to_string())
    }

    fn read_current_or_dependency_utf8_sig(
        &mut self,
        path: &Path,
    ) -> Result<Option<String>, String> {
        if let Some(file) = self.files.get(path) {
            let text = std::str::from_utf8(&file.updated)
                .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
            return Ok(Some(text.trim_start_matches('\u{feff}').to_string()));
        }
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => self.read_dependency_utf8_sig(path).map(Some),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
        }
    }

    fn write_utf8_bom(&mut self, path: &Path, text: &str) -> Result<(), String> {
        let updated = unica_xml_format::text::utf8_bom_bytes(text);
        if let Some(file) = self.files.get_mut(path) {
            file.updated = updated;
            return Ok(());
        }

        let original = match self.dependencies.remove(path) {
            Some(original) => Some(original),
            None => match fs::read(path) {
                Ok(bytes) => {
                    self.read_trace.record(path);
                    Some(bytes)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(format!("failed to read {}: {error}", path.display()));
                }
            },
        };
        self.files.insert(
            path.to_path_buf(),
            CfeBorrowPlannedFile { original, updated },
        );
        Ok(())
    }

    /// Identity the descriptor at `path` already carries, or an empty one when
    /// there is nothing there yet. Both object and form borrowing use this reader.
    fn validate_xml(&self) -> Result<(), String> {
        cfe_borrow_validate_planned_xml(
            self.files
                .iter()
                .map(|(path, file)| (path.as_path(), file.updated.as_slice())),
        )
    }

    fn commit(self) -> Result<CommitReport, String> {
        self.commit_inner(None, || Ok(()))
    }

    /// Returns what the transaction actually published, so the caller reports
    /// the effect of the commit rather than restating its own plan.
    fn commit_with_post_validation<F>(
        self,
        format_owner_targets: &[&Path],
        context: &WorkspaceContext,
        post_validation: F,
    ) -> Result<CommitReport, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        self.commit_inner(Some((format_owner_targets, context)), post_validation)
    }

    fn commit_inner<F>(
        self,
        format_guard: Option<(&[&Path], &WorkspaceContext)>,
        post_validation: F,
    ) -> Result<CommitReport, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        self.validate_xml()?;
        let CfeBorrowWritePlan {
            files,
            dependencies,
            ..
        } = self;
        let mut transaction = CompileTransaction::new();
        let mut format_snapshots = dependencies.clone();
        for (path, raw) in &dependencies {
            guard_exact_preimage_if_unprotected(&mut transaction, path, raw)?;
        }
        for (path, file) in files {
            if let Some(original) = &file.original {
                if path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
                {
                    format_snapshots.insert(path.clone(), original.clone());
                }
            }
            match file.original {
                Some(original) => {
                    transaction.replace_bytes(path, &original, file.updated)?;
                }
                None => transaction.create_bytes(path, file.updated)?,
            }
        }
        if let Some((owner_targets, context)) = format_guard {
            guard_cfe_active_format_snapshot_set(
                &mut transaction,
                &format_snapshots,
                owner_targets,
                &[],
                context,
            )?;
        }
        transaction.commit_with_post_validation(post_validation)
    }
}

pub(crate) fn cfe_borrow_resolve_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
    kind: &str,
) -> Result<PathBuf, String> {
    let raw = unica_operation_contract::input::required_path(
        args,
        names,
        if kind == "extension" {
            "ExtensionPath"
        } else {
            "ConfigPath"
        },
    )?;
    let mut path = unica_operation_contract::input::absolutize(raw, &context.cwd);
    if path.is_dir() {
        let candidate = path.join("Configuration.xml");
        if candidate.is_file() {
            path = candidate;
        } else if kind == "extension" {
            return Err(format!(
                "No Configuration.xml in extension directory: {}",
                path.display()
            ));
        } else {
            return Err(format!(
                "No Configuration.xml in config directory: {}",
                path.display()
            ));
        }
    }
    if !path.is_file() {
        if kind == "extension" {
            return Err(format!("Extension file not found: {}", path.display()));
        }
        return Err(format!("Config file not found: {}", path.display()));
    }
    Ok(path)
}

pub(crate) fn diff_cfe(args: &Map<String, Value>, context: &WorkspaceContext) -> CfeDiffExecution {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(CfeDiffData, PathBuf), String> {
        let extension_path_raw = unica_operation_contract::input::required_path(
            args,
            &["extensionPath", "ExtensionPath"],
            "ExtensionPath",
        )?;
        let config_path_raw = unica_operation_contract::input::required_path(
            args,
            &["configPath", "ConfigPath"],
            "ConfigPath",
        )?;
        let mut extension_path =
            unica_operation_contract::input::absolutize(extension_path_raw, &context.cwd);
        let mut config_path =
            unica_operation_contract::input::absolutize(config_path_raw, &context.cwd);
        if extension_path.is_file() {
            extension_path = extension_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf();
        }
        if config_path.is_file() {
            config_path = config_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf();
        }

        let ext_cfg = extension_path.join("Configuration.xml");
        let src_cfg = config_path.join("Configuration.xml");
        if !ext_cfg.is_file() {
            return Err(format!(
                "Extension Configuration.xml not found: {}",
                ext_cfg.display()
            ));
        }
        if !src_cfg.is_file() {
            return Err(format!(
                "Config Configuration.xml not found: {}",
                src_cfg.display()
            ));
        }

        let ext_text = read_utf8_sig(&ext_cfg)?;
        cfe_diff_source(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.cwd),
            &ext_text,
            ext_cfg,
            &extension_path,
            &config_path,
        )
    })();

    match result {
        Ok((data, artifact)) => CfeDiffExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.cfe.diff described {} with {} borrowed and {} own object(s)",
                    data.name, data.totals.borrowed, data.totals.own
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![artifact.display().to_string()],
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(data),
        },
        Err(error) => CfeDiffExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.cfe.diff failed in native extension diff analyzer".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

pub(crate) struct CfeDiffExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CfeDiffData>,
}

pub(crate) fn validate_cfe(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| {
        let resolved_path = resolve_cfe_validate_config_path(args, context)?;
        let reader = unica_workspace::discovery::ProjectDiscoveryReader::new(&context.cwd);
        unica_metadata_designer::extension::validate_cfe_source(
            &reader,
            args,
            resolved_path,
            &context.cwd,
            &mut cfe_validate_module_layouts,
        )
    })();

    match result {
        Ok(run) => AdapterOutcome {
            ok: run.ok,
            summary: if run.ok {
                "unica.cfe.validate completed with native extension validator".to_string()
            } else {
                "unica.cfe.validate failed in native extension validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: run.errors,
            artifacts: vec![run.artifact.display().to_string()],
            stdout: Some(run.stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cfe.validate failed in native extension validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: Some(format!("{error}\n")),
            stderr: Some(String::new()),
            command: None,
        },
    }
}

// Check 14: semantics of service child objects (HTTPService, WebService) —
// nested enum literals such as Method HTTPMethod are validated against the
// 8.3.27 platform enums, whether the service is own or adopted. The platform
// rejects the whole extension import on an invalid literal, so cfe.validate
// must reject it too instead of reporting a clean extension.

/// Enumerate roles from the domain profile, then use the same physical adapter
/// as the writer. This check covers top-level direct, owner and command modules.
fn cfe_validate_module_layouts(
    kind: &str,
    name: &str,
) -> Result<Vec<(PathBuf, &'static str)>, String> {
    use unica_address::address::QualifiedAddress;
    use unica_address::module_routes::ModuleRouteKind;
    use unica_bsl::profile::PlatformProfile;
    use unica_source_designer::module_layout::module_source_address;
    let owner = QualifiedAddress::parse(&format!("extension:{kind}.{name}"))
        .map_err(|error| error.to_string())?;
    let branch =
        QualifiedAddress::parse(&format!("{owner}.Module")).map_err(|error| error.to_string())?;
    let profile = PlatformProfile::v8_3_27();
    let children = profile.module_children(&branch);
    let candidates = profile
        .module_capability(&owner)
        .map(|capability| (&owner, capability))
        .into_iter()
        .chain(
            children
                .iter()
                .map(|module| (module.at(), module.capability())),
        );
    let mut modules = Vec::new();
    for (at, capability) in candidates {
        if !matches!(
            capability.route_kind(),
            ModuleRouteKind::Direct
                | ModuleRouteKind::Common
                | ModuleRouteKind::Service
                | ModuleRouteKind::Bot
                | ModuleRouteKind::CommonCommand
        ) {
            continue;
        }
        let address = module_source_address(
            at,
            capability.owner_kind(),
            capability.role(),
            capability.route_kind(),
        )?;
        let relative = platform_xml_module_relative(&address)?;
        let identity = platform_xml_module_identity(&relative)?;
        modules.push((relative, identity.role.as_str()));
    }
    Ok(modules)
}

fn guard_cfe_active_format_snapshot_set(
    transaction: &mut CompileTransaction,
    snapshots: &BTreeMap<PathBuf, Vec<u8>>,
    owner_targets: &[&Path],
    new_outputs: &[&Path],
    context: &WorkspaceContext,
) -> Result<(), String> {
    for (path, raw) in snapshots {
        transaction.guard_or_verify_exact_preimage(path, raw)?;
    }
    let mut snapshots = snapshots.clone();
    for target in owner_targets {
        let resolution =
            crate::infrastructure::platform_xml_owner::resolve_platform_xml_owners_with_provenance(
                target, context,
            )
            .map_err(|error| error.message)?;
        resolution.provenance.bind_to(transaction)?;
        for owner in resolution.owners {
            snapshots.entry(owner.path).or_insert(owner.raw);
        }
    }
    for output in new_outputs {
        let resolution = crate::infrastructure::platform_xml_owner::
            resolve_existing_platform_xml_owners_for_new_output_with_provenance(output, context)
            .map_err(|error| error.message)?;
        resolution.provenance.bind_to(transaction)?;
        for owner in resolution.owners {
            snapshots.entry(owner.path).or_insert(owner.raw);
        }
    }

    let mut invalid = None;
    let mut newer = None;
    let mut older = None;
    for (path, raw) in &snapshots {
        let compatibility =
            unica_source_designer::configuration_layout::classify_configuration_xml_image(
                path, raw,
            );
        match compatibility {
            Ok(FormatCompatibility::Supported { .. }) => {}
            Ok(compatibility @ FormatCompatibility::Newer { .. }) if newer.is_none() => {
                newer = Some(compatibility);
            }
            Ok(compatibility @ FormatCompatibility::Older { .. }) if older.is_none() => {
                older = Some(compatibility);
            }
            Ok(FormatCompatibility::Newer { .. } | FormatCompatibility::Older { .. }) => {}
            Err(error) if invalid.is_none() => invalid = Some(error),
            Err(_) => {}
        }
    }
    if let Some(error) = invalid {
        return Err(error);
    }
    if let Some(compatibility) = newer.or(older) {
        return Err(
            unica_source_designer::format_profile::format_compatibility_warning(&compatibility),
        );
    }
    for (path, raw) in snapshots {
        transaction.guard_or_verify_exact_preimage(path, raw)?;
    }
    Ok(())
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "cfe-validate" => Some(Ok(validate_cfe(args, context))),
        // Typed answer; data reaches the envelope through typed_result.rs.
        "cfe-diff" => Some(Ok(diff_cfe(args, context).outcome)),
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
        "cfe-borrow" => Some(borrow_cfe(args, context)),
        _ => None,
    }
}

impl unica_source_contract::proposal::TrackedSourceProposal for CfeBorrowWritePlan {
    fn is_planned(&self, path: &Path) -> bool {
        CfeBorrowWritePlan::is_planned(self, path)
    }
    fn exists(&self, path: &Path) -> bool {
        CfeBorrowWritePlan::exists(self, path)
    }
    fn read_utf8_sig(&mut self, path: &Path) -> Result<String, String> {
        CfeBorrowWritePlan::read_utf8_sig(self, path)
    }
    fn read_dependency_utf8_sig(&mut self, path: &Path) -> Result<String, String> {
        CfeBorrowWritePlan::read_dependency_utf8_sig(self, path)
    }
    fn write_utf8_bom(&mut self, path: &Path, text: &str) -> Result<(), String> {
        CfeBorrowWritePlan::write_utf8_bom(self, path, text)
    }
}

#[cfg(test)]
mod borrowed_form_binding_regressions {
    use super::*;

    #[test]
    fn no_main_attribute_removes_owner_paths_and_links_from_both_snapshots() {
        for name in ["Объект", "DocumentData"] {
            let source = format!(
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><TitleDataPath>{name}.Title</TitleDataPath><AutoCommandBar><ChoiceParameterLinks><xr:Link><xr:DataPath>{name}.Partner</xr:DataPath></xr:Link></ChoiceParameterLinks></AutoCommandBar><ChildItems><InputField name="Quantity" id="1"><DataPath>{name}.Items.Quantity</DataPath><FooterDataPath>{name}.Items.TotalQuantity</FooterDataPath><HeaderDataPath>{name}.Header</HeaderDataPath><ChoiceParameterLinks><xr:Link><xr:DataPath>{name}.Partner</xr:DataPath></xr:Link></ChoiceParameterLinks></InputField></ChildItems><Attributes><Attribute name="{name}"><MainAttribute>true</MainAttribute></Attribute></Attributes></Form>"#
            );
            let mut log = CfeBorrowLog::default();
            let generated = cfe_borrow_form_xml(
                &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
                &source,
                Path::new("."),
                "Document",
                "Order",
                false,
                &mut log,
            );
            let doc = Document::parse(&generated).unwrap();
            let root = doc.root_element();
            let base = root
                .children()
                .find(|n| n.has_tag_name("BaseForm"))
                .unwrap();
            for snapshot in [root, base] {
                let attrs = snapshot
                    .children()
                    .find(|n| n.has_tag_name("Attributes"))
                    .unwrap();
                assert_eq!(attrs.children().filter(|n| n.is_element()).count(), 0);
                for node in snapshot.descendants().filter(|n| n.is_element()) {
                    if node.tag_name().name().ends_with("DataPath") {
                        assert!(
                            !node.text().unwrap_or("").starts_with(name),
                            "dangling {} in {}",
                            node.tag_name().name(),
                            snapshot.tag_name().name()
                        );
                    }
                }
                assert_eq!(
                    snapshot
                        .descendants()
                        .filter(|n| n.has_tag_name("Link"))
                        .count(),
                    0
                );
            }
        }
    }

    const OWNER: &str = r#"<MetaDataObject><Document><Properties><Name>Order</Name></Properties><ChildObjects><TabularSection><Properties><Name>Товары</Name></Properties><ChildObjects><Attribute><Properties><Name>TotalCost</Name></Properties></Attribute><Attribute><Properties><Name>Cost</Name><Type><Type>xs:decimal</Type></Type></Properties></Attribute><Attribute><Properties><Name>Количество</Name><Type><Type>xs:decimal</Type></Type></Properties></Attribute><Attribute><Properties><Name>Label</Name><Type><Type>xs:string</Type></Type></Properties></Attribute></ChildObjects></TabularSection></ChildObjects></Document></MetaDataObject>"#;

    #[test]
    fn footer_inference_preserves_metadata_fields_reference_paths_and_unknown_owner() {
        let source = r#"<Form><FooterDataPath>Объект.Товары.TotalCost</FooterDataPath><InputField><DataPath>Объект.Товары.Количество</DataPath><FooterDataPath>Объект.Товары.TotalКоличество</FooterDataPath></InputField><FooterDataPath>Объект.Партнер.TotalCost</FooterDataPath><FooterDataPath>Объект.Unknown.TotalCost</FooterDataPath></Form>"#;
        let omitted =
            cfe_borrow_synthetic_footer_paths(source, Some(OWNER), "Document", "Order").unwrap();
        assert_eq!(
            omitted,
            HashSet::from(["Объект.Товары.TotalКоличество".to_string()])
        );
        for owner in [None, Some("<broken"), Some("<Other/>")] {
            assert!(
                cfe_borrow_synthetic_footer_paths(source, owner, "Document", "Order").is_none()
            );
        }
        assert!(
            cfe_borrow_synthetic_footer_paths(source, Some(OWNER), "Document", "Other").is_none()
        );
    }

    #[test]
    fn footer_inference_preserves_unpaired_nonnumeric_and_other_main_attribute_paths() {
        for source in [
            r#"<Form><FooterDataPath>Объект.Товары.TotalКоличество</FooterDataPath></Form>"#,
            r#"<Form><InputField><DataPath>Объект.Товары.Other</DataPath><FooterDataPath>Объект.Товары.TotalКоличество</FooterDataPath></InputField></Form>"#,
            r#"<Form><InputField><DataPath>Объект.Товары.Label</DataPath><FooterDataPath>Объект.Товары.TotalLabel</FooterDataPath></InputField></Form>"#,
            r#"<Form><InputField><DataPath>DocumentData.Товары.Количество</DataPath><FooterDataPath>DocumentData.Товары.TotalКоличество</FooterDataPath></InputField></Form>"#,
            r#"<Form><InputField><DataPath>Объект.Товары.Cost</DataPath><FooterDataPath>Объект.Товары.TotalCost</FooterDataPath></InputField></Form>"#,
        ] {
            let omitted =
                cfe_borrow_synthetic_footer_paths(source, Some(OWNER), "Document", "Order")
                    .unwrap();
            assert!(omitted.is_empty(), "must retain uncertain footer: {source}");
            assert_eq!(
                cfe_borrow_prune_omitted_form_bindings(source, &omitted),
                source
            );
        }
    }

    #[test]
    fn borrowed_form_and_base_form_preserve_valid_bindings_without_omitted_data() {
        let source = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema"><AutoCommandBar/><ChildItems><InputField name="ItemCode" id="1"><DataPath>Объект.Товары.Артикул</DataPath></InputField><InputField name="Quantity" id="2"><DataPath>Объект.Товары.Количество</DataPath><FooterDataPath>Объект.Товары.TotalКоличество</FooterDataPath><ChoiceParameterLinks><xr:Link><xr:DataPath xsi:type="xs:string">UseAgreements</xr:DataPath></xr:Link><xr:Link><xr:DataPath xsi:type="xs:string">Объект.Партнер</xr:DataPath></xr:Link></ChoiceParameterLinks></InputField></ChildItems><Attributes><Attribute name="Объект"><MainAttribute>true</MainAttribute><Columns><AdditionalColumns table="Объект.Товары"><Column name="Артикул"/></AdditionalColumns></Columns></Attribute><Attribute name="UseAgreements"/></Attributes></Form>"#;
        let mut log = CfeBorrowLog::default();
        let footer_paths =
            cfe_borrow_synthetic_footer_paths(source, Some(OWNER), "Document", "Order").unwrap();
        let cleaned = cfe_borrow_prune_omitted_form_bindings(source, &footer_paths);
        let generated = cfe_borrow_form_xml(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
            &cleaned,
            Path::new("."),
            "Document",
            "Order",
            true,
            &mut log,
        );
        let doc = Document::parse(&generated).expect("borrowed XML must parse");
        let root = doc.root_element();
        let base = root
            .children()
            .find(|node| node.has_tag_name("BaseForm"))
            .expect("BaseForm snapshot");
        for snapshot in [root, base] {
            let items = snapshot
                .children()
                .find(|node| node.has_tag_name("ChildItems"))
                .expect("snapshot items");
            let paths: Vec<_> = items
                .descendants()
                .filter(|node| matches!(node.tag_name().name(), "DataPath" | "FooterDataPath"))
                .filter_map(|node| node.text())
                .collect();
            assert_eq!(paths, vec!["Объект.Товары.Количество", "Объект.Партнер"]);
            assert_eq!(
                items
                    .descendants()
                    .filter(|node| node.has_tag_name("Link"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn removes_only_bindings_to_omitted_form_data_including_namespaced_links() {
        let source = r#"<Form><ChildItems><FooterDataPath>Объект.Товары.TotalКоличество</FooterDataPath></ChildItems><Attributes><Attribute name="Объект"><MainAttribute>true</MainAttribute><Columns><AdditionalColumns table="Объект.Товары"><Column name="Артикул"/></AdditionalColumns></Columns></Attribute><Attribute name="ИспользоватьСоглашенияСКлиентами"/></Attributes></Form>"#;
        let omitted = cfe_borrow_omitted_form_paths(source, true);
        assert!(omitted.contains("Объект.Товары.Артикул"));
        assert!(!omitted.contains("Объект.Товары.TotalКоличество"));
        assert!(!omitted.contains("Объект"));
        let input = r#"<ChildItems><DataPath>Объект.Товары.Артикул</DataPath><DataPath>Объект.Товары.Количество</DataPath><FooterDataPath>ИспользоватьСоглашенияСКлиентами</FooterDataPath><ChoiceParameterLinks><xr:Link><xr:DataPath xsi:type="xs:string">ИспользоватьСоглашенияСКлиентами</xr:DataPath></xr:Link><xr:Link><xr:DataPath xsi:type="xs:string">Объект.Партнер</xr:DataPath></xr:Link></ChoiceParameterLinks></ChildItems>"#;
        let result = cfe_borrow_prune_omitted_form_bindings(input, &omitted);
        assert!(!result.contains("Артикул"));
        assert!(!result.contains("ИспользоватьСоглашенияСКлиентами"));
        assert!(result.contains("Объект.Товары.Количество"));
        assert!(result.contains("Объект.Партнер"));
        assert_eq!(result.matches("<xr:Link>").count(), 1);
        assert_eq!(
            cfe_borrow_prune_omitted_form_bindings(&result, &omitted),
            result
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_workspace::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use unica_workspace::single_file_publisher::with_before_commit_hook;

    #[test]
    fn cfe_validate_rejects_report_without_required_child_objects() {
        let context = temp_context("validate-report-child-objects");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "Report", "PriceList", "");
        let args = Map::from_iter([("ExtensionPath".to_string(), json!("ext"))]);

        let outcome = validate_cfe(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("Report.PriceList: ChildObjects is required"),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_rejects_foreign_child_objects_namespace() {
        let context = temp_context("validate-child-objects-namespace");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(
            &context,
            "Report",
            "PriceList",
            r#"<x:ChildObjects xmlns:x="urn:foreign"/>"#,
        );
        let args = Map::from_iter([("ExtensionPath".to_string(), json!("ext"))]);

        let outcome = validate_cfe(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("Report.PriceList: ChildObjects is required"),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_rejects_child_objects_before_properties() {
        let context = temp_context("validate-child-objects-order");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let descriptor =
            register_borrowed_patch_object(&context, "Report", "PriceList", "<ChildObjects/>");
        let text = fs::read_to_string(&descriptor)
            .unwrap()
            .replacen(
                "\t\t<Properties>",
                "\t\t<ChildObjects/>\n\t\t<Properties>",
                1,
            )
            .replacen("\n\t\t<ChildObjects/>\n\t</Report>", "\n\t</Report>", 1);
        fs::write(&descriptor, text).unwrap();
        let args = Map::from_iter([("ExtensionPath".to_string(), json!("ext"))]);

        let outcome = validate_cfe(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("Report.PriceList: ChildObjects must follow Properties"),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_rejects_module_file_without_extended_property_state() {
        let context = temp_context("validate-module-property-state");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "Catalog", "Items", "<ChildObjects/>");
        write_file(
            &context.cwd.join("ext/Catalogs/Items/Ext/ObjectModule.bsl"),
            "&Before(\"Run\")\nProcedure GE_Run()\nEndProcedure\n",
        );
        let args = Map::from_iter([("ExtensionPath".to_string(), json!("ext"))]);

        let outcome = validate_cfe(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("Catalog.Items: Ext/ObjectModule.bsl exists but PropertyState ObjectModule=Extended is missing"),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_rejects_conflicting_extended_property_state_children() {
        let context = temp_context("validate-conflicting-property-state");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let descriptor =
            register_borrowed_patch_object(&context, "Catalog", "Items", "<ChildObjects/>");
        let text = fs::read_to_string(&descriptor).unwrap().replacen(
            "<InternalInfo/>",
            r#"<InternalInfo><xr:PropertyState xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><xr:Property>ObjectModule</xr:Property><xr:State>Notify</xr:State><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo>"#,
            1,
        );
        fs::write(&descriptor, text).unwrap();
        write_file(
            &context.cwd.join("ext/Catalogs/Items/Ext/ObjectModule.bsl"),
            "&Before(\"Run\")\nProcedure GE_Run()\nEndProcedure\n",
        );
        let args = Map::from_iter([("ExtensionPath".to_string(), json!("ext"))]);

        let outcome = validate_cfe(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains(
                "Catalog.Items: Ext/ObjectModule.bsl exists but PropertyState ObjectModule=Extended is missing"
            ),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_rejects_reversed_extended_property_state_children() {
        let context = temp_context("validate-reversed-property-state");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let descriptor =
            register_borrowed_patch_object(&context, "Catalog", "Items", "<ChildObjects/>");
        let text = fs::read_to_string(&descriptor).unwrap().replacen(
            "<InternalInfo/>",
            r#"<InternalInfo><xr:PropertyState xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><xr:State>Extended</xr:State><xr:Property>ObjectModule</xr:Property></xr:PropertyState></InternalInfo>"#,
            1,
        );
        fs::write(&descriptor, text).unwrap();
        write_file(
            &context.cwd.join("ext/Catalogs/Items/Ext/ObjectModule.bsl"),
            "&Before(\"Run\")\nProcedure GE_Run()\nEndProcedure\n",
        );
        let args = Map::from_iter([("ExtensionPath".to_string(), json!("ext"))]);

        let outcome = validate_cfe(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains(
                "Catalog.Items: Ext/ObjectModule.bsl exists but PropertyState ObjectModule=Extended is missing"
            ),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_checks_every_platform_direct_and_owner_module_role() {
        for (type_name, role) in [
            ("Report", "ObjectModule"),
            ("Report", "ManagerModule"),
            ("DataProcessor", "ObjectModule"),
            ("Catalog", "ObjectModule"),
            ("InformationRegister", "RecordSetModule"),
            ("InformationRegister", "ManagerModule"),
            ("Constant", "ValueManagerModule"),
            ("Constant", "ManagerModule"),
            ("CommonModule", "Module"),
            ("SettingsStorage", "ManagerModule"),
            ("Bot", "Module"),
            ("HTTPService", "Module"),
            ("WebService", "Module"),
            ("IntegrationService", "Module"),
            ("CommonCommand", "CommandModule"),
        ] {
            let context = temp_context(&format!("validate-{type_name}-{role}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            let descriptor = register_borrowed_patch_object(
                &context,
                type_name,
                "Evidence",
                if cfe_borrow_type_has_child_objects(type_name).unwrap() {
                    "<ChildObjects/>"
                } else {
                    ""
                },
            );
            let dir_name = cf_validate_child_type_dir(type_name).unwrap();
            write_file(
                &context
                    .cwd
                    .join("ext")
                    .join(dir_name)
                    .join("Evidence/Ext")
                    .join(format!("{role}.bsl")),
                "Procedure Evidence()\nEndProcedure\n",
            );
            let args = Map::from_iter([("ExtensionPath".to_string(), json!("ext"))]);

            let outcome = validate_cfe(&args, &context);

            assert!(!outcome.ok, "{type_name}.{role}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains(&format!(
                    "{type_name}.Evidence: Ext/{role}.bsl exists but PropertyState {role}=Extended is missing"
                )),
                "{type_name}.{role}: {outcome:?}"
            );
            let xml = fs::read_to_string(&descriptor).unwrap();
            let connected = xml.replace("<InternalInfo/>", &format!(
                "<InternalInfo><xr:PropertyState xmlns:xr=\"{CFE_PATCH_XR_NAMESPACE}\"><xr:Property>{role}</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo>"
            ));
            assert_ne!(connected, xml);
            fs::write(&descriptor, &connected).unwrap();
            let valid = validate_cfe(&args, &context);
            assert!(valid.ok, "{type_name}.{role}: {valid:?}");
            assert_eq!(fs::read_to_string(&descriptor).unwrap(), connected);
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-cfe-borrow-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn matching_mutation_path<'a>(paths: &'a [String], expected: &Path) -> Option<&'a str> {
        let expected = unica_workspace::test_support::canonical_path(expected);
        paths
            .iter()
            .find(|path| unica_workspace::test_support::canonical_path(Path::new(path)) == expected)
            .map(String::as_str)
    }

    fn write_minimal_borrow_fixture(
        context: &WorkspaceContext,
        base_version: &str,
        source_object_version: &str,
        extension_version: &str,
        existing_target_version: Option<&str>,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let base_owner = context.cwd.join("src/Configuration.xml");
        let source_object = context.cwd.join("src/Catalogs/Items.xml");
        let extension_owner = context.cwd.join("ext/Configuration.xml");
        write_file(
            &base_owner,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{base_version}">
	<Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#
            ),
        );
        write_file(
            &source_object,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{source_object_version}">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties><Name>Items</Name></Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#
            ),
        );
        write_file(
            &extension_owner,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{extension_version}">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>GuardedExtension</Name>
			<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>
			<NamePrefix>GE_</NamePrefix>
		</Properties>
		<ChildObjects/>
	</Configuration>
</MetaDataObject>
"#
            ),
        );
        let target = context.cwd.join("ext/Catalogs/Items.xml");
        if let Some(version) = existing_target_version {
            write_file(
                &target,
                &format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}">
	<Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Items</Name>
			<ExtendedConfigurationObject>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#
                ),
            );
        }
        (base_owner, source_object, extension_owner)
    }

    #[test]
    fn cfe_transaction_guards_reject_entity_spelled_supported_format() {
        let context = temp_context("entity-spelled-format");
        let path = context.cwd.join("Configuration.xml");
        let raw = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration/></MetaDataObject>"#
            .to_vec();
        fs::write(&path, &raw).unwrap();

        let mut snapshots = BTreeMap::new();
        snapshots.insert(path.clone(), raw.clone());
        let mut transaction = CompileTransaction::new();
        let guard_error =
            guard_cfe_active_format_snapshot_set(&mut transaction, &snapshots, &[], &[], &context)
                .unwrap_err();
        assert!(
            guard_error.contains("invalid export format version"),
            "{guard_error}"
        );

        let patch_error = match cfe_patch_supported_document(&path, &raw) {
            Ok(_) => panic!("entity-spelled version must not be accepted"),
            Err(error) => error,
        };
        assert!(
            patch_error.contains("invalid export format version"),
            "{patch_error}"
        );

        fs::remove_dir_all(context.cwd).unwrap();
    }

    #[test]
    fn cfe_borrow_common_module_copies_canonical_properties_in_8_3_27_order() {
        let source = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses">
	<Global>false</Global>
	<ClientManagedApplication>false</ClientManagedApplication>
	<Server>true</Server>
	<ExternalConnection>true</ExternalConnection>
	<ClientOrdinaryApplication>false</ClientOrdinaryApplication>
	<ServerCall>true</ServerCall>
	<Privileged>true</Privileged>
	<ReturnValuesReuse>DuringSession</ReturnValuesReuse>
</Properties>"#,
        )
        .unwrap();

        let xml = cfe_borrow_object_xml(
            "CommonModule",
            "CanonicalModule",
            "11111111-1111-1111-1111-111111111111",
            Some(source.root_element()),
            "2.20",
            &CfeBorrowIdentity::default(),
            crate::infrastructure::source_ids::fresh_uuid,
        )
        .unwrap();

        let generated = Document::parse(&xml).unwrap();
        let properties = generated
            .descendants()
            .find(|node| node.has_tag_name((CFE_PATCH_MD_NAMESPACE, "Properties")))
            .unwrap();
        let names = properties
            .children()
            .filter(|node| node.is_element())
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "ObjectBelonging",
                "Name",
                "Comment",
                "ExtendedConfigurationObject",
                "Global",
                "ClientManagedApplication",
                "Server",
                "ExternalConnection",
                "ClientOrdinaryApplication",
                "ServerCall",
                "ReturnValuesReuse",
            ]
        );
        assert!(
            properties
                .children()
                .filter(roxmltree::Node::is_element)
                .all(|node| node.tag_name().name() != "Privileged"),
            "8.3.27 omits Privileged from an adopted CommonModule descriptor: {xml}"
        );
        assert_eq!(
            cfe_patch_exact_md_text(
                properties,
                "ReturnValuesReuse",
                "test",
                Path::new("generated.xml"),
            )
            .unwrap(),
            "DuringSession"
        );

        let defaults_xml = cfe_borrow_object_xml(
            "CommonModule",
            "DefaultModule",
            "22222222-2222-2222-2222-222222222222",
            None,
            "2.20",
            &CfeBorrowIdentity::default(),
            crate::infrastructure::source_ids::fresh_uuid,
        )
        .unwrap();
        let defaults = Document::parse(&defaults_xml).unwrap();
        let default_properties = defaults
            .descendants()
            .find(|node| node.has_tag_name((CFE_PATCH_MD_NAMESPACE, "Properties")))
            .unwrap();
        assert!(
            default_properties
                .children()
                .filter(roxmltree::Node::is_element)
                .all(|node| node.tag_name().name() != "Privileged"),
            "8.3.27 omits default Privileged from an adopted CommonModule descriptor: {defaults_xml}"
        );
        assert_eq!(
            cfe_patch_exact_md_text(
                default_properties,
                "ReturnValuesReuse",
                "test",
                Path::new("generated.xml"),
            )
            .unwrap(),
            "DontUse"
        );
    }

    /// Every regular file under `root` with its exact bytes, so a test can
    /// assert that a call changed nothing at all.
    fn workspace_byte_map(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(directory: &Path, map: &mut BTreeMap<PathBuf, Vec<u8>>) {
            let Ok(entries) = fs::read_dir(directory) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, map);
                } else if let Ok(bytes) = fs::read(&path) {
                    map.insert(path, bytes);
                }
            }
        }
        let mut map = BTreeMap::new();
        walk(root, &mut map);
        map
    }

    fn minimal_borrow_args() -> Map<String, Value> {
        Map::from_iter([
            ("ExtensionPath".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src")),
            ("Object".to_string(), json!("Catalog.Items")),
        ])
    }

    #[test]
    fn borrow_cfe_rejects_a_mismatched_source_descriptor_without_mutation() {
        let cases = [
            (
                "namespace",
                "http://v8.1c.ru/8.3/MDClasses",
                "urn:wrong-md-namespace",
                "MetaDataObject namespace",
            ),
            (
                "type",
                "<Catalog uuid=",
                "<Document uuid=",
                "expected exactly one Catalog",
            ),
            (
                "name",
                "<Name>Items</Name>",
                "<Name>Other</Name>",
                "Properties/Name",
            ),
            (
                "invalid-uuid",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "not-a-valid-metadata-object-uuid-value",
                "valid non-nil uuid",
            ),
            (
                "nil-uuid",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "00000000-0000-0000-0000-000000000000",
                "valid non-nil uuid",
            ),
        ];

        for (label, from, to, expected_error) in cases {
            let context = temp_context(&format!("source-descriptor-{label}"));
            let (_, source_object, extension_owner) =
                write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            let source = fs::read_to_string(&source_object).unwrap();
            let mut mutated = source.replacen(from, to, 1);
            if label == "type" {
                mutated = mutated.replacen("</Catalog>", "</Document>", 1);
            }
            assert_ne!(source, mutated, "test mutation must change {label}");
            fs::write(&source_object, mutated.as_bytes()).unwrap();
            let source_before = fs::read(&source_object).unwrap();
            let extension_before = fs::read(&extension_owner).unwrap();

            let outcome = borrow_cfe(&minimal_borrow_args(), &context);

            assert!(!outcome.ok, "{label}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains(expected_error),
                "{label}: expected {expected_error:?}, got {outcome:?}"
            );
            assert_eq!(
                fs::read(&source_object).unwrap(),
                source_before,
                "{label}: source bytes changed"
            );
            assert_eq!(
                fs::read(&extension_owner).unwrap(),
                extension_before,
                "{label}: extension owner bytes changed"
            );
            assert!(
                !context.cwd.join("ext/Catalogs/Items.xml").exists(),
                "{label}: target descriptor must not be created"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_borrow_format_dependencies_ignore_unrelated_newer_xml_without_writes() {
        let context = temp_context("borrow-exact-format-dependencies");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        write_file(
            &context.cwd.join("src/Catalogs/Items/Forms/Main.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="dddddddd-dddd-dddd-dddd-dddddddddddd"><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"#,
        );
        write_file(
            &context
                .cwd
                .join("src/Catalogs/Items/Forms/Main/Ext/Form.xml"),
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Attributes/></Form>"#,
        );
        let unrelated = context.cwd.join("src/Catalogs/Unrelated.xml");
        write_file(
            &unrelated,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog/></MetaDataObject>"#,
        );
        let unregistered_extension = context.cwd.join("ext/Catalogs/Unregistered.xml");
        write_file(
            &unregistered_extension,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog/></MetaDataObject>"#,
        );
        let extension_before = fs::read(&extension_owner).unwrap();
        let args = Map::from_iter([
            ("ExtensionPath".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src")),
            ("Object".to_string(), json!("Catalog.Items.Form.Main")),
            ("BorrowMainAttribute".to_string(), json!("Form")),
        ]);

        let dependencies = cfe_borrow_format_dependency_paths(&args, &context).unwrap();

        assert!(
            dependencies.contains(&context.cwd.join("src/Catalogs/Items.xml")),
            "{dependencies:?}"
        );
        assert!(!dependencies.contains(&unrelated), "{dependencies:?}");
        assert!(
            !dependencies.contains(&unregistered_extension),
            "{dependencies:?}"
        );
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_borrow_format_dependencies_include_deep_referenced_source_without_writes() {
        let context = temp_context("borrow-deep-format-dependencies");
        let (_, source_object, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        write_file(
            &source_object,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties><Name>Items</Name></Properties>
		<ChildObjects>
			<Attribute uuid="cccccccc-cccc-cccc-cccc-cccccccccccc">
				<Properties>
					<Name>Customer</Name>
					<Type><v8:Type>cfg:CatalogRef.Counterparty</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        let source_form_meta = context.cwd.join("src/Catalogs/Items/Forms/Main.xml");
        write_file(
            &source_form_meta,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="dddddddd-dddd-dddd-dddd-dddddddddddd">
		<Properties><Name>Main</Name></Properties>
	</Form>
</MetaDataObject>
"#,
        );
        let source_form = context
            .cwd
            .join("src/Catalogs/Items/Forms/Main/Ext/Form.xml");
        write_file(
            &source_form,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<ChildItems>
		<InputField name="CustomerTaxId" id="1">
			<DataPath>Объект.Customer.TaxId</DataPath>
		</InputField>
	</ChildItems>
	<Attributes/>
</Form>
"#,
        );
        let deep_source = context.cwd.join("src/Catalogs/Counterparty.xml");
        write_file(
            &deep_source,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.21">
	<Catalog uuid="eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee">
		<Properties><Name>Counterparty</Name></Properties>
		<ChildObjects>
			<Attribute uuid="ffffffff-ffff-ffff-ffff-ffffffffffff">
				<Properties>
					<Name>TaxId</Name>
					<Type><v8:Type>xs:string</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        let extension_before = fs::read(&extension_owner).unwrap();
        let args = Map::from_iter([
            ("ExtensionPath".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src")),
            ("Object".to_string(), json!("Catalog.Items.Form.Main")),
            ("BorrowMainAttribute".to_string(), json!("Form")),
        ]);

        let dependencies = cfe_borrow_format_dependency_paths(&args, &context).unwrap();

        for expected in [
            &source_object,
            &source_form_meta,
            &source_form,
            &deep_source,
        ] {
            assert!(dependencies.contains(expected), "{dependencies:?}");
        }
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        assert!(!context.cwd.join("ext/Catalogs/Counterparty.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_registered_xml_dependencies_follow_only_registered_validation_graph() {
        let context = temp_context("registered-validation-dependencies");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let language = context.cwd.join("ext/Languages/Russian.xml");
        write_file(
            &language,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Language/></MetaDataObject>"#,
        );
        let object = context.cwd.join("ext/Catalogs/Registered.xml");
        write_file(
            &object,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
		<Properties><Name>Registered</Name></Properties>
		<ChildObjects><Form>Main</Form></ChildObjects>
	</Catalog>
</MetaDataObject>"#,
        );
        let form_wrapper = context.cwd.join("ext/Catalogs/Registered/Forms/Main.xml");
        write_file(
            &form_wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Form/></MetaDataObject>"#,
        );
        let form_xml = context
            .cwd
            .join("ext/Catalogs/Registered/Forms/Main/Ext/Form.xml");
        write_file(
            &form_xml,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#,
        );
        let unregistered = context.cwd.join("ext/Catalogs/Unregistered.xml");
        write_file(
            &unregistered,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog/></MetaDataObject>"#,
        );
        let extension_text = fs::read_to_string(&extension_owner)
            .unwrap()
            .replace(
                "<ChildObjects/>",
                "<ChildObjects><Language>Russian</Language><Catalog>Registered</Catalog></ChildObjects>",
            );
        fs::write(&extension_owner, extension_text).unwrap();

        let dependencies = cfe_registered_xml_dependency_paths(&extension_owner).unwrap();

        for expected in [
            &extension_owner,
            &language,
            &object,
            &form_wrapper,
            &form_xml,
        ] {
            assert!(dependencies.contains(expected), "{dependencies:?}");
        }
        assert!(!dependencies.contains(&unregistered), "{dependencies:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_rejects_newer_registered_extension_object_without_mutation() {
        let context = temp_context("borrow-registered-newer-object");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let registered = context.cwd.join("ext/Catalogs/Registered.xml");
        write_file(
            &registered,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21">
	<Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Registered</Name>
			<ExtendedConfigurationObject>cccccccc-cccc-cccc-cccc-cccccccccccc</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#,
        );
        let extension_text = fs::read_to_string(&extension_owner).unwrap().replace(
            "<ChildObjects/>",
            "<ChildObjects><Catalog>Registered</Catalog></ChildObjects>",
        );
        fs::write(&extension_owner, extension_text).unwrap();
        let extension_before = fs::read(&extension_owner).unwrap();
        let registered_before = fs::read(&registered).unwrap();

        let outcome = borrow_cfe(&minimal_borrow_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert_eq!(fs::read(&registered).unwrap(), registered_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_rejects_newer_registered_language_without_mutation() {
        let context = temp_context("borrow-registered-newer-language");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let language = context.cwd.join("ext/Languages/Russian.xml");
        write_file(
            &language,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Language uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"/></MetaDataObject>"#,
        );
        let extension_text = fs::read_to_string(&extension_owner).unwrap().replace(
            "<ChildObjects/>",
            "<ChildObjects><Language>Russian</Language></ChildObjects>",
        );
        fs::write(&extension_owner, extension_text).unwrap();
        let extension_before = fs::read(&extension_owner).unwrap();
        let language_before = fs::read(&language).unwrap();

        let outcome = borrow_cfe(&minimal_borrow_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert_eq!(fs::read(&language).unwrap(), language_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_guards_registered_form_read_by_pre_validation_before_object_replacement() {
        let context = temp_context("borrow-prevalidated-registered-form");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", Some("2.20"));
        let object = context.cwd.join("ext/Catalogs/Items.xml");
        let object_text = fs::read_to_string(&object).unwrap().replace(
            "<ChildObjects/>",
            "<ChildObjects><Form>Legacy</Form></ChildObjects>",
        );
        fs::write(&object, object_text).unwrap();
        let wrapper = context.cwd.join("ext/Catalogs/Items/Forms/Legacy.xml");
        write_file(
            &wrapper,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form/></MetaDataObject>"#,
        );
        let form_xml = context
            .cwd
            .join("ext/Catalogs/Items/Forms/Legacy/Ext/Form.xml");
        write_file(
            &form_xml,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#,
        );
        let extension_text = fs::read_to_string(&extension_owner).unwrap().replace(
            "<ChildObjects/>",
            "<ChildObjects><Catalog>Items</Catalog></ChildObjects>",
        );
        fs::write(&extension_owner, extension_text).unwrap();
        let extension_before = fs::read(&extension_owner).unwrap();
        let object_before = fs::read(&object).unwrap();
        let form_before = fs::read(&form_xml).unwrap();

        let outcome = borrow_cfe(&minimal_borrow_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert_eq!(fs::read(&object).unwrap(), object_before);
        assert_eq!(fs::read(&form_xml).unwrap(), form_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn legacy_cfe_borrow_rejects_enum_value_without_writes() {
        let context = temp_context("legacy-enum-value-rejected");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        write_file(
            &context.cwd.join("src/Enums/Kinds.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Enum uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Kinds</Name></Properties><ChildObjects><EnumValue uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>First</Name></Properties></EnumValue></ChildObjects></Enum></MetaDataObject>"#,
        );
        let mut args = minimal_borrow_args();
        args.insert("Object".to_string(), json!("Enum.Kinds.EnumValue.First"));
        let before = unica_workspace::test_support::tree_snapshot(&context.cwd);

        let outcome = borrow_cfe(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("Source object not found"), "{error}");
        assert!(error.contains("Kinds.EnumValue.First.xml"), "{error}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&context.cwd),
            before
        );
        fs::remove_dir_all(&context.cwd).unwrap();
    }

    #[test]
    fn cfe_borrow_batch_order_preserves_planned_form_registration() {
        for form_first in [false, true] {
            let context = temp_context(if form_first {
                "batch-form-then-object"
            } else {
                "batch-object-then-form"
            });
            let cfg = context.cwd.join("src");
            let ext = context.cwd.join("ext");
            write_file(
                &cfg.join("Catalogs/Items.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties><Name>Items</Name></Properties>
		<ChildObjects><Form>MainForm</Form></ChildObjects>
	</Catalog>
</MetaDataObject>"#,
            );
            let mut plan = CfeBorrowWritePlan::default();
            let mut extension =
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration><Properties><Name>Extension</Name></Properties><ChildObjects/></Configuration>
</MetaDataObject>"#
                    .to_string();
            let mut log = CfeBorrowLog::default();

            cfe_borrow_object_shell(
                &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
                &cfg,
                &ext,
                &mut plan,
                "Catalog",
                "Items",
                "2.20",
                &mut extension,
                &mut log,
                crate::infrastructure::source_ids::fresh_uuid,
            )
            .unwrap();
            if form_first {
                cfe_borrow_register_form(&ext, &mut plan, "Catalog", "Items", "MainForm", &mut log)
                    .unwrap();
                cfe_borrow_object_shell(
                    &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
                    &cfg,
                    &ext,
                    &mut plan,
                    "Catalog",
                    "Items",
                    "2.20",
                    &mut extension,
                    &mut log,
                    crate::infrastructure::source_ids::fresh_uuid,
                )
                .unwrap();
            } else {
                cfe_borrow_register_form(&ext, &mut plan, "Catalog", "Items", "MainForm", &mut log)
                    .unwrap();
            }

            let object = plan.read_utf8_sig(&ext.join("Catalogs/Items.xml")).unwrap();
            assert_eq!(
                object.matches("<Form>MainForm</Form>").count(),
                1,
                "{object}"
            );
            assert_eq!(extension.matches("<Catalog>Items</Catalog>").count(), 1);
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_borrow_form_preserves_active_version_on_structured_and_fallback_paths() {
        for (path, source) in [
            (
                "structured",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" conversionversion="keep" version = '2.20'><AutoCommandBar/></Form>"#,
            ),
            (
                "fallback",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" conversionversion="keep" version = '2.20'></Form>"#,
            ),
        ] {
            let mut log = CfeBorrowLog::default();
            let generated = cfe_borrow_form_xml(
                &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
                source,
                Path::new("."),
                "Catalog",
                "Items",
                false,
                &mut log,
            );
            let document = Document::parse(&generated).unwrap();
            let root = document.root_element();
            assert_eq!(root.attribute("version"), Some("2.20"), "{path}");
            assert_eq!(
                root.attributes()
                    .filter(|attribute| attribute.name() == "version")
                    .count(),
                1,
                "{path}: {generated}"
            );
            assert_eq!(root.attribute("conversionversion"), Some("keep"), "{path}");
        }
    }

    #[test]
    fn cfe_borrow_form_adds_exact_required_namespace_prefixes() {
        let source = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:cfgExtra="urn:cfg-extra" version="2.20"><Attributes/></Form>"#;
        let mut log = CfeBorrowLog::default();

        let generated = cfe_borrow_form_xml(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
            source,
            Path::new("."),
            "Catalog",
            "Items",
            true,
            &mut log,
        );

        let document = Document::parse(&generated).expect("borrowed form must remain valid XML");
        let root = document.root_element();
        assert_eq!(
            root.lookup_namespace_uri(Some("v8")),
            Some("http://v8.1c.ru/8.1/data/core")
        );
        assert_eq!(
            root.lookup_namespace_uri(Some("cfg")),
            Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
        );
    }

    #[test]
    fn cfe_borrow_form_rebinds_required_prefixes_with_wrong_source_uris() {
        let source = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="urn:wrong-v8" xmlns:cfg="urn:wrong-cfg" version="2.20"><Attributes/></Form>"#;
        let mut log = CfeBorrowLog::default();

        let generated = cfe_borrow_form_xml(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
            source,
            Path::new("."),
            "Catalog",
            "Items",
            true,
            &mut log,
        );

        let document = Document::parse(&generated).expect("borrowed form must remain valid XML");
        let root = document.root_element();
        assert_eq!(
            root.lookup_namespace_uri(Some("v8")),
            Some("http://v8.1c.ru/8.1/data/core")
        );
        assert_eq!(
            root.lookup_namespace_uri(Some("cfg")),
            Some("http://v8.1c.ru/8.1/data/enterprise/current-config")
        );
    }

    #[test]
    fn borrow_cfe_post_write_failure_restores_form_files_and_owner_refs() {
        let context = temp_context("borrow-form-post-write-failure");
        let src = context.cwd.join("src");
        let ext = context.cwd.join("ext");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs/ParityCatalog/Forms/MainForm.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties><Name>MainForm</Name><FormType>Managed</FormType></Properties>
	</Form>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs/ParityCatalog/Forms/MainForm/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<Attributes/>
</Form>
"#,
        );
        let extension_owner = ext.join("Configuration.xml");
        write_file(
            &extension_owner,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ParityExtension</Name>
			<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>
			<NamePrefix>PE_</NamePrefix>
		</Properties>
		<ChildObjects><Catalog>ParityCatalog</Catalog></ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        let object_owner = ext.join("Catalogs/ParityCatalog.xml");
        write_file(
            &object_owner,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ParityCatalog</Name>
			<ExtendedConfigurationObject>11111111-1111-1111-1111-111111111111</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#,
        );
        let descriptor = ext.join("Catalogs/ParityCatalog/Forms/MainForm.xml");
        let form_xml = ext.join("Catalogs/ParityCatalog/Forms/MainForm/Ext/Form.xml");
        let module = ext.join("Catalogs/ParityCatalog/Forms/MainForm/Ext/Form/Module.bsl");
        let before = [
            (extension_owner.clone(), fs::read(&extension_owner).ok()),
            (object_owner.clone(), fs::read(&object_owner).ok()),
            (descriptor.clone(), fs::read(&descriptor).ok()),
            (form_xml.clone(), fs::read(&form_xml).ok()),
            (module.clone(), fs::read(&module).ok()),
        ];
        let args = Map::from_iter([
            ("ExtensionPath".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src")),
            (
                "Object".to_string(),
                json!("Catalog.ParityCatalog.Form.MainForm"),
            ),
        ]);

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            borrow_cfe(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "fixture must reach the transactional post-write gate: {outcome:?}"
        );
        for (path, expected) in before {
            assert_eq!(fs::read(&path).ok(), expected, "{}", path.display());
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_rejects_invalid_extension_semantics_and_rolls_back_every_planned_file() {
        let context = temp_context("borrow-semantic-validation");
        let src = context.cwd.join("src");
        let ext = context.cwd.join("ext");
        crate::infrastructure::test_fixtures::copy_empty_extension(&context.cwd);

        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs/Items.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties><Name>Items</Name></Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#,
        );

        let extension_owner = ext.join("Configuration.xml");
        let valid_owner = fs::read_to_string(&extension_owner).unwrap();
        let invalid_owner = valid_owner.replacen(
            "<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>",
            "<ConfigurationExtensionPurpose>Bogus</ConfigurationExtensionPurpose>",
            1,
        );
        assert_ne!(invalid_owner, valid_owner);
        fs::write(&extension_owner, invalid_owner.as_bytes()).unwrap();
        let owner_before = fs::read(&extension_owner).unwrap();
        let language_path = ext.join("Languages/Русский.xml");
        let language_before = fs::read(&language_path).unwrap();
        let target = ext.join("Catalogs/Items.xml");

        let outcome = borrow_cfe(
            &Map::from_iter([
                ("ExtensionPath".to_string(), json!("ext")),
                ("ConfigPath".to_string(), json!("src")),
                ("Object".to_string(), json!("Catalog.Items")),
            ]),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("ConfigurationExtensionPurpose"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&extension_owner).unwrap(), owner_before);
        assert_eq!(fs::read(&language_path).unwrap(), language_before);
        assert!(
            !target.exists(),
            "failed transaction left {}",
            target.display()
        );
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    pub(crate) fn borrow_cfe_rejects_concurrent_base_format_owner_change() {
        let context = temp_context("borrow-base-owner-guard");
        crate::infrastructure::test_fixtures::copy_empty_extension(&context.cwd);
        let base_owner = context.cwd.join("src/Configuration.xml");
        write_file(
            &base_owner,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        );
        write_file(
            &context.cwd.join("src/Catalogs/Items.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties><Name>Items</Name></Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#,
        );
        let extension_owner = context.cwd.join("ext/Configuration.xml");
        let extension_before = fs::read(&extension_owner).unwrap();
        let concurrent_base = fs::read_to_string(&base_owner)
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        let base_for_hook = base_owner.clone();
        let concurrent_for_hook = concurrent_base.clone();
        let args = Map::from_iter([
            ("ExtensionPath".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src")),
            ("Object".to_string(), json!("Catalog.Items")),
        ]);

        let outcome = with_before_commit_hook(
            move |_| fs::write(&base_for_hook, concurrent_for_hook).unwrap(),
            || borrow_cfe(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&base_owner).unwrap(), concurrent_base);
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// A first borrow creates the extension descriptor; only the extension
    /// owner it registers into is updated. Reporting both as `updated` would
    /// make the typed mutation state something the filesystem contradicts.
    #[test]
    fn borrow_cfe_reports_a_first_borrow_as_created_and_its_owner_as_updated() {
        let context = temp_context("borrow-mutation-created");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let target = context.cwd.join("ext/Catalogs/Items.xml");
        assert!(!target.exists());

        let execution = borrow_cfe_with_data(&minimal_borrow_args(), &context);
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{:?}", outcome.errors);
        let mutation = &execution
            .data
            .as_ref()
            .expect("cfe.borrow answers with data")
            .mutation;
        assert!(mutation.applied);
        let target_path = matching_mutation_path(&mutation.created, &target).unwrap_or_else(|| {
            panic!("a file that did not exist must be reported as created: {mutation:?}")
        });
        assert!(
            matching_mutation_path(&mutation.updated, &target).is_none(),
            "a created file must not also be reported as updated: {mutation:?}"
        );
        let owner_path = matching_mutation_path(&mutation.updated, &extension_owner)
            .unwrap_or_else(|| {
                panic!("the registered owner is rewritten, not created: {mutation:?}")
            });
        assert!(
            matching_mutation_path(&mutation.created, &extension_owner).is_none(),
            "an existing owner must not be reported as created: {mutation:?}"
        );
        assert!(
            outcome.changes.contains(&format!("created {target_path}")),
            "changes must agree with the typed mutation: {:?}",
            outcome.changes
        );
        assert!(
            outcome.changes.contains(&format!("updated {owner_path}")),
            "changes must agree with the typed mutation: {:?}",
            outcome.changes
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Borrowing again over an existing descriptor updates it. Nothing is
    /// created, and a path never appears in both lists.
    #[test]
    fn borrow_cfe_reports_a_repeated_borrow_as_updated_only() {
        let context = temp_context("borrow-mutation-updated");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", Some("2.20"));
        let target = context.cwd.join("ext/Catalogs/Items.xml");
        assert!(target.exists());

        let execution = borrow_cfe_with_data(&minimal_borrow_args(), &context);
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{:?}", outcome.errors);
        let mutation = &execution
            .data
            .as_ref()
            .expect("cfe.borrow answers with data")
            .mutation;
        assert!(
            mutation.created.is_empty(),
            "nothing is created on a repeated borrow: {mutation:?}"
        );
        matching_mutation_path(&mutation.updated, &extension_owner)
            .unwrap_or_else(|| panic!("the owner registration is rewritten: {mutation:?}"));
        for path in &mutation.created {
            assert!(
                matching_mutation_path(&mutation.updated, Path::new(path)).is_none(),
                "{path} must not be both created and updated"
            );
        }
        for change in &outcome.changes {
            assert!(
                change.starts_with("created ") || change.starts_with("updated "),
                "every change names its effect: {change}"
            );
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// #300 asks that an identical plan publish nothing. The property belongs
    /// to the write plan, so it is proven there: planning a file with exactly
    /// the bytes it already holds must leave the commit report empty.
    #[test]
    pub(crate) fn cfe_write_plan_identical_bytes_publish_no_created_or_updated_entry() {
        let context = temp_context("borrow-noop-plan");
        let owner = context.cwd.join("Configuration.xml");
        let image = "<Owner><State>stable</State></Owner>\n";
        write_file(&owner, &format!("\u{feff}{image}"));
        let before = fs::read(&owner).unwrap();

        let mut write_plan = CfeBorrowWritePlan::default();
        let text = write_plan.read_utf8_sig(&owner).unwrap();
        write_plan.write_utf8_bom(&owner, &text).unwrap();
        let report = write_plan.commit().expect("an identical plan commits");

        assert!(report.created.is_empty(), "{report:?}");
        assert!(report.updated.is_empty(), "{report:?}");
        assert_eq!(fs::read(&owner).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// With an unchanged parent, a descriptor keeps its identity, so
    /// borrowing the same object again leaves the descriptor byte-identical
    /// and publishes nothing — the write plan already reports an identical
    /// image as no change.
    ///
    /// This replaces `borrow_cfe_repeat_reports_the_rewrite_it_actually
    /// _performs`, which asserted the opposite while the regeneration it
    /// described was still open as #435: the report was truthful about a
    /// rewrite that should never have happened.
    #[test]
    pub(crate) fn borrow_cfe_preserves_object_and_file_identity_on_repeated_borrow() {
        use unica_workspace::test_support::file_identity_for_test;

        let context = temp_context("borrow-repeat-identity");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let target = context.cwd.join("ext/Catalogs/Items.xml");

        let first = borrow_cfe_with_data(&minimal_borrow_args(), &context);
        assert!(first.outcome.ok, "{:?}", first.outcome);
        let after_first = fs::read(&target).unwrap();
        let file_identity = file_identity_for_test(&target).unwrap();

        let second = borrow_cfe_with_data(&minimal_borrow_args(), &context);

        assert!(second.outcome.ok, "{:?}", second.outcome);
        assert_eq!(
            fs::read(&target).unwrap(),
            after_first,
            "a repeated borrow must not reissue the descriptor identity"
        );
        assert_eq!(file_identity_for_test(&target).unwrap(), file_identity);
        let mutation = &second
            .data
            .as_ref()
            .expect("cfe.borrow answers with data")
            .mutation;
        let target_path = target.display().to_string();
        assert!(!mutation.created.contains(&target_path), "{mutation:?}");
        assert!(!mutation.updated.contains(&target_path), "{mutation:?}");
        assert!(
            !second
                .outcome
                .changes
                .iter()
                .any(|change| change.ends_with(&target_path)),
            "an unchanged descriptor is named in no change: {:?}",
            second.outcome.changes
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Legacy behavior: borrowing accepts a damaged identifier pair.
    /// This test records the gap in #935/#931; the target behavior rejects
    /// the damaged descriptor without attempting to repair it.
    #[test]
    fn borrow_cfe_keeps_a_surviving_identifier_when_its_pair_is_missing() {
        for (kept, dropped) in [("TypeId", "ValueId"), ("ValueId", "TypeId")] {
            let context = temp_context(&format!("borrow-partial-{kept}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            let target = context.cwd.join("ext/Catalogs/Items.xml");

            let first = borrow_cfe_with_data(&minimal_borrow_args(), &context);
            assert!(first.outcome.ok, "{:?}", first.outcome);
            let after_first = fs::read_to_string(&target).unwrap();
            let survivor = after_first
                .split(&format!("<xr:{kept}>"))
                .nth(1)
                .and_then(|tail| tail.split(&format!("</xr:{kept}>")).next())
                .expect("the first borrow issued the identifier")
                .to_string();
            let damaged = after_first
                .lines()
                .filter(|line| !line.contains(&format!("<xr:{dropped}>")))
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(&target, format!("{damaged}\n")).unwrap();

            let second = borrow_cfe_with_data(&minimal_borrow_args(), &context);

            assert!(second.outcome.ok, "{:?}", second.outcome);
            let after_second = fs::read_to_string(&target).unwrap();
            assert!(
                after_second.contains(&format!("<xr:{kept}>{survivor}</xr:{kept}>")),
                "the surviving {kept} must not be reissued: {after_second}"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    /// A rollback publishes no successful mutation at all: the typed report
    /// belongs to a commit that happened.
    #[test]
    fn borrow_cfe_rolled_back_borrow_publishes_no_mutation() {
        let context = temp_context("borrow-mutation-rollback");
        write_minimal_borrow_fixture(&context, "2.20", "2.21", "2.20", None);

        let execution = borrow_cfe_with_data(&minimal_borrow_args(), &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(
            execution.data.is_none(),
            "a failed borrow publishes no mutation classification"
        );
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_rejects_newer_derived_source_xml_without_writes() {
        let context = temp_context("borrow-newer-derived-source");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.21", "2.20", None);
        let extension_before = fs::read(&extension_owner).unwrap();

        let outcome = borrow_cfe(&minimal_borrow_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("newer than supported 2.20"), "{error}");
        assert!(error.contains("1C 8.5 support is planned"), "{error}");
        assert!(!error.contains("re-export"), "{error}");
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_rejects_older_derived_source_xml_without_migrating() {
        let context = temp_context("borrow-older-derived-source");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.19", "2.20", None);
        let extension_before = fs::read(&extension_owner).unwrap();

        let outcome = borrow_cfe(&minimal_borrow_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("older than supported 2.20"), "{error}");
        assert!(
            error.contains("will not migrate it automatically"),
            "{error}"
        );
        assert!(error.contains("re-export"), "{error}");
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_mixed_older_extension_and_newer_source_prioritizes_newer() {
        let context = temp_context("borrow-mixed-newer-priority");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.21", "2.19", None);
        let extension_before = fs::read(&extension_owner).unwrap();

        let outcome = borrow_cfe(&minimal_borrow_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("newer than supported 2.20"), "{error}");
        assert!(!error.contains("re-export"), "{error}");
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn public_cfe_borrow_mixed_older_extension_and_newer_derived_source_prioritizes_newer() {
        let context = temp_context("public-borrow-mixed-newer-priority");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.21", "2.19", None);
        let extension_before = fs::read(&extension_owner).unwrap();
        let mut args = minimal_borrow_args();
        args.insert("cwd".to_string(), json!(context.cwd.display().to_string()));
        args.insert("dryRun".to_string(), json!(false));

        let result = UnicaApplication::new()
            .call_tool("unica.cfe.borrow", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = result.warnings.join("\n");
        assert!(warning.contains("1С 8.5"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("re-export"), "{warning}");
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_rejects_newer_existing_planned_xml_without_downgrade() {
        let context = temp_context("borrow-newer-existing-target");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", Some("2.21"));
        let target = context.cwd.join("ext/Catalogs/Items.xml");
        let target_before = fs::read(&target).unwrap();
        let extension_before = fs::read(&extension_owner).unwrap();

        let outcome = borrow_cfe(&minimal_borrow_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&target).unwrap(), target_before);
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_binds_exact_derived_source_bytes_before_commit() {
        let context = temp_context("borrow-derived-source-preimage");
        let (_, source_object, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let extension_before = fs::read(&extension_owner).unwrap();
        let concurrent_source = fs::read_to_string(&source_object)
            .unwrap()
            .replacen(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
                1,
            )
            .into_bytes();
        let source_for_hook = source_object.clone();
        let concurrent_for_hook = concurrent_source.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&source_for_hook, concurrent_for_hook).unwrap(),
            || borrow_cfe(&minimal_borrow_args(), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&source_object).unwrap(), concurrent_source);
        assert_eq!(fs::read(&extension_owner).unwrap(), extension_before);
        assert!(!context.cwd.join("ext/Catalogs/Items.xml").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_validates_source_semantics_before_planning_object_files() {
        let context = temp_context("borrow-source-semantic-preflight");
        crate::infrastructure::test_fixtures::copy_empty_extension(&context.cwd);
        write_file(
            &context.cwd.join("src/Configuration.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        );

        let extension_owner = context.cwd.join("ext/Configuration.xml");
        let valid_owner = fs::read_to_string(&extension_owner).unwrap();
        let invalid_owner = valid_owner.replacen(
            "<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>",
            "<ConfigurationExtensionPurpose>Unsupported</ConfigurationExtensionPurpose>",
            1,
        );
        fs::write(&extension_owner, invalid_owner.as_bytes()).unwrap();
        let owner_before = fs::read(&extension_owner).unwrap();

        let outcome = borrow_cfe(
            &Map::from_iter([
                ("ExtensionPath".to_string(), json!("ext")),
                ("ConfigPath".to_string(), json!("src")),
                ("Object".to_string(), json!("Catalog.Missing")),
            ]),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("ConfigurationExtensionPurpose"),
            "source semantics must be checked before the missing object: {outcome:?}"
        );
        assert_eq!(fs::read(&extension_owner).unwrap(), owner_before);
        assert!(!context.cwd.join("ext/Catalogs").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_borrow_plan_rejects_concurrent_owner_change() {
        let context = temp_context("borrow-concurrent-owner-change");
        let owner = context.cwd.join("Configuration.xml");
        let original = b"\xef\xbb\xbf<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Owner><State>original</State></Owner>\r\n";
        let concurrent = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Owner><State>concurrent</State></Owner>\n";
        fs::write(&owner, original).unwrap();
        let mut write_plan = CfeBorrowWritePlan::default();
        let owner_text = write_plan.read_utf8_sig(&owner).unwrap();
        write_plan
            .write_utf8_bom(
                &owner,
                &owner_text.replace("original", "planned replacement"),
            )
            .unwrap();
        let owner_for_hook = owner.clone();

        let result = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, concurrent).unwrap(),
            || write_plan.commit(),
        );

        let error = result.expect_err("concurrent owner edit must reject the plan");
        assert!(error.contains("changed"), "{error}");
        assert_eq!(fs::read(&owner).unwrap(), concurrent);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_borrow_plan_keeps_unchanged_owner_as_a_locked_preimage() {
        let context = temp_context("borrow-unchanged-owner-change");
        let owner = context.cwd.join("Configuration.xml");
        let created = context.cwd.join("Catalogs/Items.xml");
        let original = b"\xef\xbb\xbf<Owner><State>original</State></Owner>\n";
        let concurrent = b"<Owner><State>concurrent</State></Owner>\n";
        fs::write(&owner, original).unwrap();
        let mut write_plan = CfeBorrowWritePlan::default();
        write_plan.read_utf8_sig(&owner).unwrap();
        write_plan
            .write_utf8_bom(&created, "<Created><State>planned</State></Created>\n")
            .unwrap();
        let owner_for_hook = owner.clone();

        let result = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, concurrent).unwrap(),
            || write_plan.commit(),
        );

        let error = result.expect_err("unchanged owner preimage must remain authoritative");
        assert!(error.contains("post-write byte validation"), "{error}");
        assert_eq!(fs::read(&owner).unwrap(), concurrent);
        assert!(
            !created.exists(),
            "failed transaction must remove its create"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_preserves_existing_form_module_on_repeated_form_borrow() {
        let context = temp_context("preserve-form-module");
        let src = context.cwd.join("src");
        let ext = context.cwd.join("ext");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<NamePrefix/>
		</Properties>
		<ChildObjects>
			<Catalog>ParityCatalog</Catalog>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("ParityCatalog")
                .join("Forms")
                .join("MainForm.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<Properties>
			<Name>MainForm</Name>
			<FormType>Managed</FormType>
		</Properties>
	</Form>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("ParityCatalog")
                .join("Forms")
                .join("MainForm")
                .join("Ext")
                .join("Form.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Attributes/>
</Form>
"#,
        );
        write_file(
            &ext.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ParityExtension</Name>
			<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>
			<NamePrefix>PE_</NamePrefix>
		</Properties>
		<ChildObjects>
			<Catalog>ParityCatalog</Catalog>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &ext.join("Catalogs").join("ParityCatalog.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ParityCatalog</Name>
			<ExtendedConfigurationObject>11111111-1111-1111-1111-111111111111</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects>
			<Form>MainForm</Form>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        let form_meta_path = ext
            .join("Catalogs")
            .join("ParityCatalog")
            .join("Forms")
            .join("MainForm.xml");
        let existing_form_meta_uuid = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        write_file(
            &form_meta_path,
            &format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="{existing_form_meta_uuid}">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>MainForm</Name>
			<ExtendedConfigurationObject>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa</ExtendedConfigurationObject>
			<FormType>Managed</FormType>
		</Properties>
	</Form>
</MetaDataObject>
"#
            ),
        );
        let module_path = ext
            .join("Catalogs")
            .join("ParityCatalog")
            .join("Forms")
            .join("MainForm")
            .join("Ext")
            .join("Form")
            .join("Module.bsl");
        let existing_module = "Procedure ExistingHandler()\nEndProcedure\n";
        write_file(&module_path, existing_module);

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        args.insert("ConfigPath".to_string(), json!("src"));
        args.insert(
            "Object".to_string(),
            json!("Catalog.ParityCatalog.Form.MainForm"),
        );
        args.insert("BorrowMainAttribute".to_string(), json!("Form"));

        let execution = borrow_cfe_with_data(&args, &context);
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(fs::read_to_string(&module_path).unwrap(), existing_module);
        assert!(
            fs::read_to_string(&form_meta_path)
                .unwrap()
                .contains(existing_form_meta_uuid),
            "existing form metadata uuid must survive re-borrow"
        );
        let data = execution
            .data
            .as_ref()
            .expect("cfe.borrow answers with data");
        assert!(
            data.skipped.iter().any(|item| {
                item.target == module_path.display().to_string()
                    && item.reason.contains("не перезаписан")
            }),
            "the preserved module must be reported as skipped: {:?}",
            data.skipped
        );
        let module_artifact = module_path.display().to_string();
        assert!(!outcome.artifacts.contains(&module_artifact));
        assert!(!outcome
            .changes
            .iter()
            .any(|change| change.contains(&module_artifact)));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    fn patch_method_args() -> Map<String, Value> {
        Map::from_iter([
            ("ExtensionPath".to_string(), json!("ext")),
            (
                "ModulePath".to_string(),
                json!("CommonModule.GuardedModule"),
            ),
            ("MethodName".to_string(), json!("Run")),
            ("InterceptorType".to_string(), json!("Before")),
        ])
    }

    fn assert_extended_property_state(path: &Path, expected_property: &str) {
        let xml = fs::read_to_string(path).unwrap();
        let document = Document::parse(xml.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|error| panic!("{}: {error}: {xml}", path.display()));
        let states = document
            .descendants()
            .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "PropertyState")))
            .filter_map(|state| {
                let property = state
                    .children()
                    .find(|node| {
                        node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "Property"))
                    })
                    .and_then(|node| node.text())?;
                let value = state
                    .children()
                    .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "State")))
                    .and_then(|node| node.text())?;
                Some((property, value))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            [(expected_property, "Extended")],
            "{}: {xml}",
            path.display()
        );
    }

    fn register_borrowed_patch_object(
        context: &WorkspaceContext,
        type_name: &str,
        object_name: &str,
        child_objects: &str,
    ) -> PathBuf {
        let extension_owner = context.cwd.join("ext/Configuration.xml");
        let owner = fs::read_to_string(&extension_owner).unwrap();
        let registered = owner.replacen(
            "<ChildObjects/>",
            &format!(
                "<ChildObjects>\n\t\t\t<{type_name}>{object_name}</{type_name}>\n\t\t</ChildObjects>"
            ),
            1,
        );
        fs::write(&extension_owner, registered).unwrap();
        let dir_name = cf_validate_child_type_dir(type_name).unwrap();
        let descriptor = context
            .cwd
            .join("ext")
            .join(dir_name)
            .join(format!("{object_name}.xml"));
        write_file(
            &descriptor,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<{type_name} uuid="77777777-7777-7777-7777-777777777777">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>{object_name}</Name>
			<Comment/>
			<ExtendedConfigurationObject>88888888-8888-8888-8888-888888888888</ExtendedConfigurationObject>
			{common_module_properties}
		</Properties>
		{child_objects}
	</{type_name}>
</MetaDataObject>
"#,
                common_module_properties = if type_name == "CommonModule" {
                    "<Global>false</Global>\n\t\t\t<ClientManagedApplication>false</ClientManagedApplication>\n\t\t\t<Server>true</Server>\n\t\t\t<ExternalConnection>false</ExternalConnection>\n\t\t\t<ClientOrdinaryApplication>false</ClientOrdinaryApplication>\n\t\t\t<ServerCall>false</ServerCall>\n\t\t\t<ReturnValuesReuse>DontUse</ReturnValuesReuse>"
                } else {
                    ""
                },
            ),
        );
        descriptor
    }

    fn register_borrowed_patch_form(
        context: &WorkspaceContext,
        type_name: &str,
        object_name: &str,
        form_name: &str,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let descriptor = register_borrowed_patch_object(
            context,
            type_name,
            object_name,
            &format!("<ChildObjects><Form>{form_name}</Form></ChildObjects>"),
        );
        let dir_name = cf_validate_child_type_dir(type_name).unwrap();
        let form_dir = context
            .cwd
            .join("ext")
            .join(dir_name)
            .join(object_name)
            .join("Forms");
        let wrapper = form_dir.join(format!("{form_name}.xml"));
        write_file(
            &wrapper,
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="99999999-9999-9999-9999-999999999999">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>{form_name}</Name>
			<Comment/>
			<ExtendedConfigurationObject>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa</ExtendedConfigurationObject>
			<FormType>Managed</FormType>
		</Properties>
	</Form>
</MetaDataObject>
"#
            ),
        );
        let form_xml = form_dir.join(form_name).join("Ext/Form.xml");
        write_file(
            &form_xml,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<Attributes/>
	<BaseForm version="2.20">
		<Attributes/>
	</BaseForm>
</Form>
"#,
        );
        (descriptor, wrapper, form_xml)
    }

    #[test]
    fn borrow_cfe_enriches_main_attribute_paths_and_reference_shells() {
        let context = temp_context("borrow-main-attributes");
        let src = context.cwd.join("src");
        let ext = context.cwd.join("ext");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<NamePrefix/>
		</Properties>
		<ChildObjects>
			<Catalog>Orders</Catalog>
			<Catalog>Counterparty</Catalog>
			<Catalog>Products</Catalog>
			<DefinedType>StatusType</DefinedType>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs").join("Orders.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<InternalInfo/>
		<Properties>
			<Name>Orders</Name>
		</Properties>
		<ChildObjects>
			<Attribute uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
				<Properties>
					<Name>Customer</Name>
					<Type><v8:Type>cfg:CatalogRef.Counterparty</v8:Type></Type>
				</Properties>
			</Attribute>
			<Attribute uuid="cccccccc-cccc-cccc-cccc-cccccccccccc">
				<Properties>
					<Name>Agreement</Name>
					<Type><v8:Type>cfg:DefinedType.StatusType</v8:Type></Type>
				</Properties>
			</Attribute>
			<TabularSection uuid="dddddddd-dddd-dddd-dddd-dddddddddddd">
				<InternalInfo>
					<xr:GeneratedType name="CatalogTabularSection.Orders.Items" category="TabularSection">
						<xr:TypeId>11111111-1111-1111-1111-111111111111</xr:TypeId>
						<xr:ValueId>22222222-2222-2222-2222-222222222222</xr:ValueId>
					</xr:GeneratedType>
				</InternalInfo>
				<Properties>
					<Name>Items</Name>
				</Properties>
				<ChildObjects>
					<Attribute uuid="eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee">
						<Properties>
							<Name>Product</Name>
							<Type><v8:Type>cfg:CatalogRef.Products</v8:Type></Type>
						</Properties>
					</Attribute>
					<Attribute uuid="ffffffff-ffff-ffff-ffff-ffffffffffff">
						<Properties>
							<Name>Quantity</Name>
							<Type><v8:Type>xs:decimal</v8:Type></Type>
						</Properties>
					</Attribute>
				</ChildObjects>
			</TabularSection>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs").join("Counterparty.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Catalog uuid="12345678-1234-1234-1234-123456789abc">
		<Properties><Name>Counterparty</Name></Properties>
		<ChildObjects>
			<Attribute uuid="11111111-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
				<Properties>
					<Name>TaxId</Name>
					<Type><v8:Type>xs:string</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs").join("Products.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Catalog uuid="87654321-4321-4321-4321-cba987654321">
		<Properties><Name>Products</Name></Properties>
		<ChildObjects>
			<Attribute uuid="22222222-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
				<Properties>
					<Name>Sku</Name>
					<Type><v8:Type>xs:string</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("DefinedTypes").join("StatusType.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<DefinedType uuid="99999999-9999-9999-9999-999999999999">
		<Properties>
			<Name>StatusType</Name>
			<Type><v8:Type>xs:string</v8:Type></Type>
		</Properties>
	</DefinedType>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("Orders")
                .join("Forms")
                .join("MainForm.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa">
		<Properties><Name>MainForm</Name></Properties>
	</Form>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("Orders")
                .join("Forms")
                .join("MainForm")
                .join("Ext")
                .join("Form.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<ChildItems>
		<InputField name="CustomerTaxId" id="1000001"><DataPath>Объект.Customer.TaxId</DataPath></InputField>
		<InputField name="ProductSku" id="1000002"><DataPath>Объект.Items.Product.Sku</DataPath></InputField>
		<InputField name="Quantity" id="1000003"><DataPath>Объект.Items.Quantity</DataPath></InputField>
		<CommandBar name="AgreementCommand" id="1000004"><Field>Объект.Agreement</Field></CommandBar>
	</ChildItems>
	<Attributes/>
</Form>
"#,
        );
        write_file(
            &ext.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ParityExtension</Name>
			<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>
			<NamePrefix>PE_</NamePrefix>
		</Properties>
		<ChildObjects/>
	</Configuration>
</MetaDataObject>
"#,
        );

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        args.insert("ConfigPath".to_string(), json!("src"));
        args.insert("Object".to_string(), json!("Catalog.Orders.Form.MainForm"));
        args.insert("BorrowMainAttribute".to_string(), json!("Form"));

        let outcome = borrow_cfe(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        for path in [
            ext.join("Catalogs/Orders/Forms/MainForm.xml"),
            ext.join("Catalogs/Orders/Forms/MainForm/Ext/Form.xml"),
        ] {
            let generated = fs::read_to_string(path).unwrap();
            assert!(generated.contains(r#"version="2.20""#), "{generated}");
            assert!(!generated.contains(r#"version="2.17""#), "{generated}");
        }
        assert!(
            !ext.join("Catalogs/Orders/Forms/MainForm/Ext/Form/Module.bsl")
                .exists(),
            "8.3.27 omits an absent empty borrowed form module"
        );
        let order_xml = fs::read_to_string(ext.join("Catalogs").join("Orders.xml")).unwrap();
        for expected in [
            "<Name>Customer</Name>",
            "<Name>Agreement</Name>",
            "<Name>Items</Name>",
            "<Name>Product</Name>",
            "<Name>Quantity</Name>",
        ] {
            assert!(
                order_xml.contains(expected),
                "missing {expected} in:\n{order_xml}"
            );
        }
        assert!(
            order_xml.contains("cfg:DefinedType.StatusType"),
            "{order_xml}"
        );

        let counterparty_xml =
            fs::read_to_string(ext.join("Catalogs").join("Counterparty.xml")).unwrap();
        assert!(
            counterparty_xml.contains("<Name>TaxId</Name>"),
            "{counterparty_xml}"
        );
        let products_xml = fs::read_to_string(ext.join("Catalogs").join("Products.xml")).unwrap();
        assert!(products_xml.contains("<Name>Sku</Name>"), "{products_xml}");
        let defined_type_xml =
            fs::read_to_string(ext.join("DefinedTypes").join("StatusType.xml")).unwrap();
        assert!(
            defined_type_xml.contains("<Type><v8:Type>xs:string</v8:Type></Type>"),
            "{defined_type_xml}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn borrow_cfe_enriches_existing_adopted_parent_from_form_paths() {
        let context = temp_context("borrow-existing-parent-main-attributes");
        let src = context.cwd.join("src");
        let ext = context.cwd.join("ext");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<NamePrefix/>
		</Properties>
		<ChildObjects>
			<Catalog>Orders</Catalog>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs").join("Orders.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Catalog uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
		<InternalInfo/>
		<Properties>
			<Name>Orders</Name>
		</Properties>
		<ChildObjects>
			<Attribute uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb">
				<Properties>
					<Name>Customer</Name>
					<Type><v8:Type>xs:string</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("Orders")
                .join("Forms")
                .join("MainForm.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Form uuid="aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa">
		<Properties><Name>MainForm</Name></Properties>
	</Form>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Catalogs")
                .join("Orders")
                .join("Forms")
                .join("MainForm")
                .join("Ext")
                .join("Form.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<ChildItems>
		<InputField name="CustomerField" id="1000001"><DataPath>Объект.Customer</DataPath></InputField>
	</ChildItems>
	<Attributes/>
</Form>
"#,
        );
        write_file(
            &ext.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ParityExtension</Name>
			<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>
			<NamePrefix>PE_</NamePrefix>
		</Properties>
		<ChildObjects>
			<Catalog>Orders</Catalog>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &ext.join("Catalogs").join("Orders.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Catalog uuid="77777777-7777-7777-7777-777777777777">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Orders</Name>
			<Comment/>
			<ExtendedConfigurationObject>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects>
			<Attribute uuid="88888888-8888-8888-8888-888888888888">
				<InternalInfo/>
				<Properties>
					<Name>LocalExtensionFlag</Name>
					<Type><v8:Type>xs:boolean</v8:Type></Type>
				</Properties>
			</Attribute>
		</ChildObjects>
	</Catalog>
</MetaDataObject>
"#,
        );

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        args.insert("ConfigPath".to_string(), json!("src"));
        args.insert("Object".to_string(), json!("Catalog.Orders.Form.MainForm"));
        args.insert("BorrowMainAttribute".to_string(), json!("Form"));

        let outcome = borrow_cfe(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let order_xml = fs::read_to_string(ext.join("Catalogs").join("Orders.xml")).unwrap();
        assert!(
            order_xml.contains("<Name>LocalExtensionFlag</Name>"),
            "{order_xml}"
        );
        assert!(order_xml.contains("<Name>Customer</Name>"), "{order_xml}");
        assert!(
            order_xml.contains("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            "{order_xml}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    // Fixture shape mirrors a real 8.3.27 / format-2.20 Designer dump of an
    // extension that adds its own HTTPService (the PrintWebDAV applied case).
    fn write_own_http_service_extension(context: &WorkspaceContext, methods: &[(&str, &str)]) {
        let ext = context.cwd.join("ext");
        write_file(
            &ext.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Configuration uuid="00000000-0000-0000-0000-000000000014">
		<InternalInfo>
			<xr:ContainedObject>
				<xr:ClassId>9cd510cd-abfc-11d4-9434-004095e12fc7</xr:ClassId>
				<xr:ObjectId>00000000-0000-0000-0000-000000000017</xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>9fcd25a0-4822-11d4-9414-008048da11f9</xr:ClassId>
				<xr:ObjectId>00000000-0000-0000-0000-000000000018</xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>e3687481-0a87-462c-a166-9f34594f9bba</xr:ClassId>
				<xr:ObjectId>00000000-0000-0000-0000-000000000019</xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>9de14907-ec23-4a07-96f0-85521cb6b53b</xr:ClassId>
				<xr:ObjectId>00000000-0000-0000-0000-00000000001a</xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>51f2d5d8-ea4d-4064-8892-82951750031e</xr:ClassId>
				<xr:ObjectId>00000000-0000-0000-0000-00000000001b</xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>e68182ea-4237-4383-967f-90c1e3370bc7</xr:ClassId>
				<xr:ObjectId>00000000-0000-0000-0000-00000000001c</xr:ObjectId>
			</xr:ContainedObject>
			<xr:ContainedObject>
				<xr:ClassId>fb282519-d103-4dd3-bc12-cb271d631dfc</xr:ClassId>
				<xr:ObjectId>00000000-0000-0000-0000-00000000001d</xr:ObjectId>
			</xr:ContainedObject>
		</InternalInfo>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>ПечатьWebDAV</Name>
			<Comment/>
			<ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose>
			<KeepMappingToExtendedConfigurationObjectsByIDs>true</KeepMappingToExtendedConfigurationObjectsByIDs>
			<NamePrefix>ПВД_</NamePrefix>
			<DefaultLanguage>Language.Русский</DefaultLanguage>
		</Properties>
		<ChildObjects>
			<Language>Русский</Language>
			<HTTPService>ПВД_ПечатныеФормы</HTTPService>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &ext.join("Languages/Русский.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Language uuid="00000000-0000-0000-0000-000000000015">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Русский</Name>
			<Comment/>
			<ExtendedConfigurationObject>0663bf5b-bcba-4a40-a862-a0b3baa2d884</ExtendedConfigurationObject>
			<LanguageCode>ru</LanguageCode>
		</Properties>
	</Language>
</MetaDataObject>
"#,
        );
        let methods_xml = methods
            .iter()
            .enumerate()
            .map(|(index, (name, literal))| {
                format!(
                    r#"					<Method uuid="7cdf45cd-1cc4-43e7-bfee-2b3f39140{index:03}">
						<Properties>
							<Name>{name}</Name>
							<Comment/>
							<HTTPMethod>{literal}</HTTPMethod>
							<Handler>Корень{name}</Handler>
						</Properties>
					</Method>
"#
                )
            })
            .collect::<String>();
        write_file(
            &ext.join("HTTPServices/ПВД_ПечатныеФормы.xml"),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<HTTPService uuid="a6eb1e93-3b7f-42f1-9e98-0a2df2487ef1">
		<Properties>
			<Name>ПВД_ПечатныеФормы</Name>
			<Comment/>
			<RootURL>printforms</RootURL>
			<ReuseSessions>DontUse</ReuseSessions>
			<SessionMaxAge>20</SessionMaxAge>
		</Properties>
		<ChildObjects>
			<URLTemplate uuid="866986d4-1d28-447c-bb56-f750510e6bec">
				<Properties>
					<Name>Корень</Name>
					<Comment/>
					<Template>/*</Template>
				</Properties>
				<ChildObjects>
{methods_xml}				</ChildObjects>
			</URLTemplate>
		</ChildObjects>
	</HTTPService>
</MetaDataObject>
"#
            ),
        );
    }

    #[test]
    fn cfe_validate_reports_invalid_http_method_enum_in_own_http_service() {
        let context = temp_context("http-method-enum-invalid");
        write_own_http_service_extension(&context, &[("ANY", "ANY")]);

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        let outcome = validate_cfe(&args, &context);

        assert!(
            !outcome.ok,
            "extension with HTTPMethod 'ANY' must fail validation, stdout: {:?}",
            outcome.stdout
        );
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("invalid HTTPMethod 'ANY'")),
            "{:?}",
            outcome.errors
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_warns_on_declared_service_without_readable_descriptor() {
        let context = temp_context("http-service-unreadable");
        write_own_http_service_extension(&context, &[("Чтение", "GET")]);
        fs::remove_file(context.cwd.join("ext/HTTPServices/ПВД_ПечатныеФормы.xml")).unwrap();

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        args.insert("Detailed".to_string(), json!(true));
        let outcome = validate_cfe(&args, &context);

        let stdout = outcome.stdout.unwrap_or_default();
        assert!(
            stdout.contains(
                "14. HTTPService.ПВД_ПечатныеФормы: cannot read HTTPServices/ПВД_ПечатныеФормы.xml"
            ),
            "unreadable service descriptor must be reported: {stdout}"
        );
        assert!(
            !stdout.contains("14. Services: none found"),
            "a declared but unreadable service is not an absent service: {stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_reports_invalid_transfer_direction_in_own_web_service() {
        let context = temp_context("web-service-direction");
        write_own_http_service_extension(&context, &[("Чтение", "GET")]);
        let ext = context.cwd.join("ext");
        let config = fs::read_to_string(ext.join("Configuration.xml")).unwrap();
        fs::write(
            ext.join("Configuration.xml"),
            config.replace(
                "<HTTPService>ПВД_ПечатныеФормы</HTTPService>",
                "<WebService>ПВД_Обмен</WebService>\n\t\t\t<HTTPService>ПВД_ПечатныеФормы</HTTPService>",
            ),
        )
        .unwrap();
        write_file(
            &ext.join("WebServices/ПВД_Обмен.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
	<WebService uuid="b58f7d3a-40b6-4e58-9a2e-6f60cc1f8a01">
		<Properties>
			<Name>ПВД_Обмен</Name>
			<Comment/>
		</Properties>
		<ChildObjects>
			<Operation uuid="c58f7d3a-40b6-4e58-9a2e-6f60cc1f8a02">
				<Properties>
					<Name>Пинг</Name>
					<XDTOReturningValueType>xs:boolean</XDTOReturningValueType>
					<ProcedureName>Пинг</ProcedureName>
				</Properties>
				<ChildObjects>
					<Parameter uuid="d58f7d3a-40b6-4e58-9a2e-6f60cc1f8a03">
						<Properties>
							<Name>Данные</Name>
							<TransferDirection>Inward</TransferDirection>
						</Properties>
					</Parameter>
				</ChildObjects>
			</Operation>
		</ChildObjects>
	</WebService>
</MetaDataObject>
"#,
        );

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        let outcome = validate_cfe(&args, &context);

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(
            outcome.errors.iter().any(|error| error.contains(
                "14. WebService.ПВД_Обмен Operation 'Пинг': Parameter has invalid TransferDirection 'Inward'"
            )),
            "{:?}",
            outcome.errors
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_validate_accepts_the_full_8_3_27_http_method_enum() {
        let context = temp_context("http-method-enum-valid");
        write_own_http_service_extension(
            &context,
            &[("Любой", "Any"), ("Обход", "PROPFIND"), ("Чтение", "GET")],
        );

        let mut args = Map::new();
        args.insert("ExtensionPath".to_string(), json!("ext"));
        args.insert("Detailed".to_string(), json!(true));
        let outcome = validate_cfe(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        let stdout = outcome.stdout.unwrap_or_default();
        assert!(
            stdout.contains("14. HTTPService.ПВД_ПечатныеФормы: 1 URLTemplate(s), 3 method(s)"),
            "service check line missing from detailed output: {stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }
}
