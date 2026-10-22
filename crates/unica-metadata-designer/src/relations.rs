//! Existing final relation graph and source dependency policy; admission stays composition.
use crate::resource_plan::{TypedDependencyModule, TypedRelationDependency};
use roxmltree::Document;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use unica_address::source_target::{
    MetadataAddress, ResolvedTarget, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use unica_metadata::{
    validate_metadata_relation_target_profile, MetaDiagnostic, MetaDiagnosticCode,
    MetaEditOperation, MetaRelation, MetadataKind,
};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_designer::owner::PlatformXmlResourceEvidence;
/// Each method is one original read/binding phase, with no write operation.
pub trait MetadataDependencyRead {
    type Guard;
    fn resolve_object(
        &self,
        target: &SourceTarget,
    ) -> Result<(ResolvedTarget, Self::Guard), crate::resource_plan::MetadataReadFailure>;
    fn resolve_module(
        &self,
        target: &SourceTarget,
    ) -> Result<(ResolvedTarget, Self::Guard), crate::resource_plan::MetadataReadFailure>;
    fn resource_evidence(
        &self,
        guard: &Self::Guard,
    ) -> Result<PlatformXmlResourceEvidence, crate::resource_plan::MetadataReadFailure>;
    fn ordinary_read(&self) -> &dyn ProjectDiscoveryRead;
}
pub struct TypedOperationDependencyScope<'a> {
    pub(crate) source_set: &'a str,
    pub(crate) owner_source_root: &'a Path,
    pub(crate) owner_registration_path: &'a Path,
    pub(crate) owner_registration_preimage: &'a [u8],
    pub(crate) include_handler_dependency: bool,
}

impl<'a> TypedOperationDependencyScope<'a> {
    pub fn new(
        source_set: &'a str,
        owner_source_root: &'a Path,
        owner_registration_path: &'a Path,
        owner_registration_preimage: &'a [u8],
        include_handler_dependency: bool,
    ) -> Self {
        Self {
            source_set,
            owner_source_root,
            owner_registration_path,
            owner_registration_preimage,
            include_handler_dependency,
        }
    }
}

