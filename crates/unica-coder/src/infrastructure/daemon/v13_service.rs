use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
use super::v13_read_modes::{filter_diff_data, project_view_sections, search_scope_prefix};
use crate::application::invocation_store::ToolIdentity;
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::result_store::ViewCursorStore;
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::apply::parse_request as parse_apply_request;
use crate::application::v13::find::FindRequest;
use crate::application::v13::tool_catalog::catalog_for;
use crate::application::v13::view::{ViewRequest, ViewService};
use crate::domain::address::QualifiedAddress;
use crate::domain::apply::OperationRegistry;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, InvocationFailure};
use crate::infrastructure::native_operations::apply::{
    ApplyPlanErrorKind, ApplyStagedState, PlannedApplyEffects, StagedChangeKind, StagedFileState,
};
use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
use crate::infrastructure::v13_find::{ActorFindSource, WorkspaceFindIndexBuilder};
use crate::infrastructure::workspace_actor::{
    ApplyAdmissionError, ApplyEffectDisposition, ApplyPublicationErrorKind,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Canonical v0.13 service installed by the production v3 daemon composition.
/// Each public name has a useful closed mode; unfinished variants fail with a
/// typed `unsupported_*` result rather than pretending an engine is missing.
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
        _invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(ExecutionClass::InlineCandidate)
    }

    fn execute(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        match invocation.tool() {
            ToolIdentity::View => Ok(invocation
                .rejected_logical_read_result()
                .unwrap_or_else(|| self.execute_view(invocation, &cancellation))),
            ToolIdentity::Apply => Ok(self.execute_apply(invocation, &cancellation)),
            ToolIdentity::Find => Ok(self.execute_find(invocation, &cancellation)),
            ToolIdentity::Search => Ok(self.execute_search(invocation, &cancellation)),
            ToolIdentity::Check => Ok(self.execute_check(invocation, &cancellation)),
            ToolIdentity::Diff => Ok(self.execute_diff(invocation, &cancellation)),
            ToolIdentity::Run => Ok(self.execute_run(invocation, &cancellation)),
            ToolIdentity::Docs => {
                let arguments = invocation.arguments();
                let Some(query) = arguments.get("query").and_then(Value::as_str) else {
                    return Ok(error_result(
                        None,
                        "bad_value",
                        "docs requires string argument `query`",
                    ));
                };
                let source = match arguments.get("source") {
                    None => None,
                    Some(Value::String(source)) => Some(source.as_str()),
                    Some(_) => {
                        return Ok(error_result(
                            None,
                            "bad_value",
                            "docs source must be a string",
                        ))
                    }
                };
                Ok(
                    crate::infrastructure::application_ports::canonical_v13_docs_search(
                        invocation.workspace_context(),
                        query,
                        source,
                        &cancellation,
                    ),
                )
            }
        }
    }
}

