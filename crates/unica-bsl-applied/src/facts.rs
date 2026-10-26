//! What the builder reads: plain facts about the objects of a configuration,
//! taken from their descriptors and module projections by the caller.

use unica_bsl_types::{AppliedFacet, AppliedType, Context, TypeId, TypeName};

/// One object of the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectFacts {
    /// The English kind name (`Catalog`, `InformationRegister`, ...).
    pub kind: String,
    /// The name as the metadata spells it.
    pub name: String,
    /// The logical address of the object.
    pub at: String,
    /// Attributes, and for a register its dimensions and resources, for a
    /// journal its columns, for a constant its value as one field.
    pub fields: Vec<FieldFacts>,
    pub tabular_sections: Vec<SectionFacts>,
    /// Predefined items of a catalog or a chart, values of an enumeration.
    pub predefined: Vec<String>,
    pub modules: Vec<ModuleFacts>,
    /// A register whose records belong to a recorder: its record key is the
    /// recorder and line, not the dimensions, and it has no record manager.
    /// Accumulation, accounting and calculation registers always are.
    pub subordinate: bool,
    /// Some facts could not be read: members built from them are partial.
    pub partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRole {
    Attribute,
    Dimension,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldFacts {
    pub name: String,
    pub at: String,
    pub role: FieldRole,
    /// The types of the descriptor; empty when it names none readable.
    pub types: Vec<FieldType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionFacts {
    pub name: String,
    pub at: String,
    pub fields: Vec<FieldFacts>,
}

/// A type as a descriptor writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    /// A language type in the catalog's spelling (`String`, `Number`,
    /// `Boolean`, `Date`).
    Primitive(String),
    /// A reference to an object of the configuration (`cfg:CatalogRef.X`).
    Reference { kind: String, name: String },
    /// A defined type of the configuration, expanded by the builder.
    Defined(String),
    /// A platform type by its Russian or English name (`ValueStorage`).
    Platform(String),
    /// Something the model does not express: characteristic types, a type
    /// identifier. The member stays without this type.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    Manager,
    Object,
    RecordSet,
    Common,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFacts {
    pub kind: ModuleKind,
    /// A common module with server call: a client call goes to the server.
    pub server_call: bool,
    pub methods: Vec<MethodFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodFacts {
    pub name: String,
    pub at: String,
    pub export: bool,
    pub function: bool,
    /// The normative region (INV.SOURCE.MODULE-REGION-INTERFACE).
    pub area: Option<String>,
    pub params: Vec<ParamFacts>,
    /// The documenting comment above the method, without `//`.
    pub doc: Option<String>,
    pub contexts: Vec<Context>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamFacts {
    pub name: String,
    pub optional: bool,
    pub default: Option<String>,
}

/// A defined type of the configuration and the types it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedTypeFacts {
    pub name: String,
    pub types: Vec<FieldType>,
}

/// What the platform catalog tells the builder. The stage 3 side
/// implements it over the catalog: the translation between template pages
/// and applied types lives there, once.
pub trait Templates {
    /// The catalog page of the template whose generic members `ty` has.
    fn template(&self, ty: &AppliedType) -> Option<TypeId>;
    /// The platform collection manager that holds the objects of `kind`
    /// (`CatalogsManager` for `Catalog`).
    fn collection(&self, kind: &str) -> Option<TypeId>;
    /// The applied kind and facet a type name prefix stands for, in Russian
    /// or English (`СправочникСсылка` → `Catalog`, `Ref`).
    fn applied_prefix(&self, prefix: &str) -> Option<(String, AppliedFacet)>;
    /// A platform type by its Russian or English name, primitives included.
    fn platform(&self, name: &str) -> Option<TypeName>;
}
