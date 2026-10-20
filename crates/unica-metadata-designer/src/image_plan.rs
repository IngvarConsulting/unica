//! Complete private metadata descriptor/child post-image planning over issued observations.

use crate::edit::*;
use crate::relations::{MetadataDependencyRead, TypedOperationDependencyScope};
use crate::resource_plan::{TypedChildResourcePlan, TypedOperationPostImage};
use std::path::{Path, PathBuf};

use unica_metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation, MetaRelation, MetadataKind,
};
use unica_source_contract::proposal::DirectoryMembershipSelector;
use unica_xml_format::text::{
    resolve_observed_line_ending, LineEnding, SourceTextSnapshot, Utf8Bom,
};
/// Source selection and single-file secure reading remain actual composition.
pub trait MetadataEditRead: MetadataDependencyRead {
    fn predefined_source_root(
        &self,
        source_set: &str,
    ) -> Result<PathBuf, crate::resource_plan::MetadataReadFailure>;
    fn predefined_read(&self) -> &dyn crate::predefined::PredefinedResourceRead;
}
#[derive(Clone, Copy)]
struct MetaEditSourceFormat {
    has_bom: bool,
    eol: LineEnding,
}

fn meta_edit_preserve_source_format(text: &str, format: MetaEditSourceFormat) -> Vec<u8> {
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let serialized = normalized.replace('\n', format.eol.as_str());
    let mut bytes = Vec::with_capacity(serialized.len() + usize::from(format.has_bom) * 3);
    if format.has_bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(serialized.as_bytes());
    bytes
}

fn meta_edit_preserve_existing_eol(text: &str, has_bom: bool) -> Vec<u8> {
    let source = text.trim_start_matches('\u{feff}');
    let mut bytes = Vec::with_capacity(source.len() + usize::from(has_bom) * 3);
    if has_bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(source.as_bytes());
    bytes
}

/// The `LanguageCode` new multilingual values of `operations` are written in.
///
/// `owner_image` is the source set's root descriptor: `Configuration.xml` of
/// a configuration or an extension, or the root of an external processor or
/// report; `read` returns another file of that source set by its relative
/// path. A configuration that does not name a readable default language is
/// refused only when the operations do write a multilingual value; otherwise
/// the language is not used and comes back empty.
pub fn typed_operations_text_language(
    operations: &[MetaEditOperation],
    owner_image: &[u8],
    read: impl FnOnce(&Path) -> Result<Option<Vec<u8>>, String>,
) -> Result<String, MetaDiagnostic> {
    if !crate::edit::typed_operations_write_multilingual_text(operations) {
        return Ok(String::new());
    }
    source_set_text_language(owner_image, read)
}

/// The `LanguageCode` every new multilingual value of a source set is
/// written in, read from its root descriptor `owner_image` and, through
/// `read`, its `Languages/<Name>.xml`. External processors and reports get
/// [`EXTERNAL_SOURCE_TEXT_LANGUAGE`].
pub fn source_set_text_language(
    owner_image: &[u8],
    read: impl FnOnce(&Path) -> Result<Option<Vec<u8>>, String>,
) -> Result<String, MetaDiagnostic> {
    let external = unica_xml_format::read::parse_metadata_image(owner_image)
        .ok()
        .and_then(|(_, document)| {
            document
                .root_element()
                .children()
                .find(roxmltree::Node::is_element)
                .map(|node| {
                    matches!(
                        node.tag_name().name(),
                        "ExternalDataProcessor" | "ExternalReport"
                    )
                })
        })
        .unwrap_or(false);
    if external {
        return Ok(unica_source_designer::language::EXTERNAL_SOURCE_TEXT_LANGUAGE.to_string());
    }
    unica_source_designer::language::default_text_language(owner_image, read).map_err(|message| {
        crate::edit::typed_diagnostic(
            MetaDiagnosticCode::ValidationFailed,
            format!("the configuration default language is unavailable: {message}"),
            None,
        )
    })
}

/// Required emitters/entropy, invoked only at their existing planning branches.
#[derive(Clone, Copy)]
pub struct MetadataImageBindings {
    pub resources: crate::child_resources::ChildResourceBindings,
    pub fresh_uuid: fn() -> String,
}

