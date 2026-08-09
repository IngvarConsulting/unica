use crate::domain::subsystem::{
    EffectiveSubsystemRole, SubsystemAddress, SUBSYSTEM_ADDRESS_MAX_DEPTH,
};
use crate::infrastructure::platform::secure_read::{
    capture_root_relative_regular_files, SecureTreeCaptureLimits,
};
use roxmltree::{Document, Node};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
const SUBSYSTEM_SCAN_MAX_ENTRIES: usize = 20_000;
const SUBSYSTEM_SCAN_MAX_FILES: usize = 20_000;
const SUBSYSTEM_SCAN_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubsystemTopology {
    pub(crate) roots: Vec<SubsystemTopologyNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubsystemTopologyNode {
    pub(crate) address: SubsystemAddress,
    pub(crate) name: String,
    pub(crate) role: EffectiveSubsystemRole,
    pub(crate) content: Vec<String>,
    pub(crate) children: Vec<SubsystemTopologyNode>,
}

impl SubsystemTopology {
    pub(crate) fn functional_addresses(&self) -> Vec<SubsystemAddress> {
        functional_addresses_in(&self.roots)
    }

    pub(crate) fn interface_addresses(&self) -> Vec<SubsystemAddress> {
        interface_addresses_in(&self.roots)
    }

    pub(crate) fn interface_memberships(&self, object_ref: &str) -> Vec<SubsystemAddress> {
        collect_addresses(
            &self.roots,
            EffectiveSubsystemRole::Interface,
            Some(object_ref),
        )
    }
}

pub(crate) fn functional_addresses_in(nodes: &[SubsystemTopologyNode]) -> Vec<SubsystemAddress> {
    collect_addresses(nodes, EffectiveSubsystemRole::Functional, None)
}

pub(crate) fn interface_addresses_in(nodes: &[SubsystemTopologyNode]) -> Vec<SubsystemAddress> {
    collect_addresses(nodes, EffectiveSubsystemRole::Interface, None)
}

fn collect_addresses(
    nodes: &[SubsystemTopologyNode],
    role: EffectiveSubsystemRole,
    content_item: Option<&str>,
) -> Vec<SubsystemAddress> {
    fn visit(
        nodes: &[SubsystemTopologyNode],
        role: EffectiveSubsystemRole,
        content_item: Option<&str>,
        output: &mut Vec<SubsystemAddress>,
    ) {
        for node in nodes {
            if node.role == role
                && content_item.is_none_or(|item| node.content.iter().any(|value| value == item))
            {
                output.push(node.address.clone());
            }
            visit(&node.children, role, content_item, output);
        }
    }

    let mut output = Vec::new();
    visit(nodes, role, content_item, &mut output);
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubsystemTopologyError {
    message: String,
}

impl SubsystemTopologyError {
    fn new(message: impl Into<String>) -> Self {
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
        Self::new(format!("subsystem topology snapshot failed: {error}"))
    }
}

pub(crate) fn capture_registered_subsystem_topology(
    source_root: &Path,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> Result<SubsystemTopology, SubsystemTopologyError> {
    let snapshot = capture_root_relative_regular_files(
        source_root,
        Path::new(""),
        SecureTreeCaptureLimits {
            maximum_depth: SUBSYSTEM_ADDRESS_MAX_DEPTH * 2,
            maximum_entries: SUBSYSTEM_SCAN_MAX_ENTRIES,
            maximum_files: SUBSYSTEM_SCAN_MAX_FILES,
            maximum_bytes: SUBSYSTEM_SCAN_MAX_BYTES,
        },
        should_descend,
        should_capture,
        &mut checkpoint,
    )?;
    let files = snapshot
        .files
        .iter()
        .map(|entry| (entry.logical_path.as_str(), entry.bytes.as_slice()))
        .collect::<HashMap<_, _>>();
    let configuration = files
        .get("Configuration.xml")
        .copied()
        .ok_or_else(|| SubsystemTopologyError::new("Configuration.xml is missing"))?;
    let roots = parse_configuration_registrations(configuration)?;
    let roots = build_registered_nodes(&roots, &[], true, &files, &mut checkpoint)?;
    checkpoint()?;
    Ok(SubsystemTopology { roots })
}

fn should_descend(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("Subsystems")
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("Subsystems")
}

fn should_capture(path: &Path) -> bool {
    path == Path::new("Configuration.xml")
        || (path.extension().and_then(|value| value.to_str()) == Some("xml")
            && path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some("Subsystems"))
}

fn parse_configuration_registrations(bytes: &[u8]) -> Result<Vec<String>, SubsystemTopologyError> {
    let document = parse_document(bytes, "Configuration.xml")?;
    let configuration = single_object(&document, "Configuration", "Configuration.xml")?;
    let child_objects = single_child(configuration, "ChildObjects", "Configuration.xml")?;
    registered_children(child_objects, "Configuration.xml")
}

struct DescriptorFacts {
    include: bool,
    content: Vec<String>,
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
) -> Result<Vec<String>, SubsystemTopologyError> {
    content
        .children()
        .filter(Node::is_element)
        .map(|node| {
            if node.tag_name().namespace() != Some(READABLE_NS)
                || node.tag_name().name() != "Item"
                || node.attribute((XSI_NS, "type")) != Some("xr:MDObjectRef")
            {
                return Err(SubsystemTopologyError::new(format!(
                    "registered subsystem `{logical_path}` has an invalid Content item"
                )));
            }
            let value = node.text().unwrap_or_default().trim();
            if value.is_empty() {
                return Err(SubsystemTopologyError::new(format!(
                    "registered subsystem `{logical_path}` has an empty Content item"
                )));
            }
            Ok(value.to_string())
        })
        .collect()
}

fn build_registered_nodes(
    names: &[String],
    ancestors: &[String],
    ancestors_included: bool,
    files: &HashMap<&str, &[u8]>,
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
        let bytes = files.get(logical_path.as_str()).copied().ok_or_else(|| {
            SubsystemTopologyError::new(format!(
                "registered subsystem descriptor `{logical_path}` is missing"
            ))
        })?;
        let facts = parse_subsystem_descriptor(bytes, name, &logical_path)?;
        let effective_include = ancestors_included && facts.include;
        let children = build_registered_nodes(
            &facts.children,
            &address_names,
            effective_include,
            files,
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
    let objects = root
        .children()
        .filter(Node::is_element)
        .filter(|node| node.tag_name().namespace() == Some(MD_NS))
        .collect::<Vec<_>>();
    if objects.len() != 1 || objects[0].tag_name().name() != expected {
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
        .filter(|node| node.tag_name().namespace() == Some(MD_NS))
        .filter(|node| node.tag_name().name() == name)
        .collect::<Vec<_>>();
    if children.len() != 1 {
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
    let value = child.text().unwrap_or_default().trim();
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
    child_objects
        .children()
        .filter(Node::is_element)
        .filter(|node| node.tag_name().namespace() == Some(MD_NS))
        .filter(|node| node.tag_name().name() == "Subsystem")
        .map(|node| {
            let name = node.text().unwrap_or_default().trim();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::io;
    use std::path::Path;

    fn write_configuration(root: &Path, subsystem_names: &[&str]) {
        let registrations = subsystem_names
            .iter()
            .map(|name| format!("<Subsystem>{name}</Subsystem>"))
            .collect::<String>();
        fs::write(
            root.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects>{registrations}</ChildObjects></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn write_subsystem(
        root: &Path,
        ancestors: &[&str],
        name: &str,
        include: &str,
        content: &[&str],
        children: &[&str],
    ) {
        let mut directory = root.join("Subsystems");
        for ancestor in ancestors {
            directory = directory.join(ancestor).join("Subsystems");
        }
        fs::create_dir_all(&directory).unwrap();
        let content = content
            .iter()
            .map(|reference| format!(r#"<xr:Item xsi:type="xr:MDObjectRef">{reference}</xr:Item>"#))
            .collect::<String>();
        let children = children
            .iter()
            .map(|child| format!("<Subsystem>{child}</Subsystem>"))
            .collect::<String>();
        fs::write(
            directory.join(format!("{name}.xml")),
            format!(
                r#"<MetaDataObject xmlns="{MD_NS}" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Subsystem><Properties><Name>{name}</Name><IncludeInCommandInterface>{include}</IncludeInCommandInterface><Content>{content}</Content></Properties><ChildObjects>{children}</ChildObjects></Subsystem></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn checkpoint() -> io::Result<()> {
        Ok(())
    }

    fn capture_checkpoint_count(root: &Path) -> usize {
        let count = Cell::new(0);
        capture_root_relative_regular_files(
            root,
            Path::new(""),
            SecureTreeCaptureLimits {
                maximum_depth: SUBSYSTEM_ADDRESS_MAX_DEPTH * 2,
                maximum_entries: SUBSYSTEM_SCAN_MAX_ENTRIES,
                maximum_files: SUBSYSTEM_SCAN_MAX_FILES,
                maximum_bytes: SUBSYSTEM_SCAN_MAX_BYTES,
            },
            should_descend,
            should_capture,
            || {
                count.set(count.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        count.get()
    }

    #[test]
    fn registration_order_drives_roles_and_interface_membership() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Zeta", "Alpha"]);
        write_subsystem(root.path(), &[], "Zeta", "true", &[], &["Child"]);
        write_subsystem(
            root.path(),
            &["Zeta"],
            "Child",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );
        write_subsystem(root.path(), &[], "Alpha", "false", &[], &["Hidden"]);
        write_subsystem(
            root.path(),
            &["Alpha"],
            "Hidden",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );

        let source_root = root.path().canonicalize().unwrap();
        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();

        assert_eq!(
            topology
                .roots
                .iter()
                .map(|node| node.address.as_str())
                .collect::<Vec<_>>(),
            ["Zeta", "Alpha"]
        );
        assert_eq!(
            topology
                .interface_addresses()
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Zeta", "Zeta.Child"]
        );
        assert_eq!(
            topology
                .functional_addresses()
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "Alpha.Hidden"]
        );
        assert_eq!(
            topology
                .interface_memberships("InformationRegister.Ledger")
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Zeta.Child"]
        );
    }

    #[test]
    fn unregistered_files_do_not_define_or_break_the_topology() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        fs::write(
            root.path().join("Subsystems/Unregistered.xml"),
            "not XML at all",
        )
        .unwrap();

        let source_root = root.path().canonicalize().unwrap();
        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();

        assert_eq!(
            topology
                .interface_addresses()
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Registered"]
        );
    }

    #[test]
    fn registered_name_mismatch_is_not_complete_evidence() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        let descriptor = root.path().join("Subsystems/Registered.xml");
        let text = fs::read_to_string(&descriptor)
            .unwrap()
            .replace("<Name>Registered</Name>", "<Name>Other</Name>");
        fs::write(descriptor, text).unwrap();

        let source_root = root.path().canonicalize().unwrap();
        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();

        assert!(
            error.to_string().contains("instead of `Registered`"),
            "{error}"
        );
    }

    #[test]
    fn registered_content_requires_the_platform_item_shape() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(
            root.path(),
            &[],
            "Sales",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );
        let descriptor = root.path().join("Subsystems/Sales.xml");
        let malformed = fs::read_to_string(&descriptor)
            .unwrap()
            .replace("<xr:Item ", "<Item ")
            .replace("</xr:Item>", "</Item>");
        fs::write(&descriptor, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("a foreign Content item cannot prove membership");

        assert!(error.to_string().contains("Content"), "{error}");
    }

    #[test]
    fn registered_boolean_must_be_present_once_and_canonical() {
        for (case, replacement) in [
            ("missing", ""),
            (
                "duplicate",
                "<IncludeInCommandInterface>true</IncludeInCommandInterface><IncludeInCommandInterface>false</IncludeInCommandInterface>",
            ),
            (
                "invalid",
                "<IncludeInCommandInterface>True</IncludeInCommandInterface>",
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            write_configuration(root.path(), &["Registered"]);
            write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
            let descriptor = root.path().join("Subsystems/Registered.xml");
            let text = fs::read_to_string(&descriptor).unwrap().replace(
                "<IncludeInCommandInterface>true</IncludeInCommandInterface>",
                replacement,
            );
            fs::write(descriptor, text).unwrap();

            let source_root = root.path().canonicalize().unwrap();
            let error = capture_registered_subsystem_topology(&source_root, checkpoint)
                .expect_err(case);
            assert!(
                error.to_string().contains("IncludeInCommandInterface"),
                "{case}: {error}"
            );
        }
    }

    #[test]
    fn missing_malformed_and_duplicate_registered_nodes_are_rejected() {
        let missing = tempfile::tempdir().unwrap();
        write_configuration(missing.path(), &["Missing"]);
        let source_root = missing.path().canonicalize().unwrap();
        assert!(capture_registered_subsystem_topology(&source_root, checkpoint).is_err());

        let malformed = tempfile::tempdir().unwrap();
        write_configuration(malformed.path(), &["Broken"]);
        fs::create_dir_all(malformed.path().join("Subsystems")).unwrap();
        fs::write(malformed.path().join("Subsystems/Broken.xml"), "<broken>").unwrap();
        let source_root = malformed.path().canonicalize().unwrap();
        assert!(capture_registered_subsystem_topology(&source_root, checkpoint).is_err());

        let duplicate = tempfile::tempdir().unwrap();
        write_configuration(duplicate.path(), &["Same", "Same"]);
        write_subsystem(duplicate.path(), &[], "Same", "true", &[], &[]);
        let source_root = duplicate.path().canonicalize().unwrap();
        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();
        assert!(error
            .to_string()
            .contains("duplicate subsystem registration"));
    }

    #[test]
    fn empty_registration_proves_an_empty_topology_without_a_subsystems_directory() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &[]);
        let source_root = root.path().canonicalize().unwrap();

        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();

        assert!(topology.roots.is_empty());
        assert!(topology.functional_addresses().is_empty());
        assert!(topology.interface_addresses().is_empty());
    }

    #[test]
    fn ninth_registered_level_exceeds_the_shared_address_budget() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["A"]);
        let names = ["A", "B", "C", "D", "E", "F", "G", "H", "I"];
        for (index, name) in names.iter().enumerate() {
            let children = names
                .get(index + 1)
                .copied()
                .into_iter()
                .collect::<Vec<_>>();
            write_subsystem(root.path(), &names[..index], name, "true", &[], &children);
        }
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();

        assert!(
            error.to_string().contains("depth") || error.to_string().contains("1 to 8"),
            "{error}"
        );
    }

    #[test]
    fn complete_result_requires_a_checkpoint_after_secure_capture_and_parsing() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        let source_root = root.path().canonicalize().unwrap();
        let capture_calls = capture_checkpoint_count(&source_root);
        let calls = Cell::new(0);

        let error = capture_registered_subsystem_topology(&source_root, || {
            let next = calls.get() + 1;
            calls.set(next);
            if next > capture_calls + 1 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

        assert!(error.to_string().contains("cancelled"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn registered_descriptor_symlink_is_not_followed() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Linked"]);
        write_subsystem(external.path(), &[], "Linked", "true", &[], &[]);
        fs::create_dir_all(root.path().join("Subsystems")).unwrap();
        symlink(
            external.path().join("Subsystems/Linked.xml"),
            root.path().join("Subsystems/Linked.xml"),
        )
        .unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();

        assert!(
            error.to_string().contains("symbolic links") || error.to_string().contains("missing"),
            "{error}"
        );
    }
}
