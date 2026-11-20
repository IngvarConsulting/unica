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
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // Значения, которые выгрузка 8.3.27, 8.5.1 и 8.5.4 одинаково пишет
    // объекту с опущенными в описании свойствами.
    creation: Some(CreationDeclaration {
        properties: &[
            "<Type>",
            "\t<v8:Type>xs:string</v8:Type>",
            "\t<v8:StringQualifiers>",
            "\t\t<v8:Length>10</v8:Length>",
            "\t\t<v8:AllowedLength>Variable</v8:AllowedLength>",
            "\t</v8:StringQualifiers>",
            "</Type>",
        ],
    }),
    ..KindDeclaration::new("SessionParameter")
};
