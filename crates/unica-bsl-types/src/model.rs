//! The platform catalog: types of the 1C platform as its help describes
//! them, one record per type, member and parameter in both languages.

use serde::{Deserialize, Serialize};

/// The catalog of one platform installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformCatalog {
    /// The platform build the help comes from (`8.3.27.2074`).
    pub platform: String,
    /// The version of the rules the catalog was built by.
    pub generator: u32,
    /// Every type, the global context included, sorted by id.
    pub types: Vec<PlatformType>,
}

/// A type of the platform: its help page and the member pages under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformType {
    /// The help page of the type without `.html`
    /// (`objects/catalog213/catalog393/Query`); stable across languages.
    pub id: TypeId,
    pub ru: String,
    pub en: String,
    pub properties: Vec<Property>,
    pub methods: Vec<Method>,
    pub events: Vec<Method>,
    pub constructors: Vec<Signature>,
    /// What `Для Каждого` walks, when the help names it.
    pub collection_element: Vec<TypeRef>,
    pub contexts: Vec<Context>,
}

pub type TypeId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    pub ru: String,
    pub en: String,
    /// Empty when the help names no type: unknown, not guessed.
    pub types: Vec<TypeRef>,
    pub read_only: bool,
    pub contexts: Vec<Context>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Method {
    pub ru: String,
    pub en: String,
    /// One per syntax variant, in the order of the help.
    pub variants: Vec<Signature>,
    pub contexts: Vec<Context>,
    /// The help notes that calling it from a client goes to the server.
    pub server_call: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// The variant name (`Из строки`); empty for the only one.
    pub name: String,
    pub params: Vec<Param>,
    /// Empty for a procedure and when the help names no type.
    pub returns: Vec<TypeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub ru: String,
    /// The same parameter on the English page; empty when that page lists
    /// fewer parameters.
    pub en: String,
    pub optional: bool,
    pub types: Vec<TypeRef>,
    /// The default the help states, as text.
    pub default: Option<String>,
}

/// A type the help names, by its page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeRef {
    pub ty: TypeName,
    /// The element type of an array (`Массив из элементов типа X`).
    pub element: Option<TypeName>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TypeName {
    /// A type with its own page in the catalog.
    Context(TypeId),
    /// A language type (`String`, `Number`, `Undefined`, ...).
    Primitive(String),
    /// A type the catalog has no page for, kept as written: a dangling link
    /// or a name in text (`Произвольный`).
    Unresolved(String),
}

/// Where code runs, as the help's «Доступность» lists it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Context {
    ThinClient,
    WebClient,
    MobileClient,
    Server,
    ThickClient,
    ExternalConnection,
    MobileAppClient,
    MobileAppServer,
    MobileStandaloneServer,
    Integration,
    /// A place the catalog does not know yet, kept as written.
    Other(String),
}

impl PlatformCatalog {
    pub fn type_by_id(&self, id: &str) -> Option<&PlatformType> {
        self.types
            .binary_search_by(|ty| ty.id.as_str().cmp(id))
            .ok()
            .map(|index| &self.types[index])
    }

    /// The type with this Russian or English name, in any case; `None`
    /// when no type or more than one has it.
    pub fn type_by_name(&self, name: &str) -> Option<&PlatformType> {
        let name = name.to_lowercase();
        let mut found = self
            .types
            .iter()
            .filter(|ty| ty.ru.to_lowercase() == name || ty.en.to_lowercase() == name);
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }
}

impl PlatformType {
    /// A method by its Russian or English name, in any case.
    pub fn method(&self, name: &str) -> Option<&Method> {
        let name = name.to_lowercase();
        self.methods
            .iter()
            .find(|method| method.ru.to_lowercase() == name || method.en.to_lowercase() == name)
    }

    pub fn property(&self, name: &str) -> Option<&Property> {
        let name = name.to_lowercase();
        self.properties.iter().find(|property| {
            property.ru.to_lowercase() == name || property.en.to_lowercase() == name
        })
    }
}
