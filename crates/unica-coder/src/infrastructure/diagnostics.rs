use unica_diagnostics::model::DiagnosticContext;
use unica_diagnostics::model::DiagnosticFocus;
use unica_diagnostics::model::DiagnosticItem;
use unica_diagnostics::model::DiagnosticMapError;
use unica_diagnostics::model::DiagnosticObservation;
use unica_diagnostics::model::DiagnosticObservationFocus;
use unica_diagnostics::model::DiagnosticObservationLocation;
use unica_diagnostics::model::DiagnosticProviderRequest;
use unica_diagnostics::model::DiagnosticRequest;
use unica_diagnostics::model::DiagnosticRequestError;
use unica_diagnostics::model::MetadataFocus;
use unica_diagnostics::model::UnaddressableReason;
use unica_metadata::{diagnostic_metadata_focus_route, diagnostic_metadata_property_is_canonical};
use unica_navigation::source_navigation::LocateRejection;
use unica_source_contract::location::SourceLocation;
use unica_source_contract::workspace::ResolvedSourceRoot;
use unica_source_contract::workspace::WorkspaceContext;

use crate::infrastructure::platform_xml_source_targets::{
    locate_platform_xml_source_path_in, platform_xml_resource_evidence, portable_relative,
    resolve_platform_xml_target_in, resolve_platform_xml_target_in_diagnostic_context,
    source_set_relative_path, TargetKindPolicy,
};

use crate::infrastructure::source_roots::resolve_named_source_set;
use roxmltree::Node;
use std::collections::HashMap;
use std::path::PathBuf;
use unica_address::source_target::{SourceTarget, SourceTargetErrorCode, TargetKind};
use unica_source_contract::discovery::NamedSourceSetErrorKind;
use unica_source_contract::SourceFormat;
use unica_work_control::cancellation::CancellationToken;
use url::Url;

/// Mapping for one target already proved by the actor-owned logical reader:
/// a module, a metadata object or the source root. Provider analysis can cover
/// the source set; the proved target keeps its identity. For an object or the
/// root every other resource is located like any diagnostic: a logical target,
/// or an unaddressable path with its proven owner when there is one. A module
/// keeps only its own file; other resources stay explicitly unproved.
pub(crate) struct CanonicalDiagnosticMapping {
    context: DiagnosticContext,
    /// The file a module target was proved by; `None` for an object or root.
    module_relative: Option<PathBuf>,
}

impl CanonicalDiagnosticMapping {
    pub(crate) fn from_proven_module(
        context: DiagnosticContext,
        module_relative: PathBuf,
    ) -> Result<Self, String> {
        let module_relative = source_set_relative_path(
            &context.workspace,
            &context.source_root.path,
            &context.source_root.path.join(module_relative),
        )
        .ok_or_else(|| {
            "proved diagnostic module no longer belongs to its source root".to_owned()
        })?;
        Ok(Self {
            context,
            module_relative: Some(module_relative),
        })
    }

    /// A metadata object or the source root proved by the logical reader.
    pub(crate) fn from_proven_node(context: DiagnosticContext) -> Result<Self, String> {
        if context.target.target_kind == TargetKind::Module {
            return Err("a module target is proved by its module file".to_owned());
        }
        Ok(Self {
            context,
            module_relative: None,
        })
    }

    pub(crate) fn target(&self) -> &unica_address::source_target::ResolvedTarget {
        &self.context.target
    }

    /// The analysis scope of a module target; an object or root has none.
    pub(crate) fn module_scope(&self) -> Option<unica_diagnostics::model::DiagnosticModuleScope> {
        self.module_relative.as_ref().map(|relative| {
            unica_diagnostics::model::DiagnosticModuleScope {
                source_set: self.context.target.source_set.clone(),
                source_root: self.context.source_root.path.clone(),
                module_path: self.context.source_root.path.join(relative),
            }
        })
    }

