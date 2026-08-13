use crate::application::source_navigation::LocateRejection;
use crate::domain::cancellation::CancellationToken;
use crate::domain::diagnostics::{
    DiagnosticContext, DiagnosticFocus, DiagnosticItem, DiagnosticLocation, DiagnosticMapError,
    DiagnosticObservation, DiagnosticObservationFocus, DiagnosticObservationLocation,
    DiagnosticRequest, DiagnosticRequestError, MetadataFocus, UnaddressableReason,
};
use crate::domain::metadata::{
    diagnostic_metadata_focus_route, diagnostic_metadata_property_is_canonical,
};
use crate::domain::project_sources::SourceFormat;
use crate::domain::source_roots::ResolvedSourceRoot;
use crate::domain::source_target::{SourceTarget, SourceTargetErrorCode, TargetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_source_targets::{
    locate_platform_xml_source_path_in, platform_xml_resource_evidence, portable_relative,
    resolve_platform_xml_target_in, resolve_platform_xml_target_in_diagnostic_context,
    source_set_relative_path, TargetKindPolicy,
};
use crate::infrastructure::source_roots::{resolve_named_source_set, NamedSourceSetErrorKind};
use roxmltree::Node;
use std::path::PathBuf;
use url::Url;

const MAX_METADATA_FOCUS_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn resolve_diagnostic_context(
    request: &DiagnosticRequest,
    workspace: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticContext, DiagnosticRequestError> {
    if cancellation.is_cancelled() {
        return Err(request_error(
            "cancelled",
            None,
            "diagnostics context resolution was cancelled",
        ));
    }
    let selected = resolve_named_source_set(workspace, &request.source_set).map_err(|error| {
        let (code, message) = match error.kind {
            NamedSourceSetErrorKind::NotFound => (
                "source_set_not_found",
                format!("sourceSet `{}` was not found", request.source_set),
            ),
            NamedSourceSetErrorKind::Ambiguous => (
                "source_set_ambiguous",
                format!("sourceSet `{}` is ambiguous", request.source_set),
            ),
            NamedSourceSetErrorKind::Containment => (
                "source_set_containment_denied",
                format!(
                    "sourceSet `{}` violates the workspace containment boundary",
                    request.source_set
                ),
            ),
            NamedSourceSetErrorKind::Discovery => (
                "source_set_discovery_failed",
                format!("sourceSet `{}` could not be discovered", request.source_set),
            ),
        };
        request_error(code, Some("sourceSet"), message)
    })?;
    let source_target = SourceTarget {
        source_set: selected.source_set.name.clone(),
        metadata_path: request.metadata_path.clone(),
    };
    let resolution = resolve_platform_xml_target_in(
        workspace,
        &source_target,
        TargetKindPolicy::Any,
        selected.clone(),
    )
    .map_err(|error| source_target_request_error(error.code))?;
    Ok(DiagnosticContext::new(
        workspace.clone(),
        selected.source_set.clone(),
        ResolvedSourceRoot {
            source_set: Some(selected.source_set.name),
            path: selected.path,
        },
        resolution.resolved,
    ))
}

pub(crate) fn map_diagnostic_observation(
    observation: DiagnosticObservation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticItem, DiagnosticMapError> {
    if cancellation.is_cancelled() {
        return Err(map_error(
            "cancelled",
            "diagnostic observation mapping was cancelled",
        ));
    }
    match observation {
        DiagnosticObservation::Diagnostic {
            provider,
            location,
            focus,
            code,
            severity,
            message,
            tags,
        } => {
            let location = map_location(location, context, cancellation)?;
            let focus = map_focus(focus, &location, context, cancellation);
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
        DiagnosticObservation::ResourceFailure {
            provider,
            location,
            error,
        } => Ok(DiagnosticItem::ResourceFailure {
            provider: provider.as_str(),
            location: map_location(location, context, cancellation)?,
            error,
        }),
    }
}

fn map_location(
    location: DiagnosticObservationLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticLocation, DiagnosticMapError> {
    match location {
        DiagnosticObservationLocation::Logical { metadata_path } => {
            let target = SourceTarget {
                source_set: context.target.source_set.clone(),
                metadata_path,
            };
            let resolution = resolve_platform_xml_target_in_diagnostic_context(
                &context.workspace,
                &target,
                TargetKindPolicy::Any,
                &context.source_set,
                &context.source_root.path,
            )
            .map_err(|error| map_source_target_error(error.code))?;
            Ok(DiagnosticLocation::Addressed {
                source_set: resolution.resolved.source_set,
                metadata_path: resolution.resolved.metadata_path,
                target_kind: resolution.resolved.target_kind,
            })
        }
        DiagnosticObservationLocation::Resource { handle } => {
            map_resource_location(&handle, context, cancellation)
        }
    }
}

fn map_resource_location(
    handle: &str,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticLocation, DiagnosticMapError> {
    let path = provider_resource_path(handle)?;
    let relative = source_set_relative_path(
        &context.workspace,
        &context.source_root.path,
        path.as_path(),
    )
    .ok_or_else(|| {
        map_error(
            "location_outside_source_set",
            "provider resource is outside the selected sourceSet",
        )
    })?;
    let observed_path = portable_relative(&relative);
    if context.source_set.source_format != SourceFormat::PlatformXml {
        return Ok(DiagnosticLocation::Unaddressable {
            source_set: context.target.source_set.clone(),
            owner_metadata_path: None,
            observed_path,
            reason: UnaddressableReason::SourceFormatUnsupported,
        });
    }
    let located = locate_platform_xml_source_path_in(
        &context.workspace,
        &context.source_set,
        &context.source_root.path,
        &path.to_string_lossy(),
        cancellation,
    )
    .map_err(|_| {
        map_error(
            "location_mapping_failed",
            "provider resource could not be mapped safely",
        )
    })?;
    match located.rejection {
        None => Ok(DiagnosticLocation::Addressed {
            source_set: located.source_set,
            metadata_path: located.metadata_path,
            target_kind: located.target_kind.ok_or_else(|| {
                map_error(
                    "provider_contract_invalid",
                    "addressed provider resource has no target kind",
                )
            })?,
        }),
        Some(LocateRejection::NotAddressable) => Ok(DiagnosticLocation::Unaddressable {
            source_set: located.source_set,
            owner_metadata_path: located.owner_metadata_path,
            observed_path: located.relative_path,
            reason: UnaddressableReason::ResourceNotAddressable,
        }),
        Some(LocateRejection::OwnerUnproven) => Ok(DiagnosticLocation::Unaddressable {
            source_set: located.source_set,
            owner_metadata_path: located.owner_metadata_path,
            observed_path: located.relative_path,
            reason: UnaddressableReason::OwnerUnproven,
        }),
        Some(LocateRejection::OutsideSourceSet) => Err(map_error(
            "location_outside_source_set",
            "provider resource is outside the selected sourceSet",
        )),
    }
}

fn provider_resource_path(handle: &str) -> Result<PathBuf, DiagnosticMapError> {
    let handle = handle.trim();
    if handle.is_empty() {
        return Err(map_error(
            "provider_contract_invalid",
            "provider resource handle is empty",
        ));
    }
    if looks_like_windows_drive_path(handle) {
        return Ok(PathBuf::from(handle.replace('\\', "/")));
    }
    if handle.starts_with("file:") || handle.contains("://") {
        let url = Url::parse(handle).map_err(|_| {
            map_error(
                "provider_contract_invalid",
                "provider resource URI is invalid",
            )
        })?;
        if url.scheme() != "file" {
            return Err(map_error(
                "resource_scheme_unsupported",
                "provider resource URI scheme is unsupported",
            ));
        }
        return url.to_file_path().map_err(|_| {
            map_error(
                "provider_contract_invalid",
                "provider file URI does not identify a local path",
            )
        });
    }
    Ok(PathBuf::from(handle.replace('\\', "/")))
}

fn looks_like_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn map_focus(
    focus: DiagnosticObservationFocus,
    location: &DiagnosticLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> DiagnosticFocus {
    match focus {
        DiagnosticObservationFocus::Target => DiagnosticFocus::Target,
        DiagnosticObservationFocus::SourceRange(range) if range.is_non_empty() => {
            DiagnosticFocus::SourceRange { range }
        }
        DiagnosticObservationFocus::SourceRange(_) => DiagnosticFocus::Target,
        DiagnosticObservationFocus::Metadata(focus)
            if metadata_focus_is_proven(&focus, location, context, cancellation) =>
        {
            focus.into()
        }
        DiagnosticObservationFocus::Metadata(_) => DiagnosticFocus::Target,
    }
}

fn metadata_focus_is_proven(
    focus: &MetadataFocus,
    location: &DiagnosticLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> bool {
    if cancellation.is_cancelled() {
        return false;
    }
    let DiagnosticLocation::Addressed {
        metadata_path: Some(metadata_path),
        target_kind: TargetKind::MetadataObject,
        ..
    } = location
    else {
        return false;
    };
    if focus
        .language
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return false;
    }
    if focus
        .property
        .as_deref()
        .is_some_and(|property| !diagnostic_metadata_property_is_canonical(property))
    {
        return false;
    }
    let Some(route) = diagnostic_metadata_focus_route(&focus.element_path) else {
        return false;
    };
    let target = SourceTarget {
        source_set: context.target.source_set.clone(),
        metadata_path: Some(metadata_path.clone()),
    };
    let Ok(resolution) = resolve_platform_xml_target_in_diagnostic_context(
        &context.workspace,
        &target,
        TargetKindPolicy::Any,
        &context.source_set,
        &context.source_root.path,
    ) else {
        return false;
    };
    let Ok(evidence) = platform_xml_resource_evidence(&context.workspace, &resolution.handle)
    else {
        return false;
    };
    let Ok(metadata) = std::fs::metadata(&evidence.target_path) else {
        return false;
    };
    if metadata.len() > MAX_METADATA_FOCUS_DESCRIPTOR_BYTES {
        return false;
    }
    let Ok(bytes) = std::fs::read(&evidence.target_path) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else {
        return false;
    };
    let Some(mut current) = document
        .root_element()
        .children()
        .find(|node| node.is_element())
    else {
        return false;
    };
    for (element, collection) in focus.element_path.iter().zip(route) {
        let Some(child_objects) = direct_child(current, "ChildObjects") else {
            return false;
        };
        let Some(found) = child_objects.children().find(|candidate| {
            candidate.is_element()
                && candidate.tag_name().name() == collection.xml_element_name()
                && direct_child(*candidate, "Properties")
                    .and_then(|properties| direct_child(properties, "Name"))
                    .and_then(|name| name.text())
                    == Some(element.name.as_str())
        }) else {
            return false;
        };
        current = found;
    }
    let Some(property) = focus.property.as_deref() else {
        return focus.language.is_none();
    };
    let Some(properties) = direct_child(current, "Properties") else {
        return false;
    };
    let Some(property_node) = direct_child(properties, property) else {
        return false;
    };
    focus.language.as_deref().is_none_or(|language| {
        property_node.descendants().any(|node| {
            node.is_element() && node.tag_name().name() == "lang" && node.text() == Some(language)
        })
    })
}

fn direct_child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
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

fn source_target_request_error(code: SourceTargetErrorCode) -> DiagnosticRequestError {
    match code {
        SourceTargetErrorCode::SourceSetRequired => request_error(
            "source_set_required",
            Some("sourceSet"),
            "sourceSet must name an exact project source set",
        ),
        SourceTargetErrorCode::SourceSetNotFound => request_error(
            "source_set_not_found",
            Some("sourceSet"),
            "sourceSet was not found",
        ),
        SourceTargetErrorCode::MetadataAddressInvalid => request_error(
            "metadata_address_invalid",
            Some("metadataPath"),
            "metadataPath is not a valid logical address",
        ),
        SourceTargetErrorCode::MetadataAddressNotFound => request_error(
            "metadata_address_not_found",
            Some("metadataPath"),
            "metadataPath was not found in the selected sourceSet",
        ),
        SourceTargetErrorCode::TargetKindMismatch => request_error(
            "target_kind_mismatch",
            Some("metadataPath"),
            "metadataPath does not identify a supported diagnostic target",
        ),
        SourceTargetErrorCode::SourceRootNotAddressable
        | SourceTargetErrorCode::AddressProfileUnsupported => request_error(
            "source_format_unsupported",
            Some("sourceSet"),
            "sourceSet is outside the supported logical address profile",
        ),
        SourceTargetErrorCode::ContainmentDenied => request_error(
            "source_set_containment_denied",
            Some("sourceSet"),
            "sourceSet violates the workspace containment boundary",
        ),
    }
}

fn map_source_target_error(code: SourceTargetErrorCode) -> DiagnosticMapError {
    let request = source_target_request_error(code);
    map_error(request.code, request.message)
}

fn map_error(code: &'static str, message: impl Into<String>) -> DiagnosticMapError {
    DiagnosticMapError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{map_diagnostic_observation, resolve_diagnostic_context};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::diagnostics::{
        DiagnosticAction, DiagnosticCodeFilter, DiagnosticFilter, DiagnosticFocus, DiagnosticItem,
        DiagnosticObservation, DiagnosticObservationFocus, DiagnosticObservationLocation,
        DiagnosticProviderId, DiagnosticRange, DiagnosticRequest, DiagnosticSeverity,
        DiagnosticTag, MetadataElement, MetadataFocus, UnaddressableReason,
    };
    use crate::domain::project_sources::SourceFormat;
    use crate::domain::source_target::{
        MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::TempDir;

    const TEST_PROVIDER: DiagnosticProviderId = DiagnosticProviderId::new_const("test-provider");

    struct Fixture {
        _temp: TempDir,
        context: WorkspaceContext,
    }

    impl Fixture {
        fn platform_xml() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            let source = root.join("src");
            fs::create_dir_all(&source).unwrap();
            fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Diagnostics</Name></Properties><ChildObjects><Catalog>Items</Catalog><CommonModule>Shared</CommonModule><CommonModule>Модуль с пробелом</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            Self {
                context: WorkspaceContext {
                    cwd: root.to_path_buf(),
                    workspace_root: root.to_path_buf(),
                    cache_root: root.join(".build/unica"),
                    workspace_epoch: 1,
                },
                _temp: temp,
            }
        }

        fn source(&self) -> std::path::PathBuf {
            self.context.workspace_root.join("src")
        }

        fn write_catalog(&self, name: &str) {
            let directory = self.source().join("Catalogs");
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join(format!("{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>{name}</Name><Synonym><v8:item xmlns:v8="http://v8.1c.ru/8.1/data/core"><v8:lang>ru</v8:lang><v8:content>Товары</v8:content></v8:item></Synonym><InternalField>secret</InternalField></Properties><ChildObjects><Attribute><Properties><Name>Code</Name><Type>string</Type></Properties></Attribute><TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute><Properties><Name>Price</Name><Type>number</Type></Properties></Attribute></ChildObjects></TabularSection><Form>Card</Form></ChildObjects></Catalog></MetaDataObject>"#
                ),
            )
            .unwrap();
        }

        fn write_common_module(&self, name: &str) -> std::path::PathBuf {
            let directory = self.source().join("CommonModules");
            fs::create_dir_all(directory.join(name).join("Ext")).unwrap();
            fs::write(
                directory.join(format!("{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>{name}</Name></Properties></CommonModule></MetaDataObject>"#
                ),
            )
            .unwrap();
            let module = directory.join(name).join("Ext/Module.bsl");
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
            module
        }

        fn write_form_module(&self, catalog: &str, form: &str) -> std::path::PathBuf {
            let directory = self.source().join("Catalogs").join(catalog);
            fs::create_dir_all(directory.join("Forms").join(form).join("Ext/Form")).unwrap();
            fs::write(
                directory.join("Forms").join(format!("{form}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>{form}</Name></Properties></Form></MetaDataObject>"#
                ),
            )
            .unwrap();
            let module = directory
                .join("Forms")
                .join(form)
                .join("Ext/Form/Module.bsl");
            fs::write(&module, "Procedure Open()\nEndProcedure\n").unwrap();
            module
        }
    }

    fn address(raw: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap()
    }

    fn request(metadata_path: Option<&str>) -> DiagnosticRequest {
        DiagnosticRequest {
            action: DiagnosticAction::Findings,
            source_set: "main".to_string(),
            metadata_path: metadata_path.map(address),
            requested_providers: None,
            filter: DiagnosticFilter {
                min_severity: Some(DiagnosticSeverity::Warning),
                codes: Vec::<DiagnosticCodeFilter>::new(),
            },
            range: None,
            limit: 200,
            timeout: Some(Duration::from_secs(30)),
        }
    }

    fn diagnostic(
        location: DiagnosticObservationLocation,
        focus: DiagnosticObservationFocus,
    ) -> DiagnosticObservation {
        DiagnosticObservation::Diagnostic {
            provider: TEST_PROVIDER,
            location,
            focus,
            code: "TEST001".to_string(),
            severity: DiagnosticSeverity::Warning,
            message: "finding".to_string(),
            tags: vec![DiagnosticTag::Unnecessary],
        }
    }

    fn mapped_location(item: DiagnosticItem) -> crate::domain::diagnostics::DiagnosticLocation {
        match item {
            DiagnosticItem::Diagnostic { location, .. } => location,
            item => panic!("expected diagnostic item, got {item:?}"),
        }
    }

    #[test]
    fn diagnostic_location_maps_root_object_module_and_nested_form_module() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        let module = fixture.write_common_module("Shared");
        let form_module = fixture.write_form_module("Items", "Card");
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();

        let root = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: None,
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            mapped_location(root),
            crate::domain::diagnostics::DiagnosticLocation::Addressed {
                target_kind: TargetKind::SourceRoot,
                ..
            }
        ));

        let object = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            mapped_location(object),
            crate::domain::diagnostics::DiagnosticLocation::Addressed {
                target_kind: TargetKind::MetadataObject,
                ..
            }
        ));

        for (path, expected) in [
            (module, "CommonModule.Shared.Module"),
            (form_module, "Catalog.Items.Form.Card.FormModule"),
        ] {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource {
                        handle: path.to_string_lossy().into_owned(),
                    },
                    DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                        start_line: 0,
                        start_column: 0,
                        end_line: 0,
                        end_column: 1,
                    }),
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            match mapped_location(item) {
                crate::domain::diagnostics::DiagnosticLocation::Addressed {
                    metadata_path: Some(actual),
                    target_kind: TargetKind::Module,
                    ..
                } => assert_eq!(actual.as_str(), expected),
                location => panic!("unexpected module location: {location:?}"),
            }
        }
    }

    #[test]
    fn diagnostic_location_distinguishes_unaddressable_owner_and_unproven_owner() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        fs::create_dir_all(fixture.source().join("Catalogs/Items/Ext")).unwrap();
        fs::write(
            fixture.source().join("Catalogs/Items/Ext/Unknown.xml"),
            "<unknown/>",
        )
        .unwrap();
        fs::create_dir_all(fixture.source().join("CommonModules/Ghost/Ext")).unwrap();
        fs::write(
            fixture.source().join("CommonModules/Ghost/Ext/Module.bsl"),
            "Procedure Run()\nEndProcedure",
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();

        let cases = [
            (
                "Catalogs/Items/Ext/Unknown.xml",
                UnaddressableReason::ResourceNotAddressable,
                Some("Catalog.Items"),
            ),
            (
                "CommonModules/Ghost/Ext/Module.bsl",
                UnaddressableReason::OwnerUnproven,
                None,
            ),
        ];
        for (handle, expected_reason, expected_owner) in cases {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource {
                        handle: handle.to_string(),
                    },
                    DiagnosticObservationFocus::Target,
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            match mapped_location(item) {
                crate::domain::diagnostics::DiagnosticLocation::Unaddressable {
                    owner_metadata_path,
                    observed_path,
                    reason,
                    ..
                } => {
                    assert_eq!(reason, expected_reason);
                    assert_eq!(
                        owner_metadata_path.as_ref().map(MetadataAddress::as_str),
                        expected_owner
                    );
                    assert!(!observed_path.contains('\\'));
                    assert!(!Path::new(&observed_path).is_absolute());
                }
                location => panic!("unexpected unaddressable location: {location:?}"),
            }
        }
    }

    #[test]
    fn diagnostic_location_preserves_exact_metadata_focus_and_weakens_unknown_elements() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        let cancellation = CancellationToken::new();
        let context = resolve_diagnostic_context(
            &request(Some("Catalog.Items")),
            &fixture.context,
            &cancellation,
        )
        .unwrap();

        let focuses = [
            MetadataFocus {
                element_path: Vec::new(),
                property: Some("Synonym".to_string()),
                language: Some("ru".to_string()),
            },
            MetadataFocus {
                element_path: vec![MetadataElement {
                    collection: "attributes".to_string(),
                    name: "Code".to_string(),
                }],
                property: Some("Type".to_string()),
                language: None,
            },
            MetadataFocus {
                element_path: vec![
                    MetadataElement {
                        collection: "tabularSections".to_string(),
                        name: "Lines".to_string(),
                    },
                    MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Price".to_string(),
                    },
                ],
                property: Some("Type".to_string()),
                language: None,
            },
        ];
        for expected in focuses {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Logical {
                        metadata_path: Some(address("Catalog.Items")),
                    },
                    DiagnosticObservationFocus::Metadata(expected.clone()),
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            match item {
                DiagnosticItem::Diagnostic {
                    focus:
                        DiagnosticFocus::Metadata {
                            element_path,
                            property,
                            language,
                        },
                    ..
                } => {
                    assert_eq!(element_path, expected.element_path);
                    assert_eq!(property, expected.property);
                    assert_eq!(language, expected.language);
                }
                item => panic!("expected exact metadata focus, got {item:?}"),
            }
        }

        let unknown = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: vec![MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Missing".to_string(),
                    }],
                    property: Some("Type".to_string()),
                    language: None,
                }),
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            unknown,
            DiagnosticItem::Diagnostic {
                focus: DiagnosticFocus::Target,
                ..
            }
        ));

        let private_xml_field = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: Vec::new(),
                    property: Some("InternalField".to_string()),
                    language: None,
                }),
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            private_xml_field,
            DiagnosticItem::Diagnostic {
                focus: DiagnosticFocus::Target,
                ..
            }
        ));
    }

    #[test]
    fn diagnostics_windows_normalizes_separators_unicode_file_uri_and_dot_segments() {
        let fixture = Fixture::platform_xml();
        let module = fixture.write_common_module("Модуль с пробелом");
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        let absolute_uri = url::Url::from_file_path(&module).unwrap().to_string();
        let handles = [
            "CommonModules\\Модуль с пробелом\\Ext\\Module.bsl".to_string(),
            "./CommonModules/Модуль с пробелом/Ext/./Module.bsl".to_string(),
            absolute_uri,
        ];

        for handle in handles {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource { handle },
                    DiagnosticObservationFocus::Target,
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            match mapped_location(item) {
                crate::domain::diagnostics::DiagnosticLocation::Addressed {
                    metadata_path: Some(address),
                    ..
                } => assert_eq!(address.as_str(), "CommonModule.Модуль с пробелом.Module"),
                location => panic!("unexpected normalized location: {location:?}"),
            }
        }
    }

    #[test]
    fn diagnostic_location_rejects_escape_without_leaking_the_raw_handle() {
        let fixture = Fixture::platform_xml();
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        let raw = "../outside/secret.bsl";
        let error = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Resource {
                    handle: raw.to_string(),
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap_err();

        assert_eq!(error.code, "location_outside_source_set");
        assert!(!error.message.contains(raw));
        assert!(!error.message.contains("secret.bsl"));
    }

    #[test]
    fn diagnostic_location_reports_unsupported_source_format_without_a_physical_path() {
        let fixture = Fixture::platform_xml();
        let cancellation = CancellationToken::new();
        let mut context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        context.source_set.source_format = SourceFormat::Edt;
        let item = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Resource {
                    handle: "src/CommonModules/Any/Ext/Module.bsl".to_string(),
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap();

        assert!(matches!(
            mapped_location(item),
            crate::domain::diagnostics::DiagnosticLocation::Unaddressable {
                reason: UnaddressableReason::SourceFormatUnsupported,
                observed_path,
                ..
            } if observed_path == "CommonModules/Any/Ext/Module.bsl"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn diagnostics_windows_accepts_drive_letter_case_and_rejects_another_drive() {
        let fixture = Fixture::platform_xml();
        let module = fixture.write_common_module("Shared");
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        let mut swapped = module.to_string_lossy().into_owned();
        swapped.replace_range(0..1, &swapped[0..1].to_ascii_lowercase());
        assert!(map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Resource { handle: swapped },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .is_ok());

        let current_drive = module.to_string_lossy().chars().next().unwrap();
        let other_drive = if current_drive.eq_ignore_ascii_case(&'Z') {
            'Y'
        } else {
            'Z'
        };
        let other = format!("{other_drive}:\\outside\\secret.bsl");
        let error = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Resource { handle: other },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap_err();
        assert_eq!(error.code, "location_outside_source_set");
        assert!(!error.message.contains("secret.bsl"));
    }
}
