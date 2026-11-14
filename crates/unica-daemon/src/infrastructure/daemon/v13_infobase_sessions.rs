//! Вопросы человеку о занятой базе (INV.RUNTIME.INFOBASE-SESSIONS-QUESTION).
//!
//! Раннер отказывает операции, когда базу держат открытые сеансы
//! (`infobase_sessions`) или чужой Конфигуратор (`infobase_designer_busy`), а
//! после завершения сеансов своим автономным сервером над файловой базой —
//! отказом с выходом `standalone restart`. Из такого конверта строится вопрос,
//! а его варианты привязываются к исходному вызову там, где он известен
//! ([`bind_to_call`]). Сеансы завершаются и сервер перезапускается только по
//! отметке ответа человека ([`terminates_sessions`], [`restarts_server`]).
use super::v13_infobase_exports::{runner_rejection, runner_start_rejection, RUNNER_OUTPUT_LIMIT};
use super::v13_source_import::INFOBASE_NAME;
use serde_json::{json, Map, Value};
use std::path::Path;
use unica_artifact_delivery::engine::BundledTool;
use unica_execution_contract::invocation::{
    DomainResult, HumanChoice, HumanChoiceOption, HUMAN_CONFIRMATION_KEY,
};
use unica_process::runner::{ProcessCommand, ProcessRunner};
use unica_redaction::redactor;
use unica_refusal::RefusalCode;
use unica_work_control::cancellation::CancellationToken;

const SESSIONS_ACTIVE_CODE: &str = "infobase.sessions_active";
const DESIGNER_BUSY_CODE: &str = "infobase.designer_busy";
const SERVER_RESTART_CODE: &str = "infobase.server_restart";

/// Ключи отметок согласия: они не зависят от текста вопроса, поэтому
/// согласие остаётся в силе, даже если список сеансов к повтору изменился.
const TERMINATE_KEY: &str = "infobase sessions: end the sessions that hold the infobase";
const RESTART_KEY: &str = "infobase sessions: restart the standalone server of the infobase";

const RUN_TOOL: &str = "unica.run";
const TERMINATE: &str = "terminate";
const REPEAT: &str = "retry";
const RESTART: &str = "restart";

/// Сеансов в тексте вопроса не больше; остальные — «и ещё <K>», весь список —
/// в данных вопроса.
const LISTED_SESSIONS: usize = 5;

/// Вопрос об отказе раннера в `envelope`, если отказ — о занятой базе.
/// Варианты пока без аргументов: их даёт исходный вызов ([`bind_to_call`]).
pub(super) fn choice_for(envelope: &Value) -> Option<HumanChoice> {
    let error = &envelope["error"];
    if error["next"]["command"] == "standalone restart" {
        return Some(server_restart());
    }
    match error["code"].as_str()? {
        "infobase_designer_busy" => Some(designer_busy()),
        "infobase_sessions" => Some(sessions_active(&error["sessions"])),
        _ => None,
    }
}

/// Привязывает варианты вопроса о занятой базе к исходному вызову: «повторить»
/// — тот же вызов, «завершить сеансы» и «перезапустить сервер» — тот же вызов
/// с отметкой согласия. Превью не спрашивает: вопрос снимается.
pub(super) fn bind_to_call(mut result: DomainResult, call: &Map<String, Value>) -> DomainResult {
    let Some(choice) = result.human_choice.as_deref_mut() else {
        return result;
    };
    if ![
        SESSIONS_ACTIVE_CODE,
        DESIGNER_BUSY_CODE,
        SERVER_RESTART_CODE,
    ]
    .contains(&choice.code.as_str())
    {
        return result;
    }
    if call.get("dryRun") != Some(&Value::Bool(false)) {
        result.human_choice = None;
        return result;
    }
    for option in &mut choice.options {
        option.arguments = match option.id.as_str() {
            TERMINATE => consented(TERMINATE_KEY, call),
            RESTART => consented(RESTART_KEY, &consented(TERMINATE_KEY, call)),
            _ => without_consent(call),
        };
    }
    result
}

/// Ответ после завершения сеансов называет их: `data.sessionsTerminated`.
pub(super) fn note_terminated(result: &mut DomainResult, envelope: &Value) {
    let Some(terminated) = envelope["data"]["sessions"]["terminated"].as_array() else {
        return;
    };
    if terminated.is_empty() {
        return;
    }
    if let Some(Value::Object(data)) = result.data.as_mut() {
        data.insert(
            "sessionsTerminated".to_string(),
            Value::Array(terminated.iter().map(public_session).collect()),
        );
    }
}

/// Вызов несёт согласие человека завершить сеансы, которые держат базу.
pub(super) fn terminates_sessions(call: &Map<String, Value>) -> bool {
    HumanChoice::is_confirmed(TERMINATE_KEY, RUN_TOOL, call)
}