    fn map_location(
        &self,
        location: DiagnosticObservationLocation,
        cancellation: &CancellationToken,
    ) -> Result<MappedDiagnosticLocation, DiagnosticMapError> {
        let addressed = || MappedDiagnosticLocation {
            location: SourceLocation::Addressed {
                source_set: self.context.target.source_set.clone(),
                metadata_path: self.context.target.metadata_path.clone(),
                target_kind: self.context.target.target_kind,
            },
            reason: None,
        };
        match location {
            DiagnosticObservationLocation::Logical { metadata_path }
                if metadata_path == self.context.target.metadata_path =>
            {
                Ok(addressed())
            }
            DiagnosticObservationLocation::Logical { .. } if self.module_relative.is_some() => {
                Err(map_error(
                    "location_mapping_failed",
                    "provider logical target was not proved by this module read",
                ))
            }
            logical @ DiagnosticObservationLocation::Logical { .. } => {
                map_location(logical, &self.context, cancellation)
            }
            DiagnosticObservationLocation::Resource { handle } => {
                if let Some(module_relative) = &self.module_relative {
                    let path = provider_resource_path(&handle)?;
                    let relative = source_set_relative_path(
                        &self.context.workspace,
                        &self.context.source_root.path,
                        &path,
                    )
                    .ok_or_else(|| {
                        map_error(
                            "location_outside_source_set",
                            "provider resource is outside the selected sourceSet",
                        )
                    })?;
                    if relative == *module_relative {
                        return Ok(addressed());
                    }
                    // A module check keeps only its own module; locating a
                    // foreign file could fail the section for nothing.
                    return Ok(MappedDiagnosticLocation {
                        location: SourceLocation::Unaddressable {
                            source_set: self.context.target.source_set.clone(),
                            owner_metadata_path: None,
                            path: portable_relative(&relative),
                        },
                        reason: Some(UnaddressableReason::OwnerUnproven),
                    });
                }
                map_resource_location(&handle, &self.context, cancellation)
            }
        }
    }
}

