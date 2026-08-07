use crate::application::metadata::{
    MetaAddRequest, MetaEditRequest, MetaFailure, MetaRemoveRequest,
};
use crate::application::ports::{
    MetaPublishReport, MetadataResourceImage, MetadataResourceRole, MetadataValidationSubject,
    PreparedMetadataMutation,
};
use crate::application::SupportGuardRequirement;
use crate::domain::cancellation::CancellationToken;
use crate::domain::metadata::{
    metadata_identifier_is_valid, MetaDiagnostic, MetaDiagnosticCode, MetaDiagnosticSeverity,
    MetaMutationData, MetaMutationEffect, MetaPublicationAction, MetaPublicationPlanEntry,
    MetaPublicationResource, MetaValidationData, MetaValidationStatus,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::metadata_layout;
use roxmltree::Document;
#[cfg(test)]
use serde_json::Map;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::common::guard_resolved_platform_xml_target_dependencies;
use super::super::compile_transaction::{
    CommitFailure, CommitFailureKind, CompileTransaction, DirectoryMembershipSnapshot,
    RegistrationStatus,
};
use super::edit::{
    build_typed_operation_post_image, resolve_typed_metadata_object, ResolvedMetadataObject,
    TypedChildResourcePlan, TypedOperationPostImage,
};
use super::remove::{plan_typed_remove, TypedMetaRemovePlan};
use super::template_catalog::{
    MetadataTemplateCatalog, MetadataTemplateFileMode, MetadataTemplateFileRole,
    PlatformMetadataTemplateCatalog,
};
use crate::infrastructure::platform_xml_source_targets::{
    bind_metadata_add_source_evidence, resolve_metadata_add_source, revalidate_metadata_add_source,
    revalidate_platform_xml_target, ResolvedSourceSet,
};
use crate::infrastructure::support_guard::{
    bind_resolved_support_guard_evidence, evaluate_resolved_support_guard,
    ResolvedSupportGuardCheck,
};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

#[cfg(test)]
thread_local! {
    static META_ADD_AFTER_AUTHORIZATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static META_EDIT_BEFORE_REAUTHORIZATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_meta_add_after_authorization_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            META_ADD_AFTER_AUTHORIZATION_HOOK.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    META_ADD_AFTER_AUTHORIZATION_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    let _reset = Reset;
    action()
}

#[cfg(test)]
fn run_meta_add_after_authorization_hook() {
    META_ADD_AFTER_AUTHORIZATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
pub(crate) fn with_meta_edit_before_reauthorization_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            META_EDIT_BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    META_EDIT_BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    let _reset = Reset;
    action()
}

#[cfg(test)]
fn run_meta_edit_before_reauthorization_hook() {
    META_EDIT_BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

pub(crate) struct PreparedMetaAdd {
    preview: MetaMutationData,
    validation_subject: MetadataValidationSubject,
    transaction: CompileTransaction,
    context: WorkspaceContext,
    source: ResolvedSourceSet,
}

pub(crate) struct PreparedMetaEdit {
    preview: MetaMutationData,
    validation_subject: MetadataValidationSubject,
    transaction: CompileTransaction,
    context: WorkspaceContext,
    resolved: ResolvedMetadataObject,
    expected_post_image: Vec<u8>,
}

pub(crate) struct PreparedMetaRemove {
    preview: MetaMutationData,
    validation_subject: MetadataValidationSubject,
    transaction: CompileTransaction,
    context: WorkspaceContext,
    resolved: ResolvedMetadataObject,
    expected_post_images: Vec<(PathBuf, Vec<u8>)>,
    expected_absent: Vec<PathBuf>,
}

impl PreparedMetaEdit {
    pub(super) fn prepare(
        request: &MetaEditRequest,
        resolved: ResolvedMetadataObject,
        context: &WorkspaceContext,
        post_image: Vec<u8>,
        mut diagnostics: Vec<MetaDiagnostic>,
        child_resources: TypedChildResourcePlan,
        effects: Vec<MetaMutationEffect>,
    ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
        let target = request.metadata_path.clone();
        let changed = post_image != resolved.descriptor_preimage;
        if !changed
            && child_resources.publication_plan.is_empty()
            && !child_resources.file_mutations.is_empty()
        {
            return Err(provider_failure(
                &target,
                "unchanged metadata preview cannot contain filesystem mutations".to_string(),
            ));
        }
        #[cfg(test)]
        run_meta_edit_before_reauthorization_hook();
        let mut transaction = CompileTransaction::new();
        bind_resolved_support_guard_evidence(&mut transaction, &resolved.descriptor_path, context)
            .map_err(|message| provider_failure(&target, message))?;
        match evaluate_resolved_support_guard(
            &resolved.descriptor_path,
            SupportGuardRequirement::Editable,
            context,
        ) {
            ResolvedSupportGuardCheck::Allow => {}
            ResolvedSupportGuardCheck::Warn(_) => diagnostics.push(MetaDiagnostic {
                code: MetaDiagnosticCode::SupportLocked,
                severity: MetaDiagnosticSeverity::Warning,
                message: "metadata source support policy permits editing with a warning"
                    .to_string(),
                metadata_path: Some(target.clone()),
                operation_index: None,
                field: None,
            }),
            ResolvedSupportGuardCheck::Block(_) => {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::SupportLocked,
                    "metadata source support policy blocks object editing",
                )
                .with_metadata_path(target)
                .into());
            }
        }
        if changed {
            transaction
                .replace_bytes(
                    &resolved.descriptor_path,
                    &resolved.descriptor_preimage,
                    post_image.clone(),
                )
                .map_err(|message| provider_failure(&target, message))?;
        } else {
            transaction
                .guard_or_verify_exact_preimage(
                    &resolved.descriptor_path,
                    &resolved.descriptor_preimage,
                )
                .map_err(|message| provider_failure(&target, message))?;
        }
        transaction
            .guard_or_verify_exact_preimage(&resolved.owner_path, &resolved.owner_preimage)
            .map_err(|message| provider_failure(&target, message))?;
        guard_resolved_platform_xml_target_dependencies(
            &mut transaction,
            &resolved.handle,
            context,
        )
        .map_err(|message| provider_failure(&target, message))?;
        for (path, snapshot) in child_resources.directory_guards {
            transaction
                .guard_or_verify_directory_topology(path, snapshot)
                .map_err(|message| provider_failure(&target, message))?;
        }
        for (path, bytes) in child_resources.exact_file_guards {
            transaction
                .guard_or_verify_exact_preimage(path, &bytes)
                .map_err(|message| provider_failure(&target, message))?;
        }
        for mutation in child_resources.file_mutations {
            match (mutation.pre_image, mutation.post_image) {
                (None, Some(post_image)) => transaction
                    .create_bytes(mutation.path, post_image)
                    .map_err(|message| provider_failure(&target, message))?,
                (Some(pre_image), Some(post_image)) => transaction
                    .replace_bytes(mutation.path, pre_image, post_image)
                    .map_err(|message| provider_failure(&target, message))?,
                (Some(_), None) => transaction
                    .remove_path(mutation.path)
                    .map_err(|message| provider_failure(&target, message))?,
                (None, None) => {}
            }
        }
        let mut validation_resources = vec![
            MetadataResourceImage {
                role: MetadataResourceRole::Descriptor,
                bytes: post_image.clone(),
            },
            MetadataResourceImage {
                role: MetadataResourceRole::Registration,
                bytes: resolved.owner_preimage.clone(),
            },
        ];
        for dependency in child_resources.relation_dependencies {
            transaction
                .guard_or_verify_exact_preimage(&dependency.path, &dependency.bytes)
                .map_err(|message| provider_failure(&target, message))?;
            guard_resolved_platform_xml_target_dependencies(
                &mut transaction,
                &dependency.handle,
                context,
            )
            .map_err(|message| provider_failure(&target, message))?;
            validation_resources.push(MetadataResourceImage {
                role: MetadataResourceRole::Dependency {
                    target: dependency.target,
                },
                bytes: dependency.bytes,
            });
        }

        validation_resources.extend(child_resources.validation_resources);
        for (dependency_path, dependency_target, bytes) in
            registered_language_images(&resolved.source_root, &resolved.owner_preimage)
                .map_err(|message| provider_failure(&target, message))?
        {
            transaction
                .guard_or_verify_exact_preimage(dependency_path, &bytes)
                .map_err(|message| provider_failure(&target, message))?;
            validation_resources.push(MetadataResourceImage {
                role: MetadataResourceRole::Dependency {
                    target: dependency_target,
                },
                bytes,
            });
        }
        let validation_subject = MetadataValidationSubject {
            target: target.clone(),
            resources: validation_resources,
            child_footprints: child_resources.validation_footprints,
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        };
        Ok(Box::new(Self {
            preview: MetaMutationData {
                metadata_path: target.clone(),
                changed: changed || !child_resources.publication_plan.is_empty(),
                publication_plan: changed
                    .then_some(MetaPublicationPlanEntry {
                        action: MetaPublicationAction::Update,
                        resource: MetaPublicationResource::Descriptor,
                        metadata_path: Some(target),
                    })
                    .into_iter()
                    .chain(child_resources.publication_plan)
                    .collect(),
                effects,
                validation: MetaValidationData {
                    status: MetaValidationStatus::Passed,
                    diagnostics: Vec::new(),
                },
                diagnostics,
            },
            validation_subject,
            transaction,
            context: context.clone(),
            resolved,
            expected_post_image: post_image,
        }))
    }
}

