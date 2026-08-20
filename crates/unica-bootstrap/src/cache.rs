use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::archive::{extract_verified_tar_gz, sha256_file, verify_runtime_files};
use crate::attempt::{AttemptLog, AttemptSubject, OpenAttempt, Stage};
use crate::download::Downloader;
use crate::error::{BootstrapError, Failure, Result};
use crate::manifest::{RuntimeManifest, TargetRuntime};
use crate::platform::HostTarget;

#[derive(Clone)]
pub struct RuntimeInstaller {
    cache_root: PathBuf,
    plugin_version: String,
    downloader: Arc<dyn Downloader>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeInstallation {
    pub root: PathBuf,
    pub entrypoint: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadyMarker {
    artifact: String,
    version: String,
    target: String,
    asset_sha256: String,
}

/// Сколько версий одного артефакта переживают сборку мусора.
///
/// Две: текущая и предыдущая. Откат на предыдущую версию плагина — обычное
/// действие, и заставлять его качать 69 МБ заново значит наказывать за откат.
/// Третья версия уже не нужна: до неё откатываются реже, чем стоит место.
pub const RETAINED_VERSIONS: usize = 2;

impl RuntimeInstaller {
    pub fn new(
        cache_root: PathBuf,
        plugin_version: impl Into<String>,
        downloader: Arc<dyn Downloader>,
    ) -> Self {
        Self {
            cache_root,
            plugin_version: plugin_version.into(),
            downloader,
        }
    }

    pub fn ensure(
        &self,
        manifest: &RuntimeManifest,
        host: HostTarget,
    ) -> Result<RuntimeInstallation> {
        let name = crate::manifest::CORE_ARTIFACT;
        let root = self.ensure_artifact(manifest, name, host)?;
        let target = manifest.artifact_target(name, host)?;
        Ok(installation(root, target))
    }

    /// Доставить артефакт по имени и вернуть корень установки.
    ///
    /// Ядро едет в стартовом бюджете хоста, движок — когда он понадобился, и
    /// путь у них один: блокировка на артефакт, докачка, сумма, распаковка,
    /// публикация.
    pub fn ensure_artifact(
        &self,
        manifest: &RuntimeManifest,
        name: &str,
        host: HostTarget,
    ) -> Result<PathBuf> {
        manifest.validate(&self.plugin_version)?;
        let artifact = manifest.artifact(name)?;
        let target = manifest.artifact_target(name, host)?;
        // Личность установки — артефакт, его версия и байты архива. Версия
        // плагина сюда не входит: выпуск, не менявший артефакт, не должен
        // объявлять кеш холодным.
        let final_root = self
            .cache_root
            .join(name)
            .join(&artifact.version)
            .join(host.as_str());

        fs::create_dir_all(self.cache_root.join(".locks"))?;
        let lock_path = self.cache_root.join(".locks").join(format!(
            "{name}-{}-{}.lock",
            artifact.version,
            host.as_str()
        ));
        let lock = open_lock(&lock_path)?;
        lock.lock_exclusive().map_err(|error| {
            BootstrapError::new(format!(
                "failed to lock runtime cache {}: {error}",
                lock_path.display()
            ))
        })?;

        if ready_installation(&final_root, target, name, &artifact.version, host)? {
            return Ok(final_root);
        }

        // Недокачка живёт по устойчивому пути рядом с блокировкой, а не внутри
        // транзакции: транзакция умирает вместе с попыткой, а полученные байты
        // должны её пережить и достаться следующей сессии.
        let partial_root = self.cache_root.join(".partial").join(name);
        fs::create_dir_all(&partial_root)?;
        let partial = partial_root.join(format!("{}-{}.tar.gz", artifact.version, host.as_str()));
        drop_other_partials(&self.cache_root, name, &partial);

        let transaction_root = self.cache_root.join(".transactions").join(format!(
            "{name}-{}-{}-{}",
            artifact.version,
            host.as_str(),
            Uuid::new_v4()
        ));
        let staged_root = transaction_root.join("runtime");
        fs::create_dir_all(&staged_root)?;

        // Запись о попытке открывается до первой стадии: убитый процесс не
        // печатает ничего, и всё, что он оставит, должно быть написано заранее.
        let attempt = AttemptLog::in_cache(&self.cache_root).open(AttemptSubject {
            artifact: name.to_owned(),
            version: artifact.version.clone(),
            target: host.as_str().to_owned(),
            url: target.asset.url.clone(),
            partial: partial.clone(),
        })?;

        let result = match self.downloader.download(&target.asset.url, &partial) {
            // Оборванный перенос — единственный случай, когда полученное
            // остаётся лежать: продолжить его дешевле, чем начать заново.
            Err(error) => Err(error),
            Ok(()) => {
                let published = publish_artifact(
                    &partial,
                    &staged_root,
                    &final_root,
                    target,
                    name,
                    &artifact.version,
                    host,
                    &attempt,
                );
                // Приехавшее целиком дальше либо установлено, либо негодно.
                // В обоих случаях докачивать нечего, и место занимать незачем.
                let _ = fs::remove_file(&partial);
                published
            }
        };

        // Закрытая запись больше никому не интересна: о замеченном отказе уже
        // сказано кодом выхода и потоком ошибок. Неудача самой записи работу не
        // отменяет — она диагностика, а не дело.
        let closed = match &result {
            Ok(()) => attempt.finished(),
            Err(error) => attempt.failed(error.failure(), &error.to_string()),
        };
        if let Err(error) = closed {
            eprintln!("unica-bootstrap: failed to close the attempt record: {error}");
        }

        if result.is_ok() {
            // Сборка мусора не вправе отменить состоявшуюся установку: на
            // Windows каталог с работающим бинарём удалить нельзя, и соседняя
            // сессия — законная причина отказа, а не повод не стартовать.
            if let Err(error) = Self::collect(&self.cache_root, RETAINED_VERSIONS) {
                eprintln!("unica-bootstrap: failed to tidy the runtime cache: {error}");
            }
        }

        if transaction_root.exists() {
            fs::remove_dir_all(&transaction_root).map_err(|cleanup_error| {
                BootstrapError::new(format!(
                    "{}; failed to clean transaction {}: {cleanup_error}",
                    result
                        .as_ref()
                        .err()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "runtime transaction succeeded".to_string()),
                    transaction_root.display()
                ))
            })?;
        }
        result.map(|()| final_root)
    }

    /// Оставить у каждого артефакта `keep` свежайших версий, остальные удалить.
    ///
    /// Удержание считается по артефакту, а не по ссылкам от версий ядра.
    /// Ссылочный учёт потребовал бы записывать рядом с ядром список нужных ему
    /// движков, и эта запись живёт отдельно от того, что описывает, — то есть
    /// умеет протухнуть. Удержание по артефакту даёт тот же результат без
    /// второго источника правды: движок, нужный удерживаемой версии ядра, либо
    /// не менялся и потому свежайший, либо менялся и приехал новой версией.
    ///
    /// Крайний случай честный: откат через три выпуска подряд может не найти
    /// движок в кеше и скачает его заново. Это медленнее, но не сломано.
    pub fn collect(cache_root: &Path, keep: usize) -> Result<()> {
        if !cache_root.is_dir() {
            return Ok(());
        }
        let mut stuck = Vec::new();
        for artifact in read_child_dirs(cache_root)? {
            let mut versions = read_child_dirs(&artifact)?
                .into_iter()
                .map(|path| {
                    let age = installed_at(&path);
                    (age, path)
                })
                .collect::<Vec<_>>();
            if versions.len() <= keep {
                continue;
            }
            // Свежайшие вперёд; при равном времени — по имени, чтобы порядок не
            // зависел от разрешения часов файловой системы.
            versions.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
            for (_, path) in versions.into_iter().skip(keep) {
                if fs::remove_dir_all(&path).is_err() {
                    stuck.push(path);
                }
            }
        }
        if stuck.is_empty() {
            return Ok(());
        }
        Err(BootstrapError::new(format!(
            "не удалось удалить устаревшие версии: {}",
            stuck
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

/// Опубликовать скачанный архив: сумма, распаковка, маркер, подмена каталога.
#[allow(clippy::too_many_arguments)]
fn publish_artifact(
    archive_path: &Path,
    staged_root: &Path,
    final_root: &Path,
    target: &TargetRuntime,
    artifact: &str,
    version: &str,
    host: HostTarget,
    attempt: &OpenAttempt,
) -> Result<()> {
    attempt.reached(Stage::Verify)?;
    let actual_archive_sha = sha256_file(archive_path)?;
    if actual_archive_sha != target.asset.sha256 {
        return Err(BootstrapError::of(
            Failure::Checksum,
            format!(
                "runtime archive sha256 {actual_archive_sha} != expected {} for {}",
                target.asset.sha256, target.asset.url
            ),
        ));
    }
    attempt.reached(Stage::Extract)?;
    extract_verified_tar_gz(archive_path, staged_root, &target.files)?;
    write_ready_marker(staged_root, artifact, version, host, &target.asset.sha256)?;

    attempt.reached(Stage::Publish)?;
    if final_root.exists() {
        let quarantine =
            final_root.with_file_name(format!("{}.invalid-{}", host.as_str(), Uuid::new_v4()));
        fs::rename(final_root, &quarantine)?;
        fs::remove_dir_all(quarantine)?;
    }
    if let Some(parent) = final_root.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(staged_root, final_root)?;
    Ok(())
}

/// Убрать недокачки того же артефакта, оставшиеся от других версий.
///
/// Их никто не ждёт: манифест уехал вперёд, а докачивать версию, которую больше
/// не ставят, значит держать десятки мегабайт до конца времён. Отказ удаления
/// не останавливает установку — место подождёт, старт нет.
fn drop_other_partials(cache_root: &Path, artifact: &str, keep: &Path) {
    let Ok(entries) = fs::read_dir(cache_root.join(".partial").join(artifact)) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path() != keep {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Подкаталоги без служебных: `.locks` и `.transactions` артефактами не
/// являются, и удалить их значит отобрать блокировку у соседнего процесса.
fn read_child_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !entry.path().is_dir() {
            continue;
        }
        found.push(entry.path());
    }
    Ok(found)
}

/// Когда версия появилась в кеше: свежайший маркер готовности среди её целей.
///
/// Берём маркер, а не сам каталог: каталог версии переживает установку соседней
/// цели, и его время сказало бы о последней записи, а не о появлении версии.
fn installed_at(version_root: &Path) -> SystemTime {
    read_child_dirs(version_root)
        .unwrap_or_default()
        .iter()
        .filter_map(|target| fs::metadata(target.join(".ready.json")).ok())
        .filter_map(|meta| meta.modified().ok())
        .max()
        .or_else(|| fs::metadata(version_root).ok()?.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn open_lock(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(Into::into)
}

fn ready_installation(
    root: &Path,
    target: &TargetRuntime,
    artifact: &str,
    version: &str,
    host: HostTarget,
) -> Result<bool> {
    let marker_path = root.join(".ready.json");
    if !marker_path.is_file() {
        return Ok(false);
    }
    let marker: ReadyMarker = match fs::read(&marker_path)
        .map_err(BootstrapError::from)
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(BootstrapError::from))
    {
        Ok(marker) => marker,
        Err(_) => return Ok(false),
    };
    if marker.artifact != artifact
        || marker.version != version
        || marker.target != host.as_str()
        || marker.asset_sha256 != target.asset.sha256
    {
        return Ok(false);
    }
    match verify_runtime_files(root, &target.files, None) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn write_ready_marker(
    root: &Path,
    artifact: &str,
    version: &str,
    host: HostTarget,
    asset_sha256: &str,
) -> Result<()> {
    let marker = ReadyMarker {
        artifact: artifact.to_string(),
        version: version.to_string(),
        target: host.as_str().to_string(),
        asset_sha256: asset_sha256.to_string(),
    };
    let path = root.join(".ready.json");
    let file = File::create(&path)?;
    serde_json::to_writer_pretty(&file, &marker)?;
    file.sync_all()?;
    Ok(())
}

fn installation(root: PathBuf, target: &TargetRuntime) -> RuntimeInstallation {
    // Устанавливает bootstrap только ядро, а у ядра точка входа проверена
    // манифестом. Пустое значение сюда не доходит.
    let entrypoint = target.entrypoint.clone().unwrap_or_default();
    RuntimeInstallation {
        entrypoint: root.join(entrypoint),
        root,
    }
}
