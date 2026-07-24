#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_owner::root_version_literal;
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

use super::common::*;
use super::compile_transaction::{CommitReport, CompileTransaction};
use super::{
    cf::*, dcs::*, form::*, interface::*, meta::*, mxl::*, role::*, subsystem::*, template::*,
};
pub(crate) struct CfeValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) obj_name: String,
}

#[cfg(test)]
thread_local! {
    static CFE_INIT_AFTER_BASE_READ_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
    static CFE_PATCH_AFTER_BORROWED_READ_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
fn with_cfe_init_after_base_read_hook<T>(
    hook: impl FnOnce() + 'static,
    run: impl FnOnce() -> T,
) -> T {
    CFE_INIT_AFTER_BASE_READ_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    let result = run();
    CFE_INIT_AFTER_BASE_READ_HOOK.with(|slot| {
        slot.borrow_mut().take();
    });
    result
}

#[cfg(test)]
fn run_cfe_init_after_base_read_hook() {
    CFE_INIT_AFTER_BASE_READ_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_cfe_init_after_base_read_hook() {}

#[cfg(test)]
fn with_cfe_patch_after_borrowed_read_hook<T>(
    hook: impl FnOnce() + 'static,
    run: impl FnOnce() -> T,
) -> T {
    CFE_PATCH_AFTER_BORROWED_READ_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    let result = run();
    CFE_PATCH_AFTER_BORROWED_READ_HOOK.with(|slot| {
        slot.borrow_mut().take();
    });
    result
}

#[cfg(test)]
fn run_cfe_patch_after_borrowed_read_hook() {
    CFE_PATCH_AFTER_BORROWED_READ_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_cfe_patch_after_borrowed_read_hook() {}

pub(crate) struct CfeValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) out_file: Option<PathBuf>,
    pub(crate) artifact: PathBuf,
    pub(crate) errors: Vec<String>,
}

pub(crate) struct CfeDiffObject {
    pub(crate) obj_type: String,
    pub(crate) name: String,
}

pub(crate) struct CfeDiffObjectInfo {
    pub(crate) borrowed: bool,
    pub(crate) exists: bool,
    pub(crate) dir_name: String,
    pub(crate) attrs: usize,
    pub(crate) forms: usize,
    pub(crate) tabular_sections: usize,
    pub(crate) borrowed_items: usize,
    pub(crate) form_names: Vec<String>,
}

pub(crate) struct CfeDiffInterceptor {
    pub(crate) interceptor_type: String,
    pub(crate) method: String,
    pub(crate) line: usize,
}

pub(crate) struct CfeDiffInsertionBlock {
    pub(crate) code: String,
}

impl CfeValidationReporter {
    pub(crate) fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: Vec::new(),
            obj_name: "(unknown)".to_string(),
        }
    }

    pub(crate) fn out(&mut self, message: impl Into<String>) {
        self.lines.push(message.into());
    }

    pub(crate) fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub(crate) fn finalize(mut self) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!(
                    "=== Validation OK: Extension.{} ({checks} checks) ===\n",
                    self.obj_name
                ),
                Vec::new(),
            );
        }
        self.lines.push(String::new());
        self.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            self.errors, self.warnings
        ));
        let errors = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[ERROR]"))
            .cloned()
            .collect::<Vec<_>>();
        (ok, format!("{}\n", self.lines.join("\n")), errors)
    }
}

