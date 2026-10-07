//! The pinned 0.13 executable stays behind the runner 1.0 operation contract.
//!
//! The workspace keeps the Unica vocabulary of `v8project.yaml` and
//! `v8project.local.yaml`; the runner reads its own. When the two differ, a
//! private copy of both layers is projected into the stable directory
//! `<workspace>/.build/unica/runner-project/` and the runner reads that copy.
//! The originals are never written.
//!
//! The directory is stable on purpose: runner 0.13 records the directory of the
//! config it read as the working copy that holds a file infobase, and checks
//! later whether that directory still declares the infobase. A copy removed after
//! each command would leave a gone owner that any other working copy silently
//! replaces. One directory per workspace root keeps the owner alive while the
//! working copy exists. Each projected command rewrites the copy atomically; a
//! command that needs no projection removes it, so the workspace itself becomes
//! the owner and its former projection is a gone owner of the same copy.
//!
//! The 0.13 loader resolves relative paths from the directory of the primary
//! config, and source-set paths from `basePath`, which it always sets to that
//! directory: the project schema of 0.13 has no `basePath` key. The private copy
//! lives elsewhere, so every such path is made absolute against the workspace
//! root before the copy is written.
use super::v13_workspace_bootstrap::read_yaml_config;
use crate::infrastructure::internal_adapters::{
    ProcessCommand, ProcessOutput, ProcessRunner, SystemProcessRunner,
};
use crate::infrastructure::platform::PendingProcessHandoff;
use crate::infrastructure::source_roots::normalize_path_identity;
use serde_yaml::{Mapping, Value};
use std::path::{Path, PathBuf};

pub(super) const VERSION: &str = "0.13.0";
pub(super) fn check_version(version: &str) -> Result<(), String> {
    if version == VERSION {
        Ok(())
    } else {
        Err(format!(
            "no verified Unica runner adapter for version {version}; expected {VERSION}"
        ))
    }
}

/// Marks an error of the workspace project configuration, as opposed to an
/// error starting the runner: the caller fixes its files, not its environment.
const CONFIG_REFUSAL_PREFIX: &str = "unica-runner-config-refusal: ";

/// The reason a project configuration was refused before the runner started,
/// or `None` when the error is not a configuration refusal.
pub(super) fn config_refusal(error: &str) -> Option<&str> {
    error.strip_prefix(CONFIG_REFUSAL_PREFIX)
}

fn refuse(reason: impl std::fmt::Display) -> String {
    format!("{CONFIG_REFUSAL_PREFIX}{reason}")
}

const BASE_NAME: &str = "v8project.yaml";
const LOCAL_NAME: &str = "v8project.local.yaml";
/// The only named infobase this adapter serves.
const ORIGIN: &str = "origin";
/// Provider keys renamed by runner 0.12 and kept by 0.13: previous name, canonical name.
const RENAMED_PROVIDERS: [(&str, &str); 5] = [
    ("build", "push"),
    ("dump", "pull"),
    ("load", "upload"),
    ("init", "infobase.create"),
    ("infobase.configuration.export", "download"),
];

pub(super) struct Runner013ProcessRunner;
impl ProcessRunner for Runner013ProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        run_projected(&SystemProcessRunner, command)
    }

    fn run_pending_handoff(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), String> {
        with_projected_config(command, |projected| {
            SystemProcessRunner.run_pending_handoff(projected)
        })
    }
}

fn run_projected(
    runner: &dyn ProcessRunner,
    command: &ProcessCommand,
) -> Result<ProcessOutput, String> {
    with_projected_config(command, |projected| runner.run(projected))
}

