//! The catalog of an installation, built once per user.
//!
//! The file lives under the user's Unica state root, which every worktree
//! of the user shares: `<state>/platform-types/<build>/<generator>.json.deflate`.
//! It names the help containers it was built from (file name and length):
//! one build ships one help wherever it is installed, so installations of
//! the build share the file; changed help or a new generator builds a new
//! catalog instead of serving the old one.

use crate::model::PlatformCatalog;
use crate::{build_catalog, GENERATOR};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use unica_documentation_sources::platform_help::corpus::read_markup_pages;

/// The help containers of an installation: the Russian syntax help and the
/// English one (`root` in the platform's naming).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Containers {
    pub russian: PathBuf,
    pub english: PathBuf,
}

impl Containers {
    pub fn of_installation(root: &Path) -> Self {
        Self {
            russian: root.join("shcntx_ru.hbk"),
            english: root.join("shcntx_root.hbk"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CatalogError {}

/// What a cached catalog was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fingerprint {
    containers: Vec<(String, u64)>,
}

#[derive(Serialize, Deserialize)]
struct CachedCatalog {
    fingerprint: Fingerprint,
    catalog: PlatformCatalog,
}

/// Where the catalog of `platform` lives under `state_root`.
pub fn catalog_path(state_root: &Path, platform: &str) -> PathBuf {
    state_root
        .join("platform-types")
        .join(platform)
        .join(format!("{GENERATOR}.json.deflate"))
}

/// The catalog of `platform` from its cache under `state_root`, built from
/// `containers` and cached when there is none or it is stale.
pub fn load_or_build(
    platform: &str,
    containers: &Containers,
    state_root: &Path,
) -> Result<PlatformCatalog, CatalogError> {
    // The build names a directory of the state root and nothing else.
    let valid = !platform.is_empty()
        && !platform.starts_with('.')
        && platform
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    if !valid {
        return Err(CatalogError(format!(
            "{platform:?} is not a platform build"
        )));
    }
    let fingerprint = fingerprint(containers)?;
    let path = catalog_path(state_root, platform);
    if let Some(cached) = read_cached(&path) {
        if cached.fingerprint == fingerprint && cached.catalog.generator == GENERATOR {
            return Ok(cached.catalog);
        }
    }
    let read = |path: &Path| {
        let bytes = std::fs::read(path)
            .map_err(|error| CatalogError(format!("{}: {error}", path.display())))?;
        read_markup_pages(&bytes, "objects/")
            .map_err(|error| CatalogError(format!("{}: {error:?}", path.display())))
    };
    let cached = CachedCatalog {
        fingerprint,
        catalog: build_catalog(
            platform,
            &read(&containers.russian)?,
            &read(&containers.english)?,
        ),
    };
    // A catalog that cannot be kept is still the answer; the next call
    // builds it again.
    let _ = write_cached(&path, &cached);
    Ok(cached.catalog)
}

fn fingerprint(containers: &Containers) -> Result<Fingerprint, CatalogError> {
    [&containers.russian, &containers.english]
        .into_iter()
        .map(|path| {
            let meta = std::fs::metadata(path)
                .map_err(|error| CatalogError(format!("{}: {error}", path.display())))?;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok((name, meta.len()))
        })
        .collect::<Result<_, _>>()
        .map(|containers| Fingerprint { containers })
}

fn read_cached(path: &Path) -> Option<CachedCatalog> {
    let file = std::fs::File::open(path).ok()?;
    let mut json = Vec::new();
    DeflateDecoder::new(file).read_to_end(&mut json).ok()?;
    serde_json::from_slice(&json).ok()
}

/// Written to a sibling file and renamed, so a reader never sees half.
fn write_cached(path: &Path, cached: &CachedCatalog) -> Result<(), CatalogError> {
    let failed = |error: std::io::Error| CatalogError(format!("{}: {error}", path.display()));
    let parent = path.parent().expect("a catalog path has a directory");
    std::fs::create_dir_all(parent).map_err(failed)?;
    // One partial file per writer: two processes or threads never write
    // into the same one.
    static WRITERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let partial = parent.join(format!(
        ".{}.{}.{}.partial",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("catalog"),
        std::process::id(),
        WRITERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    let written = (|| {
        let mut encoder = DeflateEncoder::new(
            std::fs::File::create(&partial).map_err(failed)?,
            Compression::default(),
        );
        let json = serde_json::to_vec(cached)
            .map_err(|error| CatalogError(format!("{}: {error}", path.display())))?;
        encoder.write_all(&json).map_err(failed)?;
        encoder
            .finish()
            .map_err(failed)?
            .sync_all()
            .map_err(failed)?;
        std::fs::rename(&partial, path).map_err(failed)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalog_is_kept_per_user_build_and_generator_and_stale_when_the_help_changes() {
        let state = tempfile::tempdir().unwrap();
        let help = tempfile::tempdir().unwrap();
        let containers = Containers::of_installation(help.path());
        std::fs::write(&containers.russian, b"ru").unwrap();
        std::fs::write(&containers.english, b"en").unwrap();

        // The place depends on the user's state, the build and the
        // generator only: every worktree of the user finds the same file.
        let path = catalog_path(state.path(), "8.3.27.2074");
        assert_eq!(
            path,
            state.path().join(format!(
                "platform-types/8.3.27.2074/{GENERATOR}.json.deflate"
            ))
        );

        let catalog = crate::build_catalog("8.3.27.2074", &[], &[]);
        let cached = CachedCatalog {
            fingerprint: fingerprint(&containers).unwrap(),
            catalog: catalog.clone(),
        };
        write_cached(&path, &cached).unwrap();
        let read = read_cached(&path).unwrap();
        assert_eq!(read.catalog, catalog);
        assert_eq!(read.fingerprint, fingerprint(&containers).unwrap());
        // The cached catalog answers without reading the help.
        assert_eq!(
            load_or_build("8.3.27.2074", &containers, state.path()).unwrap(),
            catalog
        );

        // A platform reinstalled into the same directory changes the help:
        // the cached catalog no longer answers, a new one is built from it.
        std::fs::write(&containers.russian, b"another help").unwrap();
        assert_ne!(read.fingerprint, fingerprint(&containers).unwrap());
        assert!(load_or_build("8.3.27.2074", &containers, state.path()).is_err());
        // A build is a directory name, never a path out of the state root.
        assert!(load_or_build("../x", &containers, state.path()).is_err());
    }
}
