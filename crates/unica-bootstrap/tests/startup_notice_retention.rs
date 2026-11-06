//! A failed real launcher must not acknowledge a notice the MCP client never received.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use unica_artifact_delivery::{
    AttemptLog, AttemptSubject, DownloadObserver, Downloader, HostTarget, RuntimeInstaller,
    RuntimeManifest, Stage,
};
use uuid::Uuid;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("unica-notice-retention-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Archive(Vec<u8>);

impl Downloader for Archive {
    fn download(
        &self,
        _url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_artifact_delivery::Result<()> {
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::write(destination, &self.0)?;
        Ok(())
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn warm_runtime(plugin: &Path, cache: &Path, executable: &[u8]) -> RuntimeManifest {
    let host = HostTarget::current().unwrap();
    let entrypoint = format!(
        "bin/{}/unica{}",
        host.as_str(),
        std::env::consts::EXE_SUFFIX
    );
    let mut archive = Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    let mut header = Header::new_gnu();
    header.set_size(executable.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    archive
        .append_data(&mut header, &entrypoint, executable)
        .unwrap();
    let bytes = archive.into_inner().unwrap().finish().unwrap();
    let version = env!("CARGO_PKG_VERSION");
    let targets = ["darwin-arm64", "linux-x64", "win-x64"]
        .into_iter()
        .map(|target| {
            let suffix = if target == "win-x64" { ".exe" } else { "" };
            let path = format!("bin/{target}/unica{suffix}");
            (target.to_owned(), serde_json::json!({
                "asset": {
                    "name": format!("unica-runtime-{target}.tar.gz"),
                    "url": format!("https://github.com/IngvarConsulting/unica/releases/download/v{version}/unica-runtime-{target}.tar.gz"),
                    "mediaType": "application/gzip",
                    "sha256": digest(&bytes)
                },
                "files": [{"path": path, "sha256": digest(executable), "executable": true}],
                "entrypoint": path
            }))
        })
        .collect::<serde_json::Map<_, _>>();
    let manifest: RuntimeManifest = serde_json::from_value(serde_json::json!({
        "schemaVersion": 2,
        "pluginVersion": version,
        "source": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "commit": "0123456789abcdef0123456789abcdef01234567"
        },
        "release": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "tag": format!("v{version}")
        },
        "artifacts": {"unica": {
            "version": version,
            "role": "core",
            "targets": targets
        }}
    }))
    .unwrap();
    fs::create_dir_all(plugin).unwrap();
    fs::write(
        plugin.join("runtime-manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    RuntimeInstaller::new(cache.to_owned(), version, Arc::new(Archive(bytes)))
        .ensure(&manifest, host)
        .expect("prepare a complete cache without network access");
    manifest
}

fn assert_notice_survives_failed_launcher(executable: &[u8]) {
    let scratch = Scratch::new();
    let plugin = scratch.0.join("plugin");
    let cache = scratch.0.join("cache");
    let manifest = warm_runtime(&plugin, &cache, executable);
    let partial = scratch.0.join("interrupted-download.partial");
    let received = b"bytes received before the interrupted installation";
    fs::write(&partial, received).unwrap();
    let log = AttemptLog::in_cache(&cache);
    let unfinished = log
        .open(AttemptSubject {
            artifact: "unica".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            target: HostTarget::current().unwrap().as_str().into(),
            url: manifest.artifacts["unica"].targets[HostTarget::current().unwrap().as_str()]
                .asset
                .url
                .clone(),
            partial: partial.clone(),
        })
        .unwrap();
    unfinished.reached(Stage::Verify).unwrap();
    drop(unfinished); // An interrupted installer did not record a terminal outcome.
    let before = log.unfinished().unwrap();
    assert_eq!(before.len(), 1);
    let old_id = before[0].id.clone();

    // Neither failed attempt gets as far as an MCP response. In particular,
    // attempting a second launch must not consume the first launch's notice.
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"))
            .args(["run", "--plugin-root"])
            .arg(&plugin)
            .env("UNICA_RUNTIME_CACHE_DIR", &cache)
            .env("UNICA_ARTIFACT_CACHE", &cache)
            .env("UNICA_PROVIDER_STATE_DIR", scratch.0.join("state"))
            .env_remove("UNICA_STARTUP_NOTICE")
            .env_remove("UNICA_STARTUP_NOTICE_ACK")
            .output()
            .expect("launch the real bootstrap executable");
        assert!(!output.status.success(), "the fixture must fail before MCP");
        let after = log.unfinished().unwrap();
        assert_eq!(
            after.len(),
            1,
            "a failed launcher acknowledged an undelivered startup notice; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(after[0].id, old_id);
        assert_eq!(after[0].stage, Stage::Verify);
        assert_eq!(after[0].received, received.len() as u64);
        assert_eq!(fs::read(&partial).unwrap(), received);
    }
}

/// rule: INV.PKG.STARTUP-NOTICE-RETENTION
#[test]
fn notice_survives_failure_to_execute_the_warmed_runtime() {
    assert_notice_survives_failed_launcher(b"not an executable image\0");
}

/// rule: INV.PKG.STARTUP-NOTICE-RETENTION
#[test]
fn notice_survives_a_real_runtime_process_exiting_before_mcp() {
    // A second native bootstrap is a real executable, but with no arguments it
    // rejects its CLI invocation before opening any MCP transport. It does not
    // stand in for an MCP server or synthesize a protocol response.
    let executable = fs::read(env!("CARGO_BIN_EXE_unica-bootstrap")).unwrap();
    assert_notice_survives_failed_launcher(&executable);
}
