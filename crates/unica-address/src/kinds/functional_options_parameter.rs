//! «ПараметрФункциональнойОпции», `FunctionalOptionsParameter`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПараметрФункциональнойОпции"],
    metadata: Some(MetadataDeclaration::new(&[
        "FunctionalOptionsParameters",
        "ПараметрыФункциональныхОпций",
    ])),
    root: Some(RootPlace {
        child_objects: 2100,
        tree: Some(1300),
    }),
    layout: Some(LayoutDeclaration {
        directory: "FunctionalOptionsParameters",
        display_name_ru: "Параметры ФО",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("FunctionalOptionsParameter")
};
