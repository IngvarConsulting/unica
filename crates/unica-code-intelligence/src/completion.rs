//! Completion of BSL code in Unica's own model: the names the code at a
//! position can use, from the module's own parse, the metadata of its source
//! set and the platform's global context.
//!
//! A position is a byte offset of the text being edited; the answer names
//! the byte range its items replace. What lies beyond the module comes from
//! [`CompletionSources`]; nothing of an analysis provider is asked.

use std::collections::BTreeSet;
use std::ops::Range;
use unica_address::source_target::{
    metadata_collection_kind, metadata_collection_names, metadata_kind_has_objects,
    metadata_manager_kinds,
};
use unica_bsl_parser::{parse, EditorSymbol, EditorSymbolKind, ParsedModule};

/// A method or property of the platform's global context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformMember {
    pub name: String,
    pub method: bool,
    /// Parameter names of a method's first syntax variant.
    pub parameters: Vec<String>,
}

/// Members of a metadata object a module can reach through it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectMembers {
    pub attributes: Vec<String>,
    /// Each tabular section with its columns.
    pub tabular_sections: Vec<(String, Vec<String>)>,
}

/// The metadata object a module belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleOwner {
    pub kind: String,
    pub name: String,
    /// A form module: the object is reached as `Объект`, not as
    /// `ЭтотОбъект`, and its attributes are not names of the module.
    pub form: bool,
}

/// What completion knows beyond the module text.
pub trait CompletionSources {
    /// Members of the platform's global context; empty while they are not
    /// known yet.
    fn platform_members(&self) -> &[PlatformMember];
    /// Names of the registered objects of `kind` (`CommonModule`, `Catalog`).
    fn metadata_names(&self, kind: &str) -> Vec<String>;
    /// The text of a common module, by its registered name.
    fn common_module_text(&self, name: &str) -> Option<String>;
    fn object_members(&self, kind: &str, name: &str) -> Option<ObjectMembers>;
    /// The object whose object module or form module is being edited.
    fn module_owner(&self) -> Option<ModuleOwner>;
    /// The request is no longer wanted: stop and answer nothing.
    fn cancelled(&self) -> bool;
}

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

/// How deep a variable's type is followed through loops and assignments.
// resource-limit: lsp.completion-inference-depth
// class: step
// unit: count
// scope: request
// value: 4
// resource: Шаги, на которые дополнение прослеживает тип переменной через циклы и присваивания.
// status: unproven
// origin: #2578
// problem: неизвестно
// on-reach: тип не выводится, и членов объекта в списке нет
// issue: #2578
const MAX_INFERENCE_DEPTH: usize = 4;

#[derive(Default)]
struct Items {
    items: Vec<CompletionItem>,
    seen: BTreeSet<String>,
}

impl Items {
    fn push(
        &mut self,
        label: &str,
        kind: CompletionKind,
        rank: u8,
        detail: Option<String>,
        method: Option<CompletionMethod>,
    ) {
        if label.is_empty() || !self.seen.insert(label.to_lowercase()) {
            return;
        }
        self.items.push(CompletionItem {
            label: label.to_owned(),
            kind,
            rank,
            detail,
            method,
        });
    }
}

/// The names the code at byte `offset` can use. `None` inside a comment or
/// a string, or when `sources` reports the request no longer wanted.
pub fn completion(
    text: &str,
    offset: usize,
    sources: &dyn CompletionSources,
) -> Option<Completion> {
    let parsed = parse(text);
    let site = parsed.completion_site(offset)?;
    let mut items = Items::default();
    match &site.receiver {
        None => names_in_scope(&parsed, offset, sources, &mut items),
        Some(path) => members_of(&parsed, offset, path, sources, &mut items),
    }
    if sources.cancelled() {
        return None;
    }
    Some(Completion {
        items: items.items,
        replaces: site.prefix_range,
    })
}

/// The description of a method an item names: the comment above it in
/// `module_text`, this module or the common module that exports it.
pub fn method_description(module_text: &str, method: &CompletionMethod) -> Option<String> {
    find_method(
        &parse(module_text).editor_symbols(),
        &method.name,
        method.common_module.is_some(),
    )?
    .description
}