fn cfe_registered_xml_dependency_paths_with_reader<F>(
    config_path: &Path,
    read: &mut F,
) -> Result<Vec<PathBuf>, String>
where
    F: FnMut(&Path) -> Result<Option<String>, String>,
{
    let mut paths = vec![config_path.to_path_buf()];
    let Some(config_text) = read(config_path)? else {
        return Ok(paths);
    };
    let config_document = Document::parse(&config_text)
        .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
    let registered = config_document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Configuration")
        .and_then(|configuration| meta_info_child(configuration, "ChildObjects"))
        .map(|children| {
            children
                .children()
                .filter(|child| child.is_element())
                .map(|child| {
                    (
                        child.tag_name().name().to_string(),
                        child.text().unwrap_or("").to_string(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    drop(config_document);

    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    for (type_name, object_name) in registered {
        if object_name.is_empty() {
            continue;
        }
        if type_name == "Language" {
            let language_path = config_dir
                .join("Languages")
                .join(format!("{object_name}.xml"));
            paths.push(language_path.clone());
            let _ = read(&language_path);
            continue;
        }
        let Some(type_dir) = cf_validate_child_type_dir(&type_name) else {
            continue;
        };
        let object_path = config_dir.join(type_dir).join(format!("{object_name}.xml"));
        paths.push(object_path.clone());
        let Some(object_text) = read(&object_path).ok().flatten() else {
            continue;
        };
        let Ok(object_document) = Document::parse(&object_text) else {
            continue;
        };
        let forms = object_document
            .root_element()
            .children()
            .find(|node| node.is_element())
            .and_then(|object| meta_info_child(object, "ChildObjects"))
            .map(|children| {
                meta_info_children(children, "Form")
                    .into_iter()
                    .filter_map(|form| form.text().map(ToOwned::to_owned))
                    .filter(|name| !name.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        drop(object_document);
        for form_name in forms {
            let form_dir = config_dir.join(type_dir).join(&object_name).join("Forms");
            let wrapper = form_dir.join(format!("{form_name}.xml"));
            paths.push(wrapper.clone());
            let _ = read(&wrapper);
            let form_xml = form_dir.join(&form_name).join("Ext").join("Form.xml");
            paths.push(form_xml.clone());
            let _ = read(&form_xml);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

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
    stdout: String,
    artifacts: Vec<PathBuf>,
    borrowed_count: usize,
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
    let object_spec = required_string(args, &["object", "Object"], "Object")?;
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
    let mut ext_text = write_plan.read_utf8_sig(&ext_path)?;
    let mut registered_format_dependencies =
        cfe_registered_xml_dependency_paths_with_reader(&ext_path, &mut |path| {
            write_plan.read_current_or_dependency_utf8_sig(path)
        })?;
    let ext_doc =
        Document::parse(&ext_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let ext_cfg = ext_doc
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Configuration")
        .ok_or_else(|| "No <Configuration> element found in extension".to_string())?;
    let props_el = meta_info_child(ext_cfg, "Properties")
        .ok_or_else(|| "No <Properties> element found in extension".to_string())?;
    if meta_info_child(ext_cfg, "ChildObjects").is_none() {
        return Err("No <ChildObjects> element found in extension".to_string());
    }
    let name_prefix = meta_info_child_text(props_el, "NamePrefix").unwrap_or_default();
    let format_version = ext_doc
        .root_element()
        .attribute("version")
        .unwrap_or(ACTIVE_FORMAT_PROFILE.export_format)
        .to_string();

    let items = object_spec
        .split(";;")
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Err("No objects specified in -Object".to_string());
    }
    if let Some(mode) = &borrow_main_attribute {
        if !matches!(mode.as_str(), "Form" | "All") {
            return Err("-BorrowMainAttribute accepts 'Form' or 'All' (default: Form)".to_string());
        }
        if !items.iter().any(|item| item.contains(".Form.")) {
            return Err(
                "-BorrowMainAttribute requires a form in -Object (e.g. 'Catalog.X.Form.Y')"
                    .to_string(),
            );
        }
    }

    let mut stdout = format!("[INFO] Extension NamePrefix: {name_prefix}\n");
    let mut artifacts = Vec::<PathBuf>::new();
    let mut borrowed_count = 0usize;
    for item in &items {
        let spec = cfe_borrow_parse_object_spec(item)?;
        if spec.form_name.is_some() {
            stdout.push_str(&format!(
                "[INFO] Borrowing form {}.{}.Form.{}...\n",
                spec.type_name,
                spec.object_name,
                spec.form_name.as_deref().unwrap_or_default()
            ));
            if !write_plan.exists(&cfe_borrow_target_object(
                &ext_dir,
                &spec.type_name,
                &spec.object_name,
            )) {
                stdout.push_str(&format!(
                    "[INFO]   Parent object {}.{} not yet borrowed — borrowing first...\n",
                    spec.type_name, spec.object_name
                ));
                let object_artifact = cfe_borrow_object_shell(
                    &cfg_dir,
                    &ext_dir,
                    &mut write_plan,
                    &spec.type_name,
                    &spec.object_name,
                    &format_version,
                    &mut ext_text,
                    &mut stdout,
                )?;
                artifacts.push(object_artifact);
            }
            let form_artifacts = cfe_borrow_form_shell(
                &cfg_dir,
                &ext_dir,
                &mut write_plan,
                &spec,
                &format_version,
                borrow_main_attribute.is_some(),
                &mut stdout,
            )?;
            cfe_borrow_register_form(
                &ext_dir,
                &mut write_plan,
                &spec.type_name,
                &spec.object_name,
                spec.form_name.as_deref().unwrap_or_default(),
                &mut stdout,
            )?;
            artifacts.extend(form_artifacts);
            artifacts.extend(cfe_borrow_main_attribute_artifacts(
                &cfg_dir,
                &ext_dir,
                &mut write_plan,
                &spec,
                borrow_main_attribute.as_deref(),
                &format_version,
                &mut ext_text,
                &mut stdout,
            )?);
            borrowed_count += 1;
        } else {
            stdout.push_str(&format!(
                "[INFO] Borrowing {}.{}...\n",
                spec.type_name, spec.object_name
            ));
            let artifact = cfe_borrow_object_shell(
                &cfg_dir,
                &ext_dir,
                &mut write_plan,
                &spec.type_name,
                &spec.object_name,
                &format_version,
                &mut ext_text,
                &mut stdout,
            )?;
            artifacts.push(artifact);
            borrowed_count += 1;
        }
    }

    cfe_borrow_normalize_lxml_config_serialization(&mut ext_text);
    write_plan.write_utf8_bom(&ext_path, &ext_text)?;
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
        stdout,
        artifacts,
        borrowed_count,
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

pub(crate) fn borrow_cfe(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let result = (|| -> Result<(String, Vec<PathBuf>, Vec<String>), String> {
        let PreparedCfeBorrow {
            cfg_path,
            ext_path,
            cfg_dir,
            ext_dir,
            write_plan,
            mut stdout,
            mut artifacts,
            borrowed_count,
            registered_format_dependencies,
        } = prepare_cfe_borrow(args, context)?;
        let mut format_owner_targets = vec![cfg_path.as_path(), ext_path.as_path()];
        format_owner_targets.extend(registered_format_dependencies.iter().map(PathBuf::as_path));
        format_owner_targets.sort();
        format_owner_targets.dedup();
        let cleanup_warnings =
            write_plan.commit_with_post_validation(&format_owner_targets, context, || {
                cfe_borrow_validate_extension(&ext_path, context)
            })?;
        stdout.push_str(&format!("[INFO] Saved: {}\n\n", ext_path.display()));
        stdout.push_str("=== cfe-borrow summary ===\n");
        stdout.push_str(&format!("  Extension:  {}\n", ext_dir.display()));
        stdout.push_str(&format!("  Config:     {}\n", cfg_dir.display()));
        stdout.push_str(&format!("  Borrowed:   {borrowed_count} object(s)\n"));
        for artifact in &artifacts {
            stdout.push_str(&format!("    - {}\n", artifact.display()));
        }
        artifacts.push(ext_path);
        Ok((stdout, artifacts, cleanup_warnings))
    })();

    match result {
        Ok((stdout, artifacts, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.cfe.borrow completed with native extension borrower".to_string(),
            changes: artifacts
                .iter()
                .map(|path| format!("updated {}", path.display()))
                .collect(),
            warnings,
            errors: Vec::new(),
            artifacts: artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
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

pub(crate) struct CfeBorrowSpec {
    pub(crate) type_name: String,
    pub(crate) object_name: String,
    pub(crate) form_name: Option<String>,
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
        let updated = utf8_bom_bytes(text);
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

    fn existing_metadata_uuid(
        &mut self,
        path: &Path,
        child_name: &str,
    ) -> Result<Option<String>, String> {
        if !self.exists(path) {
            return Ok(None);
        }
        let text = self.read_utf8_sig(path)?;
        let document = Document::parse(&text)
            .map_err(|error| format!("[ERROR] XML parse error in {}: {error}", path.display()))?;
        Ok(document
            .root_element()
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == child_name)
            .and_then(|node| node.attribute("uuid"))
            .map(ToOwned::to_owned))
    }

    fn validate_xml(&self) -> Result<(), String> {
        for (path, file) in &self.files {
            let is_xml = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
            if !is_xml {
                continue;
            }
            let text = std::str::from_utf8(&file.updated)
                .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?
                .trim_start_matches('\u{feff}');
            Document::parse(text)
                .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
        }
        Ok(())
    }

    fn commit(self) -> Result<Vec<String>, String> {
        self.commit_inner(None, || Ok(()))
    }

    fn commit_with_post_validation<F>(
        self,
        format_owner_targets: &[&Path],
        context: &WorkspaceContext,
        post_validation: F,
    ) -> Result<Vec<String>, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        self.commit_inner(Some((format_owner_targets, context)), post_validation)
    }

    fn commit_inner<F>(
        self,
        format_guard: Option<(&[&Path], &WorkspaceContext)>,
        post_validation: F,
    ) -> Result<Vec<String>, String>
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
        Ok(transaction
            .commit_with_post_validation(post_validation)?
            .cleanup_warnings)
    }
}

pub(crate) fn cfe_borrow_resolve_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
    kind: &str,
) -> Result<PathBuf, String> {
    let raw = required_path(
        args,
        names,
        if kind == "extension" {
            "ExtensionPath"
        } else {
            "ConfigPath"
        },
    )?;
    let mut path = absolutize(raw, &context.cwd);
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

pub(crate) fn cfe_borrow_main_attribute_mode(
    args: &Map<String, Value>,
) -> Result<Option<String>, String> {
    for name in ["borrowMainAttribute", "BorrowMainAttribute"] {
        if let Some(value) = args.get(name) {
            if value.as_bool() == Some(false) || value.is_null() {
                return Ok(None);
            }
            if value.as_bool() == Some(true) {
                return Ok(Some("Form".to_string()));
            }
            if let Some(text) = value.as_str() {
                if text.trim().is_empty() {
                    return Ok(Some("Form".to_string()));
                }
                return Ok(Some(text.trim().to_string()));
            }
        }
    }
    Ok(None)
}

pub(crate) fn cfe_borrow_parse_object_spec(value: &str) -> Result<CfeBorrowSpec, String> {
    let Some(dot_idx) = value.find('.') else {
        return Err(format!(
            "Invalid format '{value}', expected 'Type.Name' or 'Type.Name.Form.FormName'"
        ));
    };
    if dot_idx < 1 {
        return Err(format!(
            "Invalid format '{value}', expected 'Type.Name' or 'Type.Name.Form.FormName'"
        ));
    }
    let raw_type = &value[..dot_idx];
    let type_name = cfe_borrow_type_synonym(raw_type)
        .unwrap_or(raw_type)
        .to_string();
    if cfe_borrow_type_dir(&type_name).is_none() {
        return Err(format!("Unknown type '{type_name}'"));
    }
    if cfe_borrow_generated_types(&type_name).is_none() {
        return Err(format!(
            "Type '{type_name}' has no proven cfe.borrow InternalInfo profile for platform 1C 8.3.27"
        ));
    }
    let remainder = &value[dot_idx + 1..];
    let (object_name, form_name) = if let Some(form_idx) = remainder.find(".Form.") {
        (
            remainder[..form_idx].to_string(),
            Some(remainder[form_idx + 6..].to_string()),
        )
    } else {
        (remainder.to_string(), None)
    };
    cfe_validate_metadata_name("ObjectName", &object_name)?;
    if let Some(form_name) = &form_name {
        cfe_validate_metadata_name("FormName", form_name)?;
    }
    Ok(CfeBorrowSpec {
        type_name,
        object_name,
        form_name,
    })
}

fn cfe_validate_metadata_name(argument: &str, value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(value)
    ) && components.next().is_none();
    if form_is_xml_ncname(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

const CFE_BORROW_MD_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const CFE_BORROW_NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

fn cfe_borrow_validate_source_descriptor<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    source_file: &Path,
    expected_type: &str,
    expected_name: &str,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(CFE_BORROW_MD_NAMESPACE)
    {
        return Err(format!(
            "Source descriptor {} must have MetaDataObject namespace '{}'",
            source_file.display(),
            CFE_BORROW_MD_NAMESPACE
        ));
    }

    let objects = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if objects.len() != 1
        || objects[0].tag_name().name() != expected_type
        || objects[0].tag_name().namespace() != Some(CFE_BORROW_MD_NAMESPACE)
    {
        return Err(format!(
            "Source descriptor {} expected exactly one {expected_type} in namespace '{}'",
            source_file.display(),
            CFE_BORROW_MD_NAMESPACE
        ));
    }
    let object = objects[0];

    let properties = object
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Properties"
                && node.tag_name().namespace() == Some(CFE_BORROW_MD_NAMESPACE)
        })
        .collect::<Vec<_>>();
    if properties.len() != 1 {
        return Err(format!(
            "Source descriptor {} {expected_type}.{expected_name} must contain exactly one Properties element",
            source_file.display()
        ));
    }
    let names = properties[0]
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Name"
                && node.tag_name().namespace() == Some(CFE_BORROW_MD_NAMESPACE)
        })
        .collect::<Vec<_>>();
    if names.len() != 1 || names[0].text() != Some(expected_name) {
        return Err(format!(
            "Source descriptor {} Properties/Name must exactly match '{expected_name}'",
            source_file.display()
        ));
    }

    let source_uuid = object.attribute("uuid").unwrap_or_default();
    if !cf_validate_guid(source_uuid) || source_uuid.eq_ignore_ascii_case(CFE_BORROW_NIL_UUID) {
        return Err(format!(
            "Source descriptor {} {expected_type}.{expected_name} must have a valid non-nil uuid",
            source_file.display()
        ));
    }

    Ok(object)
}

pub(crate) fn cfe_borrow_type_synonym(value: &str) -> Option<&'static str> {
    match value {
        "Справочник" => Some("Catalog"),
        "Документ" => Some("Document"),
        "Перечисление" => Some("Enum"),
        "ОбщийМодуль" => Some("CommonModule"),
        "ОбщаяКартинка" => Some("CommonPicture"),
        "ОбщаяКоманда" => Some("CommonCommand"),
        "ОбщийМакет" => Some("CommonTemplate"),
        "ПланОбмена" => Some("ExchangePlan"),
        "Отчет" | "Отчёт" => Some("Report"),
        "Обработка" => Some("DataProcessor"),
        "РегистрСведений" => Some("InformationRegister"),
        "РегистрНакопления" => Some("AccumulationRegister"),
        "ПланВидовХарактеристик" => Some("ChartOfCharacteristicTypes"),
        "ПланСчетов" => Some("ChartOfAccounts"),
        "РегистрБухгалтерии" => Some("AccountingRegister"),
        "ПланВидовРасчета" => Some("ChartOfCalculationTypes"),
        "РегистрРасчета" => Some("CalculationRegister"),
        "БизнесПроцесс" => Some("BusinessProcess"),
        "Задача" => Some("Task"),
        "Подсистема" => Some("Subsystem"),
        "Роль" => Some("Role"),
        "Константа" => Some("Constant"),
        "ФункциональнаяОпция" => Some("FunctionalOption"),
        "ОпределяемыйТип" => Some("DefinedType"),
        "ОбщаяФорма" => Some("CommonForm"),
        "ЖурналДокументов" => Some("DocumentJournal"),
        "ПараметрСеанса" => Some("SessionParameter"),
        "ГруппаКоманд" => Some("CommandGroup"),
        "ПодпискаНаСобытие" => Some("EventSubscription"),
        "РегламентноеЗадание" => Some("ScheduledJob"),
        "ОбщийРеквизит" => Some("CommonAttribute"),
        "ПакетXDTO" => Some("XDTOPackage"),
        "HTTPСервис" => Some("HTTPService"),
        "СервисИнтеграции" => Some("IntegrationService"),
        _ => None,
    }
}

pub(crate) fn cfe_borrow_type_dir(type_name: &str) -> Option<&'static str> {
    cf_validate_child_type_dir(type_name)
}

pub(crate) fn cfe_borrow_target_object(
    ext_dir: &Path,
    type_name: &str,
    object_name: &str,
) -> PathBuf {
    let dir_name = cfe_borrow_type_dir(type_name).unwrap_or(type_name);
    ext_dir.join(dir_name).join(format!("{object_name}.xml"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cfe_borrow_object_shell(
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut CfeBorrowWritePlan,
    type_name: &str,
    object_name: &str,
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
) -> Result<PathBuf, String> {
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let source_file = cfg_dir.join(dir_name).join(format!("{object_name}.xml"));
    if !source_file.is_file() {
        return Err(format!(
            "Source object not found: {}",
            source_file.display()
        ));
    }
    let source_text = write_plan.read_dependency_utf8_sig(&source_file)?;
    let source_doc =
        Document::parse(&source_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_el = cfe_borrow_validate_source_descriptor(
        source_doc.root_element(),
        &source_file,
        type_name,
        object_name,
    )?;
    let source_uuid = source_el
        .attribute("uuid")
        .expect("validated source descriptor has uuid");
    stdout.push_str(&format!("[INFO]   Source UUID: {source_uuid}\n"));
    let target_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    if write_plan.is_planned(&target_file) {
        stdout.push_str(&format!(
            "[SKIP]   Object already planned in this batch: {}\n",
            target_file.display()
        ));
        cfe_borrow_add_to_child_objects(ext_text, type_name, object_name, stdout)?;
        return Ok(target_file);
    }
    let source_props = meta_info_child(source_el, "Properties");
    let xml = cfe_borrow_object_xml(
        type_name,
        object_name,
        source_uuid,
        source_props,
        format_version,
    )?;
    write_plan.write_utf8_bom(&target_file, &xml)?;
    stdout.push_str(&format!("[INFO]   Created: {}\n", target_file.display()));
    cfe_borrow_add_to_child_objects(ext_text, type_name, object_name, stdout)?;
    Ok(target_file)
}

pub(crate) fn cfe_borrow_object_xml(
    type_name: &str,
    object_name: &str,
    source_uuid: &str,
    source_props: Option<roxmltree::Node<'_, '_>>,
    format_version: &str,
) -> Result<String, String> {
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        cfe_borrow_xmlns_decl(),
        escape_xml(format_version)
    ));
    lines.push(format!("\t<{type_name} uuid=\"{}\">", fresh_uuid()));
    lines.push(cfe_borrow_internal_info_xml(
        type_name,
        object_name,
        "\t\t",
    )?);
    lines.push("\t\t<Properties>".to_string());
    lines.push("\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>".to_string());
    lines.push(format!("\t\t\t<Name>{}</Name>", escape_xml(object_name)));
    lines.push("\t\t\t<Comment/>".to_string());
    lines.push(format!(
        "\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
        escape_xml(source_uuid)
    ));
    if type_name == "CommonModule" {
        for prop_name in [
            "Global",
            "ClientManagedApplication",
            "Server",
            "ExternalConnection",
            "ClientOrdinaryApplication",
            "ServerCall",
        ] {
            let value = source_props
                .and_then(|props| meta_info_child_text(props, prop_name))
                .unwrap_or_else(|| "false".to_string());
            lines.push(format!(
                "\t\t\t<{prop_name}>{}</{prop_name}>",
                escape_xml(&value)
            ));
        }
        let return_values_reuse = source_props
            .and_then(|props| meta_info_child_text(props, "ReturnValuesReuse"))
            .unwrap_or_else(|| "DontUse".to_string());
        lines.push(format!(
            "\t\t\t<ReturnValuesReuse>{}</ReturnValuesReuse>",
            escape_xml(&return_values_reuse)
        ));
    }
    if type_name == "DefinedType" {
        if let Some(type_xml) = source_props
            .and_then(|props| meta_info_child(props, "Type"))
            .map(cfe_borrow_xml_node)
        {
            lines.push(format!("\t\t\t{type_xml}"));
        }
    }
    lines.push("\t\t</Properties>".to_string());
    if cfe_borrow_type_has_child_objects(type_name) {
        lines.push("\t\t<ChildObjects/>".to_string());
    }
    lines.push(format!("\t</{type_name}>"));
    lines.push("</MetaDataObject>".to_string());
    Ok(lines.join("\n"))
}

pub(crate) fn cfe_borrow_internal_info_xml(
    type_name: &str,
    object_name: &str,
    indent: &str,
) -> Result<String, String> {
    let types = cfe_borrow_generated_types(type_name).ok_or_else(|| {
        format!(
            "Type '{type_name}' has no proven cfe.borrow InternalInfo profile for platform 1C 8.3.27"
        )
    })?;
    if types.is_empty() {
        return Ok(format!("{indent}<InternalInfo/>"));
    }
    let mut lines = vec![format!("{indent}<InternalInfo>")];
    if type_name == "ExchangePlan" {
        lines.push(format!(
            "{indent}\t<xr:ThisNode>{}</xr:ThisNode>",
            fresh_uuid()
        ));
    }
    for (prefix, category) in types {
        lines.push(format!(
            "{indent}\t<xr:GeneratedType name=\"{}.{}\" category=\"{}\">",
            prefix,
            escape_xml(object_name),
            category
        ));
        lines.push(format!(
            "{indent}\t\t<xr:TypeId>{}</xr:TypeId>",
            fresh_uuid()
        ));
        lines.push(format!(
            "{indent}\t\t<xr:ValueId>{}</xr:ValueId>",
            fresh_uuid()
        ));
        lines.push(format!("{indent}\t</xr:GeneratedType>"));
    }
    lines.push(format!("{indent}</InternalInfo>"));
    Ok(lines.join("\n"))
}

pub(crate) fn cfe_borrow_generated_types(
    type_name: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    metadata_generated_types_8_3_27(type_name)
}

pub(crate) fn cfe_borrow_type_has_child_objects(type_name: &str) -> bool {
    matches!(
        type_name,
        "Catalog"
            | "Document"
            | "ExchangePlan"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task"
            | "Enum"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
    )
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowSourceAttribute {
    name: String,
    source_uuid: String,
    type_xml: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowGeneratedType {
    name: String,
    category: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowSourceTabularSection {
    name: String,
    source_uuid: String,
    generated_types: Vec<CfeBorrowGeneratedType>,
    attributes: Vec<CfeBorrowSourceAttribute>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CfeBorrowResolvedAttributes {
    attributes: Vec<CfeBorrowSourceAttribute>,
    tabular_sections: Vec<CfeBorrowSourceTabularSection>,
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowDeepPath {
    segments: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CfeBorrowFormPaths {
    first_level: HashSet<String>,
    deep_paths: Vec<CfeBorrowDeepPath>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cfe_borrow_main_attribute_artifacts(
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut CfeBorrowWritePlan,
    spec: &CfeBorrowSpec,
    mode: Option<&str>,
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
) -> Result<Vec<PathBuf>, String> {
    let Some(mode) = mode else {
        return Ok(Vec::new());
    };
    let type_name = spec.type_name.as_str();
    let object_name = spec.object_name.as_str();
    let form_name = spec.form_name.as_deref().unwrap_or_default();
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    stdout.push_str(&format!(
        "[INFO] Borrowing main attribute for {type_name}.{object_name} (mode: {mode})...\n"
    ));

    let form_paths = if mode == "Form" {
        let form_xml_path = cfg_dir
            .join(dir_name)
            .join(object_name)
            .join("Forms")
            .join(form_name)
            .join("Ext")
            .join("Form.xml");
        let paths = cfe_borrow_collect_form_object_paths(write_plan, &form_xml_path)?;
        stdout.push_str(&format!(
            "[INFO]   Collected {} first-level DataPath references, {} deep paths\n",
            paths.first_level.len(),
            paths.deep_paths.len()
        ));
        if paths.first_level.is_empty() && paths.deep_paths.is_empty() {
            stdout.push_str("[INFO]   No main-attribute object paths found in form\n");
            return Ok(Vec::new());
        }
        Some(paths)
    } else {
        stdout.push_str("[INFO]   Mode All: borrowing all attributes and tabular sections\n");
        None
    };

    let wanted = form_paths.as_ref().map(|paths| &paths.first_level);
    let resolved =
        cfe_borrow_resolve_source_attributes(write_plan, cfg_dir, type_name, object_name, wanted)?;
    stdout.push_str(&format!(
        "[INFO]   Resolved: {} attributes, {} tabular section(s)\n",
        resolved.attributes.len(),
        resolved.tabular_sections.len()
    ));

    let object_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    cfe_borrow_merge_resolved_into_object(write_plan, &object_file, &resolved)?;
    stdout.push_str(&format!(
        "[INFO]   Enriched object: {}\n",
        object_file.display()
    ));
    let mut artifacts = Vec::new();

    let mut type_xmls = Vec::<String>::new();
    for attr in &resolved.attributes {
        type_xmls.push(attr.type_xml.clone());
    }
    for section in &resolved.tabular_sections {
        for attr in &section.attributes {
            type_xmls.push(attr.type_xml.clone());
        }
    }
    let ref_types = cfe_borrow_collect_reference_types(&type_xmls);
    stdout.push_str(&format!(
        "[INFO]   Reference types to borrow: {}\n",
        ref_types.len()
    ));
    artifacts.extend(cfe_borrow_ensure_reference_shells(
        cfg_dir,
        ext_dir,
        write_plan,
        &type_xmls,
        format_version,
        ext_text,
        stdout,
    )?);

    if let Some(paths) = &form_paths {
        artifacts.extend(cfe_borrow_process_deep_paths(
            cfg_dir,
            ext_dir,
            write_plan,
            &resolved,
            &paths.deep_paths,
            format_version,
            ext_text,
            stdout,
        )?);
    }

    stdout.push_str("[INFO]   Main attribute borrowing complete\n");
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_collect_form_object_paths(
    write_plan: &mut CfeBorrowWritePlan,
    form_xml_path: &Path,
) -> Result<CfeBorrowFormPaths, String> {
    let source = write_plan.read_dependency_utf8_sig(form_xml_path)?;
    let doc = Document::parse(&source).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let mut paths = CfeBorrowFormPaths::default();
    let binding_tags = [
        "DataPath",
        "TitleDataPath",
        "FooterDataPath",
        "HeaderDataPath",
        "MultipleValueDataPath",
        "MultipleValuePresentDataPath",
        "RowPictureDataPath",
        "MultipleValuePictureDataPath",
        "Field",
    ];
    let mut deep_seen = HashSet::<String>::new();
    for node in doc.descendants().filter(|node| node.is_element()) {
        if !binding_tags.contains(&node.tag_name().name()) {
            continue;
        }
        let Some(text) = node.text() else {
            continue;
        };
        for segments in cfe_borrow_object_path_segments(text) {
            if segments.is_empty() || cfe_borrow_is_standard_field(&segments[0]) {
                continue;
            }
            paths.first_level.insert(segments[0].clone());
            if segments.len() >= 2 && !cfe_borrow_is_standard_field(&segments[1]) {
                let key = segments.join(".");
                if deep_seen.insert(key) {
                    paths.deep_paths.push(CfeBorrowDeepPath { segments });
                }
            }
        }
    }
    Ok(paths)
}

pub(crate) fn cfe_borrow_object_path_segments(text: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("Объект.") {
        let after = &rest[pos + "Объект.".len()..];
        let path = after
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '.')
            .collect::<String>();
        let segments = path
            .split('.')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !segments.is_empty() {
            result.push(segments);
        }
        rest = &after[path.len()..];
    }
    result
}

pub(crate) fn cfe_borrow_is_standard_field(name: &str) -> bool {
    matches!(
        name,
        "Code"
            | "Description"
            | "Ref"
            | "Parent"
            | "DeletionMark"
            | "Predefined"
            | "IsFolder"
            | "LineNumber"
            | "RowsCount"
            | "PredefinedDataName"
    )
}

pub(crate) fn cfe_borrow_resolve_source_attributes(
    write_plan: &mut CfeBorrowWritePlan,
    cfg_dir: &Path,
    type_name: &str,
    object_name: &str,
    first_level_names: Option<&HashSet<String>>,
) -> Result<CfeBorrowResolvedAttributes, String> {
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let source_file = cfg_dir.join(dir_name).join(format!("{object_name}.xml"));
    let source = write_plan.read_dependency_utf8_sig(&source_file)?;
    let doc = Document::parse(&source).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_el = doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| format!("No metadata element found in {dir_name}/{object_name}.xml"))?;
    let Some(child_objects) = meta_info_child(source_el, "ChildObjects") else {
        return Ok(CfeBorrowResolvedAttributes::default());
    };
    let mut resolved = CfeBorrowResolvedAttributes::default();
    for child in child_objects.children().filter(|node| node.is_element()) {
        let local = child.tag_name().name();
        let props = meta_info_child(child, "Properties");
        let name = props
            .and_then(|props| meta_info_child_text(props, "Name"))
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        if first_level_names.is_some_and(|names| !names.contains(&name)) {
            continue;
        }
        if local == "Attribute" {
            resolved.attributes.push(cfe_borrow_source_attribute(child));
        } else if local == "TabularSection" {
            resolved
                .tabular_sections
                .push(cfe_borrow_source_tabular_section(child));
        }
    }
    Ok(resolved)
}

pub(crate) fn cfe_borrow_source_attribute(
    node: roxmltree::Node<'_, '_>,
) -> CfeBorrowSourceAttribute {
    let props = meta_info_child(node, "Properties");
    CfeBorrowSourceAttribute {
        name: props
            .and_then(|props| meta_info_child_text(props, "Name"))
            .unwrap_or_default(),
        source_uuid: node.attribute("uuid").unwrap_or("").to_string(),
        type_xml: props
            .and_then(|props| meta_info_child(props, "Type"))
            .map(cfe_borrow_xml_node)
            .unwrap_or_default(),
    }
}

pub(crate) fn cfe_borrow_source_tabular_section(
    node: roxmltree::Node<'_, '_>,
) -> CfeBorrowSourceTabularSection {
    let props = meta_info_child(node, "Properties");
    let mut generated_types = Vec::new();
    if let Some(internal_info) = meta_info_child(node, "InternalInfo") {
        for generated in meta_info_children(internal_info, "GeneratedType") {
            generated_types.push(CfeBorrowGeneratedType {
                name: generated.attribute("name").unwrap_or("").to_string(),
                category: generated.attribute("category").unwrap_or("").to_string(),
            });
        }
    }
    let mut attributes = Vec::new();
    if let Some(child_objects) = meta_info_child(node, "ChildObjects") {
        for attr in meta_info_children(child_objects, "Attribute") {
            attributes.push(cfe_borrow_source_attribute(attr));
        }
    }
    CfeBorrowSourceTabularSection {
        name: props
            .and_then(|props| meta_info_child_text(props, "Name"))
            .unwrap_or_default(),
        source_uuid: node.attribute("uuid").unwrap_or("").to_string(),
        generated_types,
        attributes,
    }
}

pub(crate) fn cfe_borrow_merge_resolved_into_object(
    write_plan: &mut CfeBorrowWritePlan,
    object_file: &Path,
    resolved: &CfeBorrowResolvedAttributes,
) -> Result<(), String> {
    let mut object_text = write_plan.read_utf8_sig(object_file)?;
    let existing_names = cfe_borrow_existing_names(&object_text);
    let mut child_xml = Vec::<String>::new();
    for attr in &resolved.attributes {
        if !existing_names.contains(&attr.name) {
            child_xml.push(cfe_borrow_adopted_attribute_xml(attr, "\t\t\t"));
        }
    }
    for section in &resolved.tabular_sections {
        if !existing_names.contains(&section.name) {
            child_xml.push(cfe_borrow_adopted_tabular_section_xml(section, "\t\t\t"));
        }
    }
    if child_xml.is_empty() {
        return Ok(());
    }
    cfe_borrow_insert_child_objects(&mut object_text, &child_xml.join("\n"))?;
    write_plan.write_utf8_bom(object_file, &object_text)
}

pub(crate) fn cfe_borrow_existing_names(object_text: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut rest = object_text;
    while let Some(start) = rest.find("<Name>") {
        let value_start = start + "<Name>".len();
        let Some(end_rel) = rest[value_start..].find("</Name>") else {
            break;
        };
        names.insert(rest[value_start..value_start + end_rel].to_string());
        rest = &rest[value_start + end_rel + "</Name>".len()..];
    }
    names
}

pub(crate) fn cfe_borrow_insert_child_objects(
    object_text: &mut String,
    child_xml: &str,
) -> Result<(), String> {
    if object_text.contains("<ChildObjects/>") {
        *object_text = object_text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\r\n{child_xml}\r\n\t\t</ChildObjects>"),
            1,
        );
        return Ok(());
    }
    if let Some(pos) = object_text.find("</ChildObjects>") {
        object_text.insert_str(pos, &format!("\r\n{child_xml}\r\n\t\t"));
        return Ok(());
    }
    Err("Cannot merge attributes: <ChildObjects> not found".to_string())
}

pub(crate) fn cfe_borrow_adopted_attribute_xml(
    attr: &CfeBorrowSourceAttribute,
    indent: &str,
) -> String {
    let mut lines = vec![
        format!("{indent}<Attribute uuid=\"{}\">", fresh_uuid()),
        format!("{indent}\t<InternalInfo/>"),
        format!("{indent}\t<Properties>"),
        format!("{indent}\t\t<ObjectBelonging>Adopted</ObjectBelonging>"),
        format!("{indent}\t\t<Name>{}</Name>", escape_xml(&attr.name)),
        format!("{indent}\t\t<Comment/>"),
        format!(
            "{indent}\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
            escape_xml(&attr.source_uuid)
        ),
    ];
    if !attr.type_xml.is_empty() {
        lines.push(format!("{indent}\t\t{}", attr.type_xml));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Attribute>"));
    lines.join("\n")
}

pub(crate) fn cfe_borrow_adopted_tabular_section_xml(
    section: &CfeBorrowSourceTabularSection,
    indent: &str,
) -> String {
    let mut lines = vec![format!(
        "{indent}<TabularSection uuid=\"{}\">",
        fresh_uuid()
    )];
    if section.generated_types.is_empty() {
        lines.push(format!("{indent}\t<InternalInfo/>"));
    } else {
        lines.push(format!("{indent}\t<InternalInfo>"));
        for generated in &section.generated_types {
            lines.push(format!(
                "{indent}\t\t<xr:GeneratedType name=\"{}\" category=\"{}\">",
                escape_xml(&generated.name),
                escape_xml(&generated.category)
            ));
            lines.push(format!(
                "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
                fresh_uuid()
            ));
            lines.push(format!(
                "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
                fresh_uuid()
            ));
            lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
        }
        lines.push(format!("{indent}\t</InternalInfo>"));
    }
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<ObjectBelonging>Adopted</ObjectBelonging>"
    ));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&section.name)
    ));
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>",
        escape_xml(&section.source_uuid)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    if section.attributes.is_empty() {
        lines.push(format!("{indent}\t<ChildObjects/>"));
    } else {
        lines.push(format!("{indent}\t<ChildObjects>"));
        for attr in &section.attributes {
            lines.push(cfe_borrow_adopted_attribute_xml(
                attr,
                &format!("{indent}\t\t"),
            ));
        }
        lines.push(format!("{indent}\t</ChildObjects>"));
    }
    lines.push(format!("{indent}</TabularSection>"));
    lines.join("\n")
}

pub(crate) fn cfe_borrow_ensure_reference_shells(
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut CfeBorrowWritePlan,
    type_xmls: &[String],
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
) -> Result<Vec<PathBuf>, String> {
    let mut artifacts = Vec::new();
    let mut seen = HashSet::<String>::new();
    for (type_name, object_name) in cfe_borrow_collect_reference_types(type_xmls) {
        let key = format!("{type_name}.{object_name}");
        if !seen.insert(key) {
            continue;
        }
        if write_plan.exists(&cfe_borrow_target_object(ext_dir, &type_name, &object_name)) {
            continue;
        }
        let source_file = cfg_dir
            .join(cfe_borrow_type_dir(&type_name).unwrap_or(&type_name))
            .join(format!("{object_name}.xml"));
        if !source_file.exists() {
            stdout.push_str(&format!(
                "[WARN]   Source not found: {type_name}.{object_name}\n"
            ));
            continue;
        }
        let artifact = cfe_borrow_object_shell(
            cfg_dir,
            ext_dir,
            write_plan,
            &type_name,
            &object_name,
            format_version,
            ext_text,
            stdout,
        )?;
        stdout.push_str(&format!(
            "[INFO]   Auto-borrowed: {type_name}.{object_name}\n"
        ));
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_collect_reference_types(type_xmls: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for type_xml in type_xmls {
        let mut rest = type_xml.as_str();
        while let Some(pos) = rest.find("cfg:") {
            let after = &rest[pos + "cfg:".len()..];
            let token = after
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '.')
                .collect::<String>();
            if let Some((prefix, object_name)) = token.split_once('.') {
                let type_name = if prefix == "DefinedType" {
                    Some("DefinedType".to_string())
                } else {
                    prefix
                        .strip_suffix("Ref")
                        .map(ToOwned::to_owned)
                        .or_else(|| prefix.strip_suffix("Object").map(ToOwned::to_owned))
                };
                if let Some(type_name) = type_name {
                    let key = format!("{type_name}.{object_name}");
                    if seen.insert(key) {
                        result.push((type_name, object_name.to_string()));
                    }
                }
            }
            rest = &after[token.len()..];
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cfe_borrow_process_deep_paths(
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut CfeBorrowWritePlan,
    resolved: &CfeBorrowResolvedAttributes,
    deep_paths: &[CfeBorrowDeepPath],
    format_version: &str,
    ext_text: &mut String,
    stdout: &mut String,
) -> Result<Vec<PathBuf>, String> {
    let mut artifacts = Vec::new();
    let attrs_by_name = resolved
        .attributes
        .iter()
        .map(|attr| (attr.name.as_str(), attr))
        .collect::<BTreeMap<_, _>>();
    let sections_by_name = resolved
        .tabular_sections
        .iter()
        .map(|section| (section.name.as_str(), section))
        .collect::<BTreeMap<_, _>>();
    for path in deep_paths {
        let Some(first) = path.segments.first() else {
            continue;
        };
        let target = if let Some(attr) = attrs_by_name.get(first.as_str()) {
            if path.segments.len() < 2 {
                continue;
            }
            cfe_borrow_reference_target_from_type_xml(&attr.type_xml)
                .map(|target| (target, path.segments[1].clone()))
        } else if let Some(section) = sections_by_name.get(first.as_str()) {
            if path.segments.len() < 3 {
                continue;
            }
            let column_name = &path.segments[1];
            let Some(column) = section
                .attributes
                .iter()
                .find(|attr| attr.name == *column_name)
            else {
                continue;
            };
            cfe_borrow_reference_target_from_type_xml(&column.type_xml)
                .map(|target| (target, path.segments[2].clone()))
        } else {
            None
        };
        let Some(((target_type, target_object), sub_attr_name)) = target else {
            continue;
        };
        let target_path = cfe_borrow_target_object(ext_dir, &target_type, &target_object);
        if !write_plan.exists(&target_path) {
            let artifact = cfe_borrow_object_shell(
                cfg_dir,
                ext_dir,
                write_plan,
                &target_type,
                &target_object,
                format_version,
                ext_text,
                stdout,
            )?;
            stdout.push_str(&format!(
                "[INFO]   Auto-borrowed for deep path: {target_type}.{target_object}\n"
            ));
            artifacts.push(artifact);
        }
        let mut wanted = HashSet::new();
        wanted.insert(sub_attr_name);
        let sub_resolved = cfe_borrow_resolve_source_attributes(
            write_plan,
            cfg_dir,
            &target_type,
            &target_object,
            Some(&wanted),
        )?;
        if !sub_resolved.attributes.is_empty() || !sub_resolved.tabular_sections.is_empty() {
            cfe_borrow_merge_resolved_into_object(write_plan, &target_path, &sub_resolved)?;
            artifacts.push(target_path.clone());
            let mut sub_type_xmls = Vec::new();
            for attr in &sub_resolved.attributes {
                sub_type_xmls.push(attr.type_xml.clone());
            }
            for section in &sub_resolved.tabular_sections {
                for attr in &section.attributes {
                    sub_type_xmls.push(attr.type_xml.clone());
                }
            }
            artifacts.extend(cfe_borrow_ensure_reference_shells(
                cfg_dir,
                ext_dir,
                write_plan,
                &sub_type_xmls,
                format_version,
                ext_text,
                stdout,
            )?);
        }
    }
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_reference_target_from_type_xml(
    type_xml: &str,
) -> Option<(String, String)> {
    cfe_borrow_collect_reference_types(&[type_xml.to_string()])
        .into_iter()
        .next()
}

pub(crate) fn cfe_borrow_add_to_child_objects(
    ext_text: &mut String,
    type_name: &str,
    object_name: &str,
    stdout: &mut String,
) -> Result<(), String> {
    let mut children = cf_edit_child_objects(ext_text)?;
    if children
        .iter()
        .any(|(child_type, child_name)| child_type == type_name && child_name == object_name)
    {
        stdout.push_str(&format!(
            "[WARN] Already in ChildObjects: {type_name}.{object_name}\n"
        ));
        return Ok(());
    }
    children.push((type_name.to_string(), object_name.to_string()));
    children.sort_by(cf_edit_child_object_cmp);
    *ext_text = cf_edit_replace_child_objects(ext_text, &children)?;
    stdout.push_str(&format!(
        "[INFO] Added to ChildObjects: {type_name}.{object_name}\n"
    ));
    Ok(())
}

pub(crate) fn cfe_borrow_normalize_lxml_config_serialization(ext_text: &mut String) {
    if ext_text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>") {
        *ext_text = ext_text.replacen(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            1,
        );
    }
    *ext_text = ext_text.replace("<DefaultRoles></DefaultRoles>", "<DefaultRoles/>");
    if let Some((start, end, _)) = cf_edit_element_range(ext_text, "ChildObjects") {
        let child_objects = ext_text[start..end].replace("\r\n", "&#13;\n");
        ext_text.replace_range(start..end, &child_objects);
    }
    if !ext_text.ends_with('\n') {
        ext_text.push('\n');
    }
}

pub(crate) fn cfe_borrow_form_shell(
    cfg_dir: &Path,
    ext_dir: &Path,
    write_plan: &mut CfeBorrowWritePlan,
    spec: &CfeBorrowSpec,
    format_version: &str,
    borrow_main_attr: bool,
    stdout: &mut String,
) -> Result<Vec<PathBuf>, String> {
    let type_name = spec.type_name.as_str();
    let object_name = spec.object_name.as_str();
    let form_name = spec.form_name.as_deref().unwrap_or_default();
    let dir_name =
        cfe_borrow_type_dir(type_name).ok_or_else(|| format!("Unknown type '{type_name}'"))?;
    let form_meta_source = cfg_dir
        .join(dir_name)
        .join(object_name)
        .join("Forms")
        .join(format!("{form_name}.xml"));
    if !form_meta_source.is_file() {
        return Err(format!(
            "Source form not found: {}",
            form_meta_source.display()
        ));
    }
    let source_text = write_plan.read_dependency_utf8_sig(&form_meta_source)?;
    let source_doc =
        Document::parse(&source_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let source_form = source_doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| {
            format!(
                "No metadata element found in source form: {}",
                form_meta_source.display()
            )
        })?;
    let source_uuid = source_form.attribute("uuid").unwrap_or("");
    if source_uuid.is_empty() {
        return Err(format!(
            "No uuid attribute on source form element: {}",
            form_meta_source.display()
        ));
    }
    stdout.push_str(&format!("[INFO]   Source form UUID: {source_uuid}\n"));
    let source_form_xml = cfg_dir
        .join(dir_name)
        .join(object_name)
        .join("Forms")
        .join(form_name)
        .join("Ext")
        .join("Form.xml");
    if !source_form_xml.is_file() {
        return Err(format!(
            "Source Form.xml not found: {}",
            source_form_xml.display()
        ));
    }
    let form_meta_dir = ext_dir.join(dir_name).join(object_name).join("Forms");
    let form_meta_target = form_meta_dir.join(format!("{form_name}.xml"));
    let form_wrapper_uuid = write_plan
        .existing_metadata_uuid(&form_meta_target, "Form")?
        .unwrap_or_else(fresh_uuid);
    write_plan.write_utf8_bom(
        &form_meta_target,
        &cfe_borrow_form_metadata_xml(form_name, source_uuid, &form_wrapper_uuid, format_version),
    )?;
    stdout.push_str(&format!(
        "[INFO]   Created: {}\n",
        form_meta_target.display()
    ));

    let form_xml_target = form_meta_dir.join(form_name).join("Ext").join("Form.xml");
    let source_form_content = write_plan.read_dependency_utf8_sig(&source_form_xml)?;
    let borrowed_form_xml = cfe_borrow_form_xml(
        &source_form_content,
        cfg_dir,
        type_name,
        object_name,
        borrow_main_attr,
        format_version,
        stdout,
    );
    write_plan.write_utf8_bom(&form_xml_target, &borrowed_form_xml)?;
    stdout.push_str(&format!(
        "[INFO]   Created: {}\n",
        form_xml_target.display()
    ));

    let module_file = form_meta_dir
        .join(form_name)
        .join("Ext")
        .join("Form")
        .join("Module.bsl");
    let artifacts = vec![form_meta_target, form_xml_target];
    if write_plan.exists(&module_file) {
        stdout.push_str(&format!(
            "[SKIP] Module.bsl already exists: {} - not overwriting\n",
            module_file.display()
        ));
    } else {
        stdout.push_str(&format!(
            "[INFO]   Module.bsl omitted because the borrowed form defines no extension module: {}\n",
            module_file.display()
        ));
    }
    Ok(artifacts)
}

pub(crate) fn cfe_borrow_form_metadata_xml(
    form_name: &str,
    source_uuid: &str,
    wrapper_uuid: &str,
    format_version: &str,
) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject {} version=\"{}\">\n\t<Form uuid=\"{}\">\n\t\t<InternalInfo>\n\t\t\t<xr:PropertyState>\n\t\t\t\t<xr:Property>Form</xr:Property>\n\t\t\t\t<xr:State>Extended</xr:State>\n\t\t\t</xr:PropertyState>\n\t\t</InternalInfo>\n\t\t<Properties>\n\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>\n\t\t\t<Name>{}</Name>\n\t\t\t<Comment/>\n\t\t\t<ExtendedConfigurationObject>{}</ExtendedConfigurationObject>\n\t\t\t<FormType>Managed</FormType>\n\t\t</Properties>\n\t</Form>\n</MetaDataObject>",
        cfe_borrow_xmlns_decl(),
        escape_xml(format_version),
        escape_xml(wrapper_uuid),
        escape_xml(form_name),
        escape_xml(source_uuid)
    )
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowFormBlock {
    local_name: String,
    xml: String,
}

pub(crate) fn cfe_borrow_form_xml(
    source_form_content: &str,
    cfg_dir: &Path,
    type_name: &str,
    object_name: &str,
    borrow_main_attr: bool,
    _format_version: &str,
    stdout: &mut String,
) -> String {
    let source = source_form_content.trim_start_matches('\u{feff}');
    let version = ACTIVE_FORMAT_PROFILE.export_format.to_string();
    let blocks = cfe_borrow_form_top_level_blocks(source);
    if blocks.is_empty() {
        return cfe_borrow_form_xml_fallback(source, type_name, object_name, borrow_main_attr);
    }

    let xml_decl = cfe_borrow_xml_declaration(source)
        .unwrap_or("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")
        .to_string();
    let form_tag = cfe_borrow_form_open_tag(source)
        .unwrap_or_else(|| format!("<Form version=\"{}\">", escape_xml(&version)));

    let mut form_props = Vec::<String>::new();
    let mut auto_cmd_xml = None::<String>;
    let mut child_items_xml = None::<String>;
    let mut reached_visual = false;
    for block in &blocks {
        match block.local_name.as_str() {
            "AutoCommandBar" if auto_cmd_xml.is_none() => {
                reached_visual = true;
                auto_cmd_xml = Some(cfe_borrow_clean_form_fragment(&block.xml));
            }
            "ChildItems" => {
                reached_visual = true;
                if child_items_xml.is_none() {
                    child_items_xml = Some(cfe_borrow_clean_form_fragment(&block.xml));
                }
            }
            "Events" | "Attributes" | "Commands" | "Parameters" | "CommandSet" => {
                reached_visual = true;
            }
            _ if !reached_visual => {
                form_props.push(cfe_borrow_clean_form_fragment(&block.xml));
            }
            _ => {}
        }
    }

    if let Some(xml) = &mut auto_cmd_xml {
        *xml = cfe_borrow_replace_simple_element_text(xml, "CommandName", "0");
        *xml = xml.replace("<Autofill>true</Autofill>", "<Autofill>false</Autofill>");
        *xml = cfe_borrow_remove_simple_element(xml, "ExcludedCommand");
        if borrow_main_attr {
            *xml = cfe_borrow_remove_simple_element_unless_text_starts_with(
                xml,
                "DataPath",
                "Объект.",
            );
        } else {
            *xml = cfe_borrow_remove_simple_element(xml, "DataPath");
        }
    }

    if let Some(xml) = &mut child_items_xml {
        *xml = cfe_borrow_replace_simple_element_text(xml, "CommandName", "0");
        if borrow_main_attr {
            *xml = cfe_borrow_remove_simple_element_unless_text_starts_with(
                xml,
                "DataPath",
                "Объект.",
            );
            *xml = cfe_borrow_remove_simple_element_unless_text_starts_with(
                xml,
                "TitleDataPath",
                "Объект.",
            );
            *xml = cfe_borrow_remove_simple_element(xml, "RowPictureDataPath");
        } else {
            *xml = cfe_borrow_remove_simple_element(xml, "DataPath");
            *xml = cfe_borrow_remove_simple_element(xml, "TitleDataPath");
            *xml = cfe_borrow_remove_simple_element(xml, "RowPictureDataPath");
        }
        *xml = cfe_borrow_remove_simple_element(xml, "ExcludedCommand");
        *xml = cfe_borrow_remove_element_blocks(xml, "TypeLink", |block| {
            block.contains("<xr:DataPath>Items.")
        });
        *xml = cfe_borrow_remove_element_blocks(xml, "Events", |_| true);

        let mut referenced_pictures = cfe_borrow_collect_common_picture_refs(xml);
        if let Some(auto_xml) = &auto_cmd_xml {
            referenced_pictures.extend(cfe_borrow_collect_common_picture_refs(auto_xml));
        }
        referenced_pictures.sort();
        referenced_pictures.dedup();
        let mut borrowed_pictures = HashSet::<String>::new();
        for picture_name in &referenced_pictures {
            if cfg_dir
                .join("CommonPictures")
                .join(format!("{picture_name}.xml"))
                .is_file()
            {
                borrowed_pictures.insert(picture_name.clone());
            } else {
                stdout.push_str(&format!(
                    "[WARN]   CommonPicture.{picture_name} not found in source config — will strip from form\n"
                ));
            }
        }
        *xml = cfe_borrow_strip_picture_blocks(xml, &borrowed_pictures);
        if let Some(auto_xml) = &mut auto_cmd_xml {
            *auto_xml = cfe_borrow_strip_picture_blocks(auto_xml, &borrowed_pictures);
        }

        let mut referenced_styles = cfe_borrow_collect_style_item_refs(xml);
        referenced_styles.sort();
        referenced_styles.dedup();
        for style_name in referenced_styles {
            if !cfg_dir
                .join("StyleItems")
                .join(format!("{style_name}.xml"))
                .is_file()
            {
                stdout.push_str(&format!(
                    "[WARN]   StyleItem.{style_name} not found in source config\n"
                ));
            }
        }
    }

    let main_attr_type = if borrow_main_attr {
        let object_type_prefix = cfe_borrow_generated_types(type_name)
            .and_then(|items| {
                items
                    .iter()
                    .find(|(_, category)| *category == "Object")
                    .map(|(prefix, _)| *prefix)
            })
            .unwrap_or("");
        Some(format!(
            "cfg:{}.{}",
            escape_xml(object_type_prefix),
            escape_xml(object_name)
        ))
    } else {
        None
    };

    let mut parts = vec![xml_decl, "\r\n".to_string(), form_tag, "\r\n".to_string()];
    for prop_xml in &form_props {
        parts.push(format!("\t{prop_xml}\r\n"));
    }
    if let Some(xml) = &auto_cmd_xml {
        parts.push(format!("\t{xml}\r\n"));
    }
    if let Some(xml) = &child_items_xml {
        parts.push(format!("\t{xml}\r\n"));
    }
    parts.push(cfe_borrow_main_form_attributes_xml(
        "\t",
        main_attr_type.as_deref(),
    ));
    parts.push("\r\n".to_string());
    parts.push(format!(
        "\t<BaseForm version=\"{}\">\r\n",
        escape_xml(&version)
    ));
    for prop_xml in &form_props {
        parts.push(format!("\t\t{prop_xml}\r\n"));
    }
    if let Some(xml) = &auto_cmd_xml {
        parts.push(cfe_borrow_indent_form_fragment_for_base(xml));
    }
    if let Some(xml) = &child_items_xml {
        parts.push(cfe_borrow_indent_form_fragment_for_base(xml));
    }
    parts.push(cfe_borrow_main_form_attributes_xml(
        "\t\t",
        main_attr_type.as_deref(),
    ));
    parts.push("\r\n\t</BaseForm>\r\n</Form>".to_string());
    parts.concat()
}

pub(crate) fn cfe_borrow_form_xml_fallback(
    source: &str,
    type_name: &str,
    object_name: &str,
    borrow_main_attr: bool,
) -> String {
    let version = ACTIVE_FORMAT_PROFILE.export_format.to_string();
    let mut content = cfe_borrow_normalize_form_root_version(source);
    if !borrow_main_attr {
        content = cfe_borrow_strip_simple_data_paths(&content);
    }
    let main_attr = if borrow_main_attr {
        let object_type_prefix = cfe_borrow_generated_types(type_name)
            .and_then(|items| {
                items
                    .iter()
                    .find(|(_, category)| *category == "Object")
                    .map(|(prefix, _)| *prefix)
            })
            .unwrap_or(type_name);
        format!(
            "<Attributes>\n\t\t<Attribute name=\"Объект\" id=\"1000001\">\n\t\t\t<Type><v8:Type>cfg:{}.{}</v8:Type></Type>\n\t\t\t<MainAttribute>true</MainAttribute>\n\t\t\t<SavedData>true</SavedData>\n\t\t</Attribute>\n\t</Attributes>",
            escape_xml(object_type_prefix),
            escape_xml(object_name)
        )
    } else {
        "<Attributes/>".to_string()
    };
    if content.contains("</Form>") && !content.contains("<BaseForm") {
        content = content.replacen(
            "</Form>",
            &format!(
                "\t<BaseForm version=\"{}\">\n\t\t{}\n\t</BaseForm>\n</Form>",
                escape_xml(&version),
                main_attr
            ),
            1,
        );
    }
    if borrow_main_attr && content.contains("<Attributes/>") {
        content = content.replacen("<Attributes/>", &main_attr, 1);
    }
    ensure_trailing_lf(&content)
}

pub(crate) fn cfe_borrow_strip_simple_data_paths(value: &str) -> String {
    let mut text = cfe_borrow_remove_simple_element(value, "DataPath");
    text = cfe_borrow_remove_simple_element(&text, "TitleDataPath");
    cfe_borrow_remove_simple_element(&text, "RowPictureDataPath")
}

pub(crate) fn cfe_borrow_xml_declaration(source: &str) -> Option<&str> {
    if source.starts_with("<?xml") {
        let end = source.find("?>")? + 2;
        Some(&source[..end])
    } else {
        None
    }
}

pub(crate) fn cfe_borrow_form_open_tag(source: &str) -> Option<String> {
    let start = cfe_borrow_find_start_tag(source, "Form", 0)?;
    let end = source[start..].find('>')? + start + 1;
    Some(cfe_borrow_normalize_form_open_tag(&source[start..end]))
}

pub(crate) fn cfe_borrow_normalize_form_root_version(source: &str) -> String {
    let Some(start) = cfe_borrow_find_start_tag(source, "Form", 0) else {
        return source.to_string();
    };
    let Some(end) = source[start..].find('>').map(|offset| start + offset + 1) else {
        return source.to_string();
    };
    let normalized = cfe_borrow_normalize_form_open_tag(&source[start..end]);
    format!("{}{}{}", &source[..start], normalized, &source[end..])
}

pub(crate) fn cfe_borrow_normalize_form_open_tag(open_tag: &str) -> String {
    let active = ACTIVE_FORMAT_PROFILE.export_format;
    let normalized = match cfe_borrow_version_value_range(open_tag) {
        Ok(Some((value_start, value_end))) => format!(
            "{}{}{}",
            &open_tag[..value_start],
            active,
            &open_tag[value_end..]
        ),
        Err(()) => open_tag.to_string(),
        Ok(None) => {
            let insert_at = open_tag
                .rfind("/>")
                .or_else(|| open_tag.rfind('>'))
                .unwrap_or(open_tag.len());
            format!(
                "{} version=\"{}\"{}",
                &open_tag[..insert_at],
                active,
                &open_tag[insert_at..]
            )
        }
    };
    let normalized =
        cfe_borrow_ensure_namespace_declaration(normalized, "v8", "http://v8.1c.ru/8.1/data/core");
    cfe_borrow_ensure_namespace_declaration(
        normalized,
        "cfg",
        "http://v8.1c.ru/8.1/data/enterprise/current-config",
    )
}

fn cfe_borrow_ensure_namespace_declaration(
    mut open_tag: String,
    prefix: &str,
    namespace: &str,
) -> String {
    let attribute_name = format!("xmlns:{prefix}");
    match cfe_borrow_xml_attribute_value_range(&open_tag, &attribute_name) {
        Ok(Some((value_start, value_end))) => {
            if &open_tag[value_start..value_end] != namespace {
                open_tag.replace_range(value_start..value_end, namespace);
            }
            return open_tag;
        }
        Err(()) => return open_tag,
        Ok(None) => {}
    }
    let insert_at = open_tag
        .rfind("/>")
        .or_else(|| open_tag.rfind('>'))
        .unwrap_or(open_tag.len());
    open_tag.insert_str(insert_at, &format!(" xmlns:{prefix}=\"{namespace}\""));
    open_tag
}

fn cfe_borrow_has_xml_attribute(open_tag: &str, attribute_name: &str) -> bool {
    let mut search_start = 0usize;
    while let Some(relative) = open_tag[search_start..].find(attribute_name) {
        let start = search_start + relative;
        let end = start + attribute_name.len();
        let preceded_by_space = start > 0 && open_tag.as_bytes()[start - 1].is_ascii_whitespace();
        let followed_by_assignment = open_tag[end..]
            .trim_start_matches(|character: char| character.is_ascii_whitespace())
            .starts_with('=');
        if preceded_by_space && followed_by_assignment {
            return true;
        }
        search_start = end;
    }
    false
}

fn cfe_borrow_version_value_range(open_tag: &str) -> Result<Option<(usize, usize)>, ()> {
    cfe_borrow_xml_attribute_value_range(open_tag, "version")
}

fn cfe_borrow_xml_attribute_value_range(
    open_tag: &str,
    attribute_name: &str,
) -> Result<Option<(usize, usize)>, ()> {
    let bytes = open_tag.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'<'));
    while index < bytes.len() && !cfe_borrow_is_xml_space(bytes[index]) && bytes[index] != b'>' {
        index += 1;
    }

    while index < bytes.len() {
        while index < bytes.len() && cfe_borrow_is_xml_space(bytes[index]) {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'>' || bytes[index] == b'/' {
            return Ok(None);
        }

        let name_start = index;
        while index < bytes.len()
            && !cfe_borrow_is_xml_space(bytes[index])
            && !matches!(bytes[index], b'=' | b'>' | b'/')
        {
            index += 1;
        }
        let is_target = &open_tag[name_start..index] == attribute_name;
        while index < bytes.len() && cfe_borrow_is_xml_space(bytes[index]) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            if is_target {
                return Err(());
            }
            continue;
        }
        index += 1;
        while index < bytes.len() && cfe_borrow_is_xml_space(bytes[index]) {
            index += 1;
        }
        let Some(quote @ (b'\'' | b'"')) = bytes.get(index).copied() else {
            if is_target {
                return Err(());
            }
            while index < bytes.len()
                && !cfe_borrow_is_xml_space(bytes[index])
                && bytes[index] != b'>'
            {
                index += 1;
            }
            continue;
        };
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        if index >= bytes.len() {
            return if is_target { Err(()) } else { Ok(None) };
        }
        if is_target {
            return Ok(Some((value_start, index)));
        }
        index += 1;
    }
    Ok(None)
}

fn cfe_borrow_is_xml_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

pub(crate) fn cfe_borrow_form_top_level_blocks(source: &str) -> Vec<CfeBorrowFormBlock> {
    let Some(root_start) = cfe_borrow_find_start_tag(source, "Form", 0) else {
        return Vec::new();
    };
    let Some(root_open_end) = source[root_start..]
        .find('>')
        .map(|pos| root_start + pos + 1)
    else {
        return Vec::new();
    };
    let Some(root_close_rel) = source[root_open_end..].rfind("</Form>") else {
        return Vec::new();
    };
    let body = &source[root_open_end..root_open_end + root_close_rel];
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_open) = body[cursor..].find('<') {
        let open = cursor + rel_open;
        if body[open..].starts_with("</")
            || body[open..].starts_with("<?")
            || body[open..].starts_with("<!")
        {
            cursor = open + 1;
            continue;
        }
        let Some(tag) = cfe_borrow_start_tag_at(body, open) else {
            cursor = open + 1;
            continue;
        };
        let Some(end) = cfe_borrow_element_end(body, open) else {
            cursor = tag.end;
            continue;
        };
        let tail_end = body[end..]
            .find('<')
            .map(|pos| end + pos)
            .unwrap_or(body.len());
        blocks.push(CfeBorrowFormBlock {
            local_name: tag.local_name,
            xml: body[open..tail_end].to_string(),
        });
        cursor = tail_end;
    }
    blocks
}

#[derive(Clone, Debug)]
pub(crate) struct CfeBorrowTag {
    local_name: String,
    end: usize,
    self_closing: bool,
}

pub(crate) fn cfe_borrow_start_tag_at(text: &str, open: usize) -> Option<CfeBorrowTag> {
    let rest = text.get(open..)?;
    if !rest.starts_with('<')
        || rest.starts_with("</")
        || rest.starts_with("<?")
        || rest.starts_with("<!")
    {
        return None;
    }
    let after = open + 1;
    let mut name_end = after;
    for (idx, ch) in text[after..].char_indices() {
        if ch.is_whitespace() || ch == '>' || ch == '/' {
            break;
        }
        name_end = after + idx + ch.len_utf8();
    }
    if name_end == after {
        return None;
    }
    let raw_name = &text[after..name_end];
    let gt = text[open..].find('>')? + open;
    let open_tag = &text[open..=gt];
    Some(CfeBorrowTag {
        local_name: cfe_borrow_tag_local_name(raw_name).to_string(),
        end: gt + 1,
        self_closing: open_tag.trim_end().ends_with("/>"),
    })
}

pub(crate) fn cfe_borrow_closing_tag_at(text: &str, open: usize) -> Option<(String, usize)> {
    let rest = text.get(open..)?;
    if !rest.starts_with("</") {
        return None;
    }
    let after = open + 2;
    let mut name_end = after;
    for (idx, ch) in text[after..].char_indices() {
        if ch.is_whitespace() || ch == '>' {
            break;
        }
        name_end = after + idx + ch.len_utf8();
    }
    if name_end == after {
        return None;
    }
    let raw_name = &text[after..name_end];
    let gt = text[open..].find('>')? + open;
    Some((cfe_borrow_tag_local_name(raw_name).to_string(), gt + 1))
}

pub(crate) fn cfe_borrow_element_end(text: &str, open: usize) -> Option<usize> {
    let start_tag = cfe_borrow_start_tag_at(text, open)?;
    if start_tag.self_closing {
        return Some(start_tag.end);
    }
    let mut depth = 1usize;
    let mut cursor = start_tag.end;
    while let Some(rel_next) = text[cursor..].find('<') {
        let next = cursor + rel_next;
        if let Some((local_name, end)) = cfe_borrow_closing_tag_at(text, next) {
            if local_name == start_tag.local_name {
                depth -= 1;
                if depth == 0 {
                    return Some(end);
                }
            }
            cursor = end;
            continue;
        }
        if let Some(tag) = cfe_borrow_start_tag_at(text, next) {
            if tag.local_name == start_tag.local_name && !tag.self_closing {
                depth += 1;
            }
            cursor = tag.end;
            continue;
        }
        cursor = next + 1;
    }
    None
}

pub(crate) fn cfe_borrow_tag_local_name(raw_name: &str) -> &str {
    raw_name
        .rsplit_once(':')
        .map(|(_, name)| name)
        .unwrap_or(raw_name)
}

pub(crate) fn cfe_borrow_find_start_tag(
    text: &str,
    local_name: &str,
    offset: usize,
) -> Option<usize> {
    let mut cursor = offset;
    while let Some(rel_open) = text[cursor..].find('<') {
        let open = cursor + rel_open;
        if let Some(tag) = cfe_borrow_start_tag_at(text, open) {
            if tag.local_name == local_name {
                return Some(open);
            }
            cursor = tag.end;
        } else {
            cursor = open + 1;
        }
    }
    None
}

pub(crate) fn cfe_borrow_clean_form_fragment(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    cfe_borrow_strip_xmlns_declarations(&normalized)
}

pub(crate) fn cfe_borrow_strip_xmlns_declarations(value: &str) -> String {
    let mut result = value.to_string();
    let mut cursor = 0usize;
    while let Some(rel_pos) = result[cursor..].find("xmlns") {
        let pos = cursor + rel_pos;
        let after_xmlns = pos + "xmlns".len();
        let next = result[after_xmlns..].chars().next();
        if !matches!(next, Some('=') | Some(':')) {
            cursor = after_xmlns;
            continue;
        }
        let Some(prev) = result[..pos].chars().next_back() else {
            cursor = after_xmlns;
            continue;
        };
        if !prev.is_whitespace() {
            cursor = after_xmlns;
            continue;
        }
        let mut remove_start = pos;
        while remove_start > 0 {
            let Some(ch) = result[..remove_start].chars().next_back() else {
                break;
            };
            if ch.is_whitespace() {
                remove_start -= ch.len_utf8();
            } else {
                break;
            }
        }
        let Some(first_quote_rel) = result[pos..].find('"') else {
            cursor = after_xmlns;
            continue;
        };
        let first_quote = pos + first_quote_rel;
        let Some(second_quote_rel) = result[first_quote + 1..].find('"') else {
            cursor = after_xmlns;
            continue;
        };
        let remove_end = first_quote + 1 + second_quote_rel + 1;
        result.replace_range(remove_start..remove_end, "");
        cursor = remove_start;
    }
    result
}

pub(crate) fn cfe_borrow_replace_simple_element_text(
    value: &str,
    tag: &str,
    replacement: &str,
) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while let Some(rel_start) = value[cursor..].find(&open) {
        let start = cursor + rel_start;
        let value_start = start + open.len();
        let Some(rel_end) = value[value_start..].find(&close) else {
            break;
        };
        let end = value_start + rel_end;
        result.push_str(&value[cursor..value_start]);
        result.push_str(replacement);
        result.push_str(&value[end..end + close.len()]);
        cursor = end + close.len();
    }
    result.push_str(&value[cursor..]);
    result
}

pub(crate) fn cfe_borrow_remove_simple_element(value: &str, tag: &str) -> String {
    cfe_borrow_remove_simple_element_if(value, tag, |_| true)
}

pub(crate) fn cfe_borrow_remove_simple_element_unless_text_starts_with(
    value: &str,
    tag: &str,
    prefix: &str,
) -> String {
    cfe_borrow_remove_simple_element_if(value, tag, |text| !text.trim().starts_with(prefix))
}

pub(crate) fn cfe_borrow_remove_simple_element_if<F>(
    value: &str,
    tag: &str,
    should_remove: F,
) -> String
where
    F: Fn(&str) -> bool,
{
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(value.len());
    let mut copied_until = 0usize;
    let mut cursor = 0usize;
    while let Some(rel_start) = value[cursor..].find(&open) {
        let start = cursor + rel_start;
        let value_start = start + open.len();
        let Some(rel_end) = value[value_start..].find(&close) else {
            break;
        };
        let end = value_start + rel_end + close.len();
        if should_remove(&value[value_start..value_start + rel_end]) {
            let remove_start =
                cfe_borrow_preceding_whitespace_start(value, start).max(copied_until);
            result.push_str(&value[copied_until..remove_start]);
            copied_until = end;
        }
        cursor = end;
    }
    result.push_str(&value[copied_until..]);
    result
}

pub(crate) fn cfe_borrow_remove_element_blocks<F>(
    value: &str,
    tag: &str,
    should_remove: F,
) -> String
where
    F: Fn(&str) -> bool,
{
    let mut result = String::with_capacity(value.len());
    let mut copied_until = 0usize;
    let mut cursor = 0usize;
    while let Some(rel_open) = value[cursor..].find('<') {
        let open = cursor + rel_open;
        let Some(start_tag) = cfe_borrow_start_tag_at(value, open) else {
            cursor = open + 1;
            continue;
        };
        if start_tag.local_name != tag {
            cursor = start_tag.end;
            continue;
        }
        let Some(end) = cfe_borrow_element_end(value, open) else {
            cursor = start_tag.end;
            continue;
        };
        if should_remove(&value[open..end]) {
            let remove_start = cfe_borrow_preceding_whitespace_start(value, open).max(copied_until);
            result.push_str(&value[copied_until..remove_start]);
            copied_until = end;
        }
        cursor = end;
    }
    result.push_str(&value[copied_until..]);
    result
}

pub(crate) fn cfe_borrow_preceding_whitespace_start(value: &str, start: usize) -> usize {
    let mut remove_start = start;
    while remove_start > 0 {
        let Some(ch) = value[..remove_start].chars().next_back() else {
            break;
        };
        if ch.is_whitespace() {
            remove_start -= ch.len_utf8();
        } else {
            break;
        }
    }
    remove_start
}

pub(crate) fn cfe_borrow_collect_common_picture_refs(value: &str) -> Vec<String> {
    let mut result = Vec::<String>::new();
    let mut cursor = 0usize;
    let needle = "<xr:Ref>CommonPicture.";
    while let Some(rel_pos) = value[cursor..].find(needle) {
        let start = cursor + rel_pos + needle.len();
        let name = value[start..]
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect::<String>();
        if !name.is_empty() {
            result.push(name);
        }
        cursor = start;
    }
    result
}

pub(crate) fn cfe_borrow_strip_picture_blocks(
    value: &str,
    borrowed_common_pictures: &HashSet<String>,
) -> String {
    cfe_borrow_remove_element_blocks(value, "Picture", |block| {
        if let Some(name) = cfe_borrow_common_picture_name(block) {
            return !borrowed_common_pictures.contains(&name);
        }
        if let Some(name) = cfe_borrow_std_picture_name(block) {
            return name != "Print";
        }
        false
    })
}

pub(crate) fn cfe_borrow_common_picture_name(value: &str) -> Option<String> {
    let needle = "<xr:Ref>CommonPicture.";
    let start = value.find(needle)? + needle.len();
    let name = value[start..]
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

pub(crate) fn cfe_borrow_std_picture_name(value: &str) -> Option<String> {
    let needle = "<xr:Ref>StdPicture.";
    let start = value.find(needle)? + needle.len();
    let name = value[start..]
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

pub(crate) fn cfe_borrow_collect_style_item_refs(value: &str) -> Vec<String> {
    let mut result = Vec::<String>::new();
    let mut cursor = 0usize;
    let attr_needle = "ref=\"style:";
    while let Some(rel_pos) = value[cursor..].find(attr_needle) {
        let start = cursor + rel_pos + attr_needle.len();
        let name = cfe_borrow_read_identifier(&value[start..]);
        if !name.is_empty() {
            result.push(name);
        }
        cursor = start;
    }
    cursor = 0;
    let text_needle = ">style:";
    while let Some(rel_pos) = value[cursor..].find(text_needle) {
        let start = cursor + rel_pos + text_needle.len();
        let name = cfe_borrow_read_identifier(&value[start..]);
        if !name.is_empty() {
            result.push(name);
        }
        cursor = start;
    }
    result
}

pub(crate) fn cfe_borrow_read_identifier(value: &str) -> String {
    value
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect()
}

pub(crate) fn cfe_borrow_main_form_attributes_xml(
    indent: &str,
    main_attr_type: Option<&str>,
) -> String {
    if let Some(main_attr_type) = main_attr_type {
        format!(
            "{indent}<Attributes>\r\n{indent}\t<Attribute name=\"Объект\" id=\"1000001\">\r\n{indent}\t\t<Type><v8:Type>{main_attr_type}</v8:Type></Type>\r\n{indent}\t\t<MainAttribute>true</MainAttribute>\r\n{indent}\t\t<SavedData>true</SavedData>\r\n{indent}\t</Attribute>\r\n{indent}</Attributes>"
        )
    } else {
        format!("{indent}<Attributes/>")
    }
}

pub(crate) fn cfe_borrow_indent_form_fragment_for_base(value: &str) -> String {
    let mut result = String::new();
    for (idx, line) in value.split('\n').enumerate() {
        if idx == 0 {
            result.push_str("\t\t");
            result.push_str(line);
        } else {
            result.push('\t');
            result.push_str(line);
        }
        result.push_str("\r\n");
    }
    result
}

pub(crate) fn cfe_borrow_register_form(
    ext_dir: &Path,
    write_plan: &mut CfeBorrowWritePlan,
    type_name: &str,
    object_name: &str,
    form_name: &str,
    stdout: &mut String,
) -> Result<(), String> {
    let object_file = cfe_borrow_target_object(ext_dir, type_name, object_name);
    if !write_plan.exists(&object_file) {
        stdout.push_str(&format!(
            "[WARN] Parent object file not found: {} - form not registered in ChildObjects\n",
            object_file.display()
        ));
        return Ok(());
    }
    let mut text = write_plan.read_utf8_sig(&object_file)?;
    let tag = format!("<Form>{}</Form>", escape_xml(form_name));
    if text.contains(&tag) {
        stdout.push_str(&format!(
            "[WARN] Form '{form_name}' already in ChildObjects of {type_name}.{object_name}\n"
        ));
        return Ok(());
    }
    if text.contains("<ChildObjects/>") {
        text = text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\r\n\t\t\t{tag}\r\n\t\t</ChildObjects>"),
            1,
        );
    } else if text.contains("</ChildObjects>") {
        text = text.replacen(
            "</ChildObjects>",
            &format!("\t\t\t{tag}\r\n\t\t</ChildObjects>"),
            1,
        );
    } else {
        text = text.replacen(
            &format!("</{type_name}>"),
            &format!(
                "\t\t<ChildObjects>\r\n\t\t\t{tag}\r\n\t\t</ChildObjects>\r\n\t</{type_name}>"
            ),
            1,
        );
    }
    cfe_borrow_normalize_lxml_config_serialization(&mut text);
    write_plan.write_utf8_bom(&object_file, &text)?;
    stdout.push_str(&format!(
        "[INFO]   Registered form in: {}\n",
        object_file.display()
    ));
    Ok(())
}

pub(crate) fn cfe_borrow_xmlns_decl() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
}

pub(crate) fn cfe_borrow_existing_metadata_uuid(path: &Path, child_name: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    doc.root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == child_name)
        .and_then(|node| node.attribute("uuid"))
        .map(ToOwned::to_owned)
}

pub(crate) fn cfe_borrow_xml_node(node: roxmltree::Node<'_, '_>) -> String {
    if node.is_text() {
        return escape_xml(node.text().unwrap_or_default());
    }
    if !node.is_element() {
        return String::new();
    }
    let tag = cfe_borrow_prefixed_name(node.tag_name().namespace(), node.tag_name().name());
    let mut attrs = String::new();
    for attr in node.attributes() {
        let name = cfe_borrow_prefixed_name(attr.namespace(), attr.name());
        attrs.push_str(&format!(" {name}=\"{}\"", escape_xml(attr.value())));
    }
    let children = node
        .children()
        .map(cfe_borrow_xml_node)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if children.is_empty() {
        format!("<{tag}{attrs}/>")
    } else {
        format!("<{tag}{attrs}>{}</{tag}>", children.join(""), tag = tag)
    }
}

pub(crate) fn cfe_borrow_prefixed_name(namespace: Option<&str>, local_name: &str) -> String {
    match namespace {
        Some("http://v8.1c.ru/8.1/data/core") => format!("v8:{local_name}"),
        Some("http://v8.1c.ru/8.3/xcf/readable") => format!("xr:{local_name}"),
        Some("http://www.w3.org/2001/XMLSchema-instance") => format!("xsi:{local_name}"),
        _ => local_name.to_string(),
    }
}

pub(crate) fn diff_cfe(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(String, PathBuf), String> {
        let extension_path_raw =
            required_path(args, &["extensionPath", "ExtensionPath"], "ExtensionPath")?;
        let config_path_raw = required_path(args, &["configPath", "ConfigPath"], "ConfigPath")?;
        let mut extension_path = absolutize(extension_path_raw, &context.cwd);
        let mut config_path = absolutize(config_path_raw, &context.cwd);
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
        let ext_doc = Document::parse(ext_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", ext_cfg.display()))?;
        let ext_root = ext_doc.root_element();
        let ext_cfg_node = ext_root
            .descendants()
            .find(|node| role_info_element(*node, "Configuration", Some(MD_NS)));
        let ext_props = ext_cfg_node.and_then(|node| meta_info_child(node, "Properties"));
        let ext_name = ext_props
            .and_then(|props| meta_info_child_text(props, "Name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "?".to_string());
        let name_prefix = ext_props
            .and_then(|props| meta_info_child_text(props, "NamePrefix"))
            .unwrap_or_default();
        let purpose = ext_props
            .and_then(|props| meta_info_child_text(props, "ConfigurationExtensionPurpose"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "?".to_string());
        let mode = string_arg(args, &["mode", "Mode"]).unwrap_or("A");
        if !matches!(mode, "A" | "B") {
            return Err(format!(
                "argument -Mode: invalid choice: '{mode}' (choose from 'A', 'B')"
            ));
        }

        let mut lines = vec![
            format!("=== cfe-diff Mode {mode}: {ext_name} ({purpose}) ==="),
            format!("    NamePrefix: {name_prefix}"),
            String::new(),
        ];

        let child_obj_node = ext_cfg_node.and_then(|node| meta_info_child(node, "ChildObjects"));
        let Some(child_obj_node) = child_obj_node else {
            lines.push("[WARN] No ChildObjects in extension".to_string());
            return Ok((format!("{}\n", lines.join("\n")), ext_cfg));
        };

        let mut objects = Vec::<CfeDiffObject>::new();
        for child in child_obj_node.children().filter(|node| node.is_element()) {
            let obj_type = child.tag_name().name();
            if obj_type == "Language" {
                continue;
            }
            objects.push(CfeDiffObject {
                obj_type: obj_type.to_string(),
                name: child.text().unwrap_or("").to_string(),
            });
        }

        if objects.is_empty() {
            lines.push("No objects (besides Language) in extension.".to_string());
            return Ok((format!("{}\n", lines.join("\n")), ext_cfg));
        }

        if mode == "A" {
            cfe_diff_mode_a(&mut lines, &objects, &extension_path);
        } else {
            cfe_diff_mode_b(&mut lines, &objects, &extension_path, &config_path);
        }

        Ok((format!("{}\n", lines.join("\n")), ext_cfg))
    })();

    match result {
        Ok((stdout, artifact)) => AdapterOutcome {
            ok: true,
            summary: "unica.cfe.diff completed with native extension diff analyzer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
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
    }
}

pub(crate) fn cfe_diff_mode_a(
    lines: &mut Vec<String>,
    objects: &[CfeDiffObject],
    extension_path: &Path,
) {
    let mut borrowed = 0usize;
    let mut own = 0usize;

    for obj in objects {
        let Some(info) = cfe_diff_object_info(&obj.obj_type, &obj.name, extension_path) else {
            lines.push(format!(
                "  [?] {}.{} — unknown type",
                obj.obj_type, obj.name
            ));
            continue;
        };
        if !info.exists {
            lines.push(format!(
                "  [?] {}.{} — file not found",
                obj.obj_type, obj.name
            ));
            continue;
        }

        if info.borrowed {
            borrowed += 1;
            lines.push(format!("  [BORROWED] {}.{}", obj.obj_type, obj.name));

            for bsl in cfe_diff_bsl_files(&obj.obj_type, &obj.name, extension_path) {
                let rel_path = cfe_diff_relative_path(&bsl, extension_path);
                let interceptors = cfe_diff_interceptors(&bsl);
                if interceptors.is_empty() {
                    lines.push(format!("             {rel_path} (no interceptors)"));
                } else {
                    for interceptor in interceptors {
                        lines.push(format!(
                            "             &{}(\"{}\") — line {} in {rel_path}",
                            interceptor.interceptor_type, interceptor.method, interceptor.line
                        ));
                    }
                }
            }

            let mut parts = Vec::<String>::new();
            if info.attrs > 0 {
                parts.push(format!("{} own attrs", info.attrs));
            }
            if info.tabular_sections > 0 {
                parts.push(format!("{} own TS", info.tabular_sections));
            }
            if info.forms > 0 {
                parts.push(format!("{} own forms", info.forms));
            }
            if info.borrowed_items > 0 {
                parts.push(format!("{} borrowed items", info.borrowed_items));
            }
            if !parts.is_empty() {
                lines.push(format!("             ChildObjects: {}", parts.join(", ")));
            }

            for form_name in &info.form_names {
                let form_xml_path = extension_path
                    .join(&info.dir_name)
                    .join(&obj.name)
                    .join("Forms")
                    .join(form_name)
                    .join("Ext")
                    .join("Form.xml");
                let Some(form_info) = cfe_diff_form_interceptors(&form_xml_path) else {
                    lines.push(format!("             Form.{form_name} (?)"));
                    continue;
                };
                let form_tag = if form_info.0 { "borrowed" } else { "own" };
                if form_info.1.is_empty() {
                    lines.push(format!("             Form.{form_name} ({form_tag})"));
                } else {
                    lines.push(format!("             Form.{form_name} ({form_tag}):"));
                    for interceptor in form_info.1 {
                        lines.push(format!("               {interceptor}"));
                    }
                }
            }
        } else {
            own += 1;
            lines.push(format!("  [OWN]      {}.{}", obj.obj_type, obj.name));
            let mut parts = Vec::<String>::new();
            if info.attrs > 0 {
                parts.push(format!("{} attrs", info.attrs));
            }
            if info.tabular_sections > 0 {
                parts.push(format!("{} TS", info.tabular_sections));
            }
            if info.forms > 0 {
                parts.push(format!("{} forms", info.forms));
            }
            if !parts.is_empty() {
                lines.push(format!("             {}", parts.join(", ")));
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "=== Summary: {borrowed} borrowed, {own} own objects ==="
    ));
}

pub(crate) fn cfe_diff_mode_b(
    lines: &mut Vec<String>,
    objects: &[CfeDiffObject],
    extension_path: &Path,
    config_path: &Path,
) {
    let mut transferred = 0usize;
    let mut not_transferred = 0usize;
    let mut needs_review = 0usize;

    for obj in objects {
        let Some(info) = cfe_diff_object_info(&obj.obj_type, &obj.name, extension_path) else {
            continue;
        };
        if !info.exists || !info.borrowed {
            continue;
        }

        for bsl in cfe_diff_bsl_files(&obj.obj_type, &obj.name, extension_path) {
            let mac_interceptors = cfe_diff_interceptors(&bsl)
                .into_iter()
                .filter(|item| item.interceptor_type == "ИзменениеИКонтроль")
                .collect::<Vec<_>>();
            if mac_interceptors.is_empty() {
                continue;
            }
            let insert_blocks = cfe_diff_insertion_blocks(&bsl);
            for interceptor in mac_interceptors {
                if insert_blocks.is_empty() {
                    lines.push(format!(
                        "  [NEEDS_REVIEW] {}.{} — &ИзменениеИКонтроль(\"{}\") — no #Вставка blocks",
                        obj.obj_type, obj.name, interceptor.method
                    ));
                    needs_review += 1;
                    continue;
                }

                let rel_path = bsl.strip_prefix(extension_path).unwrap_or(&bsl);
                let config_bsl = config_path.join(rel_path);
                if !config_bsl.is_file() {
                    lines.push(format!(
                        "  [NEEDS_REVIEW] {}.{} — &ИзменениеИКонтроль(\"{}\") — config module not found",
                        obj.obj_type, obj.name, interceptor.method
                    ));
                    needs_review += 1;
                    continue;
                }

                let config_content = read_utf8_sig(&config_bsl).unwrap_or_default();
                let config_norm = cfe_diff_normalized_ws(&config_content);
                let all_transferred = insert_blocks.iter().all(|block| {
                    block.code.is_empty()
                        || config_norm.contains(&cfe_diff_normalized_ws(&block.code))
                });
                if all_transferred {
                    lines.push(format!(
                        "  [TRANSFERRED]     {}.{} — &ИзменениеИКонтроль(\"{}\") — {} block(s)",
                        obj.obj_type,
                        obj.name,
                        interceptor.method,
                        insert_blocks.len()
                    ));
                    transferred += 1;
                } else {
                    lines.push(format!(
                        "  [NOT_TRANSFERRED] {}.{} — &ИзменениеИКонтроль(\"{}\") — some blocks not found in config",
                        obj.obj_type, obj.name, interceptor.method
                    ));
                    not_transferred += 1;
                }
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "=== Transfer check: {transferred} transferred, {not_transferred} not transferred, {needs_review} needs review ==="
    ));
}

pub(crate) fn cfe_diff_object_info(
    obj_type: &str,
    obj_name: &str,
    extension_path: &Path,
) -> Option<CfeDiffObjectInfo> {
    let dir_name = cf_validate_child_type_dir(obj_type)?;
    let obj_file = extension_path
        .join(dir_name)
        .join(format!("{obj_name}.xml"));
    if !obj_file.is_file() {
        return Some(CfeDiffObjectInfo {
            borrowed: false,
            exists: false,
            dir_name: dir_name.to_string(),
            attrs: 0,
            forms: 0,
            tabular_sections: 0,
            borrowed_items: 0,
            form_names: Vec::new(),
        });
    }

    let text = read_utf8_sig(&obj_file).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let obj_el = doc
        .root_element()
        .children()
        .find(|child| child.is_element())?;
    let props_el = meta_info_child(obj_el, "Properties");
    let borrowed = props_el
        .and_then(|props| meta_info_child_text(props, "ObjectBelonging"))
        .map(|value| value == "Adopted")
        .unwrap_or(false);

    let mut attrs = 0usize;
    let mut forms = 0usize;
    let mut tabular_sections = 0usize;
    let mut borrowed_items = 0usize;
    let mut form_names = Vec::<String>::new();
    if let Some(child_objects) = meta_info_child(obj_el, "ChildObjects") {
        for child in child_objects.children().filter(|node| node.is_element()) {
            if borrowed {
                let child_borrowed = meta_info_child(child, "Properties")
                    .and_then(|props| meta_info_child_text(props, "ObjectBelonging"))
                    .map(|value| value == "Adopted")
                    .unwrap_or(false);
                if child_borrowed {
                    borrowed_items += 1;
                    continue;
                }
            }
            match child.tag_name().name() {
                "Attribute" => attrs += 1,
                "TabularSection" => tabular_sections += 1,
                "Form" => {
                    forms += 1;
                    if borrowed {
                        form_names.push(child.text().unwrap_or("").to_string());
                    }
                }
                _ => {}
            }
        }
    }

    Some(CfeDiffObjectInfo {
        borrowed,
        exists: true,
        dir_name: dir_name.to_string(),
        attrs,
        forms,
        tabular_sections,
        borrowed_items,
        form_names,
    })
}

pub(crate) fn cfe_diff_bsl_files(
    obj_type: &str,
    obj_name: &str,
    extension_path: &Path,
) -> Vec<PathBuf> {
    let Some(dir_name) = cf_validate_child_type_dir(obj_type) else {
        return Vec::new();
    };
    let obj_dir = extension_path.join(dir_name).join(obj_name);
    if !obj_dir.is_dir() {
        return Vec::new();
    }

    let mut result = Vec::<PathBuf>::new();
    let ext_dir = obj_dir.join("Ext");
    if let Ok(entries) = fs::read_dir(ext_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|value| value.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("bsl"))
                .unwrap_or(false)
            {
                result.push(path);
            }
        }
    }
    let forms_dir = obj_dir.join("Forms");
    cfe_diff_collect_form_modules(&forms_dir, &mut result);
    result.sort();
    result
}

pub(crate) fn cfe_diff_collect_form_modules(dir: &Path, result: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cfe_diff_collect_form_modules(&path, result);
        } else if path.file_name().and_then(|value| value.to_str()) == Some("Module.bsl") {
            result.push(path);
        }
    }
}

pub(crate) fn cfe_diff_interceptors(bsl_path: &Path) -> Vec<CfeDiffInterceptor> {
    let Ok(text) = read_utf8_sig(bsl_path) else {
        return Vec::new();
    };
    let mut result = Vec::<CfeDiffInterceptor>::new();
    for (idx, line) in text.lines().enumerate() {
        let stripped = line.trim();
        for interceptor_type in ["Перед", "После", "ИзменениеИКонтроль", "Вместо"]
        {
            let prefix = format!("&{interceptor_type}(\"");
            if let Some(rest) = stripped.strip_prefix(&prefix) {
                if let Some(end) = rest.find("\")") {
                    result.push(CfeDiffInterceptor {
                        interceptor_type: interceptor_type.to_string(),
                        method: rest[..end].to_string(),
                        line: idx + 1,
                    });
                }
            }
        }
    }
    result
}

pub(crate) fn cfe_diff_insertion_blocks(bsl_path: &Path) -> Vec<CfeDiffInsertionBlock> {
    let Ok(text) = read_utf8_sig(bsl_path) else {
        return Vec::new();
    };
    let mut blocks = Vec::<CfeDiffInsertionBlock>::new();
    let mut in_block = false;
    let mut block_lines = Vec::<String>::new();
    for line in text.lines() {
        let stripped = line.trim();
        if stripped == "#Вставка" {
            in_block = true;
            block_lines.clear();
        } else if stripped == "#КонецВставки" && in_block {
            in_block = false;
            blocks.push(CfeDiffInsertionBlock {
                code: block_lines.join("\n").trim().to_string(),
            });
        } else if in_block {
            block_lines.push(line.trim_end_matches('\r').to_string());
        }
    }
    blocks
}

pub(crate) fn cfe_diff_form_interceptors(form_xml_path: &Path) -> Option<(bool, Vec<String>)> {
    const FORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
    let text = read_utf8_sig(form_xml_path).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    let is_borrowed = dcs_child(root, "BaseForm", FORM_NS).is_some();
    let mut interceptors = Vec::<String>::new();

    if let Some(events) = dcs_child(root, "Events", FORM_NS) {
        for event in dcs_children(events, "Event", FORM_NS) {
            let call_type = event.attribute("callType").unwrap_or("");
            if !call_type.is_empty() {
                let event_name = event.attribute("name").unwrap_or("");
                let event_text = event.text().unwrap_or("");
                interceptors.push(format!("Event:{event_name} [{call_type}] -> {event_text}"));
            }
        }
    }

    if let Some(child_items) = dcs_child(root, "ChildItems", FORM_NS) {
        for element in child_items.descendants().filter(|node| node.is_element()) {
            let element_name = element.attribute("name").unwrap_or("");
            if element_name.is_empty() {
                continue;
            }
            let Some(events) = dcs_child(element, "Events", FORM_NS) else {
                continue;
            };
            for event in dcs_children(events, "Event", FORM_NS) {
                let call_type = event.attribute("callType").unwrap_or("");
                if !call_type.is_empty() {
                    let event_name = event.attribute("name").unwrap_or("");
                    let event_text = event.text().unwrap_or("");
                    interceptors.push(format!(
                        "Element:{element_name}.{event_name} [{call_type}] -> {event_text}"
                    ));
                }
            }
        }
    }

    if let Some(commands) = dcs_child(root, "Commands", FORM_NS) {
        for command in dcs_children(commands, "Command", FORM_NS) {
            let command_name = command.attribute("name").unwrap_or("");
            for action in dcs_children(command, "Action", FORM_NS) {
                let call_type = action.attribute("callType").unwrap_or("");
                if !call_type.is_empty() {
                    let action_text = action.text().unwrap_or("");
                    interceptors.push(format!(
                        "Command:{command_name} [{call_type}] -> {action_text}"
                    ));
                }
            }
        }
    }

    Some((is_borrowed, interceptors))
}

pub(crate) fn cfe_diff_relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(crate) fn cfe_diff_normalized_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn validate_cfe(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<CfeValidationRun, String> {
        let resolved_path = resolve_cfe_validate_config_path(args, context)?;
        let config_dir = resolved_path.parent().unwrap_or(context.cwd.as_path());
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));
        let detailed = bool_arg(args, &["detailed", "Detailed"]);
        let max_errors = int_arg(args, &["maxErrors", "MaxErrors"])
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(30);

        let text = read_utf8_sig(&resolved_path)?;
        let source = text.trim_start_matches('\u{feff}');
        let doc = match Document::parse(source) {
            Ok(doc) => doc,
            Err(err) => {
                let mut report = CfeValidationReporter::new(max_errors, detailed);
                report.lines.insert(
                    0,
                    "=== Validation: Extension (parse failed) ===".to_string(),
                );
                report.out("");
                report.error(format!("1. XML parse failed: {err}"));
                let (ok, stdout, errors) = report.finalize();
                return Ok(CfeValidationRun {
                    ok,
                    stdout,
                    out_file,
                    artifact: resolved_path,
                    errors,
                });
            }
        };

        let root = doc.root_element();
        let mut report = CfeValidationReporter::new(max_errors, detailed);
        report.out("");

        let root_local = root.tag_name().name();
        let root_ns = root.tag_name().namespace().unwrap_or("");
        if root_local != "MetaDataObject" {
            report.error(format!(
                "1. Root element is '{root_local}', expected 'MetaDataObject'"
            ));
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfeValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        }

        let mut check1_ok = true;
        if root_ns != MD_NS {
            report.error(format!(
                "1. Root namespace is '{root_ns}', expected '{MD_NS}'"
            ));
            check1_ok = false;
        }
        let version_literal = root_version_literal(source, root);
        match classify_root_version(version_literal.as_deref()) {
            Ok(FormatCompatibility::Supported { .. }) => report.ok("Export format: 2.20"),
            Ok(compatibility) => report.warn(format_compatibility_warning(&compatibility)),
            Err(error) => report.error(error.to_string()),
        }
        let version = version_literal.as_deref().unwrap_or("");

        let Some(cfg_node) = root
            .children()
            .find(|child| role_info_element(*child, "Configuration", Some(MD_NS)))
        else {
            report.error("1. No <Configuration> element found inside MetaDataObject");
            let (ok, stdout, errors) = report.finalize();
            return Ok(CfeValidationRun {
                ok,
                stdout,
                out_file,
                artifact: resolved_path,
                errors,
            });
        };

        let cfg_uuid = cfg_node.attribute("uuid").unwrap_or("");
        if cfg_uuid.is_empty() {
            report.error("1. Missing uuid on <Configuration>");
            check1_ok = false;
        } else if !cf_validate_guid(cfg_uuid) {
            report.error(format!("1. Invalid uuid '{cfg_uuid}' on <Configuration>"));
            check1_ok = false;
        }

        let props_node = meta_info_child(cfg_node, "Properties");
        let obj_name = props_node
            .and_then(|props| meta_info_child_text(props, "Name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unknown)".to_string());
        report.obj_name = obj_name.clone();
        report
            .lines
            .insert(0, format!("=== Validation: Extension.{obj_name} ==="));
        if check1_ok {
            report.ok(format!(
                "1. Root structure: MetaDataObject/Configuration, version {version}"
            ));
        }
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }

        cfe_validate_internal_info(&mut report, cfg_node);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        let def_lang = cfe_validate_properties(&mut report, props_node, &obj_name);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_enum_properties(&mut report, props_node);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }

        let child_obj_node = meta_info_child(cfg_node, "ChildObjects");
        let child_index = cfe_validate_child_objects(&mut report, child_obj_node);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_default_language(&mut report, child_obj_node, &def_lang);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_language_files(&mut report, child_obj_node, config_dir);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_object_dirs(&mut report, child_obj_node, config_dir);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        let borrowed_forms =
            cfe_validate_borrowed_objects(&mut report, child_obj_node, config_dir, &child_index);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_borrowed_forms(&mut report, &borrowed_forms);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_form_dependencies(&mut report, &borrowed_forms, &child_index);
        if report.stopped {
            return cfe_validation_finish(report, out_file, resolved_path);
        }
        cfe_validate_typelinks(&mut report, &borrowed_forms);

        cfe_validation_finish(report, out_file, resolved_path)
    })();

    match result {
        Ok(run) => {
            let mut stdout = run.stdout.clone();
            let mut artifacts = vec![run.artifact.display().to_string()];
            if let Some(out_file) = &run.out_file {
                match write_utf8_bom(out_file, run.stdout.trim_end_matches('\n')) {
                    Ok(()) => {
                        stdout.push_str(&format!("Written to: {}\n", out_file.display()));
                        artifacts.push(out_file.display().to_string());
                    }
                    Err(error) => {
                        return AdapterOutcome {
                            ok: false,
                            summary: "unica.cfe.validate failed in native extension validator"
                                .to_string(),
                            changes: Vec::new(),
                            warnings: Vec::new(),
                            errors: vec![error.clone()],
                            artifacts,
                            stdout: None,
                            stderr: Some(format!("{error}\n")),
                            command: None,
                        };
                    }
                }
            }
            AdapterOutcome {
                ok: run.ok,
                summary: if run.ok {
                    "unica.cfe.validate completed with native extension validator".to_string()
                } else {
                    "unica.cfe.validate failed in native extension validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: run.errors,
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
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

pub(crate) fn cfe_validation_finish(
    report: CfeValidationReporter,
    out_file: Option<PathBuf>,
    artifact: PathBuf,
) -> Result<CfeValidationRun, String> {
    let (ok, stdout, errors) = report.finalize();
    Ok(CfeValidationRun {
        ok,
        stdout,
        out_file,
        artifact,
        errors,
    })
}

pub(crate) fn cfe_validate_internal_info(
    report: &mut CfeValidationReporter,
    cfg_node: roxmltree::Node<'_, '_>,
) {
    let Some(internal_info) = meta_info_child(cfg_node, "InternalInfo") else {
        report.error("2. InternalInfo: missing");
        return;
    };
    let contained = meta_info_children(internal_info, "ContainedObject");
    if contained.len() != 7 {
        report.warn(format!(
            "2. InternalInfo: expected 7 ContainedObject, found {}",
            contained.len()
        ));
    }
    let mut check_ok = true;
    let mut found = HashSet::<String>::new();
    for item in &contained {
        let class_id = meta_info_child_text(*item, "ClassId").unwrap_or_default();
        let object_id = meta_info_child_text(*item, "ObjectId").unwrap_or_default();
        if class_id.is_empty() {
            report.error("2. ContainedObject missing ClassId");
            check_ok = false;
            continue;
        }
        if !cf_validate_class_ids().contains(&class_id.as_str()) {
            report.error(format!("2. Unknown ClassId: {class_id}"));
            check_ok = false;
        }
        if !found.insert(class_id.clone()) {
            report.error(format!("2. Duplicate ClassId: {class_id}"));
            check_ok = false;
        }
        if object_id.is_empty() {
            report.error(format!(
                "2. ContainedObject missing ObjectId for ClassId {class_id}"
            ));
            check_ok = false;
        } else if !cf_validate_guid(&object_id) {
            report.error(format!(
                "2. Invalid ObjectId '{object_id}' for ClassId {class_id}"
            ));
            check_ok = false;
        }
    }
    let missing = cf_validate_class_ids()
        .iter()
        .filter(|class_id| !found.contains(**class_id))
        .count();
    if missing > 0 {
        report.warn(format!("2. Missing ClassIds: {missing} of 7"));
    }
    if check_ok {
        report.ok(format!(
            "2. InternalInfo: {} ContainedObject, all ClassIds valid",
            contained.len()
        ));
    }
}

pub(crate) fn cfe_validate_properties(
    report: &mut CfeValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
) -> String {
    let Some(props_node) = props_node else {
        report.error("3. Properties block missing");
        return String::new();
    };
    let mut check_ok = true;
    let object_belonging = meta_info_child_text(props_node, "ObjectBelonging").unwrap_or_default();
    if object_belonging != "Adopted" {
        report.error(format!(
            "3. ObjectBelonging must be 'Adopted', got '{object_belonging}'"
        ));
        check_ok = false;
    }
    if obj_name == "(unknown)" || obj_name.is_empty() {
        report.error("3. Name is missing or empty");
        check_ok = false;
    } else if !cf_validate_identifier(obj_name) {
        report.error(format!("3. Name '{obj_name}' is not a valid 1C identifier"));
        check_ok = false;
    }
    let purpose =
        meta_info_child_text(props_node, "ConfigurationExtensionPurpose").unwrap_or_default();
    if purpose.is_empty() {
        report.error("3. ConfigurationExtensionPurpose is missing");
        check_ok = false;
    } else if !["Patch", "Customization", "AddOn"].contains(&purpose.as_str()) {
        report.error(format!(
            "3. ConfigurationExtensionPurpose '{purpose}' invalid (expected: Patch, Customization, AddOn)"
        ));
        check_ok = false;
    }
    let prefix = meta_info_child_text(props_node, "NamePrefix").unwrap_or_default();
    if prefix.is_empty() {
        report.warn("3. NamePrefix is empty");
    }
    if meta_info_child(props_node, "KeepMappingToExtendedConfigurationObjectsByIDs").is_none() {
        report.warn("3. KeepMappingToExtendedConfigurationObjectsByIDs is missing");
    }
    let def_lang = meta_info_child_text(props_node, "DefaultLanguage").unwrap_or_default();
    if check_ok {
        let prefix_text = if prefix.is_empty() {
            "(empty)"
        } else {
            prefix.as_str()
        };
        let purpose_text = if purpose.is_empty() {
            "?"
        } else {
            purpose.as_str()
        };
        report.ok(format!(
            "3. Extension properties: Name=\"{obj_name}\", Purpose={purpose_text}, Prefix={prefix_text}"
        ));
    }
    def_lang
}

pub(crate) fn cfe_validate_enum_properties(
    report: &mut CfeValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        report.warn("4. No Properties block to check");
        return;
    };
    let mut checked = 0usize;
    let mut check_ok = true;
    for property in cfe_validate_enum_properties_list() {
        let allowed = cfe_validate_enum_allowed(property);
        if let Some(value) =
            meta_info_child_text(props_node, property).filter(|value| !value.is_empty())
        {
            if !allowed.contains(&value.as_str()) {
                report.error(format!(
                    "4. Property '{property}' has invalid value '{value}'"
                ));
                check_ok = false;
            }
            checked += 1;
        }
    }
    if check_ok {
        report.ok(format!(
            "4. Property values: {checked} enum properties checked"
        ));
    }
}

pub(crate) fn cfe_validate_child_objects(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) -> HashMap<String, HashSet<String>> {
    let mut child_index = HashMap::<String, HashSet<String>>::new();
    let Some(child_obj_node) = child_obj_node else {
        report.error("5. ChildObjects block missing");
        return child_index;
    };
    let mut check_ok = true;
    let mut total_count = 0usize;
    let mut duplicates = HashSet::<String>::new();
    let mut last_type_order = 0usize;
    let mut order_ok = true;
    let mut first_type = true;
    for child in child_obj_node.children().filter(|child| child.is_element()) {
        let type_name = child.tag_name().name();
        let object_name = child.text().unwrap_or("").to_string();
        if let Some(type_index) = cf_validate_child_object_type_index(type_name) {
            if !first_type && type_index < last_type_order {
                report.warn(format!(
                    "5. Type '{type_name}' is out of canonical order (after type at position {last_type_order})"
                ));
                order_ok = false;
            }
            if first_type || type_index >= last_type_order {
                last_type_order = type_index;
            }
            first_type = false;
        } else {
            report.error(format!("5. Unknown type '{type_name}' in ChildObjects"));
            check_ok = false;
        }
        let type_items = child_index.entry(type_name.to_string()).or_default();
        if !type_items.insert(object_name.clone()) {
            let dup_key = format!("{type_name}.{object_name}");
            if duplicates.insert(dup_key.clone()) {
                report.error(format!("5. Duplicate: {dup_key}"));
                check_ok = false;
            }
        }
        total_count += 1;
    }
    if check_ok {
        let order_info = if order_ok { ", order correct" } else { "" };
        report.ok(format!(
            "5. ChildObjects: {} types, {total_count} objects{order_info}",
            child_index.len()
        ));
    }
    child_index
}

pub(crate) fn cfe_validate_default_language(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    def_lang: &str,
) {
    if def_lang.is_empty() {
        report.warn("6. Cannot check DefaultLanguage (empty)");
        return;
    }
    let Some(child_obj_node) = child_obj_node else {
        report.warn("6. Cannot check DefaultLanguage (no ChildObjects)");
        return;
    };
    let lang_name = def_lang.strip_prefix("Language.").unwrap_or(def_lang);
    let found = meta_info_children(child_obj_node, "Language")
        .iter()
        .any(|child| child.text().unwrap_or("") == lang_name);
    if found {
        report.ok(format!(
            "6. DefaultLanguage \"{def_lang}\" found in ChildObjects"
        ));
    } else {
        report.error(format!(
            "6. DefaultLanguage \"{def_lang}\" not found in ChildObjects"
        ));
    }
}

pub(crate) fn cfe_validate_language_files(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
) {
    let Some(child_obj_node) = child_obj_node else {
        report.warn("7. Cannot check language files (no ChildObjects)");
        return;
    };
    let lang_names = meta_info_children(child_obj_node, "Language")
        .into_iter()
        .map(|child| child.text().unwrap_or("").to_string())
        .collect::<Vec<_>>();
    if lang_names.is_empty() {
        report.warn("7. No Language entries in ChildObjects");
        return;
    }
    let mut exist_count = 0usize;
    for lang_name in &lang_names {
        if config_dir
            .join("Languages")
            .join(format!("{lang_name}.xml"))
            .exists()
        {
            exist_count += 1;
        } else {
            report.warn(format!(
                "7. Language file missing: Languages/{lang_name}.xml"
            ));
        }
    }
    if exist_count == lang_names.len() {
        report.ok(format!(
            "7. Language files: {exist_count}/{} exist",
            lang_names.len()
        ));
    }
}

pub(crate) fn cfe_validate_object_dirs(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut dirs = HashMap::<String, usize>::new();
    for child in child_obj_node.children().filter(|child| child.is_element()) {
        let type_name = child.tag_name().name();
        if type_name == "Language" {
            continue;
        }
        if let Some(dir_name) = cf_validate_child_type_dir(type_name) {
            *dirs.entry(dir_name.to_string()).or_default() += 1;
        }
    }
    let mut missing = dirs
        .iter()
        .filter(|(dir_name, _)| !config_dir.join(*dir_name).is_dir())
        .map(|(dir_name, count)| format!("{dir_name} ({count} objects)"))
        .collect::<Vec<_>>();
    missing.sort();
    if missing.is_empty() {
        report.ok(format!(
            "8. Object directories: {} directories, all exist",
            dirs.len()
        ));
    } else {
        for missing_dir in missing {
            report.warn(format!("8. Missing directory: {missing_dir}"));
        }
    }
}

pub(crate) struct CfeBorrowedForm {
    pub(crate) raw_text: String,
    pub(crate) context: String,
}

pub(crate) fn cfe_validate_borrowed_objects(
    report: &mut CfeValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: &Path,
    _child_index: &HashMap<String, HashSet<String>>,
) -> Vec<CfeBorrowedForm> {
    let mut forms = Vec::new();
    let Some(child_obj_node) = child_obj_node else {
        return forms;
    };
    let mut borrowed_count = 0usize;
    let mut borrowed_ok_count = 0usize;
    let mut check9_ok = true;
    let mut check10_ok = true;
    let mut sub_item_count = 0usize;
    for child in child_obj_node.children().filter(|child| child.is_element()) {
        let type_name = child.tag_name().name();
        let child_name = child.text().unwrap_or("");
        if type_name == "Language" {
            continue;
        }
        let Some(dir_name) = cf_validate_child_type_dir(type_name) else {
            continue;
        };
        let obj_file = config_dir.join(dir_name).join(format!("{child_name}.xml"));
        if !obj_file.exists() {
            continue;
        }
        let Ok(text) = read_utf8_sig(&obj_file) else {
            continue;
        };
        let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            report.warn(format!("9. Cannot parse {dir_name}/{child_name}.xml"));
            continue;
        };
        let Some(obj_el) = doc.root_element().children().find(|node| node.is_element()) else {
            continue;
        };
        let Some(obj_props) = meta_info_child(obj_el, "Properties") else {
            continue;
        };
        if meta_info_child_text(obj_props, "ObjectBelonging").as_deref() == Some("Adopted") {
            borrowed_count += 1;
            let extended =
                meta_info_child_text(obj_props, "ExtendedConfigurationObject").unwrap_or_default();
            if extended.is_empty() {
                report.error(format!(
                    "9. Borrowed {type_name}.{child_name}: missing ExtendedConfigurationObject"
                ));
                check9_ok = false;
            } else if !cf_validate_guid(&extended) {
                report.error(format!(
                    "9. Borrowed {type_name}.{child_name}: invalid ExtendedConfigurationObject UUID '{extended}'"
                ));
                check9_ok = false;
            } else {
                borrowed_ok_count += 1;
            }
        }
        if let Some(child_objects) = meta_info_child(obj_el, "ChildObjects") {
            let context = format!("{type_name}.{child_name}");
            for sub_item in child_objects.children().filter(|node| node.is_element()) {
                let sub_type = sub_item.tag_name().name();
                if matches!(sub_type, "Attribute" | "TabularSection" | "EnumValue")
                    && cfe_is_borrowed_sub_item(sub_item)
                {
                    sub_item_count += 1;
                    if !cfe_validate_borrowed_sub_item(report, "10", &context, sub_type, sub_item) {
                        check10_ok = false;
                    }
                } else if sub_type == "Form" {
                    let form_name = sub_item.text().unwrap_or("");
                    if !form_name.is_empty() {
                        let form_meta = config_dir
                            .join(dir_name)
                            .join(child_name)
                            .join("Forms")
                            .join(format!("{form_name}.xml"));
                        if !form_meta.exists() {
                            report.error(format!(
                                "10. {context}: Form.{form_name} metadata file missing"
                            ));
                            check10_ok = false;
                        }
                        let form_xml = config_dir
                            .join(dir_name)
                            .join(child_name)
                            .join("Forms")
                            .join(form_name)
                            .join("Ext")
                            .join("Form.xml");
                        if let Ok(raw_text) = read_utf8_sig(&form_xml) {
                            forms.push(CfeBorrowedForm {
                                raw_text,
                                context: format!("{context}.Form.{form_name}"),
                            });
                        }
                        sub_item_count += 1;
                    }
                }
            }
        }
    }
    if borrowed_count == 0 {
        report.ok("9. Borrowed objects: none found");
    } else if check9_ok {
        report.ok(format!(
            "9. Borrowed objects: {borrowed_ok_count}/{borrowed_count} validated"
        ));
    }
    if sub_item_count == 0 {
        report.ok("10. Sub-items: none found");
    } else if check10_ok {
        report.ok(format!(
            "10. Sub-items: {sub_item_count} validated (Attributes, TabularSections, EnumValues, Forms)"
        ));
    }
    forms
}

pub(crate) fn cfe_is_borrowed_sub_item(sub_item: roxmltree::Node<'_, '_>) -> bool {
    let Some(props) = meta_info_child(sub_item, "Properties") else {
        return false;
    };
    meta_info_child_text(props, "ObjectBelonging").is_some_and(|value| !value.is_empty())
        || meta_info_child_text(props, "ExtendedConfigurationObject")
            .is_some_and(|value| !value.is_empty())
}

pub(crate) fn cfe_validate_borrowed_sub_item(
    report: &mut CfeValidationReporter,
    check_num: &str,
    context: &str,
    sub_type: &str,
    sub_item: roxmltree::Node<'_, '_>,
) -> bool {
    let Some(props) = meta_info_child(sub_item, "Properties") else {
        report.error(format!(
            "{check_num}. {context}: {sub_type} missing Properties"
        ));
        return false;
    };
    let mut ok = true;
    if meta_info_child_text(props, "ObjectBelonging").as_deref() != Some("Adopted") {
        report.error(format!(
            "{check_num}. {context}: {sub_type} ObjectBelonging must be 'Adopted'"
        ));
        ok = false;
    }
    let name = meta_info_child_text(props, "Name").unwrap_or_default();
    if name.is_empty() {
        report.error(format!("{check_num}. {context}: {sub_type} missing Name"));
        ok = false;
    }
    let extended = meta_info_child_text(props, "ExtendedConfigurationObject").unwrap_or_default();
    if extended.is_empty() {
        report.error(format!(
            "{check_num}. {context}: {sub_type}.{name} missing ExtendedConfigurationObject"
        ));
        ok = false;
    } else if !cf_validate_guid(&extended) {
        report.error(format!(
            "{check_num}. {context}: {sub_type}.{name} invalid ExtendedConfigurationObject"
        ));
        ok = false;
    }
    ok
}

pub(crate) fn cfe_validate_borrowed_forms(
    report: &mut CfeValidationReporter,
    borrowed_forms: &[CfeBorrowedForm],
) {
    if borrowed_forms.is_empty() {
        report.ok("11. Borrowed forms: none found");
    } else {
        let with_base = borrowed_forms
            .iter()
            .filter(|form| form.raw_text.contains("<BaseForm"))
            .count();
        report.ok(format!(
            "11. Borrowed forms: {} validated ({with_base} with BaseForm)",
            borrowed_forms.len()
        ));
    }
}

pub(crate) fn cfe_validate_form_dependencies(
    report: &mut CfeValidationReporter,
    borrowed_forms: &[CfeBorrowedForm],
    child_index: &HashMap<String, HashSet<String>>,
) {
    if borrowed_forms.is_empty() {
        report.ok("12. Form dependencies: no borrowed forms with tree");
        return;
    }
    let mut check_ok = true;
    let mut dep_count = 0usize;
    for form in borrowed_forms {
        for picture in cfe_capture_refs(&form.raw_text, "<xr:Ref>CommonPicture.", "</xr:Ref>") {
            dep_count += 1;
            if !child_index
                .get("CommonPicture")
                .is_some_and(|items| items.contains(&picture))
            {
                report.warn(format!(
                    "12. {}: references CommonPicture.{picture} not borrowed in extension",
                    form.context
                ));
                check_ok = false;
            }
        }
    }
    if check_ok {
        report.ok(format!(
            "12. Form dependencies: {dep_count} references checked"
        ));
    }
}

pub(crate) fn cfe_validate_typelinks(
    report: &mut CfeValidationReporter,
    borrowed_forms: &[CfeBorrowedForm],
) {
    if borrowed_forms.is_empty() {
        report.ok("13. TypeLink: no borrowed forms with tree");
        return;
    }
    let mut check_ok = true;
    for form in borrowed_forms {
        let count = form
            .raw_text
            .matches("<TypeLink>")
            .filter(|_| form.raw_text.contains("<xr:DataPath>Items."))
            .count();
        if count > 0 {
            report.warn(format!(
                "13. {}: {count} TypeLink(s) with human-readable Items.* DataPath (should be stripped)",
                form.context
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("13. TypeLink: clean");
    }
}

pub(crate) fn cfe_capture_refs(raw: &str, prefix: &str, suffix: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find(prefix) {
        let value_start = start + prefix.len();
        let Some(end) = rest[value_start..].find(suffix) else {
            break;
        };
        values.push(rest[value_start..value_start + end].to_string());
        rest = &rest[value_start + end + suffix.len()..];
    }
    values
}

pub(crate) fn cfe_validate_enum_properties_list() -> &'static [&'static str] {
    &[
        "ConfigurationExtensionCompatibilityMode",
        "DefaultRunMode",
        "ScriptVariant",
        "InterfaceCompatibilityMode",
    ]
}

pub(crate) fn cfe_validate_enum_allowed(property: &str) -> &'static [&'static str] {
    match property {
        "ConfigurationExtensionCompatibilityMode" => {
            cf_validate_enum_allowed("ConfigurationExtensionCompatibilityMode")
        }
        "DefaultRunMode" => &["ManagedApplication", "OrdinaryApplication", "Auto"],
        "ScriptVariant" => &["Russian", "English"],
        "InterfaceCompatibilityMode" => &[
            "Version8_2",
            "Version8_2EnableTaxi",
            "Taxi",
            "TaxiEnableVersion8_2",
        ],
        _ => &[],
    }
}

fn cfe_init_validate_enum(property: &str, value: &str) -> Result<(), String> {
    let allowed = cfe_validate_enum_allowed(property);
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{property} value {value:?} is not valid for 8.3.27; expected one of: {}",
            allowed.join(", ")
        ))
    }
}

#[cfg(test)]
std::thread_local! {
    static TEST_CFE_INIT_SEMANTIC_POST_VALIDATION_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
fn with_cfe_init_semantic_post_validation_failure<T>(action: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_CFE_INIT_SEMANTIC_POST_VALIDATION_FAILURE.with(|slot| slot.set(self.0));
        }
    }

    let previous = TEST_CFE_INIT_SEMANTIC_POST_VALIDATION_FAILURE.with(|slot| slot.replace(true));
    let _reset = Reset(previous);
    action()
}

fn cfe_init_validate_post_state(
    config_path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    #[cfg(test)]
    if TEST_CFE_INIT_SEMANTIC_POST_VALIDATION_FAILURE.with(|slot| slot.get()) {
        return Err("injected cfe.init semantic post-validation failure".to_string());
    }

    cfe_borrow_validate_extension(config_path, context)
}

#[derive(Debug, Clone)]
pub(crate) struct CfeInitPlannedXml {
    pub(crate) output_dir: PathBuf,
    pub(crate) configuration: PathBuf,
    pub(crate) language: PathBuf,
    pub(crate) role: Option<PathBuf>,
}

fn cfe_init_name_prefix(args: &Map<String, Value>, name: &str) -> String {
    string_arg(args, &["namePrefix", "NamePrefix"])
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{name}_"))
}

pub(crate) fn cfe_init_planned_xml(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CfeInitPlannedXml {
    let name = string_arg(args, &["name", "Name"]).unwrap_or("");
    let name_prefix = cfe_init_name_prefix(args, name);
    let output_dir = output_dir_arg(
        args,
        context,
        &["outputDir", "OutputDir", "extensionPath", "ExtensionPath"],
        "src",
    );
    let role = (!bool_arg(args, &["noRole", "NoRole"])).then(|| {
        output_dir
            .join("Roles")
            .join(format!("{name_prefix}ОсновнаяРоль.xml"))
    });
    CfeInitPlannedXml {
        configuration: output_dir.join("Configuration.xml"),
        language: output_dir.join("Languages/Русский.xml"),
        role,
        output_dir,
    }
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
        let compatibility = (|| {
            let text = std::str::from_utf8(raw)
                .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
            let source = text.trim_start_matches('\u{feff}');
            let document = Document::parse(source).map_err(|error| {
                format!("failed to parse platform XML {}: {error}", path.display())
            })?;
            let version_literal = root_version_literal(source, document.root_element());
            classify_root_version(version_literal.as_deref()).map_err(|error| error.to_string())
        })();
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
        return Err(format_compatibility_warning(&compatibility));
    }
    for (path, raw) in snapshots {
        transaction.guard_or_verify_exact_preimage(path, raw)?;
    }
    Ok(())
}

const CFE_PATCH_MD_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const CFE_PATCH_FORM_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/logform";
const CFE_PATCH_XR_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/readable";
const CFE_PATCH_NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CfePatchModuleRole {
    CommonModule,
    ObjectModule,
    ManagerModule,
    RecordSetModule,
    ValueManagerModule,
    Form,
}

impl CfePatchModuleRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::CommonModule => "CommonModule",
            Self::ObjectModule => "ObjectModule",
            Self::ManagerModule => "ManagerModule",
            Self::RecordSetModule => "RecordSetModule",
            Self::ValueManagerModule => "ValueManagerModule",
            Self::Form => "Form",
        }
    }

    fn extended_property(self) -> &'static str {
        match self {
            Self::CommonModule => "Module",
            Self::ObjectModule => "ObjectModule",
            Self::ManagerModule => "ManagerModule",
            Self::RecordSetModule => "RecordSetModule",
            Self::ValueManagerModule => "ValueManagerModule",
            Self::Form => "Form",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CfePatchModuleTarget {
    type_name: &'static str,
    object_name: String,
    role: CfePatchModuleRole,
    form_name: Option<String>,
    descriptor: PathBuf,
    bsl_file: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CfePatchCommonModuleCapabilities {
    client_managed_application: bool,
    server: bool,
    external_connection: bool,
    client_ordinary_application: bool,
}

impl CfePatchCommonModuleCapabilities {
    fn client(self) -> bool {
        self.client_managed_application || self.client_ordinary_application
    }

    fn any_execution_context(self) -> bool {
        self.client() || self.server || self.external_connection
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CfePatchBorrowedPrecondition {
    snapshots: BTreeMap<PathBuf, Vec<u8>>,
    name_prefix: String,
    common_module_capabilities: Option<CfePatchCommonModuleCapabilities>,
    property_state_descriptor: PathBuf,
}

fn cfe_patch_module_target(
    extension_path: &Path,
    module_path: &str,
) -> Result<CfePatchModuleTarget, String> {
    let parts = module_path.split('.').collect::<Vec<_>>();
    let invalid_format = || {
        format!(
            "Invalid ModulePath format: {module_path}. Expected exactly Type.Name.Module, Type.Name.Form.FormName, or CommonModule.Name"
        )
    };

    match parts.as_slice() {
        ["CommonModule", object_name] => {
            cfe_validate_metadata_name("ModulePath object name", object_name)?;
            Ok(CfePatchModuleTarget {
                type_name: "CommonModule",
                object_name: (*object_name).to_string(),
                role: CfePatchModuleRole::CommonModule,
                form_name: None,
                descriptor: extension_path
                    .join("CommonModules")
                    .join(format!("{object_name}.xml")),
                bsl_file: extension_path
                    .join("CommonModules")
                    .join(object_name)
                    .join("Ext")
                    .join("Module.bsl"),
            })
        }
        [type_name, object_name, "Form", form_name] => {
            cfe_validate_metadata_name("ModulePath object name", object_name)?;
            cfe_validate_metadata_name("ModulePath form name", form_name)?;
            if !cfe_patch_form_role_is_supported(type_name) {
                return Err(cfe_patch_unsupported_role(module_path));
            }
            let directory = cf_validate_child_type_dir(type_name)
                .ok_or_else(|| format!("Unknown object type: {type_name}"))?;
            Ok(CfePatchModuleTarget {
                type_name: cfe_patch_canonical_type(type_name)
                    .ok_or_else(|| cfe_patch_unsupported_role(module_path))?,
                object_name: (*object_name).to_string(),
                role: CfePatchModuleRole::Form,
                form_name: Some((*form_name).to_string()),
                descriptor: extension_path
                    .join(directory)
                    .join(format!("{object_name}.xml")),
                bsl_file: extension_path
                    .join(directory)
                    .join(object_name)
                    .join("Forms")
                    .join(form_name)
                    .join("Ext")
                    .join("Form")
                    .join("Module.bsl"),
            })
        }
        [type_name, object_name, module_name] if *type_name != "CommonModule" => {
            cfe_validate_metadata_name("ModulePath object name", object_name)?;
            cfe_validate_metadata_name("ModulePath module name", module_name)?;
            let role = match *module_name {
                "ObjectModule" => CfePatchModuleRole::ObjectModule,
                "ManagerModule" => CfePatchModuleRole::ManagerModule,
                "RecordSetModule" => CfePatchModuleRole::RecordSetModule,
                "ValueManagerModule" => CfePatchModuleRole::ValueManagerModule,
                _ => return Err(cfe_patch_unsupported_role(module_path)),
            };
            if !cfe_patch_direct_role_is_supported(type_name, role) {
                return Err(cfe_patch_unsupported_role(module_path));
            }
            let directory = cf_validate_child_type_dir(type_name)
                .ok_or_else(|| format!("Unknown object type: {type_name}"))?;
            Ok(CfePatchModuleTarget {
                type_name: cfe_patch_canonical_type(type_name)
                    .ok_or_else(|| cfe_patch_unsupported_role(module_path))?,
                object_name: (*object_name).to_string(),
                role,
                form_name: None,
                descriptor: extension_path
                    .join(directory)
                    .join(format!("{object_name}.xml")),
                bsl_file: extension_path
                    .join(directory)
                    .join(object_name)
                    .join("Ext")
                    .join(format!("{module_name}.bsl")),
            })
        }
        _ => Err(invalid_format()),
    }
}

fn cfe_patch_canonical_type(type_name: &str) -> Option<&'static str> {
    match type_name {
        "Catalog" => Some("Catalog"),
        "Document" => Some("Document"),
        "Enum" => Some("Enum"),
        "Report" => Some("Report"),
        "DataProcessor" => Some("DataProcessor"),
        "ExchangePlan" => Some("ExchangePlan"),
        "ChartOfAccounts" => Some("ChartOfAccounts"),
        "ChartOfCharacteristicTypes" => Some("ChartOfCharacteristicTypes"),
        "ChartOfCalculationTypes" => Some("ChartOfCalculationTypes"),
        "BusinessProcess" => Some("BusinessProcess"),
        "Task" => Some("Task"),
        "InformationRegister" => Some("InformationRegister"),
        "AccumulationRegister" => Some("AccumulationRegister"),
        "AccountingRegister" => Some("AccountingRegister"),
        "CalculationRegister" => Some("CalculationRegister"),
        "Constant" => Some("Constant"),
        "DocumentJournal" => Some("DocumentJournal"),
        "FilterCriterion" => Some("FilterCriterion"),
        _ => None,
    }
}

fn cfe_patch_form_role_is_supported(type_name: &str) -> bool {
    cfe_patch_canonical_type(type_name).is_some() && type_name != "Constant"
}

fn cfe_patch_direct_role_is_supported(type_name: &str, role: CfePatchModuleRole) -> bool {
    match role {
        CfePatchModuleRole::ObjectModule => matches!(
            type_name,
            "Catalog"
                | "Document"
                | "ExchangePlan"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "Report"
                | "DataProcessor"
        ),
        CfePatchModuleRole::ManagerModule => matches!(
            type_name,
            "Catalog"
                | "Document"
                | "ExchangePlan"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "Report"
                | "DataProcessor"
                | "Enum"
                | "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
                | "Constant"
                | "DocumentJournal"
                | "FilterCriterion"
        ),
        CfePatchModuleRole::RecordSetModule => matches!(
            type_name,
            "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
        ),
        CfePatchModuleRole::ValueManagerModule => type_name == "Constant",
        CfePatchModuleRole::CommonModule | CfePatchModuleRole::Form => false,
    }
}

fn cfe_patch_unsupported_role(module_path: &str) -> String {
    format!(
        "ModulePath '{module_path}' is not supported by the cfe.patch_method grammar for platform 1C 8.3.27. Supported paths are CommonModule.Name, supported Type.Name.ObjectModule/ManagerModule/RecordSetModule/ValueManagerModule roles, and supported Type.Name.Form.FormName"
    )
}

fn cfe_patch_validate_bsl_identifier(argument: &str, value: &str) -> Result<(), String> {
    if cf_validate_identifier(value) && form_is_xml_ncname(value) {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid 1C identifier and Unicode XML NCName: {value:?}"
        ))
    }
}

fn cfe_patch_precondition_error(module_path: &str, detail: impl AsRef<str>) -> String {
    format!(
        "ModulePath '{module_path}' is not a borrowed extension object: {}",
        detail.as_ref()
    )
}

fn cfe_patch_read_required_xml(
    module_path: &str,
    path: &Path,
    description: &str,
) -> Result<Vec<u8>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(cfe_patch_precondition_error(
                module_path,
                format!("required {description} is missing: {}", path.display()),
            ));
        }
        Err(error) => {
            return Err(format!("failed to inspect {}: {error}", path.display()));
        }
    };
    if !metadata.file_type().is_file() {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "required {description} must be an existing regular file: {}",
                path.display()
            ),
        ));
    }
    fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn cfe_patch_supported_document<'a>(path: &Path, raw: &'a [u8]) -> Result<Document<'a>, String> {
    let text = std::str::from_utf8(raw)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source)
        .map_err(|error| format!("failed to parse platform XML {}: {error}", path.display()))?;
    let version_literal = root_version_literal(source, document.root_element());
    match classify_root_version(version_literal.as_deref()).map_err(|error| error.to_string())? {
        FormatCompatibility::Supported { .. } => Ok(document),
        compatibility => Err(format_compatibility_warning(&compatibility)),
    }
}

