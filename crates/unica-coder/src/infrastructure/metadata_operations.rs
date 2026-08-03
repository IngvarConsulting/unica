use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::ports::{
    MetaLocalInfo, MetaRelatedData, MetadataRead, MetadataValidationResult,
    MetadataValidationSubject, PreparedMetadataMutation,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{CodeIntelligenceContext, ProviderDeadline};
use crate::domain::metadata::{
    MetaCompleteness, MetaDiagnostic, MetaDiagnosticCode, MetaFreshness, MetaRelatedSection,
    MetaRelatedSections, MetaRelatedStatus,
};
use crate::domain::source_roots::ResolvedSourceRoot;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::meta::{
    prepare_meta_add, prepare_meta_remove, prepare_typed_edit, read_typed_meta_info,
    resolve_typed_edit_object, resolve_typed_metadata_object, MetadataValidator,
};
use crate::infrastructure::rlm_navigation::RlmNavigationAdapter;

pub(crate) struct MetadataOperations;

impl MetadataOperations {
    pub(crate) fn read_local(
        request: &MetaInfoRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
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
            &request.metadata_path,
            ProviderDeadline::new(std::time::Instant::now() + std::time::Duration::from_secs(5)),
            cancellation,
        )?;
        Ok(MetadataRead {
            local,
            validation_subject,
        })
    }

    pub(crate) fn read_related(
        request: &MetaInfoRequest,
        _local: &MetaLocalInfo,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> MetaRelatedData {
        let names = request
            .sections
            .iter()
            .map(|section| match section {
                crate::application::metadata::MetaInfoSection::Modules => "modules",
                crate::application::metadata::MetaInfoSection::Roles => "roles",
                crate::application::metadata::MetaInfoSection::Subscriptions => "subscriptions",
                crate::application::metadata::MetaInfoSection::FunctionalOptions => {
                    "functionalOptions"
                }
                crate::application::metadata::MetaInfoSection::PredefinedItems => "predefinedItems",
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        let source = match crate::infrastructure::source_roots::resolve_named_source_set(
            context,
            &request.source_set,
        ) {
            Ok(source) => source,
            Err(_) => return selected_unavailable_sections(request),
        };
        let provider_context = CodeIntelligenceContext::new(
            context.clone(),
            ResolvedSourceRoot {
                source_set: Some(source.source_set.name),
                path: source.path,
            },
        );
        RlmNavigationAdapter::new().metadata_related(
            request.metadata_path.as_str(),
            &names,
            request.limit,
            &provider_context,
            ProviderDeadline::new(std::time::Instant::now() + std::time::Duration::from_secs(5)),
            cancellation,
        )
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

fn unavailable_section() -> MetaRelatedSection<serde_json::Value> {
    MetaRelatedSection {
        status: MetaRelatedStatus::Unavailable,
        freshness: MetaFreshness::Unknown,
        completeness: MetaCompleteness::Unknown,
        total: 0,
        returned: 0,
        truncated: false,
        items: Vec::new(),
        diagnostics: capability_unavailable("typed related metadata provider is not available yet")
            .diagnostics,
    }
}

fn selected_unavailable_sections(request: &MetaInfoRequest) -> MetaRelatedSections {
    MetaRelatedSections {
        modules: request
            .sections
            .contains(&crate::application::metadata::MetaInfoSection::Modules)
            .then(unavailable_section),
        roles: request
            .sections
            .contains(&crate::application::metadata::MetaInfoSection::Roles)
            .then(unavailable_section),
        subscriptions: request
            .sections
            .contains(&crate::application::metadata::MetaInfoSection::Subscriptions)
            .then(unavailable_section),
        functional_options: request
            .sections
            .contains(&crate::application::metadata::MetaInfoSection::FunctionalOptions)
            .then(unavailable_section),
        predefined_items: request
            .sections
            .contains(&crate::application::metadata::MetaInfoSection::PredefinedItems)
            .then(unavailable_section),
    }
}

fn capability_unavailable(message: &str) -> MetaFailure {
    MetaDiagnostic::error(MetaDiagnosticCode::CapabilityUnavailable, message).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::metadata::{MetaAddRequest, MetaEditRequest, MetaRemoveRequest};
    use crate::application::ports::{
        ApplicationPorts, FormatGuardCheck, FormatGuardError, HandlerOutcome,
        MetadataChildDirectoryKind, MetadataChildProfile, MetadataChildResourceKind,
        MetadataResourceRole, MetadataTemplateResourcePart, MetadataTemplateType,
        SupportGuardCheck,
    };
    use crate::application::{ToolSpec, UnicaApplication};
    use crate::domain::cache::{CacheAccess, CacheReport};
    use crate::domain::events::DomainEvent;
    use crate::domain::metadata::{
        MetaCollection, MetaEditOperation, MetaElementInput, MetaElementUpdateInput,
        MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetaPublicationAction,
        MetaPublicationResource, MetaRelation, MetaScope, MetaValidationStatus, MetadataKind,
        MetadataReference, RelationEditMode,
    };
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::application_ports::InfrastructureApplicationPorts;
    use crate::infrastructure::native_operations::cf::create_configuration_scaffold;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_before_rollback_mutation_hook, with_commit_failpoint, CommitFailpoint,
    };
    use serde_json::{json, Map};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

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
            dry_run: bool,
            context: &WorkspaceContext,
        ) -> Result<(), String> {
            self.inner
                .validate_tool_context(spec, args, dry_run, context)
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
            dry_run: bool,
            cancellation: &CancellationToken,
        ) -> Result<HandlerOutcome, String> {
            self.inner
                .invoke_handler(spec, args, context, dry_run, cancellation)
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            events: &[DomainEvent],
            dry_run: bool,
            cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            self.inner
                .cache_report(context, events, dry_run, cache_access)
        }

        fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]) {
            self.inner.notify_invalidation(context, events);
        }
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
            let cancellation = CancellationToken::new();
            MetadataOperations::prepare_mutation(
                &MetadataRequest::Add(MetaAddRequest {
                    source_set: "main".into(),
                    kind,
                    name: name.into(),
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
        fs::write(old_root.join("Ext/Module.bsl"), &module).unwrap();
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
        assert_eq!(fs::read(new_root.join("Ext/Module.bsl")).unwrap(), module);
    }

    #[test]
    fn typed_form_remove_publish_deletes_descriptor_and_complete_payload_tree() {
        let fixture = Fixture::new("form-resource-publish-remove");
        let cancellation = CancellationToken::new();
        fixture.publish_form_add("Gone");
        let payload_root = fixture.root.join("src/Catalogs/Editable/Forms/Gone");
        fs::write(
            payload_root.join("Ext/Module.bsl"),
            b"procedure Gone()\nendprocedure",
        )
        .unwrap();
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
        let module = forms.join("Only/Ext/Module.bsl");
        fs::write(&module, b"procedure Kept()\nendprocedure").unwrap();
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
        let stale_module = payload_root.join("Ext/Module.bsl");
        fs::write(&stale_module, b"procedure Stale()\nendprocedure").unwrap();
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
        fs::write(
            payload_root.join("Ext/Module.bsl"),
            b"procedure Initial()\nendprocedure",
        )
        .unwrap();
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
        fs::write(
            old_root.join("Ext/Module.bsl"),
            b"procedure Kept()\nendprocedure",
        )
        .unwrap();
        let owner_before = fs::read(&fixture.descriptor).unwrap();
        let descriptor_before = fs::read(&old_descriptor).unwrap();
        let form_before = fs::read(old_root.join("Ext/Form.xml")).unwrap();
        let module_before = fs::read(old_root.join("Ext/Module.bsl")).unwrap();
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
            fs::read(old_root.join("Ext/Module.bsl")).unwrap(),
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
        let command_descriptor = object_root.join("Commands/Run.xml");
        assert!(object_root.join("Templates/Print.xml").is_file());
        assert!(template_content.is_file());
        assert!(command_descriptor.is_file());
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
        fs::write(
            fixture
                .root
                .join("src/Catalogs/Editable/Forms/Main/Ext/Module.bsl"),
            [0xff, 0x00, 0x80],
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
        let module = form
            .root
            .join("src/Catalogs/Editable/Forms/Main/Ext/Module.bsl");
        fs::write(
            &module,
            "Процедура ПриОткрытии()\nКонецПроцедуры".as_bytes(),
        )
        .unwrap();
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
        assert_eq!(
            prepared.validation_subject().child_footprints[0].directories,
            vec![
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
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
            application.call_unregistered_meta_remove_for_integration_tests(&args)
        })
        .expect("private typed meta.remove call");

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
        let Some(link_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &outside,
                fixture.root.join("src"),
            )
        else {
            fs::rename(&outside, fixture.root.join("src")).unwrap();
            return;
        };
        if link_result.is_err() {
            fs::rename(&outside, fixture.root.join("src")).unwrap();
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
        fs::remove_file(fixture.root.join("src")).unwrap();
        fs::rename(&outside, fixture.root.join("src")).unwrap();
    }
}
