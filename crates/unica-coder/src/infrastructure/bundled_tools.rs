use crate::infrastructure::platform::current_target_id;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use unica_artifact_delivery::engine::BundledToolError;
use unica_artifact_delivery::engine::EngineStage;
use unica_artifact_delivery::engine::InstallMode;
use unica_artifact_delivery::engine::MissingEngine;

use unica_artifact_delivery::engine::BundledTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundledManifest {
    #[serde(default)]
    tools: Vec<ManifestTool>,
    /// Байтовая личность внешнего артефакта для текущей цели. В исходном
    /// checkout карта может отсутствовать: там бинарии собираются рядом с
    /// плагином и кеш доставки не используется.
    #[serde(default)]
    artifact_assets: BTreeMap<String, ManifestArtifactAsset>,
    target_triple: Option<String>,
    /// Плейсхолдер исходного чекаута: инструменты там собираются на месте.
    #[serde(default)]
    source_manifest: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestArtifactAsset {
    sha256: String,
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
    #[serde(default)]
    delivered_path: Option<String>,
    sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestBinary {
    target_triple: Option<String>,
    /// Где файл лежит в дереве плагина.
    binary_path: String,
    /// Где файл лежит внутри доставленного артефакта.
    ///
    /// Раскладку задаёт издатель поставки, а не плагин, и совпадать они не
    /// обязаны. Отсутствует, пока доставки не было: дерево разработки собирает
    /// инструменты на месте и раскладку выбирает само.
    #[serde(default)]
    delivered_path: Option<String>,
    sha256: String,
}

impl ManifestBinary {
    /// Путь внутри доставленного артефакта.
    fn delivered(&self) -> &str {
        self.delivered_path.as_deref().unwrap_or(&self.binary_path)
    }
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

/// Почему запись манифеста не дала бинарь: шаг поставки и текст для
/// предупреждения предпросмотра.
type ManifestBinaryError = (EngineStage, String);

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

/// Каталог, куда bootstrap ставит артефакты по имени и неизменяемой личности
/// поставки. В дереве разработки переменной нет, и поиск идёт прежним путём —
/// рядом с плагином.
const ARTIFACT_CACHE_ENV: &str = "UNICA_ARTIFACT_CACHE";

pub(crate) fn resolve_bundled_tool(
    plugin_root: &Path,
    tool_name: &str,
    verify: bool,
) -> Result<BundledTool, BundledToolError> {
    let Ok(target_id) = current_target_id() else {
        return Err(BundledToolError::missing(describe_missing(
            plugin_root,
            tool_name,
            &host_label(),
            EngineStage::Target,
            None,
        )));
    };
    let cache = std::env::var_os(ARTIFACT_CACHE_ENV).map(PathBuf::from);
    resolve_bundled_tool_in(plugin_root, cache.as_deref(), tool_name, target_id, verify)
}

/// Движок с полной проверкой поставки, включая контрольную сумму, — или
/// описание того, на каком шаге его не оказалось.
///
/// Нужна там, где запуск уже не удался, а причину надо назвать типом: хеш
/// считается только на этом пути, а не на каждом вызове.
pub(crate) fn engine_absence(plugin_root: &Path, tool_name: &str) -> Option<MissingEngine> {
    match resolve_bundled_tool(plugin_root, tool_name, true) {
        Err(BundledToolError::Missing(engine)) => Some(*engine),
        Ok(_) | Err(BundledToolError::Io(_)) => None,
    }
}

/// Движок проверен, но его процесс не запустился. Без корня плагина пин и
/// режим установки неизвестны: следующий шаг от них на этом шаге не зависит.
pub(crate) fn start_failure(
    plugin_root: Option<&Path>,
    tool_name: &str,
    program: &Path,
) -> MissingEngine {
    let target = current_target_id()
        .map(str::to_owned)
        .unwrap_or_else(|_| host_label());
    match plugin_root {
        Some(plugin_root) => describe_missing(
            plugin_root,
            tool_name,
            &target,
            EngineStage::Start,
            Some(program.to_path_buf()),
        ),
        None => MissingEngine::new(
            tool_name,
            target,
            program.display().to_string(),
            None,
            InstallMode::Marketplace,
        )
        .at_stage(EngineStage::Start),
    }
}

/// Причина незапуска движка для ответа: текст ОС без служебного префикса,
/// без пути к самому движку и после общей редакции секретов. Другие пути,
/// если ОС их назвала, редакция не обещает убрать.
pub(crate) fn launch_failure_cause(error: &str, tool_name: &str, program: &Path) -> String {
    let error = error.strip_prefix("process_failed: ").unwrap_or(error);
    unica_redaction::redactor(&error.replace(&program.display().to_string(), tool_name))
}

/// Проверенный движок, процесс которого ОС не создаёт: отказ на шаге `start`
/// с причиной ОС.
///
/// Поставщики поиска и графа запускают движок в службе рабочего
/// пространства, и до этого маршрута доходит только текст их ошибки. Тип
/// незапуска здесь получается тем же путём, что у раннера: движок
/// запускается напрямую, и решает [`ManagedSpawnFailure::Launch`], а не
/// текст. Запущенный процесс сразу завершается. Неготовое окружение или
/// неудачное подключение процесса к дереву — не незапуск движка (#549).
///
/// [`ManagedSpawnFailure::Launch`]: unica_process::ManagedSpawnFailure::Launch
pub(crate) fn launch_probe_failure(
    plugin_root: &Path,
    tool_name: &str,
) -> Option<(MissingEngine, String)> {
    use unica_process::{ManagedChild, ManagedCommand, ManagedSpawnFailure};
    let program = installed_engine_path(plugin_root, tool_name)?;
    let probe = ManagedChild::spawn_staged(ManagedCommand {
        program: program.clone(),
        args: vec!["--version".into()],
        cwd: plugin_root.to_path_buf(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: Some((4096, 4096)),
        timeout: Some(std::time::Duration::from_secs(1)),
        cancellation: unica_work_control::cancellation::CancellationToken::new(),
    });
    let message = match probe {
        Ok(mut child) => {
            let _ = child.terminate();
            return None;
        }
        Err(ManagedSpawnFailure::Launch(message)) => message,
        Err(ManagedSpawnFailure::Prepare(_) | ManagedSpawnFailure::Attach(_)) => return None,
    };
    Some((
        start_failure(Some(plugin_root), tool_name, &program),
        launch_failure_cause(&message, tool_name, &program),
    ))
}

/// Доставка движка не удалась, и файла на месте нет.
pub(crate) fn delivery_failure(plugin_root: &Path, tool_name: &str) -> MissingEngine {
    let target = current_target_id()
        .map(str::to_owned)
        .unwrap_or_else(|_| host_label());
    describe_missing(plugin_root, tool_name, &target, EngineStage::Delivery, None)
}

/// Корень плагина не найден: поставки, из которой взялся бы движок, нет.
pub(crate) fn plugin_root_missing(tool_name: &str) -> MissingEngine {
    let target = current_target_id()
        .map(str::to_owned)
        .unwrap_or_else(|_| host_label());
    MissingEngine::new(tool_name, target, "", None, InstallMode::Marketplace)
        .at_stage(EngineStage::Manifest)
}

/// Имя платформы, под которую Unica не поставляет движков.
fn host_label() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Разрешение с явно названным кешем артефактов: так его видно в тесте, а
/// переменная окружения остаётся деталью вызывающего.
fn resolve_bundled_tool_in(
    plugin_root: &Path,
    artifact_cache: Option<&Path>,
    tool_name: &str,
    target_id: &str,
    verify: bool,
) -> Result<BundledTool, BundledToolError> {
    if let Some(cache) = artifact_cache {
        if let Some(tool) =
            resolve_from_artifact_cache(plugin_root, cache, tool_name, target_id, verify)?
        {
            return Ok(tool);
        }
    }
    resolve_bundled_tool_for_target(plugin_root, tool_name, target_id, verify)
}

fn read_manifest(plugin_root: &Path) -> Option<BundledManifest> {
    let manifest_path = plugin_root.join("third-party").join("manifest.json");
    serde_json::from_slice(&fs::read(manifest_path).ok()?).ok()
}

/// Движок в кеше артефактов:
/// `<кеш>/<артефакт>/<версия--sha256-поставки>/<цель>/<путь>`.
/// Отсутствие установки — не ошибка: её ещё могут доставить, и решает это
/// вызывающий, а не резолвер.
fn resolve_from_artifact_cache(
    plugin_root: &Path,
    cache: &Path,
    tool_name: &str,
    target_id: &str,
    verify: bool,
) -> Result<Option<BundledTool>, BundledToolError> {
    let Ok(version) = bundled_tool_version(plugin_root, tool_name) else {
        return Ok(None);
    };
    let Some(manifest) = read_manifest(plugin_root) else {
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
    let manifest_defect = || {
        BundledToolError::missing(describe_missing(
            plugin_root,
            tool_name,
            target_id,
            EngineStage::Manifest,
            None,
        ))
    };
    let delivery =
        artifact_delivery_key(&manifest, &artifact, &version).map_err(|_| manifest_defect())?;
    let target_root = cache.join(&artifact).join(delivery).join(target_id);
    let program =
        manifest_relative_path(&target_root, binary.delivered()).map_err(|_| manifest_defect())?;
    if !program.is_file() {
        return Ok(None);
    }
    if verify {
        verify_binary(plugin_root, tool_name, target_id, &program, &binary.sha256)?;
    }
    Ok(Some(BundledTool {
        program,
        warnings: Vec::new(),
        missing: None,
    }))
}

fn artifact_delivery_key(
    manifest: &BundledManifest,
    artifact: &str,
    version: &str,
) -> Result<String, String> {
    let Some(asset) = manifest.artifact_assets.get(artifact) else {
        // Дерево разработки и старые тестовые фикстуры не являются релизной
        // доставкой. Их прежний путь сохраняется; опубликованный manifest
        // всегда несёт artifactAssets и получает неизменяемый ключ ниже.
        return Ok(version.to_string());
    };
    if asset.sha256.len() != 64
        || !asset
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "artifact {artifact} has invalid delivery sha256 in the Unica manifest"
        ));
    }
    Ok(format!("{version}--{}", asset.sha256))
}

/// Имя архива, в котором приезжает инструмент. Несколько инструментов делят
/// один: `rlm-tools-bsl` несёт два.
pub(crate) fn artifact_for(plugin_root: &Path, tool_name: &str) -> Option<String> {
    let manifest = read_manifest(plugin_root)?;
    let tool = manifest.tools.iter().find(|tool| tool.name == tool_name)?;
    Some(
        tool.artifact
            .clone()
            .unwrap_or_else(|| tool_name.to_string()),
    )
}

/// Лежит ли движок на диске.
///
/// Проверяется наличие файла, а не его сумма: сумму сверит запуск, а доставке
/// довольно знать, чего ещё нет. Считать хеш десятков мегабайт на каждый вызов
/// значит платить за ответ, который уже дала файловая система.
pub(crate) fn installed_engine_path(plugin_root: &Path, tool_name: &str) -> Option<PathBuf> {
    engine_path_for(plugin_root, tool_name, current_target_id().ok()?)
}

fn engine_path_for(plugin_root: &Path, tool_name: &str, target_id: &str) -> Option<PathBuf> {
    if let Some(cache) = std::env::var_os(ARTIFACT_CACHE_ENV).map(PathBuf::from) {
        if let Ok(Some(tool)) =
            resolve_from_artifact_cache(plugin_root, &cache, tool_name, target_id, false)
        {
            return Some(tool.program);
        }
    }
    let manifest = read_manifest(plugin_root)?;
    let binary = manifest_binary(&manifest, tool_name, target_id).ok()?;
    let program = manifest_relative_path(plugin_root, &binary.binary_path).ok()?;
    program.is_file().then_some(program)
}

/// Чего не хватает инструменту, чтобы запуститься. `None` — движок на месте.
///
/// Собирается из того, что уже известно поставке: имя, цель, ожидаемый путь,
/// пин версии, режим установки и шаг, на котором движка нет. Разбирать текст
/// сообщения вызывающему не придётся — поля названы, а следующий шаг зависит
/// от режима: исходный чекаут собирает инструменты сам, опубликованная
/// поставка их доставляет. Контрольная сумма здесь не считается: её сверяет
/// [`engine_absence`] на пути отказа.
pub(crate) fn missing_engine(plugin_root: &Path, tool_name: &str) -> Option<MissingEngine> {
    let Ok(target_id) = current_target_id() else {
        return Some(describe_missing(
            plugin_root,
            tool_name,
            &host_label(),
            EngineStage::Target,
            None,
        ));
    };
    missing_engine_for(plugin_root, tool_name, target_id)
}

/// Описать отсутствие для той же цели, которую разрешал вызывающий.
fn missing_engine_for(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
) -> Option<MissingEngine> {
    if engine_path_for(plugin_root, tool_name, target_id).is_some() {
        return None;
    }
    let stage = match read_manifest(plugin_root) {
        None => EngineStage::Manifest,
        Some(manifest) => match manifest_binary(&manifest, tool_name, target_id) {
            Ok(_) => EngineStage::Delivery,
            Err((stage, _)) => stage,
        },
    };
    Some(describe_missing(
        plugin_root,
        tool_name,
        target_id,
        stage,
        None,
    ))
}

/// Режим установки: исходный checkout собирает движки сам, опубликованная
/// поставка их доставляет.
///
/// Манифест называет режим сам. Если его нет или он не читается, режим не
/// выводится из этого: опубликованная поставка с испорченным манифестом не
/// должна получать совет собрать движок из исходников. Исходным checkout
/// считается только дерево, где рядом с плагином лежит сборщик движков.
fn install_mode_of(plugin_root: &Path) -> InstallMode {
    match read_manifest(plugin_root) {
        Some(manifest) if manifest.source_manifest => InstallMode::Source,
        Some(_) => InstallMode::Marketplace,
        None if source_checkout_builder(plugin_root).is_file() => InstallMode::Source,
        None => InstallMode::Marketplace,
    }
}

/// Сборщик движков исходного checkout: `<repo>/scripts/ci/build-unica-tools.py`
/// для плагина в `<repo>/plugins/unica`.
fn source_checkout_builder(plugin_root: &Path) -> PathBuf {
    plugin_root
        .join("..")
        .join("..")
        .join("scripts")
        .join("ci")
        .join("build-unica-tools.py")
}

/// Описание движка, которого нет на шаге `stage`. Путь, если не назван, —
/// тот, где движок оказался бы, если бы приехал.
fn describe_missing(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
    stage: EngineStage,
    path: Option<PathBuf>,
) -> MissingEngine {
    let install_mode = install_mode_of(plugin_root);
    let path = path
        .or_else(|| expected_engine_path(plugin_root, tool_name, target_id))
        .unwrap_or_else(|| plugin_root.join("bin").join(target_id).join(tool_name));
    MissingEngine::new(
        tool_name,
        target_id,
        path.display().to_string(),
        bundled_tool_version(plugin_root, tool_name).ok(),
        install_mode,
    )
    .at_stage(stage)
}

/// Где движок оказался бы, если бы приехал.
fn expected_engine_path(plugin_root: &Path, tool_name: &str, target_id: &str) -> Option<PathBuf> {
    if let (Some(cache), Ok(version), Some(artifact)) = (
        std::env::var_os(ARTIFACT_CACHE_ENV).map(PathBuf::from),
        bundled_tool_version(plugin_root, tool_name),
        artifact_for(plugin_root, tool_name),
    ) {
        if let Some((binary, delivery)) = read_manifest(plugin_root).and_then(|manifest| {
            let binary = manifest_binary(&manifest, tool_name, target_id).ok()?;
            let delivery = artifact_delivery_key(&manifest, &artifact, &version).ok()?;
            Some((binary, delivery))
        }) {
            let target_root = cache.join(artifact).join(delivery).join(target_id);
            return manifest_relative_path(&target_root, binary.delivered()).ok();
        }
    }
    // Манифест называет файл движка для цели сам — тот же путь, по которому
    // его ищет запуск (`engine_path_for`). Имя из `tools.lock.json` и голое
    // имя инструмента ниже — только запасной ответ без манифеста: на Windows
    // у них нет суффикса `.exe`, и отказ назвал бы не тот файл.
    if let Some(program) = read_manifest(plugin_root).and_then(|manifest| {
        let binary = manifest_binary(&manifest, tool_name, target_id).ok()?;
        manifest_relative_path(plugin_root, &binary.binary_path).ok()
    }) {
        return Some(program);
    }
    let lock_path = plugin_root.join("third-party").join("tools.lock.json");
    let lock: ToolsLock = serde_json::from_slice(&fs::read(&lock_path).ok()?).ok()?;
    let target = lock.targets.get(target_id)?;
    let tool = lock.tools.iter().find(|tool| tool.name == tool_name)?;
    Some(
        plugin_root
            .join("bin")
            .join(target_id)
            .join(format!("{}{}", tool.binary_name, target.exe)),
    )
}

fn resolve_bundled_tool_for_target(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
    verify: bool,
) -> Result<BundledTool, BundledToolError> {
    let absent = |stage: EngineStage| {
        BundledToolError::missing(describe_missing(
            plugin_root,
            tool_name,
            target_id,
            stage,
            None,
        ))
    };
    let manifest_path = plugin_root.join("third-party").join("manifest.json");
    if !manifest_path.is_file() {
        if verify {
            return Err(absent(EngineStage::Manifest));
        }
        let reason = format!(
            "Unica third-party manifest not found: {}",
            manifest_path.display()
        );
        return resolve_from_lock_for_dry_run(plugin_root, tool_name, target_id, &reason);
    }

    // Манифест на месте, но не читается: поставка испорчена, и движка для
    // вызывающего нет так же, как без манифеста.
    let manifest: BundledManifest = fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| absent(EngineStage::Manifest))?;

    let binary = match manifest_binary(&manifest, tool_name, target_id) {
        Ok(binary) => binary,
        Err((_, reason)) if !verify => {
            return resolve_from_lock_for_dry_run(plugin_root, tool_name, target_id, &reason);
        }
        Err((stage, _)) => return Err(absent(stage)),
    };

    let program = manifest_relative_path(plugin_root, &binary.binary_path)
        .map_err(|_| absent(EngineStage::Manifest))?;
    let mut warnings = Vec::new();
    let mut missing = None;
    if verify {
        verify_binary(plugin_root, tool_name, target_id, &program, &binary.sha256)?;
    } else if !program.is_file() {
        warnings.push(format!(
            "dry run: bundled tool binary is not present yet: {}",
            program.display()
        ));
        missing = missing_engine_for(plugin_root, tool_name, target_id);
    }
    Ok(BundledTool {
        program,
        warnings,
        missing,
    })
}

fn manifest_binary(
    manifest: &BundledManifest,
    tool_name: &str,
    target_id: &str,
) -> Result<ManifestBinary, ManifestBinaryError> {
    let tool = manifest
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .ok_or_else(|| {
            (
                EngineStage::Manifest,
                format!("tool not found in manifest: {tool_name}"),
            )
        })?;

    if let Some(binaries) = &tool.binaries {
        let binary = binaries.get(target_id).ok_or_else(|| {
            let supported = binaries.keys().cloned().collect::<Vec<_>>().join(", ");
            (
                EngineStage::Target,
                format!("tool {tool_name} is not packaged for {target_id}; supported: {supported}"),
            )
        })?;
        if let Some(target_triple) = &binary.target_triple {
            let expected =
                target_triple_for_id(target_id).map_err(|error| (EngineStage::Target, error))?;
            if target_triple != expected {
                return Err((
                    EngineStage::Target,
                    format!(
                        "tool {tool_name} manifest target triple mismatch for {target_id}: {target_triple} != {expected}"
                    ),
                ));
            }
        }
        return Ok(binary.clone());
    }

    if let Some(target_triple) = &manifest.target_triple {
        let expected =
            target_triple_for_id(target_id).map_err(|error| (EngineStage::Target, error))?;
        if target_triple != expected {
            return Err((
                EngineStage::Target,
                format!("Unica ships binaries for {target_triple}; current host is {expected}."),
            ));
        }
    }
    Ok(ManifestBinary {
        target_triple: manifest.target_triple.clone(),
        binary_path: tool.binary_path.clone().ok_or_else(|| {
            (
                EngineStage::Manifest,
                format!("tool {tool_name} is missing binaryPath in third-party manifest"),
            )
        })?,
        delivered_path: tool.delivered_path.clone(),
        sha256: tool.sha256.clone().ok_or_else(|| {
            (
                EngineStage::Manifest,
                format!("tool {tool_name} is missing sha256 in third-party manifest"),
            )
        })?,
    })
}

/// Предпросмотр без манифеста строит путь по `tools.lock.json`. Если и пина
/// для цели нет, движка не описывает ни один источник поставки.
fn resolve_from_lock_for_dry_run(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
    reason: &str,
) -> Result<BundledTool, BundledToolError> {
    let lock_path = plugin_root.join("third-party").join("tools.lock.json");
    let absent = |stage: EngineStage| {
        BundledToolError::missing(describe_missing(
            plugin_root,
            tool_name,
            target_id,
            stage,
            None,
        ))
    };
    let lock: ToolsLock = fs::read_to_string(&lock_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| absent(EngineStage::Manifest))?;
    let target = lock
        .targets
        .get(target_id)
        .ok_or_else(|| absent(EngineStage::Target))?;
    let tool = lock
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .ok_or_else(|| absent(EngineStage::Manifest))?;
    if !tool.assets.contains_key(target_id) {
        return Err(absent(EngineStage::Target));
    }

    Ok(BundledTool {
        program: plugin_root
            .join("bin")
            .join(target_id)
            .join(format!("{}{}", tool.binary_name, target.exe)),
        warnings: vec![format!(
            "dry run: {reason}; using expected bundled binary path from tools.lock.json"
        )],
        missing: missing_engine_for(plugin_root, tool_name, target_id),
    })
}

fn verify_binary(
    plugin_root: &Path,
    tool_name: &str,
    target_id: &str,
    program: &Path,
    expected_sha: &str,
) -> Result<(), BundledToolError> {
    let absent = |stage: EngineStage| {
        BundledToolError::missing(describe_missing(
            plugin_root,
            tool_name,
            target_id,
            stage,
            Some(program.to_path_buf()),
        ))
    };
    if !program.is_file() {
        return Err(absent(EngineStage::Delivery));
    }
    let actual = sha256_file(program).map_err(BundledToolError::Io)?;
    if !actual.eq_ignore_ascii_case(expected_sha) {
        return Err(absent(EngineStage::Checksum));
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

        assert_eq!(stage_of(&error), Some(EngineStage::Checksum), "{error}");
    }

    #[test]
    fn an_engine_is_taken_from_the_artifact_cache_when_one_is_named() {
        // Разрез поставки уносит движки из каталога плагина: они лежат в кеше
        // под личностью поставки, и рантайм обязан находить их там.
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

    pub(super) fn stage_of(error: &BundledToolError) -> Option<EngineStage> {
        match error {
            BundledToolError::Missing(engine) => Some(engine.stage),
            BundledToolError::Io(_) => None,
        }
    }

    pub(super) fn temp_plugin_root(name: &str) -> PathBuf {
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

    /// Цель, на которой идёт прогон. Резолвер движка смотрит на неё, а не на
    /// ту, что назвал тест, поэтому фикстура обязана её покрывать: иначе набор
    /// зелен только на машине автора.
    pub(super) fn host_target() -> &'static str {
        crate::infrastructure::platform::current_target_id().expect("цель поддержана")
    }

    /// Путь бинаря инструмента для цели прогона.
    pub(super) fn host_binary(tool: &str) -> String {
        let target = host_target();
        let exe = if target == "win-x64" { ".exe" } else { "" };
        format!("bin/{target}/{tool}{exe}")
    }

    pub(super) fn write_manifest_with_bsl_analyzer(plugin_root: &Path) {
        fs::create_dir_all(plugin_root.join("bin/win-x64")).unwrap();
        fs::create_dir_all(plugin_root.join("bin/darwin-arm64")).unwrap();
        fs::create_dir_all(plugin_root.join("bin/linux-x64")).unwrap();
        fs::write(
            plugin_root.join("bin/linux-x64/bsl-analyzer"),
            "linux-binary",
        )
        .unwrap();
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
        },
        "linux-x64": {
          "targetTriple": "x86_64-unknown-linux-gnu",
          "binaryPath": "bin/linux-x64/bsl-analyzer",
          "sha256": "8e05650e3597d536838387f4fb4f08fbd95624760f1ce44bbff4e35de8a353e8"
        }
      }
    }
  ]
}"#,
        )
        .unwrap();
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use std::fs;

    #[test]
    fn installer_and_runtime_resolve_both_tools_from_one_immutable_rlm_delivery() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use unica_artifact_delivery::{
            BootstrapError, DownloadObserver, Downloader, HostTarget, RuntimeInstaller,
            RuntimeManifest, SilentDownload,
        };

        struct PublishedAssets {
            archives: BTreeMap<String, Vec<u8>>,
            calls: AtomicUsize,
        }

        impl Downloader for PublishedAssets {
            fn download(
                &self,
                url: &str,
                destination: &Path,
                observer: &dyn DownloadObserver,
            ) -> Result<(), BootstrapError> {
                let name = url.rsplit('/').next().unwrap();
                let bytes = &self.archives[name];
                fs::write(destination, bytes)?;
                observer.transferred(bytes.len() as u64, Some(bytes.len() as u64));
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
            let encoder = GzEncoder::new(Vec::new(), Compression::default());
            let mut builder = tar::Builder::new(encoder);
            for (name, bytes) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, name, *bytes).unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap()
        }

        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugin");
        fs::create_dir_all(plugin.join("third-party")).unwrap();
        let cache = root.path().join("cache");
        let script = b"#!/bin/sh\nprintf 'provider-started\\n'\n";
        let file_sha = format!("{:x}", Sha256::digest(script));
        let mut archives = BTreeMap::new();
        let mut engine_targets = serde_json::Map::new();
        let mut analyzer_targets = serde_json::Map::new();
        let mut core_targets = serde_json::Map::new();
        let mut mcp_binaries = serde_json::Map::new();
        let mut index_binaries = serde_json::Map::new();
        let mut analyzer_binaries = serde_json::Map::new();
        for host in HostTarget::ALL {
            let target = host.as_str();
            let suffix = if target == "win-x64" { ".exe" } else { "" };
            let mcp = format!("rlm-bsl-mcp{suffix}");
            let index = format!("rlm-bsl-index{suffix}");
            let bytes = archive(&[(mcp.as_str(), script), (index.as_str(), script)]);
            let asset_name = format!("rlm-tools-bsl-{target}.tar.gz");
            let asset_sha = format!("{:x}", Sha256::digest(&bytes));
            archives.insert(asset_name.clone(), bytes);
            let analyzer_name = format!("bsl-analyzer-{target}{suffix}");
            archives.insert(analyzer_name.clone(), script.to_vec());
            analyzer_targets.insert(target.to_owned(), serde_json::json!({
                "asset": {
                    "name": analyzer_name,
                    "url": format!("https://github.com/IngvarConsulting/unica-toolchain/releases/download/bsl-analyzer-v0.2.67-build.1/{analyzer_name}"),
                    "mediaType": "application/octet-stream",
                    "sha256": file_sha,
                },
                "files": [{"path": format!("bin/{target}/bsl-analyzer{suffix}"), "sha256": file_sha, "executable": true}],
            }));
            analyzer_binaries.insert(
                target.to_owned(),
                serde_json::json!({
                    "binaryPath": format!("bin/{target}/bsl-analyzer{suffix}"),
                    "deliveredPath": format!("bin/{target}/bsl-analyzer{suffix}"),
                    "sha256": file_sha,
                }),
            );
            engine_targets.insert(target.to_owned(), serde_json::json!({
                "asset": {
                    "name": asset_name,
                    "url": format!("https://github.com/IngvarConsulting/unica-toolchain/releases/download/rlm-tools-bsl-v1.33.0-build.3/{asset_name}"),
                    "mediaType": "application/gzip",
                    "sha256": asset_sha,
                },
                "files": [
                    {"path": mcp, "sha256": file_sha, "executable": true},
                    {"path": index, "sha256": file_sha, "executable": true},
                ],
            }));
            let core_exe = if target == "win-x64" {
                "unica.exe"
            } else {
                "unica"
            };
            let core_path = format!("bin/{target}/{core_exe}");
            let core_asset = format!("unica-runtime-{target}.tar.gz");
            core_targets.insert(target.to_owned(), serde_json::json!({
                "asset": {
                    "name": core_asset,
                    "url": format!("https://github.com/IngvarConsulting/unica/releases/download/v{}/{core_asset}", env!("CARGO_PKG_VERSION")),
                    "mediaType": "application/gzip",
                    "sha256": "0".repeat(64),
                },
                "files": [{"path": core_path, "sha256": "0".repeat(64), "executable": true}],
                "entrypoint": core_path,
            }));
            for (binaries, name) in [(&mut mcp_binaries, mcp), (&mut index_binaries, index)] {
                binaries.insert(
                    target.to_owned(),
                    serde_json::json!({
                        "binaryPath": format!("bin/{target}/{name}"),
                        "deliveredPath": name,
                        "sha256": file_sha,
                    }),
                );
            }
        }

        let runtime: RuntimeManifest = serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "pluginVersion": env!("CARGO_PKG_VERSION"),
            "source": {
                "repository": "https://github.com/IngvarConsulting/unica",
                "commit": "0".repeat(40),
            },
            "release": {
                "repository": "https://github.com/IngvarConsulting/unica",
                "tag": format!("v{}", env!("CARGO_PKG_VERSION")),
            },
            "artifacts": {
                "unica": {"version": env!("CARGO_PKG_VERSION"), "role": "core", "targets": core_targets},
                "rlm-tools-bsl": {"version": "1.33.0", "role": "engine", "targets": engine_targets},
                "bsl-analyzer": {"version": "0.2.67", "role": "engine", "targets": analyzer_targets},
            },
        }))
        .unwrap();
        runtime.validate(env!("CARGO_PKG_VERSION")).unwrap();
        let downloader = Arc::new(PublishedAssets {
            archives,
            calls: AtomicUsize::new(0),
        });
        let installer =
            RuntimeInstaller::new(cache.clone(), env!("CARGO_PKG_VERSION"), downloader.clone());

        for host in HostTarget::ALL {
            let target = host.as_str();
            let asset = &runtime
                .artifact_target("rlm-tools-bsl", host)
                .unwrap()
                .asset;
            let analyzer_asset = &runtime.artifact_target("bsl-analyzer", host).unwrap().asset;
            let plugin_manifest = serde_json::json!({
                "schemaVersion": 2,
                "artifactAssets": {"rlm-tools-bsl": asset, "bsl-analyzer": analyzer_asset},
                "tools": [
                    {"name": "rlm-bsl-index", "artifact": "rlm-tools-bsl", "version": "1.33.0", "binaries": index_binaries},
                    {"name": "rlm-bsl-mcp", "artifact": "rlm-tools-bsl", "version": "1.33.0", "binaries": mcp_binaries},
                    {"name": "bsl-analyzer", "version": "0.2.67", "binaries": analyzer_binaries},
                ],
            });
            fs::write(
                plugin.join("third-party/manifest.json"),
                plugin_manifest.to_string(),
            )
            .unwrap();
            let installed = installer
                .ensure_artifact(&runtime, "rlm-tools-bsl", host, &SilentDownload)
                .unwrap();
            assert_eq!(
                installed,
                cache
                    .join("rlm-tools-bsl")
                    .join(format!("1.33.0--{}", asset.sha256))
                    .join(target)
            );
            for tool in ["rlm-bsl-index", "rlm-bsl-mcp"] {
                let resolved = resolve_from_artifact_cache(&plugin, &cache, tool, target, true)
                    .unwrap()
                    .expect("installed tool is resolved");
                assert!(resolved.program.starts_with(&installed));
                assert_eq!(fs::read(&resolved.program).unwrap(), script);
                if target != "win-x64"
                    && target == crate::infrastructure::platform::current_target_id().unwrap()
                    && tool == "rlm-bsl-mcp"
                {
                    let output = std::process::Command::new(&resolved.program)
                        .output()
                        .unwrap();
                    assert!(output.status.success());
                    assert_eq!(output.stdout, b"provider-started\n");
                }
            }
            installer
                .ensure_artifact(&runtime, "rlm-tools-bsl", host, &SilentDownload)
                .unwrap();
            let analyzer_root = installer
                .ensure_artifact(&runtime, "bsl-analyzer", host, &SilentDownload)
                .unwrap();
            assert_eq!(
                analyzer_root,
                cache
                    .join("bsl-analyzer")
                    .join(format!("0.2.67--{}", analyzer_asset.sha256))
                    .join(target)
            );
            let analyzer =
                resolve_from_artifact_cache(&plugin, &cache, "bsl-analyzer", target, true)
                    .unwrap()
                    .expect("installed bsl-analyzer is resolved");
            assert!(analyzer.program.starts_with(&analyzer_root));
            assert_eq!(fs::read(&analyzer.program).unwrap(), script);
            installer
                .ensure_artifact(&runtime, "bsl-analyzer", host, &SilentDownload)
                .unwrap();
        }
        assert_eq!(downloader.calls.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn artifact_cache_resolution_uses_the_delivery_digest_not_only_the_version() {
        let plugin_root = tests::temp_plugin_root("immutable-cache-identity");
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "artifactAssets": {
    "bsl-analyzer": {
      "repository": "https://github.com/IngvarConsulting/unica-toolchain",
      "tag": "bsl-analyzer-v0.2.67-build.2",
      "name": "bsl-analyzer-darwin-arm64",
      "mediaType": "application/octet-stream",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }
  },
  "tools": [{
    "name": "bsl-analyzer",
    "version": "0.2.67",
    "binaries": {
      "darwin-arm64": {
        "targetTriple": "aarch64-apple-darwin",
        "binaryPath": "bin/darwin-arm64/bsl-analyzer",
        "deliveredPath": "bsl-analyzer-darwin-arm64",
        "sha256": "e4002e1adb76d4e2bb4846ab27463ff6368d18b727eb2bd519e1579f0baf491b"
      }
    }
  }]
}"#,
        )
        .unwrap();
        let cache = plugin_root.join("..").join("immutable-cache");
        let stale = cache.join("bsl-analyzer/0.2.67/darwin-arm64/bsl-analyzer-darwin-arm64");
        let current = cache.join(
            "bsl-analyzer/0.2.67--aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/darwin-arm64/bsl-analyzer-darwin-arm64",
        );
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        fs::write(&stale, "old-build").unwrap();
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(&current, "current-build").unwrap();

        let found = resolve_from_artifact_cache(
            &plugin_root,
            &cache,
            "bsl-analyzer",
            "darwin-arm64",
            false,
        )
        .unwrap()
        .expect("current delivery is installed");

        assert_eq!(found.program, current);
    }

    #[test]
    fn an_engine_in_the_artifact_cache_counts_as_installed() {
        let plugin_root = tests::temp_plugin_root("installed-in-cache");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        let cache = plugin_root.join("..").join("installed-cache");
        let installed = cache
            .join("bsl-analyzer")
            .join("test")
            .join("darwin-arm64")
            .join("bin/darwin-arm64");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("bsl-analyzer"), "darwin-binary").unwrap();

        let found = resolve_from_artifact_cache(
            &plugin_root,
            cache.as_path(),
            "bsl-analyzer",
            "darwin-arm64",
            false,
        )
        .unwrap();

        assert_eq!(
            found.map(|tool| tool.program),
            Some(installed.join("bsl-analyzer"))
        );
    }

    #[test]
    fn a_delivered_engine_lies_where_the_toolchain_packed_it() {
        // Тулчейн издаёт архив своей раскладкой, и переупаковка ради раскладки
        // плагина вернула бы ту самую копию, от которой ушли. Кеш повторяет
        // архив, а не дерево разработки.
        let plugin_root = tests::temp_plugin_root("delivered-layout");
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "tools": [
    {
      "name": "rlm-bsl-mcp",
      "version": "1.33.0",
      "artifact": "rlm-tools-bsl",
      "binaries": {
        "darwin-arm64": {
          "targetTriple": "aarch64-apple-darwin",
          "binaryPath": "bin/darwin-arm64/rlm-bsl-mcp",
          "deliveredPath": "rlm-bsl-mcp",
          "sha256": "e4002e1adb76d4e2bb4846ab27463ff6368d18b727eb2bd519e1579f0baf491b"
        }
      }
    }
  ]
}"#,
        )
        .unwrap();
        let cache = plugin_root.join("..").join("delivered-cache");
        let installed = cache
            .join("rlm-tools-bsl")
            .join("1.33.0")
            .join("darwin-arm64");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("rlm-bsl-mcp"), "engine").unwrap();

        let found = resolve_from_artifact_cache(
            &plugin_root,
            cache.as_path(),
            "rlm-bsl-mcp",
            "darwin-arm64",
            false,
        )
        .unwrap();

        assert_eq!(
            found.map(|tool| tool.program),
            Some(installed.join("rlm-bsl-mcp"))
        );
    }

    #[test]
    fn a_delivered_path_cannot_escape_the_verified_cache_root() {
        let plugin_root = tests::temp_plugin_root("delivered-path-traversal");
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "tools": [{
    "name": "bsl-analyzer",
    "version": "0.2.67",
    "binaries": {
      "darwin-arm64": {
        "targetTriple": "aarch64-apple-darwin",
        "binaryPath": "bin/darwin-arm64/bsl-analyzer",
        "deliveredPath": "../../outside-engine",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      }
    }
  }]
}"#,
        )
        .unwrap();
        let cache = plugin_root.join("..").join("delivered-path-cache");
        let target_root = cache.join("bsl-analyzer/0.2.67/darwin-arm64");
        fs::create_dir_all(&target_root).unwrap();
        fs::write(cache.join("bsl-analyzer/outside-engine"), b"outside").unwrap();

        let error = resolve_from_artifact_cache(
            &plugin_root,
            &cache,
            "bsl-analyzer",
            "darwin-arm64",
            false,
        )
        .expect_err("deliveredPath traversal must fail closed");

        assert_eq!(
            tests::stage_of(&error),
            Some(EngineStage::Manifest),
            "{error}"
        );
    }

    #[test]
    fn a_tree_without_a_delivered_path_keeps_the_path_it_has() {
        // Дерево разработки собирает инструменты на месте: доставки не было,
        // и раскладка остаётся его собственной.
        let plugin_root = tests::temp_plugin_root("no-delivered-path");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        let cache = plugin_root.join("..").join("legacy-cache");
        let installed = cache
            .join("bsl-analyzer")
            .join("test")
            .join("darwin-arm64")
            .join("bin/darwin-arm64");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("bsl-analyzer"), "darwin-binary").unwrap();

        let found = resolve_from_artifact_cache(
            &plugin_root,
            cache.as_path(),
            "bsl-analyzer",
            "darwin-arm64",
            false,
        )
        .unwrap();

        assert_eq!(
            found.map(|tool| tool.program),
            Some(installed.join("bsl-analyzer"))
        );
    }

    #[test]
    fn an_engine_that_never_arrived_is_not_installed() {
        let plugin_root = tests::temp_plugin_root("never-arrived");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        let cache = plugin_root.join("..").join("empty-cache");

        let found = resolve_from_artifact_cache(
            &plugin_root,
            cache.as_path(),
            "bsl-analyzer",
            "darwin-arm64",
            false,
        )
        .unwrap();

        assert!(found.is_none(), "чего нет на диске, то и не установлено");
    }

    #[test]
    fn tools_sharing_one_archive_name_one_artifact() {
        let plugin_root = tests::temp_plugin_root("artifact-name");
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "tools": [
    {"name": "rlm-bsl-index", "version": "1.33.0", "artifact": "rlm-tools-bsl"},
    {"name": "rlm-bsl-mcp", "version": "1.33.0", "artifact": "rlm-tools-bsl"},
    {"name": "v8-runner", "version": "0.4.0"}
  ]
}"#,
        )
        .unwrap();

        assert_eq!(
            artifact_for(&plugin_root, "rlm-bsl-index").as_deref(),
            Some("rlm-tools-bsl")
        );
        assert_eq!(
            artifact_for(&plugin_root, "rlm-bsl-mcp").as_deref(),
            Some("rlm-tools-bsl")
        );
        assert_eq!(
            artifact_for(&plugin_root, "v8-runner").as_deref(),
            Some("v8-runner"),
            "без имени архива артефакт зовётся как инструмент"
        );
        assert_eq!(artifact_for(&plugin_root, "unknown"), None);
    }
}

