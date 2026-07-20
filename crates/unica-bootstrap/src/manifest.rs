use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::error::{BootstrapError, Result};
use crate::platform::HostTarget;

const SOURCE_REPOSITORY: &str = "https://github.com/IngvarConsulting/unica";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeManifest {
    pub schema_version: u32,
    pub plugin_version: String,
    #[serde(default)]
    pub development: bool,
    pub source: SourceIdentity,
    pub release: ReleaseIdentity,
    pub targets: BTreeMap<String, TargetRuntime>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceIdentity {
    pub repository: String,
    pub commit: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseIdentity {
    pub repository: String,
    pub tag: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetRuntime {
    pub asset: RuntimeAsset,
    pub files: Vec<RuntimeFile>,
    pub entrypoint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAsset {
    pub name: String,
    pub url: String,
    pub media_type: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeFile {
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub executable: bool,
}

impl RuntimeManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(|error| {
            BootstrapError::new(format!(
                "failed to read runtime manifest {}: {error}",
                path.display()
            ))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            BootstrapError::new(format!(
                "failed to parse runtime manifest {}: {error}",
                path.display()
            ))
        })
    }

    pub fn validate(&self, plugin_version: &str) -> Result<()> {
        if self.schema_version != 1 {
            return Err(BootstrapError::new(format!(
                "unsupported runtime manifest schemaVersion {}",
                self.schema_version
            )));
        }
        if self.plugin_version != plugin_version {
            return Err(BootstrapError::new(format!(
                "runtime manifest plugin version {} != {plugin_version}",
                self.plugin_version
            )));
        }
        if self.source.repository != SOURCE_REPOSITORY
            || self.release.repository != SOURCE_REPOSITORY
        {
            return Err(BootstrapError::new(
                "runtime manifest repository identity is not IngvarConsulting/unica",
            ));
        }

        if self.development {
            if self.source.commit != "workspace" || self.release.tag != "workspace" {
                return Err(BootstrapError::new(
                    "development runtime manifest must use workspace identities",
                ));
            }
            if !self.targets.is_empty() {
                return Err(BootstrapError::new(
                    "development runtime manifest must not publish target assets",
                ));
            }
            return Ok(());
        }

        if !is_lower_hex(&self.source.commit, 40) {
            return Err(BootstrapError::new(
                "runtime manifest source commit must be 40 lowercase hexadecimal characters",
            ));
        }
        let expected_tag = format!("v{}", self.plugin_version);
        if self.release.tag != expected_tag {
            return Err(BootstrapError::new(format!(
                "runtime manifest release tag {} != {expected_tag}",
                self.release.tag
            )));
        }

        let actual_targets = self
            .targets
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected_targets = HostTarget::ALL
            .iter()
            .map(|target| target.as_str())
            .collect::<BTreeSet<_>>();
        if actual_targets != expected_targets {
            return Err(BootstrapError::new(format!(
                "runtime manifest targets {:?} != {:?}",
                actual_targets, expected_targets
            )));
        }

        for host_target in HostTarget::ALL {
            let name = host_target.as_str();
            let target = &self.targets[name];
            validate_target(&self.release.tag, host_target, target)?;
        }
        Ok(())
    }

    pub fn target(&self, target: HostTarget) -> Result<&TargetRuntime> {
        self.targets.get(target.as_str()).ok_or_else(|| {
            BootstrapError::new(format!(
                "runtime manifest does not contain target {}",
                target.as_str()
            ))
        })
    }
}

fn validate_target(release_tag: &str, host: HostTarget, target: &TargetRuntime) -> Result<()> {
    let name = host.as_str();
    let expected_asset = format!("unica-runtime-{name}.tar.gz");
    if target.asset.name != expected_asset {
        return Err(BootstrapError::new(format!(
            "runtime asset {} != {expected_asset}",
            target.asset.name
        )));
    }
    let expected_prefix =
        format!("https://github.com/IngvarConsulting/unica/releases/download/{release_tag}/");
    if target.asset.url != format!("{expected_prefix}{expected_asset}") {
        return Err(BootstrapError::new(format!(
            "runtime asset URL for {name} is outside the approved release origin"
        )));
    }
    if target.asset.media_type != "application/gzip" {
        return Err(BootstrapError::new(format!(
            "runtime asset mediaType for {name} must be application/gzip"
        )));
    }
    validate_sha256("runtime archive", &target.asset.sha256)?;

    if target.files.is_empty() {
        return Err(BootstrapError::new(format!(
            "runtime target {name} has no files"
        )));
    }
    let mut paths = BTreeSet::new();
    for file in &target.files {
        validate_runtime_path(&file.path)?;
        validate_sha256(&file.path, &file.sha256)?;
        if !paths.insert(file.path.as_str()) {
            return Err(BootstrapError::new(format!(
                "runtime target {name} contains duplicate file {}",
                file.path
            )));
        }
    }
    validate_runtime_path(&target.entrypoint)?;
    if !paths.contains(target.entrypoint.as_str()) {
        return Err(BootstrapError::new(format!(
            "runtime entrypoint {} is not declared in files",
            target.entrypoint
        )));
    }
    let expected_entrypoint = format!("bin/{name}/{}", host.executable_name());
    if target.entrypoint != expected_entrypoint {
        return Err(BootstrapError::new(format!(
            "runtime entrypoint {} != {expected_entrypoint}",
            target.entrypoint
        )));
    }
    Ok(())
}

fn validate_runtime_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    let unsafe_path = value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
    if unsafe_path {
        return Err(BootstrapError::new(format!(
            "unsafe runtime file path: {value}"
        )));
    }
    Ok(())
}

fn validate_sha256(label: &str, value: &str) -> Result<()> {
    if !is_lower_hex(value, 64) {
        return Err(BootstrapError::new(format!(
            "{label} sha256 must be 64 lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
