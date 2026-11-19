//! «Документ», `Document`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Документ"],
    metadata: Some(MetadataDeclaration::new(&["Documents", "Документы"]).manager().object()),
    root: Some(RootPlace { child_objects: 3100, tree: Some(3300) }),
    layout: Some(LayoutDeclaration {
        directory: "Documents",
        display_name_ru: "Документы",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("DocumentObject", "Object"),
        ("DocumentRef", "Ref"),
        ("DocumentSelection", "Selection"),
        ("DocumentList", "List"),
        ("DocumentManager", "Manager"),
    ]),
    child_order: Some(&["Attribute", "Form", "TabularSection", "Template", "Command"]),
    ..KindDeclaration::new("Document")
};
