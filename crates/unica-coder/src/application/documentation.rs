//! Public documentation search: renders `DocumentationRegistry` output into
//! the typed `data` of `unica.documentation.search` (ADR-0023, ADR-0029).
//!
//! This module owns only the section/hit-to-JSON projection and the
//! partial-success rule. The registry is assembled in the composition root
//! (`infrastructure::application_ports`), not here, so tests can inject
//! stand-in providers (ADR-0029 point 5).

use serde_json::{json, Value};

use crate::domain::documentation::*;

/// Wire status string plus its diagnostic payload. `Ok`/`Empty` carry no
/// payload; `Unavailable` carries its reason and detail; `Failed` carries its
/// diagnostic message. The caller decides success from the typed status
/// (`DocumentationSectionStatus`), never from this string.
fn status_fields(status: &DocumentationSectionStatus) -> (&'static str, Value) {
    match status {
        DocumentationSectionStatus::Ok => ("ok", Value::Null),
        DocumentationSectionStatus::Empty => ("empty", Value::Null),
        DocumentationSectionStatus::Unavailable { reason, detail } => (
            "unavailable",
            json!({ "reason": reason.as_str(), "detail": detail }),
        ),
        DocumentationSectionStatus::Failed { diagnostic } => {
            ("failed", json!({ "diagnostic": diagnostic }))
        }
    }
}

/// Poll every applicable provider and project their sections into the
/// public `data` shape unchanged: no cross-section sorting, merging or
/// deletion (ADR-0029 point 8). A provider's own failure stays inside its
/// own section and never removes another provider's sections from the
/// response (ADR-0029 point 10).
///
/// Опрос идёт параллельно: секции поставщиков независимы по контракту, и
/// федеративный вызов стоит как самый медленный поставщик, а не как их
/// сумма. Публикуются секции строго в порядке реестра — параллелен только
/// опрос, а не порядок ответа. Паника поставщика всплывает как при
/// последовательном опросе.
///
/// Search is successful if at least one applicable provider answered `ok` or
/// `empty`; if every section is `unavailable`/`failed`, the call reports an
/// error instead of an empty-looking success (ADR-0029 point 10).
///
/// A blank query is refused here, before any provider is polled: substring
/// matching makes the empty needle match every page, so a blank query would
/// be answered with arbitrary pages presented as confident hits. That is
/// worse than an error, and it is provider-neutral, so it belongs on this
/// side of the contract rather than in one provider.
pub fn search(
    registry: &DocumentationRegistry,
    request: &DocumentationSearchRequest,
    context: &DocumentationContext,
) -> Result<Value, String> {
    if request.query.trim().is_empty() {
        return Err("unica.documentation.search requires a non-blank query".to_string());
    }
    // Применимость по `sourceKinds` решается ДО опроса, по объявленным
    // корпусам: сетевой поставщик стандартов иначе ходил бы в сеть на
    // каждый вопрос о платформе. Пустой фильтр применим ко всем.
    let applicable: Vec<_> = registry
        .providers()
        .filter(|provider| {
            request.source_kinds.is_empty()
                || provider
                    .corpora()
                    .iter()
                    .any(|corpus| request.source_kinds.contains(&corpus.source_kind))
        })
        .collect();
    let mut polled: Vec<Vec<DocumentationSection>> = Vec::with_capacity(applicable.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = applicable
            .iter()
            .map(|provider| scope.spawn(move || provider.search(request, context)))
            .collect();
        for handle in handles {
            polled.push(
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            );
        }
    });
    let mut sections = Vec::new();
    let mut any_usable = false;
    for provider_sections in polled {
        for section in provider_sections {
            // Поставщик с корпусами разных смыслов отвечает всеми секциями;
            // публикуются из них только подходящие фильтру.
            if !request.source_kinds.is_empty()
                && !request.source_kinds.contains(&section.source_kind)
            {
                continue;
            }
            let (status, diagnostic) = status_fields(&section.status);
            if matches!(
                section.status,
                DocumentationSectionStatus::Ok | DocumentationSectionStatus::Empty
            ) {
                any_usable = true;
            }
            let hits: Vec<Value> = section
                .hits
                .iter()
                .map(|hit| {
                    json!({
                        "rank": hit.rank,
                        "providerScore": hit.provider_score,
                        "documentId": hit.document_id,
                        "title": hit.title,
                        "signature": hit.signature,
                        "snippet": hit.snippet,
                        "applicableVersion": hit.applicable_version,
                    })
                })
                .collect();
            sections.push(json!({
                "provider": section.provider.to_string(),
                "corpus": section.corpus,
                "sourceKind": section.source_kind.as_str(),
                "authority": section.authority.as_str(),
                // Локаль, на которой секция ответила, а не запрошенная в
                // `request.language`: справка платформы поставляется не во всех
                // локалях, и подмена обязана быть видимой в ответе.
                "language": section.language,
                "status": status,
                "diagnostic": diagnostic,
                // Неполнота, не отменяющая успеха: непрочитавшийся контейнер
                // при непустой выдаче. Пустой массив в норме.
                "warnings": section.warnings,
                "hits": hits,
            }));
        }
    }
    if !any_usable {
        return Err("ни один поставщик документации не дал результата".to_string());
    }
    Ok(json!({ "sections": sections }))
}

