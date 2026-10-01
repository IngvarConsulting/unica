//! The pinned 0.11 executable stays behind the runner 1.0 operation contract.
//! Private projected configs preserve workspace paths and are removed on every exit.
use super::v13_workspace_bootstrap::read_yaml_config;
use crate::infrastructure::internal_adapters::{
    ProcessCommand, ProcessOutput, ProcessRunner, SystemProcessRunner,
};
use serde_yaml::Value;
use std::path::{Path, PathBuf};

pub(super) const VERSION: &str = "0.11.2";
pub(super) fn check_version(version: &str) -> Result<(), String> {
    if version == VERSION {
        Ok(())
    } else {
        Err(format!(
            "no verified Unica runner adapter for version {version}; expected {VERSION}"
        ))
    }
}

pub(super) struct Runner011ProcessRunner;
impl ProcessRunner for Runner011ProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        run_projected(&SystemProcessRunner, command)
    }
}

fn run_projected(
    runner: &dyn ProcessRunner,
    command: &ProcessCommand,
) -> Result<ProcessOutput, String> {
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
    let base = read_yaml_config(root, "v8project.yaml")?.ok_or("project config is absent")?;
    let local = read_yaml_config(root, "v8project.local.yaml")?;
    if !needs_projection(&base) && !local.as_ref().is_some_and(needs_projection) {
        return runner.run(command);
    }
    let has =
        |key: &str| base.get(key).is_some() || local.as_ref().is_some_and(|v| v.get(key).is_some());
    if has("infobase") && has("infobases") {
        return Err("cannot mix legacy infobase and target infobases across project layers".into());
    }
    // Validate both layers before creating private files or starting any process.
    let base = project_layer(base, root, false)?;
    let local = local.map(|v| project_layer(v, root, true)).transpose()?;
    let directory = tempfile::Builder::new()
        .prefix("unica-runner011-")
        .tempdir()
        .map_err(|_| "cannot create private runner config directory")?;
    let config = directory.path().join("v8project.yaml");
    write_config(&config, &base)?;
    if let Some(local) = local {
        write_config(&directory.path().join("v8project.local.yaml"), &local)?;
    }
    let mut projected = command.clone();
    projected.args[position] = config.display().to_string();
    // Keep cwd and every invocation argument: only the config is a compatibility projection.
    runner.run(&projected)
}
fn write_config(path: &Path, value: &Value) -> Result<(), String> {
    let bytes =
        serde_yaml::to_string(value).map_err(|_| "cannot serialize projected runner config")?;
    std::fs::write(path, bytes).map_err(|_| "cannot write private projected runner config".into())
}
fn needs_projection(value: &Value) -> bool {
    value.get("infobases").is_some()
        || value.get("push").is_some()
        || value
            .get("providers")
            .and_then(Value::as_mapping)
            .is_some_and(|m| {
                m.keys().any(|k| {
                    matches!(
                        k.as_str(),
                        Some(
                            "push"
                                | "pull"
                                | "upload"
                                | "download"
                                | "infobase"
                                | "infobase.create"
                                | "apply"
                                | "reset"
                        )
                    )
                })
            })
        || value
            .get("source-set")
            .and_then(Value::as_sequence)
            .is_some_and(|sets| {
                sets.iter().any(|v| {
                    matches!(
                        v["type"].as_str(),
                        Some("configuration" | "extension" | "external")
                    )
                })
            })
}

