//! «ФункциональнаяОпция», `FunctionalOption`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ФункциональнаяОпция"],
    metadata: Some(MetadataDeclaration::new(&[
        "FunctionalOptions",
        "ФункциональныеОпции",
    ])),
    root: Some(RootPlace {
        child_objects: 2000,
        tree: Some(1200),
    }),
    layout: Some(LayoutDeclaration {
        directory: "FunctionalOptions",
        display_name_ru: "Функциональные опции",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // Пустые обязательные свойства — как у объекта, только что созданного в
    // Конфигураторе 8.3.27; остальные значения выгрузка 8.3.27, 8.5.1 и 8.5.4
    // пишет одинаково. Пока свойство не заполнено, `cf.validate` сообщает
    // о нём, как платформа при загрузке.
    creation: Some(CreationDeclaration::properties(&[
        "<Location/>",
        "<PrivilegedGetMode>true</PrivilegedGetMode>",
        "<Content/>",
    ])),
    ..KindDeclaration::new("FunctionalOption")
};
