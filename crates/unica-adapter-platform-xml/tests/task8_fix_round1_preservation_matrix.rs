use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Barrier},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
        family: String,
        namespace: Option<String>,
        document: usize,
        ordinal_path: String,
        semantic_kind: String,
        value: Option<String>,
        attributes: Vec<(String, String)>,
    },
    StandaloneText {
        family: String,
        role: String,
        content: String,
    },
}

type SemanticFacts = BTreeMap<SemanticFact, usize>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CountedSemanticFact {
    fact: SemanticFact,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OracleProvenance {
    source: String,
    reviewed_evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewedSemanticOracle {
    schema_version: u8,
    variant: String,
    provenance: OracleProvenance,
    removed: Vec<CountedSemanticFact>,
    added: Vec<CountedSemanticFact>,
}

#[derive(Clone)]
struct MatrixCase {
    kind: WriterCommandKind,
    command: WriterCommand,
    oracle: ReviewedSemanticOracle,
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
            assert_reviewed_delta(case, &before, &after, scenario);
            assert_all_xml_is_complete(root);
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
            assert_all_xml_is_complete(root);
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
            assert_reviewed_delta(case, &before, &once, scenario);
            let repeat = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert_repeat_outcome(case.kind, &repeat);
            assert_eq!(
                observe(case.kind, root),
                once,
                "{:?}: repeat changed semantic facts",
                case.kind
            );
            assert_all_xml_is_complete(root);
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
            let codes = result
                .diagnostics()
                .iter()
                .map(WriterDiagnostic::code)
                .collect::<Vec<_>>();
            assert_eq!(
                codes,
                [expected_denial_code(case.kind)],
                "{:?}: denial must have the exact same-family outcome",
                case.kind
            );
            assert_eq!(
                observe(case.kind, root),
                denied_before,
                "{:?}: denied operation changed unsupported semantic facts",
                case.kind
            );
            assert_all_xml_is_complete(root);
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
            assert_all_xml_is_complete(root);
        }
        Scenario::Concurrent => {
            let root = Arc::new(root.to_path_buf());
            let acquired = Arc::new(Barrier::new(2));
            let release = Arc::new(Barrier::new(2));
            let first_root = Arc::clone(&root);
            let first_command = case.command.clone();
            let first_kind = case.kind;
            let first_acquired = Arc::clone(&acquired);
            let first_release = Arc::clone(&release);
            let first_worker = thread::spawn(move || {
                PlatformXmlAdapterFactory::new().with_publication_lock_pause(
                    first_acquired,
                    first_release,
                    || {
                        execute(
                            first_command,
                            sources(first_kind, &first_root),
                            &first_root,
                            MutationMode::Apply,
                            OperationCancellation::new(),
                        )
                    },
                )
            });
            acquired.wait();

            let second_root = Arc::clone(&root);
            let second_command = case.command.clone();
            let second_kind = case.kind;
            let (contended_sender, contended_receiver) = mpsc::channel();
            let second_worker = thread::spawn(move || {
                PlatformXmlAdapterFactory::new().with_publication_lock_contention_signal(
                    contended_sender,
                    || {
                        execute(
                            second_command,
                            sources(second_kind, &second_root),
                            &second_root,
                            MutationMode::Apply,
                            OperationCancellation::new(),
                        )
                    },
                )
            });
            contended_receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|error| {
                    panic!("{:?}: second writer never contended: {error}", case.kind)
                });
            release.wait();
            let first = first_worker
                .join()
                .expect("lock owner writer must not panic");
            let second = second_worker
                .join()
                .expect("contending writer must not panic");
            assert_concurrent_outcome(case.kind, &first, &second);
            let after = observe(case.kind, &root);
            assert_reviewed_delta(case, &before, &after, scenario);
            assert_all_xml_is_complete(&root);
        }
    }
}

fn canonical_facts(facts: &SemanticFacts) -> SemanticFacts {
    let mut canonical = SemanticFacts::new();
    for (fact, count) in facts {
        let mut value = serde_json::to_value(fact).expect("semantic fact must serialize");
        normalize_volatile_identities(&mut value);
        let fact = serde_json::from_value(value).expect("canonical semantic fact must deserialize");
        *canonical.entry(fact).or_insert(0) += count;
    }
    canonical
}

fn semantic_delta(
    before: &SemanticFacts,
    after: &SemanticFacts,
) -> (Vec<CountedSemanticFact>, Vec<CountedSemanticFact>) {
    let before = canonical_facts(before);
    let after = canonical_facts(after);
    let keys = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for fact in keys {
        let before_count = before.get(&fact).copied().unwrap_or_default();
        let after_count = after.get(&fact).copied().unwrap_or_default();
        if before_count > after_count {
            removed.push(CountedSemanticFact {
                fact: fact.clone(),
                count: before_count - after_count,
            });
        }
        if after_count > before_count {
            added.push(CountedSemanticFact {
                fact,
                count: after_count - before_count,
            });
        }
    }
    (removed, added)
}

