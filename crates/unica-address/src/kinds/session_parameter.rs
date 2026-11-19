//! «ПараметрСеанса», `SessionParameter`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПараметрСеанса"],
    metadata: Some(MetadataDeclaration::new(&["SessionParameters", "ПараметрыСеанса"]).manager()),
    root: Some(RootPlace {
        child_objects: 600,
        tree: Some(300),
    }),
    layout: Some(LayoutDeclaration {
        directory: "SessionParameters",
        display_name_ru: "Параметры сеанса",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("SessionParameter")
};
