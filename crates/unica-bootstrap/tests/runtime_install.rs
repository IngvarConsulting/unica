use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use tar::{Builder, EntryType, Header};
use unica_bootstrap::{
    AttemptLog, BootstrapError, DownloadObserver, Downloader, Failure, HostTarget, RuntimeFile,
    RuntimeInstaller, RuntimeManifest, SilentDownload, Stage,
};
use uuid::Uuid;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("unica-bootstrap-{name}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp directory");
    path
}

fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let output = Vec::new();
    let encoder = GzEncoder::new(output, Compression::default());
    let mut builder = Builder::new(encoder);
    for (path, contents) in entries {
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_entry_type(EntryType::Regular);
        header.set_cksum();
        builder
            .append_data(&mut header, path, *contents)
            .expect("append tar entry");
    }
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip")
}

fn unsafe_tar_gz() -> Vec<u8> {
    let output = Vec::new();
    let encoder = GzEncoder::new(output, Compression::default());
    let mut builder = Builder::new(encoder);
    let contents = b"escape";
    let mut header = Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o755);
    header.set_entry_type(EntryType::Regular);
    header.as_mut_bytes()[..9].copy_from_slice(b"../escape");
    header.set_cksum();
    builder
        .append(&header, contents.as_slice())
        .expect("append unsafe tar entry");
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip")
}

/// Манифест схемы 2: артефакты перечислены по отдельности, у каждого своя
/// версия. Ключ установки берётся из неё, а не из версии плагина — иначе выпуск
/// объявляет холодным то, что не менялось.
fn manifest_with(
    archive: &[u8],
    runtime: &[u8],
    plugin_version: &str,
    core_version: &str,
) -> RuntimeManifest {
    let archive_hash = sha256(archive);
    let runtime_hash = sha256(runtime);
    let tag = format!("v{plugin_version}");
    let target = |name: &str, executable: &str| {
        serde_json::json!({
            "asset": {
                "name": format!("unica-runtime-{name}.tar.gz"),
                "url": format!(
                    "https://github.com/IngvarConsulting/unica/releases/download/{tag}/unica-runtime-{name}.tar.gz"
                ),
                "mediaType": "application/gzip",
                "sha256": archive_hash
            },
            "files": [{"path": executable, "sha256": runtime_hash, "executable": true}],
            "entrypoint": executable
        })
    };
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 2,
        "pluginVersion": plugin_version,
        "source": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "commit": COMMIT
        },
        "release": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "tag": tag
        },
        "artifacts": {
            "unica": {
                "version": core_version,
                "role": "core",
                "targets": {
                    "darwin-arm64": target("darwin-arm64", "bin/darwin-arm64/unica"),
                    "linux-x64": target("linux-x64", "bin/linux-x64/unica"),
                    "win-x64": target("win-x64", "bin/win-x64/unica.exe")
                }
            }
        }
    }))
    .expect("manifest fixture")
}

fn manifest(archive: &[u8], runtime: &[u8]) -> RuntimeManifest {
    manifest_with(archive, runtime, "0.7.0", "0.7.0")
}

struct FakeDownloader {
    bytes: Vec<u8>,
    calls: AtomicUsize,
}

impl FakeDownloader {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Downloader for FakeDownloader {
    fn download(
        &self,
        _url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_bootstrap::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut file = fs::File::create(destination)?;
        file.write_all(&self.bytes)?;
        file.sync_all()?;
        Ok(())
    }
}

#[test]
fn valid_archive_is_published_with_a_ready_marker() {
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);
    let cache = temp_dir("valid");
    let downloader = Arc::new(FakeDownloader::new(archive));
    let installer = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader);

    let installed = installer
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect("runtime install");

    assert_eq!(fs::read(&installed.entrypoint).unwrap(), runtime);
    assert!(installed.root.join(".ready.json").is_file());
    fs::remove_dir_all(cache).expect("remove temp directory");
}

