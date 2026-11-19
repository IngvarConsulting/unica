//! «КритерийОтбора», `FilterCriterion`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["КритерийОтбора"],
    metadata: Some(MetadataDeclaration::new(&["FilterCriteria", "КритерииОтбора"]).manager()),
    root: Some(RootPlace {
        child_objects: 900,
        tree: Some(700),
    }),
    layout: Some(LayoutDeclaration {
        directory: "FilterCriteria",
        display_name_ru: "Критерии отбора",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("FilterCriterionManager", "Manager"),
        ("FilterCriterionList", "List"),
    ]),
    ..KindDeclaration::new("FilterCriterion")
};
