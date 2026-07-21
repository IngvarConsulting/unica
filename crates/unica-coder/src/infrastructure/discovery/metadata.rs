use crate::application::discovery::ports::MetadataCatalogPort;
use crate::domain::discovery::{
    AnalyzedFile, ArtifactId, ArtifactKind, DiscoveryQuery, EvidenceLocation, FactBatch,
    PortableRelativePath, ProviderCoverage, ProviderDiagnostic, ProviderOutcome, SourceFile,
    SourceInventory, StructuralRelationKind,
};
use roxmltree::{Document, Node};
use std::collections::{BTreeMap, BTreeSet};

const METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";

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
}

impl MetadataCatalog {
    pub(super) fn descriptors(&self) -> &[MetadataDescriptor] {
        &self.descriptors
    }

    pub(super) fn nodes(&self) -> Vec<&MetadataNode> {
        let mut nodes = Vec::new();
        for descriptor in &self.descriptors {
            collect_metadata_nodes(&descriptor.root, &mut nodes);
        }
        nodes
    }
}

impl MetadataDescriptor {
    pub(super) fn declared_forms(&self) -> impl Iterator<Item = &MetadataNode> {
        self.root
            .children
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
        resolve_child_path(&self.root.children, &names)
    }
}

impl MetadataNode {
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
    let (name, remaining) = names.split_first()?;
    let normalized = crate::domain::discovery::normalize_discovery_identity(name);
    let child = children.iter().find(|child| {
        crate::domain::discovery::normalize_discovery_identity(&child.name) == normalized
    })?;
    if remaining.is_empty() {
        Some(child)
    } else {
        resolve_child_path(&child.children, remaining)
    }
}