fn with_projected_config<T>(
    command: &ProcessCommand,
    execute: impl FnOnce(&ProcessCommand) -> Result<T, String>,
) -> Result<T, String> {
    let position = command
        .args
        .iter()
        .position(|v| v == "--config")
        .ok_or("runner adapter requires an explicit project config")?
        + 1;
    let path = Path::new(
        command
            .args
            .get(position)
            .ok_or("missing project config argument")?,
    );
    let root = path.parent().ok_or("project config has no parent")?;
    let base = read_yaml_config(root, BASE_NAME)
        .map_err(refuse)?
        .ok_or_else(|| refuse(format!("{BASE_NAME} is absent")))?;
    let local = read_yaml_config(root, LOCAL_NAME).map_err(refuse)?;
    // These refusals hold whether or not a private copy is needed.
    validate_layer(&base, BASE_NAME)?;
    if let Some(local) = &local {
        validate_layer(local, LOCAL_NAME)?;
    }
    if !needs_projection(&base, local.as_ref()) {
        // The workspace itself holds the infobase now: a former projection must
        // not stay alive as another working copy that still declares it.
        let root = normalize_path_identity(root).map_err(refuse)?;
        match std::fs::remove_dir_all(projection_directory(&root)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("cannot remove the former projected runner config".into()),
        }
        return execute(command);
    }
    let root = normalize_path_identity(root).map_err(refuse)?;
    // Project both layers before creating private files or starting any process.
    let (base, local) = project(base, local, &root)?;
    let directory = projection_directory(&root);
    create_private_directory(&directory)?;
    let config = directory.join(BASE_NAME);
    write_config(&config, &base)?;
    let local_path = directory.join(LOCAL_NAME);
    match local {
        Some(local) => write_config(&local_path, &local)?,
        None => match std::fs::remove_file(&local_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("cannot remove a stale projected local layer".into()),
        },
    }
    let mut projected = command.clone();
    projected.args[position] = config.display().to_string();
    // Keep cwd and every invocation argument: only the config is a compatibility projection.
    execute(&projected)
}

/// The stable projection directory of one workspace root, in the Unica cache:
/// `<workspace>/.build/unica/runner-project` by default. A cache moved out by
/// `UNICA_CACHE_DIR` is shared by workspaces, so there the directory is keyed by
/// the workspace root.
pub(super) fn projection_directory(root: &Path) -> PathBuf {
    projection_directory_in(root, std::env::var_os("UNICA_CACHE_DIR").map(PathBuf::from))
}

fn projection_directory_in(root: &Path, cache_override: Option<PathBuf>) -> PathBuf {
    match cache_override {
        None => root.join(".build").join("unica").join("runner-project"),
        Some(cache) => {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
            let key: String = digest[..8]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            cache.join("runner-project").join(key)
        }
    }
}

/// The local layer carries credentials: the directory is readable by its owner
/// only.
fn create_private_directory(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|_| "cannot create private runner config directory")?;
    crate::infrastructure::platform::restrict_directory_to_owner(directory)
        .map_err(|_| "cannot restrict private runner config directory".into())
}

/// Writes one projected layer atomically: a concurrent command of the same
/// working copy reads either the previous or the new copy, never a half.
fn write_config(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let bytes =
        serde_yaml::to_string(value).map_err(|_| "cannot serialize projected runner config")?;
    let directory = path.parent().ok_or("projected config has no directory")?;
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .map_err(|_| "cannot write private projected runner config")?;
    file.write_all(bytes.as_bytes())
        .map_err(|_| "cannot write private projected runner config")?;
    file.persist(path)
        .map_err(|_| "cannot write private projected runner config")?;
    Ok(())
}

fn mapping<'a>(value: &'a Value, file: &str) -> Result<&'a Mapping, String> {
    value
        .as_mapping()
        .ok_or_else(|| refuse(format!("{file} must be a mapping")))
}

/// Refusals that do not depend on whether the layer is projected.
fn validate_layer(value: &Value, file: &str) -> Result<(), String> {
    let map = mapping(value, file)?;
    if map.contains_key(Value::from("basePath")) {
        // 0.13 rejects the key and always resolves source sets from the
        // directory of v8project.yaml.
        return Err(refuse(format!(
            "basePath is not supported by v8-runner 0.13: source-set paths are resolved from the directory of {BASE_NAME}; remove basePath from {file}"
        )));
    }
    for key in ["execution_timeout", "execution_timeout_seconds"] {
        if map.contains_key(Value::from(key)) {
            return Err(refuse(format!(
                "{key} is not supported by v8-runner 0.13: a command has no overall deadline, only individual steps do; remove {key} from {file}"
            )));
        }
    }
    // 0.13 has no partial-load threshold and refuses the key by name; the
    // legacy `build` section is the same setting under its previous name.
    for section in ["push", "build"] {
        if map
            .get(Value::from(section))
            .and_then(Value::as_mapping)
            .is_some_and(|settings| settings.contains_key(Value::from("partialLoadThreshold")))
        {
            return Err(refuse(format!(
                "{section}.partialLoadThreshold is not supported by v8-runner 0.13: the runner has no partial-load threshold, and a full load is requested with push full:true; remove the key from {file}"
            )));
        }
    }
    if map.contains_key(Value::from("infobase")) && map.contains_key(Value::from("infobases")) {
        return Err(refuse(format!(
            "{file} declares both infobase and infobases; keep infobases.{ORIGIN} only"
        )));
    }
    if let Some(bases) = map.get(Value::from("infobases")) {
        let bases = bases
            .as_mapping()
            .ok_or_else(|| refuse(format!("infobases in {file} must be a mapping")))?;
        if bases.keys().any(|name| name.as_str() != Some(ORIGIN)) {
            return Err(refuse(format!(
                "{file} declares an infobase other than {ORIGIN}; the runner 0.13 adapter serves only infobases.{ORIGIN}"
            )));
        }
    }
    if let Some(providers) = map.get(Value::from("providers")) {
        let providers = providers
            .as_mapping()
            .ok_or_else(|| refuse(format!("providers in {file} must be a mapping")))?;
        if let Some(command) = providers.keys().find_map(|k| {
            k.as_str()
                .filter(|k| matches!(*k, "apply" | "reset" | "diff"))
        }) {
            return Err(refuse(format!(
                "providers.{command} in {file} cannot be represented by runner 0.13"
            )));
        }
    }
    Ok(())
}

