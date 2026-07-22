use crate::application::discovery::ports::ManagedFormPort;
use crate::domain::discovery::{
    ArtifactId, ArtifactKind, DiscoveryQuery, FactBatch, FormBinding, FormFact,
    PortableRelativePath, ProviderDiagnostic, ProviderOutcome, SourceFile, SourceInventory,
};
use crate::infrastructure::discovery::metadata::{
    analyzed_file_map, build_batch, contributors_for_records, decode_xml_bytes,
    inventory_is_bounded, parse_inventory_catalog, validate_platform_identifier, xml_location,
    MetadataNode,
};
use roxmltree::{Document, Node};
use std::collections::{BTreeMap, BTreeSet};

const MANAGED_FORM_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/logform";

pub(crate) struct ManagedFormProvider;

impl ManagedFormPort for ManagedFormProvider {
    fn forms(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<FormFact>> {
        match collect_form_facts(query, files) {
            Ok(FormCollection::Complete(batch)) => ProviderOutcome::Complete(batch),
            Ok(FormCollection::Bounded { batch, diagnostic }) => ProviderOutcome::Bounded {
                data: batch,
                diagnostic,
            },
            Err(diagnostic) => ProviderOutcome::ContractViolation(diagnostic),
        }
    }
}

enum FormCollection {
    Complete(FactBatch<FormFact>),
    Bounded {
        batch: FactBatch<FormFact>,
        diagnostic: ProviderDiagnostic,
    },
}

fn collect_form_facts(
    query: &DiscoveryQuery<'_>,
    inventory: &SourceInventory,
) -> Result<FormCollection, ProviderDiagnostic> {
    let catalog = parse_inventory_catalog(inventory)?;
    let inventory_bounded = inventory_is_bounded(inventory);
    let mut analyzed_files = analyzed_file_map(&catalog);
    let inventory_files = inventory
        .files
        .iter()
        .map(|file| (file.relative_path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let mut declared_paths = BTreeSet::new();
    let mut records = Vec::new();

    for descriptor in catalog.nodes() {
        let Some(descriptor_path) = descriptor.definition_source() else {
            continue;
        };
        for form in descriptor.declared_forms() {
            let form_path = declared_form_path(descriptor_path, form)?;
            if !declared_paths.insert(form_path.clone()) {
                return Err(ProviderDiagnostic::material(
                    "managed_form_declaration_conflict",
                    format!(
                        "more than one metadata relationship declares {}",
                        form_path.as_str()
                    ),
                ));
            }
            let Some(source) = inventory_files.get(&form_path) else {
                if inventory_bounded {
                    continue;
                }
                return Err(ProviderDiagnostic::material(
                    "managed_form_declared_file_missing",
                    format!(
                        "declared managed form has no canonical source file {}",
                        form_path.as_str()
                    ),
                ));
            };
            analyzed_files.insert(form_path, source.analyzed_file());
            records.extend(parse_form(source, descriptor, form).map_err(|message| {
                ProviderDiagnostic::material(
                    "managed_form_malformed",
                    format!(
                        "managed form {} is malformed: {message}",
                        source.relative_path.as_str()
                    ),
                )
            })?);
        }
    }
    records.sort();
    if records.windows(2).any(|facts| facts[0] == facts[1]) {
        return Err(ProviderDiagnostic::material(
            "managed_form_duplicate_fact",
            "managed form input produced a duplicate typed binding",
        ));
    }
    let bounded = records.len() > usize::from(query.limits().max_evidence);
    if bounded {
        records.truncate(usize::from(query.limits().max_evidence));
    }
    let analyzed_files = analyzed_files.into_values().collect::<Vec<_>>();
    let contributors = contributors_for_records(&records, &analyzed_files);
    let batch = build_batch(records, analyzed_files, contributors)?;
    if bounded {
        Ok(FormCollection::Bounded {
            batch,
            diagnostic: ProviderDiagnostic::material(
                "managed_form_evidence_bound",
                "managed-form facts stopped at the maxEvidence limit",
            ),
        })
    } else if inventory_bounded {
        Ok(FormCollection::Bounded {
            batch,
            diagnostic: ProviderDiagnostic::material(
                "managed_form_inventory_bounded",
                "managed-form scope is incomplete because source inventory was truncated",
            ),
        })
    } else {
        Ok(FormCollection::Complete(batch))
    }
}

fn declared_form_path(
    descriptor_path: &PortableRelativePath,
    form: &MetadataNode,
) -> Result<PortableRelativePath, ProviderDiagnostic> {
    let descriptor_path = descriptor_path.as_str();
    let extension_start = descriptor_path.len().checked_sub(4).ok_or_else(|| {
        ProviderDiagnostic::material(
            "managed_form_descriptor_path",
            "metadata descriptor path is too short to end in .xml",
        )
    })?;
    if !descriptor_path[extension_start..].eq_ignore_ascii_case(".xml") {
        return Err(ProviderDiagnostic::material(
            "managed_form_descriptor_path",
            "metadata descriptor path does not end in .xml",
        ));
    }
    let path = format!(
        "{}/Forms/{}/Ext/Form.xml",
        &descriptor_path[..extension_start],
        form.name
    );
    PortableRelativePath::parse_str(&path).map_err(|error| {
        ProviderDiagnostic::material(
            "managed_form_canonical_path",
            format!("declared form has an invalid canonical path: {error}"),
        )
    })
}

fn parse_form(
    file: &SourceFile,
    descriptor: &MetadataNode,
    declared_form: &MetadataNode,
) -> Result<Vec<FormFact>, String> {
    let text = decode_xml_bytes(&file.bytes)?;
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if root.tag_name().name() != "Form"
        || root.tag_name().namespace() != Some(MANAGED_FORM_NAMESPACE)
    {
        return Err(format!(
            "expected Form root in {MANAGED_FORM_NAMESPACE}, got {} in {:?}",
            root.tag_name().name(),
            root.tag_name().namespace()
        ));
    }
    validate_form_semantic_namespaces(root)?;

    let mut records = Vec::new();
    for data_path in root
        .descendants()
        .filter(|node| is_active_form_element(*node, "DataPath"))
    {
        let Some(path) = data_path
            .text()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        validate_data_path(path)?;
        let Some(target) = descriptor.resolve_data_path(path) else {
            continue;
        };
        records.push(FormFact {
            form: declared_form.artifact.clone(),
            binding: FormBinding::Data {
                target: target.artifact.clone(),
                target_kind: target.artifact_kind,
                data_path: path.to_string(),
            },
            location: xml_location(&document, file, data_path)?,
        });
    }

    for event in root
        .descendants()
        .filter(|node| is_active_form_element(*node, "Event"))
    {
        let Some(events) = event.parent_element() else {
            continue;
        };
        if events.tag_name().name() != "Events" {
            continue;
        }
        let event_name = required_identifier_text(event.attribute("name"), "event name")?;
        let handler = required_identifier_text(event.text(), "event handler")?;
        records.push(FormFact {
            form: declared_form.artifact.clone(),
            binding: FormBinding::Event {
                event: event_name.to_string(),
                handler: handler.to_string(),
                target: handler_artifact(&declared_form.artifact, handler)?,
                target_kind: ArtifactKind::Method,
            },
            location: xml_location(&document, file, event)?,
        });
    }

    for command in root
        .descendants()
        .filter(|node| is_active_form_element(*node, "Command"))
    {
        let Some(commands) = command.parent_element() else {
            continue;
        };
        if commands.tag_name().name() != "Commands" {
            continue;
        }
        let command_name = required_identifier_text(command.attribute("name"), "command name")?;
        for action in command
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "Action")
        {
            let handler = required_identifier_text(action.text(), "command action")?;
            records.push(FormFact {
                form: declared_form.artifact.clone(),
                binding: FormBinding::Command {
                    command: command_name.to_string(),
                    handler: handler.to_string(),
                    target: handler_artifact(&declared_form.artifact, handler)?,
                    target_kind: ArtifactKind::Method,
                },
                location: xml_location(&document, file, action)?,
            });
        }
    }
    Ok(records)
}

fn is_active_form_element(node: Node<'_, '_>, local_name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == local_name
        && !node
            .ancestors()
            .any(|ancestor| ancestor.is_element() && ancestor.tag_name().name() == "BaseForm")
}

fn validate_form_semantic_namespaces(root: Node<'_, '_>) -> Result<(), String> {
    const SEMANTIC_NAMES: &[&str] = &[
        "BaseForm", "DataPath", "Events", "Event", "Commands", "Command", "Action",
    ];
    for node in root.descendants().filter(Node::is_element) {
        if SEMANTIC_NAMES.contains(&node.tag_name().name())
            && node.tag_name().namespace() != Some(MANAGED_FORM_NAMESPACE)
        {
            return Err(format!(
                "{} is outside the managed-form namespace",
                node.tag_name().name()
            ));
        }
    }
    Ok(())
}

fn validate_data_path(path: &str) -> Result<(), String> {
    let mut found = false;
    for segment in path.split('.') {
        let segment = segment.trim();
        validate_platform_identifier(segment)
            .map_err(|message| format!("DataPath segment {segment:?} is invalid: {message}"))?;
        found = true;
    }
    if !found {
        return Err("DataPath must contain at least one identifier".to_string());
    }
    Ok(())
}

fn required_binding_text<'a>(value: Option<&'a str>, label: &str) -> Result<&'a str, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{label} must not be empty"))
}

