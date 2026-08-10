//! Поставщик `v8std` за общим контрактом документации (ADR-0032 п.4).
//!
//! Движок один на поставщика и фасады: тот же endpoint, тот же JSON-RPC
//! вызов `v8std_search` через `HttpClient`, что и у `unica.standards.*`.
//! Секция несёт смысл источника «стандарт разработки» и авторитетность
//! «сообщество» — это единственная секция реестра, не являющаяся справкой
//! платформы, и именно поэтому она обязана быть помечена (проектная записка,
//! «Поставщики и корпуса»).

use std::sync::Arc;

use serde_json::{json, Value};

use crate::domain::documentation::*;
use crate::infrastructure::documentation_policy::{DocumentationPolicy, NetworkAccess};
use crate::infrastructure::internal_adapters::{HttpClient, StandardsAdapter};

/// Встроенное умолчание endpoint сервера стандартов — то же, что всегда
/// использовали фасады `unica.standards.*`.
pub const BUILTIN_STANDARDS_ENDPOINT: &str = "https://ai.v8std.ru/mcp";

/// Попадание стандарта не имеет версии платформы: стандарт говорит, как
/// принято писать, и не свидетельствует о поведении платформы. Wire-маркер
/// вместо пустой строки: обязательность применимой версии у попадания —
/// ADR-0029 п.8, и «неверсионируемо» — честное значение, а пустота — нет.
pub const UNVERSIONED: &str = "unversioned";

/// Цепочка endpoint из проектной записки («Настройка»): `unica.local.toml`,
/// затем `unica.toml` (оба уже сведены внутри политики), затем переменная
/// окружения `UNICA_STANDARDS_MCP_URL`, затем встроенное умолчание.
pub fn resolve_standards_endpoint(policy: &DocumentationPolicy) -> String {
    resolve_standards_endpoint_with(policy, std::env::var("UNICA_STANDARDS_MCP_URL").ok())
}

/// Чистая половина цепочки: окружение приходит аргументом, чтобы порядок
/// слоёв проверялся без гонок за глобальное состояние процесса.
pub fn resolve_standards_endpoint_with(
    policy: &DocumentationPolicy,
    environment: Option<String>,
) -> String {
    policy
        .endpoint("v8std")
        .or(environment)
        .unwrap_or_else(|| BUILTIN_STANDARDS_ENDPOINT.to_string())
}

pub struct V8StdDocumentationProvider {
    pub endpoint: String,
    pub network: NetworkAccess,
    pub http: Arc<dyn HttpClient + Send + Sync>,
    /// Токен вызова MCP: сетевой поставщик проверяет его перед обращением,
    /// отмена не публикует результатов (ADR-0032 п.10).
    pub cancellation: crate::domain::cancellation::CancellationToken,
    /// Срок жизни кеша поиска стандартов в памяти процесса (ADR-0036 п.5).
    pub search_cache_ttl: std::time::Duration,
}

/// Продовый срок жизни кеша поиска стандартов — как у kb-кешей: часы держат
/// цену повторных запросов сессии около нуля, а протухание не переживает
/// процесс. На диск не пишется ничего.
pub const V8STD_SEARCH_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Ключ кеша поиска: endpoint, нормализованный запрос, лимит.
type SearchCacheKey = (String, String, usize);
/// Запись кеша: момент записи и сырое тело успешного ответа сервера.
type SearchCacheEntry = (std::time::Instant, String);

/// Кеш успешных ответов `v8std_search`. Политика и отмена проверяются ДО
/// кеша, неуспех не кешируется.
static V8STD_SEARCH_CACHE: std::sync::Mutex<
    std::collections::BTreeMap<SearchCacheKey, SearchCacheEntry>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Нормализация запроса по пробелам: сервер видит один канонический текст,
/// кеш не плодит записи на каждую расстановку пробелов.
pub(crate) fn normalized_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

const CORPUS: &str = "public-standards";

impl V8StdDocumentationProvider {
    fn section(
        &self,
        language: &str,
        status: DocumentationSectionStatus,
        hits: Vec<DocumentationHit>,
    ) -> DocumentationSection {
        DocumentationSection {
            provider: self.id(),
            corpus: CORPUS.to_string(),
            source_kind: SourceKind::DevelopmentStandard,
            authority: Authority::Community,
            language: language.to_string(),
            status,
            warnings: Vec::new(),
            hits,
        }
    }
}

