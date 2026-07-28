use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    commands::*,
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationTarget,
        PropertySelection,
    },
    ports::{
        CaptureResult, FormatReadRequest, OperationCancellation, PublicationCancellation,
        PublicationRollback, WriterRequest,
    },
    source::{SourceContext, SourceFamily, SourceLocation},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scenario {
    Success,
    DryRun,
    Idempotent,
    Denied,
    Cancelled,
    Concurrent,
}

impl Scenario {
    const ALL: [Self; 6] = [
        Self::Success,
        Self::DryRun,
        Self::Idempotent,
        Self::Denied,
        Self::Cancelled,
        Self::Concurrent,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
enum SemanticFact {
    Envelope {
        status: String,
        root: Option<(String, String)>,
        consistency: Option<String>,
    },
    Diagnostic {
        code: String,
        message: String,
        details: Option<String>,
    },
    Identity {
        kind: String,
        name: String,
        reference_kind: String,
        reference_name: String,
        capability_state: String,
        capability: String,
        action_profile: String,
        facet_visibility: String,
    },
    Property {
        owner_kind: String,
        owner_name: String,
        id: String,
        value_type: String,
        value_state: String,
        value: Option<String>,
        provenance: String,
        capability: String,
    },
    Facets {
        owner_kind: String,
        owner_name: String,
        members: String,
    },
    Actions {
        owner_kind: String,
        owner_name: String,
        semantic: String,
        available: String,
    },
    Relation {
        value: String,
    },
    StandaloneStructure {
        family: &'static str,
        document: usize,
        ordinal_path: String,
        semantic_kind: String,
        value: Option<String>,
        attributes: Vec<(String, String)>,
    },
    StandaloneText {
        family: &'static str,
        role: String,
        content: String,
    },
}

type SemanticFacts = BTreeMap<SemanticFact, usize>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExpectedFact {
    Diagnostic {
        code: &'static str,
        message: &'static str,
        details: Option<&'static str>,
        present: bool,
    },
    Identity {
        kind: &'static str,
        name: &'static str,
        present: bool,
    },
    Property {
        owner: &'static str,
        id: &'static str,
        value_type: &'static str,
        value_state: &'static str,
        value: &'static str,
        present: bool,
    },
    Structure {
        family: &'static str,
        semantic_kind: &'static str,
        value: Option<&'static str>,
        present: bool,
    },
    StandaloneScalar {
        family: &'static str,
        value: &'static str,
        present: bool,
    },
    StandaloneText {
        family: &'static str,
        role: &'static str,
        content: &'static str,
        present: bool,
    },
}

#[derive(Clone)]
struct MatrixCase {
    kind: WriterCommandKind,
    command: WriterCommand,
    before: Vec<ExpectedFact>,
    after: Vec<ExpectedFact>,
    before_fact_digest: &'static str,
    after_fact_digest: &'static str,
}

#[test]
fn every_writer_variant_preserves_independent_semantics_in_every_required_scenario() {
    let mut covered = BTreeSet::new();
    for case in matrix_cases() {
        for scenario in Scenario::ALL {
            let root = fixture_root(case.kind, scenario);
            fs::create_dir_all(&root).unwrap();
            prepare_fixture(case.kind, &root);
            exercise(&case, scenario, &root);
            covered.insert((case.kind, scenario));
            fs::remove_dir_all(root).unwrap();
        }
    }

    let expected = WriterCommandKind::ALL
        .into_iter()
        .flat_map(|kind| {
            Scenario::ALL
                .into_iter()
                .map(move |scenario| (kind, scenario))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(covered, expected);
    assert_eq!(covered.len(), 25 * 6);
}

fn exercise(case: &MatrixCase, scenario: Scenario, root: &Path) {
    let before = observe(case.kind, root);
    assert_eq!(
        fact_set_digest(&before),
        case.before_fact_digest,
        "{:?} {scenario:?}: complete initial semantic fact multiset changed: {before:#?}",
        case.kind,
    );
    assert_expected_facts(&before, &case.before, case.kind, scenario, "before");

    match scenario {
        Scenario::Success => {
            let result = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Applied),
                "{:?}: {result:?}",
                case.kind
            );
            let after = observe(case.kind, root);
            assert_eq!(
                fact_set_digest(&after),
                case.after_fact_digest,
                "{:?}: complete resulting semantic fact multiset differs from the frozen oracle: \
                 {after:#?}",
                case.kind,
            );
            assert_expected_facts(&after, &case.after, case.kind, scenario, "after");
            assert_declared_delta(&before, &after, case, scenario);
        }
        Scenario::DryRun => {
            let result = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Preview,
                OperationCancellation::new(),
            );
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Previewed),
                "{:?}: {result:?}",
                case.kind
            );
            assert_eq!(observe(case.kind, root), before);
        }
        Scenario::Idempotent => {
            let first = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert!(
                matches!(first.lifecycle(), WriterLifecycle::Applied),
                "{:?}: {first:?}",
                case.kind
            );
            let once = observe(case.kind, root);
            assert_eq!(
                fact_set_digest(&once),
                case.after_fact_digest,
                "{:?}: first idempotent apply differs from the frozen oracle",
                case.kind
            );
            assert_expected_facts(&once, &case.after, case.kind, scenario, "after");
            assert_declared_delta(&before, &once, case, scenario);
            let _repeat = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert_eq!(
                observe(case.kind, root),
                once,
                "{:?}: repeat changed semantic facts",
                case.kind
            );
        }
        Scenario::Denied => {
            make_semantically_unsupported(case.kind, root);
            let denied_before = observe(case.kind, root);
            let result = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Rejected(_)),
                "{:?}: {result:?}",
                case.kind
            );
            assert_eq!(
                observe(case.kind, root),
                denied_before,
                "{:?}: denied operation changed unsupported semantic facts",
                case.kind
            );
        }
        Scenario::Cancelled => {
            let cancellation = OperationCancellation::new();
            let cancellation_for_checkpoint = cancellation.clone();
            #[cfg(feature = "test-support")]
            let result = PlatformXmlAdapterFactory::new().with_publication_mutation_checkpoint(
                move || cancellation_for_checkpoint.cancel(),
                || {
                    execute(
                        case.command.clone(),
                        sources(case.kind, root),
                        root,
                        MutationMode::Apply,
                        cancellation,
                    )
                },
            );
            #[cfg(not(feature = "test-support"))]
            let result = {
                cancellation_for_checkpoint.cancel();
                execute(
                    case.command.clone(),
                    sources(case.kind, root),
                    root,
                    MutationMode::Apply,
                    cancellation,
                )
            };
            assert!(
                matches!(
                    result.lifecycle(),
                    WriterLifecycle::Cancelled(interruption)
                        if cfg!(not(feature = "test-support"))
                            || (interruption.cancellation()
                                == PublicationCancellation::DuringPublication
                                && interruption.rollback() == PublicationRollback::Performed)
                ),
                "{:?}: {result:?}",
                case.kind
            );
            assert_eq!(observe(case.kind, root), before);
        }
        Scenario::Concurrent => {
            let root = Arc::new(root.to_path_buf());
            let mut workers = Vec::new();
            for _ in 0..2 {
                let command = case.command.clone();
                let root = Arc::clone(&root);
                let kind = case.kind;
                workers.push(thread::spawn(move || {
                    execute(
                        command,
                        sources(kind, &root),
                        &root,
                        MutationMode::Apply,
                        OperationCancellation::new(),
                    )
                }));
            }
            let results = workers
                .into_iter()
                .map(|worker| worker.join().expect("concurrent writer must not panic"))
                .collect::<Vec<_>>();
            assert!(
                results.iter().all(|result| matches!(
                    result.lifecycle(),
                    WriterLifecycle::Applied | WriterLifecycle::Rejected(_)
                )),
                "{:?}: {results:?}",
                case.kind
            );
            assert!(
                results
                    .iter()
                    .any(|result| matches!(result.lifecycle(), WriterLifecycle::Applied)),
                "{:?}: no concurrent writer completed: {results:?}",
                case.kind
            );
            let after = observe(case.kind, &root);
            assert_eq!(
                fact_set_digest(&after),
                case.after_fact_digest,
                "{:?}: concurrent result differs from the frozen oracle",
                case.kind
            );
            assert_expected_facts(&after, &case.after, case.kind, scenario, "after");
            assert_declared_delta(&before, &after, case, scenario);
        }
    }
}

