#![allow(dead_code, unused_imports)]

use crate::application::metadata::{MetaAddRequest, MetaFailure};
use crate::application::ports::{
    MetaPublishReport, MetadataResourceImage, MetadataResourceRole, MetadataValidationSubject,
    PreparedMetadataMutation,
};
use crate::application::{AdapterOutcome, SupportGuardRequirement};
use crate::domain::cancellation::CancellationToken;
use crate::domain::metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaDiagnosticSeverity, MetaMutationData,
    MetaPublicationAction, MetaPublicationPlanEntry, MetaPublicationResource, MetaValidationData,
    MetaValidationStatus,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::metadata_kind;
use crate::infrastructure::metadata_kinds::metadata_layout;
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::common::{
    absolutize, guard_active_format_dependencies, read_utf8_sig, read_utf8_sig_snapshot, string_arg,
};
use super::super::compile_transaction::{CompileTransaction, RegistrationStatus};
use super::legacy_dsl::{
    compile_meta_value, meta_compile_definition_format_dependency_paths,
    meta_compile_event_subscription_dependencies, read_meta_compile_definition_guarded,
    require_meta_configuration_owner_validation,
    validate_meta_compile_event_subscription_dependencies, validate_metadata_owner_shape_8_3_27,
};
use super::template_catalog::{
    MetadataTemplateCatalog, MetadataTemplateFileMode, MetadataTemplateFileRole,
    PlatformMetadataTemplateCatalog,
};
use crate::infrastructure::platform_xml_source_targets::{
    bind_metadata_add_source_evidence, resolve_metadata_add_source, revalidate_metadata_add_source,
    ResolvedSourceSet,
};
use crate::infrastructure::support_guard::{
    bind_resolved_support_guard_evidence, evaluate_resolved_support_guard,
    ResolvedSupportGuardCheck,
};

#[cfg(test)]
thread_local! {
    static META_ADD_AFTER_AUTHORIZATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_meta_add_after_authorization_hook<T>(
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

pub(crate) struct PreparedMetaAdd {
    preview: MetaMutationData,
    validation_subject: MetadataValidationSubject,
    transaction: CompileTransaction,
    context: WorkspaceContext,
    source: ResolvedSourceSet,
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
    let post_image =
        PlatformMetadataTemplateCatalog.minimal_object(&source, request.kind, &request.name)?;
    #[cfg(test)]
    run_meta_add_after_authorization_hook();
    let target = post_image.metadata_path.clone();
    let resource_root = source
        .source_root
        .join(metadata_layout(request.kind).directory)
        .join(&request.name);
    if fs::symlink_metadata(&resource_root).is_ok() {
        return Err(already_exists(&target));
    }
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
            .guard_or_verify_directory_topology(directory, Vec::new())
            .map_err(|_| already_exists(&target))?;
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

    let mut validation_resources = Vec::new();
    let mut publication_plan = Vec::new();
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
    for (dependency_path, target, bytes) in
        registered_language_images(&source).map_err(|message| provider_failure(&target, message))?
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
            validation: MetaValidationData {
                status: MetaValidationStatus::Passed,
                diagnostics: Vec::new(),
            },
            diagnostics: preparation_diagnostics,
        },
        validation_subject: MetadataValidationSubject {
            target,
            resources: validation_resources,
        },
        transaction,
        context: context.clone(),
        source,
    }))
}

fn registered_language_images(
    source: &ResolvedSourceSet,
) -> Result<
    Vec<(
        PathBuf,
        crate::domain::source_target::MetadataAddress,
        Vec<u8>,
    )>,
    String,