impl PreparedMetadataMutation for PreparedMetaEdit {
    fn preview(&self) -> &MetaMutationData {
        &self.preview
    }

    fn validation_subject(&self) -> &MetadataValidationSubject {
        &self.validation_subject
    }

    fn publish(
        self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<MetaPublishReport, MetaFailure> {
        if cancellation.is_cancelled() {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata edit was cancelled before publication",
            )
            .with_metadata_path(self.preview.metadata_path.clone())
            .into());
        }
        let target = self.preview.metadata_path.clone();
        revalidate_platform_xml_target(&self.context, &self.resolved.handle).map_err(|_| {
            MetaFailure::from(
                MetaDiagnostic::error(
                    MetaDiagnosticCode::ConcurrentModification,
                    format!("metadata source changed while publishing `{target}`"),
                )
                .with_metadata_path(target.clone()),
            )
        })?;
        let descriptor_path = self.resolved.descriptor_path.clone();
        let expected = self.expected_post_image.clone();
        let handle = &self.resolved.handle;
        let context = &self.context;
        self.transaction
            .commit_with_classified_post_validation(|| {
                let published = fs::read(&descriptor_path).map_err(|_| {
                    CommitFailure::provider("published metadata descriptor is unavailable")
                })?;
                if published != expected {
                    return Err(CommitFailure::provider(
                        "published metadata descriptor differs from its post-image",
                    ));
                }
                revalidate_platform_xml_target(context, handle)
                    .map(|_| ())
                    .map_err(|_| {
                        CommitFailure::concurrent("metadata target changed during publication")
                    })
            })
            .map_err(|failure| publication_failure(&target, failure.kind()))?;
        Ok(MetaPublishReport { data: self.preview })
    }
}

