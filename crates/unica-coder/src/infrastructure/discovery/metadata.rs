use crate::application::discovery::ports::MetadataCatalogPort;
use crate::domain::discovery::{
    AnalyzedFile, ArtifactId, ArtifactKind, DiscoveryQuery, EvidenceLocation, FactBatch,
    PortableRelativePath, ProviderCoverage, ProviderDiagnostic, ProviderOutcome, SourceFile,
    SourceInventory, StructuralRelationKind,
};
use crate::infrastructure::metadata_kinds::{metadata_kind, metadata_kind_by_directory};
use roxmltree::{Document, Node};
use std::collections::{BTreeMap, BTreeSet};

const METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const MAX_METADATA_DEPTH: usize = 12;

pub(crate) struct PlatformXmlMetadataProvider;

#[derive(Debug, Clone)]
pub(super) struct MetadataCatalog {
    descriptors: Vec<MetadataDescriptor>,
    analyzed_files: Vec<AnalyzedFile>,
}

#[derive(Debug, Clone)]
pub(super) struct MetadataDescriptor {
    pub relative_path: PortableRelativePath,
    pub root: MetadataNode,
}

#[derive(Debug, Clone)]
pub(super) struct MetadataNode {
    pub artifact: ArtifactId,
    pub artifact_kind: ArtifactKind,
    pub name: String,
    pub object_uuid: Option<String>,
    pub locations: Vec<EvidenceLocation>,
    pub children: Vec<MetadataNode>,
    definition_present: bool,
    definition_source: Option<PortableRelativePath>,
}

#[derive(Debug)]
struct RawMetadataDescriptor {
    relative_path: PortableRelativePath,
    analyzed_file: AnalyzedFile,
    root: RawMetadataNode,
}

#[derive(Debug)]
struct RawMetadataNode {
    xml_kind: String,
    artifact_kind: ArtifactKind,
    name: String,
    object_uuid: Option<String>,
    location: EvidenceLocation,
    children: Vec<RawMetadataNode>,
    declaration_only: bool,
}

#[derive(Debug, Clone, Copy)]
struct SubordinateKind {
    collection: &'static str,
    xml_kind: &'static str,
    artifact_kind: ArtifactKind,
}

#[derive(Debug)]
struct SubordinateDescriptorPath {
    parent_path: PortableRelativePath,
    kind: SubordinateKind,
    scope: SubordinateScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubordinateScope {
    RegisteredTopLevel,
    Nested,
}

const SUBORDINATE_KINDS: &[SubordinateKind] = &[
    SubordinateKind {
        collection: "Forms",
        xml_kind: "Form",
        artifact_kind: ArtifactKind::Form,
    },
    SubordinateKind {
        collection: "Templates",
        xml_kind: "Template",
        artifact_kind: ArtifactKind::MetadataObject,
    },
    SubordinateKind {
        collection: "Commands",
        xml_kind: "Command",
        artifact_kind: ArtifactKind::Command,
    },
];

impl MetadataCatalog {
    pub(super) fn nodes(&self) -> Vec<&MetadataNode> {
        let mut nodes = Vec::new();
        for descriptor in &self.descriptors {
            collect_metadata_nodes(&descriptor.root, &mut nodes);
        }
        nodes
    }
}

impl MetadataNode {
    pub(super) fn declared_forms(&self) -> impl Iterator<Item = &MetadataNode> {
        self.children
            .iter()
            .filter(|child| child.artifact_kind == ArtifactKind::Form)
    }

    pub(super) fn resolve_data_path(&self, data_path: &str) -> Option<&MetadataNode> {
        let mut names = data_path
            .split('.')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if names.first().is_some_and(|segment| {
            matches!(
                crate::domain::discovery::normalize_discovery_identity(segment).as_str(),
                "object" | "объект"
            )
        }) {
            names.remove(0);
        }
        resolve_child_path(&self.children, &names)
    }

    pub(super) fn definition_source(&self) -> Option<&PortableRelativePath> {
        self.definition_source.as_ref()
    }

    pub(super) fn primary_location(&self) -> Option<&EvidenceLocation> {
        self.locations.first()
    }

    fn direct_child_mut(&mut self, artifact: &ArtifactId) -> Option<&mut MetadataNode> {
        self.children
            .iter_mut()
            .find(|child| child.artifact == *artifact)
    }
}

fn resolve_child_path<'a>(
    children: &'a [MetadataNode],
    names: &[&str],
) -> Option<&'a MetadataNode> {
    if names.is_empty() || names.len() > MAX_METADATA_DEPTH {
        return None;
    }
    let mut current_children = children;
    let mut resolved = None;
    for name in names {
        let normalized = crate::domain::discovery::normalize_discovery_identity(name);
        let child = current_children.iter().find(|child| {
            crate::domain::discovery::normalize_discovery_identity(&child.name) == normalized
        })?;
        resolved = Some(child);
        current_children = &child.children;
    }
    resolved
}