> {
    let xml = std::str::from_utf8(&source.owner_preimage)
        .map_err(|_| "metadata owner image is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    let document =
        Document::parse(xml).map_err(|_| "metadata owner image is not valid XML".to_string())?;
    let mut images = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Language")
    {
        let Some(name) = node.text().map(str::trim).filter(|name| !name.is_empty()) else {
            continue;
        };
        let target = crate::domain::source_target::MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("Language.{name}"),
        )
        .map_err(|_| "registered language has an invalid logical identity".to_string())?;
        let path = source
            .source_root
            .join("Languages")
            .join(format!("{name}.xml"));
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
            .commit_with_post_validation(|| {
                revalidate_metadata_add_source(&self.context, &self.source)
                    .map_err(|failure| failure_message(&failure))
            })
            .map_err(|message| publication_failure(&target, message))?;
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
    internal: String,
) -> MetaFailure {
    let code = if internal.contains("rollback encountered") {
        MetaDiagnosticCode::RollbackFailed
    } else if internal.contains("changed")
        || internal.contains("preimage")
        || internal.contains("already exists")
        || internal.contains("guard was violated")
        || internal.contains("membership guard")
    {
        MetaDiagnosticCode::ConcurrentModification
    } else {
        MetaDiagnosticCode::ProviderUnavailable
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

fn failure_message(failure: &MetaFailure) -> String {
    failure
        .diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| "metadata source revalidation failed".to_string())
}

#[cfg(test)]
use super::{
    run_meta_compile_after_format_plan_hook, run_meta_compile_after_owner_validation_hook,
};

pub(crate) fn fresh_meta_compile_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(super) fn register_compiled_meta_in_configuration(
    output_dir: &Path,
    child_tag: &str,
    obj_name: &str,
) -> Result<Option<String>, String> {
    metadata_kind(child_tag).ok_or_else(|| format!("Unknown type '{child_tag}'"))?;
    let config_xml_path = output_dir.join("Configuration.xml");
    let mut transaction = CompileTransaction::new();
    let status = transaction.register_canonical_child(&config_xml_path, child_tag, obj_name)?;
    if status == RegistrationStatus::Added {
        transaction.commit()?;
    }
    Ok(Some(
        match status {
            RegistrationStatus::Added => "added",
            RegistrationStatus::AlreadyPresent => "already",
            RegistrationStatus::MissingTarget => "no-config",
        }
        .to_string(),
    ))
}

pub(super) fn register_compiled_meta_in_transaction(
    transaction: &mut CompileTransaction,
    output_dir: &Path,
    child_tag: &str,
    object_name: &str,
) -> Result<RegistrationStatus, String> {
    transaction.register_canonical_child(
        output_dir.join("Configuration.xml"),
        child_tag,
        object_name,
    )
}

pub(super) type MetaCompilePlan = (
    String,
    CompileTransaction,
    Vec<PathBuf>,
    Option<PathBuf>,
    Vec<PathBuf>,
);

pub(super) fn prepare_meta_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<MetaCompilePlan, String> {
    let output_dir_label = string_arg(args, &["outputDir", "OutputDir"])
        .ok_or_else(|| "missing required OutputDir argument".to_string())?
        .to_string();
    let output_dir = absolutize(PathBuf::from(&output_dir_label), &context.cwd);
    let config_path = output_dir.join("Configuration.xml");
    let config_owner = if config_path.is_file() {
        let snapshot = read_utf8_sig_snapshot(&config_path)?;
        require_meta_configuration_owner_validation(&config_path, context, "meta.compile")?;
        if fs::read(&config_path)
            .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?
            != snapshot.raw
        {
            return Err(format!(
                "Configuration owner changed while planning: {}",
                config_path.display()
            ));
        }
        #[cfg(test)]
        run_meta_compile_after_owner_validation_hook(&config_path);
        Some((config_path, snapshot.raw))
    } else {
        None
    };
    let mut transaction = CompileTransaction::new();
    let defn = read_meta_compile_definition_guarded(args, context, &mut transaction)?;
    let event_subscription_dependencies =
        meta_compile_event_subscription_dependencies(&defn, &output_dir);
    let mut format_dependencies =
        meta_compile_definition_format_dependency_paths(&defn, &output_dir);
    #[cfg(test)]
    run_meta_compile_after_format_plan_hook();
    let (stdout, planned_artifacts) = compile_meta_value(
        defn,
        &output_dir_label,
        &output_dir,
        context,
        &mut transaction,
        &mut format_dependencies,
    )?;
    validate_meta_compile_event_subscription_dependencies(
        &event_subscription_dependencies,
        &transaction,
    )?;
    if let Some((config_owner, expected_preimage)) = &config_owner {
        transaction.guard_or_verify_exact_preimage(config_owner, expected_preimage)?;
    }
    Ok((
        stdout,
        transaction,
        planned_artifacts,
        config_owner.map(|(path, _)| path),
        format_dependencies,
    ))
}

fn validate_meta_compile_post_state(
    validation_paths: &[PathBuf],
    context: &WorkspaceContext,
) -> Result<(), String> {
    for path in validation_paths {
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let xml = read_utf8_sig(path)?;
        let document = Document::parse(xml.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
        if document.root_element().tag_name().name() == "MetaDataObject" {
            validate_metadata_owner_shape_8_3_27(path, context, "meta.compile")?;
        }
    }
    Ok(())
}

pub(super) fn publish_meta_compile(
    planned: Result<MetaCompilePlan, String>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = planned.and_then(
        |(stdout, mut transaction, validation_paths, config_owner, format_dependencies)| {
            let format_dependencies = format_dependencies
                .iter()
                .map(PathBuf::as_path)
                .collect::<Vec<_>>();
            guard_active_format_dependencies(&mut transaction, &format_dependencies, context)?;
            let report = transaction.commit_with_post_validation(|| {
                if let Some(config_owner) = config_owner.as_deref() {
                    require_meta_configuration_owner_validation(
                        config_owner,
                        context,
                        "meta.compile",
                    )?;
                }
                validate_meta_compile_post_state(&validation_paths, context)
            })?;
            let mut changes = report
                .created
                .iter()
                .map(|path| format!("created {}", path.display()))
                .collect::<Vec<_>>();
            changes.extend(
                report
                    .updated
                    .iter()
                    .map(|path| format!("updated {}", path.display())),
            );
            Ok((stdout, report.created, changes, report.cleanup_warnings))
        },
    );

    match write_result {
        Ok((stdout, artifacts, changes, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.compile completed with native metadata compiler".to_string(),
            changes,
            warnings,
            errors: Vec::new(),
            artifacts: artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.compile failed in native metadata compiler".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(super) fn preview_prepared_meta_compile(
    planned: Result<MetaCompilePlan, String>,
) -> Result<AdapterOutcome, String> {
    let (_stdout, transaction, _validation_paths, _config_owner, _format_dependencies) = planned?;
    Ok(AdapterOutcome {
        ok: true,
        summary: "dry run: unica.meta.compile planned native metadata compilation".to_string(),
        changes: transaction.dry_run_changes(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: Some(transaction.dry_run_stdout()),
        stderr: None,
        command: None,
    })
}

#[cfg(test)]
mod typed_add_publication_tests {
    use super::*;
    use crate::application::metadata::MetaAddRequest;
    use crate::domain::metadata::MetadataKind;
    use crate::infrastructure::native_operations::cf::create_configuration_scaffold;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_commit_failpoint, CommitFailpoint,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

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
                dry_run: false,
            }
        }

        fn seed_common_module(&self, name: &str, module: &[u8]) {
            let (xml, _) = super::super::legacy_dsl::meta_compile_object_xml(
                &Map::new(),
                "CommonModule",
                name,
                "2.20",
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
            fn visit(root: &Path, current: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
                let mut entries = fs::read_dir(current)
                    .unwrap()
                    .map(|entry| entry.unwrap())
                    .collect::<Vec<_>>();
                entries.sort_by_key(|entry| entry.file_name());
                for entry in entries {
                    let path = entry.path();
                    if entry.file_type().unwrap().is_dir() {
                        visit(root, &path, output);
                    } else {
                        output.insert(
                            path.strip_prefix(root).unwrap().to_path_buf(),
                            fs::read(path).unwrap(),
                        );
                    }
                }
            }
            let source = self.root.join("src");
            let mut output = BTreeMap::new();
            visit(&source, &source, &mut output);
            output
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
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
    fn meta_add_validation_subject_contains_auxiliary_xml_and_guarded_bsl_images() {
        let fixture = Fixture::new("complete-post-image");
        let cancellation = CancellationToken::new();
        let exchange = prepare_meta_add(
            &MetaAddRequest {
                source_set: "main".to_string(),
                kind: MetadataKind::ExchangePlan,
                name: "Sync".to_string(),
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