fn assert_reviewed_delta(
    case: &MatrixCase,
    before: &SemanticFacts,
    after: &SemanticFacts,
    scenario: Scenario,
) {
    let (removed, added) = semantic_delta(before, after);
    assert_eq!(
        removed, case.oracle.removed,
        "{:?} {scenario:?}: removed semantic facts differ from the reviewed oracle",
        case.kind
    );
    assert_eq!(
        added, case.oracle.added,
        "{:?} {scenario:?}: added semantic facts differ from the reviewed oracle",
        case.kind
    );
}

fn assert_repeat_outcome(kind: WriterCommandKind, result: &WriterResult) {
    assert_serialized_contender_outcome(kind, result, "repeat");
}

fn assert_concurrent_outcome(kind: WriterCommandKind, first: &WriterResult, second: &WriterResult) {
    assert!(
        matches!(first.lifecycle(), WriterLifecycle::Applied),
        "{kind:?}: lock owner must apply: {first:?}"
    );
    assert_serialized_contender_outcome(kind, second, "serialized contender");
}

fn assert_serialized_contender_outcome(
    kind: WriterCommandKind,
    result: &WriterResult,
    phase: &str,
) {
    let expected = match kind {
        WriterCommandKind::ConfigurationInitialize
        | WriterCommandKind::ExtensionInitialize
        | WriterCommandKind::FormCreate
        | WriterCommandKind::HelpCreate => Some(DiagnosticCode::InvalidRequest),
        WriterCommandKind::ExternalProcessorInitialize
        | WriterCommandKind::ExternalReportInitialize
        | WriterCommandKind::FormEdit
        | WriterCommandKind::TemplateCreate => Some(DiagnosticCode::AlreadyExists),
        WriterCommandKind::MetadataRemove
        | WriterCommandKind::FormRemove
        | WriterCommandKind::TemplateRemove => Some(DiagnosticCode::NotFound),
        _ => None,
    };
    match expected {
        None => assert!(
            matches!(result.lifecycle(), WriterLifecycle::Applied),
            "{kind:?}: {phase} must apply idempotently: {result:?}"
        ),
        Some(code) => {
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Rejected(_)),
                "{kind:?}: {phase} must return the documented typed rejection: {result:?}"
            );
            let actual = result
                .diagnostics()
                .iter()
                .map(WriterDiagnostic::code)
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                [code],
                "{kind:?}: {phase} returned the wrong typed rejection"
            );
        }
    }
}

fn expected_denial_code(kind: WriterCommandKind) -> DiagnosticCode {
    match kind {
        WriterCommandKind::ConfigurationInitialize | WriterCommandKind::ExtensionInitialize => {
            DiagnosticCode::InvalidRequest
        }
        WriterCommandKind::ExternalProcessorInitialize
        | WriterCommandKind::ExternalReportInitialize => DiagnosticCode::AlreadyExists,
        _ => DiagnosticCode::UnsupportedFormat,
    }
}

fn assert_all_xml_is_complete(root: &Path) {
    for path in files(root) {
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let text = std::str::from_utf8(&bytes)
            .unwrap_or_else(|error| panic!("{} is partial/non-UTF-8 XML: {error}", path.display()));
        roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|error| panic!("{} is partial XML: {error}", path.display()));
    }
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
                        family: family.to_string(),
                        namespace: node.tag_name().namespace().map(str::to_string),
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
                    family: family.to_string(),
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

fn make_semantically_unsupported(kind: WriterCommandKind, root: &Path) {
    const UUID: &str = "11111111-1111-4111-8111-111111111111";
    match kind {
        WriterCommandKind::DataCompositionCreate | WriterCommandKind::DataCompositionEdit => {
            write(
                &root.join("standalone/dcs.xml"),
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" version=\"1.1\"/>\n",
            );
        }
        WriterCommandKind::SpreadsheetCreate => {
            write(
                &root.join("standalone/mxl.xml"),
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\" version=\"1.1\"/>\n",
            );
        }
        WriterCommandKind::ExternalProcessorInitialize => {
            write(
                &root.join("external/MatrixProcessor.xml"),
                &format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><ExternalDataProcessor uuid=\"{UUID}\"><Properties><Name>MatrixProcessor</Name></Properties><ChildObjects/></ExternalDataProcessor></MetaDataObject>"),
            );
        }
        WriterCommandKind::ExternalReportInitialize => {
            write(
                &root.join("external/MatrixReport.xml"),
                &format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><ExternalReport uuid=\"{UUID}\"><Properties><Name>MatrixReport</Name></Properties><ChildObjects/></ExternalReport></MetaDataObject>"),
            );
        }
        WriterCommandKind::ExtensionBorrow | WriterCommandKind::ExtensionPatchMethod => {
            let target = root.join("extension/Configuration.xml");
            let text = String::from_utf8(fs::read(&target).unwrap()).unwrap();
            assert!(text.contains("version=\"2.20\""));
            write(
                &target,
                &text.replacen("version=\"2.20\"", "version=\"2.21\"", 1),
            );
        }
        WriterCommandKind::ExtensionInitialize => {
            write(
                &root.join("extension/Configuration.xml"),
                &format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><ConfigurationExtension uuid=\"{UUID}\"><Properties><Name>MatrixExtension</Name></Properties><ChildObjects/></ConfigurationExtension></MetaDataObject>"),
            );
        }
        _ => {
            let target = root.join("src/Configuration.xml");
            if let Ok(bytes) = fs::read(&target) {
                let text = String::from_utf8(bytes).unwrap();
                assert!(text.contains("version=\"2.20\""));
                write(
                    &target,
                    &text.replacen("version=\"2.20\"", "version=\"2.21\"", 1),
                );
            } else {
                write(
                    &target,
                    &format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration uuid=\"{UUID}\"><Properties><Name>MatrixConfiguration</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"),
                );
            }
        }
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
    cases.sort_by_key(|case| match case.kind {
        WriterCommandKind::InterfaceEdit => 0,
        WriterCommandKind::FormCompile => 1,
        _ => 2,
    });
    cases
}

