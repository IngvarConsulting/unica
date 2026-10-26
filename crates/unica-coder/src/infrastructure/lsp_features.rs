//! Editor features Unica builds from its own BSL parse (#2175): document
//! symbols, folding ranges, semantic tokens and signature help. Each takes
//! the text the connection sees (its open buffer, otherwise the file) and
//! answers in LSP JSON: zero-based lines and UTF-16 characters.

use crate::application::lsp::position::{LineIndex, Utf16Position};
use serde_json::{json, Value};
use std::ops::Range;
use unica_address::source_target::{metadata_collection_kind, metadata_collection_names};
use unica_bsl_parser::{parse, EditorSymbol, EditorSymbolKind, FoldKind, TokenClass};
use unica_documentation_sources::platform_help::provider::{
    GlobalContextMember, GlobalContextMemberKind,
};

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

/// The platform help is not indexed yet and the call may be one of its
/// methods: "no signature" would be an answer given without its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceNotReady;

/// `textDocument/signatureHelp` for a call of a method of this module, of
/// an exported method of a common module, or of a method of the platform
/// global context. A call of anything else (an object's method) has no
/// signature here.
pub(crate) fn signature_help(
    text: &str,
    position: Utf16Position,
    common_module: &CommonModuleText<'_>,
    global_context: Option<&[GlobalContextMember]>,
) -> Result<Option<Value>, SourceNotReady> {
    let lines = LineIndex::new(text);
    let Ok(offset) = lines.offset(text, position) else {
        return Ok(None);
    };
    let parsed = parse(text);
    let Some(call) = parsed.call_at(offset) else {
        return Ok(None);
    };
    let method = match &call.qualifier {
        None => find_method(&parsed.editor_symbols(), &call.name, false),
        Some(module) => common_module(module)
            .and_then(|text| find_method(&parse(&text).editor_symbols(), &call.name, true)),
    };
    let (label, parameters, description) = match method {
        Some(method) => {
            let (label, parameters) = signature_label(&method);
            (label, parameters, method.description)
        }
        None if call.qualifier.is_none() => {
            let Some(member) = global_method(global_context.ok_or(SourceNotReady)?, &call.name)
            else {
                return Ok(None);
            };
            let (label, parameters) = global_signature_label(member);
            (label, parameters, None)
        }
        None => return Ok(None),
    };
    let mut signature = json!({
        "label": label,
        "parameters": parameters
            .iter()
            .map(|range| json!({ "label": [range.start, range.end] }))
            .collect::<Vec<_>>(),
    });
    if let Some(description) = &description {
        // Plain text: a description keeps its own line breaks and is not
        // read as markup.
        signature["documentation"] = json!({ "kind": "plaintext", "value": description });
    }
    // An argument past the last parameter highlights none, as LSP allows.
    let active = call.active_parameter;
    Ok(Some(json!({
        "signatures": [signature],
        "activeSignature": 0,
        "activeParameter": active,
    })))
}

/// A global context method by its Russian or English name, in any case.
fn global_method<'a>(
    global_context: &'a [GlobalContextMember],
    name: &str,
) -> Option<&'a GlobalContextMember> {
    let name = name.to_lowercase();
    global_context.iter().find(|member| {
        member.kind == GlobalContextMemberKind::Method
            && (member.name_ru.to_lowercase() == name
                || member
                    .name_en
                    .as_ref()
                    .is_some_and(|english| english.to_lowercase() == name))
    })
}

/// `ЗначениеЗаполнено(Значение)` and the UTF-16 ranges of its parameters.
fn global_signature_label(member: &GlobalContextMember) -> (String, Vec<Range<u32>>) {
    let mut label = format!("{}(", member.name_ru);
    let mut ranges = Vec::new();
    for (index, parameter) in member.parameters.iter().enumerate() {
        if index > 0 {
            label.push_str(", ");
        }
        let start = label.encode_utf16().count() as u32;
        label.push_str(parameter);
        ranges.push(start..label.encode_utf16().count() as u32);
    }
    label.push(')');
    (label, ranges)
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

/// Members of a metadata object a module can reach through it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ObjectMembers {
    pub(crate) attributes: Vec<String>,
    /// Each tabular section with its columns.
    pub(crate) tabular_sections: Vec<(String, Vec<String>)>,
}

