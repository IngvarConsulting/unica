//! «РегламентноеЗадание», `ScheduledJob`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["РегламентноеЗадание"],
    metadata: Some(MetadataDeclaration::new(&[
        "ScheduledJobs",
        "РегламентныеЗадания",
    ])),
    root: Some(RootPlace {
        child_objects: 1800,
        tree: Some(900),
    }),
    layout: Some(LayoutDeclaration {
        directory: "ScheduledJobs",
        display_name_ru: "Регламентные задания",
        child_objects_8_3_27: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types_8_3_27: Some(&[]),
    ..KindDeclaration::new("ScheduledJob")
};