fn cfe_patch_direct_md_child<'a>(
    document: &'a Document<'a>,
    expected_type: &str,
    module_path: &str,
    path: &Path,
) -> Result<roxmltree::Node<'a, 'a>, String> {
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must have root {{{CFE_PATCH_MD_NAMESPACE}}}MetaDataObject",
                path.display()
            ),
        ));
    }
    let direct = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if direct.len() != 1
        || direct[0].tag_name().name() != expected_type
        || direct[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must contain exactly one direct {{{CFE_PATCH_MD_NAMESPACE}}}{expected_type}",
                path.display()
            ),
        ));
    }
    Ok(direct[0])
}

fn cfe_patch_exact_md_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let children = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == local_name)
        .collect::<Vec<_>>();
    if children.len() != 1 || children[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must contain exactly one direct {{{CFE_PATCH_MD_NAMESPACE}}}{local_name}",
                path.display()
            ),
        ));
    }
    Ok(children[0])
}

fn cfe_patch_exact_md_text(
    parent: roxmltree::Node<'_, '_>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<String, String> {
    let child = cfe_patch_exact_md_child(parent, local_name, module_path, path)?;
    Ok(child.text().unwrap_or_default().to_string())
}

fn cfe_patch_required_exact_md_bool(
    parent: roxmltree::Node<'_, '_>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<bool, String> {
    match cfe_patch_exact_md_text(parent, local_name, module_path, path)?.as_str() {
        "false" => Ok(false),
        "true" => Ok(true),
        value => Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{local_name} in {} must be the exact boolean true or false, got {value:?}",
                path.display()
            ),
        )),
    }
}

