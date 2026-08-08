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
pub fn search(
    registry: &DocumentationRegistry,
    request: &DocumentationSearchRequest,
    context: &DocumentationContext,
) -> Result<Value, String> {
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
}
