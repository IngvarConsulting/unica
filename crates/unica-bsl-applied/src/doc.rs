//! Types a documenting comment names, read from the comment parsed by the
//! development standards' layout ([`unica_bsl_types::doc_comment`]).
//!
//! The layout itself is not read twice: the parse is shared with the view of
//! a method, so a comment cannot mean one thing to the type builder and
//! another to the panel that shows its fields. Here only the type texts
//! matter; what a type means is decided elsewhere.

use unica_bsl_types::doc_comment;

/// The type texts of the parameters, by parameter name, and of the result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocTypes {
    pub(crate) params: Vec<(String, Vec<String>)>,
    pub(crate) returns: Vec<String>,
}

pub(crate) fn doc_types(doc: &str) -> DocTypes {
    let parsed = doc_comment::parse(doc);
    DocTypes {
        // An entry that names a parameter without a type contributes no
        // type: an undocumented type is not an invented one.
        params: parsed
            .parameters
            .parsed()
            .map(|parameters| {
                parameters
                    .iter()
                    .filter(|parameter| !parameter.types.is_empty())
                    .map(|parameter| (parameter.name.clone(), parameter.types.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        returns: parsed
            .returns
            .parsed()
            .map(|returns| returns.types.clone())
            .unwrap_or_default(),
    }
}

/// `X` of `Массив из X` / `Array of X`.
pub(crate) fn strip_array(text: &str) -> Option<&str> {
    doc_comment::strip_array(text)
}
