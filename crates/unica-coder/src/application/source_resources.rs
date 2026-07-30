use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::source_resources::{
    ResourceScope, SourceResourceError, SourceResourceErrorCode, SOURCE_READ_LIMIT_MAX,
    SOURCE_RESOURCE_PAGE_LIMIT_MAX,
};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceResourceOperation {
    Resources,
    Read,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenResourceSnapshotRequest {
    pub(crate) target: SourceTarget,
    pub(crate) scope: ResourceScope,
    pub(crate) limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContinueResourceSnapshotRequest {
    pub(crate) snapshot_id: String,
    pub(crate) cursor: String,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceResourcesRequest {
    Open(OpenResourceSnapshotRequest),
    Continue(ContinueResourceSnapshotRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceReadRequest {
    pub(crate) snapshot_id: String,
    pub(crate) resource_id: String,
    pub(crate) offset: usize,
    pub(crate) limit: usize,
}

pub(crate) fn invoke(
    operation: SourceResourceOperation,
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let (summary, data) = match operation {
        SourceResourceOperation::Resources => {
            let page = ports
                .source_resources(resources_request(args)?, context, cancellation)
                .map_err(|error| error.to_string())?;
            (
                format!(
                    "source.resources returned {} resource(s)",
                    page.resources.len()
                ),
                serde_json::to_value(page)
                    .map_err(|error| format!("failed to serialize source.resources: {error}"))?,
            )
        }
        SourceResourceOperation::Read => {
            let read = ports
                .read_source_resource(read_request(args)?, context, cancellation)
                .map_err(|error| error.to_string())?;
            (
                format!("source.read returned {} byte(s)", read.length),
                serde_json::to_value(read)
                    .map_err(|error| format!("failed to serialize source.read: {error}"))?,
            )
        }
    };
    Ok(HandlerOutcome::with_data(AdapterOutcome::ok(summary), data))
}

fn resources_request(args: &Map<String, Value>) -> Result<SourceResourcesRequest, String> {
    let snapshot_id = optional_string(args, "snapshotId")?;
    let cursor = optional_string(args, "cursor")?;
    let has_open_fields = args.contains_key("sourceSet")
        || args.contains_key("metadataPath")
        || args.contains_key("scope");
    match (snapshot_id, cursor, has_open_fields) {
        (Some(snapshot_id), Some(cursor), false) => Ok(SourceResourcesRequest::Continue(
            ContinueResourceSnapshotRequest {
                snapshot_id,
                cursor,
                limit: optional_bounded_limit(args, SOURCE_RESOURCE_PAGE_LIMIT_MAX)?,
            },
        )),
        (None, None, _) => {
            let source_set = required_string(args, "sourceSet")?;
            let metadata_path = optional_string(args, "metadataPath")?
                .map(|raw| MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &raw))
                .transpose()
                .map_err(|_| {
                    SourceResourceError::new(
                        SourceResourceErrorCode::InvalidRequest,
                        "metadataPath is not a valid logical source address",
                    )
                    .to_string()
                })?;
            let scope = match args.get("scope").and_then(Value::as_str).unwrap_or("self") {
                "self" => ResourceScope::SelfOnly,
                "aggregate" => ResourceScope::Aggregate,
                "registrations" => ResourceScope::Registrations,
                _ => {
                    return Err(SourceResourceError::new(
                        SourceResourceErrorCode::InvalidRequest,
                        "scope must be self, aggregate, or registrations",
                    )
                    .to_string())
                }
            };
            Ok(SourceResourcesRequest::Open(OpenResourceSnapshotRequest {
                target: SourceTarget {
                    source_set,
                    metadata_path,
                },
                scope,
                limit: bounded_limit(
                    args,
                    SOURCE_RESOURCE_PAGE_LIMIT_MAX,
                    SOURCE_RESOURCE_PAGE_LIMIT_MAX,
                )?,
            }))
        }
        _ => Err(SourceResourceError::new(
            SourceResourceErrorCode::InvalidRequest,
            "snapshot continuation requires snapshotId and cursor without open-snapshot fields",
        )
        .to_string()),
    }
}

fn optional_bounded_limit(
    args: &Map<String, Value>,
    maximum: usize,
) -> Result<Option<usize>, String> {
    args.get("limit")
        .map(|_| bounded_limit(args, maximum, maximum))
        .transpose()
}

fn read_request(args: &Map<String, Value>) -> Result<SourceReadRequest, String> {
    let offset = args
        .get("offset")
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    SourceResourceError::new(
                        SourceResourceErrorCode::InvalidRequest,
                        "offset must be a non-negative byte integer",
                    )
                    .to_string()
                })
        })
        .transpose()?
        .unwrap_or(0);
    Ok(SourceReadRequest {
        snapshot_id: required_string(args, "snapshotId")?,
        resource_id: required_string(args, "resourceId")?,
        offset,
        limit: bounded_limit(args, SOURCE_READ_LIMIT_MAX, SOURCE_READ_LIMIT_MAX)?,
    })
}

