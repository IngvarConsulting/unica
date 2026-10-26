//! Types of expressions over the platform catalog (#2238, stage 3).
//!
//! The help describes applied types as templates: `Справочники` is a
//! `СправочникиМенеджер` whose placeholder property `<Имя справочника>` is a
//! `СправочникМенеджер.<Имя справочника>`, whose `СоздатьЭлемент` returns a
//! `СправочникОбъект.<Имя справочника>`. A value of a template type carries
//! the applied name it stands for (`Валюты`), so a chain keeps it from step
//! to step; the members only the configuration knows (attributes, tabular
//! sections, module methods) come from [`AppliedTypes`].

use crate::model::{Method, PlatformCatalog, PlatformType, Signature, TypeId, TypeName, TypeRef};

/// The type of a value: a catalog type, and for a template the applied
/// object it stands for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ty {
    pub id: TypeId,
    pub applied: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliedMemberKind {
    Attribute,
    TabularSection,
    Method,
}

/// A member of an applied object the configuration declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMember {
    pub name: String,
    pub kind: AppliedMemberKind,
    /// The value's types as far as the configuration states them.
    pub types: Vec<Ty>,
    /// A method's signature.
    pub signature: Option<Signature>,
}

/// What the configuration knows: the applied half of the model (stage 2).
pub trait AppliedTypes {
    /// The registered objects of a metadata kind (`Catalog`).
    fn names(&self, kind: &str) -> Vec<String>;
    /// The members of `kind` `name` seen as `facet` (`Object`, `Ref`,
    /// `Manager`, ...).
    fn members(&self, kind: &str, name: &str, facet: &str) -> Vec<AppliedMember>;
}

/// A configuration that declares nothing.
pub struct NoAppliedTypes;

impl AppliedTypes for NoAppliedTypes {
    fn names(&self, _: &str) -> Vec<String> {
        Vec::new()
    }

    fn members(&self, _: &str, _: &str, _: &str) -> Vec<AppliedMember> {
        Vec::new()
    }
}

/// Metadata kinds as template types spell them, longest first so that a
/// prefix of another (`Chart...`) does not win.
const TEMPLATE_KINDS: [&str; 20] = [
    "ChartOfCharacteristicTypes",
    "ChartOfCalculationTypes",
    "AccumulationRegister",
    "InformationRegister",
    "CalculationRegister",
    "AccountingRegister",
    "ChartOfAccounts",
    "BusinessProcess",
    "DocumentJournal",
    "FilterCriterion",
    "SettingsStorage",
    "DataProcessor",
    "ExchangePlan",
    "Constant",
    "Document",
    "Sequence",
    "Catalog",
    "Report",
    "Enum",
    "Task",
];

/// The metadata kind and facet a template type stands for:
/// `CatalogObject.<Catalog name>` is `("Catalog", "Object")`.
pub fn template_of(ty: &PlatformType) -> Option<(&'static str, String)> {
    let head = ty.en.split_once(".<")?.0;
    let kind = TEMPLATE_KINDS
        .iter()
        .find(|kind| head.starts_with(**kind))?;
    Some((kind, head[kind.len()..].to_owned()))
}

fn is_placeholder(name: &str) -> bool {
    name.starts_with('<')
}

/// One entry completion offers after a dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberItem {
    pub name: String,
    pub kind: MemberItemKind,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberItemKind {
    Method,
    Property,
    /// An applied object a placeholder stands for (`Валюты` of `Справочники.`).
    AppliedObject,
    Attribute,
    TabularSection,
}

impl PlatformCatalog {
    pub fn global(&self) -> Option<&PlatformType> {
        self.type_by_id("objects/Global context")
    }

    /// The template type of a metadata kind and facet
    /// (`Catalog`, `Object` → `СправочникОбъект.<Имя справочника>`).
    pub fn template(&self, kind: &str, facet: &str) -> Option<&PlatformType> {
        self.types.iter().find(|ty| {
            template_of(ty)
                .is_some_and(|(found, found_facet)| found == kind && found_facet == facet)
        })
    }

