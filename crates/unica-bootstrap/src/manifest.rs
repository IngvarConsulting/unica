use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::error::{BootstrapError, Failure, Result};
use crate::platform::HostTarget;

const SOURCE_REPOSITORY: &str = "https://github.com/IngvarConsulting/unica";

/// Откуда приезжает ядро: оно собирается здесь и лежит в выпуске плагина.
const CORE_RELEASE_ORIGIN: &str = "https://github.com/IngvarConsulting/unica/releases/download/";

/// Откуда приезжают движки. Тулчейн публикует их по архиву на инструмент и
/// цель, с суммами и происхождением; копия тех же байтов в выпуске плагина
/// стоила 242 МБ на выпуск и не давала ничего.
///
/// Адресов ровно два, и оба названы. Третий — новая запись реестра, а не
/// правка этого списка: поартефактная проверка защищает от опечатки ровно
/// потому, что список закрыт.
const ENGINE_RELEASE_ORIGIN: &str =
    "https://github.com/IngvarConsulting/unica-toolchain/releases/download/";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeManifest {
    pub schema_version: u32,
    pub plugin_version: String,
    #[serde(default)]
    pub development: bool,
    pub source: SourceIdentity,
    pub release: ReleaseIdentity,
    /// Артефакты по отдельности: у каждого своя версия и свой архив на цель.
    /// Ключ установки берётся из версии артефакта, поэтому выпуск плагина не
    /// объявляет холодным то, что не менялось.
    #[serde(default)]
    pub artifacts: BTreeMap<String, Artifact>,
}

/// Роль артефакта в запуске. Ядро едет в стартовом бюджете хоста, движок — нет.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactRole {
    Core,
    Engine,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub version: String,
    pub role: ArtifactRole,
    pub targets: BTreeMap<String, TargetRuntime>,
}

/// Имя единственного артефакта роли `core`.
pub const CORE_ARTIFACT: &str = "unica";

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
    /// Что запускает bootstrap. Есть только у ядра: движок он не запускает,
    /// его зовёт рантайм, и точка входа там своя на каждый инструмент.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
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
            BootstrapError::of(
                Failure::Configuration,
                format!(
                    "failed to read runtime manifest {}: {error}",
                    path.display()
                ),
            )
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            BootstrapError::of(
                Failure::Configuration,
                format!(
                    "failed to parse runtime manifest {}: {error}",
                    path.display()
                ),
            )
        })
    }

    /// Артефакт ядра — единственный, который обязан быть в манифесте всегда.
    pub fn core(&self) -> Result<&Artifact> {
        self.artifact(CORE_ARTIFACT)
    }

    /// Артефакт по имени: версия ключует установку, цели несут архивы.
    pub fn artifact(&self, name: &str) -> Result<&Artifact> {
        self.artifacts
            .get(name)
            .ok_or_else(|| BootstrapError::new(format!("runtime manifest has no artifact {name}")))
    }

    pub fn validate(&self, plugin_version: &str) -> Result<()> {
        if self.schema_version != 2 {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!(
                    "unsupported runtime manifest schemaVersion {}",
                    self.schema_version
                ),
            ));
        }
        if self.plugin_version != plugin_version {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!(
                    "runtime manifest plugin version {} != {plugin_version}",
                    self.plugin_version
                ),
            ));
        }
        if self.source.repository != SOURCE_REPOSITORY
            || self.release.repository != SOURCE_REPOSITORY
        {
            return Err(BootstrapError::of(
                Failure::Configuration,
                "runtime manifest repository identity is not IngvarConsulting/unica",
            ));
        }

        if self.development {
            if self.source.commit != "workspace" || self.release.tag != "workspace" {
                return Err(BootstrapError::of(
                    Failure::Configuration,
                    "development runtime manifest must use workspace identities",
                ));
            }
            if !self.artifacts.is_empty() {
                return Err(BootstrapError::of(
                    Failure::Configuration,
                    "development runtime manifest must not publish target assets",
                ));
            }
            return Ok(());
        }

        if !is_lower_hex(&self.source.commit, 40) {
            return Err(BootstrapError::of(
                Failure::Configuration,
                "runtime manifest source commit must be 40 lowercase hexadecimal characters",
            ));
        }
        let expected_tag = format!("v{}", self.plugin_version);
        if self.release.tag != expected_tag {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!(
                    "runtime manifest release tag {} != {expected_tag}",
                    self.release.tag
                ),
            ));
        }

        if self.artifacts.is_empty() {
            return Err(BootstrapError::of(
                Failure::Configuration,
                "runtime manifest publishes no artifacts",
            ));
        }
        let core = self.core()?;
        if core.role != ArtifactRole::Core {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!("artifact {CORE_ARTIFACT} must carry role core"),
            ));
        }
        for (name, artifact) in &self.artifacts {
            if (name == CORE_ARTIFACT) != (artifact.role == ArtifactRole::Core) {
                return Err(BootstrapError::of(
                    Failure::Configuration,
                    format!("artifact {name} declares a role that does not match its name"),
                ));
            }
            if artifact.version.is_empty() {
                return Err(BootstrapError::of(
                    Failure::Configuration,
                    format!("artifact {name} has no version"),
                ));
            }
            let actual_targets = artifact
                .targets
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let expected_targets = HostTarget::ALL
                .iter()
                .map(|target| target.as_str())
                .collect::<BTreeSet<_>>();
            if actual_targets != expected_targets {
                return Err(BootstrapError::of(
                    Failure::Configuration,
                    format!(
                        "artifact {name} targets {:?} != {:?}",
                        actual_targets, expected_targets
                    ),
                ));
            }
            for host_target in HostTarget::ALL {
                validate_target(
                    name,
                    &self.release.tag,
                    host_target,
                    &artifact.targets[host_target.as_str()],
                )?;
            }
        }
        Ok(())
    }

    /// Цель артефакта. Имя артефакта обязательно: в манифесте их несколько, и
    /// молчаливое обращение к ядру скрыло бы опечатку в имени движка.
    pub fn artifact_target(&self, artifact: &str, target: HostTarget) -> Result<&TargetRuntime> {
        let entry = self.artifacts.get(artifact).ok_or_else(|| {
            BootstrapError::of(
                Failure::Configuration,
                format!("runtime manifest has no artifact {artifact}"),
            )
        })?;
        entry.targets.get(target.as_str()).ok_or_else(|| {
            BootstrapError::of(
                Failure::Configuration,
                format!(
                    "artifact {artifact} does not contain target {}",
                    target.as_str()
                ),
            )
        })
    }

    pub fn target(&self, target: HostTarget) -> Result<&TargetRuntime> {
        self.artifact_target(CORE_ARTIFACT, target)
    }
}

