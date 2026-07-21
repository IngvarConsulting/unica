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
pub(super) struct MetadataDescriptor {
    pub relative_path: PortableRelativePath,
    pub analyzed_file: AnalyzedFile,
    pub root: MetadataNode,
}

#[derive(Debug, Clone)]
pub(super) struct MetadataNode {
    pub artifact: ArtifactId,
    pub artifact_kind: ArtifactKind,
    pub name: String,
    pub object_uuid: Option<String>,
    pub location: EvidenceLocation,
    pub children: Vec<MetadataNode>,
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
        let descriptors = match parse_inventory_descriptors(files) {
            Ok(descriptors) => descriptors,
            Err(diagnostic) => return ProviderOutcome::ContractViolation(diagnostic),
        };
        let analyzed_files = descriptors
            .iter()
            .map(|descriptor| descriptor.analyzed_file.clone())
            .collect::<Vec<_>>();
        let mut records = descriptors
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

pub(super) fn parse_inventory_descriptors(
    inventory: &SourceInventory,
) -> Result<Vec<MetadataDescriptor>, ProviderDiagnostic> {
    let mut descriptors = Vec::new();
    for file in inventory
        .files
        .iter()
        .filter(|file| is_metadata_descriptor_candidate(&file.relative_path))
    {
        let descriptor = parse_descriptor(file).map_err(|message| {
            ProviderDiagnostic::material(
                "metadata_descriptor_malformed",
                format!(
                    "metadata descriptor {} is malformed: {message}",
                    file.relative_path.as_str()
                ),
            )
        })?;
        descriptors.push(descriptor);
    }
    descriptors.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(descriptors)
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

fn parse_descriptor(file: &SourceFile) -> Result<MetadataDescriptor, String> {
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
    let root = parse_metadata_node(&document, file, object, None)?;
    Ok(MetadataDescriptor {
        relative_path: file.relative_path.clone(),
        analyzed_file: file.analyzed_file(),
        root,
    })
}

pub(super) fn decode_xml_bytes(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes)
        .map(|text| text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("input is not UTF-8: {error}"))
}

fn parse_metadata_node(
    document: &Document<'_>,
    file: &SourceFile,
    node: Node<'_, '_>,
    container: Option<&ArtifactId>,
) -> Result<MetadataNode, String> {
    let object_kind = node.tag_name().name();
    if node.tag_name().namespace() != Some(METADATA_NAMESPACE) {
        return Err(format!(
            "{object_kind} object is outside the metadata namespace"
        ));
    }
    let name = metadata_node_name(node)
        .ok_or_else(|| format!("{object_kind} object has no non-empty Name"))?;
    let artifact_text = match container {
        Some(container) => format!("{}.{object_kind}.{name}", container.as_str()),
        None => format!("{object_kind}.{name}"),
    };
    let artifact = ArtifactId::parse(&artifact_text).map_err(|error| {
        format!("{object_kind} object has invalid canonical identity {artifact_text}: {error}")
    })?;
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
    if let Some(child_objects) = direct_child(node, "ChildObjects") {
        for child in child_objects.children().filter(Node::is_element) {
            children.push(parse_metadata_node(document, file, child, Some(&artifact))?);
        }
    }
    children.sort_by(|left, right| left.artifact.cmp(&right.artifact));
    Ok(MetadataNode {
        artifact,
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

fn metadata_node_name(node: Node<'_, '_>) -> Option<String> {
    direct_child(node, "Properties")
        .and_then(|properties| direct_child(properties, "Name"))
        .and_then(|name| name.text())
        .or_else(|| node.text())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn direct_child<'a, 'input>(node: Node<'a, 'input>, local_name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == local_name)
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
    let mut facts = vec![crate::domain::discovery::MetadataFact {
        artifact: node.artifact.clone(),
        artifact_kind: node.artifact_kind,
        container: container.cloned(),
        relation: StructuralRelationKind::Contains,
        location: node.location.clone(),
    }];
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
    descriptors: &[MetadataDescriptor],
) -> BTreeMap<PortableRelativePath, AnalyzedFile> {
    descriptors
        .iter()
        .map(|descriptor| {
            (
                descriptor.relative_path.clone(),
                descriptor.analyzed_file.clone(),
            )
        })
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
