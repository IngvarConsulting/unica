//! «ПланОбмена», `ExchangePlan`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПланОбмена"],
    metadata: Some(MetadataDeclaration::new(&["ExchangePlans", "ПланыОбмена"]).manager().object()),
    root: Some(RootPlace { child_objects: 1200, tree: Some(600) }),
    layout: Some(LayoutDeclaration {
        directory: "ExchangePlans",
        display_name_ru: "Планы обмена",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("ExchangePlanObject", "Object"),
        ("ExchangePlanRef", "Ref"),
        ("ExchangePlanSelection", "Selection"),
        ("ExchangePlanList", "List"),
        ("ExchangePlanManager", "Manager"),
    ]),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("ExchangePlan")
};
