//! Prove the precise descriptor UUID at each step of an external source path.

use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_source_contract::SourceSetKind;

use crate::external_data_source_write::{self as xml, ChildStorage};

#[derive(Debug)]
pub struct SupportNode {
    pub logical: String,
    pub uuid: String,
    pub descriptor: PathBuf,
    pub directory: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct SupportPath {
    pub nodes: Vec<SupportNode>,
    pub unreadable: Option<String>,
}

fn uuid(node: Node<'_, '_>) -> Option<String> {
    uuid::Uuid::parse_str(node.attribute("uuid")?)
        .ok()
        .map(|uuid| uuid.to_string())
}

fn utf8(bytes: &[u8]) -> Result<&str, String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    std::str::from_utf8(bytes).map_err(|error| error.to_string())
}

fn parent_element<'a, 'input>(
    bytes: &'input [u8],
    document: &'a Document<'input>,
    kind: NodeKind,
    name: &str,
) -> Result<Node<'a, 'input>, String> {
    let text = utf8(bytes)?;
    let version = unica_source_designer::roots::root_version_literal(text, document.root_element())
        .ok_or_else(|| "descriptor has no literal format version".to_string())?;
    let properties = xml::properties(text, document, kind, name, &version)?;
    properties
        .parent_element()
        .ok_or_else(|| "descriptor has no object".into())
}

/// Retain every proved ancestor even when a child is damaged. A registration
/// with a missing descriptor is damaged; a child with no registration has
/// no own record. The writer still proves existence in the staged image.
pub fn walk<E>(
    at: &QualifiedAddress,
    root_path: &Path,
    root_bytes: &[u8],
    read: &mut (impl FnMut(&Path) -> Result<Option<Vec<u8>>, E> + ?Sized),
) -> Result<SupportPath, E> {
    let mut path = SupportPath::default();
    let [root, children @ ..] = at.segments() else {
        return Ok(path);
    };
    let Some(root_name) = root.name() else {
        return Ok(path);
    };
    if root.kind() != NodeKind::ExternalDataSource {
        return Ok(path);
    }
    let mut parent = (root.kind(), root_name);
    let mut logical = format!("ExternalDataSource.{root_name}");
    let mut bytes = root_bytes.to_vec();
    let mut descriptor = root_path.to_path_buf();
    let mut scope = children.iter().enumerate().peekable();
    loop {
        let checked = (|| -> Result<_, String> {
            let text = utf8(&bytes)?;
            let doc = Document::parse(text).map_err(|error| error.to_string())?;
            let owner = parent_element(&bytes, &doc, parent.0, parent.1)?;
            let owner_uuid = uuid(owner).ok_or_else(|| format!("{logical} has no valid UUID"))?;
            if path.nodes.iter().any(|node| node.uuid == owner_uuid) {
                return Err(format!("{logical} repeats an ancestor's UUID"));
            }
            path.nodes.push(SupportNode {
                logical: logical.clone(),
                uuid: owner_uuid,
                directory: Some(descriptor.with_extension("")),
                descriptor: descriptor.clone(),
            });
            let Some((index, child)) = scope.peek().copied() else {
                return Ok(None);
            };
            if unica_metadata::external_data_source::kind(&at.segments()[..index + 2]).is_none() {
                return Ok(None);
            }
            let Some(name) = child.name() else {
                return Ok(None);
            };
            let directory =
                unica_address::kinds::nested_place(parent.0.as_str(), child.kind().as_str())
                    .and_then(|place| place.directory);
            let storage = if directory.is_some() {
                ChildStorage::Reference
            } else {
                ChildStorage::Inline
            };
            let entry = xml::registered_child(owner, child.kind(), name, storage)?;
            let maybe_child = entry
                .map(|entry| {
                    let child_uuid = match storage {
                        ChildStorage::Inline => Some(uuid(entry).ok_or_else(|| {
                            format!(
                                "{logical}.{}.{name} has no valid UUID",
                                child.kind().as_str()
                            )
                        })?),
                        ChildStorage::Reference => None,
                    };
                    Ok::<_, String>((child.kind(), name, directory, child_uuid))
                })
                .transpose()?;
            Ok(maybe_child)
        })();
        let child = match checked {
            Ok(checked) => checked,
            Err(error) => {
                path.unreadable = Some(format!("{logical}: {error}"));
                return Ok(path);
            }
        };
        let Some((child_kind, child_name, directory, child_uuid)) = child else {
            return Ok(path);
        };
        scope.next();
        logical.push_str(&format!(".{}.{child_name}", child_kind.as_str()));
        if let Some(uuid) = child_uuid {
            if path.nodes.iter().any(|node| node.uuid == uuid) {
                path.unreadable = Some(format!("{logical} repeats an ancestor's UUID"));
                return Ok(path);
            }
            path.nodes.push(SupportNode {
                logical,
                uuid,
                descriptor,
                directory: None,
            });
            return Ok(path);
        }
        let Some(directory) = directory else {
            return Ok(path);
        };
        let child_path = descriptor
            .with_extension("")
            .join(directory)
            .join(format!("{child_name}.xml"));
        let Some(child_bytes) = read(&child_path)? else {
            path.unreadable = Some(format!(
                "{logical} is registered, but its descriptor is absent"
            ));
            return Ok(path);
        };
        if path.nodes.iter().any(|node| node.descriptor == child_path) {
            path.unreadable = Some(format!("{logical}: descriptor chain repeated a file"));
            return Ok(path);
        }
        parent = (child_kind, child_name);
        bytes = child_bytes;
        descriptor = child_path;
    }
}