impl unica_diagnostics::coordinator::DiagnosticMapping for CanonicalDiagnosticMapping {
    fn resolve_context(
        &self,
        request: &DiagnosticRequest,
        _workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError> {
        if cancellation.is_cancelled() {
            return Err(request_error(
                "cancelled",
                None,
                "diagnostics was cancelled",
            ));
        }
        if request.source_set != self.context.target.source_set
            || request.metadata_path != self.context.target.metadata_path
        {
            return Err(request_error(
                "target_kind_mismatch",
                None,
                "diagnostics request differs from the proved target",
            ));
        }
        Ok(self.context.clone())
    }

    fn map_observation(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError> {
        self.map_cached(
            observation,
            context,
            cancellation,
            &mut DiagnosticMappingCache::default(),
        )
    }

    fn map_observations(
        &self,
        observations: Vec<DiagnosticObservation>,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Vec<Result<DiagnosticItem, DiagnosticMapError>> {
        let mut cache = DiagnosticMappingCache::default();
        observations
            .into_iter()
            .map(|observation| self.map_cached(observation, context, cancellation, &mut cache))
            .collect()
    }
}

impl CanonicalDiagnosticMapping {
    fn map_cached(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
        cache: &mut DiagnosticMappingCache,
    ) -> Result<DiagnosticItem, DiagnosticMapError> {
        if cancellation.is_cancelled() {
            return Err(map_error(
                "cancelled",
                "diagnostic observation mapping was cancelled",
            ));
        }
        let location = match &observation {
            DiagnosticObservation::Diagnostic { location, .. }
            | DiagnosticObservation::ResourceFailure { location, .. } => location.clone(),
        };
        let mapped = match location {
            DiagnosticObservationLocation::Resource { ref handle } => cache
                .resource_locations
                .entry(handle.clone())
                .or_insert_with(|| self.map_location(location, cancellation))
                .clone()?,
            logical => self.map_location(logical, cancellation)?,
        };
        Ok(assemble_diagnostic_observation(
            observation,
            mapped,
            context,
            cancellation,
            cache,
        ))
    }
}

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
    map_diagnostic_observation_cached(
        observation,
        context,
        cancellation,
        &mut DiagnosticMappingCache::default(),
    )
}

/// One result per observation: a resource the mapper cannot prove is that
/// observation's own failure, not proof against the batch it arrived in.
pub(crate) fn map_diagnostic_observations(
    observations: Vec<DiagnosticObservation>,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Vec<Result<DiagnosticItem, DiagnosticMapError>> {
    let mut cache = DiagnosticMappingCache::default();
    observations
        .into_iter()
        .map(|observation| {
            map_diagnostic_observation_cached(observation, context, cancellation, &mut cache)
        })
        .collect()
}

fn map_diagnostic_observation_cached(
    observation: DiagnosticObservation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> Result<DiagnosticItem, DiagnosticMapError> {
    if cancellation.is_cancelled() {
        return Err(map_error(
            "cancelled",
            "diagnostic observation mapping was cancelled",
        ));
    }
    let location = match &observation {
        DiagnosticObservation::Diagnostic { location, .. }
        | DiagnosticObservation::ResourceFailure { location, .. } => location.clone(),
    };
    let mapped = map_location_cached(location, context, cancellation, cache)?;
    Ok(assemble_diagnostic_observation(
        observation,
        mapped,
        context,
        cancellation,
        cache,
    ))
}

fn assemble_diagnostic_observation(
    observation: DiagnosticObservation,
    mapped: MappedDiagnosticLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> DiagnosticItem {
    match observation {
        DiagnosticObservation::Diagnostic {
            provider,
            location: _,
            focus,
            code,
            severity,
            message,
            tags,
        } => {
            let focus = map_focus(focus, &mapped.location, context, cancellation, cache);
            DiagnosticItem::Diagnostic {
                provider: provider.as_str(),
                location: mapped.location,
                location_reason: mapped.reason,
                focus,
                code,
                severity,
                message,
                tags,
            }
        }
        DiagnosticObservation::ResourceFailure {
            provider,
            location: _,
            error,
        } => DiagnosticItem::ResourceFailure {
            provider: provider.as_str(),
            location: mapped.location,
            location_reason: mapped.reason,
            error,
        },
    }
}

#[derive(Debug, Clone)]
struct MappedDiagnosticLocation {
    location: SourceLocation,
    reason: Option<UnaddressableReason>,
}

/// Per-batch memo of everything the mapper would otherwise re-derive for every
/// observation of the same logical resource: the proven location and the object
/// descriptor a metadata focus is checked against.
#[derive(Debug, Default)]
struct DiagnosticMappingCache {
    resource_locations: HashMap<String, Result<MappedDiagnosticLocation, DiagnosticMapError>>,
    descriptors: HashMap<String, Option<String>>,
}

impl DiagnosticMappingCache {
    fn descriptor_text(
        &mut self,
        key: &str,
        load: impl FnOnce() -> Option<String>,
    ) -> Option<&str> {
        self.descriptors
            .entry(key.to_string())
            .or_insert_with(load)
            .as_deref()
    }
}

fn map_location(
    location: DiagnosticObservationLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<MappedDiagnosticLocation, DiagnosticMapError> {
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
            Ok(MappedDiagnosticLocation {
                location: SourceLocation::Addressed {
                    source_set: resolution.resolved.source_set,
                    metadata_path: resolution.resolved.metadata_path,
                    target_kind: resolution.resolved.target_kind,
                },
                reason: None,
            })
        }
        DiagnosticObservationLocation::Resource { handle } => {
            map_resource_location(&handle, context, cancellation)
        }
    }
}

fn map_location_cached(
    location: DiagnosticObservationLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> Result<MappedDiagnosticLocation, DiagnosticMapError> {
    match location {
        DiagnosticObservationLocation::Resource { handle } => {
            if let Some(mapped) = cache.resource_locations.get(&handle) {
                return mapped.clone();
            }
            let mapped = map_resource_location(&handle, context, cancellation);
            cache.resource_locations.insert(handle, mapped.clone());
            mapped
        }
        logical => map_location(logical, context, cancellation),
    }
}

fn map_resource_location(
    handle: &str,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<MappedDiagnosticLocation, DiagnosticMapError> {
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
        return Ok(MappedDiagnosticLocation {
            location: SourceLocation::Unaddressable {
                source_set: context.target.source_set.clone(),
                owner_metadata_path: None,
                path: observed_path,
            },
            reason: Some(UnaddressableReason::SourceFormatUnsupported),
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
        None => Ok(MappedDiagnosticLocation {
            location: SourceLocation::Addressed {
                source_set: located.source_set,
                metadata_path: located.metadata_path,
                target_kind: located.target_kind.ok_or_else(|| {
                    map_error(
                        "provider_contract_invalid",
                        "addressed provider resource has no target kind",
                    )
                })?,
            },
            reason: None,
        }),
        Some(LocateRejection::NotAddressable) => Ok(MappedDiagnosticLocation {
            location: SourceLocation::Unaddressable {
                source_set: located.source_set,
                owner_metadata_path: located.owner_metadata_path,
                path: located.relative_path,
            },
            reason: Some(UnaddressableReason::ResourceNotAddressable),
        }),
        Some(LocateRejection::OwnerUnproven) => Ok(MappedDiagnosticLocation {
            location: SourceLocation::Unaddressable {
                source_set: located.source_set,
                owner_metadata_path: located.owner_metadata_path,
                path: located.relative_path,
            },
            reason: Some(UnaddressableReason::OwnerUnproven),
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
    location: &SourceLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> DiagnosticFocus {
    match focus {
        DiagnosticObservationFocus::Target => DiagnosticFocus::Target,
        // A zero-width range is a caret, and weakening it to `target` would
        // both lose the position and make the finding answer every requested
        // range. `DiagnosticRange::intersects` owns the caret comparison.
        DiagnosticObservationFocus::SourceRange(range) => DiagnosticFocus::SourceRange { range },
        DiagnosticObservationFocus::Metadata(focus)
            if metadata_focus_is_proven(&focus, location, context, cancellation, cache) =>
        {
            focus.into()
        }
        DiagnosticObservationFocus::Metadata(_) => DiagnosticFocus::Target,
    }
}

fn metadata_focus_is_proven(
    focus: &MetadataFocus,
    location: &SourceLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> bool {
    if cancellation.is_cancelled() {
        return false;
    }
    let SourceLocation::Addressed {
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
    // Every observation of the same object would otherwise re-resolve the
    // target and re-read its descriptor, which is read whole (#1119).
    let Some(text) = cache.descriptor_text(metadata_path.as_str(), || {
        let target = SourceTarget {
            source_set: context.target.source_set.clone(),
            metadata_path: Some(metadata_path.clone()),
        };
        let resolution = resolve_platform_xml_target_in_diagnostic_context(
            &context.workspace,
            &target,
            TargetKindPolicy::Any,
            &context.source_set,
            &context.source_root.path,
        )
        .ok()?;
        let evidence =
            platform_xml_resource_evidence(&context.workspace, &resolution.handle).ok()?;
        let bytes = std::fs::read(&evidence.target_path).ok()?;
        String::from_utf8(bytes).ok()
    }) else {
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
    use super::{
        map_diagnostic_observation, resolve_diagnostic_context, CanonicalDiagnosticMapping,
    };
    use std::fs;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::TempDir;
    use unica_address::source_target::ResolvedTarget;
    use unica_address::source_target::{
        MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use unica_diagnostics::coordinator::DiagnosticMapping;
    use unica_diagnostics::model::DiagnosticAction;
    use unica_diagnostics::model::DiagnosticCodeFilter;
    use unica_diagnostics::model::DiagnosticContext;
    use unica_diagnostics::model::DiagnosticFilter;
    use unica_diagnostics::model::DiagnosticFocus;
    use unica_diagnostics::model::DiagnosticItem;
    use unica_diagnostics::model::DiagnosticObservation;
    use unica_diagnostics::model::DiagnosticObservationFocus;
    use unica_diagnostics::model::DiagnosticObservationLocation;
    use unica_diagnostics::model::DiagnosticProviderId;
    use unica_diagnostics::model::DiagnosticRange;
    use unica_diagnostics::model::DiagnosticRequest;
    use unica_diagnostics::model::DiagnosticSeverity;
    use unica_diagnostics::model::DiagnosticTag;
    use unica_diagnostics::model::MetadataFocus;
    use unica_diagnostics::model::UnaddressableReason;
    use unica_metadata::MetadataElement;
    use unica_source_contract::location::SourceLocation;
    use unica_source_contract::workspace::ResolvedSourceRoot;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_source_contract::SourceFormat;
    use unica_source_contract::{ProjectSourceSet, SourceSetKind, SourceSetState};
    use unica_work_control::cancellation::CancellationToken;

    const TEST_PROVIDER: DiagnosticProviderId = DiagnosticProviderId::new_const("test-provider");

    struct Fixture {
        _temp: TempDir,
        context: WorkspaceContext,
    }

    impl Fixture {
        fn platform_xml() -> Self {
            let temp = tempfile::tempdir().unwrap();
            // Match the normalized workspace identity supplied by production
            // ports; macOS otherwise mixes `/var` and `/private/var` aliases.
            let root = unica_workspace::paths::normalize_path_identity(temp.path()).unwrap();
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
                    cwd: root.clone(),
                    workspace_root: root.clone(),
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

    /// Отображение одного доказанного модуля, как его строит живой
    /// `LogicalViewReadAuthority::diagnostic_mapping` после чтения модуля.
    fn proven_module_mapping(
        fixture: &Fixture,
        module_address: &str,
        module_relative: &str,
    ) -> (CanonicalDiagnosticMapping, DiagnosticContext) {
        let source_root = fixture.source();
        let context = DiagnosticContext::new(
            fixture.context.clone(),
            ProjectSourceSet {
                name: "main".to_string(),
                kind: SourceSetKind::Configuration,
                path: source_root.to_string_lossy().into_owned(),
                source_format: SourceFormat::PlatformXml,
                source_state: SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
            ResolvedTarget {
                source_set: "main".to_string(),
                metadata_path: Some(address(module_address)),
                target_kind: TargetKind::Module,
            },
        );
        let mapping =
            CanonicalDiagnosticMapping::from_proven_module(context, module_relative.into())
                .unwrap();
        let context = mapping
            .resolve_context(
                &request(Some(module_address)),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        (mapping, context)
    }

    /// Отображение объекта или корня, как его строит живой
    /// `LogicalViewReadAuthority::diagnostic_node_mapping` после чтения узла.
    fn proven_node_mapping(
        fixture: &Fixture,
        metadata_path: Option<&str>,
    ) -> (CanonicalDiagnosticMapping, DiagnosticContext) {
        let source_root = fixture.source();
        let context = DiagnosticContext::new(
            fixture.context.clone(),
            ProjectSourceSet {
                name: "main".to_string(),
                kind: SourceSetKind::Configuration,
                path: source_root.to_string_lossy().into_owned(),
                source_format: SourceFormat::PlatformXml,
                source_state: SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
            ResolvedTarget {
                source_set: "main".to_string(),
                metadata_path: metadata_path.map(address),
                target_kind: if metadata_path.is_some() {
                    TargetKind::MetadataObject
                } else {
                    TargetKind::SourceRoot
                },
            },
        );
        let mapping = CanonicalDiagnosticMapping::from_proven_node(context).unwrap();
        let context = mapping
            .resolve_context(
                &request(metadata_path),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        (mapping, context)
    }

    fn mapped_location(item: DiagnosticItem) -> (SourceLocation, Option<UnaddressableReason>) {
        match item {
            DiagnosticItem::Diagnostic {
                location,
                location_reason,
                ..
            } => (location, location_reason),
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
            mapped_location(root).0,
            SourceLocation::Addressed {
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
            mapped_location(object).0,
            SourceLocation::Addressed {
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
            match mapped_location(item).0 {
                SourceLocation::Addressed {
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
            let (location, reason) = mapped_location(item);
            match location {
                SourceLocation::Unaddressable {
                    owner_metadata_path,
                    path,
                    ..
                } => {
                    assert_eq!(reason, Some(expected_reason));
                    assert_eq!(
                        owner_metadata_path.as_ref().map(MetadataAddress::as_str),
                        expected_owner
                    );
                    assert!(!path.contains('\\'));
                    assert!(!Path::new(&path).is_absolute());
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
        let (mapping, context) = proven_module_mapping(
            &fixture,
            "CommonModule.Модуль с пробелом.Module",
            "CommonModules/Модуль с пробелом/Ext/Module.bsl",
        );
        let absolute_uri = url::Url::from_file_path(&module).unwrap().to_string();
        let handles = [
            "CommonModules\\Модуль с пробелом\\Ext\\Module.bsl".to_string(),
            "./CommonModules/Модуль с пробелом/Ext/./Module.bsl".to_string(),
            absolute_uri,
        ];

        for handle in handles {
            let item = mapping
                .map_observation(
                    diagnostic(
                        DiagnosticObservationLocation::Resource { handle },
                        DiagnosticObservationFocus::Target,
                    ),
                    &context,
                    &cancellation,
                )
                .unwrap();
            match mapped_location(item).0 {
                SourceLocation::Addressed {
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
        fixture.write_common_module("Shared");
        let cancellation = CancellationToken::new();
        let (mapping, context) = proven_module_mapping(
            &fixture,
            "CommonModule.Shared.Module",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let raw = "../outside/secret.bsl";
        let error = mapping
            .map_observation(
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

        let (location, reason) = mapped_location(item);
        assert_eq!(reason, Some(UnaddressableReason::SourceFormatUnsupported));
        assert!(matches!(
            location,
            SourceLocation::Unaddressable { path, .. }
                if path == "CommonModules/Any/Ext/Module.bsl"
        ));
    }

    #[test]
    fn canonical_diagnostic_location_maps_root_object_module_and_nested_form_module() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        let module = fixture.write_common_module("Shared");
        let form_module = fixture.write_form_module("Items", "Card");
        let cancellation = CancellationToken::new();
        let (root, root_context) = proven_node_mapping(&fixture, None);
        let map = |mapping: &CanonicalDiagnosticMapping,
                   context: &DiagnosticContext,
                   location: DiagnosticObservationLocation| {
            mapped_location(
                mapping
                    .map_observation(
                        diagnostic(location, DiagnosticObservationFocus::Target),
                        context,
                        &cancellation,
                    )
                    .unwrap(),
            )
            .0
        };

        assert!(matches!(
            map(
                &root,
                &root_context,
                DiagnosticObservationLocation::Logical {
                    metadata_path: None
                }
            ),
            SourceLocation::Addressed {
                metadata_path: None,
                target_kind: TargetKind::SourceRoot,
                ..
            }
        ));
        let (object, object_context) = proven_node_mapping(&fixture, Some("Catalog.Items"));
        for (mapping, context) in [(&root, &root_context), (&object, &object_context)] {
            match map(
                mapping,
                context,
                DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
            ) {
                SourceLocation::Addressed {
                    metadata_path: Some(actual),
                    target_kind: TargetKind::MetadataObject,
                    ..
                } => assert_eq!(actual.as_str(), "Catalog.Items"),
                location => panic!("unexpected object location: {location:?}"),
            }
        }
        // Модуль объекта адресуется отдельно: его находка — модулю, а не объекту.
        for (mapping, context) in [(&root, &root_context), (&object, &object_context)] {
            for (path, expected) in [
                (&module, "CommonModule.Shared.Module"),
                (&form_module, "Catalog.Items.Form.Card.FormModule"),
            ] {
                match map(
                    mapping,
                    context,
                    DiagnosticObservationLocation::Resource {
                        handle: path.to_string_lossy().into_owned(),
                    },
                ) {
                    SourceLocation::Addressed {
                        metadata_path: Some(actual),
                        target_kind: TargetKind::Module,
                        ..
                    } => assert_eq!(actual.as_str(), expected),
                    location => panic!("unexpected module location: {location:?}"),
                }
            }
        }
        assert!(CanonicalDiagnosticMapping::from_proven_node(root_context.clone()).is_ok());
        let mut module_context = root_context;
        module_context.target.target_kind = TargetKind::Module;
        assert!(
            CanonicalDiagnosticMapping::from_proven_node(module_context).is_err(),
            "a module is proved by its file, not as a node"
        );
    }

    #[test]
    fn canonical_diagnostic_location_distinguishes_unaddressable_owner_and_unproven_owner() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        fixture.write_common_module("Shared");
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
        let mappings = [
            proven_node_mapping(&fixture, None),
            proven_node_mapping(&fixture, Some("Catalog.Items")),
        ];
        for (mapping, context) in &mappings {
            for (handle, expected_reason, expected_owner) in [
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
            ] {
                let item = mapping
                    .map_observation(
                        diagnostic(
                            DiagnosticObservationLocation::Resource {
                                handle: handle.to_string(),
                            },
                            DiagnosticObservationFocus::Target,
                        ),
                        context,
                        &cancellation,
                    )
                    .unwrap();
                let (location, reason) = mapped_location(item);
                match location {
                    SourceLocation::Unaddressable {
                        owner_metadata_path,
                        path,
                        ..
                    } => {
                        assert_eq!(reason, Some(expected_reason), "{handle}");
                        assert_eq!(
                            owner_metadata_path.as_ref().map(MetadataAddress::as_str),
                            expected_owner,
                            "{handle}"
                        );
                        assert_eq!(path, handle);
                        assert!(!Path::new(&path).is_absolute());
                    }
                    location => panic!("unexpected unaddressable location: {location:?}"),
                }
            }
        }
        // Модульная проверка не ищет владельца чужого файла: он ей не нужен.
        let (module, module_context) = proven_module_mapping(
            &fixture,
            "CommonModule.Shared.Module",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let item = module
            .map_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource {
                        handle: "Catalogs/Items/Ext/Unknown.xml".to_string(),
                    },
                    DiagnosticObservationFocus::Target,
                ),
                &module_context,
                &cancellation,
            )
            .unwrap();
        assert!(matches!(
            mapped_location(item),
            (
                SourceLocation::Unaddressable {
                    owner_metadata_path: None,
                    ..
                },
                Some(UnaddressableReason::OwnerUnproven)
            )
        ));
    }

    #[test]
    fn canonical_diagnostic_location_preserves_exact_metadata_focus_and_weakens_unknown_elements() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        let cancellation = CancellationToken::new();
        let (mapping, context) = proven_node_mapping(&fixture, Some("Catalog.Items"));
        let map = |focus: MetadataFocus| {
            mapping
                .map_observation(
                    diagnostic(
                        DiagnosticObservationLocation::Logical {
                            metadata_path: Some(address("Catalog.Items")),
                        },
                        DiagnosticObservationFocus::Metadata(focus),
                    ),
                    &context,
                    &cancellation,
                )
                .unwrap()
        };
        let element = |collection: &str, name: &str| MetadataElement {
            collection: collection.to_string(),
            name: name.to_string(),
        };
        for expected in [
            MetadataFocus {
                element_path: Vec::new(),
                property: Some("Synonym".to_string()),
                language: Some("ru".to_string()),
            },
            MetadataFocus {
                element_path: vec![element("attributes", "Code")],
                property: Some("Type".to_string()),
                language: None,
            },
            MetadataFocus {
                element_path: vec![
                    element("tabularSections", "Lines"),
                    element("attributes", "Price"),
                ],
                property: Some("Type".to_string()),
                language: None,
            },
        ] {
            match map(expected.clone()) {
                DiagnosticItem::Diagnostic {
                    location:
                        SourceLocation::Addressed {
                            target_kind: TargetKind::MetadataObject,
                            ..
                        },
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
        // Недоказанный элемент, язык или частное поле XML — находка объекта целиком.
        for weakened in [
            MetadataFocus {
                element_path: vec![element("attributes", "Missing")],
                property: Some("Type".to_string()),
                language: None,
            },
            MetadataFocus {
                element_path: Vec::new(),
                property: Some("Synonym".to_string()),
                language: Some("en".to_string()),
            },
            MetadataFocus {
                element_path: Vec::new(),
                property: Some("InternalField".to_string()),
                language: None,
            },
        ] {
            assert!(
                matches!(
                    map(weakened.clone()),
                    DiagnosticItem::Diagnostic {
                        focus: DiagnosticFocus::Target,
                        location: SourceLocation::Addressed {
                            target_kind: TargetKind::MetadataObject,
                            ..
                        },
                        ..
                    }
                ),
                "{weakened:?}"
            );
        }
    }

    #[test]
    fn canonical_diagnostic_location_reports_unsupported_source_format_without_a_physical_path() {
        let fixture = Fixture::platform_xml();
        let cancellation = CancellationToken::new();
        let (_, context) = proven_node_mapping(&fixture, None);
        let mut context = context;
        context.source_set.source_format = SourceFormat::Edt;
        let mapping = CanonicalDiagnosticMapping::from_proven_node(context.clone()).unwrap();
        let item = mapping
            .map_observation(
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

        let (location, reason) = mapped_location(item);
        assert_eq!(reason, Some(UnaddressableReason::SourceFormatUnsupported));
        assert!(matches!(
            location,
            SourceLocation::Unaddressable { owner_metadata_path: None, path, .. }
                if path == "CommonModules/Any/Ext/Module.bsl"
        ));
    }

    #[test]
    fn zero_width_provider_range_stays_a_caret_focus() {
        let fixture = Fixture::platform_xml();
        let module = fixture.write_common_module("Shared");
        let cancellation = CancellationToken::new();
        let (mapping, context) = proven_module_mapping(
            &fixture,
            "CommonModule.Shared.Module",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let caret = DiagnosticRange {
            start_line: 40,
            start_column: 4,
            end_line: 40,
            end_column: 4,
        };

        let item = mapping
            .map_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource {
                        handle: module.to_string_lossy().into_owned(),
                    },
                    DiagnosticObservationFocus::SourceRange(caret),
                ),
                &context,
                &cancellation,
            )
            .unwrap();

        assert!(
            matches!(
                &item,
                DiagnosticItem::Diagnostic {
                    focus: DiagnosticFocus::SourceRange { range },
                    ..
                } if *range == caret
            ),
            "a zero-width range must keep its position instead of weakening to target: {item:?}"
        );
    }

    #[test]
    fn metadata_focus_descriptor_is_read_once_per_mapped_batch() {
        let mut cache = super::DiagnosticMappingCache::default();
        let mut reads = 0usize;
        let mut load = |cache: &mut super::DiagnosticMappingCache| {
            cache
                .descriptor_text("Catalog.Items", || {
                    reads += 1;
                    Some("<MetaDataObject/>".to_string())
                })
                .map(str::to_string)
        };

        assert_eq!(load(&mut cache).as_deref(), Some("<MetaDataObject/>"));
        assert_eq!(load(&mut cache).as_deref(), Some("<MetaDataObject/>"));
        assert_eq!(reads, 1, "the descriptor must be read once per batch");

        let mut misses = 0usize;
        for _ in 0..2 {
            assert!(cache
                .descriptor_text("Catalog.Missing", || {
                    misses += 1;
                    None
                })
                .is_none());
        }
        assert_eq!(misses, 1, "an unreadable descriptor is not retried either");
    }

    mod platform_tests {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/platform/diagnostics_windows.rs"
        ));
    }
}

pub(crate) fn findings_module_path(
    request: &DiagnosticProviderRequest,
    context: &DiagnosticContext,
) -> Result<PathBuf, String> {
    let metadata_path = request
        .metadata_path
        .clone()
        .ok_or_else(|| "findings request requires metadataPath".to_string())?;
    let target = SourceTarget {
        source_set: request.source_set.clone(),
        metadata_path: Some(metadata_path),
    };
    let resolution = resolve_platform_xml_target_in_diagnostic_context(
        &context.workspace,
        &target,
        TargetKindPolicy::ModuleOnly,
        &context.source_set,
        &context.source_root.path,
    )
    .map_err(|error| format!("could not prove findings module: {:?}", error.code))?;
    platform_xml_resource_evidence(&context.workspace, &resolution.handle)
        .map(|evidence| evidence.target_path)
        .map_err(|error| format!("could not prove findings module resource: {error}"))
}

#[cfg(test)]
#[path = "../../tests/platform/bsl_diagnostics_provider.rs"]
mod bsl_diagnostics_provider_tests;
