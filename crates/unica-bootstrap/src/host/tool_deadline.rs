//! Срок, который хост отмеряет одному вызову инструмента.

use std::ffi::OsString;
use std::time::Duration;

/// Переменная Claude Code. Протокол о сроке клиента серверу не сообщает —
/// в `initialize` такого поля нет, — а эта переменная наследуется процессом
/// сервера и потому читается напрямую.
const TOOL_TIMEOUT_ENV: &str = "MCP_TOOL_TIMEOUT";

/// Сколько хост готов ждать один вызов, если он это назвал.
///
/// `None` — хост своего срока не назвал. Это не «нисколько»: умолчание
/// измерено большим десяти минут, то есть времени вдоволь.
pub fn host_tool_deadline() -> Option<Duration> {
    resolve_tool_deadline(&|name| std::env::var_os(name))
}

fn resolve_tool_deadline(read_env: &dyn Fn(&str) -> Option<OsString>) -> Option<Duration> {
    let value = read_env(TOOL_TIMEOUT_ENV)?;
    let millis = value.to_str()?.trim().parse::<u64>().ok()?;
    (millis > 0).then(|| Duration::from_millis(millis))
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
    fn a_named_deadline_is_read_as_milliseconds() {
        let deadline = resolve_tool_deadline(&environment(&[("MCP_TOOL_TIMEOUT", "15000")]));

        assert_eq!(deadline, Some(Duration::from_secs(15)));
    }

    #[test]
    fn an_unnamed_deadline_means_time_enough_not_none_at_all() {
        assert_eq!(resolve_tool_deadline(&environment(&[])), None);
    }

    #[test]
    fn a_value_that_is_not_a_number_is_the_same_as_unnamed() {
        // Соврать про срок хуже, чем его не знать: короткое окно из мусора
        // резало бы вызовы там, где хост их терпит.
        for value in ["", "   ", "soon", "-1", "0"] {
            assert_eq!(
                resolve_tool_deadline(&environment(&[("MCP_TOOL_TIMEOUT", value)])),
                None,
                "значение {value:?}"
            );
        }
    }
}
