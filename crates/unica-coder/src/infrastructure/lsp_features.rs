//! Editor features Unica builds from its own BSL parse (#2175): document
//! symbols, folding ranges, semantic tokens and signature help. Each takes
//! the text the connection sees (its open buffer, otherwise the file) and
//! answers in LSP JSON: zero-based lines and UTF-16 characters.

use crate::application::lsp::position::{LineIndex, Utf16Position};
use serde_json::{json, Value};
use std::ops::Range;
use unica_bsl_parser::{parse, EditorSymbol, EditorSymbolKind, FoldKind, TokenClass};

/// Semantic token types in legend order; a token's type is its index.
const TOKEN_TYPES: [&str; 11] = [
    "keyword",
    "method",
    "parameter",
    "variable",
    "property",
    "type",
    "macro",
    "decorator",
    "string",
    "number",
    "comment",
];
const TOKEN_MODIFIERS: [&str; 1] = ["declaration"];

// LSP `SymbolKind`.
const SYMBOL_NAMESPACE: u32 = 3;
const SYMBOL_METHOD: u32 = 6;
const SYMBOL_FUNCTION: u32 = 12;

/// The legend `initialize` announces with `semanticTokensProvider`.
pub(crate) fn semantic_tokens_legend() -> Value {
    json!({ "tokenTypes": TOKEN_TYPES, "tokenModifiers": TOKEN_MODIFIERS })
}

/// `textDocument/documentSymbol`: regions and methods as a tree.
pub(crate) fn document_symbols(text: &str) -> Value {
    let lines = LineIndex::new(text);
    let parsed = parse(text);
    Value::Array(
        parsed
            .editor_symbols()
            .iter()
            .map(|symbol| document_symbol(text, &lines, symbol))
            .collect(),
    )
}

fn document_symbol(text: &str, lines: &LineIndex, symbol: &EditorSymbol) -> Value {
    let kind = match symbol.kind {
        EditorSymbolKind::Region => SYMBOL_NAMESPACE,
        EditorSymbolKind::Procedure => SYMBOL_METHOD,
        EditorSymbolKind::Function => SYMBOL_FUNCTION,
    };
    let mut value = json!({
        "name": if symbol.name.is_empty() { "#Область" } else { symbol.name.as_str() },
        "kind": kind,
        "range": range(text, lines, &symbol.range),
        "selectionRange": range(text, lines, &symbol.selection_range),
        "children": symbol
            .children
            .iter()
            .map(|child| document_symbol(text, lines, child))
            .collect::<Vec<_>>(),
    });
    if symbol.kind != EditorSymbolKind::Region {
        value["detail"] = Value::String(signature_label(symbol).0[symbol.name.len()..].to_owned());
    }
    value
}

/// `textDocument/foldingRange`. A block keeps its end keyword visible: it
/// folds up to the line before it.
pub(crate) fn folding_ranges(text: &str) -> Value {
    let lines = LineIndex::new(text);
    let mut seen = std::collections::BTreeSet::new();
    let ranges = parse(text)
        .fold_blocks()
        .into_iter()
        .filter_map(|block| {
            let start = lines.position(text, block.range.start).line;
            let end = lines.position(text, block.range.end).line;
            let end = match block.kind {
                FoldKind::Comment => end,
                FoldKind::Region | FoldKind::Block => end.checked_sub(1)?,
            };
            (end > start && seen.insert((start, end))).then(|| {
                let mut value = json!({ "startLine": start, "endLine": end });
                match block.kind {
                    FoldKind::Region => value["kind"] = json!("region"),
                    FoldKind::Comment => value["kind"] = json!("comment"),
                    FoldKind::Block => {}
                }
                value
            })
        })
        .collect();
    Value::Array(ranges)
}

/// `textDocument/semanticTokens/full`: relative encoding, a token that
/// spans lines split into one token per line.
pub(crate) fn semantic_tokens(text: &str) -> Value {
    let lines = LineIndex::new(text);
    let mut data = Vec::new();
    let mut previous = Utf16Position {
        line: 0,
        character: 0,
    };
    for token in parse(text).classified_tokens() {
        let kind = token_type(token.class);
        let modifiers = u32::from(token.declaration);
        for piece in split_lines(text, token.range) {
            let start = lines.position(text, piece.start);
            let length = text[piece].encode_utf16().count() as u32;
            if length == 0 {
                continue;
            }
            let delta_line = start.line - previous.line;
            let delta_start = if delta_line == 0 {
                start.character - previous.character
            } else {
                start.character
            };
            data.extend([delta_line, delta_start, length, kind, modifiers]);
            previous = start;
        }
    }
    json!({ "data": data })
}