fn fact_set_digest(facts: &SemanticFacts) -> String {
    let mut encoded_facts = facts
        .iter()
        .map(|(fact, count)| {
            let mut normalized =
                serde_json::to_value((fact, count)).expect("semantic fact tuple must serialize");
            normalize_volatile_identities(&mut normalized);
            serde_json::to_vec(&normalized).expect("normalized semantic fact tuple must serialize")
        })
        .collect::<Vec<_>>();
    encoded_facts.sort();
    let mut digest = Sha256::new();
    for encoded in encoded_facts {
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded);
    }
    format!("{:x}", digest.finalize())
}

fn normalize_volatile_identities(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            *text = replace_opaque_hash_literals(&replace_uuid_literals(text))
        }
        serde_json::Value::Array(values) => {
            for value in values {
                normalize_volatile_identities(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                normalize_volatile_identities(value);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn replace_opaque_hash_literals(value: &str) -> String {
    const PREFIX: &str = "sha256:";
    const HASH_LEN: usize = 64;
    let mut normalized = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative) = value[cursor..].find(PREFIX) {
        let prefix_start = cursor + relative;
        let hash_start = prefix_start + PREFIX.len();
        let hash_end = hash_start + HASH_LEN;
        if hash_end <= value.len()
            && value[hash_start..hash_end]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            normalized.push_str(&value[cursor..hash_start]);
            normalized.push_str("<semantic-hash>");
            cursor = hash_end;
        } else {
            normalized.push_str(&value[cursor..hash_start]);
            cursor = hash_start;
        }
    }
    normalized.push_str(&value[cursor..]);
    normalized
}

fn replace_uuid_literals(value: &str) -> String {
    const UUID_LEN: usize = 36;
    let mut normalized = String::with_capacity(value.len());
    let mut copied_until = 0;
    let mut cursor = 0;
    while cursor + UUID_LEN <= value.len() {
        if value.is_char_boundary(cursor)
            && value.is_char_boundary(cursor + UUID_LEN)
            && uuid::Uuid::parse_str(&value[cursor..cursor + UUID_LEN]).is_ok()
        {
            normalized.push_str(&value[copied_until..cursor]);
            normalized.push_str("<semantic-id>");
            cursor += UUID_LEN;
            copied_until = cursor;
        } else {
            cursor += value[cursor..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
        }
    }
    normalized.push_str(&value[copied_until..]);
    normalized
}

fn execute(
    command: WriterCommand,
    sources: Vec<(WriterSourceRole, PathBuf)>,
    workspace_root: &Path,
    mode: MutationMode,
    cancellation: OperationCancellation,
) -> WriterResult {
    let session = PlatformXmlAdapterFactory::new()
        .capture_writer_session_with_extension_emitter(
            sources,
            workspace_root,
            workspace_root,
            &workspace_root.join(".cache"),
            0,
            |_plan, existing| {
                let mut body = existing.unwrap_or_default().to_vec();
                if body
                    .windows(b"// MATRIX_PATCH".len())
                    .any(|window| window == b"// MATRIX_PATCH")
                {
                    return Ok(body);
                }
                if !body.ends_with(b"\n") && !body.is_empty() {
                    body.push(b'\n');
                }
                body.extend_from_slice(b"// MATRIX_PATCH\n");
                Ok(body)
            },
        )
        .unwrap();
    let request = WriterRequest::new(session, command, mode, cancellation);
    PlatformXmlAdapterFactory::new()
        .operational_registration()
        .writer()
        .execute(&request)
        .unwrap()
}

fn observe(kind: WriterCommandKind, root: &Path) -> SemanticFacts {
    let mut facts = SemanticFacts::new();
    if let Some((source_root, target)) = production_target(kind, root) {
        if target.exists() {
            let registration = PlatformXmlAdapterFactory::new().registration();
            let source = SourceContext::new(
                SourceLocation::new(root.to_path_buf(), source_root, target),
                Some("task8-preservation".to_string()),
                SourceFamily::PlatformXml,
                None,
            );
            match registration.capture.capture(&source) {
                Ok(CaptureResult::Captured(captured)) => {
                    let envelope = registration
                        .read
                        .read(&FormatReadRequest {
                            captured: captured.clone(),
                            query: NavigationQuery {
                                target: NavigationTarget::CapturedTarget(
                                    captured.binding().target_identity.clone(),
                                ),
                                select: NavigationSelection {
                                    properties: PropertySelection::All,
                                    facets: FacetSelection::Full,
                                    relations: Vec::new(),
                                },
                            },
                        })
                        .unwrap_or_else(|error| {
                            panic!("production semantic reader must project {kind:?}: {error:?}")
                        });
                    normalize_envelope(&mut facts, &envelope);
                }
                Ok(CaptureResult::NoMatch) => {}
                Err(error) => panic!("production capture failed for {kind:?}: {error:?}"),
            }
        }
    }
    normalize_standalone(kind, root, &mut facts);
    facts
}

fn production_target(kind: WriterCommandKind, root: &Path) -> Option<(PathBuf, PathBuf)> {
    let src = root.join("src");
    let target = match kind {
        WriterCommandKind::ConfigurationInitialize | WriterCommandKind::ConfigurationEdit => {
            src.join("Configuration.xml")
        }
        WriterCommandKind::ExtensionInitialize
        | WriterCommandKind::ExtensionBorrow
        | WriterCommandKind::ExtensionPatchMethod => root.join("extension/Configuration.xml"),
        WriterCommandKind::MetadataCreate
        | WriterCommandKind::MetadataEdit
        | WriterCommandKind::MetadataRemove
        | WriterCommandKind::HelpCreate => src.join("Catalogs/Items.xml"),
        WriterCommandKind::FormCreate
        | WriterCommandKind::FormCompile
        | WriterCommandKind::FormEdit
        | WriterCommandKind::FormRemove => form_descriptor_path(root),
        WriterCommandKind::TemplateCreate | WriterCommandKind::TemplateRemove => {
            src.join("Catalogs/Items/Templates/Main.xml")
        }
        WriterCommandKind::InterfaceEdit => src.join("Subsystems/Sales.xml"),
        WriterCommandKind::RoleCreate => src.join("Roles/Reader.xml"),
        WriterCommandKind::SubsystemCreate | WriterCommandKind::SubsystemEdit => {
            src.join("Subsystems/Sales.xml")
        }
        WriterCommandKind::SupportEdit => src.join("Catalogs/Items.xml"),
        WriterCommandKind::DataCompositionCreate
        | WriterCommandKind::DataCompositionEdit
        | WriterCommandKind::SpreadsheetCreate
        | WriterCommandKind::ExternalProcessorInitialize
        | WriterCommandKind::ExternalReportInitialize => return None,
    };
    let source_root = if target.starts_with(root.join("extension")) {
        root.join("extension")
    } else if target.starts_with(&src) {
        src
    } else {
        target.parent().unwrap_or(root).to_path_buf()
    };
    Some((source_root, target))
}

fn json_text<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("closed semantic value must serialize")
}

fn add_fact(facts: &mut SemanticFacts, fact: SemanticFact) {
    *facts.entry(fact).or_insert(0) += 1;
}

fn normalize_envelope(facts: &mut SemanticFacts, envelope: &NavigationEnvelope) {
    add_fact(
        facts,
        SemanticFact::Envelope {
            status: json_text(&envelope.status),
            root: envelope
                .root
                .as_ref()
                .map(|root| (root.kind.as_str().to_string(), root.display_name.clone())),
            consistency: envelope
                .snapshot
                .as_ref()
                .map(|snapshot| json_text(&snapshot.consistency)),
        },
    );
    for diagnostic in &envelope.diagnostics {
        add_fact(
            facts,
            SemanticFact::Diagnostic {
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
                details: diagnostic.details.as_ref().map(json_text),
            },
        );
    }
    for node in &envelope.nodes {
        let owner_kind = node.object_ref.kind.as_str().to_string();
        let owner_name = node.object_ref.display_name.clone();
        add_fact(
            facts,
            SemanticFact::Identity {
                kind: owner_kind.clone(),
                name: owner_name.clone(),
                reference_kind: node.reference.kind.as_str().to_string(),
                reference_name: node.reference.display_name.clone(),
                capability_state: json_text(&node.capability_state),
                capability: json_text(&node.capability),
                action_profile: json_text(&node.action_profile),
                facet_visibility: format!("{:?}", node.facet_visibility),
            },
        );
        for (id, property) in &node.properties {
            add_fact(
                facts,
                SemanticFact::Property {
                    owner_kind: owner_kind.clone(),
                    owner_name: owner_name.clone(),
                    id: id.as_str().to_string(),
                    value_type: json_text(&property.value_type()),
                    value_state: json_text(&property.value_state()),
                    value: property.value().map(json_text),
                    provenance: json_text(&property.provenance()),
                    capability: json_text(&property.capability()),
                },
            );
        }
        add_fact(
            facts,
            SemanticFact::Facets {
                owner_kind: owner_kind.clone(),
                owner_name: owner_name.clone(),
                members: json_text(&node.facets),
            },
        );
        add_fact(
            facts,
            SemanticFact::Actions {
                owner_kind,
                owner_name,
                semantic: json_text(&node.semantic_actions),
                available: json_text(&node.actions),
            },
        );
    }
    for relation in envelope.relation_index.iter() {
        add_fact(
            facts,
            SemanticFact::Relation {
                value: json_text(relation),
            },
        );
    }
}

fn normalize_standalone(kind: WriterCommandKind, root: &Path, facts: &mut SemanticFacts) {
    let (family, artifact_root) = match kind {
        WriterCommandKind::ExternalProcessorInitialize => {
            ("externalProcessor", root.join("external"))
        }
        WriterCommandKind::ExternalReportInitialize => ("externalReport", root.join("external")),
        WriterCommandKind::DataCompositionCreate | WriterCommandKind::DataCompositionEdit => {
            ("dataComposition", root.join("standalone/dcs.xml"))
        }
        WriterCommandKind::SpreadsheetCreate => ("spreadsheet", root.join("standalone/mxl.xml")),
        WriterCommandKind::FormCompile | WriterCommandKind::FormEdit => {
            ("managedForm", form_path(root))
        }
        WriterCommandKind::InterfaceEdit => (
            "commandInterface",
            root.join("src/Subsystems/Sales/Ext/CommandInterface.xml"),
        ),
        WriterCommandKind::ExtensionPatchMethod => ("extensionModule", root.join("extension")),
        WriterCommandKind::HelpCreate => ("help", root.join("src/Catalogs/Items")),
        _ => return,
    };
    let mut document = 0;
    for path in files(&artifact_root) {
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if path.extension().and_then(|value| value.to_str()) == Some("xml") {
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let Ok(parsed) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else {
                continue;
            };
            let root_element = parsed.root_element();
            for node in root_element.descendants().filter(|node| node.is_element()) {
                let mut indexes = Vec::new();
                let mut cursor = node;
                while cursor != root_element {
                    let index = cursor
                        .prev_siblings()
                        .filter(|sibling| sibling.is_element())
                        .count();
                    indexes.push(index);
                    cursor = cursor.parent_element().expect("descendant has parent");
                }
                indexes.reverse();
                let semantic_kind = standalone_semantic_kind(
                    family,
                    node.tag_name().name(),
                    node.tag_name().namespace(),
                );
                let mut attributes = node
                    .attributes()
                    .map(|attribute| {
                        (
                            standalone_attribute_kind(attribute.name()).to_string(),
                            attribute.value().to_string(),
                        )
                    })
                    .collect::<Vec<_>>();
                attributes.sort();
                add_fact(
                    facts,
                    SemanticFact::StandaloneStructure {
                        family,
                        document,
                        ordinal_path: indexes
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join("."),
                        semantic_kind,
                        value: node
                            .children()
                            .all(|child| !child.is_element())
                            .then(|| node.text().unwrap_or_default().trim().to_string()),
                        attributes,
                    },
                );
            }
            document += 1;
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("bsl" | "html" | "txt")
        ) {
            add_fact(
                facts,
                SemanticFact::StandaloneText {
                    family,
                    role: path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("content")
                        .to_string(),
                    content: String::from_utf8_lossy(&bytes).replace("\r\n", "\n"),
                },
            );
        }
    }
}

fn standalone_semantic_kind(family: &str, local: &str, namespace: Option<&str>) -> String {
    match (family, local) {
        ("dataComposition", "DataCompositionSchema") => "document".to_string(),
        ("dataComposition", "dataSet") => "dataSet".to_string(),
        ("dataComposition", "field") => "field".to_string(),
        ("dataComposition", "parameter") => "parameter".to_string(),
        ("dataComposition", "settingsVariant") => "settingsVariant".to_string(),
        ("spreadsheet", "document") => "document".to_string(),
        ("spreadsheet", "area") => "area".to_string(),
        ("spreadsheet", "row") => "row".to_string(),
        ("spreadsheet", "cell") => "cell".to_string(),
        (_, "MetaDataObject") => "descriptor".to_string(),
        (_, "Name" | "name") => "identityName".to_string(),
        (_, "Form") => "form".to_string(),
        (_, "Template") => "template".to_string(),
        _ => format!("structure:{}:{}", namespace.unwrap_or("none"), local),
    }
}

fn standalone_attribute_kind(name: &str) -> &str {
    match name {
        "name" => "identity",
        "type" => "semanticType",
        "uuid" => "stableIdentity",
        "version" => "formatRevision",
        _ => name,
    }
}

fn assert_expected_facts(
    facts: &SemanticFacts,
    markers: &[ExpectedFact],
    kind: WriterCommandKind,
    scenario: Scenario,
    phase: &str,
) {
    for marker in markers {
        let matching = facts
            .iter()
            .filter(|(fact, _)| expected_fact_matches(marker, fact))
            .collect::<Vec<_>>();
        let count = matching.iter().map(|(_, count)| **count).sum::<usize>();
        let present = match marker {
            ExpectedFact::Diagnostic { present, .. }
            | ExpectedFact::Identity { present, .. }
            | ExpectedFact::Property { present, .. }
            | ExpectedFact::Structure { present, .. }
            | ExpectedFact::StandaloneScalar { present, .. }
            | ExpectedFact::StandaloneText { present, .. } => *present,
        };
        assert_eq!(
            count,
            usize::from(present),
            "{kind:?} {scenario:?} {phase}: exact semantic fact count mismatch for {marker:?}; \
             matches={matching:?}"
        );
    }
}

fn expected_fact_matches(marker: &ExpectedFact, fact: &SemanticFact) -> bool {
    match (marker, fact) {
        (
            ExpectedFact::Diagnostic {
                code,
                message,
                details,
                ..
            },
            SemanticFact::Diagnostic {
                code: actual_code,
                message: actual_message,
                details: actual_details,
            },
        ) => {
            actual_code == code
                && actual_message == message
                && actual_details.as_deref() == *details
        }
        (
            ExpectedFact::Identity { kind, name, .. },
            SemanticFact::Identity {
                kind: actual_kind,
                name: actual_name,
                ..
            },
        ) => actual_kind == kind && actual_name == name,
        (
            ExpectedFact::Property {
                owner,
                id,
                value_type,
                value_state,
                value: expected_value,
                ..
            },
            SemanticFact::Property {
                owner_name,
                id: actual_id,
                value_type: actual_type,
                value_state: actual_state,
                value,
                ..
            },
        ) => {
            owner_name == owner
                && actual_id == id
                && json_string(actual_type).as_deref() == Some(*value_type)
                && json_string(actual_state).as_deref() == Some(*value_state)
                && value
                    .as_ref()
                    .and_then(|value| semantic_scalar(value))
                    .as_deref()
                    == Some(*expected_value)
        }
        (
            ExpectedFact::Structure {
                family,
                semantic_kind,
                value,
                ..
            },
            SemanticFact::StandaloneStructure {
                family: actual_family,
                semantic_kind: actual_kind,
                value: actual_value,
                ..
            },
        ) => {
            actual_family == family
                && actual_kind == semantic_kind
                && value.is_none_or(|value| actual_value.as_deref() == Some(value))
        }
        (
            ExpectedFact::StandaloneScalar {
                family,
                value: expected_value,
                ..
            },
            SemanticFact::StandaloneStructure {
                family: actual_family,
                value,
                attributes,
                ..
            },
        ) => {
            actual_family == family
                && (value.as_ref().is_some_and(|value| value == expected_value)
                    || attributes.iter().any(|(_, value)| value == expected_value))
        }
        (
            ExpectedFact::StandaloneText {
                family,
                role: expected_role,
                content: expected_content,
                ..
            },
            SemanticFact::StandaloneText {
                family: actual_family,
                role,
                content,
                ..
            },
        ) => actual_family == family && role == expected_role && content == expected_content,
        _ => false,
    }
}

fn json_string(value: &str) -> Option<String> {
    serde_json::from_str(value).ok()
}

fn semantic_scalar(value: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()?
        .get("value")?
        .as_str()
        .map(str::to_string)
}

fn assert_declared_delta(
    before: &SemanticFacts,
    after: &SemanticFacts,
    case: &MatrixCase,
    scenario: Scenario,
) {
    assert_ne!(
        case.before, case.after,
        "{:?}: hand-authored before/after facts declare no semantic delta",
        case.kind
    );
    let keys = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let changed = keys
        .into_iter()
        .filter_map(|fact| {
            let before_count = before.get(&fact).copied().unwrap_or_default();
            let after_count = after.get(&fact).copied().unwrap_or_default();
            (before_count != after_count).then_some((fact, before_count, after_count))
        })
        .collect::<Vec<_>>();
    assert!(
        !changed.is_empty(),
        "{:?} {scenario:?}: production semantic facts had no delta",
        case.kind
    );

    let mut allowed_objects = case
        .before
        .iter()
        .chain(&case.after)
        .filter_map(expected_owner)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    loop {
        let mut expanded = false;
        for (fact, _, _) in &changed {
            let SemanticFact::Relation { value } = fact else {
                continue;
            };
            let Some((source, target)) = relation_endpoints(value) else {
                continue;
            };
            if allowed_objects.contains(&source) || allowed_objects.contains(&target) {
                expanded |= allowed_objects.insert(source);
                expanded |= allowed_objects.insert(target);
            }
        }
        if !expanded {
            break;
        }
    }

    for (fact, before_count, after_count) in changed {
        assert!(
            fact_is_in_declared_scope(
                &fact,
                &allowed_objects,
                case.before.iter().chain(&case.after)
            ),
            "{:?} {scenario:?}: unrelated semantic fact changed from {before_count} to \
             {after_count}: {fact:?}",
            case.kind
        );
    }
}

fn expected_owner(marker: &ExpectedFact) -> Option<&'static str> {
    match marker {
        ExpectedFact::Identity { name, .. } => Some(name),
        ExpectedFact::Property { owner, .. } => Some(owner),
        ExpectedFact::Diagnostic { .. }
        | ExpectedFact::Structure { .. }
        | ExpectedFact::StandaloneScalar { .. }
        | ExpectedFact::StandaloneText { .. } => None,
    }
}

