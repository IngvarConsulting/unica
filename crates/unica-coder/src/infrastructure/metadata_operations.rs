use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::ports::{
    MetaLocalInfo, MetaRelatedData, MetadataRead, MetadataValidationResult,
    MetadataValidationSubject, PreparedMetadataMutation,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::metadata::{
    MetaCompleteness, MetaDiagnostic, MetaDiagnosticCode, MetaFreshness, MetaRelatedItem,
    MetaRelatedSection, MetaRelatedSections, MetaRelatedStatus,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::meta::{
    prepare_meta_add, prepare_typed_edit, resolve_typed_edit_object, MetadataValidator,
};

pub(crate) struct MetadataOperations;

impl MetadataOperations {
    pub(crate) fn read_local(
        _request: &MetaInfoRequest,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<MetadataRead, MetaFailure> {
        Err(capability_unavailable(
            "typed metadata read provider is not available yet",
        ))
    }

    pub(crate) fn read_related(
        _request: &MetaInfoRequest,
        _local: &MetaLocalInfo,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetaRelatedData {
        MetaRelatedSections {
            modules: unavailable_section(),
            roles: unavailable_section(),
            subscriptions: unavailable_section(),
            functional_options: unavailable_section(),
            predefined_items: Some(unavailable_section()),
        }
    }

    pub(crate) fn validate(
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        MetadataValidator.validate(subject, context)
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
            MetadataRequest::Info(_) | MetadataRequest::Remove(_) => Err(capability_unavailable(
                "typed metadata mutation provider is not available yet",
            )),
        }
    }
}

fn unavailable_section() -> MetaRelatedSection<MetaRelatedItem> {
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

fn capability_unavailable(message: &str) -> MetaFailure {
    MetaDiagnostic::error(MetaDiagnosticCode::CapabilityUnavailable, message).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::metadata::{MetaAddRequest, MetaEditRequest};
    use crate::application::ports::MetadataResourceRole;
    use crate::domain::metadata::{
        MetaCollection, MetaEditOperation, MetaElementInput, MetaElementUpdateInput,
        MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetaScope, MetadataKind,
    };
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::native_operations::cf::create_configuration_scaffold;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_before_rollback_mutation_hook, with_commit_failpoint, CommitFailpoint,
    };
    use serde_json::{json, Map};
    use std::fs;
    use std::path::PathBuf;

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
}