pub fn resolve_typed_relation_dependencies<G>(
    dependency_scope: &TypedOperationDependencyScope<'_>,
    metadata_path: &MetadataAddress,
    operations: &[MetaEditOperation],
    reader: &dyn MetadataDependencyRead<Guard = G>,
    owner_post_image: &str,
) -> Result<Vec<TypedRelationDependency<G>>, unica_metadata::request::MetaFailure> {
    let mut dependencies = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut request_origins = BTreeMap::new();
    for (operation_index, operation) in operations.iter().enumerate() {
        let MetaEditOperation::EditRelations {
            relation, targets, ..
        } = operation
        else {
            continue;
        };
        for (target_index, target) in targets.iter().enumerate() {
            request_origins.insert(
                (relation.as_str(), target.wire_value().to_string()),
                (operation_index, target_index),
            );
        }
    }
    let final_graph = parse_typed_final_relation_graph(owner_post_image, metadata_path)?;
    let mut source_prefixes = BTreeMap::<String, BTreeSet<String>>::new();
    for (_, relation_index, target) in &final_graph {
        let unica_metadata::MetaRelationTarget::EventSource(source) = target else {
            continue;
        };
        let Some(dependency) = source.metadata_path() else {
            continue;
        };
        let generated_prefix =
            crate::xml_model::event_source_generated_prefix(source).map_err(|message| {
                unica_metadata::request::MetaFailure::from(
                    crate::edit::typed_diagnostic(
                        MetaDiagnosticCode::InvalidArguments,
                        message,
                        Some(&format!("relations.source[{relation_index}]")),
                    )
                    .with_metadata_path(metadata_path.clone()),
                )
            })?;
        source_prefixes
            .entry(dependency.as_str().to_string())
            .or_default()
            .insert(generated_prefix.to_string());
    }
    for (relation, relation_index, target) in final_graph {
        let origin = request_origins
            .get(&(relation.as_str(), target.wire_value().to_string()))
            .copied();
        let final_field = format!("relations.{}[{relation_index}]", relation.as_str());
        let diagnostic_field = origin.map_or_else(
            || final_field.clone(),
            |(operation_index, target_index)| {
                format!("operations[{operation_index}].targets[{target_index}]")
            },
        );
        validate_typed_relation_target(metadata_path, owner_post_image, relation, &target)
            .map_err(|mut diagnostic| {
                diagnostic.operation_index = origin.map(|(operation_index, _)| operation_index);
                diagnostic.field = Some(diagnostic_field.clone());
                unica_metadata::request::MetaFailure::from(
                    diagnostic.with_metadata_path(metadata_path.clone()),
                )
            })?;
        let Some(dependency) = target.dependency() else {
            continue;
        };
        if dependency == metadata_path || !seen.insert(dependency.clone()) {
            continue;
        }
        let source_target = SourceTarget {
            source_set: dependency_scope.source_set.to_string(),
            metadata_path: Some(dependency.clone()),
        };
        let resolution = reader.resolve_object(&source_target).map_err(|_| {
            let mut diagnostic = crate::edit::typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                "relation target does not resolve in the selected source set",
                Some(&diagnostic_field),
            );
            diagnostic.operation_index = origin.map(|(index, _)| index);
            unica_metadata::request::MetaFailure::from(
                diagnostic.with_metadata_path(metadata_path.clone()),
            )
        })?;
        if resolution.0.target_kind != TargetKind::MetadataObject {
            let mut diagnostic = crate::edit::typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "relation target must identify a metadata object",
                Some(&diagnostic_field),
            );
            diagnostic.operation_index = origin.map(|(index, _)| index);
            return Err(unica_metadata::request::MetaFailure::from(
                diagnostic.with_metadata_path(metadata_path.clone()),
            ));
        }
        let evidence = reader.resource_evidence(&resolution.1).map_err(|_| {
            let mut diagnostic = crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "relation target evidence is unavailable",
                Some(&diagnostic_field),
            );
            diagnostic.operation_index = origin.map(|(index, _)| index);
            unica_metadata::request::MetaFailure::from(
                diagnostic.with_metadata_path(metadata_path.clone()),
            )
        })?;
        let bytes =
            unica_source_designer::read::read_bytes(reader.ordinary_read(), &evidence.target_path)
                .map_err(|_| {
                    let mut diagnostic = crate::edit::typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "relation target pre-image is unavailable",
                        Some(&diagnostic_field),
                    );
                    diagnostic.operation_index = origin.map(|(index, _)| index);
                    unica_metadata::request::MetaFailure::from(
                        diagnostic.with_metadata_path(metadata_path.clone()),
                    )
                })?;
        if relation == MetaRelation::Source {
            if evidence.source_root != dependency_scope.owner_source_root
                || evidence.registration_path != dependency_scope.owner_registration_path
            {
                let mut diagnostic = crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "EventSubscription Source dependency must belong to the exact owner source set",
                    Some(&diagnostic_field),
                );
                diagnostic.operation_index = origin.map(|(index, _)| index);
                return Err(unica_metadata::request::MetaFailure::from(
                    diagnostic.with_metadata_path(metadata_path.clone()),
                ));
            }
            let (
                registration_kind,
                registration_name,
                descriptor_kind,
                descriptor_name,
                generated_name,
            ) = crate::validation_context::event_source_dependency_contract(dependency).map_err(
                |_| {
                    let mut diagnostic = crate::edit::typed_diagnostic(
                        MetaDiagnosticCode::InvalidArguments,
                        "EventSubscription Source dependency must identify one metadata object",
                        Some(&diagnostic_field),
                    );
                    diagnostic.operation_index = origin.map(|(index, _)| index);
                    unica_metadata::request::MetaFailure::from(
                        diagnostic.with_metadata_path(metadata_path.clone()),
                    )
                },
            )?;
            crate::validation_context::validate_event_source_registration(
                dependency_scope.owner_registration_preimage,
                registration_kind,
                registration_name,
            )
            .and_then(|()| {
                let prefixes = source_prefixes
                    .get(dependency.as_str())
                    .map(|prefixes| prefixes.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                crate::validation_context::validate_event_source_dependency_descriptor(
                    &bytes,
                    descriptor_kind,
                    descriptor_name,
                    generated_name.as_deref(),
                    &prefixes,
                )
            })
            .map_err(|message| {
                let mut diagnostic = crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    message,
                    Some(&diagnostic_field),
                );
                diagnostic.operation_index = origin.map(|(index, _)| index);
                unica_metadata::request::MetaFailure::from(
                    diagnostic.with_metadata_path(metadata_path.clone()),
                )
            })?;
        }
        dependencies.push(TypedRelationDependency {
            handle: resolution.1,
            path: evidence.target_path,
            bytes,
            target: dependency.clone(),
            modules: Vec::new(),
        });
    }
    let defined_types = dependencies
        .iter()
        .filter(|dependency| dependency.target.segments().next() == Some("DefinedType"))
        .map(|dependency| dependency.target.clone())
        .collect::<Vec<_>>();
    let mut expanded = std::collections::HashSet::new();
    for defined_type in defined_types {
        expand_defined_type_event_dependencies(
            dependency_scope,
            metadata_path,
            reader,
            &defined_type,
            &mut dependencies,
            &mut seen,
            &mut expanded,
            &mut std::collections::HashSet::new(),
        )?;
    }
    if dependency_scope.include_handler_dependency
        && metadata_path.segments().next() == Some("EventSubscription")
    {
        let handler = resolve_typed_event_subscription_handler_dependency(
            dependency_scope.source_set,
            dependency_scope.owner_source_root,
            dependency_scope.owner_registration_path,
            metadata_path,
            owner_post_image,
            reader,
        )?;
        if !seen.contains(&handler.target) {
            dependencies.push(handler);
        }
    }
    Ok(dependencies)
}