pub(super) fn project_layer(mut value: Value, root: &Path, local: bool) -> Result<Value, String> {
    let map = value
        .as_mapping_mut()
        .ok_or("project config must be a mapping")?;
    if let Some(infobases) = map.remove(Value::from("infobases")) {
        if map.contains_key(Value::from("infobase")) {
            return Err("cannot mix infobase and infobases".into());
        }
        let bases = infobases
            .as_mapping()
            .ok_or("infobases must be a mapping")?;
        if bases.len() != 1 || !bases.contains_key(Value::from("origin")) {
            return Err("runner 0.11 adapter supports only infobases.origin; multi-infobase isolation requires runner 1.0".into());
        }
        if !bases[Value::from("origin")].is_mapping() {
            return Err("infobases.origin must be a mapping".into());
        }
        map.insert(
            Value::from("infobase"),
            bases[Value::from("origin")].clone(),
        );
    }
    if let Some(providers) = map.get_mut(Value::from("providers")) {
        let providers = providers
            .as_mapping_mut()
            .ok_or("providers must be a mapping")?;
        if let Some(nested) = providers.remove(Value::from("infobase")) {
            for (k, v) in nested
                .as_mapping()
                .ok_or("providers.infobase must be a mapping")?
            {
                let key = k.as_str().ok_or("provider command must be text")?;
                if !["create", "dump", "restore"].contains(&key) {
                    return Err("unknown providers.infobase command".into());
                }
                if providers
                    .insert(Value::from(format!("infobase.{key}")), v.clone())
                    .is_some()
                {
                    return Err("duplicate provider command".into());
                }
            }
        }
        for (new, old) in [
            ("push", "build"),
            ("pull", "dump"),
            ("upload", "load"),
            ("download", "infobase.configuration.export"),
            ("infobase.create", "init"),
        ] {
            if let Some(v) = providers.remove(Value::from(new)) {
                if providers.insert(Value::from(old), v).is_some() {
                    return Err("cannot mix old and new provider names for one command".into());
                }
            }
        }
        if providers
            .keys()
            .any(|k| matches!(k.as_str(), Some("diff" | "apply" | "reset")))
        {
            return Err("provider command cannot be represented by runner 0.11".into());
        }
    }
    if let Some(push) = map.remove(Value::from("push")) {
        if map.insert(Value::from("build"), push).is_some() {
            return Err("cannot mix build and push settings".into());
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
                    return Err("mixed external source sets need the runner 1.0 adapter".into())
                }
                _ => {}
            }
        }
    }
    // These are exactly the config-directory-relative paths normalized by the 0.11 loader.
    for parts in [
        &["workPath"][..],
        &["tools", "platform", "path"],
        &["tools", "va", "epf_path"],
        &["infobase", "standalone", "exchange", "dir"],
        &["infobase", "web", "dir"],
        &["infobase", "web", "conf"],
        &["tools", "client_mcp", "extension", "source", "path"],
        &["tools", "client_mcp", "extension", "artifact", "path"],
        &["tests", "va", "params_path"],
    ] {
        absolutize(&mut value, parts, root)?;
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
    if let Some(connection) = value
        .get_mut("infobase")
        .and_then(|v| v.get_mut("connection"))
    {
        let text = connection
            .as_str()
            .ok_or("infobase connection must be text")?;
        if text.trim_start().starts_with(['/', '-']) {
            return Err("raw connection argv cannot be projected; use a structured File= or Srvr= connection".into());
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
                    return Err("File= path cannot be projected unambiguously".into());
                }
                let path = absolute(root, path);
                if path.to_string_lossy().contains(['"', ';']) {
                    return Err("workspace path cannot be represented in File= connection".into());
                }
                parts.push(format!("File=\"{}\"", path.display()));
            } else {
                parts.push(part.to_owned());
            }
        }
        *connection = Value::from(parts.join(";"));
    }
    Ok(value)
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
    let text = current.as_str().ok_or("projected path must be text")?;
    *current = Value::from(absolute(root, text).display().to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_overlay_paths_and_removes_private_files_on_failure() {
        use std::sync::Mutex;
        struct Probe {
            config: Mutex<Option<PathBuf>>,
        }
        impl ProcessRunner for Probe {
            fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
                let path = PathBuf::from(&command.args[1]);
                let base: Value = serde_yaml::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                let local: Value = serde_yaml::from_slice(
                    &std::fs::read(path.parent().unwrap().join("v8project.local.yaml")).unwrap(),
                )
                .unwrap();
                assert!(base["infobase"].get("password").is_none());
                assert_eq!(local["infobase"]["password"], "private-test-secret");
                assert_eq!(
                    local["tools"]["platform"]["path"].as_str().unwrap(),
                    command.cwd.join("platform").to_str().unwrap()
                );
                assert_eq!(
                    local["workPath"].as_str().unwrap(),
                    command.cwd.join("work-local").to_str().unwrap()
                );
                assert_eq!(local["providers"]["infobase.configuration.export"], "agent");
                *self.config.lock().unwrap() = Some(path);
                Err("simulated provider failure".into())
            }
        }
        let root = tempfile::tempdir().unwrap();
        let base="format: DESIGNER\ninfobases: {origin: {connection: 'File=base'}}\nproviders: {download: designer}\n";
        let local="infobases: {origin: {password: private-test-secret}}\nworkPath: work-local\ntools: {platform: {path: platform}}\nproviders: {download: agent}\n";
        std::fs::write(root.path().join("v8project.yaml"), base).unwrap();
        std::fs::write(root.path().join("v8project.local.yaml"), local).unwrap();
        let probe = Probe {
            config: Mutex::new(None),
        };
        let command = ProcessCommand {
            program: root.path().join("runner"),
            args: vec![
                "--config".into(),
                root.path().join("v8project.yaml").display().to_string(),
            ],
            cwd: root.path().into(),
            env: vec![],
            env_remove: vec![],
            capture_limits: None,
            timeout: None,
            cancellation: crate::domain::cancellation::CancellationToken::new(),
        };
        assert!(run_projected(&probe, &command).is_err());
        assert!(!probe
            .config
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .parent()
            .unwrap()
            .exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("v8project.yaml")).unwrap(),
            base
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("v8project.local.yaml")).unwrap(),
            local
        );
    }
    #[test]
    fn unknown_runner_versions_are_not_probed_by_executing_an_operation() {
        assert!(check_version(VERSION).is_ok());
        for version in ["0.9.0", "0.11.0", "0.11.1", "1.0.0", "1.0.0-rc.1", ""] {
            assert!(check_version(version).is_err());
        }
    }
    #[test]
    fn target_config_is_projected_without_losing_origin_or_provider_meaning() {
        let root = tempfile::tempdir().unwrap();
        let value: Value = serde_yaml::from_str("infobases:\n  origin:\n    connection: 'File=base'\nproviders:\n  download: designer\nsource-set:\n  - name: main\n    type: configuration\n    path: src\n").unwrap();
        let projected = project_layer(value, root.path(), false).unwrap();
        assert!(projected.get("infobases").is_none());
        assert_eq!(
            projected["infobase"]["connection"].as_str().unwrap(),
            format!("File=\"{}\"", root.path().join("base").display())
        );
        assert_eq!(
            projected["providers"]["infobase.configuration.export"],
            "designer"
        );
        assert_eq!(projected["source-set"][0]["type"], "CONFIGURATION");
        assert!(
            projected.get("basePath").is_none(),
            "runner 0.11 closed schema rejects basePath"
        );
        assert_eq!(
            projected["source-set"][0]["path"].as_str().unwrap(),
            root.path().join("src").to_str().unwrap()
        );
    }
    #[test]
    fn unrepresentable_configuration_is_rejected_not_discarded() {
        let root = tempfile::tempdir().unwrap();
        for yaml in [
            "infobases: {test: {connection: 'File=test'}}",
            "infobases: {origin: {}, test: {}}",
            "infobase: {}\ninfobases: {origin: {}}",
            "providers: {reset: agent}",
        ] {
            assert!(
                project_layer(serde_yaml::from_str(yaml).unwrap(), root.path(), false).is_err(),
                "{yaml}"
            );
        }
    }
}
