use super::*;
use unica_address::source_target::MetadataAddress;
use unica_source_designer::layout::{attached_resource_relative, metadata_descriptor_relative};

/// A failed point lookup never supplies a resolved address. Its diagnostic may
/// identify a layout candidate when the selected file is physically absent.
#[derive(Debug, PartialEq, Eq)]
pub enum PathMiss {
    Unplaced,
    Absent {
        at: QualifiedAddress,
        kind: NodeKind,
    },
    OutsideSourceSets,
}

pub fn classify_path_miss(
    sources: &[LayoutFindSource<'_>],
    workspace: &Path,
    query: &str,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<PathMiss, FindBuildError> {
    let normalized = normalize_layout_path(query);
    let absolute = layout_path_is_absolute(query, &normalized);
    // Git reports native names. On Unix a backslash may be the literal name
    // of an existing entry, not evidence about another file below it.
    let requested = Path::new(query);
    if requested
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Ok(PathMiss::Unplaced);
    }
    let workspace_path = workspace.join(requested);
    let workspace_path = if absolute { requested } else { &workspace_path };
    let mut relatives = deepest_source_relatives(
        sources,
        &workspace_path.to_string_lossy(),
        deadline,
        cancellation,
    )?;
    let inside = relatives.iter().any(Option::is_some);
    let explicit = absolute
        || sources
            .iter()
            .zip(&relatives)
            .any(|(source, relative)| relative.is_some() && source.root.path() != workspace);
    if !explicit {
        let parts = normalized.split('/').collect::<Vec<_>>();
        let mut prefixed = false;
        for (source, relative) in sources.iter().zip(&mut relatives) {
            *relative = None;
            if let Some(depth) = relative_query_depth(source, &normalized, &parts) {
                prefixed = true;
                let prefix_length = parts.len() - depth;
                if let Some(parent) = source.root.path().ancestors().nth(prefix_length) {
                    *relative = absolute_query_relative_path(
                        source,
                        &parent.join(requested).to_string_lossy(),
                        deadline,
                        cancellation,
                    )?;
                }
            }
        }
        if !prefixed {
            relatives.fill(Some(requested.to_owned()));
        }
    }

    let mut belongs_to_source = inside;
    let mut candidate = None;
    let mut present = false;
    for (source, relative) in sources.iter().zip(relatives) {
        let Some(relative) = relative else { continue };
        find_checkpoint(deadline, cancellation)?;
        let first = relative
            .components()
            .next()
            .and_then(|component| match component {
                std::path::Component::Normal(name) => name.to_str(),
                _ => None,
            });
        let Some(first) = first else { continue };
        let resource = layout_candidate(source, &relative);
        if resource.is_none() && explicit && inside {
            continue;
        }
        if retain_query_target(source, &relative, deadline, cancellation)?.is_some() {
            present = true;
            belongs_to_source = true;
            continue;
        }
        let Some(resource) = resource else {
            belongs_to_source |=
                metadata_kind_by_directory(first).is_some() || first.eq_ignore_ascii_case("Ext");
            continue;
        };
        belongs_to_source = true;
        if candidate
            .as_ref()
            .is_some_and(|previous| previous != &resource)
        {
            return Err(FindBuildError::new(
                RefusalCode::BadValue,
                "resolve path matches multiple equally specific objects; use their logical address",
            ));
        }
        candidate = Some(resource);
    }
    find_checkpoint(deadline, cancellation)?;
    Ok(match candidate {
        Some((at, kind)) if !present => PathMiss::Absent { at, kind },
        _ if belongs_to_source => PathMiss::Unplaced,
        _ => PathMiss::OutsideSourceSets,
    })
}

fn layout_candidate(
    source: &LayoutFindSource<'_>,
    relative: &Path,
) -> Option<(QualifiedAddress, NodeKind)> {
    // External artifacts can give their descriptor an arbitrary physical name.
    // The missing filename cannot supply the logical owner in those layouts.
    if !matches!(
        source.kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) || relative.components().count() > MAX_POINT_PATH_DEPTH
    {
        return None;
    }
    let qualify =
        |logical: &str| QualifiedAddress::parse(&format!("{}:{logical}", source.name)).ok();
    if relative == Path::new("Configuration.xml") {
        return Some((qualify("Configuration")?, NodeKind::Configuration));
    }
    if relative.extension()? == "bsl" {
        // Use the source grammar in both directions. The candidate does not
        // depend on a platform event catalog or an owner's registration.
        let identity =
            unica_source_designer::module_layout::platform_xml_module_identity(relative).ok()?;
        if module_relative(&identity.address, source.kind).ok()? != relative {
            return None;
        }
        let logical = unica_source_designer::module_layout::logical_module_from_metadata(
            identity.address.as_str(),
        )?;
        let address = qualify(&logical)?;
        unica_address::module_routes::module_route(&address)?;
        let kind = address.segments().last()?.kind();
        return Some((address, kind));
    }
    let parts = relative
        .iter()
        .map(OsStr::to_str)
        .collect::<Option<Vec<_>>>()?;
    let [collection, tail @ ..] = parts.as_slice() else {
        return None;
    };
    let owner = metadata_kind_by_directory(collection)?.tag;
    let (logical, content) = match tail {
        [file] => (format!("{owner}.{}", file.strip_suffix(".xml")?), false),
        [name, "Forms", file] => (
            format!("{owner}.{name}.Form.{}", file.strip_suffix(".xml")?),
            false,
        ),
        [name, "Ext", "Form.xml"] if owner == "CommonForm" => (format!("{owner}.{name}"), true),
        [name, "Forms", form, "Ext", "Form.xml"] => (format!("{owner}.{name}.Form.{form}"), true),
        _ => return None,
    };
    let target = MetadataAddress::parse_logical(&logical).ok()?;
    if content {
        return (attached_resource_relative(&target, "Form.xml", source.kind).ok()? == relative)
            .then_some((qualify(&logical)?, NodeKind::Form));
    }
    if metadata_descriptor_relative(&target, source.kind).ok()? != relative {
        return None;
    }
    let mut address = qualify(&logical)?;
    let mut kind = address.segments().last()?.kind();
    if matches!(kind, NodeKind::CommonForm | NodeKind::Form) {
        address = qualify(&format!("{logical}.Metadata"))?;
        kind = NodeKind::Metadata;
    }
    Some((address, kind))
}