#[cfg(test)]
mod missing_engine_tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_source_checkout_is_told_to_build_the_tool() {
        // Плейсхолдер исходного чекаута инструментов не описывает, и доставке
        // взяться неоткуда: следующий шаг — собрать их на месте.
        let plugin_root = tests::temp_plugin_root("source-mode");
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{"schemaVersion":2,"sourceManifest":true,"tools":[]}"#,
        )
        .unwrap();
        fs::write(
            plugin_root.join("third-party/tools.lock.json"),
            r#"{
  "schemaVersion": 1,
  "targets": {"darwin-arm64": {"targetTriple": "aarch64-apple-darwin", "exe": ""},
              "linux-x64": {"targetTriple": "x86_64-unknown-linux-gnu", "exe": ""},
              "win-x64": {"targetTriple": "x86_64-pc-windows-msvc", "exe": ".exe"}},
  "tools": [{"name": "v8-runner", "version": "0.5.1", "binaryName": "v8-runner",
             "assets": {"darwin-arm64": {"assetName": "v8-runner"},
                        "linux-x64": {"assetName": "v8-runner"},
                        "win-x64": {"assetName": "v8-runner.exe"}}}]
}"#,
        )
        .unwrap();

        let missing = missing_engine(&plugin_root, "v8-runner").expect("движка нет");

        assert_eq!(
            missing.code,
            unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING
        );
        assert_eq!(missing.tool, "v8-runner");
        assert_eq!(missing.install_mode, InstallMode::Source);
        assert_eq!(missing.pinned_version.as_deref(), Some("0.5.1"));
        assert_eq!(
            missing.stage,
            EngineStage::Manifest,
            "плейсхолдер исходного чекаута движка не описывает"
        );
        assert!(
            missing
                .expected_path
                .replace('\\', "/")
                .ends_with(&tests::host_binary("v8-runner")),
            "путь назван: {}",
            missing.expected_path
        );
        assert!(
            missing.next_step.contains("build-unica-tools.py"),
            "следующий шаг назван: {}",
            missing.next_step
        );
    }

    #[test]
    fn a_published_install_is_told_the_tool_will_be_delivered() {
        let plugin_root = tests::temp_plugin_root("marketplace-mode");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        fs::remove_file(plugin_root.join(tests::host_binary("bsl-analyzer"))).unwrap();

        let missing = missing_engine(&plugin_root, "bsl-analyzer").expect("движка нет");

        assert_eq!(missing.install_mode, InstallMode::Marketplace);
        assert!(
            missing.next_step.contains("delivered"),
            "следующий шаг назван: {}",
            missing.next_step
        );
    }

    #[test]
    fn the_fixture_covers_every_target_the_product_ships() {
        // Резолвер движка смотрит на цель прогона. Фикстура на две цели из трёх
        // делала набор зелёным только на машине автора: три теста прошли на
        // macOS и упали на Linux и Windows в первом же прогоне CI.
        let plugin_root = tests::temp_plugin_root("fixture-targets");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        let manifest: BundledManifest = serde_json::from_slice(
            &std::fs::read(plugin_root.join("third-party/manifest.json")).unwrap(),
        )
        .unwrap();

        let binaries = manifest.tools[0].binaries.as_ref().expect("цели объявлены");

        for target in ["darwin-arm64", "linux-x64", "win-x64"] {
            let declared = binaries.get(target).expect(target);
            assert!(
                plugin_root.join(&declared.binary_path).is_file(),
                "объявлен, но не создан: {}",
                declared.binary_path
            );
        }
        assert!(binaries.contains_key(tests::host_target()));
    }

    #[test]
    fn an_engine_on_disk_is_not_missing() {
        let plugin_root = tests::temp_plugin_root("present");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);

        assert_eq!(missing_engine(&plugin_root, "bsl-analyzer"), None);
    }
}

