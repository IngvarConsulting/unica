//! «Обработка», `DataProcessor`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Обработка"],
    metadata: Some(
        MetadataDeclaration::new(&["DataProcessors", "Обработки"])
            .manager()
            .object(),
    ),
    root: Some(RootPlace {
        child_objects: 3700,
        tree: Some(3900),
    }),
    layout: Some(LayoutDeclaration {
        directory: "DataProcessors",
        display_name_ru: "Обработки",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("DataProcessorObject", "Object"),
        ("DataProcessorManager", "Manager"),
    ]),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("DataProcessor")
};
