use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::source_target::{MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub(crate) const SOURCE_NAVIGATION_LIMIT_DEFAULT: usize = 20;
pub(crate) const SOURCE_NAVIGATION_LIMIT_MAX: usize = 50;
const CURSOR_MASK: u64 = 0xa93f_4761_c2d8_5be7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceNavigationOperation {
    Resolve,
    Children,
    Locate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceNavigationMode {
    Exact,
    Prefix,
}

impl SourceNavigationMode {
    fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.unwrap_or("exact") {
            "exact" => Ok(Self::Exact),
            "prefix" => Ok(Self::Prefix),
            value => Err(format!(
                "source navigation mode must be `exact` or `prefix`, got `{value}`"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceMatchKind {
    Exact,
    Prefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NavigationCompleteness {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceNodeKind {
    Collection,
    Item,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceNodeAddressability {
    Addressable,
    Unaddressable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum SourceLocation {
    Addressed {
        source_set: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata_path: Option<MetadataAddress>,
        target_kind: TargetKind,
    },
    Unaddressable {
        source_set: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner_metadata_path: Option<MetadataAddress>,
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceResolveCandidate {
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) target_kind: TargetKind,
    pub(crate) display_name: String,
    pub(crate) match_kind: SourceMatchKind,
    pub(crate) location: SourceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceResolveResult {
    pub(crate) candidates: Vec<SourceResolveCandidate>,
    pub(crate) completeness: NavigationCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceNode {
    pub(crate) display_name: String,
    pub(crate) node_kind: SourceNodeKind,
    pub(crate) addressability: SourceNodeAddressability,
    pub(crate) completeness: NavigationCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_kind: Option<TargetKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) location: Option<SourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceChildrenResult {
    pub(crate) children: Vec<SourceNode>,
    pub(crate) completeness: NavigationCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceResolveRequest {
    pub(crate) source_set: String,
    pub(crate) query: String,
    pub(crate) mode: SourceNavigationMode,
    pub(crate) target_kind: Option<TargetKind>,
    pub(crate) limit: usize,
    pub(crate) cursor: Option<String>,
}

/// Why a source path carries no logical address, so a caller can tell a file
/// the provider does not model from one it could not classify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LocateRejection {
    /// The path is not part of the named source set.
    OutsideSourceSet,
    /// The layout is known but nothing addressable owns this file.
    NotAddressable,
    /// The layout matches, but the owning descriptors do not prove it.
    OwnerUnproven,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceLocateResult {
    pub(crate) source_set: String,
    /// Path echoed back relative to the source set, never absolute.
    pub(crate) relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_kind: Option<TargetKind>,
    /// The metadata object that owns the file, which for a module is its owner
    /// rather than the module address itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) owner_metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rejection: Option<LocateRejection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceLocateRequest {
    pub(crate) source_set: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceChildrenRequest {
    pub(crate) source_set: String,
    pub(crate) metadata_path: Option<MetadataAddress>,
    pub(crate) limit: usize,
    pub(crate) cursor: Option<String>,
}

pub(crate) fn invoke(
    operation: SourceNavigationOperation,
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    if cancellation.is_cancelled() {
        return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(
            "source navigation stopped before provider resolution",
        )));
    }
    let (summary, data) = match operation {
        SourceNavigationOperation::Resolve => {
            let result =
                ports.resolve_source_navigation(resolve_request(args)?, context, cancellation)?;
            (
                format!(
                    "source.resolve returned {} canonical candidate(s)",
                    result.candidates.len()
                ),
                serde_json::to_value(result)
                    .map_err(|error| format!("failed to serialize source.resolve: {error}"))?,
            )
        }
        SourceNavigationOperation::Children => {
            let result =
                ports.children_source_navigation(children_request(args)?, context, cancellation)?;
            (
                format!(
                    "source.children returned {} immediate child node(s)",
                    result.children.len()
                ),
                serde_json::to_value(result)
                    .map_err(|error| format!("failed to serialize source.children: {error}"))?,
            )
        }
        SourceNavigationOperation::Locate => {
            let result =
                ports.locate_source_navigation(locate_request(args)?, context, cancellation)?;
            let summary = match result.metadata_path.as_ref() {
                Some(address) => format!("source.locate resolved `{}`", address.as_str()),
                None => "source.locate found no logical address for the path".to_string(),
            };
            (
                summary,
                serde_json::to_value(result)
                    .map_err(|error| format!("failed to serialize source.locate: {error}"))?,
            )
        }
    };
    Ok(HandlerOutcome::with_data(AdapterOutcome::ok(summary), data))
}

fn resolve_request(args: &Map<String, Value>) -> Result<SourceResolveRequest, String> {
    Ok(SourceResolveRequest {
        source_set: required_string(args, "sourceSet")?.to_string(),
        query: required_string(args, "query")?.to_string(),
        mode: SourceNavigationMode::parse(args.get("mode").and_then(Value::as_str))?,
        target_kind: args
            .get("targetKind")
            .and_then(Value::as_str)
            .map(|value| match value {
                "metadataObject" => Ok(TargetKind::MetadataObject),
                "module" => Ok(TargetKind::Module),
                _ => Err(
                    "source navigation `targetKind` must be `metadataObject` or `module`"
                        .to_string(),
                ),
            })
            .transpose()?,
        limit: navigation_limit(args)?,
        cursor: optional_non_empty_string(args, "cursor")?,
    })
}

fn locate_request(args: &Map<String, Value>) -> Result<SourceLocateRequest, String> {
    Ok(SourceLocateRequest {
        source_set: required_string(args, "sourceSet")?.to_string(),
        path: required_string(args, "path")?.to_string(),
    })
}

fn children_request(args: &Map<String, Value>) -> Result<SourceChildrenRequest, String> {
    let metadata_path = optional_non_empty_string(args, "metadataPath")?
        .map(|raw| MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &raw))
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(SourceChildrenRequest {
        source_set: required_string(args, "sourceSet")?.to_string(),
        metadata_path,
        limit: navigation_limit(args)?,
        cursor: optional_non_empty_string(args, "cursor")?,
    })
}

fn required_string<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("source navigation requires non-empty `{name}`"))
}

fn optional_non_empty_string(
    args: &Map<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    args.get(name)
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("source navigation `{name}` must be a non-empty string"))
        })
        .transpose()
}

fn navigation_limit(args: &Map<String, Value>) -> Result<usize, String> {
    let limit = args
        .get("limit")
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| "source navigation `limit` must be an integer".to_string())
        })
        .transpose()?
        .unwrap_or(SOURCE_NAVIGATION_LIMIT_DEFAULT);
    if !(1..=SOURCE_NAVIGATION_LIMIT_MAX).contains(&limit) {
        return Err(format!(
            "source navigation `limit` must be between 1 and {SOURCE_NAVIGATION_LIMIT_MAX}"
        ));
    }
    Ok(limit)
}

pub(crate) fn page_bounds(
    cursor: Option<&str>,
    cursor_key: &str,
    limit: usize,
    total: usize,
) -> Result<(usize, usize, Option<String>), String> {
    let offset = authenticate_cursor(cursor, cursor_key)?;
    page_bounds_from_offset(offset, cursor_key, limit, total)
}

pub(crate) fn authenticate_cursor(cursor: Option<&str>, cursor_key: &str) -> Result<usize, String> {
    match cursor {
        Some(cursor) => decode_cursor(cursor, cursor_key),
        None => Ok(0),
    }
}

pub(crate) fn page_bounds_from_offset(
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

#[cfg(test)]
mod tests {
    use super::page_bounds;

    #[test]
    fn source_navigation_cursor_is_opaque_and_bound_to_the_exact_request() {
        let (_, _, cursor) = page_bounds(None, "children:main:<root>", 2, 5).unwrap();
        let cursor = cursor.expect("truncated page must issue a cursor");

        assert!(cursor.starts_with("nav1-"));
        assert!(!cursor.contains("main"));
        let (start, end, next) = page_bounds(Some(&cursor), "children:main:<root>", 2, 5).unwrap();
        assert_eq!((start, end), (2, 4));
        assert!(next.is_some());
        let error = page_bounds(Some(&cursor), "children:addOn:<root>", 2, 5).unwrap_err();
        assert!(error.contains("does not belong"), "{error}");
    }
}