/// Документ целиком по устойчивому локатору попадания (`unica.documentation.get`).
/// Владельца локатора находит первый не-`None` ответ в порядке реестра:
/// форматы локаторов поставщиков не пересекаются. Отказ владельца — отказ
/// вызова; локатор без владельца — отказ, называющий сам локатор.
pub fn get(
    registry: &DocumentationRegistry,
    document_id: &str,
    language: &str,
    context: &DocumentationContext,
) -> Result<Value, String> {
    if document_id.trim().is_empty() {
        return Err("unica.documentation.get requires a non-blank documentId".to_string());
    }
    for provider in registry.providers() {
        let Some(answer) = provider.get(document_id, language, context) else {
            continue;
        };
        // Отказ владельца — отказ вызова: подмена другой страницей или
        // другим поставщиком недопустима, локатор один и владелец один.
        let document = answer?;
        return Ok(json!({
            "document": {
                "provider": document.provider.to_string(),
                "corpus": document.corpus,
                "sourceKind": document.source_kind.as_str(),
                "authority": document.authority.as_str(),
                "language": document.language,
                "documentId": document.document_id,
                "title": document.title,
                "signature": document.signature,
                "applicableVersion": document.applicable_version,
                "text": document.text,
            }
        }));
    }
    Err(format!(
        "ни один поставщик документации не владеет локатором {document_id:?}"
    ))
}

#[cfg(test)]
mod tests {
    // `super::*` already re-exports `crate::domain::documentation::*` (this
    // module imports it above), so a second direct
    // `use crate::domain::documentation::*;` here is redundant and trips
    // `unused_imports` under `-D warnings`. Same situation as
    // `platform_help::provider::tests`.
    use super::*;
    use std::sync::Arc;

    struct Stub {
        id: &'static str,
        section: DocumentationSection,
    }

    impl DocumentationProvider for Stub {
        fn id(&self) -> DocumentationProviderId {
            DocumentationProviderId::new(self.id)
        }
        fn corpora(&self) -> Vec<DocumentationCorpus> {
            Vec::new()
        }
        fn needs_network(&self) -> bool {
            false
        }
        fn search(
            &self,
            _: &DocumentationSearchRequest,
            _: &DocumentationContext,
        ) -> Vec<DocumentationSection> {
            vec![self.section.clone()]
        }
    }

    fn ok_section(id: &str) -> DocumentationSection {
        DocumentationSection {
            provider: DocumentationProviderId::new(id),
            corpus: "syntax-context".to_string(),
            source_kind: SourceKind::PlatformHelp,
            authority: Authority::Vendor,
            // Намеренно НЕ `ru` из `request()`: локаль в ответе обязана быть
            // локалью секции, а не запроса, и совпадение с запросом скрыло бы
            // проекцию, подставляющую туда `request.language`.
            language: "root".to_string(),
            status: DocumentationSectionStatus::Ok,
            warnings: Vec::new(),
            hits: vec![DocumentationHit {
                rank: 1,
                provider_score: 1.0,
                document_id: format!("{id}:syntax-context:page.html"),
                title: "Заголовок".to_string(),
                signature: Some("Имя()".to_string()),
                snippet: "текст".to_string(),
                applicable_version: "8.3.27.2074".to_string(),
            }],
        }
    }

