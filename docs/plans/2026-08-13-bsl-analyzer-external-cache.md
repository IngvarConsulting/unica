# BSL Analyzer External Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Обновить поставляемый `bsl-analyzer` до 0.2.67 и всегда направлять его workspace-кеш в стабильный каталог Unica вне корней исходников.

**Architecture:** Unica вычисляет полный SHA-256 от домен-разделённой пары нормализованных `workspaceRoot + sourceRoot` и обычно передаёт `<cacheRoot>/providers/bsl-analyzer/source-<digest>` через `mcp serve --cache-dir`. Если этот путь лежал бы внутри выбранного source root, тот же ключ переносится под частный runtime-корень вне дерева. Путь не зависит от `services/<service-id>`, но сохраняет изоляцию наборов исходников, проектов и git worktree по ADR-0018.

**Tech Stack:** Rust, `sha2`, std `Command`, Python `unittest`, JSON tool lock, GitHub CLI/API.

## Global Constraints

- Кеш обычно располагается как `<cacheRoot>/providers/bsl-analyzer/source-<full-sha256>`; вложенный в source root `cacheRoot` переключается на частный runtime-корень с тем же ключом.
- Digest вычисляется из `b"unica-provider-state-v1\0"`, байтов файловой идентичности нормализованного `workspaceRoot`, байта `NUL` и байтов файловой идентичности нормализованного `sourceRoot`; lossy-преобразование путей в UTF-8 не допускается.
- Корни нормализуются существующей `normalize_path_identity`; отдельная пользовательская настройка не добавляется.
- Путь должен быть одинаковым после пересоздания workspace-service и различным для другого source root, workspace и git worktree.
- Unica назначает каталог, но не читает, не мигрирует и не удаляет частные базы `bsl-analyzer`.
- Старые `<sourceRoot>/.build` автоматически не удаляются.
- Публичная MCP-поверхность `unica.*` не меняется; изменение реализует ADR-0018 без нового ADR.
- Bundled tool pin: `bsl-analyzer` `0.2.67`, source tag `v0.2.67`, source commit `9a92766691bbd0191a5ff02c34fa9058e4570b85`, asset tag `bsl-analyzer-v0.2.67-build.1`.
- SHA-256 артефактов: darwin-arm64 `d18c3b79d017d60f229faf4e427bcefc0a9da59a93b57acbb867b064c52926bd`; linux-x64 `c476c10fcdfa6eb7d310e83d0e69b02a27f9afeec0d394681feadb889de97301`; win-x64 `a54d883bcb7ed0e0039953fb4d5cd7c2efbf30155de9951952f1a4060776eb3e`.

---

### Task 1: Стабильный provider cache и аргументы запуска

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:1-31`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:2473-2502`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs:4068-4073`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs:4075-4303`
- Test: `crates/unica-coder/src/infrastructure/workspace_services.rs:7499-7535`

**Interfaces:**
- Consumes: `WorkspaceContext.cache_root`, `WorkspaceContext.workspace_root`, `normalize_path_identity(&Path) -> Result<PathBuf, String>`, платформенные фасады `stable_path_identity_bytes(&Path) -> Result<Vec<u8>, String>` и `path_starts_with_host_root(&Path, &Path) -> bool`, `short_private_runtime_dir()`, `configure_bsl_analyzer_runtime_dir(&mut Command) -> Result<(), String>`.
- Produces: `provider_state_key(workspace_root: &Path, source_root: &Path) -> Result<String, String>`, `bsl_analyzer_cache_dir(context: &WorkspaceContext, source_root: &Path) -> Result<PathBuf, String>`, `bsl_analyzer_command(program: &Path, context: &WorkspaceContext, source_root: &Path) -> Result<Command, String>`.

- [ ] **Step 1: Написать падающие тесты стабильной идентичности**

Добавить рядом с тестами `service_identity_*` проверки:

```rust
#[test]
fn bsl_analyzer_cache_is_stable_and_separated_by_workspace_and_source() {
    let context = test_context("bsl-provider-cache");
    let source_root = context.workspace_root.join("src");
    let repeated = bsl_analyzer_cache_dir(&context, &source_root).unwrap();
    let dotted = bsl_analyzer_cache_dir(&context, &source_root.join(".")).unwrap();
    let other_source = bsl_analyzer_cache_dir(&context, &context.workspace_root.join("extension"))
        .unwrap();
    let mut other_workspace = test_context("bsl-provider-cache-other");
    other_workspace.cache_root = context.cache_root.clone();
    let other_workspace_cache =
        bsl_analyzer_cache_dir(&other_workspace, &other_workspace.workspace_root.join("src"))
            .unwrap();

    assert_eq!(repeated, dotted);
    assert_ne!(repeated, other_source);
    assert_ne!(repeated, other_workspace_cache);
    assert!(repeated.starts_with(context.cache_root.join("providers/bsl-analyzer")));
    assert!(!repeated.starts_with(&source_root));
    assert!(!repeated.starts_with(context.cache_root.join("services")));
    assert_eq!(
        repeated.file_name().unwrap().to_string_lossy().len(),
        "source-".len() + 64
    );

    cleanup(&context);
    cleanup(&other_workspace);
}
```