impl MetadataCatalogPort for PlatformXmlMetadataProvider {
    fn metadata(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<crate::domain::discovery::MetadataFact>> {
        if let Some(outcome) = crate::infrastructure::discovery::cancellation_outcome(query) {
            return outcome;
        }
        let catalog = match parse_inventory_catalog(query, files) {
            Ok(catalog) => catalog,
            Err(diagnostic)
                if crate::infrastructure::discovery::is_cancellation_diagnostic(&diagnostic) =>
            {
                return ProviderOutcome::Failed(diagnostic);
            }
            Err(diagnostic) => return ProviderOutcome::ContractViolation(diagnostic),
        };
        let analyzed_files = catalog.analyzed_files.clone();
        let mut records = Vec::new();
        for descriptor in &catalog.descriptors {
            if let Err(diagnostic) = crate::infrastructure::discovery::check_cancellation(query) {
                return ProviderOutcome::Failed(diagnostic);
            }
            match metadata_facts(descriptor, query) {
                Ok(facts) => records.extend(facts),
                Err(diagnostic) => return ProviderOutcome::Failed(diagnostic),
            }
        }
        records.sort();

        let bounded = records.len() > usize::from(query.limits().max_evidence);
        if bounded {
            records.truncate(usize::from(query.limits().max_evidence));
        }
        let contributors = contributors_for_records(&records, &analyzed_files);
        let batch = match build_batch(records, analyzed_files, contributors) {
            Ok(batch) => batch,
            Err(diagnostic) => return ProviderOutcome::ContractViolation(diagnostic),
        };
        if bounded {
            ProviderOutcome::Bounded {
                data: batch,
                diagnostic: ProviderDiagnostic::material(
                    "metadata_evidence_bound",
                    "metadata facts stopped at the maxEvidence limit",
                ),
            }
        } else if inventory_is_bounded(files) {
            ProviderOutcome::Bounded {
                data: batch,
                diagnostic: ProviderDiagnostic::material(
                    "metadata_inventory_bounded",
                    "metadata scope is incomplete because source inventory was truncated",
                ),
            }
        } else {
            ProviderOutcome::Complete(batch)
        }
    }
}

pub(super) fn inventory_is_bounded(inventory: &SourceInventory) -> bool {
    inventory.coverage.files_seen > inventory.coverage.files_analyzed
}

pub(super) fn parse_inventory_catalog(
    query: &DiscoveryQuery<'_>,
    inventory: &SourceInventory,
) -> Result<MetadataCatalog, ProviderDiagnostic> {
    let mut raw_descriptors = Vec::new();
    for file in inventory
        .files
        .iter()
        .filter(|file| is_metadata_descriptor_candidate(&file.relative_path))
    {
        crate::infrastructure::discovery::check_cancellation(query)?;
        let descriptor = parse_raw_descriptor(file, query);
        crate::infrastructure::discovery::check_cancellation(query)?;
        let descriptor = descriptor.map_err(|message| {
            ProviderDiagnostic::material(
                "metadata_descriptor_malformed",
                format!(
                    "metadata descriptor {} is malformed: {message}",
                    file.relative_path.as_str()
                ),
            )
        })?;
        raw_descriptors.push(descriptor);
    }
    crate::infrastructure::discovery::check_cancellation(query)?;
    raw_descriptors.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let catalog = build_catalog(raw_descriptors, inventory_is_bounded(inventory), query);
    crate::infrastructure::discovery::check_cancellation(query)?;
    catalog.map_err(|message| ProviderDiagnostic::material("metadata_catalog_invalid", message))
}

fn is_metadata_descriptor_candidate(path: &PortableRelativePath) -> bool {
    let mut components = path.as_str().split('/');
    let Some(file_name) = components.next_back() else {
        return false;
    };
    file_name
        .rsplit_once('.')
        .is_some_and(|(_stem, extension)| extension.eq_ignore_ascii_case("xml"))
        && !components.any(|component| component.eq_ignore_ascii_case("Ext"))
}

fn parse_raw_descriptor(
    file: &SourceFile,
    query: &DiscoveryQuery<'_>,
) -> Result<RawMetadataDescriptor, String> {
    ensure_metadata_active(query)?;
    let text = decode_xml_bytes(&file.bytes)?;
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(METADATA_NAMESPACE)
    {
        return Err(format!(
            "expected MetaDataObject root in {METADATA_NAMESPACE}, got {} in {:?}",
            root.tag_name().name(),
            root.tag_name().namespace()
        ));
    }
    let mut object_elements = root.children().filter(Node::is_element);
    let object = object_elements
        .next()
        .ok_or_else(|| "MetaDataObject has no object element".to_string())?;
    if object_elements.next().is_some() {
        return Err("MetaDataObject has more than one root object element".to_string());
    }
    let root = parse_raw_metadata_node(&document, file, object, 1, false, query)?;
    Ok(RawMetadataDescriptor {
        relative_path: file.relative_path.clone(),
        analyzed_file: file.analyzed_file(),
        root,
    })
}

fn build_catalog(
    raw_descriptors: Vec<RawMetadataDescriptor>,
    inventory_bounded: bool,
    query: &DiscoveryQuery<'_>,
) -> Result<MetadataCatalog, String> {
    ensure_metadata_active(query)?;
    let mut analyzed_files = raw_descriptors
        .iter()
        .map(|descriptor| descriptor.analyzed_file.clone())
        .collect::<Vec<_>>();
    analyzed_files.sort();
    let raw_paths = raw_descriptors
        .iter()
        .map(|descriptor| descriptor.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let mut descriptors = Vec::new();
    let mut subordinate_descriptors = Vec::new();
    for raw in raw_descriptors {
        ensure_metadata_active(query)?;
        if let Some(path) = subordinate_descriptor_path(&raw.relative_path)? {
            let parent_is_present = raw_paths.contains(&path.parent_path);
            if path.scope == SubordinateScope::Nested || parent_is_present {
                subordinate_descriptors.push((path, raw));
                continue;
            }
        }
        let root = materialize_metadata_node(raw.root, None, 1, true)?;
        descriptors.push(MetadataDescriptor {
            relative_path: raw.relative_path,
            root,
        });
    }
    descriptors.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    validate_catalog_nodes(&descriptors)?;

    subordinate_descriptors.sort_by(|left, right| {
        descriptor_path_depth(&left.1.relative_path)
            .cmp(&descriptor_path_depth(&right.1.relative_path))
            .then_with(|| left.1.relative_path.cmp(&right.1.relative_path))
    });
    for (path, subordinate) in subordinate_descriptors {
        ensure_metadata_active(query)?;
        let (parent, parent_depth) = find_definition_node_mut(&mut descriptors, &path.parent_path)?
            .ok_or_else(|| {
                format!(
                    "subordinate {} {} has no parent descriptor {}",
                    path.kind.xml_kind,
                    subordinate.relative_path.as_str(),
                    path.parent_path.as_str()
                )
            })?;
        attach_subordinate_descriptor(parent, parent_depth, path, subordinate)?;
    }
    validate_catalog_nodes(&descriptors)?;
    if !inventory_bounded {
        validate_all_declarations_resolved(&descriptors)?;
    }
    Ok(MetadataCatalog {
        descriptors,
        analyzed_files,
    })
}

fn subordinate_descriptor_path(
    path: &PortableRelativePath,
) -> Result<Option<SubordinateDescriptorPath>, String> {
    let components = path.as_str().split('/').collect::<Vec<_>>();
    if components.len() >= 3 {
        if let Some(kind) = SUBORDINATE_KINDS
            .iter()
            .copied()
            .find(|kind| components[components.len() - 2].eq_ignore_ascii_case(kind.collection))
        {
            let parent_base = components[..components.len() - 2].join("/");
            let parent_path = PortableRelativePath::parse_str(&format!("{parent_base}.xml"))
                .map_err(|error| {
                    format!(
                        "subordinate {} parent path is invalid: {error}",
                        kind.xml_kind
                    )
                })?;
            validate_subordinate_extension(components.last().copied(), kind.xml_kind)?;
            return Ok(Some(SubordinateDescriptorPath {
                parent_path,
                kind,
                scope: SubordinateScope::Nested,
            }));
        }
    }

    if let [directory, file_name] = components.as_slice() {
        if let Some(registered) = metadata_kind_by_directory(directory) {
            validate_subordinate_extension(Some(file_name), registered.tag)?;
            return Ok(Some(SubordinateDescriptorPath {
                parent_path: PortableRelativePath::parse_str("Configuration.xml")
                    .map_err(|error| format!("Configuration path is invalid: {error}"))?,
                kind: SubordinateKind {
                    collection: registered.directory,
                    xml_kind: registered.tag,
                    artifact_kind: artifact_kind_for_xml_object(registered.tag),
                },
                scope: SubordinateScope::RegisteredTopLevel,
            }));
        }
    }
    Ok(None)
}

fn validate_subordinate_extension(file_name: Option<&str>, xml_kind: &str) -> Result<(), String> {
    let Some((_stem, extension)) = file_name.and_then(|name| name.rsplit_once('.')) else {
        return Err(format!(
            "subordinate {xml_kind} descriptor has no .xml extension"
        ));
    };
    if !extension.eq_ignore_ascii_case("xml") {
        return Err(format!(
            "subordinate {xml_kind} descriptor does not end in .xml"
        ));
    }
    Ok(())
}

fn descriptor_path_depth(path: &PortableRelativePath) -> usize {
    path.as_str().split('/').count()
}

fn materialize_metadata_node(
    raw: RawMetadataNode,
    container: Option<(&ArtifactId, &str)>,
    depth: usize,
    descriptor_root: bool,
) -> Result<MetadataNode, String> {
    validate_metadata_depth(depth)?;
    let artifact_text = match container {
        Some((_container, "Configuration")) if metadata_kind(&raw.xml_kind).is_some() => {
            format!("{}.{}", raw.xml_kind, raw.name)
        }
        Some((container, _container_kind)) => {
            format!("{}.{}.{}", container.as_str(), raw.xml_kind, raw.name)
        }
        None => format!("{}.{}", raw.xml_kind, raw.name),
    };
    let artifact = ArtifactId::parse(&artifact_text).map_err(|error| {
        format!(
            "{} object has invalid canonical identity {artifact_text}: {error}",
            raw.xml_kind
        )
    })?;
    let definition_source =
        (descriptor_root && !raw.declaration_only).then(|| raw.location.relative_path.clone());
    let mut children = raw
        .children
        .into_iter()
        .map(|child| {
            materialize_metadata_node(
                child,
                Some((&artifact, &raw.xml_kind)),
                checked_child_depth(depth)?,
                false,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    children.sort_by(|left, right| left.artifact.cmp(&right.artifact));
    Ok(MetadataNode {
        artifact,
        artifact_kind: raw.artifact_kind,
        name: raw.name,
        object_uuid: raw.object_uuid,
        locations: vec![raw.location],
        children,
        definition_present: !raw.declaration_only,
        definition_source,
    })
}

fn attach_subordinate_descriptor(
    parent: &mut MetadataNode,
    parent_depth: usize,
    path: SubordinateDescriptorPath,
    subordinate: RawMetadataDescriptor,
) -> Result<(), String> {
    if subordinate.root.xml_kind != path.kind.xml_kind
        || subordinate.root.artifact_kind != path.kind.artifact_kind
    {
        return Err(format!(
            "subordinate {} descriptor {} contains {} instead of {}",
            path.kind.xml_kind,
            subordinate.relative_path.as_str(),
            subordinate.root.xml_kind,
            path.kind.xml_kind
        ));
    }
    let artifact_text = match path.scope {
        SubordinateScope::RegisteredTopLevel => {
            format!("{}.{}", path.kind.xml_kind, subordinate.root.name)
        }
        SubordinateScope::Nested => format!(
            "{}.{}.{}",
            parent.artifact.as_str(),
            path.kind.xml_kind,
            subordinate.root.name
        ),
    };
    let artifact = ArtifactId::parse(&artifact_text).map_err(|error| {
        format!(
            "subordinate {} identity is invalid: {error}",
            path.kind.xml_kind
        )
    })?;
    let declared = parent
        .direct_child_mut(&artifact)
        .filter(|child| child.artifact_kind == path.kind.artifact_kind)
        .ok_or_else(|| {
            format!(
                "subordinate {} {} is not declared by {}",
                path.kind.xml_kind,
                subordinate.relative_path.as_str(),
                path.parent_path.as_str()
            )
        })?;
    if declared.definition_present {
        return Err(format!(
            "subordinate {} {} duplicates an existing concrete definition",
            path.kind.xml_kind,
            artifact.as_str()
        ));
    }
    let expected_path_text = match path.scope {
        SubordinateScope::RegisteredTopLevel => {
            format!("{}/{}.xml", path.kind.collection, declared.name)
        }
        SubordinateScope::Nested => {
            let descriptor_base = path
                .parent_path
                .as_str()
                .strip_suffix(".xml")
                .ok_or_else(|| "parent descriptor does not end in canonical .xml".to_string())?;
            format!(
                "{descriptor_base}/{}/{}.xml",
                path.kind.collection, declared.name
            )
        }
    };
    let expected_path = PortableRelativePath::parse_str(&expected_path_text).map_err(|error| {
        format!(
            "declared subordinate {} path is invalid: {error}",
            path.kind.xml_kind
        )
    })?;
    if subordinate.relative_path != expected_path {
        return Err(format!(
            "subordinate {} path {} conflicts with declared canonical path {}",
            path.kind.xml_kind,
            subordinate.relative_path.as_str(),
            expected_path.as_str()
        ));
    }
    match (&declared.object_uuid, &subordinate.root.object_uuid) {
        (Some(declared_uuid), Some(descriptor_uuid)) if declared_uuid != descriptor_uuid => {
            return Err(format!(
                "subordinate {} {} conflicts with its declared uuid",
                path.kind.xml_kind,
                artifact.as_str()
            ));
        }
        (None, Some(descriptor_uuid)) => declared.object_uuid = Some(descriptor_uuid.clone()),
        (Some(_), Some(_)) => {}
        (Some(_), None) | (None, None) => {
            return Err(format!(
                "subordinate {} {} has no concrete uuid",
                path.kind.xml_kind,
                artifact.as_str()
            ));
        }
    }
    if declared.locations.contains(&subordinate.root.location) {
        return Err(format!(
            "subordinate {} {} repeats an existing evidence location",
            path.kind.xml_kind,
            artifact.as_str()
        ));
    }
    declared.locations.push(subordinate.root.location);
    declared.definition_present = true;
    declared.definition_source = Some(subordinate.relative_path.clone());
    let declared_depth = checked_child_depth(parent_depth)?;
    let mut subordinate_children = subordinate
        .root
        .children
        .into_iter()
        .map(|child| {
            materialize_metadata_node(
                child,
                Some((&artifact, path.kind.xml_kind)),
                checked_child_depth(declared_depth)?,
                false,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    declared.children.append(&mut subordinate_children);
    declared
        .children
        .sort_by(|left, right| left.artifact.cmp(&right.artifact));
    Ok(())
}

fn find_definition_node_mut<'a>(
    descriptors: &'a mut [MetadataDescriptor],
    path: &PortableRelativePath,
) -> Result<Option<(&'a mut MetadataNode, usize)>, String> {
    for descriptor in descriptors {
        if let Some(found) = find_definition_node_in_tree_mut(&mut descriptor.root, path, 1)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn find_definition_node_in_tree_mut<'a>(
    node: &'a mut MetadataNode,
    path: &PortableRelativePath,
    depth: usize,
) -> Result<Option<(&'a mut MetadataNode, usize)>, String> {
    validate_metadata_depth(depth)?;
    if node.definition_source.as_ref() == Some(path) {
        return Ok(Some((node, depth)));
    }
    if node.children.is_empty() {
        return Ok(None);
    }
    let child_depth = checked_child_depth(depth)?;
    for child in &mut node.children {
        if let Some(found) = find_definition_node_in_tree_mut(child, path, child_depth)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn validate_catalog_nodes(descriptors: &[MetadataDescriptor]) -> Result<(), String> {
    let mut artifacts = BTreeMap::new();
    let mut uuids = BTreeMap::new();
    let mut definition_sources = BTreeMap::new();
    for descriptor in descriptors {
        let mut nodes = Vec::new();
        collect_metadata_nodes(&descriptor.root, &mut nodes);
        for node in nodes {
            if let Some(previous_kind) = artifacts.insert(node.artifact.clone(), node.artifact_kind)
            {
                return Err(format!(
                    "duplicate canonical metadata artifact {} ({previous_kind:?} and {:?})",
                    node.artifact.as_str(),
                    node.artifact_kind
                ));
            }
            let unique_locations = node.locations.iter().collect::<BTreeSet<_>>();
            if unique_locations.len() != node.locations.len() {
                return Err(format!(
                    "metadata artifact {} has duplicate evidence locations",
                    node.artifact.as_str()
                ));
            }
            if let Some(uuid) = &node.object_uuid {
                if let Some(previous_artifact) = uuids.insert(uuid.clone(), node.artifact.clone()) {
                    return Err(format!(
                        "metadata uuid {uuid} maps to both {} and {}",
                        previous_artifact.as_str(),
                        node.artifact.as_str()
                    ));
                }
            }
            if let Some(source) = &node.definition_source {
                if let Some(previous_artifact) =
                    definition_sources.insert(source.clone(), node.artifact.clone())
                {
                    return Err(format!(
                        "metadata source {} defines both {} and {}",
                        source.as_str(),
                        previous_artifact.as_str(),
                        node.artifact.as_str()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_all_declarations_resolved(descriptors: &[MetadataDescriptor]) -> Result<(), String> {
    for descriptor in descriptors {
        let mut nodes = Vec::new();
        collect_metadata_nodes(&descriptor.root, &mut nodes);
        if let Some(unresolved) = nodes.into_iter().find(|node| !node.definition_present) {
            return Err(format!(
                "metadata declaration {} has no concrete subordinate descriptor",
                unresolved.artifact.as_str()
            ));
        }
    }
    Ok(())
}

fn collect_metadata_nodes<'a>(node: &'a MetadataNode, output: &mut Vec<&'a MetadataNode>) {
    let mut pending = vec![node];
    while let Some(current) = pending.pop() {
        output.push(current);
        pending.extend(current.children.iter().rev());
    }
}

fn validate_metadata_depth(depth: usize) -> Result<(), String> {
    if depth > MAX_METADATA_DEPTH {
        return Err(format!(
            "metadata nesting exceeds the supported depth of {MAX_METADATA_DEPTH}"
        ));
    }
    Ok(())
}

fn checked_child_depth(depth: usize) -> Result<usize, String> {
    let child_depth = depth
        .checked_add(1)
        .ok_or_else(|| "metadata nesting depth overflowed".to_string())?;
    validate_metadata_depth(child_depth)?;
    Ok(child_depth)
}

pub(super) fn decode_xml_bytes(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes)
        .map(|text| text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("input is not UTF-8: {error}"))
}

fn parse_raw_metadata_node(
    document: &Document<'_>,
    file: &SourceFile,
    node: Node<'_, '_>,
    depth: usize,
    can_be_declaration: bool,
    query: &DiscoveryQuery<'_>,
) -> Result<RawMetadataNode, String> {
    ensure_metadata_active(query)?;
    validate_metadata_depth(depth)?;
    let object_kind = node.tag_name().name();
    if node.tag_name().namespace() != Some(METADATA_NAMESPACE) {
        return Err(format!(
            "{object_kind} object is outside the metadata namespace"
        ));
    }
    let name = metadata_node_name(node)?
        .ok_or_else(|| format!("{object_kind} object has no non-empty Name"))?;
    validate_platform_identifier(&name)
        .map_err(|message| format!("{object_kind} object has invalid Name: {message}"))?;
    let artifact_kind = artifact_kind_for_xml_object(object_kind);
    let location = xml_location(document, file, node)?;
    let object_uuid = node
        .attribute("uuid")
        .map(str::trim)
        .filter(|uuid| !uuid.is_empty())
        .map(str::to_ascii_lowercase);
    if object_uuid
        .as_deref()
        .is_some_and(|uuid| !is_uuid_text(uuid))
    {
        return Err(format!("{object_kind} object has invalid uuid"));
    }
    let declaration_only = can_be_declaration && !node.children().any(|child| child.is_element());
    if !declaration_only && object_uuid.is_none() {
        return Err(format!("concrete {object_kind} object has no uuid"));
    }
    let mut children = Vec::new();
    if let Some(child_objects) = semantic_child(node, "ChildObjects")? {
        for child in child_objects.children().filter(Node::is_element) {
            ensure_metadata_active(query)?;
            children.push(parse_raw_metadata_node(
                document,
                file,
                child,
                checked_child_depth(depth)?,
                true,
                query,
            )?);
        }
    }
    children.sort_by(|left, right| {
        left.xml_kind
            .cmp(&right.xml_kind)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(RawMetadataNode {
        xml_kind: object_kind.to_string(),
        artifact_kind,
        name,
        object_uuid,
        location,
        children,
        declaration_only,
    })
}

fn ensure_metadata_active(query: &DiscoveryQuery<'_>) -> Result<(), String> {
    crate::infrastructure::discovery::check_cancellation(query)
        .map_err(|diagnostic| diagnostic.message)
}

fn is_uuid_text(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, character)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        })
}

fn metadata_node_name(node: Node<'_, '_>) -> Result<Option<String>, String> {
    if let Some(properties) = semantic_child(node, "Properties")? {
        let name = semantic_child(properties, "Name")?
            .and_then(|name| name.text())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned);
        return Ok(name);
    }
    if node.children().any(|child| child.is_element()) {
        return Ok(None);
    }
    Ok(node
        .text()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned))
}

fn semantic_child<'a, 'input>(
    node: Node<'a, 'input>,
    local_name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let mut matches = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == local_name);
    let first = matches.next();
    if let Some(first) = first {
        if first.tag_name().namespace() != Some(METADATA_NAMESPACE) {
            return Err(format!("{local_name} is outside the metadata namespace"));
        }
    }
    if matches.next().is_some() {
        return Err(format!("more than one {local_name} section"));
    }
    Ok(first)
}

pub(super) fn validate_platform_identifier(value: &str) -> Result<(), &'static str> {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return Err("identifier must not be empty");
    };
    if first != '_' && !first.is_alphabetic() {
        return Err("identifier must start with a Unicode letter or underscore");
    }
    if characters.any(|character| character != '_' && !character.is_alphanumeric()) {
        return Err("identifier may contain only Unicode letters, digits, or underscores");
    }
    Ok(())
}

fn artifact_kind_for_xml_object(object_kind: &str) -> ArtifactKind {
    match object_kind {
        "Attribute" => ArtifactKind::Attribute,
        "TabularSection" => ArtifactKind::TabularSection,
        "Form" => ArtifactKind::Form,
        "Command" => ArtifactKind::Command,
        _ => ArtifactKind::MetadataObject,
    }
}

pub(super) fn xml_location(
    document: &Document<'_>,
    file: &SourceFile,
    node: Node<'_, '_>,
) -> Result<EvidenceLocation, String> {
    let position = document.text_pos_at(node.range().start);
    Ok(EvidenceLocation {
        relative_path: file.relative_path.clone(),
        line: Some(position.row),
        column: Some(position.col),
        xml_path: Some(node_xml_path(node)),
    })
}

fn node_xml_path(node: Node<'_, '_>) -> String {
    let segments = node
        .ancestors()
        .filter(Node::is_element)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(xml_path_segment)
        .collect::<Vec<_>>();
    format!("/{}", segments.join("/"))
}

fn xml_path_segment(node: Node<'_, '_>) -> String {
    let name = node.tag_name().name();
    let Some(parent) = node.parent_element() else {
        return name.to_string();
    };
    let siblings = parent
        .children()
        .filter(|sibling| sibling.is_element() && sibling.tag_name().name() == name)
        .collect::<Vec<_>>();
    if siblings.len() <= 1 {
        return name.to_string();
    }
    let index = match siblings
        .iter()
        .position(|sibling| sibling.id() == node.id())
    {
        Some(index) => index + 1,
        None => 1,
    };
    format!("{name}[{index}]")
}

fn metadata_facts(
    descriptor: &MetadataDescriptor,
    query: &DiscoveryQuery<'_>,
) -> Result<Vec<crate::domain::discovery::MetadataFact>, ProviderDiagnostic> {
    flatten_metadata_node(&descriptor.root, None, query)
}

fn flatten_metadata_node(
    node: &MetadataNode,
    container: Option<(&ArtifactId, ArtifactKind)>,
    query: &DiscoveryQuery<'_>,
) -> Result<Vec<crate::domain::discovery::MetadataFact>, ProviderDiagnostic> {
    let mut facts = Vec::new();
    let mut pending = vec![(node, container)];
    while let Some((current, current_container)) = pending.pop() {
        crate::infrastructure::discovery::check_cancellation(query)?;
        for location in &current.locations {
            crate::infrastructure::discovery::check_cancellation(query)?;
            facts.push(crate::domain::discovery::MetadataFact {
                artifact: current.artifact.clone(),
                search_name: current.name.clone(),
                artifact_kind: current.artifact_kind,
                container: current_container.map(|(artifact, _kind)| artifact.clone()),
                container_kind: current_container.map(|(_artifact, kind)| kind),
                relation: StructuralRelationKind::Contains,
                location: location.clone(),
            });
        }
        pending.extend(
            current
                .children
                .iter()
                .rev()
                .map(|child| (child, Some((&current.artifact, current.artifact_kind)))),
        );
    }
    Ok(facts)
}

pub(super) fn contributors_for_records<T>(
    records: &[T],
    analyzed_files: &[AnalyzedFile],
) -> Vec<AnalyzedFile>
where
    T: crate::domain::discovery::LocatedFact,
{
    let paths = records
        .iter()
        .map(|record| record.location().relative_path.clone())
        .collect::<BTreeSet<_>>();
    analyzed_files
        .iter()
        .filter(|file| paths.contains(&file.relative_path))
        .cloned()
        .collect()
}

pub(super) fn build_batch<T>(
    records: Vec<T>,
    mut analyzed_files: Vec<AnalyzedFile>,
    mut contributors: Vec<AnalyzedFile>,
) -> Result<FactBatch<T>, ProviderDiagnostic> {
    analyzed_files.sort();
    contributors.sort();
    let files_analyzed = u32::try_from(analyzed_files.len()).map_err(|_error| {
        ProviderDiagnostic::material(
            "provider_file_count_overflow",
            "provider analyzed-file count overflowed",
        )
    })?;
    let records_count = u32::try_from(records.len()).map_err(|_error| {
        ProviderDiagnostic::material(
            "provider_record_count_overflow",
            "provider record count overflowed",
        )
    })?;
    let bytes_analyzed = analyzed_files.iter().try_fold(0_u64, |total, file| {
        total.checked_add(file.bytes).ok_or_else(|| {
            ProviderDiagnostic::material(
                "provider_byte_count_overflow",
                "provider analyzed byte count overflowed",
            )
        })
    })?;
    Ok(FactBatch {
        records,
        analyzed_files,
        contributors,
        coverage: ProviderCoverage::new(
            files_analyzed,
            files_analyzed,
            bytes_analyzed,
            records_count,
        ),
    })
}

pub(super) fn analyzed_file_map(
    catalog: &MetadataCatalog,
) -> BTreeMap<PortableRelativePath, AnalyzedFile> {
    catalog
        .analyzed_files
        .iter()
        .map(|analyzed| (analyzed.relative_path.clone(), analyzed.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::PlatformXmlMetadataProvider;
    use crate::application::discovery::ports::MetadataCatalogPort;
    use crate::domain::discovery::{
        ArtifactId, ArtifactKind, ContentHash, DiscoveryQuery, DiscoveryQueryLimits,
        PortableRelativePath, ProviderCoverage, ProviderOutcome, SourceFile, SourceInventory,
        StructuralRelationKind,
    };

    const DOCUMENT_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Document uuid="10000000-0000-0000-0000-000000000001">
    <Properties><Name>ПриобретениеТоваровУслуг</Name></Properties>
    <ChildObjects>
      <TabularSection uuid="10000000-0000-0000-0000-000000000002">
        <Properties><Name>Товары</Name></Properties>
        <ChildObjects>
          <Attribute uuid="10000000-0000-0000-0000-000000000003">
            <Properties><Name>Серия</Name></Properties>
          </Attribute>
        </ChildObjects>
      </TabularSection>
      <TabularSection uuid="10000000-0000-0000-0000-000000000004">
        <Properties><Name>Серии</Name></Properties>
        <ChildObjects>
          <Attribute uuid="10000000-0000-0000-0000-000000000005">
            <Properties><Name>СрокГодности</Name></Properties>
          </Attribute>
        </ChildObjects>
      </TabularSection>
    </ChildObjects>
  </Document>
</MetaDataObject>"#;

    #[test]
    fn cancelled_query_stops_metadata_before_parsing_records() {
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        cancellation.cancel();
        let query = query(100).with_cancellation(&cancellation);

        let outcome = PlatformXmlMetadataProvider.metadata(&query, &SourceInventory::empty());

        let ProviderOutcome::Failed(diagnostic) = outcome else {
            panic!("cancelled metadata must be a failed provider outcome");
        };
        assert_eq!(diagnostic.code, "discovery_cancelled");
    }

    const PROCESSOR_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <DataProcessor uuid="20000000-0000-0000-0000-000000000001">
    <Properties><Name>ПодборСерийВДокументы</Name></Properties>
    <ChildObjects>
      <Form>РегистрацияИПодборСерийПоОднойСтрокеТоваров</Form>
    </ChildObjects>
  </DataProcessor>
</MetaDataObject>"#;

    const PROCESSOR_FORM_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Form uuid="20000000-0000-0000-0000-000000000002">
    <Properties><Name>РегистрацияИПодборСерийПоОднойСтрокеТоваров</Name></Properties>
  </Form>
</MetaDataObject>"#;

    #[test]
    fn parses_actual_root_identity_recursive_children_and_declared_form_relationships() {
        let inventory = inventory(vec![
            source_file(
                "DataProcessors/ПодборСерийВДокументы.xml",
                PROCESSOR_XML.as_bytes(),
            ),
            source_file(
                "DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров.xml",
                PROCESSOR_FORM_XML.as_bytes(),
            ),
            source_file(
                "Documents/path-name-must-not-drive-identity.xml",
                DOCUMENT_XML.as_bytes(),
            ),
        ]);

        let outcome = PlatformXmlMetadataProvider.metadata(&query(100), &inventory);

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("expected complete metadata facts");
        };
        let separate_series = artifact("Document.ПриобретениеТоваровУслуг.TabularSection.Серии");
        let goods_series =
            artifact("Document.ПриобретениеТоваровУслуг.TabularSection.Товары.Attribute.Серия");
        let processor = artifact("DataProcessor.ПодборСерийВДокументы");
        let form = artifact(
            "DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров",
        );

        assert!(batch.records.iter().any(|fact| {
            fact.artifact == separate_series
                && fact.artifact_kind == ArtifactKind::TabularSection
                && fact.container_kind == Some(ArtifactKind::MetadataObject)
                && fact.relation == StructuralRelationKind::Contains
                && fact.location.line == Some(13)
                && fact.location.xml_path.as_deref()
                    == Some("/MetaDataObject/Document/ChildObjects/TabularSection[2]")
        }));
        assert!(batch
            .records
            .iter()
            .any(|fact| fact.artifact == goods_series
                && fact.container_kind == Some(ArtifactKind::TabularSection)));
        assert!(batch.records.iter().any(|fact| {
            fact.artifact == processor
                && fact.artifact_kind == ArtifactKind::MetadataObject
                && fact.container.is_none()
        }));
        assert!(batch.records.iter().any(|fact| {
            fact.artifact == form
                && fact.artifact_kind == ArtifactKind::Form
                && fact.container.as_ref() == Some(&processor)
        }));
        assert_eq!(batch.analyzed_files.len(), 3);
        assert_eq!(batch.contributors, batch.analyzed_files);
        assert_eq!(
            batch.coverage,
            ProviderCoverage::new(
                3,
                3,
                (DOCUMENT_XML.len() + PROCESSOR_XML.len() + PROCESSOR_FORM_XML.len()) as u64,
                8,
            )
        );
    }

    #[test]
    fn malformed_metadata_descriptor_is_an_atomic_contract_violation() {
        let inventory = inventory(vec![
            source_file("Documents/Good.xml", DOCUMENT_XML.as_bytes()),
            source_file("Documents/Broken.xml", b"<MetaDataObject><Document>"),
        ]);

        let outcome = PlatformXmlMetadataProvider.metadata(&query(100), &inventory);

        let ProviderOutcome::ContractViolation(diagnostic) = outcome else {
            panic!("malformed input must invalidate the whole metadata provider");
        };
        assert_eq!(diagnostic.code, "metadata_descriptor_malformed");
    }

    #[test]
    fn truncated_inventory_keeps_metadata_outcome_bounded() {
        let mut inventory = inventory(vec![source_file(
            "Documents/Present.xml",
            DOCUMENT_XML.as_bytes(),
        )]);
        inventory.coverage.files_seen += 1;

        let outcome = PlatformXmlMetadataProvider.metadata(&query(100), &inventory);

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("truncated inventory must keep metadata bounded");
        };
        assert!(!data.records.is_empty());
        assert_eq!(diagnostic.code, "metadata_inventory_bounded");
    }

    #[test]
    fn subordinate_form_descriptor_uses_only_its_parent_declared_identity() {
        let parent = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Report uuid="50000000-0000-0000-0000-000000000001">
    <Properties><Name>Sales</Name></Properties>
    <ChildObjects><Form>Main</Form></ChildObjects>
  </Report>
</MetaDataObject>"#;
        let subordinate = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Form uuid="50000000-0000-0000-0000-000000000002">
    <Properties><Name>Main</Name></Properties>
  </Form>
</MetaDataObject>"#;
        let inventory = inventory(vec![
            source_file("Reports/Sales.xml", parent.as_bytes()),
            source_file("Reports/Sales/Forms/Main.xml", subordinate.as_bytes()),
        ]);

        let outcome = PlatformXmlMetadataProvider.metadata(&query(100), &inventory);

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("canonical subordinate metadata should be complete");
        };
        let canonical = artifact("Report.Sales.Form.Main");
        assert!(batch.records.iter().any(|fact| {
            fact.artifact == canonical && fact.artifact_kind == ArtifactKind::Form
        }));
        assert!(batch
            .records
            .iter()
            .all(|fact| fact.artifact != artifact("Form.Main")));
    }

    #[test]
    fn tracked_configuration_catalogs_attach_to_registered_canonical_identities() {
        let outcome = PlatformXmlMetadataProvider
            .metadata(&query(100), &tracked_meta_compile_on_support_inventory());

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("full tracked meta-compile inventory must be complete");
        };
        assert!(batch
            .records
            .iter()
            .any(|fact| fact.artifact == artifact("Catalog.Locked")));
        assert!(batch
            .records
            .iter()
            .any(|fact| fact.artifact == artifact("Catalog.Removed")));
        assert!(batch.records.iter().all(|fact| {
            !fact
                .artifact
                .as_str()
                .starts_with("Configuration.ТестКонфиг.Catalog.")
        }));
    }

    #[test]
    fn configuration_report_nested_descriptors_attach_topologically() {
        let configuration = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Configuration uuid="58000000-0000-0000-0000-000000000001">
    <Properties><Name>Demo</Name></Properties>
    <ChildObjects><Report>Sales</Report></ChildObjects>
  </Configuration>
</MetaDataObject>"#;
        let report = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Report uuid="58000000-0000-0000-0000-000000000002">
    <Properties><Name>Sales</Name></Properties>
    <ChildObjects><Template>Main</Template><Command>Run</Command></ChildObjects>
  </Report>
</MetaDataObject>"#;
        let template = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Template uuid="58000000-0000-0000-0000-000000000003">
    <Properties><Name>Main</Name></Properties>
  </Template>
</MetaDataObject>"#;
        let command = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Command uuid="58000000-0000-0000-0000-000000000004">
    <Properties><Name>Run</Name></Properties>
  </Command>
</MetaDataObject>"#;
        let inventory = inventory(vec![
            source_file("Configuration.xml", configuration.as_bytes()),
            source_file("Reports/Sales.xml", report.as_bytes()),
            source_file("Reports/Sales/Templates/Main.xml", template.as_bytes()),
            source_file("Reports/Sales/Commands/Run.xml", command.as_bytes()),
        ]);

        let outcome = PlatformXmlMetadataProvider.metadata(&query(100), &inventory);

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("topologically attached descriptor graph must be complete");
        };
        for expected in [
            "Report.Sales",
            "Report.Sales.Template.Main",
            "Report.Sales.Command.Run",
        ] {
            assert!(
                batch
                    .records
                    .iter()
                    .any(|fact| fact.artifact == artifact(expected)),
                "missing {expected}"
            );
        }
        assert!(batch.records.iter().all(|fact| {
            !fact
                .artifact
                .as_str()
                .starts_with("Configuration.Demo.Report.")
                && fact.artifact != artifact("Template.Main")
                && fact.artifact != artifact("Command.Run")
        }));
    }

    #[test]
    fn bounded_configuration_declarations_keep_top_level_canonical_identity() {
        let configuration = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Configuration uuid="59000000-0000-0000-0000-000000000001">
    <Properties><Name>Demo</Name></Properties>
    <ChildObjects><Catalog>Missing</Catalog></ChildObjects>
  </Configuration>
</MetaDataObject>"#;
        let complete = inventory(vec![source_file(
            "Configuration.xml",
            configuration.as_bytes(),
        )]);
        let mut bounded = complete.clone();
        bounded.coverage.files_seen += 1;

        assert!(matches!(
            PlatformXmlMetadataProvider.metadata(&query(100), &complete),
            ProviderOutcome::ContractViolation(_)
        ));
        let ProviderOutcome::Bounded { data, .. } =
            PlatformXmlMetadataProvider.metadata(&query(100), &bounded)
        else {
            panic!("bounded unresolved Configuration declaration must remain partial");
        };
        assert!(data
            .records
            .iter()
            .any(|fact| fact.artifact == artifact("Catalog.Missing")));
        assert!(data
            .records
            .iter()
            .all(|fact| { fact.artifact != artifact("Configuration.Demo.Catalog.Missing") }));
    }

    #[test]
    fn tracked_template_subordinate_uses_parent_identity_when_inventory_is_bounded() {
        let mut inventory = inventory(vec![
            source_file(
                "Reports/ParityReport.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/template-remove/ParityReport.xml"
                )),
            ),
            source_file(
                "Reports/ParityReport/Templates/MainSchema.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/template-remove/",
                    "ParityReport/Templates/MainSchema.xml"
                )),
            ),
        ]);
        inventory.coverage.files_seen += 1;

        let outcome = PlatformXmlMetadataProvider.metadata(&query(100), &inventory);

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("tracked unresolved sibling must keep the catalog bounded");
        };
        assert_eq!(diagnostic.code, "metadata_inventory_bounded");
        assert!(data.records.iter().any(|fact| {
            fact.artifact == artifact("Report.ParityReport.Template.MainSchema")
                && fact.container.as_ref() == Some(&artifact("Report.ParityReport"))
        }));
        assert!(data
            .records
            .iter()
            .all(|fact| fact.artifact != artifact("Template.MainSchema")));
    }

    #[test]
    fn complete_inventory_rejects_unresolved_declarations_but_bounded_keeps_them_partial() {
        let parent = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Report uuid="55000000-0000-0000-0000-000000000001">
    <Properties><Name>Sales</Name></Properties>
    <ChildObjects><Template>Main</Template></ChildObjects>
  </Report>
</MetaDataObject>"#;
        let complete = inventory(vec![source_file("Reports/Sales.xml", parent.as_bytes())]);
        let mut bounded = complete.clone();
        bounded.coverage.files_seen += 1;

        let complete_outcome = PlatformXmlMetadataProvider.metadata(&query(100), &complete);
        let bounded_outcome = PlatformXmlMetadataProvider.metadata(&query(100), &bounded);

        assert!(matches!(
            complete_outcome,
            ProviderOutcome::ContractViolation(_)
        ));
        let ProviderOutcome::Bounded { data, diagnostic } = bounded_outcome else {
            panic!("bounded inventory may be missing the declared subordinate descriptor");
        };
        assert_eq!(diagnostic.code, "metadata_inventory_bounded");
        assert!(data
            .records
            .iter()
            .any(|fact| fact.artifact == artifact("Report.Sales.Template.Main")));
    }

    #[test]
    fn concrete_metadata_nodes_require_valid_uuids() {
        let root_without_uuid = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Document><Properties><Name>Purchase</Name></Properties></Document>
</MetaDataObject>"#;
        assert_catalog_violation(vec![source_file(
            "Documents/Purchase.xml",
            root_without_uuid.as_bytes(),
        )]);

        let child_without_uuid = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Document uuid="56000000-0000-0000-0000-000000000001">
    <Properties><Name>Purchase</Name></Properties>
    <ChildObjects>
      <TabularSection><Properties><Name>Items</Name></Properties></TabularSection>
    </ChildObjects>
  </Document>
</MetaDataObject>"#;
        assert_catalog_violation(vec![source_file(
            "Documents/Purchase.xml",
            child_without_uuid.as_bytes(),
        )]);
    }

    #[test]
    fn metadata_nesting_over_architecture_depth_is_a_contract_violation() {
        let mut nested = String::new();
        for index in (1..=12).rev() {
            let children = if nested.is_empty() {
                String::new()
            } else {
                format!("<ChildObjects>{nested}</ChildObjects>")
            };
            nested = format!(
                "<TabularSection uuid=\"57000000-0000-0000-0000-{index:012}\"><Properties><Name>Level{index}</Name></Properties>{children}</TabularSection>"
            );
        }
        let xml = format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Document uuid=\"57000000-0000-0000-0000-000000000000\"><Properties><Name>Deep</Name></Properties><ChildObjects>{nested}</ChildObjects></Document></MetaDataObject>"
        );

        let outcome = PlatformXmlMetadataProvider.metadata(
            &query(100),
            &inventory(vec![source_file("Documents/Deep.xml", xml.as_bytes())]),
        );

        assert!(matches!(outcome, ProviderOutcome::ContractViolation(_)));
    }

