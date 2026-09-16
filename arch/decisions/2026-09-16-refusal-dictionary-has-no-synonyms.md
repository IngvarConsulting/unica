---
id: DEC.2026-09-16.REFUSAL-DICTIONARY-HAS-NO-SYNONYMS
status: active
governs: product
realized: crates/unica-coder/src/domain/refusal.rs::the_dictionary_keeps_no_synonym_of_a_code_it_already_carries
supersedes: []
superseded-by: null
establishes: []
changes: [INV.WIRE.V13-REFUSAL-CHANNEL]
design: docs/design/2026-09-04-canonical-surface-distribution-design.md
---

# Один смысл — одно имя: синонимы сняты из закрытого словаря отказов

**Решение.** Закрытый словарь отказов канонической поверхности не держит двух
кодов на один смысл. Три пары сведены, и словарь стал из сорока пяти кодов
сорока двумя:

- `revision_mismatch` снят — гонка ревизии при публикации `apply` отвечает тем
  же `stale_revision`, которым отвечает допуск;
- `provider_deadline` снят — срок чтения у `view`, `find` и `search` есть тот же
  срок операции, `deadline_exceeded`;
- `dependency_unavailable` снят — недоступная зависимость валидатора отвечает
  `provider_unavailable` с уточнением `provider_absent`.

**Почему.** Агент ветвится по коду. Два имени на одно положение дел заставляют
его знать оба и различать там, где различия нет: `provider_deadline` и
`deadline_exceeded` давали один исход «повторить как есть» и отличались только
тем, чей секундомер сработал — это деталь Unica, а не вопрос вызывающего.
`dependency_unavailable` был уже мёртв: живой маршрут `check` с прошлого среза отвечает
`provider_unavailable` с `provider_absent`, и снятый код доживал в карте
ошибок, не доходя до провода. `revision_mismatch` описывал ровно то же, что
`stale_revision`, — метку, которая больше не сойдётся.

**Цена.** Набор на проводе сузился: клиент, разбиравший три снятых имени,
их больше не увидит. Совместимости здесь не обещано — набор закрыт и до rc.1
правится решением, а не расширением. Исходы при сведении не поехали ни в одной
из трёх пар: гонка публикации как была «исправить вызов», так и осталась,
поэтому направление `revision_mismatch → concurrent_change`, звучавшее в
разборе синонимов, не взято: `concurrent_change` означает «повторить как есть», а
повтор с той же меткой не сойдётся никогда.
