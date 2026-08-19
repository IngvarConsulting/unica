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
    version: Option<String>,
    /// Из какого архива приходит инструмент. Несколько инструментов делят
    /// один: `rlm-tools-bsl` несёт два. Отсутствует — артефакт зовётся как
    /// инструмент.
    artifact: Option<String>,
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
    version: Option<String>,
    binary_name: String,
    #[serde(default)]
    assets: BTreeMap<String, serde_json::Value>,
}

pub(crate) fn bundled_tool_version(plugin_root: &Path, tool_name: &str) -> Result<String, String> {
    let manifest_path = plugin_root.join("third-party").join("manifest.json");
    if let Ok(text) = fs::read_to_string(&manifest_path) {
        let manifest: BundledManifest = serde_json::from_str(&text)
            .map_err(|error| format!("invalid Unica third-party manifest: {error}"))?;
        if let Some(version) = manifest
            .tools
            .iter()
            .find(|tool| tool.name == tool_name)
            .and_then(|tool| tool.version.clone())
        {
            return Ok(version);
        }
    }
    let lock_path = plugin_root.join("third-party").join("tools.lock.json");
    let text = fs::read_to_string(&lock_path)
        .map_err(|error| format!("failed to read Unica tools lock: {error}"))?;
    let lock: ToolsLock = serde_json::from_str(&text)
        .map_err(|error| format!("invalid Unica tools lock: {error}"))?;
    lock.tools
        .into_iter()
        .find(|tool| tool.name == tool_name)
        .and_then(|tool| tool.version)
        .ok_or_else(|| format!("tool {tool_name} has no pinned version"))
}

/// Каталог, куда bootstrap ставит артефакты по имени и версии. В дереве
/// разработки переменной нет, и поиск идёт прежним путём — рядом с плагином.
const ARTIFACT_CACHE_ENV: &str = "UNICA_ARTIFACT_CACHE";

pub(crate) fn resolve_bundled_tool(
    plugin_root: &Path,
    tool_name: &str,
    verify: bool,
) -> Result<BundledTool, String> {
    let target_id = current_target_id()?;
    let cache = std::env::var_os(ARTIFACT_CACHE_ENV).map(PathBuf::from);
    resolve_bundled_tool_in(plugin_root, cache.as_deref(), tool_name, target_id, verify)
}

/// Разрешение с явно названным кешем артефактов: так его видно в тесте, а
/// переменная окружения остаётся деталью вызывающего.
fn resolve_bundled_tool_in(
    plugin_root: &Path,
    artifact_cache: Option<&Path>,
    tool_name: &str,
    target_id: &str,
    verify: bool,
) -> Result<BundledTool, String> {
    if let Some(cache) = artifact_cache {
        if let Some(tool) =
            resolve_from_artifact_cache(plugin_root, cache, tool_name, target_id, verify)?
        {
            return Ok(tool);
        }
    }
    resolve_bundled_tool_for_target(plugin_root, tool_name, target_id, verify)
}

