//! The seam to the applied part of the type model (#2238, stage 2): types
//! of the configuration built from the workspace sources. The platform
//! catalog answers the members a generic help page describes
//! (`СправочникОбъект.<Имя справочника>`); an [`AppliedTypes`] source
//! answers what the configuration adds: its objects, attributes, tabular
//! sections and exported module methods.

use crate::model::{Method, Property, TypeId, TypeName, TypeRef};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// How code sees one object of the configuration: through its manager, an
/// object, a reference, a tabular section of it, and so on. More facets come
/// with the kinds the model learns, so a match keeps a fallback arm.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AppliedFacet {
    /// `Справочники.Валюты`: the manager of the object.
    Manager,
    /// `СправочникОбъект.Валюты`: the object to read and write.
    Object,
    /// `СправочникСсылка.Валюты`.
    Ref,
    /// `СправочникВыборка.Валюты`.
    Selection,
    /// `СправочникСписок.Валюты`.
    List,
    /// `РегистрСведенийНаборЗаписей.Курсы`.
    RecordSet,
    /// `РегистрСведенийМенеджерЗаписи.Курсы`.
    RecordManager,
    /// `РегистрСведенийКлючЗаписи.Курсы`.
    RecordKey,
    /// The tabular section with this name.
    TabularSection(String),
    /// A row of the tabular section with this name.
    TabularSectionRow(String),
    /// A common module, called by its name.
    Module,
}

/// A type of the configuration: the object of kind `kind` (always the
/// English kind name, `Catalog`, `Document`, `CommonModule`, ...) named
/// `name`, seen through `facet`. Names are canonical: as the metadata spells
/// them, so equal types are equal values; matching what code writes in
/// another case or language is the source's job, not the caller's.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AppliedType {
    pub kind: String,
    pub name: String,
    pub facet: AppliedFacet,
}

/// A property the configuration adds: an attribute, a tabular section, a
/// predefined item of a manager, an object of a platform collection manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedProperty {
    /// Its types are full references, applied ones included (an attribute of
    /// type `СправочникСсылка.Валюты` is `Applied{Catalog, Валюты, Ref}`), so a
    /// chain goes on through it. Empty types: unknown, not guessed. A name of
    /// the configuration has no English form: `en` repeats `ru`.
    pub property: Property,
    /// The logical address of what declares it, when it has one.
    pub at: Option<String>,
}

/// An exported method of a module of the configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedMethod {
    /// Parameters and return types come from the declaration and its
    /// comment; a type the comment does not name is empty, not guessed.
    pub method: Method,
    /// The logical address of the method.
    pub at: String,
    /// The normative region of the method, in the vocabulary of
    /// INV.SOURCE.MODULE-REGION-INTERFACE (`public`, `internal`,
    /// `crossSubsystem`, `override`, `private`, `deprecated`).
    pub area: Option<String>,
}

/// What the configuration adds to one type.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppliedMembers {
    /// The template page of the catalog whose generic members this type also
    /// has (`CatalogObject.<Catalog name>` for `Applied{Catalog, Object}`);
    /// they are not repeated here.
    pub platform: Option<TypeId>,
    pub properties: Vec<AppliedProperty>,
    pub methods: Vec<AppliedMethod>,
    /// Some members could not be read: the answer is not the whole type
    /// (INV.LSP.PARTIAL-ANSWERS).
    pub partial: bool,
}

/// The applied part of the type model, as type inference asks it. A server
/// holds one source for every request, so it is shared across threads.
///
/// Names are matched in Russian or English and in any case, as BSL does.
pub trait AppliedTypes: Send + Sync {
    /// The type of a global name of the configuration: a common module
    /// (`ОбщегоНазначения`). Platform globals (`Справочники`) are the
    /// catalog's; `None` for any other name.
    fn global(&self, name: &str) -> Option<TypeRef>;

    /// What the configuration adds to `ty`: the own members of an applied
    /// type (the predefined items of a manager among them), or the objects of the configuration a platform collection
    /// manager holds (`Справочники.Валюты` is a property of
    /// `СправочникиМенеджер` typed as the catalog's manager). `None` when the
    /// configuration adds nothing it knows of to this type.
    fn members(&self, ty: &TypeName) -> Option<Arc<AppliedMembers>>;
}

/// No configuration: inference over the platform catalog alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAppliedTypes;

impl AppliedTypes for NoAppliedTypes {
    fn global(&self, _name: &str) -> Option<TypeRef> {
        None
    }

    fn members(&self, _ty: &TypeName) -> Option<Arc<AppliedMembers>> {
        None
    }
}