fn case(command: WriterCommand) -> MatrixCase {
    let kind = command.kind();
    let oracle = load_reviewed_oracle(kind);
    MatrixCase {
        kind,
        command,
        oracle,
    }
}

fn load_reviewed_oracle(kind: WriterCommandKind) -> ReviewedSemanticOracle {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task8-writer-semantic-facts")
        .join(format!("{kind:?}.json"));
    let oracle: ReviewedSemanticOracle =
        serde_json::from_slice(&fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "missing reviewed semantic oracle {}: {error}",
                path.display()
            )
        }))
        .unwrap_or_else(|error| {
            panic!(
                "invalid reviewed semantic oracle {}: {error}",
                path.display()
            )
        });
    assert_eq!(oracle.schema_version, 1, "{}", path.display());
    assert_eq!(oracle.variant, format!("{kind:?}"), "{}", path.display());
    assert!(
        !oracle.provenance.source.trim().is_empty(),
        "{}",
        path.display()
    );
    assert!(
        !oracle.provenance.reviewed_evidence.is_empty(),
        "{}",
        path.display()
    );
    oracle
}

#[test]
fn exact_oracle_rejects_value_type_relation_namespace_and_add_remove_mutations() {
    fn changed(variant: WriterCommandKind, mutate: impl FnOnce(&mut ReviewedSemanticOracle)) {
        let original = load_reviewed_oracle(variant);
        let mut mutation = original.clone();
        mutate(&mut mutation);
        assert_ne!(
            mutation.removed, original.removed,
            "removed facts did not mutate"
        );
        assert_ne!(mutation.added, original.added, "added facts did not mutate");
    }

    let original = load_reviewed_oracle(WriterCommandKind::MetadataEdit);
    let mut wrong_value = original.clone();
    let value = wrong_value
        .added
        .iter_mut()
        .find_map(|entry| match &mut entry.fact {
            SemanticFact::Property { value, .. } => value.as_mut(),
            _ => None,
        })
        .expect("metadata edit oracle has a property value");
    value.push_str("-mutated");
    assert_ne!(wrong_value.added, original.added);

    let mut wrong_type = original.clone();
    let value_type = wrong_type
        .added
        .iter_mut()
        .find_map(|entry| match &mut entry.fact {
            SemanticFact::Property { value_type, .. } => Some(value_type),
            _ => None,
        })
        .expect("metadata edit oracle has a property type");
    value_type.push_str("-mutated");
    assert_ne!(wrong_type.added, original.added);

    let relation_original = load_reviewed_oracle(WriterCommandKind::ConfigurationInitialize);
    let mut wrong_relation = relation_original.clone();
    let relation = wrong_relation
        .added
        .iter_mut()
        .find_map(|entry| match &mut entry.fact {
            SemanticFact::Relation { value } => Some(value),
            _ => None,
        })
        .expect("configuration oracle has a relation");
    relation.push_str("-mutated");
    assert_ne!(wrong_relation.added, relation_original.added);

    let namespace_original = load_reviewed_oracle(WriterCommandKind::DataCompositionCreate);
    let mut wrong_namespace = namespace_original.clone();
    let namespace = wrong_namespace
        .added
        .iter_mut()
        .find_map(|entry| match &mut entry.fact {
            SemanticFact::StandaloneStructure { namespace, .. } => namespace.as_mut(),
            _ => None,
        })
        .expect("DCS oracle has a namespace");
    namespace.push_str("/mutated");
    assert_ne!(wrong_namespace.added, namespace_original.added);

    let mut wrong_add_remove = namespace_original.clone();
    let moved = wrong_add_remove
        .added
        .pop()
        .expect("DCS oracle has an addition");
    wrong_add_remove.removed.push(moved);
    assert_ne!(wrong_add_remove.added, namespace_original.added);
    assert_ne!(wrong_add_remove.removed, namespace_original.removed);

    // Keep a direct comparator mutation for removals as well (FormRemove is mandatory evidence).
    changed(WriterCommandKind::FormRemove, |oracle| {
        let moved = oracle
            .removed
            .pop()
            .expect("form removal oracle has removals");
        oracle.added.push(moved);
    });
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