fn names_in_scope(
    parsed: &ParsedModule<'_>,
    offset: usize,
    sources: &dyn CompletionSources,
    items: &mut Items,
) {
    let visible = parsed.visible_names(offset);
    for name in &visible.parameters {
        items.push(
            name,
            CompletionKind::Variable,
            0,
            Some("Параметр".to_owned()),
            None,
        );
    }
    for name in &visible.locals {
        items.push(
            name,
            CompletionKind::Variable,
            0,
            Some("Локальная переменная".to_owned()),
            None,
        );
    }
    for name in &visible.module_variables {
        items.push(
            name,
            CompletionKind::Variable,
            1,
            Some("Переменная модуля".to_owned()),
            None,
        );
    }
    for method in methods(&parsed.editor_symbols()) {
        let kind = if method.kind == EditorSymbolKind::Function {
            CompletionKind::Function
        } else {
            CompletionKind::Method
        };
        items.push(
            &method.name,
            kind,
            1,
            Some(signature_label(&method)),
            Some(CompletionMethod {
                name: method.name.clone(),
                common_module: None,
            }),
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
            CompletionKind::Module,
            3,
            Some("Общий модуль".to_owned()),
            None,
        );
    }
    for kind in metadata_manager_kinds() {
        // English first, then the Russian name code uses.
        if let Some(name) = metadata_collection_names(kind).and_then(|names| names.get(1)) {
            items.push(name, CompletionKind::Property, 4, None, None);
        }
    }
    for member in sources.platform_members() {
        let (kind, detail) = if member.method {
            (
                CompletionKind::Function,
                Some(format!("{}({})", member.name, member.parameters.join(", "))),
            )
        } else {
            (CompletionKind::Property, None)
        };
        items.push(&member.name, kind, 5, detail, None);
    }
}

fn members_of(
    parsed: &ParsedModule<'_>,
    offset: usize,
    path: &[String],
    sources: &dyn CompletionSources,
    items: &mut Items,
) {
    // `Объект.Товары.` gives no columns: a tabular section is a collection.
    // A loop variable over it does (see `object_of`).
    let [single] = path else {
        return;
    };
    if let Some(kind) = manager_kind(single) {
        for name in sources.metadata_names(kind) {
            items.push(&name, CompletionKind::Class, 0, None, None);
        }
        return;
    }
    let lower = single.to_lowercase();
    let module = sources
        .metadata_names("CommonModule")
        .into_iter()
        .find(|module| module.to_lowercase() == lower);
    if let Some(module) = module {
        if let Some(text) = sources.common_module_text(&module) {
            for method in methods(&parse(&text).editor_symbols()) {
                if method.is_export {
                    items.push(
                        &method.name,
                        CompletionKind::Method,
                        0,
                        Some(signature_label(&method)),
                        Some(CompletionMethod {
                            name: method.name.clone(),
                            common_module: Some(module.clone()),
                        }),
                    );
                }
            }
        }
        return;
    }
    if let Some(members) = object_of(parsed, offset, path, sources, MAX_INFERENCE_DEPTH) {
        object_items(&members, 0, items);
    }
}

/// The members `path` reaches, when it names an object or a row of a
/// tabular section. A variable takes the type of its last assignment or
/// loop before the cursor: an object created by
/// `Справочники.X.СоздатьЭлемент()`, a row of a section the loop walks.
/// A name neither declares is the module's own object: `ЭтотОбъект` in an
/// object module, `Объект` in a form.
fn object_of(
    parsed: &ParsedModule<'_>,
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
        return (creates && metadata_kind_has_objects(kind))
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
            CompletionKind::Field,
            rank,
            Some("Реквизит".to_owned()),
            None,
        );
    }
    for (section, _) in &members.tabular_sections {
        items.push(
            section,
            CompletionKind::Struct,
            rank,
            Some("Табличная часть".to_owned()),
            None,
        );
    }
}

fn manager_kind(name: &str) -> Option<&'static str> {
    metadata_collection_kind(name)
        .filter(|kind| metadata_manager_kinds().any(|known| known == *kind))
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

/// `Имя(Знач А, Б = 1) Экспорт`.
fn signature_label(method: &EditorSymbol) -> String {
    let parameters = method
        .parameters
        .iter()
        .map(|parameter| {
            let mut text = String::new();
            if parameter.by_value {
                text.push_str("Знач ");
            }
            text.push_str(&parameter.name);
            if let Some(default) = &parameter.default_value {
                text.push_str(" = ");
                text.push_str(default);
            }
            text
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut label = format!("{}({parameters})", method.name);
    if method.is_export {
        label.push_str(" Экспорт");
    }
    label
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
