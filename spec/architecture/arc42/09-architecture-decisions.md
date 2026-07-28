# 9. Архитектурные решения

Принятые архитектурные решения живут в [`spec/decisions/`](../../decisions/README.md).
Каждая запись отвечает на один вопрос: какую альтернативу выбрали, из-за каких
ограничений и что этот выбор запрещает делать дальше. Решение — исторический
документ: оно фиксирует момент выбора и не переписывается под изменившийся код.

Эта глава намеренно не содержит перечня решений. Ручной список здесь дважды
расходился с каталогом, потому что у одного факта оказывалось два владельца.
Единственный индекс решений — [`spec/decisions/README.md`](../../decisions/README.md);
он же перечисляет допустимые статусы.

Читать решения по одному не нужно. Действующее следствие каждого решения
вынесено в реестры: [реестр инвариантов](../invariants.md) и
[реестр требований к качеству](10-quality-requirements.md) ссылаются на решение
по ID в поле `Decision` и называют проверку, которая это следствие удерживает.
Путь «решение → правило → проверка» проходится за два перехода и не требует
этой главы.

## Правило обновления

- A change that adds, removes, or renames a public MCP tool, moves cache or
  workspace-state ownership, exposes an internal engine directly, or alters the
  packaging or host contract requires a new or superseding decision record
  before it is merged.
- The decision record, the registry entry that derives from it, and the check
  named by that entry change in one change set.
- A decision is never edited to match code that already diverged. It is
  superseded by a new record; the superseded record keeps its original text and
  only its status changes.
- A decision number is never reassigned. A decision that stops applying is
  superseded, not deleted: its file stays, so the index keeps listing it and no
  link goes dangling. The index lists exactly the records that exist on disk.

Формат записей решений, требование ссылаться по ID вместо копирования текста и
синхронность индекса с файлами нормированы в реестре инвариантов:
INV-DOC-02, INV-DOC-04, INV-DOC-05, INV-DOC-08.
