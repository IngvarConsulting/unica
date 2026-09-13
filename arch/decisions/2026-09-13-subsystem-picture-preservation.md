---
id: DEC.2026-09-13.SUBSYSTEM-PICTURE-PRESERVATION
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_subsystem_content_preserves_picture_and_preview_plan_without_entity_churn
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.SUBSYSTEM-PICTURE-PRESERVATION, INV.SOURCE.SUBSYSTEM-PICTURE-REFUSAL]
---

# Редактирование состава подсистемы сохраняет прозрачность картинки

**Решение.** Общая модель редактирования подсистемы хранит
`Picture.LoadTransparent` как необязательное исходное значение. Парсер и
writer сохраняют `true`, `false` и отсутствие свойства, в том числе без
`xr:Ref`. Новые подсистемы получают пустую картинку без этого свойства.
Форматирующие переводы строк writer создаёт как разделители строк, поэтому
они не превращаются в текстовые XML-сущности `&#13;`.

Канонические `content.add/remove` и `childSubsystem.add` используют ту же
модель, что внутренний обработчик. Создание дочернего descriptor заполняет
новое поле явно. Проверка проходит через публичный цикл `unica.apply`:
предпросмотр не пишет, а применение с его `ifRev` публикует тот же план.

Сохранение свойства не добавляет ключей в `props.set`.
`LoadTransparent` и `Picture.LoadTransparent` остаются недопустимыми;
отказ в обоих режимах сохраняет дерево рабочего пространства, включая
запрос, перед которым стоит создание дочерней подсистемы.

Основание — ошибка E0063 после слияния актуального `upstream/main`:
`subsystem_stub_xml` создавал модель без нового поля. Исправление проверяет
актуальный канонический путь; инструкция скилла следует
`DEC.2026-09-04.SKILLS-CANONICAL-SURFACE`.