#[allow(clippy::too_many_arguments)]
pub fn expand_defined_type_event_dependencies<G>(
    dependency_scope: &TypedOperationDependencyScope<'_>,
    owner: &MetadataAddress,
    reader: &dyn MetadataDependencyRead<Guard = G>,
    defined_type: &MetadataAddress,
    dependencies: &mut Vec<TypedRelationDependency<G>>,
    seen: &mut std::collections::HashSet<MetadataAddress>,
    expanded: &mut std::collections::HashSet<MetadataAddress>,
    visiting: &mut std::collections::HashSet<MetadataAddress>,
) -> Result<(), unica_metadata::request::MetaFailure> {
    if expanded.contains(defined_type) {
        return Ok(());
    }
    if !visiting.insert(defined_type.clone()) {
        return Err(defined_type_event_failure(
            owner,
            format!("DefinedType event source cycle contains `{defined_type}`"),
            MetaDiagnosticCode::ValidationFailed,
        ));
    }
    let bytes = dependencies
        .iter()
        .find(|dependency| &dependency.target == defined_type)
        .map(|dependency| dependency.bytes.clone())
        .ok_or_else(|| {
            defined_type_event_failure(
                owner,
                format!("DefinedType event source `{defined_type}` evidence is unavailable"),
                MetaDiagnosticCode::ProviderUnavailable,
            )
        })?;
    let members =
        crate::xml_model::parse_defined_type_event_sources(&bytes).map_err(|message| {
            defined_type_event_failure(owner, message, MetaDiagnosticCode::ValidationFailed)
        })?;
    for member in members {
        let source_class = member.event_source_class().map_err(|message| {
            defined_type_event_failure(owner, message, MetaDiagnosticCode::ValidationFailed)
        })?;
        let Some(target) = member.metadata_path().cloned() else {
            if source_class.is_some() {
                continue;
            }
            return Err(defined_type_event_failure(
                owner,
                "DefinedType member has no event source class",
                MetaDiagnosticCode::ValidationFailed,
            ));
        };
        if visiting.contains(&target) {
            return Err(defined_type_event_failure(
                owner,
                format!("DefinedType event source cycle contains `{target}`"),
                MetaDiagnosticCode::ValidationFailed,
            ));
        }
        if seen.insert(target.clone()) {
            let source_target = SourceTarget {
                source_set: dependency_scope.source_set.to_string(),
                metadata_path: Some(target.clone()),
            };
            let resolution = reader.resolve_object(&source_target).map_err(|_| {
                defined_type_event_failure(
                    owner,
                    format!(
                        "DefinedType member `{target}` does not resolve in the selected source set"
                    ),
                    MetaDiagnosticCode::TargetNotFound,
                )
            })?;
            let evidence = reader.resource_evidence(&resolution.1).map_err(|_| {
                defined_type_event_failure(
                    owner,
                    format!("DefinedType member `{target}` evidence is unavailable"),
                    MetaDiagnosticCode::ProviderUnavailable,
                )
            })?;
            if evidence.source_root != dependency_scope.owner_source_root
                || evidence.registration_path != dependency_scope.owner_registration_path
            {
                return Err(defined_type_event_failure(
                    owner,
                    "DefinedType member must belong to the exact owner source set",
                    MetaDiagnosticCode::InvalidArguments,
                ));
            }
            let member_bytes = unica_source_designer::read::read_bytes(
                reader.ordinary_read(),
                &evidence.target_path,
            )
            .map_err(|_| {
                defined_type_event_failure(
                    owner,
                    format!("DefinedType member `{target}` pre-image is unavailable"),
                    MetaDiagnosticCode::ProviderUnavailable,
                )
            })?;
            let (
                registration_kind,
                registration_name,
                descriptor_kind,
                descriptor_name,
                generated_name,
            ) = crate::validation_context::event_source_dependency_contract(&target).map_err(
                |message| {
                    defined_type_event_failure(owner, message, MetaDiagnosticCode::InvalidArguments)
                },
            )?;
            let generated_prefix = crate::xml_model::event_source_generated_prefix(&member)
                .map_err(|message| {
                    defined_type_event_failure(owner, message, MetaDiagnosticCode::InvalidArguments)
                })?;
            crate::validation_context::validate_event_source_registration(
                dependency_scope.owner_registration_preimage,
                registration_kind,
                registration_name,
            )
            .and_then(|()| {
                crate::validation_context::validate_event_source_dependency_descriptor(
                    &member_bytes,
                    descriptor_kind,
                    descriptor_name,
                    generated_name.as_deref(),
                    &[generated_prefix.to_string()],
                )
            })
            .map_err(|message| {
                defined_type_event_failure(owner, message, MetaDiagnosticCode::ValidationFailed)
            })?;
            dependencies.push(TypedRelationDependency {
                handle: resolution.1,
                path: evidence.target_path,
                bytes: member_bytes,
                target: target.clone(),
                modules: Vec::new(),
            });
        }
        if target.segments().next() == Some("DefinedType") {
            expand_defined_type_event_dependencies(
                dependency_scope,
                owner,
                reader,
                &target,
                dependencies,
                seen,
                expanded,
                visiting,
            )?;
        }
    }
    visiting.remove(defined_type);
    expanded.insert(defined_type.clone());
    Ok(())
}