/// Движок в кеше артефактов: `<кеш>/<инструмент>/<версия>/<цель>/<путь>`.
/// Отсутствие установки — не ошибка: её ещё могут доставить, и решает это
/// вызывающий, а не резолвер.
fn resolve_from_artifact_cache(
    plugin_root: &Path,
    cache: &Path,
    tool_name: &str,
    target_id: &str,
    verify: bool,
) -> Result<Option<BundledTool>, String> {
    let Ok(version) = bundled_tool_version(plugin_root, tool_name) else {
        return Ok(None);
    };
    let manifest_path = plugin_root.join("third-party").join("manifest.json");
    let Ok(bytes) = fs::read(&manifest_path) else {
        return Ok(None);
    };
    let Ok(manifest) = serde_json::from_slice::<BundledManifest>(&bytes) else {
        return Ok(None);
    };
    let Ok(binary) = manifest_binary(&manifest, tool_name, target_id) else {
        return Ok(None);
    };

    let artifact = manifest
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .and_then(|tool| tool.artifact.clone())
        .unwrap_or_else(|| tool_name.to_string());
    let program = cache
        .join(&artifact)
        .join(&version)
        .join(target_id)
        .join(&binary.binary_path);
    if !program.is_file() {
        return Ok(None);
    }
    if verify {
        verify_binary(tool_name, &program, &binary.sha256)?;
    }
    Ok(Some(BundledTool {
        program,
        warnings: Vec::new(),
    }))
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

        assert_eq!(
            bundled_tool_version(&plugin_root, "bsl-analyzer").unwrap(),
            "test"
        );
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

    #[test]
    fn an_engine_is_taken_from_the_artifact_cache_when_one_is_named() {
        // Разрез поставки уносит движки из каталога плагина: они лежат в кеше
        // артефактов под своей версией, и рантайм обязан находить их там.
        let plugin_root = temp_plugin_root("artifact-cache");
        write_manifest_with_bsl_analyzer(&plugin_root);
        let cache = plugin_root.join("..").join("artifact-cache");
        let installed = cache
            .join("bsl-analyzer")
            .join("test")
            .join("darwin-arm64")
            .join("bin/darwin-arm64");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("bsl-analyzer"), "darwin-binary").unwrap();

        let tool = resolve_bundled_tool_in(
            &plugin_root,
            Some(cache.as_path()),
            "bsl-analyzer",
            "darwin-arm64",
            true,
        )
        .unwrap();

        assert_eq!(tool.program, installed.join("bsl-analyzer"));
    }

    #[test]
    fn without_an_artifact_cache_the_engine_is_still_found_beside_the_plugin() {
        // Дерево разработки живёт без кеша артефактов, и прежний путь остаётся.
        let plugin_root = temp_plugin_root("no-artifact-cache");
        write_manifest_with_bsl_analyzer(&plugin_root);

        let tool =
            resolve_bundled_tool_in(&plugin_root, None, "bsl-analyzer", "darwin-arm64", true)
                .unwrap();

        assert_eq!(
            tool.program,
            plugin_root.join("bin/darwin-arm64/bsl-analyzer")
        );
    }

    #[test]
    fn tools_sharing_one_archive_resolve_to_one_artifact() {
        // rlm-tools-bsl несёт два инструмента в одном файле на 69 МБ: кеш по
        // имени инструмента скачал бы его дважды.
        let plugin_root = temp_plugin_root("shared-artifact");
        fs::create_dir_all(plugin_root.join("bin/darwin-arm64")).unwrap();
        fs::write(plugin_root.join("bin/darwin-arm64/rlm-bsl-index"), "index").unwrap();
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "targetTriple": "aarch64-apple-darwin",
  "tools": [
    {
      "name": "rlm-bsl-index",
      "version": "1.33.0",
      "artifact": "rlm-tools-bsl",
      "binaryPath": "bin/darwin-arm64/rlm-bsl-index",
      "sha256": "3c9b8f2e6d1a4c7b0e5f8a2d6c9b3e7f1a4d8c2b5e9f3a7d1c4b8e2f6a9d3c7b"
    }
  ]
}"#,
        )
        .unwrap();
        let cache = plugin_root.join("..").join("shared-cache");
        let installed = cache
            .join("rlm-tools-bsl")
            .join("1.33.0")
            .join("darwin-arm64")
            .join("bin/darwin-arm64");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("rlm-bsl-index"), "index").unwrap();

        let tool = resolve_bundled_tool_in(
            &plugin_root,
            Some(cache.as_path()),
            "rlm-bsl-index",
            "darwin-arm64",
            false,
        )
        .unwrap();

        assert_eq!(
            tool.program,
            installed.join("rlm-bsl-index"),
            "путь строится по артефакту, а не по имени инструмента"
        );
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