#[test]
fn ready_marker_waits_for_the_complete_runtime_file_closure() {
    let runtime = b"unica-runtime";
    let library = b"shared-library";
    let library_path = "bin/linux-x64/libpython3.12.so.1.0";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime), (library_path, library)]);
    let mut manifest = manifest(&archive, runtime);
    manifest
        .artifacts
        .get_mut("unica")
        .expect("core fixture")
        .targets
        .get_mut("linux-x64")
        .expect("linux fixture")
        .files
        .push(RuntimeFile {
            path: library_path.to_owned(),
            sha256: sha256(library),
            executable: false,
        });
    let cache = temp_dir("complete-closure");
    let downloader = Arc::new(FakeDownloader::new(archive));
    let installer = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader);

    let installed = installer
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect("runtime install");

    assert_eq!(
        fs::read(installed.root.join(library_path)).unwrap(),
        library
    );
    assert!(installed.root.join(".ready.json").is_file());
    fs::remove_dir_all(cache).expect("remove temp directory");
}

#[test]
fn corrupt_archive_never_publishes_a_ready_runtime() {
    let runtime = b"unica-runtime";
    let expected_archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&expected_archive, runtime);
    let cache = temp_dir("corrupt");
    let downloader = Arc::new(FakeDownloader::new(b"not a gzip".to_vec()));
    let installer = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader);

    let error = installer
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect_err("corrupt download must fail");

    assert!(error.to_string().contains("archive sha256"));
    assert!(!cache.join("0.7.0/linux-x64/.ready.json").exists());
    fs::remove_dir_all(cache).expect("remove temp directory");
}

#[test]
fn traversal_archive_is_rejected_before_publication() {
    let runtime = b"unica-runtime";
    let archive = unsafe_tar_gz();
    let mut manifest = manifest(&archive, runtime);
    manifest
        .artifacts
        .get_mut("unica")
        .unwrap()
        .targets
        .get_mut("linux-x64")
        .unwrap()
        .files[0]
        .sha256 = sha256(runtime);
    let cache = temp_dir("traversal");
    let downloader = Arc::new(FakeDownloader::new(archive));
    let installer = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader);

    let error = installer
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect_err("traversal must fail");

    assert!(error.to_string().contains("unsafe archive path"));
    assert!(!cache.parent().unwrap().join("escape").exists());
    fs::remove_dir_all(cache).expect("remove temp directory");
}

#[test]
fn concurrent_installers_download_and_publish_once() {
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = Arc::new(manifest(&archive, runtime));
    let cache = temp_dir("concurrent");
    let downloader = Arc::new(FakeDownloader::new(archive));
    let installer = Arc::new(RuntimeInstaller::new(
        cache.clone(),
        "0.7.0",
        downloader.clone(),
    ));

    let handles = (0..2)
        .map(|_| {
            let installer = installer.clone();
            let manifest = manifest.clone();
            thread::spawn(move || installer.ensure(&manifest, HostTarget::LinuxX64))
        })
        .collect::<Vec<_>>();
    let installations = handles
        .into_iter()
        .map(|handle| handle.join().expect("installer thread"))
        .collect::<Result<Vec<_>, _>>()
        .expect("both installers succeed");

    assert_eq!(installations[0].root, installations[1].root);
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 1);
    fs::remove_dir_all(cache).expect("remove temp directory");
}

#[test]
fn the_core_installs_without_any_engine_present() {
    // Ядро обязано подниматься само: движки уходят из стартового пути.
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);
    let cache = temp_dir("core-alone");
    let downloader = Arc::new(FakeDownloader::new(archive));

    let installed = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader)
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect("core installs alone");

    assert!(installed.entrypoint.is_file(), "ядро должно быть на месте");
    assert_eq!(
        fs::read_dir(installed.root.join("bin/linux-x64"))
            .expect("read runtime root")
            .count(),
        1,
        "в установке ядра нет ничего, кроме него самого"
    );
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn the_install_path_is_keyed_by_the_artifact_version() {
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest_with(&archive, runtime, "0.7.0", "0.5.1");
    let cache = temp_dir("keyed");
    let downloader = Arc::new(FakeDownloader::new(archive));

    let installed = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader)
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect("install");

    let relative = installed
        .root
        .strip_prefix(&cache)
        .expect("install lands inside the cache");
    assert_eq!(
        relative,
        Path::new("unica").join("0.5.1").join("linux-x64"),
        "путь содержит версию артефакта, а не версию плагина"
    );
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn a_plugin_release_does_not_refetch_an_unchanged_artifact() {
    // Ровно тот случай из #585: обновление плагина объявляло холодным весь кеш.
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let cache = temp_dir("unchanged");
    let downloader = Arc::new(FakeDownloader::new(archive.clone()));

    let before = manifest_with(&archive, runtime, "0.7.0", "0.5.1");
    RuntimeInstaller::new(cache.clone(), "0.7.0", downloader.clone())
        .ensure(&before, HostTarget::LinuxX64)
        .expect("first install");
    assert_eq!(downloader.calls(), 1);

    let after = manifest_with(&archive, runtime, "0.8.0", "0.5.1");
    RuntimeInstaller::new(cache.clone(), "0.8.0", downloader.clone())
        .ensure(&after, HostTarget::LinuxX64)
        .expect("second install");
    assert_eq!(
        downloader.calls(),
        1,
        "версия плагина сменилась, версия артефакта нет — качать нечего"
    );
    fs::remove_dir_all(&cache).ok();
}