pub(crate) fn prepare_meta_remove(
    request: &MetaRemoveRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
    let resolved = resolve_typed_metadata_object(
        &request.source_set,
        &request.metadata_path,
        "remove",
        context,
        cancellation,
    )?;
    let TypedMetaRemovePlan {
        preview,
        validation_subject,
        transaction,
        resolved,
        expected_post_images,
        expected_absent,
    } = plan_typed_remove(request, resolved, context, cancellation)?;
    Ok(Box::new(PreparedMetaRemove {
        preview,
        validation_subject,
        transaction,
        context: context.clone(),
        resolved,
        expected_post_images,
        expected_absent,
    }))
}

impl PreparedMetadataMutation for PreparedMetaRemove {
    fn preview(&self) -> &MetaMutationData {
        &self.preview
    }

    fn validation_subject(&self) -> &MetadataValidationSubject {
        &self.validation_subject
    }

    fn publish(
        self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<MetaPublishReport, MetaFailure> {
        if cancellation.is_cancelled() {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata removal was cancelled before publication",
            )
            .with_metadata_path(self.preview.metadata_path.clone())
            .into());
        }
        let target = self.preview.metadata_path.clone();
        revalidate_platform_xml_target(&self.context, &self.resolved.handle).map_err(|_| {
            MetaFailure::from(
                MetaDiagnostic::error(
                    MetaDiagnosticCode::ConcurrentModification,
                    format!("metadata source changed while publishing `{target}`"),
                )
                .with_metadata_path(target.clone()),
            )
        })?;
        let expected_post_images = self.expected_post_images.clone();
        let expected_absent = self.expected_absent.clone();
        self.transaction
            .commit_with_classified_post_validation(move || {
                for (path, expected) in &expected_post_images {
                    let actual = fs::read(path).map_err(|_| {
                        CommitFailure::provider("published metadata post-image is unavailable")
                    })?;
                    if &actual != expected {
                        return Err(CommitFailure::concurrent(
                            "published metadata post-image differs from its plan",
                        ));
                    }
                }
                for path in &expected_absent {
                    match fs::symlink_metadata(path) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Ok(_) => {
                            return Err(CommitFailure::concurrent(
                                "removed metadata resource is still present",
                            ))
                        }
                        Err(_) => {
                            return Err(CommitFailure::provider(
                                "removed metadata resource could not be inspected",
                            ))
                        }
                    }
                }
                Ok(())
            })
            .map_err(|failure| publication_failure(&target, failure.kind()))?;
        Ok(MetaPublishReport { data: self.preview })
    }
}

