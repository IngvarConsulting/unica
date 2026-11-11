//! Сроки анализа кода, разрешённые из настроек рабочего пространства.
//!
//! Значения без логики: координаторы анализа читают их отсюда.

use std::time::Duration;

// resource-limit: search.total-timeout-default
// class: deadline
// unit: s
// scope: request
// value: 300
// resource: Общий срок поиска кода по умолчанию, если настройки рабочего пространства его не задают.
// status: unproven
// origin: #398
// problem: неизвестно
// on-reach: срок задаёт публичный бюджет поиска и его прогресс; заданное в настройках значение заменяет умолчание
const SEARCH_TOTAL_DEFAULT_SECONDS: u64 = 300;
// resource-limit: search.semantic-timeout-default
// class: deadline
// unit: s
// scope: request
// value: 300
// resource: Срок поставщика смыслового поиска по умолчанию.
// status: unproven
// origin: #398
// problem: неизвестно
// on-reach: поставщик отменяется, его раздел помечается timed_out, остальные разделы поиска отдаются
const SEARCH_SEMANTIC_DEFAULT_SECONDS: u64 = 300;
// resource-limit: search.git-grep-timeout-default
// class: deadline
// unit: s
// scope: request
// value: 30
// resource: Срок поставщика git grep прежнего unica.code.search (поверхность v0.12, с провода v0.13 недостижима) по умолчанию. Буквальный поиск без роли этот срок не читает: он идёт без срока операции. В файлах настроек срок не задаётся.
// status: unproven
// origin: #398
// problem: Срок 2 с короче полного прохода git grep по конфигурации масштаба УТ: лексический поиск редкой строки по всей конфигурации и глубокие страницы отвечали timed_out без совпадений.
// on-reach: поставщик отменяется, его раздел помечается timed_out, остальные разделы поиска отдаются
// issue: #2080
// evidence: Замер 25.10.2026 (Apple M3 Max, 14 ядер, macOS, git 2.54, релизная сборка ветки поверх main 07306a173): команда поставщика (-F, --untracked, :(glob)**/*.[bB][sS][lL]) на отсутствующей строке, пять проходов подряд: выгрузка УТ 11 (48 159 файлов в индексе Git, 12 290 модулей BSL на 508 МБ, 31 009 XML на 692 МБ) — 5,67 с первый проход, затем 2,52–2,94 с; синтетический корпус формы УТ из scripts/dev/synthetic-configuration.py (10 757 модулей BSL на 510 МБ, 14 546 XML на 69 МБ) — 2,90 с первый (мог совпасть с фоновым обслуживанием Git после генерации), затем 1,59–1,61 с. Кеш ФС не сбрасывался.
// evidence: Публичный unica.search role=lexical (scripts/dev/measure-capacity.py, два прохода подряд, наблюдение ёмкости демона (--capacity-report)): со сроком 2 с на синтетическом корпусе — 2,5–5,0 с и «lexical search returned incomplete results» на редкой и на отсутствующей строке; со сроком 30 с — «completed» за 2,2–2,8 с на синтетическом корпусе и за 3,9–9,6 с на выгрузке УТ 11.
// evidence: 30 с — около пятикратного запаса над худшим первым проходом по УТ на этой машине; общий срок поиска по умолчанию 300 с остаётся старшим. Не замерены Windows (NTFS, антивирус), медленные диски, работа под нагрузкой и БП. Частая строка останавливается раньше, на пределе совпадений; страница после первой снова проходит корпус с начала (#2079) и на медленной машине может ждать до срока. Предел временный до снятия сроков поиска (#1254).
// review: #1119 PR-B: роль lexical слита с буквальным поиском без роли; срок остаётся только у прежнего code.search и снимается вместе с ним
const SEARCH_GIT_GREP_DEFAULT_SECONDS: u64 = 30;
// resource-limit: search.provider-read-timeout-default
// class: deadline
// unit: s
// scope: request
// value: 45
// resource: Срок одного чтения у поставщика анализа кода (например, графа вызовов) по умолчанию.
// status: unproven
// origin: #398
// problem: неизвестно
// on-reach: чтение поставщика прерывается по сроку; ответ без его данных
const PROVIDER_READ_DEFAULT_SECONDS: u64 = 45;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeIntelligenceDeadlines {
    search_total_timeout: Duration,
    search_semantic_timeout: Duration,
    search_git_grep_timeout: Duration,
    provider_read_timeout: Duration,
}

impl CodeIntelligenceDeadlines {
    /// Values already resolved and validated by the configuration owner.
    pub const fn from_timeouts(
        search_total_timeout: Duration,
        search_semantic_timeout: Duration,
        search_git_grep_timeout: Duration,
        provider_read_timeout: Duration,
    ) -> Self {
        Self {
            search_total_timeout,
            search_semantic_timeout,
            search_git_grep_timeout,
            provider_read_timeout,
        }
    }

    pub const fn compiled_defaults() -> Self {
        Self {
            search_total_timeout: Duration::from_secs(SEARCH_TOTAL_DEFAULT_SECONDS),
            search_semantic_timeout: Duration::from_secs(SEARCH_SEMANTIC_DEFAULT_SECONDS),
            search_git_grep_timeout: Duration::from_secs(SEARCH_GIT_GREP_DEFAULT_SECONDS),
            provider_read_timeout: Duration::from_secs(PROVIDER_READ_DEFAULT_SECONDS),
        }
    }

    pub const fn search_total_timeout(self) -> Duration {
        self.search_total_timeout
    }

    pub const fn search_semantic_timeout(self) -> Duration {
        self.search_semantic_timeout
    }

    pub const fn search_git_grep_timeout(self) -> Duration {
        self.search_git_grep_timeout
    }

    pub const fn provider_read_timeout(self) -> Duration {
        self.provider_read_timeout
    }

    pub fn within_search_budget(mut self, remaining: Duration) -> Self {
        self.search_total_timeout = self.search_total_timeout.min(remaining);
        self.search_git_grep_timeout = self.search_git_grep_timeout.min(remaining);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn for_test(timeout: Duration) -> Self {
        Self {
            search_total_timeout: timeout,
            search_semantic_timeout: timeout,
            search_git_grep_timeout: timeout,
            provider_read_timeout: timeout,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn for_test_values(
        search_total_timeout: Duration,
        search_semantic_timeout: Duration,
        search_git_grep_timeout: Duration,
        provider_read_timeout: Duration,
    ) -> Self {
        Self {
            search_total_timeout,
            search_semantic_timeout,
            search_git_grep_timeout,
            provider_read_timeout,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeDiagnosticsDeadlines {
    analyze_timeout: Option<Duration>,
}

impl CodeDiagnosticsDeadlines {
    /// A value already resolved and validated by the configuration owner.
    pub const fn from_timeout(analyze_timeout: Option<Duration>) -> Self {
        Self { analyze_timeout }
    }

    pub const fn compiled_defaults() -> Self {
        Self {
            analyze_timeout: None,
        }
    }

    /// `None` — пользователь срок анализа не задавал, и анализ идёт до конца.
    pub const fn analyze_timeout(self) -> Option<Duration> {
        self.analyze_timeout
    }
}
