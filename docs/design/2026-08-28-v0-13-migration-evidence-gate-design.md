- Date: `2026-08-28`
- Status: `approved`
- Decision: `DEC.2026-08-28.V0-13-MIGRATION-EVIDENCE-GATE`

# Воспроизводимый migration evidence gate для v0.13

## Контекст

Семь domain-parity shards перечисляют 74 имени v0.12.3, но одно имя семейного
инструмента может содержать несколько разных capability variants. Сам shard
не может служить независимым источником собственной полноты. Аналогично,
успешный `check-skill-upstreams.py --validate-only` проверяет текущий index, но
не доказывает, что reviewed upstream diff, 41 решение и ещё не применённые
skill/index patches образуют тот же cutover input.

## Выбор

Semantic цепочка начинается с raw `tools/list`, снятого из опубликованного
пакета v0.12.3; отдельный provenance sidecar адресует payload SHA-256 и package
asset, не пытаясь включить собственный digest. Детерминированный extractor
строит полный variant catalog по behavior discriminators: schema pointer,
immutable handler branch и executable probe закрывают каждый selector и только
reachable combinations, а обычные value enums не образуют слепой cross-product.
Неполная schema branch блокирует gate до independently grounded branch rule.
Reviewed oracle обязан совпадать с catalog по множеству
`(legacyTool, legacyVariant)`, а новые V13 capabilities учитываются отдельно.
Fixture-driven runner затем исполняет обе стороны каждого legacy mapping и
каждую новую capability.

Provenance цепочка заканчивается tracked manifest. Он адресует immutable
review, upstream target/diff, per-entry решения, точно равные reviewed
`affectedEntries`, три skill patch artifacts,
один index patch, base blobs и expected result blobs. Validator собирает
чистое временное дерево, проверяет digests, применяет patches и запускает
routing/provenance checks. В G6 applied-tree mode дополнительно строит свежий
live report и требует `upstreamDrift=false` и `affectedEntries=[]`; зелёный exit
текущего checker без этих полей недостаточен. В G6 применяются только эти же
bytes.

S2 остаётся внутренним отзывным состоянием. Upstream drift с prose-only
изменением требует нового review, manifest и W4/W5; bundled-tool либо product
gap дополнительно возвращает работу в toolchain/W1/W3. Cutover на старом S2
запрещён.

## Реализация и доказательство

W0.5 создаёт snapshot, catalog, oracle и два независимых validators. W4
создаёт tracked patch artifacts, scope-lock manifest и validator временного
дерева. После реализации обеих частей active successor устанавливает
`INV.CI.V0-13-MIGRATION-EVIDENCE-GATE` и
`INV.CI.V0-13-PROVENANCE-SCOPE-LOCK` с точными именами тестов. Planned запись
до этого не выдаётся за действующее правило.

## Отвергнутые варианты

- Один disposition на имя: теряет разные операции семейного инструмента.
- Oracle без исходного schema snapshot/catalog: может молча пропустить variant.
- Локальные patch commit SHA в execution note: не являются переносимым
  содержимым и не доказывают применимость.
- Проверка только live index: не связывает review и будущий cutover tree.