fn defined_type_event_failure(
    owner: &MetadataAddress,
    message: impl Into<String>,
    code: MetaDiagnosticCode,
) -> unica_metadata::request::MetaFailure {
    unica_metadata::request::MetaFailure::from(
        crate::edit::typed_diagnostic(code, message, Some("relations.source"))
            .with_metadata_path(owner.clone()),
    )
}

pub fn resolve_typed_event_subscription_handler_dependency<G>(
    source_set: &str,
    owner_source_root: &Path,
    owner_registration_path: &Path,
    metadata_path: &MetadataAddress,
    owner_post_image: &str,
    reader: &dyn MetadataDependencyRead<Guard = G>,
) -> Result<TypedRelationDependency<G>, unica_metadata::request::MetaFailure> {
    let document =
        Document::parse(owner_post_image.trim_start_matches('\u{feff}')).map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "EventSubscription post-image is not valid XML",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    let source =
        crate::xml_model::meta_event_subscription_source_node(&document).map_err(|message| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    message,
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    let properties = source.parent().ok_or_else(|| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "EventSubscription Properties are unavailable",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let handler = unica_xml_format::read::child_direct_text(properties, "Handler")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "EventSubscription Handler is empty",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    let parts = handler.trim().split('.').collect::<Vec<_>>();
    let module_name = match parts.as_slice() {
        ["CommonModule", module, _] => *module,
        _ => {
            return Err(unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ValidationFailed,
                    "EventSubscription Handler must identify a CommonModule procedure",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            ))
        }
    };
    let target = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("CommonModule.{module_name}"),
    )
    .map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ValidationFailed,
                "EventSubscription Handler CommonModule is invalid",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let source_target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(target.clone()),
    };
    let resolution = reader.resolve_object(&source_target)
        .map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::TargetNotFound,
                    "EventSubscription Handler CommonModule does not resolve in the selected source set",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    if resolution.0.target_kind != TargetKind::MetadataObject {
        return Err(unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Handler must resolve to a metadata object",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        ));
    }
    let evidence = reader.resource_evidence(&resolution.1).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "EventSubscription Handler evidence is unavailable",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    if evidence.source_root != owner_source_root
        || evidence.registration_path != owner_registration_path
    {
        return Err(unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Handler must belong to the exact owner source set",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        ));
    }
    let bytes =
        unica_source_designer::read::read_bytes(reader.ordinary_read(), &evidence.target_path)
            .map_err(|_| {
                unica_metadata::request::MetaFailure::from(
                    crate::edit::typed_diagnostic(
                        MetaDiagnosticCode::ProviderUnavailable,
                        "EventSubscription Handler descriptor pre-image is unavailable",
                        Some("properties.handler"),
                    )
                    .with_metadata_path(metadata_path.clone()),
                )
            })?;
    let module_address = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("CommonModule.{module_name}.Module"),
    )
    .expect("a registered common module has a canonical module address");
    let module_source_target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(module_address),
    };
    let module_resolution = reader.resolve_module(&module_source_target).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                "EventSubscription Handler module does not resolve in the selected source set",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let module_evidence = reader
        .resource_evidence(&module_resolution.1)
        .map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "EventSubscription Handler module evidence is unavailable",
                    Some("properties.handler"),
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    if module_evidence.source_root != owner_source_root
        || module_evidence.registration_path != owner_registration_path
        || evidence.module_paths.as_slice() != [module_evidence.target_path.as_path()]
    {
        return Err(unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                "EventSubscription Handler must have exactly one module in the owner source set",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        ));
    }
    let module_bytes = unica_source_designer::read::read_bytes(
        reader.ordinary_read(),
        &module_evidence.target_path,
    )
    .map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "EventSubscription Handler module pre-image is unavailable",
                Some("properties.handler"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let modules = vec![TypedDependencyModule {
        handle: module_resolution.1,
        path: module_evidence.target_path,
        bytes: module_bytes,
    }];
    Ok(TypedRelationDependency {
        handle: resolution.1,
        path: evidence.target_path,
        bytes,
        target,
        modules,
    })
}