/// Without projection 0.13 reads the workspace files itself, so the fast path
/// is taken only when it would read them exactly as the projection would.
fn needs_projection(base: &Value, local: Option<&Value>) -> bool {
    let layers = || std::iter::once(base).chain(local);
    base.get("infobase").is_some()
        || base.get("infobases").is_some()
        // 0.13 requires workPath; the projection supplies the default.
        || !layers().any(|layer| layer.get("workPath").is_some())
        || layers().any(|layer| {
            layer.get("infobase").is_some()
                || layer.get("build").is_some()
                || layer
                    .get("providers")
                    .and_then(Value::as_mapping)
                    .is_some_and(|providers| {
                        providers.keys().any(|key| {
                            key.as_str().is_some_and(|key| {
                                key == "infobase"
                                    || RENAMED_PROVIDERS.iter().any(|(old, _)| *old == key)
                            })
                        })
                    })
                || layer
                    .get("source-set")
                    .and_then(Value::as_sequence)
                    .is_some_and(|sets| {
                        sets.iter().any(|set| {
                            matches!(
                                set["type"].as_str(),
                                Some("configuration" | "extension" | "external")
                            )
                        })
                    })
        })
}

/// Projects both layers. The infobase description of either layer, legacy
/// `infobase:` or `infobases.origin`, is merged field by field (local wins, as
/// the runner merges layers) and written only into the private local layer:
/// 0.13 refuses `infobases` in the project file.
fn project(
    mut base: Value,
    mut local: Option<Value>,
    root: &Path,
) -> Result<(Value, Option<Value>), String> {
    let base_origin = take_origin(&mut base, BASE_NAME)?;
    let local_origin = match local.as_mut() {
        Some(local) => take_origin(local, LOCAL_NAME)?,
        None => None,
    };
    let origin = match (base_origin, local_origin) {
        (Some(mut base), Some(local)) => {
            merge_yaml_values(&mut base, local);
            Some(base)
        }
        (base, local) => base.or(local),
    };
    let base = project_layer(base, root, BASE_NAME)?;
    let mut local = local
        .map(|local| project_layer(local, root, LOCAL_NAME))
        .transpose()?;
    if let Some(mut origin) = origin {
        absolutize_infobase(&mut origin, root)?;
        let mut bases = Mapping::new();
        bases.insert(Value::from(ORIGIN), origin);
        local
            .get_or_insert_with(|| Value::Mapping(Mapping::new()))
            .as_mapping_mut()
            .expect("projected local layer is a mapping")
            .insert(Value::from("infobases"), Value::Mapping(bases));
    }
    Ok((base, local))
}

/// Removes the infobase description from a layer and returns its `origin`.
fn take_origin(layer: &mut Value, file: &str) -> Result<Option<Value>, String> {
    let map = layer
        .as_mapping_mut()
        .ok_or_else(|| refuse(format!("{file} must be a mapping")))?;
    let legacy = map.remove(Value::from("infobase"));
    let target = map
        .remove(Value::from("infobases"))
        .and_then(|bases| bases.as_mapping()?.get(Value::from(ORIGIN)).cloned());
    match legacy.or(target) {
        Some(section) if section.is_mapping() => Ok(Some(section)),
        Some(_) => Err(refuse(format!(
            "the {ORIGIN} infobase in {file} must be a mapping"
        ))),
        None => Ok(None),
    }
}