/// The metadata object a module belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleOwner {
    pub(crate) kind: String,
    pub(crate) name: String,
    /// A form module: the object is reached as `Объект`, not as
    /// `ЭтотОбъект`, and its attributes are not names of the module.
    pub(crate) form: bool,
}

/// What completion knows beyond the module text. The LSP route supplies
/// it over the workspace actor; tests supply a fixture.
pub(crate) trait CompletionSources {
    /// Methods and properties of the platform global context; empty while
    /// the platform help is not indexed yet.
    fn global_context(&self) -> &[GlobalContextMember];
    /// Names of the registered objects of `kind` (`CommonModule`, `Catalog`).
    fn metadata_names(&self, kind: &str) -> Vec<String>;
    /// The text of a common module, by its name in any case.
    fn common_module_text(&self, name: &str) -> Option<String>;
    fn object_members(&self, kind: &str, name: &str) -> Option<ObjectMembers>;
    /// The object whose object module or form module is being edited.
    fn module_owner(&self) -> Option<ModuleOwner>;
    /// A newer edit made the answer stale: stop and answer nothing.
    fn cancelled(&self) -> bool;
}

/// Kinds whose managers are global context properties (`Справочники`).
const MANAGER_KINDS: [&str; 22] = [
    "Catalog",
    "Document",
    "Enum",
    "Report",
    "DataProcessor",
    "ChartOfCharacteristicTypes",
    "ChartOfAccounts",
    "ChartOfCalculationTypes",
    "InformationRegister",
    "AccumulationRegister",
    "AccountingRegister",
    "CalculationRegister",
    "BusinessProcess",
    "Task",
    "ExchangePlan",
    "Constant",
    "DocumentJournal",
    "Sequence",
    "FilterCriterion",
    "SettingsStorage",
    "SessionParameter",
    "WSReference",
];

/// Kinds whose objects carry attributes and tabular sections.
const OBJECT_KINDS: [&str; 10] = [
    "Catalog",
    "Document",
    "ChartOfCharacteristicTypes",
    "ChartOfAccounts",
    "ChartOfCalculationTypes",
    "BusinessProcess",
    "Task",
    "ExchangePlan",
    "DataProcessor",
    "Report",
];

// LSP `CompletionItemKind`.
const ITEM_METHOD: u32 = 2;
const ITEM_FUNCTION: u32 = 3;
const ITEM_FIELD: u32 = 5;
const ITEM_VARIABLE: u32 = 6;
const ITEM_CLASS: u32 = 7;
const ITEM_MODULE: u32 = 9;
const ITEM_PROPERTY: u32 = 10;
const ITEM_STRUCT: u32 = 22;

/// How deep a variable's type is followed through loops and assignments.
const INFERENCE_DEPTH: usize = 4;

struct Items {
    items: Vec<Value>,
    seen: std::collections::BTreeSet<String>,
}

impl Items {
    fn push(
        &mut self,
        label: &str,
        kind: u32,
        rank: u8,
        detail: Option<String>,
        data: Option<Value>,
    ) {
        if label.is_empty() || !self.seen.insert(label.to_lowercase()) {
            return;
        }
        let mut item = json!({
            "label": label,
            "kind": kind,
            "sortText": format!("{rank}{}", label.to_lowercase()),
        });
        if let Some(detail) = detail {
            item["detail"] = Value::String(detail);
        }
        if let Some(data) = data {
            item["data"] = data;
        }
        self.items.push(item);
    }
}