    /// The types a member of `receiver` has: a property's value, a
    /// method's return value. Empty when the catalog does not know.
    pub fn member_types(
        &self,
        receiver: &Ty,
        name: &str,
        call: bool,
        applied: &dyn AppliedTypes,
    ) -> Vec<Ty> {
        let Some(ty) = self.type_by_id(&receiver.id) else {
            return Vec::new();
        };
        let template = template_of(ty);
        let carry = |reference: &TypeRef| self.ty_of(reference, template.as_ref(), receiver);
        let found = if call {
            ty.method(name)
                .map(|method| returns(method).filter_map(carry).collect::<Vec<_>>())
        } else {
            ty.property(name).map(|property| {
                if is_placeholder(&property.ru) {
                    Vec::new()
                } else {
                    property.types.iter().filter_map(carry).collect()
                }
            })
        };
        if let Some(found) = found.filter(|found| !found.is_empty()) {
            return dedup(found);
        }
        // A placeholder property stands for every applied object of its
        // kind: `Справочники.Валюты`.
        if !call {
            for property in ty
                .properties
                .iter()
                .filter(|property| is_placeholder(&property.ru))
            {
                for reference in &property.types {
                    let TypeName::Context(id) = &reference.ty else {
                        continue;
                    };
                    let Some((kind, _)) = self.type_by_id(id).and_then(template_of) else {
                        continue;
                    };
                    if let Some(object) = find_name(&applied.names(kind), name) {
                        return vec![Ty {
                            id: id.clone(),
                            applied: Some(object),
                        }];
                    }
                }
            }
        }
        // The configuration's own members of an applied object.
        match (template, &receiver.applied) {
            (Some((kind, facet)), Some(object)) => applied
                .members(kind, object, &facet)
                .into_iter()
                .find(|member| {
                    member.name.to_lowercase() == name.to_lowercase()
                        && (member.kind == AppliedMemberKind::Method) == call
                })
                .map(|member| match &member.signature {
                    Some(signature) if call => signature
                        .returns
                        .iter()
                        .filter_map(|reference| self.ty_of(reference, None, receiver))
                        .collect(),
                    _ => member.types,
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The type a reference names; a template of the receiver's kind keeps
    /// the receiver's applied object.
    fn ty_of(
        &self,
        reference: &TypeRef,
        template: Option<&(&'static str, String)>,
        receiver: &Ty,
    ) -> Option<Ty> {
        let TypeName::Context(id) = &reference.ty else {
            return None;
        };
        let target = self.type_by_id(id)?;
        let applied = match (template_of(target), template) {
            (Some((kind, _)), Some((receiver_kind, _))) if kind == *receiver_kind => {
                receiver.applied.clone()
            }
            _ => None,
        };
        Some(Ty {
            id: id.clone(),
            applied,
        })
    }

    /// What completion offers after a value of `receiver`: its methods and
    /// properties, the applied objects a placeholder stands for, and the
    /// configuration's members of an applied object.
    pub fn completion_members(&self, receiver: &Ty, applied: &dyn AppliedTypes) -> Vec<MemberItem> {
        let Some(ty) = self.type_by_id(&receiver.id) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        if let Some((kind, facet)) = template_of(ty) {
            if let Some(object) = &receiver.applied {
                for member in applied.members(kind, object, &facet) {
                    items.push(MemberItem {
                        kind: match member.kind {
                            AppliedMemberKind::Attribute => MemberItemKind::Attribute,
                            AppliedMemberKind::TabularSection => MemberItemKind::TabularSection,
                            AppliedMemberKind::Method => MemberItemKind::Method,
                        },
                        detail: member
                            .signature
                            .as_ref()
                            .map(|signature| self.label(&member.name, signature)),
                        name: member.name,
                    });
                }
            }
        }
        for property in &ty.properties {
            if is_placeholder(&property.ru) {
                for reference in &property.types {
                    let TypeName::Context(id) = &reference.ty else {
                        continue;
                    };
                    if let Some((kind, _)) = self.type_by_id(id).and_then(template_of) {
                        items.extend(applied.names(kind).into_iter().map(|name| MemberItem {
                            name,
                            kind: MemberItemKind::AppliedObject,
                            detail: None,
                        }));
                    }
                }
                continue;
            }
            items.push(MemberItem {
                name: property.ru.clone(),
                kind: MemberItemKind::Property,
                detail: Some(self.type_names(&property.types)).filter(|names| !names.is_empty()),
            });
        }
        for method in &ty.methods {
            items.push(MemberItem {
                name: method.ru.clone(),
                kind: MemberItemKind::Method,
                detail: method
                    .variants
                    .first()
                    .map(|signature| self.label(&method.ru, signature)),
            });
        }
        items
    }

    /// `Выбрать(ТипОбхода, Группировки): ВыборкаИзРезультатаЗапроса`.
    pub fn label(&self, name: &str, signature: &Signature) -> String {
        let params = signature
            .params
            .iter()
            .map(|param| param.ru.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let returns = self.type_names(&signature.returns);
        if returns.is_empty() {
            format!("{name}({params})")
        } else {
            format!("{name}({params}): {returns}")
        }
    }

    /// Russian names of referenced types, `; `-separated.
    pub fn type_names(&self, references: &[TypeRef]) -> String {
        references
            .iter()
            .map(|reference| self.type_name(&reference.ty))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn type_name(&self, name: &TypeName) -> String {
        match name {
            TypeName::Context(id) => self
                .type_by_id(id)
                .map_or_else(|| id.clone(), |ty| ty.ru.clone()),
            TypeName::Primitive(name) => match name.as_str() {
                "String" => "Строка".into(),
                "Number" => "Число".into(),
                "Boolean" => "Булево".into(),
                "Date" => "Дата".into(),
                "Undefined" => "Неопределено".into(),
                other => other.into(),
            },
            TypeName::Unresolved(name) => name.clone(),
        }
    }

    /// A value's type for people: the catalog name, with the applied
    /// object in place of the template's placeholder.
    pub fn ty_name(&self, ty: &Ty) -> String {
        let Some(found) = self.type_by_id(&ty.id) else {
            return ty.id.clone();
        };
        match (&ty.applied, found.ru.split_once(".<")) {
            (Some(object), Some((head, _))) => format!("{head}.{object}"),
            _ => found.ru.clone(),
        }
    }
}

fn returns(method: &Method) -> impl Iterator<Item = &TypeRef> {
    method
        .variants
        .iter()
        .flat_map(|variant| variant.returns.iter())
}

fn find_name(names: &[String], wanted: &str) -> Option<String> {
    let wanted = wanted.to_lowercase();
    names
        .iter()
        .find(|name| name.to_lowercase() == wanted)
        .cloned()
}

fn dedup(mut types: Vec<Ty>) -> Vec<Ty> {
    let mut seen = std::collections::BTreeSet::new();
    types.retain(|ty| seen.insert(ty.clone()));
    types
}

#[cfg(test)]
#[path = "infer_tests.rs"]
mod tests;
