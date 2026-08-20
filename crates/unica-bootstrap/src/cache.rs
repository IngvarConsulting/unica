use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::archive::{extract_verified_tar_gz, sha256_file, verify_runtime_files};
use crate::attempt::{AttemptLog, AttemptSubject, OpenAttempt, Stage};
use crate::download::{DownloadObserver, Downloader, SilentDownload};
use crate::error::{BootstrapError, Failure, Result};
use crate::manifest::{DeliveryForm, RuntimeManifest, TargetRuntime};
use crate::platform::HostTarget;

#[derive(Clone)]
pub struct RuntimeInstaller {
    cache_root: PathBuf,
    plugin_version: String,
    downloader: Arc<dyn Downloader>,
}

/// Один артефакт, доставленный прогревом.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prefetched {
    pub artifact: String,
    pub version: String,
    pub root: PathBuf,
    /// Ложь означает, что артефакт уже лежал: пересборка слоя образа не платит
    /// за него второй раз.
    pub downloaded: bool,
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
        // Ядро едет в стартовом бюджете хоста, и слушать его ход некому:
        // провода, по которому об этом рассказывают, ещё не существует.
        let root = self.ensure_artifact(manifest, name, host, &SilentDownload)?;
        let target = manifest.artifact_target(name, host)?;
        Ok(installation(root, target))
    }

    /// Довезти всё, что цели понадобится, и назвать доставленное.
    ///
    /// Ленивая доставка выигрывает старт, но проигрывает закрытый контур: в
    /// образе сети уже нет, и движок, оставленный на первый вызов, там не
    /// приедет никогда. Прогрев — та же доставка, просто вся сразу и заранее.
    ///
    /// Порядок обхода — по имени артефакта: он определён, и вывод сборки от
    /// прогона к прогону не пляшет.
    pub fn prefetch(
        &self,
        manifest: &RuntimeManifest,
        host: HostTarget,
        observer: &dyn DownloadObserver,
    ) -> Result<Vec<Prefetched>> {
        manifest.validate(&self.plugin_version)?;
        let mut delivered = Vec::new();
        for (name, artifact) in &manifest.artifacts {
            let target = manifest.artifact_target(name, host)?;
            let root = self
                .cache_root
                .join(name)
                .join(delivery_key(&artifact.version, target))
                .join(host.as_str());
            // Каталог без годного маркера кешем не является: установщик его
            // заменит, и отчёт прогрева обязан назвать эту загрузку.
            let already = ready_installation(&root, target, name, &artifact.version, host)?;
            self.ensure_artifact(manifest, name, host, observer)?;
            delivered.push(Prefetched {
                artifact: name.clone(),
                version: artifact.version.clone(),
                root,
                downloaded: !already,
            });
        }
        Ok(delivered)
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
        observer: &dyn DownloadObserver,
    ) -> Result<PathBuf> {
        manifest.validate(&self.plugin_version)?;
        let artifact = manifest.artifact(name)?;
        let target = manifest.artifact_target(name, host)?;
        let delivery = delivery_key(&artifact.version, target);
        // Личность установки — артефакт, его версия и байты архива. Одной
        // upstream-версии недостаточно: новый toolchain build может сохранить
        // её, но опубликовать другие байты. Версия плагина сюда не входит:
        // выпуск, не менявший артефакт, не должен объявлять кеш холодным.
        let final_root = self
            .cache_root
            .join(name)
            .join(&delivery)
            .join(host.as_str());

        fs::create_dir_all(self.cache_root.join(".locks"))?;
        let lock_path = delivery_lock_path(&self.cache_root, name, &delivery, host.as_str());
        let lock = open_lock(&lock_path)?;
        lock.lock_exclusive().map_err(|error| {
            BootstrapError::new(format!(
                "failed to lock runtime cache {}: {error}",
                lock_path.display()
            ))
        })?;

        // Эта блокировка охватывает ровно артефакт, неизменяемую поставку и
        // цель. Значит любая более старая транзакция с тем же префиксом уже не
        // может принадлежать живому установщику: её процесс либо завершился,
        // либо был убит. Убираем её до новой staging-области, чтобы аварийные
        // распаковки не росли без границ. `.partial` сюда намеренно не входит:
        // полученные байты должны переживать сессию и возобновляться.
        drop_stale_transactions(&self.cache_root, name, &delivery, host)?;

        if ready_installation(&final_root, target, name, &artifact.version, host)? {
            return Ok(final_root);
        }

        // Недокачка живёт по устойчивому пути рядом с блокировкой, а не внутри
        // транзакции: транзакция умирает вместе с попыткой, а полученные байты
        // должны её пережить и достаться следующей сессии.
        let partial_root = self.cache_root.join(".partial").join(name);
        fs::create_dir_all(&partial_root)?;
        let partial = partial_root.join(format!("{}-{}.tar.gz", delivery, host.as_str()));
        drop_other_partials(&self.cache_root, name, &partial);

        let transaction_root = self.cache_root.join(".transactions").join(format!(
            "{name}-{}-{}-{}",
            delivery,
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

        let result = match self
            .downloader
            .download(&target.asset.url, &partial, observer)
        {
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
            let artifact_name = artifact
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| BootstrapError::new("runtime artifact directory has no name"))?;
            for (_, path) in versions.into_iter().skip(keep) {
                let delivery = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| BootstrapError::new("runtime delivery directory has no name"))?;
                let Some(_locks) = try_lock_delivery_targets(cache_root, artifact_name, delivery)?
                else {
                    continue;
                };
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

/// Устойчивая и неизменяемая личность одной поставки.
///
/// Семантическая версия оставляет каталог читаемым человеку, а сумма архива
/// делает его байтовой личностью. Одинаковые байты под новым release tag могут
/// безопасно разделить кеш; разные байты с той же upstream-версией — никогда.
fn delivery_key(version: &str, target: &TargetRuntime) -> String {
    format!("{version}--{}", target.asset.sha256)
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
    // Форма решает, что делать с байтами: архив разворачивается, одиночный
    // файл кладётся под своим именем. Переупаковка ради единой формы вернула
    // бы копию в выпуске плагина, от которой мы ушли.
    match DeliveryForm::of(&target.asset.media_type) {
        Some(DeliveryForm::Archive) | None => {
            extract_verified_tar_gz(archive_path, staged_root, &target.files)?
        }
        Some(DeliveryForm::File) => place_single_file(archive_path, staged_root, target)?,
    }
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

/// Положить артефакт, приехавший одним файлом.
///
/// Распаковывать нечего: сумма уже сверена, остаётся дать файлу имя, под
/// которым его объявили, и права, которые ему объявлены.
fn place_single_file(source: &Path, staged_root: &Path, target: &TargetRuntime) -> Result<()> {
    let file = target.files.first().ok_or_else(|| {
        BootstrapError::of(
            Failure::Configuration,
            "a single-file artifact declares no file".to_string(),
        )
    })?;
    let destination = staged_root.join(&file.path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, &destination)?;
    crate::platform::set_executable(&destination, file.executable)?;
    verify_runtime_files(staged_root, &target.files, None)
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
        let path = entry.path();
        if path == keep {
            continue;
        }
        let Some(stem) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".tar.gz"))
            .map(str::to_owned)
        else {
            continue;
        };
        let Ok(lock) = open_lock(
            &cache_root
                .join(".locks")
                .join(format!("{artifact}-{stem}.lock")),
        ) else {
            continue;
        };
        if lock.try_lock_exclusive().is_ok() {
            let _ = fs::remove_file(path);
        }
    }
}

fn delivery_lock_path(cache_root: &Path, artifact: &str, delivery: &str, target: &str) -> PathBuf {
    cache_root
        .join(".locks")
        .join(format!("{artifact}-{delivery}-{target}.lock"))
}

/// Захватить все целевые блокировки версии перед удалением её каталога.
///
/// Цели известны заранее, поэтому конкурент не может опубликовать четвёртый
/// каталог между перечислением и удалением. Занята хотя бы одна — версия жива
/// у другого процесса и сборка мусора её пропускает.
fn try_lock_delivery_targets(
    cache_root: &Path,
    artifact: &str,
    delivery: &str,
) -> Result<Option<Vec<File>>> {
    fs::create_dir_all(cache_root.join(".locks"))?;
    let mut locked = Vec::new();
    for target in HostTarget::ALL {
        let path = delivery_lock_path(cache_root, artifact, delivery, target.as_str());
        let lock = open_lock(&path)?;
        match lock.try_lock_exclusive() {
            Ok(()) => locked.push(lock),
            Err(error) if lock_is_contended(&error) => return Ok(None),
            Err(error) => {
                return Err(BootstrapError::new(format!(
                    "failed to inspect runtime cache lock {}: {error}",
                    path.display()
                )))
            }
        }
    }
    Ok(Some(locked))
}

/// `fs2` preserves the native lock error on Windows (error 33), while Unix
/// commonly maps contention to `WouldBlock`. Both mean that another delivery
/// owns the installation and collection must leave it alone.
fn lock_is_contended(error: &std::io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == std::io::ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

fn drop_stale_transactions(
    cache_root: &Path,
    artifact: &str,
    delivery: &str,
    host: HostTarget,
) -> Result<()> {
    let root = cache_root.join(".transactions");
    let Ok(entries) = fs::read_dir(&root) else {
        return Ok(());
    };
    let prefix = format!("{artifact}-{delivery}-{}-", host.as_str());
    for entry in entries {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        fs::remove_dir_all(entry.path()).map_err(|error| {
            BootstrapError::new(format!(
                "failed to clean stale runtime transaction {}: {error}",
                entry.path().display()
            ))
        })?;
    }
    Ok(())
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
