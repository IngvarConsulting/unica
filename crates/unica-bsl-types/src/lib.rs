//! The BSL type model (#2238). This stage builds the platform catalog from
//! the help of an installed platform: types, their members, parameters,
//! return types and execution contexts, Russian and English names in one
//! record. The Russian and English help hold the same pages under the same
//! paths: a page path is the key that matches the two languages.

mod applied;
mod cache;
mod model;
mod page;

pub use applied::{
    AppliedFacet, AppliedMembers, AppliedMethod, AppliedProperty, AppliedType, AppliedTypes,
    NoAppliedTypes,
};

pub use cache::{catalog_path, load_or_build, CatalogError, Containers};
pub use model::{
    Context, Method, Param, PlatformCatalog, PlatformType, Property, Signature, TypeId, TypeName,
    TypeRef,
};

use page::{Page, PageKind, RawSignature};
use std::collections::{BTreeMap, BTreeSet};

/// The version of the rules a catalog is built by. A change of what the
/// catalog holds or how pages are read raises it, so no old catalog is
/// served by new rules.
pub const GENERATOR: u32 = 1;

/// The catalog of one installation from its help pages: Russian pages and
/// English pages as `(path, html)`.
pub fn build_catalog(
    platform: &str,
    russian: &[(String, String)],
    english: &[(String, String)],
) -> PlatformCatalog {
    let english = english
        .iter()
        .filter_map(|(path, html)| page::parse(path, html))
        .map(|page| (page.id.clone(), page))
        .collect::<BTreeMap<_, _>>();
    let pages = russian
        .iter()
        .filter_map(|(path, html)| page::parse(path, html))
        .collect::<Vec<_>>();

    let mut types = BTreeMap::<String, PlatformType>::new();
    for page in pages.iter().filter(|page| page.kind == PageKind::Type) {
        let en = english_name(page, english.get(&page.id));
        types.insert(
            page.id.clone(),
            PlatformType {
                id: page.id.clone(),
                ru: page.name.clone(),
                en,
                properties: Vec::new(),
                methods: Vec::new(),
                events: Vec::new(),
                constructors: Vec::new(),
                collection_element: page.collection_element.clone(),
                contexts: page.contexts.clone(),
            },
        );
    }
    for page in pages.iter().filter(|page| page.kind != PageKind::Type) {
        let Some(ty) = page.owner.as_ref().and_then(|owner| types.get_mut(owner)) else {
            continue;
        };
        let english_page = english.get(&page.id);
        match page.kind {
            PageKind::Method | PageKind::Event => {
                let member = Method {
                    ru: page.name.clone(),
                    en: english_name(page, english_page),
                    variants: signatures(&page.variants, english_page),
                    contexts: page.contexts.clone(),
                    server_call: page.server_call,
                };
                if page.kind == PageKind::Method {
                    ty.methods.push(member);
                } else {
                    ty.events.push(member);
                }
            }
            PageKind::Property => ty.properties.push(Property {
                ru: page.name.clone(),
                en: english_name(page, english_page),
                types: page.types.clone(),
                read_only: page.read_only,
                contexts: page.contexts.clone(),
            }),
            PageKind::Constructor => {
                // A constructor page names its variant, not a member.
                for mut signature in signatures(&page.variants, english_page) {
                    if signature.name.is_empty() {
                        signature.name = page.name.clone();
                    }
                    ty.constructors.push(signature);
                }
            }
            PageKind::Type => {}
        }
    }

    let known = types.keys().cloned().collect::<BTreeSet<_>>();
    let mut types = types.into_values().collect::<Vec<_>>();
    for ty in &mut types {
        ty.properties.sort_by(|a, b| a.ru.cmp(&b.ru));
        ty.methods.sort_by(|a, b| a.ru.cmp(&b.ru));
        ty.events.sort_by(|a, b| a.ru.cmp(&b.ru));
        resolve_type(ty, &known);
    }
    PlatformCatalog {
        platform: platform.to_owned(),
        generator: GENERATOR,
        types,
    }
}

/// The English name: from the Russian heading's parentheses, else the
/// English page's heading.
fn english_name(page: &Page, english: Option<&Page>) -> String {
    page.alias
        .clone()
        .or_else(|| english.map(|english| english.name.clone()))
        .unwrap_or_default()
}

/// The Russian variants with the English parameter names of the same
/// variant, by position, when the English variant has as many.
fn signatures(russian: &[RawSignature], english: Option<&Page>) -> Vec<Signature> {
    russian
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            // Positions match only when both pages list as many parameters;
            // otherwise the English names stay empty rather than misplaced.
            let english = english
                .and_then(|page| page.variants.get(index))
                .filter(|english| english.params.len() == variant.params.len());
            Signature {
                name: variant.name.clone(),
                params: variant
                    .params
                    .iter()
                    .enumerate()
                    .map(|(position, param)| Param {
                        ru: param.name.clone(),
                        en: english
                            .and_then(|variant| variant.params.get(position))
                            .map(|param| param.name.clone())
                            .unwrap_or_default(),
                        optional: param.optional,
                        types: param.types.clone(),
                        default: param.default.clone(),
                    })
                    .collect(),
                returns: variant.returns.clone(),
            }
        })
        .collect()
}

/// A link to a page the catalog has no type for stays as written: the
/// help named it, the catalog cannot say what it is.
fn resolve_type(ty: &mut PlatformType, known: &BTreeSet<String>) {
    let resolve = |name: &mut TypeName| {
        if let TypeName::Context(id) = name {
            if !known.contains(id.as_str()) {
                *name = TypeName::Unresolved(std::mem::take(id));
            }
        }
    };
    let resolve_ref = |reference: &mut TypeRef| {
        resolve(&mut reference.ty);
        if let Some(element) = reference.element.as_mut() {
            resolve(element);
        }
    };
    ty.collection_element.iter_mut().for_each(resolve_ref);
    for property in &mut ty.properties {
        property.types.iter_mut().for_each(resolve_ref);
    }
    for signature in ty
        .methods
        .iter_mut()
        .chain(ty.events.iter_mut())
        .flat_map(|method| method.variants.iter_mut())
        .chain(ty.constructors.iter_mut())
    {
        signature.returns.iter_mut().for_each(resolve_ref);
        for param in &mut signature.params {
            param.types.iter_mut().for_each(resolve_ref);
        }
    }
}

#[cfg(test)]
mod tests;
