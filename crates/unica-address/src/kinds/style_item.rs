//! «ЭлементСтиля», `StyleItem`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ЭлементСтиля"],
    metadata: Some(MetadataDeclaration::new(&["StyleItems", "ЭлементыСтиля"])),
    root: Some(RootPlace {
        child_objects: 300,
        tree: Some(2800),
    }),
    layout: Some(LayoutDeclaration {
        directory: "StyleItems",
        display_name_ru: "Элементы стиля",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // Значения, которые выгрузка 8.3.27, 8.5.1 и 8.5.4 одинаково пишет
    // объекту с опущенными в описании свойствами.
    creation: Some(CreationDeclaration::properties(&[
        "<Type>Color</Type>",
        "<Value xsi:type=\"v8ui:Color\">#000000</Value>",
    ])),
    ..KindDeclaration::new("StyleItem")
};
