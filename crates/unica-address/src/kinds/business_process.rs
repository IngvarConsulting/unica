//! «БизнесПроцесс», `BusinessProcess`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["БизнесПроцесс"],
    metadata: Some(MetadataDeclaration::new(&["BusinessProcesses", "БизнесПроцессы"]).manager().object()),
    root: Some(RootPlace { child_objects: 4500, tree: Some(4700) }),
    layout: Some(LayoutDeclaration {
        directory: "BusinessProcesses",
        display_name_ru: "Бизнес-процессы",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("BusinessProcessObject", "Object"),
        ("BusinessProcessRef", "Ref"),
        ("BusinessProcessSelection", "Selection"),
        ("BusinessProcessList", "List"),
        ("BusinessProcessManager", "Manager"),
        ("BusinessProcessRoutePointRef", "RoutePointRef"),
    ]),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("BusinessProcess")
};