#[cfg(test)]
mod missing_binary_refusal_tests {
    use super::*;
    use std::fs;

    fn missing(error: BundledToolError) -> MissingEngine {
        match error {
            BundledToolError::Missing(engine) => *engine,
            BundledToolError::Io(message) => panic!("expected a typed absence, got I/O: {message}"),
        }
    }

    #[test]
    fn a_missing_binary_is_refused_with_a_typed_delivery_stage() {
        // #549: «Unica binary is missing: <path>» не машиночитаемо и не
        // подсказывает действие. Отказ называет инструмент, цель, путь, пин,
        // шаг поставки и следующий шаг полями, а не фразой.
        let plugin_root = tests::temp_plugin_root("missing-code");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        let target = tests::host_target();
        let binary = plugin_root.join(tests::host_binary("bsl-analyzer"));
        fs::remove_file(&binary).unwrap();

        let engine = missing(
            resolve_bundled_tool_for_target(&plugin_root, "bsl-analyzer", target, true)
                .unwrap_err(),
        );

        assert_eq!(engine.tool, "bsl-analyzer");
        assert_eq!(engine.target, target);
        assert_eq!(engine.stage, EngineStage::Delivery);
        assert_eq!(engine.pinned_version.as_deref(), Some("test"));
        assert_eq!(engine.expected_path, binary.display().to_string());
        assert_eq!(engine.install_mode, InstallMode::Marketplace);
        assert!(engine.next_step.contains("delivered"), "{engine:?}");
    }