impl DocumentationProvider for V8StdDocumentationProvider {
    fn id(&self) -> DocumentationProviderId {
        DocumentationProviderId::new("v8std")
    }

    fn corpora(&self) -> Vec<DocumentationCorpus> {
        vec![DocumentationCorpus {
            id: CORPUS.to_string(),
            source_kind: SourceKind::DevelopmentStandard,
            authority: Authority::Community,
        }]
    }

    fn needs_network(&self) -> bool {
        true
    }

    /// Стандарт целиком по адресу `https://v8std.ru/...` — тем же движком,
    /// что и фасад `unica.standards.explain` (`v8std_get_page`): текст —
    /// `body_markdown`, стандарт без версии платформы несёт `unversioned`.
    fn get(
        &self,
        document_id: &str,
        _language: &str,
        _context: &DocumentationContext,
    ) -> Option<Result<DocumentationDocument, String>> {
        if !document_id.starts_with("https://v8std.ru/") {
            return None;
        }
        if self.network == NetworkAccess::Deny {
            return Some(Err(
                "сетевой выход v8std запрещён политикой unica.toml".to_string()
            ));
        }
        if self.cancellation.is_cancelled() {
            return Some(Err(
                "вызов отменён до обращения к серверу стандартов".to_string()
            ));
        }
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "v8std_get_page",
                "arguments": { "id_or_alias_or_url": document_id },
            }
        });
        let body = match self.http.post_json(&self.endpoint, &payload) {
            Ok(body) => body,
            Err(error) => {
                return Some(Err(format!("сервер стандартов недоступен: {error}")));
            }
        };
        let outcome = StandardsAdapter::outcome_from_http_body(
            "get",
            &self.endpoint,
            "v8std_get_page",
            &body,
        );
        if !outcome.outcome.ok {
            return Some(Err(outcome.outcome.errors.join("; ")));
        }
        let inner = outcome
            .data
            .as_ref()
            .and_then(|data| data.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .and_then(|entry| entry.get("text"))
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Value>(text).ok());
        let Some(inner) = inner else {
            return Some(Err("ответ v8std_get_page не разбирается".to_string()));
        };
        if !inner.get("found").and_then(Value::as_bool).unwrap_or(false) {
            return Some(Err(format!("стандарт {document_id:?} не найден")));
        }
        let Some(page) = inner.get("page") else {
            return Some(Err("ответ v8std_get_page не несёт page".to_string()));
        };
        let text = page
            .get("body_markdown")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // Успешный «документ» без текста — не доказательство: планка
        // ADR-0029 п.4 держится на тексте открытой страницы.
        if text.trim().is_empty() {
            return Some(Err(format!(
                "страница стандарта {document_id:?} не несёт текста"
            )));
        }
        Some(Ok(DocumentationDocument {
            provider: self.id(),
            corpus: CORPUS.to_string(),
            source_kind: SourceKind::DevelopmentStandard,
            authority: Authority::Community,
            language: "ru".to_string(),
            // Канонический адрес из ответа принимается только как локатор
            // v8std: чужой адрес маршрутизировался бы другому поставщику.
            document_id: page
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| url.starts_with("https://v8std.ru/"))
                .unwrap_or(document_id)
                .to_string(),
            title: page
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(document_id)
                .to_string(),
            signature: None,
            applicable_version: UNVERSIONED.to_string(),
            text,
        }))
    }

    fn search(
        &self,
        request: &DocumentationSearchRequest,
        _context: &DocumentationContext,
    ) -> Vec<DocumentationSection> {
        // Запрет политики — отказ ДО транспорта: пользователь запретил
        // обращение сам, и ответ называет это, а не выдаёт за сбой.
        if self.network == NetworkAccess::Deny {
            return vec![self.section(
                &request.language,
                DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::PolicyDenied,
                    detail: "сетевой выход v8std запрещён политикой unica.toml".to_string(),
                },
                Vec::new(),
            )];
        }
        if self.cancellation.is_cancelled() {
            return vec![self.section(
                &request.language,
                DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::Timeout,
                    detail: "вызов отменён до обращения к серверу стандартов".to_string(),
                },
                Vec::new(),
            )];
        }
        let query = normalized_query(&request.query);
        let cache_key = (self.endpoint.clone(), query.clone(), request.limit);
        // Кеш читается ПОСЛЕ политики и отмены: запрет отвечает запретом, а
        // не вчерашним успехом (ADR-0036 п.5).
        let cached_body = {
            let cache = V8STD_SEARCH_CACHE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.get(&cache_key).and_then(|(born, body)| {
                (born.elapsed() < self.search_cache_ttl).then(|| body.clone())
            })
        };
        let from_cache = cached_body.is_some();
        let body = match cached_body {
            Some(body) => body,
            None => {
                let payload = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": "v8std_search",
                        "arguments": { "query": query, "limit": request.limit },
                    }
                });
                match self.http.post_json(&self.endpoint, &payload) {
                    Ok(body) => body,
                    Err(error) => {
                        return vec![self.section(
                            "ru",
                            DocumentationSectionStatus::Failed {
                                diagnostic: format!("сервер стандартов недоступен: {error}"),
                            },
                            Vec::new(),
                        )];
                    }
                }
            }
        };
        // Конверт JSON-RPC разбирает тот же код, что и у фасадов: один
        // разбор на движок, а не два расходящихся.
        let outcome = StandardsAdapter::outcome_from_http_body(
            "search",
            &self.endpoint,
            "v8std_search",
            &body,
        );
        if !outcome.outcome.ok {
            return vec![self.section(
                "ru",
                DocumentationSectionStatus::Failed {
                    diagnostic: outcome.outcome.errors.join("; "),
                },
                Vec::new(),
            )];
        }
        let hits = outcome
            .data
            .as_ref()
            .and_then(|data| data.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .and_then(|entry| entry.get("text"))
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|inner| inner.get("results").cloned())
            .and_then(|results| results.as_array().cloned());
        let Some(results) = hits else {
            return vec![self.section(
                "ru",
                DocumentationSectionStatus::Failed {
                    diagnostic: "ответ v8std_search не несёт results".to_string(),
                },
                Vec::new(),
            )];
        };
        // Разбор дошёл до results — ответ настоящий, его можно переиспользовать.
        // Неуспехи выше до этой строки не доходят и не кешируются.
        if !from_cache {
            let mut cache = V8STD_SEARCH_CACHE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.insert(cache_key, (std::time::Instant::now(), body.clone()));
        }
        // Локатор попадания — контракт владельца: чужой адрес из сетевого
        // ответа маршрутизировался бы другому поставщику при получении.
        // Такое попадание пропускается и называется предупреждением секции.
        let mut warnings = Vec::new();
        let mut hits: Vec<DocumentationHit> = Vec::new();
        for result in results.iter() {
            let url = result
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !url.starts_with("https://v8std.ru/") {
                warnings.push(format!("попадание с чужим адресом пропущено: {url:?}"));
                continue;
            }
            hits.push(DocumentationHit {
                rank: hits.len() as u32 + 1,
                provider_score: result
                    .get("score")
                    .and_then(Value::as_f64)
                    .unwrap_or_default() as f32,
                document_id: url.to_string(),
                title: result
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                signature: None,
                snippet: result
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                // У стандарта нет версии платформы: маркер, а не пустота и не
                // выдуманная версия (обязательность поля — ADR-0029 п.8).
                applicable_version: UNVERSIONED.to_string(),
            });
        }
        let status = if hits.is_empty() {
            DocumentationSectionStatus::Empty
        } else {
            DocumentationSectionStatus::Ok
        };
        // Локаль секции — `ru`: вендор стандартов поставляет их по-русски,
        // и ответившая локаль называется, как того требует контракт секций.
        let mut section = self.section("ru", status, hits);
        section.warnings = warnings;
        vec![section]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Канон ответа живого сервера (зонд 2026-08-09): JSON-RPC конверт, в
    /// `result.content[0].text` — JSON-строка с массивом `results`.
    const LIVE_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\n  \"query\": \"навигационная ссылка\",\n  \"results\": [\n    {\n      \"id\": \"std702\",\n      \"type\": \"standard\",\n      \"title\": \"Реквизит Ссылка #std702\",\n      \"description\": \"Через команду Еще пользователь может добавить реквизиты.\",\n      \"url\": \"https://v8std.ru/std/702/\",\n      \"markdown_url\": \"https://v8std.ru/std/702.md\",\n      \"score\": 3990.97\n    },\n    {\n      \"id\": \"std403\",\n      \"type\": \"standard\",\n      \"title\": \"Второй стандарт\",\n      \"description\": \"Описание второго.\",\n      \"url\": \"https://v8std.ru/std/403/\",\n      \"score\": 100.5\n    }\n  ]\n}"}]}}"#;

    struct FakeHttp {
        body: Result<String, String>,
        calls: AtomicUsize,
        last_payload: std::sync::Mutex<Option<Value>>,
    }

    impl HttpClient for FakeHttp {
        fn post_json(&self, _endpoint: &str, payload: &Value) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_payload.lock().expect("payload lock") = Some(payload.clone());
            self.body.clone()
        }
    }

    fn provider(
        network: NetworkAccess,
        body: Result<String, String>,
    ) -> (V8StdDocumentationProvider, Arc<FakeHttp>) {
        // Endpoint уникален на тест: кеш поиска — процессный, и общий адрес
        // делил бы записи между независимыми тестами.
        static ENDPOINTS: AtomicUsize = AtomicUsize::new(0);
        let unique = ENDPOINTS.fetch_add(1, Ordering::SeqCst);
        let http = Arc::new(FakeHttp {
            body,
            calls: AtomicUsize::new(0),
            last_payload: std::sync::Mutex::new(None),
        });
        (
            V8StdDocumentationProvider {
                endpoint: format!("http://stand.in/{unique}/mcp"),
                network,
                http: Arc::clone(&http) as Arc<dyn HttpClient + Send + Sync>,
                cancellation: crate::domain::cancellation::CancellationToken::default(),
                search_cache_ttl: Duration::from_secs(3600),
            },
            http,
        )
    }

    fn request() -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: "ссылка".to_string(),
            source_kinds: Vec::new(),
            limit: 20,
            language: "ru".to_string(),
        }
    }

    fn context() -> DocumentationContext {
        DocumentationContext {
            platform_version: None,
            installation_root: None,
        }
    }

    #[test]
    fn v8std_provider_maps_results_into_a_development_standard_section() {
        let (provider, _http) = provider(NetworkAccess::Allow, Ok(LIVE_BODY.to_string()));
        let sections = provider.search(&request(), &context());
        assert_eq!(sections.len(), 1, "один корпус — одна секция");
        let section = &sections[0];
        assert_eq!(section.corpus, "public-standards");
        assert_eq!(section.source_kind, SourceKind::DevelopmentStandard);
        assert_eq!(section.authority, Authority::Community);
        assert_eq!(section.language, "ru", "стандарты поставляются по-русски");
        assert!(matches!(section.status, DocumentationSectionStatus::Ok));
        assert_eq!(section.hits.len(), 2);
        let first = &section.hits[0];
        assert_eq!(first.rank, 1);
        assert_eq!(first.document_id, "https://v8std.ru/std/702/");
        assert_eq!(first.title, "Реквизит Ссылка #std702");
        assert_eq!(
            first.snippet,
            "Через команду Еще пользователь может добавить реквизиты."
        );
        assert_eq!(
            first.applicable_version, UNVERSIONED,
            "у стандарта нет версии платформы — маркер, а не пустота"
        );
        assert!(first.signature.is_none());
        assert!(first.provider_score > section.hits[1].provider_score);
    }

    /// Повторный одинаковый запрос сессии отвечается кешем процесса, без
    /// второго обращения к серверу (ADR-0036 п.5).
    #[test]
    fn a_repeated_search_answers_from_the_process_cache() {
        let (provider, http) = provider(NetworkAccess::Allow, Ok(LIVE_BODY.to_string()));
        let first = provider.search(&request(), &context());
        let second = provider.search(&request(), &context());
        assert_eq!(
            http.calls.load(Ordering::SeqCst),
            1,
            "второй одинаковый запрос обязан ответить из кеша"
        );
        assert!(matches!(second[0].status, DocumentationSectionStatus::Ok));
        assert_eq!(first[0].hits.len(), second[0].hits.len());
    }

    /// Протухшая запись кеша перечитывается: TTL — не вечность.
    #[test]
    fn an_expired_search_cache_entry_is_refetched() {
        let (mut provider, http) = provider(NetworkAccess::Allow, Ok(LIVE_BODY.to_string()));
        provider.search_cache_ttl = Duration::ZERO;
        provider.search(&request(), &context());
        provider.search(&request(), &context());
        assert_eq!(
            http.calls.load(Ordering::SeqCst),
            2,
            "нулевой срок жизни обязан перечитывать сервер"
        );
    }

    /// Запрет политики не читает кеш: `policy-denied` означает «обращение
    /// запрещено», а не «ответим вчерашним» (ADR-0036 п.5).
    #[test]
    fn policy_deny_does_not_answer_from_the_search_cache() {
        let (mut provider, http) = provider(NetworkAccess::Allow, Ok(LIVE_BODY.to_string()));
        provider.search(&request(), &context());
        assert_eq!(http.calls.load(Ordering::SeqCst), 1, "кеш прогрет");
        provider.network = NetworkAccess::Deny;
        let denied = provider.search(&request(), &context());
        assert!(
            matches!(
                denied[0].status,
                DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::PolicyDenied,
                    ..
                }
            ),
            "запрет не подменяется кешированным успехом: {:?}",
            denied[0].status
        );
        assert_eq!(http.calls.load(Ordering::SeqCst), 1, "сеть не тронута");
    }

    /// Запрос нормализуется по пробелам до отправки: сервер видит один
    /// канонический текст, и кеш не плодит записи на каждую расстановку
    /// пробелов.
    #[test]
    fn the_query_is_normalized_before_the_server_call() {
        let (provider, http) = provider(NetworkAccess::Allow, Ok(LIVE_BODY.to_string()));
        let mut spaced = request();
        spaced.query = "  навигационная \t ссылка  ".to_string();
        provider.search(&spaced, &context());
        let payload = http
            .last_payload
            .lock()
            .expect("payload lock")
            .clone()
            .expect("вызов состоялся");
        assert_eq!(
            payload["params"]["arguments"]["query"],
            json!("навигационная ссылка"),
            "{payload}"
        );
    }

    /// Запрет политики — отказ ДО транспорта: сам смысл `policy-denied` в том,
    /// что пользователь запретил обращение, а не в том, что оно не удалось.
    #[test]
    fn v8std_denied_by_policy_answers_policy_denied_without_touching_the_network() {
        let (provider, http) = provider(NetworkAccess::Deny, Ok(LIVE_BODY.to_string()));
        let sections = provider.search(&request(), &context());
        assert_eq!(sections.len(), 1);
        match &sections[0].status {
            DocumentationSectionStatus::Unavailable { reason, detail } => {
                assert_eq!(*reason, UnavailableReason::PolicyDenied);
                assert!(
                    detail.contains("unica.toml"),
                    "отказ обязан назвать файл политики, получено {detail}"
                );
            }
            other => panic!("ожидался Unavailable{{PolicyDenied}}, получено {other:?}"),
        }
        assert_eq!(
            http.calls.load(Ordering::SeqCst),
            0,
            "запрещённый поставщик не должен трогать сеть"
        );
    }

    /// Локатор попадания — контракт владельца: `unica.documentation.get`
    /// маршрутизирует по префиксу, и чужой адрес из сетевого ответа создал
    /// бы попадание, которое v8std отдать не может, а другой поставщик —
    /// может, но не то. Ответные адреса проверяются: чужой в поиске
    /// пропускается с предупреждением секции, чужой канонический адрес в
    /// получении не подменяет запрошенный.
    #[test]
    fn foreign_urls_from_the_network_response_do_not_become_locators() {
        let mixed = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"results\": [{\"title\": \"Свой\", \"description\": \"д\", \"url\": \"https://v8std.ru/std/1/\", \"score\": 2.0}, {\"title\": \"Чужой\", \"description\": \"д\", \"url\": \"https://kb.1ci.com/x/\", \"score\": 1.0}]}"}]}}"#;
        let (searcher, _http) = provider(NetworkAccess::Allow, Ok(mixed.to_string()));
        let sections = searcher.search(&request(), &context());
        let section = &sections[0];
        assert_eq!(
            section.hits.len(),
            1,
            "чужой адрес не становится локатором: {:?}",
            section.hits
        );
        assert_eq!(section.hits[0].document_id, "https://v8std.ru/std/1/");
        assert_eq!(
            section.warnings.len(),
            1,
            "пропуск чужого адреса обязан быть назван предупреждением"
        );
        assert!(
            section.warnings[0].contains("kb.1ci.com"),
            "предупреждение обязано назвать адрес, получено {}",
            section.warnings[0]
        );

        let foreign_canonical = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"found\": true, \"page\": {\"title\": \"Т\", \"url\": \"https://kb.1ci.com/x/\", \"body_markdown\": \"Текст.\"}}"}]}}"#;
        let (getter, _http) = provider(NetworkAccess::Allow, Ok(foreign_canonical.to_string()));
        let document = getter
            .get("https://v8std.ru/std/702/", "ru", &context())
            .expect("локатор наш")
            .expect("стандарт найден");
        assert_eq!(
            document.document_id, "https://v8std.ru/std/702/",
            "чужой канонический адрес не подменяет запрошенный локатор"
        );
    }

    /// Отмена вызова MCP приоритетна и для сервера стандартов: отменённый
    /// вызов не трогает сеть и отвечает диагностикой, а не результатом
    /// (ADR-0032 п.10).
    #[test]
    fn v8std_cancellation_stops_before_the_network_for_search_and_get() {
        let (mut cancelled, http) = provider(NetworkAccess::Allow, Ok(LIVE_BODY.to_string()));
        let token = crate::domain::cancellation::CancellationToken::default();
        token.cancel();
        cancelled.cancellation = token;

        let sections = cancelled.search(&request(), &context());
        assert!(
            matches!(
                sections[0].status,
                DocumentationSectionStatus::Unavailable { .. }
            ),
            "отменённый поиск — диагностичная секция, получено {:?}",
            sections[0].status
        );
        let error = cancelled
            .get("https://v8std.ru/std/702/", "ru", &context())
            .expect("локатор наш")
            .expect_err("отменённое получение — отказ владельца");
        assert!(
            error.contains("отмен"),
            "отказ обязан назвать отмену, получено {error}"
        );
        assert_eq!(
            http.calls.load(Ordering::SeqCst),
            0,
            "отменённый вызов не должен трогать сеть"
        );
    }

    #[test]
    fn v8std_transport_failure_is_failed_not_empty() {
        let (provider, _http) =
            provider(NetworkAccess::Allow, Err("connection refused".to_string()));
        let sections = provider.search(&request(), &context());
        assert_eq!(sections.len(), 1);
        match &sections[0].status {
            DocumentationSectionStatus::Failed { diagnostic } => assert!(
                diagnostic.contains("connection refused"),
                "диагностика обязана нести причину, получено {diagnostic}"
            ),
            other => panic!("ожидался Failed, получено {other:?}"),
        }
    }

    #[test]
    fn v8std_empty_results_are_empty_not_failed() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"results\": []}"}]}}"#;
        let (provider, _http) = provider(NetworkAccess::Allow, Ok(body.to_string()));
        let sections = provider.search(&request(), &context());
        assert!(matches!(
            sections[0].status,
            DocumentationSectionStatus::Empty
        ));
        assert!(sections[0].hits.is_empty());
    }

    /// Канон живого `v8std_get_page` (зонд 2026-08-09): `found` и `page` с
    /// `title`, `url`, `body_markdown`.
    const GET_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"found\": true, \"page\": {\"id\": \"std702\", \"title\": \"Реквизит Ссылка #std702\", \"url\": \"https://v8std.ru/std/702/\", \"body_markdown\": \"ID: #std702\\n\\n# Реквизит Ссылка\\n\\nПолный текст стандарта.\"}}"}]}}"#;

    /// `get` открывает стандарт целиком через тот же движок
    /// (`v8std_get_page`): текст — `body_markdown`, локатор канонизируется
    /// адресом страницы. Стандарт без версии платформы несёт маркер
    /// `unversioned`, как и попадания поиска.
    #[test]
    fn v8std_get_returns_the_standard_body_by_its_url() {
        let (provider, _http) = provider(NetworkAccess::Allow, Ok(GET_BODY.to_string()));
        let document = provider
            .get("https://v8std.ru/std/702/", "ru", &context())
            .expect("локатор наш")
            .expect("стандарт найден");
        assert_eq!(document.corpus, "public-standards");
        assert_eq!(document.source_kind, SourceKind::DevelopmentStandard);
        assert_eq!(document.authority, Authority::Community);
        assert_eq!(document.language, "ru");
        assert_eq!(document.title, "Реквизит Ссылка #std702");
        assert_eq!(document.applicable_version, UNVERSIONED);
        assert!(
            document.text.contains("Полный текст стандарта."),
            "текст — body_markdown, получено {}",
            document.text
        );

        assert!(
            provider
                .get("https://kb.1ci.com/x/", "ru", &context())
                .is_none(),
            "чужая база — не мой локатор"
        );
    }

    #[test]
    fn v8std_get_not_found_and_policy_deny_are_owner_failures() {
        let not_found = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"found\": false, \"candidates\": []}"}]}}"#;
        let (provider_missing, _http) = provider(NetworkAccess::Allow, Ok(not_found.to_string()));
        let error = provider_missing
            .get("https://v8std.ru/std/99999/", "ru", &context())
            .expect("локатор наш")
            .expect_err("ненайденный стандарт — отказ владельца");
        assert!(
            error.contains("не найден"),
            "отказ обязан назвать причину, получено {error}"
        );

        // Страница без body_markdown: успешный «документ» с пустым текстом —
        // не доказательство, а обман планки ADR-0029 п.4.
        let empty_page = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"found\": true, \"page\": {\"title\": \"Пустой\", \"url\": \"https://v8std.ru/std/1/\"}}"}]}}"#;
        let (provider_empty, _http) = provider(NetworkAccess::Allow, Ok(empty_page.to_string()));
        let error = provider_empty
            .get("https://v8std.ru/std/1/", "ru", &context())
            .expect("локатор наш")
            .expect_err("страница без текста — отказ владельца");
        assert!(
            error.contains("текст"),
            "отказ обязан назвать отсутствие текста, получено {error}"
        );

        let (provider_denied, http) = provider(NetworkAccess::Deny, Ok(GET_BODY.to_string()));
        let error = provider_denied
            .get("https://v8std.ru/std/702/", "ru", &context())
            .expect("локатор наш")
            .expect_err("запрет политики — отказ владельца");
        assert!(
            error.contains("unica.toml"),
            "отказ обязан назвать политику, получено {error}"
        );
        assert_eq!(
            http.calls.load(Ordering::SeqCst),
            0,
            "запрещённый поставщик не должен трогать сеть"
        );
    }

    /// Порядок слоёв цепочки endpoint: файл политики (локальный оверлей уже
    /// сведён внутри неё) старше окружения, окружение старше встроенного
    /// умолчания.
    #[test]
    fn resolve_standards_endpoint_prefers_config_then_env_then_builtin() {
        let dir = tempfile::tempdir().expect("каталог");
        std::fs::write(
            dir.path().join("unica.toml"),
            "[providers.v8std]\nendpoint = \"http://from.config/mcp\"\n",
        )
        .expect("файл политики");
        let configured =
            DocumentationPolicy::load(dir.path(), &["v8std", "kb-1ci"]).expect("политика");
        let empty = DocumentationPolicy::load(
            tempfile::tempdir().expect("каталог").path(),
            &["v8std", "kb-1ci"],
        )
        .expect("умолчания");

        assert_eq!(
            resolve_standards_endpoint_with(&configured, Some("http://from.env/mcp".to_string())),
            "http://from.config/mcp",
            "файл политики старше окружения"
        );
        assert_eq!(
            resolve_standards_endpoint_with(&empty, Some("http://from.env/mcp".to_string())),
            "http://from.env/mcp",
            "окружение старше встроенного умолчания"
        );
        assert_eq!(
            resolve_standards_endpoint_with(&empty, None),
            BUILTIN_STANDARDS_ENDPOINT
        );
    }
}