fn relation_endpoints(value: &str) -> Option<(String, String)> {
    let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
    Some((
        value
            .get("source")?
            .get("displayName")?
            .as_str()?
            .to_string(),
        value
            .get("target")?
            .get("displayName")?
            .as_str()?
            .to_string(),
    ))
}

fn fact_is_in_declared_scope<'a>(
    fact: &SemanticFact,
    allowed_objects: &BTreeSet<String>,
    mut markers: impl Iterator<Item = &'a ExpectedFact>,
) -> bool {
    match fact {
        SemanticFact::Envelope { .. } => true,
        SemanticFact::Diagnostic {
            code,
            message,
            details,
        } => {
            let explicitly_declared = markers.any(|marker| {
                matches!(
                    marker,
                    ExpectedFact::Diagnostic {
                        code: expected_code,
                        message: expected_message,
                        details: expected_details,
                        ..
                    } if code == expected_code
                        && message == expected_message
                        && details.as_deref() == *expected_details
                )
            });
            explicitly_declared
                || details
                    .as_ref()
                    .and_then(|details| serde_json::from_str::<serde_json::Value>(details).ok())
                    .and_then(|details| {
                        details
                            .get("objectRef")
                            .and_then(|object| object.get("displayName"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .is_some_and(|owner| allowed_objects.contains(&owner))
        }
        SemanticFact::Identity { name, kind, .. } => {
            kind == "sourceRoot" || allowed_objects.contains(name)
        }
        SemanticFact::Property {
            owner_kind,
            owner_name,
            ..
        }
        | SemanticFact::Facets {
            owner_kind,
            owner_name,
            ..
        }
        | SemanticFact::Actions {
            owner_kind,
            owner_name,
            ..
        } => owner_kind == "sourceRoot" || allowed_objects.contains(owner_name),
        SemanticFact::Relation { value } => {
            relation_endpoints(value).is_some_and(|(source, target)| {
                allowed_objects.contains(&source) || allowed_objects.contains(&target)
            })
        }
        SemanticFact::StandaloneStructure { family, .. }
        | SemanticFact::StandaloneText { family, .. } => markers.any(|marker| match marker {
            ExpectedFact::Structure {
                family: expected, ..
            }
            | ExpectedFact::StandaloneScalar {
                family: expected, ..
            }
            | ExpectedFact::StandaloneText {
                family: expected, ..
            } => family == expected,
            ExpectedFact::Diagnostic { .. }
            | ExpectedFact::Identity { .. }
            | ExpectedFact::Property { .. } => false,
        }),
    }
}

fn make_semantically_unsupported(kind: WriterCommandKind, root: &Path) {
    let target = match kind {
        WriterCommandKind::ExtensionBorrow | WriterCommandKind::ExtensionPatchMethod => {
            root.join("extension/Configuration.xml")
        }
        WriterCommandKind::DataCompositionCreate | WriterCommandKind::DataCompositionEdit => {
            root.join("standalone/dcs.xml")
        }
        WriterCommandKind::SpreadsheetCreate => root.join("standalone/mxl.xml"),
        WriterCommandKind::ExternalProcessorInitialize => root.join("external/MatrixProcessor.xml"),
        WriterCommandKind::ExternalReportInitialize => root.join("external/MatrixReport.xml"),
        _ if has_configuration_fixture(kind) => root.join("src/Configuration.xml"),
        _ => production_target(kind, root)
            .map(|(_, target)| target)
            .unwrap_or_else(|| root.join("external/Configuration.xml")),
    };
    if let Ok(bytes) = fs::read(&target) {
        let text = String::from_utf8_lossy(&bytes);
        let updated = if text.contains("version=\"2.20\"") {
            text.replacen("version=\"2.20\"", "version=\"2.21\"", 1)
        } else if let Some(position) = text.find('>') {
            format!(
                "{} version=\"2.21\"{}",
                &text[..position],
                &text[position..]
            )
        } else {
            text.to_string()
        };
        fs::write(target, updated.as_bytes()).unwrap();
    } else {
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            target,
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration><Properties><Name>UnsupportedOwner</Name></Properties><ChildObjects/></Configuration></MetaDataObject>",
        )
        .unwrap();
    }
}

fn prepare_fixture(kind: WriterCommandKind, root: &Path) {
    if matches!(
        kind,
        WriterCommandKind::DataCompositionCreate
            | WriterCommandKind::DataCompositionEdit
            | WriterCommandKind::SpreadsheetCreate
    ) {
        fs::create_dir_all(root.join("standalone")).unwrap();
    }

    if has_configuration_fixture(kind) {
        prerequisite(
            WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(text(
                "MatrixBase",
                ConfigurationName::new,
            ))),
            vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
            root,
        );
    }

    if needs_metadata_fixture(kind) {
        prerequisite(
            WriterCommand::MetadataCreate(MetadataCreate::new(metadata_definition())),
            vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
            root,
        );
    }

    match kind {
        WriterCommandKind::ExtensionBorrow | WriterCommandKind::ExtensionPatchMethod => {
            prerequisite(
                WriterCommand::ExtensionInitialize(ExtensionInitialize::new(text(
                    "MatrixExtension",
                    ExtensionName::new,
                ))),
                vec![
                    (
                        WriterSourceRole::DestinationDirectory,
                        root.join("extension"),
                    ),
                    (
                        WriterSourceRole::Configuration,
                        root.join("src/Configuration.xml"),
                    ),
                ],
                root,
            );
            if kind == WriterCommandKind::ExtensionPatchMethod {
                prerequisite(
                    WriterCommand::ExtensionBorrow(ExtensionBorrow::new(text(
                        "Catalog.Items",
                        MetadataObjectReference::new,
                    ))),
                    vec![
                        (WriterSourceRole::Extension, root.join("extension")),
                        (WriterSourceRole::Configuration, root.join("src")),
                    ],
                    root,
                );
                write(
                    &root.join("extension/Catalogs/Items/Ext/ObjectModule.bsl"),
                    "Procedure BeforeWrite()\nEndProcedure\n",
                );
            }
        }
        WriterCommandKind::MetadataEdit | WriterCommandKind::MetadataRemove => {}
        WriterCommandKind::FormCompile | WriterCommandKind::FormEdit => {
            prerequisite(
                WriterCommand::FormCreate(FormCreate::new(
                    text("Catalog.Items", FormOwnerReference::new),
                    text("ObjectForm", FormName::new),
                )),
                vec![(
                    WriterSourceRole::Object,
                    root.join("src/Catalogs/Items.xml"),
                )],
                root,
            );
            if kind == WriterCommandKind::FormEdit {
                prerequisite(
                    WriterCommand::FormCompile(FormCompile::new(
                        ManagedFormDefinition::empty(),
                        false,
                    )),
                    vec![(WriterSourceRole::DestinationArtifact, form_path(root))],
                    root,
                );
            }
        }
        WriterCommandKind::FormRemove => {
            prerequisite(
                WriterCommand::FormCreate(FormCreate::new(
                    text("Catalog.Items", FormOwnerReference::new),
                    text("ObjectForm", FormName::new),
                )),
                vec![(
                    WriterSourceRole::Object,
                    root.join("src/Catalogs/Items.xml"),
                )],
                root,
            );
        }
        WriterCommandKind::TemplateRemove => {
            prerequisite(
                WriterCommand::TemplateCreate(TemplateCreate::new(
                    text("Catalog.Items", TemplateOwnerReference::new),
                    text("Main", TemplateName::new),
                    TemplateKind::Text,
                )),
                vec![(WriterSourceRole::SourceCollection, root.join("src"))],
                root,
            );
        }
        WriterCommandKind::InterfaceEdit => {
            prerequisite(
                WriterCommand::SubsystemCreate(SubsystemCreate::from_definition(
                    SubsystemDefinition::new(text("Sales", SubsystemName::new)),
                )),
                vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
                root,
            );
            let path = root.join("src/Subsystems/Sales/Ext/CommandInterface.xml");
            if !path.exists() {
                write(
                    &path,
                    concat!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                        "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" ",
                        "xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" ",
                        "version=\"2.20\"><CommandsVisibility/><CommandsPlacement/>",
                        "<CommandsOrder/><SubsystemsOrder/><GroupsOrder/>",
                        "</CommandInterface>\n",
                    ),
                );
            }
        }
        WriterCommandKind::SubsystemEdit => {
            prerequisite(
                WriterCommand::SubsystemCreate(SubsystemCreate::from_definition(
                    SubsystemDefinition::new(text("Sales", SubsystemName::new)),
                )),
                vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
                root,
            );
            prerequisite(
                WriterCommand::SubsystemCreate(SubsystemCreate::from_definition(
                    SubsystemDefinition::new(text("SalesReports", SubsystemName::new)),
                )),
                vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
                root,
            );
        }
        WriterCommandKind::SupportEdit => {
            let configuration_uuid =
                xml_attribute(&root.join("src/Configuration.xml"), "Configuration", "uuid")
                    .expect("support fixture configuration must have a UUID");
            let object_uuid =
                xml_attribute(&root.join("src/Catalogs/Items.xml"), "Catalog", "uuid")
                    .expect("support fixture catalog must have a UUID");
            let payload = format!(
                concat!(
                    "{{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                    "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                    "\"VendorConf\",2,1,0,{0},0,0,{1},{1}}}"
                ),
                configuration_uuid, object_uuid
            );
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(payload.as_bytes());
            let path = root.join("src/Ext/ParentConfigurations.bin");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        WriterCommandKind::DataCompositionEdit => {
            prerequisite(
                WriterCommand::DataCompositionCreate(DataCompositionCreate::new(
                    data_composition_definition(),
                )),
                vec![(
                    WriterSourceRole::DestinationArtifact,
                    root.join("standalone/dcs.xml"),
                )],
                root,
            );
        }
        _ => {}
    }
}

fn prerequisite(command: WriterCommand, sources: Vec<(WriterSourceRole, PathBuf)>, root: &Path) {
    let kind = command.kind();
    let result = execute(
        command,
        sources,
        root,
        MutationMode::Apply,
        OperationCancellation::new(),
    );
    assert!(
        matches!(result.lifecycle(), WriterLifecycle::Applied),
        "{kind:?} fixture preparation failed: {result:?}"
    );
}

fn sources(kind: WriterCommandKind, root: &Path) -> Vec<(WriterSourceRole, PathBuf)> {
    let src = root.join("src");
    match kind {
        WriterCommandKind::ConfigurationInitialize => {
            vec![(WriterSourceRole::DestinationDirectory, src)]
        }
        WriterCommandKind::ConfigurationEdit => vec![(
            WriterSourceRole::Configuration,
            src.join("Configuration.xml"),
        )],
        WriterCommandKind::ExtensionInitialize => vec![
            (
                WriterSourceRole::DestinationDirectory,
                root.join("extension"),
            ),
            (
                WriterSourceRole::Configuration,
                src.join("Configuration.xml"),
            ),
        ],
        WriterCommandKind::ExtensionBorrow => vec![
            (WriterSourceRole::Extension, root.join("extension")),
            (WriterSourceRole::Configuration, src),
        ],
        WriterCommandKind::ExtensionPatchMethod => {
            vec![(WriterSourceRole::Extension, root.join("extension"))]
        }
        WriterCommandKind::ExternalProcessorInitialize
        | WriterCommandKind::ExternalReportInitialize => vec![(
            WriterSourceRole::DestinationDirectory,
            root.join("external"),
        )],
        WriterCommandKind::MetadataCreate
        | WriterCommandKind::RoleCreate
        | WriterCommandKind::SubsystemCreate => vec![(WriterSourceRole::DestinationDirectory, src)],
        WriterCommandKind::MetadataEdit => {
            vec![(WriterSourceRole::Object, src.join("Catalogs/Items.xml"))]
        }
        WriterCommandKind::MetadataRemove => vec![(WriterSourceRole::ConfigurationDirectory, src)],
        WriterCommandKind::FormCreate => {
            vec![(WriterSourceRole::Object, src.join("Catalogs/Items.xml"))]
        }
        WriterCommandKind::FormCompile => {
            vec![(WriterSourceRole::DestinationArtifact, form_path(root))]
        }
        WriterCommandKind::FormEdit => {
            vec![(WriterSourceRole::Form, form_path(root))]
        }
        WriterCommandKind::FormRemove => vec![(WriterSourceRole::SourceCollection, src)],
        WriterCommandKind::TemplateCreate
        | WriterCommandKind::TemplateRemove
        | WriterCommandKind::HelpCreate => vec![(WriterSourceRole::SourceCollection, src)],
        WriterCommandKind::InterfaceEdit => vec![(
            WriterSourceRole::Interface,
            src.join("Subsystems/Sales/Ext/CommandInterface.xml"),
        )],
        WriterCommandKind::SubsystemEdit => vec![(
            WriterSourceRole::Subsystem,
            src.join("Subsystems/Sales.xml"),
        )],
        WriterCommandKind::SupportEdit => vec![(
            WriterSourceRole::SupportTarget,
            src.join("Catalogs/Items.xml"),
        )],
        WriterCommandKind::DataCompositionCreate => vec![(
            WriterSourceRole::DestinationArtifact,
            root.join("standalone/dcs.xml"),
        )],
        WriterCommandKind::DataCompositionEdit => {
            vec![(WriterSourceRole::Template, root.join("standalone/dcs.xml"))]
        }
        WriterCommandKind::SpreadsheetCreate => vec![(
            WriterSourceRole::DestinationArtifact,
            root.join("standalone/mxl.xml"),
        )],
    }
}

fn matrix_cases() -> Vec<MatrixCase> {
    let object = || text("Catalog.Items", MetadataObjectReference::new);
    let form_owner = || text("Catalog.Items", FormOwnerReference::new);
    let template_owner = || text("Catalog.Items", TemplateOwnerReference::new);
    let mut cases = vec![
        case(WriterCommand::ConfigurationInitialize(
            ConfigurationInitialize::new(text("MatrixConfiguration", ConfigurationName::new)),
        )),
        case(WriterCommand::ConfigurationEdit(ConfigurationEdit::mutate(
            ConfigurationMutation::SetProperty(
                ConfigurationPropertyPatch::new(
                    ConfigurationProperty::Comment,
                    ConfigurationPropertyValue::Text(text(
                        "matrix configuration comment",
                        ConfigurationTextValue::new,
                    )),
                )
                .unwrap(),
            ),
        ))),
        case(WriterCommand::ExtensionInitialize(
            ExtensionInitialize::new(text("MatrixExtension", ExtensionName::new)),
        )),
        case(WriterCommand::ExtensionBorrow(ExtensionBorrow::new(
            object(),
        ))),
        case(WriterCommand::ExtensionPatchMethod(
            ExtensionPatchMethod::new(
                ExtensionModuleTarget::Object {
                    owner: text("Catalog.Items", MetadataObjectReference::new),
                    role: ExtensionObjectModuleRole::Object,
                },
                text("BeforeWrite", MethodName::new),
                InterceptorKind::Before,
                ExecutionContext::Automatic,
                false,
            ),
        )),
        case(WriterCommand::ExternalProcessorInitialize(
            ExternalArtifactInitialize::new(text("MatrixProcessor", ExternalArtifactName::new)),
        )),
        case(WriterCommand::ExternalReportInitialize(
            ExternalArtifactInitialize::new(text("MatrixReport", ExternalArtifactName::new)),
        )),
        case(WriterCommand::MetadataCreate(MetadataCreate::new(
            metadata_definition(),
        ))),
        case(WriterCommand::MetadataEdit(MetadataEdit::new(
            object(),
            MetadataPatch::SetProperties(MetadataPropertyChanges::one(
                MetadataPropertyPatch::new(
                    MetadataObjectProperty::Comment,
                    MetadataPropertyValue::Comment(text(
                        "matrix metadata comment",
                        CommentText::new,
                    )),
                )
                .unwrap(),
            )),
        ))),
        case(WriterCommand::MetadataRemove(MetadataRemove::new(
            object(),
            false,
        ))),
        case(WriterCommand::FormCreate(FormCreate::new(
            form_owner(),
            text("ObjectForm", FormName::new),
        ))),
        case(WriterCommand::FormCompile(FormCompile::new(
            compiled_form_definition(),
            false,
        ))),
        case(WriterCommand::FormEdit(
            FormEdit::new(
                vec![FormPatch::AddElement(FormElementDefinition::new(
                    text("MatrixField", FormElementName::new),
                    FormElementType::Input,
                ))],
                false,
            )
            .unwrap(),
        )),
        case(WriterCommand::FormRemove(FormRemove::new(
            form_owner(),
            text("ObjectForm", FormName::new),
        ))),
        case(WriterCommand::TemplateCreate(TemplateCreate::new(
            template_owner(),
            text("Main", TemplateName::new),
            TemplateKind::Text,
        ))),
        case(WriterCommand::TemplateRemove(TemplateRemove::new(
            template_owner(),
            text("Main", TemplateName::new),
        ))),
        case(WriterCommand::HelpCreate(HelpCreate::new(
            text("Catalog.Items", HelpOwnerReference::new),
            Some(text("en", LanguageCode::new)),
        ))),
        case(WriterCommand::InterfaceEdit(InterfaceEdit::Place(
            InterfacePlacement::new(
                InterfaceItemReference::new(
                    InterfaceItemKind::Command,
                    text("Catalog.Items.Command.Open", InterfaceItemName::new),
                ),
                text("Main", InterfaceGroupName::new),
                1,
            ),
        ))),
        case(WriterCommand::RoleCreate(RoleCreate::from_definition(
            RoleDefinition::new(text("Reader", RoleName::new)),
        ))),
        case(WriterCommand::SubsystemCreate(
            SubsystemCreate::from_definition(SubsystemDefinition::new(text(
                "Sales",
                SubsystemName::new,
            ))),
        )),
        case(WriterCommand::SubsystemEdit(SubsystemEdit::AddChild(text(
            "SalesReports",
            SubsystemName::new,
        )))),
        case(WriterCommand::SupportEdit(SupportEdit::ObjectRule(
            SupportObjectRule::Editable,
        ))),
        case(WriterCommand::DataCompositionCreate(
            DataCompositionCreate::new(data_composition_definition()),
        )),
        case(WriterCommand::DataCompositionEdit(
            DataCompositionEdit::new(DataCompositionMutation::AddParameter(
                DataCompositionParameter::new(text(
                    "MatrixParameter",
                    DataCompositionParameterName::new,
                )),
            )),
        )),
        case(WriterCommand::SpreadsheetCreate(SpreadsheetCreate::new(
            spreadsheet_document(),
        ))),
    ];
    let mut expectations = BTreeMap::from([
        (
            WriterCommandKind::ConfigurationInitialize,
            (
                vec![identity("configuration", "MatrixConfiguration", false)],
                vec![identity("configuration", "MatrixConfiguration", true)],
            ),
        ),
        (
            WriterCommandKind::ConfigurationEdit,
            (
                vec![
                    identity("configuration", "MatrixBase", true),
                    property(
                        "MatrixBase",
                        "metadata.comment",
                        "matrix configuration comment",
                        false,
                    ),
                ],
                vec![
                    identity("configuration", "MatrixBase", true),
                    property(
                        "MatrixBase",
                        "metadata.comment",
                        "matrix configuration comment",
                        true,
                    ),
                ],
            ),
        ),
        (
            WriterCommandKind::ExtensionInitialize,
            (
                vec![identity("configuration", "MatrixExtension", false)],
                vec![identity("configuration", "MatrixExtension", true)],
            ),
        ),
        (
            WriterCommandKind::ExtensionBorrow,
            (
                vec![identity("catalog", "Items", false)],
                vec![identity("catalog", "Items", true)],
            ),
        ),
        (
            WriterCommandKind::ExtensionPatchMethod,
            (
                vec![
                    identity("configuration", "MatrixExtension", true),
                    standalone_text(
                        "extensionModule",
                        "ObjectModule.bsl",
                        "Procedure BeforeWrite()\nEndProcedure\n// MATRIX_PATCH\n",
                        false,
                    ),
                ],
                vec![
                    identity("configuration", "MatrixExtension", true),
                    standalone_text(
                        "extensionModule",
                        "ObjectModule.bsl",
                        "Procedure BeforeWrite()\nEndProcedure\n// MATRIX_PATCH\n",
                        true,
                    ),
                ],
            ),
        ),
        (
            WriterCommandKind::ExternalProcessorInitialize,
            (
                vec![structure(
                    "externalProcessor",
                    "identityName",
                    Some("MatrixProcessor"),
                    false,
                )],
                vec![structure(
                    "externalProcessor",
                    "identityName",
                    Some("MatrixProcessor"),
                    true,
                )],
            ),
        ),
        (
            WriterCommandKind::ExternalReportInitialize,
            (
                vec![structure(
                    "externalReport",
                    "identityName",
                    Some("MatrixReport"),
                    false,
                )],
                vec![structure(
                    "externalReport",
                    "identityName",
                    Some("MatrixReport"),
                    true,
                )],
            ),
        ),
        (
            WriterCommandKind::MetadataCreate,
            (
                vec![identity("catalog", "Items", false)],
                vec![identity("catalog", "Items", true)],
            ),
        ),
        (
            WriterCommandKind::MetadataEdit,
            (
                vec![
                    identity("catalog", "Items", true),
                    property(
                        "Items",
                        "metadata.comment",
                        "matrix metadata comment",
                        false,
                    ),
                ],
                vec![
                    identity("catalog", "Items", true),
                    property("Items", "metadata.comment", "matrix metadata comment", true),
                ],
            ),
        ),
        (
            WriterCommandKind::MetadataRemove,
            (
                vec![identity("catalog", "Items", true)],
                vec![identity("catalog", "Items", false)],
            ),
        ),
        (
            WriterCommandKind::FormCreate,
            (
                vec![identity("form", "ObjectForm", false)],
                vec![identity("form", "ObjectForm", true)],
            ),
        ),
        (
            WriterCommandKind::FormCompile,
            (
                vec![
                    identity("form", "ObjectForm", true),
                    standalone_value("managedForm", "MatrixCompiled", false),
                ],
                vec![
                    identity("form", "ObjectForm", true),
                    standalone_value("managedForm", "MatrixCompiled", true),
                ],
            ),
        ),
        (
            WriterCommandKind::FormEdit,
            (
                vec![
                    identity("form", "ObjectForm", true),
                    standalone_value("managedForm", "MatrixField", false),
                ],
                vec![
                    identity("form", "ObjectForm", true),
                    standalone_value("managedForm", "MatrixField", true),
                ],
            ),
        ),
        (
            WriterCommandKind::FormRemove,
            (
                vec![identity("form", "ObjectForm", true)],
                vec![identity("form", "ObjectForm", false)],
            ),
        ),
        (
            WriterCommandKind::TemplateCreate,
            (
                vec![
                    identity("template", "Main", false),
                    partial_coverage_diagnostic(false),
                ],
                vec![
                    identity("template", "Main", true),
                    partial_coverage_diagnostic(true),
                ],
            ),
        ),
        (
            WriterCommandKind::TemplateRemove,
            (
                vec![
                    identity("template", "Main", true),
                    partial_coverage_diagnostic(true),
                ],
                vec![
                    identity("template", "Main", false),
                    partial_coverage_diagnostic(false),
                ],
            ),
        ),
        (
            WriterCommandKind::HelpCreate,
            (
                vec![
                    identity("catalog", "Items", true),
                    standalone_value("help", "en", false),
                ],
                vec![
                    identity("catalog", "Items", true),
                    standalone_value("help", "en", true),
                ],
            ),
        ),
        (
            WriterCommandKind::InterfaceEdit,
            (
                vec![
                    identity("subsystem", "Sales", true),
                    standalone_value("commandInterface", "Catalog.Items.Command.Open", false),
                ],
                vec![
                    identity("subsystem", "Sales", true),
                    standalone_value("commandInterface", "Catalog.Items.Command.Open", true),
                ],
            ),
        ),
        (
            WriterCommandKind::RoleCreate,
            (
                vec![identity("role", "Reader", false)],
                vec![identity("role", "Reader", true)],
            ),
        ),
        (
            WriterCommandKind::SubsystemCreate,
            (
                vec![identity("subsystem", "Sales", false)],
                vec![identity("subsystem", "Sales", true)],
            ),
        ),
        (
            WriterCommandKind::SubsystemEdit,
            (
                vec![identity("subsystem", "SalesReports", false)],
                vec![identity("subsystem", "SalesReports", true)],
            ),
        ),
        (
            WriterCommandKind::SupportEdit,
            (
                vec![property(
                    "Items",
                    "support.state",
                    "supportedEditable",
                    false,
                )],
                vec![property(
                    "Items",
                    "support.state",
                    "supportedEditable",
                    true,
                )],
            ),
        ),
        (
            WriterCommandKind::DataCompositionCreate,
            (
                vec![structure("dataComposition", "document", None, false)],
                vec![
                    structure("dataComposition", "document", None, true),
                    standalone_value("dataComposition", "MatrixData", true),
                ],
            ),
        ),
        (
            WriterCommandKind::DataCompositionEdit,
            (
                vec![standalone_value(
                    "dataComposition",
                    "MatrixParameter",
                    false,
                )],
                vec![standalone_value("dataComposition", "MatrixParameter", true)],
            ),
        ),
        (
            WriterCommandKind::SpreadsheetCreate,
            (
                vec![structure("spreadsheet", "document", None, false)],
                vec![
                    structure("spreadsheet", "document", None, true),
                    standalone_value("spreadsheet", "MatrixArea", true),
                ],
            ),
        ),
    ]);
    for case in &mut cases {
        let (before, after) = expectations
            .remove(&case.kind)
            .unwrap_or_else(|| panic!("missing hand-authored facts for {:?}", case.kind));
        case.before = before;
        case.after = after;
    }
    assert!(expectations.is_empty());
    cases.sort_by_key(|case| match case.kind {
        WriterCommandKind::InterfaceEdit => 0,
        WriterCommandKind::FormCompile => 1,
        _ => 2,
    });
    cases
}

fn case(command: WriterCommand) -> MatrixCase {
    let kind = command.kind();
    let (before_fact_digest, after_fact_digest) = expected_fact_digests(kind);
    MatrixCase {
        kind,
        command,
        before: Vec::new(),
        after: Vec::new(),
        before_fact_digest,
        after_fact_digest,
    }
}

fn expected_fact_digests(kind: WriterCommandKind) -> (&'static str, &'static str) {
    use WriterCommandKind as Kind;
    match kind {
        Kind::ConfigurationInitialize => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "f6eb5e6bb653fa673fa60100a98f0bf25d011ea9246a4af7cdadce0c2d52d7bd",
        ),
        Kind::ConfigurationEdit => (
            "a18686c5fafc21be234b429aa5995a2e3ce3047c4604c69fdc7e73989a8f12b4",
            "44f13acdfb5f5cd6884791e5d8e912e6d9f5a5be8a225d2062a654e134bb3983",
        ),
        Kind::ExtensionInitialize => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "ba3f602df52171f46b57984a99f676ac994f0415a4b364f66d205dbc4a29cf24",
        ),
        Kind::ExtensionBorrow => (
            "ba3f602df52171f46b57984a99f676ac994f0415a4b364f66d205dbc4a29cf24",
            "03ca46b4210b91e9bcfd6cac6bd8888d843cf7d9c5e04338ef8d65d90177eb86",
        ),
        Kind::ExtensionPatchMethod => (
            "9bb4eee0e9712ddf4bc2142c2f4fb6f92e936c09fae25da4d7081d48ec4f551c",
            "1485145d9028267810cddf699781ee1432fc7b9e80ea0cd7e5b9c9a36e40635b",
        ),
        Kind::ExternalProcessorInitialize => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "46eff225520320533870506797919859f326f52421bcb15050bb5a6866f8e433",
        ),
        Kind::ExternalReportInitialize => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "5f38ccbc2bbc6fc179b2cd9d4a79c821ea503edcfb51918f39d205e1e52e2006",
        ),
        Kind::MetadataCreate => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "7ed400329d7e1b176ad151fcddfd2ae1ce58fd7187f23ea2ed9a9e62ee632393",
        ),
        Kind::MetadataEdit => (
            "7ed400329d7e1b176ad151fcddfd2ae1ce58fd7187f23ea2ed9a9e62ee632393",
            "e472b135d5ea0b67fa092e4df4d639765aedf129babe0cacc98dde08ca7c3e1b",
        ),
        Kind::MetadataRemove => (
            "7ed400329d7e1b176ad151fcddfd2ae1ce58fd7187f23ea2ed9a9e62ee632393",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        Kind::FormCreate => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "5244cc5128f08caa3a4eef8aea4a93192c4eea813c9b9e2bde2e7f4f45e1ec25",
        ),
        Kind::FormCompile => (
            "7050f6f03d7d5a76b51a70f82e592f9bc6861439540009dd62fa6d20ca114bec",
            "558ea4366ea1be88993767fe09e7d32b3bd7ef76e75a3e9e9ede22b32155fb76",
        ),
        Kind::FormEdit => (
            "35673c0b77a5fe50be68036f849c830a56b6e5c68b8b48e6ada6c7b1f1e2924f",
            "958a70dde657ad66454a29aa18d0425574c9f36ed3d849d4b7c9ae6238357c4e",
        ),
        Kind::FormRemove => (
            "5244cc5128f08caa3a4eef8aea4a93192c4eea813c9b9e2bde2e7f4f45e1ec25",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        Kind::TemplateCreate => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "72c3a12f5cd14ae0bb6e5239838798b7a17452e5d12ff683e64c9d789a35ad19",
        ),
        Kind::TemplateRemove => (
            "72c3a12f5cd14ae0bb6e5239838798b7a17452e5d12ff683e64c9d789a35ad19",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        Kind::HelpCreate => (
            "62ba5859ce87b5e53ba2f4c55276da100a4e8f0f3adaa273b1c56e406c68a83f",
            "54cbb91bb009ee83ba35b2bb3ebecd9d14a65b997b93d9d4fe0338eb71f257e7",
        ),
        Kind::InterfaceEdit => (
            "d5e46933660b055a1b3727c42911d95ca45b9b036b056e6215c61221a0c60492",
            "9c70e09f9ae6aad4d46a806cedb96c60e8f8d611a5d77ff50a531e985bb09cbf",
        ),
        Kind::RoleCreate => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "e56bc23e42efc78b192f783b4370a7939dab233ecfa101271993c9a5bf721cdc",
        ),
        Kind::SubsystemCreate => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "a30a3e0e40f47401355652c0d68d99379a09d33a8bd957ff5d4fa951ccd1fcc6",
        ),
        Kind::SubsystemEdit => (
            "a30a3e0e40f47401355652c0d68d99379a09d33a8bd957ff5d4fa951ccd1fcc6",
            "7df25e03d1427b7ec397335d8f1ce7d6c17e36b9839af31c19a5b5ae5fab8859",
        ),
        Kind::SupportEdit => (
            "d74521a968c1c2377982d05bea66973ae2dc7a23d9567dc476661b99640cde1b",
            "34958c1a1d47f008858b1e82c6972bcd62f5d82d3554c25198c6990e03a00a37",
        ),
        Kind::DataCompositionCreate => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "1010906cbaea2e463a625120bf3b93e72dcc3df4ddad2f00ef56163dc9c708b1",
        ),
        Kind::DataCompositionEdit => (
            "1010906cbaea2e463a625120bf3b93e72dcc3df4ddad2f00ef56163dc9c708b1",
            "15260596e7fc8cc4ef6851bc361fb57f57e4a54ea95e3df64e510b24af792cdc",
        ),
        Kind::SpreadsheetCreate => (
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "d21e39c6263f5584b349f5b82957c80d7a27d8a41a0234c6b2d20d513fba9b67",
        ),
    }
}

