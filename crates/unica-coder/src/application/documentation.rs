//! Public documentation search: renders `DocumentationRegistry` output into
//! the typed `data` of `unica.documentation.search` (ADR-0023, ADR-0029).
//!
//! This module owns only the section/hit-to-JSON projection and the
//! partial-success rule. The registry is assembled in the composition root
//! (`infrastructure::application_ports`), not here, so tests can inject
//! stand-in providers (ADR-0029 point 8).

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

/// Poll every provider in registry order and project their sections into the
/// public `data` shape unchanged: no cross-section sorting, merging or
/// deletion (ADR-0029 point 11). A provider's own failure stays inside its
/// own section and never removes another provider's sections from the
/// response (ADR-0029 point 13).
///
/// Search is successful if at least one applicable provider answered `ok` or
/// `empty`; if every section is `unavailable`/`failed`, the call reports an
/// error instead of an empty-looking success (ADR-0029 point 13).
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
    let mut sections = Vec::new();
    let mut any_usable = false;
    for provider in registry.providers() {
        for section in provider.search(request, context) {
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
                "status": status,
                "diagnostic": diagnostic,
                "hits": hits,
            }));
        }
    }
    if !any_usable {
        return Err("ни один поставщик документации не дал результата".to_string());
    }
    Ok(json!({ "sections": sections }))
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
            status: DocumentationSectionStatus::Ok,
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
        // (ADR-0029 point 13).
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
}
