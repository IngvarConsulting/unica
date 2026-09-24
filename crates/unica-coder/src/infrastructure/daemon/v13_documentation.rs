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
        let is_locator =
            crate::infrastructure::application_ports::documentation_locator(&self.query).is_some();
        let result = crate::infrastructure::application_ports::canonical_v13_docs_search_with_limit(
            &self.context,
            &self.query,
            self.source.as_deref(),
            200,
            &cancellation,
        );
        if !result.ok {
            return result;
        }
        let binding = SearchCursorBinding {
            workspace_identity: self.workspace_identity_hash().as_str().to_owned(),
            query: self.query.clone(),
            scope: self.source.clone(),
            mode: if is_locator { "docs-document" } else { "docs" }.to_string(),
            kind: None,
            source_sets: Vec::new(),
            revisions: Vec::new(),
            result_fingerprint: None,
            page_limit: self.limit,
        };
        if is_locator {
            page_document_text(
                &self.cursors,
                result,
                binding,
                self.cursor.as_deref(),
                &cancellation,
            )
        } else {
            page_documentation(
                &self.cursors,
                result,
                binding,
                self.cursor.as_deref(),
                &cancellation,
            )
        }
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

const DOCUMENT_TEXT_FRAGMENT_BYTES: usize = 16 * 1024;
const DOCUMENT_CURSOR_PROBE: &str = "sc1.00000000000000000000000000000000";

#[derive(Clone, Copy)]
struct DocumentTextRange {
    start: usize,
    end: usize,
    total: usize,
    fragments: usize,
}

