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
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("ScheduledJob")
};