fn cfe_patch_optional_exact_md_bool(
    parent: roxmltree::Node<'_, '_>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<Option<bool>, String> {
    let children = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == local_name)
        .collect::<Vec<_>>();
    if children.is_empty() {
        return Ok(None);
    }
    if children.len() != 1 || children[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must contain at most one direct {{{CFE_PATCH_MD_NAMESPACE}}}{local_name}",
                path.display()
            ),
        ));
    }
    match children[0].text().unwrap_or_default() {
        "false" => Ok(Some(false)),
        "true" => Ok(Some(true)),
        value => Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{local_name} in {} must be the exact boolean true or false, got {value:?}",
                path.display()
            ),
        )),
    }
}

fn cfe_patch_validate_non_nil_uuid(
    module_path: &str,
    value: Option<&str>,
    field: &str,
    path: &Path,
) -> Result<(), String> {
    let value = value.unwrap_or_default();
    if !cf_validate_guid(value) || value.eq_ignore_ascii_case(CFE_PATCH_NIL_UUID) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!("{field} in {} must be a valid non-nil UUID", path.display()),
        ));
    }
    Ok(())
}

fn cfe_patch_validate_adopted_descriptor(
    module_path: &str,
    path: &Path,
    raw: &[u8],
    expected_type: &str,
    expected_name: &str,
) -> Result<Option<CfePatchCommonModuleCapabilities>, String> {
    let document = cfe_patch_supported_document(path, raw)?;
    let object = cfe_patch_direct_md_child(&document, expected_type, module_path, path)?;
    cfe_patch_validate_non_nil_uuid(module_path, object.attribute("uuid"), "uuid", path)?;
    let properties = cfe_patch_exact_md_child(object, "Properties", module_path, path)?;
    if cfe_patch_exact_md_text(properties, "Name", module_path, path)? != expected_name {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "Name in {} must exactly match '{expected_name}'",
                path.display()
            ),
        ));
    }
    if cfe_patch_exact_md_text(properties, "ObjectBelonging", module_path, path)? != "Adopted" {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!("ObjectBelonging in {} must be Adopted", path.display()),
        ));
    }
    let extended =
        cfe_patch_exact_md_text(properties, "ExtendedConfigurationObject", module_path, path)?;
    cfe_patch_validate_non_nil_uuid(
        module_path,
        Some(&extended),
        "ExtendedConfigurationObject",
        path,
    )?;
    if expected_type != "CommonModule" {
        return Ok(None);
    }
    let global = cfe_patch_required_exact_md_bool(properties, "Global", module_path, path)?;
    let capabilities = CfePatchCommonModuleCapabilities {
        client_managed_application: cfe_patch_required_exact_md_bool(
            properties,
            "ClientManagedApplication",
            module_path,
            path,
        )?,
        server: cfe_patch_required_exact_md_bool(properties, "Server", module_path, path)?,
        external_connection: cfe_patch_required_exact_md_bool(
            properties,
            "ExternalConnection",
            module_path,
            path,
        )?,
        client_ordinary_application: cfe_patch_required_exact_md_bool(
            properties,
            "ClientOrdinaryApplication",
            module_path,
            path,
        )?,
    };
    let privileged = cfe_patch_optional_exact_md_bool(properties, "Privileged", module_path, path)?
        .unwrap_or(false);
    if global && capabilities.server {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "global server CommonModule cannot be extended: {}",
                path.display()
            ),
        ));
    }
    if privileged && !capabilities.server {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "Privileged CommonModule must also have Server=true: {}",
                path.display()
            ),
        ));
    }
    if !capabilities.any_execution_context() {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "CommonModule has no enabled execution context: {}",
                path.display()
            ),
        ));
    }
    Ok(Some(capabilities))
}

