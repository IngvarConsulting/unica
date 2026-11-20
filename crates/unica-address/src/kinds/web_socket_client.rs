//! «WebSocketКлиент», `WebSocketClient`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["WebSocketКлиент"],
    metadata: Some(MetadataDeclaration::new(&[
        "WebSocketClients",
        "WebSocketКлиенты",
    ])),
    root: Some(RootPlace {
        child_objects: 1650,
        tree: Some(2500),
    }),
    // Выгрузка `all_kinds` 2.20–2.22: `WebSocketClients/<Имя>.xml` без
    // `InternalInfo` и `ChildObjects`, модуль — `<Имя>/Ext/Module.bsl`.
    layout: Some(LayoutDeclaration {
        directory: "WebSocketClients",
        display_name_ru: "WebSocket-клиенты",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    ..KindDeclaration::new("WebSocketClient")
};