/// Build the complete descriptor and child-resource plan privately. No
/// transaction mutation is registered until this function returns success.
pub fn build_typed_operation_post_image<G>(
    dependency_scope: TypedOperationDependencyScope<'_>,
    descriptor: crate::resource_plan::MetadataDescriptorTarget<'_>,
    descriptor_preimage: &[u8],
    operations: &[MetaEditOperation],
    reader: &dyn MetadataEditRead<Guard = G>,
    bindings: MetadataImageBindings,
) -> Result<TypedOperationPostImage<G>, unica_metadata::request::MetaFailure> {
    let crate::resource_plan::MetadataDescriptorTarget {
        path: descriptor_path,
        address: target,
    } = descriptor;
    let MetadataImageBindings {
        resources,
        fresh_uuid,
    } = bindings;
    let crate::child_resources::ChildResourceBindings {
        observe_membership,
        templates,
        form_seed,
    } = resources;

    let source_only = !operations.is_empty()
        && operations.iter().all(|operation| {
            matches!(
                operation,
                MetaEditOperation::EditRelations {
                    relation: MetaRelation::Source,
                    ..
                }
            )
        });
    let snapshot = SourceTextSnapshot::from_bytes(descriptor_preimage).map_err(|error| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                format!("metadata descriptor snapshot failed: {error}"),
                None,
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    // Смешанный профиль — дефект содержимого источника, а не отказ провайдера:
    // байты прочитаны и декодированы, но их вид не проходит проверку writer-а,
    // как и неканоничные fill-значения ниже по файлу.
    let eol = resolve_observed_line_ending(&snapshot, None).map_err(|error| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                format!("metadata descriptor EOL policy failed: {error}"),
                None,
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    let source_format = MetaEditSourceFormat {
        has_bom: snapshot.bom() == Utf8Bom::Present,
        eol,
    };
    let mut xml = snapshot.text().to_string();
    let normalized_preimage = xml.clone();
    let lang = typed_operations_text_language(
        operations,
        dependency_scope.owner_registration_preimage,
        |relative| {
            let path = dependency_scope.owner_source_root.join(relative);
            match unica_source_designer::read::read_bytes(reader.ordinary_read(), &path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(format!("failed to read {}: {error}", path.display())),
            }
        },
    )
    .map_err(|diagnostic| {
        unica_metadata::request::MetaFailure::from(diagnostic.with_metadata_path(target.clone()))
    })?;
    expand_standalone_children_for_operations(
        reader.ordinary_read(),
        &mut xml,
        descriptor_path,
        operations,
    );
    let mut next_uuid = fresh_uuid;
    let applied = apply_typed_operations_with_uuid(&mut xml, operations, &lang, &mut next_uuid)
        .map_err(|mut failure| {
            for diagnostic in &mut failure.diagnostics {
                diagnostic.metadata_path = Some(target.clone());
            }
            failure
        })?;
    restore_initial_empty_top_child_objects(&normalized_preimage, &mut xml);
    let (object_kind, object_name) =
        crate::edit::meta_edit_object_identity(&xml).map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "typed metadata post-image identity is unavailable",
                    None,
                )
                .with_metadata_path(target.clone()),
            )
        })?;
    let mut child_resources = crate::child_resources::plan_typed_child_resources::<G>(
        reader.ordinary_read(),
        crate::child_resources::ChildResourceBindings {
            observe_membership,
            templates,
            form_seed,
        },
        crate::resource_plan::MetadataDescriptorTarget {
            path: descriptor_path,
            address: target,
        },
        &object_kind,
        &object_name,
        operations,
        &xml,
    )?;
    // #1325: описания форм и макетов уже вынесены планом в отдельные файлы;
    // во владельце платформа ждёт только имя. Встроенное описание приводит
    // ibcmd 8.3.27 к бесконечному импорту.
    collapse_standalone_child_descriptors(&normalized_preimage, &mut xml);
    // Вид вне закрытого набора по-прежнему пропускает проверки, которым нужен
    // typed-профиль; жёстко он требуется только предопределённым
    // данным ниже.
    let owner_kind = MetadataKind::parse(&object_kind).ok();
    // Разрешение набора исходников нужно только companion-writer-у
    // `Ext/Predefined.xml`; правка, не касающаяся предопределённых данных и
    // `CodeType`, не обязана его требовать.
    let planned_predefined =
        if crate::predefined::descriptor_edit_requires_predefined_planning(operations) {
            let owner_kind = owner_kind.ok_or_else(|| {
                unica_metadata::request::MetaFailure::from(
                    crate::edit::typed_diagnostic(
                        MetaDiagnosticCode::UnsupportedKind,
                        format!("metadata kind `{object_kind}` has no typed operation profile"),
                        None,
                    )
                    .with_metadata_path(target.clone()),
                )
            })?;
            let source_root = reader
                .predefined_source_root(dependency_scope.source_set)
                .map_err(|_| {
                    unica_metadata::request::MetaFailure::from(
                        crate::edit::typed_diagnostic(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata source set cannot be resolved for predefined data",
                            None,
                        )
                        .with_metadata_path(target.clone()),
                    )
                })?;
            crate::predefined::plan_predefined_resource_after_descriptor_edit::<G>(
                reader.predefined_read(),
                &source_root,
                crate::resource_plan::MetadataDescriptorTarget {
                    path: descriptor_path,
                    address: target,
                },
                owner_kind,
                &normalized_preimage,
                &xml,
                operations,
            )?
        } else {
            crate::predefined::PlannedPredefinedResource {
                resources: TypedChildResourcePlan::default(),
                effects: Vec::new(),
            }
        };
    let predefined_resources = planned_predefined.resources;
    child_resources
        .file_mutations
        .extend(predefined_resources.file_mutations);
    child_resources
        .absent_path_guards
        .extend(predefined_resources.absent_path_guards);
    child_resources
        .exact_file_guards
        .extend(predefined_resources.exact_file_guards);
    child_resources
        .directory_guards
        .extend(predefined_resources.directory_guards);
    child_resources
        .publication_plan
        .extend(predefined_resources.publication_plan);
    child_resources
        .validation_resources
        .extend(predefined_resources.validation_resources);
    child_resources
        .validation_footprints
        .extend(predefined_resources.validation_footprints);
    child_resources
        .expected_post_images
        .extend(predefined_resources.expected_post_images);
    // ADR-0072: справка — фасет владельца рядом с дескриптором; её файлы
    // планируются тем же post-image построением, что и остальные ресурсы.
    let help_resources = crate::help::plan_help_resource_after_descriptor_edit::<G>(
        reader.ordinary_read(),
        descriptor_path,
        target,
        &object_name,
        operations,
    )?;
    child_resources
        .file_mutations
        .extend(help_resources.file_mutations);
    child_resources
        .absent_path_guards
        .extend(help_resources.absent_path_guards);
    child_resources
        .exact_file_guards
        .extend(help_resources.exact_file_guards);
    child_resources
        .publication_plan
        .extend(help_resources.publication_plan);
    child_resources
        .expected_post_images
        .extend(help_resources.expected_post_images);
    child_resources.relation_dependencies = crate::relations::resolve_typed_relation_dependencies(
        &dependency_scope,
        target,
        operations,
        reader,
        &xml,
    )?;
    if target.segments().next() == Some("EventSubscription") {
        let directory = descriptor_path.parent().ok_or_else(|| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "EventSubscription collection topology is unavailable",
                    Some("relations.source"),
                )
                .with_metadata_path(target.clone()),
            )
        })?;
        let snapshot = observe_membership(directory, DirectoryMembershipSelector::XmlFiles)
            .map_err(|_| {
                unica_metadata::request::MetaFailure::from(
                    crate::edit::typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "EventSubscription collection topology is unavailable",
                        Some("relations.source"),
                    )
                    .with_metadata_path(target.clone()),
                )
            })?;
        child_resources.directory_guards.push((
            directory.to_path_buf(),
            DirectoryMembershipSelector::XmlFiles,
            snapshot,
        ));
    }
    // Итоговое состояние вызова, а не каждая операция по отдельности: замена
    // единственного измерения через `remove` вместе с `add` остаётся законной,
    // потому что промежуточная пустота здесь не наблюдается. Вид,
    // которого нет в закрытом наборе, условий не имеет и проверку пропускает.
    if let Some(owner_kind) = owner_kind {
        crate::integrity_check::check_meta_object_integrity(owner_kind, xml.as_bytes()).map_err(
            |diagnostic| {
                unica_metadata::request::MetaFailure::from(
                    diagnostic.with_metadata_path(target.clone()),
                )
            },
        )?;
    }
    let descriptor = if xml == normalized_preimage {
        descriptor_preimage.to_vec()
    } else if source_only {
        meta_edit_preserve_existing_eol(&xml, source_format.has_bom)
    } else {
        meta_edit_preserve_source_format(&xml, source_format)
    };
    let mut effects = applied.effects;
    effects.extend(planned_predefined.effects);
    effects.sort_by_key(|effect| effect.operation_index);
    Ok(TypedOperationPostImage {
        descriptor,
        child_resources,
        effects,
    })
}