impl MetadataCatalogPort for PlatformXmlMetadataProvider {
    fn metadata(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<crate::domain::discovery::MetadataFact>> {
        let catalog = match parse_inventory_catalog(files) {
            Ok(catalog) => catalog,
            Err(diagnostic) => return ProviderOutcome::ContractViolation(diagnostic),
        };
        let analyzed_files = catalog.analyzed_files.clone();
        let mut records = catalog
            .descriptors
            .iter()
            .flat_map(metadata_facts)
            .collect::<Vec<_>>();
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
    inventory: &SourceInventory,
) -> Result<MetadataCatalog, ProviderDiagnostic> {
    let mut raw_descriptors = Vec::new();
    for file in inventory
        .files
        .iter()
        .filter(|file| is_metadata_descriptor_candidate(&file.relative_path))
    {
        let descriptor = parse_raw_descriptor(file).map_err(|message| {
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
    raw_descriptors.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    build_catalog(raw_descriptors)
        .map_err(|message| ProviderDiagnostic::material("metadata_catalog_invalid", message))
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

fn parse_raw_descriptor(file: &SourceFile) -> Result<RawMetadataDescriptor, String> {
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
    let root = parse_raw_metadata_node(&document, file, object)?;
    Ok(RawMetadataDescriptor {
        relative_path: file.relative_path.clone(),
        analyzed_file: file.analyzed_file(),
        root,
    })
}

fn build_catalog(raw_descriptors: Vec<RawMetadataDescriptor>) -> Result<MetadataCatalog, String> {
    let mut analyzed_files = raw_descriptors
        .iter()
        .map(|descriptor| descriptor.analyzed_file.clone())
        .collect::<Vec<_>>();
    analyzed_files.sort();
    let mut descriptors = Vec::new();
    let mut subordinate_forms = Vec::new();
    for raw in raw_descriptors {
        if subordinate_form_parent_path(&raw.relative_path)?.is_some() {
            subordinate_forms.push(raw);
            continue;
        }
        let root = materialize_metadata_node(raw.root, None)?;
        descriptors.push(MetadataDescriptor {
            relative_path: raw.relative_path,
            root,
        });
    }
    descriptors.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    validate_catalog_nodes(&descriptors)?;

    for subordinate in subordinate_forms {
        attach_subordinate_form(&mut descriptors, subordinate)?;
    }
    validate_catalog_nodes(&descriptors)?;
    Ok(MetadataCatalog {
        descriptors,
        analyzed_files,
    })
}

fn subordinate_form_parent_path(
    path: &PortableRelativePath,
) -> Result<Option<PortableRelativePath>, String> {
    let components = path.as_str().split('/').collect::<Vec<_>>();
    if components.len() < 3 || !components[components.len() - 2].eq_ignore_ascii_case("Forms") {
        return Ok(None);
    }
    let Some((_stem, extension)) = components
        .last()
        .and_then(|file_name| file_name.rsplit_once('.'))
    else {
        return Err("subordinate form descriptor has no .xml extension".to_string());
    };
    if !extension.eq_ignore_ascii_case("xml") {
        return Err("subordinate form descriptor does not end in .xml".to_string());
    }
    let parent_base = components[..components.len() - 2].join("/");
    PortableRelativePath::parse_str(&format!("{parent_base}.xml"))
        .map(Some)
        .map_err(|error| format!("subordinate form parent path is invalid: {error}"))
}

fn materialize_metadata_node(
    raw: RawMetadataNode,
    container: Option<&ArtifactId>,
) -> Result<MetadataNode, String> {
    let artifact_text = match container {
        Some(container) => format!("{}.{}.{}", container.as_str(), raw.xml_kind, raw.name),
        None => format!("{}.{}", raw.xml_kind, raw.name),
    };
    let artifact = ArtifactId::parse(&artifact_text).map_err(|error| {
        format!(
            "{} object has invalid canonical identity {artifact_text}: {error}",
            raw.xml_kind
        )
    })?;
    let mut children = raw
        .children
        .into_iter()
        .map(|child| materialize_metadata_node(child, Some(&artifact)))
        .collect::<Result<Vec<_>, _>>()?;
    children.sort_by(|left, right| left.artifact.cmp(&right.artifact));
    Ok(MetadataNode {
        artifact,
        artifact_kind: raw.artifact_kind,
        name: raw.name,
        object_uuid: raw.object_uuid,
        locations: vec![raw.location],
        children,
    })
}

fn attach_subordinate_form(
    descriptors: &mut [MetadataDescriptor],
    subordinate: RawMetadataDescriptor,
) -> Result<(), String> {
    if subordinate.root.xml_kind != "Form" || subordinate.root.artifact_kind != ArtifactKind::Form {
        return Err(format!(
            "subordinate form descriptor {} contains {} instead of Form",
            subordinate.relative_path.as_str(),
            subordinate.root.xml_kind
        ));
    }
    let parent_path = subordinate_form_parent_path(&subordinate.relative_path)?
        .ok_or_else(|| "subordinate form path classification diverged".to_string())?;
    let descriptor = descriptors
        .iter_mut()
        .find(|descriptor| descriptor.relative_path == parent_path)
        .ok_or_else(|| {
            format!(
                "subordinate form {} has no parent descriptor {}",
                subordinate.relative_path.as_str(),
                parent_path.as_str()
            )
        })?;
    let artifact = ArtifactId::parse(&format!(
        "{}.Form.{}",
        descriptor.root.artifact.as_str(),
        subordinate.root.name
    ))
    .map_err(|error| format!("subordinate form identity is invalid: {error}"))?;
    let declared = descriptor
        .root
        .direct_child_mut(&artifact)
        .filter(|child| child.artifact_kind == ArtifactKind::Form)
        .ok_or_else(|| {
            format!(
                "subordinate form {} is not declared by {}",
                subordinate.relative_path.as_str(),
                parent_path.as_str()
            )
        })?;
    let descriptor_base = descriptor
        .relative_path
        .as_str()
        .strip_suffix(".xml")
        .ok_or_else(|| "parent descriptor does not end in canonical .xml".to_string())?;
    let expected_path =
        PortableRelativePath::parse_str(&format!("{descriptor_base}/Forms/{}.xml", declared.name))
            .map_err(|error| format!("declared subordinate form path is invalid: {error}"))?;
    if subordinate.relative_path != expected_path {
        return Err(format!(
            "subordinate form path {} conflicts with declared canonical path {}",
            subordinate.relative_path.as_str(),
            expected_path.as_str()
        ));
    }
    match (&declared.object_uuid, &subordinate.root.object_uuid) {
        (Some(declared_uuid), Some(descriptor_uuid)) if declared_uuid != descriptor_uuid => {
            return Err(format!(
                "subordinate form {} conflicts with its declared uuid",
                artifact.as_str()
            ));
        }
        (None, Some(descriptor_uuid)) => declared.object_uuid = Some(descriptor_uuid.clone()),
        (Some(_), Some(_)) | (Some(_), None) | (None, None) => {}
    }
    if declared.locations.contains(&subordinate.root.location) {
        return Err(format!(
            "subordinate form {} repeats an existing evidence location",
            artifact.as_str()
        ));
    }
    declared.locations.push(subordinate.root.location);
    let mut subordinate_children = subordinate
        .root
        .children
        .into_iter()
        .map(|child| materialize_metadata_node(child, Some(&artifact)))
        .collect::<Result<Vec<_>, _>>()?;
    declared.children.append(&mut subordinate_children);
    declared
        .children
        .sort_by(|left, right| left.artifact.cmp(&right.artifact));
    Ok(())
}

fn validate_catalog_nodes(descriptors: &[MetadataDescriptor]) -> Result<(), String> {
    let mut artifacts = BTreeMap::new();
    let mut uuids = BTreeMap::new();
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
        }
    }
    Ok(())
}

fn collect_metadata_nodes<'a>(node: &'a MetadataNode, output: &mut Vec<&'a MetadataNode>) {
    output.push(node);
    for child in &node.children {
        collect_metadata_nodes(child, output);
    }
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
) -> Result<RawMetadataNode, String> {
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
    let mut children = Vec::new();
    if let Some(child_objects) = semantic_child(node, "ChildObjects")? {
        for child in child_objects.children().filter(Node::is_element) {
            children.push(parse_raw_metadata_node(document, file, child)?);
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
    })
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
) -> impl Iterator<Item = crate::domain::discovery::MetadataFact> + '_ {
    flatten_metadata_node(&descriptor.root, None).into_iter()
}

fn flatten_metadata_node(
    node: &MetadataNode,
    container: Option<&ArtifactId>,
) -> Vec<crate::domain::discovery::MetadataFact> {
    let mut facts = node
        .locations
        .iter()
        .map(|location| crate::domain::discovery::MetadataFact {
            artifact: node.artifact.clone(),
            artifact_kind: node.artifact_kind,
            container: container.cloned(),
            relation: StructuralRelationKind::Contains,
            location: location.clone(),
        })
        .collect::<Vec<_>>();
    for child in &node.children {
        facts.extend(flatten_metadata_node(child, Some(&node.artifact)));
    }
    facts
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

    const PROCESSOR_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <DataProcessor uuid="20000000-0000-0000-0000-000000000001">
    <Properties><Name>ПодборСерийВДокументы</Name></Properties>
    <ChildObjects>
      <Form>РегистрацияИПодборСерийПоОднойСтрокеТоваров</Form>
    </ChildObjects>
  </DataProcessor>
</MetaDataObject>"#;

    #[test]
    fn parses_actual_root_identity_recursive_children_and_declared_form_relationships() {
        let inventory = inventory(vec![
            source_file(
                "DataProcessors/ПодборСерийВДокументы.xml",
                PROCESSOR_XML.as_bytes(),
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
                && fact.relation == StructuralRelationKind::Contains
                && fact.location.line == Some(13)
                && fact.location.xml_path.as_deref()
                    == Some("/MetaDataObject/Document/ChildObjects/TabularSection[2]")
        }));
        assert!(batch
            .records
            .iter()
            .any(|fact| fact.artifact == goods_series));
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
        assert_eq!(batch.analyzed_files.len(), 2);
        assert_eq!(batch.contributors, batch.analyzed_files);
        assert_eq!(
            batch.coverage,
            ProviderCoverage::new(2, 2, (DOCUMENT_XML.len() + PROCESSOR_XML.len()) as u64, 7)
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