- [ ] **Step 2: Запустить тест идентичности и подтвердить ожидаемое падение**

Run:

```powershell
cargo test -p unica-coder --lib bsl_analyzer_cache_is_stable_and_separated_by_workspace_and_source
```

Expected: компиляция завершается ошибкой `cannot find function bsl_analyzer_cache_dir`.

- [ ] **Step 3: Реализовать стабильный ключ и путь**

Импортировать `sha2` и добавить функции рядом с `service_key`:

```rust
use sha2::{Digest, Sha256};

const PROVIDER_STATE_KEY_DOMAIN: &[u8] = b"unica-provider-state-v1\0";

fn provider_state_key(workspace_root: &Path, source_root: &Path) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(PROVIDER_STATE_KEY_DOMAIN);
    digest.update(stable_path_identity_bytes(workspace_root)?);
    digest.update([0]);
    digest.update(stable_path_identity_bytes(source_root)?);
    Ok(format!("source-{:x}", digest.finalize()))
}

fn bsl_analyzer_cache_dir(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    let workspace_root = normalize_path_identity(&context.workspace_root)?;
    let source_root = normalize_path_identity(source_root)?;
    let cache_root = normalize_path_identity(&context.cache_root)?;
    let key = provider_state_key(&workspace_root, &source_root)?;
    let preferred = cache_root.join("providers").join("bsl-analyzer").join(&key);
    let preferred = normalize_path_identity(&preferred)?;
    if !crate::infrastructure::platform::filesystem::path_starts_with_host_root(
        &preferred,
        &source_root,
    ) {
        return Ok(preferred);
    }

    let external_root = short_private_runtime_dir()
        .map_err(|error| {
            format!("failed to prepare external bsl-analyzer cache directory: {error}")
        })?
        .unwrap_or_else(|| std::env::temp_dir().join("unica-bsl"));
    let external = external_root
        .join("cache")
        .join("providers")
        .join("bsl-analyzer")
        .join(key);
    let external = normalize_path_identity(&external)?;
    if crate::infrastructure::platform::filesystem::path_starts_with_host_root(
        &external,
        &source_root,
    ) {
        return Err(
            "failed to place bsl-analyzer cache outside the indexed source tree".to_string(),
        );
    }
    Ok(external)
}
```

- [ ] **Step 4: Запустить тест идентичности и подтвердить прохождение**

Run:

```powershell
cargo test -p unica-coder --lib bsl_analyzer_cache_is_stable_and_separated_by_workspace_and_source
```

Expected: PASS.

- [ ] **Step 5: Написать падающий тест команды анализатора**

Добавить рядом с существующими тестами runtime directory:

```rust
#[test]
fn bsl_analyzer_command_passes_external_provider_cache() {
    use std::ffi::OsString;

    let context = test_context("bsl-provider-command");
    let source_root = context.workspace_root.join("src");
    let expected_cache = bsl_analyzer_cache_dir(&context, &source_root).unwrap();
    let command = bsl_analyzer_command(Path::new("bsl-analyzer"), &context, &source_root).unwrap();
    let args = command.get_args().map(OsString::from).collect::<Vec<_>>();

    assert_eq!(
        args,
        vec![
            "mcp".into(),
            "serve".into(),
            "--profile".into(),
            "workspace".into(),
            "--source-dir".into(),
            normalize_path_identity(&source_root).unwrap().into_os_string(),
            "--cache-dir".into(),
            expected_cache.into_os_string(),
            "--mode".into(),
            "stdio".into(),
        ]
    );

    cleanup(&context);
}
```

- [ ] **Step 6: Запустить тест команды и подтвердить ожидаемое падение**

Run:

```powershell
cargo test -p unica-coder --lib bsl_analyzer_command_passes_external_provider_cache
```

Expected: компиляция завершается ошибкой `cannot find function bsl_analyzer_command`.

- [ ] **Step 7: Выделить построение команды и подключить его к persistent session**

Добавить функцию:

```rust
fn bsl_analyzer_command(
    program: &Path,
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<Command, String> {
    let source_root = normalize_path_identity(source_root)?;
    let cache_dir = bsl_analyzer_cache_dir(context, &source_root)?;
    let mut command = Command::new(program);
    command
        .arg("mcp")
        .arg("serve")
        .arg("--profile")
        .arg("workspace")
        .arg("--source-dir")
        .arg(&source_root)
        .arg("--cache-dir")
        .arg(cache_dir)
        .arg("--mode")
        .arg("stdio")
        .current_dir(&context.cwd);
    configure_bsl_analyzer_runtime_dir(&mut command)?;
    Ok(command)
}
```

Заменить ручное построение команды в `PersistentMcpSession::start`:

```rust
let program = resolve_bundled_tool(&plugin_root, "bsl-analyzer", true)?.program;
let command = bsl_analyzer_command(&program, context, source_root)?;
Self::start_with_command(command, cancellation)
```

- [ ] **Step 8: Запустить оба теста и существующие тесты identity/runtime directory**

Run:

```powershell
cargo test -p unica-coder --lib bsl_analyzer_cache_is_stable_and_separated_by_workspace_and_source
cargo test -p unica-coder --lib bsl_analyzer_command_passes_external_provider_cache
cargo test -p unica-coder --lib service_identity_reuses_normalized_paths
cargo test -p unica-coder --lib bsl_analyzer_runtime_directory_is_passed_to_the_child_and_fits_socket_budget
```

Expected: все команды завершаются PASS.

- [ ] **Step 9: Отформатировать и закоммитить изменение runtime**

```powershell
cargo fmt --all
cargo fmt --all -- --check
git add -- crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "fix(workspace): route analyzer cache outside source roots"
```

### Task 2: Обновление bundled bsl-analyzer до 0.2.67

**Files:**
- Modify: `Cargo.toml:14-15`
- Modify: `Cargo.lock`
- Modify: `tests/ci/test_skill_provenance.py:539-562`
- Modify: `plugins/unica/third-party/tools.lock.json:34-58`

**Interfaces:**
- Consumes: release `itrous/bsl-analyzer@v0.2.67` and toolchain release `IngvarConsulting/unica-toolchain@bsl-analyzer-v0.2.67-build.1`.
- Produces: package contract pin for version, source commit, release assets and SHA-256 values listed in Global Constraints.
- Produces: `parser` and `syntax` workspace dependencies from the same source commit as the bundled analyzer.

- [ ] **Step 1: Обновить контрактный тест на 0.2.67**

Переименовать тест в `test_bsl_analyzer_contract_is_v0_2_67` и заменить ожидаемые значения:

```python
self.assertEqual(analyzer["version"], "0.2.67")
self.assertEqual(analyzer["sourceTag"], "v0.2.67")
self.assertEqual(
    analyzer["sourceCommit"],
    "9a92766691bbd0191a5ff02c34fa9058e4570b85",
)
self.assertEqual(analyzer["assetTag"], "bsl-analyzer-v0.2.67-build.1")
self.assertEqual(
    {target: asset["sha256"] for target, asset in analyzer["assets"].items()},
    {
        "darwin-arm64": "d18c3b79d017d60f229faf4e427bcefc0a9da59a93b57acbb867b064c52926bd",
        "linux-x64": "c476c10fcdfa6eb7d310e83d0e69b02a27f9afeec0d394681feadb889de97301",
        "win-x64": "a54d883bcb7ed0e0039953fb4d5cd7c2efbf30155de9951952f1a4060776eb3e",
    },
)
```

- [ ] **Step 2: Запустить контрактный тест и подтвердить ожидаемое падение**

Run:

```powershell
python -m unittest discover -s tests/ci -p test_skill_provenance.py
```

Expected: `test_bsl_analyzer_contract_is_v0_2_67` падает, потому что lock всё ещё содержит 0.2.62.

- [ ] **Step 3: Обновить единственный package-contract lock**

В записи `bsl-analyzer` установить:

```json
{
  "version": "0.2.67",
  "sourceTag": "v0.2.67",
  "sourceCommit": "9a92766691bbd0191a5ff02c34fa9058e4570b85",
  "assetTag": "bsl-analyzer-v0.2.67-build.1",
  "assets": {
    "darwin-arm64": {
      "assetName": "bsl-analyzer-darwin-arm64",
      "sha256": "d18c3b79d017d60f229faf4e427bcefc0a9da59a93b57acbb867b064c52926bd"
    },
    "linux-x64": {
      "assetName": "bsl-analyzer-linux-x64",
      "sha256": "c476c10fcdfa6eb7d310e83d0e69b02a27f9afeec0d394681feadb889de97301"
    },
    "win-x64": {
      "assetName": "bsl-analyzer-win-x64.exe",
      "sha256": "a54d883bcb7ed0e0039953fb4d5cd7c2efbf30155de9951952f1a4060776eb3e"
    }
  }
}
```