fn required_identifier_text<'a>(value: Option<&'a str>, label: &str) -> Result<&'a str, String> {
    let value = required_binding_text(value, label)?;
    validate_platform_identifier(value)
        .map_err(|message| format!("{label} {value:?} is invalid: {message}"))?;
    Ok(value)
}

fn handler_artifact(form: &ArtifactId, handler: &str) -> Result<ArtifactId, String> {
    ArtifactId::parse(&format!(
        "{}.Module.FormModule.Method.{handler}",
        form.as_str()
    ))
    .map_err(|error| format!("handler {handler} has invalid canonical identity: {error}"))
}

#[cfg(test)]
mod tests {
    use super::ManagedFormProvider;
    use crate::application::discovery::ports::ManagedFormPort;
    use crate::domain::discovery::{
        ArtifactId, ArtifactKind, ContentHash, DiscoveryQuery, DiscoveryQueryLimits, FormBinding,
        PortableRelativePath, ProviderCoverage, ProviderOutcome, SourceFile, SourceInventory,
    };

    const DESCRIPTOR: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Document uuid="30000000-0000-0000-0000-000000000001">
    <Properties><Name>ПриобретениеТоваровУслуг</Name></Properties>
    <ChildObjects>
      <TabularSection uuid="30000000-0000-0000-0000-000000000002">
        <Properties><Name>Товары</Name></Properties>
        <ChildObjects>
          <Attribute uuid="30000000-0000-0000-0000-000000000003">
            <Properties><Name>Серия</Name></Properties>
          </Attribute>
        </ChildObjects>
      </TabularSection>
      <Form>ФормаДокумента</Form>
    </ChildObjects>
  </Document>
</MetaDataObject>"#;

