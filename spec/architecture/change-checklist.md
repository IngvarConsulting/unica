# Чек-лист архитектурного изменения

Используйте этот чек-лист, когда меняете публичные MCP-инструменты,
маршрутизацию скиллов, адаптеры, поведение кеша или метаданные упаковки.

Каждый пункт назван идентификатором записи реестра — инварианта из
[`invariants.md`](invariants.md) или требования из
[`arc42/10-quality-requirements.md`](arc42/10-quality-requirements.md) — и не
повторяет её формулировку: правило принадлежит реестру, чек-лист лишь
напоминает, что его нужно проверить (`INV-DOC-08`). Если пункт нельзя
выполнить, изменению нужна новая или заменяющая запись решения, а не исключение
в этом файле.

## MCP-поверхность

- [ ] Internal engines are still reachable only through internal adapters
  (`INV-MCP-01`).
- [ ] `.mcp.json` still declares exactly one `mcpServers` entry named `unica`
  (`INV-MCP-02`).
- [ ] `initialize` still returns `serverInfo.name = "unica"` (`INV-MCP-03`).
- [ ] `tools/list` exposes only `unica.<group>.<operation>` names and no removed
  alias (`INV-MCP-04`).
- [ ] Tool names, descriptions, and schemas still come from the data-driven
  descriptors and expose no raw adapter argument (`INV-MCP-05`).
- [ ] Transport code and `rmcp` types stay inside `interfaces/mcp.rs`
  (`INV-MCP-06`).
- [ ] Admission bound, overload error, cancellation, and shutdown grace are
  preserved or re-tested (`INV-MCP-07`).
- [ ] The Rust registry, the parity harness, the routing skill, and the owning
  decision record move in one change set (`INV-MCP-08`).

## Маршрутизация скиллов

- [ ] Updated skills route through MCP `unica` and name the `unica.*` tool they
  call (`INV-SKILL-01`).
- [ ] Updated skills name no internal adapter server or adapter tool identifier
  as a routing target (`INV-SKILL-02`).
- [ ] Updated skills ship and reference no skill-local Python, PowerShell, or
  shell operation file (`INV-SKILL-03`).
- [ ] Mutating guidance keeps the preview path as its default (`INV-SKILL-05`).
- [ ] Every `tools/call` example in a touched skill still executes as an MCP dry
  run (`INV-SKILL-06`).
- [ ] Bundled low-level engines are never named as call targets in
  prompt-visible text (`INV-PRODUCT-03`).

## Кеш и события

- [ ] The mutating operation emits the right `DomainEventKind`, and the reported
  cache impact matches the caches those events invalidate (`INV-CACHE-02`).
- [ ] The dry run reports impact and writes no workspace state, index, or
  service record (`INV-CACHE-04`).
- [ ] The applied operation writes state only after a successful mutation and
  notifies live workspace services with the same events (`INV-CACHE-05`).
- [ ] No cache state is written outside the volatile cache root (`INV-CACHE-03`).
- [ ] Workspace identity and service keys keep a linked worktree isolated
  (`INV-CACHE-06`).

## Адаптеры и границы слоёв

- [ ] New dispatch enters through `UnicaApplication`, not through the transport
  or an adapter (`INV-APP-01`, `INV-APP-02`).
- [ ] Adapters reach the workspace through `ApplicationPorts` and render no MCP
  response (`INV-APP-03`); their failures surface through the shared envelope
  fields `warnings` and `errors` (`REQ-OBS-01`).
- [ ] No production code path spawns a script interpreter as an operation
  backend (`INV-APP-04`).
- [ ] Layer dependency direction and the composition root are unchanged
  (`INV-APP-05`).
- [ ] Application code constructs no `git` child process (`INV-APP-06`).
- [ ] Analyzer and index work that needs warm state goes through the hidden
  workspace service manager, and a cheap read-only tool such as
  `unica.code.grep` still starts no service (`INV-APP-07`).
- [ ] OS-specific code stays behind the platform facades and child processes are
  owned as process trees (`INV-PLATFORM-01`, `INV-PLATFORM-04`).

## Source sets

- [ ] Format detection stays a property of a single source-set (`INV-SOURCE-01`,
  `INV-SOURCE-02`).
- [ ] A native platform XML operation still resolves a `platform_xml` source-set
  before touching files (`INV-SOURCE-04`).
- [ ] Source-root selection remains deterministic and shared with the analyzer,
  the index, and the project tools (`INV-SOURCE-05`).

## Упаковка

- [ ] `third-party/tools.lock.json` still names the bundled public binary
  `unica` and remains the sole version authority (`INV-PKG-04`,
  `INV-PRODUCT-05`).