impl CanonicalV13ReadService {
    fn execute_apply(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let source_sets = invocation.admitted_source_set_names();
        let request = match parse_apply_request(invocation.arguments(), &source_sets) {
            Ok(request) => request,
            Err(error) => {
                return error_result(
                    Some(error.location().to_string()),
                    error.code(),
                    error.to_string(),
                )
            }
        };
        let (binding, admission) = match invocation.admit_apply(&request, cancellation) {
            Ok(admitted) => admitted,
            // A stale `ifRev` is a caller conflict with a known recovery —
            // re-read the revision and retry — so it answers with its own
            // code instead of masquerading as an unavailable provider.
            Err(error @ ApplyAdmissionError::StaleRevision { .. }) => {
                return error_result(
                    Some(request.at().to_string()),
                    "stale_revision",
                    error.to_string(),
                )
            }
            Err(ApplyAdmissionError::Other(error)) => {
                return error_result(
                    Some(request.at().to_string()),
                    "provider_unavailable",
                    error,
                )
            }
        };
        for (index, operation) in request.ops().iter().enumerate() {
            let Some(descriptor) = OperationRegistry::closed().lookup(operation.name()) else {
                return error_result(
                    Some(format!("ops[{index}].op")),
                    "unsupported_operation",
                    format!(
                        "apply operation `{}` is not in the canonical registry",
                        operation.name()
                    ),
                );
            };
            let target_kind = operation
                .at()
                .segments()
                .last()
                .expect("a parsed logical address has a terminal segment")
                .kind();
            if !descriptor.applies_to_operation_target(operation.at()) {
                return error_result(
                    Some(format!("ops[{index}].at")),
                    "bad_value",
                    format!(
                        "apply operation `{}` does not apply to {}",
                        operation.name(),
                        target_kind.as_str()
                    ),
                );
            }
        }
        let operations = request
            .ops()
            .iter()
            .enumerate()
            .map(|(index, operation)| {
                serde_json::json!({
                    "index": index,
                    "op": operation.name(),
                    "at": operation.at(),
                })
            })
            .collect::<Vec<_>>();
        let (staged, mut effects) = match plan_hidden_v13_apply(&request, &binding, &admission) {
            Err(error)
                if error.kind() == ApplyPlanErrorKind::ProviderUnavailable
                    && error.to_string().contains("not implemented") =>
            {
                return error_result(
                    error.path().map(str::to_string),
                    "unsupported_operation",
                    "canonical v0.13 apply operation is not implemented",
                )
            }
            Err(error) => {
                return error_result(
                    error.path().map(str::to_string),
                    apply_plan_error_code(error.kind()),
                    error.to_string(),
                )
            }
            Ok(planned) => planned,
        };
        let has_changes = !staged.planned_changes().is_empty();
        if !has_changes {
            effects = Default::default();
        }
        let plan_warnings = effects.warnings().to_vec();
        let plan_hash = apply_plan_hash(&staged, &effects);
        let changed = effects
            .events()
            .iter()
            .map(|event| {
                let at = if event.artifact.contains(':') {
                    event.artifact.clone()
                } else {
                    format!("{}:{}", request.at().source_set(), event.artifact)
                };
                serde_json::json!({
                    "at": at,
                    "event": event.name(),
                })
            })
            .collect::<Vec<_>>();
        let prepared = match admission.prepare_with_effects(staged, effects) {
            Ok(prepared) => prepared,
            Err(error) => {
                return error_result(
                    Some("ops".to_string()),
                    "provider_unavailable",
                    error.to_string(),
                )
            }
        };
        let publication = match invocation.publish_prepared_apply(prepared) {
            Ok(publication) => publication,
            Err(error) => {
                return error_result(
                    Some(request.at().to_string()),
                    apply_publication_error_code(error.kind()),
                    error.to_string(),
                )
            }
        };
        let disposition = match publication.effects().disposition() {
            ApplyEffectDisposition::Projected => "preview",
            ApplyEffectDisposition::Committed => "published",
        };
        let mut result = DomainResult::success(match publication.effects().disposition() {
            ApplyEffectDisposition::Projected => "metadata apply plan prepared without publication",
            ApplyEffectDisposition::Committed => "metadata apply published atomically",
        });
        result.at = Some(request.at().to_string());
        result.data = Some(serde_json::json!({
            "validated": true,
            "mode": disposition,
            "executable": true,
            "operations": operations,
            "planHash": plan_hash,
            "effects": publication.effects().events().len(),
            "cache": publication.effects().cache(),
        }));
        if has_changes {
            result.changed = changed;
        }
        result.warnings.extend(plan_warnings);
        if !publication.cleanup_diagnostics().is_empty() {
            result.warnings.push(serde_json::json!({
                "code": "retained_cleanup_incomplete",
                "count": publication.cleanup_diagnostics().len(),
                "message": "published apply left bounded internal recovery cleanup diagnostics"
            }));
        }
        result.rev = Some(publication.rev().to_string());
        result
    }

    fn execute_view(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        self.execute_view_arguments(invocation, invocation.arguments(), cancellation)
    }