fn identity(kind: &'static str, name: &'static str, present: bool) -> ExpectedFact {
    ExpectedFact::Identity {
        kind,
        name,
        present,
    }
}

fn partial_coverage_diagnostic(present: bool) -> ExpectedFact {
    ExpectedFact::Diagnostic {
        code: "partialCoverage",
        message: "requested semantic coverage is partial",
        details: None,
        present,
    }
}

fn property(
    owner: &'static str,
    id: &'static str,
    value: &'static str,
    present: bool,
) -> ExpectedFact {
    let (value_type, value_state) = if id == "support.state" {
        ("enum", "computed")
    } else {
        ("string", "explicit")
    };
    ExpectedFact::Property {
        owner,
        id,
        value_type,
        value_state,
        value,
        present,
    }
}

fn structure(
    family: &'static str,
    semantic_kind: &'static str,
    value: Option<&'static str>,
    present: bool,
) -> ExpectedFact {
    ExpectedFact::Structure {
        family,
        semantic_kind,
        value,
        present,
    }
}

fn standalone_value(family: &'static str, value: &'static str, present: bool) -> ExpectedFact {
    ExpectedFact::StandaloneScalar {
        family,
        value,
        present,
    }
}

fn standalone_text(
    family: &'static str,
    role: &'static str,
    content: &'static str,
    present: bool,
) -> ExpectedFact {
    ExpectedFact::StandaloneText {
        family,
        role,
        content,
        present,
    }
}