/// The runner's own layer merge: mappings merge key by key, anything else is
/// replaced by the overlay.
fn merge_yaml_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Mapping(base), Value::Mapping(overlay)) => {
            for (key, overlay_value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => merge_yaml_values(base_value, overlay_value),
                    None => {
                        base.insert(key, overlay_value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn project_layer(mut value: Value, root: &Path, file: &str) -> Result<Value, String> {
    let local = file == LOCAL_NAME;
    let map = value
        .as_mapping_mut()
        .ok_or_else(|| refuse(format!("{file} must be a mapping")))?;
    if let Some(providers) = map.get_mut(Value::from("providers")) {
        let providers = providers
            .as_mapping_mut()
            .ok_or_else(|| refuse(format!("providers in {file} must be a mapping")))?;
        if let Some(nested) = providers.remove(Value::from("infobase")) {
            for (k, v) in nested
                .as_mapping()
                .ok_or_else(|| refuse(format!("providers.infobase in {file} must be a mapping")))?
            {
                let key = k
                    .as_str()
                    .ok_or_else(|| refuse(format!("provider command in {file} must be text")))?;
                if !["create", "dump", "restore"].contains(&key) {
                    return Err(refuse(format!(
                        "unknown providers.infobase.{key} in {file}"
                    )));
                }
                if providers
                    .insert(Value::from(format!("infobase.{key}")), v.clone())
                    .is_some()
                {
                    return Err(refuse(format!(
                        "providers.infobase.{key} is declared twice in {file}"
                    )));
                }
            }
        }
        for (old, new) in RENAMED_PROVIDERS {
            if let Some(v) = providers.remove(Value::from(old)) {
                if providers.insert(Value::from(new), v).is_some() {
                    return Err(refuse(format!(
                        "providers.{old} and providers.{new} name one command in {file}; keep providers.{new}"
                    )));
                }
            }
        }
    }
    if let Some(build) = map.remove(Value::from("build")) {
        if map.insert(Value::from("push"), build).is_some() {
            return Err(refuse(format!(
                "build and push settings are both declared in {file}; keep push"
            )));
        }
    }
    if !local {
        map.entry(Value::from("workPath"))
            .or_insert(Value::from(root.join("build").display().to_string()));
    }
    if let Some(sets) = map
        .get_mut(Value::from("source-set"))
        .and_then(Value::as_sequence_mut)
    {
        for set in sets {
            absolutize(set, &["path"], root)?;
            match set["type"].as_str() {
                Some("configuration") => set["type"] = Value::from("CONFIGURATION"),
                Some("extension") => set["type"] = Value::from("EXTENSION"),
                Some("external") => {
                    return Err(refuse(
                        "mixed external source sets need the runner 1.0 adapter",
                    ))
                }
                _ => {}
            }
        }
    }
    // These are exactly the config-directory-relative paths normalized by the
    // 0.13 loader outside the infobase sections (`normalize_config_paths`).
    for parts in [
        &["workPath"][..],
        &["tools", "platform", "path"],
        &["tools", "va", "epf_path"],
        // Секция агента в схеме раннера названа через дефис (`kebab-case`).
        &["tools", "designer_agent", "host-key"],
        &["tools", "designer_agent", "base-dir"],
        &["tools", "client_mcp", "extension", "source", "path"],
        &["tools", "client_mcp", "extension", "artifact", "path"],
        &["tests", "va", "params_path"],
    ] {
        absolutize(&mut value, parts, root)?;
    }
    // A bare name without a directory is an EDT discovery hint, not a path.
    if let Some(edt) = value
        .get_mut("tools")
        .and_then(|v| v.get_mut("edt_cli"))
        .and_then(|v| v.get_mut("path"))
    {
        if edt
            .as_str()
            .is_some_and(|text| Path::new(text).components().count() > 1)
        {
            let text = edt.as_str().expect("checked text");
            *edt = Value::from(absolute(root, text).display().to_string());
        }
    }
    if let Some(profiles) = value
        .get_mut("tests")
        .and_then(|v| v.get_mut("va"))
        .and_then(|v| v.get_mut("profiles"))
        .and_then(Value::as_mapping_mut)
    {
        for profile in profiles.values_mut() {
            absolutize(profile, &["feature_path"], root)?;
        }
    }
    Ok(value)
}

/// The paths of one infobase section the 0.13 loader resolves from the config
/// directory (`normalize_infobase_paths`).
fn absolutize_infobase(infobase: &mut Value, root: &Path) -> Result<(), String> {
    for parts in [
        &["standalone", "exchange", "dir"][..],
        &["web", "dir"],
        &["web", "conf"],
    ] {
        absolutize(infobase, parts, root)?;
    }
    if let Some(connection) = infobase.get_mut("connection") {
        let text = connection
            .as_str()
            .ok_or_else(|| refuse("infobase connection must be text"))?;
        if text.trim_start().starts_with(['/', '-']) {
            return Err(refuse("raw connection argv cannot be projected; use a structured File= or Srvr= connection"));
        }
        let mut parts = Vec::new();
        for part in text.split(';') {
            let trimmed = part.trim();
            if trimmed
                .get(..5)
                .is_some_and(|p| p.eq_ignore_ascii_case("file="))
            {
                let path = trimmed[5..].trim();
                let path = if path.starts_with('"') && path.ends_with('"') && path.len() > 1 {
                    &path[1..path.len() - 1]
                } else {
                    path
                };
                if path.contains(['"', '\'', '\n', '\r']) || path.is_empty() {
                    return Err(refuse("File= path cannot be projected unambiguously"));
                }
                let path = absolute(root, path);
                if path.to_string_lossy().contains(['"', ';']) {
                    return Err(refuse(
                        "workspace path cannot be represented in File= connection",
                    ));
                }
                parts.push(format!("File=\"{}\"", path.display()));
            } else {
                parts.push(part.to_owned());
            }
        }
        *connection = Value::from(parts.join(";"));
    }
    Ok(())
}

fn absolute(root: &Path, text: &str) -> PathBuf {
    let path = Path::new(text);
    if path.is_absolute() {
        path.into()
    } else {
        root.join(path)
    }
}

fn absolutize(value: &mut Value, parts: &[&str], root: &Path) -> Result<(), String> {
    let mut current = value;
    for part in parts {
        match current.get_mut(*part) {
            Some(v) => current = v,
            None => return Ok(()),
        }
    }
    if current.is_null() {
        return Ok(());
    }
    let text = current
        .as_str()
        .ok_or_else(|| refuse(format!("{} must be a path", parts.join("."))))?;
    *current = Value::from(absolute(root, text).display().to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Reads the projected layers the runner would see, then fails like a
    /// provider would, so cleanup on failure is observed as well.
    struct Probe {
        config: Mutex<Option<PathBuf>>,
        layers: Mutex<Option<(Value, Option<Value>)>>,
    }

    impl Probe {
        fn new() -> Self {
            Self {
                config: Mutex::new(None),
                layers: Mutex::new(None),
            }
        }
    }

    impl ProcessRunner for Probe {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let path = PathBuf::from(&command.args[1]);
            let base: Value = serde_yaml::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let local = std::fs::read(path.parent().unwrap().join(LOCAL_NAME))
                .ok()
                .map(|bytes| serde_yaml::from_slice(&bytes).unwrap());
            *self.layers.lock().unwrap() = Some((base, local));
            *self.config.lock().unwrap() = Some(path);
            Err("simulated provider failure".into())
        }
    }

    fn command(root: &Path) -> ProcessCommand {
        ProcessCommand {
            program: root.join("runner"),
            args: vec![
                "--config".into(),
                root.join(BASE_NAME).display().to_string(),
            ],
            cwd: root.into(),
            env: vec![],
            env_remove: vec![],
            capture_limits: None,
            timeout: None,
            cancellation: crate::domain::cancellation::CancellationToken::new(),
        }
    }

    fn workspace(base: &str, local: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(BASE_NAME), base).unwrap();
        if let Some(local) = local {
            std::fs::write(root.path().join(LOCAL_NAME), local).unwrap();
        }
        root
    }

    fn identity(root: &Path) -> PathBuf {
        normalize_path_identity(root).unwrap()
    }

    /// Runs the projection and returns what the runner read; the private copy
    /// stays in the stable projection directory and the workspace files are
    /// untouched.
    fn projected(base: &str, local: Option<&str>) -> (PathBuf, Value, Option<Value>) {
        let root = workspace(base, local);
        let probe = Probe::new();
        assert_eq!(
            run_projected(&probe, &command(root.path())).unwrap_err(),
            "simulated provider failure"
        );
        let config = probe.config.lock().unwrap().clone().unwrap();
        assert_eq!(
            config,
            projection_directory(&identity(root.path())).join(BASE_NAME),
            "projection expected"
        );
        assert!(config.is_file(), "the projected copy is kept");
        assert_eq!(
            std::fs::read_to_string(root.path().join(BASE_NAME)).unwrap(),
            base
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join(LOCAL_NAME)).ok(),
            local.map(str::to_owned)
        );
        let (base, local) = probe.layers.lock().unwrap().take().unwrap();
        (identity(root.path()), base, local)
    }

    fn refused(base: &str, local: Option<&str>) -> String {
        let root = workspace(base, local);
        let probe = Probe::new();
        let error = run_projected(&probe, &command(root.path())).unwrap_err();
        assert!(
            probe.config.lock().unwrap().is_none(),
            "runner started: {error}"
        );
        config_refusal(&error)
            .unwrap_or_else(|| panic!("not a configuration refusal: {error}"))
            .to_owned()
    }

    fn text(path: PathBuf) -> String {
        path.display().to_string()
    }

    /// Раннер 0.13 записывает владельцем файловой базы каталог прочитанного
    /// конфига. Каталог проекции поэтому постоянный и один на рабочую копию:
    /// повтор пишет туда же, лишний местный слой убирается, а команда без
    /// проекции убирает сам каталог, и владельцем становится рабочая копия.
    #[test]
    fn the_projection_directory_is_stable_per_working_copy() {
        let root = workspace(
            "infobase: {connection: 'File=ib'}\n",
            Some("infobases: {origin: {shared: true}}\n"),
        );
        let directory = projection_directory(&identity(root.path()));
        for _ in 0..2 {
            let probe = Probe::new();
            run_projected(&probe, &command(root.path())).unwrap_err();
            assert_eq!(
                probe.config.lock().unwrap().clone().unwrap(),
                directory.join(BASE_NAME)
            );
        }
        let local: Value =
            serde_yaml::from_slice(&std::fs::read(directory.join(LOCAL_NAME)).unwrap()).unwrap();
        assert_eq!(local["infobases"]["origin"]["shared"], true);
        assert!(crate::infrastructure::platform::directory_is_owner_only(&directory).unwrap());

        // Без местного слоя прежняя копия местного слоя не остаётся.
        std::fs::remove_file(root.path().join(LOCAL_NAME)).unwrap();
        std::fs::write(
            root.path().join(BASE_NAME),
            "workPath: build\nsource-set: [{name: main, type: configuration, path: src}]\n",
        )
        .unwrap();
        run_projected(&Probe::new(), &command(root.path())).unwrap_err();
        assert!(directory.join(BASE_NAME).is_file());
        assert!(!directory.join(LOCAL_NAME).exists());

        // Конфиг, который раннер читает сам, убирает каталог проекции.
        std::fs::write(
            root.path().join(BASE_NAME),
            "workPath: build\nformat: DESIGNER\n",
        )
        .unwrap();
        let probe = Probe::new();
        run_projected(&probe, &command(root.path())).unwrap_err();
        assert_eq!(
            probe.config.lock().unwrap().clone().unwrap(),
            root.path().join(BASE_NAME)
        );
        assert!(!directory.exists());
    }

    /// Кеш, вынесенный `UNICA_CACHE_DIR`, общий для рабочих копий: каталог
    /// проекции в нём ключуется корнем рабочей копии и в корень не пишется.
    #[test]
    fn a_moved_cache_keys_the_projection_by_the_working_copy() {
        let cache = PathBuf::from("/cache");
        let a = projection_directory_in(Path::new("/work/a"), Some(cache.clone()));
        let b = projection_directory_in(Path::new("/work/b"), Some(cache.clone()));
        assert_ne!(a, b);
        assert!(
            a.starts_with(cache.join("runner-project")),
            "{}",
            a.display()
        );
        assert_eq!(
            a,
            projection_directory_in(Path::new("/work/a"), Some(cache))
        );
        assert_eq!(
            projection_directory_in(Path::new("/work/a"), None),
            Path::new("/work/a/.build/unica/runner-project")
        );
    }

    #[test]
    fn projection_preserves_overlay_paths_and_writes_private_files() {
        let (root, base, local) = projected(
            "format: DESIGNER\ninfobases: {origin: {connection: 'File=base'}}\nproviders: {download: designer}\n",
            Some("infobases: {origin: {password: private-test-secret}}\nworkPath: work-local\ntools: {platform: {path: platform}, designer_agent: {host-key: keys/agent, base-dir: agent-base}}\nproviders: {infobase.configuration.export: agent}\n"),
        );
        let local = local.unwrap();
        assert!(base.get("infobases").is_none() && base.get("infobase").is_none());
        let origin = &local["infobases"]["origin"];
        assert_eq!(origin["password"], "private-test-secret");
        assert_eq!(
            origin["connection"].as_str().unwrap(),
            format!("File=\"{}\"", root.join("base").display())
        );
        assert_eq!(
            local["tools"]["platform"]["path"],
            text(root.join("platform"))
        );
        assert_eq!(
            local["tools"]["designer_agent"]["host-key"],
            text(root.join("keys/agent"))
        );
        assert_eq!(
            local["tools"]["designer_agent"]["base-dir"],
            text(root.join("agent-base"))
        );
        assert_eq!(local["workPath"], text(root.join("work-local")));
        assert_eq!(local["providers"]["download"], "agent");
        assert!(local["providers"]
            .get("infobase.configuration.export")
            .is_none());
        assert!(base.get("basePath").is_none());
    }

    #[test]
    fn legacy_infobase_in_the_project_file_merges_with_the_local_infobases() {
        let (root, base, local) = projected(
            "format: DESIGNER\nworkPath: build\ninfobase: {connection: 'File=ib', user: Admin, web: {dir: pub}}\n",
            Some("infobases: {origin: {user: Tester, password: private-test-secret}}\n"),
        );
        assert!(base.get("infobase").is_none());
        let origin = &local.unwrap()["infobases"]["origin"];
        assert_eq!(origin["user"], "Tester", "the local layer wins");
        assert_eq!(origin["password"], "private-test-secret");
        assert_eq!(origin["web"]["dir"], text(root.join("pub")));
        assert_eq!(
            origin["connection"].as_str().unwrap(),
            format!("File=\"{}\"", root.join("ib").display())
        );
    }

    #[test]
    fn infobase_without_a_local_layer_moves_into_a_private_local_layer() {
        let (root, base, local) = projected("infobase: {connection: 'File=ib'}\n", None);
        assert!(base.get("infobase").is_none());
        assert_eq!(base["workPath"], text(root.join("build")));
        assert_eq!(
            local.unwrap()["infobases"]["origin"]["connection"]
                .as_str()
                .unwrap(),
            format!("File=\"{}\"", root.join("ib").display())
        );
    }

    #[test]
    fn target_config_is_projected_without_losing_origin_or_provider_meaning() {
        // Абсолютный путь на любой ОС: `/abs/sales` под Windows относителен.
        let sales = std::env::temp_dir().join("unica-abs-sales");
        let sales = sales.display();
        let config = format!(
            "infobases:\n  origin:\n    connection: 'File=base'\nbuild: {{}}\nproviders:\n  build: designer\n  dump: designer\n  load: designer\n  init: designer\n  infobase: {{dump: ibcmd, restore: ibcmd}}\nsource-set:\n  - name: main\n    type: configuration\n    path: src\n  - name: sales\n    type: extension\n    path: '{sales}'\ntools: {{edt_cli: {{path: 1cedtcli}}, va: {{epf_path: va/va.epf}}}}\ntests: {{va: {{params_path: va/params.json, profiles: {{smoke: {{feature_path: features}}}}}}}}\n"
        );
        let (root, base, local) = projected(&config, None);
        assert!(base.get("build").is_none());
        assert!(base["push"].as_mapping().is_some_and(Mapping::is_empty));
        let providers = &base["providers"];
        for key in ["push", "pull", "upload", "infobase.create"] {
            assert_eq!(providers[key], "designer", "{key}");
        }
        assert_eq!(providers["infobase.dump"], "ibcmd");
        assert_eq!(providers["infobase.restore"], "ibcmd");
        for key in ["build", "dump", "load", "init", "infobase"] {
            assert!(providers.get(key).is_none(), "{key}");
        }
        assert_eq!(base["source-set"][0]["type"], "CONFIGURATION");
        assert_eq!(base["source-set"][1]["type"], "EXTENSION");
        assert_eq!(base["source-set"][0]["path"], text(root.join("src")));
        assert_eq!(base["source-set"][1]["path"], sales.to_string());
        assert!(
            base.get("basePath").is_none(),
            "0.13 closed schema rejects basePath"
        );
        assert_eq!(base["tools"]["edt_cli"]["path"], "1cedtcli");
        assert_eq!(
            base["tools"]["va"]["epf_path"],
            text(root.join("va/va.epf"))
        );
        assert_eq!(
            base["tests"]["va"]["params_path"],
            text(root.join("va/params.json"))
        );
        assert_eq!(
            base["tests"]["va"]["profiles"]["smoke"]["feature_path"],
            text(root.join("features"))
        );
        assert_eq!(
            local.unwrap()["infobases"]["origin"]["connection"]
                .as_str()
                .unwrap(),
            format!("File=\"{}\"", root.join("base").display())
        );
    }

    #[test]
    fn an_edt_cli_location_is_kept_relative_to_the_workspace() {
        let (root, base, _) = projected(
            "infobase: {connection: 'File=ib'}\ntools: {edt_cli: {path: edt/1cedtcli}}\n",
            None,
        );
        assert_eq!(
            base["tools"]["edt_cli"]["path"],
            text(root.join("edt/1cedtcli"))
        );
    }

    #[test]
    fn a_config_the_runner_reads_itself_is_passed_through_unchanged() {
        let root = workspace(
            "format: DESIGNER\nworkPath: build\nproviders: {push: designer}\nsource-set:\n  - {name: main, type: CONFIGURATION, path: src}\n",
            Some("infobases: {origin: {connection: 'File=ib'}}\n"),
        );
        let probe = Probe::new();
        assert!(run_projected(&probe, &command(root.path())).is_err());
        assert_eq!(
            probe.config.lock().unwrap().as_deref(),
            Some(root.path().join(BASE_NAME).as_path())
        );
    }

    #[test]
    fn execution_timeout_is_refused_before_the_runner_starts_in_either_layer() {
        for (base, local, file) in [
            (
                "workPath: build\nexecution_timeout: 300000\n",
                None,
                BASE_NAME,
            ),
            (
                "infobase: {connection: 'File=ib'}\n",
                Some("execution_timeout: 300000\n"),
                LOCAL_NAME,
            ),
            ("execution_timeout_seconds: 300\n", None, BASE_NAME),
        ] {
            let reason = refused(base, local);
            assert!(
                reason.contains("not supported by v8-runner 0.13"),
                "{reason}"
            );
            assert!(reason.contains("no overall deadline"), "{reason}");
            assert!(reason.contains(&format!("from {file}")), "{reason}");
        }
    }

    /// Раннер 0.13 отклоняет порог частичной загрузки по имени. Unica
    /// отвечает до запуска раннера, в обоих слоях и в прежней секции `build`,
    /// и тогда, когда проекция не нужна и раннер читал бы файлы сам.
    #[test]
    fn partial_load_threshold_is_refused_before_the_runner_starts() {
        for (base, local, section, file) in [
            (
                "workPath: build\npush: {partialLoadThreshold: 20}\n",
                Some("infobases: {origin: {connection: 'File=ib'}}\n"),
                "push",
                BASE_NAME,
            ),
            (
                "build: {partialLoadThreshold: 20}\n",
                None,
                "build",
                BASE_NAME,
            ),
            (
                "workPath: build\n",
                Some("push: {partialLoadThreshold: 5}\n"),
                "push",
                LOCAL_NAME,
            ),
        ] {
            let reason = refused(base, local);
            assert!(
                reason.contains(&format!(
                    "{section}.partialLoadThreshold is not supported by v8-runner 0.13"
                )),
                "{reason}"
            );
            assert!(reason.contains("full:true"), "{reason}");
            assert!(reason.contains(&format!("from {file}")), "{reason}");
        }
    }

    #[test]
    fn unknown_runner_versions_are_not_probed_by_executing_an_operation() {
        assert!(check_version(VERSION).is_ok());
        for version in [
            "0.9.0",
            "0.11.3",
            "0.11.4",
            "0.12.0",
            "0.13.1",
            "1.0.0",
            "1.0.0-rc.1",
            "",
        ] {
            assert!(check_version(version).is_err());
        }
    }

    #[test]
    fn unrepresentable_configuration_is_rejected_not_discarded() {
        for (base, local) in [
            ("infobases: {test: {connection: 'File=test'}}", None),
            ("workPath: build", Some("infobases: {origin: {}, test: {}}")),
            ("infobase: {}\ninfobases: {origin: {}}", None),
            (
                "workPath: build",
                Some("infobase: {}\ninfobases: {origin: {}}"),
            ),
            ("infobase: 'File=ib'", None),
            ("providers: {reset: agent}", None),
            ("providers: {apply: designer}", None),
            ("providers: {build: designer, push: designer}", None),
            (
                "providers: {infobase.create: designer, infobase: {create: ibcmd}}",
                None,
            ),
            ("build: {}\npush: {}", None),
            ("infobase: {connection: '/F ib'}", None),
            ("source-set: [{name: x, type: external, path: x}]", None),
            ("workPath: build\nbasePath: project", None),
        ] {
            refused(base, local);
        }
    }

    #[test]
    fn mixing_spellings_across_layers_is_allowed() {
        let (_, base, local) = projected(
            "infobase: {connection: 'File=ib'}\nproviders: {build: designer}\n",
            Some("infobases: {origin: {user: Tester}}\nproviders: {push: ibcmd}\n"),
        );
        assert_eq!(base["providers"]["push"], "designer");
        let local = local.unwrap();
        assert_eq!(local["providers"]["push"], "ibcmd");
        assert_eq!(local["infobases"]["origin"]["user"], "Tester");
    }
}
