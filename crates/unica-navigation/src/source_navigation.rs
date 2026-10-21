use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use unica_address::source_target::{MetadataAddress, TargetKind};
pub use unica_source_contract::location::LocateRejection;
pub use unica_source_contract::location::SourceLocation;

pub const SOURCE_NAVIGATION_LIMIT_DEFAULT: usize = 20;
pub const SOURCE_NAVIGATION_LIMIT_MAX: usize = 50;
const CURSOR_MASK: u64 = 0xa93f_4761_c2d8_5be7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocateResult {
    pub source_set: String,
    /// Path echoed back relative to the source set, never absolute.
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<TargetKind>,
    /// The metadata object that owns the file, which for a module is its owner
    /// rather than the module address itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection: Option<LocateRejection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocateRequest {
    pub source_set: String,
    pub path: String,
}

pub fn authenticate_cursor(cursor: Option<&str>, cursor_key: &str) -> Result<usize, String> {
    match cursor {
        Some(cursor) => decode_cursor(cursor, cursor_key),
        None => Ok(0),
    }
}

pub fn page_bounds_from_offset(
    offset: usize,
    cursor_key: &str,
    limit: usize,
    total: usize,
) -> Result<(usize, usize, Option<String>), String> {
    if offset > total {
        return Err("source navigation cursor is outside the current result set".to_string());
    }
    let end = offset.saturating_add(limit).min(total);
    let next = (end < total).then(|| encode_cursor(end, cursor_key));
    Ok((offset, end, next))
}

fn encode_cursor(offset: usize, cursor_key: &str) -> String {
    let offset = u64::try_from(offset).expect("navigation offset must fit u64");
    let obscured = offset ^ CURSOR_MASK;
    format!(
        "nav1-{obscured:016x}-{:016x}",
        cursor_checksum(offset, cursor_key)
    )
}

fn decode_cursor(cursor: &str, cursor_key: &str) -> Result<usize, String> {
    let mut parts = cursor.split('-');
    let valid_prefix = parts.next() == Some("nav1");
    let obscured = parts
        .next()
        .and_then(|part| u64::from_str_radix(part, 16).ok());
    let checksum = parts
        .next()
        .and_then(|part| u64::from_str_radix(part, 16).ok());
    if !valid_prefix || parts.next().is_some() || obscured.is_none() || checksum.is_none() {
        return Err("source navigation cursor is invalid".to_string());
    }
    let offset = obscured.expect("checked") ^ CURSOR_MASK;
    if checksum.expect("checked") != cursor_checksum(offset, cursor_key) {
        return Err("source navigation cursor does not belong to this request".to_string());
    }
    usize::try_from(offset).map_err(|_| "source navigation cursor is invalid".to_string())
}

/// Per-process secret behind every navigation cursor. It gives the same
/// guarantee the resource-snapshot cursor already carries: a continuation
/// token is evidence issued by this running application, not a reversible
/// encoding anyone can recompute. `DefaultHasher` could not carry it — it
/// hashes with a fixed key and an algorithm std does not promise to keep
/// stable across releases.
fn cursor_secret() -> &'static str {
    static SECRET: OnceLock<String> = OnceLock::new();
    SECRET.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

fn cursor_checksum(offset: u64, cursor_key: &str) -> u64 {
    let mut hasher = Sha256::new();
    for component in [
        cursor_secret(),
        "unica-source-navigation-v1",
        &offset.to_string(),
        cursor_key,
    ] {
        hasher.update(component.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    u64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("a sha256 digest is always 32 bytes"),
    )
}