pub(crate) fn prepare_meta_add(
    request: &MetaAddRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
    if cancellation.is_cancelled() {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata creation was cancelled before source resolution",
        )
        .into());
    }
    let source = resolve_metadata_add_source(context, &request.source_set)?;
    match evaluate_resolved_support_guard(
        &source.owner_path,
        SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => {}
        ResolvedSupportGuardCheck::Block(_) => {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::SupportLocked,
                "metadata source support policy blocks object creation",
            )
            .into());
        }
    }
    let mut post_image =
        PlatformMetadataTemplateCatalog.minimal_object(&source, request.kind, &request.name)?;
    let target = post_image.metadata_path.clone();
    let descriptor_file = post_image
        .files
        .iter_mut()
        .find(|file| file.role == MetadataTemplateFileRole::Descriptor)
        .expect("every metadata template has one descriptor");
    let descriptor_path = source.source_root.join(&descriptor_file.relative_path);
    let TypedOperationPostImage {
        descriptor,
        child_resources,
        effects,
    } = build_typed_operation_post_image(
        &request.source_set,
        &descriptor_path,
        &target,
        &descriptor_file.bytes,
        &request.operations,
        context,
    )?;
    descriptor_file.bytes = descriptor;
    let mut mutation_effects = vec![MetaMutationEffect {
        operation_index: None,
        operation: "createTemplate".to_string(),
        target: target.as_str().to_string(),
        before: None,
        after: Some(serde_json::json!({
            "kind": request.kind.as_str(),
            "name": request.name,
        })),
    }];
    mutation_effects.extend(effects);

    #[cfg(test)]
    run_meta_add_after_authorization_hook();

    // The transaction starts only after template operations and every derived
    // child/dependency plan have built one complete private post-image.
    let mut transaction = CompileTransaction::new();
    bind_resolved_support_guard_evidence(&mut transaction, &source.owner_path, context).map_err(
        |_| {
            MetaFailure::from(MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata support-policy evidence could not be bound",
            ))
        },
    )?;
    let mut preparation_diagnostics = Vec::new();
    match evaluate_resolved_support_guard(
        &source.owner_path,
        SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow => {}
        ResolvedSupportGuardCheck::Warn(_) => preparation_diagnostics.push(MetaDiagnostic {
            code: MetaDiagnosticCode::SupportLocked,
            severity: MetaDiagnosticSeverity::Warning,
            message: "metadata source support policy permits creation with a warning".to_string(),
            metadata_path: None,
            operation_index: None,
            field: None,
        }),
        ResolvedSupportGuardCheck::Block(_) => {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::SupportLocked,
                "metadata source support policy blocks object creation",
            )
            .into());
        }
    }
    let resource_root = source
        .source_root
        .join(metadata_layout(request.kind).directory)
        .join(&request.name);
    let mut guarded_resource_directories = BTreeSet::from([resource_root.clone()]);
    for file in &post_image.files {
        let path = source.source_root.join(&file.relative_path);
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| directory.starts_with(&resource_root))
        {
            guarded_resource_directories.insert(directory.to_path_buf());
            if directory == resource_root {
                break;
            }
            parent = directory.parent();
        }
    }
    for directory in guarded_resource_directories {
        transaction
            .guard_or_verify_directory_topology(directory, DirectoryMembershipSnapshot::Absent)
            .map_err(|_| already_exists(&target))?;
    }

    for (path, snapshot) in child_resources.directory_guards {
        transaction
            .guard_or_verify_directory_topology(path, snapshot)
            .map_err(|message| provider_failure(&target, message))?;
    }
    for (path, bytes) in child_resources.exact_file_guards {
        transaction
            .guard_or_verify_exact_preimage(path, &bytes)
            .map_err(|message| provider_failure(&target, message))?;
    }

    for file in &post_image.files {
        let path = source.source_root.join(&file.relative_path);
        match file.mode {
            MetadataTemplateFileMode::Create => transaction
                .create_bytes(path, file.bytes.clone())
                .map_err(|_| already_exists(&target))?,
            MetadataTemplateFileMode::Guard => transaction
                .guard_or_verify_exact_preimage(
                    path,
                    file.preimage.as_deref().unwrap_or(&file.bytes),
                )
                .map_err(|message| provider_failure(&target, message))?,
            MetadataTemplateFileMode::Replace => transaction
                .replace_bytes(
                    path,
                    file.preimage.as_deref().unwrap_or(&file.bytes),
                    file.bytes.clone(),
                )
                .map_err(|message| provider_failure(&target, message))?,
        }
    }
    for mutation in child_resources.file_mutations {
        match (mutation.pre_image, mutation.post_image) {
            (None, Some(post_image)) => transaction
                .create_bytes(mutation.path, post_image)
                .map_err(|message| provider_failure(&target, message))?,
            (Some(pre_image), Some(post_image)) => transaction
                .replace_bytes(mutation.path, pre_image, post_image)
                .map_err(|message| provider_failure(&target, message))?,
            (Some(_), None) => transaction
                .remove_path(mutation.path)
                .map_err(|message| provider_failure(&target, message))?,
            (None, None) => {}
        }
    }
    let registration = transaction
        .register_canonical_child(&source.owner_path, request.kind.as_str(), &request.name)
        .map_err(|message| provider_failure(&target, message))?;
    if registration != RegistrationStatus::Added {
        return Err(already_exists(&target));
    }
    bind_metadata_add_source_evidence(&mut transaction, context, &source)?;
    let registration_image = transaction
        .planned_registration_image(&source.owner_path)
        .ok_or_else(|| {
            provider_failure(
                &target,
                "metadata owner registration post-image is unavailable".to_string(),
            )
        })?;

    let mut validation_resources = child_resources.validation_resources;
    let mut publication_plan = child_resources.publication_plan;
    for file in post_image.files {
        let (validation_role, publication_resource, publication_target) = match file.role {
            MetadataTemplateFileRole::Descriptor => (
                Some(MetadataResourceRole::Descriptor),
                MetaPublicationResource::Descriptor,
                target.clone(),
            ),
            MetadataTemplateFileRole::Module => (
                Some(MetadataResourceRole::Module {
                    owner: target.clone(),
                }),
                MetaPublicationResource::Module,
                target.clone(),
            ),
            MetadataTemplateFileRole::AuxiliaryXml(kind) => (
                Some(MetadataResourceRole::AuxiliaryXml {
                    owner: target.clone(),
                    kind,
                }),
                MetaPublicationResource::Dependency,
                target.clone(),
            ),
            MetadataTemplateFileRole::Dependency(dependency) => {
                let publication_target = dependency.clone();
                (
                    Some(MetadataResourceRole::Dependency { target: dependency }),
                    MetaPublicationResource::Dependency,
                    publication_target,
                )
            }
            MetadataTemplateFileRole::DependencyModule(dependency) => {
                let publication_target = dependency.clone();
                (
                    Some(MetadataResourceRole::Module { owner: dependency }),
                    MetaPublicationResource::Dependency,
                    publication_target,
                )
            }
        };
        if let Some(role) = validation_role {
            validation_resources.push(MetadataResourceImage {
                role,
                bytes: file.bytes,
            });
        }
        if file.mode != MetadataTemplateFileMode::Guard {
            publication_plan.push(MetaPublicationPlanEntry {
                action: match file.mode {
                    MetadataTemplateFileMode::Create => MetaPublicationAction::Create,
                    MetadataTemplateFileMode::Replace => MetaPublicationAction::Update,
                    MetadataTemplateFileMode::Guard => unreachable!(),
                },
                resource: publication_resource,
                metadata_path: Some(publication_target),
            });
        }
    }
    validation_resources.push(MetadataResourceImage {
        role: MetadataResourceRole::Registration,
        bytes: registration_image,
    });
    for dependency in child_resources.relation_dependencies {
        transaction
            .guard_or_verify_exact_preimage(&dependency.path, &dependency.bytes)
            .map_err(|message| provider_failure(&target, message))?;
        guard_resolved_platform_xml_target_dependencies(
            &mut transaction,
            &dependency.handle,
            context,
        )
        .map_err(|message| provider_failure(&target, message))?;
        validation_resources.push(MetadataResourceImage {
            role: MetadataResourceRole::Dependency {
                target: dependency.target,
            },
            bytes: dependency.bytes,
        });
    }
    for (dependency_path, target, bytes) in
        registered_language_images(&source.source_root, &source.owner_preimage)
            .map_err(|message| provider_failure(&target, message))?
    {
        transaction
            .guard_or_verify_exact_preimage(dependency_path, &bytes)
            .map_err(|message| provider_failure(&target, message))?;
        validation_resources.push(MetadataResourceImage {
            role: MetadataResourceRole::Dependency { target },
            bytes,
        });
    }
    publication_plan.push(MetaPublicationPlanEntry {
        action: MetaPublicationAction::Update,
        resource: MetaPublicationResource::Registration,
        metadata_path: Some(target.clone()),
    });

    Ok(Box::new(PreparedMetaAdd {
        preview: MetaMutationData {
            metadata_path: target.clone(),
            changed: true,
            publication_plan,
            effects: mutation_effects,
            validation: MetaValidationData {
                status: MetaValidationStatus::Passed,
                diagnostics: Vec::new(),
            },
            diagnostics: preparation_diagnostics,
        },
        validation_subject: MetadataValidationSubject {
            target,
            resources: validation_resources,
            child_footprints: child_resources.validation_footprints,
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        },
        transaction,
        context: context.clone(),
        source,
    }))
}

