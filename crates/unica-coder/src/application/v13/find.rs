use crate::domain::address::NodeKind;
use serde::Serialize;
use std::cmp::Ordering;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindRequest {
    query: String,
    kind: Option<String>,
    limit: usize,
}

impl FindRequest {
    pub(crate) fn new(query: &str) -> Result<Self, FindError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(FindError::new("bad_value", "find query must not be empty"));
        }
        Ok(Self {
            query: query.to_string(),
            kind: None,
            limit: DEFAULT_LIMIT,
        })
    }

    pub(crate) fn with_kind(mut self, kind: &str) -> Result<Self, FindError> {
        let kind = NodeKind::parse(kind)
            .map_err(|_| FindError::new("bad_value", format!("unknown node kind `{kind}`")))?;
        self.kind = Some(kind.as_str().to_string());
        Ok(self)
    }

    pub(crate) fn with_limit(mut self, limit: usize) -> Result<Self, FindError> {
        if limit == 0 {
            return Err(FindError::new("bad_value", "find limit must be positive"));
        }
        self.limit = limit.min(MAX_LIMIT);
        Ok(self)
    }
}

/// One searchable fact whose origin is closed to identity data. There is no
/// content/body variant, so content search cannot leak into `find` by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindFactKind {
    Name,
    Synonym,
    ExportPath,
    Address,
    Kind,
}

