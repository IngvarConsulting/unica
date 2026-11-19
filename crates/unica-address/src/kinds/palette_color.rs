//! «ЦветПалитры», `PaletteColor`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ЦветПалитры"],
    metadata: Some(MetadataDeclaration::new(&["PaletteColors", "ЦветаПалитры"])),
    // Цвета палитры: дерево Конфигуратора 8.5.4, между сервисами интеграции
    // и элементами стиля.
    root: Some(RootPlace {
        child_objects: 2400,
        tree: Some(2700),
    }),
    layout: Some(LayoutDeclaration {
        directory: "PaletteColors",
        display_name_ru: "Цвета палитры",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    since: Some([8, 5, 1]),
    ..KindDeclaration::new("PaletteColor")
};
