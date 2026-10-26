use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
#[cfg(test)]
use serde_json::json;
use serde_json::{Map, Value};
#[cfg(test)]
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::MetaValidationStatus;
#[cfg(test)]
use unica_metadata::*;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;

pub(crate) fn invoke(
    operation: unica_metadata::request::MetadataOperation,
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let request = match unica_metadata::request::parse_metadata_request(operation, args) {
        Ok(request) => request,
        Err(failure) => {
            return Ok(metadata_failure(
                "metadata arguments are invalid",
                failure,
                None,
            ))
        }
    };
    match request {
        unica_metadata::request::MetadataRequest::Info(request) => {
            invoke_info(ports, &request, context, cancellation)
        }
        request => invoke_mutation(ports, request, context, cancellation),
    }
}

fn invoke_info(
    ports: &dyn ApplicationPorts,
    request: &unica_metadata::request::MetaInfoRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let read = match ports.read_metadata_local(request, context, cancellation) {
        Ok(read) => read,
        Err(failure) => return Ok(metadata_failure("metadata read failed", failure, None)),
    };
    let mut validation =
        ports.validate_metadata_read(&read.validation_subject, context, cancellation);
    if read
        .local
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == unica_metadata::MetaDiagnosticSeverity::Error)
    {
        validation.status = MetaValidationStatus::Failed;
    }
    validation
        .diagnostics
        .extend(read.local.diagnostics.iter().cloned());
    // The enrichment sections are read from the source tree and answer on their
    // own evidence, so a descriptor that failed validation says nothing about
    // whether they are correct. Withholding them made an unrelated failure cost
    // the caller data that was never in question (ADR-0028).
    let enrichment = if request.sections.is_empty() {
        Default::default()
    } else {
        ports.read_metadata_related(request, &read.local, context, cancellation)
    };
    if enrichment
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == unica_metadata::MetaDiagnosticSeverity::Error)
    {
        validation.status = MetaValidationStatus::Failed;
    }
    validation
        .diagnostics
        .extend(enrichment.diagnostics.iter().cloned());
    let failed = validation.status == MetaValidationStatus::Failed;
    let (functional_subsystems, interface_subsystems) =
        match &read.validation_subject.subsystem_evidence {
            Some(unica_metadata::validation_subject::MetadataSubsystemEvidence::Complete {
                functional_subsystems,
                interface_subsystems,
            }) => (
                Some(functional_subsystems.clone()),
                Some(interface_subsystems.clone()),
            ),
            Some(unica_metadata::validation_subject::MetadataSubsystemEvidence::Unavailable(_))
            | None => (None, None),
        };
    let diagnostics = validation.diagnostics.clone();
    let data = serde_json::to_value(read.local.into_info(
        validation,
        enrichment,
        functional_subsystems,
        interface_subsystems,
    ))
    .map_err(|error| format!("cannot serialize metadata info result: {error}"))?;
    if failed {
        return Ok(metadata_failure(
            "metadata validation failed",
            unica_metadata::request::MetaFailure { diagnostics },
            Some(data),
        ));
    }
    Ok(metadata_success(
        "metadata information inspected",
        data,
        Vec::new(),
        Vec::new(),
    ))
}

