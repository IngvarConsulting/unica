//! «ПодпискаНаСобытие», `EventSubscription`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПодпискаНаСобытие"],
    metadata: Some(MetadataDeclaration::new(&[
        "EventSubscriptions",
        "ПодпискиНаСобытия",
    ])),
    root: Some(RootPlace {
        child_objects: 1700,
        tree: Some(800),
    }),
    layout: Some(LayoutDeclaration {
        directory: "EventSubscriptions",
        display_name_ru: "Подписки на события",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("EventSubscription")
};
