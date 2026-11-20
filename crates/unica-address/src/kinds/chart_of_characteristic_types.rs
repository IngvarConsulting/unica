//! «ПланВидовХарактеристик», `ChartOfCharacteristicTypes`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПланВидовХарактеристик"],
    metadata: Some(
        MetadataDeclaration::new(&["ChartsOfCharacteristicTypes", "ПланыВидовХарактеристик"])
            .manager()
            .object(),
    ),
    root: Some(RootPlace {
        child_objects: 4000,
        tree: Some(4000),
    }),
    layout: Some(LayoutDeclaration {
        directory: "ChartsOfCharacteristicTypes",
        display_name_ru: "ПВХ",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("ChartOfCharacteristicTypesObject", "Object"),
        ("ChartOfCharacteristicTypesRef", "Ref"),
        ("ChartOfCharacteristicTypesSelection", "Selection"),
        ("ChartOfCharacteristicTypesList", "List"),
        ("Characteristic", "Characteristic"),
        ("ChartOfCharacteristicTypesManager", "Manager"),
    ]),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("ChartOfCharacteristicTypes")
};
