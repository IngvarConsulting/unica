//! Вопрос человеку о базе другой рабочей копии (INV.RUNTIME.RUNNER-BASE-REFUSALS).
//!
//! Запись в файловую базу, которую держит другая рабочая копия, раннер отказом
//! не считает: он предупреждает и пишет. Поэтому исполняемые `push`, `pull`,
//! изменение расширений и `infobase.restore` над файловой базой до записи
//! сверяются с раннером в режиме превью ([`needs_preview`]). Если превью
//! предупреждает о базе другой копии, а согласия человека на этот вызов нет,
//! вызов отменяется с вопросом ([`refusal_for`]). Клиент, который спрашивать не
//! умеет, исполняет вызов как прежде, с прежним предупреждением.
use super::runner::{origin_kind, OriginKind};
use super::v13_infobase_exports::another_copy_warning;
use super::v13_source_import::INFOBASE_NAME;
use serde_json::{json, Map, Value};
use std::path::Path;
use unica_execution_contract::invocation::{DomainResult, HumanChoice};
use unica_refusal::RefusalCode;

const CODE: &str = "infobase.another_copy";
const RUN_TOOL: &str = "unica.run";
const CONTINUE: &str = "continue";

fn question() -> String {
    format!("Базу «{INFOBASE_NAME}» изменяла другая рабочая копия проекта.")
}

/// Исполнение этого вызова сначала сверяется с раннером в режиме превью:
/// база файловая, а согласия человека писать в базу другой копии нет. У
/// серверной базы метки владельца нет, и превью не делается.
pub(super) fn needs_preview(workspace_root: &Path, call: &Map<String, Value>) -> bool {
    !consented(call) && matches!(origin_kind(workspace_root), Ok(OriginKind::File))
}

/// Вызов несёт согласие человека писать в базу другой копии.
pub(super) fn consented(call: &Map<String, Value>) -> bool {
    HumanChoice::is_confirmed(&question(), RUN_TOOL, call)
}

/// Отказ `cancelled` с вопросом, если превью `preview` предупреждает о базе
/// другой копии: база не тронута. `at` — имя операции в отказе.
pub(super) fn refusal_for(
    at: &str,
    preview: &Value,
    call: &Map<String, Value>,
) -> Option<DomainResult> {
    another_copy_warning(preview)?;
    let mut rejection = DomainResult::canonical_rejection(
        Some(at.to_string()),
        RefusalCode::Cancelled,
        format!("{at} writes the file infobase that another working copy holds and runs only with the person's consent; it was not given, and the infobase is unchanged"),
    );
    super::v13_infobase_exports::note_runner_warnings(&mut rejection, preview);
    let mut choice = HumanChoice::confirmation(
        CODE,
        question(),
        "Продолжить с этой базой",
        "правки другой копии могут быть перезаписаны",
        RUN_TOOL,
        call,
    )
    .with_data(json!({"infobase": INFOBASE_NAME}));
    choice.options[0].id = CONTINUE.to_string();
    rejection.human_choice = Some(Box::new(choice));
    Some(rejection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> Map<String, Value> {
        json!({"op": "push", "args": {"sourceSet": "main"}, "dryRun": false})
            .as_object()
            .unwrap()
            .clone()
    }

    fn warned() -> Value {
        json!({"ok": true, "warnings": [
            "push writes the infobase '/x/ib' of another working copy: it is held by /y (pid 1)"
        ]})
    }

    /// Превью, предупреждающее о базе другой копии, отменяет исполнение с
    /// вопросом; «Продолжить с этой базой» — тот же вызов с согласием, и
    /// клиент без вопросов исполняет его как прежде.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn a_write_into_another_copys_base_asks_the_person() {
        let refused = refusal_for("push", &warned(), &call()).expect("a question");
        assert_eq!(refused.diagnostics[0]["code"], "cancelled", "{refused:?}");
        let choice = refused.human_choice.as_deref().unwrap();
        assert_eq!(choice.code, "infobase.another_copy");
        assert_eq!(
            choice.question,
            "Базу «origin» изменяла другая рабочая копия проекта."
        );
        assert!(choice.proceeds_unasked);
        assert_eq!(choice.options[0].id, "continue");
        assert_eq!(choice.options[0].title, "Продолжить с этой базой");
        assert_eq!(
            choice.options[0].description,
            "правки другой копии могут быть перезаписаны"
        );
        assert!(!consented(&call()));
        assert!(consented(&choice.options[0].arguments));
        assert!(refused
            .warnings
            .iter()
            .any(|warning| warning["code"] == "infobase_of_another_copy"));

        assert!(refusal_for("push", &json!({"ok": true, "warnings": []}), &call()).is_none());
    }
}

/// The call with the person's consent to write into the infobase of another
/// copy, as the door sets it from the answer: for tests of what follows it.
#[cfg(test)]
pub(super) fn with_consent(call: &Map<String, Value>) -> Map<String, Value> {
    HumanChoice::confirmation(CODE, question(), "", "", RUN_TOOL, call)
        .options
        .remove(0)
        .arguments
}
