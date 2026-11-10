//! Ход подготовки анализа как `$/progress` редактора.
//!
//! Токен ходу работы создаёт сервер запросом `window/workDoneProgress/create`
//! и пользуется им только после успешного ответа редактора: сообщения до
//! ответа ждут в очереди, а отказ редактора их отбрасывает. Редактор, который
//! не объявил `window.workDoneProgress`, ничего не получает: о неготовности
//! ему по-прежнему говорит пометка неполного ответа.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use unica_lsp_contract::protocol::V5LspProgressPhase;

/// Номер запроса создания токена: по нему дверь узнаёт ответ редактора.
const CREATE_REQUEST: &str = "unica-progress-";

pub struct WorkDoneProgress {
    enabled: bool,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    /// Запросы создания, ждущие ответа: номер запроса → токен.
    requests: HashMap<String, String>,
    tokens: HashMap<String, Token>,
}

enum Token {
    /// Создание запрошено; сообщения ждут ответа редактора.
    Creating(Vec<Value>),
    Created,
}

impl WorkDoneProgress {
    /// `enabled` — редактор объявил `window.workDoneProgress`.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            inner: Mutex::default(),
        }
    }

    /// Сообщения редактору об одной стадии, в порядке отправки. Начало
    /// неизвестного токена запрашивает его создание; стадия токена, которого
    /// нет, теряется: ход работы без начала редактор не покажет.
    pub fn stage(
        &self,
        token: &str,
        phase: V5LspProgressPhase,
        title: Option<&str>,
        message: &str,
    ) -> Vec<Value> {
        if !self.enabled {
            return Vec::new();
        }
        let value = match phase {
            V5LspProgressPhase::Begin => json!({
                "kind": "begin",
                "title": title.unwrap_or_default(),
                "message": message,
            }),
            V5LspProgressPhase::Report => json!({"kind": "report", "message": message}),
            V5LspProgressPhase::End => json!({"kind": "end", "message": message}),
        };
        let progress = json!({
            "jsonrpc": "2.0",
            "method": "$/progress",
            "params": {"token": token, "value": value},
        });
        let mut inner = crate::door::lock(&self.inner);
        match inner.tokens.get_mut(token) {
            Some(Token::Creating(queue)) => {
                queue.push(progress);
                Vec::new()
            }
            Some(Token::Created) => {
                if phase == V5LspProgressPhase::End {
                    inner.tokens.remove(token);
                }
                vec![progress]
            }
            None if phase == V5LspProgressPhase::Begin => {
                let id = format!("{CREATE_REQUEST}{}", inner.next);
                inner.next += 1;
                inner.requests.insert(id.clone(), token.to_owned());
                inner
                    .tokens
                    .insert(token.to_owned(), Token::Creating(vec![progress]));
                vec![json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "window/workDoneProgress/create",
                    "params": {"token": token},
                })]
            }
            None => Vec::new(),
        }
    }

    /// Ответ редактора на запрос с номером `id`: `None` — запрос не этот.
    /// Успех отдаёт ждавшие сообщения, отказ их отбрасывает.
    pub fn answered(&self, id: &Value, created: bool) -> Option<Vec<Value>> {
        let id = id.as_str().filter(|id| id.starts_with(CREATE_REQUEST))?;
        let mut inner = crate::door::lock(&self.inner);
        let token = inner.requests.remove(id)?;
        let Some(Token::Creating(queue)) = inner.tokens.remove(&token) else {
            return Some(Vec::new());
        };
        if !created {
            return Some(Vec::new());
        }
        let ended = queue
            .last()
            .is_some_and(|last| last.pointer("/params/value/kind") == Some(&json!("end")));
        if !ended {
            inner.tokens.insert(token, Token::Created);
        }
        Some(queue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(messages: &[Value]) -> Vec<String> {
        messages
            .iter()
            .map(|message| {
                message
                    .pointer("/params/value/kind")
                    .or_else(|| message.get("method"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }

    /// rule: INV.LSP.PARTIAL-ANSWERS
    #[test]
    fn progress_waits_for_the_editor_to_create_its_token() {
        let progress = WorkDoneProgress::new(true);
        let create = progress.stage(
            "unica-analysis-work-main",
            V5LspProgressPhase::Begin,
            Some("Анализатор готовит базу"),
            "Запуск анализатора",
        );
        assert_eq!(kinds(&create), ["window/workDoneProgress/create"]);
        assert_eq!(create[0]["params"]["token"], "unica-analysis-work-main");
        assert!(progress
            .stage(
                "unica-analysis-work-main",
                V5LspProgressPhase::Report,
                None,
                "Строится база: 5 с",
            )
            .is_empty());

        let flushed = progress.answered(&create[0]["id"], true).unwrap();
        assert_eq!(kinds(&flushed), ["begin", "report"]);
        assert_eq!(flushed[0]["params"]["value"]["title"], "Анализатор готовит базу");
        let end = progress.stage(
            "unica-analysis-work-main",
            V5LspProgressPhase::End,
            None,
            "База готова: 5902 модуля",
        );
        assert_eq!(kinds(&end), ["end"]);

        // A later build of the same set creates its token again.
        let again = progress.stage(
            "unica-analysis-work-main",
            V5LspProgressPhase::Begin,
            Some("Анализатор готовит базу"),
            "Запуск анализатора",
        );
        assert_eq!(kinds(&again), ["window/workDoneProgress/create"]);
    }

    #[test]
    fn a_refused_token_drops_its_progress_and_an_editor_without_the_capability_hears_none() {
        let progress = WorkDoneProgress::new(true);
        let create = progress.stage(
            "token",
            V5LspProgressPhase::Begin,
            Some("title"),
            "start",
        );
        assert_eq!(progress.answered(&create[0]["id"], false), Some(Vec::new()));
        assert!(progress
            .stage("token", V5LspProgressPhase::Report, None, "lost")
            .is_empty());
        assert_eq!(progress.answered(&json!(7), true), None);

        let silent = WorkDoneProgress::new(false);
        assert!(silent
            .stage("token", V5LspProgressPhase::Begin, Some("title"), "start")
            .is_empty());
    }

    #[test]
    fn an_end_before_the_answer_closes_the_token_with_the_queue() {
        let progress = WorkDoneProgress::new(true);
        let create = progress.stage(
            "token",
            V5LspProgressPhase::Begin,
            Some("title"),
            "start",
        );
        progress.stage("token", V5LspProgressPhase::End, None, "done");
        let flushed = progress.answered(&create[0]["id"], true).unwrap();
        assert_eq!(kinds(&flushed), ["begin", "end"]);
        assert!(progress
            .stage("token", V5LspProgressPhase::Report, None, "late")
            .is_empty());
    }
}
