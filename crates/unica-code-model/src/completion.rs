//! A completion answer in Unica's own terms: the names offered and the bytes
//! of the text the chosen one replaces. Plain data: nothing here parses code.

use std::ops::Range;

/// What an item is, in the editor's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Method,
    Function,
    Field,
    Variable,
    Class,
    Module,
    Property,
    Struct,
}

/// A method an item names, for its description on resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionMethod {
    pub name: String,
    /// The common module that exports it; `None` for this module's own.
    pub common_module: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    /// Orders items: the closer the name, the smaller.
    pub rank: u8,
    pub detail: Option<String>,
    pub method: Option<CompletionMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub items: Vec<CompletionItem>,
    /// The bytes of the text the chosen item replaces: the name typed so far.
    pub replaces: Range<usize>,
}
