use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use crate::domain::cache::CacheReport;
use crate::domain::cancellation::CancellationToken;
use crate::domain::events::DomainEvent;
use crate::domain::source_resources::{
    ResourceScope, SourceApplyResult, SourceResourceError, SourceResourceErrorCode,
    SOURCE_READ_LIMIT_MAX, SOURCE_REPLACEMENT_MAX_BYTES, SOURCE_RESOURCE_PAGE_LIMIT_MAX,
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
    Apply,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceApplyRequest {
    pub(crate) snapshot_id: String,
    pub(crate) resource_id: String,
    pub(crate) expected_hash: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceApplyExecution {
    pub(crate) result: SourceApplyResult,
    pub(crate) event: Option<DomainEvent>,
    pub(crate) projected_event: Option<DomainEvent>,
    pub(crate) recorded_cache: Option<CacheReport>,
    pub(crate) warnings: Vec<String>,
}

pub(crate) fn invoke(
    operation: SourceResourceOperation,
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let (summary, data, events, projected_events, changes, recorded_cache, warnings) =
        match operation {
            SourceResourceOperation::Resources => {
                let page = ports
                    .source_resources(resources_request(args)?, context, cancellation)
                    .map_err(|error| error.to_string())?;
                (
                    format!(
                        "source.resources returned {} resource(s)",
                        page.resources.len()
                    ),
                    serde_json::to_value(page).map_err(|error| {
                        format!("failed to serialize source.resources: {error}")
                    })?,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    Vec::new(),
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
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    Vec::new(),
                )
            }
            SourceResourceOperation::Apply => {
                let execution = ports
                    .apply_source_resource(apply_request(args)?, context, dry_run, cancellation)
                    .map_err(|error| error.to_string())?;
                let summary = if execution.result.no_op {
                    "unica.source.apply replacement is already present".to_string()
                } else if dry_run {
                    "dry run: unica.source.apply planned one BSL resource replacement".to_string()
                } else {
                    "unica.source.apply replaced one BSL resource".to_string()
                };
                let changes = (!dry_run && !execution.result.no_op)
                    .then(|| {
                        format!(
                            "{} + {}: replaced BSL resource",
                            execution.result.source_set,
                            execution
                                .result
                                .target
                                .metadata_path
                                .as_ref()
                                .map(|address| address.as_str())
                                .unwrap_or("<source-root>")
                        )
                    })
                    .into_iter()
                    .collect();
                let events = execution.event.into_iter().collect();
                let projected_events = execution.projected_event.into_iter().collect();
                let recorded_cache = execution.recorded_cache;
                let warnings = execution.warnings;
                (
                    summary,
                    serde_json::to_value(execution.result)
                        .map_err(|error| format!("failed to serialize source.apply: {error}"))?,
                    events,
                    projected_events,
                    changes,
                    recorded_cache,
                    warnings,
                )
            }
        };
    let mut adapter = AdapterOutcome::ok(summary);
    adapter.changes = changes;
    adapter.warnings = warnings;
    let mut outcome = if events.is_empty() && projected_events.is_empty() {
        HandlerOutcome::with_data(adapter, data)
    } else if projected_events.is_empty() {
        HandlerOutcome::with_data_and_events(adapter, data, events)
    } else {
        HandlerOutcome::with_data_events_and_projection(adapter, data, events, projected_events)
    };
    outcome.recorded_cache = recorded_cache;
    Ok(outcome)
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

fn apply_request(args: &Map<String, Value>) -> Result<SourceApplyRequest, String> {
    if args
        .get("contentEncoding")
        .is_some_and(|value| value.as_str() != Some("utf-8"))
    {
        return Err(SourceResourceError::new(
            SourceResourceErrorCode::InvalidRequest,
            "contentEncoding must be utf-8",
        )
        .to_string());
    }
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SourceResourceError::new(
                SourceResourceErrorCode::InvalidRequest,
                "content must be a UTF-8 string",
            )
            .to_string()
        })?
        .to_string();
    if content.len() > SOURCE_REPLACEMENT_MAX_BYTES {
        return Err(SourceResourceError::new(
            SourceResourceErrorCode::ContentTooLarge,
            "replacement content exceeds the one MiB decoded byte limit",
        )
        .to_string());
    }
    Ok(SourceApplyRequest {
        snapshot_id: required_string(args, "snapshotId")?,
        resource_id: required_string(args, "resourceId")?,
        expected_hash: required_string(args, "expectedHash")?,
        content,
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

    #[test]
    fn source_apply_accepts_one_utf8_replacement_and_no_public_path() {
        let request = apply_request(
            json!({
                "snapshotId": "snapshot",
                "resourceId": "resource",
                "expectedHash": "sha256:before",
                "content": "Procedure Run()\\nEndProcedure\\n",
                "contentEncoding": "utf-8"
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            request,
            SourceApplyRequest {
                snapshot_id: "snapshot".to_string(),
                resource_id: "resource".to_string(),
                expected_hash: "sha256:before".to_string(),
                content: "Procedure Run()\\nEndProcedure\\n".to_string(),
            }
        );
    }

    #[test]
    fn source_apply_rejects_multiple_resources_and_non_utf8_encoding() {
        let multiple = apply_request(
            json!({
                "snapshotId": "snapshot",
                "resourceId": ["one", "two"],
                "expectedHash": "sha256:before",
                "content": "Procedure Run()\\nEndProcedure\\n"
            })
            .as_object()
            .unwrap(),
        )
        .unwrap_err();
        assert!(multiple.contains("invalid_request"), "{multiple}");

        let encoding = apply_request(
            json!({
                "snapshotId": "snapshot",
                "resourceId": "resource",
                "expectedHash": "sha256:before",
                "content": "Procedure Run()\\nEndProcedure\\n",
                "contentEncoding": "base64"
            })
            .as_object()
            .unwrap(),
        )
        .unwrap_err();
        assert!(encoding.contains("invalid_request"), "{encoding}");
    }

    #[test]
    fn source_apply_rejects_decoded_content_over_one_mib_before_provider_call() {
        let error = apply_request(
            json!({
                "snapshotId": "snapshot",
                "resourceId": "resource",
                "expectedHash": "sha256:before",
                "content": "x".repeat(1024 * 1024 + 1)
            })
            .as_object()
            .unwrap(),
        )
        .unwrap_err();

        assert!(error.contains("content_too_large"), "{error}");
    }
}
