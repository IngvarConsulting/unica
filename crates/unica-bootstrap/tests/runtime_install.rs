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
use unica_bootstrap::{Downloader, HostTarget, RuntimeFile, RuntimeInstaller, RuntimeManifest};
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
    fn download(&self, _url: &str, destination: &Path) -> unica_bootstrap::Result<()> {
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

    RuntimeInstaller::collect(&cache, 1).expect("collect");

    assert!(cache.join(".locks").is_dir());
    assert!(cache.join(".transactions").join("in-flight-a").is_dir());
    assert!(cache.join(".transactions").join("in-flight-b").is_dir());
    fs::remove_dir_all(&cache).ok();
}