    #[test]
    fn the_refusal_names_the_target_it_resolved_not_the_host() {
        let plugin_root = tests::temp_plugin_root("foreign-target");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        let host = tests::host_target();
        let foreign = ["darwin-arm64", "linux-x64", "win-x64"]
            .into_iter()
            .find(|target| *target != host)
            .expect("the product ships more than one target");
        let exe = if foreign == "win-x64" { ".exe" } else { "" };
        fs::remove_file(plugin_root.join(format!("bin/{foreign}/bsl-analyzer{exe}"))).unwrap();

        let engine = missing(
            resolve_bundled_tool_for_target(&plugin_root, "bsl-analyzer", foreign, true)
                .unwrap_err(),
        );

        assert_eq!(engine.target, foreign);
        assert!(!engine.message().contains(host), "{engine:?}");
    }

    /// Каждая причина отсутствия выходит своим шагом поставки, и ни одна не
    /// требует читать текст: нет манифеста, движок в нём не назван, не
    /// упакован под платформу, файла нет, сумма не совпала.
    #[test]
    fn every_delivery_step_is_named_by_type_without_reading_text() {
        let target = tests::host_target();

        let no_manifest = tests::temp_plugin_root("stage-no-manifest");
        let engine = missing(
            resolve_bundled_tool_for_target(&no_manifest, "bsl-analyzer", target, true)
                .unwrap_err(),
        );
        assert_eq!(engine.stage, EngineStage::Manifest);
        // Без манифеста режим не выводится в исходный checkout: совет
        // собрать движок получает только дерево со сборщиком рядом.
        assert_eq!(engine.install_mode, InstallMode::Marketplace);

        let unreadable = tests::temp_plugin_root("stage-unreadable-manifest");
        fs::write(unreadable.join("third-party/manifest.json"), "{not json").unwrap();
        let engine = missing(
            resolve_bundled_tool_for_target(&unreadable, "bsl-analyzer", target, true).unwrap_err(),
        );
        assert_eq!(engine.stage, EngineStage::Manifest);
        assert_eq!(engine.install_mode, InstallMode::Marketplace);
        assert!(engine.next_step.contains("reinstall"), "{engine:?}");
        assert!(
            !engine.next_step.contains("build-unica-tools"),
            "{engine:?}"
        );

        let checkout = tests::temp_plugin_root("stage-unreadable-in-checkout");
        let checkout_plugin = checkout.join("plugins").join("unica");
        fs::create_dir_all(checkout_plugin.join("third-party")).unwrap();
        fs::write(
            checkout_plugin.join("third-party/manifest.json"),
            "{not json",
        )
        .unwrap();
        fs::create_dir_all(checkout.join("scripts/ci")).unwrap();
        fs::write(checkout.join("scripts/ci/build-unica-tools.py"), "").unwrap();
        let engine = missing(
            resolve_bundled_tool_for_target(&checkout_plugin, "bsl-analyzer", target, true)
                .unwrap_err(),
        );
        assert_eq!(engine.install_mode, InstallMode::Source);
        assert!(engine.next_step.contains("build-unica-tools"), "{engine:?}");

        let unnamed = tests::temp_plugin_root("stage-unnamed");
        tests::write_manifest_with_bsl_analyzer(&unnamed);
        assert_eq!(
            missing(
                resolve_bundled_tool_for_target(&unnamed, "v8-runner", target, true).unwrap_err()
            )
            .stage,
            EngineStage::Manifest
        );

        let foreign_platform = tests::temp_plugin_root("stage-target");
        fs::write(
            foreign_platform.join("third-party/manifest.json"),
            r#"{"schemaVersion":2,"tools":[{"name":"bsl-analyzer","version":"test",
                "binaries":{"elsewhere":{"binaryPath":"bin/x/bsl-analyzer","sha256":"00"}}}]}"#,
        )
        .unwrap();
        assert_eq!(
            missing(
                resolve_bundled_tool_for_target(&foreign_platform, "bsl-analyzer", target, true)
                    .unwrap_err()
            )
            .stage,
            EngineStage::Target
        );

        let absent = tests::temp_plugin_root("stage-delivery");
        tests::write_manifest_with_bsl_analyzer(&absent);
        fs::remove_file(absent.join(tests::host_binary("bsl-analyzer"))).unwrap();
        assert_eq!(
            missing(
                resolve_bundled_tool_for_target(&absent, "bsl-analyzer", target, true).unwrap_err()
            )
            .stage,
            EngineStage::Delivery
        );

        let tampered = tests::temp_plugin_root("stage-checksum");
        tests::write_manifest_with_bsl_analyzer(&tampered);
        let binary = tampered.join(tests::host_binary("bsl-analyzer"));
        fs::write(&binary, "tampered").unwrap();
        let engine = missing(
            resolve_bundled_tool_for_target(&tampered, "bsl-analyzer", target, true).unwrap_err(),
        );
        assert_eq!(engine.stage, EngineStage::Checksum);
        assert_eq!(engine.expected_path, binary.display().to_string());
    }

    /// Наличие проверяется без хеша, а полная проверка на пути отказа
    /// различает подменённый файл.
    #[test]
    fn presence_skips_the_checksum_and_absence_on_the_failure_path_does_not() {
        let plugin_root = tests::temp_plugin_root("presence-vs-absence");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        fs::write(
            plugin_root.join(tests::host_binary("bsl-analyzer")),
            "tampered",
        )
        .unwrap();

        assert_eq!(missing_engine(&plugin_root, "bsl-analyzer"), None);
        assert_eq!(
            engine_absence(&plugin_root, "bsl-analyzer").map(|engine| engine.stage),
            Some(EngineStage::Checksum)
        );

        let intact = tests::temp_plugin_root("presence-intact");
        tests::write_manifest_with_bsl_analyzer(&intact);
        assert_eq!(engine_absence(&intact, "bsl-analyzer"), None);
    }

    #[test]
    fn a_failed_start_names_the_start_step_and_the_program() {
        let plugin_root = tests::temp_plugin_root("start-failure");
        tests::write_manifest_with_bsl_analyzer(&plugin_root);
        let program = plugin_root.join(tests::host_binary("bsl-analyzer"));

        let engine = start_failure(Some(&plugin_root), "bsl-analyzer", &program);

        assert_eq!(engine.stage, EngineStage::Start);
        assert_eq!(engine.expected_path, program.display().to_string());
        assert_eq!(engine.pinned_version.as_deref(), Some("test"));
        assert!(engine.next_step.contains("permissions"), "{engine:?}");
        assert_eq!(
            start_failure(None, "bsl-analyzer", &program).stage,
            EngineStage::Start
        );
    }
}