/// `textDocument/completion` at `position`: the names the code there can
/// use. `None` inside a comment or a string, or when `sources` reports the
/// request stale.
pub(crate) fn completion(
    text: &str,
    position: Utf16Position,
    sources: &dyn CompletionSources,
) -> Option<Value> {
    let lines = LineIndex::new(text);
    let offset = lines.offset(text, position).ok()?;
    let parsed = parse(text);
    let site = parsed.completion_site(offset)?;
    let mut items = Items {
        items: Vec::new(),
        seen: Default::default(),
    };
    match &site.receiver {
        None => names_in_scope(&parsed, offset, sources, &mut items),
        Some(path) => members_of(&parsed, offset, path, sources, &mut items),
    }
    if sources.cancelled() {
        return None;
    }
    let edit_range = range(text, &lines, &site.prefix_range);
    for item in &mut items.items {
        item["textEdit"] = json!({ "range": edit_range, "newText": item["label"] });
    }
    Some(json!({ "isIncomplete": false, "items": items.items }))
}

fn names_in_scope(
    parsed: &unica_bsl_parser::ParsedModule<'_>,
    offset: usize,
    sources: &dyn CompletionSources,
    items: &mut Items,
) {
    let visible = parsed.visible_names(offset);
    for name in &visible.parameters {
        items.push(name, ITEM_VARIABLE, 0, Some("Параметр".to_owned()), None);
    }
    for name in &visible.locals {
        items.push(
            name,
            ITEM_VARIABLE,
            0,
            Some("Локальная переменная".to_owned()),
            None,
        );
    }
    for name in &visible.module_variables {
        items.push(
            name,
            ITEM_VARIABLE,
            1,
            Some("Переменная модуля".to_owned()),
            None,
        );
    }
    for method in methods(&parsed.editor_symbols()) {
        let (label, _) = signature_label(&method);
        let kind = if method.kind == EditorSymbolKind::Function {
            ITEM_FUNCTION
        } else {
            ITEM_METHOD
        };
        items.push(
            &method.name,
            kind,
            1,
            Some(label),
            Some(json!({ "method": method.name })),
        );
    }
    // In an object module the object's attributes are names of the module.
    if let Some(owner) = sources.module_owner().filter(|owner| !owner.form) {
        if let Some(members) = object_members(sources, &owner.kind, &owner.name) {
            object_items(&members, 2, items);
        }
    }
    if sources.cancelled() {
        return;
    }
    for module in sources.metadata_names("CommonModule") {
        items.push(
            &module,
            ITEM_MODULE,
            3,
            Some("Общий модуль".to_owned()),
            None,
        );
    }
    for kind in MANAGER_KINDS {
        if let Some(names) = metadata_collection_names(kind) {
            // English first, then the Russian name code uses.
            if let Some(name) = names.get(1) {
                items.push(name, ITEM_PROPERTY, 4, None, None);
            }
        }
    }
    for member in sources.global_context() {
        let (kind, detail) = match member.kind {
            GlobalContextMemberKind::Method => {
                (ITEM_FUNCTION, Some(global_signature_label(member).0))
            }
            GlobalContextMemberKind::Property => (ITEM_PROPERTY, None),
        };
        items.push(&member.name_ru, kind, 5, detail, None);
    }
}

fn members_of(
    parsed: &unica_bsl_parser::ParsedModule<'_>,
    offset: usize,
    path: &[String],
    sources: &dyn CompletionSources,
    items: &mut Items,
) {
    match path {
        [] => {}
        [single] => {
            if let Some(kind) = manager_kind(single) {
                for name in sources.metadata_names(kind) {
                    items.push(&name, ITEM_CLASS, 0, None, None);
                }
                return;
            }
            let lower = single.to_lowercase();
            let is_module = sources
                .metadata_names("CommonModule")
                .iter()
                .any(|module| module.to_lowercase() == lower);
            if is_module {
                if let Some(module) = sources.common_module_text(single) {
                    for method in methods(&parse(&module).editor_symbols()) {
                        if method.is_export {
                            let (label, _) = signature_label(&method);
                            items.push(
                                &method.name,
                                ITEM_METHOD,
                                0,
                                Some(label),
                                Some(json!({ "commonModule": single, "method": method.name })),
                            );
                        }
                    }
                }
                return;
            }
            if let Some(members) = object_of(parsed, offset, path, sources, INFERENCE_DEPTH) {
                object_items(&members, 0, items);
            }
        }
        // `Объект.Товары.` gives no columns: a tabular section is a
        // collection. A loop variable over it does (see `object_of`).
        _ => {}
    }
}

