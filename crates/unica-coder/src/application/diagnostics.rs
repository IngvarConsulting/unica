use super::ports::ApplicationPorts;
use crate::domain::cancellation::CancellationToken;
use crate::domain::diagnostics::*;
use crate::domain::source_target::{MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) trait DiagnosticMapping: Send + Sync {
    fn resolve_context(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError>;

    fn map_observation(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError>;
}

impl<T: ApplicationPorts + ?Sized> DiagnosticMapping for T {
    fn resolve_context(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError> {
        ApplicationPorts::resolve_diagnostic_context(self, request, workspace, cancellation)
    }

    fn map_observation(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError> {
        ApplicationPorts::map_diagnostic_observation(self, observation, context, cancellation)
    }
}

pub(crate) struct DiagnosticCoordinator<'a> {
    registry: DiagnosticProviderRegistry,
    mapping: &'a dyn DiagnosticMapping,
}

impl<'a> DiagnosticCoordinator<'a> {
    pub(crate) fn new(
        registry: DiagnosticProviderRegistry,
        mapping: &'a dyn DiagnosticMapping,
    ) -> Self {
        Self { registry, mapping }
    }

    pub(crate) fn execute(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticResult, DiagnosticRequestError> {
        let context = self
            .mapping
            .resolve_context(request, workspace, cancellation)?;
        if request.range.is_some() && context.target.target_kind != TargetKind::Module {
            return Err(request_error(
                "target_kind_mismatch",
                Some("range"),
                "range is supported only for a module findings target",
            ));
        }
        let selected = select_providers(&self.registry, request, context.target.target_kind)?;
        let selection = selection(request, &context, &selected);
        let provider_order = selected
            .iter()
            .enumerate()
            .map(|(index, selected)| (selected.descriptor.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let provider_request = DiagnosticProviderRequest {
            action: request.action,
            source_set: request.source_set.clone(),
            metadata_path: request.metadata_path.clone(),
            target_kind: context.target.target_kind,
            filter: request.filter.clone(),
            range: request.range,
        };
        let mut sections = Vec::with_capacity(selected.len());
        let mut all_items = Vec::new();
        for selected_provider in &selected {
            if !selected_provider.applicable {
                sections.push(unsupported_section(
                    selected_provider.descriptor,
                    request.action,
                ));
                continue;
            }
            let deadline =
                ProviderDeadline::from_budget(request.timeout.unwrap_or(Duration::from_secs(120)));
            let outcome = selected_provider.provider.execute(
                &provider_request,
                &context,
                deadline,
                cancellation,
            );
            let mut provider_items = Vec::new();
            let mut mapping_error = None;
            for observation in outcome.observations {
                match self
                    .mapping
                    .map_observation(observation, &context, cancellation)
                {
                    Ok(item) => provider_items.push(item),
                    Err(error) => {
                        mapping_error = Some(error);
                        break;
                    }
                }
            }
            if let Some(error) = mapping_error {
                sections.push(failed_mapping_section(
                    selected_provider.descriptor,
                    outcome.version,
                    error,
                    request.action,
                ));
                continue;
            }
            if request.action == DiagnosticAction::Catalog {
                provider_items.extend(outcome.rules.into_iter().map(|rule| {
                    DiagnosticItem::DiagnosticRule {
                        provider: rule.provider.as_str(),
                        code: rule.code,
                        default_severity: rule.default_severity,
                        title: rule.title,
                        description: rule.description,
                        tags: rule.tags,
                    }
                }));
            }
            provider_items.retain(|item| item_matches_request(item, request, &context));
            let resource_failures = provider_items
                .iter()
                .filter(|item| matches!(item, DiagnosticItem::ResourceFailure { .. }))
                .count();
            let items_total = provider_items.len();
            all_items.extend(provider_items);
            sections.push(DiagnosticProviderSection {
                id: selected_provider.descriptor.id.as_str(),
                status: outcome.status,
                complete: outcome.complete,
                version: outcome.version,
                capabilities: (request.action == DiagnosticAction::Catalog)
                    .then(|| selected_provider.descriptor.into()),
                readiness: outcome.readiness,
                items_total: action_has_items(request.action).then_some(items_total),
                items_returned: action_has_items(request.action).then_some(0),
                resource_failures: action_has_observations(request.action)
                    .then_some(resource_failures),
                truncated: action_has_items(request.action).then_some(false),
                error: outcome.error,
            });
        }

        all_items.sort_by(|left, right| {
            diagnostic_sort_key(left, &provider_order)
                .cmp(&diagnostic_sort_key(right, &provider_order))
        });
        let items_total = all_items.len();
        all_items.truncate(request.limit);
        let items_returned = all_items.len();
        let truncated = items_returned < items_total;
        for section in &mut sections {
            if action_has_items(request.action) {
                let returned = all_items
                    .iter()
                    .filter(|item| item_provider(item) == section.id)
                    .count();
                section.items_returned = Some(returned);
                section.truncated = Some(returned < section.items_total.unwrap_or(0));
            }
        }
        let any_success = sections.iter().any(|section| {
            matches!(
                section.status,
                DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
            )
        });
        let all_complete = !sections.is_empty()
            && sections.iter().all(|section| {
                matches!(
                    section.status,
                    DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
                ) && section.complete
            });
        let (ok, state, complete) = if all_complete {
            (true, DiagnosticResultState::Completed, true)
        } else if any_success {
            (true, DiagnosticResultState::Partial, false)
        } else {
            (false, DiagnosticResultState::Failed, false)
        };
        Ok(DiagnosticResult {
            ok,
            action: request.action,
            selection,
            state,
            complete,
            providers: sections,
            items_total: action_has_items(request.action).then_some(items_total),
            items_returned: action_has_items(request.action).then_some(items_returned),
            truncated: action_has_items(request.action).then_some(truncated),
            items: all_items,
        })
    }
}

struct SelectedProvider {
    descriptor: &'static DiagnosticProviderDescriptor,
    provider: Arc<dyn DiagnosticProvider>,
    applicable: bool,
}

fn select_providers(
    registry: &DiagnosticProviderRegistry,
    request: &DiagnosticRequest,
    target_kind: TargetKind,
) -> Result<Vec<SelectedProvider>, DiagnosticRequestError> {
    let registered = registry
        .descriptors()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<HashSet<_>>();
    if let Some(requested) = &request.requested_providers {
        if requested.iter().any(|id| !registered.contains(id.as_str())) {
            return Err(request_error(
                "provider_unknown",
                Some("providers"),
                "providers contains an unregistered diagnostics provider",
            ));
        }
        if request
            .filter
            .codes
            .iter()
            .any(|code| !requested.iter().any(|provider| provider == &code.provider))
        {
            return Err(request_error(
                "filter_provider_not_selected",
                Some("filter.codes"),
                "filter.codes names a provider absent from providers",
            ));
        }
    }
    let code_providers = request
        .filter
        .codes
        .iter()
        .map(|filter| filter.provider.as_str())
        .collect::<HashSet<_>>();
    let mut selected = Vec::new();
    for provider in registry.providers() {
        let descriptor = provider.descriptor();
        let explicitly_selected = request
            .requested_providers
            .as_ref()
            .is_some_and(|requested| requested.iter().any(|id| id == descriptor.id.as_str()));
        let applicable = descriptor.supports_action(request.action)
            && (request.action != DiagnosticAction::Findings
                || descriptor.supports_findings_target(target_kind));
        let auto_selected = request.requested_providers.is_none()
            && (code_providers.is_empty() || code_providers.contains(descriptor.id.as_str()))
            && applicable;
        if explicitly_selected || auto_selected {
            selected.push(SelectedProvider {
                descriptor,
                provider: Arc::clone(provider),
                applicable,
            });
        }
    }
    if selected.is_empty() {
        return Err(request_error(
            "no_applicable_provider",
            Some("providers"),
            "no registered diagnostics provider supports the selected action and target",
        ));
    }
    Ok(selected)
}

fn selection(
    request: &DiagnosticRequest,
    context: &DiagnosticContext,
    providers: &[SelectedProvider],
) -> DiagnosticSelection {
    let exposes_target = matches!(
        request.action,
        DiagnosticAction::Analyze | DiagnosticAction::Findings
    );
    let exposes_filter = request.action != DiagnosticAction::Status;
    let exposes_limit = action_has_items(request.action);
    DiagnosticSelection {
        source_set: request.source_set.clone(),
        metadata_path: request.metadata_path.clone(),
        target_kind: exposes_target.then_some(context.target.target_kind),
        providers: providers
            .iter()
            .map(|provider| provider.descriptor.id.as_str())
            .collect(),
        filter: exposes_filter.then_some(request.filter.clone()),
        limit: exposes_limit.then_some(request.limit),
    }
}

fn unsupported_section(
    descriptor: &DiagnosticProviderDescriptor,
    action: DiagnosticAction,
) -> DiagnosticProviderSection {
    DiagnosticProviderSection {
        id: descriptor.id.as_str(),
        status: DiagnosticProviderStatus::Unsupported,
        complete: false,
        version: None,
        capabilities: (action == DiagnosticAction::Catalog).then(|| descriptor.into()),
        readiness: None,
        items_total: action_has_items(action).then_some(0),
        items_returned: action_has_items(action).then_some(0),
        resource_failures: action_has_observations(action).then_some(0),
        truncated: action_has_items(action).then_some(false),
        error: Some(DiagnosticError {
            code: "target_not_supported".to_string(),
            message: format!(
                "provider {} does not support {} for the selected target",
                descriptor.id.as_str(),
                action.as_str()
            ),
            retryable: false,
        }),
    }
}

fn failed_mapping_section(
    descriptor: &DiagnosticProviderDescriptor,
    version: Option<String>,
    error: DiagnosticMapError,
    action: DiagnosticAction,
) -> DiagnosticProviderSection {
    DiagnosticProviderSection {
        id: descriptor.id.as_str(),
        status: DiagnosticProviderStatus::Failed,
        complete: false,
        version,
        capabilities: (action == DiagnosticAction::Catalog).then(|| descriptor.into()),
        readiness: None,
        items_total: action_has_items(action).then_some(0),
        items_returned: action_has_items(action).then_some(0),
        resource_failures: action_has_observations(action).then_some(0),
        truncated: action_has_items(action).then_some(false),
        error: Some(DiagnosticError {
            code: error.code.to_string(),
            message: error.message,
            retryable: false,
        }),
    }
}

fn item_matches_request(
    item: &DiagnosticItem,
    request: &DiagnosticRequest,
    context: &DiagnosticContext,
) -> bool {
    if let DiagnosticItem::Diagnostic {
        provider,
        code,
        severity,
        ..
    } = item
    {
        if request
            .filter
            .min_severity
            .is_some_and(|minimum| severity_rank(*severity) < severity_rank(minimum))
        {
            return false;
        }
        if !request.filter.codes.is_empty()
            && !request
                .filter
                .codes
                .iter()
                .any(|filter| filter.provider == *provider && filter.code == *code)
        {
            return false;
        }
    }
    let Some(requested_range) = request.range else {
        return true;
    };
    let location = item_location(item);
    if !location.is_some_and(|location| location_matches_target(location, &context.target)) {
        return false;
    }
    match item {
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::SourceRange { range },
            ..
        } => range.intersects(requested_range),
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::Target,
            ..
        }
        | DiagnosticItem::ResourceFailure { .. } => true,
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::Metadata { .. },
            ..
        }
        | DiagnosticItem::DiagnosticRule { .. } => false,
    }
}

fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Hint => 0,
        DiagnosticSeverity::Info => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Error => 3,
    }
}

fn location_matches_target(
    location: &DiagnosticLocation,
    target: &crate::domain::source_target::ResolvedTarget,
) -> bool {
    matches!(
        location,
        DiagnosticLocation::Addressed {
            source_set,
            metadata_path,
            target_kind,
        } if source_set == &target.source_set
            && metadata_path == &target.metadata_path
            && target_kind == &target.target_kind
    )
}

fn item_location(item: &DiagnosticItem) -> Option<&DiagnosticLocation> {
    match item {
        DiagnosticItem::Diagnostic { location, .. }
        | DiagnosticItem::ResourceFailure { location, .. } => Some(location),
        DiagnosticItem::DiagnosticRule { .. } => None,
    }
}

fn diagnostic_sort_key(
    item: &DiagnosticItem,
    provider_order: &HashMap<&str, usize>,
) -> (String, String, usize, u8, String, String) {
    let location = match item_location(item) {
        Some(DiagnosticLocation::Addressed {
            source_set,
            metadata_path,
            target_kind,
        }) => format!(
            "0|{source_set}|{}|{:?}",
            metadata_path.as_ref().map_or("", MetadataAddress::as_str),
            target_kind
        ),
        Some(DiagnosticLocation::Unaddressable {
            source_set,
            owner_metadata_path,
            observed_path,
            ..
        }) => format!(
            "1|{source_set}|{}|{observed_path}",
            owner_metadata_path
                .as_ref()
                .map_or("", MetadataAddress::as_str)
        ),
        None => "2".to_string(),
    };
    let focus = match item {
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::Target,
            ..
        }
        | DiagnosticItem::ResourceFailure { .. } => "0".to_string(),
        DiagnosticItem::Diagnostic {
            focus: DiagnosticFocus::SourceRange { range },
            ..
        } => format!(
            "1|{:020}|{:020}|{:020}|{:020}",
            range.start_line, range.start_column, range.end_line, range.end_column
        ),
        DiagnosticItem::Diagnostic {
            focus:
                DiagnosticFocus::Metadata {
                    element_path,
                    property,
                    language,
                },
            ..
        } => format!(
            "2|{}|{}|{}",
            element_path
                .iter()
                .map(|element| format!("{}:{}", element.collection, element.name))
                .collect::<Vec<_>>()
                .join("/"),
            property.as_deref().unwrap_or(""),
            language.as_deref().unwrap_or("")
        ),
        DiagnosticItem::DiagnosticRule { .. } => "3".to_string(),
    };
    let provider = item_provider(item);
    let provider_rank = provider_order.get(provider).copied().unwrap_or(usize::MAX);
    let (kind, code, message) = match item {
        DiagnosticItem::Diagnostic { code, message, .. } => (0, code.clone(), message.clone()),
        DiagnosticItem::ResourceFailure { error, .. } => {
            (1, error.code.clone(), error.message.clone())
        }
        DiagnosticItem::DiagnosticRule { code, title, .. } => (2, code.clone(), title.clone()),
    };
    (location, focus, provider_rank, kind, code, message)
}