    const FORM_XML: &str = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform">
  <Events>
    <Event name="OnOpen">ПроверитьСрокГодности</Event>
  </Events>
  <ChildItems>
    <InputField name="СерияТовара" id="1">
      <DataPath>Объект.Товары.Серия</DataPath>
    </InputField>
  </ChildItems>
  <Commands>
    <Command name="Проверить" id="1">
      <Action callType="Client">ПроверитьСрокГодности</Action>
    </Command>
  </Commands>
</Form>"#;

    const FORM_DESCRIPTOR_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
  <Form uuid="30000000-0000-0000-0000-000000000004">
    <Properties><Name>ФормаДокумента</Name></Properties>
  </Form>
</MetaDataObject>"#;

    #[test]
    fn enumerates_only_declared_canonical_forms_and_keeps_typed_bindings() {
        let canonical_form_path = concat!(
            "Documents/ПриобретениеТоваровУслуг/Forms/",
            "ФормаДокумента/Ext/Form.xml"
        );
        let inventory = inventory(vec![
            source_file(
                "Documents/ПриобретениеТоваровУслуг.xml",
                DESCRIPTOR.as_bytes(),
            ),
            metadata_form_descriptor_file(),
            source_file(canonical_form_path, FORM_XML.as_bytes()),
            source_file(
                "Decoy/Forms/ФормаДокумента/Ext/Form.xml",
                FORM_XML.as_bytes(),
            ),
        ]);

        let outcome = ManagedFormProvider.forms(&query(100), &inventory);

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("expected complete managed-form facts");
        };
        let form = artifact("Document.ПриобретениеТоваровУслуг.Form.ФормаДокумента");
        let series =
            artifact("Document.ПриобретениеТоваровУслуг.TabularSection.Товары.Attribute.Серия");
        assert!(batch.records.iter().any(|fact| {
            fact.form == form
                && matches!(
                    &fact.binding,
                    FormBinding::Data {
                        target,
                        target_kind: ArtifactKind::Attribute,
                        data_path,
                    } if target.as_str() == series.as_str() && data_path == "Объект.Товары.Серия"
                )
                && fact.location.line == Some(7)
        }));
        assert!(batch.records.iter().any(|fact| matches!(
            &fact.binding,
            FormBinding::Command { command, handler, target_kind: ArtifactKind::Method, .. }
                if command == "Проверить" && handler == "ПроверитьСрокГодности"
        )));
        assert!(batch.records.iter().any(|fact| matches!(
            &fact.binding,
            FormBinding::Event { event, handler, target_kind: ArtifactKind::Method, .. }
                if event == "OnOpen" && handler == "ПроверитьСрокГодности"
        )));
        assert_eq!(batch.records.len(), 3);
        assert_eq!(batch.analyzed_files.len(), 3);
        assert_eq!(batch.contributors.len(), 1);
        assert_eq!(
            batch.contributors[0].relative_path.as_str(),
            canonical_form_path
        );
        assert_eq!(
            batch.coverage,
            ProviderCoverage::new(
                3,
                3,
                (DESCRIPTOR.len() + FORM_DESCRIPTOR_XML.len() + FORM_XML.len()) as u64,
                3,
            )
        );
    }

    #[test]
    fn malformed_declared_form_is_an_atomic_contract_violation() {
        let canonical_form_path = concat!(
            "Documents/ПриобретениеТоваровУслуг/Forms/",
            "ФормаДокумента/Ext/Form.xml"
        );
        let inventory = inventory(vec![
            source_file(
                "Documents/ПриобретениеТоваровУслуг.xml",
                DESCRIPTOR.as_bytes(),
            ),
            metadata_form_descriptor_file(),
            source_file(canonical_form_path, b"<Form><Events>"),
        ]);

        let outcome = ManagedFormProvider.forms(&query(100), &inventory);

        let ProviderOutcome::ContractViolation(diagnostic) = outcome else {
            panic!("malformed declared form must invalidate the whole provider");
        };
        assert_eq!(diagnostic.code, "managed_form_malformed");
    }

    #[test]
    fn truncated_inventory_does_not_turn_a_missing_declared_form_into_a_violation() {
        let mut inventory = inventory(vec![source_file(
            "Documents/ПриобретениеТоваровУслуг.xml",
            DESCRIPTOR.as_bytes(),
        )]);
        inventory.coverage.files_seen += 1;

        let outcome = ManagedFormProvider.forms(&query(100), &inventory);

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("truncated inventory must keep managed-form discovery bounded");
        };
        assert!(data.records.is_empty());
        assert_eq!(diagnostic.code, "managed_form_inventory_bounded");
    }

    #[test]
    fn tracked_configuration_catalog_inventory_is_a_complete_empty_form_scope() {
        let outcome =
            ManagedFormProvider.forms(&query(100), &tracked_meta_compile_on_support_inventory());

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("tracked catalog-only configuration must not violate form discovery");
        };
        assert!(batch.records.is_empty());
        assert_eq!(batch.analyzed_files.len(), 3);
        assert!(batch.contributors.is_empty());
    }

    #[test]
    fn foreign_namespace_cannot_inject_or_suppress_form_semantics() {
        let canonical_form_path = concat!(
            "Documents/ПриобретениеТоваровУслуг/Forms/",
            "ФормаДокумента/Ext/Form.xml"
        );
        let injected = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:evil="urn:evil">
  <evil:Events><evil:Event name="OnOpen">Injected</evil:Event></evil:Events>
</Form>"#;
        let suppressed = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:evil="urn:evil">
  <evil:BaseForm><Events><Event name="OnOpen">Suppressed</Event></Events></evil:BaseForm>
</Form>"#;

        for xml in [injected, suppressed] {
            let outcome = ManagedFormProvider.forms(
                &query(100),
                &inventory(vec![
                    source_file(
                        "Documents/ПриобретениеТоваровУслуг.xml",
                        DESCRIPTOR.as_bytes(),
                    ),
                    metadata_form_descriptor_file(),
                    source_file(canonical_form_path, xml.as_bytes()),
                ]),
            );
            assert!(matches!(outcome, ProviderOutcome::ContractViolation(_)));
        }
    }

    #[test]
    fn invalid_event_command_and_handler_identifiers_are_rejected() {
        let canonical_form_path = concat!(
            "Documents/ПриобретениеТоваровУслуг/Forms/",
            "ФормаДокумента/Ext/Form.xml"
        );
        let cases = [
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Events><Event name="Bad Event">Handler</Event></Events></Form>"#,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Events><Event name="OnOpen">Bad.Handler</Event></Events></Form>"#,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Commands><Command name="9Bad"><Action>Handler</Action></Command></Commands></Form>"#,
        ];

        for xml in cases {
            let outcome = ManagedFormProvider.forms(
                &query(100),
                &inventory(vec![
                    source_file(
                        "Documents/ПриобретениеТоваровУслуг.xml",
                        DESCRIPTOR.as_bytes(),
                    ),
                    metadata_form_descriptor_file(),
                    source_file(canonical_form_path, xml.as_bytes()),
                ]),
            );
            assert!(matches!(outcome, ProviderOutcome::ContractViolation(_)));
        }
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

    fn metadata_form_descriptor_file() -> SourceFile {
        source_file(
            "Documents/ПриобретениеТоваровУслуг/Forms/ФормаДокумента.xml",
            FORM_DESCRIPTOR_XML.as_bytes(),
        )
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

    fn inventory(files: Vec<SourceFile>) -> SourceInventory {
        let bytes = files.iter().map(|file| file.bytes.len() as u64).sum();
        let count = files.len() as u32;
        SourceInventory {
            files,
            coverage: ProviderCoverage::new(count, count, bytes, count),
        }
    }
}