- [ ] The packaged `third-party/manifest.json` stays generated from the lock and
  records its digest instead of becoming a second version authority
  (`INV-PRODUCT-05`).
- [ ] `cargo run --quiet --bin unica -- --help` still works from a source
  checkout (`INV-PKG-04`).
- [ ] The public marketplace package stays thin: its `.mcp.json` enters through
  the command-scoped Git shell alias that resolves the plugin root for both
  hosts and hands it to `bootstrap/launch.sh`, with no per-target command matrix
  and no full runtime binary (`INV-PKG-02`).
- [ ] The launcher that starts the host-target `unica` binary directly, without
  a bootstrap payload, stays confined to the development-only local debug
  package (`INV-PKG-07`).
- [ ] Both host manifests declare the same version, and no generated binary
  becomes a tracked file (`INV-PKG-05`, `INV-PKG-01`).
- [ ] Manifest and catalog keys stay inside the oldest supported client floor
  (`INV-PKG-06`).
- [ ] The runtime cache resolution order is unchanged (`INV-CACHE-07`).
- [ ] Attribution stays complete and reachable from both READMEs
  (`INV-PKG-08`).
- [ ] Contracts verified on the source checkout are verified again on the
  generated package, including a clean-cache install on each host
  (`INV-PRODUCT-04`, `INV-PRODUCT-01`); the procedure is in
  [`../acceptance/unica-mcp-validation.md`](../acceptance/unica-mcp-validation.md).

## Фикстуры

- [ ] Retained donor reference models stay test-only fixtures, and their byte
  policy follows
  [`../acceptance/unica-mcp-validation.md`](../acceptance/unica-mcp-validation.md)
  (`INV-SKILL-04`).

## Синхронизация документации

Изменение публичной поверхности — инструментов `unica.*` и их контрактов,
идентичности MCP-сервера, маршрутизации скиллов, контракта упаковки и релиза или
границ слоёв — обязано принести с собой документацию в том же изменении.

- [ ] The owning decision record under [`../decisions/`](../decisions/README.md)
  is added, updated, or superseded (`INV-MCP-08`).
- [ ] The affected entry in [`invariants.md`](invariants.md) is updated together
  with the check it names (`INV-DOC-03`).
- [ ] The indexes stay synchronized: [`../decisions/README.md`](../decisions/README.md)
  for records, [`arc42/architecture.md`](arc42/architecture.md) for chapters, and
  [`../README.md`](../README.md) for the catalogue (`INV-DOC-04`).
- [ ] Documents reference the new rule by ID instead of restating it
  (`INV-DOC-08`), normative sentences are written in English (`INV-DOC-07`), and
  every added relative link resolves from its own document (`INV-DOC-06`).

`tests/ci/test_architecture_registry.py` не читает этот чек-лист. Он удерживает
часть названных здесь правил на самих реестрах и индексах: индекс решений
перечисляет ровно записи на диске, а индекс arc42 — все главы (`INV-DOC-04`);
каждая неручная проверка записи называет существующий путь (`INV-DOC-03`);
нормативные поля записей написаны по-английски (`INV-DOC-07`); относительные
ссылки активных документов, включая этот файл, разрешаются (`INV-DOC-06`).
Остальные пункты раздела проверяет ревью.

## Проверка

Локально достаточно тех же команд, которые прогоняют задания `verify-source` и
`test-rust-primary` из `.github/workflows/unica-plugin-release.yml` на
Python 3.12:

```sh
python -m pip install -r tests/ci/requirements.txt
python -m unittest discover -s tests/ci --durations 20
python -m unittest discover -s tests/dev --durations 20
python -m py_compile scripts/ci/*.py tests/ci/*.py
python -m py_compile scripts/dev/*.py tests/dev/*.py
python scripts/ci/check-version-contract.py
python scripts/ci/check-architecture-sync.py --base "$(git merge-base HEAD origin/main)"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
git diff --check
```

Первая строка — предусловие задания: без установленных зависимостей часть тестов
`tests/ci` падает на отсутствующем модуле, а не на разбираемом изменении.
`check-architecture-sync.py` CI выполняет только для pull request и от merge-base
с базовой веткой; локально базу задаёт та же `git merge-base`.

`git diff --check` — локальная гигиена пробелов, а не шаг конвейера; исключение
для parity-фикстур описано в
[`../acceptance/unica-mcp-validation.md`](../acceptance/unica-mcp-validation.md).
Остальные шаги — сборка инструментов, контракты бинарников, упаковка плагина и
runtime, смоук bootstrap на каждом хосте и агрегирующий гейт — запускает сам
workflow.