pub fn parse_typed_final_relation_graph(
    owner_post_image: &str,
    owner: &MetadataAddress,
) -> Result<
    Vec<(MetaRelation, usize, unica_metadata::MetaRelationTarget)>,
    unica_metadata::request::MetaFailure,
> {
    let document =
        Document::parse(owner_post_image.trim_start_matches('\u{feff}')).map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "owner post-image is not valid XML",
                    Some("relations"),
                )
                .with_metadata_path(owner.clone()),
            )
        })?;
    let object = crate::edit::meta_edit_object_node(&document).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "owner post-image object is unavailable",
                Some("relations"),
            )
            .with_metadata_path(owner.clone()),
        )
    })?;
    let properties =
        unica_xml_format::read::child_element(object, "Properties").ok_or_else(|| {
            unica_metadata::request::MetaFailure::from(
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "owner post-image properties are unavailable",
                    Some("relations"),
                )
                .with_metadata_path(owner.clone()),
            )
        })?;
    let mut graph = Vec::new();
    for (relation, tag) in [
        (MetaRelation::Owners, "Owners"),
        (MetaRelation::RegisterRecords, "RegisterRecords"),
        (MetaRelation::BasedOn, "BasedOn"),
        (MetaRelation::InputByString, "InputByString"),
    ] {
        let Some(container) = unica_xml_format::read::child_element(properties, tag) else {
            continue;
        };
        for (index, item) in container
            .children()
            .filter(|node| node.is_element())
            .enumerate()
        {
            let value = item.text().unwrap_or_default().trim();
            let target = if relation == MetaRelation::InputByString {
                unica_metadata::MetaRelationTarget::Field(
                    unica_metadata::MetadataFieldPath::parse(value).map_err(|mut diagnostic| {
                        diagnostic.field =
                            Some(format!("relations.{}[{index}]", relation.as_str()));
                        unica_metadata::request::MetaFailure::from(
                            diagnostic.with_metadata_path(owner.clone()),
                        )
                    })?,
                )
            } else {
                let metadata_path = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value)
                    .map_err(|_| {
                        unica_metadata::request::MetaFailure::from(
                            crate::edit::typed_diagnostic(
                                MetaDiagnosticCode::ValidationFailed,
                                "final relation target is not a metadata address",
                                Some(&format!("relations.{}[{index}]", relation.as_str())),
                            )
                            .with_metadata_path(owner.clone()),
                        )
                    })?;
                unica_metadata::MetaRelationTarget::Object(unica_metadata::MetadataReference {
                    metadata_path,
                })
            };
            graph.push((relation, index, target));
        }
    }
    if owner.segments().next() == Some("EventSubscription") {
        let source = crate::xml_model::meta_event_subscription_source_node(&document)
            .and_then(crate::xml_model::parse_meta_event_subscription_source)
            .map_err(|message| {
                unica_metadata::request::MetaFailure::from(
                    crate::edit::typed_diagnostic(
                        MetaDiagnosticCode::ValidationFailed,
                        message,
                        Some("relations.source"),
                    )
                    .with_metadata_path(owner.clone()),
                )
            })?;
        // `v8:TypeId` members name no metadata in this source set; they are
        // kept in place and are not relation targets.
        for (index, source) in source.sources.into_iter().enumerate() {
            graph.push((
                MetaRelation::Source,
                index,
                unica_metadata::MetaRelationTarget::EventSource(source),
            ));
        }
    }
    Ok(graph)
}

