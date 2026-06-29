use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Deserialize)]
struct ToolManifest {
    #[serde(default)]
    tools: Vec<ManifestTool>,
}

#[derive(Debug, Deserialize)]
struct ManifestTool {
    name: String,
    #[serde(default)]
    binaries: BTreeMap<String, BinaryEntry>,
    #[serde(rename = "binaryPath")]
    binary_path: Option<String>,
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BinaryEntry {
    #[serde(rename = "binaryPath")]
    binary_path: String,
    sha256: String,
}

pub fn resolve_bundled_tool(plugin_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
    resolve_bundled_tool_for_target(plugin_root, tool_name, current_target_id()?)
}

pub(crate) fn resolve_bundled_tool_for_target(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
) -> Result<PathBuf, String> {
    let manifest_path = plugin_root.join("third-party").join("manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "Unica third-party manifest not found: {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest: ToolManifest = serde_json::from_str(&manifest_text).map_err(|error| {
        format!(
            "failed to read Unica third-party manifest {}: {error}",
            manifest_path.display()
        )
    })?;

    let tool = manifest
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .ok_or_else(|| format!("tool not found in Unica third-party manifest: {tool_name}"))?;

    let binary = select_binary(tool, tool_name, target_id)?;
    let relative = validate_binary_path(tool_name, target_id, binary.binary_path)?;
    let binary_path = plugin_root.join(relative);
    if !binary_path.is_file() {
        return Err(format!(
            "Unica binary is missing for {tool_name} ({target_id}): {}",
            binary_path.display()
        ));
    }

    let actual_sha = file_sha256(&binary_path).map_err(|error| {
        format!(
            "failed to hash Unica binary {}: {error}",
            binary_path.display()
        )
    })?;
    let expected_sha = binary.sha256.to_ascii_lowercase();
    if actual_sha != expected_sha {
        return Err(format!(
            "Unica binary checksum mismatch for {tool_name} ({target_id}) at {}. expected: {}, actual: {}",
            binary_path.display(),
            binary.sha256,
            actual_sha
        ));
    }

    Ok(binary_path)
}

fn current_target_id() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("win-x64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        (os, arch) => Err(format!("Unica does not ship binaries for {os}-{arch}")),
    }
}

fn select_binary<'a>(
    tool: &'a ManifestTool,
    tool_name: &str,
    target_id: &str,
) -> Result<BinaryRef<'a>, String> {
    if !tool.binaries.is_empty() {
        let binary = tool.binaries.get(target_id).ok_or_else(|| {
            let supported = tool.binaries.keys().cloned().collect::<Vec<_>>().join(", ");
            format!("tool {tool_name} is not packaged for {target_id}; supported: {supported}")
        })?;
        return Ok(BinaryRef {
            binary_path: binary.binary_path.as_str(),
            sha256: binary.sha256.as_str(),
        });
    }

    let binary_path = tool.binary_path.as_deref().ok_or_else(|| {
        format!("tool {tool_name} manifest entry has no binaries for {target_id}")
    })?;
    let sha256 = tool
        .sha256
        .as_deref()
        .ok_or_else(|| format!("tool {tool_name} manifest entry has no sha256 for {target_id}"))?;
    Ok(BinaryRef {
        binary_path,
        sha256,
    })
}

#[derive(Debug, Clone, Copy)]
struct BinaryRef<'a> {
    binary_path: &'a str,
    sha256: &'a str,
}

fn validate_binary_path(
    tool_name: &str,
    target_id: &str,
    binary_path: &str,
) -> Result<PathBuf, String> {
    let path = Path::new(binary_path);
    if path.is_absolute() || looks_rooted_or_drive_absolute(binary_path) {
        return Err(format!(
            "Unica binary path for {tool_name} ({target_id}) must be relative: {binary_path}"
        ));
    }

    let mut relative = PathBuf::new();
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                has_normal_component = true;
                relative.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "Unica binary path for {tool_name} ({target_id}) escapes plugin root: {binary_path}"
                ));
            }
        }
    }

    if !has_normal_component {
        return Err(format!(
            "Unica binary path for {tool_name} ({target_id}) is empty"
        ));
    }
    Ok(relative)
}

fn looks_rooted_or_drive_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with('/')
        || value.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

