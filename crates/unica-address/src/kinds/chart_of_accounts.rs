//! «ПланСчетов», `ChartOfAccounts`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПланСчетов"],
    metadata: Some(
        MetadataDeclaration::new(&["ChartsOfAccounts", "ПланыСчетов"])
            .manager()
            .object(),
    ),
    root: Some(RootPlace {
        child_objects: 4100,
        tree: Some(4100),
    }),
    layout: Some(LayoutDeclaration {
        directory: "ChartsOfAccounts",
        display_name_ru: "Планы счетов",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("ChartOfAccountsObject", "Object"),
        ("ChartOfAccountsRef", "Ref"),
        ("ChartOfAccountsSelection", "Selection"),
        ("ChartOfAccountsList", "List"),
        ("ChartOfAccountsManager", "Manager"),
        ("ChartOfAccountsExtDimensionTypes", "ExtDimensionTypes"),
        (
            "ChartOfAccountsExtDimensionTypesRow",
            "ExtDimensionTypesRow",
        ),
    ]),
    child_order: Some(&[
        "Attribute",
        "TabularSection",
        "AccountingFlag",
        "ExtDimensionAccountingFlag",
        "Form",
        "Template",
        "Command",
    ]),
    ..KindDeclaration::new("ChartOfAccounts")
};