impl FindFactKind {
    const fn reason(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Synonym => "synonym",
            Self::ExportPath => "exportPath",
            Self::Address => "address",
            Self::Kind => "kind",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindFact {
    kind: FindFactKind,
    value: String,
}

impl FindFact {
    pub(crate) fn new(kind: FindFactKind, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindDocument {
    at: String,
    kind: String,
    title: String,
    facts: Vec<FindFact>,
}

impl FindDocument {
    pub(crate) fn new(
        at: impl Into<String>,
        kind: impl Into<String>,
        title: impl Into<String>,
        mut facts: Vec<FindFact>,
    ) -> Self {
        let at = at.into();
        let kind = kind.into();
        facts.push(FindFact::new(FindFactKind::Address, at.clone()));
        facts.push(FindFact::new(FindFactKind::Kind, kind.clone()));
        Self {
            at,
            kind,
            title: title.into(),
            facts,
        }
    }

    pub(crate) fn estimated_identity_bytes(&self) -> usize {
        self.at
            .len()
            .saturating_add(self.kind.len())
            .saturating_add(self.title.len())
            .saturating_add(
                self.facts
                    .iter()
                    .fold(0usize, |total, fact| total.saturating_add(fact.value.len())),
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FindCandidate {
    at: String,
    kind: String,
    title: String,
    reason: String,
}

impl FindCandidate {
    pub(crate) fn at(&self) -> &str {
        &self.at
    }

    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    pub(crate) fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FindResult {
    candidates: Vec<FindCandidate>,
    #[serde(skip_serializing_if = "is_false")]
    nearest: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl FindResult {
    pub(crate) fn candidates(&self) -> &[FindCandidate] {
        &self.candidates
    }

    pub(crate) const fn is_nearest(&self) -> bool {
        self.nearest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindError {
    code: &'static str,
    message: String,
}

impl FindError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for FindError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug)]
pub(crate) struct FindIndex {
    documents: Vec<FindDocument>,
}

impl FindIndex {
    pub(crate) fn new(mut documents: Vec<FindDocument>) -> Self {
        documents.sort_by(|left, right| left.at.cmp(&right.at));
        documents.dedup_by(|left, right| left.at == right.at);
        Self { documents }
    }

    pub(crate) fn find(&self, request: FindRequest) -> FindResult {
        let query = normalize(&request.query);
        let eligible = self.documents.iter().filter(|document| {
            request
                .kind
                .as_ref()
                .is_none_or(|kind| kind == &document.kind)
        });
        let mut direct = eligible
            .clone()
            .filter_map(|document| best_direct_match(document, &query))
            .collect::<Vec<_>>();
        direct.sort_by(scored_order);
        direct.dedup_by(|left, right| left.document.at == right.document.at);
        if !direct.is_empty() {
            return FindResult {
                candidates: direct
                    .into_iter()
                    .take(request.limit)
                    .map(ScoredCandidate::into_candidate)
                    .collect(),
                nearest: false,
            };
        }

        let mut nearest = eligible
            .filter_map(|document| nearest_match(document, &query))
            .collect::<Vec<_>>();
        nearest.sort_by(scored_order);
        nearest.dedup_by(|left, right| left.document.at == right.document.at);
        FindResult {
            candidates: nearest
                .into_iter()
                .take(request.limit.min(10))
                .map(ScoredCandidate::into_candidate)
                .collect(),
            nearest: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn fact_bytes(&self) -> Vec<u8> {
        let facts = self
            .documents
            .iter()
            .map(|document| {
                serde_json::json!({
                    "at": document.at,
                    "kind": document.kind,
                    "title": document.title,
                    "facts": document.facts.iter().map(|fact| {
                        serde_json::json!({
                            "reason": fact.kind.reason(),
                            "value": fact.value,
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_vec(&facts).expect("find identity facts serialize")
    }
}

#[derive(Clone)]
struct ScoredCandidate<'a> {
    document: &'a FindDocument,
    score: usize,
    reason: String,
}

impl ScoredCandidate<'_> {
    fn into_candidate(self) -> FindCandidate {
        FindCandidate {
            at: self.document.at.clone(),
            kind: self.document.kind.clone(),
            title: self.document.title.clone(),
            reason: self.reason,
        }
    }
}

fn scored_order(left: &ScoredCandidate<'_>, right: &ScoredCandidate<'_>) -> Ordering {
    left.score
        .cmp(&right.score)
        .then_with(|| left.document.at.cmp(&right.document.at))
}

fn best_direct_match<'a>(document: &'a FindDocument, query: &str) -> Option<ScoredCandidate<'a>> {
    document
        .facts
        .iter()
        .filter_map(|fact| {
            let value = normalize(&fact.value);
            let score = if value == query {
                0
            } else if value.starts_with(query) {
                10 + value.len().saturating_sub(query.len())
            } else if value.contains(query) {
                100 + value.len().saturating_sub(query.len())
            } else {
                return None;
            };
            Some(ScoredCandidate {
                document,
                score,
                reason: fact.kind.reason().to_string(),
            })
        })
        .min_by(scored_order)
}

fn nearest_match<'a>(document: &'a FindDocument, query: &str) -> Option<ScoredCandidate<'a>> {
    document
        .facts
        .iter()
        .filter(|fact| matches!(fact.kind, FindFactKind::Name | FindFactKind::Address))
        .filter_map(|fact| {
            let value = normalize(&fact.value);
            let score = bounded_levenshtein(query, &value, 32)?;
            Some(ScoredCandidate {
                document,
                score,
                reason: format!("nearest:{}", fact.kind.reason()),
            })
        })
        .min_by(scored_order)
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

fn bounded_levenshtein(left: &str, right: &str, bound: usize) -> Option<usize> {
    let left = left.chars().take(bound + 1).collect::<Vec<_>>();
    let right = right.chars().take(bound + 1).collect::<Vec<_>>();
    if left.len() > bound || right.len() > bound {
        return None;
    }
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_char) in left.iter().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_char) in right.iter().enumerate() {
            current.push(
                (previous[right_index + 1] + 1)
                    .min(current[right_index] + 1)
                    .min(previous[right_index] + usize::from(left_char != right_char)),
            );
        }
        previous = current;
    }
    previous.last().copied()
}

#[cfg(test)]
mod tests {
    use super::{FindDocument, FindFact, FindFactKind, FindIndex, FindRequest};

    fn index() -> FindIndex {
        FindIndex::new(vec![
            FindDocument::new(
                "main:Catalog.Валюты",
                "Catalog",
                "Валюты — Currencies",
                vec![
                    FindFact::new(FindFactKind::Name, "Валюты"),
                    FindFact::new(FindFactKind::Synonym, "Currencies"),
                    FindFact::new(FindFactKind::ExportPath, "Catalogs/Валюты.xml"),
                ],
            ),
            FindDocument::new(
                "main:Document.Заказ",
                "Document",
                "Заказ",
                vec![FindFact::new(FindFactKind::Name, "Заказ")],
            ),
        ])
    }

    #[test]
    fn find_returns_address_facts_and_not_content_hits() {
        for query in ["Валюты", "Catalog", "Currencies", "Catalogs/Валюты.xml"] {
            let result = index().find(FindRequest::new(query).unwrap());
            assert_eq!(
                result.candidates()[0].at(),
                "main:Catalog.Валюты",
                "{query}"
            );
            assert_eq!(result.candidates()[0].kind(), "Catalog", "{query}");
            assert!(!result.candidates()[0].reason().contains("body"));
            assert!(!result.candidates()[0].reason().contains("content"));
        }
    }

    #[test]
    fn exact_kind_alias_filter_limit_and_nearest_are_deterministic() {
        let catalog = index().find(
            FindRequest::new("а")
                .unwrap()
                .with_kind("Справочник")
                .unwrap()
                .with_limit(1)
                .unwrap(),
        );
        assert_eq!(catalog.candidates().len(), 1);
        assert_eq!(catalog.candidates()[0].kind(), "Catalog");

        let nearest = index().find(FindRequest::new("Валюта").unwrap());
        assert!(nearest.is_nearest());
        assert_eq!(nearest.candidates()[0].at(), "main:Catalog.Валюты");
        assert!(nearest.candidates()[0].reason().starts_with("nearest:"));
    }

    #[test]
    fn default_and_oversized_limits_are_bounded() {
        let documents = (0..150)
            .map(|index| {
                FindDocument::new(
                    format!("main:Catalog.Node{index:03}"),
                    "Catalog",
                    format!("Node {index:03}"),
                    vec![FindFact::new(FindFactKind::Name, format!("Node{index:03}"))],
                )
            })
            .collect();
        let index = FindIndex::new(documents);

        assert_eq!(
            index
                .find(FindRequest::new("Node").unwrap())
                .candidates()
                .len(),
            20,
        );
        assert_eq!(
            index
                .find(
                    FindRequest::new("Node")
                        .unwrap()
                        .with_limit(usize::MAX)
                        .unwrap(),
                )
                .candidates()
                .len(),
            100,
        );
    }
}
