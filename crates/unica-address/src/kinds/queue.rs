//! «Очередь», `Queue`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Очередь"],
    metadata: Some(MetadataDeclaration::new(&["Queues", "Очереди"])),
    // Очереди: дерево Конфигуратора 8.5.4, между регламентными
    // заданиями и ботами.
    root: Some(RootPlace {
        child_objects: 2500,
        tree: Some(1000),
    }),
    layout: Some(LayoutDeclaration {
        directory: "Queues",
        display_name_ru: "Очереди",
        requires_child_objects: true,
        ..LayoutDeclaration::EMPTY
    }),
    // The queue of 8.5.4: its message is a record, its manager a manager.
    // The platform refuses to import a queue descriptor without them.
    generated_types: Some(&[("QueueMessage", "Record"), ("QueueManager", "Manager")]),
    since: Some([8, 5, 4]),
    // Очередь пишет измерения перед реквизитами: выгрузка 8.5.4
    // собирает их в этом порядке даже из источника с обратным.
    child_order: Some(&["Dimension", "Attribute"]),
    // Значения, которые 8.5.4 даёт очереди с опущенными свойствами:
    // транзакционная, порции по 1000, обработка автоматическая.
    creation: Some(CreationDeclaration {
        properties: &[
            "<Transactioned>true</Transactioned>",
            "<ProcessingPortionSize>1000</ProcessingPortionSize>",
            "<AutoProcessing>true</AutoProcessing>",
        ],
    }),
    ..KindDeclaration::new("Queue")
};