/// Вызов несёт согласие человека перезапустить свой автономный сервер базы.
pub(super) fn restarts_server(call: &Map<String, Value>) -> bool {
    HumanChoice::is_confirmed(RESTART_KEY, RUN_TOOL, call)
}

/// Перезапускает свой автономный сервер базы раннером (`standalone restart`)
/// перед повтором вызова, по согласию человека.
#[allow(clippy::result_large_err)]
pub(super) fn restart_server(
    config: &Path,
    cwd: &Path,
    tool: &BundledTool,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
) -> Result<(), DomainResult> {
    const AT: &str = "push";
    let output = runner
        .run_staged(&ProcessCommand {
            program: tool.program.clone(),
            args: vec![
                "--config".to_string(),
                config.display().to_string(),
                "--json-message".to_string(),
                "standalone".to_string(),
                "restart".to_string(),
            ],
            cwd: cwd.to_path_buf(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
            timeout: None,
            cancellation: cancellation.clone(),
        })
        .map_err(|error| runner_start_rejection(Some(AT.to_string()), &error, &tool.program))?;
    let envelope: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);
    if output.status_success && envelope["ok"] == true {
        return Ok(());
    }
    let code = envelope["error"]["code"]
        .as_str()
        .unwrap_or("provider_failed");
    let message = envelope["error"]["message"]
        .as_str()
        .map(redactor)
        .unwrap_or_else(|| "v8-runner could not restart the standalone server".to_string());
    if output.cancelled {
        return Err(DomainResult::canonical_rejection(
            Some(AT.to_string()),
            RefusalCode::Cancelled,
            "push cancelled while the standalone server restarted",
        ));
    }
    Err(runner_rejection(
        Some(AT.to_string()),
        code,
        envelope["error"]["layer"].as_str(),
        message,
    ))
}

/// The call without the consent to end sessions or restart the server: «I
/// closed them, repeat» repeats the call as such. Other confirmations stay.
fn without_consent(call: &Map<String, Value>) -> Map<String, Value> {
    let ours = [TERMINATE_KEY, RESTART_KEY].map(|key| mark(key, call));
    let mut call = call.clone();
    let kept: Vec<String> = call
        .get(HUMAN_CONFIRMATION_KEY)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split(' ')
        .filter(|mark| !mark.is_empty() && !ours.iter().any(|ours| ours == mark))
        .map(str::to_string)
        .collect();
    if kept.is_empty() {
        call.remove(HUMAN_CONFIRMATION_KEY);
    } else {
        call.insert(
            HUMAN_CONFIRMATION_KEY.to_string(),
            Value::String(kept.join(" ")),
        );
    }
    call
}

