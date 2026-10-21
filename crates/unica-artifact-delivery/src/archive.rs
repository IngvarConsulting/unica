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

/// Распаковать `zip` издателя под тем же контролем, что и `tar.gz`.
///
/// Каталоги пропускаются, ссылки и зашифрованные записи отклоняются, повтор
/// пути — тоже. Состав сверяется с манифестом целиком: лишний или пропавший
/// файл не даёт установке стать готовой.
pub fn extract_verified_zip(
    archive_path: &Path,
    destination: &Path,
    expected_files: &[RuntimeFile],
) -> Result<()> {
    fs::create_dir_all(destination)?;
    let mut archive = zip::ZipArchive::new(File::open(archive_path)?)
        .map_err(|error| BootstrapError::new(format!("failed to read runtime archive: {error}")))?;
    // Читатель сводит записи с одним именем в одну, и повтор пропал бы молча.
    // Число записей, объявленное самим архивом, его выдаёт.
    if declared_zip_entries(archive_path)? != archive.len() {
        return Err(BootstrapError::new(
            "duplicate runtime archive file: the archive declares more entries than it names",
        ));
    }
    let mut extracted = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            BootstrapError::new(format!("failed to read runtime archive entry: {error}"))
        })?;
        let name = entry.name().to_string();
        validate_archive_path(Path::new(&name))?;
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() || entry.encrypted() || !entry.is_file() {
            return Err(BootstrapError::new(format!(
                "unsupported runtime archive entry type for {name}"
            )));
        }
        if !extracted.insert(name.clone()) {
            return Err(BootstrapError::new(format!(
                "duplicate runtime archive file: {name}"
            )));
        }
        let target = destination.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| {
                BootstrapError::new(format!(
                    "failed to extract runtime archive file {name}: {error}"
                ))
            })?;
        std::io::copy(&mut entry, &mut output).map_err(|error| {
            BootstrapError::new(format!(
                "failed to extract runtime archive file {name}: {error}"
            ))
        })?;
    }

    verify_runtime_files(destination, expected_files, Some(&extracted))
}

/// Сколько записей объявляет запись конца центрального каталога.
///
/// Архив zip64 отклоняется: поставки движков малы, а проверить его счёт
/// этой записью нельзя.
fn declared_zip_entries(archive_path: &Path) -> Result<usize> {
    const SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const RECORD: usize = 22;
    let bytes = fs::read(archive_path)?;
    let malformed =
        || BootstrapError::new("failed to read runtime archive: no end of central directory");
    if bytes.len() < RECORD {
        return Err(malformed());
    }
    // Запись стоит в конце и может нести комментарий до 65535 байт.
    let earliest = bytes.len().saturating_sub(RECORD + usize::from(u16::MAX));
    // Настоящая запись кончается ровно там, где кончается её комментарий. Если
    // так сходятся две записи, вторая спрятана в комментарии первой, и какой из
    // них верить, решить нельзя — такой архив не ставится.
    let candidates = (earliest..=bytes.len() - RECORD)
        .filter(|&offset| {
            let comment = usize::from(u16::from_le_bytes([bytes[offset + 20], bytes[offset + 21]]));
            bytes[offset..offset + 4] == SIGNATURE && offset + RECORD + comment == bytes.len()
        })
        .collect::<Vec<_>>();
    let start = match candidates.as_slice() {
        [start] => *start,
        [] => return Err(malformed()),
        _ => {
            return Err(BootstrapError::new(
                "unsupported runtime archive: the end of central directory is ambiguous",
            ))
        }
    };
    let total = u16::from_le_bytes([bytes[start + 10], bytes[start + 11]]);
    if total == u16::MAX {
        return Err(BootstrapError::new(
            "unsupported runtime archive: zip64 archives are not delivered",
        ));
    }
    Ok(usize::from(total))
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
