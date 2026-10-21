//! Shared metadata owner and collection facts, independent of representation.
use crate::{metadata_kind_collections, MetaCollection, MetadataKind};
use unica_address::address::NodeKind;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;

pub fn metadata_collection_matches(kind: NodeKind, collection: MetaCollection) -> bool {
    MetadataKind::parse(kind.as_str())
        .ok()
        .is_some_and(|kind| metadata_kind_collections(kind).contains(&collection))
}

pub trait MetadataSiteExt {
    fn metadata_object(&self) -> Option<MetadataKind>;
    fn metadata_owner_with(&self, collection: MetaCollection) -> bool;
    fn owned_form(&self) -> bool;
    fn owned_template(&self) -> bool;
    fn metadata_member(&self, kind: NodeKind, collection: MetaCollection) -> bool;
}
impl MetadataSiteExt for OperationSite<'_> {
    /// Объект метаданных верхнего уровня одного из типизированных видов:
    /// `Catalog.X`, `CommonModule.X`. Маршрут не важен — общий модуль читается
    /// как модуль, но остаётся объектом метаданных.
    fn metadata_object(&self) -> Option<MetadataKind> {
        match self.segments() {
            [owner] if owner.name().is_some() => MetadataKind::parse(owner.kind().as_str()).ok(),
            _ => None,
        }
    }

    fn metadata_owner_with(&self, collection: MetaCollection) -> bool {
        self.metadata_object()
            .is_some_and(|kind| metadata_kind_collections(kind).contains(&collection))
    }

    /// Форма объекта: `Owner.X.Form.F`, где у владельца есть коллекция форм.
    fn owned_form(&self) -> bool {
        matches!(self.segments(), [owner, form]
            if owner.name().is_some()
                && form.kind() == NodeKind::Form
                && form.name().is_some()
                && metadata_collection_matches(owner.kind(), MetaCollection::Forms))
    }

    /// Табличный документ или схема как узел владельца: `Owner.X.Template.T`.
    fn owned_template(&self) -> bool {
        self.metadata_member(NodeKind::Template, MetaCollection::Templates)
    }

    /// Член коллекции объекта метаданных верхнего уровня:
    /// `Owner.X.<kind>.Name`, где у вида владельца есть коллекция `collection`.
    fn metadata_member(&self, kind: NodeKind, collection: MetaCollection) -> bool {
        self.route() == RouteClass::Metadata
            && matches!(self.segments(), [owner, member]
                if owner.name().is_some()
                    && member.kind() == kind
                    && member.name().is_some()
                    && metadata_collection_matches(owner.kind(), collection))
    }
}