fn metadata_definition() -> MetadataDefinition {
    MetadataDefinition::new(
        MetadataCommonDefinition::new(text("Items", MetadataChildName::new)),
        MetadataKindDefinition::new(MetadataKind::Catalog, Vec::new()).unwrap(),
    )
}

fn compiled_form_definition() -> ManagedFormDefinition {
    ManagedFormDefinition::new(
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![FormElementDefinition::new(
            text("MatrixCompiled", FormElementName::new),
            FormElementType::Input,
        )],
        Vec::new(),
    )
}

fn data_composition_definition() -> DataCompositionDefinition {
    DataCompositionDefinition::new(vec![DataCompositionDataSet::Query(
        DataCompositionQueryDataSet::new(
            text("MatrixData", DataSetName::new),
            text("SELECT 1 AS MatrixValue", DataCompositionQueryText::new),
        ),
    )])
    .unwrap()
}

fn spreadsheet_document() -> SpreadsheetDocument {
    let cell = SpreadsheetCell::new(
        1,
        SpreadsheetCellValue::Text(text("Matrix value", SpreadsheetCellText::new)),
    )
    .unwrap();
    let area = SpreadsheetArea::new(
        text("MatrixArea", SpreadsheetAreaName::new),
        vec![SpreadsheetRow::new(vec![cell])],
    )
    .unwrap();
    SpreadsheetDocument::new(vec![area]).unwrap()
}

