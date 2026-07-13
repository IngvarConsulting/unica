//! Failure-contained preparation for object-scoped Designer dumps.
//!
//! A partial dump must never write platform output into the working source
//! before Unica has classified divergence. This module prepares an isolated
//! source set and a pinned private configuration pair. Both the primary file
//! and its optional local overlay live inside the owned transaction, so the
//! runner cannot rediscover mutable workspace configuration after preflight.

#[cfg(test)]
use crate::domain::project_sources::{
    discover_project_source_map, ProjectSourceSet, SourceFormat, SourceSetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::source_sync::SourceSyncRepository;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_yaml::Value as YamlValue;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const TRANSACTION_PREFIX: &str = "shadow-dump-";
const TRANSACTION_OWNERSHIP_MARKER: &str = "ownership.json";
const TRANSACTION_OWNERSHIP_TEMP: &str = ".ownership.json.tmp";
const TRANSACTION_OWNERSHIP_SCHEMA_VERSION: u32 = 1;
const PRIMARY_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShadowTransactionOwnership {
    schema_version: u32,
    transaction_id: String,
}

pub(crate) struct ShadowPlatformSeeds<'a> {
    pub configuration: &'a [u8],
    pub config_dump_info: Option<&'a [u8]>,
}

#[derive(Debug)]
pub(crate) struct ShadowDumpPreparation {
    transaction_root: PathBuf,
    transaction_dir: PathBuf,
    shadow_source_dir: PathBuf,
    temporary_config_path: PathBuf,
    local_config_path: PathBuf,
    runtime_args: Map<String, Value>,
    cleaned: bool,
}

