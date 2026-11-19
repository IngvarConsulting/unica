//! «Задача», `Task`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Задача"],
    metadata: Some(
        MetadataDeclaration::new(&["Tasks", "Задачи"])
            .manager()
            .object(),
    ),
    root: Some(RootPlace {
        child_objects: 4600,
        tree: Some(4800),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Tasks",
        display_name_ru: "Задачи",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("TaskObject", "Object"),
        ("TaskRef", "Ref"),
        ("TaskSelection", "Selection"),
        ("TaskList", "List"),
        ("TaskManager", "Manager"),
    ]),
    child_order: Some(&[
        "Attribute",
        "TabularSection",
        "Form",
        "Template",
        "AddressingAttribute",
        "Command",
    ]),
    ..KindDeclaration::new("Task")
};