    fn failed_section(id: &str) -> DocumentationSection {
        DocumentationSection {
            status: DocumentationSectionStatus::Failed {
                diagnostic: "сломалось".to_string(),
            },
            hits: Vec::new(),
            ..ok_section(id)
        }
    }

    fn empty_section(id: &str) -> DocumentationSection {
        DocumentationSection {
            status: DocumentationSectionStatus::Empty,
            hits: Vec::new(),
            ..ok_section(id)
        }
    }

    fn unavailable_section(id: &str) -> DocumentationSection {
        DocumentationSection {
            status: DocumentationSectionStatus::Unavailable {
                reason: UnavailableReason::NotConfigured,
                detail: "установка платформы не разрешена для рабочего пространства".to_string(),
            },
            hits: Vec::new(),
            ..ok_section(id)
        }
    }

    fn request() -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: "x".to_string(),
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

    /// Стаб рандеву: сигналит о своём старте и ждёт старта соседа с
    /// таймаутом. При последовательном опросе рандеву не состоится — стаб
    /// отвечает диагностичной секцией-маркером, и тест падает по значению,
    /// а не зависает.
    struct Rendezvous {
        id: &'static str,
        my_start: std::sync::mpsc::Sender<()>,
        other_started: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl DocumentationProvider for Rendezvous {
        fn id(&self) -> DocumentationProviderId {
            DocumentationProviderId::new(self.id)
        }
        fn corpora(&self) -> Vec<DocumentationCorpus> {
            Vec::new()
        }
        fn needs_network(&self) -> bool {
            false
        }
        fn search(
            &self,
            _: &DocumentationSearchRequest,
            _: &DocumentationContext,
        ) -> Vec<DocumentationSection> {
            let _ = self.my_start.send(());
            let saw_the_other = self
                .other_started
                .lock()
                .expect("замок рандеву")
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok();
            let mut section = ok_section(self.id);
            if !saw_the_other {
                section.status = DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::NotConfigured,
                    detail: "сосед не стартовал: опрос последовательный".to_string(),
                };
                section.hits = Vec::new();
            }
            vec![section]
        }
    }

