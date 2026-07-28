## Что меняется

<!-- Одним абзацем: какая задача решена и каким способом. Ссылка на issue. -->

## Архитектурный слой

<!--
Назовите ID затронутых записей реестра: INV-* из spec/architecture/invariants.md,
REQ-* из spec/architecture/quality-requirements.md.
Если изменение ничего из этого не затрагивает, напишите "нет".
-->

- Затронутые записи реестра:
- Решение (ADR), если публичный или архитектурный контракт меняется:

Изменение публичной поверхности `unica.*` требует записи решения, записи
реестра и проверки в одном наборе изменений (`INV-MCP-08`). Это проверяет
`scripts/ci/check-architecture-sync.py`.

- [ ] Пройден [чек-лист изменений](../spec/architecture/change-checklist.md)
      в части, относящейся к этому изменению.

## Проверка

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
python3.12 -m unittest discover -s tests/ci
git diff --check
```

- [ ] Новое поведение покрыто тестом, который можно назвать по имени.
- [ ] Документация, описывающая изменённое поведение, обновлена в этом же PR.
