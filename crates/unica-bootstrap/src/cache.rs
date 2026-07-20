use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::archive::{extract_verified_tar_gz, sha256_file, verify_runtime_files};
use crate::download::Downloader;
use crate::error::{BootstrapError, Result};
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
    plugin_version: String,
    target: String,
    manifest_sha256: String,
}

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
        manifest.validate(&self.plugin_version)?;
        let target = manifest.target(host)?;
        let manifest_sha256 = manifest_sha256(manifest)?;
        let final_root = self
            .cache_root
            .join(&self.plugin_version)
            .join(host.as_str());

        fs::create_dir_all(self.cache_root.join(".locks"))?;
        let lock_path = self.cache_root.join(".locks").join(format!(
            "{}-{}.lock",
            self.plugin_version,
            host.as_str()
        ));
        let lock = open_lock(&lock_path)?;
        lock.lock_exclusive().map_err(|error| {
            BootstrapError::new(format!(
                "failed to lock runtime cache {}: {error}",
                lock_path.display()
            ))
        })?;

        if ready_installation(
            &final_root,
            target,
            &self.plugin_version,
            host,
            &manifest_sha256,
        )? {
            return Ok(installation(final_root, target));
        }

        let transaction_root = self.cache_root.join(".transactions").join(format!(
            "{}-{}-{}",
            self.plugin_version,
            host.as_str(),
            Uuid::new_v4()
        ));
        let archive_path = transaction_root.join("runtime.tar.gz");
        let staged_root = transaction_root.join("runtime");
        fs::create_dir_all(&staged_root)?;

        let result = (|| {
            self.downloader.download(&target.asset.url, &archive_path)?;
            let actual_archive_sha = sha256_file(&archive_path)?;
            if actual_archive_sha != target.asset.sha256 {
                return Err(BootstrapError::new(format!(
                    "runtime archive sha256 {actual_archive_sha} != expected {}",
                    target.asset.sha256
                )));
            }
            extract_verified_tar_gz(&archive_path, &staged_root, &target.files)?;
            write_ready_marker(&staged_root, &self.plugin_version, host, &manifest_sha256)?;

            if final_root.exists() {
                let quarantine = final_root.with_file_name(format!(
                    "{}.invalid-{}",
                    host.as_str(),
                    Uuid::new_v4()
                ));
                fs::rename(&final_root, &quarantine)?;
                fs::remove_dir_all(quarantine)?;
            }
            if let Some(parent) = final_root.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&staged_root, &final_root)?;
            Ok(installation(final_root.clone(), target))
        })();

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
        result
    }
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
    plugin_version: &str,
    host: HostTarget,
    manifest_sha256: &str,
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
    if marker.plugin_version != plugin_version
        || marker.target != host.as_str()
        || marker.manifest_sha256 != manifest_sha256
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
    plugin_version: &str,
    host: HostTarget,
    manifest_sha256: &str,
) -> Result<()> {
    let marker = ReadyMarker {
        plugin_version: plugin_version.to_string(),
        target: host.as_str().to_string(),
        manifest_sha256: manifest_sha256.to_string(),
    };
    let path = root.join(".ready.json");
    let file = File::create(&path)?;
    serde_json::to_writer_pretty(&file, &marker)?;
    file.sync_all()?;
    Ok(())
}

fn manifest_sha256(manifest: &RuntimeManifest) -> Result<String> {
    let bytes = serde_json::to_vec(manifest)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn installation(root: PathBuf, target: &TargetRuntime) -> RuntimeInstallation {
    RuntimeInstallation {
        entrypoint: root.join(&target.entrypoint),
        root,
    }
}
