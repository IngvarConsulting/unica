---
id: INV.SOURCE.SUBSYSTEM-PICTURE-PRESERVATION
status: active
governs: product
decision: DEC.2026-09-13.SUBSYSTEM-PICTURE-PRESERVATION
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_subsystem_content_preserves_picture_and_preview_plan_without_entity_churn
scope: [source, platform, wire]
---

# Правка состава сохраняет прозрачность картинки без служебных XML-сущностей

Канонические `content.add/remove` сохраняют `Picture.LoadTransparent`:
`true`, `false` и отсутствие значения, со ссылкой на картинку и без неё.
Форматирование не добавляет сущностей возврата каретки. Предпросмотр
сохраняет дерево рабочего пространства; применение публикует тот же
`planHash` и число эффектов. `childSubsystem.add` создаёт descriptor с
пустой картинкой без `LoadTransparent` и регистрирует ребёнка у родителя.
