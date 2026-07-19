use serde_json::Value;
use std::env;
use std::path::{Path, PathBuf};

pub fn find_plugin_root(cwd: &Path) -> Option<PathBuf> {
    if let Ok(root) = env::var("UNICA_PLUGIN_ROOT") {
        let root = PathBuf::from(root);
        if is_plugin_runtime_root(&root) {
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
        if is_plugin_runtime_root(base) {
            return Some(base.to_path_buf());
        }
    }
    None
}

fn is_plugin_runtime_root(base: &Path) -> bool {
    (base.join("skills").is_dir() && base.join(".mcp.json").is_file())
        || base.join("third-party").join("manifest.json").is_file()
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

pub fn value_to_cli_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
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

    #[test]
    fn downloaded_runtime_root_is_discovered_from_its_tool_manifest() {
        let root = temp_root("runtime-cache-root");
        let runtime_root = root.join("runtimes/0.7.0/linux-x64");
        fs::create_dir_all(runtime_root.join("third-party")).unwrap();
        fs::write(runtime_root.join("third-party/manifest.json"), "{}").unwrap();
        let exe = runtime_root.join("bin/linux-x64/unica");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();

        assert_eq!(
            find_plugin_root_with_exe(&root.join("workspace"), Some(&exe)),
            Some(runtime_root)
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
