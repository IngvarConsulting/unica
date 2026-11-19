//! «ЖурналДокументов», `DocumentJournal`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ЖурналДокументов"],
    metadata: Some(MetadataDeclaration::new(&["DocumentJournals", "ЖурналыДокументов"]).manager()),
    root: Some(RootPlace {
        child_objects: 3400,
        tree: Some(3600),
    }),
    layout: Some(LayoutDeclaration {
        directory: "DocumentJournals",
        display_name_ru: "Журналы документов",
        child_objects_8_3_27: true,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[
        ("DocumentJournalSelection", "Selection"),
        ("DocumentJournalList", "List"),
        ("DocumentJournalManager", "Manager"),
    ]),
    child_order: Some(&["Column", "Form", "Template", "Command"]),
    ..KindDeclaration::new("DocumentJournal")
};