fn file_sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 64];
    loop {
        let bytes = file.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }
    Ok(hex_digest(&hasher.finalize()))
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_win_x64_binary_from_manifest() {
        let root = test_plugin_root("resolve-win");
        let binary = write_binary(&root, "bin/win-x64/rlm-bsl-index.exe", b"fake-index");
        write_manifest(
            &root,
            "rlm-bsl-index",
            "win-x64",
            "bin/win-x64/rlm-bsl-index.exe",
            &file_sha256(&binary).unwrap(),
        );

        let resolved = resolve_bundled_tool_for_target(&root, "rlm-bsl-index", "win-x64").unwrap();

        assert_eq!(resolved, binary);
        cleanup(&root);
    }

    #[test]
    fn reports_unsupported_target_with_supported_targets() {
        let root = test_plugin_root("unsupported");
        let binary = write_binary(&root, "bin/win-x64/rlm-bsl-index.exe", b"fake-index");
        write_manifest(
            &root,
            "rlm-bsl-index",
            "win-x64",
            "bin/win-x64/rlm-bsl-index.exe",
            &file_sha256(&binary).unwrap(),
        );

        let error =
            resolve_bundled_tool_for_target(&root, "rlm-bsl-index", "linux-x64").unwrap_err();

        assert!(error.contains("rlm-bsl-index"));
        assert!(error.contains("linux-x64"));
        assert!(error.contains("win-x64"));
        cleanup(&root);
    }

    #[test]
    fn reports_missing_binary_with_actual_path() {
        let root = test_plugin_root("missing");
        write_manifest(
            &root,
            "rlm-bsl-index",
            "win-x64",
            "bin/win-x64/rlm-bsl-index.exe",
            "abc123",
        );

        let error = resolve_bundled_tool_for_target(&root, "rlm-bsl-index", "win-x64").unwrap_err();

        assert!(error.contains("rlm-bsl-index"));
        assert!(error.contains("win-x64"));
        assert!(error.contains("bin"));
        assert!(error.contains("rlm-bsl-index.exe"));
        cleanup(&root);
    }

    #[test]
    fn rejects_checksum_mismatch() {
        let root = test_plugin_root("checksum");
        write_binary(&root, "bin/win-x64/rlm-bsl-index.exe", b"fake-index");
        write_manifest(
            &root,
            "rlm-bsl-index",
            "win-x64",
            "bin/win-x64/rlm-bsl-index.exe",
            "000000",
        );

        let error = resolve_bundled_tool_for_target(&root, "rlm-bsl-index", "win-x64").unwrap_err();

        assert!(error.contains("checksum mismatch"));
        assert!(error.contains("rlm-bsl-index"));
        assert!(error.contains("actual"));
        cleanup(&root);
    }

    #[test]
    fn rejects_binary_path_escape() {
        let root = test_plugin_root("escape");
        write_manifest(
            &root,
            "rlm-bsl-index",
            "win-x64",
            "../rlm-bsl-index.exe",
            "abc123",
        );

        let error = resolve_bundled_tool_for_target(&root, "rlm-bsl-index", "win-x64").unwrap_err();

        assert!(error.contains("escapes plugin root"));
        cleanup(&root);
    }

    #[test]
    fn rejects_windows_absolute_binary_path_on_any_host() {
        let root = test_plugin_root("absolute");
        write_manifest(
            &root,
            "rlm-bsl-index",
            "win-x64",
            "C:\\tools\\rlm-bsl-index.exe",
            "abc123",
        );

        let error = resolve_bundled_tool_for_target(&root, "rlm-bsl-index", "win-x64").unwrap_err();

        assert!(error.contains("must be relative"));
        cleanup(&root);
    }

    fn test_plugin_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-bundled-tools-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("third-party")).unwrap();
        root
    }

    fn write_binary(root: &Path, relative: &str, content: &[u8]) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn write_manifest(root: &Path, name: &str, target: &str, binary_path: &str, sha256: &str) {
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "tools": [
                {
                    "name": name,
                    "binaries": {
                        target: {
                            "targetTriple": "x86_64-pc-windows-msvc",
                            "binaryPath": binary_path,
                            "sha256": sha256
                        }
                    }
                }
            ]
        });
        fs::write(
            root.join("third-party").join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }
}
