//! The applied part of the BSL type model (#2238, stage 2): the types a
//! configuration adds to the platform — managers, objects, references,
//! record sets, tabular sections, attributes with their types and the
//! exported methods of modules with their regions and documented types.
//!
//! The builder is a pure function of facts the caller reads from the
//! sources; caching by epoch is the caller's. What the platform catalog
//! knows comes through [`Templates`].

mod doc;
mod facts;

pub use facts::{
    Declared, DefinedTypeFacts, FieldFacts, FieldRole, FieldType, MethodFacts, ModuleFacts,
    ModuleKind, ObjectFacts, ParamFacts, SectionFacts, Templates,
};

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use unica_bsl_types::{
    AppliedFacet, AppliedMembers, AppliedMethod, AppliedProperty, AppliedType, AppliedTypes,
    Method, Param, Property, Signature, TypeId, TypeName, TypeRef,
};

/// The applied types of one configuration.
#[derive(Debug, Clone, Default)]
pub struct AppliedModel {
    members: HashMap<AppliedType, Arc<AppliedMembers>>,
    collections: HashMap<TypeId, Arc<AppliedMembers>>,
    /// Common modules by their lower-cased name.
    globals: HashMap<String, TypeRef>,
}

impl AppliedTypes for AppliedModel {
    fn global(&self, name: &str) -> Option<TypeRef> {
        self.globals.get(&name.to_lowercase()).cloned()
    }

    fn members(&self, ty: &TypeName) -> Option<Arc<AppliedMembers>> {
        match ty {
            TypeName::Applied(applied) => self.members.get(applied).cloned(),
            TypeName::Context(id) => self.collections.get(id).cloned(),
            _ => None,
        }
    }
}

/// The facets `object` is seen through, tabular sections aside.
fn facets(object: &ObjectFacts) -> &'static [AppliedFacet] {
    use AppliedFacet::*;
    match object.kind.as_str() {
        "Catalog"
        | "Document"
        | "ChartOfCharacteristicTypes"
        | "ChartOfAccounts"
        | "ChartOfCalculationTypes"
        | "ExchangePlan"
        | "BusinessProcess"
        | "Task" => &[Manager, Object, Ref, Selection, List],
        "Enum" => &[Manager, Ref, List],
        "DataProcessor" | "Report" => &[Manager, Object],
        // Only an independent information register has a record manager.
        "InformationRegister" if !object.subordinate => &[
            Manager,
            RecordSet,
            Record,
            RecordManager,
            RecordKey,
            Selection,
            List,
        ],
        "InformationRegister" => &[Manager, RecordSet, Record, RecordKey, Selection, List],
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister" => {
            &[Manager, RecordSet, Record, RecordKey, Selection, List]
        }
        "Constant" => &[Manager, ValueManager],
        "DocumentJournal" => &[Manager, Selection, List],
        "CommonModule" => &[Module],
        _ => &[],
    }
}

/// Kinds whose facets carry tabular sections.
fn has_tabular_sections(kind: &str) -> bool {
    matches!(
        kind,
        "Catalog"
            | "Document"
            | "ChartOfCharacteristicTypes"
            | "ChartOfAccounts"
            | "ChartOfCalculationTypes"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
            | "DataProcessor"
            | "Report"
    )
}

