use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::AdapterOutcome;
use serde_json::{Map, Value};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct LegacyScriptAdapter;

impl LegacyScriptAdapter {
    pub fn invoke(
        skill: &str,
        script_name: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<AdapterOutcome, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root; set UNICA_PLUGIN_ROOT or run from a repository/package containing plugins/unica".to_string()
        })?;
        let script = legacy_script_path(&plugin_root, skill, script_name);
        let mut command = vec!["python3".to_string(), script.display().to_string()];
        command.extend(script_args(args));

        if dry_run {
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!(
                    "dry run: would execute {tool_name} through legacy script {script_name}"
                ),
                changes: if mutating {
                    vec!["no files changed because dryRun is true".to_string()]
                } else {
                    Vec::new()
                },
                warnings: if script.exists() {
                    Vec::new()
                } else {
                    vec![format!("fallback script not found: {}", script.display())]
                },
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: Some(command),
            });
        }

        if !script.exists() {
            return Err(format!("fallback script not found: {}", script.display()));
        }

        let output = Command::new("python3")
            .arg(&script)
            .args(script_args(args))
            .current_dir(&context.cwd)
            .output()
            .map_err(|err| format!("failed to execute python fallback: {err}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let ok = output.status.success();
        Ok(AdapterOutcome {
            ok,
            summary: if ok {
                format!("{tool_name} completed through legacy script {script_name}")
            } else {
                format!("{tool_name} failed through legacy script {script_name}")
            },
            changes: if mutating {
                vec!["legacy script executed with dryRun=false".to_string()]
            } else {
                Vec::new()
            },
            warnings: if ok {
                Vec::new()
            } else {
                vec![format!("fallback exited with status {}", output.status)]
            },
            errors: if ok {
                Vec::new()
            } else {
                vec![stderr.trim().to_string()]
            },
            artifacts: Vec::new(),
            stdout: Some(stdout),
            stderr: Some(stderr),
            command: Some(command),
        })
    }
}

pub fn legacy_script_path(plugin_root: &Path, skill: &str, script_name: &str) -> PathBuf {
    plugin_root
        .join("scripts")
        .join("legacy")
        .join(skill)
        .join(script_name)
}

pub fn find_plugin_root(cwd: &Path) -> Option<PathBuf> {
    if let Ok(root) = env::var("UNICA_PLUGIN_ROOT") {
        let root = PathBuf::from(root);
        if root.join("skills").is_dir() {
            return Some(root);
        }
    }

    let current_exe = env::current_exe().ok();
    find_plugin_root_with_exe(cwd, current_exe.as_deref())
}

fn find_plugin_root_with_exe(cwd: &Path, current_exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(exe) = current_exe {
        if let Some(root) = plugin_root_containing_exe(exe) {
            return Some(root);
        }
    }

    for base in cwd.ancestors() {
        if let Some(root) = plugin_root_from_base(base) {
            return Some(root);
        }
    }

    if let Some(exe) = current_exe {
        for base in exe.ancestors() {
            if let Some(root) = plugin_root_from_base(base) {
                return Some(root);
            }
        }
    }

    None
}

fn plugin_root_containing_exe(exe: &Path) -> Option<PathBuf> {
    for base in exe.ancestors() {
        if base.join("skills").is_dir() && base.join(".mcp.json").is_file() {
            return Some(base.to_path_buf());
        }
    }
    None
}

fn plugin_root_from_base(base: &Path) -> Option<PathBuf> {
    let candidate = base.join("plugins").join("unica");
    if candidate.join("skills").is_dir() {
        return Some(candidate);
    }
    if base.join("skills").is_dir() && base.join(".mcp.json").is_file() {
        return Some(base.to_path_buf());
    }
    None
}

pub fn script_args(args: &Map<String, Value>) -> Vec<String> {
    let mut result = Vec::new();
    for (key, value) in args {
        if matches!(key.as_str(), "dryRun" | "cwd" | "confirm" | "args") {
            continue;
        }
        let flag = format!("-{}", pascal_case_key(key));
        match value {
            Value::Bool(true) => result.push(flag),
            Value::Bool(false) | Value::Null => {}
            Value::Array(items) => {
                result.push(flag);
                result.push(
                    items
                        .iter()
                        .map(value_to_cli_string)
                        .collect::<Vec<_>>()
                        .join(" ;; "),
                );
            }
            other => {
                result.push(flag);
                result.push(value_to_cli_string(other));
            }
        }
    }
    result
}

pub fn value_to_cli_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn pascal_case_key(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::find_plugin_root_with_exe;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn packaged_executable_plugin_root_wins_over_cwd_source_checkout() {
        let root = temp_root("exe-root-wins");
        let source_checkout = root.join("source");
        let source_plugin = source_checkout.join("plugins/unica");
        create_plugin_root(&source_plugin);
        let workspace_cwd = source_checkout.join(".build/release-assessment/work/bsp/ssl_3_2");
        fs::create_dir_all(&workspace_cwd).unwrap();

        let package_plugin = root.join("marketplace/plugins/unica");
        create_plugin_root(&package_plugin);
        let exe = package_plugin.join("bin/linux-x64/unica");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();

        assert_eq!(
            find_plugin_root_with_exe(&workspace_cwd, Some(&exe)),
            Some(package_plugin)
        );
    }

    #[test]
    fn cwd_plugin_root_is_used_when_executable_is_not_inside_a_plugin() {
        let root = temp_root("cwd-root-fallback");
        let workspace = root.join("workspace");
        let plugin_root = workspace.join("plugins/unica");
        create_plugin_root(&plugin_root);
        let cwd = workspace.join("src/project");
        fs::create_dir_all(&cwd).unwrap();
        let exe = root.join("target/debug/unica");

        assert_eq!(
            find_plugin_root_with_exe(&cwd, Some(&exe)),
            Some(plugin_root)
        );
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-plugin-root-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn create_plugin_root(path: &Path) {
        fs::create_dir_all(path.join("skills")).unwrap();
        fs::write(path.join(".mcp.json"), "{}").unwrap();
    }
}
