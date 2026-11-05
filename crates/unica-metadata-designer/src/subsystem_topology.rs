//! Actual registered-only XML topology parser/read walk, with final retained identity proof.
use roxmltree::{Document, Node};
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::subsystem::{EffectiveSubsystemRole, SubsystemAddress};
use unica_metadata::subsystem_topology::*;
use uuid::Uuid;
const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemTopologyError {
    message: String,
}

impl SubsystemTopologyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SubsystemTopologyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SubsystemTopologyError {}

impl From<io::Error> for SubsystemTopologyError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::Interrupted
            && error
                .to_string()
                .starts_with(unica_work_control::cancellation::CANCELLED_PREFIX)
        {
            return Self::new(error.to_string());
        }
        Self::new(format!("subsystem topology snapshot failed: {error}"))
    }
}

pub fn read_registered_subsystem_topology(
    mut reader: Box<dyn unica_source_contract::read::RetainedSourceRead>,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<SubsystemTopology, SubsystemTopologyError> {
    let configuration_path = PathBuf::from("Configuration.xml");
    let configuration = reader
        .read_regular_file(&configuration_path, &mut checkpoint)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SubsystemTopologyError::new("Configuration.xml is missing")
            } else {
                SubsystemTopologyError::from(error)
            }
        })?;
    let root_registrations = parse_configuration_registrations(&configuration)?;
    let mut dependency_paths = vec![configuration_path];
    let mut dependency_documents = vec![SubsystemTopologyDependencyDocument {
        path: dependency_paths[0].clone(),
        bytes: configuration,
    }];
    let roots = build_registered_nodes(
        &root_registrations,
        &[],
        true,
        reader.as_mut(),
        &mut dependency_paths,
        &mut dependency_documents,
        &mut checkpoint,
    )?;
    reader.complete(&mut checkpoint)?;
    Ok(SubsystemTopology {
        roots,
        dependency_paths,
        dependency_documents,
    })
}

/// Разделы командного интерфейса: подсистемы верхнего уровня из
/// `Configuration.xml`, включённые в интерфейс, в порядке регистрации.
/// Ими ограничен порядок разделов, пока он ещё не задан.
pub fn command_interface_sections<E>(
    configuration: &[u8],
    mut descriptor: impl FnMut(&Path) -> Result<Option<Vec<u8>>, E>,
) -> Result<Vec<String>, CommandInterfaceSectionsError<E>> {
    let registrations = parse_configuration_registrations(configuration)?;
    reject_duplicate_names(&registrations, &[])?;
    let mut sections = Vec::new();
    for name in registrations {
        let logical_path = descriptor_logical_path(std::slice::from_ref(&name));
        let bytes = descriptor(Path::new(&logical_path))
            .map_err(CommandInterfaceSectionsError::Read)?
            .ok_or_else(|| {
                SubsystemTopologyError::new(format!(
                    "registered subsystem descriptor `{logical_path}` is missing"
                ))
            })?;
        if parse_subsystem_descriptor(&bytes, &name, &logical_path)?.include {
            sections.push(name);
        }
    }
    Ok(sections)
}

/// Отказ чтения разделов: исходники противоречат топологии или не читаются.
#[derive(Debug)]
pub enum CommandInterfaceSectionsError<E> {
    Topology(SubsystemTopologyError),
    Read(E),
}

impl<E> From<SubsystemTopologyError> for CommandInterfaceSectionsError<E> {
    fn from(error: SubsystemTopologyError) -> Self {
        Self::Topology(error)
    }
}

fn parse_configuration_registrations(bytes: &[u8]) -> Result<Vec<String>, SubsystemTopologyError> {
    let document = parse_document(bytes, "Configuration.xml")?;
    let configuration = single_object(&document, "Configuration", "Configuration.xml")?;
    let child_objects = single_child(configuration, "ChildObjects", "Configuration.xml")?;
    registered_children(child_objects, "Configuration.xml")
}

struct DescriptorFacts {
    include: bool,
    content: Vec<ContentReference>,
    children: Vec<String>,
}

fn parse_subsystem_descriptor(
    bytes: &[u8],
    expected_name: &str,
    logical_path: &str,
) -> Result<DescriptorFacts, SubsystemTopologyError> {
    let document = parse_document(bytes, logical_path)?;
    let subsystem = single_object(&document, "Subsystem", logical_path)?;
    let properties = single_child(subsystem, "Properties", logical_path)?;
    let name = single_text_child(properties, "Name", logical_path)?;
    if name != expected_name {
        return Err(SubsystemTopologyError::new(format!(
            "registered subsystem `{logical_path}` declares Name `{name}` instead of `{expected_name}`"
        )));
    }
    let include = match single_text_child(
        properties,
        "IncludeInCommandInterface",
        logical_path,
    )?
    .as_str()
    {
        "true" => true,
        "false" => false,
        value => {
            return Err(SubsystemTopologyError::new(format!(
                "registered subsystem `{logical_path}` has non-canonical IncludeInCommandInterface `{value}`"
            )))
        }
    };
    let content = content_items(
        single_child(properties, "Content", logical_path)?,
        logical_path,
    )?;
    let child_objects = single_child(subsystem, "ChildObjects", logical_path)?;
    let children = registered_children(child_objects, logical_path)?;
    Ok(DescriptorFacts {
        include,
        content,
        children,
    })
}

