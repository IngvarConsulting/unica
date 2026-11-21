//! «Отчет», `Report`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Отчет", "Отчёт"],
    metadata: Some(
        MetadataDeclaration::new(&["Reports", "Отчеты", "Отчёты"])
            .manager()
            .object(),
    ),
    root: Some(RootPlace {
        child_objects: 3600,
        tree: Some(3800),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Reports",
        display_name_ru: "Отчёты",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[("ReportObject", "Object"), ("ReportManager", "Manager")]),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("Report")
};
