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
    // Свойства нового объекта — как их пишет Конфигуратор 8.3.27 по «Добавить».
    creation: Some(CreationDeclaration::properties(&[
        "<Predefined>false</Predefined>",
        "<AutoConnect>false</AutoConnect>",
        "<ServerURL/>",
        "<User/>",
        "<Password/>",
        "<Headers xsi:type=\"xr:ValueList\"/>",
        "<UseOSProxy>false</UseOSProxy>",
        "<UseOSAuthentication>false</UseOSAuthentication>",
        "<Timeout>30</Timeout>",
    ])),
    ..KindDeclaration::new("WebSocketClient")
};
