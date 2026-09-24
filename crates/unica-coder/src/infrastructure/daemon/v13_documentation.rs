//! `unica.docs` отвечает до допуска рабочей области.
//!
//! Справка — вопрос о платформе и о стандартах разработки, а не о наборах
//! исходников: поставщики читают установку платформы, сеть и политику
//! проекта, и ни один из них не адресуется логическим адресом. Единственный
//! свод, который читал бы рабочее пространство, —
//! `configuration-documentation`, — до actor-owned nofollow-читателя отвечает
//! типизированным `unsupported_source`, поэтому аренда исходников актору
//! ничего не даёт и на входе не требуется.
//!
//! Без этого маршрута дорога «до рабочей области» обрывалась: `unica.view {}`
//! и словарь `unica.run` объясняют, чего не хватает, а спросить справку о том,
//! как это завести, было нельзя — допуск отвечал общим отказом. Маршрут
//! повторяет форму выгрузок ИБ: подготовка до admission, исполнение обычным
//! конвейером — со своим cutoff и Task, потому что поставщики ходят в сеть.

use super::protocol::InvocationRequest;
use crate::application::invocation_store::ToolIdentity;
use crate::application::result_store::{SearchCursorBinding, SearchCursorStore};
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct PreparedDocumentationSearch {
    query: String,
    source: Option<String>,
    limit: usize,
    cursor: Option<String>,
    cursors: Arc<SearchCursorStore>,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedDocumentationSearch>),
}

pub(super) fn prepare(request: &InvocationRequest, cursors: Arc<SearchCursorStore>) -> Preparation {
    if request.tool() != ToolIdentity::Docs {
        return Preparation::NotApplicable;
    }
    let arguments = request.arguments();
    let Some(query) = arguments.get("query").and_then(Value::as_str) else {
        return reject(
            RefusalCode::BadValue,
            "docs requires string argument `query`",
        );
    };
    let source = match arguments.get("source") {
        None => None,
        Some(Value::String(source)) => Some(source.clone()),
        Some(_) => return reject(RefusalCode::BadValue, "docs source must be a string"),
    };
    let limit = match arguments.get("limit") {
        None => 20,
        Some(value) => match value.as_u64().and_then(|limit| usize::try_from(limit).ok()) {
            Some(limit @ 1..=50) => limit,
            _ => {
                return reject(
                    RefusalCode::BadValue,
                    "docs limit must be from 1 through 50",
                )
            }
        },
    };
    let cursor = match arguments.get("cursor") {
        None => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        Some(_) => return reject(RefusalCode::BadValue, "docs cursor must be a string"),
    };
    if crate::infrastructure::application_ports::documentation_locator(query).is_some()
        && (arguments.contains_key("limit") || cursor.is_some())
    {
        return reject(
            RefusalCode::BadValue,
            "a documentation page locator does not accept pagination",
        );
    }
    // Корень нужен политике сети и закреплённой версии платформы, а не
    // допуску исходников: пустой каталог — это отсутствие ограничений, и
    // обнаружение здесь не отказывает по их отсутствию.
    let context = match discover_workspace(Some(PathBuf::from(request.workspace_hint()))) {
        Ok(context) => context,
        Err(error) => {
            return reject(
                RefusalCode::ProviderUnavailable,
                format!("workspace discovery failed: {error}"),
            )
        }
    };
    Preparation::Ready(Arc::new(PreparedDocumentationSearch {
        query: query.to_string(),
        source,
        limit,
        cursor,
        cursors,
        context,
    }))
}

fn reject(code: RefusalCode, message: impl Into<String>) -> Preparation {
    Preparation::Rejected(Box::new(DomainResult::canonical_rejection(
        None, code, message,
    )))
}