fn invoke_mutation(
    ports: &dyn ApplicationPorts,
    request: unica_metadata::request::MetadataRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let dry_run = mutation_dry_run(&request);
    let prepared = match ports.prepare_metadata_mutation(&request, context, cancellation) {
        Ok(prepared) => prepared,
        Err(failure) => {
            return Ok(metadata_failure(
                "metadata mutation is unavailable",
                failure,
                None,
            ))
        }
    };
    let validation = ports.validate_metadata(prepared.validation_subject(), context, cancellation);
    if validation.status == MetaValidationStatus::Failed {
        let diagnostics = validation.diagnostics.clone();
        let mut data = prepared.preview().clone();
        data.validation = validation;
        let data = serde_json::to_value(data)
            .map_err(|error| format!("cannot serialize metadata mutation result: {error}"))?;
        return Ok(metadata_failure(
            "metadata validation failed",
            unica_metadata::request::MetaFailure { diagnostics },
            Some(data),
        ));
    }

    if dry_run {
        let mut data = prepared.preview().clone();
        data.validation = validation;
        let projected_events = metadata_change_event(&data, true);
        let (changes, artifacts) = mutation_receipt_lines(&data, false);
        let data = serde_json::to_value(data)
            .map_err(|error| format!("cannot serialize metadata mutation preview: {error}"))?;
        let mut outcome = metadata_success(
            "metadata mutation preview prepared",
            data,
            Vec::new(),
            projected_events,
        );
        outcome.adapter.changes = changes;
        outcome.adapter.artifacts = artifacts;
        return Ok(outcome);
    }

    if cancellation.is_cancelled() {
        return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(
            "metadata mutation stopped before publication",
        )));
    }

    let report = match prepared.publish(cancellation) {
        Ok(report) => report,
        Err(failure) => {
            return Ok(metadata_failure(
                "metadata publication failed",
                failure,
                None,
            ))
        }
    };
    let mut data = report.data;
    data.validation = validation;
    let events = if report.events.is_empty() {
        metadata_change_event(&data, false)
    } else {
        report.events
    };
    let (changes, artifacts) = mutation_receipt_lines(&data, true);
    let data = serde_json::to_value(data)
        .map_err(|error| format!("cannot serialize metadata mutation result: {error}"))?;
    let mut outcome = metadata_success("metadata mutation published", data, events, Vec::new());
    outcome.recorded_cache = report.recorded_cache;
    outcome.adapter.warnings = report.warnings;
    outcome.adapter.changes = changes;
    outcome.adapter.artifacts = artifacts;
    Ok(outcome)
}

/// Конверт квитанции строится из тех же затронутых путей, что несёт
/// типизированный результат: применение — фактические строки, предпросмотр —
/// план.
fn mutation_receipt_lines(
    data: &unica_metadata::MetaMutationData,
    applied: bool,
) -> (Vec<String>, Vec<String>) {
    use unica_metadata::MetaPublicationAction;
    let mut changes = Vec::new();
    let mut artifacts = Vec::new();
    for change in &data.changed_paths {
        let verb = match (applied, change.action) {
            (true, MetaPublicationAction::Create) => "created",
            (true, MetaPublicationAction::Update) => "updated",
            (true, MetaPublicationAction::Remove) => "removed",
            (false, MetaPublicationAction::Create) => "would create",
            (false, MetaPublicationAction::Update) => "would update",
            (false, MetaPublicationAction::Remove) => "would remove",
        };
        changes.push(format!("{verb} {}", change.path));
        artifacts.push(change.path.clone());
    }
    (changes, artifacts)
}

fn mutation_dry_run(request: &unica_metadata::request::MetadataRequest) -> bool {
    match request {
        unica_metadata::request::MetadataRequest::Info(_) => false,
        unica_metadata::request::MetadataRequest::Add(request) => request.dry_run,
        unica_metadata::request::MetadataRequest::Edit(request) => request.dry_run,
    }
}

fn metadata_change_event(
    data: &unica_metadata::MetaMutationData,
    projected: bool,
) -> Vec<DomainEvent> {
    if !data.changed {
        return Vec::new();
    }
    let artifact = if projected {
        format!("preview:{}", data.metadata_path.as_str())
    } else {
        data.metadata_path.as_str().to_string()
    };
    vec![DomainEvent::new(DomainEventKind::MetadataChanged, artifact)]
}

fn metadata_success(
    summary: &str,
    data: Value,
    events: Vec<DomainEvent>,
    projected_events: Vec<DomainEvent>,
) -> HandlerOutcome {
    HandlerOutcome {
        adapter: AdapterOutcome::ok(summary),
        data: Some(data),
        job: None,
        events,
        projected_events,
        recorded_cache: None,
        diagnostics: None,
    }
}