Остальные поля записи и порядок инструментов не менять.

- [ ] **Step 4: Запустить проверки package contract**

Run:

```powershell
python -m unittest discover -s tests/ci -p test_skill_provenance.py
python -m unittest discover -s tests/ci -p test_build_unica_tools.py
python -m unittest discover -s tests/ci -p test_attributions.py
```

Expected: все тесты PASS.

- [ ] **Step 5: Обновить parser-library до source commit 0.2.67**

Обновить `rev` зависимостей `bsl-parser` и `bsl-syntax` в корневом
`Cargo.toml` до `9a92766691bbd0191a5ff02c34fa9058e4570b85`, затем механически обновить
lock-файл:

```powershell
cargo update -p parser
```

Проверить существующий контракт синхронизации parser-library:

```powershell
cargo test -p unica-coder --lib parser_library_commit_matches_the_bundled_analyzer_contract
```

Expected: PASS; блоки `parser` и `syntax` в `Cargo.lock` содержат source commit
0.2.67.

- [ ] **Step 6: Проверить JSON и закоммитить обновление**

```powershell
Get-Content -Raw plugins/unica/third-party/tools.lock.json | ConvertFrom-Json | Out-Null
git diff --check
git add -- Cargo.toml Cargo.lock plugins/unica/third-party/tools.lock.json tests/ci/test_skill_provenance.py docs/provenance/reviews/2026-07-29-product-update-backlog.json plugins/unica/ATTRIBUTIONS.md
git commit -m "build: update bsl-analyzer to 0.2.67"
```

### Task 3: Runtime-контракт и полная проверка

**Files:**
- Modify: `spec/architecture/runtime.md:182-184`

**Interfaces:**
- Consumes: ADR-0018, `INV-CACHE-ORCHESTRATOR-OWNED`, фактический путь из Task 1.
- Produces: актуальное описание наблюдаемого размещения provider cache без нового нормативного правила.

- [ ] **Step 1: Уточнить runtime-документ**

Дополнить пункт о постоянных дочерних процессах текстом:

```markdown
Частный кеш `bsl-analyzer` обычно располагается под
`<cacheRoot>/providers/bsl-analyzer/source-<identity-digest>`. Если этот путь
оказался бы внутри выбранного корня исходников, тот же ключ размещается под
частным runtime-корнем вне дерева. Digest выводится из нормализованной пары
`workspaceRoot + sourceRoot`, поэтому кеш переживает пересоздание сервиса, но
не разделяется между наборами исходников и связанными рабочими деревьями
(INV-CACHE-ORCHESTRATOR-OWNED, INV-CACHE-WORKTREE-ISOLATION, ADR-0018).
```

- [ ] **Step 2: Запустить архитектурные и документные проверки**

Run:

```powershell
python -m unittest discover -s tests/ci -p test_architecture_registry.py
python -m unittest discover -s tests/ci -p test_product_contracts.py
python -m unittest discover -s tests/ci -p test_design_documents.py
```

Expected: все тесты PASS.

- [ ] **Step 3: Запустить Rust-проверки затронутого crate**

Run:

```powershell
cargo fmt --all -- --check
cargo test -p unica-coder --lib
cargo test -p unica-coder --test issue_89_workspace_service
```

Expected: все проверки PASS; если платформенный интеграционный тест корректно пропущен вне своей платформы, это фиксируется в отчёте как SKIP, а не FAIL.

- [ ] **Step 4: Запустить полный CI-набор Python**

Run:

```powershell
python -m unittest discover -s tests/ci
```

Expected: PASS без новых падений.

- [ ] **Step 5: Закоммитить синхронизацию runtime-документа**

```powershell
git diff --check
git add -- spec/architecture/runtime.md
git commit -m "docs: document analyzer provider cache"
```

### Task 4: Issue, публикация ветки и upstream PR

**Files:**
- Inspect: `docs/design/2026-08-12-bsl-analyzer-external-cache-design.md`
- Inspect: `docs/plans/2026-08-13-bsl-analyzer-external-cache.md`
- Inspect: final `git diff upstream/main...HEAD`

**Interfaces:**
- Consumes: полностью проверенная ветка, commit SHA каждого изменения, закрытый контекст `IngvarConsulting/unica#412`.
- Produces: новая issue в `IngvarConsulting/unica`, опубликованная ветка `APonkratov/unica:codex/bsl-analyzer-external-cache`, PR в `IngvarConsulting/unica:main`.

