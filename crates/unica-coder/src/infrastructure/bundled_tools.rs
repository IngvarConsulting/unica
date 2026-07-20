use crate::infrastructure::platform::current_target_id;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct BundledTool {
    pub(crate) program: PathBuf,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundledManifest {
    #[serde(default)]
    tools: Vec<ManifestTool>,
    target_triple: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestTool {
    name: String,
    binaries: Option<BTreeMap<String, ManifestBinary>>,
    binary_path: Option<String>,
    sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestBinary {
    target_triple: Option<String>,
    binary_path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolsLock {
    targets: BTreeMap<String, LockTarget>,
    tools: Vec<LockTool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockTarget {
    exe: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockTool {
    name: String,
    binary_name: String,
    #[serde(default)]
    assets: BTreeMap<String, serde_json::Value>,
}

pub(crate) fn resolve_bundled_tool(
    plugin_root: &Path,
    tool_name: &str,
    verify: bool,
) -> Result<BundledTool, String> {
    let target_id = current_target_id()?;
    resolve_bundled_tool_for_target(plugin_root, tool_name, target_id, verify)
}

fn resolve_bundled_tool_for_target(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
    verify: bool,
) -> Result<BundledTool, String> {
    let manifest_path = plugin_root.join("third-party").join("manifest.json");
    if !manifest_path.is_file() {
        let reason = format!(
            "Unica third-party manifest not found: {}",
            manifest_path.display()
        );
        if verify {
            return Err(reason);
        }
        return resolve_from_lock_for_dry_run(plugin_root, tool_name, target_id, &reason);
    }

    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read Unica third-party manifest: {err}"))?;
    let manifest: BundledManifest = serde_json::from_str(&manifest_text)
        .map_err(|err| format!("invalid Unica third-party manifest: {err}"))?;

    let binary = match manifest_binary(&manifest, tool_name, target_id) {
        Ok(binary) => binary,
        Err(error) if !verify => {
            return resolve_from_lock_for_dry_run(plugin_root, tool_name, target_id, &error);
        }
        Err(error) => return Err(error),
    };

    let program = manifest_relative_path(plugin_root, &binary.binary_path)?;
    let mut warnings = Vec::new();
    if verify {
        verify_binary(tool_name, &program, &binary.sha256)?;
    } else if !program.is_file() {
        warnings.push(format!(
            "dry run: bundled tool binary is not present yet: {}",
            program.display()
        ));
    }
    Ok(BundledTool { program, warnings })
}

fn manifest_binary(
    manifest: &BundledManifest,
    tool_name: &str,
    target_id: &str,
) -> Result<ManifestBinary, String> {
    let tool = manifest
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .ok_or_else(|| format!("tool not found in manifest: {tool_name}"))?;

    if let Some(binaries) = &tool.binaries {
        let binary = binaries.get(target_id).ok_or_else(|| {
            let supported = binaries.keys().cloned().collect::<Vec<_>>().join(", ");
            format!("tool {tool_name} is not packaged for {target_id}; supported: {supported}")
        })?;
        if let Some(target_triple) = &binary.target_triple {
            let expected = target_triple_for_id(target_id)?;
            if target_triple != expected {
                return Err(format!(
                    "tool {tool_name} manifest target triple mismatch for {target_id}: {target_triple} != {expected}"
                ));
            }
        }
        return Ok(binary.clone());
    }

    if let Some(target_triple) = &manifest.target_triple {
        let expected = target_triple_for_id(target_id)?;
        if target_triple != expected {
            return Err(format!(
                "Unica ships binaries for {target_triple}; current host is {expected}."
            ));
        }
    }
    Ok(ManifestBinary {
        target_triple: manifest.target_triple.clone(),
        binary_path: tool.binary_path.clone().ok_or_else(|| {
            format!("tool {tool_name} is missing binaryPath in third-party manifest")
        })?,
        sha256: tool
            .sha256
            .clone()
            .ok_or_else(|| format!("tool {tool_name} is missing sha256 in third-party manifest"))?,
    })
}

fn resolve_from_lock_for_dry_run(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
    reason: &str,
) -> Result<BundledTool, String> {
    let lock_path = plugin_root.join("third-party").join("tools.lock.json");
    let lock_text = fs::read_to_string(&lock_path).map_err(|err| {
        format!("{reason}; failed to read Unica tools lock for dry run fallback: {err}")
    })?;
    let lock: ToolsLock = serde_json::from_str(&lock_text)
        .map_err(|err| format!("{reason}; invalid Unica tools lock: {err}"))?;
    let target = lock
        .targets
        .get(target_id)
        .ok_or_else(|| format!("{reason}; tools lock has no target {target_id}"))?;
    let tool = lock
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .ok_or_else(|| format!("{reason}; tool not found in tools lock: {tool_name}"))?;
    if !tool.assets.contains_key(target_id) {
        return Err(format!(
            "{reason}; tool {tool_name} has no tools.lock asset for {target_id}"
        ));
    }

    Ok(BundledTool {
        program: plugin_root
            .join("bin")
            .join(target_id)
            .join(format!("{}{}", tool.binary_name, target.exe)),
        warnings: vec![format!(
            "dry run: {reason}; using expected bundled binary path from tools.lock.json"
        )],
    })
}

fn verify_binary(tool_name: &str, program: &Path, expected_sha: &str) -> Result<(), String> {
    if !program.is_file() {
        return Err(format!("Unica binary is missing: {}", program.display()));
    }
    let actual = sha256_file(program)?;
    if !actual.eq_ignore_ascii_case(expected_sha) {
        return Err(format!(
            "Unica binary checksum mismatch for {tool_name}. expected: {expected_sha}; actual: {actual}"
        ));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|err| format!("failed to open bundled tool for checksum: {err}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 64];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("failed to read bundled tool for checksum: {err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn manifest_relative_path(plugin_root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(format!(
            "manifest binaryPath must be relative to plugin root: {relative}"
        ));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "manifest binaryPath must stay inside plugin root: {relative}"
                ));
            }
        }
    }
    Ok(plugin_root.join(path))
}

fn target_triple_for_id(target_id: &str) -> Result<&'static str, String> {
    match target_id {
        "darwin-arm64" => Ok("aarch64-apple-darwin"),
        "linux-x64" => Ok("x86_64-unknown-linux-gnu"),
        "win-x64" => Ok("x86_64-pc-windows-msvc"),
        other => Err(format!("unsupported Unica bundled target: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_windows_binary_from_manifest_without_script_wrapper() {
        let plugin_root = temp_plugin_root("win-target");
        write_manifest_with_bsl_analyzer(&plugin_root);

        let tool =
            resolve_bundled_tool_for_target(&plugin_root, "bsl-analyzer", "win-x64", true).unwrap();

        assert_eq!(
            tool.program,
            plugin_root.join("bin/win-x64/bsl-analyzer.exe")
        );
        assert!(!tool
            .program
            .components()
            .any(|component| component.as_os_str() == "scripts"));
        assert!(!tool.program.to_string_lossy().ends_with(".ps1"));
        assert!(!tool.program.to_string_lossy().ends_with(".sh"));
    }

    #[test]
    fn dry_run_resolves_expected_binary_path_from_tools_lock_for_source_manifest() {
        let plugin_root = temp_plugin_root("source-manifest");
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{"schemaVersion":2,"sourceManifest":true,"tools":[]}"#,
        )
        .unwrap();
        fs::write(
            plugin_root.join("third-party/tools.lock.json"),
            r#"{
  "schemaVersion": 1,
  "targets": {
    "linux-x64": {
      "targetTriple": "x86_64-unknown-linux-gnu",
      "exe": ""
    }
  },
  "tools": [
    {
      "name": "v8-runner",
      "binaryName": "v8-runner",
      "assets": {"linux-x64": {"assetName": "v8-runner"}}
    }
  ]
}"#,
        )
        .unwrap();

        let tool =
            resolve_bundled_tool_for_target(&plugin_root, "v8-runner", "linux-x64", false).unwrap();

        assert_eq!(tool.program, plugin_root.join("bin/linux-x64/v8-runner"));
        assert!(tool
            .warnings
            .iter()
            .any(|warning| warning.contains("dry run")));
        assert!(!tool.program.to_string_lossy().contains("run-v8-runner.sh"));
    }

    #[test]
    fn rejects_checksum_mismatch_before_execution() {
        let plugin_root = temp_plugin_root("checksum");
        write_manifest_with_bsl_analyzer(&plugin_root);
        fs::write(
            plugin_root.join("bin/darwin-arm64/bsl-analyzer"),
            "different",
        )
        .unwrap();

        let error =
            resolve_bundled_tool_for_target(&plugin_root, "bsl-analyzer", "darwin-arm64", true)
                .unwrap_err();

        assert!(error.contains("checksum mismatch"));
    }

    fn temp_plugin_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-bundled-tools-{name}-{nanos}"));
        let plugin_root = root.join("plugins/unica");
        fs::create_dir_all(plugin_root.join("third-party")).unwrap();
        fs::create_dir_all(plugin_root.join("skills")).unwrap();
        plugin_root
    }

    fn write_manifest_with_bsl_analyzer(plugin_root: &Path) {
        fs::create_dir_all(plugin_root.join("bin/win-x64")).unwrap();
        fs::create_dir_all(plugin_root.join("bin/darwin-arm64")).unwrap();
        fs::write(
            plugin_root.join("bin/win-x64/bsl-analyzer.exe"),
            "win-binary",
        )
        .unwrap();
        fs::write(
            plugin_root.join("bin/darwin-arm64/bsl-analyzer"),
            "darwin-binary",
        )
        .unwrap();
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "tools": [
    {
      "name": "bsl-analyzer",
      "version": "test",
      "binaries": {
        "win-x64": {
          "targetTriple": "x86_64-pc-windows-msvc",
          "binaryPath": "bin/win-x64/bsl-analyzer.exe",
          "sha256": "81202f8a7e65792b816fb962ae81f4c7d91e6be81fc691db7fbf942455c1bc80"
        },
        "darwin-arm64": {
          "targetTriple": "aarch64-apple-darwin",
          "binaryPath": "bin/darwin-arm64/bsl-analyzer",
          "sha256": "e4002e1adb76d4e2bb4846ab27463ff6368d18b727eb2bd519e1579f0baf491b"
        }
      }
    }
  ]
}"#,
        )
        .unwrap();
    }
}