    #[test]
    fn undeclared_subordinate_descriptor_invalidates_the_catalog_atomically() {
        let parent = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Report uuid="51000000-0000-0000-0000-000000000001">
    <Properties><Name>Sales</Name></Properties>
  </Report>
</MetaDataObject>"#;
        let subordinate = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Form uuid="51000000-0000-0000-0000-000000000002">
    <Properties><Name>Main</Name></Properties>
  </Form>
</MetaDataObject>"#;

        assert_catalog_violation(vec![
            source_file("Reports/Sales.xml", parent.as_bytes()),
            source_file("Reports/Sales/Forms/Main.xml", subordinate.as_bytes()),
        ]);
    }

    #[test]
    fn duplicate_artifact_or_uuid_identity_invalidates_the_catalog_atomically() {
        let duplicate_artifact = descriptor_xml(
            "Document",
            "Purchase",
            "52000000-0000-0000-0000-000000000001",
            "",
        );
        assert_catalog_violation(vec![
            source_file("Documents/First.xml", duplicate_artifact.as_bytes()),
            source_file("Documents/Second.xml", duplicate_artifact.as_bytes()),
        ]);

        let first = descriptor_xml(
            "Document",
            "Purchase",
            "52000000-0000-0000-0000-000000000002",
            "",
        );
        let second = descriptor_xml(
            "Report",
            "Sales",
            "52000000-0000-0000-0000-000000000002",
            "",
        );
        assert_catalog_violation(vec![
            source_file("Documents/Purchase.xml", first.as_bytes()),
            source_file("Reports/Sales.xml", second.as_bytes()),
        ]);
    }