fn token_type(class: TokenClass) -> u32 {
    let name = match class {
        TokenClass::Keyword => "keyword",
        TokenClass::Method => "method",
        TokenClass::Parameter => "parameter",
        TokenClass::Variable => "variable",
        TokenClass::Property => "property",
        TokenClass::Type => "type",
        TokenClass::Directive => "macro",
        TokenClass::Annotation => "decorator",
        TokenClass::String => "string",
        TokenClass::Number => "number",
        TokenClass::Comment => "comment",
    };
    TOKEN_TYPES
        .iter()
        .position(|known| *known == name)
        .expect("every token class is in the legend") as u32
}

/// The pieces of `range` on each line, without line terminators.
fn split_lines(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut start = range.start;
    let bytes = text.as_bytes();
    let mut index = range.start;
    while index < range.end {
        if matches!(bytes[index], b'\n' | b'\r') {
            pieces.push(start..index);
            if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            start = index + 1;
        }
        index += 1;
    }
    pieces.push(start..range.end.max(start));
    pieces
}

/// Finds the module text of a common module by name, from the buffer the
/// connection has open or from the file; `None` when there is none.
pub(crate) type CommonModuleText<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// `textDocument/signatureHelp` for a call of a method of this module or
/// of an exported method of a common module. A call of anything else
/// (a platform method, an object's method) has no signature here.
pub(crate) fn signature_help(
    text: &str,
    position: Utf16Position,
    common_module: &CommonModuleText<'_>,
) -> Option<Value> {
    let lines = LineIndex::new(text);
    let offset = lines.offset(text, position).ok()?;
    let parsed = parse(text);
    let call = parsed.call_at(offset)?;
    let method = match &call.qualifier {
        None => find_method(&parsed.editor_symbols(), &call.name, false),
        Some(module) => {
            let module_text = common_module(module)?;
            find_method(&parse(&module_text).editor_symbols(), &call.name, true)
        }
    }?;
    let (label, parameters) = signature_label(&method);
    let mut signature = json!({
        "label": label,
        "parameters": parameters
            .iter()
            .map(|range| json!({ "label": [range.start, range.end] }))
            .collect::<Vec<_>>(),
    });
    if let Some(description) = &method.description {
        // Plain text: a description keeps its own line breaks and is not
        // read as markup.
        signature["documentation"] = json!({ "kind": "plaintext", "value": description });
    }
    // An argument past the last parameter highlights none, as LSP allows.
    let active = call.active_parameter;
    Some(json!({
        "signatures": [signature],
        "activeSignature": 0,
        "activeParameter": active,
    }))
}

fn find_method(symbols: &[EditorSymbol], name: &str, exported: bool) -> Option<EditorSymbol> {
    let name = name.to_lowercase();
    symbols.iter().find_map(|symbol| {
        if symbol.kind == EditorSymbolKind::Region {
            find_method(&symbol.children, &name, exported)
        } else {
            (symbol.name.to_lowercase() == name && (!exported || symbol.is_export))
                .then(|| symbol.clone())
        }
    })
}

/// `Имя(Знач А, Б = 1) Экспорт` and the UTF-16 ranges of each parameter in
/// it, as `ParameterInformation.label` offsets.
fn signature_label(method: &EditorSymbol) -> (String, Vec<Range<u32>>) {
    let mut label = format!("{}(", method.name);
    let mut ranges = Vec::new();
    for (index, parameter) in method.parameters.iter().enumerate() {
        if index > 0 {
            label.push_str(", ");
        }
        let start = label.encode_utf16().count() as u32;
        if parameter.by_value {
            label.push_str("Знач ");
        }
        label.push_str(&parameter.name);
        if let Some(default) = &parameter.default_value {
            label.push_str(" = ");
            label.push_str(default);
        }
        ranges.push(start..label.encode_utf16().count() as u32);
    }
    label.push(')');
    if method.is_export {
        label.push_str(" Экспорт");
    }
    (label, ranges)
}

fn range(text: &str, lines: &LineIndex, range: &Range<usize>) -> Value {
    let position = |offset| {
        let at = lines.position(text, offset);
        json!({ "line": at.line, "character": at.character })
    };
    json!({ "start": position(range.start), "end": position(range.end) })
}

#[cfg(test)]
#[path = "../../tests/platform/lsp_features.rs"]
mod tests;
