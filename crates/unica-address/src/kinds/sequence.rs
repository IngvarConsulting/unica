//! «Последовательность», `Sequence`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Последовательность"],
    metadata: Some(MetadataDeclaration::new(&["Sequences", "Последовательности"]).manager()),
    root: Some(RootPlace { child_objects: 3300, tree: Some(3500) }),
    layout: Some(LayoutDeclaration {
        directory: "Sequences",
        display_name_ru: "Последовательности",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    // The platform writes these three for a sequence in its own export
    // (`tests/fixtures/formats/platform_xml/<format>/all_kinds/src/
    // Sequences/Probe.xml`, 2.20 to 2.22) and refuses to load a borrowed
    // sequence without its record and manager.
    generated_types_8_3_27: Some(&[
        ("SequenceRecord", "Record"),
        ("SequenceManager", "Manager"),
        ("SequenceRecordSet", "RecordSet"),
    ]),
    ..KindDeclaration::new("Sequence")
};