impl ShadowDumpPreparation {
    /// Prepare a shadow dump exclusively from bytes pinned by the application
    /// while it holds the lifecycle lock. This method deliberately never
    /// reads `v8project*.yaml` or platform root seeds from the workspace.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_pinned(
        context: &WorkspaceContext,
        source_set: &str,
        objects: &[String],
        caller_args: &Map<String, Value>,
        original_source_root: &Path,
        primary_bytes: &[u8],
        local_bytes: Option<&[u8]>,
        seeds: ShadowPlatformSeeds<'_>,
    ) -> Result<Self, String> {
        let (objects, mut primary_yaml) = validate_pinned_shadow_inputs(
            source_set,
            objects,
            caller_args,
            primary_bytes,
            local_bytes,
        )?;
        let workspace_root = canonical_directory(&context.workspace_root, "workspace root")?;
        let original_source_root =
            canonical_directory(original_source_root, "selected source-set root")?;
        if !original_source_root.starts_with(&workspace_root) {
            return Err(format!(
                "source-set `{source_set}` resolves outside the workspace"
            ));
        }
        let transaction_root = transaction_root(context)?;
        ensure_cache_transaction_root(&transaction_root)?;
        let (_transaction_id, transaction_dir) =
            create_unique_owned_transaction(&transaction_root)?;
        let mut pending = PendingCleanup::new(transaction_root.clone(), transaction_dir.clone());
        let shadow_source_dir = transaction_dir.join("source");
        let shadow_work_dir = transaction_dir.join("work");
        create_private_child_directory(&shadow_source_dir)?;
        create_private_child_directory(&shadow_work_dir)?;

        seed_platform_root_from_bytes(&shadow_source_dir, seeds)?;
        replace_selected_source_path(&mut primary_yaml, source_set, &shadow_source_dir)?;
        let temporary_yaml = serde_yaml::to_string(&primary_yaml)
            .map_err(|error| format!("failed to serialize temporary project config: {error}"))?;
        let temporary_config_path = transaction_dir.join(PRIMARY_CONFIG_NAME);
        let local_config_path = transaction_dir.join(LOCAL_CONFIG_NAME);
        write_private_atomic_file(&temporary_config_path, temporary_yaml.as_bytes())?;
        if let Some(local_bytes) = local_bytes {
            write_private_atomic_file(&local_config_path, local_bytes)?;
        }

        let mut runtime_args = caller_args.clone();
        let platform_objects = objects
            .iter()
            .map(|selector| {
                selector
                    .split_once(':')
                    .map(|(object_type, name)| format!("{object_type}.{name}"))
                    .ok_or_else(|| {
                        format!("normalized selector `{selector}` cannot be converted for Designer")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        runtime_args.remove("force");
        runtime_args.remove("object");
        runtime_args.remove("objects");
        runtime_args.insert("objects".to_string(), serde_json::json!(platform_objects));
        runtime_args.insert(
            "config".to_string(),
            Value::String(temporary_config_path.display().to_string()),
        );
        runtime_args.insert(
            "workdir".to_string(),
            Value::String(shadow_work_dir.display().to_string()),
        );
        runtime_args.insert(
            "sourceSet".to_string(),
            Value::String(source_set.to_string()),
        );

        pending.disarm();
        Ok(Self {
            transaction_root,
            transaction_dir,
            shadow_source_dir,
            temporary_config_path,
            local_config_path,
            runtime_args,
            cleaned: false,
        })
    }

    /// Test-only compatibility path. Production callers must provide already
    /// pinned inputs through `prepare_pinned`.
    #[cfg(test)]
    pub(crate) fn prepare(
        context: &WorkspaceContext,
        source_set: &str,
        objects: &[String],
        caller_args: &Map<String, Value>,
    ) -> Result<Self, String> {
        validate_partial_dump_request(source_set, caller_args)?;
        let objects = normalize_objects(objects)?;
        if objects != normalized_caller_objects(caller_args)? {
            return Err(
                "normalized shadow selectors do not match caller object selectors".to_string(),
            );
        }
        let workspace_root = canonical_directory(&context.workspace_root, "workspace root")?;
        let primary_config_path = context.workspace_root.join(PRIMARY_CONFIG_NAME);
        ensure_regular_file_without_symlink(&primary_config_path, "primary project config")?;
        let canonical_primary = fs::canonicalize(&primary_config_path).map_err(|error| {
            format!(
                "failed to canonicalize primary project config {}: {error}",
                primary_config_path.display()
            )
        })?;
        validate_requested_config(caller_args, context, &canonical_primary)?;

        let primary_bytes = fs::read(&primary_config_path).map_err(|error| {
            format!(
                "failed to read primary project config {}: {error}",
                primary_config_path.display()
            )
        })?;
        let source_map = discover_project_source_map(&context.workspace_root)?;
        let configured_primary = source_map
            .config_path
            .as_deref()
            .ok_or_else(|| "shadow dump requires the default v8project.yaml".to_string())?;
        let configured_primary = fs::canonicalize(configured_primary)
            .map_err(|error| format!("failed to canonicalize configured project file: {error}"))?;
        if configured_primary != canonical_primary {
            return Err("shadow dump requires the default v8project.yaml".to_string());
        }
        let selected = select_platform_configuration(&source_map.source_sets, source_set)?;
        let original_source_root = canonical_directory(
            &context.workspace_root.join(&selected.path),
            "selected source-set root",
        )?;
        if !original_source_root.starts_with(&workspace_root) {
            return Err(format!(
                "source-set `{source_set}` resolves outside the workspace"
            ));
        }

        let local_path = context.workspace_root.join(LOCAL_CONFIG_NAME);
        let local_bytes = read_optional_regular_file(&local_path, "local project config")?;
        let configuration = read_required_regular_file(
            &original_source_root.join("Configuration.xml"),
            "platform Configuration.xml seed",
        )?;
        let config_dump_info = read_optional_regular_file(
            &original_source_root.join("ConfigDumpInfo.xml"),
            "platform ConfigDumpInfo.xml seed",
        )?;
        Self::prepare_pinned(
            context,
            source_set,
            &objects,
            caller_args,
            &original_source_root,
            &primary_bytes,
            local_bytes.as_deref(),
            ShadowPlatformSeeds {
                configuration: &configuration,
                config_dump_info: config_dump_info.as_deref(),
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn transaction_dir(&self) -> &Path {
        &self.transaction_dir
    }

    pub(crate) fn shadow_source_dir(&self) -> &Path {
        &self.shadow_source_dir
    }

    pub(crate) fn temporary_config_path(&self) -> &Path {
        &self.temporary_config_path
    }

    pub(crate) fn runtime_config_paths(&self) -> [PathBuf; 2] {
        [
            self.temporary_config_path.clone(),
            self.local_config_path.clone(),
        ]
    }

    pub(crate) fn runtime_args(&self) -> &Map<String, Value> {
        &self.runtime_args
    }

    #[allow(
        dead_code,
        reason = "kept as the redacted reporting boundary for runtime integration"
    )]
    pub(crate) fn reported_args(&self) -> Map<String, Value> {
        redact_json_map(&self.runtime_args)
    }

    pub(crate) fn cleanup(&mut self) -> Result<(), String> {
        if self.cleaned {
            return Ok(());
        }
        validate_generated_paths(&self.transaction_root, &self.transaction_dir)?;
        let result = remove_owned_transaction(&self.transaction_dir);
        if result.is_ok() {
            self.cleaned = true;
        }
        result
    }
}

pub(crate) fn validate_pinned_shadow_config(
    source_set: &str,
    objects: &[String],
    caller_args: &Map<String, Value>,
    primary_bytes: &[u8],
    local_bytes: Option<&[u8]>,
) -> Result<(), String> {
    validate_pinned_shadow_inputs(source_set, objects, caller_args, primary_bytes, local_bytes)
        .map(|_| ())
}

fn validate_pinned_shadow_inputs(
    source_set: &str,
    objects: &[String],
    caller_args: &Map<String, Value>,
    primary_bytes: &[u8],
    local_bytes: Option<&[u8]>,
) -> Result<(Vec<String>, YamlValue), String> {
    validate_partial_dump_request(source_set, caller_args)?;
    let objects = normalize_objects(objects)?;
    if objects != normalized_caller_objects(caller_args)? {
        return Err("normalized shadow selectors do not match caller object selectors".to_string());
    }
    let mut primary_yaml = serde_yaml::from_slice::<YamlValue>(primary_bytes)
        .map_err(|error| format!("failed to parse pinned primary project config: {error}"))?;
    validate_primary_config_security(&primary_yaml)?;
    validate_designer_backend(&primary_yaml)?;
    if let Some(local_bytes) = local_bytes {
        serde_yaml::from_slice::<YamlValue>(local_bytes)
            .map_err(|error| format!("failed to parse pinned local project config: {error}"))?;
    }
    // Count and validate the selected entry without observing any filesystem.
    replace_selected_source_path(
        &mut primary_yaml,
        source_set,
        Path::new("/unica-private-shadow-preview"),
    )?;
    Ok((objects, primary_yaml))
}

impl Drop for ShadowDumpPreparation {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Remove artifacts left by an interrupted preparation.
///
/// The caller must hold the workspace source-sync lifecycle lock. Under that
/// lock each private transaction directory is an orphan. Recursive deletion
/// is authorized only by a durable marker whose UUID matches the exact
/// transaction directory; a filename prefix alone is never ownership proof.
pub(crate) fn recover_stale_shadow_dumps(
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    canonical_directory(&context.workspace_root, "workspace root")?;
    let transaction_root = transaction_root(context)?;
    let mut removed = Vec::new();
    let mut errors = Vec::new();

    for directory in controlled_cache_directories(&transaction_root)? {
        ensure_existing_directory_not_symlink(directory, "shadow cache path")?;
    }
    if transaction_root.exists() {
        for directory in controlled_cache_directories(&transaction_root)? {
            ensure_directory_without_symlink(directory, "shadow cache path")?;
        }
        for path in generated_entries(&transaction_root, is_transaction_name)? {
            match recover_owned_or_pre_marker_transaction(&path) {
                Ok(true) => removed.push(path),
                Ok(false) => {}
                Err(error) => errors.push(error),
            }
        }
    }

    if errors.is_empty() {
        removed.sort();
        Ok(removed)
    } else {
        Err(format!(
            "failed to recover shadow dump artifacts: {}",
            errors.join("; ")
        ))
    }
}

fn validate_partial_dump_request(
    selected_source_set: &str,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if selected_source_set.trim().is_empty() || selected_source_set.chars().any(char::is_control) {
        return Err("shadow dump source-set name must be non-blank and printable".to_string());
    }
    match args.get("operation").and_then(Value::as_str) {
        Some("dump") => {}
        Some(_) | None => {
            return Err("shadow preparation requires runtime operation `dump`".to_string())
        }
    }
    match args.get("mode").and_then(Value::as_str) {
        Some("partial") => {}
        Some(_) | None => return Err("shadow preparation requires dump mode `partial`".to_string()),
    }
    if args.contains_key("extension") {
        return Err("shadow partial dump does not support extensions".to_string());
    }
    if args.get("force").is_some_and(|force| !force.is_boolean()) {
        return Err("runtime argument `force` must be boolean".to_string());
    }
    if let Some(requested) = args.get("sourceSet") {
        let requested = requested
            .as_str()
            .ok_or_else(|| "runtime argument `sourceSet` must be string".to_string())?;
        if requested != selected_source_set {
            return Err(format!(
                "runtime sourceSet `{requested}` does not match selected source-set `{selected_source_set}`"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn validate_requested_config(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    canonical_primary: &Path,
) -> Result<(), String> {
    let Some(config) = args.get("config") else {
        return Ok(());
    };
    let config = config
        .as_str()
        .ok_or_else(|| "runtime argument `config` must be string".to_string())?;
    let path = PathBuf::from(config);
    let path = if path.is_absolute() {
        path
    } else {
        context.cwd.join(path)
    };
    let canonical = fs::canonicalize(&path).map_err(|error| {
        format!(
            "failed to resolve requested project config {}: {error}",
            path.display()
        )
    })?;
    if canonical != canonical_primary {
        return Err("shadow dump supports only the default v8project.yaml".to_string());
    }
    Ok(())
}

#[cfg(test)]
fn select_platform_configuration<'a>(
    source_sets: &'a [ProjectSourceSet],
    selected_name: &str,
) -> Result<&'a ProjectSourceSet, String> {
    let matches = source_sets
        .iter()
        .filter(|source_set| source_set.name == selected_name)
        .collect::<Vec<_>>();
    let selected = match matches.as_slice() {
        [] => return Err(format!("source-set `{selected_name}` was not found")),
        [selected] => *selected,
        [_, _, ..] => {
            return Err(format!(
                "source-set `{selected_name}` is ambiguous in v8project.yaml"
            ))
        }
    };
    match (selected.kind, selected.source_format) {
        (SourceSetKind::Configuration, SourceFormat::PlatformXml) => Ok(selected),
        (SourceSetKind::Configuration, SourceFormat::Edt) => Err(format!(
            "source-set `{selected_name}` uses EDT; shadow partial dump supports platform XML only"
        )),
        (SourceSetKind::Configuration, SourceFormat::Unknown) => Err(format!(
            "source-set `{selected_name}` has unknown source format"
        )),
        (SourceSetKind::Configuration, SourceFormat::Invalid) => Err(format!(
            "source-set `{selected_name}` has conflicting source format evidence"
        )),
        (SourceSetKind::Extension, _)
        | (SourceSetKind::ExternalProcessor, _)
        | (SourceSetKind::ExternalReport, _) => Err(format!(
            "source-set `{selected_name}` is not a configuration source-set"
        )),
    }
}

fn normalize_objects(objects: &[String]) -> Result<Vec<String>, String> {
    if objects.is_empty() {
        return Err("shadow partial dump requires at least one object selector".to_string());
    }
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for object in objects {
        if object.chars().any(char::is_control) {
            return Err("partial dump selector must not contain control characters".to_string());
        }
        let object = object.trim();
        let (object_type, object_name) = object
            .split_once(':')
            .ok_or_else(|| "partial dump selector must use `TYPE:NAME` format".to_string())?;
        if object_name.contains(':') {
            return Err("partial dump selector must contain exactly one `:` separator".to_string());
        }
        let object_type = object_type.trim();
        let object_name = object_name.trim();
        if object_type.is_empty() || object_name.is_empty() {
            return Err("partial dump selector must contain non-blank TYPE and NAME".to_string());
        }
        let object = format!("{object_type}:{object_name}");
        if seen.insert(object.clone()) {
            normalized.push(object);
        }
    }
    Ok(normalized)
}

fn normalized_caller_objects(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut objects = Vec::new();
    if let Some(value) = args.get("object") {
        objects.push(
            value
                .as_str()
                .ok_or_else(|| "runtime argument `object` must be string".to_string())?
                .to_string(),
        );
    }
    if let Some(value) = args.get("objects") {
        let values = value
            .as_array()
            .ok_or_else(|| "runtime argument `objects` must be array".to_string())?;
        objects.extend(
            values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        "runtime argument `objects` must contain strings".to_string()
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    normalize_objects(&objects)
}

fn validate_primary_config_security(root: &YamlValue) -> Result<(), String> {
    validate_yaml_value_security(root, None)
}

fn validate_designer_backend(root: &YamlValue) -> Result<(), String> {
    let mapping = root
        .as_mapping()
        .ok_or_else(|| "primary project config root must be a mapping".to_string())?;
    let builder = match mapping.get(yaml_key("builder")) {
        Some(value) => value
            .as_str()
            .ok_or_else(|| "primary project config `builder` must be a string".to_string())?,
        None => "DESIGNER",
    };
    if !builder.eq_ignore_ascii_case("DESIGNER") {
        return Err(
            "shadow partial dump requires the DESIGNER builder; IBCMD object scope is unsafe"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_yaml_value_security(value: &YamlValue, parent_key: Option<&str>) -> Result<(), String> {
    match value {
        YamlValue::Null | YamlValue::Bool(_) | YamlValue::Number(_) => Ok(()),
        YamlValue::String(text) => {
            if parent_key.is_some_and(is_connection_key) && connection_contains_password(text) {
                return Err(
                    "primary project config contains a password-bearing connection; move credentials to v8project.local.yaml"
                        .to_string(),
                );
            }
            Ok(())
        }
        YamlValue::Sequence(items) => {
            for item in items {
                validate_yaml_value_security(item, parent_key)?;
            }
            Ok(())
        }
        YamlValue::Mapping(mapping) => {
            for (key, nested) in mapping {
                let key = key.as_str().ok_or_else(|| {
                    "primary project config contains a non-string mapping key".to_string()
                })?;
                if is_sensitive_key(key) {
                    return Err(
                        "primary project config contains a sensitive key; move credentials to v8project.local.yaml"
                            .to_string(),
                    );
                }
                validate_yaml_value_security(nested, Some(key))?;
            }
            Ok(())
        }
        YamlValue::Tagged(tagged) => validate_yaml_value_security(&tagged.value, parent_key),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let compact = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    ["password", "pwd", "token", "secret"]
        .iter()
        .any(|marker| compact.contains(marker))
}

fn is_connection_key(key: &str) -> bool {
    key.to_ascii_lowercase().contains("connection")
}

fn connection_contains_password(connection: &str) -> bool {
    let lower = connection.to_ascii_lowercase();
    let compact = lower
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    if compact.contains("pwd=") || compact.contains("password=") {
        return true;
    }
    if let Some((_scheme, remainder)) = lower.split_once("://") {
        let authority = remainder.split('/').next().unwrap_or(remainder);
        if authority
            .split_once('@')
            .is_some_and(|(userinfo, _host)| userinfo.contains(':'))
        {
            return true;
        }
    }
    lower.split_ascii_whitespace().any(|token| {
        token == "/p"
            || token.starts_with("/p=")
            || token.starts_with("/p\"")
            || token.starts_with("/p'")
            || token == "--password"
            || token.starts_with("--password=")
    })
}

fn replace_selected_source_path(
    root: &mut YamlValue,
    selected_name: &str,
    shadow_source_dir: &Path,
) -> Result<(), String> {
    let mapping = root
        .as_mapping_mut()
        .ok_or_else(|| "primary project config root must be a mapping".to_string())?;
    let source_sets = mapping
        .get_mut(yaml_key("source-set"))
        .ok_or_else(|| "primary project config has no `source-set`".to_string())?;
    let replacement = YamlValue::String(shadow_source_dir.display().to_string());
    let mut replacements = 0_usize;

    match source_sets {
        YamlValue::Sequence(entries) => {
            for entry in entries {
                let entry_mapping = entry.as_mapping_mut().ok_or_else(|| {
                    "primary project config source-set entries must be mappings".to_string()
                })?;
                let name = entry_mapping
                    .get(yaml_key("name"))
                    .and_then(YamlValue::as_str)
                    .unwrap_or("main");
                if name == selected_name {
                    entry_mapping.insert(yaml_key("path"), replacement.clone());
                    replacements += 1;
                }
            }
        }
        YamlValue::Mapping(entries) => {
            for (name, entry) in entries {
                let name = name.as_str().ok_or_else(|| {
                    "primary project config source-set names must be strings".to_string()
                })?;
                if name == selected_name {
                    let entry = entry.as_mapping_mut().ok_or_else(|| {
                        "primary project config source-set entries must be mappings".to_string()
                    })?;
                    entry.insert(yaml_key("path"), replacement.clone());
                    replacements += 1;
                }
            }
        }
        YamlValue::Null
        | YamlValue::Bool(_)
        | YamlValue::Number(_)
        | YamlValue::String(_)
        | YamlValue::Tagged(_) => {
            return Err("primary project config `source-set` must be a list or mapping".to_string())
        }
    }

    match replacements {
        1 => Ok(()),
        0 => Err(format!(
            "source-set `{selected_name}` was not found while writing shadow config"
        )),
        _ => Err(format!(
            "source-set `{selected_name}` is ambiguous while writing shadow config"
        )),
    }
}

fn yaml_key(key: &str) -> YamlValue {
    YamlValue::String(key.to_string())
}

fn seed_platform_root_from_bytes(
    shadow_root: &Path,
    seeds: ShadowPlatformSeeds<'_>,
) -> Result<(), String> {
    write_private_atomic_file(&shadow_root.join("Configuration.xml"), seeds.configuration)?;
    if let Some(config_dump_info) = seeds.config_dump_info {
        write_private_atomic_file(&shadow_root.join("ConfigDumpInfo.xml"), config_dump_info)?;
    }
    Ok(())
}

fn transaction_root(context: &WorkspaceContext) -> Result<PathBuf, String> {
    Ok(SourceSyncRepository::new(context)?
        .transaction_root()
        .join("transactions"))
}

fn ensure_cache_transaction_root(path: &Path) -> Result<(), String> {
    let directories = controlled_cache_directories(path)?;
    for directory in &directories {
        ensure_existing_directory_not_symlink(directory, "shadow cache path")?;
    }
    for directory in directories {
        if !directory.exists() {
            create_private_directory(directory).map_err(|error| {
                format!(
                    "failed to create shadow cache directory {}: {error}",
                    directory.display()
                )
            })?;
            let parent = directory.parent().ok_or_else(|| {
                format!(
                    "shadow cache directory {} has no parent",
                    directory.display()
                )
            })?;
            sync_directory(parent)?;
        }
        ensure_directory_without_symlink(directory, "shadow cache path")?;
    }
    Ok(())
}

/// Return the workspace-controlled source-sync namespace, excluding system ancestors.
///
/// `path` is always `<workspace>/.build/unica/source-sync/<workspace-id>/transactions`.
/// System roots such as macOS `/var -> /private/var` are trusted by the process,
/// while every component from the source-sync authority root down is checked.
fn controlled_cache_directories(path: &Path) -> Result<Vec<&Path>, String> {
    let mut directories = path.ancestors().take(5).collect::<Vec<_>>();
    if directories.len() != 5 {
        return Err(format!(
            "shadow transaction path {} has no complete cache namespace",
            path.display()
        ));
    }
    directories.reverse();
    Ok(directories)
}

fn ensure_existing_directory_not_symlink(path: &Path, description: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(format!(
            "{description} {} must be a non-symlink directory",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect {description} {}: {error}",
            path.display()
        )),
    }
}

fn create_unique_owned_transaction(transaction_root: &Path) -> Result<(Uuid, PathBuf), String> {
    for _attempt in 0..8 {
        let id = Uuid::new_v4();
        let path = transaction_root.join(format!("{TRANSACTION_PREFIX}{id}"));
        match create_private_directory(&path) {
            Ok(()) => {
                if let Err(error) = sync_directory(transaction_root) {
                    let _ = fs::remove_dir(&path);
                    return Err(error);
                }
                if let Err(error) = write_transaction_ownership(&path, id) {
                    let _ = recover_owned_or_pre_marker_transaction(&path);
                    return Err(error);
                }
                return Ok((id, path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create shadow transaction {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Err("failed to allocate a unique shadow transaction".to_string())
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn create_private_child_directory(path: &Path) -> Result<(), String> {
    create_private_directory(path).map_err(|error| {
        format!(
            "failed to create private shadow directory {}: {error}",
            path.display()
        )
    })?;
    sync_directory(
        path.parent()
            .ok_or_else(|| format!("shadow directory {} has no parent", path.display()))?,
    )
}

fn open_private_new_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))
}

fn write_and_sync_file(file: &mut File, path: &Path, bytes: &[u8]) -> Result<(), String> {
    file.write_all(bytes)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("failed to flush {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", path.display()))
}

fn write_transaction_ownership(transaction_dir: &Path, id: Uuid) -> Result<(), String> {
    let ownership = ShadowTransactionOwnership {
        schema_version: TRANSACTION_OWNERSHIP_SCHEMA_VERSION,
        transaction_id: id.hyphenated().to_string(),
    };
    let bytes = serde_json::to_vec(&ownership)
        .map_err(|error| format!("failed to serialize shadow ownership: {error}"))?;
    let temporary = transaction_dir.join(TRANSACTION_OWNERSHIP_TEMP);
    let marker = transaction_dir.join(TRANSACTION_OWNERSHIP_MARKER);
    let mut file = open_private_new_file(&temporary)?;
    write_and_sync_file(&mut file, &temporary, &bytes)?;
    fs::rename(&temporary, &marker).map_err(|error| {
        format!(
            "failed to commit shadow ownership {}: {error}",
            marker.display()
        )
    })?;
    sync_directory(transaction_dir)
}

fn write_private_atomic_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("private shadow file {} has no parent", path.display()))?;
    let stage = parent.join(format!(".unica-stage-{}.tmp", Uuid::new_v4()));
    let mut file = open_private_new_file(&stage)?;
    write_and_sync_file(&mut file, &stage, bytes)?;
    fs::rename(&stage, path).map_err(|error| {
        format!(
            "failed to commit private shadow file {}: {error}",
            path.display()
        )
    })?;
    sync_directory(parent)
}

#[cfg(test)]
fn read_required_regular_file(path: &Path, description: &str) -> Result<Vec<u8>, String> {
    read_optional_regular_file(path, description)?
        .ok_or_else(|| format!("{description} {} is missing", path.display()))
}

#[cfg(test)]
fn read_optional_regular_file(path: &Path, description: &str) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "{description} {} must be a regular non-symlink file",
            path.display()
        )),
        Ok(_) => fs::read(path)
            .map(Some)
            .map_err(|error| format!("failed to read {description} {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to inspect {description} {}: {error}",
            path.display()
        )),
    }
}

fn read_transaction_ownership(
    transaction_dir: &Path,
) -> Result<Option<ShadowTransactionOwnership>, String> {
    let marker = transaction_dir.join(TRANSACTION_OWNERSHIP_MARKER);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to inspect shadow ownership {}: {error}",
                marker.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "shadow ownership {} must be a regular non-symlink file",
            marker.display()
        ));
    }
    let first = fs::read(&marker).map_err(|error| {
        format!(
            "failed to read shadow ownership {}: {error}",
            marker.display()
        )
    })?;
    let second = fs::read(&marker).map_err(|error| {
        format!(
            "failed to verify shadow ownership {}: {error}",
            marker.display()
        )
    })?;
    if first != second {
        return Err(format!(
            "shadow ownership {} changed while it was read",
            marker.display()
        ));
    }
    let ownership =
        serde_json::from_slice::<ShadowTransactionOwnership>(&first).map_err(|error| {
            format!(
                "failed to parse shadow ownership {}: {error}",
                marker.display()
            )
        })?;
    let expected = transaction_dir
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(transaction_uuid)
        .ok_or_else(|| "shadow transaction name is not canonical".to_string())?;
    if ownership.schema_version != TRANSACTION_OWNERSHIP_SCHEMA_VERSION
        || ownership.transaction_id != expected.hyphenated().to_string()
    {
        return Err(format!(
            "shadow ownership {} does not match its transaction",
            marker.display()
        ));
    }
    Ok(Some(ownership))
}

fn canonical_directory(path: &Path, description: &str) -> Result<PathBuf, String> {
    ensure_directory_without_symlink(path, description)?;
    fs::canonicalize(path).map_err(|error| {
        format!(
            "failed to canonicalize {description} {}: {error}",
            path.display()
        )
    })
}

fn ensure_directory_without_symlink(path: &Path, description: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect {description} {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "{description} {} must be a non-symlink directory",
            path.display()
        ));
    }
    Ok(())
}

fn ensure_regular_file_without_symlink(path: &Path, description: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect {description} {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{description} {} must be a regular non-symlink file",
            path.display()
        ));
    }
    Ok(())
}

fn redact_json_map(args: &Map<String, Value>) -> Map<String, Value> {
    args.iter()
        .map(|(key, value)| {
            let value = if is_sensitive_key(key)
                || (is_connection_key(key)
                    && value.as_str().is_some_and(connection_contains_password))
            {
                Value::String("[REDACTED]".to_string())
            } else {
                redact_json_value(value)
            };
            (key.clone(), value)
        })
        .collect()
}

fn redact_json_value(value: &Value) -> Value {
    match value {
        Value::Object(mapping) => Value::Object(redact_json_map(mapping)),
        Value::Array(items) => Value::Array(items.iter().map(redact_json_value).collect()),
        Value::String(text) if connection_contains_password(text) => {
            Value::String("[REDACTED]".to_string())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

fn validate_generated_paths(transaction_root: &Path, transaction_dir: &Path) -> Result<(), String> {
    let transaction_id = transaction_dir
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(transaction_uuid);
    if transaction_dir.parent() != Some(transaction_root) || transaction_id.is_none() {
        return Err("refusing to clean paths outside the shadow dump namespace".to_string());
    }
    for directory in controlled_cache_directories(transaction_root)? {
        ensure_existing_directory_not_symlink(directory, "shadow cache path")?;
    }
    Ok(())
}

fn remove_generated_tree(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            fs::remove_file(path)
                .map_err(|error| format!("failed to remove {}: {error}", path.display()))
        }
        Ok(_) => fs::remove_dir_all(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn recover_owned_or_pre_marker_transaction(path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "failed to inspect shadow transaction {}: {error}",
                path.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "shadow transaction {} must be a non-symlink directory",
            path.display()
        ));
    }
    if read_transaction_ownership(path)?.is_some() {
        remove_owned_transaction(path)?;
        return Ok(true);
    }

    let entries = fs::read_dir(path)
        .map_err(|error| {
            format!(
                "failed to scan shadow transaction {}: {error}",
                path.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to read shadow transaction {}: {error}",
                path.display()
            )
        })?;
    if entries.is_empty() {
        fs::remove_dir(path)
            .map_err(|error| format!("failed to remove empty {}: {error}", path.display()))?;
        sync_parent_directory(path)?;
        return Ok(true);
    }
    if entries.len() == 1 && entries[0].file_name() == TRANSACTION_OWNERSHIP_TEMP {
        let temporary = entries[0].path();
        let metadata = fs::symlink_metadata(&temporary).map_err(|error| {
            format!(
                "failed to inspect incomplete shadow ownership {}: {error}",
                temporary.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "incomplete shadow ownership {} is not a regular file",
                temporary.display()
            ));
        }
        fs::remove_file(&temporary).map_err(|error| {
            format!(
                "failed to remove incomplete shadow ownership {}: {error}",
                temporary.display()
            )
        })?;
        sync_directory(path)?;
        fs::remove_dir(path).map_err(|error| {
            format!(
                "failed to remove incomplete shadow transaction {}: {error}",
                path.display()
            )
        })?;
        sync_parent_directory(path)?;
        return Ok(true);
    }
    Err(format!(
        "shadow transaction {} has no valid ownership marker and contains foreign entries",
        path.display()
    ))
}

fn remove_owned_transaction(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to inspect owned shadow transaction {}: {error}",
                path.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "owned shadow transaction {} must remain a non-symlink directory",
            path.display()
        ));
    }
    if read_transaction_ownership(path)?.is_none() {
        return match recover_owned_or_pre_marker_transaction(path)? {
            true => Ok(()),
            false => Ok(()),
        };
    }

    let mut removable = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| format!("failed to scan owned shadow transaction: {error}"))?
    {
        let entry = entry.map_err(|error| format!("failed to read shadow entry: {error}"))?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| "shadow transaction contains a non-UTF-8 entry".to_string())?
            .to_string();
        if name == TRANSACTION_OWNERSHIP_MARKER {
            continue;
        }
        if matches!(
            name.as_str(),
            "source" | "work" | PRIMARY_CONFIG_NAME | LOCAL_CONFIG_NAME
        ) || name.starts_with(".unica-stage-")
            || name == TRANSACTION_OWNERSHIP_TEMP
        {
            removable.push(entry.path());
        } else {
            return Err(format!(
                "owned shadow transaction {} contains unknown top-level entry `{name}`",
                path.display()
            ));
        }
    }
    removable.sort();
    for entry in removable {
        remove_generated_tree(&entry)?;
    }
    // Credential-bearing config and all generated output are durably gone
    // before the ownership proof is removed.
    sync_directory(path)?;
    let marker = path.join(TRANSACTION_OWNERSHIP_MARKER);
    fs::remove_file(&marker).map_err(|error| {
        format!(
            "failed to remove shadow ownership {}: {error}",
            marker.display()
        )
    })?;
    sync_directory(path)?;
    fs::remove_dir(path).map_err(|error| {
        format!(
            "failed to remove shadow transaction {}: {error}",
            path.display()
        )
    })?;
    sync_parent_directory(path)
}

