---
id: DEC.2026-08-28.V0-13-MIGRATION-EVIDENCE-GATE
status: planned
governs: process
realized: null
supersedes: []
superseded-by: null
establishes: []
design: docs/design/2026-08-28-v0-13-migration-evidence-gate-design.md
---

# v0.13 требует воспроизводимого semantic и provenance evidence

**Решение.** S2 достигается только когда independently captured published
v0.12.3 `tools/list` порождает полный variant catalog, catalog точно совпадает
с reviewed capability oracle, а executable V13 runner закрывает каждый legacy
variant и каждую отдельно объявленную новую capability. Provenance cutover
дополнительно связывается tracked content-addressed manifest с immutable
upstream review, per-entry решениями, patch/index artifacts и ожидаемыми
post-application blobs, проверяемыми в чистом временном дереве.

S2 является отзывным: drift либо новый bundled-tool/product gap возвращает
работу в соответствующую implementation wave и требует повторных W4/W5 gates
до G6. Пока validators и derived rules не реализованы и не получили active
successor, это решение не является действующим release gate.

**Почему.** Name-level inventory и проверка текущего index допускают два
ложно-зелёных результата: пропущенный variant старого семейного инструмента и
неприменимый либо подменённый skill patch. Оба дефекта видит пользователь уже
после cutover, поэтому ручной execution note не является достаточным
доказательством.

**Цена.** До W1 добавляется сбор baseline evidence и последовательная
integrator consolidation; до G6 хранятся проверяемые patch artifacts, а любой
upstream drift отзывает ранее полученный S2.
