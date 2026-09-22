---
id: INV.APP.LEXICAL-SEARCH-PREFIX
check:
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_is_literal_source_scoped_and_uses_the_upstream_deadline
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_returns_the_first_unranked_provider_traversal_prefix
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_timeout_preserves_already_streamed_hits_as_a_lower_bound
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_exit_one_without_stderr_is_empty
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_non_empty_malformed_output_is_failed
  - crates/unica-coder/src/domain/code_intelligence.rs::empty_section_rejects_an_incomplete_count
gap: https://github.com/IngvarConsulting/unica/issues/960
---

# Ограниченный буквальный поиск не выдаёт первые совпадения за лучшие

Поставщик `git-grep` ищет буквальную строку с учётом регистра. Он сохраняет
первые проверенные совпадения в порядке
получения и ограничивает их числом `limit`. Он не сортирует их по
релевантности и не добавляет ранг или оценку.

Достижение лимита и истечение срока означают неполный результат.
При истечении срока уже найденные совпадения сохраняются; их число —
нижняя граница, а не точный итог всего корпуса. Ноль при остановленном
поиске не означает отсутствия совпадений.

Доказанный пустой результат имеет точный нулевой счёт. Неразбираемый ответ
поставщика не превращается в успешную пустую выдачу.
Проверки относятся к адаптеру поставщика и форме секции, а не ко всему MCP-вызову.

При отмене уже собранные совпадения отбрасываются. Это отличается
от истечения срока; прямой потоковый сценарий отмены ещё требует проверки.