impl PreparedDocumentationSearch {
    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-documentation-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        let result = crate::infrastructure::application_ports::canonical_v13_docs_search_with_limit(
            &self.context,
            &self.query,
            self.source.as_deref(),
            200,
            &cancellation,
        );
        if !result.ok
            || crate::infrastructure::application_ports::documentation_locator(&self.query)
                .is_some()
        {
            return result;
        }
        let binding = SearchCursorBinding {
            workspace_identity: self.workspace_identity_hash().as_str().to_owned(),
            query: self.query.clone(),
            scope: self.source.clone(),
            mode: "docs".to_string(),
            kind: None,
            source_sets: Vec::new(),
            revisions: Vec::new(),
            result_fingerprint: None,
            page_limit: self.limit,
        };
        page_documentation(
            &self.cursors,
            result,
            binding,
            self.cursor.as_deref(),
            &cancellation,
        )
    }
}

fn page_documentation(
    cursors: &SearchCursorStore,
    mut result: DomainResult,
    mut binding: SearchCursorBinding,
    cursor_token: Option<&str>,
    cancellation: &CancellationToken,
) -> DomainResult {
    use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
    use crate::application::v13::view::PREFERRED_PAGE_BYTES;

    if cancellation.is_cancelled() {
        return DomainResult::canonical_rejection(
            None,
            RefusalCode::Cancelled,
            "docs page cancelled",
        );
    }
    let Some(data) = result.data.take() else {
        return DomainResult::canonical_rejection(
            None,
            RefusalCode::ProviderFailed,
            "docs providers returned no sections",
        );
    };
    let Some(sections) = data.get("sections").and_then(Value::as_array) else {
        return DomainResult::canonical_rejection(
            None,
            RefusalCode::ProviderFailed,
            "docs providers returned invalid sections",
        );
    };
    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-docs-page-v1\0");
    hasher.update(serde_json::to_vec(&data).expect("docs evidence is serializable"));
    binding.result_fingerprint = Some(format!("docs-sha256-v1:{:x}", hasher.finalize()));

    let mut section_metadata = sections.clone();
    let mut all_section_hits = Vec::new();
    for (section_index, section) in section_metadata.iter_mut().enumerate() {
        let Some(raw_hits) = section.get_mut("hits").and_then(Value::as_array_mut) else {
            return DomainResult::canonical_rejection(
                None,
                RefusalCode::ProviderFailed,
                "docs section has no hits array",
            );
        };
        all_section_hits.push((section_index, std::mem::take(raw_hits).into_iter()));
        section["matches"]["returned"] = json!(0);
    }
    // A docs question usually spans several corpora. Interleave them before
    // applying the page budget, while preserving each provider's ranking
    // inside its section. One indivisible large hit can still fill a page.
    let mut hits = Vec::new();
    loop {
        let mut advanced = false;
        for (section_index, remaining) in &mut all_section_hits {
            if let Some(hit) = remaining.next() {
                hits.push((*section_index, hit));
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    let probe = |selected: &[(usize, Value)]| {
        serde_json::to_vec(&docs_page_result(
            &result,
            &section_metadata,
            selected,
            "limit",
            Some("sc1.00000000000000000000000000000000"),
        ))
        .expect("docs page is serializable")
        .len()
    };
    if probe(&[]) > MAX_CANONICAL_RESULT_BYTES {
        return DomainResult::canonical_rejection(
            None,
            RefusalCode::ResultTooLarge,
            "docs page metadata exceeds the transport limit",
        );
    }
    for hit in &hits {
        if cancellation.is_cancelled() {
            return DomainResult::canonical_rejection(
                None,
                RefusalCode::Cancelled,
                "docs page cancelled",
            );
        }
        if probe(std::slice::from_ref(hit)) > MAX_CANONICAL_RESULT_BYTES {
            return DomainResult::canonical_rejection(
                None,
                RefusalCode::ResultTooLarge,
                "one docs result exceeds the transport limit",
            );
        }
    }
    let cursor = match cursor_token {
        None => None,
        Some(token) => match cursors.read(token, &binding) {
            Ok(stored) => Some((token, stored)),
            Err(error) => {
                return DomainResult::canonical_rejection(
                    None,
                    error.code(),
                    "docs cursor is invalid or stale",
                )
            }
        },
    };
    let offset = cursor.as_ref().map_or(0, |(_, stored)| stored.offset);
    if cursor.is_some() && offset >= hits.len() {
        return DomainResult::canonical_rejection(
            None,
            RefusalCode::InvalidCursor,
            "docs cursor is invalid",
        );
    }
    let mut page_hits = Vec::new();
    let mut byte_stop = false;
    for hit in hits.iter().skip(offset).take(binding.page_limit) {
        let mut candidate = page_hits.clone();
        candidate.push(hit.clone());
        let bytes = probe(&candidate);
        if bytes > MAX_CANONICAL_RESULT_BYTES {
            byte_stop = true;
            break;
        }
        if bytes > PREFERRED_PAGE_BYTES && !page_hits.is_empty() {
            byte_stop = true;
            break;
        }
        page_hits.push(hit.clone());
    }
    let next_offset = offset.saturating_add(page_hits.len());
    let more = next_offset < hits.len();
    let stopped_by = if !more {
        "complete"
    } else if byte_stop {
        "bytes"
    } else {
        "limit"
    };
    if cancellation.is_cancelled() {
        return DomainResult::canonical_rejection(
            None,
            RefusalCode::Cancelled,
            "docs page cancelled",
        );
    }
    let mut page = docs_page_result(&result, &section_metadata, &page_hits, stopped_by, None);
    if more {
        let issued = match cursor {
            Some((token, stored)) => cursors.insert_next(&stored, next_offset, token),
            None => cursors.insert_first(binding, next_offset),
        };
        let Some(issued) = issued else {
            return DomainResult::canonical_rejection(
                None,
                RefusalCode::ResultTooLarge,
                "docs continuation could not be retained",
            );
        };
        page.cursor = Some(issued);
    }
    page
}

fn docs_page_result(
    base: &DomainResult,
    section_metadata: &[Value],
    hits: &[(usize, Value)],
    stopped_by: &str,
    cursor: Option<&str>,
) -> DomainResult {
    let mut sections = section_metadata.to_vec();
    for (section_index, hit) in hits {
        sections[*section_index]["hits"]
            .as_array_mut()
            .expect("docs hits remain arrays")
            .push(hit.clone());
        let returned = sections[*section_index]["hits"].as_array().unwrap().len();
        sections[*section_index]["matches"]["returned"] = json!(returned);
    }
    let search_complete = sections
        .iter()
        .all(|section| section["searchComplete"] == true);
    let mut page = base.clone();
    page.data = Some(json!({"sections": sections, "searchComplete": search_complete}));
    page.page = Some(json!({"stoppedBy": stopped_by}));
    page.cursor = cursor.map(str::to_owned);
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(limit: usize) -> SearchCursorBinding {
        SearchCursorBinding {
            workspace_identity: "workspace".into(),
            query: "search".into(),
            scope: None,
            mode: "docs".into(),
            kind: None,
            source_sets: Vec::new(),
            revisions: Vec::new(),
            result_fingerprint: None,
            page_limit: limit,
        }
    }

    fn response(snippet_bytes: usize) -> DomainResult {
        let section = |provider: &str, count: usize, complete: bool| {
            json!({
                "provider": provider,
                "corpus": provider,
                "sourceKind": if provider == "standards" { "development-standard" } else { "platform-help" },
                "authority": "vendor",
                "language": "ru",
                "status": "ok",
                "diagnostic": null,
                "warnings": [],
                "searchComplete": complete,
                "matches": {"returned": count, "total": count,
                    "relation": if complete { "exact" } else { "lowerBound" }},
                "hits": (1..=count).map(|rank| json!({
                    "rank": rank, "documentId": format!("{provider}:{rank}"),
                    "title": format!("Page {rank}"), "snippet": "x".repeat(snippet_bytes),
                })).collect::<Vec<_>>(),
            })
        };
        let mut result = DomainResult::success("unica.docs completed");
        result.data = Some(json!({"sections": [
            section("platform", 25, false),
            {"provider": "unavailable", "corpus": "unavailable", "sourceKind": "platform-help",
             "authority": "vendor", "language": "ru", "status": "unavailable",
             "diagnostic": {"reason": "not-configured"}, "warnings": [],
             "searchComplete": false, "matches": {"returned": 0, "total": 0, "relation": "lowerBound"}, "hits": []},
            section("standards", 30, true),
        ]}));
        result
    }

    #[test]
    fn docs_pages_mixed_sections_and_rejects_changed_answers() {
        let store = SearchCursorStore::default();
        let cancellation = CancellationToken::new();
        let mut cursor = None;
        let mut ids = Vec::new();
        for index in 0..3 {
            let page = page_documentation(
                &store,
                response(8),
                binding(20),
                cursor.as_deref(),
                &cancellation,
            );
            assert!(page.ok, "{page:?}");
            assert_eq!(page.data.as_ref().unwrap()["searchComplete"], false);
            let sections = page.data.as_ref().unwrap()["sections"].as_array().unwrap();
            assert_eq!(sections.len(), 3);
            assert_eq!(sections[1]["status"], "unavailable");
            assert_eq!(sections[1]["hits"].as_array().unwrap().len(), 0);
            if index == 0 {
                assert_eq!(sections[0]["hits"].as_array().unwrap().len(), 10);
                assert_eq!(sections[2]["hits"].as_array().unwrap().len(), 10);
                assert_eq!(sections[0]["sourceKind"], "platform-help");
                assert_eq!(sections[2]["sourceKind"], "development-standard");
            }
            for section in sections {
                let hits = section["hits"].as_array().unwrap();
                assert_eq!(section["matches"]["returned"], hits.len());
                ids.extend(
                    hits.iter()
                        .map(|hit| hit["documentId"].as_str().unwrap().to_string()),
                );
            }
            if index == 0 {
                let token = page.cursor.as_deref().unwrap();
                let replay = page_documentation(
                    &store,
                    response(8),
                    binding(20),
                    Some(token),
                    &cancellation,
                );
                let replay_again = page_documentation(
                    &store,
                    response(8),
                    binding(20),
                    Some(token),
                    &cancellation,
                );
                assert_eq!(replay, replay_again);
                let changed = page_documentation(
                    &store,
                    response(9),
                    binding(20),
                    Some(token),
                    &cancellation,
                );
                assert_eq!(changed.diagnostics[0]["code"], "stale_cursor");
            }
            cursor = page.cursor;
        }
        assert!(cursor.is_none());
        assert_eq!(ids.len(), 55);
        let unique = ids.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), 55);
        assert!(unique.contains("platform:1"));
        assert!(unique.contains("platform:25"));
        assert!(unique.contains("standards:1"));
        assert!(unique.contains("standards:30"));
    }

    #[test]
    fn late_oversized_docs_hit_refuses_before_first_cursor() {
        let store = SearchCursorStore::default();
        let cancellation = CancellationToken::new();
        let mut result = response(8);
        result.data.as_mut().unwrap()["sections"][2]["hits"][29]["snippet"] =
            json!("x".repeat(crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES));
        let page = page_documentation(&store, result, binding(20), None, &cancellation);
        assert!(!page.ok);
        assert_eq!(page.diagnostics[0]["code"], "result_too_large");
        assert!(page.cursor.is_none());
    }

    #[test]
    fn one_docs_hit_over_page_target_is_returned_whole() {
        let page = page_documentation(
            &SearchCursorStore::default(),
            response(80_000),
            binding(20),
            None,
            &CancellationToken::new(),
        );
        assert!(page.ok, "{page:?}");
        assert_eq!(page.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert_eq!(
            page.data.as_ref().unwrap()["sections"][0]["hits"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            page.data.as_ref().unwrap()["sections"][0]["hits"][0]["snippet"]
                .as_str()
                .unwrap()
                .len(),
            80_000
        );
        assert!(page.cursor.is_some());
    }

    #[test]
    fn prepared_docs_search_continues_across_calls_before_source_admission() {
        use crate::domain::documentation::{
            Authority, DocumentationContext, DocumentationCorpus, DocumentationHit,
            DocumentationProvider, DocumentationProviderId, DocumentationSearchRequest,
            DocumentationSection, DocumentationSectionStatus, SourceKind,
        };

        struct Pages;
        impl DocumentationProvider for Pages {
            fn id(&self) -> DocumentationProviderId {
                DocumentationProviderId::new("pages")
            }
            fn corpora(&self) -> Vec<DocumentationCorpus> {
                vec![
                    DocumentationCorpus {
                        id: "syntax".into(),
                        source_kind: SourceKind::PlatformHelp,
                        authority: Authority::Vendor,
                    },
                    DocumentationCorpus {
                        id: "standards".into(),
                        source_kind: SourceKind::DevelopmentStandard,
                        authority: Authority::Community,
                    },
                ]
            }
            fn needs_network(&self) -> bool {
                false
            }
            fn search(
                &self,
                request: &DocumentationSearchRequest,
                _: &DocumentationContext,
            ) -> Vec<DocumentationSection> {
                [
                    ("syntax", SourceKind::PlatformHelp, Authority::Vendor),
                    (
                        "standards",
                        SourceKind::DevelopmentStandard,
                        Authority::Community,
                    ),
                ]
                .into_iter()
                .map(|(corpus, source_kind, authority)| DocumentationSection {
                    provider: self.id(),
                    corpus: corpus.into(),
                    source_kind,
                    authority,
                    language: "ru".into(),
                    status: DocumentationSectionStatus::Ok,
                    warnings: Vec::new(),
                    hits: (1..=25)
                        .take(request.limit)
                        .map(|rank| DocumentationHit {
                            rank,
                            provider_score: 1.0,
                            document_id: format!("pages:{corpus}:{rank}"),
                            title: format!("Page {rank}"),
                            signature: None,
                            snippet: "found".into(),
                            applicable_version: "8.3".into(),
                        })
                        .collect(),
                })
                .collect()
            }
        }

        let _provider =
            crate::infrastructure::application_ports::install_documentation_registry_stand_in(
                Arc::new(Pages),
            );
        let workspace = tempfile::tempdir().unwrap();
        let cursors = Arc::new(SearchCursorStore::default());
        let mut cursor = None;
        let mut documents = Vec::new();
        for index in 0..5 {
            let mut arguments = json!({"query": "Page", "limit": 10});
            if let Some(token) = cursor.as_ref() {
                arguments["cursor"] = json!(token);
            }
            let request = InvocationRequest::new(
                ToolIdentity::Docs,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let Preparation::Ready(prepared) = prepare(&request, Arc::clone(&cursors)) else {
                panic!("docs search must prepare");
            };
            let page = prepared.execute(CancellationToken::new());
            assert!(page.ok, "{page:?}");
            let sections = page.data.as_ref().unwrap()["sections"].as_array().unwrap();
            assert_eq!(sections.len(), 2);
            if index == 0 {
                assert_eq!(sections[0]["sourceKind"], "platform-help");
                assert_eq!(sections[1]["sourceKind"], "development-standard");
                assert_eq!(sections[0]["hits"].as_array().unwrap().len(), 5);
                assert_eq!(sections[1]["hits"].as_array().unwrap().len(), 5);
            }
            for section in sections {
                documents.extend(
                    section["hits"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|hit| hit["documentId"].as_str().unwrap().to_string()),
                );
            }
            cursor = page.cursor;
        }
        assert!(cursor.is_none());
        assert_eq!(documents.len(), 50);
        let unique = documents
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), 50);
        assert!(unique.contains("pages:syntax:1"));
        assert!(unique.contains("pages:standards:25"));
    }

    #[test]
    fn docs_locator_rejects_pagination_arguments() {
        let workspace = tempfile::tempdir().unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Docs,
            json!({"query": "https://v8std.ru/standard/1", "limit": 20}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let Preparation::Rejected(result) =
            prepare(&request, Arc::new(SearchCursorStore::default()))
        else {
            panic!("document locator cannot accept a page limit");
        };
        assert_eq!(result.diagnostics[0]["code"], "bad_value");
    }
}