    #[test]
    fn duplicate_children_and_semantic_sections_invalidate_the_catalog() {
        let duplicate_child = descriptor_xml(
            "Document",
            "Purchase",
            "53000000-0000-0000-0000-000000000001",
            "<ChildObjects><Form>Main</Form><Form>Main</Form></ChildObjects>",
        );
        assert_catalog_violation(vec![source_file(
            "Documents/Purchase.xml",
            duplicate_child.as_bytes(),
        )]);

        let duplicate_properties = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Document uuid="53000000-0000-0000-0000-000000000002">
    <Properties><Name>Purchase</Name></Properties>
    <Properties><Name>Other</Name></Properties>
  </Document>
</MetaDataObject>"#;
        assert_catalog_violation(vec![source_file(
            "Documents/Purchase.xml",
            duplicate_properties.as_bytes(),
        )]);
    }

    #[test]
    fn foreign_semantic_namespaces_and_invalid_identifiers_are_rejected() {
        let foreign_properties = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:evil="urn:evil">
  <Document uuid="54000000-0000-0000-0000-000000000001">
    <evil:Properties><evil:Name>Purchase</evil:Name></evil:Properties>
  </Document>
</MetaDataObject>"#;
        assert_catalog_violation(vec![source_file(
            "Documents/Purchase.xml",
            foreign_properties.as_bytes(),
        )]);

        for invalid in ["Bad Name", "Bad.Name", "9Bad", "Bad/Name", "Bad\nName"] {
            let xml = descriptor_xml(
                "Document",
                invalid,
                "54000000-0000-0000-0000-000000000002",
                "",
            );
            assert_catalog_violation(vec![source_file("Documents/Invalid.xml", xml.as_bytes())]);
        }
    }

