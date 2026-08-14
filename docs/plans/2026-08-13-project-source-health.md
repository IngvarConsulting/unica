# Project Source Health Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking. Every production change starts with a
> failing test, records the observed RED reason, and ends with the narrow GREEN
> command before broader verification.

**Goal:** Реализовать ADR-0060: `unica.project.status` одним read-only вызовом
публикует типизированные `ready`, `repositoryReady`, `checks[]` и
`diagnostics[]` для workspace, Git-репозитория и каждого source set, а AI
получает доказанную безопасную инструкцию исправления.

**Architecture:** Application-координатор получает один immutable
`ProjectHealthSnapshot` через `ApplicationPorts` и передаёт его чистому
доменному evaluator-у. Infrastructure собирает только факты: layout, Git index,
ignore provenance, attributes, EOL и staged blobs; severity, readiness,
дедупликация и remediation принадлежат домену. Полная проверка запускается
только `unica.project.status`; `unica.project.map` остаётся чистой картой и Git
не становится обязательным условием других операций Unica.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, существующие
`CancellationToken`, `ProviderDeadline`, `ManagedChild` и `ProcessRunner`, Git
CLI с NUL-протоколами, Python 3.12 contract tests, Markdown ADR/invariant corpus.

## Global Constraints

- ADR-0060 и выведенные из него `INV-MCP-PROJECT-READINESS`,
  `INV-SOURCE-ROOT-SEPARATION`, `INV-SOURCE-PORTABLE-GIT` владеют действующей
  границей реализации; датированная design-записка фиксирует только происхождение
  решения. До последнего task ADR остаётся `proposed`; в `accepted` она
  переводится только вместе с работающим кодом, тестами и выведенными правилами.
- Публичный инструмент остаётся один: `unica.project.status`. Новый
  `unica.project.check`, новые аргументы и `outputSchema` не добавляются.
- `unica.project.map` возвращает только карту источников и не запускает Git.
- Найденные проблемы проекта дают `ok=true`; отмена всего вызова или внутренний
  отказ без достоверного снимка дают `ok=false` и не публикуют правдоподобные
  частичные данные.
- `ready` зависит только от областей `workspace|sourceSet`;
  `repositoryReady` — только от `repository`. Отсутствие Git может оставить
  `ready=true`, но всегда делает `repositoryReady=false`.
- Неразобранный `v8project.yaml` даёт
  `source_set.inspection_incomplete`; полный снимок без source sets —
  `source_set.none_found`. Repository checks, которым нужны roots, имеют один
  repository-wide `notRun`, а не `passed`; несуществующие per-set checks не
  выдумываются без идентичности набора.
- Нормализованный source root обязан быть строгим потомком workspace для
  health policy. `.`, `./`, абсолютный alias и symlink/reparse на workspace
  дают `source_set.root_is_workspace`; предметные инструменты не получают
  скрытый preflight и не блокируются автоматически.
- Общие layout/Git проверки применяются ко всем source sets. Форматный профиль
  первого среза — только Platform XML; EDT получает `notApplicable` для
  специальных resource checks.
- Переносимый ignore доказывается только правилом из tracked `.gitignore`.
  `.git/info/exclude`, global excludes и untracked `.gitignore` недостаточны.
- Переносимые attributes доказываются только staged `.gitattributes` из index;
  local `.git/info/attributes`, global/system policies и working-only edits не
  засчитываются.
- `ConfigDumpInfo.xml` классифицируется по staged blob существующим
  `config_dump_info_xml_kind`. При неоднозначности команда `git rm` запрещена.
- Platform XML text policy принимает effective `text=set|auto` либо явный
  `eol=lf|crlf`; `text=unset` означает binary. Index EOL `lf|none` допустим,
  `crlf|mixed` ошибочен. Working EOL `lf|crlf|none` допустим, `mixed` и
  одиночный `cr` имеют разные коды.
- `XDTOPackages/<name>/Ext/Package.bin` всегда text и перекрывает broad
  `*.bin binary`; остальные доказанные `.bin`, `.axdt` и `.addin` считаются
  binary. Неизвестная роль не угадывается.
- Working EOL проверяется потоково только для уже классифицированных tracked
  text paths: максимум 32 MiB на файл и 256 MiB на один status. Превышение,
  read error, symlink/reparse или отмена не превращаются в pass.
- LFS остаётся `info` и не меняет флаги. Пороги первого профиля:
  `10 * 1024 * 1024` байт для одного binary и `100 * 1024 * 1024` байт
  суммарно; broad `*.bin` command не публикуется.
- Remediation никогда не выполняется. Команда имеет форму
  `RemediationCommand { program, argv, cwd }`, без shell-строки. Максимум
  20 примеров paths и 20 evidence; `count` сохраняет полное число совпадений.
  Git-команды используют repo-relative `argv` и `cwd` доказанного repository
  root, поэтому parent-repository/linked-worktree не искажают path.
- Все Git-процессы делят общий `ProviderDeadline` публичного вызова, соблюдают
  cancellation и пределы stdout/stderr. Truncation, lossy UTF-8, неизвестная
  запись, неполный stdin и timeout никогда не становятся `passed`.
- Application не запускает Git напрямую (`INV-APP-NO-DIRECT-GIT`) и status/map
  не запускают hidden services (`INV-APP-LAZY-HIDDEN-SERVICES`).
