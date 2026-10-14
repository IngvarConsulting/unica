//! Сколько байтов ответа инструмента хост гарантированно передаёт модели
//! целиком, без усечения и без выноса в файл.
//!
//! Бюджет относится ко всему ответу: ко всем текстовым блокам вместе.
//! Известные хосты получают наибольший бюджет, который они надёжно передают;
//! остальные — умолчание.

use std::ffi::OsString;

/// Бюджет по умолчанию — для хостов, чьи пределы не измерены.
pub const DEFAULT_RESPONSE_BUDGET: usize = 16 * 1024;

/// Codex по умолчанию даёт ответу MCP-инструмента 10 000 токенов политики
/// усечения модели и 20 % надбавки на сериализацию, то есть 12 000 токенов.
/// Токены он не считает токенизатором: каждый текстовый блок стоит
/// `ceil(байты / 4)`, итого 48 000 байт; сверх этого он выбрасывает середину
/// (`codex-rs/utils/output-truncation`, `with_serialization_allowance`,
/// `truncate_function_output_items_with_policy`; `models.json`, 14.10.2026).
/// Перед ответом Codex ставит строку о времени вызова (около 40 байт), каждый
/// блок округляется вверх. 46 КиБ оставляют 896 байт на это и на запас.
const CODEX_RESPONSE_BUDGET: usize = 46 * 1024;

/// Claude Code переносит в файл текстовый ответ длиннее 50 000 символов, если
/// инструмент не объявил свой предел `anthropic/maxResultSizeChars`; с
/// объявленным пределом (до 500 000 символов) он не применяет и предел
/// `MAX_MCP_OUTPUT_TOKENS` в 25 000 токенов. Unica объявляет 100 000 символов
/// у `unica.view` ([`source_text_tool_meta`]). Бюджет держит ответ в тех же
/// 25 000 токенов, что хост задаёт по умолчанию: замер 15.10.2026 на трёх
/// модулях УТ показал 3,02–3,27 байта UTF-8 на токен, 72 КиБ — не больше
/// 24 400 токенов. Символов в нём не больше байтов, то есть меньше 100 000.
/// Тот же замер: 80 КБ без объявления доходят до модели целиком, 92 КБ уходят
/// в файл; с объявлением 500 000 символов целиком доходят 800 КБ.
const CLAUDE_RESPONSE_BUDGET: usize = 72 * 1024;

/// Предел символов текстового ответа, который объявляют `unica.view`
/// и `unica.task.result`. Он относится к любому их ответу, не только
/// к тексту `Body`: страницы коллекций у этого хоста тоже доходят до модели
/// целиком до 100 000 символов.
const CLAUDE_RESULT_SIZE_CHARS: u64 = 100_000;

/// Наибольший бюджет среди известных хостов: больший текст не отдаётся никому.
pub const MAX_RESPONSE_BUDGET: usize = CLAUDE_RESPONSE_BUDGET;

/// Явный бюджет в байтах: настройка пользователя для любого хоста.
const BUDGET_ENV: &str = "UNICA_RESPONSE_BUDGET";

/// Переменные, которые хост выставляет процессу сервера своего плагина.
/// ZCode повторяет переменные Claude Code, поэтому проверяется первым.
const ZCODE_LAUNCH_ENV: &str = "ZCODE_PLUGIN_DATA";
const CLAUDE_LAUNCH_ENV: &str = "CLAUDE_PLUGIN_DATA";

/// `clientInfo.name` из `initialize`.
const CODEX_CLIENT: &str = "codex-mcp-client";
const CLAUDE_CLIENT: &str = "claude-code";

/// Бюджет ответа одного клиента в байтах UTF-8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponseBudget(usize);

impl ResponseBudget {
    pub const fn bytes(self) -> usize {
        self.0
    }

    pub const fn from_bytes(bytes: usize) -> Self {
        Self(bytes)
    }
}

impl Default for ResponseBudget {
    fn default() -> Self {
        Self(DEFAULT_RESPONSE_BUDGET)
    }
}

/// Бюджет для клиента, назвавшегося `client_name` в `initialize`.
///
/// Порядок: явная настройка `UNICA_RESPONSE_BUDGET`; имя клиента из
/// `initialize` — оно называет сам собеседник, а не окружение, унаследованное
/// от запустившего процесса, и неизвестное имя даёт умолчание; только без
/// имени — переменные, которыми хост запускает сервер своего плагина;
/// умолчание. Единый `.mcp.json` для всех хостов не может
/// назвать хост сам.
pub fn host_response_budget(client_name: Option<&str>) -> ResponseBudget {
    resolve_response_budget(client_name, &|name| std::env::var_os(name))
}

/// Объявление предела ответа для инструмента, отдающего исходный текст.
pub fn source_text_tool_meta() -> (&'static str, u64) {
    ("anthropic/maxResultSizeChars", CLAUDE_RESULT_SIZE_CHARS)
}