    fn assert_catalog_violation(files: Vec<SourceFile>) {
        let outcome = PlatformXmlMetadataProvider.metadata(&query(100), &inventory(files));
        assert!(matches!(outcome, ProviderOutcome::ContractViolation(_)));
    }

    fn tracked_meta_compile_on_support_inventory() -> SourceInventory {
        inventory(vec![
            source_file(
                "Configuration.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Configuration.xml"
                )),
            ),
            source_file(
                "Catalogs/Locked.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Catalogs/Locked.xml"
                )),
            ),
            source_file(
                "Catalogs/Removed.xml",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Catalogs/Removed.xml"
                )),
            ),
            source_file(
                "Ext/ParentConfigurations.bin",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/",
                    "meta-compile/fixtures/on-support/Ext/ParentConfigurations.bin"
                )),
            ),
        ])
    }

    fn descriptor_xml(kind: &str, name: &str, uuid: &str, body: &str) -> String {
        format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><{kind} uuid=\"{uuid}\"><Properties><Name>{name}</Name></Properties>{body}</{kind}></MetaDataObject>"
        )
    }

    fn query(max_evidence: u16) -> DiscoveryQuery<'static> {
        DiscoveryQuery::new(
            "серии",
            &[],
            &[],
            &[],
            DiscoveryQueryLimits {
                max_files: 100,
                max_bytes: 1_000_000,
                max_evidence,
                max_candidates: 100,
                max_graph_depth: 12,
            },
        )
    }

    fn artifact(value: &str) -> ArtifactId {
        ArtifactId::parse(value).expect("valid test artifact")
    }

    fn source_file(path: &str, bytes: &[u8]) -> SourceFile {
        SourceFile {
            relative_path: PortableRelativePath::parse_str(path).expect("portable test path"),
            bytes: bytes.to_vec(),
            raw_hash: ContentHash::sha256(bytes),
        }
    }

    fn inventory(files: Vec<SourceFile>) -> SourceInventory {
        let bytes = files.iter().map(|file| file.bytes.len() as u64).sum();
        let count = files.len() as u32;
        SourceInventory {
            files,
            coverage: ProviderCoverage::new(count, count, bytes, count),
        }
    }
}
