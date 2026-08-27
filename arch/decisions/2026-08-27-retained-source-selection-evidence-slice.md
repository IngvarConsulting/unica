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

Evidence охватывает exact bytes `v8project.yaml` и каждого успешно содержательно
классифицируемого `ConfigDumpInfo.xml`, named absence и wrong-kind от ближайшего
существующего retained ancestor, identity всех пройденных каталогов, полный
детерминированный membership использованных контейнеров и marker inputs каждой
строки карты, включая неподдерживаемые и невыбранные строки. Маркер, семантика
которого использует только существование, удерживает retained parent, имя и
`FileIdentity`: replacement или wrong-kind отвергается, но in-place смена его
неиспользуемых bytes не объявляется сменой карты. Оно не сериализуется, не
клонируется, не хранится в durable task и не является actor key или digest.
Recoverable oversized `ConfigDumpInfo.xml` оставляет typed terminal observation:
retained parent, имя, `FileIdentity` и порог, выше которого находится metadata
length. Только такое наблюдение сохраняет `format_probe_error`; прочая ошибка
actor probe закрывает admission. Оба validation pass заново доказывают regular
kind, ту же physical identity и класс `length > maximum`, поэтому in-place
truncate/repair отвергается.

Один проход канонически дедуплицирует повторные наблюдения и отвергает
противоречивые повторы. Его совокупные retained-state пределы: 32 MiB canonical
exact bytes, 65 536 evidence records, 128 retained directory capabilities и
8 MiB route/name bytes. Отдельный pass-global work ledger допускает не более
32 MiB exact metadata length и 16 384 перечисленных членов по всем external
source sets, включая повторы, не XML и wrong-kind. Каждое exact наблюдение
списывает metadata length до content read; повтор сравнивается с canonical bytes
bounded streaming без второго полного буфера. Это actor-authority envelope
поверх parser/output limits в 1 024 source sets и 16 384 format-evidence rows, а
не их умножение. Membership до enumeration учитывает применимые member,
record и route/name capacity; исчерпанная directory capacity отвергает unseen
member до child open, а exact reader проверяет длину следующего chunk до
расширения буфера. Отказ возвращает стабильную provider error.
Первый проход преобразуется в handle-free snapshot до второго; retained evidence
второго держит не более 128 directory handles, а regular-file handle закрывается
сразу после capture. Сравнение проходит по заимствованному каноническому evidence
без клонирования bytes и проверяет deadline/cancellation до и внутри линейной по
records/bytes работы.

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