    /// Поставщики независимы (ADR-0029: без слияния и пересортировки секций),
    /// поэтому опрашиваются параллельно: федеративный вызов стоит как самый
    /// медленный поставщик, а не как их сумма.
    #[test]
    fn providers_are_polled_concurrently() {
        let (first_start, first_started) = std::sync::mpsc::channel();
        let (second_start, second_started) = std::sync::mpsc::channel();
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Rendezvous {
                id: "first",
                my_start: first_start,
                other_started: std::sync::Mutex::new(second_started),
            }) as Arc<dyn DocumentationProvider>,
            Arc::new(Rendezvous {
                id: "second",
                my_start: second_start,
                other_started: std::sync::Mutex::new(first_started),
            }),
        ])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections.len(), 2);
        for section in sections {
            assert_eq!(
                section["status"], "ok",
                "рандеву обязано состояться у обоих: {section}"
            );
        }
        // Порядок публикации — порядок реестра и при параллельном опросе.
        assert_eq!(sections[0]["provider"], "first");
        assert_eq!(sections[1]["provider"], "second");
    }

    #[test]
    fn sections_follow_registry_order_and_carry_provenance() {
        // Provider ids are declared in DESCENDING alphabetical order ("zeta"
        // before "alpha"): the brief's own "first"/"second" happen to already
        // be ascending, so a `search` that (wrongly) sorted sections
        // ascending by provider name would coincidentally pass. This mirrors
        // the fix `domain::documentation::tests::registry_preserves_declared_order`
        // already applies to the same blind spot.
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Stub {
                id: "zeta",
                section: ok_section("zeta"),
            }) as Arc<dyn DocumentationProvider>,
            Arc::new(Stub {
                id: "alpha",
                section: ok_section("alpha"),
            }),
        ])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections[0]["provider"], "zeta");
        assert_eq!(sections[1]["provider"], "alpha");
        assert_eq!(sections[0]["sourceKind"], "platform-help");
        assert_eq!(sections[0]["authority"], "vendor");
        // `corpus` is named by the invariant as one of the four provenance
        // fields, yet nothing asserted it: renaming or dropping the key in the
        // projection above used to pass every test in this file.
        assert_eq!(sections[0]["corpus"], "syntax-context");
        // Локаль ответа — часть происхождения секции: запрос шёл на `ru`
        // (см. `request()`), а секция объявляет `root`, и проекция обязана
        // публиковать именно её.
        assert_eq!(sections[0]["language"], "root");
        assert_eq!(sections[0]["hits"][0]["applicableVersion"], "8.3.27.2074");
    }

    #[test]
    fn one_failed_provider_does_not_hide_the_other() {
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Stub {
                id: "broken",
                section: failed_section("broken"),
            }) as Arc<dyn DocumentationProvider>,
            Arc::new(Stub {
                id: "working",
                section: ok_section("working"),
            }),
        ])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections[0]["status"], "failed");
        assert_eq!(sections[1]["status"], "ok");
        assert_eq!(sections[1]["hits"].as_array().expect("попадания").len(), 1);
    }

    /// A blank `query` passes the port's `as_str` check, and
    /// `title.contains("")` is true for every page, so every page scores 1.0
    /// and the caller is handed the first twenty pages of each corpus by path
    /// order, presented as confident matches. Refusing is the only honest
    /// answer, and it has to happen before any provider runs: a provider that
    /// answered `ok` would make the call succeed.
    #[test]
    fn blank_query_is_refused_before_any_provider_runs() {
        let registry = DocumentationRegistry::new(vec![Arc::new(Stub {
            id: "working",
            section: ok_section("working"),
        })
            as Arc<dyn DocumentationProvider>])
        .expect("реестр");
        for blank in ["", "   ", "\t\n"] {
            let request = DocumentationSearchRequest {
                query: blank.to_string(),
                ..request()
            };
            let error = match search(&registry, &request, &context()) {
                Ok(value) => panic!("пустой запрос обязан быть отклонён, получено {value}"),
                Err(error) => error,
            };
            assert!(
                error.contains("query"),
                "отказ обязан назвать аргумент, получено {error}"
            );
        }
    }

    /// Поставщик со счётчиком опросов и объявленным смыслом корпуса: фильтр
    /// `sourceKinds` обязан решать применимость по `corpora()`, не опрашивая
    /// неподходящего поставщика вовсе, — сетевой поставщик стандартов иначе
    /// ходил бы в сеть на каждый вопрос о платформе.
    struct KindStub {
        id: &'static str,
        kind: SourceKind,
        section: DocumentationSection,
        polled: std::sync::atomic::AtomicUsize,
    }

    impl DocumentationProvider for KindStub {
        fn id(&self) -> DocumentationProviderId {
            DocumentationProviderId::new(self.id)
        }
        fn corpora(&self) -> Vec<DocumentationCorpus> {
            vec![DocumentationCorpus {
                id: "corpus".to_string(),
                source_kind: self.kind,
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
            self.polled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            vec![self.section.clone()]
        }
    }

    /// Фильтр по смыслу источника: секции неподходящего смысла не публикуются,
    /// неподходящий поставщик не опрашивается, а правило успеха считает только
    /// применимые секции — единственная применимая `Ok`-секция делает вызов
    /// успешным, хотя неприменимый поставщик даже не отвечал.
    #[test]
    fn a_source_kind_filter_skips_non_matching_providers_and_sections() {
        let platform = std::sync::Arc::new(KindStub {
            id: "platform",
            kind: SourceKind::PlatformHelp,
            section: ok_section("platform"),
            polled: std::sync::atomic::AtomicUsize::new(0),
        });
        let standards_section = DocumentationSection {
            source_kind: SourceKind::DevelopmentStandard,
            ..ok_section("standards")
        };
        let standards = std::sync::Arc::new(KindStub {
            id: "standards",
            kind: SourceKind::DevelopmentStandard,
            section: standards_section,
            polled: std::sync::atomic::AtomicUsize::new(0),
        });
        let registry = DocumentationRegistry::new(vec![
            std::sync::Arc::clone(&platform) as Arc<dyn DocumentationProvider>,
            std::sync::Arc::clone(&standards) as Arc<dyn DocumentationProvider>,
        ])
        .expect("реестр");

        let filtered = DocumentationSearchRequest {
            source_kinds: vec![SourceKind::DevelopmentStandard],
            ..request()
        };
        let value = search(&registry, &filtered, &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(
            sections.len(),
            1,
            "публикуются только секции подходящего смысла"
        );
        assert_eq!(sections[0]["provider"], "standards");
        assert_eq!(sections[0]["sourceKind"], "development-standard");
        assert_eq!(
            platform.polled.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "неприменимый поставщик не должен опрашиваться вовсе"
        );
        assert_eq!(
            standards.polled.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "применимый поставщик опрошен ровно раз"
        );

        // Пустой фильтр — прежнее поведение: обе секции, оба опроса.
        let unfiltered = search(&registry, &request(), &context()).expect("результат");
        assert_eq!(
            unfiltered["sections"]
                .as_array()
                .expect("массив секций")
                .len(),
            2,
            "без фильтра публикуются секции всех поставщиков"
        );
    }

    #[test]
    fn all_providers_failed_is_an_error() {
        let registry = DocumentationRegistry::new(vec![Arc::new(Stub {
            id: "broken",
            section: failed_section("broken"),
        })
            as Arc<dyn DocumentationProvider>])
        .expect("реестр");
        assert!(search(&registry, &request(), &context()).is_err());
    }

    #[test]
    fn empty_status_counts_as_success_not_just_ok() {
        // `any_usable` requires `Ok | Empty`; none of the other three tests
        // build an Empty-only registry (they use only `ok_section`/
        // `failed_section`), so a mutation dropping the `Empty` arm from
        // that check would still pass all three. This is not a corner case:
        // `PlatformSyntaxHelpProvider::search` returns exactly this status
        // whenever the installation is found but the query has no hits
        // (ADR-0029 point 10).
        let registry = DocumentationRegistry::new(vec![Arc::new(Stub {
            id: "quiet",
            section: empty_section("quiet"),
        })
            as Arc<dyn DocumentationProvider>])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections[0]["status"], "empty");
        assert!(sections[0]["hits"]
            .as_array()
            .expect("попадания")
            .is_empty());
    }

    /// Предупреждения секции — неполнота, не отменяющая успеха: контейнер, не
    /// прочитавшийся при непустой выдаче. Статус остаётся `ok`, но проекция
    /// обязана донести предупреждения до публичного результата: молчание
    /// выдавало бы частичный корпус за целый.
    #[test]
    fn warnings_reach_the_public_result_next_to_an_ok_status() {
        let mut section = ok_section("partial");
        section.warnings =
            vec!["1cv8_ru.hbk: контейнер обрезан: в файле меньше данных".to_string()];
        let registry = DocumentationRegistry::new(vec![Arc::new(Stub {
            id: "partial",
            section,
        })
            as Arc<dyn DocumentationProvider>])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections[0]["status"], "ok");
        assert_eq!(
            sections[0]["warnings"][0], "1cv8_ru.hbk: контейнер обрезан: в файле меньше данных",
            "предупреждение обязано дойти до публичного результата"
        );
    }

    #[test]
    fn unavailable_status_carries_reason_and_detail_in_diagnostic() {
        // This is the first response shape a user without a configured
        // platform installation sees — `PlatformSyntaxHelpProvider::search`
        // returns exactly this when `installation_root` is `None` — yet no
        // other test builds an `Unavailable` section, so a swapped key or a
        // dropped field in `status_fields` would ship unnoticed. Paired with
        // a working provider so `search` succeeds and the shape is
        // inspectable (an `Unavailable`-only registry would instead exercise
        // `all_providers_failed_is_an_error`'s path).
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Stub {
                id: "unset",
                section: unavailable_section("unset"),
            }) as Arc<dyn DocumentationProvider>,
            Arc::new(Stub {
                id: "working",
                section: ok_section("working"),
            }),
        ])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections[0]["status"], "unavailable");
        assert_eq!(sections[0]["diagnostic"]["reason"], "not-configured");
        assert_eq!(
            sections[0]["diagnostic"]["detail"],
            "установка платформы не разрешена для рабочего пространства"
        );
    }

    struct GetStub {
        id: &'static str,
        answer: Option<Result<DocumentationDocument, String>>,
    }

    impl DocumentationProvider for GetStub {
        fn id(&self) -> DocumentationProviderId {
            DocumentationProviderId::new(self.id)
        }
        fn corpora(&self) -> Vec<DocumentationCorpus> {
            Vec::new()
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
            _document_id: &str,
            _language: &str,
            _context: &DocumentationContext,
        ) -> Option<Result<DocumentationDocument, String>> {
            self.answer.clone()
        }
    }

    fn document(id: &str) -> DocumentationDocument {
        DocumentationDocument {
            provider: DocumentationProviderId::new(id),
            corpus: "syntax-context".to_string(),
            source_kind: SourceKind::PlatformHelp,
            authority: Authority::Vendor,
            language: "ru".to_string(),
            document_id: "platform-syntax-help:syntax-context:page.html".to_string(),
            title: "Заголовок".to_string(),
            signature: Some("Имя()".to_string()),
            applicable_version: "8.3.27.2074".to_string(),
            text: "Полный текст открытой страницы.".to_string(),
        }
    }

    /// Владельца находит первый не-`None` в порядке реестра, и его документ
    /// доходит до публичного результата всеми полями происхождения.
    #[test]
    fn get_skips_a_non_owner_and_projects_the_owners_document() {
        let registry = DocumentationRegistry::new(vec![
            Arc::new(GetStub {
                id: "stranger",
                answer: None,
            }) as Arc<dyn DocumentationProvider>,
            Arc::new(GetStub {
                id: "owner",
                answer: Some(Ok(document("owner"))),
            }),
        ])
        .expect("реестр");
        let value = get(
            &registry,
            "platform-syntax-help:syntax-context:page.html",
            "ru",
            &context(),
        )
        .expect("документ");
        let doc = &value["document"];
        assert_eq!(doc["provider"], "owner");
        assert_eq!(doc["corpus"], "syntax-context");
        assert_eq!(doc["sourceKind"], "platform-help");
        assert_eq!(doc["authority"], "vendor");
        assert_eq!(doc["language"], "ru");
        assert_eq!(
            doc["documentId"],
            "platform-syntax-help:syntax-context:page.html"
        );
        assert_eq!(doc["title"], "Заголовок");
        assert_eq!(doc["signature"], "Имя()");
        assert_eq!(doc["applicableVersion"], "8.3.27.2074");
        assert_eq!(doc["text"], "Полный текст открытой страницы.");
    }

    /// Локатор без владельца — отказ, называющий сам локатор: молчаливый
    /// пустой успех оставил бы вызывающего гадать, что пошло не так.
    #[test]
    fn a_locator_no_provider_owns_is_refused_naming_it() {
        let registry = DocumentationRegistry::new(vec![Arc::new(GetStub {
            id: "stranger",
            answer: None,
        })
            as Arc<dyn DocumentationProvider>])
        .expect("реестр");
        let error = get(&registry, "alien:whatever", "ru", &context())
            .expect_err("чужой локатор обязан отклоняться");
        assert!(
            error.contains("alien:whatever"),
            "отказ обязан назвать локатор, получено {error}"
        );
    }

    /// Отказ владельца — отказ вызова с его причиной: страница исчезла или
    /// сеть запрещена политикой, и подмена другой страницей недопустима.
    #[test]
    fn the_owners_failure_is_the_calls_failure() {
        let registry = DocumentationRegistry::new(vec![Arc::new(GetStub {
            id: "owner",
            answer: Some(Err(
                "страница не отдана: перенаправление на корень".to_string()
            )),
        })
            as Arc<dyn DocumentationProvider>])
        .expect("реестр");
        let error = get(&registry, "https://kb.1ci.com/x/", "ru", &context())
            .expect_err("отказ владельца обязан стать отказом вызова");
        assert!(
            error.contains("страница не отдана"),
            "причина владельца обязана дойти, получено {error}"
        );
    }

    #[test]
    fn a_blank_locator_is_refused_before_any_provider_runs() {
        let registry = DocumentationRegistry::new(vec![Arc::new(GetStub {
            id: "owner",
            answer: Some(Ok(document("owner"))),
        })
            as Arc<dyn DocumentationProvider>])
        .expect("реестр");
        for blank in ["", "   "] {
            let error = get(&registry, blank, "ru", &context())
                .expect_err("пустой локатор обязан отклоняться");
            assert!(
                error.contains("documentId"),
                "отказ обязан назвать аргумент, получено {error}"
            );
        }
    }
}