pub fn validate_typed_relation_target(
    owner: &MetadataAddress,
    owner_post_image: &str,
    relation: MetaRelation,
    target: &unica_metadata::MetaRelationTarget,
) -> Result<(), MetaDiagnostic> {
    let owner_kind = owner
        .segments()
        .next()
        .and_then(|kind| MetadataKind::parse(kind).ok())
        .ok_or_else(|| {
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata owner kind is unavailable",
                Some("targets"),
            )
        })?;
    validate_metadata_relation_target_profile(owner_kind, owner, relation, target)?;
    if relation == MetaRelation::InputByString {
        let unica_metadata::MetaRelationTarget::Field(field) = target else {
            unreachable!("domain relation profile rejects non-field inputByString targets")
        };
        let target_kind = owner_kind.as_str();
        if field.kind == unica_metadata::MetadataFieldKind::Attribute {
            let document = Document::parse(owner_post_image).map_err(|_| {
                crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "owner post-image is not valid XML",
                    Some("targets"),
                )
            })?;
            let found = document
                .descendants()
                .filter(|node| node.is_element() && node.tag_name().name() == "Attribute")
                .any(|node| {
                    crate::edit::meta_edit_child_object_name(node).as_deref() == Some(&field.name)
                });
            if !found {
                return Err(crate::edit::typed_diagnostic(
                    MetaDiagnosticCode::TargetNotFound,
                    "inputByString attribute does not exist in the owner post-image",
                    Some("targets"),
                ));
            }
        } else if !crate::template_catalog::metadata_standard_attribute_names(target_kind)
            .contains(&field.name.as_str())
        {
            return Err(crate::edit::typed_diagnostic(
                MetaDiagnosticCode::TargetNotFound,
                "inputByString standard attribute does not exist for the owner kind",
                Some("targets"),
            ));
        }
    }
    Ok(())
}
