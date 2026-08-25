use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
use crate::application::invocation_store::ToolIdentity;
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::result_store::ViewCursorStore;
use crate::application::v13::find::FindRequest;
use crate::application::v13::view::{ViewRequest, ViewService};
use crate::application::v13::LOGICAL_READ_OPERATION_BUDGET;
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{DomainResult, InvocationFailure};
use crate::domain::platform_profile::PlatformProfile;
use crate::infrastructure::v13_find::{ActorFindSource, WorkspaceFindIndexBuilder};
use crate::infrastructure::v13_read::LogicalViewReadAuthority;
use serde_json::{json, Value};
use std::sync::Arc;

/// Explicitly injected hidden v0.13 reader. `DaemonServerConfig::new` keeps the
/// dormant service until the atomic Task 22 surface cutover.
pub(crate) struct CanonicalV13ReadService {
    cursors: Arc<ViewCursorStore>,
    find_builder: WorkspaceFindIndexBuilder,
}

impl Default for CanonicalV13ReadService {
    fn default() -> Self {
        Self {
            cursors: Arc::new(ViewCursorStore::default()),
            find_builder: WorkspaceFindIndexBuilder::default(),
        }
    }
}

impl CanonicalInvocationService for CanonicalV13ReadService {
    fn prepare(
        &self,
        invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        match invocation.tool() {
            ToolIdentity::View | ToolIdentity::Find => Ok(ExecutionClass::InlineCandidate),
            tool => Err(Box::new(error_result(
                None,
                "provider_unavailable",
                format!(
                    "canonical v0.13 `{}` handler is not implemented in the Task 14 read slice",
                    tool.catalog_name()
                ),
            ))),
        }
    }

    fn execute(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        match invocation.tool() {
            ToolIdentity::View => Ok(self.execute_view(invocation, &cancellation)),
            ToolIdentity::Find => Ok(self.execute_find(invocation, &cancellation)),
            tool => Ok(error_result(
                None,
                "provider_unavailable",
                format!(
                    "canonical v0.13 `{}` handler is unavailable",
                    tool.catalog_name()
                ),
            )),
        }
    }
}

impl CanonicalV13ReadService {
    fn execute_view(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let Some(at) = arguments.get("at").and_then(Value::as_str) else {
            return error_result(None, "bad_value", "view requires string argument `at`");
        };
        let mut request = match ViewRequest::new(at) {
            Ok(request) => request,
            Err(error) => {
                return error_result(Some(at.to_string()), error.code(), error.to_string())
            }
        };
        if let Some(filter) = arguments.get("filter") {
            let Some(filter) = filter.as_object() else {
                return error_result(
                    Some(at.to_string()),
                    "bad_value",
                    "view filter must be an object",
                );
            };
            request = request.with_filter(filter.clone());
        }
        if let Some(limit) = arguments.get("limit") {
            let Some(limit) = bounded_usize(limit) else {
                return error_result(
                    Some(at.to_string()),
                    "bad_value",
                    "view limit must be a positive integer",
                );
            };
            request = match request.with_limit(limit) {
                Ok(request) => request,
                Err(error) => {
                    return error_result(Some(at.to_string()), error.code(), error.to_string())
                }
            };
        }
        if let Some(cursor) = arguments.get("cursor") {
            let Some(cursor) = cursor.as_str() else {
                return error_result(
                    Some(at.to_string()),
                    "bad_value",
                    "view cursor must be a string",
                );
            };
            request = request.with_cursor(cursor.to_string());
        }
        let address = match QualifiedAddress::parse(at) {
            Ok(address) => address,
            Err(error) => {
                return error_result(Some(at.to_string()), "bad_value", error.to_string())
            }
        };
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(Some(at.to_string()), "provider_unavailable", error),
        };
        let Some(source) = sources
            .into_iter()
            .find(|source| source.name == address.source_set())
        else {
            return error_result(
                Some(at.to_string()),
                "not_found",
                "view source set was not admitted by the workspace actor",
            );
        };
        let authority = LogicalViewReadAuthority::new(
            cancellation,
            source.name,
            source.identity,
            source.kind,
            source.revisions,
            source.retained_root,
            PlatformProfile::v8_3_27(),
        );
        ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors)).view(request)
    }

    fn execute_find(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let Some(query) = arguments.get("query").and_then(Value::as_str) else {
            return error_result(None, "bad_value", "find requires string argument `query`");
        };
        let mut request = match FindRequest::new(query) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        if let Some(kind) = arguments.get("kind") {
            let Some(kind) = kind.as_str() else {
                return error_result(None, "bad_value", "find kind must be a string");
            };
            request = match request.with_kind(kind) {
                Ok(request) => request,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
        }
        if let Some(limit) = arguments.get("limit") {
            let Some(limit) = bounded_usize(limit) else {
                return error_result(None, "bad_value", "find limit must be a positive integer");
            };
            request = match request.with_limit(limit) {
                Ok(request) => request,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
        }
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, "provider_unavailable", error),
        };
        let authorities = sources
            .into_iter()
            .map(|source| {
                (
                    source.name.clone(),
                    LogicalViewReadAuthority::new(
                        cancellation,
                        source.name,
                        source.identity,
                        source.kind,
                        source.revisions,
                        source.retained_root,
                        PlatformProfile::v8_3_27(),
                    ),
                )
            })
            .collect::<Vec<_>>();
        let find_sources = authorities
            .iter()
            .map(|(name, authority)| ActorFindSource::new(name, authority))
            .collect::<Vec<_>>();
        let built = match self.find_builder.build_with_revision(
            &find_sources,
            ProviderDeadline::from_budget(LOGICAL_READ_OPERATION_BUDGET),
            cancellation,
        ) {
            Ok(built) => built,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        let found = built.index.find(request);
        let mut result = DomainResult::success("logical address candidates resolved");
        result.data = Some(
            serde_json::to_value(found)
                .expect("the closed FindResult model always serializes to JSON"),
        );
        result.rev = Some(built.revision);
        result
    }
}

fn bounded_usize(value: &Value) -> Option<usize> {
    usize::try_from(value.as_u64()?)
        .ok()
        .filter(|value| *value > 0)
}

fn error_result(
    at: Option<String>,
    code: &'static str,
    message: impl Into<String>,
) -> DomainResult {
    let message = message.into();
    let mut result = DomainResult::success(message.clone());
    result.ok = false;
    result.at = at;
    result.diagnostics = vec![json!({"code": code, "message": message})];
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn logical_read_operation_budget_outlives_task_handoff_and_completes_once() {
        crate::application::invocation::tests::assert_operation_budget_survives_handoff_and_completes_once(
            crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
        );
    }
}