fn text<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, SemanticValueError>) -> T {
    constructor(value.to_string()).unwrap()
}

fn has_configuration_fixture(kind: WriterCommandKind) -> bool {
    !matches!(
        kind,
        WriterCommandKind::ConfigurationInitialize
            | WriterCommandKind::ExternalProcessorInitialize
            | WriterCommandKind::ExternalReportInitialize
            | WriterCommandKind::DataCompositionCreate
            | WriterCommandKind::DataCompositionEdit
            | WriterCommandKind::SpreadsheetCreate
    )
}

fn needs_metadata_fixture(kind: WriterCommandKind) -> bool {
    matches!(
        kind,
        WriterCommandKind::ExtensionBorrow
            | WriterCommandKind::ExtensionPatchMethod
            | WriterCommandKind::MetadataEdit
            | WriterCommandKind::MetadataRemove
            | WriterCommandKind::FormCreate
            | WriterCommandKind::FormCompile
            | WriterCommandKind::FormEdit
            | WriterCommandKind::FormRemove
            | WriterCommandKind::TemplateCreate
            | WriterCommandKind::TemplateRemove
            | WriterCommandKind::HelpCreate
            | WriterCommandKind::InterfaceEdit
            | WriterCommandKind::SupportEdit
    )
}

fn form_path(root: &Path) -> PathBuf {
    root.join("src/Catalogs/Items/Forms/ObjectForm/Ext/Form.xml")
}

fn form_descriptor_path(root: &Path) -> PathBuf {
    root.join("src/Catalogs/Items/Forms/ObjectForm.xml")
}

fn xml_attribute(path: &Path, element: &str, attribute: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == element)
        .and_then(|node| node.attribute(attribute))
        .map(str::to_owned)
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_file() {
            files.push(path);
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        pending.extend(entries.into_iter().rev());
    }
    files
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn fixture_root(kind: WriterCommandKind, scenario: Scenario) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-task8-fix2-{kind:?}-{scenario:?}-{}-{nonce}",
        std::process::id()
    ))
}
