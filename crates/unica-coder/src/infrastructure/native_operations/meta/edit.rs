use unica_metadata_designer::relations::TypedOperationDependencyScope;
use unica_metadata_designer::resource_plan::ResolvedMetadataObject;

pub(super) fn template_seed_emitters(
) -> unica_metadata_designer::child_resources::TemplateSeedEmitters {
    unica_metadata_designer::child_resources::TemplateSeedEmitters {
        spreadsheet: unica_metadata_designer::planner::TemplateSeedEmitter(
            unica_templates_designer::mxl_root::empty_spreadsheet_document_xml,
        ),
        html_descriptor: unica_templates_designer::descriptor::html_template_descriptor,
        html_page: unica_templates_designer::descriptor::html_template_page,
        content_xml: unica_templates_designer::descriptor::template_content_xml,
    }
}
#[cfg(test)]
use unica_metadata_designer::edit::*;
use unica_metadata_designer::resource_plan::TypedOperationPostImage;
#[cfg(test)]
use unica_metadata_designer::resource_plan::{TypedChildObservation, TypedChildResourcePlan};

use crate::application::ports::PreparedMetadataMutation;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    TargetKindPolicy,
};
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
#[cfg(test)]
use roxmltree::Document;
use std::fs;
#[cfg(test)]
use std::path::{Path, PathBuf};
use unica_address::source_target::{MetadataAddress, SourceTarget, TargetKind};
use unica_metadata::MetaDiagnosticCode;
#[cfg(test)]
use unica_metadata::{
    MetaDiagnostic, MetaElementDefinition, MetaFillValue, MetaPublicationAction, MetadataKind,
    MetadataType,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;
use unica_work_control::cancellation::CancellationToken;

use super::publisher::PreparedMetaEdit;

use unica_workspace::compile_transaction::snapshot_directory_membership;

pub(crate) fn resolve_typed_edit_object(
    request: &unica_metadata::request::MetaEditRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<ResolvedMetadataObject<ClosedPlatformXmlTarget>, unica_metadata::request::MetaFailure> {
    resolve_typed_metadata_object(
        &request.source_set,
        &request.metadata_path,
        "edit",
        context,
        cancellation,
    )
}

pub(crate) fn resolve_typed_metadata_object(
    source_set: &str,
    metadata_path: &MetadataAddress,
    operation: &str,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<ResolvedMetadataObject<ClosedPlatformXmlTarget>, unica_metadata::request::MetaFailure> {
    if cancellation.is_cancelled() {
        return Err(unica_metadata_designer::edit::typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            format!("metadata {operation} was cancelled before source resolution"),
            None,
        )
        .with_metadata_path(metadata_path.clone())
        .into());
    }
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(metadata_path.clone()),
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| typed_resolution_failure(metadata_path, operation, error.code))?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(unica_metadata_designer::edit::typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "metadataPath must identify one existing metadata object",
            Some("metadataPath"),
        )
        .with_metadata_path(metadata_path.clone())
        .into());
    }
    let resolved_metadata_path = resolution.resolved.metadata_path.clone().ok_or_else(|| {
        unica_metadata::request::MetaFailure::from(
            unica_metadata_designer::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "resolved metadata object has no logical identity",
                Some("metadataPath"),
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let evidence = platform_xml_resource_evidence(context, &resolution.handle)
        .map_err(|error| typed_resolution_failure(metadata_path, operation, error.code))?;
    let descriptor_preimage = fs::read(&evidence.target_path).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            unica_metadata_designer::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor image is unavailable",
                None,
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let owner_preimage = fs::read(&evidence.registration_path).map_err(|_| {
        unica_metadata::request::MetaFailure::from(
            unica_metadata_designer::edit::typed_diagnostic(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata owner image is unavailable",
                None,
            )
            .with_metadata_path(metadata_path.clone()),
        )
    })?;
    let (_, descriptor) = unica_xml_format::read::parse_metadata_image(&descriptor_preimage)
        .map_err(|_| {
            unica_metadata::request::MetaFailure::from(
                unica_metadata_designer::edit::typed_diagnostic(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata descriptor image is unreadable",
                    None,
                )
                .with_metadata_path(metadata_path.clone()),
            )
        })?;
    if descriptor.root_element().attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format) {
        return Err(unica_metadata_designer::edit::typed_diagnostic(
            MetaDiagnosticCode::CapabilityUnavailable,
            format!(
                "metadata object is outside the supported {} export format",
                ACTIVE_FORMAT_PROFILE.export_format
            ),
            None,
        )
        .with_metadata_path(metadata_path.clone())
        .into());
    }
    Ok(ResolvedMetadataObject {
        handle: resolution.handle,
        resolved_target: resolution.resolved,
        metadata_path: resolved_metadata_path,
        descriptor_path: evidence.target_path,
        descriptor_preimage,
        source_root: evidence.source_root,
        owner_path: evidence.registration_path,
        owner_preimage,
    })
}

fn typed_resolution_failure(
    metadata_path: &MetadataAddress,
    operation: &str,
    code: unica_address::source_target::SourceTargetErrorCode,
) -> unica_metadata::request::MetaFailure {
    use unica_address::source_target::SourceTargetErrorCode;
    let diagnostic_code = match code {
        SourceTargetErrorCode::SourceSetNotFound
        | SourceTargetErrorCode::MetadataAddressNotFound => MetaDiagnosticCode::TargetNotFound,
        SourceTargetErrorCode::MetadataAddressInvalid
        | SourceTargetErrorCode::TargetKindMismatch
        | SourceTargetErrorCode::SourceSetRequired => MetaDiagnosticCode::InvalidArguments,
        SourceTargetErrorCode::ContainmentDenied => MetaDiagnosticCode::ProviderUnavailable,
        _ => MetaDiagnosticCode::CapabilityUnavailable,
    };
    unica_metadata_designer::edit::typed_diagnostic(
        diagnostic_code,
        match diagnostic_code {
            MetaDiagnosticCode::TargetNotFound => "metadata target was not found",
            MetaDiagnosticCode::InvalidArguments => "metadata target is not a mutable object",
            MetaDiagnosticCode::ProviderUnavailable => {
                "metadata target could not be resolved safely"
            }
            _ => match operation {
                "remove" => "metadata target does not provide typed Platform XML removal",
                _ => "metadata target does not provide typed Platform XML editing",
            },
        },
        Some("metadataPath"),
    )
    .with_metadata_path(metadata_path.clone())
    .into()
}

pub(crate) fn prepare_typed_edit(
    request: &unica_metadata::request::MetaEditRequest,
    resolved: ResolvedMetadataObject<ClosedPlatformXmlTarget>,
    context: &WorkspaceContext,
) -> Result<Box<dyn PreparedMetadataMutation>, unica_metadata::request::MetaFailure> {
    let target = request.metadata_path.clone();
    let diagnostics = Vec::new();
    match evaluate_resolved_support_guard(
        &resolved.descriptor_path,
        crate::application::SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => {}
        ResolvedSupportGuardCheck::Block(_) => {
            return Err(unica_metadata_designer::edit::typed_diagnostic(
                MetaDiagnosticCode::SupportLocked,
                "metadata source support policy blocks object editing",
                None,
            )
            .with_metadata_path(target)
            .into())
        }
    }
    let TypedOperationPostImage {
        descriptor: post_image,
        child_resources,
        effects,
    } = unica_metadata_designer::image_plan::build_typed_operation_post_image(
        TypedOperationDependencyScope::new(
            &request.source_set,
            &resolved.source_root,
            &resolved.owner_path,
            &resolved.owner_preimage,
            true,
        ),
        unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
            path: &resolved.descriptor_path,
            address: &request.metadata_path,
        },
        &resolved.descriptor_preimage,
        &request.operations,
        &super::template_source::MetadataDependencyReadAdapter::new(context),
        unica_metadata_designer::image_plan::MetadataImageBindings {
            resources: unica_metadata_designer::child_resources::ChildResourceBindings {
                observe_membership: snapshot_directory_membership,
                templates: template_seed_emitters(),
                form_seed: unica_forms_designer::planner::minimal_typed_form_content,
            },
            fresh_uuid: crate::infrastructure::source_ids::fresh_uuid,
        },
    )?;
    PreparedMetaEdit::prepare(
        request,
        resolved,
        context,
        post_image,
        diagnostics,
        child_resources,
        effects,
    )
}

#[cfg(test)]
pub(super) fn parse_typed_fill_value(
    properties_text: &str,
) -> Result<Option<MetaFillValue>, MetaDiagnostic> {
    const WRAPPER_START: &str = r#"<Root xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#;
    let wrapped = format!("{WRAPPER_START}{properties_text}</Root>");
    let document = Document::parse(&wrapped).map_err(|_| {
        unica_metadata_designer::edit::typed_diagnostic(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata element properties are not valid XML",
            Some("fillValue"),
        )
    })?;
    unica_metadata_designer::edit::parse_typed_fill_value_node(document.root_element())
}