/// Клиенты с измеренным бюджетом: имя из `initialize` и бюджет в байтах.
/// Для проверок по проводу, которым нельзя называть хосты самим.
pub fn known_client_budgets() -> [(&'static str, usize); 2] {
    [
        (CODEX_CLIENT, CODEX_RESPONSE_BUDGET),
        (CLAUDE_CLIENT, CLAUDE_RESPONSE_BUDGET),
    ]
}

/// Переменные окружения, от которых зависит бюджет: проверкам, которые
/// запускают сервер, нужно их убрать, чтобы не зависеть от окружения.
pub fn response_budget_environment_keys() -> [&'static str; 3] {
    [BUDGET_ENV, ZCODE_LAUNCH_ENV, CLAUDE_LAUNCH_ENV]
}

/// Негодная настройка не применяется, но и не молчит: одно сообщение
/// в stderr за процесс.
fn warn_invalid_budget(value: &OsString) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        eprintln!(
            "unica: {BUDGET_ENV}={value:?} is ignored: expected bytes from 1024 to {MAX_RESPONSE_BUDGET}"
        );
    });
}

fn resolve_response_budget(
    client_name: Option<&str>,
    read_env: &dyn Fn(&str) -> Option<OsString>,
) -> ResponseBudget {
    if let Some(value) = read_env(BUDGET_ENV) {
        match value
            .to_str()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|bytes| (1024..=MAX_RESPONSE_BUDGET).contains(bytes))
        {
            Some(bytes) => return ResponseBudget(bytes),
            None => warn_invalid_budget(&value),
        }
    }
    // Названный клиент решает сам: неизвестное имя — умолчание, даже если
    // окружение унаследовано от процесса другого хоста.
    if let Some(name) = client_name {
        return match name {
            CODEX_CLIENT => ResponseBudget(CODEX_RESPONSE_BUDGET),
            CLAUDE_CLIENT => ResponseBudget(CLAUDE_RESPONSE_BUDGET),
            _ => ResponseBudget::default(),
        };
    }
    if read_env(ZCODE_LAUNCH_ENV).is_some() {
        return ResponseBudget::default();
    }
    if read_env(CLAUDE_LAUNCH_ENV).is_some() {
        return ResponseBudget(CLAUDE_RESPONSE_BUDGET);
    }
    ResponseBudget::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let entries = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(*value)))
            .collect::<std::collections::BTreeMap<_, _>>();
        move |name: &str| entries.get(name).cloned()
    }

    #[test]
    fn known_hosts_get_their_measured_budgets_and_others_the_default() {
        let none = environment(&[]);
        assert_eq!(
            resolve_response_budget(Some("codex-mcp-client"), &none).bytes(),
            47_104
        );
        assert_eq!(
            resolve_response_budget(Some("claude-code"), &none).bytes(),
            73_728
        );
        for other in [None, Some("zcode"), Some("inspector"), Some("")] {
            assert_eq!(
                resolve_response_budget(other, &none).bytes(),
                16_384,
                "{other:?}"
            );
        }
        // Бюджет Codex вместе со строкой времени и округлением блоков
        // укладывается в его 12 000 токенов по четыре байта.
        let header = "Wall time: 1234.5678 seconds\nOutput:".len();
        let tokens = |bytes: usize| bytes.div_ceil(4);
        assert!(tokens(header) + tokens(400) + tokens(CODEX_RESPONSE_BUDGET - 400) <= 12_000);
        // Символов в бюджете Claude Code не больше объявленного предела.
        const { assert!(CLAUDE_RESPONSE_BUDGET as u64 <= CLAUDE_RESULT_SIZE_CHARS) };
        assert_eq!(
            MAX_RESPONSE_BUDGET,
            CLAUDE_RESPONSE_BUDGET.max(CODEX_RESPONSE_BUDGET)
        );
    }

    #[test]
    fn explicit_setting_then_client_name_then_launch_environment_then_default() {
        let explicit = environment(&[
            ("UNICA_RESPONSE_BUDGET", "20000"),
            ("CLAUDE_PLUGIN_DATA", "/d"),
        ]);
        assert_eq!(
            resolve_response_budget(Some("codex-mcp-client"), &explicit).bytes(),
            20_000
        );
        // Негодная настройка не считается: ни ноль, ни больше общего предела.
        for value in ["0", "100", "abc", "10000000"] {
            let invalid = environment(&[("UNICA_RESPONSE_BUDGET", value)]);
            assert_eq!(
                resolve_response_budget(Some("codex-mcp-client"), &invalid).bytes(),
                CODEX_RESPONSE_BUDGET,
                "{value}"
            );
        }
        // Имя клиента сильнее окружения, унаследованного от другого хоста;
        // неизвестное имя — умолчание, окружение запуска тогда не читается.
        let claude_launch = environment(&[("CLAUDE_PLUGIN_DATA", "/d")]);
        assert_eq!(
            resolve_response_budget(Some("codex-mcp-client"), &claude_launch).bytes(),
            CODEX_RESPONSE_BUDGET
        );
        assert_eq!(
            resolve_response_budget(Some("inspector"), &claude_launch).bytes(),
            DEFAULT_RESPONSE_BUDGET
        );
        assert_eq!(
            resolve_response_budget(None, &claude_launch).bytes(),
            CLAUDE_RESPONSE_BUDGET
        );
        // ZCode повторяет переменные Claude Code, но его предел не измерен.
        let zcode_launch =
            environment(&[("CLAUDE_PLUGIN_DATA", "/d"), ("ZCODE_PLUGIN_DATA", "/z")]);
        assert_eq!(
            resolve_response_budget(None, &zcode_launch).bytes(),
            DEFAULT_RESPONSE_BUDGET
        );
    }
}
