//! «Справочник», `Catalog`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Справочник"],
    metadata: Some(
        MetadataDeclaration::new(&["Catalogs", "Справочники"])
            .manager()
            .object(),
    ),
    root: Some(RootPlace {
        child_objects: 3000,
        tree: Some(3200),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Catalogs",
        display_name_ru: "Справочники",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("CatalogObject", "Object"),
        ("CatalogRef", "Ref"),
        ("CatalogSelection", "Selection"),
        ("CatalogList", "List"),
        ("CatalogManager", "Manager"),
    ]),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("Catalog")
};
