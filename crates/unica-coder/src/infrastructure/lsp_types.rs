//! Types of BSL expressions for the editor (#2238, stage 3): the receiver
//! chain of a member access, a variable by its last assignment or loop,
//! the module's own object, followed step by step through the platform
//! catalog and the configuration's applied types.

use crate::application::lsp::position::{LineIndex, Utf16Position};
use crate::infrastructure::lsp_features::ModuleOwner;
use serde_json::{json, Value};
use unica_bsl_parser::{parse, AssignedValue, ChainSegment, ParsedModule};
use unica_bsl_types::{AppliedTypes, PlatformCatalog, Ty, TypeName};

/// How many assignments and loops a variable's type is followed through.
pub(crate) const VARIABLE_DEPTH: usize = 6;

/// What types are known from: the platform catalog of the workspace's
/// installation, the configuration, and the module's own object.
pub(crate) struct TypeSources<'a> {
    pub(crate) catalog: &'a PlatformCatalog,
    pub(crate) applied: &'a dyn AppliedTypes,
    pub(crate) owner: Option<ModuleOwner>,
}

/// The types `chain` has at `offset`; empty when they are not known.
pub(crate) fn chain_types(
    parsed: &ParsedModule<'_>,
    offset: usize,
    chain: &[ChainSegment],
    types: &TypeSources<'_>,
    depth: usize,
) -> Vec<Ty> {
    let Some((head, rest)) = chain.split_first() else {
        return Vec::new();
    };
    let mut current = head_types(parsed, offset, head, types, depth);
    for segment in rest {
        if current.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for receiver in &current {
            for ty in
                types
                    .catalog
                    .member_types(receiver, &segment.name, segment.call, types.applied)
            {
                if !next.contains(&ty) {
                    next.push(ty);
                }
            }
        }
        current = next;
    }
    current
}

fn head_types(
    parsed: &ParsedModule<'_>,
    offset: usize,
    head: &ChainSegment,
    types: &TypeSources<'_>,
    depth: usize,
) -> Vec<Ty> {
    if !head.call {
        if let Some(found) = variable_types(parsed, offset, &head.name, types, depth) {
            return found;
        }
        let lower = head.name.to_lowercase();
        if let Some(owner) = &types.owner {
            let names_owner = if owner.form {
                matches!(lower.as_str(), "объект" | "object")
            } else {
                matches!(lower.as_str(), "этотобъект" | "thisobject")
            };
            if names_owner {
                return types
                    .catalog
                    .template(&owner.kind, "Object")
                    .map(|template| {
                        vec![Ty {
                            id: template.id.clone(),
                            applied: Some(owner.name.clone()),
                        }]
                    })
                    .unwrap_or_default();
            }
        }
    }
    let Some(global) = types.catalog.global() else {
        return Vec::new();
    };
    types.catalog.member_types(
        &Ty {
            id: global.id.clone(),
            applied: None,
        },
        &head.name,
        head.call,
        types.applied,
    )
}

/// A variable's types by what it was last given before `offset`: `None`
/// when no assignment or loop of the method names it.
fn variable_types(
    parsed: &ParsedModule<'_>,
    offset: usize,
    name: &str,
    types: &TypeSources<'_>,
    depth: usize,
) -> Option<Vec<Ty>> {
    let depth = depth.checked_sub(1)?;
    let assigned = parsed.assigned_value(offset, name);
    let looped = parsed.loop_chain(offset, name);
    let assigned_last = match (&assigned, &looped) {
        (Some((assigned, _)), Some((looped, _))) => assigned > looped,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => return None,
    };
    if assigned_last {
        let (at, value) = assigned?;
        return Some(match value {
            AssignedValue::New(type_name) => types
                .catalog
                .type_by_name(&type_name)
                .map(|ty| {
                    vec![Ty {
                        id: ty.id.clone(),
                        applied: None,
                    }]
                })
                .unwrap_or_default(),
            // The value is read where it was assigned, so its own
            // variables mean what they meant there.
            AssignedValue::Chain(chain) => chain_types(parsed, at, &chain, types, depth),
            AssignedValue::Other => Vec::new(),
        });
    }
    let (at, collection) = looped?;
    let collections = chain_types(parsed, at, &collection, types, depth);
    Some(
        collections
            .iter()
            .filter_map(|collection| types.catalog.type_by_id(&collection.id))
            .flat_map(|collection| collection.collection_element.iter())
            .filter_map(|reference| match &reference.ty {
                TypeName::Context(id) => Some(Ty {
                    id: id.clone(),
                    applied: None,
                }),
                _ => None,
            })
            .collect(),
    )
}

/// `textDocument/hover`: the type of the name under the cursor, or the
/// signature of the method it calls.
pub(crate) fn hover(text: &str, position: Utf16Position, types: &TypeSources<'_>) -> Option<Value> {
    let lines = LineIndex::new(text);
    let offset = lines.offset(text, position).ok()?;
    let parsed = parse(text);
    let site = parsed.completion_site(offset)?;
    let word = &text[site.prefix_range.clone()];
    if word.is_empty() {
        return None;
    }
    let call = text[site.prefix_range.end..].trim_start().starts_with('(');
    let segment = ChainSegment {
        name: word.to_owned(),
        call,
    };
    let receiver = site.chain.clone().unwrap_or_default();
    if site.chain.is_some() && receiver.is_empty() {
        return None;
    }
    let mut chain = receiver.clone();
    chain.push(segment);
    let found = chain_types(
        &parsed,
        site.prefix_range.start,
        &chain,
        types,
        VARIABLE_DEPTH,
    );
    let signature = if call {
        method_label(&parsed, site.prefix_range.start, &receiver, word, types)
    } else {
        None
    };
    let value = match (signature, found.is_empty()) {
        (Some(label), _) => format!("```bsl\n{label}\n```"),
        (None, false) => format!(
            "Тип: {}",
            found
                .iter()
                .map(|ty| types.catalog.ty_name(ty))
                .collect::<Vec<_>>()
                .join(" | ")
        ),
        (None, true) => return None,
    };
    let range = |offset| {
        let at = lines.position(text, offset);
        json!({ "line": at.line, "character": at.character })
    };
    Some(json!({
        "contents": { "kind": "markdown", "value": value },
        "range": { "start": range(site.prefix_range.start), "end": range(site.prefix_range.end) },
    }))
}

/// The label of `name` called on `receiver` (or of a global method).
fn method_label(
    parsed: &ParsedModule<'_>,
    offset: usize,
    receiver: &[ChainSegment],
    name: &str,
    types: &TypeSources<'_>,
) -> Option<String> {
    let receivers = if receiver.is_empty() {
        vec![Ty {
            id: types.catalog.global()?.id.clone(),
            applied: None,
        }]
    } else {
        chain_types(parsed, offset, receiver, types, VARIABLE_DEPTH)
    };
    receivers.iter().find_map(|ty| {
        let method = types.catalog.type_by_id(&ty.id)?.method(name)?;
        let signature = method.variants.first()?;
        Some(types.catalog.label(&method.ru, signature))
    })
}

#[cfg(test)]
#[path = "../../tests/platform/lsp_types.rs"]
mod tests;