    fn execute_view_arguments(
        &self,
        invocation: &ActorBoundExecution,
        arguments: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
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
            .find(|source| source.source_set_name() == address.source_set())
        else {
            return error_result(
                Some(at.to_string()),
                "provider_unavailable",
                "view source set was not admitted by the workspace actor",
            );
        };
        let authority = match source.logical_view_read_authority(cancellation) {
            Ok(authority) => authority,
            Err(error) => return error_result(Some(at.to_string()), "provider_unavailable", error),
        };
        let mut result =
            ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors)).view(request);
        let sections = arguments
            .get("filter")
            .and_then(Value::as_object)
            .and_then(|filter| filter.get("sections"));
        if result.ok {
            if let (Some(data), Some(sections)) = (result.data.as_ref(), sections) {
                match project_view_sections(data, sections) {
                    Ok(projected) => result.data = Some(projected),
                    Err(error) => {
                        return error_result(Some(at.to_string()), error.code(), error.to_string())
                    }
                }
            }
        }
        result
    }

    fn execute_search(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let Some(query) = arguments.get("query").and_then(Value::as_str) else {
            return error_result(None, "bad_value", "search requires string argument `query`");
        };
        if query.trim().is_empty() {
            return error_result(None, "bad_value", "search query must not be blank");
        }
        let (matcher, mode) = match arguments.get("regex") {
            None | Some(Value::Bool(false)) => (
                super::v13_read_modes::SearchMatcher::Literal(query.to_string()),
                "literal",
            ),
            Some(Value::Bool(true)) => {
                // Bounded compile: a pattern that cannot be compiled within
                // the size budget is a caller error, not a provider failure.
                match regex::RegexBuilder::new(query)
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                {
                    Ok(compiled) => (
                        super::v13_read_modes::SearchMatcher::Regex(compiled),
                        "regex",
                    ),
                    Err(error) => {
                        return error_result(
                            None,
                            "bad_value",
                            format!("search regex is not a valid pattern: {error}"),
                        )
                    }
                }
            }
            Some(_) => return error_result(None, "bad_value", "search regex must be a boolean"),
        };
        let limit = match arguments.get("limit") {
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 200) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        "bad_value",
                        "search limit must be an integer from 1 through 200",
                    )
                }
            },
            None => 20,
        };
        let scope = match arguments.get("scope") {
            None => None,
            Some(Value::String(scope)) => match QualifiedAddress::parse(scope) {
                Ok(address) => Some(address),
                Err(error) => {
                    return error_result(Some(scope.to_string()), "bad_value", error.to_string())
                }
            },
            Some(_) => return error_result(None, "bad_value", "search scope must be a string"),
        };
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, "provider_unavailable", error),
        };
        let selected = sources
            .into_iter()
            .filter(|source| {
                scope
                    .as_ref()
                    .is_none_or(|scope| source.source_set_name() == scope.source_set())
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return error_result(
                scope.map(|scope| scope.to_string()),
                "not_found",
                "search scope does not name an admitted source set",
            );
        }
        if let Some(scope) = scope.as_ref() {
            if let Err(error) = search_scope_prefix(scope) {
                return error_result(Some(scope.to_string()), error.code(), error.to_string());
            }
            let viewed = self.execute_view_arguments(
                invocation,
                &Map::from_iter([("at".to_string(), Value::String(scope.to_string()))]),
                cancellation,
            );
            if !viewed.ok {
                return viewed;
            }
        }
        let mut matches = Vec::new();
        let mut revisions = Vec::new();
        for source in selected {
            revisions.push(source.revision_identity());
            let scope_at = scope.clone().unwrap_or_else(|| {
                QualifiedAddress::parse(&format!("{}:Configuration", source.source_set_name()))
                    .expect("actor source-set names and Configuration address are canonical")
            });
            let scope_prefix = match search_scope_prefix(&scope_at) {
                Ok(prefix) => prefix,
                Err(error) => {
                    return error_result(
                        Some(scope_at.to_string()),
                        error.code(),
                        error.to_string(),
                    )
                }
            };
            match source.search_bsl_literal(
                &matcher,
                limit.saturating_sub(matches.len()),
                scope_prefix.as_deref(),
                &scope_at,
                cancellation,
            ) {
                Ok(found) => matches.extend(found),
                Err(error) => return error_result(None, "provider_unavailable", error),
            }
            if matches.len() == limit {
                break;
            }
        }
        let mut result = DomainResult::success(format!("{mode} BSL search completed"));
        result.data = Some(serde_json::json!({
            "mode": mode,
            "matches": matches,
        }));
        result.rev = combined_revision(&revisions);
        result
    }

    fn execute_check(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        if arguments.contains_key("filter") {
            return error_result(
                None,
                "bad_value",
                "check takes only `at`; the validators of a node follow from its kind",
            );
        }
        if let Some(at) = arguments.get("at") {
            let Some(at) = at.as_str() else {
                return error_result(None, "bad_value", "check at must be a string");
            };
            let view_arguments =
                Map::from_iter([("at".to_string(), Value::String(at.to_string()))]);
            let viewed = self.execute_view_arguments(invocation, &view_arguments, cancellation);
            if !viewed.ok {
                if let Some(refusal) = unreadable_target_format_refusal(invocation, at) {
                    return refusal;
                }
                return viewed;
            }
            let kind = viewed
                .data
                .as_ref()
                .and_then(|data| data.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut result =
                run_node_checks(invocation, at, &kind, viewed.data.as_ref(), cancellation);
            if result.ok {
                result.rev = viewed.rev;
            }
            return result;
        }
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, "provider_unavailable", error),
        };
        let names = sources
            .iter()
            .map(|source| Value::String(source.source_set_name().to_string()))
            .collect::<Vec<_>>();
        let revisions = sources
            .iter()
            .map(|source| source.revision_identity())
            .collect::<Vec<_>>();
        let mut result = DomainResult::success("workspace source sets are admitted");
        result.data = Some(serde_json::json!({
            "status": "admitted",
            "sources": names,
        }));
        result.rev = combined_revision(&revisions);
        result
    }

    fn execute_diff(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let filter = arguments.get("filter");
        if arguments.contains_key("cursor") {
            return error_result(
                None,
                "unsupported_cursor",
                "diff pagination cursors are not implemented",
            );
        }
        let Some(left) = arguments.get("left").and_then(Value::as_str) else {
            return error_result(None, "bad_value", "diff requires string argument `left`");
        };
        let Some(right) = arguments.get("right").and_then(Value::as_str) else {
            return error_result(None, "bad_value", "diff requires string argument `right`");
        };
        let limit = match arguments.get("limit") {
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 1_000) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        "bad_value",
                        "diff limit must be an integer from 1 through 1000",
                    )
                }
            },
            None => 100,
        };
        let left_result = self.execute_view_arguments(
            invocation,
            &Map::from_iter([("at".to_string(), Value::String(left.to_string()))]),
            cancellation,
        );
        if !left_result.ok {
            return left_result;
        }
        let right_result = self.execute_view_arguments(
            invocation,
            &Map::from_iter([("at".to_string(), Value::String(right.to_string()))]),
            cancellation,
        );
        if !right_result.ok {
            return right_result;
        }
        let mut left_data = left_result
            .data
            .as_ref()
            .expect("successful view has data")
            .clone();
        let mut right_data = right_result
            .data
            .as_ref()
            .expect("successful view has data")
            .clone();
        if let Some(filter) = filter {
            left_data = match filter_diff_data(&left_data, filter) {
                Ok(data) => data,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
            right_data = match filter_diff_data(&right_data, filter) {
                Ok(data) => data,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
        }
        if left_data.get("kind") != right_data.get("kind") {
            return error_result(
                None,
                "incomparable_nodes",
                "diff requires nodes of the same logical kind",
            );
        }
        let mut changes = Vec::new();
        collect_json_changes("", &left_data, &right_data, limit + 1, &mut changes);
        let truncated = changes.len() > limit;
        changes.truncate(limit);
        let equal = changes.is_empty() && !truncated;
        let revisions = [left_result.rev, right_result.rev]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut result = DomainResult::success("logical nodes compared");
        result.data = Some(serde_json::json!({
            "equal": equal,
            "changes": changes,
            "truncated": truncated,
        }));
        result.rev = combined_revision(&revisions);
        result
    }

    fn execute_run(
        &self,
        invocation: &ActorBoundExecution,
        _cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
        let Some(op) = arguments.get("op") else {
            return super::v13_workspace_initialize::run_dictionary_result();
        };
        let Some(op) = op.as_str() else {
            return error_result(None, "bad_value", "run op must be a string");
        };
        if arguments.get("args").is_some_and(|args| !args.is_object()) {
            return error_result(None, "bad_value", "run args must be an object");
        }
        if !catalog
            .run_dictionary
            .iter()
            .any(|operation| operation.name() == op)
        {
            return error_result(
                Some(op.to_string()),
                "unsupported_operation",
                format!("unknown canonical run operation `{op}`"),
            );
        }
        error_result(
            Some(op.to_string()),
            "unsupported_operation",
            format!("canonical run operation `{op}` is not implemented yet"),
        )
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
        let Some(deadline) = sources.first().map(|source| source.deadline()) else {
            return error_result(
                None,
                "provider_unavailable",
                "find has no admitted logical source sets",
            );
        };
        let authorities = sources
            .into_iter()
            .map(|source| {
                let name = source.source_set_name().to_string();
                source
                    .logical_view_read_authority(cancellation)
                    .map(|authority| (name, authority))
            })
            .collect::<Result<Vec<_>, _>>();
        let authorities = match authorities {
            Ok(authorities) => authorities,
            Err(error) => return error_result(None, "provider_unavailable", error),
        };
        let find_sources = authorities
            .iter()
            .map(|(name, authority)| ActorFindSource::new(name, authority))
            .collect::<Vec<_>>();
        let built =
            match self
                .find_builder
                .build_with_revision(&find_sources, deadline, cancellation)
            {
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

fn apply_plan_hash(staged: &ApplyStagedState, effects: &PlannedApplyEffects) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-apply-plan-v1\0");
    for change in staged.planned_changes() {
        hasher.update(change.relative_path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update([match change.kind {
            StagedChangeKind::Create => 1,
            StagedChangeKind::Replace => 2,
            StagedChangeKind::Remove => 3,
        }]);
        match change.current {
            StagedFileState::Bytes(bytes) => {
                hasher.update([1]);
                hasher.update((bytes.len() as u64).to_be_bytes());
                hasher.update(bytes);
            }
            StagedFileState::Absent => hasher.update([0]),
        }
    }
    for event in effects.events() {
        hasher.update(event.name().as_bytes());
        hasher.update([0]);
        hasher.update(event.artifact.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn collect_json_changes(
    path: &str,
    left: &Value,
    right: &Value,
    limit: usize,
    changes: &mut Vec<Value>,
) {
    if left == right || changes.len() >= limit {
        return;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let child_path = format!("{path}/{escaped}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => {
                        collect_json_changes(&child_path, left, right, limit, changes)
                    }
                    (left, right) => changes.push(serde_json::json!({
                        "path": child_path,
                        "left": left,
                        "right": right,
                    })),
                }
                if changes.len() >= limit {
                    break;
                }
            }
        }
        _ => changes.push(serde_json::json!({
            "path": if path.is_empty() { "/" } else { path },
            "left": left,
            "right": right,
        })),
    }
}

fn combined_revision(revisions: &[String]) -> Option<String> {
    if revisions.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-read-set-v1\0");
    for revision in revisions {
        hasher.update(revision.as_bytes());
        hasher.update([0]);
    }
    Some(format!("unica-read-set-sha256-v1:{:x}", hasher.finalize()))
}

fn apply_plan_error_code(kind: ApplyPlanErrorKind) -> &'static str {
    match kind {
        ApplyPlanErrorKind::BadValue => "bad_value",
        ApplyPlanErrorKind::NotFound => "not_found",
        ApplyPlanErrorKind::ProviderUnavailable => "provider_unavailable",
        ApplyPlanErrorKind::InvalidState => "invalid_state",
        ApplyPlanErrorKind::InvalidSource => "invalid_source",
        ApplyPlanErrorKind::Staging(_) => "provider_unavailable",
        ApplyPlanErrorKind::Postcondition => "postcondition_failed",
    }
}

fn apply_publication_error_code(kind: ApplyPublicationErrorKind) -> &'static str {
    match kind {
        ApplyPublicationErrorKind::Cancelled => "cancelled",
        ApplyPublicationErrorKind::Deadline => "deadline_exceeded",
        ApplyPublicationErrorKind::ConcurrentRevision => "revision_mismatch",
        ApplyPublicationErrorKind::ContainmentIdentity => "provider_unavailable",
        ApplyPublicationErrorKind::ProviderPostvalidation => "postcondition_failed",
        ApplyPublicationErrorKind::SourceSelectionChanged => "source_selection_changed",
        ApplyPublicationErrorKind::RollbackIncomplete => "rollback_incomplete",
        ApplyPublicationErrorKind::Invariant => "provider_unavailable",
    }
}

fn bounded_usize(value: &Value) -> Option<usize> {
    usize::try_from(value.as_u64()?)
        .ok()
        .filter(|value| *value > 0)
}

/// Runs every validator a readable node owns and folds the verdicts into one
/// check envelope. The bridged validators keep the source of validation
/// truth; the plan follows from the node kind and the facts the projection
/// states, never from a caller choice. A node without validators reports
/// readability only.
fn run_node_checks(
    invocation: &ActorBoundExecution,
    at: &str,
    kind: &str,
    viewed: Option<&Value>,
    cancellation: &CancellationToken,
) -> DomainResult {
    use crate::application::v13::check::{plan_for_node, CheckStep};
    use crate::infrastructure::native_operations::v13_analysis::node_facts;

    let address = match QualifiedAddress::parse(at) {
        Ok(address) => address,
        Err(error) => return error_result(Some(at.to_string()), "bad_value", error.to_string()),
    };
    let context = invocation.workspace_context();
    let plan = plan_for_node(kind, node_facts(&address, viewed, context));
    if plan.is_empty() {
        let mut result = DomainResult::success("logical node is readable");
        result.at = Some(at.to_string());
        result.data = Some(serde_json::json!({
            "status": "readable",
            "at": at,
            "kind": kind,
            "validators": [],
            "diagnostics": [],
        }));
        return result;
    }
    let mut passed = true;
    let mut diagnostics = Vec::new();
    let mut validators = Vec::new();
    for step in plan {
        let verdict = match step {
            CheckStep::Native(validator) => {
                run_native_validator(&address, kind, validator, context)
            }
            CheckStep::Meta => run_meta_validator(&address, at, context, cancellation),
        };
        match verdict {
            Err(refusal) => return *refusal,
            Ok((step_passed, step_diagnostics)) => {
                passed &= step_passed;
                validators.push(step.name());
                diagnostics.extend(step_diagnostics.into_iter().map(|mut diagnostic| {
                    if let Some(object) = diagnostic.as_object_mut() {
                        object.insert("validator".to_string(), Value::String(step.name().into()));
                    }
                    diagnostic
                }));
            }
        }
    }
    let mut result = DomainResult::success(if passed {
        "validation passed"
    } else {
        "validation reported findings"
    });
    result.at = Some(at.to_string());
    result.data = Some(serde_json::json!({
        "status": if passed { "passed" } else { "failed" },
        "at": at,
        "kind": kind,
        "validators": validators,
        "diagnostics": diagnostics,
    }));
    result
}

fn run_native_validator(
    address: &QualifiedAddress,
    kind: &str,
    validator: crate::application::v13::check::CheckValidator,
    context: &crate::domain::workspace::WorkspaceContext,
) -> Result<(bool, Vec<Value>), Box<DomainResult>> {
    use crate::application::v13::check::{normalize_native_outcome, CheckError};
    use crate::infrastructure::native_operations::v13_analysis::{validate, validator_selector};

    let at = address.to_string();
    let selector = validator_selector(validator, address, context)
        .map_err(|error| Box::new(error_result(Some(at.clone()), "bad_value", error)))?;
    let native = validate(validator, &selector, context);
    match normalize_native_outcome(address, kind, validator, native) {
        Ok(checked) => Ok((
            checked.ok(),
            checked
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    serde_json::json!({
                        "severity": diagnostic.severity(),
                        "code": diagnostic.code(),
                        "message": diagnostic.message(),
                    })
                })
                .collect(),
        )),
        Err(CheckError::DependencyUnavailable) => Err(Box::new(error_result(
            Some(at),
            "provider_unavailable",
            "the native validator dependency is unavailable",
        ))),
        Err(error) => Err(Box::new(error_result(
            Some(at),
            error.code(),
            error.to_string(),
        ))),
    }
}