- [ ] **Step 1: Провести финальную проверку перед публикацией**

```powershell
git status --short --branch
git diff --check upstream/main...HEAD
git log --oneline --decorate upstream/main..HEAD
```

Expected: рабочее дерево чистое; ветка содержит только design, plan, runtime-код, tool lock, тесты и runtime-документ этой задачи.

- [ ] **Step 2: Создать отдельную issue-продолжение #412**

Title:

```text
fix(workspace): вынести кеш bsl-analyzer из корней исходников
```

Body:

```markdown
## Проблема

Unica 0.11.0 поставляет `bsl-analyzer` 0.2.62 и запускает workspace MCP с
`--source-dir`, но без отдельного `--cache-dir`. Анализатор создаёт
`<sourceRoot>/.build/bsl-graph.db` и `<sourceRoot>/.build/bsl-search.db*` прямо
в `src/cf` и `src/cfe/*`.

Это остаточная первопричина из #412. В воспроизведённом сценарии следующая
runtime-загрузка приняла файлы кеша за часть XML-выгрузки, завершилась ошибкой
про неизвестное свойство `bsl-graph` у `Configuration`, а последующее
инкрементальное перечисление сохранило загрязнённое состояние. Фильтрация
`.build` downstream полезна как defense in depth, но служебные базы изначально
не должны создаваться внутри source root.

## Разблокировка upstream

В `bsl-analyzer` 0.2.67 выпущен поддерживаемый флаг
`mcp serve --cache-dir <PATH>`. Для него опубликованы платформенные артефакты
`bsl-analyzer-v0.2.67-build.1` в `IngvarConsulting/unica-toolchain`.

## Ожидаемое поведение

- Unica поставляет `bsl-analyzer` 0.2.67.
- Workspace-service всегда передаёт внешний `--cache-dir`.
- Кеш обычно располагается под
  `<cacheRoot>/providers/bsl-analyzer/source-<identity-digest>`, а при вложенном
  `cacheRoot` использует внешний частный runtime-корень с тем же ключом.
- Identity выводится из нормализованной пары `workspaceRoot + sourceRoot`.
- Кеши `cf`, каждого `cfe` и git worktree изолированы.
- Кеш переживает пересоздание workspace-service и не зависит от `services/`.
- При общем `UNICA_CACHE_DIR` разные workspace/worktree не смешивают индексы.

## Критерии приёмки

1. Аргументы запуска содержат ровно один `--cache-dir` с ожидаемым путём.
2. Внутри source root после analyzer-backed операции не создаётся `.build`
   анализатора.
3. Одинаковая нормализованная пара корней повторно использует путь; другой
   source root или workspace получает другой.
4. Версия, source commit, имена артефактов и SHA-256 закреплены package-contract
   тестом.

## Неграницы

- Изменение обхода generated-каталогов в `v8-runner`.
- Миграция или удаление существующих `<sourceRoot>/.build`.
- Новая публичная настройка Unica.
```

Добавить существующую метку `found-on:release`.

- [ ] **Step 3: Опубликовать ветку по workflow `github:yeet`**

Перед публикацией прочитать и применить `github:yeet`, затем выполнить scoped push:

```powershell
git push -u origin codex/bsl-analyzer-external-cache
```

Expected: remote branch создан в `APonkratov/unica` и отслеживается локальной веткой.

- [ ] **Step 4: Создать PR в upstream**

Title:

```text
fix(workspace): route bsl-analyzer cache outside source roots
```

PR body должен содержать:

```markdown
## Summary

- update the bundled bsl-analyzer to 0.2.67
- derive a stable provider cache from normalized workspace and source roots
- pass the external cache through `mcp serve --cache-dir`
- document the provider-cache runtime layout

## Validation

- `cargo fmt --all -- --check`
- `cargo test -p unica-coder --lib`
- `cargo test -p unica-coder --test issue_89_workspace_service`
- `python -m unittest discover -s tests/ci`

Closes #<created-issue-number>
```

Base: `IngvarConsulting/unica:main`; head: `APonkratov:codex/bsl-analyzer-external-cache`.

- [ ] **Step 5: Проверить состояние опубликованного PR**

```powershell
gh pr view --repo IngvarConsulting/unica --json number,title,url,state,baseRefName,headRefName,isDraft,statusCheckRollup
git status --short --branch
```

Expected: PR открыт в `main`, head-ветка верна, рабочее дерево чистое; текущие проверки и их состояние перечислены без заявления об успехе до фактического завершения.
