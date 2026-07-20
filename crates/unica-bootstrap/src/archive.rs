use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use crate::error::{BootstrapError, Result};
use crate::manifest::RuntimeFile;
use crate::platform::set_executable;

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn extract_verified_tar_gz(
    archive_path: &Path,
    destination: &Path,
    expected_files: &[RuntimeFile],
) -> Result<()> {
    fs::create_dir_all(destination)?;
    let archive_file = File::open(archive_path)?;
    let decoder = GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    let mut extracted = BTreeSet::new();

    let entries = archive
        .entries()
        .map_err(|error| BootstrapError::new(format!("failed to read runtime archive: {error}")))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| {
            BootstrapError::new(format!("failed to read runtime archive entry: {error}"))
        })?;
        let path = entry
            .path()
            .map_err(|error| BootstrapError::new(format!("unsafe archive path: {error}")))?;
        validate_archive_path(&path)?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        if !entry_type.is_file() {
            return Err(BootstrapError::new(format!(
                "unsupported runtime archive entry type for {}",
                path.display()
            )));
        }
        let path_string = path.to_string_lossy().into_owned();
        if !extracted.insert(path_string.clone()) {
            return Err(BootstrapError::new(format!(
                "duplicate runtime archive file: {path_string}"
            )));
        }
        let unpacked = entry.unpack_in(destination).map_err(|error| {
            BootstrapError::new(format!(
                "failed to extract runtime archive file {path_string}: {error}"
            ))
        })?;
        if !unpacked {
            return Err(BootstrapError::new(format!(
                "unsafe archive path: {path_string}"
            )));
        }
    }

    verify_runtime_files(destination, expected_files, Some(&extracted))
}

pub fn verify_runtime_files(
    root: &Path,
    expected_files: &[RuntimeFile],
    extracted_files: Option<&BTreeSet<String>>,
) -> Result<()> {
    let expected = expected_files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    if let Some(actual) = extracted_files {
        if actual != &expected {
            return Err(BootstrapError::new(format!(
                "runtime archive file set {:?} != expected {:?}",
                actual, expected
            )));
        }
    }

    for file in expected_files {
        let path = root.join(&file.path);
        if !path.is_file() {
            return Err(BootstrapError::new(format!(
                "runtime file is missing: {}",
                file.path
            )));
        }
        let actual = sha256_file(&path)?;
        if actual != file.sha256 {
            return Err(BootstrapError::new(format!(
                "runtime file {} sha256 {} != expected {}",
                file.path, actual, file.sha256
            )));
        }
        set_executable(&path, file.executable)?;
    }
    Ok(())
}

fn validate_archive_path(path: &Path) -> Result<()> {
    let unsafe_path = path.as_os_str().is_empty()
        || path.is_absolute()
        || path.to_string_lossy().contains('\\')
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
    if unsafe_path {
        return Err(BootstrapError::new(format!(
            "unsafe archive path: {}",
            path.display()
        )));
    }
    Ok(())
}