/// Builds the model of one configuration.
/// Builds the model of `objects`. `declared` answers for the objects of
/// the configuration that are not built here, so a model of one object still
/// resolves the names its comments give.
pub fn build(
    objects: &[ObjectFacts],
    defined: &[DefinedTypeFacts],
    declared: &dyn Declared,
    templates: &dyn Templates,
) -> AppliedModel {
    let defined = defined
        .iter()
        .map(|facts| (facts.name.as_str(), facts.types.as_slice()))
        .collect::<HashMap<_, _>>();
    // Comments name objects in any case; the model keys them as declared.
    let built = objects
        .iter()
        .map(|object| {
            (
                (object.kind.clone(), object.name.to_lowercase()),
                object.name.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let builder = Builder {
        defined: &defined,
        built: &built,
        declared,
        templates,
    };
    let mut model = AppliedModel::default();
    let mut collections = BTreeMap::<TypeId, AppliedMembers>::new();
    for object in objects {
        let applied = |facet: AppliedFacet| AppliedType {
            kind: object.kind.clone(),
            name: object.name.clone(),
            facet,
        };
        for facet in facets(object) {
            let ty = applied(facet.clone());
            let members = builder.members(object, &ty);
            model.members.insert(ty, Arc::new(members));
        }
        if has_tabular_sections(&object.kind) {
            for section in &object.tabular_sections {
                for facet in [
                    AppliedFacet::TabularSection(section.name.clone()),
                    AppliedFacet::TabularSectionRow(section.name.clone()),
                ] {
                    let ty = applied(facet);
                    let members = builder.members(object, &ty);
                    model.members.insert(ty, Arc::new(members));
                }
            }
        }
        if object.kind == "CommonModule" {
            model.globals.insert(
                object.name.to_lowercase(),
                reference(applied(AppliedFacet::Module)),
            );
        } else if let Some(collection) = (!facets(object).is_empty())
            .then(|| templates.collection(&object.kind))
            .flatten()
        {
            // `Справочники.Валюты` and `Константы.Валюта` are managers.
            let holder = applied(AppliedFacet::Manager);
            let entry = collections.entry(collection).or_default();
            entry.properties.push(AppliedProperty {
                property: property(&object.name, vec![reference(holder)], true),
                at: Some(object.at.clone()),
            });
            entry.partial |= object.partial;
        }
    }
    model.collections = collections
        .into_iter()
        .map(|(id, members)| (id, Arc::new(members)))
        .collect();
    model
}

fn reference(ty: AppliedType) -> TypeRef {
    TypeRef {
        ty: TypeName::Applied(ty),
        element: None,
    }
}

/// A member of the configuration: its name has no English form.
fn property(name: &str, types: Vec<TypeRef>, read_only: bool) -> Property {
    Property {
        ru: name.to_string(),
        en: name.to_string(),
        types,
        read_only,
        contexts: Vec::new(),
    }
}

struct Builder<'a> {
    defined: &'a HashMap<&'a str, &'a [FieldType]>,
    built: &'a HashMap<(String, String), String>,
    declared: &'a dyn Declared,
    templates: &'a dyn Templates,
}

impl Builder<'_> {
    fn members(&self, object: &ObjectFacts, ty: &AppliedType) -> AppliedMembers {
        use AppliedFacet::*;
        let mut members = AppliedMembers {
            platform: self.templates.template(ty),
            partial: object.partial,
            ..AppliedMembers::default()
        };
        let own = |facet: AppliedFacet| AppliedType {
            kind: object.kind.clone(),
            name: object.name.clone(),
            facet,
        };
        let module = |kind: ModuleKind| {
            object
                .modules
                .iter()
                .filter(move |module| module.kind == kind)
                .flat_map(|module| {
                    module
                        .methods
                        .iter()
                        .filter(|method| method.export)
                        .map(move |method| (method, module.server_call))
                })
        };
        match &ty.facet {
            Manager => {
                // Predefined items and enum values are references of the object.
                if object.kind != "Constant" {
                    for name in &object.predefined {
                        members.properties.push(AppliedProperty {
                            property: property(name, vec![reference(own(Ref))], true),
                            at: None,
                        });
                    }
                }
                members.methods = module(ModuleKind::Manager)
                    .map(|(method, server_call)| self.method(method, server_call))
                    .collect();
            }
            Object | Ref | Selection => {
                let read_only = ty.facet != Object;
                self.fields(&mut members, &object.fields, |_| true, read_only);
                if has_tabular_sections(&object.kind) {
                    for section in &object.tabular_sections {
                        members.properties.push(AppliedProperty {
                            property: property(
                                &section.name,
                                vec![reference(own(TabularSection(section.name.clone())))],
                                read_only,
                            ),
                            at: Some(section.at.clone()),
                        });
                    }
                }
                if ty.facet == Object {
                    members.methods = module(ModuleKind::Object)
                        .map(|(method, server_call)| self.method(method, server_call))
                        .collect();
                }
            }
            Record | RecordManager => {
                self.fields(&mut members, &object.fields, |_| true, false);
            }
            // The key of a subordinate register is its recorder and line,
            // which the template names.
            RecordKey if !object.subordinate => {
                self.fields(
                    &mut members,
                    &object.fields,
                    |role| role == FieldRole::Dimension,
                    true,
                );
            }
            RecordSet => {
                members.methods = module(ModuleKind::RecordSet)
                    .map(|(method, server_call)| self.method(method, server_call))
                    .collect();
                members.element = Some(reference(own(Record)));
            }
            TabularSection(name) => {
                members.element = Some(reference(own(TabularSectionRow(name.clone()))));
            }
            // The value of a constant is its one field.
            ValueManager => {
                self.fields(&mut members, &object.fields, |_| true, false);
            }
            TabularSectionRow(name) => {
                if let Some(section) = object
                    .tabular_sections
                    .iter()
                    .find(|section| &section.name == name)
                {
                    self.fields(&mut members, &section.fields, |_| true, false);
                }
            }
            Module => {
                members.methods = module(ModuleKind::Common)
                    .map(|(method, server_call)| self.method(method, server_call))
                    .collect();
            }
            // The template answers these alone.
            _ => {}
        }
        members
    }

    fn fields(
        &self,
        members: &mut AppliedMembers,
        fields: &[FieldFacts],
        keep: impl Fn(FieldRole) -> bool,
        read_only: bool,
    ) {
        for field in fields.iter().filter(|field| keep(field.role)) {
            members.properties.push(AppliedProperty {
                property: property(&field.name, self.field_types(&field.types), read_only),
                at: Some(field.at.clone()),
            });
        }
    }

    fn field_types(&self, types: &[FieldType]) -> Vec<TypeRef> {
        let mut found = Vec::new();
        self.expand(types, &mut found, 0);
        found
    }

    fn expand(&self, types: &[FieldType], found: &mut Vec<TypeRef>, depth: usize) {
        for ty in types {
            let reference = match ty {
                FieldType::Primitive(name) => Some(TypeRef {
                    ty: TypeName::Primitive(name.clone()),
                    element: None,
                }),
                FieldType::Reference { kind, name } => Some(reference(AppliedType {
                    kind: kind.clone(),
                    name: name.clone(),
                    facet: AppliedFacet::Ref,
                })),
                FieldType::Platform(name) => self
                    .templates
                    .platform(name)
                    .map(|ty| TypeRef { ty, element: None }),
                // A defined type cannot name another; the depth guards a
                // malformed source, not a real chain.
                FieldType::Defined(name) => {
                    if depth < 4 {
                        if let Some(inner) = self.defined.get(name.as_str()) {
                            self.expand(inner, found, depth + 1);
                        }
                    }
                    None
                }
                FieldType::Unknown => None,
            };
            if let Some(reference) = reference {
                if !found.contains(&reference) {
                    found.push(reference);
                }
            }
        }
    }

    fn method(&self, facts: &MethodFacts, server_call: bool) -> AppliedMethod {
        let documented = facts.doc.as_deref().map(doc::doc_types).unwrap_or_default();
        let params = facts
            .params
            .iter()
            .map(|param| Param {
                ru: param.name.clone(),
                en: param.name.clone(),
                optional: param.optional,
                types: documented
                    .params
                    .iter()
                    .find(|(name, _)| name.to_lowercase() == param.name.to_lowercase())
                    .map(|(_, texts)| self.resolve_all(texts))
                    .unwrap_or_default(),
                default: param.default.clone(),
            })
            .collect();
        let returns = if facts.function {
            self.resolve_all(&documented.returns)
        } else {
            Vec::new()
        };
        AppliedMethod {
            method: Method {
                ru: facts.name.clone(),
                en: facts.name.clone(),
                variants: vec![Signature {
                    name: String::new(),
                    params,
                    returns,
                }],
                contexts: facts.contexts.clone(),
                server_call,
            },
            at: facts.at.clone(),
            area: facts.area.clone(),
        }
    }

    fn resolve_all(&self, texts: &[String]) -> Vec<TypeRef> {
        let mut found = Vec::new();
        for text in texts {
            let reference = self.resolve(text);
            if !found.contains(&reference) {
                found.push(reference);
            }
        }
        found
    }

    /// A type text of a comment: `Массив из X`, `СправочникСсылка.Валюты`, a
    /// platform type or a language type. What none of them names stays
    /// `Unresolved` as written.
    fn resolve(&self, text: &str) -> TypeRef {
        if let Some(element) = doc::strip_array(text) {
            let array = || {
                self.templates
                    .platform("Массив")
                    .unwrap_or_else(|| TypeName::Unresolved("Массив".to_string()))
            };
            // TypeRef represents the immediate element only. A nested array
            // needs its Array type, not recursion into unrepresented depths.
            let element = if doc::strip_array(element).is_some() {
                array()
            } else {
                self.resolve(element).ty
            };
            return TypeRef {
                ty: array(),
                element: Some(element),
            };
        }
        if let Some((prefix, name)) = text.split_once('.') {
            if let Some((kind, facet)) = self.templates.applied_prefix(prefix) {
                // Only an object of this configuration, keyed as declared.
                let declared = self
                    .built
                    .get(&(kind.clone(), name.to_lowercase()))
                    .cloned()
                    .or_else(|| self.declared.declared(&kind, name));
                if let Some(name) = declared {
                    return reference(AppliedType { kind, name, facet });
                }
            }
        }
        TypeRef {
            ty: self
                .templates
                .platform(text)
                .unwrap_or_else(|| TypeName::Unresolved(text.to_string())),
            element: None,
        }
    }
}

#[cfg(test)]
mod tests;