/// Resolve the exact record for the existing supportRule.set publisher.
pub(crate) fn staged_uuid(
    staged: &mut unica_source_contract::proposal::ApplyProposal<'_>,
    at: &QualifiedAddress,
    site: &str,
) -> Result<String, unica_source_contract::proposal::ApplyPlanError> {
    use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
    fn error(kind: ApplyPlanErrorKind, site: &str, message: impl Into<String>) -> ApplyPlanError {
        ApplyPlanError::new(kind, message).at_path(site.to_string())
    }
    let Some(root_name) = at.segments().first().and_then(|root| root.name()) else {
        return Err(error(
            ApplyPlanErrorKind::BadValue,
            site,
            "external source has no name",
        ));
    };
    let configuration = staged
        .read(Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, site.to_string()))?
        .ok_or_else(|| {
            error(
                ApplyPlanErrorKind::NotFound,
                site,
                "Configuration.xml was not found",
            )
        })?;
    let config_text = utf8(&configuration)
        .map_err(|message| error(ApplyPlanErrorKind::InvalidSource, site, message))?;
    let document = Document::parse(config_text)
        .map_err(|failure| error(ApplyPlanErrorKind::InvalidSource, site, failure.to_string()))?;
    let props = crate::descriptor_comment::descriptor_properties(&document, "Configuration")
        .map_err(|message| error(ApplyPlanErrorKind::InvalidSource, site, message))?;
    let found = xml::registered_child(
        props
            .parent_element()
            .expect("configuration Properties has its owner"),
        NodeKind::ExternalDataSource,
        root_name,
        ChildStorage::Reference,
    )
    .map_err(|message| error(ApplyPlanErrorKind::InvalidSource, site, message))?;
    if found.is_none() {
        return Err(error(
            ApplyPlanErrorKind::NotFound,
            site,
            "external data source is not registered",
        ));
    }
    let root = unica_address::source_target::MetadataAddress::parse_logical(&format!(
        "ExternalDataSource.{root_name}"
    ))
    .map_err(|failure| error(ApplyPlanErrorKind::BadValue, site, failure.to_string()))?;
    let root_path = unica_source_designer::layout::metadata_descriptor_relative(
        &root,
        SourceSetKind::Configuration,
    )
    .map_err(|message| error(ApplyPlanErrorKind::BadValue, site, message))?;
    let root_bytes = staged
        .read(&root_path)
        .map_err(|error| ApplyPlanError::staging(error, site.to_string()))?
        .ok_or_else(|| {
            error(
                ApplyPlanErrorKind::InvalidSource,
                site,
                "registered external data source descriptor is missing",
            )
        })?;
    let path = walk(at, &root_path, &root_bytes, &mut |path| staged.read(path))
        .map_err(|error| ApplyPlanError::staging(error, site.to_string()))?;
    if let Some(reason) = path.unreadable {
        return Err(error(ApplyPlanErrorKind::InvalidSource, site, reason));
    }
    let logical = at.logical_path();
    path.nodes
        .into_iter()
        .find(|node| node.logical == logical)
        .map(|node| node.uuid)
        .ok_or_else(|| {
            error(
                ApplyPlanErrorKind::NotFound,
                site,
                "external source node has no proved registration",
            )
        })
}
