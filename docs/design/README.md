# Design documents

Здесь лежат датированные проектные документы: как мы пришли к выбору, какие
варианты рассматривали и почему отвергли. Их пишет скилл `brainstorming`
(superpowers) на шаге «Write design doc»; каталог задан проекту в
[`AGENTS.md`](../../AGENTS.md) и переопределяет путь плагина по умолчанию.

**archived planning material, not a source of truth.** Документ отражает
состояние на свою дату и после реализации не обновляется. Он не нормирует
поведение системы — ни сейчас, ни в момент написания. Нормативный слой целиком
живёт в [`spec/`](../../spec/README.md): решения в `spec/decisions/`, правила в
реестрах `spec/architecture/`.

## Обязательная шапка

Каждый документ открывается тремя полями:

```markdown
- Date: `YYYY-MM-DD`
- Status: `draft` | `approved` | `superseded`
- Decision: `ADR-NNNN` | `none — no architectural contract changed`
```

Поле `Decision` — главное. Оно отвечает на вопрос, который иначе никто не
задаёт: породил ли этот дизайн архитектурное решение. `none` — это утверждение,
которое ревью может отклонить, а не значение по умолчанию. Перечень того, что
считается архитектурным контрактом, и порядок повышения дизайна до ADR заданы в
[`AGENTS.md`](../../AGENTS.md).

Формат проверяет `tests/ci/test_design_documents.py`.

## Документы, на которые опирается CI

Эти файлы удалять и переименовывать нельзя без правки тестов:

- `2026-07-23-platform-8-3-27-format-2-20-design.md` — закреплён как контракт
  профиля формата в `tests/ci/test_format_profile_contract.py` и
  `tests/dev/test_verify_8_3_27_platform.py`.
- `2026-07-24-updatable-donor-parity-relations-design.md` — назван как
  evidence в `tests/fixtures/unica_mcp_script_parity/donor-relations.json` и в
  записях `plugins/unica/provenance/reviews/`.