/// A locator still opens one document. Only its text is divided; all of the
/// owner's metadata accompanies every page. The cursor offset is a byte offset
/// in the original UTF-8 string, not an index into a normalized rendering.
fn page_document_text(
    cursors: &SearchCursorStore,
    mut result: DomainResult,
    mut binding: SearchCursorBinding,
    cursor_token: Option<&str>,
    cancellation: &CancellationToken,
) -> DomainResult {
    use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
    use crate::application::v13::view::PREFERRED_PAGE_BYTES;

    let refuse = |code, message| DomainResult::canonical_rejection(None, code, message);
    if cancellation.is_cancelled() {
        return refuse(RefusalCode::Cancelled, "docs document page cancelled");
    }
    let Some(mut data) = result.data.take() else {
        return refuse(
            RefusalCode::ProviderFailed,
            "docs provider returned no document",
        );
    };
    let Some(document) = data.get_mut("document").and_then(Value::as_object_mut) else {
        return refuse(
            RefusalCode::ProviderFailed,
            "docs provider returned an invalid document",
        );
    };
    let Some(Value::String(text)) =
        document.insert("text".to_string(), Value::String(String::new()))
    else {
        return refuse(
            RefusalCode::ProviderFailed,
            "docs document has no text string",
        );
    };
    result.data = Some(data);

    // Keep the existing one-response shape for short documents. An explicit
    // cursor must still be validated, even if the document later became short.
    if cursor_token.is_none() && text.len() <= PREFERRED_PAGE_BYTES {
        let mut whole = result.clone();
        whole.data.as_mut().expect("document data exists")["document"]["text"] = json!(text);
        if serde_json::to_vec(&whole)
            .expect("document result is serializable")
            .len()
            <= PREFERRED_PAGE_BYTES
        {
            return whole;
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(b"unica-v13-docs-document-page-v1\0");
    hasher.update(
        serde_json::to_vec(result.data.as_ref().expect("document data exists"))
            .expect("document metadata is serializable"),
    );
    hasher.update((text.len() as u64).to_le_bytes());
    for chunk in text.as_bytes().chunks(64 * 1024) {
        if cancellation.is_cancelled() {
            return refuse(RefusalCode::Cancelled, "docs document page cancelled");
        }
        hasher.update(chunk);
    }
    binding.result_fingerprint = Some(format!("docs-document-sha256-v1:{:x}", hasher.finalize()));

    let total = text.len();
    let probe = |start: usize, end: usize, stopped_by: &str| {
        serde_json::to_vec(&document_text_page_result(
            &result,
            &text[start..end],
            DocumentTextRange {
                start,
                end,
                total,
                fragments: binding.page_limit,
            },
            stopped_by,
            Some(DOCUMENT_CURSOR_PROBE),
        ))
        .expect("document page is serializable")
        .len()
    };
    // Maximum decimal widths and the longest stoppedBy value make this a
    // conservative bound for every later page with a real 36-byte token.
    let metadata_bytes = probe(total, total, "complete");
    if metadata_bytes > MAX_CANONICAL_RESULT_BYTES {
        return refuse(
            RefusalCode::ResultTooLarge,
            "docs document metadata exceeds the transport limit",
        );
    }
    let available = MAX_CANONICAL_RESULT_BYTES - metadata_bytes;
    if !text.is_empty() && available == 0 {
        return refuse(
            RefusalCode::ResultTooLarge,
            "docs document metadata leaves no room for text",
        );
    }
    // JSON escaping uses at most six output bytes per input byte. When the
    // metadata is almost at the transport limit, validate every scalar too:
    // a later UTF-8 character must not make an issued cursor unusable.
    let fragment_bytes = DOCUMENT_TEXT_FRAGMENT_BYTES.min((available / 6).max(1));
    if available < 24 {
        for character in text.chars() {
            if cancellation.is_cancelled() {
                return refuse(RefusalCode::Cancelled, "docs document page cancelled");
            }
            let encoded = serde_json::to_string(&character.to_string())
                .expect("document character is serializable");
            if encoded.len() - 2 > available {
                return refuse(
                    RefusalCode::ResultTooLarge,
                    "one docs document character exceeds the transport limit",
                );
            }
        }
    }

    let cursor = match cursor_token {
        None => None,
        Some(token) => match cursors.read(token, &binding) {
            Ok(stored) => Some((token, stored)),
            Err(error) => return refuse(error.code(), "docs document cursor is invalid or stale"),
        },
    };
    let start = cursor.as_ref().map_or(0, |(_, stored)| stored.offset);
    if start > total || !text.is_char_boundary(start) || (cursor.is_some() && start == total) {
        return refuse(
            RefusalCode::InvalidCursor,
            "docs document cursor is invalid",
        );
    }
    let mut end = start;
    let mut fragments = 0;
    let mut byte_stop = false;
    while end < total && fragments < binding.page_limit {
        if cancellation.is_cancelled() {
            return refuse(RefusalCode::Cancelled, "docs document page cancelled");
        }
        let next = next_document_fragment_end(&text, end, fragment_bytes);
        let bytes = probe(start, next, "complete");
        if bytes > MAX_CANONICAL_RESULT_BYTES {
            if end == start {
                return refuse(
                    RefusalCode::ResultTooLarge,
                    "one docs document fragment exceeds the transport limit",
                );
            }
            byte_stop = true;
            break;
        }
        if bytes > PREFERRED_PAGE_BYTES && end > start {
            byte_stop = true;
            break;
        }
        end = next;
        fragments += 1;
    }
    let more = end < total;
    let stopped_by = if !more {
        "complete"
    } else if byte_stop {
        "bytes"
    } else {
        "limit"
    };
    let mut page = document_text_page_result(
        &result,
        &text[start..end],
        DocumentTextRange {
            start,
            end,
            total,
            fragments,
        },
        stopped_by,
        None,
    );
    if cancellation.is_cancelled() {
        return refuse(RefusalCode::Cancelled, "docs document page cancelled");
    }
    if more {
        let issued = match cursor {
            Some((token, stored)) => cursors.insert_next(&stored, end, token),
            None => cursors.insert_first(binding, end),
        };
        let Some(issued) = issued else {
            return refuse(
                RefusalCode::ResultTooLarge,
                "docs document continuation could not be retained",
            );
        };
        page.cursor = Some(issued);
    }
    page
}

fn next_document_fragment_end(text: &str, start: usize, max_bytes: usize) -> usize {
    let remainder = &text[start..];
    let mut end = text.len().min(start.saturating_add(max_bytes));
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start {
        end = start + remainder.chars().next().expect("text remains").len_utf8();
    }
    // Search only this fragment's bounded window. Scanning the entire
    // remainder on each page makes one long line quadratic.
    remainder[..end - start]
        .find('\n')
        .map_or(end, |relative| start + relative + 1)
}

fn document_text_page_result(
    base: &DomainResult,
    text: &str,
    range: DocumentTextRange,
    stopped_by: &str,
    cursor: Option<&str>,
) -> DomainResult {
    let mut page = base.clone();
    page.data.as_mut().expect("document data exists")["document"]["text"] = json!(text);
    page.page = Some(json!({
        "startByte": range.start,
        "endByte": range.end,
        "totalBytes": range.total,
        "fragmentsReturned": range.fragments,
        "stoppedBy": stopped_by,
    }));
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

    fn document_binding(limit: usize) -> SearchCursorBinding {
        SearchCursorBinding {
            mode: "docs-document".into(),
            query: "platform-help:sample".into(),
            page_limit: limit,
            ..binding(limit)
        }
    }

    fn document_response(text: &str) -> DomainResult {
        let mut result = DomainResult::success("unica.docs opened the page");
        result.data = Some(json!({"document": {
            "provider": "platform-help", "corpus": "syntax", "sourceKind": "platform-help",
            "authority": "vendor", "language": "ru", "documentId": "platform-help:sample",
            "title": "Пример", "signature": null, "applicableVersion": "8.3.27",
            "text": text,
        }}));
        result
    }

    #[test]
    fn short_opened_document_preserves_the_whole_response() {
        let original = document_response("Первая строка\r\nВторая 🙂\n");
        let page = page_document_text(
            &SearchCursorStore::default(),
            original.clone(),
            document_binding(20),
            None,
            &CancellationToken::new(),
        );
        assert_eq!(page, original);
    }

    #[test]
    fn opened_document_pages_reassemble_exact_utf8_text_and_replay() {
        let original = format!("Заголовок\r\n{}\n\nКонец🙂", "Строка🙂".repeat(18_000));
        let store = SearchCursorStore::default();
        let cancellation = CancellationToken::new();
        let mut cursor = None;
        let mut bytes = Vec::new();
        let mut expected_start = 0;
        let mut pages = 0;
        loop {
            let page = page_document_text(
                &store,
                document_response(&original),
                document_binding(3),
                cursor.as_deref(),
                &cancellation,
            );
            assert!(page.ok, "{page:?}");
            let frame = page.page.as_ref().unwrap();
            assert_eq!(frame["startByte"], expected_start);
            assert_eq!(frame["totalBytes"], original.len());
            assert!(frame["fragmentsReturned"].as_u64().unwrap() <= 3);
            let text = page.data.as_ref().unwrap()["document"]["text"]
                .as_str()
                .unwrap();
            bytes.extend_from_slice(text.as_bytes());
            expected_start = frame["endByte"].as_u64().unwrap() as usize;
            assert_eq!(bytes.len(), expected_start);
            if pages == 0 {
                let token = page.cursor.as_deref().unwrap();
                let replay = page_document_text(
                    &store,
                    document_response(&original),
                    document_binding(3),
                    Some(token),
                    &cancellation,
                );
                let replay_again = page_document_text(
                    &store,
                    document_response(&original),
                    document_binding(3),
                    Some(token),
                    &cancellation,
                );
                assert_eq!(replay, replay_again);
                let mut changed_metadata = document_response(&original);
                changed_metadata.data.as_mut().unwrap()["document"]["title"] = json!("Иное");
                let stale = page_document_text(
                    &store,
                    changed_metadata,
                    document_binding(3),
                    Some(token),
                    &cancellation,
                );
                assert_eq!(stale.diagnostics[0]["code"], "stale_cursor");
                let wrong_locator = page_document_text(
                    &store,
                    document_response(&original),
                    SearchCursorBinding {
                        query: "platform-help:other".into(),
                        ..document_binding(3)
                    },
                    Some(token),
                    &cancellation,
                );
                assert_eq!(wrong_locator.diagnostics[0]["code"], "invalid_cursor");
            }
            pages += 1;
            cursor = page.cursor;
            if cursor.is_none() {
                assert_eq!(frame["stoppedBy"], "complete");
                break;
            }
            assert!(pages < 100);
        }
        assert!(pages > 2);
        assert_eq!(bytes, original.as_bytes());
    }

    #[test]
    fn document_over_transport_size_on_one_line_starts_with_a_useful_page() {
        let text =
            "🙂".repeat(crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES / 4 + 1);
        let page = page_document_text(
            &SearchCursorStore::default(),
            document_response(&text),
            document_binding(20),
            None,
            &CancellationToken::new(),
        );
        assert!(page.ok, "{page:?}");
        assert!(
            serde_json::to_vec(&page).unwrap().len()
                <= crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES
        );
        assert_eq!(page.page.as_ref().unwrap()["startByte"], 0);
        assert!(page.page.as_ref().unwrap()["endByte"].as_u64().unwrap() > 0);
        assert!(page.cursor.is_some());
        let first = page.data.as_ref().unwrap()["document"]["text"]
            .as_str()
            .unwrap();
        assert!(text.starts_with(first));
    }

    #[test]
    fn oversized_document_metadata_refuses_before_issuing_a_cursor() {
        let mut result = document_response("текст");
        result.data.as_mut().unwrap()["document"]["title"] =
            json!("x".repeat(crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES));
        let page = page_document_text(
            &SearchCursorStore::default(),
            result,
            document_binding(20),
            None,
            &CancellationToken::new(),
        );
        assert!(!page.ok);
        assert_eq!(page.diagnostics[0]["code"], "result_too_large");
        assert!(page.cursor.is_none());
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
    fn docs_locator_accepts_pagination_arguments() {
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
        let Preparation::Ready(prepared) =
            prepare(&request, Arc::new(SearchCursorStore::default()))
        else {
            panic!("document locator must accept a page limit");
        };
        assert_eq!(prepared.limit, 20);
    }

    #[test]
    fn prepared_document_locator_continues_across_calls_before_source_admission() {
        use crate::domain::documentation::{
            Authority, DocumentationContext, DocumentationCorpus, DocumentationDocument,
            DocumentationProvider, DocumentationProviderId, DocumentationSearchRequest,
            DocumentationSection, SourceKind,
        };
        use std::sync::Mutex;

        struct PageOwner {
            text: Arc<Mutex<String>>,
        }
        impl DocumentationProvider for PageOwner {
            fn id(&self) -> DocumentationProviderId {
                DocumentationProviderId::new("page-owner")
            }
            fn corpora(&self) -> Vec<DocumentationCorpus> {
                vec![DocumentationCorpus {
                    id: "syntax".into(),
                    source_kind: SourceKind::PlatformHelp,
                    authority: Authority::Vendor,
                }]
            }
            fn needs_network(&self) -> bool {
                false
            }
            fn search(
                &self,
                _: &DocumentationSearchRequest,
                _: &DocumentationContext,
            ) -> Vec<DocumentationSection> {
                Vec::new()
            }
            fn get(
                &self,
                document_id: &str,
                _: &str,
                _: &DocumentationContext,
            ) -> Option<Result<DocumentationDocument, String>> {
                (document_id == "platform-help:sample").then(|| {
                    Ok(DocumentationDocument {
                        provider: self.id(),
                        corpus: "syntax".into(),
                        source_kind: SourceKind::PlatformHelp,
                        authority: Authority::Vendor,
                        language: "ru".into(),
                        document_id: document_id.into(),
                        title: "Пример".into(),
                        signature: None,
                        applicable_version: "8.3.27".into(),
                        text: self.text.lock().unwrap().clone(),
                    })
                })
            }
        }

        let body = Arc::new(Mutex::new("Справка🙂".repeat(12_000)));
        let _provider =
            crate::infrastructure::application_ports::install_documentation_registry_stand_in(
                Arc::new(PageOwner {
                    text: Arc::clone(&body),
                }),
            );
        let workspace_dir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(workspace_dir.path()).unwrap();
        let cursors = Arc::new(SearchCursorStore::default());
        let invoke = |cursor: Option<&str>| {
            let mut arguments = json!({"query": "platform-help:sample", "limit": 1});
            if let Some(cursor) = cursor {
                arguments["cursor"] = json!(cursor);
            }
            let request = InvocationRequest::new(
                ToolIdentity::Docs,
                arguments,
                workspace.to_string_lossy(),
                7_000,
            )
            .unwrap();
            let Preparation::Ready(prepared) = prepare(&request, Arc::clone(&cursors)) else {
                panic!("document locator must prepare");
            };
            prepared.execute(CancellationToken::new())
        };

        let first = invoke(None);
        assert!(first.ok, "{first:?}");
        assert_eq!(first.page.as_ref().unwrap()["startByte"], 0);
        let token = first.cursor.as_deref().unwrap();
        let second = invoke(Some(token));
        assert!(second.ok, "{second:?}");
        assert_eq!(
            second.page.as_ref().unwrap()["startByte"],
            first.page.as_ref().unwrap()["endByte"]
        );
        assert_eq!(second, invoke(Some(token)));
        *body.lock().unwrap() = "другая справка".repeat(12_000);
        let changed = invoke(Some(token));
        assert_eq!(changed.diagnostics[0]["code"], "stale_cursor");
    }
}
