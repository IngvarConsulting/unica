use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use tar::{Builder, EntryType, Header};
use unica_artifact_delivery::{
    BootstrapError, DownloadObserver, Downloader, HostTarget, RuntimeInstaller, RuntimeManifest,
    SilentDownload,
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

fn manifest_with_engine(
    core_archive: &[u8],
    core_file: &[u8],
    engine_archive: &[u8],
    engine_file: &[u8],
) -> RuntimeManifest {
    let mut manifest = manifest(core_archive, core_file);
    let engine_hash = sha256(engine_archive);
    let file_hash = sha256(engine_file);
    // Движок приезжает из тулчейна под своим тегом и своим именем: копия в
    // выпуске плагина стоила 242 МБ и не давала ничего.
    let target = |name: &str| {
        serde_json::json!({
            "asset": {
                "name": format!("rlm-tools-bsl-{name}.tar.gz"),
                "url": format!(
                    "https://github.com/IngvarConsulting/unica-toolchain/releases/download/rlm-tools-bsl-v1.33.0-build.3/rlm-tools-bsl-{name}.tar.gz"
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
}

impl Downloader for AssetDownloader {
    fn download(
        &self,
        url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_artifact_delivery::Result<()> {
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

#[test]
fn prefetch_cli_reports_success_and_artifact_disk_failures() {
    let host = HostTarget::current().expect("supported test host");
    let core = b"unica-runtime";
    let core_path = format!(
        "bin/{}/unica{}",
        host.as_str(),
        std::env::consts::EXE_SUFFIX
    );
    let engine = b"rlm-bsl-index";
    let engine_path = format!("bin/{}/rlm-bsl-index", host.as_str());
    let core_archive = tar_gz(&[(&core_path, core)]);
    let engine_archive = tar_gz(&[(&engine_path, engine)]);
    let mut manifest = manifest_with_engine(&core_archive, core, &engine_archive, engine);
    manifest.plugin_version = env!("CARGO_PKG_VERSION").to_owned();
    manifest.release.tag = format!("v{}", manifest.plugin_version);
    for target in manifest
        .artifacts
        .get_mut("unica")
        .unwrap()
        .targets
        .values_mut()
    {
        target.asset.url = target
            .asset
            .url
            .replace("/v0.7.0/", &format!("/{}/", manifest.release.tag));
    }
    let core_asset = format!("unica-runtime-{}.tar.gz", host.as_str());
    let engine_asset = format!("rlm-tools-bsl-{}.tar.gz", host.as_str());
    let downloader = Arc::new(AssetDownloader::new(vec![
        (&core_asset, core_archive),
        (&engine_asset, engine_archive),
    ]));
    let scratch = temp_dir("prefetch-cli");
    let cache = scratch.join("cache");
    let plugin = scratch.join("plugin");
    fs::create_dir_all(&plugin).unwrap();
    fs::write(
        plugin.join("runtime-manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let installer = RuntimeInstaller::new(cache.clone(), env!("CARGO_PKG_VERSION"), downloader);
    let delivered = installer
        .prefetch(&manifest, host, &SilentDownload)
        .expect("warm test cache");
    let run_cli = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"))
            .args(["prefetch", "--plugin-root"])
            .arg(&plugin)
            .env("UNICA_RUNTIME_CACHE_DIR", &cache)
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("NO_PROXY", "")
            .output()
            .expect("run prefetch CLI")
    };
    let output = run_cli();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    for item in &delivered {
        assert!(
            stderr.contains(&format!("{} {} cached", item.artifact, item.version)),
            "{stderr}"
        );
    }
    for item in &delivered {
        fs::remove_dir_all(&item.root).unwrap();
        let partial = cache.join(".partial").join(&item.artifact);
        if partial.exists() {
            fs::remove_dir_all(&partial).unwrap();
        }
        fs::write(&partial, b"a file blocks the required directory").unwrap();
        let output = run_cli();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            output.status.code(),
            Some(74),
            "{}: {stderr}",
            item.artifact
        );
        assert!(stderr.contains("reason: disk"), "{stderr}");
        fs::remove_file(partial).unwrap();
        installer
            .ensure_artifact(&manifest, &item.artifact, host, &SilentDownload)
            .unwrap();
    }
    fs::remove_dir_all(scratch).unwrap();
}
