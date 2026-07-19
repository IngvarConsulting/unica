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
use unica_bootstrap::{Downloader, HostTarget, RuntimeInstaller, RuntimeManifest};
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

fn manifest(archive: &[u8], runtime: &[u8]) -> RuntimeManifest {
    let archive_hash = sha256(archive);
    let runtime_hash = sha256(runtime);
    let target = |name: &str, executable: &str| {
        serde_json::json!({
            "asset": {
                "name": format!("unica-runtime-{name}.tar.gz"),
                "url": format!(
                    "https://github.com/IngvarConsulting/unica/releases/download/v0.7.0/unica-runtime-{name}.tar.gz"
                ),
                "mediaType": "application/gzip",
                "sha256": archive_hash
            },
            "files": [{"path": executable, "sha256": runtime_hash, "executable": true}],
            "entrypoint": executable
        })
    };
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "pluginVersion": "0.7.0",
        "source": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "commit": COMMIT
        },
        "release": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "tag": "v0.7.0"
        },
        "targets": {
            "darwin-arm64": target("darwin-arm64", "bin/darwin-arm64/unica"),
            "linux-x64": target("linux-x64", "bin/linux-x64/unica"),
            "win-x64": target("win-x64", "bin/win-x64/unica.exe")
        }
    }))
    .expect("manifest fixture")
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
    manifest.targets.get_mut("linux-x64").unwrap().files[0].sha256 = sha256(runtime);
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
