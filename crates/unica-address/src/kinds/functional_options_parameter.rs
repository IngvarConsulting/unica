//! «ПараметрФункциональнойОпции», `FunctionalOptionsParameter`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПараметрФункциональнойОпции"],
    metadata: Some(MetadataDeclaration::new(&["FunctionalOptionsParameters", "ПараметрыФункциональныхОпций"])),
    root: Some(RootPlace { child_objects: 2100, tree: Some(1300) }),
    layout: Some(LayoutDeclaration {
        directory: "FunctionalOptionsParameters",
        display_name_ru: "Параметры ФО",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("FunctionalOptionsParameter")
};