fn cfe_patch_validate_registration(
    module_path: &str,
    cfg_file: &Path,
    cfg_preimage: &[u8],
    target: &CfePatchModuleTarget,
) -> Result<String, String> {
    let document = cfe_patch_supported_document(cfg_file, cfg_preimage)?;
    let configuration =
        cfe_patch_direct_md_child(&document, "Configuration", module_path, cfg_file)?;
    cfe_patch_validate_non_nil_uuid(
        module_path,
        configuration.attribute("uuid"),
        "Configuration uuid",
        cfg_file,
    )?;
    let properties = cfe_patch_exact_md_child(configuration, "Properties", module_path, cfg_file)?;
    let object_belonging = cfe_patch_exact_md_text(
        properties,
        "ObjectBelonging",
        module_path,
        cfg_file,
    )
    .map_err(|_| {
        cfe_patch_precondition_error(module_path, "Configuration ObjectBelonging must be Adopted")
    })?;
    if object_belonging != "Adopted" {
        return Err(cfe_patch_precondition_error(
            module_path,
            "Configuration ObjectBelonging must be Adopted",
        ));
    }
    let purpose = cfe_patch_exact_md_text(
        properties,
        "ConfigurationExtensionPurpose",
        module_path,
        cfg_file,
    )?;
    if !matches!(purpose.as_str(), "Patch" | "Customization" | "AddOn") {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "ConfigurationExtensionPurpose in {} must be one of Patch, Customization, AddOn; got {purpose:?}",
                cfg_file.display()
            ),
        ));
    }
    let name_prefix = cfe_patch_exact_md_text(properties, "NamePrefix", module_path, cfg_file)?;
    if name_prefix.is_empty() {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "NamePrefix in {} must be non-empty and configured by the extension",
                cfg_file.display()
            ),
        ));
    }
    let child_objects =
        cfe_patch_exact_md_child(configuration, "ChildObjects", module_path, cfg_file)?;
    let registrations = child_objects
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == target.type_name
                && node.text() == Some(target.object_name.as_str())
        })
        .collect::<Vec<_>>();
    let registered = registrations.len() == 1
        && registrations[0].tag_name().namespace() == Some(CFE_PATCH_MD_NAMESPACE);
    if !registered {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{}.{} is not registered in extension Configuration.xml",
                target.type_name, target.object_name
            ),
        ));
    }
    Ok(name_prefix)
}

fn cfe_patch_validate_form_registration(
    module_path: &str,
    descriptor: &Path,
    descriptor_raw: &[u8],
    expected_type: &str,
    form_name: &str,
) -> Result<(), String> {
    let document = cfe_patch_supported_document(descriptor, descriptor_raw)?;
    let object = cfe_patch_direct_md_child(&document, expected_type, module_path, descriptor)?;
    let child_objects = cfe_patch_exact_md_child(object, "ChildObjects", module_path, descriptor)?;
    let forms = child_objects
        .children()
        .filter(|node| {
            node.is_element() && node.tag_name().name() == "Form" && node.text() == Some(form_name)
        })
        .collect::<Vec<_>>();
    if forms.len() != 1 || forms[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "Form.{form_name} is not registered in ChildObjects of {}",
                descriptor.display()
            ),
        ));
    }
    Ok(())
}

fn cfe_patch_validate_form_xml(module_path: &str, path: &Path, raw: &[u8]) -> Result<(), String> {
    let document = cfe_patch_supported_document(path, raw)?;
    let root = document.root_element();
    if root.tag_name().name() != "Form"
        || root.tag_name().namespace() != Some(CFE_PATCH_FORM_NAMESPACE)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "borrowed Form.xml {} must have root {{{CFE_PATCH_FORM_NAMESPACE}}}Form",
                path.display()
            ),
        ));
    }
    let base_forms = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "BaseForm")
        .collect::<Vec<_>>();
    if base_forms.len() != 1
        || base_forms[0].tag_name().namespace() != Some(CFE_PATCH_FORM_NAMESPACE)
        || base_forms[0].attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "borrowed Form.xml {} must contain exactly one direct BaseForm version=\"{}\"",
                path.display(),
                ACTIVE_FORMAT_PROFILE.export_format
            ),
        ));
    }
    Ok(())
}

fn cfe_patch_mark_extended_property(
    module_path: &str,
    path: &Path,
    raw: &[u8],
    property: &str,
) -> Result<Vec<u8>, String> {
    let raw_text = std::str::from_utf8(raw)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    let (bom, text) = raw_text
        .strip_prefix('\u{feff}')
        .map_or(("", raw_text), |text| ("\u{feff}", text));
    let document = Document::parse(text)
        .map_err(|error| format!("failed to parse platform XML {}: {error}", path.display()))?;
    let root = document.root_element();
    if let Some(namespace) = root.lookup_namespace_uri(Some("xr")) {
        if namespace != CFE_PATCH_XR_NAMESPACE {
            return Err(cfe_patch_precondition_error(
                module_path,
                format!(
                    "{} binds the xr prefix to unsupported namespace {namespace:?}",
                    path.display()
                ),
            ));
        }
    }
    let object = root
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| {
            cfe_patch_precondition_error(
                module_path,
                format!("{} has no metadata object", path.display()),
            )
        })?;
    let internal_info = cfe_patch_exact_md_child(object, "InternalInfo", module_path, path)?;
    let matching_states = internal_info
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "PropertyState")
        .filter_map(|state| {
            let state_namespace = state.tag_name().namespace();
            let property_node = state.children().find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(CFE_PATCH_XR_NAMESPACE)
                    && node.tag_name().name() == "Property"
            });
            let value_node = state.children().find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(CFE_PATCH_XR_NAMESPACE)
                    && node.tag_name().name() == "State"
            });
            property_node
                .and_then(|node| node.text())
                .filter(|value| *value == property)
                .map(|_| (state_namespace, value_node.and_then(|node| node.text())))
        })
        .collect::<Vec<_>>();
    match matching_states.as_slice() {
        [] => {}
        [(Some(namespace), Some("Extended"))] if *namespace == CFE_PATCH_XR_NAMESPACE => {
            return Ok(raw.to_vec());
        }
        [state] => {
            return Err(cfe_patch_precondition_error(
                module_path,
                format!(
                    "{} has incompatible PropertyState for {property}: {state:?}",
                    path.display()
                ),
            ));
        }
        _ => {
            return Err(cfe_patch_precondition_error(
                module_path,
                format!(
                    "{} has duplicate PropertyState entries for {property}",
                    path.display()
                ),
            ));
        }
    }

    let range = internal_info.range();
    let fragment = &text[range.clone()];
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let indent = form_edit_line_indent_at(text, range.start);
    let child_indent = format!("{indent}\t");
    let value_indent = format!("{child_indent}\t");
    let state_xml = format!(
        "{child_indent}<xr:PropertyState>{newline}{value_indent}<xr:Property>{property}</xr:Property>{newline}{value_indent}<xr:State>Extended</xr:State>{newline}{child_indent}</xr:PropertyState>{newline}"
    );
    let replacement = if fragment.trim_end().ends_with("/>") {
        let close = fragment
            .rfind("/>")
            .expect("trimmed empty XML element must have a closing marker");
        let tag_name = fragment
            .strip_prefix('<')
            .and_then(|value| {
                value
                    .split(|character: char| character.is_ascii_whitespace() || character == '/')
                    .next()
            })
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                cfe_patch_precondition_error(
                    module_path,
                    format!("cannot identify InternalInfo tag in {}", path.display()),
                )
            })?;
        format!(
            "{}>{newline}{state_xml}{indent}</{tag_name}>",
            fragment[..close].trim_end()
        )
    } else {
        let close = fragment.rfind("</").ok_or_else(|| {
            cfe_patch_precondition_error(
                module_path,
                format!(
                    "cannot locate InternalInfo closing tag in {}",
                    path.display()
                ),
            )
        })?;
        let insertion = fragment[..close]
            .rfind('\n')
            .map(|line_break| line_break + 1)
            .unwrap_or(close);
        format!(
            "{}{}{}",
            &fragment[..insertion],
            state_xml,
            &fragment[insertion..]
        )
    };

    let mut updated = format!(
        "{}{}{}",
        &text[..range.start],
        replacement,
        &text[range.end..]
    );
    let root_start = cfe_borrow_find_start_tag(&updated, "MetaDataObject", 0).ok_or_else(|| {
        cfe_patch_precondition_error(
            module_path,
            format!("cannot locate MetaDataObject root in {}", path.display()),
        )
    })?;
    let root_end = updated[root_start..]
        .find('>')
        .map(|offset| root_start + offset + 1)
        .ok_or_else(|| {
            cfe_patch_precondition_error(
                module_path,
                format!(
                    "cannot locate MetaDataObject root end in {}",
                    path.display()
                ),
            )
        })?;
    let root_open = cfe_borrow_ensure_namespace_declaration(
        updated[root_start..root_end].to_string(),
        "xr",
        CFE_PATCH_XR_NAMESPACE,
    );
    updated.replace_range(root_start..root_end, &root_open);
    let updated_with_bom = format!("{bom}{updated}");
    let verified = Document::parse(&updated)
        .map_err(|error| format!("failed to build platform XML {}: {error}", path.display()))?;
    let matching = verified
        .descendants()
        .filter(|node| node.has_tag_name((CFE_PATCH_XR_NAMESPACE, "PropertyState")))
        .filter(|state| {
            state.children().any(|node| {
                node.has_tag_name((CFE_PATCH_XR_NAMESPACE, "Property"))
                    && node.text() == Some(property)
            }) && state.children().any(|node| {
                node.has_tag_name((CFE_PATCH_XR_NAMESPACE, "State"))
                    && node.text() == Some("Extended")
            })
        })
        .count();
    if matching != 1 {
        return Err(format!(
            "failed to build exactly one Extended PropertyState for {property} in {}",
            path.display()
        ));
    }
    Ok(updated_with_bom.into_bytes())
}

fn cfe_patch_borrowed_snapshots(
    extension_path: &Path,
    cfg_file: &Path,
    cfg_preimage: &[u8],
    module_path: &str,
    target: &CfePatchModuleTarget,
) -> Result<CfePatchBorrowedPrecondition, String> {
    let name_prefix = cfe_patch_validate_registration(module_path, cfg_file, cfg_preimage, target)?;
    let descriptor_raw = cfe_patch_read_required_xml(
        module_path,
        &target.descriptor,
        "borrowed object descriptor",
    )?;
    let common_module_capabilities = cfe_patch_validate_adopted_descriptor(
        module_path,
        &target.descriptor,
        &descriptor_raw,
        target.type_name,
        &target.object_name,
    )?;

    let mut snapshots = BTreeMap::from([
        (cfg_file.to_path_buf(), cfg_preimage.to_vec()),
        (target.descriptor.clone(), descriptor_raw.clone()),
    ]);
    let mut property_state_descriptor = target.descriptor.clone();
    if target.role == CfePatchModuleRole::Form {
        let form_name = target
            .form_name
            .as_deref()
            .expect("form target has form name");
        cfe_patch_validate_form_registration(
            module_path,
            &target.descriptor,
            &descriptor_raw,
            target.type_name,
            form_name,
        )?;
        let directory = cf_validate_child_type_dir(target.type_name)
            .expect("validated patch type has a directory");
        let form_dir = extension_path
            .join(directory)
            .join(&target.object_name)
            .join("Forms");
        let wrapper = form_dir.join(format!("{form_name}.xml"));
        let wrapper_raw =
            cfe_patch_read_required_xml(module_path, &wrapper, "borrowed form wrapper")?;
        cfe_patch_validate_adopted_descriptor(
            module_path,
            &wrapper,
            &wrapper_raw,
            "Form",
            form_name,
        )?;
        let form_xml = form_dir.join(form_name).join("Ext").join("Form.xml");
        let form_xml_raw =
            cfe_patch_read_required_xml(module_path, &form_xml, "borrowed Form.xml")?;
        cfe_patch_validate_form_xml(module_path, &form_xml, &form_xml_raw)?;
        property_state_descriptor = wrapper.clone();
        snapshots.insert(wrapper, wrapper_raw);
        snapshots.insert(form_xml, form_xml_raw);
    }
    Ok(CfePatchBorrowedPrecondition {
        snapshots,
        name_prefix,
        common_module_capabilities,
        property_state_descriptor,
    })
}