/// The typed metadata validator of one object descriptor: the same read and
/// validation the metadata family runs, folded into the check envelope.
fn run_meta_validator(
    address: &QualifiedAddress,
    at: &str,
    context: &crate::domain::workspace::WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<(bool, Vec<Value>), Box<DomainResult>> {
    use crate::infrastructure::native_operations::v13_analysis::validator_metadata_path;

    let Some(path) = validator_metadata_path(address) else {
        return Err(Box::new(error_result(
            Some(at.to_string()),
            "bad_value",
            "the metadata validator needs one object descriptor; this address names none",
        )));
    };
    let target = crate::domain::source_target::MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &path,
    )
    .map_err(|error| {
        Box::new(error_result(
            Some(at.to_string()),
            "bad_value",
            error.to_string(),
        ))
    })?;
    let request = crate::application::metadata::MetaInfoRequest {
        source_set: address.source_set().to_string(),
        metadata_path: target,
        sections: Vec::new(),
        limit: 0,
    };
    let readers = crate::infrastructure::support_state::WorkspaceSupportStateReaderFactory;
    let reader =
        crate::infrastructure::support_state::SupportStateReaderFactory::create(&readers, context);
    match crate::infrastructure::metadata_operations::MetadataOperations::read_local(
        &request,
        context,
        cancellation,
        reader.as_ref(),
    ) {
        Ok(read) => {
            let validated =
                crate::infrastructure::metadata_operations::MetadataOperations::validate(
                    &read.validation_subject,
                    context,
                    cancellation,
                );
            let diagnostics = validated
                .diagnostics
                .iter()
                .map(|diagnostic| serde_json::to_value(diagnostic).unwrap_or(Value::Null))
                .collect::<Vec<_>>();
            Ok((
                validated.status == crate::domain::metadata::MetaValidationStatus::Passed,
                diagnostics,
            ))
        }
        Err(failure) => {
            let message = failure
                .diagnostics
                .iter()
                .filter_map(|diagnostic| {
                    serde_json::to_value(diagnostic).ok().and_then(|value| {
                        value
                            .get("message")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
                .collect::<Vec<_>>()
                .join("; ");
            Err(Box::new(error_result(
                Some(at.to_string()),
                "provider_unavailable",
                if message.is_empty() {
                    "the metadata descriptor could not be read for validation".to_string()
                } else {
                    message
                },
            )))
        }
    }
}

/// A target the read port cannot open is named by its export format when
/// the format guard of its validator knows it: the closed refusal replaces a
/// provider failure that would otherwise hide a root outside the active
/// profile (`DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE`).
fn unreadable_target_format_refusal(
    invocation: &ActorBoundExecution,
    at: &str,
) -> Option<DomainResult> {
    use crate::application::ports::FormatGuardCheck;
    use crate::application::v13::check::CheckValidator;
    use crate::infrastructure::native_operations::v13_analysis::{
        source_set_is_extension, validator_selector,
    };

    let address = QualifiedAddress::parse(at).ok()?;
    let context = invocation.workspace_context();
    let validator =
        CheckValidator::for_unread_address(&address, source_set_is_extension(&address, context))?;
    let selector = validator_selector(validator, &address, context).ok()?;
    let check = crate::infrastructure::format_guard::evaluate_read_format_guard(
        validator.native_operation(),
        &selector,
        context,
    )
    .ok()?;
    let (warning, diagnostic) = match check {
        FormatGuardCheck::Allow => return None,
        FormatGuardCheck::Warn {
            warning,
            diagnostic,
        } => (warning, diagnostic),
        FormatGuardCheck::Block {
            outcome,
            diagnostic,
            ..
        } => (outcome.warnings.join(" "), diagnostic),
    };
    let actual = diagnostic
        .get("actualFormat")
        .and_then(Value::as_str)
        .map(|actual| format!(" (export format {actual})"))
        .unwrap_or_default();
    Some(error_result(
        Some(at.to_string()),
        "invalid_source",
        format!("{warning}{actual}"),
    ))
}

fn error_result(
    at: Option<String>,
    code: &'static str,
    message: impl Into<String>,
) -> DomainResult {
    DomainResult::canonical_rejection(at, code, message)
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
