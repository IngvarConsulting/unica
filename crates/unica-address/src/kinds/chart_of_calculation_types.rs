//! «ПланВидовРасчета», `ChartOfCalculationTypes`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПланВидовРасчета", "ПланВидовРасчёта"],
    metadata: Some(
        MetadataDeclaration::new(&[
            "ChartsOfCalculationTypes",
            "ПланыВидовРасчета",
            "ПланыВидовРасчёта",
        ])
        .manager()
        .object(),
    ),
    root: Some(RootPlace {
        child_objects: 4300,
        tree: Some(4200),
    }),
    layout: Some(LayoutDeclaration {
        directory: "ChartsOfCalculationTypes",
        display_name_ru: "ПВР",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[
        ("ChartOfCalculationTypesObject", "Object"),
        ("ChartOfCalculationTypesRef", "Ref"),
        ("ChartOfCalculationTypesSelection", "Selection"),
        ("ChartOfCalculationTypesList", "List"),
        ("ChartOfCalculationTypesManager", "Manager"),
        ("DisplacingCalculationTypes", "DisplacingCalculationTypes"),
        (
            "DisplacingCalculationTypesRow",
            "DisplacingCalculationTypesRow",
        ),
        ("BaseCalculationTypes", "BaseCalculationTypes"),
        ("BaseCalculationTypesRow", "BaseCalculationTypesRow"),
        ("LeadingCalculationTypes", "LeadingCalculationTypes"),
        ("LeadingCalculationTypesRow", "LeadingCalculationTypesRow"),
    ]),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("ChartOfCalculationTypes")
};
