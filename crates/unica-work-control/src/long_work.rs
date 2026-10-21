//! Состояние работы, которая не успела кончиться внутри вызова.
//!
//! Словарь взят у задач протокола (SEP-2663): когда хост их поддержит, это
//! состояние уедет в `Task` без правки поверхности — сменится транспорт, а не
//! форма.

use std::time::Duration;

use serde::Serialize;

/// Как у задач протокола. `working` — не неудача, а состояние.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Working,
    Completed,
    Failed,
    Cancelled,
}

/// Что вызывающий узнаёт о незаконченной работе.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkState {
    pub status: WorkStatus,
    /// Пояснение для человека и модели.
    pub status_message: String,
    /// Через сколько повторять. Отсутствует, пока считать не из чего.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u64>,
}

/// Сколько вызов ждёт синхронно, прежде чем ответить состоянием.
///
/// Тридцать секунд выведены из замеренного канала: 4,2 МБ/с на живой машине
/// покрывают за это время около ста двадцати мегабайт, то есть обычный случай
/// остаётся одним вызовом.
pub const DEFAULT_SYNC_WINDOW: Duration = Duration::from_secs(30);

/// Запас, чтобы успеть ответить раньше хоста.
///
/// Две секунды — вчетверо больше шага, которым обновляется состояние доставки;
/// сам ответ стоит миллисекунды. Если хост даёт меньше запаса, отвечать надо
/// немедленно: выиграть гонку всё равно нельзя, а его срез уносит наш ответ
/// целиком.
pub const HOST_CUT_MARGIN: Duration = Duration::from_secs(2);

/// Окно синхронного ожидания при известном сроке хоста.
pub fn sync_window(host_deadline: Option<Duration>) -> Duration {
    match host_deadline {
        Some(deadline) => deadline
            .saturating_sub(HOST_CUT_MARGIN)
            .min(DEFAULT_SYNC_WINDOW),
        None => DEFAULT_SYNC_WINDOW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_host_deadline_leaves_our_own_window() {
        // Умолчание хоста измерено большим десяти минут, так что неизвестность
        // здесь означает «времени вдоволь».
        assert_eq!(sync_window(None), DEFAULT_SYNC_WINDOW);
    }

    #[test]
    fn a_generous_host_deadline_does_not_stretch_our_window() {
        assert_eq!(
            sync_window(Some(Duration::from_secs(600))),
            DEFAULT_SYNC_WINDOW
        );
    }

    #[test]
    fn a_short_host_deadline_leaves_room_to_answer_first() {
        // Срез хоста уносит наш ответ целиком, поэтому отвечаем раньше него.
        assert_eq!(
            sync_window(Some(Duration::from_secs(15))),
            Duration::from_secs(13)
        );
    }

    #[test]
    fn a_deadline_shorter_than_the_margin_answers_at_once() {
        assert_eq!(sync_window(Some(Duration::from_secs(1))), Duration::ZERO);
    }

    #[test]
    fn a_working_state_serializes_with_the_protocol_vocabulary() {
        let state = WorkState {
            status: WorkStatus::Working,
            status_message: "delivering rlm-tools-bsl: 12 of 69 bytes".to_owned(),
            poll_interval_ms: Some(5_000),
        };

        let wire = serde_json::to_value(&state).expect("serializable");

        assert_eq!(wire["status"], "working");
        assert_eq!(wire["pollIntervalMs"], 5_000);
        assert!(wire["statusMessage"].as_str().is_some());
    }

    #[test]
    fn an_unknown_poll_interval_is_absent_rather_than_zero() {
        let state = WorkState {
            status: WorkStatus::Working,
            status_message: "delivering".to_owned(),
            poll_interval_ms: None,
        };

        let wire = serde_json::to_value(&state).expect("serializable");

        assert!(wire.get("pollIntervalMs").is_none(), "{wire}");
    }
}