/// The mark of the consent `key` for `call`.
fn mark(key: &str, call: &Map<String, Value>) -> String {
    let bare = unica_execution_contract::invocation::without_confirmation(call);
    consented(key, &bare)
        .get(HUMAN_CONFIRMATION_KEY)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn consented(key: &str, call: &Map<String, Value>) -> Map<String, Value> {
    HumanChoice::confirmation(String::new(), key, "", "", RUN_TOOL, call)
        .options
        .remove(0)
        .arguments
}

fn option(id: &str, title: &str, description: &str) -> HumanChoiceOption {
    HumanChoiceOption {
        id: id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        tool: RUN_TOOL.to_string(),
        arguments: Map::new(),
        then: Vec::new(),
    }
}

fn choice(
    code: &str,
    question: String,
    data: Value,
    options: Vec<HumanChoiceOption>,
) -> HumanChoice {
    HumanChoice {
        code: code.to_string(),
        question,
        data: Some(data),
        options,
        cancel_description: None,
        proceeds_unasked: false,
    }
}

fn sessions_active(sessions: &Value) -> HumanChoice {
    let list = sessions["list"].as_array();
    let terminable = sessions["terminable"] == true;
    let count = list.map(Vec::len);
    let lines = list.map(|list| listed(list)).unwrap_or_default();
    let data = json!({
        "infobase": INFOBASE_NAME,
        "sessions": list.map(|list| list.iter().map(public_session).collect::<Vec<_>>()),
        "terminable": terminable,
        "evidence": sessions["evidence"],
    });
    let retry = option(REPEAT, "Я закрыл сеансы, повторить", "");
    if !terminable {
        let question = match count {
            Some(count) => format!(
                "Базу «{INFOBASE_NAME}» держат открытые сеансы: {count}. Завершить их отсюда нельзя.{lines}"
            ),
            None => format!(
                "Базу «{INFOBASE_NAME}» держат открытые сеансы. Завершить их отсюда нельзя."
            ),
        };
        return choice(SESSIONS_ACTIVE_CODE, question, data, vec![retry]);
    }
    let count = count.unwrap_or(0);
    let (question, loss) = match sessions["executor_error"].as_str() {
        Some(cause) if sessions["evidence"] == "server_list" => (
            format!(
                "Обновление базы «{INFOBASE_NAME}» не прошло: {}. Базу держат открытые сеансы: {count} — возможно, из-за них.{lines}",
                redactor(cause.trim_end_matches('.'))
            ),
            "несохранённые данные в этих сеансах пропадут; если причина в другом, обновление снова не пройдёт",
        ),
        _ => (
            format!("Базу «{INFOBASE_NAME}» держат открытые сеансы: {count}.{lines}"),
            "несохранённые данные в этих сеансах пропадут",
        ),
    };
    choice(
        SESSIONS_ACTIVE_CODE,
        question,
        data,
        vec![
            option(TERMINATE, "Завершить сеансы и продолжить", loss),
            retry,
        ],
    )
}

fn designer_busy() -> HumanChoice {
    choice(
        DESIGNER_BUSY_CODE,
        format!("Базу «{INFOBASE_NAME}» открыл Конфигуратор."),
        json!({"infobase": INFOBASE_NAME}),
        vec![option(REPEAT, "Я закрыл Конфигуратор, повторить", "")],
    )
}

fn server_restart() -> HumanChoice {
    choice(
        SERVER_RESTART_CODE,
        format!(
            "Обновление базы «{INFOBASE_NAME}» не прошло после завершения сеансов. Обычно помогает перезапуск сервера."
        ),
        json!({"infobase": INFOBASE_NAME}),
        vec![option(
            RESTART,
            "Перезапустить сервер и повторить",
            "сеансы, открытые заново, оборвутся",
        )],
    )
}

/// Строки сеансов для текста вопроса: «<компьютер> — <приложение>, с
/// <время>», пользователь — если раннер его знает; не больше пяти, дальше
/// «и ещё <K>».
fn listed(list: &[Value]) -> String {
    let mut text = String::new();
    for session in list.iter().take(LISTED_SESSIONS) {
        let computer = session["computer"].as_str().unwrap_or("?");
        let application = session["application_title"].as_str().unwrap_or("?");
        let started = session["started"]
            .as_str()
            .map(started_at)
            .unwrap_or_default();
        let user = session["user"]
            .as_str()
            .map(|user| format!(", {user}"))
            .unwrap_or_default();
        text.push_str(&format!("\n{computer} — {application}{user}, с {started}"));
    }
    if list.len() > LISTED_SESSIONS {
        text.push_str(&format!("\nи ещё {}", list.len() - LISTED_SESSIONS));
    }
    text
}

/// `ГГГГ-ММ-ДДTчч:мм:сс` раннера — `ГГГГ-ММ-ДД чч:мм`.
fn started_at(started: &str) -> String {
    let started = started.replacen('T', " ", 1);
    match started.char_indices().nth(16) {
        Some((end, _)) => started[..end].to_string(),
        None => started,
    }
}

/// Сеанс в данных вопроса: без учётных данных и служебного номера.
fn public_session(session: &Value) -> Value {
    json!({
        "computer": session["computer"],
        "application": session["application"],
        "applicationTitle": session["application_title"],
        "started": session["started"],
        "user": session["user"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sessions(count: usize, terminable: bool, evidence: &str) -> Value {
        let list: Vec<Value> = (1..=count)
            .map(|index| {
                json!({
                    "computer": format!("PC{index}"),
                    "application": "1CV8C",
                    "application_title": "Тонкий клиент",
                    "started": "2026-11-12T09:05:41",
                    "user": null,
                    "id": format!("{index}"),
                })
            })
            .collect();
        json!({"error": {"code": "infobase_sessions", "sessions": {
            "base": "Srvr=srv;Ref=dev",
            "list": list,
            "from_this_machine": false,
            "terminable": terminable,
            "lock": {},
            "evidence": evidence,
            "executor_error": if evidence == "server_list" { json!("update_db_cfg failed.") } else { Value::Null },
        }}})
    }

    fn call() -> Map<String, Value> {
        json!({"op": "push", "args": {"sourceSet": "main"}, "dryRun": false})
            .as_object()
            .unwrap()
            .clone()
    }

    /// Сеансы, которые раннер может завершить и из-за которых точно отказал:
    /// согласованный текст со списком не длиннее пяти строк, вариант
    /// «завершить» несёт согласие, «повторить» — тот же вызов.
    /// rule: INV.RUNTIME.INFOBASE-SESSIONS-QUESTION
    #[test]
    fn terminable_sessions_are_asked_with_their_list() {
        let choice = choice_for(&sessions(7, true, "executor")).expect("a question");
        assert_eq!(choice.code, "infobase.sessions_active");
        assert_eq!(
            choice.question,
            "Базу «origin» держат открытые сеансы: 7.\n\
             PC1 — Тонкий клиент, с 2026-11-12 09:05\n\
             PC2 — Тонкий клиент, с 2026-11-12 09:05\n\
             PC3 — Тонкий клиент, с 2026-11-12 09:05\n\
             PC4 — Тонкий клиент, с 2026-11-12 09:05\n\
             PC5 — Тонкий клиент, с 2026-11-12 09:05\n\
             и ещё 2"
        );
        let titles: Vec<_> = choice.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Завершить сеансы и продолжить",
                "Я закрыл сеансы, повторить"
            ]
        );
        assert_eq!(
            choice.options[0].description,
            "несохранённые данные в этих сеансах пропадут"
        );
        assert_eq!(
            choice.data.as_ref().unwrap()["sessions"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        assert!(!choice.data.as_ref().unwrap().to_string().contains("Srvr"));

        let mut result = DomainResult::success("refused");
        result.ok = false;
        result.human_choice = Some(Box::new(choice));
        let bound = bind_to_call(result, &call());
        let options = &bound.human_choice.as_ref().unwrap().options;
        assert!(terminates_sessions(&options[0].arguments));
        assert!(!terminates_sessions(&options[1].arguments));
        assert_eq!(options[1].arguments, call());
    }

    /// Отказ по списку сервера называет причину исполнителя и предупреждает,
    /// что завершение может не помочь.
    /// rule: INV.RUNTIME.INFOBASE-SESSIONS-QUESTION
    #[test]
    fn a_server_list_names_the_executor_error() {
        let choice = choice_for(&sessions(1, true, "server_list")).unwrap();
        assert!(choice.question.starts_with(
            "Обновление базы «origin» не прошло: update_db_cfg failed. Базу держат открытые сеансы: 1 — возможно, из-за них."
        ), "{}", choice.question);
        assert_eq!(
            choice.options[0].description,
            "несохранённые данные в этих сеансах пропадут; если причина в другом, обновление снова не пройдёт"
        );
    }

    /// Файловая база: завершить сеансы отсюда нельзя, остаётся «повторить».
    /// rule: INV.RUNTIME.INFOBASE-SESSIONS-QUESTION
    #[test]
    fn a_file_infobase_offers_only_a_retry() {
        let choice = choice_for(&sessions(1, false, "executor")).unwrap();
        assert_eq!(
            choice.question,
            "Базу «origin» держат открытые сеансы: 1. Завершить их отсюда нельзя.\nPC1 — Тонкий клиент, с 2026-11-12 09:05"
        );
        assert_eq!(choice.options.len(), 1);
        assert_eq!(choice.options[0].title, "Я закрыл сеансы, повторить");
        let unlisted = json!({"error": {"code": "infobase_sessions", "sessions": {"list": null, "terminable": false, "evidence": "executor"}}});
        assert_eq!(
            choice_for(&unlisted).unwrap().question,
            "Базу «origin» держат открытые сеансы. Завершить их отсюда нельзя."
        );
    }

    /// Чужой Конфигуратор и неудачный повтор после завершения сеансов у своего
    /// сервера — свои вопросы; перезапуск несёт и согласие завершить сеансы.
    /// Превью не спрашивает.
    /// rule: INV.RUNTIME.INFOBASE-SESSIONS-QUESTION
    #[test]
    fn a_designer_and_a_server_restart_have_their_own_questions() {
        let designer = choice_for(&json!({"error": {"code": "infobase_designer_busy"}})).unwrap();
        assert_eq!(designer.code, "infobase.designer_busy");
        assert_eq!(designer.question, "Базу «origin» открыл Конфигуратор.");
        assert_eq!(
            designer.options[0].title,
            "Я закрыл Конфигуратор, повторить"
        );

        let restart = choice_for(&json!({"error": {"code": "platform_failure", "next": {"command": "standalone restart"}}})).unwrap();
        assert_eq!(restart.code, "infobase.server_restart");
        assert_eq!(
            restart.question,
            "Обновление базы «origin» не прошло после завершения сеансов. Обычно помогает перезапуск сервера."
        );
        let mut result = DomainResult::success("refused");
        result.ok = false;
        result.human_choice = Some(Box::new(restart.clone()));
        let bound = bind_to_call(result, &call());
        let arguments = &bound.human_choice.as_ref().unwrap().options[0].arguments;
        assert!(restarts_server(arguments) && terminates_sessions(arguments));

        let mut preview = call();
        preview.insert("dryRun".into(), Value::Bool(true));
        let mut result = DomainResult::success("refused");
        result.ok = false;
        result.human_choice = Some(Box::new(restart));
        assert!(bind_to_call(result, &preview).human_choice.is_none());
    }
}