/// Установить артефакт заданной версии в кеш, минуя загрузку: сборке мусора
/// важна раскладка, а не то, как она возникла.
fn seed_installation(cache: &Path, artifact: &str, version: &str, host: HostTarget) -> PathBuf {
    let root = cache.join(artifact).join(version).join(host.as_str());
    fs::create_dir_all(root.join("bin").join(host.as_str())).expect("seed install");
    fs::write(root.join(".ready.json"), "{}").expect("seed marker");
    root
}

#[test]
fn collecting_keeps_the_newest_versions_of_each_artifact() {
    let cache = temp_dir("collect-keeps");
    for version in ["1.0.0", "1.1.0", "1.2.0"] {
        seed_installation(&cache, "rlm-tools-bsl", version, HostTarget::LinuxX64);
        // Отметки времени должны различаться, иначе «свежайшие» неопределимы.
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    seed_installation(&cache, "unica", "0.13.0", HostTarget::LinuxX64);

    RuntimeInstaller::collect(&cache, 2).expect("collect");

    let kept = |artifact: &str, version: &str| cache.join(artifact).join(version).is_dir();
    assert!(
        !kept("rlm-tools-bsl", "1.0.0"),
        "самая старая версия удаляется"
    );
    assert!(kept("rlm-tools-bsl", "1.1.0"));
    assert!(kept("rlm-tools-bsl", "1.2.0"));
    assert!(kept("unica", "0.13.0"), "чужой артефакт не задет");
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn collecting_leaves_an_artifact_that_is_within_the_limit() {
    let cache = temp_dir("collect-within");
    seed_installation(&cache, "bsl-analyzer", "0.2.67", HostTarget::LinuxX64);

    RuntimeInstaller::collect(&cache, 2).expect("collect");

    assert!(cache.join("bsl-analyzer").join("0.2.67").is_dir());
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn collecting_does_not_touch_the_lock_and_transaction_areas() {
    // Служебные каталоги кеша артефактами не являются, и удалять их — потерять
    // блокировку у соседнего процесса.
    let cache = temp_dir("collect-service");
    fs::create_dir_all(cache.join(".locks")).expect("locks");
    // Двух незавершённых транзакций при пределе в одну достаточно, чтобы отличить
    // пропуск служебных каталогов от их случайного попадания под лимит.
    fs::create_dir_all(cache.join(".transactions").join("in-flight-a")).expect("transactions");
    fs::create_dir_all(cache.join(".transactions").join("in-flight-b")).expect("transactions");
    fs::create_dir_all(cache.join(".partial").join("unica")).expect("partial");
    fs::write(
        cache
            .join(".partial")
            .join("unica")
            .join("0.13.0-linux-x64.tar.gz"),
        b"half an archive",
    )
    .expect("partial download");
    fs::create_dir_all(cache.join(".attempts").join("unica")).expect("attempts");
    fs::write(
        cache
            .join(".attempts")
            .join("unica")
            .join("0.13.0-linux-x64.jsonl"),
        b"{}\n",
    )
    .expect("attempt record");

    RuntimeInstaller::collect(&cache, 1).expect("collect");

    assert!(cache.join(".locks").is_dir());
    assert!(cache.join(".transactions").join("in-flight-a").is_dir());
    assert!(cache.join(".transactions").join("in-flight-b").is_dir());
    assert!(
        cache
            .join(".partial")
            .join("unica")
            .join("0.13.0-linux-x64.tar.gz")
            .is_file(),
        "недокачка переживает сборку мусора: её ждёт следующая сессия"
    );
    assert!(
        cache
            .join(".attempts")
            .join("unica")
            .join("0.13.0-linux-x64.jsonl")
            .is_file(),
        "запись о попытке переживает сборку мусора: о ней ещё не сообщили"
    );
    fs::remove_dir_all(&cache).ok();
}

/// Манифест с ядром и движком: у каждого своя версия и свой архив.
fn manifest_with_engine(
    core_archive: &[u8],
    core_file: &[u8],
    engine_archive: &[u8],
    engine_file: &[u8],
) -> RuntimeManifest {
    let mut manifest = manifest(core_archive, core_file);
    let engine_hash = sha256(engine_archive);
    let file_hash = sha256(engine_file);
    let target = |name: &str| {
        serde_json::json!({
            "asset": {
                "name": format!("rlm-tools-bsl-runtime-{name}.tar.gz"),
                "url": format!(
                    "https://github.com/IngvarConsulting/unica/releases/download/v0.7.0/rlm-tools-bsl-runtime-{name}.tar.gz"
                ),
                "mediaType": "application/gzip",
                "sha256": engine_hash
            },
            "files": [{
                "path": format!("bin/{name}/rlm-bsl-index"),
                "sha256": file_hash,
                "executable": true
            }]
        })
    };
    let engine = serde_json::json!({
        "version": "1.33.0",
        "role": "engine",
        "targets": {
            "darwin-arm64": target("darwin-arm64"),
            "linux-x64": target("linux-x64"),
            "win-x64": target("win-x64")
        }
    });
    manifest.artifacts.insert(
        "rlm-tools-bsl".to_owned(),
        serde_json::from_value(engine).expect("engine fixture"),
    );
    manifest
}

/// Загрузчик, отдающий байты по имени ассета: артефактов в манифесте несколько,
/// и подмена одного другим прошла бы незамеченной.
struct AssetDownloader {
    assets: std::collections::BTreeMap<String, Vec<u8>>,
    calls: AtomicUsize,
}

impl AssetDownloader {
    fn new(assets: Vec<(&str, Vec<u8>)>) -> Self {
        Self {
            assets: assets
                .into_iter()
                .map(|(name, bytes)| (name.to_owned(), bytes))
                .collect(),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Downloader for AssetDownloader {
    fn download(
        &self,
        url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_bootstrap::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let name = url.rsplit('/').next().unwrap_or_default();
        let bytes = self
            .assets
            .get(name)
            .ok_or_else(|| BootstrapError::new(format!("стенд не публикует ассет {name}")))?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(destination)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }
}

/// Загрузчик, обрывающийся на середине первой попытки и дописывающий хвост на
/// второй: так ведёт себя канал, убитый посреди установки.
struct InterruptedDownloader {
    bytes: Vec<u8>,
    cut: usize,
    starts: std::sync::Mutex<Vec<u64>>,
}

impl InterruptedDownloader {
    fn new(bytes: Vec<u8>, cut: usize) -> Self {
        Self {
            bytes,
            cut,
            starts: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn starts(&self) -> Vec<u64> {
        self.starts.lock().expect("starts").clone()
    }
}

impl Downloader for InterruptedDownloader {
    fn download(
        &self,
        _url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_bootstrap::Result<()> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let already = fs::metadata(destination)
            .map(|meta| meta.len() as usize)
            .unwrap_or(0);
        self.starts.lock().expect("starts").push(already as u64);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(destination)?;
        if already == 0 {
            file.write_all(&self.bytes[..self.cut])?;
            file.sync_all()?;
            return Err(BootstrapError::new("канал оборвался посреди загрузки"));
        }
        file.write_all(&self.bytes[already..])?;
        file.sync_all()?;
        Ok(())
    }
}

/// Первая попытка приносит байты нужной длины, но не те; вторая — настоящие.
struct PoisonedThenHonest {
    poison: Vec<u8>,
    honest: Vec<u8>,
    calls: AtomicUsize,
}

impl Downloader for PoisonedThenHonest {
    fn download(
        &self,
        _url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_bootstrap::Result<()> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let already = fs::metadata(destination)
            .map(|meta| meta.len() as usize)
            .unwrap_or(0);
        let bytes = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            &self.poison
        } else {
            &self.honest
        };
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(destination)?;
        file.write_all(&bytes[already.min(bytes.len())..])?;
        file.sync_all()?;
        Ok(())
    }
}

/// Что осталось недокачанным у артефакта. Раскладка здесь — не деталь: по ней
/// следующая сессия находит оборванную загрузку.
fn partials(cache: &Path, artifact: &str) -> Vec<String> {
    let mut names = fs::read_dir(cache.join(".partial").join(artifact))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn an_interrupted_download_resumes_in_the_next_session() {
    let runtime = b"unica-runtime-that-is-long-enough-to-cut-in-half";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);
    let cache = temp_dir("resume");
    let cut = archive.len() / 2;
    let downloader = Arc::new(InterruptedDownloader::new(archive.clone(), cut));

    RuntimeInstaller::new(cache.clone(), "0.7.0", downloader.clone())
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect_err("первая сессия обрывается посреди загрузки");

    let installed = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader.clone())
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect("вторая сессия доводит установку");

    assert_eq!(fs::read(&installed.entrypoint).unwrap(), runtime);
    assert_eq!(
        downloader.starts(),
        vec![0, cut as u64],
        "вторая сессия начинает там, где оборвалась первая"
    );
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn a_partial_that_hashes_wrong_is_dropped_instead_of_resumed_forever() {
    // Докачивать не те байты значит вечно не сходиться по сумме.
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);
    let cache = temp_dir("poisoned");
    let downloader = Arc::new(PoisonedThenHonest {
        poison: vec![0_u8; archive.len() / 2],
        honest: archive.clone(),
        calls: AtomicUsize::new(0),
    });

    RuntimeInstaller::new(cache.clone(), "0.7.0", downloader.clone())
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect_err("сумма архива не сходится");

    let installed = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader)
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect("следующая попытка качает заново, а не дописывает мусор");

    assert_eq!(fs::read(&installed.entrypoint).unwrap(), runtime);
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn an_abandoned_partial_does_not_outlive_the_version_that_replaced_it() {
    let runtime = b"unica-runtime-that-is-long-enough-to-cut-in-half";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let cache = temp_dir("abandoned");
    let cut = archive.len() / 2;

    let broken = Arc::new(InterruptedDownloader::new(archive.clone(), cut));
    RuntimeInstaller::new(cache.clone(), "0.7.0", broken)
        .ensure(
            &manifest_with(&archive, runtime, "0.7.0", "0.5.1"),
            HostTarget::LinuxX64,
        )
        .expect_err("обрыв на версии 0.5.1");
    assert_eq!(
        partials(&cache, "unica").len(),
        1,
        "оборванная загрузка дожидается следующей сессии"
    );

    RuntimeInstaller::new(
        cache.clone(),
        "0.7.0",
        Arc::new(FakeDownloader::new(archive)),
    )
    .ensure(
        &manifest_with(b"", runtime, "0.7.0", "0.5.2"),
        HostTarget::LinuxX64,
    )
    .ok();

    assert!(
        partials(&cache, "unica").is_empty(),
        "недокачка брошенной версии не переживает установку следующей: {:?}",
        partials(&cache, "unica")
    );
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn an_engine_is_installed_on_demand_under_its_own_version() {
    let core = b"unica-runtime";
    let core_archive = tar_gz(&[("bin/linux-x64/unica", core)]);
    let engine = b"rlm-bsl-index";
    let engine_archive = tar_gz(&[("bin/linux-x64/rlm-bsl-index", engine)]);
    let manifest = manifest_with_engine(&core_archive, core, &engine_archive, engine);
    let cache = temp_dir("engine");
    let downloader = Arc::new(AssetDownloader::new(vec![
        ("unica-runtime-linux-x64.tar.gz", core_archive),
        ("rlm-tools-bsl-runtime-linux-x64.tar.gz", engine_archive),
    ]));

    let root = RuntimeInstaller::new(cache.clone(), "0.7.0", downloader.clone())
        .ensure_artifact(
            &manifest,
            "rlm-tools-bsl",
            HostTarget::LinuxX64,
            &SilentDownload,
        )
        .expect("движок ставится по имени");

    assert_eq!(
        root.strip_prefix(&cache).expect("установка внутри кеша"),
        Path::new("rlm-tools-bsl").join("1.33.0").join("linux-x64"),
        "путь движка содержит его собственную версию"
    );
    assert_eq!(
        fs::read(root.join("bin/linux-x64/rlm-bsl-index")).unwrap(),
        engine
    );
    assert_eq!(
        downloader.calls(),
        1,
        "доставка движка не тянет за собой ядро"
    );
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn two_sessions_acquire_one_engine_once() {
    let core = b"unica-runtime";
    let core_archive = tar_gz(&[("bin/linux-x64/unica", core)]);
    let engine = b"rlm-bsl-index";
    let engine_archive = tar_gz(&[("bin/linux-x64/rlm-bsl-index", engine)]);
    let manifest = Arc::new(manifest_with_engine(
        &core_archive,
        core,
        &engine_archive,
        engine,
    ));
    let cache = temp_dir("engine-concurrent");
    let downloader = Arc::new(AssetDownloader::new(vec![
        ("unica-runtime-linux-x64.tar.gz", core_archive),
        ("rlm-tools-bsl-runtime-linux-x64.tar.gz", engine_archive),
    ]));
    let installer = Arc::new(RuntimeInstaller::new(
        cache.clone(),
        "0.7.0",
        downloader.clone(),
    ));

    let roots = (0..2)
        .map(|_| {
            let installer = installer.clone();
            let manifest = manifest.clone();
            thread::spawn(move || {
                installer.ensure_artifact(
                    &manifest,
                    "rlm-tools-bsl",
                    HostTarget::LinuxX64,
                    &SilentDownload,
                )
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().expect("installer thread"))
        .collect::<Result<Vec<_>, _>>()
        .expect("обе сессии получают движок");

    assert_eq!(roots[0], roots[1]);
    assert_eq!(downloader.calls(), 1, "две сессии качают движок один раз");
    fs::remove_dir_all(&cache).ok();
}

/// Загрузчик, у которого канал не отвечает.
struct DeadChannel;

impl Downloader for DeadChannel {
    fn download(
        &self,
        url: &str,
        _destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_bootstrap::Result<()> {
        Err(BootstrapError::of(
            Failure::Network,
            format!("failed to download runtime asset {url}: connection refused"),
        ))
    }
}

#[test]
fn a_checksum_mismatch_is_told_apart_from_a_broken_channel() {
    // Убитая посреди старта сессия текста не увидит: различать отказы придётся
    // по коду выхода.
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);

    let poisoned = temp_dir("kind-checksum");
    let checksum = RuntimeInstaller::new(
        poisoned.clone(),
        "0.7.0",
        Arc::new(FakeDownloader::new(b"not a gzip".to_vec())),
    )
    .ensure(&manifest, HostTarget::LinuxX64)
    .expect_err("сумма не сходится");

    let offline = temp_dir("kind-network");
    let network = RuntimeInstaller::new(offline.clone(), "0.7.0", Arc::new(DeadChannel))
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect_err("канал мёртв");

    assert_eq!(checksum.failure(), Failure::Checksum);
    assert_eq!(network.failure(), Failure::Network);
    assert_ne!(checksum.exit_code(), network.exit_code());
    fs::remove_dir_all(&poisoned).ok();
    fs::remove_dir_all(&offline).ok();
}

#[test]
fn a_host_the_release_does_not_serve_is_a_configuration_failure() {
    let error = HostTarget::detect("plan9", "risc-v").expect_err("цель не обслуживается");

    assert_eq!(error.failure(), Failure::Configuration);
    assert_eq!(error.exit_code(), 78, "тот же смысл, что у 78 в launch.sh");
}

#[test]
fn a_manifest_that_does_not_validate_is_a_configuration_failure() {
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    // Версия плагина разошлась с манифестом: поставка собрана не для этого
    // загрузчика, и качать по ней нечего.
    let manifest = manifest_with(&archive, runtime, "0.7.0", "0.7.0");
    let cache = temp_dir("kind-configuration");

    let error = RuntimeInstaller::new(
        cache.clone(),
        "0.8.0",
        Arc::new(FakeDownloader::new(archive)),
    )
    .ensure(&manifest, HostTarget::LinuxX64)
    .expect_err("манифест не для этой версии плагина");

    assert_eq!(error.failure(), Failure::Configuration);
    fs::remove_dir_all(&cache).ok();
}

/// Загрузчик, заглядывающий в журнал попыток посреди загрузки: ровно в этот
/// момент хост и убивает дерево процессов.
struct PeekingDownloader {
    bytes: Vec<u8>,
    cache: PathBuf,
    seen: std::sync::Mutex<Vec<(String, Stage)>>,
}

impl Downloader for PeekingDownloader {
    fn download(
        &self,
        _url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_bootstrap::Result<()> {
        let open = AttemptLog::in_cache(&self.cache)
            .unfinished()
            .expect("read the attempt log");
        *self.seen.lock().expect("seen") = open
            .into_iter()
            .map(|attempt| (attempt.artifact, attempt.stage))
            .collect();
        let mut file = fs::File::create(destination)?;
        file.write_all(&self.bytes)?;
        file.sync_all()?;
        Ok(())
    }
}

#[test]
fn an_attempt_is_already_open_while_the_download_runs() {
    // Запись открывается до стадии, а не после неё: у убитого процесса второго
    // шанса не будет.
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);
    let cache = temp_dir("attempt-open");
    let downloader = Arc::new(PeekingDownloader {
        bytes: archive,
        cache: cache.clone(),
        seen: std::sync::Mutex::new(Vec::new()),
    });

    RuntimeInstaller::new(cache.clone(), "0.7.0", downloader.clone())
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect("install");

    assert_eq!(
        *downloader.seen.lock().expect("seen"),
        vec![("unica".to_owned(), Stage::Download)],
        "посреди загрузки попытка уже записана"
    );
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn a_finished_install_leaves_nothing_unfinished() {
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);
    let cache = temp_dir("attempt-closed");

    RuntimeInstaller::new(
        cache.clone(),
        "0.7.0",
        Arc::new(FakeDownloader::new(archive)),
    )
    .ensure(&manifest, HostTarget::LinuxX64)
    .expect("install");

    assert!(AttemptLog::in_cache(&cache)
        .unfinished()
        .expect("read the attempt log")
        .is_empty());
    fs::remove_dir_all(&cache).ok();
}

#[test]
fn a_failure_the_bootstrap_reported_is_not_repeated_by_the_next_session() {
    // О замеченном отказе сказано кодом выхода и потоком ошибок. Показывать его
    // ещё раз следующей сессией значит показывать одно дважды.
    let runtime = b"unica-runtime";
    let archive = tar_gz(&[("bin/linux-x64/unica", runtime)]);
    let manifest = manifest(&archive, runtime);
    let cache = temp_dir("attempt-reported");

    RuntimeInstaller::new(cache.clone(), "0.7.0", Arc::new(DeadChannel))
        .ensure(&manifest, HostTarget::LinuxX64)
        .expect_err("канал мёртв");

    assert!(AttemptLog::in_cache(&cache)
        .unfinished()
        .expect("read the attempt log")
        .is_empty());
    fs::remove_dir_all(&cache).ok();
}

/// Загрузчик, докладывающий один раз: проверяется провод, а не арифметика.
struct ReportingDownloader {
    bytes: Vec<u8>,
}

impl Downloader for ReportingDownloader {
    fn download(
        &self,
        _url: &str,
        destination: &Path,
        observer: &dyn DownloadObserver,
    ) -> unica_bootstrap::Result<()> {
        let mut file = fs::File::create(destination)?;
        file.write_all(&self.bytes)?;
        file.sync_all()?;
        observer.transferred(self.bytes.len() as u64, Some(self.bytes.len() as u64));
        Ok(())
    }
}

#[derive(Default)]
struct WatchedDelivery {
    seen: std::sync::Mutex<Vec<(u64, Option<u64>)>>,
}

impl DownloadObserver for WatchedDelivery {
    fn transferred(&self, received: u64, total: Option<u64>) {
        self.seen.lock().expect("seen").push((received, total));
    }
}

#[test]
fn an_engine_delivery_reports_its_progress_to_the_caller() {
    // Вызов, дожидающийся движка, показывает ход доставки. Установщик обязан
    // донести наблюдателя до загрузчика, а не потерять его по дороге.
    let core = b"unica-runtime";
    let core_archive = tar_gz(&[("bin/linux-x64/unica", core)]);
    let engine = b"rlm-bsl-index";
    let engine_archive = tar_gz(&[("bin/linux-x64/rlm-bsl-index", engine)]);
    let manifest = manifest_with_engine(&core_archive, core, &engine_archive, engine);
    let cache = temp_dir("engine-progress");
    let watched = WatchedDelivery::default();

    RuntimeInstaller::new(
        cache.clone(),
        "0.7.0",
        Arc::new(ReportingDownloader {
            bytes: engine_archive.clone(),
        }),
    )
    .ensure_artifact(&manifest, "rlm-tools-bsl", HostTarget::LinuxX64, &watched)
    .expect("движок ставится");

    assert_eq!(
        *watched.seen.lock().expect("seen"),
        vec![(
            engine_archive.len() as u64,
            Some(engine_archive.len() as u64)
        )],
        "наблюдатель вызывающего дошёл до загрузчика"
    );
    fs::remove_dir_all(&cache).ok();
}