- PR [#473](https://github.com/IngvarConsulting/unica/pull/473) владеет cache-dir
  `bsl-analyzer`. Этот implementation PR имеет base `main`, не базируется на
  head #473 и не повторяет его изменение. Если #473 сольётся во время работы,
  обновить branch от `main` и повторить layout/cache tests.
- Не сканировать `target`, `.build`, `dist`, `docs-local`, `docs/design`,
  `docs/plans` как произвольные корпуса. Health inspector использует declared
  roots, exact service paths и tracked Git paths.
- Каждый defect выполняется red → green. Rust-тесты запускать с
  `-- --test-threads=1`; Python CI — через `python3.12`.
- После каждого task запускать `cargo fmt --all`, `git diff --check` и
  `git status --short`. Коммиты ниже — самостоятельные review-гейты одного
  итогового implementation PR.

## File Structure

| File | Responsibility |
| --- | --- |
| Create: `crates/unica-coder/src/domain/project_health.rs` | Публичные typed-модели, закрытый fact enum, readiness calculus, коды, агрегация и безопасная remediation. |
| Create: `crates/unica-coder/src/domain/project_health/layout.rs` | Чистые правила source discovery/layout и подавление только производных layout-фактов. |
| Create: `crates/unica-coder/src/domain/project_health/repository.rs` | Чистые правила Git/ignore/CDFI/attributes/EOL/LFS. |
| Create: `crates/unica-coder/src/application/project_health.rs` | `ProjectHealthCoordinator`, mapping complete/cancelled/fatal inspection в `HandlerOutcome`. |
| Modify: `crates/unica-coder/src/application/{mod,ports}.rs` | Публичная диспетчеризация status, port снимка, удаление старого status builder и Git warning из map. |
| Create: `crates/unica-coder/src/infrastructure/project_health.rs` | Композиция одного снимка и общий deadline/cancellation. |
| Create: `crates/unica-coder/src/infrastructure/project_health/layout.rs` | `SourceLayoutInspector` и безопасная path identity без обхода source tree. |
| Create: `crates/unica-coder/src/infrastructure/project_health/git.rs` | `GitRepositoryInspector`, NUL parsers, repo/index/ignore/staged blob facts. |
| Create: `crates/unica-coder/src/infrastructure/project_health/resources.rs` | `SourceResourcePolicyInspector`, Platform XML role classifier и LFS size facts. |
| Modify: `crates/unica-coder/src/infrastructure/{mod,application_ports,internal_adapters,source_roots}.rs` | Подключение inspector-а, переиспользуемая route validation, перенос старого CDFI adapter-а. |
| Modify: `crates/unica-coder/src/infrastructure/platform/process.rs` | Bounded stdin для `git ... --stdin` без shell/pipe deadlock и явный признак invalid UTF-8 вместо эвристики по `U+FFFD`. |
| Create: `crates/unica-coder/tests/platform_project_health.rs` | Публичные symlink/reparse, parent-repo и linked-worktree contracts. |
| Modify: `crates/unica-coder/src/interfaces/mcp.rs`, `tests/ci/test_unica_mcp_smoke.py` | Consumer-visible status payload и read-only MCP smoke. |
| Create: `tests/ci/test_project_health_contract.py` | Синхронизация ADR, invariants, tool review, acceptance и AI guidance. |
| Modify: `spec/architecture/{invariants,change-checklist,building-blocks,concepts,runtime,tool-surface-review.json,tool-surface.md}` | Действующие правила и ведомость изменённого результата. |
| Modify: `spec/decisions/{0056-project-status-publikuet-gotovnost-proekta.md,README.md}` | Принятие решения только после GREEN реализации. |
| Modify: `spec/acceptance/unica-mcp-validation.md` | Исполняемая матрица project health. |
| Modify: `plugins/unica/skills/v8-runner/SKILL.md`, `plugins/unica/references/use-cases/workspace-runtime.md`, `tests/ci/test_unica_skills.py` | AI вызывает preflight, читает typed remediation и не выполняет её без полномочия. |

---

## Task 0: Зафиксировать изоляцию и baseline

**Files:** none.

**Interfaces:**

- Consumes: reviewed commit `1fea780c` и план
  `docs/plans/2026-08-13-project-source-health.md`.
- Produces: branch `codex/project-source-health` в текущем app-managed worktree,
  основанная на `main`, а не на head PR #473.

- [ ] **Step 1: Verify the worktree and clean tree.**

```bash
git rev-parse --git-dir
git rev-parse --git-common-dir
git status --short --branch
git log -3 --oneline
```

Expected: app-managed linked worktree, только reviewed design/ADR/plan поверх
base, незакоммиченных файлов нет. Если есть чужие изменения, остановиться и не
переносить их автоматически.

- [ ] **Step 2: Create the implementation branch.**

```bash
git fetch origin main
git switch -c codex/project-source-health
git merge-base --is-ancestor origin/main HEAD
gh pr view 473 --repo IngvarConsulting/unica --json state,baseRefName,headRefName,mergeCommit
```

Expected: ветка создана; `origin/main` является предком; #473 имеет base
`main`, но его head не является base текущей ветки. Если #473 уже merged,
сначала fast-forward/rebase от свежего `origin/main`, затем повторить baseline.

- [ ] **Step 3: Run the narrow baseline.**

```bash
python3.12 -m unittest tests.ci.test_design_documents tests.ci.test_architecture_registry
cargo test -p unica-coder project_status --lib -- --test-threads=1
cargo test -p unica-coder config_dump_info --lib -- --test-threads=1
```

Expected: PASS на исходной ветке. Это не доказывает новую функцию, а отделяет
регрессии реализации от baseline.

---

## Task 1: Ввести доменный typed-контракт и readiness calculus

**Files:**

- Create: `crates/unica-coder/src/domain/project_health.rs`
- Create: `crates/unica-coder/src/domain/project_health/layout.rs`
- Create: `crates/unica-coder/src/domain/project_health/repository.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`

**Interfaces:**

- Consumes: `ProjectSourceSet` из `domain::project_sources`.
- Produces for Tasks 2, 4, 5 and 6:

```rust
pub(crate) const MAX_PROJECT_DIAGNOSTIC_PATHS: usize = 20;
pub(crate) const MAX_PROJECT_DIAGNOSTIC_EVIDENCE: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticScope { Workspace, Repository, SourceSet }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticSeverity { Error, Warning, Info }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProjectCheckStatus { Passed, Failed, NotRun, NotApplicable }

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ProjectCheckId {
    SourceDiscovery,
    SourceLayout,
    SourceFormat,
    SourceGeneratedPaths,
    RepositoryDiscovery,
    RepositoryIndex,
    RepositoryIgnore,
    RepositoryGeneratedPaths,
    RepositoryConfigDumpInfo,
    RepositoryAttributes,
    RepositoryIndexEol,
    RepositoryWorkingEol,
    RepositoryLfs,
}

impl ProjectCheckId {
    pub(crate) const ALL: [Self; 13];
    pub(crate) const fn as_str(self) -> &'static str;
    pub(crate) const fn scope(self) -> DiagnosticScope;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectCheckOutcome {
    Completed,
    NotRun { reason: String },
    NotApplicable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectCheckObservation {
    pub(crate) id: ProjectCheckId,
    pub(crate) scope: DiagnosticScope,
    pub(crate) source_set: Option<String>,
    pub(crate) outcome: ProjectCheckOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemediationCommand {
    pub(crate) program: String,
    pub(crate) argv: Vec<String>,
    pub(crate) cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Remediation {
    pub(crate) summary: String,
    pub(crate) steps: Vec<String>,
    pub(crate) commands: Vec<RemediationCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectCheck {
    pub(crate) id: String,
    pub(crate) scope: DiagnosticScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_set: Option<String>,
    pub(crate) status: ProjectCheckStatus,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectDiagnostic {
    pub(crate) code: String,
    pub(crate) severity: DiagnosticSeverity,
    pub(crate) scope: DiagnosticScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_set: Option<String>,
    pub(crate) paths: Vec<String>,
    pub(crate) count: usize,
    pub(crate) message: String,
    pub(crate) evidence: Vec<String>,
    pub(crate) remediation: Remediation,
}

pub(crate) struct ProjectHealthSnapshot {
    pub(crate) workspace_root: String,
    pub(crate) cache_root: String,
    pub(crate) repository_root: Option<String>,
    pub(crate) source_sets: Option<Vec<ProjectSourceSet>>,
    pub(crate) source_targets_complete: bool,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectHealthInspectionError {
    Cancelled,
    Fatal(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectHealthReport {
    pub(crate) workspace_root: String,
    pub(crate) cache_root: String,
    pub(crate) ready: bool,
    pub(crate) repository_ready: bool,
    pub(crate) checks: Vec<ProjectCheck>,
    pub(crate) source_sets: Option<Vec<ProjectSourceSet>>,
    pub(crate) diagnostics: Vec<ProjectDiagnostic>,
}

pub(crate) fn evaluate_project_health(
    snapshot: ProjectHealthSnapshot,
) -> Result<ProjectHealthReport, String>;
```

Закрытый каталог check IDs и scopes:

| ID | Scope | Когда создаётся observation |
| --- | --- | --- |
| `source.discovery` | `workspace` | Ровно один раз на попытку разобрать source map. |
| `source.layout` | `sourceSet` | Для каждого однозначно обнаруженного набора. |
| `source.format` | `sourceSet` | Для каждого обнаруженного набора. |
| `source.generated_paths` | `sourceSet` | Для каждого безопасно адресуемого root. |
| `repository.discovery` | `repository` | Ровно один раз. |
| `repository.index` | `repository` | После доказанного Git work tree. |
| `repository.ignore` | `repository` | Для cache и каждого применимого source set; `sourceSet` ставится у per-set observation. |
| `repository.generated_paths` | `repository` | Для index-контроля служебных путей. |
| `repository.config_dump_info` | `repository` | Для staged классификации одноимённых файлов. |
| `repository.attributes` | `repository` | Для каждого Platform XML set; EDT — `notApplicable`. |
| `repository.index_eol` | `repository` | Для каждого Platform XML set; EDT — `notApplicable`. |
| `repository.working_eol` | `repository` | Для каждого Platform XML set; EDT — `notApplicable`. |
| `repository.lfs` | `repository` | Для каждого Platform XML set; EDT — `notApplicable`. |

`ProjectCheckId::as_str()` является единственным mapping к этим строкам.
Infrastructure может доказать только `Completed`, `NotRun` или
`NotApplicable`; public `Passed|Failed` вычисляются доменом. Каждый error fact
total-match-ом связывается с одним check key `(id, scope, sourceSet)`.

`ProjectHealthFact` — закрытый enum со всеми первичными фактами:

```rust
pub(crate) enum ProjectHealthFact {
    SourceInspectionIncomplete { reason: String },
    NoSourceSets,
    SourceRootIsWorkspace { source_set: String, path: String, evidence: Vec<String> },
    SourcePathMissing { source_set: String, path: String },
    SourcePathUnsafe { source_set: String, path: String, reason: String },
    SourceNameAmbiguous { name: String, count: usize },
    SourceFormatInvalid { source_set: String, evidence: Vec<String> },
    SourceFormatUnknown { source_set: String, evidence: Vec<String> },
    CacheInsideSourceSet { source_set: String, source_root: String, cache_root: String },
    GeneratedBuildPresent { source_set: String, path: String },
    GitRepositoryAbsent,
    GitExecutableUnavailable { reason: String },
    GitInspectionTimeout { check: ProjectCheckId, source_set: Option<String> },
    GitInspectionIncomplete {
        check: ProjectCheckId,
        source_set: Option<String>,
        reason: String,
    },
    IgnoreRuleMissing { source_set: Option<String>, path: String },
    IgnoreRuleLocalOnly { source_set: Option<String>, path: String, origin: String },
    GeneratedPathTracked { source_set: Option<String>, path: String },
    RuntimeSidecarTracked { source_set: String, path: String },
    ConfigDumpInfoUnclassified { source_set: Option<String>, path: String, reason: String },
    AttributesLocalOnly { source_set: String, path: String, evidence: Vec<String> },
    TextPolicyMissing { source_set: String, path: String },
    BinaryPolicyMissing { source_set: String, path: String },
    TextResourceMarkedBinary { source_set: String, path: String },
    IndexEolNotLf { source_set: String, path: String, observed: String },
    MixedEol { source_set: String, path: String },
    WorkingEolUnsupported { source_set: String, path: String, observed: String },
    LfsConsider {
        source_set: String,
        count: usize,
        total_bytes: u64,
        largest_path: String,
        largest_bytes: u64,
        single_threshold_bytes: u64,
        aggregate_threshold_bytes: u64,
        paths: Vec<String>,
    },
}
```

Total mapping fact → check key:

| Fact variants | Check ID / scope |
| --- | --- |
| `SourceInspectionIncomplete`, `NoSourceSets`, `SourceNameAmbiguous` | `source.discovery` / `workspace` |
| `SourceRootIsWorkspace`, `SourcePathMissing`, `SourcePathUnsafe`, `CacheInsideSourceSet` | `source.layout` / `sourceSet` |
| `SourceFormatInvalid`, `SourceFormatUnknown` | `source.format` / `sourceSet` |
| `GeneratedBuildPresent` | `source.generated_paths` / `sourceSet` |
| `GitRepositoryAbsent`, `GitExecutableUnavailable` | `repository.discovery` / `repository` |
| `GitInspectionTimeout`, `GitInspectionIncomplete` | ID и optional `sourceSet` из fact; scope проверяется через `ProjectCheckId::scope()` |
| `IgnoreRuleMissing`, `IgnoreRuleLocalOnly` | `repository.ignore` / `repository` |
| `GeneratedPathTracked` | `repository.generated_paths` / `repository` |
| `RuntimeSidecarTracked`, `ConfigDumpInfoUnclassified` | `repository.config_dump_info` / `repository` |
| `AttributesLocalOnly`, `TextPolicyMissing`, `BinaryPolicyMissing`, `TextResourceMarkedBinary` | `repository.attributes` / `repository` |
| `IndexEolNotLf` | `repository.index_eol` / `repository` |
| `MixedEol`, `WorkingEolUnsupported` | `repository.working_eol` / `repository` |
| `LfsConsider` | `repository.lfs` / `repository` |

`repository.index` не получает policy fact: parse/process failure создаёт
`GitInspectionTimeout|GitInspectionIncomplete` с этим ID. Все repository facts
с `source_set` должны совпасть с per-set observation того же ID; repository-wide
facts используют `None`.

- [ ] **Step 1: Write failing serialization and readiness tests.**

В `domain/project_health.rs` сначала добавить тесты:

```rust
#[test]
fn project_health_serializes_the_closed_public_shape() {
    let report = evaluate_project_health(snapshot_with(
        vec![ProjectHealthFact::GitRepositoryAbsent],
        vec![observation(
            ProjectCheckId::RepositoryDiscovery,
            DiagnosticScope::Repository,
            None,
            ProjectCheckOutcome::Completed,
        )],
    )).unwrap();
    let value = serde_json::to_value(report).unwrap();

    assert_eq!(value["ready"], true);
    assert_eq!(value["repositoryReady"], false);
    assert_eq!(value["diagnostics"][0]["code"], "git.repository_absent");
    assert_eq!(value["diagnostics"][0]["count"], 1);
    assert!(value["diagnostics"][0]["remediation"]["commands"].is_array());
}

#[test]
fn not_run_closes_only_its_scope_and_not_applicable_closes_neither() {
    let report = evaluate_project_health(snapshot_with(
        Vec::new(),
        vec![
            observation(
                ProjectCheckId::SourceLayout,
                DiagnosticScope::SourceSet,
                Some("main"),
                ProjectCheckOutcome::NotApplicable { reason: "profile does not use layout".into() },
            ),
            observation(
                ProjectCheckId::RepositoryIgnore,
                DiagnosticScope::Repository,
                None,
                ProjectCheckOutcome::NotRun { reason: "Git is absent".into() },
            ),
        ],
    )).unwrap();

    assert!(report.ready);
    assert!(!report.repository_ready);
}
```

В test module определить используемые builders явно, без production API:

```rust
fn snapshot_with(
    facts: Vec<ProjectHealthFact>,
    replacements: Vec<ProjectCheckObservation>,
) -> ProjectHealthSnapshot {
    let repository_absent = facts.iter().any(|fact| {
        matches!(fact, ProjectHealthFact::GitRepositoryAbsent)
    });
    let mut observations = ProjectCheckId::ALL
        .into_iter()
        .map(|id| {
            let source_set = (id.scope() == DiagnosticScope::SourceSet)
                .then_some("main");
            let outcome = if repository_absent
                && id.scope() == DiagnosticScope::Repository
                && id != ProjectCheckId::RepositoryDiscovery
            {
                ProjectCheckOutcome::NotRun { reason: "Git is absent".into() }
            } else {
                ProjectCheckOutcome::Completed
            };
            observation(id, id.scope(), source_set, outcome)
        })
        .collect::<Vec<_>>();
    for replacement in replacements {
        observations.retain(|existing| {
            (existing.id, existing.scope, existing.source_set.as_deref())
                != (replacement.id, replacement.scope, replacement.source_set.as_deref())
        });
        observations.push(replacement);
    }
    ProjectHealthSnapshot {
        workspace_root: "/workspace".into(),
        cache_root: "/workspace/.build/unica".into(),
        repository_root: (!repository_absent).then(|| "/workspace".into()),
        source_sets: Some(vec![ProjectSourceSet {
            name: "main".into(),
            kind: SourceSetKind::Configuration,
            path: "src".into(),
            source_format: SourceFormat::PlatformXml,
            format_evidence: vec!["Configuration.xml".into()],
            format_probe_error: None,
        }]),
        source_targets_complete: true,
        observations,
        facts,
    }
}

fn observation(
    id: ProjectCheckId,
    scope: DiagnosticScope,
    source_set: Option<&str>,
    outcome: ProjectCheckOutcome,
) -> ProjectCheckObservation {
    ProjectCheckObservation {
        id,
        scope,
        source_set: source_set.map(str::to_owned),
        outcome,
    }
}
```

Добавить отдельные tests для `SourceInspectionIncomplete`, `NoSourceSets`,
error/info, сериализации зарезервированного warning enum, `failed`, scope
isolation и обязательного `sourceSet` у source-scoped записей.

- [ ] **Step 2: Run the domain tests and witness RED.**

```bash
cargo test -p unica-coder project_health --lib -- --test-threads=1
```

Expected: FAIL, потому что module/types/evaluator ещё отсутствуют. Ошибка не
должна быть синтаксической ошибкой теста.

- [ ] **Step 3: Implement the closed models and readiness calculus.**

Подключить временно
`#[allow(dead_code)] pub(crate) mod project_health;` в `domain/mod.rs`.
`evaluate_project_health` сначала проверяет snapshot invariants: observation
keys уникальны; `sourceSet` обязателен только для scope `sourceSet` и допустим
для per-set repository checks; `source.discovery` и `repository.discovery`
присутствуют ровно один раз; каждый repository ID представлен хотя бы одним
observation; каждый fact ссылается на существующий `Completed` check key;
`NotRun|NotApplicable` не имеют facts. Если `source_targets_complete=true`,
каждый уникальный source set имеет все три per-set source observations; при
отсутствии адресуемого set они не выдумываются. Несогласованность возвращает
`Err("project_health_snapshot_invalid: ...")`, которое coordinator отдаёт как
fatal `ok=false`.

Repository fact, который предлагает Git-command, дополнительно требует
`snapshot.repository_root=Some(...)`; без доказанного root evaluator возвращает
snapshot-invalid и не подставляет workspace как догадку.

После validation домен строит public checks: `Completed` + error fact того же
key → `Failed`, иной `Completed` → `Passed`, остальные outcomes переносятся в
`NotRun|NotApplicable` с reason. `Failed.reason` равен message первой по
детерминированной сортировке error-диагностики этого key; `Passed.reason=null`.
Public checks сортируются по `(scope, id, sourceSet)`.
Затем:

```rust
let ready = !public_checks.iter().any(|check| {
    matches!(check.scope, DiagnosticScope::Workspace | DiagnosticScope::SourceSet)
        && matches!(check.status, ProjectCheckStatus::Failed | ProjectCheckStatus::NotRun)
});

let repository_ready = !public_checks.iter().any(|check| {
    check.scope == DiagnosticScope::Repository
        && check.id != ProjectCheckId::RepositoryLfs.as_str()
        && matches!(check.status, ProjectCheckStatus::Failed | ProjectCheckStatus::NotRun)
});
```

`layout::diagnostic` и `repository::diagnostic` обязаны быть total match по
`ProjectHealthFact`; каждый variant получает ровно код из design и одну
область. `SourceInspectionIncomplete` →
`source_set.inspection_incomplete`, `NoSourceSets` →
`source_set.none_found`.

- [ ] **Step 4: Implement deterministic aggregation and safe remediation.**

Агрегировать одинаковые `(code, severity, scope, sourceSet, message)` до
построения remediation; складывать полный `count`, сортировать и dedup полные
paths/evidence. Remediation строить по агрегату, затем обрезать только публичные
примеры до 20. Итоговый порядок:

```rust
diagnostics.sort_by_key(|diagnostic| (
    severity_rank(diagnostic.severity),
    diagnostic.scope,
    diagnostic.source_set.clone(),
    diagnostic.code.clone(),
    diagnostic.paths.first().cloned(),
));
```

Начальный count равен `1` для обычного fact, `SourceNameAmbiguous.count` для
группы одноимённых записей и `LfsConsider.count` для набора бинарных файлов;
агрегация складывает эти значения, а не число уже агрегированных объектов.

Для доказанных runtime sidecars числом 1–20 публиковать одну команду
`git --literal-pathspecs rm --cached -- <path>...` с каждым sorted path отдельным argv. При count
выше 20 не публиковать частичную команду: steps сообщают полный count, требуют
получить exact список и повторить status. То же правило применяется к любой
агрегированной mutating command: команда либо покрывает весь агрегат, либо
отсутствует.
`cwd` этой команды равен `snapshot.repository_root`, а paths остаются
repo-relative значениями ровно из NUL index snapshot.

Добавить tests `diagnostics_are_sorted_and_aggregated_without_losing_count`,
`ambiguous_config_dump_info_has_no_command` и
`runtime_sidecar_command_keeps_the_path_as_one_argv_item`, а также
`runtime_sidecar_aggregation_never_publishes_a_partial_command`. Последний
проверяет 21 path → `count=21`, `paths.len()=20`, `commands=[]`. Тест с одним
path требует:

```rust
assert_eq!(command.program, "git");
assert_eq!(command.argv, ["rm", "--cached", "--", "line\nbreak/ConfigDumpInfo.xml"]);
```

- [ ] **Step 5: Run GREEN and commit.**

```bash
cargo test -p unica-coder project_health --lib -- --test-threads=1
cargo fmt --all
git diff --check
git add crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/domain/project_health.rs crates/unica-coder/src/domain/project_health
git commit -m "feat(project): add typed health report model"
```

Expected: domain tests PASS; commit содержит только closed model, evaluator и
его tests.

---

## Task 2: Проверять source discovery и layout без обхода дерева

**Files:**

- Create: `crates/unica-coder/src/infrastructure/project_health.rs`
- Create: `crates/unica-coder/src/infrastructure/project_health/layout.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_roots.rs`

**Interfaces:**

- Consumes: `ProjectHealthSnapshot`, `ProjectHealthFact`,
  `ProjectCheckObservation` from Task 1; `discover_project_source_map`;
  `normalize_path_identity` and host path policy.
- Produces for Task 6:

```rust
pub(crate) struct SourceLayoutInspector;

pub(crate) struct SourceLayoutInspection {
    pub(crate) source_sets: Option<Vec<ProjectSourceSet>>,
    pub(crate) source_targets_complete: bool,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
    pub(crate) roots: Vec<InspectedSourceRoot>,
}

pub(crate) struct InspectedSourceRoot {
    pub(crate) source_set: ProjectSourceSet,
    pub(crate) path: PathBuf,
}

impl SourceLayoutInspector {
    pub(crate) fn inspect(
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<SourceLayoutInspection, ProjectHealthInspectionError>;
}
```

В `source_roots.rs` выделить из `resolve_named_source_set` общий снимок route:

```rust
pub(crate) struct DeclaredSourceRootRoute {
    pub(crate) lexical_path: PathBuf,
    pub(crate) identity_path: PathBuf,
}

pub(crate) fn inspect_declared_source_root_route(
    workspace_root: &Path,
    configured_path: &str,
) -> Result<DeclaredSourceRootRoute, NamedSourceSetError>;
```

Он выполняет lexical containment и нормализует identity, но сам не запрещает
symlink/reparse. Действующий `resolve_named_source_set` после helper-а вызывает
прежний `reject_linked_source_root_route`, сохраняя поведение предметных
операций. Health inspector сначала сравнивает identity с workspace: linked alias
на workspace получает первичный `SourceRootIsWorkspace`; для иной identity
linked route получает `SourcePathUnsafe`.

- [ ] **Step 1: Write failing discovery/layout tests.**

В `infrastructure/project_health/layout.rs` добавить table tests для:

```rust
#[test]
fn root_identity_equal_to_workspace_is_one_primary_fact() {
    for configured in [".", "./", "src/.."] {
        let fixture = layout_fixture(configured);
        let inspection = SourceLayoutInspector::inspect(
            &fixture.context,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        ).unwrap();

        assert_eq!(inspection.facts.len(), 1);
        let ProjectHealthFact::SourceRootIsWorkspace { evidence, .. } =
            &inspection.facts[0]
        else {
            panic!("root-is-workspace fact expected");
        };
        assert!(evidence.iter().any(|value| value.contains(".build")));
    }
}
```

Остальные tests: malformed `v8project.yaml` → inspection incomplete и пустые
roots; no source sets → none found; duplicate list names; missing path;
`../outside`; symlink/reparse на workspace → root-is-workspace; symlink/reparse
на иной путь → unsafe; Invalid/Unknown format; proper `src/cf`, `src/cfe`,
`src/epf`, `src/erf`; physical `src/.build`; cache inside source.

- [ ] **Step 2: Run layout tests and witness RED.**

```bash
cargo test -p unica-coder project_health::layout --lib -- --test-threads=1
```

Expected: FAIL из-за отсутствующих inspector/helper; fixture сама должна
создаваться успешно.

- [ ] **Step 3: Extract route validation without changing existing callers.**

Сначала перенести lexical/identity часть из `resolve_named_source_set` в
`inspect_declared_source_root_route`, заменить старый caller, оставив его
`reject_linked_source_root_route` после helper-а, и запустить:

```bash
cargo test -p unica-coder source_roots --lib -- --test-threads=1
```

Expected: существующие source-root tests PASS до добавления новой policy.
Helper не должен выбирать health cause или принимать linked route: он только
возвращает доказанный route snapshot. Равенство workspace и приоритет этой
причины решает health evaluator, а предметный caller сохраняет прежний запрет.

- [ ] **Step 4: Implement `SourceLayoutInspector`.**

Порядок одного снимка:

1. проверить cancellation и remaining deadline;
2. вызвать `discover_project_source_map` ровно один раз;
3. при ошибке вернуть source discovery `NotRun`, пустые roots,
   `source_targets_complete=false` и
   `SourceInspectionIncomplete`, но не fatal; домен превратит discovery в
   `failed`, а Git inspector из Task 4 создаст repository-wide `NotRun` для
   source-dependent checks;
4. при пустом полном результате вернуть `NoSourceSets` и incomplete targets;
5. посчитать duplicate names до per-set inspection;
6. для duplicate-name group создать один `SourceNameAmbiguous` с полным count,
   выставить incomplete targets и
   не создавать неадресуемые per-set observations/roots; уникальные sets
   продолжить независимо;
7. для каждого уникального set вызвать `inspect_declared_source_root_route`, затем сначала
   сравнить identity с workspace; только для иной identity проверить linked
   component, existence, `.build` exact path и cache containment;
8. для root-is-workspace положить linked alias/cache/build только в evidence и не добавлять
   `CacheInsideSourceSet`/`GeneratedBuildPresent`;
9. path, identity которого нельзя безопасно получить, а также Invalid/Unknown
   format выставляют incomplete targets; roots могут оставаться в snapshot как
   evidence, но source-dependent repository checks до полного доказательства
   области получают только repository-wide `NotRun`;
10. не читать произвольные дочерние файлы и не запускать Git.

- [ ] **Step 5: Run GREEN and commit.**

```bash
cargo test -p unica-coder project_health::layout --lib -- --test-threads=1
cargo test -p unica-coder source_roots --lib -- --test-threads=1
cargo fmt --all
git diff --check
git add crates/unica-coder/src/infrastructure/mod.rs crates/unica-coder/src/infrastructure/project_health.rs crates/unica-coder/src/infrastructure/project_health crates/unica-coder/src/infrastructure/source_roots.rs
git commit -m "feat(project): inspect source set layout"
```

Expected: layout tests PASS; public test из Task 6 ещё не создавался, новых
ignored tests нет.

---

## Task 3: Добавить bounded stdin и точную полноту process output

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/platform/process.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Interfaces:**

- Consumes: существующие `ManagedChild`, `ManagedCommand`, `ProcessCommand`,
  `ProcessOutput` и capture limits.
- Produces for Tasks 4–5:

```rust
impl ManagedChild {
    pub fn run_with_input(
        command: ManagedCommand,
        input: Vec<u8>,
    ) -> Result<ManagedOutput, String>;
}

pub trait ProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String>;

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        input: &[u8],
    ) -> Result<ProcessOutput, String>;
}
```

Default `ProcessRunner::run_with_input` вызывает `run` только для пустого
input; непустой input возвращает стабильный
`process_failed: process runner does not support stdin`. `SystemProcessRunner`
переопределяет метод через `ManagedChild::run_with_input`.

`ProcessCommand` получает `env: Vec<(OsString, OsString)>`; все существующие
callers задают `Vec::new()`, а Git inspector передаёт только `LC_ALL=C` и
`LANG=C`, чтобы отличать stable no-repository stderr от прочих Git failures без
зависимости от локали. `SystemProcessRunner` переносит `command.env` в
`ManagedCommand.env`; fake runners сохраняют env вместе с argv для assertions.

Существующие `ManagedOutput` и `ProcessOutput` дополнительно получают:

```rust
pub stdout_had_invalid_utf8: bool,
pub stderr_had_invalid_utf8: bool,
pub stderr_truncated: bool,
```

Флаги вычисляются из результата `String::from_utf8`, а отображаемая строка при
ошибке по-прежнему строится через lossy conversion. Буквальный валидный символ
`U+FFFD` не выставляет флаг. Все существующие test fixtures дополняются явными
`false`, поэтому downstream inspector не угадывает потерю по содержимому строки.
`ProcessOutput` уже имел `stdout_truncated`; новый `stderr_truncated` перестаёт
терять вторую половину completeness metadata при mapping из `ManagedOutput`.

- [ ] **Step 1: Write failing process tests.**

Расширить `managed_child_test_helper` режимом `echo_stdin_len`, который читает
`read_to_end` и печатает длину и число NUL. Добавить:

```rust
#[test]
fn managed_child_writes_binary_stdin_and_closes_it() {
    let input = b"src/.build/probe\0src/ConfigDumpInfo.xml\0".to_vec();
    let expected_bytes = input.len();
    let output = run_helper_with_input(
        "echo_stdin_len",
        input,
        Duration::from_secs(1),
        CancellationToken::new(),
    ).unwrap();

    assert!(output.status_success);
    assert!(output.stdout.contains(&format!("bytes={expected_bytes}")), "{}", output.stdout);
    assert!(output.stdout.contains("nul=2"), "{}", output.stdout);
}
```

Добавить tests: input больше pipe buffer не deadlock-ит; cancellation и timeout
завершают writer/child; раннее закрытие stdin возвращает bounded error без
zombie. Режим helper-а `write_invalid_utf8` пишет `b"ok\xffend"`; отдельные
tests доказывают `stdout_had_invalid_utf8=true` для этих байтов и `false` для
валидной строки `"ok\u{fffd}end"`.

- [ ] **Step 2: Run process tests and witness RED.**

```bash
cargo test -p unica-coder managed_child_writes_binary_stdin --lib -- --test-threads=1
```

Expected: FAIL, потому что `run_with_input` отсутствует.

- [ ] **Step 3: Implement concurrent stdin ownership.**

`ManagedChild::run_with_input` должен spawn child, take stdin ровно один раз,
запустить writer thread с `write_all(input)` и drop stdin, а основной thread
сразу начать `wait_for_output`, чтобы stdout и stdin не заполнили pipe друг
друга. После output join writer; write error не маскирует
`cancelled|timed_out`, но при обычном исходе возвращается как `process_failed:`.

В обоих путях завершения (`finish_output` и `finish_after_termination`)
декодировать captured bytes общим helper-ом
`decode_captured_text(bytes) -> (String, bool)`. Не определять invalid UTF-8 по
наличию `U+FFFD`: это допустимый Unicode-символ, а не доказательство потери.

Не добавлять `stdin` в `ProcessCommand`: это заставило бы менять каждый caller
и смешало бы optional input с immutable описанием команды.

- [ ] **Step 4: Wire `SystemProcessRunner` and run GREEN.**

```bash
cargo test -p unica-coder managed_child_ --lib -- --test-threads=1
cargo test -p unica-coder internal_adapters --lib -- --test-threads=1
cargo fmt --all
git diff --check
git add crates/unica-coder/src/infrastructure/platform/process.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "refactor(process): support bounded stdin"
```

Expected: все ManagedChild и existing adapter tests PASS; ни один текущий
caller `run` не изменил команду, а новое decoding metadata точно отличает
invalid bytes от валидного `U+FFFD`.

---

## Task 4: Собрать Git repository, ignore, index и staged CDFI facts

**Files:**

- Create: `crates/unica-coder/src/infrastructure/project_health/git.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Interfaces:**

- Consumes: `ProcessRunner::run_with_input` из Task 3,
  `SourceLayoutInspection` из Task 2, общий `ProviderDeadline` и
  `config_dump_info_xml_kind`.
- Produces for Tasks 5–6:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitIndexEntry {
    pub(crate) repo_path: String,
    pub(crate) blob_oid: Option<String>,
    pub(crate) mode: Option<String>,
}

pub(crate) struct GitRepositoryInspection {
    pub(crate) repository_root: Option<PathBuf>,
    pub(crate) entries: Vec<GitIndexEntry>,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct GitRepositoryInspector<'a> {
    runner: &'a dyn ProcessRunner,
}

impl<'a> GitRepositoryInspector<'a> {
    pub(crate) fn new() -> Self;

    #[cfg(test)]
    pub(crate) fn with_runner(
        runner: &'a dyn ProcessRunner,
    ) -> Self;

    pub(crate) fn inspect_base(
        &self,
        context: &WorkspaceContext,
        layout: &SourceLayoutInspection,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<GitRepositoryInspection, ProjectHealthInspectionError>;
}
```

`new()` использует существующий `internal_adapters::system_process_runner()`;
новый process singleton не создаётся. `with_runner` сохраняет нынешние
`RefCell`-based regression fixtures и не требует от fake runner `Sync`.

Внутренние typed parsers:

```rust
fn parse_git_index_entries(stdout: &str) -> Result<Vec<GitIndexEntry>, String>;
fn parse_check_ignore_verbose_z(stdout: &str) -> Result<Vec<IgnoreMatch>, String>;
fn classify_staged_config_dump_info(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    entry: &GitIndexEntry,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> StagedConfigDumpInfo;
```

- [ ] **Step 1: Move the existing CDFI regression oracle before changing behavior.**

Скопировать tests старого `GitTrackingAdapter` в новый module как tests
`project_health_git_*`, но assertions перевести со строки на facts. Обязательные
cases: runtime staged blob при legitimate worktree; legitimate EPF/ERF staged
blob; nested metadata descriptor; unmerged stages; zero OID; malformed,
oversized и truncated blob; symlink mode; cancellation; Unicode, comma и
перевод строки в пути.

Пример нового oracle:

```rust
assert_eq!(inspection.facts.len(), 1);
let ProjectHealthFact::RuntimeSidecarTracked { path, .. } = &inspection.facts[0]
else { panic!("runtime fact expected") };
assert_eq!(path, "epf/line\nbreak/ConfigDumpInfo.xml");
```

- [ ] **Step 2: Add failing repository and ignore tests.**

Добавить real-Git fixtures для:

- no repository;
- missing executable через fake runner;
- parent repository с workspace в подкаталоге;
- tracked root/nested `.gitignore`;
- rule только в `.git/info/exclude`;
- local `core.excludesFile` как global-style origin;
- matching, но untracked `.gitignore`;
- отсутствующее правило до физического файла;
- tracked `.build`, cache path и `DumpFilesIndex.txt`;
- linked worktree;
- timeout, stdout truncation, invalid UTF-8 bytes, валидный literal `U+FFFD` и
  malformed NUL record.

Проверка отсутствующего файла создаёт synthetic candidate и не пишет его:

```rust
assert!(!workspace.join("src/ConfigDumpInfo.xml").exists());
assert!(matches!(
    inspection.facts.as_slice(),
    [ProjectHealthFact::IgnoreRuleMissing { path, .. }]
        if path == "src/ConfigDumpInfo.xml"
));
```

- [ ] **Step 3: Run the Git inspector tests and witness RED.**

```bash
cargo test -p unica-coder project_health_git --lib -- --test-threads=1
```

Expected: FAIL из-за отсутствующих inspector/parsers; старый string adapter ещё
работает и не является причиной RED.

- [ ] **Step 4: Implement repository discovery and one bounded index snapshot.**

Использовать:

```text
git rev-parse --show-toplevel --is-inside-work-tree
git ls-files --cached --stage -z
```

Только discovery запускается с `cwd=context.workspace_root`. Полученный
`--show-toplevel` нормализуется host path policy и обязан содержать workspace.
Все последующие Git-команды запускаются с `cwd=repository_root`; поэтому index,
ignore, attributes и EOL paths всегда repo-relative даже для workspace в
подкаталоге. Каждая команда получает `LC_ALL=C`, `LANG=C` из Task 3.

`process_failed:` на spawn → `GitExecutableUnavailable`; complete valid UTF-8
stderr под принудительной C locale с точным префиксом
`fatal: not a git repository` либо успешный `--is-inside-work-tree=false` →
`GitRepositoryAbsent`; другие nonzero/truncated/lossy outputs →
`GitInspectionIncomplete`. После отсутствующего Git создать один fact,
`repository.discovery=Completed`, остальные Git observations `NotRun` и не
запускать следующие команды; домен построит public discovery `failed`.

`lossy` определяется только `stdout_had_invalid_utf8`/
`stderr_had_invalid_utf8` из Task 3. Literal `U+FFFD` в валидном Git path или
blob не является самостоятельной причиной ошибки. Любой из флагов
`stdout_truncated|stderr_truncated` также запрещает semantic classification.

`parse_git_index_entries` группирует все stages одного path. Ровно один stage 0
обычного файла с nonzero hex OID сохраняет `blob_oid`; symlink, unmerged,
duplicate или zero OID сохраняют path, но `blob_oid=None`.

- [ ] **Step 5: Implement portable ignore provenance.**

Required candidates:

1. фактический cache root, если он внутри repository root;
2. `<sourceRoot>/.build/.unica-health-probe` для каждого inspected root;
3. root `ConfigDumpInfo.xml` и `DumpFilesIndex.txt` только для Platform XML
   Configuration/Extension.

Если `layout.source_targets_complete=false`, завершаются только repository
discovery и полный index snapshot. Все source-dependent repository checks
получают repository-wide `NotRun` (`sourceSet=None`) и не публикуют per-set
`Completed`: иначе частично доказанный root выглядел бы как готовая политика
при неизвестной полной области source set.

Передать все repo-relative candidates как NUL input в:

```text
git check-ignore -v -z --no-index --stdin
```

Parser читает quadruples `source\0line\0pattern\0path\0`. Match считается
portable, только если `source` находится внутри repository и присутствует в
полном tracked index как `.gitignore`. Любой иной matching origin даёт
`IgnoreRuleLocalOnly`; отсутствие path в output — `IgnoreRuleMissing`.

Tracked generated paths проверять по уже полученному index, не вторым обходом.
Ignore match не скрывает tracked-path error.
`GeneratedPathTracked` создаётся для cache, `.build` и `DumpFilesIndex.txt`.
Любой `ConfigDumpInfo.xml` исключён из generic path fact: его staged blob даёт
ровно `RuntimeSidecarTracked`, no fact для доказанного descriptor либо
`ConfigDumpInfoUnclassified`.

- [ ] **Step 6: Implement staged CDFI classification and compatibility bridge.**

Для каждого case-insensitive basename `ConfigDumpInfo.xml` вызвать с remaining
deadline:

```text
git --no-replace-objects cat-file blob <oid>
```

Runtime root → `RuntimeSidecarTracked`; ExternalProcessor/ExternalReport/
MetadataDescriptor → no fact; Other или missing OID →
`ConfigDumpInfoUnclassified`. Timeout, truncated/lossy output и process failure
делают `repository.config_dump_info` `NotRun` с typed
`GitInspectionTimeout|GitInspectionIncomplete`, а не содержательной
диагностикой blob. Blob cache keyed by OID запрещает повторное чтение.

До Task 6 старый `GitTrackingAdapter` должен делегировать CDFI snapshot новому
helper и только рендерить compatibility warning. Удалить из
`internal_adapters.rs` старые parsers/classifier loop и перенести их tests;
два независимых классификатора не оставлять.

- [ ] **Step 7: Run GREEN and commit.**

```bash
cargo test -p unica-coder project_health_git --lib -- --test-threads=1
cargo test -p unica-coder config_dump_info --lib -- --test-threads=1
cargo fmt --all
git diff --check
git add crates/unica-coder/src/infrastructure/project_health.rs crates/unica-coder/src/infrastructure/project_health/git.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "feat(project): inspect portable git state"
```

Expected: typed Git tests и временный compatibility adapter GREEN; source of
truth для staged CDFI один.

---

## Task 5: Проверить resource attributes, EOL и LFS

**Files:**

- Create: `crates/unica-coder/src/infrastructure/project_health/resources.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health/git.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health.rs`

**Interfaces:**

- Consumes: inspected Platform XML roots и `GitIndexEntry` из Tasks 2/4.
- Produces:

```rust
pub(crate) const LFS_SINGLE_FILE_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const LFS_AGGREGATE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
pub(crate) const MAX_WORKING_EOL_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_WORKING_EOL_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryResourceKind { Text, Binary }

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryResource {
    pub(crate) source_set: String,
    pub(crate) repo_path: String,
    pub(crate) worktree_path: PathBuf,
    pub(crate) kind: RepositoryResourceKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceOwnershipError {
    pub(crate) repo_path: String,
    pub(crate) source_sets: Vec<String>,
}

pub(crate) struct RepositoryPolicyInspection {
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct SourceResourcePolicyInspector<'a> {
    runner: &'a dyn ProcessRunner,
}

impl<'a> SourceResourcePolicyInspector<'a> {
    pub(crate) fn new() -> Self;

    #[cfg(test)]
    pub(crate) fn with_runner(runner: &'a dyn ProcessRunner) -> Self;

    pub(crate) fn classify(
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
    ) -> Result<Vec<RepositoryResource>, ResourceOwnershipError>;

    pub(crate) fn inspect(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError>;
}
```

Path role registry первого профиля:

```rust
fn classify_platform_xml_relative_path(
    normalized_relative_path: &str,
) -> Option<RepositoryResourceKind> {
  match normalized_relative_path {
    path if is_xdto_package_bin(path) => Some(RepositoryResourceKind::Text),
    path if has_extension(path, &["xml", "bsl"]) => Some(RepositoryResourceKind::Text),
    path if has_extension(path, &[
        "bin", "axdt", "addin", "cf", "cfe", "epf", "erf",
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "zip", "7z", "gz",
    ]) => Some(RepositoryResourceKind::Binary),
    _ => None,
  }
}
```

`is_xdto_package_bin` требует exact relative segments
`XDTOPackages/<one-name>/Ext/Package.bin`; prefix/suffix containment не
принимаются.

EDT и Unknown/Invalid не получают format-specific resource candidates.
Каждый `InspectedSourceRoot.path` сначала переводится в repo-relative prefix
через тот же host-aware containment policy; index entry принадлежит уникальному
самому глубокому prefix. Неоднозначные equal-depth roots дают отдельные
`GitInspectionIncomplete` для attributes/index-EOL/working-EOL/LFS observations,
а не случайного владельца. `classify` возвращает sorted
`ResourceOwnershipError`; compositor преобразует его в эти четыре facts.

- [ ] **Step 1: Write failing role classification tests.**

Tests обязаны доказать:

```rust
assert_eq!(
    classify_platform_xml_relative_path("XDTOPackages/Sales/Ext/Package.bin"),
    Some(RepositoryResourceKind::Text),
);
assert_eq!(
    classify_platform_xml_relative_path("Templates/Blob/Ext/Template.bin"),
    Some(RepositoryResourceKind::Binary),
);
assert_eq!(classify_platform_xml_relative_path("unknown.dat"), None);
```

Добавить exact containment tests: похожий
`Other/XDTOPackages/Sales/Ext/Package.bin` вне канонического root shape не
получает text exception; path другого source set не присваивается соседу;
самый глубокий root побеждает однозначно.

- [ ] **Step 2: Write failing attributes/EOL tests with real Git.**

Матрица:

- `*.xml text`, `*.xml text=auto`, `*.xml eol=crlf` — text policy satisfied;
- unspecified text — `git.text_policy_missing`;
- `*.bin -text` — binary policy satisfied;
- broad `*.bin binary` + no later XDTO override —
  `git.text_resource_marked_binary` only;
- later `XDTOPackages/**/Ext/Package.bin text` repairs the conflict;
- index `lf|none` and worktree `lf|crlf|none` pass;
- index `crlf|mixed` → `git.index_eol_not_lf`;
- worktree `mixed` → `git.mixed_eol`;
- worktree uniform `cr` → `git.working_eol_unsupported`;
- working text больше per-file/total budget, read error и подмена symlink →
  `git.inspection_incomplete` и EOL check `notRun`;
- nested `.gitattributes` override and Unicode/newline path;
- attribute только из `.git/info/attributes` не считается portable; literal
  tracked `U+FFFD` path остаётся валидным;
- missing/invalid/truncated check-attr or EOL output →
  `git.inspection_incomplete`, never passed.

- [ ] **Step 3: Run resource policy tests and witness RED.**

```bash
cargo test -p unica-coder project_health_repository_policy --lib -- --test-threads=1
```

Expected: FAIL из-за отсутствующих classifier/attribute/EOL parsers.

- [ ] **Step 4: Implement effective attributes collection.**

Для portable policy прочитать effective attributes из staged index:

```text
git check-attr -z --cached text eol filter --stdin
```

Для local-only policy создать временный пустой index совместимыми с Git 2.40
plumbing-командами и передать тем же путям controlled `GIT_INDEX_FILE`:

```text
git read-tree --empty
git check-attr -z --cached text eol filter --stdin
```

Первый результат содержит staged tracked `.gitattributes` вместе с local,
global и system policy. Пустой alternate index исключает tracked policy и
изолирует непереносимые источники без Git-2.43-only `check-attr --source`.
Если local-only результат для кандидата не `unspecified`, публиковать
`AttributesLocalOnly` независимо от совпадения effective value и не принимать
policy как portable. Working-only `.gitattributes` исключает `--cached`.

Parser читает triples `path\0attribute\0value\0` и требует ровно по одному
значению каждого requested attribute на каждый path в обоих результатах. Text policy satisfied,
если `text` равен `set|auto` или `eol` равен `lf|crlf`, и не satisfied при
`text=unset`. Binary policy satisfied только при `text=unset`.

Primary suppression:

- known text + `text=unset` → только `TextResourceMarkedBinary`;
- local-only attribute → только `AttributesLocalOnly` для затронутого path,
  остальные policy/EOL следствия этого path подавляются;
- missing text policy не дублировать `IndexEolNotLf` до исправления policy;
- unknown role не получает attributes diagnostic.

- [ ] **Step 5: Implement index/worktree EOL parsing.**

Один Git-вызов даёт index EOL и связывает path с worktree entry:

```text
git ls-files --eol -z
```

Разобрать `i/<kind> w/<kind> attr/<summary>\t<path>\0`, сопоставить только
classified text paths. `i/lf|i/none` проходят;
`i/crlf|i/mixed|i/-text` дают `IndexEolNotLf` с observed value, а неизвестный
kind — `GitInspectionIncomplete`. Не ожидать несуществующего Git-состояния
`w/cr`.

Для существующего regular worktree-файла выполнить потоковый read блоками по
64 KiB с проверкой cancellation/deadline между блоками и общими пределами
`MAX_WORKING_EOL_*`. State machine различает `LF`, `CRLF` и bare `CR`, включая
`CR` на границе блоков: один стиль `LF|CRLF` или отсутствие terminator проходят;
несколько стилей дают `MixedEol`; только bare `CR` даёт
`WorkingEolUnsupported`. Отсутствующий worktree-файл не имеет working EOL и не
мешает проверке index; symlink/reparse, directory, overflow и I/O error дают
`GitInspectionIncomplete` и делают EOL check `notRun`. Файл не переписывается.

- [ ] **Step 6: Implement advisory LFS aggregation.**

Для classified binary regular files открыть exact repository-relative path
component-wise no-follow и взять размер по metadata уже открытого handle.
`filter=lfs` исключает файл из рекомендации.
Публиковать один `LfsConsider`, когда любой файл ≥10 MiB или сумма ≥100 MiB;
fact несёт full count/total/largest и полный sorted paths, а доменный public
aggregator из Task 1 обрезает только `diagnostics[].paths` до 20. Commands в
remediation пусты; steps предлагают выбрать exact paths и повторить status, не
предлагают `*.bin`.

- [ ] **Step 7: Run GREEN and commit.**

```bash
cargo test -p unica-coder project_health_repository_policy --lib -- --test-threads=1
cargo test -p unica-coder project_health_git --lib -- --test-threads=1
cargo fmt --all
git diff --check
git add crates/unica-coder/src/infrastructure/project_health.rs crates/unica-coder/src/infrastructure/project_health/git.rs crates/unica-coder/src/infrastructure/project_health/resources.rs
git commit -m "feat(project): evaluate repository resource policy"
```

Expected: Platform XML policy GREEN; EDT tests доказывают `notApplicable`, LFS
никогда не меняет readiness.

---

## Task 6: Подключить coordinator и публичный `project.status`

**Files:**

- Create: `crates/unica-coder/src/application/project_health.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `crates/unica-coder/src/infrastructure/project_health.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Create: `crates/unica-coder/tests/platform_project_health.rs`
- Modify: `tests/ci/test_unica_mcp_smoke.py`

**Interfaces:**

- Consumes: complete snapshot/evaluator from Tasks 1–5.
- Produces:

```rust
pub(crate) trait ApplicationPorts: Send + Sync {
    fn inspect_project_health(
        &self,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError>;
}

pub(crate) fn inspect_project_health(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError>;

#[cfg(test)]
pub(crate) fn inspect_project_health_with_runner(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    runner: &dyn ProcessRunner,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError>;

pub(crate) struct ProjectHealthCoordinator<'a> {
    ports: &'a dyn ApplicationPorts,
}

impl<'a> ProjectHealthCoordinator<'a> {
    pub(crate) fn invoke(
        &self,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<HandlerOutcome, String>;
}
```

`InfrastructureApplicationPorts::inspect_project_health` только вызывает
`infrastructure::project_health::inspect_project_health`; Git остаётся за port
boundary.

Infrastructure compositor выполняет ровно один `SourceLayoutInspector`, затем
один `GitRepositoryInspector::inspect_base`, затем — только при доказанном Git
root/index — один `SourceResourcePolicyInspector::inspect`. Все получают один
`ProviderDeadline` и cancellation token. После каждого phase cancellation даёт
`Cancelled`; нулевой remaining deadline превращается в typed timeout facts и
`NotRun` оставшихся checks, если снимок уже достоверен. Fatal зарезервирован для
невозможности сформировать внутренне согласованный snapshot. Compositor
объединяет source sets, `source_targets_complete`, repository root,
observations/facts и не выполняет evaluator.

- [ ] **Step 1: Write failing application coordinator tests.**

Fake port возвращает три состояния:

```rust
Ok(snapshot_with_problem)
Err(ProjectHealthInspectionError::Cancelled)
Err(ProjectHealthInspectionError::Fatal("snapshot invariant failed".into()))
```

Assertions:

- complete problem → adapter `ok=true`, data present, warnings/errors/stdout
  empty;
- cancelled → standard cancelled adapter, `ok=false`, data absent;
- fatal → `ok=false`, one error, data absent;
- map dispatch never calls `inspect_project_health`.

- [ ] **Step 2: Write failing public Rust tests.**

Обновить старые project status/map tests и добавить:

```rust
#[test]
fn project_without_git_can_be_source_ready_but_not_repository_ready() {
    let fixture = platform_xml_workspace("src");
    let result = call_status(&fixture);

    assert!(result.ok);
    assert_eq!(result.data.as_ref().unwrap()["ready"], true);
    assert_eq!(result.data.as_ref().unwrap()["repositoryReady"], false);
    assert_has_code(&result, "git.repository_absent");
}

#[test]
fn workspace_root_source_set_is_reported_without_running_a_subject_tool() {
    let fixture = platform_xml_workspace(".");
    let result = call_status(&fixture);

    assert!(result.ok);
    assert_eq!(result.data.as_ref().unwrap()["ready"], false);
    assert_has_code(&result, "source_set.root_is_workspace");
}
```

Добавить full-ready Git fixture с tracked `.gitignore`, `.gitattributes`, LF
index и `src` root; malformed map; no source sets; tracked runtime sidecar;
ambiguous sidecar without command; deterministic diagnostic order; status does
not create `.build/unica/services`; status does not modify a byte snapshot.
`full-ready` означает отсутствие error/notRun, поэтому optional LFS `info` не
мешает `ready=true` и `repositoryReady=true`; отсутствие findings не является
отдельным требованием.

В `platform_project_health.rs` добавить real parent-repo, linked-worktree и
cfg-aware symlink/reparse public tests. Parent-repo fixture отдельно проверяет,
что remediation `cwd` равен repository root, а argv содержит путь с префиксом
workspace-подкаталога. Ни один test не помечать ignored.

- [ ] **Step 3: Run public tests and witness RED.**

```bash
cargo test -p unica-coder project_status --lib -- --test-threads=1
cargo test -p unica-coder --test platform_project_health -- --test-threads=1
```

Expected: FAIL — status ещё имеет старую форму, map всё ещё публикует Git
warning, platform test file ещё не подключён.

- [ ] **Step 4: Implement the application coordinator and port.**

Добавить application special dispatch рядом с metadata/source coordinators:

```rust
ToolHandler::ProjectStatus => ProjectHealthCoordinator::new(ports).invoke(
    &context,
    cancellation,
    deadline,
)?,
```

Complete snapshot сериализуется через `evaluate_project_health`; summary равен
`project health inspected: ready=<bool>; repositoryReady=<bool>`, artifacts
содержат workspace/cache roots. Found diagnostics не копируются в envelope
warnings/errors.

Coordinator total-match-ит результат port-а: `Cancelled` →
`HandlerOutcome::plain(AdapterOutcome::cancelled("project health inspection"))`;
`Fatal(reason)` и `evaluate_project_health(snapshot)=Err(reason)` → plain
`AdapterOutcome { ok: false, summary: "project health inspection failed",
errors: ["project_health_inspection_failed: <reason>"], ...empty }`. Эти случаи
не возвращают Rust `Err` и не получают `data`; Rust `Err` остаётся только для
ошибки сериализации, которая для derived `Serialize` считается invariant panic
через `expect`, как у существующих typed readers.

Удалить временные `#[allow(dead_code)]` у project_health modules.

- [ ] **Step 5: Make `project.map` pure and remove compatibility adapter.**

Из `InfrastructureApplicationPorts` удалить Git вызов в обоих arms:

- `ProjectStatus` больше не обслуживается generic infrastructure handler и
  возвращает guard error при прямом обходе coordinator;
- `ProjectMap` вызывает только `discover_project_source_map` и
  `project_map(source_map)`.

Удалить `GitTrackingAdapter`, `ConfigDumpInfoGitCheck`, compatibility renderer
и оставшиеся string tests из `internal_adapters.rs`. `config_dump_info_xml_kind`
остаётся в domain, typed Git inspector — единственный consumer Git index.

Обновить description status на:

```rust
"Inspect typed workspace, source-set, and portable Git readiness without changing the project."
```

- [ ] **Step 6: Add MCP consumer tests.**

В `interfaces/mcp.rs` и `tests/ci/test_unica_mcp_smoke.py` проверить:

- `data` содержит ровно `workspaceRoot`, `cacheRoot`, `ready`,
  `repositoryReady`, `checks`, `sourceSets`, `diagnostics`;
- project issue сохраняет JSON-RPC success и `isError=false`;
- no Git разделяет два флага;
- full-ready fixture даёт оба `true`;
- content text равен `structuredContent` там, где host публикует оба;
- `project.status` по-прежнему не публикует `outputSchema` и не принимает
  `dryRun`;
- вызов не меняет workspace snapshot.

- [ ] **Step 7: Run GREEN and commit.**

```bash
cargo test -p unica-coder project_health --lib -- --test-threads=1
cargo test -p unica-coder project_status --lib -- --test-threads=1
cargo test -p unica-coder --test platform_project_health -- --test-threads=1
python3.12 -m unittest tests.ci.test_unica_mcp_smoke
cargo fmt --all
git diff --check
git add crates/unica-coder/src/application/project_health.rs crates/unica-coder/src/application/mod.rs crates/unica-coder/src/application/ports.rs crates/unica-coder/src/infrastructure/application_ports.rs crates/unica-coder/src/infrastructure/internal_adapters.rs crates/unica-coder/src/infrastructure/project_health.rs crates/unica-coder/src/interfaces/mcp.rs crates/unica-coder/tests/platform_project_health.rs tests/ci/test_unica_mcp_smoke.py
git commit -m "feat(project): publish typed project readiness"
```

Перед `git add` проверить `git diff --name-only`: не stage-ить unrelated файлы;
команда выше перечисляет только paths из секции Files, изменяемые этим task.

Expected: public typed contract GREEN; `project.map` не запускает Git; no hidden
service/write regressions.

---

## Task 7: Принять ADR и синхронизировать архитектуру и AI guidance

**Files:**

- Create: `tests/ci/test_project_health_contract.py`
- Modify: `spec/decisions/0056-project-status-publikuet-gotovnost-proekta.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/change-checklist.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/concepts.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `spec/architecture/tool-surface-review.json`
- Regenerate: `spec/architecture/tool-surface.md`
- Modify: `spec/acceptance/unica-mcp-validation.md`
- Modify: `plugins/unica/skills/v8-runner/SKILL.md`
- Modify: `plugins/unica/references/use-cases/workspace-runtime.md`
- Modify: `tests/ci/test_unica_skills.py`

**Interfaces:**

- Consumes: GREEN public behavior from Task 6.
- Produces: accepted ADR-0056 and three executable rules:

```text
INV-MCP-PROJECT-READINESS
INV-SOURCE-ROOT-SEPARATION
INV-SOURCE-PORTABLE-GIT
```

- [ ] **Step 1: Write the failing cross-document contract test.**

`tests/ci/test_project_health_contract.py` должен проверить:

```python
def test_project_health_contract_is_accepted_and_routed(self) -> None:
    adr = self.read("spec/decisions/0056-project-status-publikuet-gotovnost-proekta.md")
    invariants = self.read("spec/architecture/invariants.md")
    review = json.loads(self.read("spec/architecture/tool-surface-review.json"))
    workflow = self.read("plugins/unica/references/use-cases/workspace-runtime.md")

    self.assertIn("- Статус: `accepted`", adr)
    for invariant in (
        "INV-MCP-PROJECT-READINESS",
        "INV-SOURCE-ROOT-SEPARATION",
        "INV-SOURCE-PORTABLE-GIT",
    ):
        self.assertIn(f"### {invariant}", invariants)
    self.assertIn("ready", review["unica.project.status"]["result"]["now"])
    self.assertIn("repositoryReady", review["unica.project.status"]["result"]["now"])
    self.assertIn("unica.project.status", workflow)
    self.assertIn("remediation", workflow)
```

Добавить assertions, что ADR-0056 находится в accepted section README и
отсутствует в proposed section, а `project.map` review не обещает health.

- [ ] **Step 2: Run the contract test and witness RED.**

```bash
python3.12 -m unittest tests.ci.test_project_health_contract
```

Expected: FAIL на `proposed`/missing invariants, не на import/path error.

- [ ] **Step 3: Add the three invariant records and current architecture.**

Нормативные `Rule` формулировать по-русски и ссылать на ADR-0056:

- `INV-MCP-PROJECT-READINESS`: status typed data, два флага, checks,
  diagnostics, `ok=true` для findings; map не делает health check;
- `INV-SOURCE-ROOT-SEPARATION`: status считает равный workspace source root
  source-error и не смешивает его производные layout facts;
- `INV-SOURCE-PORTABLE-GIT`: repository readiness отдельно от source readiness,
  tracked ignore, role-aware attributes/EOL, staged CDFI и advisory LFS.

Каждый record называет реальные Rust/Python checks из Tasks 1–7. Обновить
change checklist по ID, а building-blocks/concepts/runtime — только ссылками на
владельцев, без второго нормативного текста.

- [ ] **Step 4: Accept ADR and update acceptance.**

Перевести ADR-0056 `proposed → accepted`, переместить ровно одну ссылку в
decision README. В `unica-mcp-validation.md` добавить executable matrix:

1. no Git → ready true/repository false;
2. path `.` → ready false;
3. missing/local-only ignore;
4. staged runtime vs legitimate CDFI;
5. XDTO Package.bin override;
6. LF index + CRLF worktree pass, mixed/CR fail;
7. cancellation/truncation no partial pass;
8. no writes/hidden services.

- [ ] **Step 5: Update tool ledger and regenerate the surface.**

В `tool-surface-review.json` заменить current result status на typed health
fields и добавить AI remediation scenario; `project.map` оставить map-only.
Затем:

```bash
python3.12 scripts/ci/generate-tool-surface.py
```

Expected: меняется status section `tool-surface.md`, tool count/name/input
surface не меняются.

- [ ] **Step 6: Teach the packaged AI workflow without granting mutation.**

`workspace-runtime.md` и `v8-runner/SKILL.md` должны говорить:

1. после clone/init и перед build/dump вызвать `unica.project.status`;
2. `ready=false` блокирует source operation до исправления;
3. `repositoryReady=false` не означает, что Unica без Git не работает, но
   блокирует утверждение о переносимой командной готовности;
4. следовать `diagnostics[].remediation.steps`;
5. `commands[]` — объяснение, а не автоматическое разрешение менять index;
6. после одобренного исправления повторить status.

Расширить `test_unica_skills.py`, чтобы reference оставался reachable и содержал
`unica.project.status`, `ready`, `repositoryReady`, `remediation` и запрет
автовыполнения.

- [ ] **Step 7: Run documentation/architecture GREEN and commit.**

```bash
python3.12 -m unittest tests.ci.test_project_health_contract tests.ci.test_design_documents tests.ci.test_architecture_registry tests.ci.test_unica_skills
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
git diff --check
git add tests/ci/test_project_health_contract.py tests/ci/test_unica_skills.py spec/decisions/0056-project-status-publikuet-gotovnost-proekta.md spec/decisions/README.md spec/architecture/invariants.md spec/architecture/change-checklist.md spec/architecture/building-blocks.md spec/architecture/concepts.md spec/architecture/runtime.md spec/architecture/tool-surface-review.json spec/architecture/tool-surface.md spec/acceptance/unica-mcp-validation.md plugins/unica/skills/v8-runner/SKILL.md plugins/unica/references/use-cases/workspace-runtime.md
git commit -m "docs(project): accept project health contract"
```

Expected: ADR/invariant/index guards GREEN; generated tool surface clean;
accepted docs describe delivered code, not future behavior.

---

## Final Verification

- [ ] **Step 1: Run formatting, lint and all Rust tests.**

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets -- -D warnings
cargo test -p unica-coder -- --test-threads=1
```

Expected: all commands exit 0, no ignored `platform_project_health` tests.

- [ ] **Step 2: Run focused and full Python contracts.**

```bash
python3.12 -m unittest tests.ci.test_project_health_contract tests.ci.test_unica_mcp_smoke tests.ci.test_design_documents tests.ci.test_architecture_registry tests.ci.test_unica_skills
python3.12 -m unittest discover -s tests/ci
```

Expected: focused and full suites PASS.

- [ ] **Step 3: Re-run architecture and MCP smoke gates.**

```bash
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
python3.12 scripts/ci/smoke-unica-mcp.py --binary target/debug/unica --plugin-root .
```

Expected: accepted ADR/invariants/public payload are synchronized; packaged
smoke retains the existing source flow, while `test_unica_mcp_smoke` above
proves the read-only `project.status` call and typed payload.

- [ ] **Step 4: Verify scope and history.**

```bash
git diff --check origin/main...HEAD
git status --short --branch
git log --oneline --decorate origin/main..HEAD
git diff --stat origin/main...HEAD
```

Expected: clean branch, семь review commits после planning artifacts, no
tools.lock/package/version change, no PR #473 implementation copied.

- [ ] **Step 5: Request final code review before push/PR.**

Use `superpowers:requesting-code-review`. Review must explicitly verify:

- no Git dependency leaked into subject tools;
- no auto remediation or shell command string;
- no lossy/truncated observation becomes passed;
- CDFI has one classifier and staged-blob semantics;
- path `.` reports the root cause while independent Git findings remain;
- ADR-0056 is accepted only with all checks GREEN.