fn registered_language_images(
    source_root: &Path,
    owner_preimage: &[u8],
) -> Result<
    Vec<(
        PathBuf,
        crate::domain::source_target::MetadataAddress,
        Vec<u8>,
    )>,
    String,
> {
    let xml = std::str::from_utf8(owner_preimage)
        .map_err(|_| "metadata owner image is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    let document =
        Document::parse(xml).map_err(|_| "metadata owner image is not valid XML".to_string())?;
    let configuration = document.root_element().children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "Configuration"
    });
    let child_objects = configuration.and_then(|configuration| {
        configuration.children().find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "ChildObjects"
        })
    });
    let mut images = Vec::new();
    for node in child_objects
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Language"
        })
    {
        let Some(name) = node.text().map(str::trim).filter(|name| !name.is_empty()) else {
            continue;
        };
        if !metadata_identifier_is_valid(name) {
            return Err(format!(
                "registered language `{name}` is not a valid 1C identifier"
            ));
        }
        let target = crate::domain::source_target::MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("Language.{name}"),
        )
        .map_err(|_| "registered language has an invalid logical identity".to_string())?;
        let path = source_root.join("Languages").join(format!("{name}.xml"));
        let bytes = fs::read(&path)
            .map_err(|_| format!("registered language `{name}` image is unavailable"))?;
        images.push((path, target, bytes));
    }
    Ok(images)
}

impl PreparedMetadataMutation for PreparedMetaAdd {
    fn preview(&self) -> &MetaMutationData {
        &self.preview
    }

    fn validation_subject(&self) -> &MetadataValidationSubject {
        &self.validation_subject
    }

    fn publish(
        self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<MetaPublishReport, MetaFailure> {
        if cancellation.is_cancelled() {
            return Err(MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata creation was cancelled before publication",
            )
            .with_metadata_path(self.preview.metadata_path.clone())
            .into());
        }
        revalidate_metadata_add_source(&self.context, &self.source)?;
        let target = self.preview.metadata_path.clone();
        self.transaction
            .commit_with_classified_post_validation(|| {
                revalidate_metadata_add_source(&self.context, &self.source)
                    .map_err(|failure| commit_failure_from_meta(&failure))
            })
            .map_err(|failure| publication_failure(&target, failure.kind()))?;
        Ok(MetaPublishReport { data: self.preview })
    }
}

fn already_exists(target: &crate::domain::source_target::MetadataAddress) -> MetaFailure {
    MetaDiagnostic::error(
        MetaDiagnosticCode::AlreadyExists,
        format!("metadata object `{target}` already exists or has a partial footprint"),
    )
    .with_metadata_path(target.clone())
    .into()
}

fn provider_failure(
    target: &crate::domain::source_target::MetadataAddress,
    _internal: String,
) -> MetaFailure {
    MetaDiagnostic::error(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("metadata provider could not prepare `{target}`"),
    )
    .with_metadata_path(target.clone())
    .into()
}

fn publication_failure(
    target: &crate::domain::source_target::MetadataAddress,
    kind: CommitFailureKind,
) -> MetaFailure {
    let code = match kind {
        CommitFailureKind::ConcurrentModification => MetaDiagnosticCode::ConcurrentModification,
        CommitFailureKind::ProviderUnavailable => MetaDiagnosticCode::ProviderUnavailable,
        CommitFailureKind::RollbackFailed => MetaDiagnosticCode::RollbackFailed,
    };
    MetaDiagnostic::error(
        code,
        match code {
            MetaDiagnosticCode::RollbackFailed => {
                format!("metadata publication for `{target}` failed and rollback was incomplete")
            }
            MetaDiagnosticCode::ConcurrentModification => {
                format!("metadata source changed while publishing `{target}`")
            }
            _ => format!("metadata provider could not publish `{target}`"),
        },
    )
    .with_metadata_path(target.clone())
    .into()
}

