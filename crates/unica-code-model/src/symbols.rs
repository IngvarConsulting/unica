//! Answers about symbols of BSL code in Unica's own terms: places, what a
//! name is, and why an answer may miss something. Plain data: nothing here
//! parses code or names a provider.

use serde::Serialize;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

/// A position in a module: 0-based line and 0-based UTF-16 column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolPosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolRange {
    pub start: SymbolPosition,
    pub end: SymbolPosition,
}

/// A place in Unica's terms: a logical address and, inside a module, a range.
/// A metadata node has no range: the place is the node itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolLocation {
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<SymbolRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    Method,
    Parameter,
    Variable,
    Metadata,
    Platform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SymbolLocation>,
}

/// Why an answer may miss something. Unica's own codes: a provider's words
/// never reach the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "count")]
pub enum Incompleteness {
    /// The buffer differs from the saved module; the part answered beyond the
    /// module's own text describes the saved revision.
    Stale,
    /// The provider is still preparing this answer; asking again later can
    /// give the full one.
    ProviderNotReady,
    /// No provider could answer beyond the module's own text.
    ProviderUnavailable,
    /// The provider answered with part of what it found, or from an index
    /// behind the saved sources; or the name index reached its budget and
    /// left modules out.
    Capped,
    /// Places in other modules are not answered: only this module's are.
    OtherModules,
    /// The name index of the source set is still being built; asking again
    /// later can give the places in other modules too.
    NamesNotIndexed,
    /// Uses through a variable holding an object are not found: the code
    /// does not say what a variable holds.
    UntypedReferences,
    /// So many places the provider named have no logical address here.
    Unaddressable(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolAnswer<T> {
    pub value: T,
    pub complete: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<Incompleteness>,
}

impl<T> SymbolAnswer<T> {
    /// An answer nothing is missing from.
    pub fn complete(value: T) -> Self {
        Self {
            value,
            complete: true,
            reasons: Vec::new(),
        }
    }

    /// An answer that may miss what `reasons` name; none — complete.
    pub fn partial(value: T, mut reasons: Vec<Incompleteness>) -> Self {
        reasons.dedup();
        Self {
            complete: reasons.is_empty(),
            value,
            reasons,
        }
    }
}

/// What a symbol request carries besides its question: the time its answer
/// is still wanted and its cancellation.
pub struct SymbolRequestContext<'a> {
    pub deadline: ProviderDeadline,
    pub cancellation: &'a CancellationToken,
}
