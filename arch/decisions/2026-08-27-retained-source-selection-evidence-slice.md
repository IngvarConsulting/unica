---
id: DEC.2026-08-27.RETAINED-SOURCE-SELECTION-EVIDENCE-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_source_selection_finality_contract_is_complete
supersedes: []
superseded-by: null
establishes: [INV.APP.RETAINED-SOURCE-SELECTION-FINALITY]
---

# Apply удерживает полное evidence выбора источников

**Решение.** Actor admission строит карту источников только через retained
workspace capability и два полных descriptor-relative no-follow прохода.
Admission принимается, когда полная каноническая семантика карты и физическое
evidence обоих проходов совпали; сохраняется evidence второго прохода.

Evidence охватывает exact bytes конфигурации, named absence и wrong-kind от
ближайшего существующего retained ancestor, identity всех пройденных каталогов,
полный детерминированный membership использованных контейнеров и marker inputs
каждой строки карты, включая неподдерживаемые и невыбранные строки. Оно не
сериализуется, не клонируется, не хранится в durable task и не является actor
key или digest.

Daemon создаёт actor из полного поддерживаемого Platform XML projection этого
admission. Каждый apply admission заново получает resolved admission, сверяет
projection и выбранный actor binding и перемещает sealed evidence через
prepared batch. Два полных validation pass выполняются до публикации, после
dry-run revision confirmation и в retained final gate после source/cache
postimages. Поздний отказ использует существующий reverse rollback; writers и
transaction participants остаются ровно `Source + WorkspaceCache`.

Уже admitted logical read завершает retained snapshot независимо от поздней
смены карты; последующий invocation получает новую семантическую карту. Решение
не заменяет V12 `ProjectSourceMapProvenance`, не меняет публичный wire-контракт и
не обещает historical/ABA oracle между отдельными checkpoints. Focused actor
discovery tests дополнительно подтверждают обычные external processor/report
строки карты.