fn bounded_limit(
    args: &Map<String, Value>,
    default: usize,
    maximum: usize,
) -> Result<usize, String> {
    let limit = args
        .get("limit")
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    SourceResourceError::new(
                        SourceResourceErrorCode::InvalidRequest,
                        "limit must be a positive byte integer",
                    )
                    .to_string()
                })
        })
        .transpose()?
        .unwrap_or(default);
    if !(1..=maximum).contains(&limit) {
        return Err(SourceResourceError::new(
            SourceResourceErrorCode::LimitExceeded,
            format!("limit must be between 1 and {maximum}"),
        )
        .to_string());
    }
    Ok(limit)
}

fn required_string(args: &Map<String, Value>, name: &str) -> Result<String, String> {
    optional_string(args, name)?.ok_or_else(|| {
        SourceResourceError::new(
            SourceResourceErrorCode::InvalidRequest,
            format!("{name} must be a non-empty string"),
        )
        .to_string()
    })
}

fn optional_string(args: &Map<String, Value>, name: &str) -> Result<Option<String>, String> {
    args.get(name)
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    SourceResourceError::new(
                        SourceResourceErrorCode::InvalidRequest,
                        format!("{name} must be a non-empty string"),
                    )
                    .to_string()
                })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resources_requests_distinguish_open_from_request_bound_continuation() {
        let open = resources_request(
            json!({
                "sourceSet": "main",
                "metadataPath": "CommonModule.Shared.Module",
                "scope": "self",
                "limit": 25
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            open,
            SourceResourcesRequest::Open(OpenResourceSnapshotRequest { limit: 25, .. })
        ));

        let continuation = resources_request(
            json!({
                "snapshotId": "snapshot",
                "cursor": "cursor",
                "limit": 25
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            continuation,
            SourceResourcesRequest::Continue(ContinueResourceSnapshotRequest {
                limit: Some(25),
                ..
            })
        ));
        let continuation_without_limit = resources_request(
            json!({"snapshotId": "snapshot", "cursor": "cursor"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            continuation_without_limit,
            SourceResourcesRequest::Continue(ContinueResourceSnapshotRequest { limit: None, .. })
        ));

        let mixed = resources_request(
            json!({
                "sourceSet": "main",
                "snapshotId": "snapshot",
                "cursor": "cursor"
            })
            .as_object()
            .unwrap(),
        )
        .unwrap_err();
        assert!(mixed.contains("invalid_request"), "{mixed}");
    }

    #[test]
    fn read_requests_enforce_the_byte_limit_before_calling_a_provider() {
        let error = read_request(
            json!({
                "snapshotId": "snapshot",
                "resourceId": "resource",
                "offset": 0,
                "limit": 65_537
            })
            .as_object()
            .unwrap(),
        )
        .unwrap_err();

        assert!(error.contains("limit_exceeded"), "{error}");
    }
}
