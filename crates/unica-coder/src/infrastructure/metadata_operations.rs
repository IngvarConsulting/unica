use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::ports::{
    MetaEnrichment, MetaLocalInfo, MetaRelatedData, MetadataRead, MetadataValidationResult,
    MetadataValidationSubject, PreparedMetadataMutation,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::{MetaDiagnostic, MetaDiagnosticCode};
use crate::domain::support_state::SupportStateReader;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::meta::{
    prepare_meta_add, prepare_meta_remove, prepare_typed_edit, read_typed_meta_info,
    resolve_typed_edit_object, resolve_typed_metadata_object, scan_local_enrichment,
    LocalEnrichment, LocalSection, MetadataValidator,
};
use crate::infrastructure::source_roots::NamedSourceSetErrorKind;

pub(crate) struct MetadataOperations;

impl MetadataOperations {
    pub(crate) fn read_local(
        request: &MetaInfoRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
        support_reader: &dyn SupportStateReader,
    ) -> Result<MetadataRead, MetaFailure> {
        let resolved = resolve_typed_metadata_object(
            &request.source_set,
            &request.metadata_path,
            "info",
            context,
            cancellation,
        )?;
        let (local, validation_subject) = read_typed_meta_info(
            &resolved,
            &request.source_set,
            &request.metadata_path,
            context,
            ProviderDeadline::new(std::time::Instant::now() + std::time::Duration::from_secs(5)),
            cancellation,
            support_reader,
        )?;
        Ok(MetadataRead {
            local,
            validation_subject,
        })
    }

    /// Enrich a metadata read from the two sources that can answer.
    ///
    /// Roles, subscriptions, functional options and predefined items are read
    /// from the source tree, so they are exact and cannot disagree with the
    /// descriptor beside them. Only "which modules mention this object" needs a
    /// code index, and only it can therefore be stale or unavailable — which is
    /// why it is the one section that still reports index metadata.
    pub(crate) fn read_related(
        request: &MetaInfoRequest,
        local: &MetaLocalInfo,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> MetaRelatedData {
        use crate::application::metadata::MetaInfoSection;

        let source = match crate::infrastructure::source_roots::resolve_named_source_set(
            context,
            &request.source_set,
        ) {
            Ok(source) => source,
            Err(error) => {
                return selected_unavailable_sections(
                    request,
                    &logical_source_set_error(&request.source_set, error.kind),
                )
            }
        };

        let local_sections = request
            .sections
            .iter()
            .map(|section| match section {
                MetaInfoSection::Roles => LocalSection::Roles,
                MetaInfoSection::Subscriptions => LocalSection::Subscriptions,
                MetaInfoSection::FunctionalOptions => LocalSection::FunctionalOptions,
                MetaInfoSection::PredefinedItems => LocalSection::PredefinedItems,
            })
            .collect::<Vec<_>>();
        let LocalEnrichment {
            usage,
            predefined_items,
            diagnostics,
        } = if local_sections.is_empty() {
            LocalEnrichment::default()
        } else {
            scan_local_enrichment(
                &source.path,
                local.kind,
                &local.metadata_path,
                local.predefined_code_type.as_deref(),
                &local_sections,
                request.limit,
                cancellation,
            )
        };

        MetaEnrichment {
            predefined_items,
            usage,
            diagnostics,
        }
    }

    pub(crate) fn validate(
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        MetadataValidator.validate(subject, context)
    }

    pub(crate) fn validate_read(
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        if cancellation.is_cancelled() {
            return MetadataValidationResult {
                status: crate::domain::metadata::MetaValidationStatus::Failed,
                diagnostics: vec![MetaDiagnostic::error(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata read validation was cancelled",
                )
                .with_metadata_path(subject.target.clone())],
            };
        }
        MetadataValidator.validate_complete_read(subject, context)
    }

    pub(crate) fn prepare_mutation(
        request: &MetadataRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
        match request {
            MetadataRequest::Add(request) => prepare_meta_add(request, context, cancellation),
            MetadataRequest::Edit(request) => {
                if request.operations.is_empty() {
                    return Err(MetaDiagnostic::error(
                        MetaDiagnosticCode::InvalidArguments,
                        "metadata edit operations must not be empty",
                    )
                    .with_metadata_path(request.metadata_path.clone())
                    .with_field("operations")
                    .into());
                }
                let resolved = resolve_typed_edit_object(request, context, cancellation)?;
                prepare_typed_edit(request, resolved, context)
            }
            MetadataRequest::Remove(request) => prepare_meta_remove(request, context, cancellation),
            MetadataRequest::Info(_) => Err(capability_unavailable(
                "typed metadata mutation provider is not available yet",
            )),
        }
    }
}

fn logical_source_set_error(source_set: &str, kind: NamedSourceSetErrorKind) -> String {
    match kind {
        NamedSourceSetErrorKind::NotFound => {
            format!("source set `{source_set}` was not found")
        }
        NamedSourceSetErrorKind::Ambiguous => {
            format!("source set `{source_set}` is ambiguous")
        }
        NamedSourceSetErrorKind::Containment => {
            format!("source set `{source_set}` violates the workspace containment boundary")
        }
        NamedSourceSetErrorKind::Discovery => {
            format!("source set `{source_set}` could not be discovered")
        }
    }
}

/// The answer when the source set itself cannot be resolved.
///
/// Nothing can be read, and an empty usage list would claim the object is used
/// nowhere. The sections stay absent instead, and the failure is reported by
/// the read that owns it.
fn selected_unavailable_sections(_request: &MetaInfoRequest, _message: &str) -> MetaEnrichment {
    MetaEnrichment::default()
}

fn capability_unavailable(message: &str) -> MetaFailure {
    MetaDiagnostic::error(MetaDiagnosticCode::CapabilityUnavailable, message).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::metadata::{
        MetaAddRequest, MetaEditRequest, MetaInfoRequest, MetaInfoSection, MetaRemoveRequest,
    };
    use crate::application::ports::{
        ApplicationPorts, FormatGuardCheck, FormatGuardError, HandlerOutcome,
        MetadataChildDirectoryKind, MetadataChildProfile, MetadataChildResourceKind,
        MetadataResourceRole, MetadataTemplateResourcePart, MetadataTemplateType,
        SupportGuardCheck,
    };
    use crate::application::{InvocationMode, ToolSpec, UnicaApplication};
    use crate::domain::cache::{CacheAccess, CacheReport};
    use crate::domain::events::DomainEvent;
    use crate::domain::metadata::{
        MetaCollection, MetaEditOperation, MetaElementInput, MetaElementUpdateInput,
        MetaEventSource, MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue,
        MetaPublicationAction, MetaPublicationResource, MetaRelation, MetaRelationTarget,
        MetaScope, MetaValidationStatus, MetadataKind, MetadataReference, RelationEditMode,
    };
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::application_ports::InfrastructureApplicationPorts;
    use crate::infrastructure::native_operations::cf::create_configuration_scaffold;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_before_rollback_mutation_hook, with_commit_failpoint, CommitFailpoint,
    };
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use crate::infrastructure::platform::filesystem::{
        create_dir_symlink_for_test, remove_dir_symlink_for_test,
    };
    use serde_json::{json, Map};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Designer keeps a managed form's module one level below the rest of
    /// `Ext`, so a fixture has to create that directory before writing it.
    fn write_form_module(payload_root: &std::path::Path, bytes: &[u8]) -> PathBuf {
        let path = payload_root.join("Ext/Form/Module.bsl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    struct FixedWorkspaceApplicationPorts {
        context: WorkspaceContext,
        inner: InfrastructureApplicationPorts,
    }

    impl ApplicationPorts for FixedWorkspaceApplicationPorts {
        fn discover_workspace(
            &self,
            _requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            Ok(self.context.clone())
        }

        fn validate_tool_context(
            &self,
            spec: ToolSpec,
            args: &Map<String, serde_json::Value>,
            mode: InvocationMode,
            context: &WorkspaceContext,
        ) -> Result<(), String> {
            self.inner.validate_tool_context(spec, args, mode, context)
        }

        fn prepare_metadata_mutation(
            &self,
            request: &MetadataRequest,
            context: &WorkspaceContext,
            cancellation: &CancellationToken,
        ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
            self.inner
                .prepare_metadata_mutation(request, context, cancellation)
        }

        fn validate_metadata(
            &self,
            subject: &MetadataValidationSubject,
            context: &WorkspaceContext,
            cancellation: &CancellationToken,
        ) -> MetadataValidationResult {
            self.inner.validate_metadata(subject, context, cancellation)
        }

        fn evaluate_format_guard(
            &self,
            spec: ToolSpec,
            args: &Map<String, serde_json::Value>,
            context: &WorkspaceContext,
        ) -> Result<FormatGuardCheck, FormatGuardError> {
            self.inner.evaluate_format_guard(spec, args, context)
        }

        fn evaluate_support_guard(
            &self,
            spec: ToolSpec,
            args: &Map<String, serde_json::Value>,
            context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            self.inner.evaluate_support_guard(spec, args, context)
        }

        fn invoke_handler(
            &self,
            spec: ToolSpec,
            args: &Map<String, serde_json::Value>,
            context: &WorkspaceContext,
            mode: InvocationMode,
            cancellation: &CancellationToken,
        ) -> Result<HandlerOutcome, String> {
            self.inner
                .invoke_handler(spec, args, context, mode, cancellation)
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            events: &[DomainEvent],
            mode: InvocationMode,
            cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            self.inner.cache_report(context, events, mode, cache_access)
        }

        fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]) {
            self.inner.notify_invalidation(context, events);
        }
    }

    fn empty_workspace(label: &str) -> (PathBuf, WorkspaceContext) {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-typed-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 0,
        };
        let args = Map::from_iter([
            ("Name".to_string(), json!("MetaTyped")),
            ("OutputDir".to_string(), json!("src")),
        ]);
        let outcome = create_configuration_scaffold(&args, &context);
        assert!(outcome.ok, "{:?}", outcome.errors);
        fs::write(
            root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .unwrap();
        (root, context)
    }

    fn add_exported_event_handler(root: &std::path::Path, context: &WorkspaceContext) {
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::CommonModule,
                name: "EventHandlers".into(),
                operations: Vec::new(),
                dry_run: false,
            }),
            context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        fs::write(
            root.join("src/CommonModules/EventHandlers/Ext/Module.bsl"),
            "Procedure OnEvent(Source, Cancel) Export\nEndProcedure\n",
        )
        .unwrap();
    }

    fn source_replace(sources: Vec<MetaEventSource>) -> MetaEditOperation {
        MetaEditOperation::edit_relation_targets(
            MetaRelation::Source,
            RelationEditMode::Replace,
            sources
                .into_iter()
                .map(MetaRelationTarget::EventSource)
                .collect(),
        )
        .unwrap()
    }

    fn source_family(source_class: crate::domain::metadata::EventSourceClass) -> MetaEventSource {
        MetaEventSource::Family { source_class }
    }

    fn prepared_event_subscription_source_change(
        label: &str,
    ) -> (PathBuf, Box<dyn PreparedMetadataMutation>, PathBuf, Vec<u8>) {
        let (root, context) = empty_workspace(label);
        add_exported_event_handler(&root, &context);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "Events".into(),
                operations: vec![source_replace(vec![source_family(
                    crate::domain::metadata::EventSourceClass::CatalogObject,
                )])],
                dry_run: false,
            }),
            &context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let descriptor = root.join("src/EventSubscriptions/Events.xml");
        let preimage = fs::read(&descriptor).unwrap();
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let prepared = MetadataOperations::prepare_mutation(
            &MetadataRequest::Edit(MetaEditRequest {
                source_set: "main".into(),
                metadata_path: target,
                operations: vec![source_replace(vec![source_family(
                    crate::domain::metadata::EventSourceClass::DocumentObject,
                )])],
                dry_run: false,
            }),
            &context,
            &cancellation,
        )
        .unwrap();
        (root, prepared, descriptor, preimage)
    }

    struct Fixture {
        root: PathBuf,
        context: WorkspaceContext,
        target: MetadataAddress,
        descriptor: PathBuf,
        owner: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "unica-meta-edit-typed-{label}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&root).unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 0,
            };
            let args = Map::from_iter([
                ("Name".to_string(), json!("MetaEditTyped")),
                ("OutputDir".to_string(), json!("src")),
            ]);
            let outcome = create_configuration_scaffold(&args, &context);
            assert!(outcome.ok, "{:?}", outcome.errors);
            fs::write(
                root.join("v8project.yaml"),
                concat!(
                    "format: DESIGNER\n",
                    "source-set:\n",
                    "  - name: main\n",
                    "    type: CONFIGURATION\n",
                    "    path: src\n",
                ),
            )
            .unwrap();
            let cancellation = CancellationToken::new();
            MetadataOperations::prepare_mutation(
                &MetadataRequest::Add(MetaAddRequest {
                    source_set: "main".into(),
                    kind: MetadataKind::Catalog,
                    name: "Editable".into(),
                    operations: Vec::new(),
                    dry_run: false,
                }),
                &context,
                &cancellation,
            )
            .unwrap()
            .publish(&cancellation)
            .unwrap();
            Self {
                descriptor: root.join("src/Catalogs/Editable.xml"),
                owner: root.join("src/Configuration.xml"),
                target: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Editable")
                    .unwrap(),
                root,
                context,
            }
        }

        fn edit(&self, property: &str, value: MetaPropertyValue) -> MetadataRequest {
            MetadataRequest::Edit(MetaEditRequest {
                source_set: "main".into(),
                metadata_path: self.target.clone(),
                operations: vec![MetaEditOperation::SetProperties {
                    values: MetaPropertyChanges::convert(
                        MetadataKind::Catalog,
                        vec![MetaPropertyInput::new(property, value)],
                    )
                    .unwrap(),
                }],
                dry_run: false,
            })
        }

        fn add_object(&self, kind: MetadataKind, name: &str) -> MetadataAddress {
            self.add_object_with_operations(kind, name, Vec::new())
        }

        fn add_object_with_operations(
            &self,
            kind: MetadataKind,
            name: &str,
            operations: Vec<MetaEditOperation>,
        ) -> MetadataAddress {
            let cancellation = CancellationToken::new();
            MetadataOperations::prepare_mutation(
                &MetadataRequest::Add(MetaAddRequest {
                    source_set: "main".into(),
                    kind,
                    name: name.into(),
                    operations,
                    dry_run: false,
                }),
                &self.context,
                &cancellation,
            )
            .unwrap()
            .publish(&cancellation)
            .unwrap();
            MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("{}.{}", kind.as_str(), name),
            )
            .unwrap()
        }

        fn owners_request(
            &self,
            mode: RelationEditMode,
            targets: Vec<MetadataAddress>,
        ) -> MetadataRequest {
            MetadataRequest::Edit(MetaEditRequest {
                source_set: "main".into(),
                metadata_path: self.target.clone(),
                operations: vec![MetaEditOperation::edit_relations(
                    MetaRelation::Owners,
                    mode,
                    targets
                        .into_iter()
                        .map(|metadata_path| MetadataReference { metadata_path })
                        .collect(),
                )
                .unwrap()],
                dry_run: false,
            })
        }

        fn typed_edit(&self, operations: Vec<MetaEditOperation>) -> MetadataRequest {
            MetadataRequest::Edit(MetaEditRequest {
                source_set: "main".into(),
                metadata_path: self.target.clone(),
                operations,
                dry_run: false,
            })
        }

        fn remove_request(
            &self,
            target: MetadataAddress,
            dry_run: bool,
            force: bool,
            confirm: bool,
        ) -> MetadataRequest {
            MetadataRequest::Remove(MetaRemoveRequest {
                source_set: "main".into(),
                metadata_path: target,
                dry_run,
                force,
                confirm,
            })
        }

        fn publish_form_add(&self, name: &str) {
            let cancellation = CancellationToken::new();
            MetadataOperations::prepare_mutation(
                &self.typed_edit(vec![MetaEditOperation::add(
                    MetaCollection::Forms,
                    None,
                    vec![MetaElementInput::named(name)],
                )
                .unwrap()]),
                &self.context,
                &cancellation,
            )
            .unwrap()
            .publish(&cancellation)
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn related_source_resolution_failure_is_logical_and_path_independent() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-related-source-error-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  unsafe:\n    type: CONFIGURATION\n    path: ../outside\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 0,
        };
        let request = MetaInfoRequest {
            source_set: "unsafe".into(),
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
            sections: vec![MetaInfoSection::Roles],
            limit: 20,
        };

        let related = MetadataOperations::read_related(
            &request,
            &crate::application::ports::MetaLocalInfo {
                metadata_path: request.metadata_path.clone(),
                kind: MetadataKind::Catalog,
                details: crate::domain::metadata::MetaInfoDetails::empty(MetadataKind::Catalog),
                name: "Items".into(),
                synonym: None,
                support: crate::domain::metadata::MetaSupportStatus::Supported,
                properties: Vec::new(),
                declarations: crate::domain::metadata::MetaInfoDeclarations::default(),
                predefined_code_type: Some("String".to_string()),
                relations: crate::domain::metadata::MetaRelationsData {
                    owners: Vec::new(),
                    register_records: Vec::new(),
                    based_on: Vec::new(),
                    input_by_string: Vec::new(),
                    data_lock_fields: None,
                    source: Vec::new(),
                },
                collections: crate::domain::metadata::MetaCollectionsData {
                    attributes: Vec::new(),
                    columns: Vec::new(),
                    tabular_sections: Vec::new(),
                    dimensions: Vec::new(),
                    resources: Vec::new(),
                    recalculations: None,
                    accounting_flags: None,
                    ext_dimension_accounting_flags: None,
                    addressing_attributes: None,
                    enum_values: Vec::new(),
                    forms: Vec::new(),
                    templates: Vec::new(),
                    commands: Vec::new(),
                },
                diagnostics: Vec::new(),
            },
            &context,
            &CancellationToken::new(),
        );
        // Enrichment reads a source set the local read already resolved, so an
        // unresolvable one leaves every section absent rather than empty: an
        // empty list would claim the object is used nowhere. Nothing is
        // rendered from the failure, so nothing can leak the workspace root.
        assert!(related.usage.roles.is_none());
        assert!(related.usage.subscriptions.is_none());
        assert!(related.usage.functional_options.is_none());
        assert!(related.predefined_items.is_none());
        let rendered = serde_json::to_string(&related.usage).unwrap();
        assert!(
            !rendered.contains(root.to_string_lossy().as_ref()),
            "enrichment leaked workspace root: {rendered}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn meta_add_event_subscription_source_replace_needs_no_catalog_and_round_trips() {
        let (root, context) = empty_workspace("event-source-add");
        add_exported_event_handler(&root, &context);
        let cancellation = CancellationToken::new();
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "EventSubscription.AllEvents",
        )
        .unwrap();
        let add = MetadataRequest::Add(MetaAddRequest {
            source_set: "main".into(),
            kind: MetadataKind::EventSubscription,
            name: "AllEvents".into(),
            operations: vec![source_replace(vec![source_family(
                crate::domain::metadata::EventSourceClass::CatalogObject,
            )])],
            dry_run: false,
        });

        let prepared = MetadataOperations::prepare_mutation(&add, &context, &cancellation).unwrap();
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .all(|resource| {
                !matches!(
                    &resource.role,
                    MetadataResourceRole::Dependency { target }
                        if target.as_str().starts_with("Catalog.")
                )
            }));
        let descriptor = prepared
            .validation_subject()
            .resources
            .iter()
            .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
            .unwrap();
        assert!(String::from_utf8_lossy(&descriptor.bytes)
            .contains("<v8:TypeSet>cfg:CatalogObject</v8:TypeSet>"));
        assert_eq!(
            prepared.preview().effects[1].after,
            Some(json!([{
                "kind": "family",
                "sourceClass": "catalogObject",
            }]))
        );
        prepared.publish(&cancellation).unwrap();

        let read = MetadataOperations::read_local(
            &MetaInfoRequest {
                source_set: "main".into(),
                metadata_path: target.clone(),
                sections: Vec::new(),
                limit: 20,
            },
            &context,
            &cancellation,
            &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(&context),
        )
        .unwrap();
        assert_eq!(
            read.local.relations.source,
            vec![source_family(
                crate::domain::metadata::EventSourceClass::CatalogObject,
            )]
        );

        let noop = MetadataOperations::prepare_mutation(
            &MetadataRequest::Edit(MetaEditRequest {
                source_set: "main".into(),
                metadata_path: target,
                operations: vec![source_replace(vec![source_family(
                    crate::domain::metadata::EventSourceClass::CatalogObject,
                )])],
                dry_run: false,
            }),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(!noop.preview().changed);
        assert!(noop.preview().publication_plan.is_empty());
        noop.publish(&cancellation).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn meta_add_event_subscription_uses_explicit_final_handler_without_template_arity_gate() {
        let (root, context) = empty_workspace("event-binding-add");
        add_exported_event_handler(&root, &context);
        fs::write(
            root.join("src/CommonModules/EventHandlers/Ext/Module.bsl"),
            "Procedure OnRecordSetEvent(Source, Cancel, Replacing) Export\nEndProcedure\n",
        )
        .unwrap();
        let binding = MetaEditOperation::SetProperties {
            values: MetaPropertyChanges::convert(
                MetadataKind::EventSubscription,
                vec![MetaPropertyInput::new(
                    "Handler",
                    MetaPropertyValue::String(
                        "CommonModule.EventHandlers.OnRecordSetEvent".to_string(),
                    ),
                )],
            )
            .unwrap(),
        };
        let cancellation = CancellationToken::new();
        let prepared = MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "RecordSetEvents".into(),
                operations: vec![
                    source_replace(vec![source_family(
                        crate::domain::metadata::EventSourceClass::InformationRegisterRecordSet,
                    )]),
                    binding,
                ],
                dry_run: false,
            }),
            &context,
            &cancellation,
        )
        .unwrap();
        let validation =
            MetadataOperations::validate(prepared.validation_subject(), &context, &cancellation);
        assert_eq!(
            validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );
        prepared.publish(&cancellation).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn add_object_event_subscription(fixture: &Fixture, name: &str) -> MetadataAddress {
        add_exported_event_handler(&fixture.root, &fixture.context);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: name.into(),
                operations: vec![source_replace(vec![MetaEventSource::Object {
                    metadata_path: fixture.target.clone(),
                }])],
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("EventSubscription.{name}"),
        )
        .unwrap()
    }

    #[test]
    fn event_subscription_requires_explicit_non_global_server_module_facts() {
        let fixture = Fixture::new("event-handler-context-facts");
        let subscription = add_object_event_subscription(&fixture, "CatalogEvents");
        let module_descriptor = fixture.root.join("src/CommonModules/EventHandlers.xml");
        let descriptor = fs::read_to_string(&module_descriptor).unwrap();
        let malformed = descriptor.replacen("\t\t\t<Global>false</Global>\n", "", 1);
        assert_ne!(malformed, descriptor, "fixture must remove Global evidence");
        fs::write(module_descriptor, malformed).unwrap();

        let cancellation = CancellationToken::new();
        let read = MetadataOperations::read_local(
            &MetaInfoRequest {
                source_set: "main".into(),
                metadata_path: subscription,
                sections: Vec::new(),
                limit: 20,
            },
            &fixture.context,
            &cancellation,
            &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                &fixture.context,
            ),
        )
        .unwrap();
        let validation = MetadataOperations::validate_read(
            &read.validation_subject,
            &fixture.context,
            &cancellation,
        );
        assert_eq!(validation.status, MetaValidationStatus::Failed);
        assert!(
            validation.diagnostics.iter().any(|diagnostic| {
                diagnostic.field.as_deref() == Some("properties.handler")
                    && diagnostic.message.contains("Global=false")
            }),
            "{:?}",
            validation.diagnostics
        );
    }

    fn replace_defined_type_members(root: &std::path::Path, name: &str, members: &[&str]) {
        let path = root.join(format!("src/DefinedTypes/{name}.xml"));
        let descriptor = fs::read_to_string(&path).unwrap();
        let body = members
            .iter()
            .map(|member| {
                let tag = if member.starts_with("DefinedType.") || !member.contains('.') {
                    "TypeSet"
                } else {
                    "Type"
                };
                format!("\t\t\t\t<v8:{tag}>cfg:{member}</v8:{tag}>\n")
            })
            .collect::<String>();
        let replacement = format!("\t\t\t<Type>\n{body}\t\t\t</Type>");
        let changed = descriptor.replacen("\t\t\t<Type/>", &replacement, 1);
        assert_ne!(changed, descriptor, "fixture must replace DefinedType Type");
        fs::write(path, changed).unwrap();
    }

    #[test]
    fn defined_type_event_source_expands_to_concrete_event_classes_and_dependencies() {
        let fixture = Fixture::new("defined-type-event-source");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let defined_type = fixture.add_object(MetadataKind::DefinedType, "CatalogObjects");
        replace_defined_type_members(
            &fixture.root,
            "CatalogObjects",
            &["CatalogObject.Editable", "CatalogObject"],
        );
        let cancellation = CancellationToken::new();
        let prepared = MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "DefinedEvents".into(),
                operations: vec![source_replace(vec![MetaEventSource::DefinedType {
                    metadata_path: defined_type.clone(),
                }])],
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap();

        for expected in [&defined_type, &fixture.target] {
            assert!(
                prepared
                    .validation_subject()
                    .resources
                    .iter()
                    .any(|resource| {
                        matches!(
                            &resource.role,
                            MetadataResourceRole::Dependency { target } if target == expected
                        )
                    }),
                "missing dependency evidence for {expected}"
            );
        }
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );
        assert_eq!(
            validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );
        prepared.publish(&cancellation).unwrap();
    }

    #[test]
    fn meta_info_surfaces_a_strict_defined_type_member_parse_failure() {
        let fixture = Fixture::new("defined-type-event-source-invalid-member");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let defined_type = fixture.add_object(MetadataKind::DefinedType, "CatalogObjects");
        replace_defined_type_members(&fixture.root, "CatalogObjects", &["CatalogObject.Editable"]);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "DefinedEvents".into(),
                operations: vec![source_replace(vec![MetaEventSource::DefinedType {
                    metadata_path: defined_type,
                }])],
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let subscription = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "EventSubscription.DefinedEvents",
        )
        .unwrap();
        let defined_type_descriptor = fixture.root.join("src/DefinedTypes/CatalogObjects.xml");
        let descriptor = fs::read_to_string(&defined_type_descriptor).unwrap();
        let malformed = descriptor.replacen("cfg:CatalogObject.Editable", "cfg:String", 1);
        assert_ne!(
            malformed, descriptor,
            "fixture must corrupt DefinedType member"
        );
        fs::write(defined_type_descriptor, malformed).unwrap();

        let read = MetadataOperations::read_local(
            &MetaInfoRequest {
                source_set: "main".into(),
                metadata_path: subscription,
                sections: Vec::new(),
                limit: 20,
            },
            &fixture.context,
            &cancellation,
            &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                &fixture.context,
            ),
        )
        .unwrap();
        let validation = MetadataOperations::validate_read(
            &read.validation_subject,
            &fixture.context,
            &cancellation,
        );

        assert_eq!(validation.status, MetaValidationStatus::Failed);
        assert!(
            validation.diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .field
                    .as_deref()
                    .is_some_and(|field| field.starts_with("relations.source"))
                    && diagnostic
                        .message
                        .contains("configuration type cfg:String is malformed")
            }),
            "{:?}",
            validation.diagnostics
        );
    }

    #[test]
    fn defined_type_event_source_cycle_is_rejected_before_publication() {
        let fixture = Fixture::new("defined-type-event-cycle");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let first = fixture.add_object(MetadataKind::DefinedType, "First");
        fixture.add_object(MetadataKind::DefinedType, "Second");
        replace_defined_type_members(&fixture.root, "First", &["DefinedType.Second"]);
        replace_defined_type_members(&fixture.root, "Second", &["DefinedType.First"]);
        let cancellation = CancellationToken::new();
        let failure = match MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "CyclicEvents".into(),
                operations: vec![source_replace(vec![MetaEventSource::DefinedType {
                    metadata_path: first,
                }])],
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        ) {
            Ok(_) => panic!("cyclic DefinedType event source unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert!(
            failure.diagnostics.iter().any(|diagnostic| {
                diagnostic.field.as_deref() == Some("relations.source")
                    && diagnostic.message.contains("cycle")
            }),
            "{:?}",
            failure.diagnostics
        );
        assert!(!fixture
            .root
            .join("src/EventSubscriptions/CyclicEvents.xml")
            .exists());
    }

    fn assert_event_source_info_validation_fails(
        fixture: &Fixture,
        target: MetadataAddress,
        expected_source: MetadataAddress,
        expected_message: &str,
    ) {
        let cancellation = CancellationToken::new();
        let read = MetadataOperations::read_local(
            &MetaInfoRequest {
                source_set: "main".into(),
                metadata_path: target,
                sections: Vec::new(),
                limit: 20,
            },
            &fixture.context,
            &cancellation,
            &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                &fixture.context,
            ),
        )
        .unwrap();
        assert_eq!(
            read.local.relations.source,
            vec![MetaEventSource::Object {
                metadata_path: expected_source,
            }],
            "meta.info must retain typed partial data when dependency proof fails"
        );

        let validation = MetadataOperations::validate_read(
            &read.validation_subject,
            &fixture.context,
            &cancellation,
        );
        assert_eq!(
            validation.status,
            MetaValidationStatus::Failed,
            "{:?}",
            validation.diagnostics
        );
        assert!(validation.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .field
                .as_deref()
                .is_some_and(|field| field.starts_with("relations.source"))
                && diagnostic.message.contains(expected_message)
        }));
    }

    #[test]
    fn meta_info_rejects_missing_event_source_dependency_but_keeps_typed_readback() {
        let fixture = Fixture::new("event-source-info-missing-dependency");
        let subscription = add_object_event_subscription(&fixture, "CatalogEvents");
        let descriptor = fixture
            .root
            .join("src/EventSubscriptions/CatalogEvents.xml");
        let source = String::from_utf8(fs::read(&descriptor).unwrap()).unwrap();
        let missing = source.replacen("cfg:CatalogObject.Editable", "cfg:CatalogObject.Missing", 1);
        assert_ne!(missing, source, "fixture must rewrite the Source QName");
        fs::write(descriptor, missing).unwrap();
        let missing_target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Missing").unwrap();

        assert_event_source_info_validation_fails(
            &fixture,
            subscription,
            missing_target,
            "unavailable",
        );
    }

    #[test]
    fn meta_info_rejects_event_source_dependency_with_wrong_generated_type() {
        let fixture = Fixture::new("event-source-info-generated-type");
        let subscription = add_object_event_subscription(&fixture, "CatalogEvents");
        let source = String::from_utf8(fs::read(&fixture.descriptor).unwrap()).unwrap();
        let malformed = source.replacen(
            "name=\"CatalogObject.Editable\"",
            "name=\"CatalogObject.Shadow\"",
            1,
        );
        assert_ne!(malformed, source, "fixture must corrupt GeneratedType");
        fs::write(&fixture.descriptor, malformed).unwrap();

        assert_event_source_info_validation_fails(
            &fixture,
            subscription,
            fixture.target.clone(),
            "GeneratedType",
        );
    }

    #[test]
    fn meta_info_event_source_dependency_scan_honors_an_expired_deadline() {
        let fixture = Fixture::new("event-source-info-deadline");
        let subscription = add_object_event_subscription(&fixture, "CatalogEvents");
        let cancellation = CancellationToken::new();
        let resolved = resolve_typed_metadata_object(
            "main",
            &subscription,
            "info",
            &fixture.context,
            &cancellation,
        )
        .unwrap();

        let (local, _) = read_typed_meta_info(
            &resolved,
            "main",
            &subscription,
            &fixture.context,
            ProviderDeadline::new(std::time::Instant::now() - std::time::Duration::from_millis(1)),
            &cancellation,
            &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                &fixture.context,
            ),
        )
        .unwrap();

        assert!(local.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == MetaDiagnosticCode::ProviderUnavailable
                && diagnostic.field.as_deref() == Some("relations.source")
                && diagnostic.message.contains("deadline")
        }));
    }

    #[test]
    fn typed_event_source_generated_dependencies_are_aggregated_validated_and_guarded() {
        let fixture = Fixture::new("event-source-dependencies");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "CatalogEvents".into(),
                operations: Vec::new(),
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let subscription = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "EventSubscription.CatalogEvents",
        )
        .unwrap();
        let sources = vec![
            MetaEventSource::Object {
                metadata_path: fixture.target.clone(),
            },
            source_family(crate::domain::metadata::EventSourceClass::CatalogObject),
        ];
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: subscription.clone(),
            operations: vec![source_replace(sources.clone())],
            dry_run: false,
        });

        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let dependency_count = prepared
            .validation_subject()
            .resources
            .iter()
            .filter(|resource| {
                matches!(
                    &resource.role,
                    MetadataResourceRole::Dependency { target } if target == &fixture.target
                )
            })
            .count();
        assert_eq!(dependency_count, 1);
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| {
                matches!(
                    &resource.role,
                    MetadataResourceRole::Module { owner }
                        if owner.as_str() == "CommonModule.EventHandlers"
                )
            }));
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );
        assert_eq!(
            validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );
        prepared.publish(&cancellation).unwrap();
        let read = MetadataOperations::read_local(
            &MetaInfoRequest {
                source_set: "main".into(),
                metadata_path: subscription.clone(),
                sections: Vec::new(),
                limit: 20,
            },
            &fixture.context,
            &cancellation,
            &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                &fixture.context,
            ),
        )
        .unwrap();
        assert_eq!(read.local.relations.source, sources);
        let read_validation = MetadataOperations::validate_read(
            &read.validation_subject,
            &fixture.context,
            &cancellation,
        );
        assert_eq!(
            read_validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            read_validation.diagnostics
        );

        let subscription_before = fs::read(
            fixture
                .root
                .join("src/EventSubscriptions/CatalogEvents.xml"),
        )
        .unwrap();
        let noop = MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
            .unwrap();
        assert!(!noop.preview().changed);
        let mut external = fs::read(&fixture.descriptor).unwrap();
        external.extend_from_slice(b"\n");
        fs::write(&fixture.descriptor, &external).unwrap();
        let failure = match noop.publish(&cancellation) {
            Ok(_) => panic!("EventSubscription source dependency drift unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), external);
        assert_eq!(
            fs::read(
                fixture
                    .root
                    .join("src/EventSubscriptions/CatalogEvents.xml")
            )
            .unwrap(),
            subscription_before
        );
    }

    #[test]
    fn typed_event_source_rejects_collection_membership_drift_after_prepare() {
        let (root, prepared, descriptor, preimage) =
            prepared_event_subscription_source_change("event-source-membership-drift");
        let late = root.join("src/EventSubscriptions/Late.xml");
        let late_bytes = b"<concurrent/>";
        fs::write(&late, late_bytes).unwrap();

        let failure = match prepared.publish(&CancellationToken::new()) {
            Ok(_) => panic!("EventSubscription collection drift unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(descriptor).unwrap(), preimage);
        assert_eq!(fs::read(late).unwrap(), late_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn typed_event_source_rolls_back_for_collection_drift_during_commit() {
        let (root, prepared, descriptor, preimage) =
            prepared_event_subscription_source_change("event-source-membership-rollback");
        let late = root.join("src/EventSubscriptions/Late.xml");
        let late_for_hook = late.clone();
        let late_bytes = b"<concurrent/>";

        let result = with_before_commit_hook(
            move |_| fs::write(&late_for_hook, late_bytes).unwrap(),
            || prepared.publish(&CancellationToken::new()),
        );
        let failure = match result {
            Ok(_) => panic!("late EventSubscription collection drift unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(descriptor).unwrap(), preimage);
        assert_eq!(fs::read(late).unwrap(), late_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn typed_event_source_rejects_dependency_ancestor_symlink_after_prepare() {
        let fixture = Fixture::new("event-source-dependency-ancestor-symlink");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "CatalogEvents".into(),
                operations: Vec::new(),
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let subscription = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "EventSubscription.CatalogEvents",
        )
        .unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: subscription,
            operations: vec![source_replace(vec![MetaEventSource::Object {
                metadata_path: fixture.target.clone(),
            }])],
            dry_run: false,
        });
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let subscription_path = fixture
            .root
            .join("src/EventSubscriptions/CatalogEvents.xml");
        let subscription_before = fs::read(&subscription_path).unwrap();
        let dependency_before = fs::read(&fixture.descriptor).unwrap();
        let dependency_directory = fixture.root.join("src/Catalogs");
        let outside_directory = fixture.root.join("outside-catalogs");
        fs::rename(&dependency_directory, &outside_directory).unwrap();
        let Some(link_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &outside_directory,
                &dependency_directory,
            )
        else {
            fs::rename(&outside_directory, &dependency_directory).unwrap();
            return;
        };
        if link_result.is_err() {
            fs::rename(&outside_directory, &dependency_directory).unwrap();
            return;
        }

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("symlinked EventSubscription source dependency unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(subscription_path).unwrap(), subscription_before);
        assert_eq!(
            fs::read(outside_directory.join("Editable.xml")).unwrap(),
            dependency_before
        );
    }

    #[test]
    fn typed_event_source_rejects_handler_module_ancestor_symlink_after_prepare() {
        let fixture = Fixture::new("event-handler-module-ancestor-symlink");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "Events".into(),
                operations: Vec::new(),
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let subscription =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: subscription,
            operations: vec![source_replace(vec![source_family(
                crate::domain::metadata::EventSourceClass::CatalogObject,
            )])],
            dry_run: false,
        });
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let subscription_path = fixture.root.join("src/EventSubscriptions/Events.xml");
        let subscription_before = fs::read(&subscription_path).unwrap();
        let extension_directory = fixture.root.join("src/CommonModules/EventHandlers/Ext");
        let outside_directory = fixture.root.join("outside-handler-ext");
        fs::rename(&extension_directory, &outside_directory).unwrap();
        let Some(link_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &outside_directory,
                &extension_directory,
            )
        else {
            fs::rename(&outside_directory, &extension_directory).unwrap();
            return;
        };
        if link_result.is_err() {
            fs::rename(&outside_directory, &extension_directory).unwrap();
            return;
        }

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("symlinked EventSubscription handler module unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(subscription_path).unwrap(), subscription_before);
        assert_eq!(
            fs::read(outside_directory.join("Module.bsl")).unwrap(),
            b"Procedure OnEvent(Source, Cancel) Export\nEndProcedure\n"
        );
    }

    #[test]
    fn typed_event_source_rejects_generated_type_descriptor_mismatch() {
        let fixture = Fixture::new("event-source-generated-type-mismatch");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "CatalogEvents".into(),
                operations: Vec::new(),
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let mut catalog = String::from_utf8(fs::read(&fixture.descriptor).unwrap()).unwrap();
        assert_eq!(
            catalog.matches("name=\"CatalogObject.Editable\"").count(),
            1
        );
        catalog = catalog.replacen(
            "name=\"CatalogObject.Editable\"",
            "name=\"CatalogObject.Shadow\"",
            1,
        );
        fs::write(&fixture.descriptor, catalog.as_bytes()).unwrap();
        let subscription = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "EventSubscription.CatalogEvents",
        )
        .unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: subscription,
            operations: vec![source_replace(vec![MetaEventSource::Object {
                metadata_path: fixture.target.clone(),
            }])],
            dry_run: false,
        });

        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("mismatched EventSubscription GeneratedType unexpectedly prepared"),
                Err(failure) => failure,
            };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert!(failure.diagnostics[0]
            .message
            .contains("exactly one GeneratedType"));
    }

    #[test]
    fn typed_event_source_obeys_profile_support_and_atomic_rollback_guards() {
        let fixture = Fixture::new("event-source-guards-rollback");
        add_exported_event_handler(&fixture.root, &fixture.context);
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::EventSubscription,
                name: "GuardedEvents".into(),
                operations: Vec::new(),
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let subscription = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "EventSubscription.GuardedEvents",
        )
        .unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: subscription,
            operations: vec![source_replace(vec![source_family(
                crate::domain::metadata::EventSourceClass::CatalogObject,
            )])],
            dry_run: false,
        });
        let descriptor = fixture
            .root
            .join("src/EventSubscriptions/GuardedEvents.xml");
        let descriptor_preimage = fs::read(&descriptor).unwrap();

        let unsupported = String::from_utf8(descriptor_preimage.clone())
            .unwrap()
            .replacen("version=\"2.20\"", "version=\"2.19\"", 1);
        fs::write(&descriptor, unsupported.as_bytes()).unwrap();
        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("unsupported EventSubscription source unexpectedly prepared"),
                Err(failure) => failure,
            };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::CapabilityUnavailable
        );
        assert_eq!(fs::read(&descriptor).unwrap(), unsupported.as_bytes());
        fs::write(&descriptor, &descriptor_preimage).unwrap();

        let support = fixture.root.join("src/Ext/ParentConfigurations.bin");
        fs::create_dir_all(support.parent().unwrap()).unwrap();
        fs::write(
            &support,
            concat!(
                "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",0,0,0}"
            ),
        )
        .unwrap();
        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("support-locked EventSubscription source unexpectedly prepared"),
                Err(failure) => failure,
            };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::SupportLocked
        );
        assert_eq!(fs::read(&descriptor).unwrap(), descriptor_preimage);
        fs::remove_file(support).unwrap();

        let guarded_paths = [
            descriptor.clone(),
            fixture.owner.clone(),
            fixture.root.join("src/CommonModules/EventHandlers.xml"),
            fixture
                .root
                .join("src/CommonModules/EventHandlers/Ext/Module.bsl"),
        ];
        let preimages = guarded_paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let failure = match with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            prepared.publish(&cancellation)
        }) {
            Ok(_) => panic!("EventSubscription source rollback failpoint unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        for (path, preimage) in guarded_paths.iter().zip(preimages) {
            assert_eq!(fs::read(path).unwrap(), preimage, "{}", path.display());
        }
    }

    #[test]
    fn typed_edit_preview_bytes_equal_the_applied_post_image() {
        let fixture = Fixture::new("preview-apply");
        let cancellation = CancellationToken::new();
        let request = fixture.edit("Comment", MetaPropertyValue::String("typed".into()));
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let preview = prepared
            .validation_subject()
            .resources
            .iter()
            .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
            .unwrap()
            .bytes
            .clone();
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );
        assert_eq!(
            validation.status,
            crate::domain::metadata::MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );

        prepared.publish(&cancellation).unwrap();

        assert_eq!(fs::read(&fixture.descriptor).unwrap(), preview);
    }

    #[test]
    fn typed_resource_append_and_position_after_preserve_the_source_format() {
        let fixture = Fixture::new("resource-preview-apply-source-format");
        let first_resource = MetaEditOperation::add(
            MetaCollection::Resources,
            None,
            vec![MetaElementInput::named("First")],
        )
        .unwrap();
        let target = fixture.add_object_with_operations(
            MetadataKind::InformationRegister,
            "Facts",
            vec![first_resource],
        );
        let descriptor = fixture.root.join("src/InformationRegisters/Facts.xml");
        let generated = fs::read(&descriptor).unwrap();
        let source_without_bom = generated
            .strip_prefix(b"\xef\xbb\xbf")
            .expect("generated descriptor has one UTF-8 BOM");
        assert!(!source_without_bom.starts_with(b"\xef\xbb\xbf"));
        let source = String::from_utf8(source_without_bom.to_vec())
            .unwrap()
            .replacen("<Comment/>", "<Comment>First&#13;Second</Comment>", 1);
        let source = source
            .trim_end_matches(['\r', '\n'])
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n");
        let mut source_bytes = Vec::with_capacity(source.len() + 3);
        source_bytes.extend_from_slice(b"\xef\xbb\xbf");
        source_bytes.extend_from_slice(source.as_bytes());
        fs::write(&descriptor, &source_bytes).unwrap();

        let application = UnicaApplication::with_ports(Arc::new(FixedWorkspaceApplicationPorts {
            context: fixture.context.clone(),
            inner: InfrastructureApplicationPorts::new(),
        }));
        let dry_run_args = Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!(target.as_str())),
            (
                "operations".to_string(),
                json!([{
                    "op": "add",
                    "collection": "resources",
                    "elements": [{"name": "Last"}]
                }, {
                    "op": "add",
                    "collection": "resources",
                    "elements": [{
                        "name": "Second",
                        "position": {"after": "First"}
                    }]
                }]),
            ),
            ("dryRun".to_string(), json!(true)),
        ]);
        let dry_run = application
            .call_tool("unica.meta.edit", &dry_run_args)
            .unwrap();
        assert!(dry_run.ok, "{:?}", dry_run.errors);
        let dry_run_data = dry_run.data.as_ref().expect("typed preview data");
        assert_eq!(dry_run_data["changed"], true);
        assert_eq!(dry_run_data["validation"]["status"], "passed");
        assert_eq!(fs::read(&descriptor).unwrap(), source_bytes);

        let mut apply_args = dry_run_args;
        apply_args.insert("dryRun".to_string(), json!(false));
        let applied = application
            .call_tool("unica.meta.edit", &apply_args)
            .unwrap();
        assert!(applied.ok, "{:?}", applied.errors);
        assert_eq!(applied.data.as_ref().unwrap()["changed"], true);

        let updated = fs::read(&descriptor).unwrap();
        assert!(updated.starts_with(b"\xef\xbb\xbf"));
        assert!(!updated[3..].starts_with(b"\xef\xbb\xbf"));
        assert!(!updated.ends_with(b"\r\n"));
        let updated_text = std::str::from_utf8(&updated[3..]).unwrap();
        let without_crlf = updated_text.replace("\r\n", "");
        assert!(!without_crlf.contains(['\r', '\n']), "{updated_text}");
        assert!(
            updated_text.contains("<Comment>First&#13;Second</Comment>"),
            "{updated_text}"
        );
        assert!(!updated_text.contains("</Resource>&#13;"), "{updated_text}");
        let first = updated_text.find("<Name>First</Name>").unwrap();
        let second = updated_text.find("<Name>Second</Name>").unwrap();
        let last = updated_text.find("<Name>Last</Name>").unwrap();
        assert!(first < second && second < last, "{updated_text}");
    }

    #[test]
    fn typed_edit_rejects_both_mixed_eol_orders_before_preview_or_apply_side_effects() {
        fn mixed_eol_source(source: &[u8], first: &str, remaining: &str) -> Vec<u8> {
            let normalized = String::from_utf8(source.to_vec())
                .unwrap()
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            let mut output = String::with_capacity(normalized.len() + 64);
            let mut line_ending_index = 0;
            for segment in normalized.split_inclusive('\n') {
                if let Some(body) = segment.strip_suffix('\n') {
                    output.push_str(body);
                    output.push_str(if line_ending_index == 0 {
                        first
                    } else {
                        remaining
                    });
                    line_ending_index += 1;
                } else {
                    output.push_str(segment);
                }
            }
            assert!(line_ending_index > 1, "fixture must contain multiple lines");
            output.into_bytes()
        }

        for (label, first, remaining) in [
            ("lf-then-crlf", "\n", "\r\n"),
            ("crlf-then-lf", "\r\n", "\n"),
        ] {
            let fixture = Fixture::new(label);
            let original = fs::read(&fixture.descriptor).unwrap();
            let mixed = mixed_eol_source(&original, first, remaining);
            let _cwd = crate::test_support::ProcessCwdGuard::enter(&fixture.root).unwrap();
            let application = UnicaApplication::new();
            let mut observations = Vec::new();

            for dry_run in [true, false] {
                fs::write(&fixture.descriptor, &mixed).unwrap();
                let before = crate::test_support::tree_snapshot(&fixture.root);
                let result = application
                    .call_tool(
                        "unica.meta.edit",
                        &Map::from_iter([
                            ("sourceSet".to_string(), json!("main")),
                            ("metadataPath".to_string(), json!(fixture.target.as_str())),
                            (
                                "operations".to_string(),
                                json!([{
                                    "op": "setProperties",
                                    "values": {"Comment": "must not publish"}
                                }]),
                            ),
                            ("dryRun".to_string(), json!(dry_run)),
                        ]),
                    )
                    .expect("public typed meta.edit call");
                let after = crate::test_support::tree_snapshot(&fixture.root);
                observations.push((dry_run, result, before, after));
            }

            for (dry_run, result, before, after) in observations {
                assert!(
                    !result.ok,
                    "{label} dryRun={dry_run} unexpectedly succeeded"
                );
                assert_eq!(
                    result
                        .diagnostics
                        .as_ref()
                        .and_then(serde_json::Value::as_array)
                        .and_then(|items| items.first())
                        .and_then(|item| item.get("code"))
                        .and_then(serde_json::Value::as_str),
                    Some("validation_failed"),
                    "{label} dryRun={dry_run}: {:?}",
                    result.diagnostics
                );
                let message = result
                    .diagnostics
                    .as_ref()
                    .and_then(serde_json::Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(|item| item.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                assert!(
                    message.contains("metadata descriptor EOL policy failed")
                        && message.contains("mixed line endings"),
                    "{label} dryRun={dry_run}: {message}"
                );
                assert!(result.cache.events.is_empty(), "{label} dryRun={dry_run}");
                assert!(
                    result.cache.invalidated.is_empty(),
                    "{label} dryRun={dry_run}"
                );
                assert!(
                    result.cache.refreshed.is_empty(),
                    "{label} dryRun={dry_run}"
                );
                assert_eq!(after, before, "{label} dryRun={dry_run}");
            }
        }
    }

    #[test]
    fn typed_edit_preserves_uniform_eol_bom_and_terminal_newline_profiles() {
        fn uniform_source(
            source: &[u8],
            eol: &str,
            has_bom: bool,
            terminal_newline: bool,
        ) -> Vec<u8> {
            let normalized = String::from_utf8(source.to_vec())
                .unwrap()
                .trim_start_matches('\u{feff}')
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            let normalized = normalized.trim_end_matches('\n').replace('\n', eol);
            let mut output = Vec::new();
            if has_bom {
                output.extend_from_slice(b"\xef\xbb\xbf");
            }
            output.extend_from_slice(normalized.as_bytes());
            if terminal_newline {
                output.extend_from_slice(eol.as_bytes());
            }
            output
        }

        for (eol_label, eol) in [("lf", "\n"), ("crlf", "\r\n"), ("cr", "\r")] {
            for (has_bom, terminal_newline) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                let label = format!(
                    "{eol_label}-{}-{}",
                    if has_bom { "bom" } else { "no-bom" },
                    if terminal_newline {
                        "terminal"
                    } else {
                        "no-terminal"
                    }
                );
                let fixture = Fixture::new(&label);
                let source = uniform_source(
                    &fs::read(&fixture.descriptor).unwrap(),
                    eol,
                    has_bom,
                    terminal_newline,
                );
                fs::write(&fixture.descriptor, &source).unwrap();
                let request = fixture.edit(
                    "Comment",
                    MetaPropertyValue::String("uniform source edit".into()),
                );
                let cancellation = CancellationToken::new();

                let prepared =
                    MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                        .unwrap();
                let post_image = prepared
                    .validation_subject()
                    .resources
                    .iter()
                    .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
                    .unwrap()
                    .bytes
                    .clone();
                let body = if has_bom {
                    assert!(post_image.starts_with(b"\xef\xbb\xbf"), "{label}");
                    &post_image[3..]
                } else {
                    assert!(!post_image.starts_with(b"\xef\xbb\xbf"), "{label}");
                    post_image.as_slice()
                };
                let without_eol = body
                    .split(|byte| *byte == b'\n' || *byte == b'\r')
                    .collect::<Vec<_>>();
                assert!(without_eol.len() > 1, "{label}");
                match eol {
                    "\n" => assert!(!body.contains(&b'\r'), "{label}"),
                    "\r\n" => {
                        let stripped = body.windows(2).filter(|pair| *pair == b"\r\n").count();
                        assert_eq!(stripped, body.iter().filter(|byte| **byte == b'\n').count());
                        assert_eq!(stripped, body.iter().filter(|byte| **byte == b'\r').count());
                    }
                    "\r" => assert!(!body.contains(&b'\n'), "{label}"),
                    _ => unreachable!(),
                }
                assert_eq!(body.ends_with(eol.as_bytes()), terminal_newline, "{label}");

                prepared.publish(&cancellation).unwrap();
                assert_eq!(
                    fs::read(&fixture.descriptor).unwrap(),
                    post_image,
                    "{label}"
                );
            }
        }
    }

    #[test]
    fn typed_edit_uses_explicit_lf_policy_when_source_has_no_line_endings() {
        for has_bom in [false, true] {
            let label = if has_bom {
                "no-eol-explicit-lf-bom"
            } else {
                "no-eol-explicit-lf-no-bom"
            };
            let fixture = Fixture::new(label);
            let normalized = String::from_utf8(fs::read(&fixture.descriptor).unwrap())
                .unwrap()
                .trim_start_matches('\u{feff}')
                .replace("\r\n", "")
                .replace(['\r', '\n'], "");
            let mut source = Vec::new();
            if has_bom {
                source.extend_from_slice(b"\xef\xbb\xbf");
            }
            source.extend_from_slice(normalized.as_bytes());
            fs::write(&fixture.descriptor, source).unwrap();
            let request = fixture.edit(
                "Synonym",
                MetaPropertyValue::String("explicit LF policy".into()),
            );
            let cancellation = CancellationToken::new();

            let prepared =
                MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                    .unwrap();
            let post_image = prepared
                .validation_subject()
                .resources
                .iter()
                .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
                .unwrap()
                .bytes
                .clone();

            assert_eq!(post_image.starts_with(b"\xef\xbb\xbf"), has_bom);
            let body = if has_bom {
                &post_image[3..]
            } else {
                post_image.as_slice()
            };
            assert!(body.contains(&b'\n'));
            assert!(!body.contains(&b'\r'));
            assert!(!body.ends_with(b"\n"));
            prepared.publish(&cancellation).unwrap();
            assert_eq!(fs::read(&fixture.descriptor).unwrap(), post_image);
        }
    }

    #[test]
    fn typed_edit_republishes_double_bom_preamble_as_single_bom() {
        // Патологическая двойная преамбула чинится до одной, и больше ничего
        // не сдвигается: пост-образ от двух-BOM источника побайтово равен
        // пост-образу той же правки над одно-BOM источником.
        let fixture = Fixture::new("double-bom");
        let body = String::from_utf8(fs::read(&fixture.descriptor).unwrap())
            .unwrap()
            .trim_start_matches('\u{feff}')
            .to_string();
        let cancellation = CancellationToken::new();

        let mut post_images = Vec::new();
        let mut prepared_double = None;
        for bom_count in [1usize, 2] {
            let mut source = Vec::new();
            for _ in 0..bom_count {
                source.extend_from_slice(b"\xef\xbb\xbf");
            }
            source.extend_from_slice(body.as_bytes());
            fs::write(&fixture.descriptor, source).unwrap();
            let request = fixture.edit(
                "Comment",
                MetaPropertyValue::String("single bom preamble".into()),
            );
            let prepared =
                MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                    .unwrap();
            post_images.push(
                prepared
                    .validation_subject()
                    .resources
                    .iter()
                    .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
                    .unwrap()
                    .bytes
                    .clone(),
            );
            prepared_double = Some(prepared);
        }

        assert_eq!(post_images[0], post_images[1]);
        assert!(post_images[1].starts_with(b"\xef\xbb\xbf"));
        assert!(!post_images[1].starts_with(b"\xef\xbb\xbf\xef\xbb\xbf"));
        prepared_double.unwrap().publish(&cancellation).unwrap();
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), post_images[1]);
    }

    #[test]
    fn typed_edit_semantic_noop_has_no_publication_plan_and_writes_nothing() {
        let fixture = Fixture::new("noop");
        let cancellation = CancellationToken::new();
        let before = fs::read(&fixture.descriptor).unwrap();
        let request = fixture.edit("Comment", MetaPropertyValue::String(String::new()));
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();

        assert!(!prepared.preview().changed);
        assert!(prepared.preview().publication_plan.is_empty());
        prepared.publish(&cancellation).unwrap();
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), before);
    }

    #[test]
    fn ordinary_edit_validates_the_complete_unchanged_final_child_graph() {
        let fixture = Fixture::new("ordinary-edit-complete-child-graph");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Existing");
        let mut owner = String::from_utf8(fs::read(&fixture.descriptor).unwrap()).unwrap();
        let form_start = owner.find("<Form uuid=").unwrap();
        let form_end = owner[form_start..].find("</Form>").unwrap() + form_start + "</Form>".len();
        owner.replace_range(form_start..form_end, "<Form>Existing</Form>");
        fs::write(&fixture.descriptor, owner.as_bytes()).unwrap();
        let child_descriptor = fixture
            .root
            .join("src/Catalogs/Editable/Forms/Existing.xml");
        let child_content = fixture
            .root
            .join("src/Catalogs/Editable/Forms/Existing/Ext/Form.xml");
        let child_descriptor_before = fs::read(&child_descriptor).unwrap();
        let child_content_before = fs::read(&child_content).unwrap();
        let request = fixture.edit("Comment", MetaPropertyValue::String("changed".into()));

        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );

        assert_eq!(
            validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );
        assert!(prepared
            .validation_subject()
            .child_footprints
            .iter()
            .any(|footprint| {
                footprint.child.as_str() == "Catalog.Editable.Form.Existing"
                    && footprint.profile == MetadataChildProfile::Form
            }));
        assert!(prepared
            .preview()
            .publication_plan
            .iter()
            .all(|entry| entry.resource == MetaPublicationResource::Descriptor));

        prepared.publish(&cancellation).unwrap();

        assert_eq!(fs::read(child_descriptor).unwrap(), child_descriptor_before);
        assert_eq!(fs::read(child_content).unwrap(), child_content_before);
    }

    #[test]
    fn transient_typed_child_add_remove_preserves_preexisting_empty_collection_directory() {
        for (label, collection, directory) in [
            ("forms", MetaCollection::Forms, "Forms"),
            ("templates", MetaCollection::Templates, "Templates"),
            ("commands", MetaCollection::Commands, "Commands"),
        ] {
            let fixture = Fixture::new(&format!("empty-{label}-add-remove-noop"));
            let cancellation = CancellationToken::new();
            let collection_dir = fixture
                .root
                .join(format!("src/Catalogs/Editable/{directory}"));
            fs::create_dir_all(&collection_dir).unwrap();
            let owner_before = fs::read(&fixture.descriptor).unwrap();
            let request = fixture.typed_edit(vec![
                MetaEditOperation::add(
                    collection,
                    None,
                    vec![MetaElementInput::named("Transient")],
                )
                .unwrap(),
                MetaEditOperation::remove(collection, None, vec!["Transient".into()]).unwrap(),
            ]);

            let prepared =
                MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                    .unwrap();

            assert!(!prepared.preview().changed, "{label}");
            assert!(prepared.preview().publication_plan.is_empty(), "{label}");
            assert!(
                prepared.validation_subject().child_footprints.is_empty(),
                "{label}"
            );
            let report = prepared.publish(&cancellation).unwrap();
            assert!(!report.data.changed, "{label}");
            assert!(report.data.publication_plan.is_empty(), "{label}");
            assert_eq!(
                fs::read(&fixture.descriptor).unwrap(),
                owner_before,
                "{label}"
            );
            assert!(collection_dir.is_dir(), "{label}");
            assert_eq!(fs::read_dir(&collection_dir).unwrap().count(), 0, "{label}");
        }
    }

    #[test]
    fn net_zero_child_sequences_restore_every_empty_childobjects_spelling_byte_exactly() {
        for (spelling_label, spelling) in [
            ("self-closing", "<ChildObjects/>"),
            ("expanded", "<ChildObjects></ChildObjects>"),
            ("whitespace-expanded", "<ChildObjects>\n\t\t</ChildObjects>"),
            (
                "comment-expanded",
                "<ChildObjects><!-- preserve me -->\n\t\t</ChildObjects>",
            ),
        ] {
            for (collection_label, collection, directory) in [
                ("forms", MetaCollection::Forms, "Forms"),
                ("templates", MetaCollection::Templates, "Templates"),
                ("commands", MetaCollection::Commands, "Commands"),
            ] {
                let fixture =
                    Fixture::new(&format!("net-zero-{spelling_label}-{collection_label}"));
                let cancellation = CancellationToken::new();
                let source = String::from_utf8(fs::read(&fixture.descriptor).unwrap()).unwrap();
                let source = source.replacen("<ChildObjects/>", spelling, 1);
                fs::write(&fixture.descriptor, source.as_bytes()).unwrap();
                let collection_dir = fixture
                    .root
                    .join(format!("src/Catalogs/Editable/{directory}"));
                fs::create_dir_all(&collection_dir).unwrap();
                let owner_before = fs::read(&fixture.descriptor).unwrap();
                let request = fixture.typed_edit(vec![
                    MetaEditOperation::add(
                        collection,
                        None,
                        vec![MetaElementInput::named("Transient")],
                    )
                    .unwrap(),
                    MetaEditOperation::update(
                        collection,
                        None,
                        vec![MetaElementUpdateInput {
                            name: "Transient".into(),
                            new_name: Some("Renamed".into()),
                            ..MetaElementUpdateInput::default()
                        }],
                    )
                    .unwrap(),
                    MetaEditOperation::remove(collection, None, vec!["Renamed".into()]).unwrap(),
                ]);

                let prepared =
                    MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                        .unwrap();

                assert!(
                    !prepared.preview().changed,
                    "{spelling_label}/{collection_label}"
                );
                assert!(
                    prepared.preview().publication_plan.is_empty(),
                    "{spelling_label}/{collection_label}"
                );
                assert!(
                    prepared.validation_subject().child_footprints.is_empty(),
                    "{spelling_label}/{collection_label}"
                );
                let report = prepared.publish(&cancellation).unwrap();
                assert!(!report.data.changed, "{spelling_label}/{collection_label}");
                assert_eq!(
                    fs::read(&fixture.descriptor).unwrap(),
                    owner_before,
                    "{spelling_label}/{collection_label}"
                );
                assert!(collection_dir.is_dir());
                assert_eq!(fs::read_dir(collection_dir).unwrap().count(), 0);
            }
        }
    }

    #[test]
    fn empty_fragment_restoration_does_not_erase_property_or_unrelated_child_edits() {
        let fixture = Fixture::new("net-zero-child-with-property-edit");
        let cancellation = CancellationToken::new();
        let source = String::from_utf8(fs::read(&fixture.descriptor).unwrap())
            .unwrap()
            .replacen(
                "<ChildObjects/>",
                "<ChildObjects><!-- exact source comment -->\n\t\t</ChildObjects>",
                1,
            );
        fs::write(&fixture.descriptor, source.as_bytes()).unwrap();
        let request = fixture.typed_edit(vec![
            MetaEditOperation::SetProperties {
                values: MetaPropertyChanges::convert(
                    MetadataKind::Catalog,
                    vec![MetaPropertyInput::new(
                        "Comment",
                        MetaPropertyValue::String("real property edit".into()),
                    )],
                )
                .unwrap(),
            },
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput::named("Transient")],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Transient".into()])
                .unwrap(),
        ]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let descriptor_post_image = prepared
            .validation_subject()
            .resources
            .iter()
            .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
            .unwrap()
            .bytes
            .clone();
        let post_image = String::from_utf8(descriptor_post_image.clone()).unwrap();
        assert!(prepared.preview().changed);
        assert!(post_image.contains("<Comment>real property edit</Comment>"));
        assert!(
            post_image.contains("<ChildObjects><!-- exact source comment -->\n\t\t</ChildObjects>")
        );
        assert!(prepared
            .preview()
            .publication_plan
            .iter()
            .all(|entry| entry.resource == MetaPublicationResource::Descriptor));
        prepared.publish(&cancellation).unwrap();
        assert_eq!(
            fs::read(&fixture.descriptor).unwrap(),
            descriptor_post_image
        );

        let retained = Fixture::new("net-zero-child-with-retained-child-edit");
        retained.publish_form_add("Stable");
        let request = retained.typed_edit(vec![
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput::named("Transient")],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Forms,
                None,
                vec![MetaElementUpdateInput {
                    name: "Stable".into(),
                    comment: Some("real child edit".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Transient".into()])
                .unwrap(),
        ]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &retained.context, &cancellation)
                .unwrap();
        assert!(prepared.preview().changed);
        assert!(prepared.preview().publication_plan.iter().any(|entry| {
            entry.resource == MetaPublicationResource::Form
                && entry.action == MetaPublicationAction::Update
                && entry
                    .metadata_path
                    .as_ref()
                    .is_some_and(|path| path.as_str() == "Catalog.Editable.Form.Stable")
        }));
        assert!(!prepared.preview().publication_plan.iter().any(|entry| {
            entry
                .metadata_path
                .as_ref()
                .is_some_and(|path| path.as_str().contains("Transient"))
        }));
        prepared.publish(&cancellation).unwrap();
        assert!(String::from_utf8(
            fs::read(retained.root.join("src/Catalogs/Editable/Forms/Stable.xml")).unwrap()
        )
        .unwrap()
        .contains("real child edit"));
    }

    #[test]
    fn typed_edit_concurrency_and_rollback_preserve_exact_external_or_preimage_bytes() {
        let fixture = Fixture::new("concurrency-rollback");
        let cancellation = CancellationToken::new();
        let request = fixture.edit("Comment", MetaPropertyValue::String("changed".into()));
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let mut external = fs::read(&fixture.descriptor).unwrap();
        external.extend_from_slice(b"\n");
        fs::write(&fixture.descriptor, &external).unwrap();
        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("concurrent metadata edit unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), external);

        fs::write(&fixture.descriptor, &external[..external.len() - 1]).unwrap();
        let before = fs::read(&fixture.descriptor).unwrap();
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let failure = match with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            prepared.publish(&cancellation)
        }) {
            Ok(_) => panic!("metadata edit failpoint unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), before);

        let owner_before = fs::read(&fixture.owner).unwrap();
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let mut owner_external = owner_before.clone();
        owner_external.extend_from_slice(b"\n");
        fs::write(&fixture.owner, &owner_external).unwrap();
        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("concurrent owner edit unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(&fixture.owner).unwrap(), owner_external);
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), before);
    }

    #[test]
    fn typed_form_resource_creation_rolls_back_with_the_owner_descriptor() {
        let fixture = Fixture::new("form-resource-rollback");
        let cancellation = CancellationToken::new();
        let descriptor_before = fs::read(&fixture.descriptor).unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: fixture.target.clone(),
            operations: vec![MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput::named("ObjectForm")],
            )
            .unwrap()],
            dry_run: false,
        });
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();

        let failure = match with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            prepared.publish(&cancellation)
        }) {
            Ok(_) => panic!("form child failpoint unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), descriptor_before);
        assert!(!fixture
            .root
            .join("src/Catalogs/Editable/Forms/ObjectForm.xml")
            .exists());
        assert!(!fixture
            .root
            .join("src/Catalogs/Editable/Forms/ObjectForm")
            .exists());
    }

    #[test]
    fn typed_form_add_validation_covers_content_and_publish_writes_exact_footprint() {
        let fixture = Fixture::new("form-resource-publish-add");
        let cancellation = CancellationToken::new();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: fixture.target.clone(),
            operations: vec![MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput::named("ObjectForm")],
            )
            .unwrap()],
            dry_run: false,
        });
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let expected_content = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
            "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n",
            "</Form>"
        )
        .as_bytes();
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| resource.bytes == expected_content));

        prepared.publish(&cancellation).unwrap();

        let descriptor = fixture
            .root
            .join("src/Catalogs/Editable/Forms/ObjectForm.xml");
        let content = fixture
            .root
            .join("src/Catalogs/Editable/Forms/ObjectForm/Ext/Form.xml");
        assert!(descriptor.is_file());
        assert_eq!(fs::read(content).unwrap(), expected_content);
    }

    #[test]
    fn typed_form_rename_publish_preserves_every_payload_file_and_removes_old_tree() {
        let fixture = Fixture::new("form-resource-publish-rename");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Old");
        let old_root = fixture.root.join("src/Catalogs/Editable/Forms/Old");
        let old_descriptor = fixture.root.join("src/Catalogs/Editable/Forms/Old.xml");
        let content = fs::read(old_root.join("Ext/Form.xml")).unwrap();
        let module = b"procedure Kept()\nendprocedure".to_vec();
        write_form_module(&old_root, &module);
        let request = fixture.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Old".into(),
                new_name: Some("New".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| resource.bytes == content));
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| resource.bytes == module));

        prepared.publish(&cancellation).unwrap();

        let new_root = fixture.root.join("src/Catalogs/Editable/Forms/New");
        assert!(!old_descriptor.exists());
        assert!(!old_root.exists());
        assert!(fixture
            .root
            .join("src/Catalogs/Editable/Forms/New.xml")
            .is_file());
        assert_eq!(fs::read(new_root.join("Ext/Form.xml")).unwrap(), content);
        assert_eq!(
            fs::read(new_root.join("Ext/Form/Module.bsl")).unwrap(),
            module
        );
    }

    #[test]
    fn typed_form_remove_publish_deletes_descriptor_and_complete_payload_tree() {
        let fixture = Fixture::new("form-resource-publish-remove");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Gone");
        let payload_root = fixture.root.join("src/Catalogs/Editable/Forms/Gone");
        write_form_module(&payload_root, b"procedure Gone()\nendprocedure");
        let child_descriptor = fixture.root.join("src/Catalogs/Editable/Forms/Gone.xml");
        let request = fixture.typed_edit(vec![MetaEditOperation::remove(
            MetaCollection::Forms,
            None,
            vec!["Gone".into()],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        assert_eq!(prepared.preview().publication_plan.len(), 2);

        prepared.publish(&cancellation).unwrap();

        assert!(!child_descriptor.exists());
        assert!(!payload_root.exists());
        assert!(!fixture.root.join("src/Catalogs/Editable/Forms").exists());
    }

    #[test]
    fn typed_form_remove_preserves_collection_directory_when_another_child_remains() {
        let fixture = Fixture::new("form-resource-publish-remove-with-sibling");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Removed");
        fixture.publish_form_add("Remaining");
        let forms = fixture.root.join("src/Catalogs/Editable/Forms");
        let request = fixture.typed_edit(vec![MetaEditOperation::remove(
            MetaCollection::Forms,
            None,
            vec!["Removed".into()],
        )
        .unwrap()]);

        MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
            .unwrap()
            .publish(&cancellation)
            .unwrap();

        assert!(forms.is_dir());
        assert!(!forms.join("Removed.xml").exists());
        assert!(!forms.join("Removed").exists());
        assert!(forms.join("Remaining.xml").is_file());
        assert!(forms.join("Remaining/Ext/Form.xml").is_file());
    }

    #[test]
    fn typed_last_form_remove_rollback_restores_the_exact_collection_tree() {
        let fixture = Fixture::new("form-resource-last-remove-rollback");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Only");
        let forms = fixture.root.join("src/Catalogs/Editable/Forms");
        let descriptor = forms.join("Only.xml");
        let content = forms.join("Only/Ext/Form.xml");
        let module = write_form_module(&forms.join("Only"), b"procedure Kept()\nendprocedure");
        let owner_before = fs::read(&fixture.descriptor).unwrap();
        let descriptor_before = fs::read(&descriptor).unwrap();
        let content_before = fs::read(&content).unwrap();
        let module_before = fs::read(&module).unwrap();
        let request = fixture.typed_edit(vec![MetaEditOperation::remove(
            MetaCollection::Forms,
            None,
            vec!["Only".into()],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();

        let failure = match with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            prepared.publish(&cancellation)
        }) {
            Ok(_) => panic!("last-form removal failpoint unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert!(forms.is_dir());
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), owner_before);
        assert_eq!(fs::read(descriptor).unwrap(), descriptor_before);
        assert_eq!(fs::read(content).unwrap(), content_before);
        assert_eq!(fs::read(module).unwrap(), module_before);
    }

    #[test]
    fn typed_form_remove_then_add_publish_replaces_the_complete_existing_tree() {
        let fixture = Fixture::new("form-resource-publish-remove-add");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("A");
        let payload_root = fixture.root.join("src/Catalogs/Editable/Forms/A");
        let descriptor = fixture.root.join("src/Catalogs/Editable/Forms/A.xml");
        let stale_module = write_form_module(&payload_root, b"procedure Stale()\nendprocedure");
        let request = fixture.typed_edit(vec![
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput {
                    name: "A".into(),
                    comment: Some("replacement".into()),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
        ]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let child_plan = prepared
            .preview()
            .publication_plan
            .iter()
            .find(|entry| entry.resource == MetaPublicationResource::Form)
            .unwrap();
        assert_eq!(child_plan.action, MetaPublicationAction::Update);

        prepared.publish(&cancellation).unwrap();

        let descriptor_bytes = fs::read(descriptor).unwrap();
        assert!(String::from_utf8_lossy(&descriptor_bytes).contains("replacement"));
        assert_eq!(
            fs::read(payload_root.join("Ext/Form.xml")).unwrap(),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
                "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n",
                "</Form>"
            )
            .as_bytes()
        );
        assert!(!stale_module.exists());
    }

    #[test]
    fn typed_form_remove_add_remove_publish_deletes_only_the_initial_tree() {
        let fixture = Fixture::new("form-resource-publish-remove-add-remove");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("A");
        let payload_root = fixture.root.join("src/Catalogs/Editable/Forms/A");
        let descriptor = fixture.root.join("src/Catalogs/Editable/Forms/A.xml");
        write_form_module(&payload_root, b"procedure Initial()\nendprocedure");
        let request = fixture.typed_edit(vec![
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput {
                    name: "A".into(),
                    comment: Some("transient".into()),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
        ]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let child_plan = prepared
            .preview()
            .publication_plan
            .iter()
            .find(|entry| entry.resource == MetaPublicationResource::Form)
            .unwrap();
        assert_eq!(child_plan.action, MetaPublicationAction::Remove);

        prepared.publish(&cancellation).unwrap();

        assert!(!descriptor.exists());
        assert!(!payload_root.exists());
        assert!(!fixture.root.join("src/Catalogs/Editable/Forms").exists());
    }

    #[test]
    fn typed_form_exact_noop_update_has_no_plan_write_or_effect() {
        let fixture = Fixture::new("form-resource-noop");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Stable");
        let owner_before = fs::read(&fixture.descriptor).unwrap();
        let child = fixture.root.join("src/Catalogs/Editable/Forms/Stable.xml");
        let child_before = fs::read(&child).unwrap();
        let request = fixture.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Stable".into(),
                comment: Some(String::new()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        assert!(!prepared.preview().changed);
        assert!(prepared.preview().publication_plan.is_empty());

        prepared.publish(&cancellation).unwrap();

        assert_eq!(fs::read(&fixture.descriptor).unwrap(), owner_before);
        assert_eq!(fs::read(child).unwrap(), child_before);
    }

    #[test]
    fn typed_form_rename_rejects_payload_byte_and_topology_drift_without_partial_publish() {
        for drift_kind in ["bytes", "topology"] {
            let fixture = Fixture::new(&format!("form-resource-drift-{drift_kind}"));
            let cancellation = CancellationToken::new();
            fixture.publish_form_add("Old");
            let old_root = fixture.root.join("src/Catalogs/Editable/Forms/Old");
            let owner_before = fs::read(&fixture.descriptor).unwrap();
            let request = fixture.typed_edit(vec![MetaEditOperation::update(
                MetaCollection::Forms,
                None,
                vec![MetaElementUpdateInput {
                    name: "Old".into(),
                    new_name: Some("New".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()]);
            let prepared =
                MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                    .unwrap();
            let external_path = if drift_kind == "bytes" {
                let path = old_root.join("Ext/Form.xml");
                fs::write(&path, b"<Form external=\"true\"/>").unwrap();
                path
            } else {
                let path = old_root.join("Ext/External.bsl");
                fs::write(&path, b"procedure External()\nendprocedure").unwrap();
                path
            };
            let external = fs::read(&external_path).unwrap();

            let failure = match prepared.publish(&cancellation) {
                Ok(_) => panic!("{drift_kind} drift unexpectedly published"),
                Err(failure) => failure,
            };
            assert_eq!(
                failure.diagnostics[0].code,
                MetaDiagnosticCode::ConcurrentModification
            );
            assert_eq!(fs::read(&external_path).unwrap(), external);
            assert_eq!(fs::read(&fixture.descriptor).unwrap(), owner_before);
            assert!(!fixture
                .root
                .join("src/Catalogs/Editable/Forms/New.xml")
                .exists());
            assert!(!fixture
                .root
                .join("src/Catalogs/Editable/Forms/New")
                .exists());
        }
    }

    #[test]
    fn typed_form_rename_fault_rolls_back_to_exact_initial_tree_and_no_final_tree() {
        let fixture = Fixture::new("form-resource-rename-rollback");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Old");
        let old_root = fixture.root.join("src/Catalogs/Editable/Forms/Old");
        let old_descriptor = fixture.root.join("src/Catalogs/Editable/Forms/Old.xml");
        write_form_module(&old_root, b"procedure Kept()\nendprocedure");
        let owner_before = fs::read(&fixture.descriptor).unwrap();
        let descriptor_before = fs::read(&old_descriptor).unwrap();
        let form_before = fs::read(old_root.join("Ext/Form.xml")).unwrap();
        let module_before = fs::read(old_root.join("Ext/Form/Module.bsl")).unwrap();
        let request = fixture.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Old".into(),
                new_name: Some("New".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();

        let failure = match with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            prepared.publish(&cancellation)
        }) {
            Ok(_) => panic!("rename failpoint unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), owner_before);
        assert_eq!(fs::read(old_descriptor).unwrap(), descriptor_before);
        assert_eq!(
            fs::read(old_root.join("Ext/Form.xml")).unwrap(),
            form_before
        );
        assert_eq!(
            fs::read(old_root.join("Ext/Form/Module.bsl")).unwrap(),
            module_before
        );
        assert!(!fixture
            .root
            .join("src/Catalogs/Editable/Forms/New.xml")
            .exists());
        assert!(!fixture
            .root
            .join("src/Catalogs/Editable/Forms/New")
            .exists());
    }

    #[test]
    fn typed_template_and_command_publish_complete_add_rename_remove_lifecycle() {
        let fixture = Fixture::new("template-command-lifecycle");
        let cancellation = CancellationToken::new();
        let add = fixture.typed_edit(vec![
            MetaEditOperation::add(
                MetaCollection::Templates,
                None,
                vec![MetaElementInput::named("Print")],
            )
            .unwrap(),
            MetaEditOperation::add(
                MetaCollection::Commands,
                None,
                vec![MetaElementInput::named("Run")],
            )
            .unwrap(),
        ]);
        let prepared =
            MetadataOperations::prepare_mutation(&add, &fixture.context, &cancellation).unwrap();
        assert!(prepared
            .validation_subject()
            .child_footprints
            .iter()
            .any(|footprint| {
                footprint.profile == MetadataChildProfile::Command
                    && footprint.directories.is_empty()
            }));
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );
        assert_eq!(
            validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );
        prepared.publish(&cancellation).unwrap();
        let object_root = fixture.root.join("src/Catalogs/Editable");
        let template_content = object_root.join("Templates/Print/Ext/Template.xml");
        assert!(object_root.join("Templates/Print.xml").is_file());
        assert!(template_content.is_file());
        assert!(
            !object_root.join("Commands/Run.xml").exists(),
            "platform commands are described inline in the owner"
        );
        assert!(!object_root.join("Commands/Run").exists());
        let command_module = object_root.join("Commands/Run/Ext/CommandModule.bsl");
        fs::create_dir_all(command_module.parent().unwrap()).unwrap();
        fs::write(&command_module, b"procedure Run()\nendprocedure").unwrap();
        let template_bytes = fs::read(&template_content).unwrap();
        let module_bytes = fs::read(&command_module).unwrap();

        let rename = fixture.typed_edit(vec![
            MetaEditOperation::update(
                MetaCollection::Templates,
                None,
                vec![MetaElementUpdateInput {
                    name: "Print".into(),
                    new_name: Some("PrintNew".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Commands,
                None,
                vec![MetaElementUpdateInput {
                    name: "Run".into(),
                    new_name: Some("RunNew".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ]);
        let prepared =
            MetadataOperations::prepare_mutation(&rename, &fixture.context, &cancellation).unwrap();
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| resource.bytes == template_bytes));
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| resource.bytes == module_bytes));
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| matches!(
                resource.role,
                MetadataResourceRole::ChildResource {
                    kind: MetadataChildResourceKind::Module,
                    ordinal: 0,
                    ref child,
                } if child.as_str() == "Catalog.Editable.Command.RunNew"
            )));
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );
        assert_eq!(
            validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );
        prepared.publish(&cancellation).unwrap();
        assert!(!object_root.join("Templates/Print.xml").exists());
        assert!(!object_root.join("Commands/Run.xml").exists());
        assert_eq!(
            fs::read(object_root.join("Templates/PrintNew/Ext/Template.xml")).unwrap(),
            template_bytes
        );
        assert_eq!(
            fs::read(object_root.join("Commands/RunNew/Ext/CommandModule.bsl")).unwrap(),
            module_bytes
        );

        let remove = fixture.typed_edit(vec![
            MetaEditOperation::remove(MetaCollection::Templates, None, vec!["PrintNew".into()])
                .unwrap(),
            MetaEditOperation::remove(MetaCollection::Commands, None, vec!["RunNew".into()])
                .unwrap(),
        ]);
        MetadataOperations::prepare_mutation(&remove, &fixture.context, &cancellation)
            .unwrap()
            .publish(&cancellation)
            .unwrap();
        assert!(!object_root.join("Templates/PrintNew.xml").exists());
        assert!(!object_root.join("Templates/PrintNew").exists());
        assert!(!object_root.join("Commands/RunNew.xml").exists());
        assert!(!object_root.join("Commands/RunNew").exists());
        assert!(!object_root.join("Templates").exists());
        assert!(!object_root.join("Commands").exists());
    }

    #[test]
    fn retained_template_payload_must_match_its_descriptor_template_type() {
        let fixture = Fixture::new("template-payload-type-mismatch");
        let cancellation = CancellationToken::new();
        let add = fixture.typed_edit(vec![MetaEditOperation::add(
            MetaCollection::Templates,
            None,
            vec![MetaElementInput::named("Main")],
        )
        .unwrap()]);
        MetadataOperations::prepare_mutation(&add, &fixture.context, &cancellation)
            .unwrap()
            .publish(&cancellation)
            .unwrap();
        let child_descriptor = fixture
            .root
            .join("src/Catalogs/Editable/Templates/Main.xml");
        for descriptor in [&fixture.descriptor, &child_descriptor] {
            let bytes = fs::read(descriptor).unwrap();
            let text = String::from_utf8(bytes).unwrap().replace(
                "<TemplateType>SpreadsheetDocument</TemplateType>",
                "<TemplateType>DataCompositionSchema</TemplateType>",
            );
            fs::write(descriptor, text).unwrap();
        }
        let request = fixture.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Templates,
            None,
            vec![MetaElementUpdateInput {
                name: "Main".into(),
                comment: Some("retained".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();

        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );

        assert_eq!(validation.status, MetaValidationStatus::Failed);
        assert!(validation.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .field
                .as_deref()
                .is_some_and(|field| field.starts_with("resources[") && field.ends_with("].bytes"))
        }));
    }

    #[test]
    fn retained_template_footprint_mismatch_is_logical_and_provider_neutral() {
        let fixture = Fixture::new("template-footprint-type-mismatch");
        let cancellation = CancellationToken::new();
        let add = fixture.typed_edit(vec![MetaEditOperation::add(
            MetaCollection::Templates,
            None,
            vec![MetaElementInput::named("Main")],
        )
        .unwrap()]);
        MetadataOperations::prepare_mutation(&add, &fixture.context, &cancellation)
            .unwrap()
            .publish(&cancellation)
            .unwrap();
        let child_descriptor = fixture
            .root
            .join("src/Catalogs/Editable/Templates/Main.xml");
        for descriptor in [&fixture.descriptor, &child_descriptor] {
            let text = String::from_utf8(fs::read(descriptor).unwrap())
                .unwrap()
                .replace(
                    "<TemplateType>SpreadsheetDocument</TemplateType>",
                    "<TemplateType>TextDocument</TemplateType>",
                );
            fs::write(descriptor, text).unwrap();
        }
        let request = fixture.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Templates,
            None,
            vec![MetaElementUpdateInput {
                name: "Main".into(),
                comment: Some("retained".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);

        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("Template.xml accepted for TextDocument"),
                Err(failure) => failure,
            };

        let diagnostic = &failure.diagnostics[0];
        assert!(diagnostic
            .message
            .contains("Catalog.Editable.Template.Main"));
        assert!(!diagnostic
            .message
            .contains(&fixture.root.to_string_lossy().into_owned()));
        assert_eq!(diagnostic.field.as_deref(), Some("resources.child.payload"));
    }

    #[test]
    fn retained_mandatory_child_payload_cannot_be_wholly_absent() {
        for (label, collection, template_type) in [
            ("form", MetaCollection::Forms, None),
            (
                "spreadsheet",
                MetaCollection::Templates,
                Some("SpreadsheetDocument"),
            ),
            (
                "schema",
                MetaCollection::Templates,
                Some("DataCompositionSchema"),
            ),
            ("text", MetaCollection::Templates, Some("TextDocument")),
            ("binary", MetaCollection::Templates, Some("BinaryData")),
            ("html", MetaCollection::Templates, Some("HTMLDocument")),
        ] {
            let fixture = Fixture::new(&format!("missing-whole-child-payload-{label}"));
            let cancellation = CancellationToken::new();
            let add = fixture.typed_edit(vec![MetaEditOperation::add(
                collection,
                None,
                vec![MetaElementInput::named("Main")],
            )
            .unwrap()]);
            MetadataOperations::prepare_mutation(&add, &fixture.context, &cancellation)
                .unwrap()
                .publish(&cancellation)
                .unwrap();
            let collection_dir = match collection {
                MetaCollection::Forms => "Forms",
                MetaCollection::Templates => "Templates",
                _ => unreachable!(),
            };
            let child_descriptor = fixture
                .root
                .join(format!("src/Catalogs/Editable/{collection_dir}/Main.xml"));
            if let Some(template_type) = template_type {
                for descriptor in [&fixture.descriptor, &child_descriptor] {
                    let text = String::from_utf8(fs::read(descriptor).unwrap())
                        .unwrap()
                        .replace(
                            "<TemplateType>SpreadsheetDocument</TemplateType>",
                            &format!("<TemplateType>{template_type}</TemplateType>"),
                        );
                    fs::write(descriptor, text).unwrap();
                }
            }
            fs::remove_dir_all(
                fixture
                    .root
                    .join(format!("src/Catalogs/Editable/{collection_dir}/Main")),
            )
            .unwrap();
            let request = fixture.typed_edit(vec![MetaEditOperation::update(
                collection,
                None,
                vec![MetaElementUpdateInput {
                    name: "Main".into(),
                    comment: Some("retained".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()]);

            let failure = match MetadataOperations::prepare_mutation(
                &request,
                &fixture.context,
                &cancellation,
            ) {
                Ok(_) => panic!("{label} accepted without its mandatory payload"),
                Err(failure) => failure,
            };

            let diagnostic = &failure.diagnostics[0];
            assert!(
                diagnostic.message.contains(&format!(
                    "Catalog.Editable.{}.Main",
                    if collection == MetaCollection::Forms {
                        "Form"
                    } else {
                        "Template"
                    }
                )),
                "{label}: {diagnostic:?}"
            );
            assert_eq!(
                diagnostic.field.as_deref(),
                Some("resources.child.payload"),
                "{label}"
            );
            assert!(
                !diagnostic
                    .message
                    .contains(&fixture.root.to_string_lossy().into_owned()),
                "{label}: {diagnostic:?}"
            );
        }
    }

    #[test]
    fn retained_form_payload_rejects_wrong_namespace_and_version_without_path_leakage() {
        for (label, payload) in [
            (
                "namespace",
                br#"<Form xmlns="urn:not-logform" version="2.20"/>"#.as_slice(),
            ),
            (
                "version",
                br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#.as_slice(),
            ),
        ] {
            let fixture = Fixture::new(&format!("retained-form-invalid-{label}"));
            let cancellation = CancellationToken::new();
            fixture.publish_form_add("Main");
            fs::write(
                fixture
                    .root
                    .join("src/Catalogs/Editable/Forms/Main/Ext/Form.xml"),
                payload,
            )
            .unwrap();
            let request = fixture.typed_edit(vec![MetaEditOperation::update(
                MetaCollection::Forms,
                None,
                vec![MetaElementUpdateInput {
                    name: "Main".into(),
                    comment: Some("retained".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()]);
            let prepared =
                MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                    .unwrap();

            let validation = MetadataOperations::validate(
                prepared.validation_subject(),
                &fixture.context,
                &cancellation,
            );

            assert_eq!(validation.status, MetaValidationStatus::Failed, "{label}");
            let diagnostic = validation
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic
                        .metadata_path
                        .as_ref()
                        .is_some_and(|path| path.as_str() == "Catalog.Editable.Form.Main")
                })
                .unwrap();
            assert!(!diagnostic
                .message
                .contains(&fixture.root.to_string_lossy().into_owned()));
            assert!(
                diagnostic.field.as_deref().is_some_and(
                    |field| field.starts_with("resources[") && field.ends_with("].bytes")
                )
            );
        }
    }

    #[test]
    fn retained_optional_child_module_is_validated_by_its_declared_role() {
        let fixture = Fixture::new("retained-form-invalid-module");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Main");
        write_form_module(
            &fixture.root.join("src/Catalogs/Editable/Forms/Main"),
            &[0xff, 0x00, 0x80],
        );
        let request = fixture.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Main".into(),
                comment: Some("retained".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();

        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );

        assert_eq!(validation.status, MetaValidationStatus::Failed);
        assert!(validation.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .metadata_path
                .as_ref()
                .is_some_and(|path| path.as_str() == "Catalog.Editable.Form.Main")
                && diagnostic.message.contains("not UTF-8")
        }));
    }

    #[test]
    fn retained_template_profiles_preserve_and_validate_xml_text_and_binary_payloads() {
        let cases = [
            (
                "spreadsheet",
                "SpreadsheetDocument",
                "Template.xml",
                crate::infrastructure::native_operations::mxl::empty_spreadsheet_document_xml()
                    .into_bytes(),
                MetadataTemplateType::SpreadsheetDocument,
            ),
            (
                "schema",
                "DataCompositionSchema",
                "Template.xml",
                br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#
                    .to_vec(),
                MetadataTemplateType::DataCompositionSchema,
            ),
            (
                "text",
                "TextDocument",
                "Template.txt",
                "Сохранённый текст".as_bytes().to_vec(),
                MetadataTemplateType::TextDocument,
            ),
            (
                "binary",
                "BinaryData",
                "Template.bin",
                vec![0xff, 0x00, 0x80, 0x01],
                MetadataTemplateType::BinaryData,
            ),
        ];
        for (label, template_type, file_name, payload, expected_type) in cases {
            let fixture = Fixture::new(&format!("retained-template-profile-{label}"));
            let cancellation = CancellationToken::new();
            let add = fixture.typed_edit(vec![MetaEditOperation::add(
                MetaCollection::Templates,
                None,
                vec![MetaElementInput::named("Main")],
            )
            .unwrap()]);
            MetadataOperations::prepare_mutation(&add, &fixture.context, &cancellation)
                .unwrap()
                .publish(&cancellation)
                .unwrap();
            let child_descriptor = fixture
                .root
                .join("src/Catalogs/Editable/Templates/Main.xml");
            if template_type != "SpreadsheetDocument" {
                for descriptor in [&fixture.descriptor, &child_descriptor] {
                    let text = String::from_utf8(fs::read(descriptor).unwrap())
                        .unwrap()
                        .replace(
                            "<TemplateType>SpreadsheetDocument</TemplateType>",
                            &format!("<TemplateType>{template_type}</TemplateType>"),
                        );
                    fs::write(descriptor, text).unwrap();
                }
            }
            let ext = fixture
                .root
                .join("src/Catalogs/Editable/Templates/Main/Ext");
            let original = ext.join("Template.xml");
            let payload_path = ext.join(file_name);
            if payload_path != original {
                fs::remove_file(&original).unwrap();
            }
            fs::write(&payload_path, &payload).unwrap();
            let request = fixture.typed_edit(vec![MetaEditOperation::update(
                MetaCollection::Templates,
                None,
                vec![MetaElementUpdateInput {
                    name: "Main".into(),
                    comment: Some("retained".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()]);
            let prepared =
                MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                    .unwrap();
            assert!(prepared
                .validation_subject()
                .resources
                .iter()
                .any(|resource| {
                    matches!(
                        resource.role,
                        MetadataResourceRole::ChildResource {
                            kind: MetadataChildResourceKind::TemplateContent { template_type, .. },
                            ..
                        } if template_type == expected_type
                    ) && resource.bytes == payload
                }));
            let validation = MetadataOperations::validate(
                prepared.validation_subject(),
                &fixture.context,
                &cancellation,
            );
            assert_eq!(validation.status, MetaValidationStatus::Passed, "{label}");

            prepared.publish(&cancellation).unwrap();

            assert_eq!(fs::read(payload_path).unwrap(), payload, "{label}");
        }
    }

    #[test]
    fn retained_optional_form_module_and_html_companion_tree_have_closed_valid_footprints() {
        let cancellation = CancellationToken::new();

        let form = Fixture::new("valid-optional-form-module-footprint");
        form.publish_form_add("Main");
        write_form_module(
            &form.root.join("src/Catalogs/Editable/Forms/Main"),
            "Процедура ПриОткрытии()\nКонецПроцедуры".as_bytes(),
        );
        let form_request = form.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Main".into(),
                comment: Some("retained".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&form_request, &form.context, &cancellation)
                .unwrap();
        assert_eq!(
            prepared.validation_subject().child_footprints[0].profile,
            MetadataChildProfile::Form
        );
        // `Ext/Form` holds the module, so the closure of the payload files
        // carries it as an unnamed nested directory.
        assert_eq!(
            prepared.validation_subject().child_footprints[0].directories,
            vec![
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
                MetadataChildDirectoryKind::Nested,
            ]
        );
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| matches!(
                resource.role,
                MetadataResourceRole::ChildResource {
                    kind: MetadataChildResourceKind::FormContent,
                    ordinal: 0,
                    ..
                }
            )));
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| matches!(
                resource.role,
                MetadataResourceRole::ChildResource {
                    kind: MetadataChildResourceKind::Module,
                    ordinal: 1,
                    ..
                }
            )));
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &form.context,
            &cancellation,
        );
        assert_eq!(validation.status, MetaValidationStatus::Passed);

        let html = Fixture::new("valid-html-companion-footprint");
        let add = html.typed_edit(vec![MetaEditOperation::add(
            MetaCollection::Templates,
            None,
            vec![MetaElementInput::named("Main")],
        )
        .unwrap()]);
        MetadataOperations::prepare_mutation(&add, &html.context, &cancellation)
            .unwrap()
            .publish(&cancellation)
            .unwrap();
        let child_descriptor = html.root.join("src/Catalogs/Editable/Templates/Main.xml");
        for descriptor in [&html.descriptor, &child_descriptor] {
            let text = String::from_utf8(fs::read(descriptor).unwrap())
                .unwrap()
                .replace(
                    "<TemplateType>SpreadsheetDocument</TemplateType>",
                    "<TemplateType>HTMLDocument</TemplateType>",
                );
            fs::write(descriptor, text).unwrap();
        }
        let ext = html.root.join("src/Catalogs/Editable/Templates/Main/Ext");
        fs::write(
            ext.join("Template.xml"),
            br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#,
        )
        .unwrap();
        fs::create_dir_all(ext.join("Template")).unwrap();
        fs::write(
            ext.join("Template/ru.html"),
            br#"<html><head><meta charset="utf-8"/></head><body/></html>"#,
        )
        .unwrap();
        let html_request = html.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Templates,
            None,
            vec![MetaElementUpdateInput {
                name: "Main".into(),
                comment: Some("retained".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);
        let prepared =
            MetadataOperations::prepare_mutation(&html_request, &html.context, &cancellation)
                .unwrap();
        assert_eq!(
            prepared.validation_subject().child_footprints[0].profile,
            MetadataChildProfile::Template(MetadataTemplateType::HtmlDocument)
        );
        assert_eq!(
            prepared.validation_subject().child_footprints[0].directories,
            vec![
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
                MetadataChildDirectoryKind::HtmlPages,
            ]
        );
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| matches!(
                resource.role,
                MetadataResourceRole::ChildResource {
                    kind: MetadataChildResourceKind::TemplateContent {
                        part: MetadataTemplateResourcePart::Primary,
                        ..
                    },
                    ordinal: 0,
                    ..
                }
            )));
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| matches!(
                resource.role,
                MetadataResourceRole::ChildResource {
                    kind: MetadataChildResourceKind::TemplateContent {
                        part: MetadataTemplateResourcePart::HtmlPage,
                        ..
                    },
                    ordinal: 1,
                    ..
                }
            )));
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &html.context,
            &cancellation,
        );
        assert_eq!(
            validation.status,
            MetaValidationStatus::Passed,
            "{:?}",
            validation.diagnostics
        );
    }

    #[test]
    fn child_topology_failures_report_logical_identity_without_provider_paths() {
        let fixture = Fixture::new("child-topology-sanitized-link");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Main");
        let ext = fixture.root.join("src/Catalogs/Editable/Forms/Main/Ext");
        let external = fixture.root.join("provider-secret.txt");
        fs::write(&external, b"secret").unwrap();
        let Some(link_result) =
            crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
                &external,
                ext.join("LinkedResource"),
            )
        else {
            return;
        };
        if link_result.is_err() {
            return;
        }
        let request = fixture.typed_edit(vec![MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Main".into(),
                comment: Some("touch".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap()]);

        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("linked topology unexpectedly prepared"),
                Err(failure) => failure,
            };

        let diagnostic = &failure.diagnostics[0];
        assert!(
            diagnostic.message.contains("Catalog.Editable.Form.Main"),
            "{diagnostic:?}"
        );
        assert!(
            !diagnostic
                .message
                .contains(&fixture.root.to_string_lossy().into_owned()),
            "{diagnostic:?}"
        );
        assert_eq!(
            diagnostic.metadata_path.as_ref(),
            Some(
                &MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Catalog.Editable.Form.Main"
                )
                .unwrap()
            )
        );
        assert_eq!(
            diagnostic.field.as_deref(),
            Some("resources.child.topology")
        );
    }

    #[test]
    fn retained_child_payload_rejects_unexpected_empty_directory_subtrees() {
        for (label, collection, relative) in [
            ("form", MetaCollection::Forms, "Forms/Main/Ext/Unexpected"),
            (
                "template",
                MetaCollection::Templates,
                "Templates/Main/Ext/Unexpected",
            ),
        ] {
            let fixture = Fixture::new(&format!("unexpected-empty-{label}-subtree"));
            let cancellation = CancellationToken::new();
            let add = fixture.typed_edit(vec![MetaEditOperation::add(
                collection,
                None,
                vec![MetaElementInput::named("Main")],
            )
            .unwrap()]);
            MetadataOperations::prepare_mutation(&add, &fixture.context, &cancellation)
                .unwrap()
                .publish(&cancellation)
                .unwrap();
            fs::create_dir_all(fixture.root.join("src/Catalogs/Editable").join(relative)).unwrap();
            let request = fixture.typed_edit(vec![MetaEditOperation::update(
                collection,
                None,
                vec![MetaElementUpdateInput {
                    name: "Main".into(),
                    comment: Some("retained".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()]);

            let failure = match MetadataOperations::prepare_mutation(
                &request,
                &fixture.context,
                &cancellation,
            ) {
                Ok(_) => panic!("{label} accepted an unexpected empty subtree"),
                Err(failure) => failure,
            };

            let diagnostic = &failure.diagnostics[0];
            assert_eq!(
                diagnostic.field.as_deref(),
                Some("resources.child.topology"),
                "{label}"
            );
            assert!(
                !diagnostic
                    .message
                    .contains(&fixture.root.to_string_lossy().into_owned()),
                "{label}: {diagnostic:?}"
            );
        }
    }

    #[test]
    fn child_collection_snapshot_failure_is_provider_neutral() {
        let fixture = Fixture::new("child-collection-snapshot-sanitized");
        let cancellation = CancellationToken::new();
        let external = fixture.root.join("provider-private-forms");
        fs::create_dir_all(&external).unwrap();
        let collection = fixture.root.join("src/Catalogs/Editable/Forms");
        let Some(link_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &external,
                &collection,
            )
        else {
            return;
        };
        if link_result.is_err() {
            return;
        }
        let request = fixture.typed_edit(vec![MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput::named("Main")],
        )
        .unwrap()]);

        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("symlinked child collection unexpectedly prepared"),
                Err(failure) => failure,
            };

        let diagnostic = &failure.diagnostics[0];
        assert!(diagnostic.message.contains("Catalog.Editable.forms"));
        assert!(!diagnostic
            .message
            .contains(&fixture.root.to_string_lossy().into_owned()));
        assert_eq!(diagnostic.metadata_path.as_ref(), Some(&fixture.target));
        assert_eq!(
            diagnostic.field.as_deref(),
            Some("resources.child.collection")
        );
    }

    #[test]
    fn typed_relation_target_is_resolved_and_guarded_against_linked_drift() {
        let fixture = Fixture::new("relation-dependency-drift");
        let cancellation = CancellationToken::new();
        MetadataOperations::prepare_mutation(
            &MetadataRequest::Add(MetaAddRequest {
                source_set: "main".into(),
                kind: MetadataKind::Catalog,
                name: "Parent".into(),
                operations: Vec::new(),
                dry_run: false,
            }),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let parent_address =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Parent").unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: fixture.target.clone(),
            operations: vec![MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Add,
                vec![MetadataReference {
                    metadata_path: parent_address.clone(),
                }],
            )
            .unwrap()],
            dry_run: false,
        });
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        assert!(prepared.validation_subject().resources.iter().any(|resource| {
            matches!(&resource.role, MetadataResourceRole::Dependency { target } if target == &parent_address)
        }));
        let parent_path = fixture.root.join("src/Catalogs/Parent.xml");
        let mut external = fs::read(&parent_path).unwrap();
        external.extend_from_slice(b"\n");
        fs::write(&parent_path, &external).unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("linked target drift unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(&parent_path).unwrap(), external);
    }

    #[test]
    fn typed_relation_dependencies_cover_unchanged_and_new_final_targets() {
        let fixture = Fixture::new("relation-complete-final-graph");
        let cancellation = CancellationToken::new();
        let unchanged = fixture.add_object(MetadataKind::Catalog, "ParentA");
        let added = fixture.add_object(MetadataKind::Catalog, "ParentB");
        MetadataOperations::prepare_mutation(
            &fixture.owners_request(RelationEditMode::Add, vec![unchanged.clone()]),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();

        let prepared = MetadataOperations::prepare_mutation(
            &fixture.owners_request(RelationEditMode::Add, vec![added.clone()]),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let dependencies = prepared
            .validation_subject()
            .resources
            .iter()
            .filter_map(|resource| match &resource.role {
                MetadataResourceRole::Dependency { target } => Some(target.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(dependencies.contains(&unchanged.as_str()));
        assert!(dependencies.contains(&added.as_str()));
    }

    #[test]
    fn typed_relation_missing_unchanged_final_target_blocks_unrelated_edit_prepare() {
        let fixture = Fixture::new("relation-unchanged-missing");
        let cancellation = CancellationToken::new();
        let unchanged = fixture.add_object(MetadataKind::Catalog, "Parent");
        MetadataOperations::prepare_mutation(
            &fixture.owners_request(RelationEditMode::Add, vec![unchanged]),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        fs::remove_file(fixture.root.join("src/Catalogs/Parent.xml")).unwrap();

        let failure = match MetadataOperations::prepare_mutation(
            &fixture.edit("Comment", MetaPropertyValue::String("unrelated".into())),
            &fixture.context,
            &cancellation,
        ) {
            Ok(_) => panic!("missing unchanged relation target unexpectedly prepared"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::TargetNotFound
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("relations.owners[0]")
        );
    }

    #[test]
    fn typed_relation_unchanged_final_target_drift_aborts_unrelated_publish() {
        let fixture = Fixture::new("relation-unchanged-drift");
        let cancellation = CancellationToken::new();
        let unchanged = fixture.add_object(MetadataKind::Catalog, "Parent");
        MetadataOperations::prepare_mutation(
            &fixture.owners_request(RelationEditMode::Add, vec![unchanged]),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let prepared = MetadataOperations::prepare_mutation(
            &fixture.edit("Comment", MetaPropertyValue::String("unrelated".into())),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let parent_path = fixture.root.join("src/Catalogs/Parent.xml");
        let mut external = fs::read(&parent_path).unwrap();
        external.extend_from_slice(b"\n");
        fs::write(&parent_path, &external).unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("unchanged relation target drift unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(parent_path).unwrap(), external);
    }

    #[test]
    fn typed_relation_remove_and_replace_guard_only_their_complete_final_graph() {
        let cancellation = CancellationToken::new();

        let remove_fixture = Fixture::new("relation-final-remove");
        let removed = remove_fixture.add_object(MetadataKind::Catalog, "Removed");
        let retained = remove_fixture.add_object(MetadataKind::Catalog, "Retained");
        MetadataOperations::prepare_mutation(
            &remove_fixture.owners_request(
                RelationEditMode::Add,
                vec![removed.clone(), retained.clone()],
            ),
            &remove_fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let prepared = MetadataOperations::prepare_mutation(
            &remove_fixture.owners_request(RelationEditMode::Remove, vec![removed.clone()]),
            &remove_fixture.context,
            &cancellation,
        )
        .unwrap();
        let dependencies = prepared
            .validation_subject()
            .resources
            .iter()
            .filter_map(|resource| match &resource.role {
                MetadataResourceRole::Dependency { target } => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!dependencies.contains(&&removed));
        assert!(dependencies.contains(&&retained));

        let replace_fixture = Fixture::new("relation-final-replace");
        let replaced = replace_fixture.add_object(MetadataKind::Catalog, "Replaced");
        let replacement = replace_fixture.add_object(MetadataKind::Catalog, "Replacement");
        MetadataOperations::prepare_mutation(
            &replace_fixture.owners_request(RelationEditMode::Add, vec![replaced.clone()]),
            &replace_fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let prepared = MetadataOperations::prepare_mutation(
            &replace_fixture.owners_request(RelationEditMode::Replace, vec![replacement.clone()]),
            &replace_fixture.context,
            &cancellation,
        )
        .unwrap();
        let dependencies = prepared
            .validation_subject()
            .resources
            .iter()
            .filter_map(|resource| match &resource.role {
                MetadataResourceRole::Dependency { target } => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!dependencies.contains(&&replaced));
        assert!(dependencies.contains(&&replacement));
    }

    #[test]
    fn typed_relation_final_graph_deduplicates_remove_then_add_guards() {
        let fixture = Fixture::new("relation-final-dedup");
        let cancellation = CancellationToken::new();
        let parent = fixture.add_object(MetadataKind::Catalog, "Parent");
        MetadataOperations::prepare_mutation(
            &fixture.owners_request(RelationEditMode::Add, vec![parent.clone()]),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: fixture.target.clone(),
            operations: vec![
                MetaEditOperation::edit_relations(
                    MetaRelation::Owners,
                    RelationEditMode::Remove,
                    vec![MetadataReference {
                        metadata_path: parent.clone(),
                    }],
                )
                .unwrap(),
                MetaEditOperation::edit_relations(
                    MetaRelation::Owners,
                    RelationEditMode::Add,
                    vec![MetadataReference {
                        metadata_path: parent.clone(),
                    }],
                )
                .unwrap(),
            ],
            dry_run: false,
        });

        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let count = prepared
            .validation_subject()
            .resources
            .iter()
            .filter(|resource| {
                matches!(&resource.role, MetadataResourceRole::Dependency { target } if target == &parent)
            })
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn typed_relation_missing_target_fails_prepare_with_the_exact_target_field() {
        let fixture = Fixture::new("relation-target-missing");
        let cancellation = CancellationToken::new();
        let missing =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Missing").unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: fixture.target.clone(),
            operations: vec![MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Add,
                vec![MetadataReference {
                    metadata_path: missing,
                }],
            )
            .unwrap()],
            dry_run: false,
        });

        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("missing relation target unexpectedly prepared"),
                Err(failure) => failure,
            };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::TargetNotFound
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[0].targets[0]")
        );
    }

    #[test]
    fn typed_edit_third_operation_failure_preserves_every_exact_preimage() {
        let fixture = Fixture::new("third-operation-failure");
        let cancellation = CancellationToken::new();
        let descriptor_before = fs::read(&fixture.descriptor).unwrap();
        let owner_before = fs::read(&fixture.owner).unwrap();
        let request = MetadataRequest::Edit(MetaEditRequest {
            source_set: "main".into(),
            metadata_path: fixture.target.clone(),
            operations: vec![
                MetaEditOperation::add(
                    MetaCollection::TabularSections,
                    None,
                    vec![MetaElementInput::named("Lines")],
                )
                .unwrap(),
                MetaEditOperation::add(
                    MetaCollection::Attributes,
                    Some(MetaScope {
                        tabular_section: "Lines".into(),
                    }),
                    vec![MetaElementInput::named("Value")],
                )
                .unwrap(),
                MetaEditOperation::update(
                    MetaCollection::Attributes,
                    None,
                    vec![MetaElementUpdateInput {
                        name: "Missing".into(),
                        synonym: Some("must not upsert".into()),
                        ..MetaElementUpdateInput::default()
                    }],
                )
                .unwrap(),
            ],
            dry_run: false,
        });

        let failure =
            match MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation) {
                Ok(_) => panic!("failing third typed operation unexpectedly prepared"),
                Err(failure) => failure,
            };

        assert_eq!(failure.diagnostics[0].operation_index, Some(2));
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::TargetNotFound
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), descriptor_before);
        assert_eq!(fs::read(&fixture.owner).unwrap(), owner_before);
    }

    #[test]
    fn typed_edit_cancellation_never_changes_descriptor_bytes() {
        let fixture = Fixture::new("cancellation");
        let before = fs::read(&fixture.descriptor).unwrap();
        let request = fixture.edit("Comment", MetaPropertyValue::String("cancelled".into()));

        let cancelled_before_prepare = CancellationToken::new();
        cancelled_before_prepare.cancel();
        let failure = match MetadataOperations::prepare_mutation(
            &request,
            &fixture.context,
            &cancelled_before_prepare,
        ) {
            Ok(_) => panic!("cancelled metadata edit unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), before);

        let publish_cancellation = CancellationToken::new();
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &publish_cancellation)
                .unwrap();
        publish_cancellation.cancel();
        let failure = match prepared.publish(&publish_cancellation) {
            Ok(_) => panic!("cancelled metadata edit unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), before);
    }

    #[test]
    fn typed_edit_reports_rollback_failed_without_overwriting_external_bytes() {
        let fixture = Fixture::new("rollback-failed");
        let cancellation = CancellationToken::new();
        let request = fixture.edit("Comment", MetaPropertyValue::String("changed".into()));
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let external = b"external bytes retained after rollback conflict".to_vec();
        let hook_bytes = external.clone();

        let failure = match with_before_rollback_mutation_hook(
            move |path| fs::write(path, &hook_bytes).unwrap(),
            || {
                with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                    prepared.publish(&cancellation)
                })
            },
        ) {
            Ok(_) => panic!("metadata edit rollback-conflict failpoint unexpectedly published"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::RollbackFailed
        );
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), external);
        assert!(!failure.diagnostics[0]
            .message
            .contains(fixture.root.to_string_lossy().as_ref()));
    }

    #[test]
    fn meta_remove_preview_resolves_the_logical_request_through_the_typed_mutation_path() {
        let fixture = Fixture::new("remove-logical-preview");
        let cancellation = CancellationToken::new();
        let request = MetadataRequest::Remove(MetaRemoveRequest {
            source_set: "main".into(),
            metadata_path: fixture.target.clone(),
            dry_run: true,
            force: false,
            confirm: false,
        });
        let descriptor_before = fs::read(&fixture.descriptor).unwrap();
        let owner_before = fs::read(&fixture.owner).unwrap();

        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .expect("typed logical remove must prepare a guarded preview");

        assert_eq!(prepared.preview().metadata_path, fixture.target);
        assert!(prepared.preview().changed);
        assert!(!prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| matches!(resource.role, MetadataResourceRole::Descriptor)));
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), descriptor_before);
        assert_eq!(fs::read(&fixture.owner).unwrap(), owner_before);
    }

    #[test]
    fn meta_remove_plans_and_deletes_the_complete_logical_resource_footprint() {
        let fixture = Fixture::new("remove-resource-footprint");
        fixture.publish_form_add("Main");
        let cancellation = CancellationToken::new();
        let request = fixture.remove_request(fixture.target.clone(), false, false, false);

        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        assert!(prepared.preview().publication_plan.iter().any(|entry| {
            entry.action == MetaPublicationAction::Remove
                && entry.resource == MetaPublicationResource::Descriptor
                && entry.metadata_path.as_ref() == Some(&fixture.target)
        }));
        assert!(prepared.preview().publication_plan.iter().any(|entry| {
            entry.action == MetaPublicationAction::Remove
                && entry.resource == MetaPublicationResource::Form
                && entry
                    .metadata_path
                    .as_ref()
                    .is_some_and(|target| target.as_str() == "Catalog.Editable.Form.Main")
        }));
        assert!(prepared.preview().publication_plan.iter().any(|entry| {
            entry.action == MetaPublicationAction::Update
                && entry.resource == MetaPublicationResource::Registration
        }));
        let public_preview = serde_json::to_string(prepared.preview()).unwrap();
        assert!(!public_preview.contains(fixture.root.to_string_lossy().as_ref()));
        assert!(!public_preview.contains("PlatformXml"));

        prepared.publish(&cancellation).unwrap();

        assert!(!fixture.root.join("src/Catalogs/Editable.xml").exists());
        assert!(!fixture.root.join("src/Catalogs/Editable").exists());
        assert!(!fixture.root.join("src/Catalogs").exists());
    }

    #[test]
    fn meta_remove_force_apply_validates_only_changed_post_images_and_guards_reference_evidence() {
        let fixture = Fixture::new("remove-forced-reference");
        let cancellation = CancellationToken::new();
        let referenced = fixture.add_object(MetadataKind::Catalog, "Referenced");
        MetadataOperations::prepare_mutation(
            &fixture.owners_request(RelationEditMode::Add, vec![referenced.clone()]),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let referencing_descriptor = fixture.descriptor.clone();
        let referenced_descriptor = fixture.root.join("src/Catalogs/Referenced.xml");

        let blocked = match MetadataOperations::prepare_mutation(
            &fixture.remove_request(referenced.clone(), true, false, false),
            &fixture.context,
            &cancellation,
        ) {
            Ok(_) => panic!("unforced referenced object unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert_eq!(
            blocked.diagnostics[0].code,
            MetaDiagnosticCode::ReferenceConflict
        );

        let request = fixture.remove_request(referenced.clone(), false, true, true);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let validation = MetadataOperations::validate(
            prepared.validation_subject(),
            &fixture.context,
            &cancellation,
        );
        assert_eq!(validation.status, MetaValidationStatus::Passed);
        assert!(!prepared.validation_subject().resources.iter().any(|resource| {
            matches!(&resource.role, MetadataResourceRole::Dependency { target } if target == &fixture.target)
        }));

        let mut external_reference = fs::read(&referencing_descriptor).unwrap();
        external_reference.extend_from_slice(b"\n");
        fs::write(&referencing_descriptor, &external_reference).unwrap();
        let conflict = match prepared.publish(&cancellation) {
            Ok(_) => panic!("reference evidence drift unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            conflict.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert!(referenced_descriptor.is_file());

        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        assert_eq!(
            MetadataOperations::validate(
                prepared.validation_subject(),
                &fixture.context,
                &cancellation,
            )
            .status,
            MetaValidationStatus::Passed
        );
        prepared.publish(&cancellation).unwrap();

        assert!(!referenced_descriptor.exists());
        assert!(
            String::from_utf8(fs::read(&referencing_descriptor).unwrap())
                .unwrap()
                .contains("Catalog.Referenced")
        );
        assert!(!String::from_utf8(fs::read(&fixture.owner).unwrap())
            .unwrap()
            .contains("<Catalog>Referenced</Catalog>"));
    }

    #[test]
    fn meta_remove_reference_diagnostics_expose_deterministic_logical_referrers_only() {
        let fixture = Fixture::new("remove-logical-reference-identities");
        let cancellation = CancellationToken::new();
        let referenced = fixture.add_object(MetadataKind::Catalog, "Referenced");
        MetadataOperations::prepare_mutation(
            &fixture.owners_request(RelationEditMode::Add, vec![referenced.clone()]),
            &fixture.context,
            &cancellation,
        )
        .unwrap()
        .publish(&cancellation)
        .unwrap();
        let second = fixture.add_object(MetadataKind::Catalog, "Second");
        let second_descriptor = fixture.root.join("src/Catalogs/Second.xml");
        let second_image = String::from_utf8(fs::read(&second_descriptor).unwrap())
            .unwrap()
            .replacen(
                "</Properties>",
                "<BasedOn>Catalog.Referenced</BasedOn></Properties>",
                1,
            );
        fs::write(&second_descriptor, second_image).unwrap();
        let second_module = fixture
            .root
            .join("src/Catalogs/Second/Ext/ObjectModule.bsl");
        fs::create_dir_all(second_module.parent().unwrap()).unwrap();
        fs::write(
            &second_module,
            "Procedure UseReference()\n    Value = Catalogs.Referenced;\nEndProcedure\n",
        )
        .unwrap();

        let blocked = match MetadataOperations::prepare_mutation(
            &fixture.remove_request(referenced.clone(), true, false, false),
            &fixture.context,
            &cancellation,
        ) {
            Ok(_) => panic!("referenced object unexpectedly prepared without force"),
            Err(failure) => failure,
        };
        let blocked_referrers = blocked
            .diagnostics
            .iter()
            .map(|diagnostic| {
                assert_eq!(diagnostic.code, MetaDiagnosticCode::ReferenceConflict);
                diagnostic
                    .metadata_path
                    .as_ref()
                    .expect("reference diagnostic has a logical referrer")
                    .as_str()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            blocked_referrers,
            vec![
                "Catalog.Editable",
                "Catalog.Second",
                "Catalog.Second.ObjectModule"
            ]
        );

        let forced = MetadataOperations::prepare_mutation(
            &fixture.remove_request(referenced, true, true, true),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let forced_referrers = forced
            .preview()
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == MetaDiagnosticCode::ReferenceConflict)
            .map(|diagnostic| {
                diagnostic
                    .metadata_path
                    .as_ref()
                    .expect("forced reference warning has a logical referrer")
                    .as_str()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(forced_referrers, blocked_referrers);
        let diagnostics =
            serde_json::to_string(&(blocked.diagnostics, forced.preview().diagnostics.clone()))
                .unwrap();
        for forbidden in [
            fixture.root.to_string_lossy().as_ref(),
            "Catalogs/",
            ".xml",
            ".bsl",
        ] {
            assert!(
                !diagnostics.contains(forbidden),
                "{forbidden}: {diagnostics}"
            );
        }
        assert_eq!(second.as_str(), "Catalog.Second");
    }

    #[test]
    fn meta_remove_preview_and_apply_clean_subsystems_and_validate_the_surviving_post_state() {
        let fixture = Fixture::new("remove-subsystem-cleanup");
        let cancellation = CancellationToken::new();
        let subsystem = fixture.root.join("src/Subsystems/Main.xml");
        fs::create_dir_all(subsystem.parent().unwrap()).unwrap();
        let subsystem_before = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n",
            "<Subsystem uuid=\"11111111-1111-1111-1111-111111111111\">\n",
            "<Properties><Name>Main</Name></Properties>\n",
            "<Content><Item>Catalog.Editable</Item><Item>Catalog.Keep</Item></Content>\n",
            "</Subsystem>\n",
            "</MetaDataObject>"
        );
        fs::write(&subsystem, subsystem_before).unwrap();
        let request = fixture.remove_request(fixture.target.clone(), false, false, false);

        let prepared =
            MetadataOperations::prepare_mutation(&request, &fixture.context, &cancellation)
                .unwrap();
        let dependency = prepared
            .validation_subject()
            .resources
            .iter()
            .find(|resource| {
                matches!(&resource.role, MetadataResourceRole::Dependency { target } if target.as_str() == "Subsystem.Main")
            })
            .expect("subsystem post-image must be a logical validation dependency");
        let dependency_text = String::from_utf8(dependency.bytes.clone()).unwrap();
        assert!(!dependency_text.contains("Catalog.Editable"));
        assert!(dependency_text.contains("Catalog.Keep"));
        assert!(!dependency.bytes.starts_with(b"\xef\xbb\xbf"));
        assert!(!dependency.bytes.ends_with(b"\n"));
        assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before.as_bytes());
        assert_eq!(
            MetadataOperations::validate(
                prepared.validation_subject(),
                &fixture.context,
                &cancellation,
            )
            .status,
            MetaValidationStatus::Passed
        );

        prepared.publish(&cancellation).unwrap();

        let subsystem_after = fs::read(&subsystem).unwrap();
        assert!(!subsystem_after.starts_with(b"\xef\xbb\xbf"));
        assert!(!subsystem_after.ends_with(b"\n"));
        let subsystem_after = String::from_utf8(subsystem_after).unwrap();
        assert!(!subsystem_after.contains("Catalog.Editable"));
        assert!(subsystem_after.contains("Catalog.Keep"));
        assert!(!fixture.descriptor.exists());
    }

    #[test]
    fn meta_remove_reports_logical_missing_format_support_and_cancellation_diagnostics() {
        let cancellation = CancellationToken::new();

        let missing_source = Fixture::new("remove-missing-source");
        let mut request =
            missing_source.remove_request(missing_source.target.clone(), true, false, false);
        let MetadataRequest::Remove(request) = &mut request else {
            unreachable!()
        };
        request.source_set = "missing".into();
        let request = MetadataRequest::Remove(request.clone());
        let failure = match MetadataOperations::prepare_mutation(
            &request,
            &missing_source.context,
            &cancellation,
        ) {
            Ok(_) => panic!("unknown source set unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::TargetNotFound
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("metadataPath")
        );
        assert!(!failure.diagnostics[0]
            .message
            .contains(missing_source.root.to_string_lossy().as_ref()));

        let missing_target = Fixture::new("remove-missing-target");
        let missing =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Missing").unwrap();
        let failure = match MetadataOperations::prepare_mutation(
            &missing_target.remove_request(missing, true, false, false),
            &missing_target.context,
            &cancellation,
        ) {
            Ok(_) => panic!("missing metadata object unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::TargetNotFound
        );

        let unsupported = Fixture::new("remove-format-gate");
        let descriptor = String::from_utf8(fs::read(&unsupported.descriptor).unwrap())
            .unwrap()
            .replacen("version=\"2.20\"", "version=\"2.19\"", 1);
        fs::write(&unsupported.descriptor, descriptor).unwrap();
        let failure = match MetadataOperations::prepare_mutation(
            &unsupported.remove_request(unsupported.target.clone(), true, false, false),
            &unsupported.context,
            &cancellation,
        ) {
            Ok(_) => panic!("unsupported descriptor format unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::CapabilityUnavailable,
            "{failure:?}"
        );

        let locked = Fixture::new("remove-support-lock");
        fs::create_dir_all(locked.root.join("src/Ext")).unwrap();
        fs::write(
            locked.root.join("src/Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",0,0,0}"
            ),
        )
        .unwrap();
        let failure = match MetadataOperations::prepare_mutation(
            &locked.remove_request(locked.target.clone(), true, false, false),
            &locked.context,
            &cancellation,
        ) {
            Ok(_) => panic!("support-locked metadata object unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::SupportLocked
        );

        let cancelled = Fixture::new("remove-cancelled-prepare");
        let cancelled_token = CancellationToken::new();
        cancelled_token.cancel();
        let failure = match MetadataOperations::prepare_mutation(
            &cancelled.remove_request(cancelled.target.clone(), false, false, false),
            &cancelled.context,
            &cancelled_token,
        ) {
            Ok(_) => panic!("cancelled metadata removal unexpectedly prepared"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert!(cancelled.descriptor.exists());
    }

    #[test]
    fn meta_remove_rejects_a_read_capable_source_without_remove_capability() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-remove-read-only-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(root.join("erf")).unwrap();
        fs::write(
            root.join("erf/ReadOnly.xml"),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
                "<ExternalReport uuid=\"11111111-1111-1111-1111-111111111111\">",
                "<Properties><Name>ReadOnly</Name></Properties>",
                "</ExternalReport></MetaDataObject>\n"
            ),
        )
        .unwrap();
        fs::write(
            root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: EXTERNAL_REPORTS\n",
                "    path: erf\n"
            ),
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 0,
        };
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "ExternalReport.ReadOnly")
                .unwrap();
        let cancellation = CancellationToken::new();
        let request = MetadataRequest::Remove(MetaRemoveRequest {
            source_set: "main".into(),
            metadata_path: target,
            dry_run: true,
            force: false,
            confirm: false,
        });

        let failure = match MetadataOperations::prepare_mutation(&request, &context, &cancellation)
        {
            Ok(_) => panic!("read-only source unexpectedly provided remove capability"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::CapabilityUnavailable
        );
        assert!(root.join("erf/ReadOnly.xml").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn meta_remove_publish_honors_cancellation_and_owner_exact_preimages() {
        let cancellation_fixture = Fixture::new("remove-publish-cancelled");
        let cancellation = CancellationToken::new();
        let request = cancellation_fixture.remove_request(
            cancellation_fixture.target.clone(),
            false,
            false,
            false,
        );
        let prepared = MetadataOperations::prepare_mutation(
            &request,
            &cancellation_fixture.context,
            &cancellation,
        )
        .unwrap();
        let descriptor_before = fs::read(&cancellation_fixture.descriptor).unwrap();
        let owner_before = fs::read(&cancellation_fixture.owner).unwrap();
        cancellation.cancel();
        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("cancelled metadata removal unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(
            fs::read(&cancellation_fixture.descriptor).unwrap(),
            descriptor_before
        );
        assert_eq!(fs::read(&cancellation_fixture.owner).unwrap(), owner_before);

        let owner_fixture = Fixture::new("remove-owner-drift");
        let cancellation = CancellationToken::new();
        let request =
            owner_fixture.remove_request(owner_fixture.target.clone(), false, false, false);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &owner_fixture.context, &cancellation)
                .unwrap();
        let descriptor_before = fs::read(&owner_fixture.descriptor).unwrap();
        let mut external_owner = fs::read(&owner_fixture.owner).unwrap();
        external_owner.extend_from_slice(b"\n");
        fs::write(&owner_fixture.owner, &external_owner).unwrap();
        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("owner-preimage drift unexpectedly published"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(&owner_fixture.owner).unwrap(), external_owner);
        assert_eq!(
            fs::read(&owner_fixture.descriptor).unwrap(),
            descriptor_before
        );
    }

    #[test]
    fn meta_remove_rollback_restores_exact_bytes_and_reports_incomplete_rollback() {
        let rollback = Fixture::new("remove-rollback");
        rollback.add_object(MetadataKind::Catalog, "Sibling");
        let cancellation = CancellationToken::new();
        let request = rollback.remove_request(rollback.target.clone(), false, false, false);
        let descriptor_before = fs::read(&rollback.descriptor).unwrap();
        let owner_before = fs::read(&rollback.owner).unwrap();
        let prepared =
            MetadataOperations::prepare_mutation(&request, &rollback.context, &cancellation)
                .unwrap();
        let failure = match with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            prepared.publish(&cancellation)
        }) {
            Ok(_) => panic!("post-write failure unexpectedly published metadata removal"),
            Err(failure) => failure,
        };
        assert_ne!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::RollbackFailed
        );
        assert_eq!(fs::read(&rollback.descriptor).unwrap(), descriptor_before);
        assert_eq!(fs::read(&rollback.owner).unwrap(), owner_before);

        let incomplete = Fixture::new("remove-rollback-failed");
        incomplete.add_object(MetadataKind::Catalog, "Sibling");
        let cancellation = CancellationToken::new();
        let request = incomplete.remove_request(incomplete.target.clone(), false, false, false);
        let prepared =
            MetadataOperations::prepare_mutation(&request, &incomplete.context, &cancellation)
                .unwrap();
        let failure = match with_before_rollback_mutation_hook(
            |path| fs::write(path, b"external bytes during rollback").unwrap(),
            || {
                with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                    prepared.publish(&cancellation)
                })
            },
        ) {
            Ok(_) => panic!("rollback-conflict failpoint unexpectedly published removal"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::RollbackFailed
        );
        assert!(!failure.diagnostics[0]
            .message
            .contains(incomplete.root.to_string_lossy().as_ref()));
    }

    #[test]
    fn meta_remove_full_stack_failure_rolls_back_bytes_without_events_or_cache_invalidation() {
        let fixture = Fixture::new("remove-full-stack-rollback");
        let descriptor_before = fs::read(&fixture.descriptor).unwrap();
        let owner_before = fs::read(&fixture.owner).unwrap();
        let application = UnicaApplication::with_ports(Arc::new(FixedWorkspaceApplicationPorts {
            context: fixture.context.clone(),
            inner: InfrastructureApplicationPorts::new(),
        }));
        let args = Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!(fixture.target.as_str())),
            ("dryRun".to_string(), json!(false)),
        ]);

        let result = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            application.call_tool("unica.meta.remove", &args)
        })
        .expect("public typed meta.remove call");

        assert!(!result.ok);
        assert!(result.cache.events.is_empty());
        assert!(result.cache.invalidated.is_empty());
        assert!(result.cache.refreshed.is_empty());
        assert_eq!(fs::read(&fixture.descriptor).unwrap(), descriptor_before);
        assert_eq!(fs::read(&fixture.owner).unwrap(), owner_before);
    }

    #[test]
    fn meta_remove_denies_a_source_set_that_escapes_workspace_containment() {
        let fixture = Fixture::new("remove-containment");
        let outside = std::env::temp_dir().join(format!(
            "unica-meta-remove-outside-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::rename(fixture.root.join("src"), &outside).unwrap();
        let source_link = fixture.root.join("src");
        let Some(link_result) = create_dir_symlink_for_test(&outside, &source_link) else {
            fs::rename(&outside, &source_link).unwrap();
            return;
        };
        if link_result.is_err() {
            fs::rename(&outside, &source_link).unwrap();
            return;
        }
        let cancellation = CancellationToken::new();

        let failure = match MetadataOperations::prepare_mutation(
            &fixture.remove_request(fixture.target.clone(), true, false, false),
            &fixture.context,
            &cancellation,
        ) {
            Ok(_) => panic!("out-of-workspace source set unexpectedly prepared"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert!(!failure.diagnostics[0]
            .message
            .contains(outside.to_string_lossy().as_ref()));
        remove_dir_symlink_for_test(&source_link).unwrap();
        fs::rename(&outside, &source_link).unwrap();
    }
}
