//! «СервисИнтеграции», `IntegrationService`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["СервисИнтеграции"],
    metadata: Some(MetadataDeclaration::new(&[
        "IntegrationServices",
        "СервисыИнтеграции",
    ])),
    root: Some(RootPlace {
        child_objects: 4700,
        tree: Some(2600),
    }),
    layout: Some(LayoutDeclaration {
        directory: "IntegrationServices",
        display_name_ru: "Сервисы интеграции",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[("IntegrationServiceManager", "Manager")]),
    ..KindDeclaration::new("IntegrationService")
};