fn sync_parent_directory(path: &Path) -> Result<(), String> {
    sync_directory(
        path.parent()
            .ok_or_else(|| format!("path {} has no parent directory", path.display()))?,
    )
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "failed to sync shadow directory {}: {error}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn generated_entries(root: &Path, predicate: fn(&str) -> bool) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("failed to scan {}: {error}", root.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to read entry in {}: {error}", root.display()))?;
        if entry.file_name().to_str().is_some_and(predicate) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_transaction_name(name: &str) -> bool {
    transaction_uuid(name).is_some()
}

fn transaction_uuid(name: &str) -> Option<Uuid> {
    canonical_uuid(name.strip_prefix(TRANSACTION_PREFIX)?)
}

fn canonical_uuid(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value)
        .ok()
        .filter(|uuid| uuid.hyphenated().to_string() == value)
}

struct PendingCleanup {
    transaction_root: PathBuf,
    transaction_dir: PathBuf,
    armed: bool,
}

impl PendingCleanup {
    fn new(transaction_root: PathBuf, transaction_dir: PathBuf) -> Self {
        Self {
            transaction_root,
            transaction_dir,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingCleanup {
    fn drop(&mut self) {
        if self.armed
            && validate_generated_paths(&self.transaction_root, &self.transaction_dir).is_ok()
        {
            let _ = remove_owned_transaction(&self.transaction_dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn prepares_isolated_config_seeds_raw_bytes_and_builds_safe_args() {
        let workspace = TestWorkspace::platform_sequence("shadow-prepare");
        let config_bytes = b"\xef\xbb\xbf<Configuration>\r\n</Configuration>\r\n";
        let dump_info_bytes = b"<ConfigDumpInfo version=\"42\"/>\r\n";
        fs::write(workspace.root.join("src/Configuration.xml"), config_bytes).unwrap();
        fs::write(
            workspace.root.join("src/ConfigDumpInfo.xml"),
            dump_info_bytes,
        )
        .unwrap();
        let local_path = workspace.root.join(LOCAL_CONFIG_NAME);
        let local_bytes = b"infobase:\n  user: Admin\n  password: local-secret\n";
        fs::write(&local_path, local_bytes).unwrap();
        let original_configuration =
            fs::read(workspace.root.join("src/Configuration.xml")).unwrap();

        let caller = json_map(serde_json::json!({
            "operation": "dump",
            "mode": "partial",
            "object": "Catalog:Items",
            "objects": ["Document:Order"],
            "force": true,
            "sourceSet": "main",
        }));
        let mut prepared = ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &[
                " Catalog : Items ".to_string(),
                "Catalog:Items".to_string(),
                "Document:Order".to_string(),
            ],
            &caller,
        )
        .unwrap();

        assert!(prepared.transaction_dir().starts_with(
            workspace
                .root
                .canonicalize()
                .unwrap()
                .join(".build/unica/source-sync"),
        ));
        assert!(prepared.shadow_source_dir().is_dir());
        assert!(prepared.transaction_dir().join("work").is_dir());
        assert_eq!(
            prepared.temporary_config_path().parent(),
            Some(prepared.transaction_dir())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for private_dir in [
                prepared.transaction_dir().to_path_buf(),
                prepared.shadow_source_dir().to_path_buf(),
                prepared.transaction_dir().join("work"),
            ] {
                assert_eq!(
                    fs::metadata(private_dir).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
            assert_eq!(
                fs::metadata(prepared.temporary_config_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            for seed in ["Configuration.xml", "ConfigDumpInfo.xml"] {
                assert_eq!(
                    fs::metadata(prepared.shadow_source_dir().join(seed))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        assert_eq!(fs::read(&local_path).unwrap(), local_bytes);
        assert_eq!(
            fs::read(prepared.transaction_dir().join(LOCAL_CONFIG_NAME)).unwrap(),
            local_bytes
        );
        assert_eq!(
            fs::read(prepared.shadow_source_dir().join("Configuration.xml")).unwrap(),
            config_bytes
        );
        assert_eq!(
            fs::read(prepared.shadow_source_dir().join("ConfigDumpInfo.xml")).unwrap(),
            dump_info_bytes
        );
        let temporary_yaml: YamlValue =
            serde_yaml::from_slice(&fs::read(prepared.temporary_config_path()).unwrap()).unwrap();
        assert_eq!(
            sequence_source_path(&temporary_yaml, "main"),
            prepared.shadow_source_dir().display().to_string()
        );
        assert_eq!(
            sequence_source_path(&temporary_yaml, "secondary"),
            "secondary"
        );
        assert_eq!(
            yaml_string_at(&temporary_yaml, &["infobase", "connection"]),
            "File=build/ib"
        );
        assert!(!fs::read_to_string(prepared.temporary_config_path())
            .unwrap()
            .contains("local-secret"));

        let runtime = prepared.runtime_args();
        assert!(runtime.get("force").is_none());
        assert!(runtime.get("object").is_none());
        assert_eq!(
            runtime.get("objects"),
            Some(&serde_json::json!(["Catalog.Items", "Document.Order"]))
        );
        assert_eq!(
            runtime.get("sourceSet"),
            Some(&Value::String("main".to_string()))
        );
        assert_eq!(
            runtime.get("config"),
            Some(&Value::String(
                prepared.temporary_config_path().display().to_string()
            ))
        );
        assert_eq!(
            runtime.get("workdir"),
            Some(&Value::String(
                prepared
                    .transaction_dir()
                    .join("work")
                    .display()
                    .to_string()
            ))
        );
        assert!(!serde_json::to_string(&prepared.reported_args())
            .unwrap()
            .contains("local-secret"));

        let transaction = prepared.transaction_dir().to_path_buf();
        let temporary_config = prepared.temporary_config_path().to_path_buf();
        prepared.cleanup().unwrap();
        prepared.cleanup().unwrap();
        assert!(!transaction.exists());
        assert!(!temporary_config.exists());
        assert_eq!(
            fs::read(workspace.root.join("src/Configuration.xml")).unwrap(),
            original_configuration
        );
    }

    #[test]
    fn drop_cleans_unique_preparations() {
        let workspace = TestWorkspace::platform_sequence("shadow-drop");
        let caller = partial_caller();
        let first = ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &["Catalog:Items".to_string()],
            &caller,
        )
        .unwrap();
        let second = ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &["Catalog:Items".to_string()],
            &caller,
        )
        .unwrap();
        assert_ne!(first.transaction_dir(), second.transaction_dir());
        let first_transaction = first.transaction_dir().to_path_buf();
        let first_config = first.temporary_config_path().to_path_buf();
        drop(first);
        assert!(!first_transaction.exists());
        assert!(!first_config.exists());
        assert!(second.transaction_dir().exists());
    }

    #[test]
    fn mapping_source_sets_are_rejected_as_non_runtime_topology() {
        let workspace = TestWorkspace::new(
            "shadow-mapping",
            r#"
format: DESIGNER
builder: DESIGNER
workPath: build
infobase:
  connection: File=build/ib
source-set:
  main:
    type: CONFIGURATION
    path: src
  secondary:
    type: CONFIGURATION
    path: secondary
"#,
        );
        workspace.seed_platform_roots();
        let error = ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap_err();
        assert!(error.contains("must be a list"));
        assert!(!workspace.cache.exists());
        assert!(!workspace.root.join(".unica-shadow-dump.yaml").exists());
    }

    #[test]
    fn rejects_secrets_in_primary_config_without_echoing_values() {
        let cases = [
            ("password: top-secret", "top-secret"),
            ("db_pwd: hidden-pwd", "hidden-pwd"),
            ("accessToken: hidden-token", "hidden-token"),
            ("client-secret: hidden-secret", "hidden-secret"),
        ];
        for (extra, secret) in cases {
            let workspace = TestWorkspace::new(
                "shadow-secret-key",
                &format!("{}\ncredentials:\n  {extra}\n", safe_primary_prefix()),
            );
            workspace.seed_main_configuration();
            let error = ShadowDumpPreparation::prepare(
                &workspace.context,
                "main",
                &["Catalog:Items".to_string()],
                &partial_caller(),
            )
            .unwrap_err();
            assert!(error.contains("sensitive key"));
            assert!(!error.contains(secret));
        }
    }

    #[test]
    fn rejects_password_bearing_connection_but_accepts_safe_file_connection() {
        for connection in [
            "Srvr=cluster;Ref=demo;Pwd=very-secret",
            "Srvr=cluster;Ref=demo;Password = very-secret",
            "http://user:very-secret@example.invalid/ib",
            "/S cluster\\demo /P very-secret",
        ] {
            let workspace = TestWorkspace::with_connection("shadow-secret-connection", connection);
            workspace.seed_main_configuration();
            let error = ShadowDumpPreparation::prepare(
                &workspace.context,
                "main",
                &["Catalog:Items".to_string()],
                &partial_caller(),
            )
            .unwrap_err();
            assert!(error.contains("password-bearing connection"), "{error}");
            assert!(!error.contains("very-secret"));
        }

        let workspace =
            TestWorkspace::with_connection("shadow-safe-connection", "/F \"build/my infobase\"");
        workspace.seed_main_configuration();
        ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap();
    }

    #[test]
    fn rejects_wrong_scope_ambiguous_source_sets_and_custom_config() {
        let edt = TestWorkspace::new(
            "shadow-edt",
            &safe_config_with_source("main", "CONFIGURATION", "src", "EDT"),
        );
        fs::create_dir_all(edt.root.join("src/Configuration")).unwrap();
        fs::write(edt.root.join("src/.project"), b"<project/>").unwrap();
        fs::write(
            edt.root.join("src/Configuration/Configuration.mdo"),
            b"<Configuration/>",
        )
        .unwrap();
        let error = ShadowDumpPreparation::prepare(
            &edt.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap_err();
        assert!(error.contains("uses EDT"));

        let extension = TestWorkspace::new(
            "shadow-extension",
            &safe_config_with_source("ext", "EXTENSION", "ext", "DESIGNER"),
        );
        fs::create_dir_all(extension.root.join("ext")).unwrap();
        fs::write(
            extension.root.join("ext/Configuration.xml"),
            b"<Configuration/>",
        )
        .unwrap();
        let error = ShadowDumpPreparation::prepare(
            &extension.context,
            "ext",
            &["Catalog:Items".to_string()],
            &json_map(serde_json::json!({
                "operation": "dump",
                "mode": "partial",
                "extension": "ext",
            })),
        )
        .unwrap_err();
        assert!(error.contains("does not support extensions"));

        let ibcmd = TestWorkspace::new(
            "shadow-ibcmd",
            &safe_primary().replace("builder: DESIGNER", "builder: IBCMD"),
        );
        ibcmd.seed_platform_roots();
        let error = ShadowDumpPreparation::prepare(
            &ibcmd.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap_err();
        assert!(error.contains("requires the DESIGNER builder"));

        let malformed_builder = TestWorkspace::new(
            "shadow-malformed-builder",
            &safe_primary().replace("builder: DESIGNER", "builder: 42"),
        );
        malformed_builder.seed_platform_roots();
        let error = ShadowDumpPreparation::prepare(
            &malformed_builder.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap_err();
        assert!(error.contains("`builder` must be a string"));

        let duplicate = TestWorkspace::new(
            "shadow-duplicate",
            &format!(
                "{}\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: main\n    type: CONFIGURATION\n    path: other\n",
                safe_primary_prefix_without_sources()
            ),
        );
        fs::create_dir_all(duplicate.root.join("src")).unwrap();
        fs::create_dir_all(duplicate.root.join("other")).unwrap();
        fs::write(
            duplicate.root.join("src/Configuration.xml"),
            b"<Configuration/>",
        )
        .unwrap();
        fs::write(
            duplicate.root.join("other/Configuration.xml"),
            b"<Configuration/>",
        )
        .unwrap();
        let error = ShadowDumpPreparation::prepare(
            &duplicate.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap_err();
        assert!(error.contains("ambiguous"));

        let custom = TestWorkspace::platform_sequence("shadow-custom-config");
        let custom_path = custom.root.join("custom.yaml");
        fs::write(&custom_path, safe_primary()).unwrap();
        let caller = json_map(serde_json::json!({
            "operation": "dump",
            "mode": "partial",
            "config": custom_path,
            "object": "Catalog:Items",
        }));
        let error = ShadowDumpPreparation::prepare(
            &custom.context,
            "main",
            &["Catalog:Items".to_string()],
            &caller,
        )
        .unwrap_err();
        assert!(error.contains("only the default"));
    }

    #[test]
    fn reported_args_redact_nested_caller_secrets() {
        let workspace = TestWorkspace::platform_sequence("shadow-redaction");
        let caller = json_map(serde_json::json!({
            "operation": "dump",
            "mode": "partial",
            "object": "Catalog:Items",
            "dbPassword": "caller-secret",
            "nested": {"accessToken": "nested-secret"},
            "connection": "Srvr=cluster;Pwd=connection-secret",
            "raw": ["--password=raw-secret"],
        }));
        let prepared = ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &["Catalog:Items".to_string()],
            &caller,
        )
        .unwrap();
        let report = serde_json::to_string(&prepared.reported_args()).unwrap();
        for secret in [
            "caller-secret",
            "nested-secret",
            "connection-secret",
            "raw-secret",
        ] {
            assert!(!report.contains(secret));
        }
        assert!(report.contains("[REDACTED]"));
    }

    #[test]
    fn rejects_mismatched_or_malformed_runtime_intent_before_creating_artifacts() {
        let workspace = TestWorkspace::platform_sequence("shadow-runtime-intent");
        let cases = [
            (
                json_map(serde_json::json!({
                    "operation": "dump",
                    "mode": "partial",
                    "object": "Catalog:Other",
                })),
                "do not match caller",
            ),
            (
                json_map(serde_json::json!({
                    "operation": "dump",
                    "mode": "partial",
                    "object": "Catalog:Items",
                    "force": "false",
                })),
                "force` must be boolean",
            ),
            (
                json_map(serde_json::json!({
                    "operation": "dump",
                    "mode": "full",
                    "object": "Catalog:Items",
                })),
                "mode `partial`",
            ),
            (
                json_map(serde_json::json!({
                    "operation": "dump",
                    "mode": "partial",
                    "object": "Catalog:Items",
                    "sourceSet": "other",
                })),
                "does not match selected",
            ),
        ];

        for (caller, expected) in cases {
            let error = ShadowDumpPreparation::prepare(
                &workspace.context,
                "main",
                &["Catalog:Items".to_string()],
                &caller,
            )
            .unwrap_err();
            assert!(error.contains(expected), "unexpected error: {error}");
        }
        assert!(recover_stale_shadow_dumps(&workspace.context)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn recovery_removes_only_valid_owned_transactions() {
        let workspace = TestWorkspace::platform_sequence("shadow-recovery");
        let root = transaction_root(&workspace.context).unwrap();
        fs::create_dir_all(&root).unwrap();
        let transaction_id = Uuid::new_v4();
        let transaction = root.join(format!("{TRANSACTION_PREFIX}{transaction_id}"));
        fs::create_dir_all(&transaction).unwrap();
        write_transaction_ownership(&transaction, transaction_id).unwrap();
        fs::write(transaction.join(PRIMARY_CONFIG_NAME), b"orphan").unwrap();
        let invalid_transaction = root.join(format!("{TRANSACTION_PREFIX}not-a-uuid"));
        fs::create_dir_all(&invalid_transaction).unwrap();
        let removed = recover_stale_shadow_dumps(&workspace.context).unwrap();
        assert_eq!(removed, vec![transaction.clone()]);
        assert!(!transaction.exists());
        assert!(invalid_transaction.exists());
        assert!(workspace.root.join(PRIMARY_CONFIG_NAME).exists());
    }

    #[test]
    fn failed_preparation_never_removes_a_workspace_file_it_did_not_create() {
        let workspace = TestWorkspace::platform_sequence("shadow-config-ownership");
        let root = transaction_root(&workspace.context).unwrap();
        fs::create_dir_all(&root).unwrap();
        let id = Uuid::new_v4();
        let transaction = root.join(format!("{TRANSACTION_PREFIX}{id}"));
        fs::create_dir(&transaction).unwrap();
        write_transaction_ownership(&transaction, id).unwrap();
        let preexisting_config = workspace.root.join("user-owned.yaml");
        fs::write(&preexisting_config, b"preexisting").unwrap();

        drop(PendingCleanup::new(root, transaction.clone()));

        assert!(!transaction.exists());
        assert_eq!(fs::read(&preexisting_config).unwrap(), b"preexisting");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_seed_is_rejected_and_partial_artifacts_are_cleaned() {
        use std::os::unix::fs::symlink;

        let workspace = TestWorkspace::platform_sequence("shadow-symlink-seed");
        let outside = workspace.root.join("outside.xml");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, workspace.root.join("src/ConfigDumpInfo.xml")).unwrap();
        let error = ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap_err();
        assert!(error.contains("regular non-symlink file"));
        assert!(recover_stale_shadow_dumps(&workspace.context)
            .unwrap()
            .is_empty());
        assert_eq!(fs::read(&outside).unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_authority_root_is_rejected_before_external_directories_are_created() {
        use std::os::unix::fs::symlink;

        let workspace = TestWorkspace::platform_sequence("shadow-symlink-authority");
        let outside_authority = workspace.root.join("outside-authority");
        fs::create_dir_all(&outside_authority).unwrap();
        fs::create_dir_all(workspace.root.join(".build")).unwrap();
        symlink(
            &outside_authority,
            workspace.root.join(".build").join("unica"),
        )
        .unwrap();

        let error = ShadowDumpPreparation::prepare(
            &workspace.context,
            "main",
            &["Catalog:Items".to_string()],
            &partial_caller(),
        )
        .unwrap_err();
        assert!(error.contains("non-symlink directory"));
        assert!(fs::read_dir(&outside_authority).unwrap().next().is_none());
    }

    fn partial_caller() -> Map<String, Value> {
        json_map(serde_json::json!({
            "operation": "dump",
            "mode": "partial",
            "object": "Catalog:Items",
        }))
    }

    fn json_map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn sequence_source_path(root: &YamlValue, selected: &str) -> String {
        root.as_mapping()
            .unwrap()
            .get(yaml_key("source-set"))
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .find(|entry| {
                entry
                    .as_mapping()
                    .and_then(|mapping| mapping.get(yaml_key("name")))
                    .and_then(YamlValue::as_str)
                    == Some(selected)
            })
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(yaml_key("path"))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    fn yaml_string_at(root: &YamlValue, path: &[&str]) -> String {
        path.iter()
            .fold(root, |value, key| {
                value.as_mapping().unwrap().get(yaml_key(key)).unwrap()
            })
            .as_str()
            .unwrap()
            .to_string()
    }

    struct TestWorkspace {
        root: PathBuf,
        cache: PathBuf,
        context: WorkspaceContext,
    }

    impl TestWorkspace {
        fn new(prefix: &str, config: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "{prefix}-{}-{}",
                std::process::id(),
                Uuid::new_v4()
            ));
            let cache = root.join("cache");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(PRIMARY_CONFIG_NAME), config).unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: cache.clone(),
                workspace_epoch: 1,
            };
            Self {
                root,
                cache,
                context,
            }
        }

        fn platform_sequence(prefix: &str) -> Self {
            let workspace = Self::new(prefix, &safe_primary());
            workspace.seed_platform_roots();
            workspace
        }

        fn with_connection(prefix: &str, connection: &str) -> Self {
            Self::new(
                prefix,
                &format!(
                    "format: DESIGNER\nbuilder: DESIGNER\nworkPath: build\ninfobase:\n  connection: {}\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                    serde_yaml::to_string(connection).unwrap().trim()
                ),
            )
        }

        fn seed_main_configuration(&self) {
            fs::create_dir_all(self.root.join("src")).unwrap();
            fs::write(self.root.join("src/Configuration.xml"), b"<Configuration/>").unwrap();
        }

        fn seed_platform_roots(&self) {
            self.seed_main_configuration();
            fs::create_dir_all(self.root.join("secondary")).unwrap();
            fs::write(
                self.root.join("secondary/Configuration.xml"),
                b"<Configuration/>",
            )
            .unwrap();
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn safe_primary() -> String {
        format!(
            "{}\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: secondary\n    type: CONFIGURATION\n    path: secondary\n",
            safe_primary_prefix_without_sources()
        )
    }

    fn safe_primary_prefix() -> String {
        safe_primary()
    }

    fn safe_primary_prefix_without_sources() -> &'static str {
        "format: DESIGNER\nbuilder: DESIGNER\nworkPath: build\ninfobase:\n  connection: File=build/ib"
    }

    fn safe_config_with_source(name: &str, kind: &str, path: &str, format: &str) -> String {
        format!(
            "format: {format}\nbuilder: DESIGNER\nworkPath: build\ninfobase:\n  connection: File=build/ib\nsource-set:\n  - name: {name}\n    type: {kind}\n    path: {path}\n"
        )
    }
}