fn content_items(
    content: Node<'_, '_>,
    logical_path: &str,
) -> Result<Vec<ContentReference>, SubsystemTopologyError> {
    reject_non_whitespace_direct_text(content, logical_path, "Content")?;
    content
        .children()
        .filter(Node::is_element)
        .map(|node| {
            let type_is_metadata_reference = node
                .attribute((XSI_NS, "type"))
                .and_then(|value| value.split_once(':'))
                .is_some_and(|(prefix, local)| {
                    !prefix.is_empty()
                        && local == "MDObjectRef"
                        && node.lookup_namespace_uri(Some(prefix)) == Some(READABLE_NS)
                });
            if node.tag_name().namespace() != Some(READABLE_NS)
                || node.tag_name().name() != "Item"
                || !type_is_metadata_reference
                || node.children().any(|child| child.is_element())
            {
                return Err(SubsystemTopologyError::new(format!(
                    "registered subsystem `{logical_path}` has an invalid Content item"
                )));
            }
            let scalar = node
                .children()
                .filter(|child| child.is_text())
                .filter_map(|child| child.text())
                .collect::<String>();
            let value = scalar.trim();
            if value.is_empty() {
                return Err(SubsystemTopologyError::new(format!(
                    "registered subsystem `{logical_path}` has an empty Content item"
                )));
            }
            parse_content_reference(value, logical_path)
        })
        .collect()
}

fn build_registered_nodes(
    names: &[String],
    ancestors: &[String],
    ancestors_included: bool,
    reader: &mut dyn unica_source_contract::read::RetainedSourceRead,
    dependency_paths: &mut Vec<PathBuf>,
    dependency_documents: &mut Vec<SubsystemTopologyDependencyDocument>,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> Result<Vec<SubsystemTopologyNode>, SubsystemTopologyError> {
    reject_duplicate_names(names, ancestors)?;
    let mut nodes = Vec::with_capacity(names.len());
    for name in names {
        checkpoint()?;
        let mut address_names = ancestors.to_vec();
        address_names.push(name.clone());
        let address = SubsystemAddress::from_names(address_names.iter().map(String::as_str))
            .map_err(|error| SubsystemTopologyError::new(error.to_string()))?;
        let logical_path = descriptor_logical_path(&address_names);
        let bytes = reader
            .read_regular_file(Path::new(&logical_path), &mut *checkpoint)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    SubsystemTopologyError::new(format!(
                        "registered subsystem descriptor `{logical_path}` is missing"
                    ))
                } else {
                    SubsystemTopologyError::from(error)
                }
            })?;
        dependency_paths.push(PathBuf::from(&logical_path));
        let facts = parse_subsystem_descriptor(&bytes, name, &logical_path)?;
        dependency_documents.push(SubsystemTopologyDependencyDocument {
            path: PathBuf::from(&logical_path),
            bytes,
        });
        let effective_include = ancestors_included && facts.include;
        let children = build_registered_nodes(
            &facts.children,
            &address_names,
            effective_include,
            reader,
            dependency_paths,
            dependency_documents,
            checkpoint,
        )?;
        nodes.push(SubsystemTopologyNode {
            address,
            name: name.clone(),
            role: if effective_include {
                EffectiveSubsystemRole::Interface
            } else {
                EffectiveSubsystemRole::Functional
            },
            content: facts.content,
            children,
        });
    }
    Ok(nodes)
}

fn descriptor_logical_path(names: &[String]) -> String {
    let mut path = PathBuf::from("Subsystems");
    for name in &names[..names.len() - 1] {
        path.push(name);
        path.push("Subsystems");
    }
    path.push(format!("{}.xml", names.last().expect("address has a leaf")));
    path.to_string_lossy().replace('\\', "/")
}

fn reject_duplicate_names(
    names: &[String],
    ancestors: &[String],
) -> Result<(), SubsystemTopologyError> {
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name) {
            let parent = if ancestors.is_empty() {
                "Configuration".to_string()
            } else {
                ancestors.join(".")
            };
            return Err(SubsystemTopologyError::new(format!(
                "duplicate subsystem registration `{name}` under `{parent}`"
            )));
        }
    }
    Ok(())
}