fn item_provider(item: &DiagnosticItem) -> &'static str {
    match item {
        DiagnosticItem::Diagnostic { provider, .. }
        | DiagnosticItem::ResourceFailure { provider, .. }
        | DiagnosticItem::DiagnosticRule { provider, .. } => provider,
    }
}

fn action_has_observations(action: DiagnosticAction) -> bool {
    matches!(
        action,
        DiagnosticAction::Analyze | DiagnosticAction::Findings
    )
}

fn action_has_items(action: DiagnosticAction) -> bool {
    action != DiagnosticAction::Status
}

pub(crate) fn parse_diagnostic_request(
    args: &Map<String, Value>,
) -> Result<DiagnosticRequest, DiagnosticRequestError> {
    let action_raw = required_string(args, "action")?;
    let action = DiagnosticAction::parse(action_raw).ok_or_else(|| {
        request_error(
            "action_invalid",
            Some("action"),
            "action must be analyze, findings, status, or catalog",
        )
    })?;
    let source_set = required_string(args, "sourceSet")?.to_string();
    let metadata_path = args
        .get("metadataPath")
        .and_then(Value::as_str)
        .map(|raw| MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw))
        .transpose()
        .map_err(|_| {
            request_error(
                "metadata_address_invalid",
                Some("metadataPath"),
                "metadataPath is not a valid logical address",
            )
        })?;
    let requested_providers = args.get("providers").map(|providers| {
        providers
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let default_minimum = matches!(
        action,
        DiagnosticAction::Analyze | DiagnosticAction::Findings
    )
    .then_some(DiagnosticSeverity::Warning);
    let filter_value = args.get("filter").and_then(Value::as_object);
    let min_severity = filter_value
        .and_then(|filter| filter.get("minSeverity"))
        .and_then(Value::as_str)
        .map(parse_severity)
        .transpose()?
        .or(default_minimum);
    let codes = filter_value
        .and_then(|filter| filter.get("codes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|code| DiagnosticCodeFilter {
            provider: code
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            code: code
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    let range = args
        .get("range")
        .and_then(Value::as_object)
        .map(|range| DiagnosticRange {
            start_line: json_usize(range, "startLine"),
            start_column: json_usize(range, "startColumn"),
            end_line: json_usize(range, "endLine"),
            end_column: json_usize(range, "endColumn"),
        });
    if range.is_some_and(|range| !range.is_non_empty()) {
        return Err(request_error(
            "range_invalid",
            Some("range"),
            "range must be ordered, non-empty, zero-based, and end-exclusive",
        ));
    }
    Ok(DiagnosticRequest {
        action,
        source_set,
        metadata_path,
        requested_providers,
        filter: DiagnosticFilter {
            min_severity,
            codes,
        },
        range,
        limit: args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(DIAGNOSTIC_LIMIT_DEFAULT),
        timeout: args
            .get("timeoutSeconds")
            .and_then(Value::as_u64)
            .map(Duration::from_secs),
    })
}

fn required_string<'a>(
    args: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, DiagnosticRequestError> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.trim() == *value)
        .ok_or_else(|| {
            request_error(
                "required_argument_missing",
                Some(field),
                format!("{field} must be a non-empty unpadded string"),
            )
        })
}

fn parse_severity(value: &str) -> Result<DiagnosticSeverity, DiagnosticRequestError> {
    match value {
        "error" => Ok(DiagnosticSeverity::Error),
        "warning" => Ok(DiagnosticSeverity::Warning),
        "info" => Ok(DiagnosticSeverity::Info),
        "hint" => Ok(DiagnosticSeverity::Hint),
        _ => Err(request_error(
            "severity_invalid",
            Some("filter.minSeverity"),
            "minSeverity must be error, warning, info, or hint",
        )),
    }
}

fn json_usize(object: &Map<String, Value>, field: &str) -> usize {
    object
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn request_error(
    code: &'static str,
    field: Option<&'static str>,
    message: impl Into<String>,
) -> DiagnosticRequestError {
    DiagnosticRequestError {
        code,
        field,
        message: message.into(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_diagnostic_request, DiagnosticCoordinator, DiagnosticMapping};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::diagnostics::*;
    use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetKind};
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::source_target::{
        MetadataAddress, ResolvedTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map, Value};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const ANALYZER: DiagnosticProviderId = DiagnosticProviderId::new_const("bsl-analyzer");
    const LANGUAGE_SERVER: DiagnosticProviderId =
        DiagnosticProviderId::new_const("bsl-language-server");
    const METADATA_VALIDATOR: DiagnosticProviderId =
        DiagnosticProviderId::new_const("metadata-validator");

    static ANALYZER_DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
        id: ANALYZER,
        actions: &[
            DiagnosticAction::Analyze,
            DiagnosticAction::Findings,
            DiagnosticAction::Status,
            DiagnosticAction::Catalog,
        ],
        findings_target_kinds: &[TargetKind::Module],
        emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
    };
    static LANGUAGE_SERVER_DESCRIPTOR: DiagnosticProviderDescriptor =
        DiagnosticProviderDescriptor {
            id: LANGUAGE_SERVER,
            actions: &[DiagnosticAction::Findings, DiagnosticAction::Status],
            findings_target_kinds: &[TargetKind::Module],
            emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
        };
    static METADATA_DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
        id: METADATA_VALIDATOR,
        actions: &[DiagnosticAction::Analyze, DiagnosticAction::Findings],
        findings_target_kinds: &[TargetKind::MetadataObject],
        emits_focus_kinds: &[DiagnosticFocusKind::Metadata],
    };

    struct FakeProvider {
        descriptor: &'static DiagnosticProviderDescriptor,
        outcome: DiagnosticProviderOutcome,
        calls: Arc<AtomicUsize>,
    }

    impl DiagnosticProvider for FakeProvider {
        fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
            self.descriptor
        }

        fn execute(
            &self,
            _request: &DiagnosticProviderRequest,
            _context: &DiagnosticContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> DiagnosticProviderOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcome.clone()
        }
    }

    #[derive(Default)]
    struct FakeMapping;

    impl DiagnosticMapping for FakeMapping {
        fn resolve_context(
            &self,
            request: &DiagnosticRequest,
            workspace: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> Result<DiagnosticContext, DiagnosticRequestError> {
            let target_kind = request
                .metadata_path
                .as_ref()
                .map_or(TargetKind::SourceRoot, MetadataAddress::target_kind);
            Ok(DiagnosticContext::new(
                workspace.clone(),
                ProjectSourceSet {
                    name: request.source_set.clone(),
                    kind: SourceSetKind::Configuration,
                    path: "src".to_string(),
                    source_format: SourceFormat::PlatformXml,
                    format_evidence: Vec::new(),
                },
                ResolvedSourceRoot {
                    source_set: Some(request.source_set.clone()),
                    path: workspace.workspace_root.join("src"),
                },
                ResolvedTarget {
                    source_set: request.source_set.clone(),
                    metadata_path: request.metadata_path.clone(),
                    target_kind,
                },
            ))
        }

        fn map_observation(
            &self,
            observation: DiagnosticObservation,
            context: &DiagnosticContext,
            _cancellation: &CancellationToken,
        ) -> Result<DiagnosticItem, DiagnosticMapError> {
            let (provider, observation_location, focus, payload) = match observation {
                DiagnosticObservation::Diagnostic {
                    provider,
                    location,
                    focus,
                    code,
                    severity,
                    message,
                    tags,
                } => (
                    provider,
                    location,
                    focus,
                    Some((code, severity, message, tags)),
                ),
                DiagnosticObservation::ResourceFailure {
                    provider,
                    location,
                    error,
                } => {
                    let location = fake_location(location, context)?;
                    return Ok(DiagnosticItem::ResourceFailure {
                        provider: provider.as_str(),
                        location,
                        error,
                    });
                }
            };
            let location = fake_location(observation_location, context)?;
            let focus = match focus {
                DiagnosticObservationFocus::Target => DiagnosticFocus::Target,
                DiagnosticObservationFocus::SourceRange(range) => {
                    DiagnosticFocus::SourceRange { range }
                }
                DiagnosticObservationFocus::Metadata(focus) => focus.into(),
            };
            let (code, severity, message, tags) = payload.unwrap();
            Ok(DiagnosticItem::Diagnostic {
                provider: provider.as_str(),
                location,
                focus,
                code,
                severity,
                message,
                tags,
            })
        }
    }

    fn fake_location(
        location: DiagnosticObservationLocation,
        context: &DiagnosticContext,
    ) -> Result<DiagnosticLocation, DiagnosticMapError> {
        let metadata_path = match location {
            DiagnosticObservationLocation::Logical { metadata_path } => metadata_path,
            DiagnosticObservationLocation::Resource { handle } if handle == "outside" => {
                return Err(DiagnosticMapError {
                    code: "location_outside_source_set",
                    message: "provider resource is outside the selected sourceSet".to_string(),
                })
            }
            DiagnosticObservationLocation::Resource { handle } if handle == "selected" => {
                context.target.metadata_path.clone()
            }
            DiagnosticObservationLocation::Resource { handle } => {
                Some(address(&format!("CommonModule.{handle}.Module")))
            }
        };
        let target_kind = metadata_path
            .as_ref()
            .map_or(TargetKind::SourceRoot, MetadataAddress::target_kind);
        Ok(DiagnosticLocation::Addressed {
            source_set: context.target.source_set.clone(),
            metadata_path,
            target_kind,
        })
    }

    fn provider(
        descriptor: &'static DiagnosticProviderDescriptor,
        outcome: DiagnosticProviderOutcome,
    ) -> (Arc<dyn DiagnosticProvider>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(FakeProvider {
                descriptor,
                outcome,
                calls: Arc::clone(&calls),
            }),
            calls,
        )
    }

    fn successful(observations: Vec<DiagnosticObservation>) -> DiagnosticProviderOutcome {
        DiagnosticProviderOutcome {
            status: if observations.is_empty() {
                DiagnosticProviderStatus::Empty
            } else {
                DiagnosticProviderStatus::Completed
            },
            complete: true,
            version: Some("test".to_string()),
            observations,
            rules: Vec::new(),
            readiness: None,
            error: None,
        }
    }

    fn fake_registry(
        outcomes: [DiagnosticProviderOutcome; 3],
    ) -> (DiagnosticProviderRegistry, [Arc<AtomicUsize>; 3]) {
        let (analyzer, analyzer_calls) = provider(&ANALYZER_DESCRIPTOR, outcomes[0].clone());
        let (language_server, language_server_calls) =
            provider(&LANGUAGE_SERVER_DESCRIPTOR, outcomes[1].clone());
        let (metadata, metadata_calls) = provider(&METADATA_DESCRIPTOR, outcomes[2].clone());
        (
            DiagnosticProviderRegistry::new(vec![analyzer, language_server, metadata]).unwrap(),
            [analyzer_calls, language_server_calls, metadata_calls],
        )
    }

    fn address(raw: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap()
    }

    fn workspace() -> WorkspaceContext {
        WorkspaceContext {
            cwd: PathBuf::from("workspace"),
            workspace_root: PathBuf::from("workspace"),
            cache_root: PathBuf::from("workspace/.build/unica"),
            workspace_epoch: 1,
        }
    }

    fn findings_request() -> DiagnosticRequest {
        DiagnosticRequest {
            action: DiagnosticAction::Findings,
            source_set: "main".to_string(),
            metadata_path: Some(address("CommonModule.Selected.Module")),
            requested_providers: None,
            filter: DiagnosticFilter::default(),
            range: None,
            limit: 200,
            timeout: None,
        }
    }

    fn diagnostic(
        provider: DiagnosticProviderId,
        handle: &str,
        code: &str,
        severity: DiagnosticSeverity,
        focus: DiagnosticObservationFocus,
    ) -> DiagnosticObservation {
        DiagnosticObservation::Diagnostic {
            provider,
            location: DiagnosticObservationLocation::Resource {
                handle: handle.to_string(),
            },
            focus,
            code: code.to_string(),
            severity,
            message: format!("message {code}"),
            tags: Vec::new(),
        }
    }

    fn run(
        registry: DiagnosticProviderRegistry,
        request: &DiagnosticRequest,
    ) -> Result<DiagnosticResult, DiagnosticRequestError> {
        DiagnosticCoordinator::new(registry, &FakeMapping).execute(
            request,
            &workspace(),
            &CancellationToken::new(),
        )
    }

    #[test]
    fn diagnostics_provider_selection_uses_registry_order_and_keeps_explicit_unsupported() {
        let empty = successful(Vec::new());
        let (registry, calls) = fake_registry([empty.clone(), empty.clone(), empty]);
        let automatic = run(registry, &findings_request()).unwrap();
        assert_eq!(
            automatic.selection.providers,
            vec!["bsl-analyzer", "bsl-language-server"]
        );
        assert_eq!(automatic.state, DiagnosticResultState::Completed);
        assert!(automatic.complete);
        assert_eq!(calls[2].load(Ordering::SeqCst), 0);

        let empty = successful(Vec::new());
        let (registry, calls) = fake_registry([empty.clone(), empty.clone(), empty]);
        let mut explicit = findings_request();
        explicit.requested_providers = Some(vec![
            "metadata-validator".to_string(),
            "bsl-language-server".to_string(),
            "bsl-analyzer".to_string(),
        ]);
        let result = run(registry, &explicit).unwrap();
        assert_eq!(
            result.selection.providers,
            vec!["bsl-analyzer", "bsl-language-server", "metadata-validator"]
        );
        assert_eq!(
            result.providers[2].status,
            DiagnosticProviderStatus::Unsupported
        );
        assert_eq!(calls[2].load(Ordering::SeqCst), 0);
    }

    #[test]
    fn diagnostics_provider_selection_honors_code_filters_before_execution() {
        let empty = successful(Vec::new());
        let (registry, calls) = fake_registry([empty.clone(), empty.clone(), empty]);
        let mut narrowed = findings_request();
        narrowed.filter.codes = vec![DiagnosticCodeFilter {
            provider: "bsl-language-server".to_string(),
            code: "UNKNOWN".to_string(),
        }];
        let result = run(registry, &narrowed).unwrap();
        assert_eq!(result.selection.providers, vec!["bsl-language-server"]);
        assert_eq!(result.items_total, Some(0));
        assert_eq!(calls[0].load(Ordering::SeqCst), 0);
        assert_eq!(calls[1].load(Ordering::SeqCst), 1);

        let empty = successful(Vec::new());
        let (registry, calls) = fake_registry([empty.clone(), empty.clone(), empty]);
        let mut inconsistent = findings_request();
        inconsistent.requested_providers = Some(vec!["bsl-analyzer".to_string()]);
        inconsistent.filter.codes = vec![DiagnosticCodeFilter {
            provider: "bsl-language-server".to_string(),
            code: "LS001".to_string(),
        }];
        let error = run(registry, &inconsistent).unwrap_err();
        assert_eq!(error.code, "filter_provider_not_selected");
        assert!(calls.iter().all(|calls| calls.load(Ordering::SeqCst) == 0));
    }

    #[test]
    fn diagnostics_provider_selection_rejects_no_automatic_applicable_provider() {
        let empty = successful(Vec::new());
        let (registry, calls) = fake_registry([empty.clone(), empty.clone(), empty]);
        let mut request = findings_request();
        request.metadata_path = Some(address("Catalog.Items"));
        request.filter.codes = vec![DiagnosticCodeFilter {
            provider: "bsl-analyzer".to_string(),
            code: "ANY".to_string(),
        }];
        let error = run(registry, &request).unwrap_err();
        assert_eq!(error.code, "no_applicable_provider");
        assert!(calls.iter().all(|calls| calls.load(Ordering::SeqCst) == 0));
    }

    #[test]
    fn diagnostics_result_assembly_filters_sorts_and_applies_one_global_limit() {
        let requested_range = DiagnosticRange {
            start_line: 2,
            start_column: 0,
            end_line: 4,
            end_column: 0,
        };
        let analyzer = successful(vec![
            diagnostic(
                ANALYZER,
                "selected",
                "Z-WARNING",
                DiagnosticSeverity::Warning,
                DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                    start_line: 2,
                    start_column: 0,
                    end_line: 2,
                    end_column: 2,
                }),
            ),
            diagnostic(
                ANALYZER,
                "Alpha",
                "A-INFO",
                DiagnosticSeverity::Info,
                DiagnosticObservationFocus::Target,
            ),
            diagnostic(
                ANALYZER,
                "selected",
                "SELECTED",
                DiagnosticSeverity::Error,
                DiagnosticObservationFocus::Target,
            ),
            DiagnosticObservation::ResourceFailure {
                provider: ANALYZER,
                location: DiagnosticObservationLocation::Resource {
                    handle: "selected".to_string(),
                },
                error: DiagnosticError {
                    code: "source_decode_failed".to_string(),
                    message: "decode failed".to_string(),
                    retryable: false,
                },
            },
        ]);
        let language_server = successful(vec![diagnostic(
            LANGUAGE_SERVER,
            "selected",
            "SELECTED",
            DiagnosticSeverity::Error,
            DiagnosticObservationFocus::Target,
        )]);
        let (registry, _) = fake_registry([analyzer, language_server, successful(Vec::new())]);
        let mut request = findings_request();
        request.range = Some(requested_range);
        request.limit = 3;
        request.filter.min_severity = Some(DiagnosticSeverity::Warning);

        let result = run(registry, &request).unwrap();
        assert_eq!(result.items_total, Some(4));
        assert_eq!(result.items_returned, Some(3));
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.truncated, Some(true));
        assert!(
            result.complete,
            "global truncation does not reduce provider coverage"
        );
        assert_eq!(result.providers[0].resource_failures, Some(1));
        assert_eq!(result.providers[0].items_total, Some(3));
        assert_eq!(result.providers[0].items_returned, Some(2));
        assert_eq!(result.providers[1].items_total, Some(1));
        assert_eq!(result.providers[1].items_returned, Some(1));

        let retained = result
            .items
            .iter()
            .map(|item| match item {
                DiagnosticItem::Diagnostic { provider, code, .. } => {
                    format!("{provider}:{code}")
                }
                DiagnosticItem::ResourceFailure { provider, .. } => {
                    format!("{provider}:resourceFailure")
                }
                DiagnosticItem::DiagnosticRule { .. } => "rule".to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            retained,
            vec![
                "bsl-analyzer:SELECTED",
                "bsl-analyzer:resourceFailure",
                "bsl-language-server:SELECTED"
            ]
        );
    }

    #[test]
    fn diagnostics_result_assembly_keeps_cross_provider_duplicates_and_metadata_focus_order() {
        let metadata_outcome = successful(vec![
            DiagnosticObservation::Diagnostic {
                provider: METADATA_VALIDATOR,
                location: DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                focus: DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: vec![MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Zeta".to_string(),
                    }],
                    property: Some("Type".to_string()),
                    language: None,
                }),
                code: "META".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: "same".to_string(),
                tags: Vec::new(),
            },
            DiagnosticObservation::Diagnostic {
                provider: METADATA_VALIDATOR,
                location: DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                focus: DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: vec![MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Alpha".to_string(),
                    }],
                    property: Some("Type".to_string()),
                    language: Some("ru".to_string()),
                }),
                code: "META".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: "same".to_string(),
                tags: Vec::new(),
            },
        ]);
        let empty = successful(Vec::new());
        let (registry, _) = fake_registry([empty.clone(), empty, metadata_outcome]);
        let mut request = findings_request();
        request.metadata_path = Some(address("Catalog.Items"));
        let result = run(registry, &request).unwrap();
        let paths = result
            .items
            .iter()
            .map(|item| match item {
                DiagnosticItem::Diagnostic {
                    focus: DiagnosticFocus::Metadata { element_path, .. },
                    ..
                } => element_path[0].name.as_str(),
                item => panic!("unexpected item: {item:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["Alpha", "Zeta"]);
    }

    #[test]
    fn diagnostics_request_parser_materializes_defaults_and_stable_errors() {
        let args = Map::from_iter([
            ("action".to_string(), json!("findings")),
            ("sourceSet".to_string(), json!("main")),
            (
                "metadataPath".to_string(),
                json!("CommonModule.Shared.Module"),
            ),
        ]);
        let request = parse_diagnostic_request(&args).unwrap();
        assert_eq!(request.action, DiagnosticAction::Findings);
        assert_eq!(request.limit, 200);
        assert_eq!(
            request.filter.min_severity,
            Some(DiagnosticSeverity::Warning)
        );
        assert!(request.filter.codes.is_empty());

        let invalid = Map::<String, Value>::from_iter([
            ("action".to_string(), json!("findings")),
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("not-an-address")),
        ]);
        let error = parse_diagnostic_request(&invalid).unwrap_err();
        assert_eq!(error.code, "metadata_address_invalid");
        assert_eq!(error.field, Some("metadataPath"));
        assert!(!error.retryable);
    }
}