pub(crate) fn patch_extension_method(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = (|| -> Result<(String, CommitReport), String> {
        let mut extension_path =
            required_path(args, &["extensionPath", "ExtensionPath"], "ExtensionPath")
                .map(|path| absolutize(path, &context.cwd))?;
        if extension_path.is_file() {
            extension_path = extension_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| context.cwd.clone());
        }

        let cfg_file = extension_path.join("Configuration.xml");
        if !cfg_file.is_file() {
            return Err(format!(
                "Configuration.xml not found in: {}",
                extension_path.display()
            ));
        }
        let cfg_preimage = fs::read(&cfg_file)
            .map_err(|error| format!("failed to read {}: {error}", cfg_file.display()))?;

        let module_path = required_string(args, &["modulePath", "ModulePath"], "ModulePath")?;
        let method_name = required_string(args, &["methodName", "MethodName"], "MethodName")?;
        let interceptor_type = required_string(
            args,
            &["interceptorType", "InterceptorType"],
            "InterceptorType",
        )?;
        let context_name = string_arg(args, &["context", "Context"]);
        let is_function = bool_arg(args, &["isFunction", "IsFunction"]);

        cfe_patch_validate_bsl_identifier("MethodName", method_name)?;
        if let Some(context_name) = context_name {
            if !matches!(
                context_name,
                "НаСервере" | "НаКлиенте" | "НаСервереБезКонтекста"
            ) {
                return Err(format!(
                    "Context must be one of: НаСервере, НаКлиенте, НаСервереБезКонтекста; got {context_name:?}"
                ));
            }
        }
        if is_function {
            return Err(
                "cfe.patch_method v1 requires a parameterless procedure; a base method signature resolver for functions and parameterized methods is not implemented"
                    .to_string(),
            );
        }
        let decorator = match interceptor_type {
            "Before" => "&Перед",
            "After" => "&После",
            _ => {
                return Err(format!(
                    "cfe.patch_method v1 supports only Before and After for a parameterless procedure; an exact base method body/signature resolver required for {interceptor_type:?} is not implemented"
                ));
            }
        };
        let target = cfe_patch_module_target(&extension_path, module_path)?;
        let CfePatchBorrowedPrecondition {
            snapshots: borrowed_snapshots,
            name_prefix,
            common_module_capabilities,
            property_state_descriptor,
        } = cfe_patch_borrowed_snapshots(
            &extension_path,
            &cfg_file,
            &cfg_preimage,
            module_path,
            &target,
        )?;
        let property_state_preimage = borrowed_snapshots
            .get(&property_state_descriptor)
            .ok_or_else(|| {
                format!(
                    "missing borrowed descriptor snapshot: {}",
                    property_state_descriptor.display()
                )
            })?;
        let property_state_updated = cfe_patch_mark_extended_property(
            module_path,
            &property_state_descriptor,
            property_state_preimage,
            target.role.extended_property(),
        )?;
        let context_annotation = match target.role {
            CfePatchModuleRole::CommonModule => {
                let capabilities = common_module_capabilities.ok_or_else(|| {
                    cfe_patch_precondition_error(
                        module_path,
                        "CommonModule execution capabilities were not validated",
                    )
                })?;
                match context_name {
                    None => None,
                    Some("НаСервере") if capabilities.server => Some("&НаСервере"),
                    Some("НаСервере") => {
                        return Err(
                            "Context НаСервере requires Server=true in the borrowed CommonModule"
                                .to_string(),
                        );
                    }
                    Some("НаКлиенте") if capabilities.client() => Some("&НаКлиенте"),
                    Some("НаКлиенте") => {
                        return Err(
                            "Context НаКлиенте requires ClientManagedApplication=true or ClientOrdinaryApplication=true in the borrowed CommonModule"
                                .to_string(),
                        );
                    }
                    Some("НаСервереБезКонтекста") => {
                        return Err(
                            "Context НаСервереБезКонтекста is not available in a CommonModule on platform 1C 8.3.27"
                                .to_string(),
                        );
                    }
                    Some(_) => unreachable!("Context was validated above"),
                }
            }
            CfePatchModuleRole::Form => match context_name.unwrap_or("НаСервере") {
                "НаСервере" => Some("&НаСервере"),
                "НаКлиенте" => Some("&НаКлиенте"),
                "НаСервереБезКонтекста" => Some("&НаСервереБезКонтекста"),
                _ => unreachable!("Context was validated above"),
            },
            CfePatchModuleRole::ObjectModule
            | CfePatchModuleRole::ManagerModule
            | CfePatchModuleRole::RecordSetModule
            | CfePatchModuleRole::ValueManagerModule => {
                if context_name.is_some() {
                    return Err(format!(
                        "Context is not available for {} in platform 1C 8.3.27; omit Context so no compilation directive is emitted",
                        target.role.as_str()
                    ));
                }
                None
            }
        };
        run_cfe_patch_after_borrowed_read_hook();
        let bsl_file = target.bsl_file;
        let proc_name = format!("{name_prefix}{method_name}");
        cfe_patch_validate_bsl_identifier("generated interceptor procedure name", &proc_name)?;

        let body_line = match interceptor_type {
            "Before" => "\t// TODO: код перед вызовом оригинального метода",
            "After" => "\t// TODO: код после вызова оригинального метода",
            _ => unreachable!("InterceptorType was validated above"),
        };
        let mut bsl_code = Vec::new();
        if let Some(context_annotation) = context_annotation {
            bsl_code.push(context_annotation.to_string());
        }
        bsl_code.extend([
            format!("{decorator}(\"{method_name}\")"),
            format!("Процедура {proc_name}()"),
            body_line.to_string(),
            "КонецПроцедуры".to_string(),
        ]);
        let bsl_text = format!("{}\r\n", bsl_code.join("\r\n"));

        let mut stdout = String::new();
        let mut transaction = CompileTransaction::new();
        let created = if bsl_file.is_file() {
            let original = fs::read(&bsl_file)
                .map_err(|err| format!("failed to read {}: {err}", bsl_file.display()))?;
            let existing = std::str::from_utf8(&original)
                .map_err(|error| format!("{} is not valid UTF-8: {error}", bsl_file.display()))?
                .trim_start_matches('\u{feff}');
            let separator = if !existing.is_empty() && !existing.ends_with('\n') {
                "\r\n\r\n"
            } else {
                "\r\n"
            };
            transaction.replace_bytes(
                &bsl_file,
                &original,
                utf8_bom_bytes(&format!("{existing}{separator}{bsl_text}")),
            )?;
            false
        } else {
            transaction.create_utf8_bom_text(&bsl_file, &bsl_text)?;
            true
        };
        let descriptor_changed =
            property_state_updated.as_slice() != property_state_preimage.as_slice();
        if descriptor_changed {
            transaction.replace_bytes(
                &property_state_descriptor,
                property_state_preimage,
                property_state_updated,
            )?;
        }
        let owner_targets = borrowed_snapshots
            .keys()
            .map(PathBuf::as_path)
            .collect::<Vec<_>>();
        guard_cfe_active_format_snapshot_set(
            &mut transaction,
            &borrowed_snapshots,
            &owner_targets,
            &[bsl_file.as_path()],
            context,
        )?;
        let report = transaction.commit()?;

        if created {
            stdout.push_str("[OK] Создан файл модуля\n");
        } else {
            stdout.push_str("[OK] Добавлен перехватчик в существующий файл\n");
        }
        stdout.push_str(&format!("     Файл:         {}\n", bsl_file.display()));
        stdout.push_str(&format!(
            "     Декоратор:    {decorator}(\"{method_name}\")\n"
        ));
        stdout.push_str(&format!("     Процедура:    {proc_name}()\n"));
        if let Some(context_annotation) = context_annotation {
            stdout.push_str(&format!("     Контекст:     {context_annotation}\n"));
        } else {
            stdout.push_str("     Контекст:     без директивы компиляции\n");
        }
        if descriptor_changed {
            stdout.push_str(&format!(
                "     XML-состояние: {} = Extended ({})\n",
                target.role.extended_property(),
                property_state_descriptor.display()
            ));
        }

        Ok((stdout, report))
    })();

    match write_result {
        Ok((stdout, report)) => {
            let mut changes = report
                .created
                .iter()
                .map(|path| format!("created {}", path.display()))
                .collect::<Vec<_>>();
            changes.extend(
                report
                    .updated
                    .iter()
                    .map(|path| format!("updated {}", path.display())),
            );
            let artifacts = report
                .created
                .iter()
                .chain(&report.updated)
                .map(|path| path.display().to_string())
                .collect();
            AdapterOutcome {
                ok: true,
                summary: "unica.cfe.patch_method completed with native BSL/XML interceptor writer"
                    .to_string(),
                changes,
                warnings: report.cleanup_warnings,
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: None,
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cfe.patch_method failed in native BSL interceptor writer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn create_extension_scaffold(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let name = string_arg(args, &["name", "Name"]).unwrap_or("");
    if name.is_empty() {
        return AdapterOutcome {
            ok: false,
            summary: "unica.cfe.init failed in native XML scaffold writer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec!["missing required Name argument".to_string()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some("missing required Name argument\n".to_string()),
            command: None,
        };
    }
    let synonym = string_arg(args, &["synonym", "Synonym"]).unwrap_or(name);
    let name_prefix = cfe_init_name_prefix(args, name);
    let planned = cfe_init_planned_xml(args, context);
    let out_dir = planned.output_dir;
    let config = planned.configuration;
    let language = planned.language;
    let no_role = planned.role.is_none();
    let role_name = format!("{name_prefix}ОсновнаяРоль");
    let role = planned.role;
    let purpose = string_arg(args, &["purpose", "Purpose"]).unwrap_or("Customization");

    let write_result = (|| -> Result<(String, Vec<String>), String> {
        cfe_validate_metadata_name("Name", name)?;
        if !no_role {
            cfe_validate_metadata_name("RoleName", &role_name)?;
        }
        if !matches!(purpose, "Patch" | "Customization" | "AddOn") {
            return Err(format!(
                "Purpose value {purpose:?} is not valid for 8.3.27; expected one of: Patch, Customization, AddOn"
            ));
        }

        let mut stdout_prefix = String::new();
        let mut base_lang_uuid = "00000000-0000-0000-0000-000000000000".to_string();
        let mut compatibility = string_arg(args, &["compatibilityMode", "CompatibilityMode"])
            .unwrap_or("Version8_3_24")
            .to_string();
        let mut base_config_path = None;
        let mut base_config_preimage = None;
        let mut base_language_preimage = None;
        cfe_init_validate_enum("ConfigurationExtensionCompatibilityMode", &compatibility)?;
        let interface_mode = if let Some(config_path) =
            path_arg(args, &["configPath", "ConfigPath"])
        {
            let mut config_path = absolutize(config_path, &context.cwd);
            if config_path.is_dir() {
                let candidate = config_path.join("Configuration.xml");
                if candidate.exists() {
                    config_path = candidate;
                } else {
                    return Err(format!(
                        "No Configuration.xml in config directory: {}",
                        config_path.display()
                    ));
                }
            }
            if !config_path.exists() {
                return Err(format!("Config file not found: {}", config_path.display()));
            }

            let cfg_dir = config_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| context.cwd.clone());
            let base_raw = fs::read(&config_path).map_err(|error| {
                format!(
                    "failed to read base config {}: {error}",
                    config_path.display()
                )
            })?;
            let base_text = std::str::from_utf8(&base_raw).map_err(|error| {
                format!(
                    "failed to read base config {} as UTF-8: {error}",
                    config_path.display()
                )
            })?;
            let base_document =
                Document::parse(base_text.trim_start_matches('\u{feff}')).map_err(|error| {
                    format!(
                        "failed to parse base config {}: {error}",
                        config_path.display()
                    )
                })?;
            let base_root = base_document.root_element();
            if base_root.tag_name().name() != "MetaDataObject" {
                return Err(format!(
                    "base config root must be MetaDataObject, got {}",
                    base_root.tag_name().name()
                ));
            }
            if base_root.tag_name().namespace() != Some("http://v8.1c.ru/8.3/MDClasses") {
                return Err("base config root must use the MDClasses namespace".to_string());
            }
            base_config_path = Some(config_path.clone());
            stdout_prefix.push_str(&format!(
                "[INFO] Base config MDClasses format version: {}\n",
                ACTIVE_FORMAT_PROFILE.export_format
            ));
            let base_lang_file = cfg_dir.join("Languages").join("Русский.xml");
            if base_lang_file.exists() {
                match fs::read(&base_lang_file) {
                    Ok(raw) => {
                        base_language_preimage = Some((base_lang_file.clone(), raw.clone()));
                        match std::str::from_utf8(&raw).ok().and_then(|text| {
                            Document::parse(text.trim_start_matches('\u{feff}')).ok()
                        }) {
                            Some(doc) => {
                                if let Some(uuid) = doc
                                    .descendants()
                                    .find(|node| {
                                        node.is_element() && node.tag_name().name() == "Language"
                                    })
                                    .and_then(|node| node.attribute("uuid"))
                                {
                                    if !is_valid_uuid(uuid)
                                        || uuid == "00000000-0000-0000-0000-000000000000"
                                    {
                                        return Err(format!(
                                            "Base language {} has invalid UUID {uuid:?}",
                                            base_lang_file.display()
                                        ));
                                    }
                                    base_lang_uuid = uuid.to_string();
                                    stdout_prefix.push_str(&format!(
                                        "[INFO] Base config Language UUID: {base_lang_uuid}\n"
                                    ));
                                }
                            }
                            None => {
                                stdout_prefix.push_str(&format!(
                                    "[WARN] Could not parse {}\n",
                                    base_lang_file.display()
                                ));
                            }
                        }
                    }
                    Err(_) => {
                        stdout_prefix.push_str(&format!(
                            "[WARN] Could not parse {}\n",
                            base_lang_file.display()
                        ));
                    }
                }
            } else {
                stdout_prefix.push_str(&format!(
                    "[WARN] Base config language not found: {}\n",
                    base_lang_file.display()
                ));
            }

            if let Some(value) = first_text(&base_document, "CompatibilityMode") {
                compatibility = value;
                stdout_prefix.push_str(&format!(
                    "[INFO] Base config CompatibilityMode: {compatibility}\n"
                ));
            } else {
                stdout_prefix.push_str(&format!(
                    "[WARN] CompatibilityMode not found in base config, using default: {compatibility}\n"
                ));
            }
            let interface_mode = if let Some(value) =
                first_text(&base_document, "InterfaceCompatibilityMode")
            {
                stdout_prefix.push_str(&format!(
                    "[INFO] Base config InterfaceCompatibilityMode: {value}\n"
                ));
                value
            } else {
                let value = "TaxiEnableVersion8_2".to_string();
                stdout_prefix.push_str(&format!(
                    "[WARN] InterfaceCompatibilityMode not found in base config, using default: {value}\n"
                ));
                value
            };
            base_config_preimage = Some(base_raw);
            interface_mode
        } else {
            stdout_prefix.push_str("[WARN] Language ExtendedConfigurationObject set to zeros. Use -ConfigPath to auto-resolve from base config, or fix manually before loading.\n");
            "TaxiEnableVersion8_2".to_string()
        };
        cfe_init_validate_enum("ConfigurationExtensionCompatibilityMode", &compatibility)?;
        cfe_init_validate_enum("InterfaceCompatibilityMode", &interface_mode)?;
        run_cfe_init_after_base_read_hook();

        let uuid_cfg = stable_uuid(20);
        let uuid_lang = stable_uuid(21);
        let uuid_role = stable_uuid(22);
        let contained_object_ids = (23..30).map(stable_uuid).collect::<Vec<_>>();
        let contained_objects = contained_objects_xml(&contained_object_ids);
        let format_version_xml = ACTIVE_FORMAT_PROFILE.export_format;
        let purpose_xml = escape_xml(purpose);
        let compatibility_xml = escape_xml(&compatibility);
        let interface_mode_xml = escape_xml(&interface_mode);
        let vendor_xml = string_arg(args, &["vendor", "Vendor"])
            .map(escape_xml)
            .unwrap_or_default();
        let version_xml = string_arg(args, &["version", "Version"])
            .map(escape_xml)
            .unwrap_or_default();
        let synonym_xml = format!(
            "\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content>{}</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t",
            escape_xml(synonym)
        );
        let default_roles_xml = if no_role {
            String::new()
        } else {
            format!(
                "\r\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">Role.{}</xr:Item>\r\n\t\t\t",
                escape_xml(&role_name)
            )
        };
        let mut child_objects_xml = "\r\n\t\t\t<Language>Русский</Language>".to_string();
        if !no_role {
            child_objects_xml.push_str(&format!(
                "\r\n\t\t\t<Role>{}</Role>",
                escape_xml(&role_name)
            ));
        }
        child_objects_xml.push_str("\r\n\t\t");

        let mut transaction = CompileTransaction::new();
        transaction.create_utf8_bom_text(
            &config,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version_xml}">
	<Configuration uuid="{uuid_cfg}">
		<InternalInfo>
{contained_objects}		</InternalInfo>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>{name}</Name>
			<Synonym>{synonym_xml}</Synonym>
			<Comment/>
			<ConfigurationExtensionPurpose>{purpose_xml}</ConfigurationExtensionPurpose>
			<KeepMappingToExtendedConfigurationObjectsByIDs>true</KeepMappingToExtendedConfigurationObjectsByIDs>
			<NamePrefix>{name_prefix}</NamePrefix>
			<ConfigurationExtensionCompatibilityMode>{compatibility_xml}</ConfigurationExtensionCompatibilityMode>
			<DefaultRunMode>ManagedApplication</DefaultRunMode>
			<UsePurposes>
				<v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value>
			</UsePurposes>
			<ScriptVariant>Russian</ScriptVariant>
			<DefaultRoles>{default_roles_xml}</DefaultRoles>
			<Vendor>{vendor_xml}</Vendor>
			<Version>{version_xml}</Version>
			<DefaultLanguage>Language.Русский</DefaultLanguage>
			<BriefInformation/>
			<DetailedInformation/>
			<Copyright/>
			<VendorInformationAddress/>
			<ConfigurationInformationAddress/>
			<InterfaceCompatibilityMode>{interface_mode_xml}</InterfaceCompatibilityMode>
		</Properties>
		<ChildObjects>{child_objects_xml}</ChildObjects>
	</Configuration>
</MetaDataObject>"#,
                name = escape_xml(name),
                name_prefix = escape_xml(&name_prefix),
            ),
        )?;
        transaction.create_utf8_bom_text(
            &language,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version_xml}">
	<Language uuid="{uuid_lang}">
		<InternalInfo/>
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Русский</Name>
			<Comment/>
			<ExtendedConfigurationObject>{base_lang_uuid}</ExtendedConfigurationObject>
			<LanguageCode>ru</LanguageCode>
		</Properties>
	</Language>
</MetaDataObject>"#
            ),
        )?;

        if let Some(role) = &role {
            transaction.create_utf8_bom_text(
                role,
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:cmi="http://v8.1c.ru/8.2/managed-application/cmi" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise" xmlns:lf="http://v8.1c.ru/8.2/managed-application/logform" xmlns:style="http://v8.1c.ru/8.1/data/ui/style" xmlns:sys="http://v8.1c.ru/8.1/data/ui/fonts/system" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:v8ui="http://v8.1c.ru/8.1/data/ui" xmlns:web="http://v8.1c.ru/8.1/data/ui/colors/web" xmlns:win="http://v8.1c.ru/8.1/data/ui/colors/windows" xmlns:xen="http://v8.1c.ru/8.3/xcf/enums" xmlns:xpr="http://v8.1c.ru/8.3/xcf/predef" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="{format_version_xml}">
	<Role uuid="{uuid_role}">
		<Properties>
			<Name>{role_name}</Name>
			<Synonym/>
			<Comment/>
		</Properties>
	</Role>
</MetaDataObject>"#,
                    role_name = escape_xml(&role_name),
                ),
            )?;
        }
        if let (Some(base_config_path), Some(base_config_preimage)) =
            (&base_config_path, &base_config_preimage)
        {
            guard_exact_preimage_if_unprotected(
                &mut transaction,
                base_config_path,
                base_config_preimage,
            )?;
        }
        if let Some((base_language_path, base_language_preimage)) = &base_language_preimage {
            guard_exact_preimage_if_unprotected(
                &mut transaction,
                base_language_path,
                base_language_preimage,
            )?;
        }
        let mut base_snapshots = BTreeMap::new();
        let mut base_owner_targets = Vec::new();
        if let (Some(base_config_path), Some(base_config_preimage)) =
            (&base_config_path, &base_config_preimage)
        {
            base_snapshots.insert(base_config_path.clone(), base_config_preimage.clone());
            base_owner_targets.push(base_config_path.as_path());
        }
        if let Some((base_language_path, base_language_preimage)) = &base_language_preimage {
            base_snapshots.insert(base_language_path.clone(), base_language_preimage.clone());
        }
        guard_cfe_active_format_snapshot_set(
            &mut transaction,
            &base_snapshots,
            &base_owner_targets,
            &[&out_dir],
            context,
        )?;

        let report = transaction
            .commit_with_post_validation(|| cfe_init_validate_post_state(&config, context))?;

        let mut stdout = format!(
            "{stdout_prefix}[OK] Создано расширение: {name}\n     Каталог:            {}\n     Назначение:         {purpose}\n     Префикс:           {name_prefix}\n     Совместимость:     {compatibility}\n     Configuration.xml:  {}\n     Languages:          {}\n",
            out_dir.display(),
            config.display(),
            language.display()
        );
        if let Some(role) = &role {
            stdout.push_str(&format!("     Role:               {}\n", role.display()));
        }
        Ok((stdout, report.cleanup_warnings))
    })();

    match write_result {
        Ok((stdout, warnings)) => {
            let mut changes = vec![
                format!("created {}", config.display()),
                format!("created {}", language.display()),
            ];
            let mut artifacts = vec![config.display().to_string(), language.display().to_string()];
            if let Some(role) = &role {
                changes.push(format!("created {}", role.display()));
                artifacts.push(role.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.cfe.init completed with native XML scaffold writer".to_string(),
                changes,
                warnings,
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: None,
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cfe.init failed in native XML scaffold writer".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn bsl_file_for_module_path(
    extension_path: &Path,
    module_path: &str,
) -> Result<PathBuf, String> {
    cfe_patch_module_target(extension_path, module_path).map(|target| target.bsl_file)
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "cfe-validate" => Some(Ok(validate_cfe(args, context))),
        "cfe-diff" => Some(Ok(diff_cfe(args, context))),
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
        "cfe-init" => Some(create_extension_scaffold(args, context)),
        "cfe-patch-method" => Some(patch_extension_method(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_commit_failpoint, CommitFailpoint,
    };
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn cfe_borrow_has_complete_8_3_27_generated_type_profiles() {
        let profiles: &[(&str, &[(&str, &str)])] = &[
            (
                "Catalog",
                &[
                    ("CatalogObject", "Object"),
                    ("CatalogRef", "Ref"),
                    ("CatalogSelection", "Selection"),
                    ("CatalogList", "List"),
                    ("CatalogManager", "Manager"),
                ],
            ),
            (
                "Document",
                &[
                    ("DocumentObject", "Object"),
                    ("DocumentRef", "Ref"),
                    ("DocumentSelection", "Selection"),
                    ("DocumentList", "List"),
                    ("DocumentManager", "Manager"),
                ],
            ),
            (
                "Enum",
                &[
                    ("EnumRef", "Ref"),
                    ("EnumManager", "Manager"),
                    ("EnumList", "List"),
                ],
            ),
            (
                "Constant",
                &[
                    ("ConstantManager", "Manager"),
                    ("ConstantValueManager", "ValueManager"),
                    ("ConstantValueKey", "ValueKey"),
                ],
            ),
            (
                "InformationRegister",
                &[
                    ("InformationRegisterRecord", "Record"),
                    ("InformationRegisterManager", "Manager"),
                    ("InformationRegisterSelection", "Selection"),
                    ("InformationRegisterList", "List"),
                    ("InformationRegisterRecordSet", "RecordSet"),
                    ("InformationRegisterRecordKey", "RecordKey"),
                    ("InformationRegisterRecordManager", "RecordManager"),
                ],
            ),
            (
                "AccumulationRegister",
                &[
                    ("AccumulationRegisterRecord", "Record"),
                    ("AccumulationRegisterManager", "Manager"),
                    ("AccumulationRegisterSelection", "Selection"),
                    ("AccumulationRegisterList", "List"),
                    ("AccumulationRegisterRecordSet", "RecordSet"),
                    ("AccumulationRegisterRecordKey", "RecordKey"),
                ],
            ),
            (
                "AccountingRegister",
                &[
                    ("AccountingRegisterRecord", "Record"),
                    ("AccountingRegisterExtDimensions", "ExtDimensions"),
                    ("AccountingRegisterRecordSet", "RecordSet"),
                    ("AccountingRegisterRecordKey", "RecordKey"),
                    ("AccountingRegisterSelection", "Selection"),
                    ("AccountingRegisterList", "List"),
                    ("AccountingRegisterManager", "Manager"),
                ],
            ),
            (
                "CalculationRegister",
                &[
                    ("CalculationRegisterRecord", "Record"),
                    ("CalculationRegisterManager", "Manager"),
                    ("CalculationRegisterSelection", "Selection"),
                    ("CalculationRegisterList", "List"),
                    ("CalculationRegisterRecordSet", "RecordSet"),
                    ("CalculationRegisterRecordKey", "RecordKey"),
                    ("RecalculationsManager", "Recalcs"),
                ],
            ),
            (
                "ChartOfAccounts",
                &[
                    ("ChartOfAccountsObject", "Object"),
                    ("ChartOfAccountsRef", "Ref"),
                    ("ChartOfAccountsSelection", "Selection"),
                    ("ChartOfAccountsList", "List"),
                    ("ChartOfAccountsManager", "Manager"),
                    ("ChartOfAccountsExtDimensionTypes", "ExtDimensionTypes"),
                    (
                        "ChartOfAccountsExtDimensionTypesRow",
                        "ExtDimensionTypesRow",
                    ),
                ],
            ),
            (
                "ChartOfCharacteristicTypes",
                &[
                    ("ChartOfCharacteristicTypesObject", "Object"),
                    ("ChartOfCharacteristicTypesRef", "Ref"),
                    ("ChartOfCharacteristicTypesSelection", "Selection"),
                    ("ChartOfCharacteristicTypesList", "List"),
                    ("Characteristic", "Characteristic"),
                    ("ChartOfCharacteristicTypesManager", "Manager"),
                ],
            ),
            (
                "ChartOfCalculationTypes",
                &[
                    ("ChartOfCalculationTypesObject", "Object"),
                    ("ChartOfCalculationTypesRef", "Ref"),
                    ("ChartOfCalculationTypesSelection", "Selection"),
                    ("ChartOfCalculationTypesList", "List"),
                    ("ChartOfCalculationTypesManager", "Manager"),
                    ("DisplacingCalculationTypes", "DisplacingCalculationTypes"),
                    (
                        "DisplacingCalculationTypesRow",
                        "DisplacingCalculationTypesRow",
                    ),
                    ("BaseCalculationTypes", "BaseCalculationTypes"),
                    ("BaseCalculationTypesRow", "BaseCalculationTypesRow"),
                    ("LeadingCalculationTypes", "LeadingCalculationTypes"),
                    ("LeadingCalculationTypesRow", "LeadingCalculationTypesRow"),
                ],
            ),
            (
                "BusinessProcess",
                &[
                    ("BusinessProcessObject", "Object"),
                    ("BusinessProcessRef", "Ref"),
                    ("BusinessProcessSelection", "Selection"),
                    ("BusinessProcessList", "List"),
                    ("BusinessProcessManager", "Manager"),
                    ("BusinessProcessRoutePointRef", "RoutePointRef"),
                ],
            ),
            (
                "Task",
                &[
                    ("TaskObject", "Object"),
                    ("TaskRef", "Ref"),
                    ("TaskSelection", "Selection"),
                    ("TaskList", "List"),
                    ("TaskManager", "Manager"),
                ],
            ),
            (
                "ExchangePlan",
                &[
                    ("ExchangePlanObject", "Object"),
                    ("ExchangePlanRef", "Ref"),
                    ("ExchangePlanSelection", "Selection"),
                    ("ExchangePlanList", "List"),
                    ("ExchangePlanManager", "Manager"),
                ],
            ),
            (
                "DocumentJournal",
                &[
                    ("DocumentJournalSelection", "Selection"),
                    ("DocumentJournalList", "List"),
                    ("DocumentJournalManager", "Manager"),
                ],
            ),
            (
                "Report",
                &[("ReportObject", "Object"), ("ReportManager", "Manager")],
            ),
            (
                "DataProcessor",
                &[
                    ("DataProcessorObject", "Object"),
                    ("DataProcessorManager", "Manager"),
                ],
            ),
            ("DefinedType", &[("DefinedType", "DefinedType")]),
        ];

        for (object_type, expected) in profiles {
            let shared = metadata_generated_types_8_3_27(object_type).unwrap();
            let borrowed = cfe_borrow_generated_types(object_type).unwrap();
            assert_eq!(borrowed, *expected, "{object_type}");
            assert!(
                std::ptr::eq(borrowed, shared),
                "{object_type} must use the shared 8.3.27 GeneratedType profile"
            );

            let borrowed_xml =
                cfe_borrow_internal_info_xml(object_type, "SharedContract", "\t\t").unwrap();
            let mut meta_lines = Vec::new();
            let mut next = || "11111111-1111-1111-1111-111111111111".to_string();
            emit_meta_internal_info(
                &mut meta_lines,
                "\t\t",
                object_type,
                "SharedContract",
                &mut next,
            );
            let meta_xml = meta_lines.join("\n");
            for (prefix, category) in *expected {
                let generated = format!("name=\"{prefix}.SharedContract\" category=\"{category}\"");
                assert!(
                    borrowed_xml.contains(&generated),
                    "cfe.borrow lost {object_type} {generated}: {borrowed_xml}"
                );
                assert!(
                    meta_xml.contains(&generated),
                    "meta.compile lost {object_type} {generated}: {meta_xml}"
                );
            }
            assert_eq!(
                borrowed_xml.matches("<xr:GeneratedType ").count(),
                expected.len(),
                "cfe.borrow emitted extra GeneratedType entries for {object_type}"
            );
            assert_eq!(
                meta_xml.matches("<xr:GeneratedType ").count(),
                expected.len(),
                "meta.compile emitted extra GeneratedType entries for {object_type}"
            );
        }
        assert_eq!(
            metadata_generated_types_8_3_27("CommonModule"),
            Some(&[][..])
        );
        assert_eq!(cfe_borrow_generated_types("CommonModule"), Some(&[][..]));

        let exchange =
            cfe_borrow_internal_info_xml("ExchangePlan", "CorpusExchangePlan", "\t\t").unwrap();
        assert!(
            exchange.contains("<xr:ThisNode>"),
            "ExchangePlan requires its own internal node id: {exchange}"
        );
        let mut exchange_meta = Vec::new();
        let mut next = || "11111111-1111-1111-1111-111111111111".to_string();
        emit_meta_internal_info(
            &mut exchange_meta,
            "\t\t",
            "ExchangePlan",
            "CorpusExchangePlan",
            &mut next,
        );
        assert_eq!(exchange.matches("<xr:ThisNode>").count(), 1);
        assert_eq!(exchange_meta.join("\n").matches("<xr:ThisNode>").count(), 1);
    }

    #[test]
    fn cfe_borrow_generated_type_profile_is_total_for_the_metadata_registry() {
        let missing = cf_validate_child_object_types()
            .iter()
            .copied()
            .filter(|object_type| cfe_borrow_generated_types(object_type).is_none())
            .collect::<Vec<_>>();

        assert!(
            missing.is_empty(),
            "cfe.borrow must fail closed instead of using an implicit empty InternalInfo profile: {missing:?}"
        );
        assert_eq!(cfe_borrow_generated_types("UnknownMetadataType"), None);
        let error =
            cfe_borrow_internal_info_xml("UnknownMetadataType", "Unknown", "\t\t").unwrap_err();
        assert!(error.contains("no proven cfe.borrow InternalInfo profile"));
        assert_eq!(
            cfe_borrow_internal_info_xml("CommonModule", "KnownEmpty", "\t\t").unwrap(),
            "\t\t<InternalInfo/>"
        );
    }

    #[test]
    fn cfe_borrow_generated_type_profiles_cover_8_3_27_dynamic_families() {
        let profiles: &[(&str, &[(&str, &str)])] = &[
            (
                "FilterCriterion",
                &[
                    ("FilterCriterionManager", "Manager"),
                    ("FilterCriterionList", "List"),
                ],
            ),
            ("SettingsStorage", &[("SettingsStorageManager", "Manager")]),
            ("Sequence", &[("SequenceRecordSet", "RecordSet")]),
            (
                "IntegrationService",
                &[("IntegrationServiceManager", "Manager")],
            ),
        ];

        for (object_type, expected) in profiles {
            assert_eq!(cfe_borrow_generated_types(object_type), Some(*expected));
            let borrowed_xml =
                cfe_borrow_internal_info_xml(object_type, "Evidence", "\t\t").unwrap();
            let mut meta_lines = Vec::new();
            let mut next = || "11111111-1111-1111-1111-111111111111".to_string();
            emit_meta_internal_info(&mut meta_lines, "\t\t", object_type, "Evidence", &mut next);
            let meta_xml = meta_lines.join("\n");
            for (prefix, category) in *expected {
                let fragment = format!("name=\"{prefix}.Evidence\" category=\"{category}\"");
                assert!(borrowed_xml.contains(&fragment), "{borrowed_xml}");
                assert!(meta_xml.contains(&fragment), "{meta_xml}");
            }
        }
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
    fn cfe_borrow_parse_rejects_unsafe_object_and_form_names_before_path_use() {
        for value in [
            "Catalog.../Outside",
            "Catalog.Items.Form.../Outside",
            "Catalog.Bad:Name",
            "Catalog.Items.Form.Bad:Name",
            "Catalog.",
            "Catalog.Items.Form.",
        ] {
            let Err(error) = cfe_borrow_parse_object_spec(value) else {
                panic!("unsafe metadata name must be rejected: {value}");
            };
            assert!(error.contains("XML NCName"), "{value}: {error}");
            assert!(error.contains("path component"), "{value}: {error}");
        }

        let parsed = cfe_borrow_parse_object_spec("Catalog.Товары.Form.ФормаЭлемента")
            .expect("Unicode metadata names must remain supported");
        assert_eq!(parsed.object_name, "Товары");
        assert_eq!(parsed.form_name.as_deref(), Some("ФормаЭлемента"));
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
            let mut stdout = String::new();

            cfe_borrow_object_shell(
                &cfg,
                &ext,
                &mut plan,
                "Catalog",
                "Items",
                "2.20",
                &mut extension,
                &mut stdout,
            )
            .unwrap();
            if form_first {
                cfe_borrow_register_form(
                    &ext,
                    &mut plan,
                    "Catalog",
                    "Items",
                    "MainForm",
                    &mut stdout,
                )
                .unwrap();
                cfe_borrow_object_shell(
                    &cfg,
                    &ext,
                    &mut plan,
                    "Catalog",
                    "Items",
                    "2.20",
                    &mut extension,
                    &mut stdout,
                )
                .unwrap();
            } else {
                cfe_borrow_register_form(
                    &ext,
                    &mut plan,
                    "Catalog",
                    "Items",
                    "MainForm",
                    &mut stdout,
                )
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
            let mut stdout = String::new();
            let generated = cfe_borrow_form_xml(
                source,
                Path::new("."),
                "Catalog",
                "Items",
                false,
                "2.20",
                &mut stdout,
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
        let mut stdout = String::new();

        let generated = cfe_borrow_form_xml(
            source,
            Path::new("."),
            "Catalog",
            "Items",
            true,
            "2.20",
            &mut stdout,
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
        let mut stdout = String::new();

        let generated = cfe_borrow_form_xml(
            source,
            Path::new("."),
            "Catalog",
            "Items",
            true,
            "2.20",
            &mut stdout,
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
    fn cfe_borrow_form_metadata_marks_the_form_property_as_extended() {
        let generated = cfe_borrow_form_metadata_xml(
            "MainForm",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "2.20",
        );
        let document = Document::parse(&generated).unwrap();
        let internal_info = document
            .descendants()
            .find(|node| node.has_tag_name("InternalInfo"))
            .expect("borrowed form metadata must contain InternalInfo");
        let property_states = internal_info
            .children()
            .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "PropertyState")))
            .collect::<Vec<_>>();

        assert_eq!(property_states.len(), 1, "{generated}");
        let property_state = property_states[0];
        assert_eq!(
            property_state
                .children()
                .find(|node| {
                    node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "Property"))
                })
                .and_then(|node| node.text()),
            Some("Form"),
            "{generated}"
        );
        assert_eq!(
            property_state
                .children()
                .find(|node| { node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "State")) })
                .and_then(|node| node.text()),
            Some("Extended"),
            "{generated}"
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
        let init = create_extension_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("SemanticExtension")),
                ("OutputDir".to_string(), json!("ext")),
                ("NoRole".to_string(), json!(true)),
            ]),
            &context,
        );
        assert!(init.ok, "{init:?}");

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
    fn borrow_cfe_rejects_concurrent_base_format_owner_change() {
        let context = temp_context("borrow-base-owner-guard");
        let init = create_extension_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("OwnerGuardExtension")),
                ("OutputDir".to_string(), json!("ext")),
                ("NoRole".to_string(), json!(true)),
            ]),
            &context,
        );
        assert!(init.ok, "{init:?}");
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
        let init = create_extension_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("PreflightExtension")),
                ("OutputDir".to_string(), json!("ext")),
                ("NoRole".to_string(), json!(true)),
            ]),
            &context,
        );
        assert!(init.ok, "{init:?}");
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

        let outcome = borrow_cfe(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(fs::read_to_string(&module_path).unwrap(), existing_module);
        assert!(
            fs::read_to_string(&form_meta_path)
                .unwrap()
                .contains(existing_form_meta_uuid),
            "existing form metadata uuid must survive re-borrow"
        );
        let stdout = outcome.stdout.as_deref().unwrap_or_default();
        assert!(
            stdout.contains("[SKIP] Module.bsl already exists"),
            "{stdout}"
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
    fn cfe_patch_method_rejects_newer_extension_owner_without_creating_module() {
        let context = temp_context("patch-newer-owner");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.21", None);
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("newer than supported 2.20"),
            "{outcome:?}"
        );
        assert!(!module.exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_unborrowed_common_module_without_writes() {
        let context = temp_context("patch-unborrowed-common-module");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("is not a borrowed extension object"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_publishes_supported_module_through_transaction() {
        let context = temp_context("patch-supported-owner");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let descriptor =
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(outcome.ok, "{outcome:?}");
        let bytes = fs::read(&module).unwrap();
        assert!(bytes.starts_with(b"\xef\xbb\xbf"), "{bytes:?}");
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("&Перед(\"Run\")"), "{text}");
        assert!(text.contains("Процедура GE_Run()"), "{text}");
        assert_eq!(
            outcome
                .artifacts
                .iter()
                .map(|path| {
                    crate::infrastructure::platform::testing::normalize_path_text_for_test(path)
                })
                .collect::<Vec<_>>(),
            vec![
                crate::infrastructure::platform::testing::path_text_for_test(&module),
                crate::infrastructure::platform::testing::path_text_for_test(&descriptor)
            ]
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_marks_the_extended_module_property_in_platform_xml() {
        let cases = [
            (
                "CommonModule",
                "GuardedModule",
                "CommonModule.GuardedModule",
                "",
                "Module",
            ),
            (
                "Catalog",
                "Items",
                "Catalog.Items.ObjectModule",
                "<ChildObjects/>",
                "ObjectModule",
            ),
            (
                "Catalog",
                "Items",
                "Catalog.Items.ManagerModule",
                "<ChildObjects/>",
                "ManagerModule",
            ),
            (
                "InformationRegister",
                "Items",
                "InformationRegister.Items.RecordSetModule",
                "<ChildObjects/>",
                "RecordSetModule",
            ),
            (
                "Constant",
                "Items",
                "Constant.Items.ValueManagerModule",
                "",
                "ValueManagerModule",
            ),
        ];

        for (index, (type_name, object_name, module_path, child_objects, property)) in
            cases.into_iter().enumerate()
        {
            let context = temp_context(&format!("patch-property-state-{index}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            let descriptor =
                register_borrowed_patch_object(&context, type_name, object_name, child_objects);
            let mut args = patch_method_args();
            args.insert("ModulePath".to_string(), json!(module_path));

            let outcome = patch_extension_method(&args, &context);

            assert!(outcome.ok, "{module_path}: {outcome:?}");
            assert_extended_property_state(&descriptor, property);
            let _ = fs::remove_dir_all(&context.cwd);
        }

        let context = temp_context("patch-form-property-state");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let (_, wrapper, _) = register_borrowed_patch_form(&context, "Catalog", "Items", "Main");
        let mut args = patch_method_args();
        args.insert("ModulePath".to_string(), json!("Catalog.Items.Form.Main"));

        let outcome = patch_extension_method(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert_extended_property_state(&wrapper, "Form");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_accepts_borrowed_object_and_manager_modules() {
        for module_name in ["ObjectModule", "ManagerModule"] {
            let context = temp_context(&format!("patch-borrowed-{module_name}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            register_borrowed_patch_object(&context, "Catalog", "Items", "<ChildObjects/>");
            let mut args = patch_method_args();
            args.insert(
                "ModulePath".to_string(),
                json!(format!("Catalog.Items.{module_name}")),
            );
            let module = context
                .cwd
                .join("ext/Catalogs/Items/Ext")
                .join(format!("{module_name}.bsl"));

            let outcome = patch_extension_method(&args, &context);

            assert!(outcome.ok, "{module_name}: {outcome:?}");
            assert!(module.is_file(), "{module_name}: {outcome:?}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_patch_method_accepts_borrowed_form_module() {
        let context = temp_context("patch-borrowed-form");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_form(&context, "Catalog", "Items", "Main");
        let mut args = patch_method_args();
        args.insert("ModulePath".to_string(), json!("Catalog.Items.Form.Main"));
        let module = context
            .cwd
            .join("ext/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl");

        let outcome = patch_extension_method(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert!(module.is_file(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_accepts_authoritative_8_3_27_extension_role_additions() {
        let cases = [
            ("Constant", "ManagerModule", None),
            ("Constant", "ValueManagerModule", None),
            ("DocumentJournal", "ManagerModule", None),
            ("FilterCriterion", "ManagerModule", None),
            ("DocumentJournal", "Form", Some("Main")),
            ("FilterCriterion", "Form", Some("Main")),
        ];
        for (index, (type_name, role, form_name)) in cases.into_iter().enumerate() {
            let context = temp_context(&format!("patch-authoritative-role-{index}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            let module_path = if let Some(form_name) = form_name {
                register_borrowed_patch_form(&context, type_name, "Items", form_name);
                format!("{type_name}.Items.Form.{form_name}")
            } else {
                let child_objects = if type_name == "Constant" {
                    ""
                } else {
                    "<ChildObjects/>"
                };
                register_borrowed_patch_object(&context, type_name, "Items", child_objects);
                format!("{type_name}.Items.{role}")
            };
            let mut args = patch_method_args();
            args.insert("ModulePath".to_string(), json!(module_path));

            let outcome = patch_extension_method(&args, &context);

            assert!(
                outcome.ok,
                "{type_name}.{role} must be supported: {outcome:?}"
            );
            assert_eq!(
                outcome.artifacts.len(),
                2,
                "BSL and the descriptor must be published together: {outcome:?}"
            );
            assert!(
                outcome
                    .artifacts
                    .iter()
                    .all(|artifact| Path::new(artifact).is_file()),
                "{type_name}.{role}: {outcome:?}"
            );
            assert_eq!(
                outcome
                    .artifacts
                    .iter()
                    .filter(|artifact| artifact.ends_with(".bsl"))
                    .count(),
                1,
                "{type_name}.{role}: {outcome:?}"
            );
            assert_eq!(
                outcome
                    .artifacts
                    .iter()
                    .filter(|artifact| artifact.ends_with(".xml"))
                    .count(),
                1,
                "{type_name}.{role}: {outcome:?}"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_patch_method_accepts_borrowed_record_set_module() {
        let context = temp_context("patch-borrowed-record-set");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "InformationRegister", "Items", "<ChildObjects/>");
        let mut args = patch_method_args();
        args.insert(
            "ModulePath".to_string(),
            json!("InformationRegister.Items.RecordSetModule"),
        );
        let module = context
            .cwd
            .join("ext/InformationRegisters/Items/Ext/RecordSetModule.bsl");

        let outcome = patch_extension_method(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert!(module.is_file(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_8_3_27_grammar_matrix_has_exactly_51_roles() {
        let object_types = [
            "Catalog",
            "Document",
            "ExchangePlan",
            "ChartOfAccounts",
            "ChartOfCharacteristicTypes",
            "ChartOfCalculationTypes",
            "BusinessProcess",
            "Task",
            "Report",
            "DataProcessor",
        ];
        let manager_types = [
            "Catalog",
            "Document",
            "ExchangePlan",
            "ChartOfAccounts",
            "ChartOfCharacteristicTypes",
            "ChartOfCalculationTypes",
            "BusinessProcess",
            "Task",
            "Report",
            "DataProcessor",
            "Enum",
            "InformationRegister",
            "AccumulationRegister",
            "AccountingRegister",
            "CalculationRegister",
            "Constant",
            "DocumentJournal",
            "FilterCriterion",
        ];
        let record_set_types = [
            "InformationRegister",
            "AccumulationRegister",
            "AccountingRegister",
            "CalculationRegister",
        ];
        let form_types = [
            "Catalog",
            "Document",
            "Enum",
            "Report",
            "DataProcessor",
            "ExchangePlan",
            "ChartOfAccounts",
            "ChartOfCharacteristicTypes",
            "ChartOfCalculationTypes",
            "BusinessProcess",
            "Task",
            "InformationRegister",
            "AccumulationRegister",
            "AccountingRegister",
            "CalculationRegister",
            "DocumentJournal",
            "FilterCriterion",
        ];
        let extension = Path::new("/extension");
        assert!(
            cfe_patch_module_target(extension, "CommonModule.Items").is_ok(),
            "CommonModule role"
        );
        for type_name in object_types {
            assert!(
                cfe_patch_module_target(extension, &format!("{type_name}.Items.ObjectModule"))
                    .is_ok(),
                "{type_name}.ObjectModule"
            );
        }
        for type_name in manager_types {
            assert!(
                cfe_patch_module_target(extension, &format!("{type_name}.Items.ManagerModule"))
                    .is_ok(),
                "{type_name}.ManagerModule"
            );
        }
        for type_name in record_set_types {
            assert!(
                cfe_patch_module_target(extension, &format!("{type_name}.Items.RecordSetModule"))
                    .is_ok(),
                "{type_name}.RecordSetModule"
            );
        }
        assert!(
            cfe_patch_module_target(extension, "Constant.Items.ValueManagerModule").is_ok(),
            "Constant.ValueManagerModule"
        );
        for type_name in form_types {
            assert!(
                cfe_patch_module_target(extension, &format!("{type_name}.Items.Form.Main")).is_ok(),
                "{type_name}.Form"
            );
        }

        let mut accepted = 1usize;
        for type_name in cf_validate_child_object_types() {
            for role in [
                "ObjectModule",
                "ManagerModule",
                "RecordSetModule",
                "ValueManagerModule",
            ] {
                accepted += cfe_patch_module_target(extension, &format!("{type_name}.Items.{role}"))
                    .is_ok() as usize;
            }
            accepted += cfe_patch_module_target(extension, &format!("{type_name}.Items.Form.Main"))
                .is_ok() as usize;
        }
        assert_eq!(accepted, 51, "8.3.27 cfe.patch_method grammar matrix");
    }

    #[test]
    fn cfe_patch_method_rejects_unregistered_descriptor_without_writes() {
        let context = temp_context("patch-unregistered-descriptor");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let descriptor =
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let owner = context.cwd.join("ext/Configuration.xml");
        let text = fs::read_to_string(&owner).unwrap().replace(
            "<ChildObjects>\n\t\t\t<CommonModule>GuardedModule</CommonModule>\n\t\t</ChildObjects>",
            "<ChildObjects/>",
        );
        fs::write(&owner, text).unwrap();
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(descriptor.is_file());
        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("is not registered"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_main_configuration_masquerading_as_extension() {
        let context = temp_context("patch-main-configuration");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let owner = context.cwd.join("ext/Configuration.xml");
        let main_configuration = fs::read_to_string(&owner)
            .unwrap()
            .replace("\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>\n", "");
        fs::write(&owner, main_configuration).unwrap();
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("Configuration ObjectBelonging must be Adopted"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_invalid_extension_root_identity_without_writes() {
        let cases = [
            (
                "missing-configuration-uuid",
                " uuid=\"66666666-6666-6666-6666-666666666666\"",
                "",
            ),
            (
                "nil-configuration-uuid",
                "uuid=\"66666666-6666-6666-6666-666666666666\"",
                "uuid=\"00000000-0000-0000-0000-000000000000\"",
            ),
            (
                "missing-extension-purpose",
                "\n\t\t\t<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>",
                "",
            ),
            (
                "invalid-extension-purpose",
                "<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>",
                "<ConfigurationExtensionPurpose>Unsupported</ConfigurationExtensionPurpose>",
            ),
        ];
        let mut accepted = Vec::new();

        for (case, from, to) in cases {
            let context = temp_context(&format!("patch-root-identity-{case}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
            let owner = context.cwd.join("ext/Configuration.xml");
            let original = fs::read_to_string(&owner).unwrap();
            let invalid = original.replacen(from, to, 1);
            assert_ne!(invalid, original, "{case}: fixture mutation must apply");
            fs::write(&owner, invalid).unwrap();
            let module = context
                .cwd
                .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

            let outcome = patch_extension_method(&patch_method_args(), &context);

            if outcome.ok
                || module.exists()
                || !outcome.changes.is_empty()
                || !outcome.artifacts.is_empty()
            {
                accepted.push(format!("{case}: {outcome:?}"));
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }

        assert!(
            accepted.is_empty(),
            "invalid extension identities must fail closed:\n{}",
            accepted.join("\n")
        );
    }

    #[test]
    fn cfe_patch_method_rejects_ambiguous_or_wrong_namespace_name_prefix() {
        let cases = [
            (
                "duplicate",
                "<NamePrefix>GE_</NamePrefix>\n\t\t\t<NamePrefix>DUP_</NamePrefix>",
            ),
            (
                "wrong-namespace",
                "<evil:NamePrefix xmlns:evil=\"urn:evil\">EVIL_</evil:NamePrefix>\n\t\t\t<NamePrefix>GE_</NamePrefix>",
            ),
        ];
        let mut accepted = Vec::new();

        for (case, replacement) in cases {
            let context = temp_context(&format!("patch-name-prefix-{case}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
            let owner = context.cwd.join("ext/Configuration.xml");
            let original = fs::read_to_string(&owner).unwrap();
            let invalid = original.replacen("<NamePrefix>GE_</NamePrefix>", replacement, 1);
            assert_ne!(invalid, original, "{case}: fixture mutation must apply");
            fs::write(&owner, invalid).unwrap();
            let module = context
                .cwd
                .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

            let outcome = patch_extension_method(&patch_method_args(), &context);

            if outcome.ok
                || module.exists()
                || !outcome.changes.is_empty()
                || !outcome.artifacts.is_empty()
            {
                accepted.push(format!("{case}: {outcome:?}"));
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }

        assert!(
            accepted.is_empty(),
            "ambiguous or non-MD NamePrefix must fail closed:\n{}",
            accepted.join("\n")
        );
    }

    #[test]
    fn cfe_patch_method_rejects_global_server_common_module_without_writes() {
        let context = temp_context("patch-global-server-common-module");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let descriptor =
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let original = fs::read_to_string(&descriptor).unwrap();
        let global_server = original.replacen("<Global>false</Global>", "<Global>true</Global>", 1);
        assert_ne!(
            global_server, original,
            "fixture mutation must make the common module global and server"
        );
        fs::write(&descriptor, global_server).unwrap();
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("global server CommonModule"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_missing_or_malformed_common_module_flags() {
        let cases = [
            ("missing-global", "\n\t\t\t<Global>false</Global>", ""),
            (
                "duplicate-server",
                "<Server>true</Server>",
                "<Server>true</Server>\n\t\t\t<Server>true</Server>",
            ),
            (
                "wrong-namespace-external",
                "<ExternalConnection>false</ExternalConnection>",
                "<evil:ExternalConnection xmlns:evil=\"urn:evil\">false</evil:ExternalConnection>",
            ),
            (
                "bad-client-ordinary",
                "<ClientOrdinaryApplication>false</ClientOrdinaryApplication>",
                "<ClientOrdinaryApplication>maybe</ClientOrdinaryApplication>",
            ),
            (
                "malformed-privileged",
                "\n\t\t\t<ReturnValuesReuse>",
                "\n\t\t\t<Privileged>maybe</Privileged>\n\t\t\t<ReturnValuesReuse>",
            ),
        ];
        let mut accepted = Vec::new();

        for (case, from, to) in cases {
            let context = temp_context(&format!("patch-common-flags-{case}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            let descriptor =
                register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
            let original = fs::read_to_string(&descriptor).unwrap();
            let invalid = original.replacen(from, to, 1);
            assert_ne!(invalid, original, "{case}: fixture mutation must apply");
            fs::write(&descriptor, invalid).unwrap();
            let module = context
                .cwd
                .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

            let outcome = patch_extension_method(&patch_method_args(), &context);

            if outcome.ok
                || module.exists()
                || !outcome.changes.is_empty()
                || !outcome.artifacts.is_empty()
            {
                accepted.push(format!("{case}: {outcome:?}"));
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }

        assert!(
            accepted.is_empty(),
            "CommonModule execution flags must be required exact MD booleans:\n{}",
            accepted.join("\n")
        );
    }

    #[test]
    fn cfe_patch_method_common_module_context_uses_exact_capabilities() {
        struct Case {
            name: &'static str,
            replacements: &'static [(&'static str, &'static str)],
            explicit_context: Option<&'static str>,
            should_succeed: bool,
            expected_directive: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "server-default",
                replacements: &[],
                explicit_context: None,
                should_succeed: true,
                expected_directive: None,
            },
            Case {
                name: "server-explicit-server",
                replacements: &[],
                explicit_context: Some("НаСервере"),
                should_succeed: true,
                expected_directive: Some("&НаСервере"),
            },
            Case {
                name: "server-explicit-client",
                replacements: &[],
                explicit_context: Some("НаКлиенте"),
                should_succeed: false,
                expected_directive: None,
            },
            Case {
                name: "client-default",
                replacements: &[
                    ("<Server>true</Server>", "<Server>false</Server>"),
                    (
                        "<ClientManagedApplication>false</ClientManagedApplication>",
                        "<ClientManagedApplication>true</ClientManagedApplication>",
                    ),
                ],
                explicit_context: None,
                should_succeed: true,
                expected_directive: None,
            },
            Case {
                name: "client-explicit-client",
                replacements: &[
                    ("<Server>true</Server>", "<Server>false</Server>"),
                    (
                        "<ClientManagedApplication>false</ClientManagedApplication>",
                        "<ClientManagedApplication>true</ClientManagedApplication>",
                    ),
                ],
                explicit_context: Some("НаКлиенте"),
                should_succeed: true,
                expected_directive: Some("&НаКлиенте"),
            },
            Case {
                name: "client-explicit-server",
                replacements: &[
                    ("<Server>true</Server>", "<Server>false</Server>"),
                    (
                        "<ClientManagedApplication>false</ClientManagedApplication>",
                        "<ClientManagedApplication>true</ClientManagedApplication>",
                    ),
                ],
                explicit_context: Some("НаСервере"),
                should_succeed: false,
                expected_directive: None,
            },
            Case {
                name: "external-default",
                replacements: &[
                    ("<Server>true</Server>", "<Server>false</Server>"),
                    (
                        "<ExternalConnection>false</ExternalConnection>",
                        "<ExternalConnection>true</ExternalConnection>",
                    ),
                ],
                explicit_context: None,
                should_succeed: true,
                expected_directive: None,
            },
            Case {
                name: "external-explicit-server",
                replacements: &[
                    ("<Server>true</Server>", "<Server>false</Server>"),
                    (
                        "<ExternalConnection>false</ExternalConnection>",
                        "<ExternalConnection>true</ExternalConnection>",
                    ),
                ],
                explicit_context: Some("НаСервере"),
                should_succeed: false,
                expected_directive: None,
            },
            Case {
                name: "all-contexts-disabled",
                replacements: &[("<Server>true</Server>", "<Server>false</Server>")],
                explicit_context: None,
                should_succeed: false,
                expected_directive: None,
            },
            Case {
                name: "privileged-without-server",
                replacements: &[
                    ("<Server>true</Server>", "<Server>false</Server>"),
                    (
                        "<ExternalConnection>false</ExternalConnection>",
                        "<ExternalConnection>true</ExternalConnection>",
                    ),
                    (
                        "\n\t\t\t<ReturnValuesReuse>",
                        "\n\t\t\t<Privileged>true</Privileged>\n\t\t\t<ReturnValuesReuse>",
                    ),
                ],
                explicit_context: None,
                should_succeed: false,
                expected_directive: None,
            },
        ];

        for case in cases {
            let context = temp_context(&format!("patch-common-capability-{}", case.name));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            let descriptor =
                register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
            let mut descriptor_text = fs::read_to_string(&descriptor).unwrap();
            for (from, to) in case.replacements {
                let mutated = descriptor_text.replacen(from, to, 1);
                assert_ne!(
                    mutated, descriptor_text,
                    "{}: fixture mutation {from:?} must apply",
                    case.name
                );
                descriptor_text = mutated;
            }
            fs::write(&descriptor, descriptor_text).unwrap();
            let mut args = patch_method_args();
            if let Some(explicit_context) = case.explicit_context {
                args.insert("Context".to_string(), json!(explicit_context));
            }
            let module = context
                .cwd
                .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

            let outcome = patch_extension_method(&args, &context);

            assert_eq!(
                outcome.ok, case.should_succeed,
                "{}: {outcome:?}",
                case.name
            );
            if case.should_succeed {
                let text = fs::read_to_string(&module).unwrap();
                let expected_start = match case.expected_directive {
                    Some(directive) => format!("\u{feff}{directive}\r\n&Перед"),
                    None => "\u{feff}&Перед".to_string(),
                };
                assert!(
                    text.starts_with(&expected_start),
                    "{}: expected {expected_start:?}, got {text:?}",
                    case.name
                );
            } else {
                assert!(!module.exists(), "{}: {outcome:?}", case.name);
                assert!(outcome.changes.is_empty(), "{}: {outcome:?}", case.name);
                assert!(outcome.artifacts.is_empty(), "{}: {outcome:?}", case.name);
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_patch_method_rejects_empty_configured_name_prefix_without_writes() {
        let context = temp_context("patch-empty-name-prefix");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let owner = context.cwd.join("ext/Configuration.xml");
        let original = fs::read_to_string(&owner).unwrap();
        let empty = original.replacen("<NamePrefix>GE_</NamePrefix>", "<NamePrefix/>", 1);
        assert_ne!(empty, original, "fixture mutation must apply");
        fs::write(&owner, empty).unwrap();
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("NamePrefix"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_unsupported_v1_interception_shapes_atomically() {
        let cases = [
            ("modification-and-control", "InterceptorType"),
            ("function", "IsFunction"),
        ];
        let mut failures = Vec::new();

        for (case, mutation) in cases {
            let context = temp_context(&format!("patch-v1-shape-{case}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
            let mut args = patch_method_args();
            match mutation {
                "InterceptorType" => {
                    args.insert(
                        "InterceptorType".to_string(),
                        json!("ModificationAndControl"),
                    );
                }
                "IsFunction" => {
                    args.insert("IsFunction".to_string(), json!(true));
                }
                _ => unreachable!(),
            }
            let module = context
                .cwd
                .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

            let outcome = patch_extension_method(&args, &context);
            let error = outcome.errors.join("\n");

            if outcome.ok
                || module.exists()
                || !outcome.changes.is_empty()
                || !outcome.artifacts.is_empty()
                || !error.contains("parameterless procedure")
                || !error.contains("not implemented")
            {
                failures.push(format!("{case}: {outcome:?}"));
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }

        assert!(
            failures.is_empty(),
            "unsupported v1 interception shapes must fail closed:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn cfe_patch_method_emits_role_aware_exact_bsl_for_six_module_layouts() {
        let cases = [
            (
                "common",
                "CommonModule",
                "GuardedModule",
                "CommonModule.GuardedModule",
                "",
                "ext/CommonModules/GuardedModule/Ext/Module.bsl",
                false,
            ),
            (
                "object",
                "Catalog",
                "Items",
                "Catalog.Items.ObjectModule",
                "<ChildObjects/>",
                "ext/Catalogs/Items/Ext/ObjectModule.bsl",
                false,
            ),
            (
                "manager",
                "Catalog",
                "Items",
                "Catalog.Items.ManagerModule",
                "<ChildObjects/>",
                "ext/Catalogs/Items/Ext/ManagerModule.bsl",
                false,
            ),
            (
                "record-set",
                "InformationRegister",
                "Items",
                "InformationRegister.Items.RecordSetModule",
                "<ChildObjects/>",
                "ext/InformationRegisters/Items/Ext/RecordSetModule.bsl",
                false,
            ),
            (
                "value-manager",
                "Constant",
                "Items",
                "Constant.Items.ValueManagerModule",
                "",
                "ext/Constants/Items/Ext/ValueManagerModule.bsl",
                false,
            ),
            (
                "form",
                "Catalog",
                "Items",
                "Catalog.Items.Form.Main",
                "",
                "ext/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
                true,
            ),
        ];

        for (case, type_name, object_name, module_path, child_objects, relative, has_context) in
            cases
        {
            let context = temp_context(&format!("patch-role-bsl-{case}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            if case == "form" {
                register_borrowed_patch_form(&context, type_name, object_name, "Main");
            } else {
                register_borrowed_patch_object(&context, type_name, object_name, child_objects);
            }
            let mut args = patch_method_args();
            args.insert("ModulePath".to_string(), json!(module_path));
            let module = context.cwd.join(relative);

            let outcome = patch_extension_method(&args, &context);

            assert!(outcome.ok, "{case}: {outcome:?}");
            let context_line = if has_context {
                "&НаСервере\r\n"
            } else {
                ""
            };
            let expected = format!(
                "\u{feff}{context_line}&Перед(\"Run\")\r\nПроцедура GE_Run()\r\n\t// TODO: код перед вызовом оригинального метода\r\nКонецПроцедуры\r\n"
            );
            assert_eq!(
                fs::read(&module).unwrap(),
                expected.as_bytes(),
                "{case}: exact role-aware BSL"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_patch_method_enforces_role_aware_explicit_context_policy() {
        let direct_context = temp_context("patch-direct-explicit-context");
        write_minimal_borrow_fixture(&direct_context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&direct_context, "Catalog", "Items", "<ChildObjects/>");
        let mut direct_args = patch_method_args();
        direct_args.insert(
            "ModulePath".to_string(),
            json!("Catalog.Items.ObjectModule"),
        );
        direct_args.insert("Context".to_string(), json!("НаСервере"));
        let direct_module = direct_context
            .cwd
            .join("ext/Catalogs/Items/Ext/ObjectModule.bsl");

        let direct_outcome = patch_extension_method(&direct_args, &direct_context);

        assert!(!direct_outcome.ok, "{direct_outcome:?}");
        assert!(
            direct_outcome
                .errors
                .join("\n")
                .contains("Context is not available"),
            "{direct_outcome:?}"
        );
        assert!(!direct_module.exists(), "{direct_outcome:?}");
        assert!(direct_outcome.changes.is_empty(), "{direct_outcome:?}");
        assert!(direct_outcome.artifacts.is_empty(), "{direct_outcome:?}");
        let _ = fs::remove_dir_all(&direct_context.cwd);

        let common_context = temp_context("patch-common-context-policy");
        write_minimal_borrow_fixture(&common_context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&common_context, "CommonModule", "GuardedModule", "");
        let mut common_args = patch_method_args();
        common_args.insert("Context".to_string(), json!("НаСервереБезКонтекста"));
        let common_module = common_context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let common_outcome = patch_extension_method(&common_args, &common_context);

        assert!(!common_outcome.ok, "{common_outcome:?}");
        assert!(
            common_outcome
                .errors
                .join("\n")
                .contains("not available in a CommonModule"),
            "{common_outcome:?}"
        );
        assert!(!common_module.exists(), "{common_outcome:?}");
        assert!(common_outcome.changes.is_empty(), "{common_outcome:?}");
        assert!(common_outcome.artifacts.is_empty(), "{common_outcome:?}");
        let _ = fs::remove_dir_all(&common_context.cwd);

        let form_context = temp_context("patch-form-no-context");
        write_minimal_borrow_fixture(&form_context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_form(&form_context, "Catalog", "Items", "Main");
        let mut form_args = patch_method_args();
        form_args.insert("ModulePath".to_string(), json!("Catalog.Items.Form.Main"));
        form_args.insert("Context".to_string(), json!("НаСервереБезКонтекста"));
        let form_module = form_context
            .cwd
            .join("ext/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl");

        let form_outcome = patch_extension_method(&form_args, &form_context);

        assert!(form_outcome.ok, "{form_outcome:?}");
        let form_text = fs::read_to_string(&form_module).unwrap();
        assert!(
            form_text.starts_with("\u{feff}&НаСервереБезКонтекста\r\n&Перед"),
            "{form_text:?}"
        );
        let _ = fs::remove_dir_all(&form_context.cwd);
    }

    #[test]
    fn cfe_patch_method_binds_descriptor_bytes_used_for_borrowed_precondition() {
        let context = temp_context("patch-descriptor-precondition-race");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let descriptor =
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let concurrent = fs::read_to_string(&descriptor)
            .unwrap()
            .replace(
                "<ObjectBelonging>Adopted</ObjectBelonging>",
                "<ObjectBelonging>Own</ObjectBelonging>",
            )
            .into_bytes();
        let descriptor_for_hook = descriptor.clone();
        let concurrent_for_hook = concurrent.clone();
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = with_cfe_patch_after_borrowed_read_hook(
            move || fs::write(&descriptor_for_hook, concurrent_for_hook).unwrap(),
            || patch_extension_method(&patch_method_args(), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("changed"), "{outcome:?}");
        assert_eq!(fs::read(&descriptor).unwrap(), concurrent);
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_binds_form_wrapper_used_for_borrowed_precondition() {
        let context = temp_context("patch-form-wrapper-precondition-race");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let (_, wrapper, _) = register_borrowed_patch_form(&context, "Catalog", "Items", "Main");
        let concurrent = fs::read_to_string(&wrapper)
            .unwrap()
            .replace(
                "<ObjectBelonging>Adopted</ObjectBelonging>",
                "<ObjectBelonging>Own</ObjectBelonging>",
            )
            .into_bytes();
        let wrapper_for_hook = wrapper.clone();
        let concurrent_for_hook = concurrent.clone();
        let mut args = patch_method_args();
        args.insert("ModulePath".to_string(), json!("Catalog.Items.Form.Main"));
        let module = context
            .cwd
            .join("ext/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl");

        let outcome = with_cfe_patch_after_borrowed_read_hook(
            move || fs::write(&wrapper_for_hook, concurrent_for_hook).unwrap(),
            || patch_extension_method(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("changed"), "{outcome:?}");
        assert_eq!(fs::read(&wrapper).unwrap(), concurrent);
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_form_without_borrowed_wrapper_without_writes() {
        let context = temp_context("patch-form-without-wrapper");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let (_, wrapper, _) = register_borrowed_patch_form(&context, "Catalog", "Items", "Main");
        fs::remove_file(wrapper).unwrap();
        let mut args = patch_method_args();
        args.insert("ModulePath".to_string(), json!("Catalog.Items.Form.Main"));
        let module = context
            .cwd
            .join("ext/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl");

        let outcome = patch_extension_method(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("borrowed form wrapper"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_form_without_base_form_without_writes() {
        let context = temp_context("patch-form-without-base-form");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let (_, _, form_xml) = register_borrowed_patch_form(&context, "Catalog", "Items", "Main");
        fs::write(
            &form_xml,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Attributes/></Form>"#,
        )
        .unwrap();
        let mut args = patch_method_args();
        args.insert("ModulePath".to_string(), json!("Catalog.Items.Form.Main"));
        let module = context
            .cwd
            .join("ext/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl");

        let outcome = patch_extension_method(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("direct BaseForm"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_wrong_namespace_duplicate_registration() {
        let context = temp_context("patch-wrong-namespace-registration");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let owner = context.cwd.join("ext/Configuration.xml");
        let text = fs::read_to_string(&owner).unwrap().replace(
            "<CommonModule>GuardedModule</CommonModule>",
            "<CommonModule>GuardedModule</CommonModule><evil:CommonModule xmlns:evil=\"urn:evil\">GuardedModule</evil:CommonModule>",
        );
        fs::write(&owner, text).unwrap();
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = patch_extension_method(&patch_method_args(), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("is not registered"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_wrong_namespace_duplicate_base_form() {
        let context = temp_context("patch-wrong-namespace-base-form");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let (_, _, form_xml) = register_borrowed_patch_form(&context, "Catalog", "Items", "Main");
        let text = fs::read_to_string(&form_xml).unwrap().replace(
            "</Form>",
            "<evil:BaseForm xmlns:evil=\"urn:evil\" version=\"2.20\"/></Form>",
        );
        fs::write(&form_xml, text).unwrap();
        let mut args = patch_method_args();
        args.insert("ModulePath".to_string(), json!("Catalog.Items.Form.Main"));
        let module = context
            .cwd
            .join("ext/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl");

        let outcome = patch_extension_method(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("direct BaseForm"),
            "{outcome:?}"
        );
        assert!(!module.exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_unsupported_direct_module_role() {
        let context = temp_context("patch-unsupported-direct-module");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "Catalog", "Items", "<ChildObjects/>");
        for module_path in [
            "Catalog.Items.CommandModule",
            "Catalog.Items.ArbitraryModule",
            "InformationRegister.Items.ObjectModule",
        ] {
            let mut args = patch_method_args();
            args.insert("ModulePath".to_string(), json!(module_path));

            let outcome = patch_extension_method(&args, &context);

            assert!(!outcome.ok, "{module_path}: {outcome:?}");
            assert!(
                outcome
                    .errors
                    .join("\n")
                    .contains("is not supported by the cfe.patch_method grammar"),
                "{module_path}: {outcome:?}"
            );
        }
        assert!(!context
            .cwd
            .join("ext/Catalogs/Items/Ext/CommandModule.bsl")
            .exists());
        assert!(!context
            .cwd
            .join("ext/Catalogs/Items/Ext/ArbitraryModule.bsl")
            .exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_rejects_invalid_method_name_and_context_without_writes() {
        let invalid_cases = [
            ("MethodName", "Bad-Name"),
            ("MethodName", "Bad.Name"),
            ("MethodName", "Bad\"Name"),
            ("MethodName", "Bad\nName"),
            ("Context", "НаСервере\n&НаКлиенте"),
            ("Context", "AtServer"),
        ];
        for (index, (argument, value)) in invalid_cases.into_iter().enumerate() {
            let context = temp_context(&format!("patch-invalid-bsl-argument-{index}"));
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
            register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
            let mut args = patch_method_args();
            args.insert(argument.to_string(), json!(value));
            let module = context
                .cwd
                .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

            let outcome = patch_extension_method(&args, &context);

            assert!(!outcome.ok, "{argument}={value:?}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains(argument),
                "{argument}={value:?}: {outcome:?}"
            );
            assert!(!module.exists(), "{argument}={value:?}: {outcome:?}");
            assert!(
                outcome.changes.is_empty(),
                "{argument}={value:?}: {outcome:?}"
            );
            assert!(
                outcome.artifacts.is_empty(),
                "{argument}={value:?}: {outcome:?}"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_patch_method_rejects_module_path_that_escapes_extension_root() {
        let context = temp_context("patch-module-path-escape");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        let outside = std::env::temp_dir().join(format!(
            "unica-cfe-patch-escape-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let outside_module = outside.join("Ext/ObjectModule.bsl");
        let mut args = patch_method_args();
        args.insert(
            "ModulePath".to_string(),
            json!(format!("Catalog.{}.ObjectModule", outside.display())),
        );

        let outcome = patch_extension_method(&args, &context);
        let escaped = outside_module.exists();
        let errors = outcome.errors.join("\n");
        let ok = outcome.ok;
        let debug = format!("{outcome:?}");
        let _ = fs::remove_dir_all(&outside);
        let _ = fs::remove_dir_all(&context.cwd);

        assert!(!ok, "{debug}");
        assert!(
            errors.contains("valid Unicode XML NCName and a single path component"),
            "{debug}"
        );
        assert!(!escaped, "{debug}");
    }

    #[test]
    fn cfe_patch_method_rejects_non_component_and_non_exact_module_paths() {
        let context = temp_context("patch-invalid-module-paths");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);

        for module_path in [
            r"Catalog.\tmp\escape.ObjectModule",
            r"Catalog.C:\tmp\escape.ObjectModule",
            "Catalog...ObjectModule",
            "Catalog.Safe.ObjectModule.Extra",
            "CommonModule.Safe.Extra",
        ] {
            let mut args = patch_method_args();
            args.insert("ModulePath".to_string(), json!(module_path));

            let outcome = patch_extension_method(&args, &context);

            assert!(!outcome.ok, "{module_path}: {outcome:?}");
        }
        assert!(!context
            .cwd
            .join(r"ext/Catalogs/\tmp\escape/Ext/ObjectModule.bsl")
            .exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_binds_exact_existing_bsl_preimage() {
        let context = temp_context("patch-bsl-preimage");
        write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");
        write_file(&module, "Procedure Existing()\nEndProcedure\n");
        let concurrent = b"Procedure Concurrent()\nEndProcedure\n".to_vec();
        let module_for_hook = module.clone();
        let concurrent_for_hook = concurrent.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&module_for_hook, concurrent_for_hook).unwrap(),
            || patch_extension_method(&patch_method_args(), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("changed"), "{outcome:?}");
        assert_eq!(fs::read(&module).unwrap(), concurrent);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_patch_method_binds_owner_snapshot_used_for_name_prefix() {
        let context = temp_context("patch-owner-preimage");
        let (_, _, extension_owner) =
            write_minimal_borrow_fixture(&context, "2.20", "2.20", "2.20", None);
        register_borrowed_patch_object(&context, "CommonModule", "GuardedModule", "");
        let concurrent_owner = fs::read_to_string(&extension_owner)
            .unwrap()
            .replacen(
                "<NamePrefix>GE_</NamePrefix>",
                "<NamePrefix>NEW_</NamePrefix>",
                1,
            )
            .into_bytes();
        let owner_for_hook = extension_owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();
        let module = context
            .cwd
            .join("ext/CommonModules/GuardedModule/Ext/Module.bsl");

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, concurrent_for_hook).unwrap(),
            || patch_extension_method(&patch_method_args(), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&extension_owner).unwrap(), concurrent_owner);
        assert!(!module.exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_uses_active_format_with_supported_base_config() {
        let context = temp_context("init-format-version");
        let src = context.cwd.join("src");
        write_file(
            &src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &src.join("Languages").join("Русский.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Language uuid="77777777-7777-7777-7777-777777777777"/>
</MetaDataObject>
"#,
        );

        let mut args = Map::new();
        args.insert("Name".to_string(), json!("ParityExtension"));
        args.insert("OutputDir".to_string(), json!("ext"));
        args.insert("ConfigPath".to_string(), json!("src/Configuration.xml"));

        let outcome = create_extension_scaffold(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        for path in [
            context.cwd.join("ext").join("Configuration.xml"),
            context
                .cwd
                .join("ext")
                .join("Languages")
                .join("Русский.xml"),
            context
                .cwd
                .join("ext")
                .join("Roles")
                .join("ParityExtension_ОсновнаяРоль.xml"),
        ] {
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                text.contains(r#"version="2.20""#),
                "{} did not use the active MDClasses format version:\n{text}",
                path.display()
            );
            assert!(!text.contains(r#"version="2.17""#), "{text}");
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_rejects_concurrent_base_format_owner_change() {
        let context = temp_context("init-base-owner-guard");
        let base_owner = context.cwd.join("src/Configuration.xml");
        write_file(
            &base_owner,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>BaseConfiguration</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &context.cwd.join("src/Languages/Русский.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Language uuid="77777777-7777-7777-7777-777777777777"/>
</MetaDataObject>
"#,
        );
        let concurrent_base = fs::read_to_string(&base_owner)
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        let base_for_hook = base_owner.clone();
        let concurrent_for_hook = concurrent_base.clone();
        let args = Map::from_iter([
            ("Name".to_string(), json!("OwnerGuardExtension")),
            ("OutputDir".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src/Configuration.xml")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let outcome = with_before_commit_hook(
            move |_| fs::write(&base_for_hook, concurrent_for_hook).unwrap(),
            || create_extension_scaffold(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&base_owner).unwrap(), concurrent_base);
        assert!(!context.cwd.join("ext").exists(), "{outcome:?}");
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_binds_the_exact_base_bytes_used_for_derived_properties() {
        let context = temp_context("init-base-derived-preimage");
        let base_owner = context.cwd.join("src/Configuration.xml");
        write_file(
            &base_owner,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>BaseConfiguration</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        let same_format_replacement = fs::read_to_string(&base_owner)
            .unwrap()
            .replacen("Version8_3_25", "Version8_3_24", 1)
            .into_bytes();
        let base_for_hook = base_owner.clone();
        let replacement_for_hook = same_format_replacement.clone();
        let args = Map::from_iter([
            ("Name".to_string(), json!("DerivedSnapshotExtension")),
            ("OutputDir".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src/Configuration.xml")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let outcome = with_cfe_init_after_base_read_hook(
            move || fs::write(&base_for_hook, replacement_for_hook).unwrap(),
            || create_extension_scaffold(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("changed after planning"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&base_owner).unwrap(), same_format_replacement);
        assert!(!context.cwd.join("ext").exists(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_mixed_older_base_and_newer_language_prioritizes_newer() {
        let context = temp_context("init-mixed-newer-priority");
        write_file(
            &context.cwd.join("src/Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>BaseConfiguration</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        write_file(
            &context.cwd.join("src/Languages/Русский.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21">
	<Language uuid="77777777-7777-7777-7777-777777777777"/>
</MetaDataObject>
"#,
        );
        let args = Map::from_iter([
            ("Name".to_string(), json!("MixedVersionExtension")),
            ("OutputDir".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src/Configuration.xml")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let outcome = create_extension_scaffold(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("newer than supported 2.20"), "{error}");
        assert!(!error.contains("re-export"), "{error}");
        assert!(!context.cwd.join("ext").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_rejects_new_output_inside_older_configuration_source_set() {
        let context = temp_context("init-older-containing-owner");
        write_file(
            &context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let owner = context.cwd.join("src/Configuration.xml");
        write_file(
            &owner,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19">
	<Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        );
        let owner_before = fs::read(&owner).unwrap();
        let args = Map::from_iter([
            ("Name".to_string(), json!("NestedExtension")),
            ("OutputDir".to_string(), json!("src/Extensions/Nested")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let outcome = create_extension_scaffold(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("older than supported 2.20"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), owner_before);
        assert!(!context.cwd.join("src/Extensions").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_mixed_older_output_owner_and_newer_base_prioritizes_newer() {
        let context = temp_context("init-output-owner-mixed-newer-priority");
        write_file(
            &context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let output_owner = context.cwd.join("src/Configuration.xml");
        write_file(
            &output_owner,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19">
	<Configuration uuid="55555555-5555-5555-5555-555555555555"/>
</MetaDataObject>
"#,
        );
        write_file(
            &context.cwd.join("base/Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21">
	<Configuration uuid="66666666-6666-6666-6666-666666666666">
		<Properties>
			<Name>NewerBase</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        let output_owner_before = fs::read(&output_owner).unwrap();
        let args = Map::from_iter([
            ("Name".to_string(), json!("NestedExtension")),
            ("OutputDir".to_string(), json!("src/Extensions/Nested")),
            ("ConfigPath".to_string(), json!("base/Configuration.xml")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let outcome = create_extension_scaffold(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(error.contains("newer than supported 2.20"), "{error}");
        assert!(!error.contains("re-export"), "{error}");
        assert_eq!(fs::read(&output_owner).unwrap(), output_owner_before);
        assert!(!context.cwd.join("src/Extensions").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_binds_exact_containing_owner_before_commit() {
        let context = temp_context("init-containing-owner-preimage");
        write_file(
            &context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let owner = context.cwd.join("src/Configuration.xml");
        write_file(
            &owner,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties><Name>Original</Name></Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        let concurrent_owner = fs::read_to_string(&owner)
            .unwrap()
            .replacen("<Name>Original</Name>", "<Name>Concurrent</Name>", 1)
            .into_bytes();
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();
        let args = Map::from_iter([
            ("Name".to_string(), json!("NestedExtension")),
            ("OutputDir".to_string(), json!("src/Extensions/Nested")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, concurrent_for_hook).unwrap(),
            || create_extension_scaffold(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
        assert!(!context.cwd.join("src/Extensions").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_binds_exact_base_language_bytes_used_for_uuid() {
        let context = temp_context("init-language-derived-preimage");
        write_file(
            &context.cwd.join("src/Configuration.xml"),
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>BaseConfiguration</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        let language = context.cwd.join("src/Languages/Русский.xml");
        write_file(
            &language,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Language uuid="77777777-7777-7777-7777-777777777777"/>
</MetaDataObject>
"#,
        );
        let concurrent_language = fs::read_to_string(&language)
            .unwrap()
            .replacen(
                "77777777-7777-7777-7777-777777777777",
                "88888888-8888-8888-8888-888888888888",
                1,
            )
            .into_bytes();
        let language_for_hook = language.clone();
        let concurrent_for_hook = concurrent_language.clone();
        let args = Map::from_iter([
            ("Name".to_string(), json!("LanguageSnapshotExtension")),
            ("OutputDir".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src/Configuration.xml")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let outcome = with_cfe_init_after_base_read_hook(
            move || fs::write(&language_for_hook, concurrent_for_hook).unwrap(),
            || create_extension_scaffold(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("changed after planning"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&language).unwrap(), concurrent_language);
        assert!(!context.cwd.join("ext").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn public_cfe_init_rejects_invalid_base_language_uuid_without_writes() {
        let context = temp_context("init-invalid-base-language-uuid");
        write_file(
            &context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let base_config = context.cwd.join("src/Configuration.xml");
        write_file(
            &base_config,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<CompatibilityMode>Version8_3_25</CompatibilityMode>
			<InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#,
        );
        let base_language = context.cwd.join("src/Languages/Русский.xml");
        write_file(
            &base_language,
            r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Language uuid="not-a-uuid"/>
</MetaDataObject>
"#,
        );
        let config_before = fs::read(&base_config).unwrap();
        let language_before = fs::read(&base_language).unwrap();
        let args = Map::from_iter([
            ("cwd".to_string(), json!(context.cwd.display().to_string())),
            ("dryRun".to_string(), json!(false)),
            ("Name".to_string(), json!("ParityExtension")),
            ("OutputDir".to_string(), json!("ext")),
            ("ConfigPath".to_string(), json!("src/Configuration.xml")),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.cfe.init", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        let errors = result.errors.join("\n");
        assert!(errors.contains("Русский.xml"), "{result:?}");
        assert!(errors.contains("not-a-uuid"), "{result:?}");
        assert!(errors.contains("UUID"), "{result:?}");
        assert_eq!(fs::read(&base_config).unwrap(), config_before);
        assert_eq!(fs::read(&base_language).unwrap(), language_before);
        assert!(!context.cwd.join("ext").exists(), "{result:?}");
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.artifacts.is_empty(), "{result:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn public_cfe_init_semantic_post_validation_failure_rolls_back_all_files() {
        let context = temp_context("init-semantic-post-validation-rollback");
        let args = Map::from_iter([
            ("cwd".to_string(), json!(context.cwd.display().to_string())),
            ("dryRun".to_string(), json!(false)),
            ("Name".to_string(), json!("ParityExtension")),
            ("OutputDir".to_string(), json!("ext")),
        ]);

        let result = with_cfe_init_semantic_post_validation_failure(|| {
            UnicaApplication::new()
                .call_tool("unica.cfe.init", &args)
                .unwrap()
        });

        assert!(!result.ok, "{result:?}");
        assert!(
            result
                .errors
                .join("\n")
                .contains("cfe.init semantic post-validation failure"),
            "{result:?}"
        );
        assert!(!context.cwd.join("ext").exists(), "{result:?}");
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.artifacts.is_empty(), "{result:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_without_base_emits_only_active_format() {
        let context = temp_context("init-active-format-without-base");
        let mut args = Map::new();
        args.insert("Name".to_string(), json!("NoBaseExtension"));
        args.insert("OutputDir".to_string(), json!("ext"));

        let outcome = create_extension_scaffold(&args, &context);

        assert!(outcome.ok, "{:?}", outcome.errors);
        for path in [
            context.cwd.join("ext/Configuration.xml"),
            context.cwd.join("ext/Languages/Русский.xml"),
            context
                .cwd
                .join("ext/Roles/NoBaseExtension_ОсновнаяРоль.xml"),
        ] {
            let generated = fs::read_to_string(&path).unwrap();
            assert!(generated.contains(r#"version="2.20""#), "{generated}");
            assert!(!generated.contains(r#"version="2.17""#), "{generated}");
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_rejects_partial_existing_language_or_role_without_creating_other_files() {
        for existing in ["language", "role"] {
            let context = temp_context(&format!("init-partial-{existing}"));
            let config = context.cwd.join("ext/Configuration.xml");
            let language = context.cwd.join("ext/Languages/Русский.xml");
            let role = context
                .cwd
                .join("ext/Roles/ParityExtension_ОсновнаяРоль.xml");
            let existing_path = if existing == "language" {
                &language
            } else {
                &role
            };
            write_file(existing_path, "pre-existing-bytes\n");
            let before = [
                (config.clone(), fs::read(&config).ok()),
                (language.clone(), fs::read(&language).ok()),
                (role.clone(), fs::read(&role).ok()),
            ];
            let args = Map::from_iter([
                ("Name".to_string(), json!("ParityExtension")),
                ("OutputDir".to_string(), json!("ext")),
            ]);

            let outcome = create_extension_scaffold(&args, &context);

            assert!(!outcome.ok, "{existing}: {outcome:?}");
            assert!(
                outcome.errors.iter().any(|error| {
                    error.contains("create-only")
                        && error.contains(existing_path.file_name().unwrap().to_str().unwrap())
                }),
                "{existing}: {outcome:?}"
            );
            for (path, expected) in before {
                assert_eq!(fs::read(&path).ok(), expected, "{}", path.display());
            }
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_init_rejects_unsafe_names_and_invalid_enums_before_writing() {
        let cases = [
            ("name", "Name", "../BadExtension"),
            ("prefix", "NamePrefix", "../"),
            ("purpose", "Purpose", "Bogus</Purpose><Injected>"),
            (
                "compatibility",
                "CompatibilityMode",
                "Version8_3_24</ConfigurationExtensionCompatibilityMode><Injected>",
            ),
            (
                "future-compatibility-8-3-28",
                "CompatibilityMode",
                "Version8_3_28",
            ),
            (
                "future-compatibility-8-5-1",
                "CompatibilityMode",
                "Version8_5_1",
            ),
        ];
        for (case, key, value) in cases {
            let context = temp_context(&format!("init-invalid-{case}"));
            let mut args = Map::from_iter([
                ("Name".to_string(), json!("ParityExtension")),
                ("OutputDir".to_string(), json!("ext")),
            ]);
            args.insert(key.to_string(), json!(value));

            let outcome = create_extension_scaffold(&args, &context);

            assert!(!outcome.ok, "{case}: {outcome:?}");
            assert!(!context.cwd.join("ext").exists(), "{case}: {outcome:?}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_init_rejects_invalid_compatibility_enums_from_base_before_writing() {
        for (case, property, invalid, compatibility, interface_mode) in [
            (
                "compatibility",
                "CompatibilityMode",
                "Bogus",
                "Bogus",
                "Taxi",
            ),
            (
                "future-compatibility-8-3-28",
                "CompatibilityMode",
                "Version8_3_28",
                "Version8_3_28",
                "Taxi",
            ),
            (
                "future-compatibility-8-5-1",
                "CompatibilityMode",
                "Version8_5_1",
                "Version8_5_1",
                "Taxi",
            ),
            (
                "interface",
                "InterfaceCompatibilityMode",
                "Bogus",
                "Version8_3_25",
                "Bogus",
            ),
            (
                "interface-taxi-8-5",
                "InterfaceCompatibilityMode",
                "TaxiEnableVersion8_5",
                "Version8_3_25",
                "TaxiEnableVersion8_5",
            ),
            (
                "interface-8-5-enable-taxi",
                "InterfaceCompatibilityMode",
                "Version8_5EnableTaxi",
                "Version8_3_25",
                "Version8_5EnableTaxi",
            ),
            (
                "interface-8-5",
                "InterfaceCompatibilityMode",
                "Version8_5",
                "Version8_3_25",
                "Version8_5",
            ),
            (
                "interface-8-3-24",
                "InterfaceCompatibilityMode",
                "Version8_3_24",
                "Version8_3_25",
                "Version8_3_24",
            ),
        ] {
            let context = temp_context(&format!("init-invalid-base-enum-{case}"));
            write_file(
                &context.cwd.join("src/Configuration.xml"),
                &format!(
                    r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration uuid="55555555-5555-5555-5555-555555555555">
		<Properties>
			<Name>ParityConfiguration</Name>
			<CompatibilityMode>{compatibility}</CompatibilityMode>
			<InterfaceCompatibilityMode>{interface_mode}</InterfaceCompatibilityMode>
		</Properties>
	</Configuration>
</MetaDataObject>
"#
                ),
            );
            let args = Map::from_iter([
                ("Name".to_string(), json!("ParityExtension")),
                ("OutputDir".to_string(), json!("ext")),
                ("ConfigPath".to_string(), json!("src")),
            ]);

            let outcome = create_extension_scaffold(&args, &context);

            assert!(!outcome.ok, "{case}: {outcome:?}");
            assert!(
                outcome
                    .errors
                    .iter()
                    .any(|error| error.contains(property) && error.contains(invalid)),
                "{case}: {outcome:?}"
            );
            assert!(!context.cwd.join("ext").exists(), "{case}: {outcome:?}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_init_post_write_failure_rolls_back_all_scaffold_files() {
        let context = temp_context("init-post-write-failure");
        let config = context.cwd.join("ext/Configuration.xml");
        let language = context.cwd.join("ext/Languages/Русский.xml");
        let role = context
            .cwd
            .join("ext/Roles/ParityExtension_ОсновнаяРоль.xml");
        let args = Map::from_iter([
            ("Name".to_string(), json!("ParityExtension")),
            ("OutputDir".to_string(), json!("ext")),
        ]);

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            create_extension_scaffold(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("post-write validation")));
        for path in [config, language, role] {
            assert!(!path.exists(), "{}", path.display());
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn cfe_init_rejects_unsupported_base_format_before_writing_output() {
        for version in ["2.19", "2.21"] {
            let context = temp_context(&format!("init-reject-base-{version}"));
            write_file(
                &context.cwd.join("src/Configuration.xml"),
                &format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration/></MetaDataObject>"#
                ),
            );
            let mut args = Map::new();
            args.insert("Name".to_string(), json!("RejectedExtension"));
            args.insert("OutputDir".to_string(), json!("ext"));
            args.insert("ConfigPath".to_string(), json!("src"));

            let outcome = create_extension_scaffold(&args, &context);

            assert!(!outcome.ok, "base {version} must be rejected");
            assert!(
                outcome.errors.join("\n").contains(version),
                "{:?}",
                outcome.errors
            );
            assert!(
                !context.cwd.join("ext").exists(),
                "base {version} must be rejected before creating output"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn cfe_init_rejects_invalid_base_before_writing_output() {
        for (name, xml) in [
            ("malformed", "<MetaDataObject"),
            ("wrong-root", "<Configuration version=\"2.20\"/>"),
            (
                "wrong-namespace",
                "<MetaDataObject xmlns=\"urn:wrong\" version=\"2.20\"/>",
            ),
            (
                "missing-version",
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"/>",
            ),
        ] {
            let context = temp_context(&format!("init-invalid-base-{name}"));
            write_file(&context.cwd.join("src/Configuration.xml"), xml);
            let mut args = Map::new();
            args.insert("Name".to_string(), json!("RejectedExtension"));
            args.insert("OutputDir".to_string(), json!("ext"));
            args.insert("ConfigPath".to_string(), json!("src"));

            let outcome = create_extension_scaffold(&args, &context);

            assert!(!outcome.ok, "base {name} must be rejected");
            assert!(
                !context.cwd.join("ext").exists(),
                "base {name} must be rejected before creating output"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
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
}