fn parse_document<'a>(
    bytes: &'a [u8],
    logical_path: &str,
) -> Result<Document<'a>, SubsystemTopologyError> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        SubsystemTopologyError::new(format!("`{logical_path}` is not UTF-8: {error}"))
    })?;
    Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| {
        SubsystemTopologyError::new(format!("`{logical_path}` is malformed XML: {error}"))
    })
}

fn single_object<'a>(
    document: &'a Document<'a>,
    expected: &str,
    logical_path: &str,
) -> Result<Node<'a, 'a>, SubsystemTopologyError> {
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_NS) || root.tag_name().name() != "MetaDataObject" {
        return Err(SubsystemTopologyError::new(format!(
            "`{logical_path}` has an unexpected root element"
        )));
    }
    let objects = root.children().filter(Node::is_element).collect::<Vec<_>>();
    if objects.len() != 1
        || objects[0].tag_name().namespace() != Some(MD_NS)
        || objects[0].tag_name().name() != expected
    {
        return Err(SubsystemTopologyError::new(format!(
            "`{logical_path}` must contain exactly one {expected} object"
        )));
    }
    Ok(objects[0])
}

fn single_child<'a>(
    parent: Node<'a, 'a>,
    name: &str,
    logical_path: &str,
) -> Result<Node<'a, 'a>, SubsystemTopologyError> {
    let children = parent
        .children()
        .filter(Node::is_element)
        .filter(|node| node.tag_name().name() == name)
        .collect::<Vec<_>>();
    if children.len() != 1 || children[0].tag_name().namespace() != Some(MD_NS) {
        return Err(SubsystemTopologyError::new(format!(
            "`{logical_path}` must contain exactly one {name}"
        )));
    }
    Ok(children[0])
}

fn single_text_child(
    parent: Node<'_, '_>,
    name: &str,
    logical_path: &str,
) -> Result<String, SubsystemTopologyError> {
    let child = single_child(parent, name, logical_path)?;
    if child.children().any(|node| node.is_element()) {
        return Err(SubsystemTopologyError::new(format!(
            "`{logical_path}` contains a non-scalar {name}"
        )));
    }
    let scalar = child
        .children()
        .filter(|node| node.is_text())
        .filter_map(|node| node.text())
        .collect::<String>();
    let value = scalar.trim();
    if value.is_empty() {
        return Err(SubsystemTopologyError::new(format!(
            "`{logical_path}` contains an empty {name}"
        )));
    }
    Ok(value.to_string())
}

fn registered_children(
    child_objects: Node<'_, '_>,
    logical_path: &str,
) -> Result<Vec<String>, SubsystemTopologyError> {
    reject_non_whitespace_direct_text(child_objects, logical_path, "ChildObjects")?;
    child_objects
        .children()
        .filter(Node::is_element)
        .filter(|node| node.tag_name().name() == "Subsystem")
        .map(|node| {
            if node.tag_name().namespace() != Some(MD_NS)
                || node.tag_name().name() != "Subsystem"
                || node.children().any(|child| child.is_element())
            {
                return Err(SubsystemTopologyError::new(format!(
                    "`{logical_path}` contains an invalid Subsystem registration"
                )));
            }
            let scalar = node
                .children()
                .filter(|child| child.is_text())
                .filter_map(|child| child.text())
                .collect::<String>();
            let name = scalar.trim();
            if name.is_empty() {
                Err(SubsystemTopologyError::new(format!(
                    "`{logical_path}` contains an empty Subsystem registration"
                )))
            } else {
                Ok(name.to_string())
            }
        })
        .collect()
}

fn reject_non_whitespace_direct_text(
    container: Node<'_, '_>,
    logical_path: &str,
    container_name: &str,
) -> Result<(), SubsystemTopologyError> {
    if container
        .children()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .any(|text| !text.trim().is_empty())
    {
        return Err(SubsystemTopologyError::new(format!(
            "`{logical_path}` contains raw text in {container_name}"
        )));
    }
    Ok(())
}

fn parse_content_reference(
    raw: &str,
    logical_path: &str,
) -> Result<ContentReference, SubsystemTopologyError> {
    if let Ok(address) = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw) {
        return Ok(ContentReference::MetadataAddress(address));
    }
    if let Ok(uuid) = Uuid::parse_str(raw) {
        return Ok(ContentReference::Uuid(uuid));
    }
    // A nested subsystem in the content is written by its full platform path.
    if let Some(path) = raw.strip_prefix("Subsystem.") {
        if let Ok(address) = SubsystemAddress::from_names(path.split(".Subsystem.")) {
            return Ok(ContentReference::Subsystem(address));
        }
    }
    Err(SubsystemTopologyError::new(format!(
        "registered subsystem `{logical_path}` has an invalid Content reference `{raw}`"
    )))
}
