//! Offline rule descriptions from the exact analyzer revision in tools.lock.
//! The snapshot is refreshed by scripts/ci/sync-diagnostic-rule-docs.py.

use serde::Deserialize;
use std::sync::LazyLock;
use unica_documentation::model::*;

#[derive(Deserialize)]
struct Snapshot {
    repository: String,
    #[serde(rename = "sourceCommit")]
    source_commit: String,
    version: String,
    documents: Vec<RuleDocument>,
}
#[derive(Deserialize)]
struct RuleDocument {
    code: String,
    language: String,
    path: String,
    text: String,
}
static SNAPSHOT: LazyLock<Snapshot> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/diagnostic-rules.json"))
        .expect("compiled diagnostic rule snapshot must be valid")
});

pub struct DiagnosticRulesProvider;

/// A code is one exact identifier, never a conjunction or a free-text search.
pub fn is_rule_code(query: &str) -> bool {
    let mut chars = query.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric())
}

impl DiagnosticRulesProvider {
    fn corpus(&self) -> String {
        format!("bsl-analyzer-{}-rules", SNAPSHOT.version)
    }
    fn locator(&self, document: &RuleDocument) -> String {
        format!(
            "{}/blob/{}/{}",
            SNAPSHOT.repository, SNAPSHOT.source_commit, document.path
        )
    }
    fn title(document: &RuleDocument) -> String {
        document
            .text
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .unwrap_or(&document.code)
            .to_string()
    }
    fn section(&self, language: &str) -> DocumentationSection {
        DocumentationSection::empty(
            self.id(),
            &self.corpus(),
            SourceKind::DiagnosticRule,
            Authority::Community,
            language,
        )
    }
}

impl DocumentationProvider for DiagnosticRulesProvider {
    fn id(&self) -> DocumentationProviderId {
        DocumentationProviderId::new("bsl-analyzer-rules")
    }
    fn corpora(&self) -> Vec<DocumentationCorpus> {
        vec![DocumentationCorpus {
            id: self.corpus(),
            source_kind: SourceKind::DiagnosticRule,
            authority: Authority::Community,
        }]
    }
    fn needs_network(&self) -> bool {
        false
    }
    fn search(
        &self,
        request: &DocumentationSearchRequest,
        _: &DocumentationContext,
    ) -> Vec<DocumentationSection> {
        let code = request.query.trim();
        let mut section = self.section(&request.language);
        if !is_rule_code(code) {
            section.status = DocumentationSectionStatus::Failed {
                diagnostic:
                    "diagnostic-rule requires one exact diagnostic code; query each code separately"
                        .to_string(),
            };
            return vec![section];
        }
        let document = SNAPSHOT
            .documents
            .iter()
            .find(|d| d.code == code && d.language == request.language)
            .or_else(|| {
                SNAPSHOT
                    .documents
                    .iter()
                    .find(|d| d.code == code && d.language == "en")
            });
        if let Some(document) = document {
            section.language.clone_from(&document.language);
            section.status = DocumentationSectionStatus::Ok;
            section.hits.push(DocumentationHit {
                rank: 1,
                provider_score: 1.0,
                document_id: self.locator(document),
                title: Self::title(document),
                signature: Some(document.code.clone()),
                snippet: document.text.chars().take(400).collect(),
                applicable_version: SNAPSHOT.version.clone(),
            });
        } else {
            section.warnings.push(format!("The bundled bsl-analyzer {} documentation snapshot contains no description for `{code}`. This does not establish that the diagnostic rule or its basis is absent.", SNAPSHOT.version));
        }
        vec![section]
    }
    fn get(
        &self,
        document_id: &str,
        _: &str,
        _: &DocumentationContext,
    ) -> Option<Result<DocumentationDocument, String>> {
        // Exact snapshot locators only: no arbitrary GitHub URL is fetched.
        let document = SNAPSHOT
            .documents
            .iter()
            .find(|document| self.locator(document) == document_id)?;
        Some(Ok(DocumentationDocument {
            provider: self.id(),
            corpus: self.corpus(),
            source_kind: SourceKind::DiagnosticRule,
            authority: Authority::Community,
            language: document.language.clone(),
            document_id: document_id.to_string(),
            title: Self::title(document),
            signature: Some(document.code.clone()),
            applicable_version: SNAPSHOT.version.clone(),
            text: document.text.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> DocumentationContext {
        DocumentationContext {
            platform_version: None,
            installation_root: None,
        }
    }
    fn request(code: &str) -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: code.to_string(),
            source_kinds: vec![SourceKind::DiagnosticRule],
            limit: 3,
            language: "ru".to_string(),
        }
    }
    #[test]
    fn each_reported_code_has_its_own_pinned_offline_description() {
        let provider = DiagnosticRulesProvider;
        assert!(!provider.needs_network());
        for code in [
            "UnresolvedMethodCall",
            "AmbiguousFieldInQuery",
            "UnresolvedField",
        ] {
            let sections = provider.search(&request(code), &context());
            assert_eq!(sections[0].hits.len(), 1);
            let hit = &sections[0].hits[0];
            assert_eq!(hit.signature.as_deref(), Some(code));
            assert_eq!(hit.applicable_version, SNAPSHOT.version);
            assert!(hit
                .document_id
                .contains(&format!("/blob/{}/", SNAPSHOT.source_commit)));
            let document = provider
                .get(&hit.document_id, "en", &context())
                .unwrap()
                .unwrap();
            assert_eq!(document.language, "ru");
            assert!(document.text.contains(code));
            assert!(document.text.len() > hit.snippet.len());
            assert!(provider
                .get(
                    &hit.document_id.replace(&SNAPSHOT.source_commit, "main"),
                    "ru",
                    &context()
                )
                .is_none());
        }
    }
    #[test]
    fn unknown_and_combined_codes_do_not_prove_that_rules_are_absent() {
        let provider = DiagnosticRulesProvider;
        let unknown = provider.search(&request("NoSuchDiagnostic123"), &context());
        assert!(unknown[0].hits.is_empty());
        assert!(unknown[0].warnings[0].contains("does not establish"));
        for query in [
            "UnresolvedMethodCall UnresolvedField",
            "UnresolvedMethodCall,UnresolvedField",
            "../UnresolvedField",
        ] {
            let sections = provider.search(&request(query), &context());
            assert!(matches!(
                sections[0].status,
                DocumentationSectionStatus::Failed { .. }
            ));
            assert!(sections[0].hits.is_empty());
        }
    }
    #[test]
    fn every_snapshot_page_round_trips_in_its_actual_language() {
        let provider = DiagnosticRulesProvider;
        for doc in &SNAPSHOT.documents {
            let opened = provider
                .get(&provider.locator(doc), "ru", &context())
                .unwrap()
                .unwrap();
            assert_eq!(opened.text, doc.text);
            assert_eq!(opened.language, doc.language);
        }
    }
}
