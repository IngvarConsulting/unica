//! «НумераторДокументов», `DocumentNumerator`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["НумераторДокументов"],
    metadata: Some(MetadataDeclaration::new(&[
        "DocumentNumerators",
        "НумераторыДокументов",
    ])),
    root: Some(RootPlace {
        child_objects: 3200,
        tree: Some(3400),
    }),
    layout: Some(LayoutDeclaration {
        directory: "DocumentNumerators",
        display_name_ru: "Нумераторы",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // Значения, которые выгрузка 8.3.27, 8.5.1 и 8.5.4 одинаково пишет
    // объекту с опущенными в описании свойствами.
    creation: Some(CreationDeclaration::properties(&[
        "<NumberType>String</NumberType>",
        "<NumberLength>9</NumberLength>",
        "<NumberAllowedLength>Variable</NumberAllowedLength>",
        "<NumberPeriodicity>Nonperiodical</NumberPeriodicity>",
        "<CheckUnique>true</CheckUnique>",
    ])),
    ..KindDeclaration::new("DocumentNumerator")
};
