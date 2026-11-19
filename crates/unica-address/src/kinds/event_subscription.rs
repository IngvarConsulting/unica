//! «ПодпискаНаСобытие», `EventSubscription`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПодпискаНаСобытие"],
    metadata: Some(MetadataDeclaration::new(&["EventSubscriptions", "ПодпискиНаСобытия"])),
    root: Some(RootPlace { child_objects: 1700, tree: Some(800) }),
    layout: Some(LayoutDeclaration {
        directory: "EventSubscriptions",
        display_name_ru: "Подписки на события",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("EventSubscription")
};