fn validate_target(
    artifact: &str,
    release_tag: &str,
    host: HostTarget,
    target: &TargetRuntime,
) -> Result<()> {
    let name = host.as_str();
    if artifact == CORE_ARTIFACT {
        // Ядро собирается здесь: имя выводится единым правилом, а адрес прибит
        // к выпуску плагина под тегом его версии.
        let expected_asset = format!("{artifact}-runtime-{name}.tar.gz");
        if target.asset.name != expected_asset {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!("runtime asset {} != {expected_asset}", target.asset.name),
            ));
        }
        if target.asset.url != format!("{CORE_RELEASE_ORIGIN}{release_tag}/{expected_asset}") {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!("runtime asset URL for {name} is outside the approved release origin"),
            ));
        }
    } else {
        validate_engine_asset(artifact, name, target)?;
    }
    if target.asset.media_type != "application/gzip" {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("runtime asset mediaType for {name} must be application/gzip"),
        ));
    }
    validate_sha256("runtime archive", &target.asset.sha256)?;

    if target.files.is_empty() {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("runtime target {name} has no files"),
        ));
    }
    let mut paths = BTreeSet::new();
    for file in &target.files {
        validate_runtime_path(&file.path)?;
        validate_sha256(&file.path, &file.sha256)?;
        if !paths.insert(file.path.as_str()) {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!(
                    "runtime target {name} contains duplicate file {}",
                    file.path
                ),
            ));
        }
    }
    let Some(entrypoint) = target.entrypoint.as_deref() else {
        return Ok(());
    };
    validate_runtime_path(entrypoint)?;
    if !paths.contains(entrypoint) {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("runtime entrypoint {entrypoint} is not declared in files"),
        ));
    }
    let expected_entrypoint = format!("bin/{name}/{}", host.executable_name());
    if entrypoint != expected_entrypoint {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("runtime entrypoint {entrypoint} != {expected_entrypoint}"),
        ));
    }
    Ok(())
}

/// Движок приезжает из тулчейна под своим тегом и своим именем.
///
/// Тег и имя назвал замок инструментов, и выводить их заново значит завести
/// второй источник правды. Проверяется то, что здесь и вправду известно:
/// происхождение адреса и то, что он кончается именно этим ассетом.
fn validate_engine_asset(artifact: &str, name: &str, target: &TargetRuntime) -> Result<()> {
    if target.asset.name.is_empty()
        || target.asset.name.contains('/')
        || target.asset.name.contains("..")
    {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("runtime asset name for {artifact} {name} is not a file name"),
        ));
    }
    let outside = || {
        BootstrapError::of(
            Failure::Configuration,
            format!(
                "runtime asset URL for {artifact} {name} is outside the approved release origin"
            ),
        )
    };
    let tail = target
        .asset
        .url
        .strip_prefix(ENGINE_RELEASE_ORIGIN)
        .ok_or_else(outside)?;
    let tag = tail
        .strip_suffix(&format!("/{}", target.asset.name))
        .ok_or_else(outside)?;
    if tag.is_empty() || tag.contains('/') || tag.contains("..") {
        return Err(outside());
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
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("unsafe runtime file path: {value}"),
        ));
    }
    Ok(())
}

fn validate_sha256(label: &str, value: &str) -> Result<()> {
    if !is_lower_hex(value, 64) {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("{label} sha256 must be 64 lowercase hexadecimal characters"),
        ));
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
