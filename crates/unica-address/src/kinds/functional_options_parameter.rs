//! «ПараметрФункциональныхОпций», `FunctionalOptionsParameter`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПараметрФункциональныхОпций", "ПараметрФункциональнойОпции"],
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
    // Пустые обязательные свойства — как у объекта, только что созданного в
    // Конфигураторе 8.3.27; остальные значения выгрузка 8.3.27, 8.5.1 и 8.5.4
    // пишет одинаково. Пока свойство не заполнено, `cf.validate` сообщает
    // о нём, как платформа при загрузке.
    creation: Some(CreationDeclaration::properties(&["<Use/>"])),
    ..KindDeclaration::new("FunctionalOptionsParameter")
};
