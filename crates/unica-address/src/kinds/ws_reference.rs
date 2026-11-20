//! «WSСсылка», `WSReference`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["WSСсылка"],
    metadata: Some(MetadataDeclaration::new(&["WSReferences", "WSСсылки"]).manager()),
    root: Some(RootPlace {
        child_objects: 1600,
        tree: Some(2400),
    }),
    layout: Some(LayoutDeclaration {
        directory: "WSReferences",
        display_name_ru: "WS-ссылки",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    // The platform writes the manager of a WS reference in its own export
    // (`tests/fixtures/formats/platform_xml/<format>/all_kinds/src/
    // WSReferences/Probe.xml`, 2.20 to 2.22) and refuses to load a
    // borrowed WS reference without it.
    generated_types: Some(&[("WSReferenceManager", "Manager")]),
    // Значения, которые выгрузка 8.3.27, 8.5.1 и 8.5.4 одинаково пишет
    // объекту с опущенными в описании свойствами.
    creation: Some(CreationDeclaration {
        properties: &["<LocationURL/>"],
    }),
    ..KindDeclaration::new("WSReference")
};