fn commit_failure_from_meta(failure: &MetaFailure) -> CommitFailure {
    let message = failure
        .diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| "metadata source revalidation failed".to_string());
    if failure.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.code,
            MetaDiagnosticCode::ConcurrentModification | MetaDiagnosticCode::AlreadyExists
        )
    }) {
        CommitFailure::concurrent(message)
    } else {
        CommitFailure::provider(message)
    }
}

pub(crate) fn fresh_metadata_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod typed_add_publication_tests {
    use super::*;
    use crate::application::metadata::MetaAddRequest;
    use crate::domain::metadata::{
        MetaCollection, MetaEditOperation, MetaElementInput, MetaRelation, MetadataKind,
        MetadataReference, RelationEditMode,
    };
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use crate::infrastructure::native_operations::cf::create_configuration_scaffold;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_commit_failpoint, CommitFailpoint,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn publication_failure_code_is_selected_by_typed_commit_kind() {
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
        for (kind, expected) in [
            (
                CommitFailureKind::ConcurrentModification,
                MetaDiagnosticCode::ConcurrentModification,
            ),
            (
                CommitFailureKind::ProviderUnavailable,
                MetaDiagnosticCode::ProviderUnavailable,
            ),
            (
                CommitFailureKind::RollbackFailed,
                MetaDiagnosticCode::RollbackFailed,
            ),
        ] {
            let failure = publication_failure(&target, kind);
            assert_eq!(failure.diagnostics[0].code, expected);
        }
    }

    struct Fixture {
        root: PathBuf,
        context: WorkspaceContext,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "unica-meta-add-publisher-{label}-{}-{}",
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
                ("Name".to_string(), json!("MetaPublisher")),
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
            Self { root, context }
        }

        fn request(&self, name: &str) -> MetaAddRequest {
            MetaAddRequest {
                source_set: "main".to_string(),
                kind: MetadataKind::Catalog,
                name: name.to_string(),
                operations: Vec::new(),
                dry_run: false,
            }
        }

        fn seed_common_module(&self, name: &str, module: &[u8]) {
            let (xml, _) = super::super::template_catalog::minimal_metadata_xml_for_tests(
                MetadataKind::CommonModule,
                name,
            )
            .unwrap();
            let descriptor = self
                .root
                .join("src/CommonModules")
                .join(format!("{name}.xml"));
            fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
            fs::write(descriptor, xml).unwrap();
            let module_path = self
                .root
                .join("src/CommonModules")
                .join(name)
                .join("Ext/Module.bsl");
            fs::create_dir_all(module_path.parent().unwrap()).unwrap();
            fs::write(module_path, module).unwrap();
            let mut transaction = CompileTransaction::new();
            assert_eq!(
                transaction
                    .register_canonical_child(
                        self.root.join("src/Configuration.xml"),
                        "CommonModule",
                        name,
                    )
                    .unwrap(),
                RegistrationStatus::Added
            );
            transaction.commit().unwrap();
        }

        fn source_snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            let source = self.root.join("src");
            crate::test_support::tree_snapshot(&source)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn registered_language_images_use_only_valid_child_registrations() {
        let fixture = Fixture::new("language-registration-scope");
        let rogue = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Configuration><Properties><Language>Ghost</Language></Properties><ChildObjects/></Configuration></MetaDataObject>"#;
        assert!(registered_language_images(&fixture.root.join("src"), rogue)
            .unwrap()
            .is_empty());

        let invalid = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Configuration><ChildObjects><Language>/etc/passwd</Language></ChildObjects></Configuration></MetaDataObject>"#;
        let error = registered_language_images(&fixture.root.join("src"), invalid)
            .expect_err("a registered language must be a valid 1C identifier");
        assert!(error.contains("valid 1C identifier"), "{error}");
    }

    #[test]
    fn meta_add_detects_concurrent_owner_change_without_overwriting_it() {
        let fixture = Fixture::new("concurrent-owner");
        let cancellation = CancellationToken::new();
        let prepared = prepare_meta_add(
            &fixture.request("Concurrent"),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let owner = fixture.root.join("src/Configuration.xml");
        let mut external_image = fs::read(&owner).unwrap();
        external_image.extend_from_slice(b"\n");
        fs::write(&owner, &external_image).unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("concurrent publication unexpectedly succeeded"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(owner).unwrap(), external_image);
        assert!(!fixture.root.join("src/Catalogs/Concurrent.xml").exists());
    }

    #[test]
    fn meta_add_rolls_back_object_files_when_commit_aborts_before_registration() {
        let fixture = Fixture::new("rollback");
        let cancellation = CancellationToken::new();
        let prepared = prepare_meta_add(
            &fixture.request("RolledBack"),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let before = fixture.source_snapshot();

        let result = with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            prepared.publish(&cancellation)
        });
        let failure = match result {
            Ok(_) => panic!("failpoint publication unexpectedly succeeded"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(fixture.source_snapshot(), before);
    }

    #[test]
    fn meta_add_physical_form_footprint_commits_and_rolls_back_as_one_transaction() {
        let fixture = Fixture::new("physical-form-footprint");
        let cancellation = CancellationToken::new();
        let with_form = |name: &str| MetaAddRequest {
            operations: vec![MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput::named("Main")],
            )
            .unwrap()],
            ..fixture.request(name)
        };

        let prepared =
            prepare_meta_add(&with_form("WithForm"), &fixture.context, &cancellation).unwrap();
        assert!(prepared
            .validation_subject()
            .resources
            .iter()
            .any(|resource| {
                matches!(
                    &resource.role,
                    MetadataResourceRole::Form { owner, name }
                        if owner.as_str() == "Catalog.WithForm" && name == "Main"
                )
            }));
        prepared.publish(&cancellation).unwrap();

        for relative in [
            "Catalogs/WithForm.xml",
            "Catalogs/WithForm/Ext/ObjectModule.bsl",
            "Catalogs/WithForm/Forms/Main.xml",
            "Catalogs/WithForm/Forms/Main/Ext/Form.xml",
        ] {
            assert!(
                fixture.root.join("src").join(relative).is_file(),
                "missing physical form footprint: {relative}"
            );
        }
        assert!(
            fs::read_to_string(fixture.root.join("src/Configuration.xml"))
                .unwrap()
                .contains("<Catalog>WithForm</Catalog>")
        );

        let before_rollback = fixture.source_snapshot();
        let prepared = prepare_meta_add(
            &with_form("RolledBackForm"),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let failure = match with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            prepared.publish(&cancellation)
        }) {
            Ok(_) => panic!("physical form footprint unexpectedly survived commit failpoint"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(fixture.source_snapshot(), before_rollback);
    }

    #[test]
    fn meta_add_relation_target_drift_aborts_without_partial_creation() {
        let fixture = Fixture::new("relation-target-drift");
        let cancellation = CancellationToken::new();
        prepare_meta_add(&fixture.request("Parent"), &fixture.context, &cancellation)
            .unwrap()
            .publish(&cancellation)
            .unwrap();
        let parent_address =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Parent").unwrap();
        let request = MetaAddRequest {
            operations: vec![MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Add,
                vec![MetadataReference {
                    metadata_path: parent_address.clone(),
                }],
            )
            .unwrap()],
            ..fixture.request("Dependent")
        };
        let prepared = prepare_meta_add(&request, &fixture.context, &cancellation).unwrap();
        assert!(prepared.validation_subject().resources.iter().any(|resource| {
            matches!(&resource.role, MetadataResourceRole::Dependency { target } if target == &parent_address)
        }));
        let owner_before = fs::read(fixture.root.join("src/Configuration.xml")).unwrap();
        let parent_path = fixture.root.join("src/Catalogs/Parent.xml");
        let mut external_image = fs::read(&parent_path).unwrap();
        external_image.extend_from_slice(b"\n");
        let mut expected = fixture.source_snapshot();
        expected.insert(PathBuf::from("Catalogs/Parent.xml"), external_image.clone());
        fs::write(&parent_path, &external_image).unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("relation-target drift unexpectedly published metadata"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fixture.source_snapshot(), expected);
        assert_eq!(
            fs::read(fixture.root.join("src/Configuration.xml")).unwrap(),
            owner_before
        );
        assert!(!fixture.root.join("src/Catalogs/Dependent.xml").exists());
        assert!(!fixture.root.join("src/Catalogs/Dependent").exists());
    }

    #[test]
    fn meta_add_rejects_format_owner_drift_between_authorization_and_planning() {
        let fixture = Fixture::new("format-authorization-drift");
        let cancellation = CancellationToken::new();
        let owner = fixture.root.join("src/Configuration.xml");
        let owner_for_hook = owner.clone();

        let result = with_meta_add_after_authorization_hook(
            move || {
                let current = fs::read_to_string(&owner_for_hook).unwrap();
                let unsupported = current.replacen("version=\"2.20\"", "version=\"2.19\"", 1);
                assert_ne!(unsupported, current);
                fs::write(&owner_for_hook, unsupported).unwrap();
            },
            || {
                prepare_meta_add(
                    &fixture.request("FormatDrift"),
                    &fixture.context,
                    &cancellation,
                )
            },
        );

        let failure = match result {
            Ok(_) => panic!("stale format authorization unexpectedly prepared a mutation"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert!(!fixture.root.join("src/Catalogs/FormatDrift.xml").exists());
    }

    #[test]
    fn meta_add_rejects_prerequisite_bsl_drift_after_prepare() {
        let fixture = Fixture::new("prerequisite-drift");
        fixture.seed_common_module("Proof", b"Procedure Run() Export\nEndProcedure\n");
        let cancellation = CancellationToken::new();
        let prepared = prepare_meta_add(
            &MetaAddRequest {
                source_set: "main".to_string(),
                kind: MetadataKind::ScheduledJob,
                name: "Nightly".to_string(),
                operations: Vec::new(),
                dry_run: true,
            },
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let prerequisite = fixture.root.join("src/CommonModules/Proof/Ext/Module.bsl");
        fs::write(
            &prerequisite,
            b"Procedure Run() Export\n// concurrent edit\nEndProcedure\n",
        )
        .unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("prerequisite drift unexpectedly published metadata"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert!(!fixture.root.join("src/ScheduledJobs/Nightly.xml").exists());
        assert!(
            fs::read_to_string(prerequisite)
                .unwrap()
                .contains("concurrent edit"),
            "concurrent prerequisite edit must be preserved"
        );
    }

    #[test]
    fn meta_add_rejects_support_lock_that_appears_after_prepare() {
        let fixture = Fixture::new("support-drift");
        let cancellation = CancellationToken::new();
        let prepared = prepare_meta_add(
            &fixture.request("SupportDrift"),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let owner_before = fs::read(fixture.root.join("src/Configuration.xml")).unwrap();
        let support = fixture.root.join("src/Ext/ParentConfigurations.bin");
        fs::write(
            &support,
            concat!(
                "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",0,0,0}"
            ),
        )
        .unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("support drift unexpectedly published metadata"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(
            fs::read(fixture.root.join("src/Configuration.xml")).unwrap(),
            owner_before
        );
        assert!(!fixture.root.join("src/Catalogs/SupportDrift.xml").exists());
        assert!(
            support.is_file(),
            "concurrent support evidence must be preserved"
        );
    }

    #[test]
    fn meta_add_rejects_support_policy_change_after_prepare() {
        let fixture = Fixture::new("support-policy-drift");
        let cancellation = CancellationToken::new();
        fs::write(
            fixture.root.join("src/Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",0,0,0}"
            ),
        )
        .unwrap();
        let policy = fixture.root.join(".v8-project.json");
        fs::write(&policy, r#"{"editingAllowedCheck":"off"}"#).unwrap();
        let prepared = prepare_meta_add(
            &fixture.request("PolicyDrift"),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        fs::write(&policy, r#"{"editingAllowedCheck":"deny"}"#).unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("support-policy drift unexpectedly published metadata"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert!(!fixture.root.join("src/Catalogs/PolicyDrift.xml").exists());
    }

    #[test]
    fn meta_add_rejects_unplanned_resource_that_appears_after_prepare() {
        let fixture = Fixture::new("resource-drift");
        let cancellation = CancellationToken::new();
        let prepared = prepare_meta_add(
            &fixture.request("ResourceDrift"),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let owner_before = fs::read(fixture.root.join("src/Configuration.xml")).unwrap();
        let unexpected = fixture.root.join("src/Catalogs/ResourceDrift/Ext/Help.xml");
        fs::create_dir_all(unexpected.parent().unwrap()).unwrap();
        fs::write(&unexpected, b"concurrent").unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("unplanned resource unexpectedly survived publication"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(fs::read(&unexpected).unwrap(), b"concurrent");
        assert_eq!(
            fs::read(fixture.root.join("src/Configuration.xml")).unwrap(),
            owner_before
        );
        assert!(!fixture.root.join("src/Catalogs/ResourceDrift.xml").exists());
    }

    #[test]
    fn meta_add_rejects_empty_resource_root_that_appears_after_prepare() {
        let fixture = Fixture::new("empty-resource-root-drift");
        let cancellation = CancellationToken::new();
        let prepared = prepare_meta_add(
            &fixture.request("EmptyRootDrift"),
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        let owner_before = fs::read(fixture.root.join("src/Configuration.xml")).unwrap();
        let external_root = fixture.root.join("src/Catalogs/EmptyRootDrift");
        fs::create_dir_all(&external_root).unwrap();

        let failure = match prepared.publish(&cancellation) {
            Ok(_) => panic!("external empty resource root was adopted by publication"),
            Err(failure) => failure,
        };

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ConcurrentModification
        );
        assert_eq!(
            fs::read(fixture.root.join("src/Configuration.xml")).unwrap(),
            owner_before
        );
        assert!(!fixture
            .root
            .join("src/Catalogs/EmptyRootDrift.xml")
            .exists());
        assert_eq!(fs::read_dir(external_root).unwrap().count(), 0);
    }

    #[test]
    fn meta_add_validation_subject_contains_auxiliary_xml_and_guarded_bsl_images() {
        let fixture = Fixture::new("complete-post-image");
        let cancellation = CancellationToken::new();
        let exchange = prepare_meta_add(
            &MetaAddRequest {
                source_set: "main".to_string(),
                kind: MetadataKind::ExchangePlan,
                name: "Sync".to_string(),
                operations: Vec::new(),
                dry_run: true,
            },
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        assert_eq!(exchange.validation_subject().resources.len(), 5);
        assert!(
            exchange
                .validation_subject()
                .resources
                .iter()
                .any(|resource| {
                    matches!(
                resource.role,
                MetadataResourceRole::AuxiliaryXml {
                    kind: crate::application::ports::MetadataAuxiliaryXmlKind::ExchangePlanContent,
                    ..
                }
            )
                })
        );

        fixture.seed_common_module("Proof", b"Procedure Run() Export\nEndProcedure\n");
        let scheduled = prepare_meta_add(
            &MetaAddRequest {
                source_set: "main".to_string(),
                kind: MetadataKind::ScheduledJob,
                name: "Nightly".to_string(),
                operations: Vec::new(),
                dry_run: true,
            },
            &fixture.context,
            &cancellation,
        )
        .unwrap();
        assert_eq!(scheduled.validation_subject().resources.len(), 5);
        assert!(scheduled
            .validation_subject()
            .resources
            .iter()
            .any(|resource| {
                matches!(
                    &resource.role,
                    MetadataResourceRole::Module { owner }
                        if owner.as_str() == "CommonModule.Proof"
                )
            }));
    }
}