/// The members `path` reaches, when it names an object or a row of a
/// tabular section. A variable takes the type of its last assignment or
/// loop before the cursor: an object created by
/// `Справочники.X.СоздатьЭлемент()`, a row of a section the loop walks.
/// A name neither declares is the module's own object: `ЭтотОбъект` in an
/// object module, `Объект` in a form.
fn object_of(
    parsed: &unica_bsl_parser::ParsedModule<'_>,
    offset: usize,
    path: &[String],
    sources: &dyn CompletionSources,
    depth: usize,
) -> Option<ObjectMembers> {
    let depth = depth.checked_sub(1)?;
    let [variable] = path else {
        return None;
    };
    let assigned = parsed.assigned_call(offset, variable);
    let looped = parsed.loop_collection(offset, variable);
    let assigned_last = match (&assigned, &looped) {
        (Some((assigned, _)), Some((looped, _))) => assigned > looped,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => {
            let owner = sources.module_owner()?;
            let lower = variable.to_lowercase();
            let names_owner = if owner.form {
                matches!(lower.as_str(), "объект" | "object")
            } else {
                matches!(lower.as_str(), "этотобъект" | "thisobject")
            };
            return names_owner
                .then(|| object_members(sources, &owner.kind, &owner.name))
                .flatten();
        }
    };
    if assigned_last {
        let (_, call) = assigned?;
        let [manager, name, create] = call.as_slice() else {
            return None;
        };
        let kind = manager_kind(manager)?;
        let create = create.to_lowercase();
        let creates = create.starts_with("создать") || create.starts_with("create");
        return (creates && OBJECT_KINDS.contains(&kind))
            .then(|| object_members(sources, kind, name))
            .flatten();
    }
    let (_, collection) = looped?;
    let (owner, section) = collection.split_at(collection.len().checked_sub(1)?);
    let owner = object_of(parsed, offset, owner, sources, depth)?;
    let section = section[0].to_lowercase();
    owner
        .tabular_sections
        .into_iter()
        .find(|(name, _)| name.to_lowercase() == section)
        .map(|(_, columns)| ObjectMembers {
            attributes: columns,
            tabular_sections: Vec::new(),
        })
}

/// The members of `kind` `name`, the name matched to its registration in
/// any case, as code may spell it.
fn object_members(
    sources: &dyn CompletionSources,
    kind: &str,
    name: &str,
) -> Option<ObjectMembers> {
    let lower = name.to_lowercase();
    let registered = sources
        .metadata_names(kind)
        .into_iter()
        .find(|known| known.to_lowercase() == lower)?;
    sources.object_members(kind, &registered)
}

fn object_items(members: &ObjectMembers, rank: u8, items: &mut Items) {
    for attribute in &members.attributes {
        items.push(
            attribute,
            ITEM_FIELD,
            rank,
            Some("Реквизит".to_owned()),
            None,
        );
    }
    for (section, _) in &members.tabular_sections {
        items.push(
            section,
            ITEM_STRUCT,
            rank,
            Some("Табличная часть".to_owned()),
            None,
        );
    }
}

fn manager_kind(name: &str) -> Option<&'static str> {
    metadata_collection_kind(name).filter(|kind| MANAGER_KINDS.contains(kind))
}

/// Every method of a symbol tree, regions flattened.
fn methods(symbols: &[EditorSymbol]) -> Vec<EditorSymbol> {
    let mut found = Vec::new();
    for symbol in symbols {
        if symbol.kind == EditorSymbolKind::Region {
            found.extend(methods(&symbol.children));
        } else {
            found.push(symbol.clone());
        }
    }
    found
}

/// `completionItem/resolve`: the description of a module method, read
/// from `module_text` (this module, or the common module the item names).
pub(crate) fn resolve_completion(mut item: Value, module_text: &str) -> Value {
    let Some(method) = item["data"]["method"].as_str() else {
        return item;
    };
    let exported = item["data"]["commonModule"].is_string();
    if let Some(found) = find_method(&parse(module_text).editor_symbols(), method, exported) {
        if let Some(description) = found.description {
            item["documentation"] = json!({ "kind": "plaintext", "value": description });
        }
    }
    item
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
