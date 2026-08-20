use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::error::{BootstrapError, Failure, Result};
use crate::platform::HostTarget;

const SOURCE_REPOSITORY: &str = "https://github.com/IngvarConsulting/unica";

/// Откуда приезжает ядро: оно собирается здесь и лежит в выпуске плагина.
const CORE_RELEASE_ORIGIN: &str = "https://github.com/IngvarConsulting/unica/releases/download/";

/// Откуда приезжает всё остальное. Тулчейн публикует поставки по цели, с
/// суммами и происхождением; копия тех же байтов в выпуске плагина стоила
/// 439 МБ на выпуск и не давала ничего.
///
/// Адресов ровно два, и оба названы. Третий — новая запись реестра, а не
/// правка этого списка: поартефактная проверка защищает от опечатки ровно
/// потому, что список закрыт.
const TOOLCHAIN_RELEASE_ORIGIN: &str =
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
    /// Ключ установки берётся из версии и суммы архива, поэтому выпуск плагина
    /// не объявляет холодными неизменившиеся байты, а новый toolchain build с
    /// прежней upstream-версией не подменяет старую установку.
    #[serde(default)]
    pub artifacts: BTreeMap<String, Artifact>,
}

/// Зачем артефакт нужен. Ядро едет в стартовом бюджете хоста, всё прочее —
/// нет: оно приезжает из тулчейна по требованию.
///
/// Перечень закрытый, потому что роль решает, что с байтами делать: движок
/// запускают, поставку конфигурации отдают платформе. Молча принять незнакомую
/// роль значит доставить неизвестно что и неизвестно зачем, поэтому новый вид
/// поставки — новая ветка здесь, а не отсутствие проверки.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactRole {
    Core,
    Engine,
}

impl ArtifactRole {
    /// Происхождение решает роль, а не имя: ядро собирается здесь, всё
    /// остальное приезжает из тулчейна.
    fn release_origin(self) -> &'static str {
        match self {
            Self::Core => CORE_RELEASE_ORIGIN,
            Self::Engine => TOOLCHAIN_RELEASE_ORIGIN,
        }
    }
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

/// Как артефакт приезжает.
///
/// Форма — про байты, а не про то, чем артефакт является: движок, расширение
/// поставки и внешняя обработка могут приехать любой из них. Определяется
/// типом содержимого, потому что его и объявляет издатель.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryForm {
    /// Архив: распаковывается, и набор файлов сверяется целиком.
    Archive,
    /// Один файл: кладётся под своим именем, сверяется суммой.
    File,
}

impl DeliveryForm {
    /// `None` — тип содержимого не описывает ни одной известной формы.
    pub fn of(media_type: &str) -> Option<Self> {
        match media_type {
            "application/gzip" => Some(Self::Archive),
            "application/octet-stream" => Some(Self::File),
            _ => None,
        }
    }
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

    /// Артефакт по имени: версия вместе с суммой цели ключует установку.
    pub fn artifact(&self, name: &str) -> Result<&Artifact> {
        self.artifacts.get(name).ok_or_else(|| {
            BootstrapError::of(
                Failure::Configuration,
                format!("runtime manifest has no artifact {name}"),
            )
        })
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
                    artifact.role,
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
        let entry = self.artifact(artifact)?;
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
    role: ArtifactRole,
    release_tag: &str,
    host: HostTarget,
    target: &TargetRuntime,
) -> Result<()> {
    let name = host.as_str();
    if role == ArtifactRole::Core {
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
        validate_toolchain_asset(artifact, role, name, target)?;
    }
    // Ядро несёт бинарь и его окружение: одним файлом оно не бывает.
    let form = match DeliveryForm::of(&target.asset.media_type) {
        Some(form) if role != ArtifactRole::Core || form == DeliveryForm::Archive => form,
        _ => {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!(
                    "runtime asset mediaType {} for {artifact} {name} is not a delivery form",
                    target.asset.media_type
                ),
            ))
        }
    };
    validate_sha256("runtime archive", &target.asset.sha256)?;

    if target.files.is_empty() {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!("runtime target {name} has no files"),
        ));
    }
    // Форма «один файл» ничего не распаковывает, поэтому перечислять больше
    // одного файла ей нечем.
    if form == DeliveryForm::File && target.files.len() != 1 {
        return Err(BootstrapError::of(
            Failure::Configuration,
            format!(
                "{artifact} {name} arrives as a single file but declares {} files",
                target.files.len()
            ),
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
    let entrypoint = match (role, target.entrypoint.as_deref()) {
        (ArtifactRole::Core, Some(entrypoint)) => entrypoint,
        (ArtifactRole::Core, None) => {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!("core entrypoint is missing for runtime target {name}"),
            ))
        }
        (ArtifactRole::Engine, Some(entrypoint)) => {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!("engine entrypoint is not allowed for {artifact} {name}: {entrypoint}"),
            ))
        }
        (ArtifactRole::Engine, None) => return Ok(()),
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

/// Поставка приезжает из тулчейна под своим тегом и своим именем.
///
/// Тег и имя назвал замок инструментов, и выводить их заново значит завести
/// второй источник правды. Проверяется то, что здесь и вправду известно:
/// происхождение адреса и то, что он кончается именно этим ассетом. Правило
/// одно на все виды поставки: расширению и обработке нового не понадобится.
fn validate_toolchain_asset(
    artifact: &str,
    role: ArtifactRole,
    name: &str,
    target: &TargetRuntime,
) -> Result<()> {
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
        .strip_prefix(role.release_origin())
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