#[cfg(test)]
pub(super) fn parse_typed_metadata_type(
    properties_text: &str,
) -> Result<MetadataType, MetaDiagnostic> {
    let observed =
        unica_metadata_designer::info_projection::parse_observed_metadata_type(properties_text)?;
    unica_metadata_designer::edit::narrow_observed_metadata_type(observed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Эмиттеры шаблонов designer, связанные напрямую: тесты плана образов
    /// не зависят от продуктовой привязки писателя v0.12.
    fn template_seed_emitters() -> unica_metadata_designer::child_resources::TemplateSeedEmitters {
        unica_metadata_designer::child_resources::TemplateSeedEmitters {
            spreadsheet: unica_metadata_designer::planner::TemplateSeedEmitter(
                unica_templates_designer::mxl_root::empty_spreadsheet_document_xml,
            ),
            html_descriptor: unica_templates_designer::descriptor::html_template_descriptor,
            html_page: unica_templates_designer::descriptor::html_template_page,
            content_xml: unica_templates_designer::descriptor::template_content_xml,
        }
    }

    fn build_typed_operation_post_image(
        scope: TypedOperationDependencyScope<'_>,
        descriptor_path: &Path,
        target: &MetadataAddress,
        descriptor: &[u8],
        operations: &[MetaEditOperation],
        context: &WorkspaceContext,
    ) -> Result<
        TypedOperationPostImage<ClosedPlatformXmlTarget>,
        unica_metadata::request::MetaFailure,
    > {
        unica_metadata_designer::image_plan::build_typed_operation_post_image(
            scope,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: descriptor_path,
                address: target,
            },
            descriptor,
            operations,
            &super::super::template_source::MetadataDependencyReadAdapter::new(context),
            unica_metadata_designer::image_plan::MetadataImageBindings {
                resources: unica_metadata_designer::child_resources::ChildResourceBindings {
                    observe_membership: snapshot_directory_membership,
                    templates: template_seed_emitters(),
                    form_seed: unica_forms_designer::planner::minimal_typed_form_content,
                },
                fresh_uuid: crate::infrastructure::source_ids::fresh_uuid,
            },
        )
    }

    use unica_forms_designer::planner::minimal_typed_form_content;
    use unica_metadata_designer::child_resources::TypedChildTreeError;
    fn plan_typed_child_resources(
        descriptor_path: &Path,
        owner: &MetadataAddress,
        object_kind: &str,
        object_name: &str,
        operations: &[MetaEditOperation],
        post_image: &str,
    ) -> Result<TypedChildResourcePlan<ClosedPlatformXmlTarget>, unica_metadata::request::MetaFailure>
    {
        unica_metadata_designer::child_resources::plan_typed_child_resources(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(descriptor_path),
            unica_metadata_designer::child_resources::ChildResourceBindings {
                observe_membership: snapshot_directory_membership,
                templates: template_seed_emitters(),
                form_seed: minimal_typed_form_content,
            },
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: descriptor_path,
                address: owner,
            },
            object_kind,
            object_name,
            operations,
            post_image,
        )
    }
    fn observe_typed_child_resources(
        descriptor_path: &Path,
        owner: &MetadataAddress,
        object_kind: &str,
        object_name: &str,
        owner_xml: &str,
    ) -> Result<TypedChildObservation, unica_metadata::request::MetaFailure> {
        unica_metadata_designer::child_resources::observe_typed_child_resources(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(descriptor_path),
            descriptor_path,
            owner,
            object_kind,
            object_name,
            owner_xml,
        )
    }
    fn read_typed_child_tree(
        root: &Path,
    ) -> Result<unica_metadata_designer::child_resources::TypedChildTree, TypedChildTreeError> {
        unica_metadata_designer::child_resources::read_typed_child_tree(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(root),
            root,
        )
    }

    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_metadata::{
        DateFractions, MetaCollection, MetaEditOperation, MetaElementInput, MetaElementUpdateInput,
        MetaEventSource, MetaFillValue, MetaPosition, MetaPropertyChanges, MetaPropertyInput,
        MetaPropertyValue, MetaRelation, MetaRelationTarget, MetaScope, MetadataReference,
        MetadataType, MetadataTypeVariant, NumberSign, RelationEditMode, StringLengthMode,
    };

    /// The engine in the `ru` default language the descriptor fixtures here
    /// are written in; the language profile itself is tested separately.
    fn apply_typed_operations(
        xml_text: &mut String,
        operations: &[MetaEditOperation],
    ) -> Result<unica_metadata_designer::edit::MetaEditCounts, unica_metadata::request::MetaFailure>
    {
        let mut next_uuid = crate::infrastructure::source_ids::fresh_uuid;
        unica_metadata_designer::edit::apply_typed_operations_with_uuid(
            xml_text,
            operations,
            "ru",
            &mut next_uuid,
        )
    }

    fn object_xml(kind: &str, name: &str, properties: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<{kind} uuid="11111111-1111-4111-8111-111111111111">
		<Properties><Name>{name}</Name><Synonym/><Comment/>{properties}</Properties>
		<ChildObjects/>
	</{kind}>
</MetaDataObject>
"#
        )
    }

    fn constant_type_change(metadata_type: MetadataType) -> MetaEditOperation {
        MetaEditOperation::SetProperties {
            values: unica_metadata::MetaPropertyChanges::convert(
                MetadataKind::Constant,
                vec![unica_metadata::MetaPropertyInput::new(
                    "Type",
                    MetaPropertyValue::Type(metadata_type),
                )],
            )
            .unwrap(),
        }
    }

    #[test]
    fn constant_type_uses_bound_qnames_and_preserves_exact_equivalent_xml() {
        use unica_metadata::MetadataTypeVariant;
        for variant in [
            MetadataTypeVariant::Boolean,
            MetadataTypeVariant::Reference {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Catalog.Items",
                )
                .unwrap(),
            },
        ] {
            let metadata_type = MetadataType::new(vec![variant]).unwrap();
            for shadowed in [false, true] {
                let mut xml = object_xml(
                    "Constant",
                    "Setting",
                    "<Type><v8:Type>xs:string</v8:Type></Type>",
                )
                .replace("xmlns:v8=", "xmlns:d=")
                .replace("v8:Type", "d:Type")
                .replace("xmlns:xs=", "xmlns:s=")
                .replace("xs:string", "s:string")
                .replace("xmlns:cfg=", "xmlns:c=");
                if shadowed {
                    xml = xml.replace("version=\"2.20\"", "xmlns:v8=\"urn:foreign-data\" xmlns:xs=\"urn:foreign-schema\" xmlns:cfg=\"urn:foreign-config\" version=\"2.20\"");
                }
                let operation = constant_type_change(metadata_type.clone());
                apply_typed_operations(&mut xml, std::slice::from_ref(&operation)).unwrap();
                let doc = Document::parse(&xml).unwrap();
                let object = unica_metadata_designer::edit::meta_edit_object_node(&doc).unwrap();
                let properties =
                    unica_xml_format::read::child_element(object, "Properties").unwrap();
                let observed =
                    unica_metadata_designer::info_projection::parse_observed_metadata_type_node(
                        properties,
                    )
                    .unwrap();
                assert_eq!(MetadataType::try_from(observed).unwrap(), metadata_type);
                let before = xml.clone();
                let counts = apply_typed_operations(&mut xml, &[operation]).unwrap();
                assert_eq!(xml, before, "same logical type must not rewrite XML");
                assert_eq!(counts.effects[0].before, counts.effects[0].after);
            }
            for default_namespace in ["", " xmlns=\"urn:foreign-default\""] {
                for bom_count in 0..=2 {
                    let mut xml = format!(
                        r#"<m:MetaDataObject xmlns:m="http://v8.1c.ru/8.3/MDClasses"{default_namespace} xmlns:d="http://v8.1c.ru/8.1/data/core" xmlns:s="http://www.w3.org/2001/XMLSchema" version="2.20"><m:Constant uuid="11111111-1111-4111-8111-111111111111"><m:Properties><m:Name>Setting</m:Name><m:Synonym/><m:Comment/><m:Type><d:Type>s:string</d:Type></m:Type></m:Properties><m:ChildObjects/></m:Constant></m:MetaDataObject>"#
                    );
                    xml.insert_str(0, &"\u{feff}".repeat(bom_count));
                    let operation = constant_type_change(metadata_type.clone());
                    apply_typed_operations(&mut xml, std::slice::from_ref(&operation)).unwrap();
                    let doc = Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
                    let object =
                        unica_metadata_designer::edit::meta_edit_object_node(&doc).unwrap();
                    let properties =
                        unica_xml_format::read::child_element(object, "Properties").unwrap();
                    assert!(
                        properties
                            .children()
                            .any(|child| child
                                .has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Type"))),
                        "Constant.Type must remain in the metadata namespace: {xml}"
                    );
                    let observed =
                        unica_metadata_designer::info_projection::parse_observed_metadata_type_node(
                            properties,
                        )
                        .unwrap();
                    assert_eq!(MetadataType::try_from(observed).unwrap(), metadata_type);
                    let before = xml.clone();
                    apply_typed_operations(&mut xml, &[operation]).unwrap();
                    assert_eq!(xml, before, "equivalent Type must not rewrite aliased XML");
                }
            }
        }
    }

    #[test]
    fn constant_type_replaces_the_complete_node_with_nested_unprefixed_type() {
        let mut xml = object_xml(
            "Constant",
            "Setting",
            r#"<Type><Type xmlns="http://v8.1c.ru/8.1/data/core">xs:string</Type></Type>"#,
        );
        let operation =
            constant_type_change(MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap());
        apply_typed_operations(&mut xml, std::slice::from_ref(&operation)).unwrap();
        let doc = Document::parse(&xml).unwrap();
        let properties = unica_xml_format::read::child_element(
            unica_metadata_designer::edit::meta_edit_object_node(&doc).unwrap(),
            "Properties",
        )
        .unwrap();
        let observed =
            unica_metadata_designer::info_projection::parse_observed_metadata_type_node(properties)
                .unwrap();
        assert_eq!(
            MetadataType::try_from(observed).unwrap(),
            MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap()
        );
        assert_eq!(
            unica_xml_format::read::child_direct_text(properties, "Name").as_deref(),
            Some("Setting")
        );
        let before = xml.clone();
        apply_typed_operations(&mut xml, &[operation]).unwrap();
        assert_eq!(xml, before);
    }

    #[test]
    fn constant_independent_property_preserves_unsupported_type_and_explicit_type_replaces_it() {
        use unica_metadata::{MetaPropertyChanges, MetaPropertyInput, MetadataTypeVariant};
        let retained = "<Type><v8:TypeSet>cfg:Characteristic.Products</v8:TypeSet></Type>";
        let mut xml = object_xml("Constant", "Setting", retained);
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::SetProperties {
                values: MetaPropertyChanges::convert(
                    MetadataKind::Constant,
                    vec![MetaPropertyInput::new(
                        "Comment",
                        MetaPropertyValue::String("changed".into()),
                    )],
                )
                .unwrap(),
            }],
        )
        .unwrap();
        assert!(xml.contains(retained));
        apply_typed_operations(
            &mut xml,
            &[constant_type_change(
                MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap(),
            )],
        )
        .unwrap();
        assert!(!xml.contains("Characteristic.Products"));
        assert!(xml.contains("xs:boolean"));
        assert!(xml.contains("<Comment>changed</Comment>"));
    }

    fn event_subscription_xml(source: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<EventSubscription uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>Events</Name>
			<Synonym/>
			<Comment/>
			{source}
			<Event>BeforeWrite</Event>
			<Handler>CommonModule.Handler.OnEvent</Handler>
		</Properties>
	</EventSubscription>
</MetaDataObject>
"#
        )
    }

    fn event_subscription_alias_xml(source: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:d="http://v8.1c.ru/8.1/data/core" xmlns:s="http://www.w3.org/2001/XMLSchema" version="2.20">
	<EventSubscription uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>Events</Name>
			<Synonym/>
			<Comment/>
			{source}
			<Event>BeforeWrite</Event>
			<Handler>CommonModule.Handler.OnEvent</Handler>
		</Properties>
	</EventSubscription>
</MetaDataObject>
"#
        )
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

    fn catalog_family() -> MetaEventSource {
        MetaEventSource::Family {
            source_class: unica_metadata::EventSourceClass::CatalogObject,
        }
    }

    fn metadata_reference(path: &str) -> MetadataReference {
        MetadataReference {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, path).unwrap(),
        }
    }

    #[test]
    fn register_collections_insert_into_their_canonical_platform_slot() {
        // The 8.3.27 export orders register children Resource → Attribute →
        // Dimension (773/773 registers in the reference dump). Appending in
        // operation order makes the platform reorder the file on roundtrip,
        // which the exact gate reports as accepted-normalized.
        fn register_xml(children: &[(&str, &str)]) -> String {
            let mut body = String::new();
            for (index, (tag, name)) in children.iter().enumerate() {
                body.push_str(&format!(
                    concat!(
                        "\t\t\t<{tag} uuid=\"22222222-2222-4222-8222-22222222222{n}\">\n",
                        "\t\t\t\t<Properties><Name>{name}</Name></Properties>\n",
                        "\t\t\t</{tag}>\n",
                    ),
                    tag = tag,
                    name = name,
                    n = index,
                ));
            }
            format!(
                concat!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                    "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n",
                    "\t<InformationRegister uuid=\"11111111-1111-4111-8111-111111111111\">\n",
                    "\t\t<Properties><Name>Corpus</Name></Properties>\n",
                    "\t\t<ChildObjects>\n{body}\t\t</ChildObjects>\n",
                    "\t</InformationRegister>\n",
                    "</MetaDataObject>\n",
                ),
                body = body,
            )
        }

        // (existing children, inserted tag, expected child-name order)
        type SlotCase = (
            &'static [(&'static str, &'static str)],
            &'static str,
            &'static [&'static str],
        );
        let matrix: &[SlotCase] = &[
            // every earlier slot lands before every later slot already present
            (&[("Dimension", "Item")], "Resource", &["New", "Item"]),
            (&[("Attribute", "Note")], "Resource", &["New", "Note"]),
            (&[("Dimension", "Item")], "Attribute", &["New", "Item"]),
            (
                &[("Attribute", "Note"), ("Dimension", "Item")],
                "Resource",
                &["New", "Note", "Item"],
            ),
            // the last slot and same-slot siblings keep append order
            (&[("Resource", "Price")], "Dimension", &["Price", "New"]),
            (&[("Dimension", "Item")], "Dimension", &["Item", "New"]),
            (&[("Resource", "Price")], "Resource", &["Price", "New"]),
        ];
        for (existing, inserted, expected) in matrix {
            let mut xml = register_xml(existing);
            unica_metadata_designer::edit::meta_edit_insert_top_child_object(
                &mut xml,
                &[
                    format!("\t\t\t<{inserted} uuid=\"33333333-3333-4333-8333-333333333333\">"),
                    "\t\t\t\t<Properties><Name>New</Name></Properties>".to_string(),
                    format!("\t\t\t</{inserted}>"),
                ],
            )
            .unwrap();

            let order: Vec<&str> = expected
                .iter()
                .map(|name| (xml.find(&format!("<Name>{name}</Name>")).unwrap(), *name))
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_values()
                .collect();
            assert_eq!(
                &order, expected,
                "insert {inserted} into {existing:?}: {xml}"
            );
        }
    }

    /// #1385: the 8.3.27 export groups an owner's `ChildObjects` by kind in an
    /// order of its own per owner kind (settled by load and export of
    /// reverse-ordered descriptors), so a new entry lands before the first
    /// entry of a later kind, else at the end.
    #[test]
    fn child_objects_insert_into_the_platform_slot_of_their_owner_kind() {
        fn owner_xml(kind: &str, children: &[(&str, &str)]) -> String {
            let mut body = String::new();
            for (tag, name) in children {
                if matches!(*tag, "Form" | "Template") {
                    body.push_str(&format!("\t\t\t<{tag}>{name}</{tag}>\n"));
                } else {
                    body.push_str(&format!(
                        "\t\t\t<{tag} uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties><Name>{name}</Name></Properties>\n\t\t\t</{tag}>\n"
                    ));
                }
            }
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<{kind} uuid=\"11111111-1111-4111-8111-111111111111\">\n\t\t<Properties><Name>Owner</Name></Properties>\n\t\t<ChildObjects>\n{body}\t\t</ChildObjects>\n\t</{kind}>\n</MetaDataObject>\n"
            )
        }
        fn child_names(xml: &str) -> Vec<String> {
            let doc = Document::parse(xml).unwrap();
            let object = unica_metadata_designer::edit::meta_edit_object_node(&doc).unwrap();
            unica_xml_format::read::child_element(object, "ChildObjects")
                .unwrap()
                .children()
                .filter(|child| child.is_element())
                .map(|child| {
                    unica_metadata_designer::edit::meta_edit_child_object_name(child)
                        .unwrap_or_else(|| child.text().unwrap_or_default().to_string())
                })
                .collect()
        }
        type Case = (
            &'static str,
            &'static [(&'static str, &'static str)],
            &'static str,
            &'static [&'static str],
        );
        let cases: &[Case] = &[
            // the header attribute of a catalog goes before its sections
            (
                "Catalog",
                &[
                    ("Attribute", "A"),
                    ("TabularSection", "T"),
                    ("Form", "F"),
                    ("Command", "C"),
                ],
                "Attribute",
                &["A", "New", "T", "F", "C"],
            ),
            (
                "Catalog",
                &[("Attribute", "A"), ("TabularSection", "T"), ("Form", "F")],
                "TabularSection",
                &["A", "T", "New", "F"],
            ),
            // a document writes its forms before its tabular sections
            (
                "Document",
                &[
                    ("Attribute", "A"),
                    ("Form", "F"),
                    ("TabularSection", "T"),
                    ("Command", "C"),
                ],
                "TabularSection",
                &["A", "F", "T", "New", "C"],
            ),
            (
                "Document",
                &[("Form", "F"), ("TabularSection", "T")],
                "Attribute",
                &["New", "F", "T"],
            ),
            // commands close every object; templates come before them
            (
                "Enum",
                &[("EnumValue", "V"), ("Command", "C")],
                "EnumValue",
                &["V", "New", "C"],
            ),
            (
                "Task",
                &[
                    ("Attribute", "A"),
                    ("AddressingAttribute", "AA"),
                    ("Command", "C"),
                ],
                "TabularSection",
                &["A", "New", "AA", "C"],
            ),
            // an accounting register starts with its dimensions
            (
                "AccountingRegister",
                &[("Dimension", "D"), ("Resource", "R"), ("Attribute", "A")],
                "Dimension",
                &["D", "New", "R", "A"],
            ),
            (
                "AccumulationRegister",
                &[
                    ("Resource", "R"),
                    ("Attribute", "A"),
                    ("Dimension", "D"),
                    ("Command", "C"),
                ],
                "Dimension",
                &["R", "A", "D", "New", "C"],
            ),
            (
                "CalculationRegister",
                &[("Resource", "R"), ("Recalculation", "P"), ("Template", "M")],
                "Dimension",
                &["R", "New", "P", "M"],
            ),
            (
                "DocumentJournal",
                &[("Column", "K"), ("Template", "M")],
                "Command",
                &["K", "M", "New"],
            ),
        ];
        for (kind, existing, inserted, expected) in cases {
            let mut xml = owner_xml(kind, existing);
            unica_metadata_designer::edit::meta_edit_insert_top_child_object(
                &mut xml,
                &[
                    format!("\t\t\t<{inserted} uuid=\"33333333-3333-4333-8333-333333333333\">"),
                    "\t\t\t\t<Properties><Name>New</Name></Properties>".to_string(),
                    format!("\t\t\t</{inserted}>"),
                ],
            )
            .unwrap();
            assert_eq!(
                child_names(&xml),
                *expected,
                "{kind}: insert {inserted} into {existing:?}"
            );
        }
    }

    #[test]
    fn typed_fill_parser_uses_the_exact_xsd_timezone_boundary() {
        for (timezone, valid) in [
            ("+13:59", true),
            ("-13:59", true),
            ("+14:00", true),
            ("-14:00", true),
            ("+14:01", false),
            ("-14:01", false),
            ("+14:59", false),
            ("-14:59", false),
        ] {
            let properties = format!(
                r#"<FillValue xsi:type="xs:dateTime">2026-01-01T12:00:00{timezone}</FillValue>"#
            );
            let result = parse_typed_fill_value(&properties);
            assert_eq!(
                result.is_ok(),
                valid,
                "writer fill parser disagrees at timezone {timezone}: {result:?}"
            );
        }
    }

    #[test]
    fn typed_fill_parser_uses_complete_direct_text_and_rejects_nil_suffix() {
        assert_eq!(
            parse_typed_fill_value(
                r#"<FillValue xsi:type="xs:string">ab<!--split-->cd</FillValue>"#,
            )
            .unwrap(),
            Some(MetaFillValue::String("abcd".to_string()))
        );
        assert!(parse_typed_fill_value(
            r#"<FillValue xsi:nil="true"><!--split-->garbage</FillValue>"#,
        )
        .is_err());
    }

    #[test]
    fn typed_metadata_type_uses_the_complete_direct_text_value() {
        let parsed = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:str<!--future-separator-->ing</v8:Type>",
            "<v8:StringQualifiers><v8:Length>0</v8:Length>",
            "<v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>"
        ))
        .unwrap();
        assert!(matches!(
            parsed.variants.as_slice(),
            [MetadataTypeVariant::String { .. }]
        ));

        let invalid = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:string<!--future-suffix-->garbage</v8:Type>",
            "<v8:StringQualifiers><v8:Length>0</v8:Length>",
            "<v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>"
        ));
        assert!(invalid.is_err());

        let qualified = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:string</v8:Type>",
            "<v8:StringQualifiers>",
            "<v8:Length>1<!--future-separator-->0</v8:Length>",
            "<v8:AllowedLength>Fi<!--future-separator-->xed</v8:AllowedLength>",
            "</v8:StringQualifiers>"
        ))
        .unwrap();
        assert!(matches!(
            qualified.variants.as_slice(),
            [MetadataTypeVariant::String {
                length: 10,
                allowed_length: StringLengthMode::Fixed,
            }]
        ));

        let invalid_qualifier = parse_typed_metadata_type(concat!(
            "<v8:Type>xs:string</v8:Type>",
            "<v8:StringQualifiers>",
            "<v8:Length>10<!--future-suffix-->garbage</v8:Length>",
            "</v8:StringQualifiers>"
        ));
        assert!(invalid_qualifier.is_err());

        let container =
            parse_typed_metadata_type("<Type><v8:Type>xs:boolean</v8:Type></Type>").unwrap();
        assert!(matches!(
            container.variants.as_slice(),
            [MetadataTypeVariant::Boolean]
        ));

        let scoped = parse_typed_metadata_type(concat!(
            "<Properties>",
            "<future:Type xmlns:future=\"urn:future\"><v8:Type>xs:boolean</v8:Type></future:Type>",
            "<Type><v8:Type>xs:string</v8:Type>",
            "<v8:StringQualifiers><v8:Length>0</v8:Length>",
            "<v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>",
            "</Properties>"
        ))
        .unwrap();
        assert!(matches!(
            scoped.variants.as_slice(),
            [MetadataTypeVariant::String {
                length: 0,
                allowed_length: StringLengthMode::Variable,
            }]
        ));

        assert!(parse_typed_metadata_type(concat!(
            "<Type><v8:Type>xs:string</v8:Type>",
            "<future:Extension xmlns:future=\"urn:future\"/>",
            "</Type>"
        ))
        .is_err());
    }

    /// #2163: `props.set` takes the synonym as every language
    /// `[{language, content}]`; the element then holds exactly those
    /// languages, while plain text still sets only the project language.
    #[test]
    fn synonym_takes_every_language_or_the_project_text() {
        use unica_metadata::MetadataKind::Catalog;
        let (mut xml, _) =
            unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                Catalog, "Evidence",
            )
            .unwrap();
        let set = |xml: &mut String, value: serde_json::Value| {
            let changes = unica_metadata::request::parse_property_changes(
                serde_json::json!({ "Synonym": value }).as_object().unwrap(),
                Catalog,
            )
            .unwrap();
            apply_typed_operations(xml, &[MetaEditOperation::SetProperties { values: changes }])
                .unwrap();
        };
        let synonym = |xml: &str| {
            let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
            let properties = document
                .descendants()
                .find(|node| node.has_tag_name("Properties"))
                .unwrap();
            let synonym = properties
                .children()
                .find(|node| node.has_tag_name("Synonym"))
                .unwrap();
            synonym
                .descendants()
                .filter(|node| node.tag_name().name() == "item")
                .map(|item| {
                    let text = |name: &str| {
                        item.children()
                            .find(|node| node.tag_name().name() == name)
                            .and_then(|node| node.text())
                            .unwrap_or_default()
                            .to_string()
                    };
                    (text("lang"), text("content"))
                })
                .collect::<Vec<_>>()
        };
        set(
            &mut xml,
            serde_json::json!([{"language":"ru","content":"Товары"},{"language":"en","content":"Goods"}]),
        );
        assert_eq!(
            synonym(&xml),
            [
                ("ru".into(), "Товары".into()),
                ("en".into(), "Goods".into())
            ]
        );
        let before = xml.clone();
        set(
            &mut xml,
            serde_json::json!([{"language":"ru","content":"Товары"},{"language":"en","content":"Goods"}]),
        );
        assert_eq!(xml, before, "the value read back is written without change");
        set(&mut xml, serde_json::json!("Номенклатура"));
        assert_eq!(
            synonym(&xml),
            [
                ("ru".into(), "Номенклатура".into()),
                ("en".into(), "Goods".into())
            ]
        );
        set(
            &mut xml,
            serde_json::json!([{"language":"en","content":"Items"}]),
        );
        assert_eq!(synonym(&xml), [("en".into(), "Items".into())]);
        for (value, path) in [
            (
                serde_json::json!([{"language":"ru","content":"А"},{"language":"ru","content":"Б"}]),
                "values.Synonym[1].language",
            ),
            (
                serde_json::json!([{"language":"","content":"А"}]),
                "values.Synonym[0].language",
            ),
            (
                serde_json::json!([{"language":"ru"}]),
                "values.Synonym[0].content",
            ),
            (serde_json::json!(1), "values.Synonym"),
        ] {
            let error = unica_metadata::request::parse_property_changes(
                serde_json::json!({ "Synonym": value }).as_object().unwrap(),
                Catalog,
            )
            .unwrap_err();
            assert_eq!(error.field.as_deref(), Some(path), "{value}");
        }
        let comment = unica_metadata::request::parse_property_changes(
            serde_json::json!({"Comment":[{"language":"ru","content":"А"}]})
                .as_object()
                .unwrap(),
            Catalog,
        )
        .unwrap_err();
        assert_eq!(comment.field.as_deref(), Some("values.Comment"));
    }

    #[test]
    fn typed_set_properties_and_relations_apply_without_legacy_dsl() {
        let mut xml = object_xml(
            "Document",
            "Order",
            "<NumberLength>9</NumberLength><CheckUnique>true</CheckUnique><RegisterRecords/><BasedOn/><InputByString/>",
        );
        let operations = vec![
            MetaEditOperation::SetProperties {
                values: MetaPropertyChanges::convert(
                    unica_metadata::MetadataKind::Document,
                    vec![
                        MetaPropertyInput::new(
                            "Synonym",
                            MetaPropertyValue::String("Customer order".into()),
                        ),
                        MetaPropertyInput::new(
                            "NumberLength",
                            MetaPropertyValue::UnsignedInteger(12),
                        ),
                        MetaPropertyInput::new("CheckUnique", MetaPropertyValue::Boolean(false)),
                    ],
                )
                .unwrap(),
            },
            MetaEditOperation::edit_relations(
                MetaRelation::RegisterRecords,
                RelationEditMode::Replace,
                vec![metadata_reference("InformationRegister.OrderFacts")],
            )
            .unwrap(),
            MetaEditOperation::edit_relations(
                MetaRelation::BasedOn,
                RelationEditMode::Add,
                vec![metadata_reference("Document.Quote")],
            )
            .unwrap(),
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Replace,
                vec![unica_metadata::MetaRelationTarget::Field(
                    unica_metadata::MetadataFieldPath::parse(
                        "Document.Order.StandardAttribute.Number",
                    )
                    .unwrap(),
                )],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        assert!(xml.contains("<v8:content>Customer order</v8:content>"));
        assert!(xml.contains("<NumberLength>12</NumberLength>"));
        assert!(xml.contains("<CheckUnique>false</CheckUnique>"));
        assert!(xml.contains(
            "<xr:Item xsi:type=\"xr:MDObjectRef\">InformationRegister.OrderFacts</xr:Item>"
        ));
        assert!(xml.contains("<xr:Item xsi:type=\"xr:MDObjectRef\">Document.Quote</xr:Item>"));
        assert!(xml.contains("<xr:Field>Document.Order.StandardAttribute.Number</xr:Field>"));
    }

    #[test]
    fn retired_dsl_scalar_typed_properties_round_trip_through_template_writer_and_info() {
        use unica_metadata::MetadataKind::*;

        let cases: &[(
            &str,
            &str,
            MetaPropertyValue,
            &[unica_metadata::MetadataKind],
        )] = &[
            (
                "ActionPeriod",
                "ActionPeriod",
                MetaPropertyValue::Boolean(true),
                &[CalculationRegister],
            ),
            (
                "ActionPeriodUse",
                "ActionPeriodUse",
                MetaPropertyValue::Boolean(true),
                &[ChartOfCalculationTypes],
            ),
            (
                "AutoOrderByCode",
                "AutoOrderByCode",
                MetaPropertyValue::Boolean(false),
                &[ChartOfAccounts],
            ),
            (
                "BasePeriod",
                "BasePeriod",
                MetaPropertyValue::Boolean(true),
                &[CalculationRegister],
            ),
            (
                "ChoiceMode",
                "ChoiceMode",
                MetaPropertyValue::String("FromForm".into()),
                &[Catalog],
            ),
            (
                "ClientManagedApplication",
                "ClientManagedApplication",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "ClientOrdinaryApplication",
                "ClientOrdinaryApplication",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "CodeAllowedLength",
                "CodeAllowedLength",
                MetaPropertyValue::String("Fixed".into()),
                &[Catalog],
            ),
            (
                "CodeLength",
                "CodeLength",
                MetaPropertyValue::UnsignedInteger(12),
                &[
                    Catalog,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    ExchangePlan,
                ],
            ),
            (
                "CodeMask",
                "CodeMask",
                MetaPropertyValue::String("@@@.@@".into()),
                &[ChartOfAccounts],
            ),
            (
                "CodeType",
                "CodeType",
                MetaPropertyValue::String("Number".into()),
                &[Catalog],
            ),
            (
                "Correspondence",
                "Correspondence",
                MetaPropertyValue::Boolean(true),
                &[AccountingRegister],
            ),
            (
                "DefaultPresentation",
                "DefaultPresentation",
                MetaPropertyValue::String("AsCode".into()),
                &[Catalog],
            ),
            (
                "DependenceOnCalculationTypes",
                "DependenceOnCalculationTypes",
                MetaPropertyValue::String("OnActionPeriod".into()),
                &[ChartOfCalculationTypes],
            ),
            (
                "Description",
                "Description",
                MetaPropertyValue::String("Night import".into()),
                &[ScheduledJob],
            ),
            (
                "DescriptionLength",
                "DescriptionLength",
                MetaPropertyValue::UnsignedInteger(120),
                &[
                    Catalog,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    Task,
                    ExchangePlan,
                ],
            ),
            (
                "DistributedInfoBase",
                "DistributedInfoBase",
                MetaPropertyValue::Boolean(true),
                &[ExchangePlan],
            ),
            (
                "EnableTotalsSplitting",
                "EnableTotalsSplitting",
                MetaPropertyValue::Boolean(false),
                &[AccumulationRegister],
            ),
            (
                "ExternalConnection",
                "ExternalConnection",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "FoldersOnTop",
                "FoldersOnTop",
                MetaPropertyValue::Boolean(false),
                &[Catalog],
            ),
            (
                "Global",
                "Global",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "HierarchyType",
                "HierarchyType",
                MetaPropertyValue::String("HierarchyOfItems".into()),
                &[Catalog],
            ),
            (
                "LevelCount",
                "LevelCount",
                MetaPropertyValue::UnsignedInteger(4),
                &[Catalog],
            ),
            (
                "LimitLevelCount",
                "LimitLevelCount",
                MetaPropertyValue::Boolean(true),
                &[Catalog],
            ),
            (
                "MainFilterOnPeriod",
                "MainFilterOnPeriod",
                MetaPropertyValue::Boolean(true),
                &[InformationRegister],
            ),
            (
                "MaxExtDimensionCount",
                "MaxExtDimensionCount",
                MetaPropertyValue::UnsignedInteger(4),
                &[ChartOfAccounts],
            ),
            (
                "Namespace",
                "Namespace",
                MetaPropertyValue::String("urn:unica:test".into()),
                &[WebService],
            ),
            (
                "NumberAllowedLength",
                "NumberAllowedLength",
                MetaPropertyValue::String("Fixed".into()),
                &[Document],
            ),
            (
                "NumberLength",
                "NumberLength",
                MetaPropertyValue::UnsignedInteger(15),
                &[Document, BusinessProcess, Task],
            ),
            (
                "NumberPeriodicity",
                "NumberPeriodicity",
                MetaPropertyValue::String("Month".into()),
                &[Document],
            ),
            (
                "NumberType",
                "NumberType",
                MetaPropertyValue::String("Number".into()),
                &[Document, BusinessProcess, Task],
            ),
            (
                "OrderLength",
                "OrderLength",
                MetaPropertyValue::UnsignedInteger(6),
                &[ChartOfAccounts],
            ),
            (
                "PeriodAdjustmentLength",
                "PeriodAdjustmentLength",
                MetaPropertyValue::UnsignedInteger(2),
                &[AccountingRegister],
            ),
            (
                "Periodicity",
                "InformationRegisterPeriodicity",
                MetaPropertyValue::String("Quarter".into()),
                &[InformationRegister],
            ),
            (
                "Periodicity",
                "Periodicity",
                MetaPropertyValue::String("Quarter".into()),
                &[CalculationRegister],
            ),
            (
                "PostInPrivilegedMode",
                "PostInPrivilegedMode",
                MetaPropertyValue::Boolean(false),
                &[Document],
            ),
            (
                "Posting",
                "Posting",
                MetaPropertyValue::String("Deny".into()),
                &[Document],
            ),
            (
                "Predefined",
                "Predefined",
                MetaPropertyValue::Boolean(true),
                &[ScheduledJob],
            ),
            (
                "Privileged",
                "Privileged",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "QuickChoice",
                "QuickChoice",
                MetaPropertyValue::Boolean(true),
                &[Catalog],
            ),
            (
                "RealTimePosting",
                "RealTimePosting",
                MetaPropertyValue::String("Allow".into()),
                &[Document],
            ),
            (
                "RegisterRecordsDeletion",
                "RegisterRecordsDeletion",
                MetaPropertyValue::String("AutoDeleteOff".into()),
                &[Document],
            ),
            (
                "RegisterRecordsWritingOnPost",
                "RegisterRecordsWritingOnPost",
                MetaPropertyValue::String("WriteAll".into()),
                &[Document],
            ),
            (
                "RestartCountOnFailure",
                "RestartCountOnFailure",
                MetaPropertyValue::UnsignedInteger(5),
                &[ScheduledJob],
            ),
            (
                "RestartIntervalOnFailure",
                "RestartIntervalOnFailure",
                MetaPropertyValue::UnsignedInteger(30),
                &[ScheduledJob],
            ),
            (
                "ReturnValuesReuse",
                "ReturnValuesReuse",
                MetaPropertyValue::String("DuringRequest".into()),
                &[CommonModule],
            ),
            (
                "ReuseSessions",
                "ReuseSessions",
                MetaPropertyValue::String("AutoUse".into()),
                &[HTTPService, WebService],
            ),
            (
                "RootURL",
                "RootURL",
                MetaPropertyValue::String("v2".into()),
                &[HTTPService],
            ),
            (
                "Server",
                "Server",
                MetaPropertyValue::Boolean(false),
                &[CommonModule],
            ),
            (
                "ServerCall",
                "ServerCall",
                MetaPropertyValue::Boolean(true),
                &[CommonModule],
            ),
            (
                "SessionMaxAge",
                "SessionMaxAge",
                MetaPropertyValue::UnsignedInteger(45),
                &[HTTPService, WebService],
            ),
            (
                "SubordinationUse",
                "SubordinationUse",
                MetaPropertyValue::String("ToFoldersAndItems".into()),
                &[Catalog],
            ),
            (
                "UnpostInPrivilegedMode",
                "UnpostInPrivilegedMode",
                MetaPropertyValue::Boolean(false),
                &[Document],
            ),
            (
                "Use",
                "Use",
                MetaPropertyValue::Boolean(true),
                &[ScheduledJob],
            ),
            (
                "WriteMode",
                "WriteMode",
                MetaPropertyValue::String("RecorderSubordinate".into()),
                &[InformationRegister],
            ),
            (
                "RegisterType",
                "RegisterType",
                MetaPropertyValue::String("Turnovers".into()),
                &[AccumulationRegister],
            ),
            (
                "IncludeHelpInContents",
                "IncludeHelpInContents",
                MetaPropertyValue::Boolean(true),
                &[
                    Catalog,
                    Document,
                    InformationRegister,
                    AccumulationRegister,
                    AccountingRegister,
                    CalculationRegister,
                    ChartOfAccounts,
                    ChartOfCharacteristicTypes,
                    ChartOfCalculationTypes,
                    BusinessProcess,
                    Task,
                    ExchangePlan,
                    DocumentJournal,
                    Report,
                    DataProcessor,
                ],
            ),
        ];

        for (public_name, xml_tag, value, kinds) in cases {
            for kind in *kinds {
                let (mut xml, _) =
                    unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                        *kind, "Evidence",
                    )
                    .unwrap();
                let changes = MetaPropertyChanges::convert(
                    *kind,
                    vec![MetaPropertyInput::new(*public_name, value.clone())],
                )
                .unwrap_or_else(|diagnostic| {
                    panic!("{public_name} for {}: {diagnostic:?}", kind.as_str())
                });

                apply_typed_operations(
                    &mut xml,
                    &[MetaEditOperation::SetProperties { values: changes }],
                )
                .unwrap_or_else(|failure| {
                    panic!("{public_name} for {}: {failure:?}", kind.as_str())
                });

                let expected_xml = match value {
                    MetaPropertyValue::Type(_) | MetaPropertyValue::Localized(_) => {
                        unreachable!("scalar property test matrix")
                    }
                    MetaPropertyValue::String(value) => {
                        format!(
                            "<{xml_tag}>{}</{xml_tag}>",
                            unica_xml_format::text::escape_xml(value)
                        )
                    }
                    MetaPropertyValue::Boolean(value) => {
                        format!("<{xml_tag}>{value}</{xml_tag}>")
                    }
                    MetaPropertyValue::UnsignedInteger(value) => {
                        format!("<{xml_tag}>{value}</{xml_tag}>")
                    }
                };
                assert!(
                    xml.contains(&expected_xml),
                    "{public_name} for {}: {xml}",
                    kind.as_str()
                );

                let document = roxmltree::Document::parse(&xml).unwrap();
                let object =
                    unica_metadata_designer::edit::meta_edit_object_node(&document).unwrap();
                let properties = unica_xml_format::read::child_element(object, "Properties");
                let observed = unica_metadata_designer::info::typed_properties(properties, *kind);
                let observed = observed
                    .iter()
                    .find(|property| property.key == *public_name)
                    .unwrap_or_else(|| {
                        panic!("{public_name} missing from info for {}", kind.as_str())
                    });
                assert_eq!(
                    serde_json::to_value(&observed.value).unwrap(),
                    serde_json::to_value(value).unwrap(),
                    "{public_name} for {}",
                    kind.as_str()
                );
            }
        }
    }

    #[test]
    fn include_help_in_contents_on_a_platform_report_changes_only_its_value() {
        let original = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace/src/Reports/",
            "АнализВерсийОбъектов.xml"
        ));
        let mut xml = original.to_string();
        let changes = MetaPropertyChanges::convert(
            unica_metadata::MetadataKind::Report,
            vec![MetaPropertyInput::new(
                "IncludeHelpInContents",
                MetaPropertyValue::Boolean(true),
            )],
        )
        .unwrap();

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::SetProperties { values: changes }],
        )
        .unwrap();

        // The platform element keeps its place and formatting; every other
        // byte of the dumped descriptor survives.
        assert_eq!(
            xml,
            original.replacen(
                "<IncludeHelpInContents>false</IncludeHelpInContents>",
                "<IncludeHelpInContents>true</IncludeHelpInContents>",
                1,
            )
        );
    }

    #[test]
    fn register_type_switch_keeps_the_8_3_27_property_order_and_reports_both_values() {
        let kind = unica_metadata::MetadataKind::AccumulationRegister;
        let (mut xml, _) =
            unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                kind, "Sales",
            )
            .unwrap();
        let before = xml.clone();
        let changes = MetaPropertyChanges::convert(
            kind,
            vec![MetaPropertyInput::new(
                "RegisterType",
                MetaPropertyValue::String("Turnovers".into()),
            )],
        )
        .unwrap();

        let counts = apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::SetProperties { values: changes }],
        )
        .unwrap();

        assert_eq!(
            xml,
            before.replacen(
                "<RegisterType>Balance</RegisterType>",
                "<RegisterType>Turnovers</RegisterType>",
                1,
            )
        );
        // Root property sequence of every turnover register in the 8.3.27
        // vendor dump (format 2.20).
        let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
        let object = unica_metadata_designer::edit::meta_edit_object_node(&document).unwrap();
        let order = unica_xml_format::read::child_element(object, "Properties")
            .unwrap()
            .children()
            .filter(|node| node.is_element())
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "DefaultListForm",
                "AuxiliaryListForm",
                "RegisterType",
                "IncludeHelpInContents",
                "StandardAttributes",
                "DataLockControlMode",
                "FullTextSearch",
                "EnableTotalsSplitting",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
            ]
        );
        assert_eq!(counts.effects.len(), 1);
        assert_eq!(
            counts.effects[0].before,
            Some(serde_json::json!({"RegisterType": "Balance"}))
        );
        assert_eq!(
            counts.effects[0].after,
            Some(serde_json::json!({"RegisterType": "Turnovers"}))
        );
    }

    #[test]
    fn typed_collection_matrix_adds_updates_and_removes_every_supported_collection() {
        let cases = [
            (MetaCollection::Attributes, "Document", "Attribute"),
            (
                MetaCollection::TabularSections,
                "Document",
                "TabularSection",
            ),
            (
                MetaCollection::Dimensions,
                "InformationRegister",
                "Dimension",
            ),
            (MetaCollection::Resources, "InformationRegister", "Resource"),
            (MetaCollection::EnumValues, "Enum", "EnumValue"),
            (MetaCollection::Columns, "DocumentJournal", "Column"),
            (MetaCollection::Forms, "Document", "Form"),
            (MetaCollection::Templates, "Document", "Template"),
            (MetaCollection::Commands, "Document", "Command"),
        ];

        for (collection, kind, tag) in cases {
            let mut xml = object_xml(kind, "Sample", "");
            let mut added = MetaElementInput::named("First");
            if matches!(
                collection,
                MetaCollection::Attributes
                    | MetaCollection::Dimensions
                    | MetaCollection::Resources
                    | MetaCollection::Columns
            ) {
                added.r#type = Some(
                    MetadataType::new(vec![MetadataTypeVariant::String {
                        length: 20,
                        allowed_length: StringLengthMode::Variable,
                    }])
                    .unwrap(),
                );
            }
            let operations = vec![
                MetaEditOperation::add(collection, None, vec![added]).unwrap(),
                MetaEditOperation::update(
                    collection,
                    None,
                    vec![MetaElementUpdateInput {
                        name: "First".into(),
                        new_name: Some("Second".into()),
                        ..MetaElementUpdateInput::default()
                    }],
                )
                .unwrap(),
                MetaEditOperation::remove(collection, None, vec!["Second".into()]).unwrap(),
            ];

            apply_typed_operations(&mut xml, &operations).unwrap();
            assert!(!xml.contains(&format!("<{tag} uuid=")), "{collection:?}");
        }
    }

    /// #1385: the platform names a tabular section's generated types after
    /// it and renames them with it; their ids stay. Every other byte of the
    /// platform descriptor survives the rename.
    #[test]
    fn tabular_section_rename_renames_its_generated_types_and_keeps_their_ids() {
        let original = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace/src/Catalogs/Валюты.xml"
        ));
        let mut xml = original.to_string();
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::update(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementUpdateInput {
                    name: "Представления".into(),
                    new_name: Some("Переводы".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()],
        )
        .unwrap();

        let expected = original
            .replacen(
                r#"name="CatalogTabularSection.Валюты.Представления""#,
                r#"name="CatalogTabularSection.Валюты.Переводы""#,
                1,
            )
            .replacen(
                r#"name="CatalogTabularSectionRow.Валюты.Представления""#,
                r#"name="CatalogTabularSectionRow.Валюты.Переводы""#,
                1,
            )
            .replacen("<Name>Представления</Name>", "<Name>Переводы</Name>", 1);
        assert_ne!(expected, original, "fixture must name the section's types");
        let first_difference = xml
            .char_indices()
            .zip(expected.chars())
            .find(|((_, left), right)| left != right)
            .map(|((index, _), _)| index);
        assert!(
            xml == expected,
            "renamed descriptor differs from the platform shape at byte {first_difference:?} \
             (lengths {} and {})",
            xml.len(),
            expected.len()
        );
    }

    /// #1385: the 8.3.27 export writes `Use` before `LineNumberLength`; the
    /// sections of catalogs and characteristic-type charts carry both,
    /// documents only `LineNumberLength`, data processors neither.
    #[test]
    fn new_tabular_section_properties_end_in_the_platform_order() {
        for (kind, tail) in [
            (
                "Catalog",
                &["StandardAttributes", "Use", "LineNumberLength"][..],
            ),
            (
                "ChartOfCharacteristicTypes",
                &["StandardAttributes", "Use", "LineNumberLength"][..],
            ),
            (
                "Document",
                &["FillChecking", "StandardAttributes", "LineNumberLength"][..],
            ),
            (
                "DataProcessor",
                &["ToolTip", "FillChecking", "StandardAttributes"][..],
            ),
        ] {
            let mut xml = object_xml(kind, "Owner", "");
            apply_typed_operations(
                &mut xml,
                &[MetaEditOperation::add(
                    MetaCollection::TabularSections,
                    None,
                    vec![MetaElementInput::named("Rows")],
                )
                .unwrap()],
            )
            .unwrap();
            let document = Document::parse(&xml).unwrap();
            let object = unica_metadata_designer::edit::meta_edit_object_node(&document).unwrap();
            let section =
                unica_metadata_designer::edit::meta_edit_find_tabular_section(object, "Rows")
                    .unwrap();
            let properties = unica_xml_format::read::child_element(section, "Properties")
                .unwrap()
                .children()
                .filter(|node| node.is_element())
                .map(|node| node.tag_name().name())
                .collect::<Vec<_>>();
            assert_eq!(&properties[properties.len() - tail.len()..], tail, "{kind}");
        }
    }

    /// #1385: an owner command carries the property set of the 8.3.27 export.
    #[test]
    fn new_owner_command_has_the_platform_property_set() {
        let mut xml = object_xml("Catalog", "Owner", "");
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Commands,
                None,
                vec![MetaElementInput::named("Open")],
            )
            .unwrap()],
        )
        .unwrap();
        let document = Document::parse(&xml).unwrap();
        let object = unica_metadata_designer::edit::meta_edit_object_node(&document).unwrap();
        let command = unica_xml_format::read::child_elements(
            unica_xml_format::read::child_element(object, "ChildObjects").unwrap(),
            "Command",
        )
        .into_iter()
        .next()
        .unwrap();
        let properties = unica_xml_format::read::child_element(command, "Properties")
            .unwrap()
            .children()
            .filter(|node| node.is_element())
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>();
        assert_eq!(
            properties,
            [
                "Name",
                "Synonym",
                "Comment",
                "Group",
                "CommandParameterType",
                "ParameterUseMode",
                "ModifiesData",
                "Representation",
                "ToolTip",
                "Picture",
                "Shortcut",
                "OnMainServerUnavalableBehavior",
            ]
        );
    }

    #[test]
    fn typed_collection_targets_are_case_insensitive() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Товар")],
            )
            .unwrap()],
        )
        .unwrap();

        let duplicate = apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("товар")],
            )
            .unwrap()],
        )
        .unwrap_err();
        assert_eq!(
            duplicate.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "тОвАр".into(),
                    new_name: Some("Позиция".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap()],
        )
        .unwrap();
        assert!(xml.contains("<Name>Позиция</Name>"));

        apply_typed_operations(
            &mut xml,
            &[
                MetaEditOperation::remove(MetaCollection::Attributes, None, vec!["пОзИцИя".into()])
                    .unwrap(),
            ],
        )
        .unwrap();
        assert!(!xml.contains("<Attribute uuid="));
    }

    #[test]
    fn typed_resource_add_keeps_digits_apart_in_the_generated_synonym() {
        let mut xml = object_xml("InformationRegister", "PaymentTerms", "");
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![
                    MetaElementInput::named("СуммаЗакупокЗа30Дней"),
                    MetaElementInput {
                        name: "СуммаПродажЗа30Дней".into(),
                        synonym: Some("Сумма продаж за месяц".into()),
                        ..MetaElementInput::default()
                    },
                ],
            )
            .unwrap()],
        )
        .unwrap();

        assert!(
            xml.contains("<v8:content>Сумма закупок за 30 дней</v8:content>"),
            "{xml}"
        );
        assert!(
            xml.contains("<v8:content>Сумма продаж за месяц</v8:content>"),
            "{xml}"
        );
    }

    /// #323, first defect. A CRLF descriptor must not grow a literal `&#13;`
    /// entity: the platform writes none — the 8.3.27 dump has zero occurrences
    /// across 31 009 structural XML files — so an inserted element carries the
    /// file's own EOL rather than an escaped carriage return in content.
    #[test]
    fn typed_resource_add_does_not_escape_the_carriage_return_of_a_crlf_descriptor() {
        let mut xml = object_xml("InformationRegister", "PaymentTerms", "").replace('\n', "\r\n");

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![MetaElementInput::named("СуммаЗакупокЗа30Дней")],
            )
            .unwrap()],
        )
        .unwrap();

        assert!(!xml.contains("&#13;"), "{xml}");
        assert!(xml.contains("<Resource uuid="), "{xml}");
    }

    /// #323, second defect. A new register field must not silently disagree
    /// with the fields already in the register. The corpus gives no single
    /// standard value — a 8.3.27 vendor dump has 2169 of 2490 information
    /// register resources on `Use/Use` and 42 registers mixed inside
    /// themselves — so the new field takes what its siblings agree on and
    /// falls back to the template default only when they do not.
    #[test]
    fn typed_resource_add_inherits_search_and_history_from_its_siblings() {
        let existing = rendered_resource("СуммаПродаж")
            .replace(
                "<FullTextSearch>Use</FullTextSearch>",
                "<FullTextSearch>DontUse</FullTextSearch>",
            )
            .replace(
                "<DataHistory>Use</DataHistory>",
                "<DataHistory>DontUse</DataHistory>",
            );
        let mut xml = object_xml("InformationRegister", "PaymentTerms", &existing);

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![MetaElementInput::named("СуммаЗакупок")],
            )
            .unwrap()],
        )
        .unwrap();

        assert_eq!(
            xml.matches("<FullTextSearch>DontUse</FullTextSearch>")
                .count(),
            2,
            "the new resource follows its sibling: {xml}"
        );
        assert_eq!(
            xml.matches("<DataHistory>DontUse</DataHistory>").count(),
            2,
            "the new resource follows its sibling: {xml}"
        );
    }

    /// Review of #444: the change covers three settings and two collections,
    /// so the regression has to prove `Indexing` and the `Dimension` branch as
    /// well — otherwise either could break with the suite still green.
    #[test]
    fn typed_register_field_add_inherits_indexing_and_covers_dimensions() {
        let existing = rendered_resource("СуммаПродаж").replace(
            "<Indexing>DontIndex</Indexing>",
            "<Indexing>Index</Indexing>",
        );
        let mut xml = object_xml("InformationRegister", "PaymentTerms", &existing);

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Resources,
                None,
                vec![MetaElementInput::named("СуммаЗакупок")],
            )
            .unwrap()],
        )
        .unwrap();

        assert_eq!(
            xml.matches("<Indexing>Index</Indexing>").count(),
            2,
            "the new resource follows its sibling indexing: {xml}"
        );

        let dimension = rendered_dimension("Организация")
            .replace(
                "<FullTextSearch>Use</FullTextSearch>",
                "<FullTextSearch>DontUse</FullTextSearch>",
            )
            .replace(
                "<Indexing>DontIndex</Indexing>",
                "<Indexing>Index</Indexing>",
            );
        let mut xml = object_xml("InformationRegister", "PaymentTerms", &dimension);

        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::add(
                MetaCollection::Dimensions,
                None,
                vec![MetaElementInput::named("Склад")],
            )
            .unwrap()],
        )
        .unwrap();

        assert_eq!(
            xml.matches("<FullTextSearch>DontUse</FullTextSearch>")
                .count(),
            2,
            "the dimension branch inherits too: {xml}"
        );
        assert_eq!(
            xml.matches("<Indexing>Index</Indexing>").count(),
            2,
            "the dimension branch inherits indexing too: {xml}"
        );
    }

    #[test]
    fn typed_child_tree_rejects_excessive_depth_before_capture() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-child-depth-{}",
            uuid::Uuid::new_v4()
        ));
        let mut current = root.clone();
        for depth in 0..65 {
            current = current.join(format!("d{depth}"));
        }
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("payload.bin"), b"payload").unwrap();

        let result = read_typed_child_tree(&root);

        assert!(matches!(result, Err(TypedChildTreeError::Unavailable)));
        let _ = std::fs::remove_dir_all(root);
    }

    /// #1119: a child tree past the former 64 MiB is read whole; its size is
    /// no reason to refuse.
    #[test]
    fn typed_child_tree_reads_past_the_former_byte_cap() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-child-large-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/small.bin"), b"x").unwrap();
        std::fs::File::create(root.join("large.bin"))
            .unwrap()
            .set_len(65 * 1024 * 1024)
            .unwrap();

        let (read, _) = read_typed_child_tree(&root).unwrap();

        assert_eq!(read.len(), 2);
        let large = read
            .iter()
            .find(|(path, _)| path == std::path::Path::new("large.bin"))
            .unwrap();
        assert_eq!(large.1.len(), 65 * 1024 * 1024);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_operations_are_ordered_support_scoped_attributes_and_emit_structured_types() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let structured_types = vec![
            MetadataTypeVariant::String {
                length: 40,
                allowed_length: StringLengthMode::Fixed,
            },
            MetadataTypeVariant::Number {
                digits: 15,
                fraction: 3,
                sign: NumberSign::NonNegative,
            },
            MetadataTypeVariant::Boolean,
            MetadataTypeVariant::Uuid,
            MetadataTypeVariant::Date {
                fractions: DateFractions::DateTime,
            },
            MetadataTypeVariant::BinaryData {
                length: 512,
                allowed_length: StringLengthMode::Fixed,
            },
            MetadataTypeVariant::Reference {
                metadata_path: metadata_reference("Catalog.Items").metadata_path,
            },
            MetadataTypeVariant::DefinedType {
                metadata_path: metadata_reference("DefinedType.ExternalCode").metadata_path,
            },
        ];
        let operations = vec![
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
                vec![MetaElementInput {
                    name: "Value".into(),
                    r#type: Some(MetadataType::new(structured_types).unwrap()),
                    required: Some(true),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        assert!(xml.contains("<v8:Type>xs:string</v8:Type>"));
        assert!(xml.contains("<v8:AllowedLength>Fixed</v8:AllowedLength>"));
        assert!(xml.contains("<v8:Type>xs:decimal</v8:Type>"));
        assert!(xml.contains("<v8:Type>xs:boolean</v8:Type>"));
        assert!(xml.contains("<v8:Type>v8:UUID</v8:Type>"));
        assert!(xml.contains("<v8:Type>xs:dateTime</v8:Type>"));
        assert!(xml.contains("<v8:Type>cfg:CatalogRef.Items</v8:Type>"));
        assert!(xml.contains("<v8:TypeSet>cfg:DefinedType.ExternalCode</v8:TypeSet>"));
        assert!(xml.contains("<FillChecking>ShowError</FillChecking>"));
    }

    #[test]
    fn tracked_platform_uuid_fixture_round_trips_through_the_typed_emitter() {
        let xml = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/",
            "meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let document = Document::parse(xml).unwrap();
        let properties = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
            .find(|properties| {
                unica_xml_format::read::child_element(*properties, "Type").is_some_and(
                    |type_node| {
                        unica_xml_format::read::child_elements(type_node, "Type")
                            .iter()
                            .any(|node| node.text() == Some("v8:UUID"))
                    },
                )
            })
            .expect("tracked fixture contains a UUID metadata field");
        let parsed = parse_typed_metadata_type(&xml[properties.range()]).unwrap();
        assert_eq!(parsed.variants, vec![MetadataTypeVariant::Uuid]);

        let mut emitted = Vec::new();
        unica_metadata_designer::xml_model::emit_meta_typed_value_type(&mut emitted, "\t", &parsed);
        let properties = format!("<Properties>{}</Properties>", emitted.join("\n"));
        assert_eq!(parse_typed_metadata_type(&properties).unwrap(), parsed);
    }

    fn information_register_xml_with_data_cr() -> String {
        object_xml(
            "InformationRegister",
            "Sample",
            "<InformationRegisterPeriodicity>Nonperiodical</InformationRegisterPeriodicity>",
        )
        .replace("<Comment/>", "<Comment>First&#13;Second</Comment>")
    }

    fn rendered_dimension(name: &str) -> String {
        let element = MetaElementDefinition::convert(
            MetaCollection::Dimensions,
            MetaElementInput::named(name),
        )
        .unwrap();
        let mut next_uuid = crate::infrastructure::source_ids::fresh_uuid;
        unica_metadata_designer::edit::render_typed_element(
            "InformationRegister",
            "Sample",
            MetaCollection::Dimensions,
            false,
            &element,
            "ru",
            &mut next_uuid,
        )
        .unwrap()
        .join("\n")
    }

    fn rendered_resource(name: &str) -> String {
        let element = MetaElementDefinition::convert(
            MetaCollection::Resources,
            MetaElementInput::named(name),
        )
        .unwrap();
        let mut next_uuid = crate::infrastructure::source_ids::fresh_uuid;
        unica_metadata_designer::edit::render_typed_element(
            "InformationRegister",
            "Sample",
            MetaCollection::Resources,
            false,
            &element,
            "ru",
            &mut next_uuid,
        )
        .unwrap()
        .join("\n")
    }

    fn assert_structural_resource_separators(xml: &str, expected: usize) {
        assert!(
            xml.contains("<Comment>First&#13;Second</Comment>"),
            "data CR entity was not preserved: {xml}"
        );
        assert!(
            !xml.contains("</Resource>&#13;"),
            "structural CR entity was emitted: {xml}"
        );
        assert_eq!(
            xml.matches("</Resource>\n\t\t\t<Resource").count(),
            expected,
            "resource elements are not separated by newlines: {xml}"
        );
        Document::parse(xml).unwrap_or_else(|error| panic!("{error}\n{xml}"));
    }

    #[test]
    fn typed_child_append_uses_a_newline_instead_of_a_carriage_return_entity() {
        let first_resource = rendered_resource("First");
        let mut xml = information_register_xml_with_data_cr().replace(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n{first_resource}\n\t\t</ChildObjects>"),
        );
        let operation = MetaEditOperation::add(
            MetaCollection::Resources,
            None,
            vec![MetaElementInput::named("Second")],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        assert_structural_resource_separators(&xml, 1);
        let first = xml.find("<Name>First</Name>").unwrap();
        let second = xml.find("<Name>Second</Name>").unwrap();
        assert!(first < second, "{xml}");
    }

    #[test]
    fn typed_child_position_after_uses_a_newline_instead_of_a_carriage_return_entity() {
        let resources = [rendered_resource("First"), rendered_resource("Last")].join("\n");
        let mut xml = information_register_xml_with_data_cr().replace(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n{resources}\n\t\t</ChildObjects>"),
        );
        let operation = MetaEditOperation::add(
            MetaCollection::Resources,
            None,
            vec![MetaElementInput {
                name: "Second".into(),
                position: Some(MetaPosition::new(None, Some("First".into())).unwrap()),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        assert_structural_resource_separators(&xml, 2);
        let first = xml.find("<Name>First</Name>").unwrap();
        let second = xml.find("<Name>Second</Name>").unwrap();
        let last = xml.find("<Name>Last</Name>").unwrap();
        assert!(first < second && second < last, "{xml}");
    }

    #[test]
    fn typed_positions_and_fill_values_preserve_requested_order_and_wire_types() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let typed_attribute =
            |name: &str,
             variant: MetadataTypeVariant,
             fill_value: Option<MetaFillValue>,
             position: Option<MetaPosition>| MetaElementInput {
                name: name.into(),
                r#type: Some(MetadataType::new(vec![variant]).unwrap()),
                fill_value,
                position,
                ..MetaElementInput::default()
            };
        let operations = vec![
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![
                    typed_attribute(
                        "StringValue",
                        MetadataTypeVariant::String {
                            length: 30,
                            allowed_length: StringLengthMode::Variable,
                        },
                        Some(MetaFillValue::String("A&B".into())),
                        None,
                    ),
                    typed_attribute(
                        "BooleanValue",
                        MetadataTypeVariant::Boolean,
                        Some(MetaFillValue::Boolean(true)),
                        Some(MetaPosition::new(Some("StringValue".into()), None).unwrap()),
                    ),
                    typed_attribute(
                        "NumberValue",
                        MetadataTypeVariant::Number {
                            digits: 10,
                            fraction: 2,
                            sign: NumberSign::Any,
                        },
                        Some(MetaFillValue::Number("12.50".into())),
                        Some(MetaPosition::new(None, Some("StringValue".into())).unwrap()),
                    ),
                    typed_attribute(
                        "DateValue",
                        MetadataTypeVariant::Date {
                            fractions: DateFractions::DateTime,
                        },
                        Some(MetaFillValue::DateTime("2026-08-03T10:11:12".into())),
                        None,
                    ),
                    typed_attribute(
                        "ReferenceValue",
                        MetadataTypeVariant::Reference {
                            metadata_path: metadata_reference("Catalog.Items").metadata_path,
                        },
                        Some(MetaFillValue::Reference(metadata_reference(
                            "Catalog.Items",
                        ))),
                        None,
                    ),
                    typed_attribute(
                        "StorageValue",
                        MetadataTypeVariant::ValueStorage,
                        None,
                        None,
                    ),
                ],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "DateValue".into(),
                    position: Some(MetaPosition::new(Some("BooleanValue".into()), None).unwrap()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        let boolean = xml.find("<Name>BooleanValue</Name>").unwrap();
        let date = xml.find("<Name>DateValue</Name>").unwrap();
        let string = xml.find("<Name>StringValue</Name>").unwrap();
        let number = xml.find("<Name>NumberValue</Name>").unwrap();
        assert!(
            date < boolean && boolean < string && string < number,
            "unexpected typed attribute order: date={date}, boolean={boolean}, string={string}, number={number}\n{xml}"
        );
        assert!(xml.contains("<FillValue xsi:type=\"xs:string\">A&amp;B</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xs:boolean\">true</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xs:decimal\">12.50</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xs:dateTime\">2026-08-03T10:11:12</FillValue>"));
        assert!(xml.contains("<FillValue xsi:type=\"xr:DesignTimeRef\">Catalog.Items</FillValue>"));
        assert!(xml.contains("<v8:Type>v8:ValueStorage</v8:Type>"));
    }

    #[test]
    fn typed_relations_cover_all_relations_and_edit_modes() {
        let mut catalog = object_xml("Catalog", "Child", "<Owners/>");
        let owners = vec![
            MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Replace,
                vec![metadata_reference("Catalog.Parent")],
            )
            .unwrap(),
            MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Add,
                vec![metadata_reference("Catalog.SecondParent")],
            )
            .unwrap(),
            MetaEditOperation::edit_relations(
                MetaRelation::Owners,
                RelationEditMode::Remove,
                vec![metadata_reference("Catalog.Parent")],
            )
            .unwrap(),
        ];
        apply_typed_operations(&mut catalog, &owners).unwrap();
        assert!(!catalog.contains("Catalog.Parent</xr:Item>"));
        assert!(catalog.contains("Catalog.SecondParent</xr:Item>"));

        let mut document = object_xml(
            "Document",
            "Order",
            "<RegisterRecords/><BasedOn/><InputByString/>",
        );
        for (relation, target) in [
            (
                MetaRelation::RegisterRecords,
                "InformationRegister.OrderFacts",
            ),
            (MetaRelation::BasedOn, "Document.Quote"),
        ] {
            let operations = [
                MetaEditOperation::edit_relations(
                    relation,
                    RelationEditMode::Replace,
                    vec![metadata_reference(target)],
                )
                .unwrap(),
                MetaEditOperation::edit_relations(
                    relation,
                    RelationEditMode::Remove,
                    vec![metadata_reference(target)],
                )
                .unwrap(),
            ];
            apply_typed_operations(&mut document, &operations).unwrap();
        }
        let field =
            unica_metadata::MetadataFieldPath::parse("Document.Order.StandardAttribute.Number")
                .unwrap();
        let input_operations = [
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Replace,
                vec![unica_metadata::MetaRelationTarget::Field(field.clone())],
            )
            .unwrap(),
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Remove,
                vec![unica_metadata::MetaRelationTarget::Field(field)],
            )
            .unwrap(),
        ];
        apply_typed_operations(&mut document, &input_operations).unwrap();
        assert!(document.contains("<RegisterRecords/>"));
        assert!(document.contains("<BasedOn/>"));
        assert!(document.contains("<InputByString/>"));
    }

    fn source_edit(mode: RelationEditMode, paths: &[&str]) -> MetaEditOperation {
        MetaEditOperation::EditRelations {
            relation: MetaRelation::Source,
            mode,
            targets: paths
                .iter()
                .map(|path| {
                    MetaRelationTarget::EventSource(MetaEventSource::Object {
                        metadata_path: MetadataAddress::parse(
                            PLATFORM_XML_8_3_27_FORMAT_2_20,
                            path,
                        )
                        .unwrap(),
                    })
                })
                .collect(),
        }
    }

    #[test]
    fn typed_event_source_add_remove_preserve_old_order_bytes_and_repeat_as_noop() {
        let first = "<v8:Type>cfg:CatalogObject.ПримерВ</v8:Type>";
        let second = "<v8:Type>cfg:CatalogObject.ПримерА</v8:Type>";
        let mut xml = event_subscription_xml(&format!("<Source>{first}  {second}</Source>"));
        let before = xml.clone();
        let add = source_edit(RelationEditMode::Add, &["Catalog.ПримерБ"]);
        apply_typed_operations(&mut xml, std::slice::from_ref(&add)).unwrap();
        let after = xml.clone();
        assert!(after.contains(&format!("{first}  {second}")));
        assert!(
            after.find("CatalogObject.ПримерА").unwrap()
                < after.find("CatalogObject.ПримерБ").unwrap()
        );
        apply_typed_operations(&mut xml, &[add]).unwrap();
        assert_eq!(xml, after);
        apply_typed_operations(
            &mut xml,
            &[source_edit(RelationEditMode::Remove, &["Catalog.ПримерА"])],
        )
        .unwrap();
        assert_eq!(xml, after.replace(&format!("  {second}"), ""));
        let removed = xml.clone();
        apply_typed_operations(
            &mut xml,
            &[source_edit(RelationEditMode::Remove, &["Catalog.ПримерА"])],
        )
        .unwrap();
        assert_eq!(xml, removed);
        assert_ne!(xml, before);
    }

    #[test]
    fn typed_event_source_incremental_edits_keep_aliases_type_ids_and_distinct_manager_classes() {
        let retained = "<d:Type>c:ConstantManager.Setting</d:Type>";
        let removed = "<d:Type>c:ConstantValueManager.Setting</d:Type>";
        let family = "<d:TypeSet>c:CatalogObject</d:TypeSet>";
        let type_id = "<d:TypeId>97a5ab75-0974-4551-8659-72579c7cef1e</d:TypeId>";
        let mut xml = event_subscription_alias_xml(&format!(
            "<Source>{retained}{removed}{type_id}{family}</Source>"
        ));
        let original = xml.clone();
        let remove = MetaEditOperation::EditRelations {
            relation: MetaRelation::Source,
            mode: RelationEditMode::Remove,
            targets: vec![MetaRelationTarget::EventSource(MetaEventSource::Manager {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Constant.setting",
                )
                .unwrap(),
                source_class: Some(unica_metadata::EventSourceClass::ConstantValueManager),
            })],
        };
        apply_typed_operations(&mut xml, &[remove]).unwrap();
        assert_eq!(xml, original.replace(removed, ""));
        apply_typed_operations(
            &mut xml,
            &[source_edit(
                RelationEditMode::Add,
                &["Catalog.Z", "Catalog.A"],
            )],
        )
        .unwrap();
        assert!(xml.contains(&format!("{type_id}{family}")));
        assert!(xml.find("CatalogObject.A").unwrap() < xml.find("CatalogObject.Z").unwrap());
        assert!(xml.find("CatalogObject.Z").unwrap() < xml.find(type_id).unwrap());
        let before = xml.clone();
        apply_typed_operations(
            &mut xml,
            &[source_edit(
                RelationEditMode::Add,
                &["Catalog.z", "Catalog.a"],
            )],
        )
        .unwrap();
        assert_eq!(xml, before);
        let family_add = MetaEditOperation::EditRelations {
            relation: MetaRelation::Source,
            mode: RelationEditMode::Add,
            targets: vec![MetaRelationTarget::EventSource(MetaEventSource::Family {
                source_class: unica_metadata::EventSourceClass::DocumentObject,
            })],
        };
        apply_typed_operations(&mut xml, &[family_add]).unwrap();
        let appended = "\n\t\t\t\t<d:TypeSet>c:DocumentObject</d:TypeSet>";
        assert_eq!(xml, before.replace(family, &format!("{family}{appended}")));
    }

    #[test]
    fn typed_event_source_incremental_add_declares_namespaces_and_preserves_bom_eol() {
        for eol in ["\n", "\r\n", "\r"] {
            let source = "<m:Source xmlns:m='http://v8.1c.ru/8.3/MDClasses'/>";
            let mut xml = format!("\u{feff}{}", event_subscription_xml(source))
                .replace(
                    "xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
                    "xmlns:v8=\"foreign-data\"",
                )
                .replace(
                    "xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
                    "xmlns:cfg=\"foreign-config\"",
                )
                .replace('\n', eol);
            let before = xml.clone();
            apply_typed_operations(
                &mut xml,
                &[source_edit(RelationEditMode::Add, &["Catalog.New"])],
            )
            .unwrap();
            assert!(xml.contains("</m:Source>"));
            assert!(xml.starts_with('\u{feff}'));
            let (prefix, suffix) = before.split_once(source).unwrap();
            assert!(xml.starts_with(prefix));
            assert!(xml.ends_with(suffix));
            let doc = Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
            let node =
                unica_metadata_designer::xml_model::meta_event_subscription_source_node(&doc)
                    .unwrap();
            let observed =
                unica_metadata_designer::xml_model::parse_meta_event_subscription_source(node)
                    .unwrap();
            assert_eq!(
                observed.sources[0].metadata_path().unwrap().as_str(),
                "Catalog.New"
            );
            if eol == "\r\n" {
                assert!(!xml.replace("\r\n", "").contains('\n'));
            }
            if eol == "\r" {
                assert!(!xml.contains('\n'));
            }
        }
    }

    #[test]
    fn typed_event_source_incremental_edits_refuse_invalid_and_empty_results_without_writes() {
        for source in [
            "<Source><v8:Type>cfg:CatalogObject.Items</v8:Type></Source>",
            "<Source><v8:Type>cfg:CatalogObject.Items</v8:Type><v8:TypeId>97a5ab75-0974-4551-8659-72579c7cef1e</v8:TypeId></Source>",
        ] {
            let mut xml = event_subscription_xml(source);
            let before = xml.clone();
            assert!(apply_typed_operations(&mut xml, &[source_edit(RelationEditMode::Remove, &["Catalog.Items"])]).is_err());
            assert_eq!(xml, before);
        }
        for mode in [RelationEditMode::Add, RelationEditMode::Remove] {
            let mut xml = event_subscription_xml(
                "<Source><v8:Type>cfg:CatalogObject.Items</v8:Type><Unknown/></Source>",
            );
            let before = xml.clone();
            assert!(
                apply_typed_operations(&mut xml, &[source_edit(mode, &["Catalog.Items"])]).is_err()
            );
            assert_eq!(xml, before);
        }

        for operations in [
            [
                source_edit(RelationEditMode::Remove, &["Catalog.Items"]),
                source_edit(RelationEditMode::Add, &["Catalog.Replacement"]),
            ],
            [
                source_edit(RelationEditMode::Add, &["Catalog.Replacement"]),
                source_edit(RelationEditMode::Remove, &["Catalog.Items"]),
            ],
        ] {
            let mut xml = event_subscription_xml(
                "<Source><v8:Type>cfg:CatalogObject.Items</v8:Type></Source>",
            );

            apply_typed_operations(&mut xml, &operations).unwrap();

            assert!(!xml.contains("CatalogObject.Items"), "{xml}");
            assert!(xml.contains("CatalogObject.Replacement"), "{xml}");
        }
    }

    #[test]
    fn typed_event_source_replaces_only_the_exact_direct_source_and_is_semantically_idempotent() {
        let nested_decoy = "<Wrapper><Source><v8:Type>xs:boolean</v8:Type></Source></Wrapper>";
        let mut xml = event_subscription_xml(&format!("<Source/>{nested_decoy}"));
        let operation = source_replace(vec![catalog_family()]);

        let result = apply_typed_operations(&mut xml, std::slice::from_ref(&operation)).unwrap();

        assert_eq!(result.effects.len(), 1);
        assert_eq!(result.effects[0].before, Some(serde_json::json!([])));
        assert_eq!(
            result.effects[0].after,
            Some(serde_json::json!([{
                "kind": "family",
                "sourceClass": "catalogObject",
            }]))
        );
        assert!(xml.contains(
            "<Source>\n\t\t\t\t<v8:TypeSet>cfg:CatalogObject</v8:TypeSet>\n\t\t\t</Source>"
        ));
        assert!(xml.contains(nested_decoy));

        let post_image = xml.clone();
        apply_typed_operations(&mut xml, &[operation]).unwrap();
        assert_eq!(xml, post_image);
    }

    /// Source расширения из штатной выгрузки 8.3.27: тип основной
    /// конфигурации записан `v8:TypeId` (#1361). Его не трогают ни правка
    /// другого свойства, ни замена логических источников.
    #[test]
    fn typed_event_source_replace_keeps_extension_type_ids_verbatim() {
        use unica_metadata::{MetaPropertyChanges, MetaPropertyInput};
        let type_id = "<v8:TypeId>97a5ab75-0974-4551-8659-72579c7cef1e</v8:TypeId>";
        let mut xml = event_subscription_xml(&format!(
            "<Source><v8:Type>cfg:CatalogObject.Settings</v8:Type>{type_id}</Source>"
        ));
        apply_typed_operations(
            &mut xml,
            &[MetaEditOperation::SetProperties {
                values: MetaPropertyChanges::convert(
                    MetadataKind::EventSubscription,
                    vec![MetaPropertyInput::new(
                        "Comment",
                        MetaPropertyValue::String("changed".into()),
                    )],
                )
                .unwrap(),
            }],
        )
        .unwrap();
        assert!(xml.contains("<Comment>changed</Comment>"), "{xml}");
        assert!(xml.contains(type_id), "{xml}");

        // The visible logical members unchanged: an exact-byte no-op.
        let visible = source_replace(vec![MetaEventSource::Object {
            metadata_path: MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "Catalog.Settings",
            )
            .unwrap(),
        }]);
        let before = xml.clone();
        apply_typed_operations(&mut xml, std::slice::from_ref(&visible)).unwrap();
        assert_eq!(xml, before);

        // A changed logical list keeps the TypeId after the logical members.
        apply_typed_operations(&mut xml, &[source_replace(vec![catalog_family()])]).unwrap();
        let family = xml
            .find("<v8:TypeSet>cfg:CatalogObject</v8:TypeSet>")
            .unwrap_or_else(|| panic!("{xml}"));
        let kept = xml.find(type_id).unwrap_or_else(|| panic!("{xml}"));
        assert!(family < kept, "{xml}");
        assert!(!xml.contains("CatalogObject.Settings"), "{xml}");
    }

    #[test]
    fn typed_event_source_semantic_noop_is_order_insensitive_and_preserves_exact_bytes() {
        let source = concat!(
            "<Source>",
            "<v8:Type>cfg:CatalogObject.Items</v8:Type>",
            "<v8:Type>cfg:DocumentObject.Sale</v8:Type>",
            "</Source>"
        );
        let mut xml = event_subscription_xml(source);
        let before = xml.clone();
        let operation = source_replace(vec![
            MetaEventSource::Object {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Document.Sale",
                )
                .unwrap(),
            },
            MetaEventSource::Object {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Catalog.Items",
                )
                .unwrap(),
            },
        ]);

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        assert_eq!(xml, before);
    }

    #[test]
    fn typed_event_source_alias_qname_noop_is_byte_exact_and_changes_reuse_aliases() {
        let source = concat!(
            "<Source>",
            "<d:Type>c:CatalogObject.Items</d:Type>",
            "<d:TypeSet>c:CatalogObject</d:TypeSet>",
            "</Source>"
        );
        let mut xml = event_subscription_alias_xml(source);
        let before = xml.clone();
        let object = MetaEventSource::Object {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
        };

        apply_typed_operations(
            &mut xml,
            &[source_replace(vec![object.clone(), catalog_family()])],
        )
        .unwrap();
        assert_eq!(xml, before);

        apply_typed_operations(&mut xml, &[source_replace(vec![object])]).unwrap();
        assert!(xml.contains("<d:Type>c:CatalogObject.Items</d:Type>"));
        assert!(!xml.contains("<v8:") && !xml.contains("cfg:") && !xml.contains("xs:"));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn typed_event_source_change_declares_missing_canonical_namespaces_locally() {
        let mut xml = event_subscription_alias_xml("<Source/>")
            .replace(
                r#" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config""#,
                "",
            )
            .replace(r#" xmlns:s="http://www.w3.org/2001/XMLSchema""#, "");
        let target = MetaEventSource::Object {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
        };

        apply_typed_operations(&mut xml, &[source_replace(vec![target])]).unwrap();

        assert!(xml.contains(r#"xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config""#));
        assert!(xml.contains("<d:Type>cfg:CatalogObject.Items</d:Type>"));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn typed_event_source_change_does_not_reuse_declarations_on_replaced_source() {
        let local_source = concat!(
            r#"<Source xmlns:d="http://v8.1c.ru/8.1/data/core" "#,
            r#"xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config" "#,
            r#"xmlns:s="http://www.w3.org/2001/XMLSchema">"#,
            "<d:Type>s:boolean</d:Type>",
            "</Source>"
        );
        let mut xml = event_subscription_alias_xml(local_source)
            .replacen(
                r#" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config""#,
                "",
                1,
            )
            .replacen(r#" xmlns:d="http://v8.1c.ru/8.1/data/core""#, "", 1)
            .replacen(r#" xmlns:s="http://www.w3.org/2001/XMLSchema""#, "", 1);
        let target = MetaEventSource::Object {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
        };

        apply_typed_operations(&mut xml, &[source_replace(vec![target])]).unwrap();

        assert!(xml.contains(
            r#"<Source xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">"#
        ));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn typed_event_source_build_preserves_bom_and_crlf_for_change_and_noop() {
        let lf = event_subscription_xml("<Source/>");
        let mut preimage = b"\xef\xbb\xbf".to_vec();
        preimage.extend_from_slice(lf.replace('\n', "\r\n").as_bytes());
        let operation = source_replace(vec![catalog_family()]);
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let context = unica_source_contract::workspace::WorkspaceContext {
            cwd: std::path::PathBuf::from("/unused"),
            workspace_root: std::path::PathBuf::from("/unused"),
            cache_root: std::path::PathBuf::from("/unused/.build"),
            workspace_epoch: 0,
        };

        let changed = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            &preimage,
            std::slice::from_ref(&operation),
            &context,
        )
        .unwrap()
        .descriptor;

        assert!(changed.starts_with(b"\xef\xbb\xbf"));
        assert!(changed.windows(2).any(|pair| pair == b"\r\n"));
        assert!(!changed
            .iter()
            .enumerate()
            .any(|(index, byte)| *byte == b'\n' && (index == 0 || changed[index - 1] != b'\r')));

        let noop = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            &changed,
            std::slice::from_ref(&operation),
            &context,
        )
        .unwrap()
        .descriptor;
        assert_eq!(noop, changed);
    }

    #[test]
    fn typed_event_source_build_preserves_cr_only_indentation() {
        let preimage = event_subscription_xml("<Source/>")
            .replace('\n', "\r")
            .into_bytes();
        let operation = source_replace(vec![catalog_family()]);
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let context = unica_source_contract::workspace::WorkspaceContext {
            cwd: std::path::PathBuf::from("/unused"),
            workspace_root: std::path::PathBuf::from("/unused"),
            cache_root: std::path::PathBuf::from("/unused/.build"),
            workspace_epoch: 0,
        };

        let changed = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            &preimage,
            &[operation],
            &context,
        )
        .unwrap()
        .descriptor;
        let changed = String::from_utf8(changed).unwrap();

        assert!(!changed.contains('\n'), "{changed:?}");
        assert!(
            changed
                .contains("\t\t\t<Source>\r\t\t\t\t<v8:TypeSet>cfg:CatalogObject</v8:TypeSet>\r\t\t\t</Source>"),
            "{changed:?}"
        );
    }

    #[test]
    fn typed_event_source_build_rejects_mixed_eol_before_source_only_serialization() {
        let mixed = event_subscription_xml("<Source/>").replacen('\n', "\r\n", 1);
        let operation = source_replace(vec![catalog_family()]);
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "EventSubscription.Events")
                .unwrap();
        let context = unica_source_contract::workspace::WorkspaceContext {
            cwd: std::path::PathBuf::from("/unused"),
            workspace_root: std::path::PathBuf::from("/unused"),
            cache_root: std::path::PathBuf::from("/unused/.build"),
            workspace_epoch: 0,
        };

        let failure = build_typed_operation_post_image(
            TypedOperationDependencyScope::new(
                "main",
                std::path::Path::new("/unused"),
                std::path::Path::new("/unused/Configuration.xml"),
                b"",
                false,
            ),
            std::path::Path::new("/unused/EventSubscriptions/Events.xml"),
            &target,
            mixed.as_bytes(),
            &[operation],
            &context,
        )
        .err()
        .expect("mixed EOL must be rejected before publication planning");

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert!(
            failure.diagnostics[0]
                .message
                .contains("mixed line endings"),
            "{:?}",
            failure.diagnostics
        );
    }

    /// #1325: регистрация, встроенная во владельца прежними версиями Unica,
    /// узнаётся по имени метаданных без учёта регистра и остаётся встроенной;
    /// новая — сворачивается в ссылку по имени.
    #[test]
    fn collapse_keeps_the_owner_namespace_prefix_of_a_new_registration() {
        let pre = r#"<md:MetaDataObject xmlns:md="http://v8.1c.ru/8.3/MDClasses" version="2.20"><md:Catalog uuid="11111111-1111-4111-8111-111111111111"><md:Properties><md:Name>Items</md:Name></md:Properties><md:ChildObjects/></md:Catalog></md:MetaDataObject>"#;
        let mut post = String::from(
            r#"<md:MetaDataObject xmlns:md="http://v8.1c.ru/8.3/MDClasses" version="2.20"><md:Catalog uuid="11111111-1111-4111-8111-111111111111"><md:Properties><md:Name>Items</md:Name></md:Properties><md:ChildObjects><md:Template uuid="22222222-2222-4222-8222-222222222222"><md:Properties><md:Name>Print</md:Name></md:Properties></md:Template></md:ChildObjects></md:Catalog></md:MetaDataObject>"#,
        );
        collapse_standalone_child_descriptors(pre, &mut post);
        assert!(
            post.contains("<md:ChildObjects><md:Template>Print</md:Template></md:ChildObjects>"),
            "{post}"
        );
        let document = Document::parse(&post).unwrap();
        let object = unica_metadata_designer::edit::meta_edit_object_node(&document).unwrap();
        let children = unica_xml_format::read::child_element(object, "ChildObjects").unwrap();
        assert_eq!(
            unica_xml_format::read::child_elements(children, "Template").len(),
            1
        );
    }

    #[test]
    fn collapse_keeps_legacy_inline_registration_matched_case_insensitively() {
        let owner = |children: &str| {
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Catalog uuid=\"11111111-1111-4111-8111-111111111111\"><Properties><Name>Owner</Name></Properties><ChildObjects>{children}</ChildObjects></Catalog></MetaDataObject>"
            )
        };
        let inline = |name: &str| {
            format!(
                "<Template uuid=\"22222222-2222-4222-8222-222222222222\"><Properties><Name>{name}</Name></Properties></Template>"
            )
        };
        let pre_image = owner(&inline("Print"));
        let mut post_image = owner(&format!(
            "{}<Template uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Other</Name></Properties></Template>",
            inline("PRINT")
        ));
        collapse_standalone_child_descriptors(&pre_image, &mut post_image);
        assert_eq!(
            post_image,
            owner(&format!("{}<Template>Other</Template>", inline("PRINT")))
        );
    }

    #[test]
    fn typed_relation_registry_matches_template_nodes_and_guards_private_images() {
        use unica_metadata::{
            metadata_relation_spec, MetaRelationTarget, MetaRelationTargetPolicy,
        };

        for kind in MetadataKind::ALL {
            for relation in MetaRelation::ALL {
                let (template, _) =
                    unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                        *kind, "Owner",
                    )
                    .unwrap();
                let tag = match relation {
                    MetaRelation::Owners => "Owners",
                    MetaRelation::RegisterRecords => "RegisterRecords",
                    MetaRelation::BasedOn => "BasedOn",
                    MetaRelation::InputByString => "InputByString",
                    MetaRelation::Source => "Source",
                };
                let document = Document::parse(&template).unwrap();
                let has_node = document.descendants().any(|node| {
                    node.is_element()
                        && node.tag_name().name() == tag
                        && node.parent().is_some_and(|parent| {
                            parent.is_element() && parent.tag_name().name() == "Properties"
                        })
                });
                let spec = metadata_relation_spec(*kind, *relation);
                assert_eq!(has_node, spec.is_some(), "{} {tag}", kind.as_str());

                let owner = format!("{}.Owner", kind.as_str());
                let target = match spec.map(|spec| spec.target_policy) {
                    Some(MetaRelationTargetPolicy::MetadataKinds(kinds)) => {
                        MetaRelationTarget::Object(metadata_reference(&format!(
                            "{}.Target",
                            kinds[0].as_str()
                        )))
                    }
                    Some(MetaRelationTargetPolicy::SameOwnerField) => {
                        let field = unica_metadata_designer::template_catalog::metadata_standard_attribute_names(kind.as_str())
                            .first()
                            .copied()
                            .expect("every inputByString owner has a standard field");
                        MetaRelationTarget::Field(
                            unica_metadata::MetadataFieldPath::parse(&format!(
                                "{owner}.StandardAttribute.{field}"
                            ))
                            .unwrap(),
                        )
                    }
                    Some(MetaRelationTargetPolicy::EventSources) => {
                        MetaRelationTarget::EventSource(catalog_family())
                    }
                    None => match relation {
                        MetaRelation::InputByString => MetaRelationTarget::Field(
                            unica_metadata::MetadataFieldPath::parse(&format!(
                                "{owner}.StandardAttribute.Name"
                            ))
                            .unwrap(),
                        ),
                        MetaRelation::Owners => {
                            MetaRelationTarget::Object(metadata_reference("Catalog.Target"))
                        }
                        MetaRelation::RegisterRecords => MetaRelationTarget::Object(
                            metadata_reference("InformationRegister.Target"),
                        ),
                        MetaRelation::BasedOn => MetaRelationTarget::Object(metadata_reference(
                            &format!("{}.Target", kind.as_str()),
                        )),
                        MetaRelation::Source => MetaRelationTarget::EventSource(catalog_family()),
                    },
                };
                let operation = MetaEditOperation::edit_relation_targets(
                    *relation,
                    RelationEditMode::Replace,
                    vec![target],
                )
                .unwrap();
                let mut working = template.clone();
                let result = apply_typed_operations(&mut working, &[operation]);
                if spec.is_some() {
                    result.unwrap_or_else(|failure| {
                        panic!("{} {tag}: {:?}", kind.as_str(), failure.diagnostics)
                    });
                    assert_ne!(working, template, "{} {tag}", kind.as_str());
                } else {
                    let failure = result.expect_err(&format!("{} {tag}", kind.as_str()));
                    assert_eq!(
                        failure.diagnostics[0].field.as_deref(),
                        Some("operations[0].relation")
                    );
                    assert_eq!(working, template, "{} {tag}", kind.as_str());
                }
            }
        }
    }

    #[test]
    fn typed_fill_value_registry_guards_every_public_field_context_before_mutation() {
        use unica_metadata::{metadata_fill_value_is_allowed, MetaElementScope};

        let filled = |name: &str| MetaElementInput {
            name: name.into(),
            r#type: Some(
                MetadataType::new(vec![MetadataTypeVariant::String {
                    length: 20,
                    allowed_length: StringLengthMode::Variable,
                }])
                .unwrap(),
            ),
            fill_value: Some(MetaFillValue::String("value".into())),
            ..MetaElementInput::default()
        };

        for kind in MetadataKind::ALL {
            for collection in [
                MetaCollection::Attributes,
                MetaCollection::Dimensions,
                MetaCollection::Resources,
            ] {
                if !unica_metadata::metadata_kind_collections(*kind).contains(&collection) {
                    continue;
                }
                let (template, _) =
                    unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                        *kind, "Owner",
                    )
                    .unwrap();
                let operation =
                    MetaEditOperation::add(collection, None, vec![filled("Filled")]).unwrap();
                let mut working = template.clone();
                let result = apply_typed_operations(&mut working, &[operation]);
                let allowed =
                    metadata_fill_value_is_allowed(*kind, collection, MetaElementScope::TopLevel);
                if allowed {
                    result.unwrap_or_else(|failure| {
                        panic!(
                            "{} {}: {:?}",
                            kind.as_str(),
                            collection.as_str(),
                            failure.diagnostics
                        )
                    });
                    assert!(working.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));
                } else {
                    let failure =
                        result.expect_err(&format!("{} {}", kind.as_str(), collection.as_str()));
                    assert_eq!(
                        failure.diagnostics[0].field.as_deref(),
                        Some("operations[0].elements[0].fillValue")
                    );
                    assert_eq!(working, template);
                }
            }

            if !unica_metadata::metadata_kind_collections(*kind)
                .contains(&MetaCollection::TabularSections)
            {
                continue;
            }
            let (template, _) =
                unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                    *kind, "Owner",
                )
                .unwrap();
            let operation = MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![filled("Filled")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap();
            let mut working = template.clone();
            let result = apply_typed_operations(&mut working, &[operation]);
            let allowed = metadata_fill_value_is_allowed(
                *kind,
                MetaCollection::Attributes,
                MetaElementScope::TabularSection,
            );
            if allowed {
                result.unwrap_or_else(|failure| {
                    panic!("{} scoped: {:?}", kind.as_str(), failure.diagnostics)
                });
                assert!(working.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));
            } else {
                let failure = result.expect_err(&format!("{} scoped", kind.as_str()));
                assert_eq!(
                    failure.diagnostics[0].field.as_deref(),
                    Some("operations[0].elements[0].attributes[0].fillValue")
                );
                assert_eq!(working, template);
            }
        }
    }

    #[test]
    fn typed_failures_are_indexed_and_never_upsert_missing_targets() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let original = xml.clone();
        let operations = vec![
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Created")],
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
        ];

        let failure = apply_typed_operations(&mut xml, &operations).unwrap_err();

        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].code,
            unica_metadata::MetaDiagnosticCode::TargetNotFound
        );
        assert_eq!(
            xml, original,
            "failed ordered edits must leave the caller image unchanged"
        );
    }

    #[test]
    fn typed_duplicate_rename_collision_and_invalid_scope_have_stable_diagnostics() {
        let mut duplicate_xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let duplicates = [
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Same")],
            )
            .unwrap(),
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Same")],
            )
            .unwrap(),
        ];
        let failure = apply_typed_operations(&mut duplicate_xml, &duplicates).unwrap_err();
        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );

        let mut collision_xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let operations = [
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![
                    MetaElementInput::named("First"),
                    MetaElementInput::named("Second"),
                ],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "First".into(),
                    new_name: Some("Second".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ];
        let failure = apply_typed_operations(&mut collision_xml, &operations).unwrap_err();
        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );

        let mut scope_xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let invalid_scope = MetaEditOperation::add(
            MetaCollection::Attributes,
            Some(MetaScope {
                tabular_section: "Missing".into(),
            }),
            vec![MetaElementInput::named("Value")],
        )
        .unwrap();
        let failure = apply_typed_operations(&mut scope_xml, &[invalid_scope]).unwrap_err();
        assert_eq!(failure.diagnostics[0].operation_index, Some(0));
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::TargetNotFound
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[0].scope.tabularSection")
        );
    }

    #[test]
    fn typed_scoped_add_missing_anchor_reports_the_exact_element_position() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let operations = [
            MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![MetaElementInput::named("Existing")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::add(
                MetaCollection::Attributes,
                Some(MetaScope {
                    tabular_section: "Lines".into(),
                }),
                vec![MetaElementInput {
                    name: "Inserted".into(),
                    position: Some(MetaPosition::new(Some("Missing".into()), None).unwrap()),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
        ];

        let failure = apply_typed_operations(&mut xml, &operations).unwrap_err();

        assert_eq!(failure.diagnostics[0].operation_index, Some(1));
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].position")
        );
    }

    #[test]
    fn typed_invalid_property_is_rejected_before_xml_mutation() {
        let diagnostic = MetaPropertyChanges::convert(
            unica_metadata::MetadataKind::Catalog,
            vec![MetaPropertyInput::new(
                "NumberLength",
                MetaPropertyValue::UnsignedInteger(10),
            )],
        )
        .unwrap_err();

        assert_eq!(diagnostic.code, MetaDiagnosticCode::UnsupportedKind);
        assert_eq!(diagnostic.field.as_deref(), Some("values.NumberLength"));
    }

    #[test]
    fn typed_kind_collection_matrix_is_exact_and_reports_the_full_operation_field() {
        use unica_metadata::MetadataKind;

        let all = [
            MetaCollection::Attributes,
            MetaCollection::TabularSections,
            MetaCollection::Dimensions,
            MetaCollection::Resources,
            MetaCollection::EnumValues,
            MetaCollection::Columns,
            MetaCollection::Forms,
            MetaCollection::Templates,
            MetaCollection::Commands,
        ];
        let cases: &[(MetadataKind, &[MetaCollection])] = &[
            (
                MetadataKind::Catalog,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::Document,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (MetadataKind::Enum, &[all[4], all[6], all[7], all[8]]),
            (MetadataKind::Constant, &[all[6]]),
            (
                MetadataKind::InformationRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::AccumulationRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::AccountingRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::CalculationRegister,
                &[all[0], all[2], all[3], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ChartOfAccounts,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ChartOfCharacteristicTypes,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ChartOfCalculationTypes,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::BusinessProcess,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::Task,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::ExchangePlan,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::DocumentJournal,
                &[all[5], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::Report,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (
                MetadataKind::DataProcessor,
                &[all[0], all[1], all[6], all[7], all[8]],
            ),
            (MetadataKind::CommonModule, &[]),
            (MetadataKind::ScheduledJob, &[]),
            (MetadataKind::EventSubscription, &[]),
            (MetadataKind::HTTPService, &[]),
            (MetadataKind::WebService, &[]),
            (MetadataKind::DefinedType, &[]),
        ];

        assert_eq!(cases.len(), MetadataKind::ALL.len());
        for (kind, allowed) in cases {
            for collection in all {
                let mut xml = object_xml(kind.as_str(), "Sample", "");
                let operation = MetaEditOperation::add(
                    collection,
                    None,
                    vec![MetaElementInput::named("Child")],
                )
                .unwrap();
                let result = apply_typed_operations(&mut xml, &[operation]);
                if allowed.contains(&collection) {
                    assert!(result.is_ok(), "{kind:?}.{collection:?}: {result:?}");
                } else {
                    let failure = result.unwrap_err();
                    assert_eq!(failure.diagnostics[0].operation_index, Some(0));
                    assert_eq!(
                        failure.diagnostics[0].code,
                        MetaDiagnosticCode::UnsupportedKind,
                        "{kind:?}.{collection:?}"
                    );
                    assert_eq!(
                        failure.diagnostics[0].field.as_deref(),
                        Some("operations[0].collection"),
                        "{kind:?}.{collection:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn typed_inline_tabular_attributes_keep_fields_and_apply_nested_positions() {
        let mut xml = object_xml("Report", "Sales", "");
        let operation = MetaEditOperation::add(
            MetaCollection::TabularSections,
            None,
            vec![MetaElementInput {
                name: "Lines".into(),
                attributes: Some(vec![
                    MetaElementInput::named("Second"),
                    MetaElementInput {
                        name: "First".into(),
                        comment: Some("typed nested field".into()),
                        r#type: Some(
                            MetadataType::new(vec![MetadataTypeVariant::Number {
                                digits: 12,
                                fraction: 2,
                                sign: NumberSign::NonNegative,
                            }])
                            .unwrap(),
                        ),
                        required: Some(true),
                        fill_value: Some(MetaFillValue::Number("3.50".into())),
                        position: Some(MetaPosition::new(Some("Second".into()), None).unwrap()),
                        ..MetaElementInput::default()
                    },
                ]),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        let first = xml.find("<Name>First</Name>").unwrap();
        let second = xml.find("<Name>Second</Name>").unwrap();
        assert!(first < second, "nested position.before was ignored\n{xml}");
        let first_xml = &xml[xml[..first].rfind("<Attribute ").unwrap()
            ..xml[first..].find("</Attribute>").unwrap() + first + "</Attribute>".len()];
        assert!(first_xml.contains("<Comment>typed nested field</Comment>"));
        assert!(first_xml.contains("<v8:Type>xs:decimal</v8:Type>"));
        assert!(first_xml.contains("<v8:AllowedSign>Nonnegative</v8:AllowedSign>"));
        assert!(first_xml.contains("<FillValue xsi:type=\"xs:decimal\">3.50</FillValue>"));
        assert!(first_xml.contains("<FillChecking>ShowError</FillChecking>"));
    }

    #[test]
    fn typed_top_level_attributes_use_the_kind_aware_platform_profile() {
        for (kind, forbidden) in [
            (
                "Report",
                &[
                    "<FillFromFillingValue>",
                    "<FillValue ",
                    "<Indexing>",
                    "<FullTextSearch>",
                    "<DataHistory>",
                ][..],
            ),
            (
                "AccumulationRegister",
                &["<FillFromFillingValue>", "<FillValue ", "<DataHistory>"][..],
            ),
        ] {
            let mut xml = object_xml(kind, "Sample", "");
            let operation = MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput::named("Profiled")],
            )
            .unwrap();
            apply_typed_operations(&mut xml, &[operation]).unwrap();
            for tag in forbidden {
                assert!(!xml.contains(tag), "{kind} emitted forbidden {tag}\n{xml}");
            }
        }
    }

    /// The `<Attribute>` named `column` inside tabular section `section`:
    /// its source block with the line indent and uuids masked, and the
    /// ordered names of its properties.
    fn tabular_attribute_shape(xml: &str, section: &str, column: &str) -> (String, Vec<String>) {
        let document = Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
        let offset = xml.len() - xml.trim_start_matches('\u{feff}').len();
        let attribute = document
            .descendants()
            .filter(|node| node.has_tag_name("TabularSection"))
            .find(|node| {
                unica_metadata_designer::edit::meta_edit_child_object_name(*node).as_deref()
                    == Some(section)
            })
            .and_then(|node| unica_xml_format::read::child_element(node, "ChildObjects"))
            .into_iter()
            .flat_map(|node| unica_xml_format::read::child_elements(node, "Attribute"))
            .find(|node| {
                unica_metadata_designer::edit::meta_edit_child_object_name(*node).as_deref()
                    == Some(column)
            })
            .unwrap_or_else(|| panic!("{section}.{column} is not a tabular attribute\n{xml}"));
        let properties = unica_xml_format::read::child_element(attribute, "Properties")
            .unwrap()
            .children()
            .filter(roxmltree::Node::is_element)
            .map(|node| node.tag_name().name().to_string())
            .collect();
        let range = attribute.range();
        let start = offset + range.start;
        let line_start = xml[..start].rfind('\n').map_or(0, |index| index + 1);
        let block = &xml[line_start..offset + range.end];
        let uuid = regex::Regex::new(r#"uuid="[^"]*""#).unwrap();
        (
            uuid.replace_all(block, "uuid=\"*\"").into_owned(),
            properties,
        )
    }

    #[test]
    fn typed_scoped_attribute_add_uses_the_tabular_section_attribute_profile() {
        let dumps = [
            (
                "Catalog",
                include_str!(
                    "../../../../../../tests/fixtures/acceptance/workspace/src/Catalogs/Валюты.xml"
                ),
                "Представления",
                "КодЯзыка",
            ),
            (
                "Document",
                include_str!(
                    "../../../../../../tests/fixtures/acceptance/workspace/src/Documents/АктОбУничтоженииПерсональныхДанных.xml"
                ),
                "КатегорииДанных",
                "",
            ),
        ];
        for (kind, dump, dump_section, dump_column) in dumps {
            let section = |attributes: Vec<MetaElementInput>| {
                MetaEditOperation::add(
                    MetaCollection::TabularSections,
                    None,
                    vec![MetaElementInput {
                        name: "Lines".into(),
                        attributes: Some(attributes),
                        ..MetaElementInput::default()
                    }],
                )
                .unwrap()
            };

            let mut created = object_xml(kind, "Owner", "");
            apply_typed_operations(
                &mut created,
                &[section(vec![
                    MetaElementInput::named("Seed"),
                    MetaElementInput::named("Column"),
                ])],
            )
            .unwrap();

            let mut scoped = object_xml(kind, "Owner", "");
            apply_typed_operations(
                &mut scoped,
                &[
                    section(vec![MetaElementInput::named("Seed")]),
                    MetaEditOperation::add(
                        MetaCollection::Attributes,
                        Some(MetaScope {
                            tabular_section: "Lines".into(),
                        }),
                        vec![MetaElementInput::named("Column")],
                    )
                    .unwrap(),
                ],
            )
            .unwrap();

            let (created_block, created_properties) =
                tabular_attribute_shape(&created, "Lines", "Column");
            let (scoped_block, scoped_properties) =
                tabular_attribute_shape(&scoped, "Lines", "Column");
            assert_eq!(
                scoped_block, created_block,
                "{kind}: attribute.add with scope differs from tabularSection.add"
            );
            assert_eq!(scoped_properties, created_properties);
            for forbidden in ["FillFromFillingValue", "FillValue", "Use"] {
                assert!(
                    !scoped_properties.iter().any(|name| name == forbidden),
                    "{kind}: tabular attribute carries {forbidden}\n{scoped_block}"
                );
            }

            // The platform's own dump of a tabular-section attribute of the
            // same owner kind carries exactly this property set and order.
            let document = Document::parse(dump.trim_start_matches('\u{feff}')).unwrap();
            let dump_column = if dump_column.is_empty() {
                document
                    .descendants()
                    .filter(|node| node.has_tag_name("TabularSection"))
                    .find(|node| {
                        unica_metadata_designer::edit::meta_edit_child_object_name(*node).as_deref()
                            == Some(dump_section)
                    })
                    .and_then(|node| unica_xml_format::read::child_element(node, "ChildObjects"))
                    .into_iter()
                    .flat_map(|node| unica_xml_format::read::child_elements(node, "Attribute"))
                    .find_map(unica_metadata_designer::edit::meta_edit_child_object_name)
                    .unwrap()
            } else {
                dump_column.to_string()
            };
            let (_, dump_properties) = tabular_attribute_shape(dump, dump_section, &dump_column);
            assert_eq!(
                scoped_properties, dump_properties,
                "{kind}: tabular attribute profile differs from the 8.3.27 dump"
            );
        }
    }

    #[test]
    fn typed_composite_type_uses_platform_node_order() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let operation = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput {
                name: "Composite".into(),
                r#type: Some(
                    MetadataType::new(vec![
                        MetadataTypeVariant::DefinedType {
                            metadata_path: metadata_reference("DefinedType.ExternalCode")
                                .metadata_path,
                        },
                        MetadataTypeVariant::Number {
                            digits: 12,
                            fraction: 2,
                            sign: NumberSign::Any,
                        },
                        MetadataTypeVariant::String {
                            length: 20,
                            allowed_length: StringLengthMode::Variable,
                        },
                    ])
                    .unwrap(),
                ),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[operation]).unwrap();

        let decimal = xml.find("<v8:Type>xs:decimal</v8:Type>").unwrap();
        let string = xml.find("<v8:Type>xs:string</v8:Type>").unwrap();
        let type_set = xml
            .find("<v8:TypeSet>cfg:DefinedType.ExternalCode</v8:TypeSet>")
            .unwrap();
        let number_qualifiers = xml.find("<v8:NumberQualifiers>").unwrap();
        let string_qualifiers = xml.find("<v8:StringQualifiers>").unwrap();
        assert!(decimal < string && string < type_set);
        assert!(type_set < number_qualifiers && number_qualifiers < string_qualifiers);
    }

    #[test]
    fn typed_composite_type_canonicalizes_qualifiers_for_variant_permutations() {
        let metadata_type = MetadataType::new(vec![
            MetadataTypeVariant::Date {
                fractions: DateFractions::DateTime,
            },
            MetadataTypeVariant::BinaryData {
                length: 512,
                allowed_length: StringLengthMode::Fixed,
            },
            MetadataTypeVariant::String {
                length: 40,
                allowed_length: StringLengthMode::Variable,
            },
            MetadataTypeVariant::Number {
                digits: 12,
                fraction: 2,
                sign: NumberSign::Any,
            },
        ])
        .unwrap();
        let mut lines = Vec::new();

        unica_metadata_designer::xml_model::emit_meta_typed_value_type(
            &mut lines,
            "",
            &metadata_type,
        );

        let xml = lines.join("\n");
        let number = xml.find("<v8:NumberQualifiers>").unwrap();
        let string = xml.find("<v8:StringQualifiers>").unwrap();
        let date = xml.find("<v8:DateQualifiers>").unwrap();
        let binary = xml.find("<v8:BinaryDataQualifiers>").unwrap();
        assert!(number < string && string < date && date < binary, "{xml}");
    }

    #[test]
    fn typed_type_and_fill_validation_rejects_invalid_profile_values_with_full_fields() {
        let cases = [
            (
                MetadataType::new(vec![MetadataTypeVariant::Reference {
                    metadata_path: metadata_reference("Report.Sales").metadata_path,
                }])
                .unwrap(),
                None,
                "operations[0].elements[0].type.variants[0].metadataPath",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::DefinedType {
                    metadata_path: metadata_reference("Catalog.Items").metadata_path,
                }])
                .unwrap(),
                None,
                "operations[0].elements[0].type.variants[0].metadataPath",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::Number {
                    digits: 10,
                    fraction: 2,
                    sign: NumberSign::Any,
                }])
                .unwrap(),
                Some(MetaFillValue::Number("1e3".into())),
                "operations[0].elements[0].fillValue",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::Date {
                    fractions: DateFractions::DateTime,
                }])
                .unwrap(),
                Some(MetaFillValue::DateTime("2026-02-30T25:61:00".into())),
                "operations[0].elements[0].fillValue",
            ),
            (
                MetadataType::new(vec![MetadataTypeVariant::String {
                    length: 10,
                    allowed_length: StringLengthMode::Variable,
                }])
                .unwrap(),
                Some(MetaFillValue::Boolean(true)),
                "operations[0].elements[0].fillValue",
            ),
        ];

        for (metadata_type, fill_value, expected_field) in cases {
            let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
            let operation = MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput {
                    name: "Invalid".into(),
                    r#type: Some(metadata_type),
                    fill_value,
                    ..MetaElementInput::default()
                }],
            )
            .unwrap();
            let failure = apply_typed_operations(&mut xml, &[operation]).unwrap_err();
            assert_eq!(
                failure.diagnostics[0].code,
                MetaDiagnosticCode::InvalidArguments
            );
            assert_eq!(
                failure.diagnostics[0].field.as_deref(),
                Some(expected_field)
            );
        }
    }

    #[test]
    fn typed_fill_value_legality_follows_owner_and_scope_context_for_add_and_update() {
        let string_type = || {
            MetadataType::new(vec![MetadataTypeVariant::String {
                length: 20,
                allowed_length: StringLengthMode::Variable,
            }])
            .unwrap()
        };
        let filled = |name: &str| MetaElementInput {
            name: name.into(),
            r#type: Some(string_type()),
            fill_value: Some(MetaFillValue::String("value".into())),
            ..MetaElementInput::default()
        };

        let mut report = object_xml("Report", "Sales", "");
        let failure = apply_typed_operations(
            &mut report,
            &[
                MetaEditOperation::add(MetaCollection::Attributes, None, vec![filled("Top")])
                    .unwrap(),
            ],
        )
        .unwrap_err();
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[0].elements[0].fillValue")
        );
        assert!(!report.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut catalog = object_xml("Catalog", "Items", "");
        apply_typed_operations(
            &mut catalog,
            &[
                MetaEditOperation::add(MetaCollection::Attributes, None, vec![filled("Top")])
                    .unwrap(),
            ],
        )
        .unwrap();
        assert!(catalog.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut stored_tabular = object_xml("Catalog", "Items", "");
        let failure = apply_typed_operations(
            &mut stored_tabular,
            &[MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![filled("Value")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap()],
        )
        .unwrap_err();
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[0].elements[0].attributes[0].fillValue")
        );
        assert!(!stored_tabular.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut report_tabular = object_xml("Report", "Sales", "");
        apply_typed_operations(
            &mut report_tabular,
            &[MetaEditOperation::add(
                MetaCollection::TabularSections,
                None,
                vec![MetaElementInput {
                    name: "Lines".into(),
                    attributes: Some(vec![filled("Value")]),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap()],
        )
        .unwrap();
        assert!(report_tabular.contains("<FillValue xsi:type=\"xs:string\">value</FillValue>"));

        let mut report_update = object_xml("Report", "Sales", "");
        let operations = [
            MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput {
                    name: "Top".into(),
                    r#type: Some(string_type()),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::update(
                MetaCollection::Attributes,
                None,
                vec![MetaElementUpdateInput {
                    name: "Top".into(),
                    fill_value: Some(MetaFillValue::String("value".into())),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
        ];
        let failure = apply_typed_operations(&mut report_update, &operations).unwrap_err();
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].fillValue")
        );
    }

    #[test]
    fn typed_update_validates_new_type_against_the_existing_fill_value() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput {
                name: "CodeText".into(),
                r#type: Some(
                    MetadataType::new(vec![MetadataTypeVariant::String {
                        length: 10,
                        allowed_length: StringLengthMode::Variable,
                    }])
                    .unwrap(),
                ),
                fill_value: Some(MetaFillValue::String("ABC".into())),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "CodeText".into(),
                r#type: Some(
                    MetadataType::new(vec![MetadataTypeVariant::Number {
                        digits: 10,
                        fraction: 0,
                        sign: NumberSign::NonNegative,
                    }])
                    .unwrap(),
                ),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();

        let failure = apply_typed_operations(&mut xml, &[add, update]).unwrap_err();

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::InvalidArguments
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].type")
        );
    }

    #[test]
    fn typed_update_validates_new_fill_value_against_the_existing_type() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput {
                name: "CodeText".into(),
                r#type: Some(
                    MetadataType::new(vec![MetadataTypeVariant::String {
                        length: 10,
                        allowed_length: StringLengthMode::Variable,
                    }])
                    .unwrap(),
                ),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "CodeText".into(),
                fill_value: Some(MetaFillValue::Boolean(true)),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();

        let failure = apply_typed_operations(&mut xml, &[add, update]).unwrap_err();

        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::InvalidArguments
        );
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("operations[1].elements[0].fillValue")
        );
    }

    #[test]
    fn empty_design_time_ref_is_unset_without_accepting_malformed_fill_values() {
        for fill in [
            r#"<FillValue xsi:type="xr:DesignTimeRef"/>"#,
            r#"<FillValue xsi:type="xr:DesignTimeRef"></FillValue>"#,
            r#"<FillValue xmlns:r="http://v8.1c.ru/8.3/xcf/readable" xsi:type="r:DesignTimeRef"/>"#,
        ] {
            let xml = format!(
                r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{fill}</Properties>"#
            );
            let document = Document::parse(&xml).unwrap();
            assert_eq!(
                unica_metadata_designer::edit::parse_typed_fill_value_node(document.root_element())
                    .unwrap(),
                None,
                "{fill}"
            );
        }
        for fill in [
            "<FillValue/>",
            r#"<FillValue xsi:type="xr:DesignTimeRef"> </FillValue>"#,
            r#"<FillValue xsi:type="xr:DesignTimeRef">broken</FillValue>"#,
            r#"<FillValue xmlns:r="urn:foreign" xsi:type="r:DesignTimeRef"/>"#,
            r#"<FillValue xsi:type="xr:DesignTimeRef" extra="x"/>"#,
            r#"<FillValue xsi:type="xr:DesignTimeRef"><Nested/></FillValue>"#,
            r#"<FillValue xsi:type="xr:DesignTimeRef"/><FillValue xsi:type="xr:DesignTimeRef"/>"#,
        ] {
            let xml = format!(
                r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{fill}</Properties>"#
            );
            let document = Document::parse(&xml).unwrap();
            assert!(
                unica_metadata_designer::edit::parse_typed_fill_value_node(document.root_element())
                    .is_err(),
                "{fill}"
            );
        }
    }

    #[test]
    fn adding_boolean_preserves_an_existing_empty_design_time_ref() {
        let existing = r#"<Attribute uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>StorageUnit</Name><Type><v8:Type>cfg:CatalogRef.Units</v8:Type></Type><FillValue xsi:type="xr:DesignTimeRef"/><FillChecking>ShowError</FillChecking></Properties></Attribute>"#;
        for fill_value in [None, Some(MetaFillValue::Boolean(false))] {
            let expected_fill = fill_value.clone();
            let mut xml = object_xml("Catalog", "Items", "").replace(
                "<ChildObjects/>",
                &format!("<ChildObjects>{existing}</ChildObjects>"),
            );
            let operation = MetaEditOperation::add(
                MetaCollection::Attributes,
                None,
                vec![MetaElementInput {
                    name: "NewFlag".into(),
                    r#type: Some(MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap()),
                    fill_value,
                    required: Some(false),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap();
            let counts = apply_typed_operations(&mut xml, &[operation]).unwrap();
            assert_eq!(counts.added, 1);
            assert!(xml.contains(existing), "existing bytes must survive");
            let document = Document::parse(&xml).unwrap();
            let new_flag = document
                .descendants()
                .filter(|node| node.is_element() && node.tag_name().name() == "Attribute")
                .find(|node| meta_edit_child_object_name(*node).as_deref() == Some("NewFlag"))
                .unwrap();
            assert_eq!(
                unica_metadata_designer::edit::parse_typed_fill_value_node(new_flag).unwrap(),
                expected_fill
            );
            assert!(new_flag.descendants().any(|node| {
                node.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type"))
                    && node.text() == Some("xs:boolean")
            }));
        }
    }

    #[test]
    fn observed_enum_value_designator_preserves_the_full_xml_value() {
        let xml = r#"<Properties xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns="http://v8.1c.ru/8.3/MDClasses"><FillValue xsi:type="xr:DesignTimeRef">Enum.Flags.EnumValue.Yes</FillValue></Properties>"#;
        let document = Document::parse(xml).unwrap();
        let value =
            unica_metadata_designer::edit::parse_typed_fill_value_node(document.root_element())
                .unwrap()
                .unwrap();
        assert_eq!(
            serde_json::to_value(&value).unwrap(),
            serde_json::json!({
                "kind": "reference", "metadataPath": "Enum.Flags", "item": "EnumValue.Yes"
            })
        );
        let mut output = Vec::new();
        unica_metadata_designer::xml_model::emit_meta_typed_fill_value(
            &mut output,
            "",
            Some(&value),
        );
        assert_eq!(
            output,
            vec![r#"<FillValue xsi:type="xr:DesignTimeRef">Enum.Flags.EnumValue.Yes</FillValue>"#]
        );
        for designator in [
            "Enum.Flags.EnumValue",
            "Enum.Flags.EnumValue.Yes.Extra",
            "Enum.Bad-Name.EnumValue.Yes",
            "Enum.Flags.EnumValue.Bad-Name",
        ] {
            let malformed = xml.replace("Enum.Flags.EnumValue.Yes", designator);
            let document = Document::parse(&malformed).unwrap();
            assert!(
                unica_metadata_designer::edit::parse_typed_fill_value_node(document.root_element())
                    .is_err(),
                "{designator}"
            );
        }
    }

    #[test]
    fn fill_value_only_update_preserves_inherited_type_namespace_aliases() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput {
                name: "CodeText".into(),
                r#type: Some(
                    MetadataType::new(vec![MetadataTypeVariant::String {
                        length: 10,
                        allowed_length: StringLengthMode::Variable,
                    }])
                    .unwrap(),
                ),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut xml, &[add]).unwrap();
        xml = xml
            .replace(
                "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
                "xmlns:schema=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
            )
            .replace(
                "xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
                "xmlns:core=\"http://v8.1c.ru/8.1/data/core\"",
            )
            .replace("<v8:", "<core:")
            .replace("</v8:", "</core:")
            .replace(">xs:string<", ">schema:string<");
        let document = roxmltree::Document::parse(&xml).unwrap();
        let observed_type = document
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
                    && node.tag_name().name() == "Type"
            })
            .unwrap();
        assert_eq!(
            observed_type.lookup_namespace_uri(Some("schema")),
            Some("http://www.w3.org/2001/XMLSchema")
        );
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "CodeText".into(),
                fill_value: Some(MetaFillValue::String("ok".into())),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();

        apply_typed_operations(&mut xml, &[update]).unwrap();

        assert!(xml.contains("schema:string"));
        assert!(xml.contains("xsi:type=\"xs:string\">ok</FillValue>"));
    }

    #[test]
    fn typed_form_add_plans_owner_descriptor_and_both_physical_form_images() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let operation = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "ObjectForm".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut post_image, std::slice::from_ref(&operation)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[operation],
            &post_image,
        )
        .unwrap();
        let create_paths = resources
            .file_mutations
            .iter()
            .filter_map(|mutation| mutation.post_image.as_ref().map(|_| &mutation.path))
            .collect::<Vec<_>>();

        assert_eq!(create_paths.len(), 2);
        assert!(create_paths.contains(&&root.join("Documents/Order/Forms/ObjectForm.xml")));
        assert!(create_paths.contains(&&root.join("Documents/Order/Forms/ObjectForm/Ext/Form.xml")));
        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(resources.validation_resources.len(), 2);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_rename_moves_the_descriptor_and_payload_as_one_guarded_resource() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-rename-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let old_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let old_payload = root.join("Documents/Order/Forms/Old/Ext/Form.xml");
        std::fs::create_dir_all(old_payload.parent().unwrap()).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        let child = typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap();
        std::fs::write(&old_descriptor, child).unwrap();
        std::fs::write(&old_payload, b"<Form version=\"2.20\"/>").unwrap();
        let rename = MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Old".into(),
                new_name: Some("New".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, std::slice::from_ref(&rename)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[rename],
            &post_image,
        )
        .unwrap();

        assert!(resources
            .file_mutations
            .iter()
            .any(|mutation| { mutation.path == old_descriptor && mutation.post_image.is_none() }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == old_payload.parent().unwrap().parent().unwrap()
                && mutation.post_image.is_none()
        }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == root.join("Documents/Order/Forms/New.xml")
                && mutation.post_image.is_some()
        }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == root.join("Documents/Order/Forms/New/Ext/Form.xml")
                && mutation.post_image.as_deref() == Some(b"<Form version=\"2.20\"/>".as_slice())
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_two_template_preview_entries_have_distinct_exact_logical_addresses() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-template-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Reports/Sales.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let operation = MetaEditOperation::add(
            MetaCollection::Templates,
            None,
            vec![
                MetaElementInput {
                    name: "A".into(),
                    ..MetaElementInput::default()
                },
                MetaElementInput {
                    name: "B".into(),
                    ..MetaElementInput::default()
                },
            ],
        )
        .unwrap();
        let mut post_image = object_xml("Report", "Sales", "");
        apply_typed_operations(&mut post_image, std::slice::from_ref(&operation)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Report.Sales").metadata_path,
            "Report",
            "Sales",
            &[operation],
            &post_image,
        )
        .unwrap();
        let addresses = resources
            .publication_plan
            .iter()
            .map(|entry| entry.metadata_path.as_ref().unwrap().as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            addresses,
            ["Report.Sales.Template.A", "Report.Sales.Template.B"]
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_command_add_does_not_invent_a_standalone_descriptor() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-command-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let operation = MetaEditOperation::add(
            MetaCollection::Commands,
            None,
            vec![MetaElementInput {
                name: "Open".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(&mut post_image, std::slice::from_ref(&operation)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[operation],
            &post_image,
        )
        .unwrap();

        assert!(
            resources.file_mutations.is_empty(),
            "platform commands are inline owner descriptors: {:?}",
            resources.file_mutations
        );
        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0]
                .metadata_path
                .as_ref()
                .unwrap()
                .as_str(),
            "Document.Order.Command.Open"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn child_observation_localizes_an_invalid_address_and_keeps_valid_siblings() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-observe-invalid-child-address-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Catalogs/Items.xml");
        let owner = metadata_reference("Catalog.Items").metadata_path;
        let owner_xml = object_xml("Catalog", "Items", "").replace(
            "<ChildObjects/>",
            r#"<ChildObjects>
			<Form>Bad..Name</Form>
			<Command uuid="22222222-2222-4222-8222-222222222222">
				<Properties><Name>Refresh</Name></Properties>
			</Command>
		</ChildObjects>"#,
        );

        let observation =
            observe_typed_child_resources(&descriptor_path, &owner, "Catalog", "Items", &owner_xml)
                .unwrap();

        assert!(observation.diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "typed child logical address cannot be represented"
        }));
        assert!(observation.validation_resources.iter().any(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Command { owner, name }
                    if owner.as_str() == "Catalog.Items" && name == "Refresh"
            )
        }));
    }

    #[test]
    fn typed_last_form_remove_plans_one_whole_collection_tree_removal() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let payload_dir = root.join("Documents/Order/Forms/Old");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let remove =
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Old".into()]).unwrap();
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, std::slice::from_ref(&remove)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[remove],
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
        assert!(resources.file_mutations[0].post_image.is_none());
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_relation_wire_uses_md_object_refs_and_field_paths() {
        let mut xml = object_xml("Document", "Order", "<RegisterRecords/><InputByString/>");
        let operations = [
            MetaEditOperation::edit_relations(
                MetaRelation::RegisterRecords,
                RelationEditMode::Add,
                vec![metadata_reference("InformationRegister.OrderFacts")],
            )
            .unwrap(),
            MetaEditOperation::edit_relation_targets(
                MetaRelation::InputByString,
                RelationEditMode::Add,
                vec![unica_metadata::MetaRelationTarget::Field(
                    unica_metadata::MetadataFieldPath::parse(
                        "Document.Order.StandardAttribute.Number",
                    )
                    .unwrap(),
                )],
            )
            .unwrap(),
        ];

        apply_typed_operations(&mut xml, &operations).unwrap();

        assert!(xml.contains(
            "<xr:Item xsi:type=\"xr:MDObjectRef\">InformationRegister.OrderFacts</xr:Item>"
        ));
        assert!(xml.contains("<xr:Field>Document.Order.StandardAttribute.Number</xr:Field>"));
    }

    #[test]
    fn typed_form_add_then_rename_collapses_to_one_new_multi_image_resource() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-add-rename-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        let rename = MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "Old".into(),
                new_name: Some("New".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        let operations = [add, rename];
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert!(resources
            .file_mutations
            .iter()
            .all(|item| !item.path.to_string_lossy().contains("/Old")));
        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Create
        );
        assert_eq!(
            resources.publication_plan[0]
                .metadata_path
                .as_ref()
                .unwrap()
                .as_str(),
            "Document.Order.Form.New"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_child_footprint_profile_matrix_rejects_every_missing_or_extra_member() {
        let child = metadata_reference("Catalog.Editable.Template.Main").metadata_path;
        let cases = vec![
            (
                "form",
                MetaCollection::Forms,
                None,
                vec!["Ext/Form.xml"],
                vec!["", "Ext"],
                "Ext/Template.xml",
            ),
            (
                "command-module",
                MetaCollection::Commands,
                None,
                vec!["Ext/CommandModule.bsl"],
                vec!["", "Ext"],
                "Ext/Form.xml",
            ),
            (
                "spreadsheet",
                MetaCollection::Templates,
                Some(unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument),
                vec!["Ext/Template.xml"],
                vec!["", "Ext"],
                "Ext/Template.txt",
            ),
            (
                "schema",
                MetaCollection::Templates,
                Some(
                    unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema,
                ),
                vec!["Ext/Template.xml"],
                vec!["", "Ext"],
                "Ext/Template.bin",
            ),
            (
                "text",
                MetaCollection::Templates,
                Some(unica_metadata::validation_subject::MetadataTemplateType::TextDocument),
                vec!["Ext/Template.txt"],
                vec!["", "Ext"],
                "Ext/Template.xml",
            ),
            (
                "binary",
                MetaCollection::Templates,
                Some(unica_metadata::validation_subject::MetadataTemplateType::BinaryData),
                vec!["Ext/Template.bin"],
                vec!["", "Ext"],
                "Ext/Template.xml",
            ),
            (
                "html",
                MetaCollection::Templates,
                Some(unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument),
                vec!["Ext/Template.xml", "Ext/Template/ru.html"],
                vec!["", "Ext", "Ext/Template"],
                "Ext/Template/de.html",
            ),
        ];

        for (label, collection, template_type, file_names, directory_names, wrong_part) in cases {
            let mut files = file_names
                .iter()
                .map(|path| (PathBuf::from(path), Vec::new()))
                .collect::<Vec<_>>();
            if template_type
                == Some(unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument)
            {
                files
                    .iter_mut()
                    .find(|(path, _)| path == Path::new("Ext/Template.xml"))
                    .unwrap()
                    .1 = br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#
                    .to_vec();
            }
            let directories = directory_names
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            validate_typed_child_footprint(collection, template_type, &child, &files, &directories)
                .unwrap_or_else(|failure| panic!("valid {label} footprint failed: {failure:?}"));

            let mut missing_file = files.clone();
            missing_file.pop();
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &missing_file,
                    &directories,
                )
                .is_err(),
                "{label} accepted a missing file"
            );

            let mut extra_file = files.clone();
            extra_file.push((PathBuf::from("Ext/Unexpected.bin"), Vec::new()));
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &extra_file,
                    &directories,
                )
                .is_err(),
                "{label} accepted an extra file"
            );

            let mut wrong_file_role = files.clone();
            *wrong_file_role.last_mut().unwrap() = (PathBuf::from(wrong_part), Vec::new());
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &wrong_file_role,
                    &directories,
                )
                .is_err(),
                "{label} accepted a wrong file role or part"
            );

            let mut missing_directory = directories.clone();
            missing_directory.pop();
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &files,
                    &missing_directory,
                )
                .is_err(),
                "{label} accepted a missing directory"
            );

            let mut extra_directory = directories.clone();
            extra_directory.push(PathBuf::from("Ext/Unexpected"));
            assert!(
                validate_typed_child_footprint(
                    collection,
                    template_type,
                    &child,
                    &files,
                    &extra_directory,
                )
                .is_err(),
                "{label} accepted an extra directory"
            );
        }

        validate_typed_child_footprint(MetaCollection::Commands, None, &child, &[], &[])
            .expect("descriptor-only Command is a valid closed footprint");
    }

    #[test]
    fn html_template_footprint_follows_all_declared_pages() {
        let child = metadata_reference("Catalog.Editable.Template.Main").metadata_path;
        let files = vec![
            (
                PathBuf::from("Ext/Template.xml"),
                br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page><Page>en</Page></Help>"#.to_vec(),
            ),
            (
                PathBuf::from("Ext/Template/ru.html"),
                b"<html><body>ru</body></html>".to_vec(),
            ),
            (
                PathBuf::from("Ext/Template/en.html"),
                b"<html><body>en</body></html>".to_vec(),
            ),
        ];
        let directories = vec![
            PathBuf::new(),
            PathBuf::from("Ext"),
            PathBuf::from("Ext/Template"),
        ];

        validate_typed_child_footprint(
            MetaCollection::Templates,
            Some(unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument),
            &child,
            &files,
            &directories,
        )
        .unwrap();

        let mut missing_page = files;
        missing_page.pop();
        assert!(validate_typed_child_footprint(
            MetaCollection::Templates,
            Some(unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument),
            &child,
            &missing_page,
            &directories,
        )
        .is_err());
    }

    #[test]
    fn typed_form_add_then_remove_collapses_to_a_resource_noop() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-add-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
        let operations = [
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput {
                    name: "Gone".into(),
                    ..MetaElementInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Gone".into()]).unwrap(),
        ];
        let mut post_image = object_xml("Document", "Order", "<RegisterRecords/>");
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert!(resources.file_mutations.is_empty());
        assert!(resources.publication_plan.is_empty());
        assert!(resources.validation_resources.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_rename_then_remove_removes_only_the_original_resource_tree() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-rename-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let payload_dir = root.join("Documents/Order/Forms/Old");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let operations = [
            MetaEditOperation::update(
                MetaCollection::Forms,
                None,
                vec![MetaElementUpdateInput {
                    name: "Old".into(),
                    new_name: Some("New".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["New".into()]).unwrap(),
        ];
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        assert!(resources
            .file_mutations
            .iter()
            .all(|item| !item.path.to_string_lossy().contains("/New")));
        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_form_update_then_remove_plans_the_original_resource_once() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-update-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/Old.xml");
        let payload_dir = root.join("Documents/Order/Forms/Old");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput {
                name: "Old".into(),
                ..MetaElementInput::default()
            }],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "Old").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let operations = [
            MetaEditOperation::update(
                MetaCollection::Forms,
                None,
                vec![MetaElementUpdateInput {
                    name: "Old".into(),
                    comment: Some("changed".into()),
                    ..MetaElementUpdateInput::default()
                }],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["Old".into()]).unwrap(),
        ];
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_existing_form_remove_then_add_is_one_replacement_not_create_on_existing() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-remove-add-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/A.xml");
        let form_content = root.join("Documents/Order/Forms/A/Ext/Form.xml");
        std::fs::create_dir_all(form_content.parent().unwrap()).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let initial_add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput::named("A")],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[initial_add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "A").unwrap(),
        )
        .unwrap();
        std::fs::write(&form_content, b"<Form version=\"old\"/>").unwrap();
        let operations = [
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
        ];
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Update
        );
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == child_descriptor
                && mutation.pre_image.is_some()
                && mutation.post_image.is_some()
        }));
        assert!(resources.file_mutations.iter().any(|mutation| {
            mutation.path == form_content
                && mutation.pre_image.is_some()
                && mutation.post_image.is_some()
        }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typed_existing_form_remove_add_remove_deletes_only_the_initial_tree() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-remove-add-remove-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/A.xml");
        let payload_dir = root.join("Documents/Order/Forms/A");
        std::fs::create_dir_all(payload_dir.join("Ext")).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let initial_add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput::named("A")],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[initial_add]).unwrap();
        std::fs::write(
            &child_descriptor,
            typed_child_descriptor_image(&pre_image, "Form", "A").unwrap(),
        )
        .unwrap();
        std::fs::write(payload_dir.join("Ext/Form.xml"), b"<Form/>").unwrap();
        let operations = [
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
            MetaEditOperation::add(
                MetaCollection::Forms,
                None,
                vec![MetaElementInput::named("A")],
            )
            .unwrap(),
            MetaEditOperation::remove(MetaCollection::Forms, None, vec!["A".into()]).unwrap(),
        ];
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, &operations).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &operations,
            &post_image,
        )
        .unwrap();

        assert_eq!(resources.publication_plan.len(), 1);
        assert_eq!(
            resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        assert_eq!(resources.file_mutations.len(), 1);
        assert_eq!(
            resources.file_mutations[0].path,
            root.join("Documents/Order/Forms")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn typed_exact_noop_form_update_preserves_resource_bytes_and_identities() {
        use unica_workspace::test_support::file_identity_for_test;

        let root = std::env::temp_dir().join(format!(
            "unica-meta-edit-form-noop-{}",
            uuid::Uuid::new_v4()
        ));
        let descriptor_path = root.join("Documents/Order.xml");
        let child_descriptor = root.join("Documents/Order/Forms/A.xml");
        std::fs::create_dir_all(child_descriptor.parent().unwrap()).unwrap();
        let mut pre_image = object_xml("Document", "Order", "<RegisterRecords/>");
        let initial_add = MetaEditOperation::add(
            MetaCollection::Forms,
            None,
            vec![MetaElementInput::named("A")],
        )
        .unwrap();
        apply_typed_operations(&mut pre_image, &[initial_add]).unwrap();
        let child_bytes = typed_child_descriptor_image(&pre_image, "Form", "A").unwrap();
        std::fs::write(&child_descriptor, &child_bytes).unwrap();
        let form_content = root.join("Documents/Order/Forms/A/Ext/Form.xml");
        std::fs::create_dir_all(form_content.parent().unwrap()).unwrap();
        std::fs::write(
            &form_content,
            minimal_typed_form_content("Document", "Order").as_bytes(),
        )
        .unwrap();
        let child_before = std::fs::read(&child_descriptor).unwrap();
        let content_before = std::fs::read(&form_content).unwrap();
        let child_identity = file_identity_for_test(&child_descriptor).unwrap();
        let content_identity = file_identity_for_test(&form_content).unwrap();
        let operation = MetaEditOperation::update(
            MetaCollection::Forms,
            None,
            vec![MetaElementUpdateInput {
                name: "A".into(),
                comment: Some(String::new()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        let mut post_image = pre_image;
        apply_typed_operations(&mut post_image, std::slice::from_ref(&operation)).unwrap();

        let resources = plan_typed_child_resources(
            &descriptor_path,
            &metadata_reference("Document.Order").metadata_path,
            "Document",
            "Order",
            &[operation],
            &post_image,
        )
        .unwrap();

        assert!(resources.file_mutations.is_empty());
        assert!(resources.publication_plan.is_empty());
        assert_eq!(std::fs::read(&child_descriptor).unwrap(), child_before);
        assert_eq!(std::fs::read(&form_content).unwrap(), content_before);
        assert_eq!(
            file_identity_for_test(&child_descriptor).unwrap(),
            child_identity
        );
        assert_eq!(
            file_identity_for_test(&form_content).unwrap(),
            content_identity
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Every payload member the platform actually writes beside a managed form,
    /// measured over a real 8.3.27 Designer dump. `Ext/Form/Module.bsl` is the
    /// module (not `Ext/Module.bsl`); `Ext/Help*` is form help; the pictures
    /// under `Ext/Form/Items` belong to individual form items.
    const REAL_FORM_PAYLOADS: &[(&str, &[&str])] = &[
        ("bare", &[]),
        ("module", &["Ext/Form/Module.bsl"]),
        (
            "module-and-help",
            &["Ext/Form/Module.bsl", "Ext/Help.xml", "Ext/Help/ru.html"],
        ),
        (
            "help-assets",
            &[
                "Ext/Help.xml",
                "Ext/Help/ru.html",
                "Ext/Help/_files/note.png",
            ],
        ),
        (
            "item-pictures",
            &[
                "Ext/Form/Module.bsl",
                "Ext/Form/Items/Список/Picture.png",
                "Ext/Form/Items/Список/RowsPicture.png",
            ],
        ),
    ];

    fn write_designer_form_object(root: &Path, extra_payload: &[&str]) -> String {
        let owner_image = object_xml("Catalog", "Users", "").replace(
            "<ChildObjects/>",
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
        );
        let forms = root.join("Catalogs/Users/Forms");
        std::fs::create_dir_all(forms.join("ItemForm/Ext")).unwrap();
        std::fs::write(root.join("Catalogs/Users.xml"), &owner_image).unwrap();
        std::fs::write(
            forms.join("ItemForm.xml"),
            object_xml("Form", "ItemForm", "<FormType>Managed</FormType>")
                .replace("\n\t\t<ChildObjects/>", ""),
        )
        .unwrap();
        std::fs::write(
            forms.join("ItemForm/Ext/Form.xml"),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
                "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n",
                "</Form>"
            ),
        )
        .unwrap();
        for relative in extra_payload {
            let path = forms.join("ItemForm").join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"payload").unwrap();
        }
        owner_image
    }

    fn designer_form_child_diagnostics(root: &Path, owner_image: &str) -> Vec<String> {
        let owner = metadata_reference("Catalog.Users").metadata_path;
        let plan = match plan_typed_child_resources(
            &root.join("Catalogs/Users.xml"),
            &owner,
            "Catalog",
            "Users",
            &[],
            owner_image,
        ) {
            Ok(plan) => plan,
            Err(failure) => {
                return failure
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.clone())
                    .collect()
            }
        };
        let mut resources = vec![
            unica_metadata::validation_subject::MetadataResourceImage {
                role: unica_metadata::validation_subject::MetadataResourceRole::Descriptor,
                bytes: owner_image.as_bytes().to_vec(),
            },
            unica_metadata::validation_subject::MetadataResourceImage {
                role: unica_metadata::validation_subject::MetadataResourceRole::Registration,
                bytes: br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
<Configuration uuid="22222222-2222-4222-8222-222222222222">
<Properties><Name>Owner</Name></Properties><ChildObjects><Catalog>Users</Catalog></ChildObjects>
</Configuration></MetaDataObject>"#
                    .to_vec(),
            },
        ];
        resources.extend(plan.validation_resources);
        let subject = unica_metadata::validation_subject::MetadataValidationSubject {
            target: owner,
            resources,
            child_footprints: plan.validation_footprints,
            registrar_evidence: Default::default(),
            subsystem_evidence: Default::default(),
        };
        let context = WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".unica/cache"),
            workspace_epoch: 1,
        };

        // Keep only what this regression is about: the object itself is a
        // hand-written stub, so its own completeness diagnostics are noise.
        unica_metadata_designer::validation::MetadataValidator::new(
            unica_metadata_designer::validation::FormRootValidator(
                unica_forms_designer::root::require_form_root,
            ),
        )
        .validate(&subject, &context)
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .metadata_path
                .as_ref()
                .is_some_and(|path| path.as_str().contains(".Form."))
        })
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
    }

    /// Regression for #360. A Designer dump registers a form in the owner's
    /// `ChildObjects` as a bare `<Form>Name</Form>` element and keeps the real
    /// descriptor in `Forms/Name.xml`. Reading such an object must succeed for
    /// every payload shape the platform actually writes.
    #[test]
    fn real_designer_form_payloads_validate_without_child_diagnostics() {
        for (label, extra_payload) in REAL_FORM_PAYLOADS {
            let root = std::env::temp_dir().join(format!(
                "unica-meta-issue-360-{label}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let owner_image = write_designer_form_object(&root, extra_payload);

            let diagnostics = designer_form_child_diagnostics(&root, &owner_image);

            let _ = std::fs::remove_dir_all(&root);
            assert_eq!(diagnostics, Vec::<String>::new(), "{label}");
        }
    }

    /// The closed payload guard still has to reject bytes the platform never
    /// writes, otherwise a mutation would silently clobber them.
    #[test]
    fn unmodelled_form_payload_members_are_still_rejected() {
        for (label, extra_payload) in [
            ("stray-root-file", "stray.txt"),
            ("legacy-module-path", "Ext/Module.bsl"),
            ("stray-ext-file", "Ext/Unexpected.xml"),
        ] {
            let root = std::env::temp_dir().join(format!(
                "unica-meta-issue-360-reject-{label}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let owner_image = write_designer_form_object(&root, &[extra_payload]);

            let diagnostics = designer_form_child_diagnostics(&root, &owner_image);

            let _ = std::fs::remove_dir_all(&root);
            assert!(!diagnostics.is_empty(), "{label} was accepted");
        }
    }
}
