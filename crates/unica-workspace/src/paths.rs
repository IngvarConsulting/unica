//! Host path observations and canonical workspace path identity.
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub use crate::platform::filesystem::{
    encode_relative_source_path, host_directory_child_identity, host_directory_child_names_equal,
    host_filesystem_case_sensitive, host_path_components_equal, host_path_text,
    is_foreign_absolute_path, is_link_loop_error, metadata_is_link_or_reparse_point,
    path_lock_identity, path_starts_with_host_root, provider_state_path_identity,
    stable_path_identity_bytes, strip_windows_extended_length_prefix, EncodedRelativePath,
};

/// The generated cache. It sits inside the source root but is not source, so
/// source planning and lexical search exclude it. Anything else that
/// descends the whole tree owes the same skip and reads the name from here
/// rather than spelling it again.
pub const GENERATED_DIR_NAME: &str = ".build";

pub fn normalize_path_identity(path: &Path) -> Result<PathBuf, String> {
    let _probe = crate::probe_timing::span(9);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| format!("failed to determine current directory: {err}"))?
            .join(path)
    };
    let prepared = strip_windows_extended_length_prefix(&absolute);
    let canonical = canonicalize_existing_ancestor(&prepared)?;
    Ok(strip_windows_extended_length_prefix(&canonical))
}

fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let canonical = fs::canonicalize(ancestor).map_err(|err| {
                    format!(
                        "failed to resolve existing path ancestor {}: {err}",
                        ancestor.display()
                    )
                })?;
                let remainder = path.strip_prefix(ancestor).map_err(|err| {
                    format!(
                        "failed to preserve path suffix for {}: {err}",
                        path.display()
                    )
                })?;
                return Ok(normalize_lexically(&canonical.join(remainder)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to inspect path ancestor {}: {error}",
                    ancestor.display()
                ));
            }
        }
    }

    Ok(normalize_lexically(path))
}

pub fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
pub fn metadata_change_time(metadata: &std::fs::Metadata) -> Option<std::time::SystemTime> {
    crate::platform::filesystem::metadata_change_time(metadata)
}

/// Normalize the parent while retaining the exact final component name.
pub fn normalize_leaf_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to determine current directory: {error}"))?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", absolute.display()))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", absolute.display()))?;
    Ok(normalize_path_identity(parent)?.join(name))
}

pub use crate::platform::filesystem::is_file_sharing_violation;
