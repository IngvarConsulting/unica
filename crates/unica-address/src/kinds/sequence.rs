//! «Последовательность», `Sequence`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Последовательность"],
    metadata: Some(MetadataDeclaration::new(&["Sequences", "Последовательности"]).manager()),
    root: Some(RootPlace {
        child_objects: 3300,
        tree: Some(3500),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Sequences",
        display_name_ru: "Последовательности",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    // The platform writes these three for a sequence in its own export
    // (`tests/fixtures/formats/platform_xml/<format>/all_kinds/src/
    // Sequences/Probe.xml`, 2.20 to 2.22) and refuses to load a borrowed
    // sequence without its record and manager.
    generated_types: Some(&[
        ("SequenceRecord", "Record"),
        ("SequenceManager", "Manager"),
        ("SequenceRecordSet", "RecordSet"),
    ]),
    // Пустые обязательные свойства — как у объекта, только что созданного в
    // Конфигураторе 8.3.27; остальные значения выгрузка 8.3.27, 8.5.1 и 8.5.4
    // пишет одинаково. Пока свойство не заполнено, `cf.validate` сообщает
    // о нём, как платформа при загрузке.
    creation: Some(CreationDeclaration::properties(&[
        "<MoveBoundaryOnPosting>Move</MoveBoundaryOnPosting>",
        "<Documents/>",
        "<RegisterRecords/>",
        "<DataLockControlMode>Managed</DataLockControlMode>",
    ])),
    ..KindDeclaration::new("Sequence")
};