fn metadata_failure(
    summary: &str,
    failure: unica_metadata::request::MetaFailure,
    data: Option<Value>,
) -> HandlerOutcome {
    let errors = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    let diagnostics = serde_json::to_value(&failure.diagnostics)
        .expect("metadata diagnostics are always serializable");
    HandlerOutcome {
        adapter: AdapterOutcome {
            ok: false,
            summary: summary.to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors,
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
        data,
        job: None,
        events: Vec::new(),
        projected_events: Vec::new(),
        recorded_cache: None,
        diagnostics: Some(diagnostics),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{
        ApplicationPorts, HandlerOutcome, MetaPublishReport, PreparedMetadataMutation,
        SupportGuardCheck,
    };
    use crate::application::{InvocationMode, ToolSpec};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use unica_metadata::{
        MetaCollectionsData, MetaDiagnosticCode, MetaInfoDetails, MetaMutationData,
        MetaSupportStatus, MetaValidationData, MetaValidationStatus, MetadataKind,
    };
    use unica_source_contract::events::{DomainEvent, DomainEventKind};
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;
    use unica_workspace::cache::{CacheAccess, CacheReport};

    fn object(value: Value) -> Map<String, Value> {
        value
            .as_object()
            .expect("test input must be an object")
            .clone()
    }

    #[derive(Default)]
    struct CoordinatorState {
        calls: Vec<&'static str>,
        publish_calls: usize,
    }

    struct FakePreparedMutation {
        state: Arc<Mutex<CoordinatorState>>,
        preview: MetaMutationData,
        validation_subject: unica_metadata::validation_subject::MetadataValidationSubject,
        publication: Result<MetaPublishReport, unica_metadata::request::MetaFailure>,
    }

    impl PreparedMetadataMutation for FakePreparedMutation {
        fn preview(&self) -> &MetaMutationData {
            &self.preview
        }

        fn validation_subject(
            &self,
        ) -> &unica_metadata::validation_subject::MetadataValidationSubject {
            &self.validation_subject
        }

        fn publish(
            self: Box<Self>,
            _cancellation: &CancellationToken,
        ) -> Result<MetaPublishReport, unica_metadata::request::MetaFailure> {
            let mut state = self.state.lock().unwrap();
            state.calls.push("publish");
            state.publish_calls += 1;
            drop(state);
            self.publication
        }
    }

    struct FakeMetadataPorts {
        state: Arc<Mutex<CoordinatorState>>,
        cache_events: Arc<Mutex<Vec<DomainEvent>>>,
        read: Mutex<
            Option<
                Result<
                    unica_metadata::validation_subject::MetadataRead,
                    unica_metadata::request::MetaFailure,
                >,
            >,
        >,
        related: unica_metadata::validation_subject::MetaEnrichment,
        validation: MetaValidationData,
        prepared: Mutex<
            Option<Result<Box<dyn PreparedMetadataMutation>, unica_metadata::request::MetaFailure>>,
        >,
        cancel_after_validation: Option<CancellationToken>,
    }

    impl ApplicationPorts for FakeMetadataPorts {
        fn discover_workspace(
            &self,
            _requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            Ok(coordinator_context())
        }

        fn validate_tool_context(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _mode: InvocationMode,
            _context: &WorkspaceContext,
        ) -> Result<(), String> {
            Ok(())
        }

        fn read_metadata_local(
            &self,
            _request: &unica_metadata::request::MetaInfoRequest,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> Result<
            unica_metadata::validation_subject::MetadataRead,
            unica_metadata::request::MetaFailure,
        > {
            self.state.lock().unwrap().calls.push("read");
            self.read
                .lock()
                .unwrap()
                .take()
                .expect("read is called exactly once")
        }

        fn read_metadata_related(
            &self,
            _request: &unica_metadata::request::MetaInfoRequest,
            _local: &unica_metadata::validation_subject::MetaLocalInfo,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> unica_metadata::validation_subject::MetaEnrichment {
            self.state.lock().unwrap().calls.push("related");
            self.related.clone()
        }

        fn validate_metadata(
            &self,
            _subject: &unica_metadata::validation_subject::MetadataValidationSubject,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> MetaValidationData {
            self.state.lock().unwrap().calls.push("validate");
            if let Some(cancellation) = self.cancel_after_validation.as_ref() {
                cancellation.cancel();
            }
            self.validation.clone()
        }

        fn prepare_metadata_mutation(
            &self,
            _request: &unica_metadata::request::MetadataRequest,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> Result<Box<dyn PreparedMetadataMutation>, unica_metadata::request::MetaFailure>
        {
            self.state.lock().unwrap().calls.push("prepare");
            self.prepared
                .lock()
                .unwrap()
                .take()
                .expect("mutation is prepared exactly once")
        }

        fn evaluate_support_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            Ok(SupportGuardCheck::Allow)
        }

        fn invoke_handler(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _mode: InvocationMode,
            _cancellation: &CancellationToken,
        ) -> Result<HandlerOutcome, String> {
            unreachable!("metadata has a dedicated coordinator")
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            events: &[DomainEvent],
            mode: InvocationMode,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            *self.cache_events.lock().unwrap() = events.to_vec();
            Ok(CacheReport {
                mode: if mode.is_preview() {
                    "preview"
                } else {
                    "apply"
                }
                .to_string(),
                root: context.cache_root.display().to_string(),
                workspace_epoch: context.workspace_epoch,
                events: events
                    .iter()
                    .map(|event| event.name().to_string())
                    .collect(),
                invalidated: Vec::new(),
                refreshed: Vec::new(),
                lazy_rebuilt: Vec::new(),
                stale: Vec::new(),
                fresh: Vec::new(),
                publication_warnings: Vec::new(),
            })
        }
        fn notify_committed_changes(
            &self,
            context: &WorkspaceContext,
            events: &[unica_source_contract::events::DomainEvent],
            _cancellation: &CancellationToken,
            _deadline: unica_work_control::operation_deadline::OperationDeadline,
        ) {
            self.notify_invalidation(context, events);
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    fn coordinator_context() -> WorkspaceContext {
        WorkspaceContext {
            cwd: PathBuf::from("/workspace"),
            workspace_root: PathBuf::from("/workspace"),
            cache_root: PathBuf::from("/workspace/.unica/cache"),
            workspace_epoch: 1,
        }
    }

    fn passed_validation() -> MetaValidationData {
        MetaValidationData {
            status: MetaValidationStatus::Passed,
            diagnostics: Vec::new(),
        }
    }

    fn empty_collections() -> MetaCollectionsData {
        MetaCollectionsData {
            attributes: Vec::new(),
            tabular_sections: Vec::new(),
            dimensions: Vec::new(),
            resources: Vec::new(),
            recalculations: None,
            accounting_flags: None,
            ext_dimension_accounting_flags: None,
            addressing_attributes: None,
            enum_values: Vec::new(),
            columns: Vec::new(),
            forms: Vec::new(),
            templates: Vec::new(),
            commands: Vec::new(),
        }
    }

    fn empty_relations() -> unica_metadata::MetaRelationsData {
        unica_metadata::MetaRelationsData::default()
    }

    fn validation_subject() -> unica_metadata::validation_subject::MetadataValidationSubject {
        unica_metadata::validation_subject::MetadataValidationSubject {
            target: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Document.Order")
                .unwrap(),
            resources: vec![unica_metadata::validation_subject::MetadataResourceImage {
                role: unica_metadata::validation_subject::MetadataResourceRole::Descriptor,
                bytes: b"<MetaDataObject/>".to_vec(),
            }],
            child_footprints: Vec::new(),
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        }
    }

    fn mutation_data(changed: bool) -> MetaMutationData {
        MetaMutationData {
            metadata_path: MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "Document.Order",
            )
            .unwrap(),
            changed,
            publication_plan: Vec::new(),
            changed_paths: Vec::new(),
            effects: Vec::new(),
            validation: passed_validation(),
            diagnostics: Vec::new(),
        }
    }

    fn fake_ports(
        validation: MetaValidationData,
        preview_changed: bool,
        publication: Result<MetaPublishReport, unica_metadata::request::MetaFailure>,
    ) -> (FakeMetadataPorts, Arc<Mutex<CoordinatorState>>) {
        let state = Arc::new(Mutex::new(CoordinatorState::default()));
        let plan = FakePreparedMutation {
            state: Arc::clone(&state),
            preview: mutation_data(preview_changed),
            validation_subject: validation_subject(),
            publication,
        };
        (
            FakeMetadataPorts {
                state: Arc::clone(&state),
                cache_events: Arc::new(Mutex::new(Vec::new())),
                read: Mutex::new(None),
                related: unica_metadata::validation_subject::MetaEnrichment::default(),
                validation,
                prepared: Mutex::new(Some(Ok(Box::new(plan)))),
                cancel_after_validation: None,
            },
            state,
        )
    }

    fn info_ports() -> (FakeMetadataPorts, Arc<Mutex<CoordinatorState>>) {
        let state = Arc::new(Mutex::new(CoordinatorState::default()));
        let subject = validation_subject();
        (
            FakeMetadataPorts {
                state: Arc::clone(&state),
                cache_events: Arc::new(Mutex::new(Vec::new())),
                read: Mutex::new(Some(Ok(unica_metadata::validation_subject::MetadataRead {
                    local: unica_metadata::validation_subject::MetaLocalInfo {
                        member_read_profile: None,
                        metadata_path: subject.target.clone(),
                        kind: MetadataKind::Document,
                        details: MetaInfoDetails::empty(MetadataKind::Document),
                        name: "Order".to_string(),
                        synonym: Some("Order".to_string()),
                        support: MetaSupportStatus::Supported,
                        properties: Vec::new(),
                        declarations: unica_metadata::MetaInfoDeclarations::default(),
                        predefined_code_type: None,
                        relations: empty_relations(),
                        collections: empty_collections(),
                        diagnostics: Vec::new(),
                    },
                    validation_subject: subject,
                }))),
                related: unica_metadata::validation_subject::MetaEnrichment::default(),
                validation: passed_validation(),
                prepared: Mutex::new(None),
                cancel_after_validation: None,
            },
            state,
        )
    }

    fn add_args(dry_run: bool) -> Map<String, Value> {
        object(json!({
            "sourceSet": "main",
            "kind": "Document",
            "name": "Order",
            "dryRun": dry_run
        }))
    }

    #[test]
    fn preview_effects_follow_operation_order() {
        let workspace = std::env::temp_dir().join(format!(
            "unica-meta-effect-order-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        // Файловая фикстура пустой конфигурации 8.3.27 вместо снятого cf.init.
        fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy_tree(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/platform_8_3_27/empty_configuration"),
            &workspace,
        );
        let cwd = unica_workspace::test_support::ProcessCwdGuard::enter(&workspace).unwrap();
        let application = crate::application::UnicaApplication::new();
        for name in ["Owner", "Subject"] {
            let added = application
                .call_tool(
                    "unica.meta.add",
                    &object(json!({
                        "sourceSet": "main",
                        "kind": "Catalog",
                        "name": name,
                        "dryRun": false
                    })),
                )
                .unwrap();
            assert!(added.ok, "{name}: {:?}", added.errors);
        }
        let operations = json!([
            {"op": "setProperties", "values": {"Comment": "ordered"}},
            {"op": "add", "collection": "attributes", "elements": [{"name": "Transient"}]},
            {"op": "update", "collection": "attributes", "elements": [{"name": "Transient", "newName": "Updated"}]},
            {"op": "remove", "collection": "attributes", "names": ["Updated"]},
            {
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "SearchKey",
                    "type": {"variants": [{
                        "kind": "string",
                        "length": 20,
                        "allowedLength": "variable"
                    }]},
                    "fillValue": {"kind": "string", "value": "ready"}
                }]
            },
            {"op": "editRelations", "relation": "owners", "mode": "replace", "targets": [{"metadataPath": "Catalog.Owner"}]},
            {"op": "editRelations", "relation": "inputByString", "mode": "replace", "targets": [{"fieldPath": "Catalog.Subject.Attribute.SearchKey"}]}
        ]);
        let preview_call = json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Subject",
            "operations": operations,
            "dryRun": true
        });
        let missing_final_field_call = json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Subject",
            "operations": [{
                "op": "editRelations",
                "relation": "inputByString",
                "mode": "replace",
                "targets": [{
                    "fieldPath": "Catalog.Subject.Attribute.Missing"
                }]
            }],
            "dryRun": true
        });
        let edit_schema = unica_metadata::request::metadata_input_schema(
            unica_metadata::request::MetadataOperation::Edit,
        );
        let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();
        assert!(edit_validator.is_valid(&preview_call));
        assert!(edit_validator.is_valid(&missing_final_field_call));
        let preview = application
            .call_tool("unica.meta.edit", &object(preview_call))
            .unwrap();
        let missing_final_field = application
            .call_tool("unica.meta.edit", &object(missing_final_field_call))
            .unwrap();
        let template = application
            .call_tool(
                "unica.meta.add",
                &object(json!({
                    "sourceSet": "main",
                    "kind": "Catalog",
                    "name": "TemplateOnly",
                    "dryRun": true
                })),
            )
            .unwrap();
        drop(cwd);
        let _ = std::fs::remove_dir_all(&workspace);

        assert!(preview.ok, "{:?}", preview.errors);
        assert!(!missing_final_field.ok);
        assert!(missing_final_field
            .errors
            .iter()
            .any(|error| error.contains("does not exist in the owner post-image")));
        assert_eq!(
            missing_final_field.diagnostics.unwrap()[0]["field"],
            "operations[0].targets[0]"
        );
        let effects = preview.data.unwrap()["effects"].clone();
        assert_eq!(
            effects
                .as_array()
                .unwrap()
                .iter()
                .map(|effect| effect["operation"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "setProperties",
                "add",
                "update",
                "remove",
                "add",
                "editRelations",
                "editRelations",
            ]
        );
        assert!(effects
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .all(|(index, effect)| effect["operationIndex"] == index as u64));
        assert_eq!(
            template.data.unwrap()["effects"][0]["operation"],
            "createTemplate"
        );
    }

    #[test]
    fn coordinator_info_reads_validates_then_enriches_and_keeps_local_data() {
        let state = Arc::new(Mutex::new(CoordinatorState::default()));
        let subject = validation_subject();
        let ports = FakeMetadataPorts {
            state: Arc::clone(&state),
            cache_events: Arc::new(Mutex::new(Vec::new())),
            read: Mutex::new(Some(Ok(unica_metadata::validation_subject::MetadataRead {
                local: unica_metadata::validation_subject::MetaLocalInfo {
                    member_read_profile: None,
                    metadata_path: subject.target.clone(),
                    kind: MetadataKind::Document,
                    details: MetaInfoDetails::empty(MetadataKind::Document),
                    name: "Order".to_string(),
                    synonym: Some("Order".to_string()),
                    support: MetaSupportStatus::Supported,
                    properties: Vec::new(),
                    declarations: unica_metadata::MetaInfoDeclarations::default(),
                    predefined_code_type: None,
                    relations: empty_relations(),
                    collections: empty_collections(),
                    diagnostics: Vec::new(),
                },
                validation_subject: subject,
            }))),
            related: unica_metadata::validation_subject::MetaEnrichment::default(),
            validation: passed_validation(),
            prepared: Mutex::new(None),
            cancel_after_validation: None,
        };

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Info,
            &ports,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "sections": ["roles", "subscriptions", "functionalOptions"],
                "limit": 50,
            })),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.data.as_ref().unwrap()["name"], "Order");
        assert!(outcome.data.as_ref().unwrap().get("related").is_none());
        assert_eq!(state.lock().unwrap().calls, ["read", "validate", "related"]);
    }

    #[test]
    fn coordinator_info_with_no_selected_sections_skips_enrichment() {
        for sections in [None, Some(json!([]))] {
            let (ports, state) = info_ports();
            let mut args = object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
            }));
            if let Some(sections) = sections {
                args.insert("sections".to_string(), sections);
            }

            let outcome = invoke(
                unica_metadata::request::MetadataOperation::Info,
                &ports,
                &args,
                &coordinator_context(),
                &CancellationToken::new(),
            )
            .unwrap();

            assert!(outcome.adapter.ok);
            assert!(outcome.data.as_ref().unwrap().get("related").is_none());
            assert_eq!(state.lock().unwrap().calls, ["read", "validate"]);
        }
    }

    /// ADR-0028: the enrichment sections stand on their own evidence from the
    /// source tree, so a failed descriptor validation must not withhold them.
    /// The call still reports the failure — it just does not also drop data
    /// that the failure says nothing about.
    #[test]
    fn coordinator_info_validation_failure_still_enriches() {
        let (mut ports, state) = info_ports();
        ports.validation = MetaValidationData {
            status: MetaValidationStatus::Failed,
            diagnostics: vec![MetaDiagnostic::error(
                MetaDiagnosticCode::ValidationFailed,
                "local metadata validation failed",
            )],
        };
        ports.related = unica_metadata::validation_subject::MetaEnrichment {
            predefined_items: Some(unica_metadata::MetaPredefinedItemsData {
                total: 1,
                returned: 1,
                truncated: false,
                items: vec![unica_metadata::MetaPredefinedItemData {
                    id: "00000000-0000-0000-0000-000000000001".to_string(),
                    parent_id: None,
                    name: "Kept".to_string(),
                    code: String::new(),
                    description: String::new(),
                    is_folder: None,
                    r#type: None,
                    account_type: None,
                    off_balance: None,
                    order: None,
                    accounting_flags: None,
                    ext_dimension_types: None,
                    action_period_is_base: None,
                }],
            }),
            ..Default::default()
        };

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Info,
            &ports,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "sections": ["roles"],
            })),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert_eq!(
            outcome.data.as_ref().unwrap()["predefinedItems"]["items"][0]["name"],
            json!("Kept")
        );
        assert_eq!(state.lock().unwrap().calls, ["read", "validate", "related"]);
    }

    #[test]
    fn coordinator_info_enrichment_error_changes_validation_status_to_failed() {
        let (mut ports, state) = info_ports();
        ports.related.diagnostics.push(
            MetaDiagnostic::error(
                MetaDiagnosticCode::ValidationFailed,
                "predefined data has the wrong owner type",
            )
            .with_field("predefinedItems"),
        );
        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Info,
            &ports,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "sections": ["roles"],
            })),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert_eq!(
            outcome.data.as_ref().unwrap()["validation"]["status"],
            "failed"
        );
        assert_eq!(state.lock().unwrap().calls, ["read", "validate", "related"]);
    }

    #[test]
    fn coordinator_validation_failure_blocks_publication_and_returns_typed_diagnostics() {
        let diagnostic = MetaDiagnostic::error(
            MetaDiagnosticCode::ValidationFailed,
            "post-image validation failed",
        );
        let failed_validation = MetaValidationData {
            status: MetaValidationStatus::Failed,
            diagnostics: vec![diagnostic.clone()],
        };
        let (ports, state) = fake_ports(
            failed_validation,
            true,
            Ok(MetaPublishReport::source_only(mutation_data(true))),
        );

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.diagnostics, Some(json!([diagnostic])));
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate"]);
        assert_eq!(state.publish_calls, 0);
    }

    #[test]
    fn coordinator_preview_never_publishes_and_emits_only_a_projected_metadata_event() {
        let (ports, state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport::source_only(mutation_data(true))),
        );

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Add,
            &ports,
            &add_args(true),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert!(outcome.events.is_empty());
        assert_eq!(outcome.projected_events.len(), 1);
        assert_eq!(
            outcome.projected_events[0].kind,
            DomainEventKind::MetadataChanged
        );
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate"]);
        assert_eq!(state.publish_calls, 0);
    }

    #[test]
    fn coordinator_apply_publishes_once_and_emits_only_an_actual_metadata_event() {
        let mut publication = MetaPublishReport::source_only(mutation_data(true));
        publication.warnings =
            vec!["publication_cleanup_incomplete: metadata mutation was committed".to_string()];
        let (ports, state) = fake_ports(passed_validation(), true, Ok(publication));

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.events[0].kind, DomainEventKind::MetadataChanged);
        assert!(outcome.projected_events.is_empty());
        assert_eq!(outcome.adapter.warnings.len(), 1);
        assert!(outcome.adapter.warnings[0].contains("publication_cleanup_incomplete"));
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate", "publish"]);
        assert_eq!(state.publish_calls, 1);
    }

    #[test]
    fn coordinator_internal_dispatch_preserves_the_actual_event_for_cache_publication() {
        let (ports, _state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport::source_only(mutation_data(true))),
        );
        let cache_events = Arc::clone(&ports.cache_events);
        let spec = ToolSpec {
            name: "unica.meta.add",
            description: "internal metadata coordinator test",
            execution: crate::application::ToolExecution::Mutation,
            result_contract: crate::application::ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: crate::application::ToolHandler::Metadata {
                operation: unica_metadata::request::MetadataOperation::Add,
            },
        };

        let result = crate::application::call_tool(
            spec,
            &add_args(false),
            &ports,
            &CancellationToken::new(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
        )
        .unwrap();

        assert!(result.ok);
        assert_eq!(result.stdout, None);
        assert_eq!(result.data.as_ref().unwrap()["changed"], true);
        assert_eq!(result.cache.events, ["MetadataChanged"]);
        assert_eq!(
            cache_events.lock().unwrap()[0].kind,
            DomainEventKind::MetadataChanged
        );
    }

    #[test]
    fn coordinator_cancellation_after_validation_never_calls_publication() {
        let cancellation = CancellationToken::new();
        let (mut ports, state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport::source_only(mutation_data(true))),
        );
        ports.cancel_after_validation = Some(cancellation.clone());

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &cancellation,
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate"]);
        assert_eq!(state.publish_calls, 0);
    }

    #[test]
    fn coordinator_apply_noop_uses_the_publication_report_and_emits_no_event() {
        let (ports, state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport::source_only(mutation_data(false))),
        );

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.data.as_ref().unwrap()["changed"], false);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        assert_eq!(state.lock().unwrap().publish_calls, 1);
    }

    #[test]
    fn coordinator_rollback_failure_stays_a_typed_error_without_events() {
        let diagnostic = MetaDiagnostic::error(
            MetaDiagnosticCode::RollbackFailed,
            "publication failed and rollback did not restore the preimage",
        );
        let (ports, state) = fake_ports(
            passed_validation(),
            true,
            Err(unica_metadata::request::MetaFailure {
                diagnostics: vec![diagnostic.clone()],
            }),
        );

        let outcome = invoke(
            unica_metadata::request::MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.diagnostics, Some(json!([diagnostic])));
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        assert_eq!(state.lock().unwrap().publish_calls, 1);
    }
}
