//! A refusal that offers a person a choice (`HumanChoice`) instead of a dead
//! end. The door asks the person in the course of the call when the client
//! can ask: by an `elicitation/create` request before protocol `2026-07-28`
//! and by an `input_required` result from `2026-07-28` on. The chosen answer
//! runs as its own call, and the response names it. A client that cannot ask,
//! a cancelled or declined question, an answer outside the offered options
//! and a question nobody answered in time all keep the refusal.
//!
//! rule: INV.RUNTIME.RUNNER-BASE-REFUSALS

use rmcp::model::{
    CallToolResult, ClientCapabilities, ContentBlock, ElicitRequestParams, ElicitResult,
    ElicitationAction, InputRequest, InputRequiredResult, InputRequests, InputResponses,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use unica_execution_contract::invocation::{HumanChoice, HumanChoiceOption};

/// Key of the question in `inputRequests` and `inputResponses`.
pub(crate) const CHOICE_INPUT_KEY: &str = "unica.choice";
/// Answer property of the question form.
const CHOICE_FIELD: &str = "choice";
/// The answer that keeps the refusal; never an option id.
const CANCEL: &str = "cancel";
/// Label of the cancel answer; agreed like every label of a question.
const CANCEL_TITLE: &str = "Отменить";
/// A question asked by an `elicitation/create` request waits this long for the
/// person; past it the refusal stands. The person reads the question and
/// decides whose change is right, so the wait is minutes, not seconds.
// resource-limit: mcp.human-choice-elicitation-timeout-ms
// class: deadline
// unit: ms
// scope: request
// value: 600000
// resource: Срок ожидания ответа человека на вопрос elicitation/create в ходе вызова.
// status: unproven
// origin: #2797
// problem: Человек читает вопрос и решает, чья правка верна: ждать нужно минуты, а не секунды.
// on-reach: Вопрос снимается, ответом остаётся прежний отказ; база и набор не меняются.
pub(crate) const ELICITATION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How the door can ask the person during this call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChoiceChannel {
    /// The client declared no form elicitation: the refusal is the answer.
    None,
    /// A session before `2026-07-28`: an `elicitation/create` request.
    Request,
    /// Protocol `2026-07-28`: an `input_required` result and a retry.
    InputRequired,
}

pub(crate) fn choice_channel(
    capabilities: Option<&ClientCapabilities>,
    modern: bool,
) -> ChoiceChannel {
    let form = capabilities
        .and_then(|capabilities| capabilities.elicitation.as_ref())
        // An empty capability object is the form mode of the first revision.
        .is_some_and(|elicitation| elicitation.form.is_some() || elicitation.url.is_none());
    match (form, modern) {
        (false, _) => ChoiceChannel::None,
        (true, false) => ChoiceChannel::Request,
        (true, true) => ChoiceChannel::InputRequired,
    }
}

/// The form asking the question: one required single-select answer, each
/// offered option with its title and the cancel answer last.
pub(crate) fn elicitation_params(choice: &HumanChoice) -> ElicitRequestParams {
    let mut one_of: Vec<Value> = choice
        .options
        .iter()
        .map(|option| json!({"const": option.id, "title": option.title}))
        .collect();
    one_of.push(json!({"const": CANCEL, "title": CANCEL_TITLE}));
    serde_json::from_value(json!({
        "mode": "form",
        "message": question_text(choice),
        "requestedSchema": {
            "type": "object",
            "properties": {
                CHOICE_FIELD: {"type": "string", "title": "Choice", "oneOf": one_of}
            },
            "required": [CHOICE_FIELD]
        }
    }))
    .expect("the choice form is a valid elicitation request")
}

/// The question with what each answer loses, one line per option: a form
/// option carries only its label.
pub(crate) fn question_text(choice: &HumanChoice) -> String {
    let mut text = choice.question.clone();
    text.push('
');
    for option in &choice.options {
        text.push_str(&format!("
{}: {}.", option.title, option.description));
    }
    text
}

/// The `input_required` result asking the question. `requestState` binds the
/// retry to this very question: the retry repeats the call, and the answer
/// applies only if the call is refused with the same question again.
pub(crate) fn input_required(choice: &HumanChoice) -> InputRequiredResult {
    let request = serde_json::from_value(json!({
        "method": "elicitation/create",
        "params": elicitation_params(choice),
    }))
    .expect("the choice form is a valid elicitation request");
    let mut requests = InputRequests::new();
    requests.insert(
        CHOICE_INPUT_KEY.to_string(),
        InputRequest::Elicitation(request),
    );
    InputRequiredResult::new(Some(requests), Some(choice_state(choice)))
}

/// Digest of the question and its calls. It is not trusted: it only says
/// which question the echoed answer belongs to.
pub(crate) fn choice_state(choice: &HumanChoice) -> String {
    let bytes = serde_json::to_vec(choice).expect("a choice serializes");
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("unica-choice-1:{hex}")
}

/// The answer to an `input_required` question carried by a retry, if it
/// answers this question.
pub(crate) fn retried_answer(
    choice: &HumanChoice,
    responses: Option<&InputResponses>,
    state: Option<&str>,
) -> Option<ElicitResult> {
    if state != Some(choice_state(choice).as_str()) {
        return None;
    }
    let response = responses?.get(CHOICE_INPUT_KEY)?;
    // A malformed answer is no answer from the person: it cancels.
    Some(
        serde_json::from_value(response.clone())
            .unwrap_or_else(|_| ElicitResult::new(ElicitationAction::Cancel)),
    )
}

/// The option the person chose, or `None` for every other outcome: cancel,
/// decline, no content, an unknown answer.
pub(crate) fn chosen<'a>(
    choice: &'a HumanChoice,
    answer: &ElicitResult,
) -> Option<&'a HumanChoiceOption> {
    if answer.action != ElicitationAction::Accept {
        return None;
    }
    let id = answer.content.as_ref()?.get(CHOICE_FIELD)?.as_str()?;
    choice.options.iter().find(|option| option.id == id)
}

/// The response of the chosen call names the choice before its own content.
pub(crate) fn name_the_choice(
    mut result: CallToolResult,
    option: &HumanChoiceOption,
) -> CallToolResult {
    result.content.insert(
        0,
        ContentBlock::text(format!(
            "Chosen by the person instead of the refusal: {} ({} {}).",
            option.title,
            option.tool,
            serde_json::to_string(&option.arguments).expect("arguments serialize"),
        )),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ElicitationCapability;
    use serde_json::Map;

    fn choice() -> HumanChoice {
        let call = |op: &str, args: Value| {
            let mut arguments = Map::new();
            arguments.insert("op".into(), json!(op));
            arguments.insert("args".into(), args);
            arguments.insert("dryRun".into(), json!(false));
            arguments
        };
        HumanChoice {
            question: "push of source set `main` loaded nothing".into(),
            options: vec![
                HumanChoiceOption {
                    id: "overwrite".into(),
                    title: "Overwrite the infobase".into(),
                    description: "infobase changes are lost".into(),
                    tool: "unica.run".into(),
                    arguments: call("push", json!({"force": true, "sourceSet": "main"})),
                },
                HumanChoiceOption {
                    id: "pull".into(),
                    title: "Take the infobase into the set".into(),
                    description: "unsaved file edits are lost".into(),
                    tool: "unica.run".into(),
                    arguments: call("pull", json!({"force": true, "sourceSet": "main"})),
                },
            ],
        }
    }

    fn accepted(id: &str) -> ElicitResult {
        ElicitResult::new(ElicitationAction::Accept).with_content(json!({CHOICE_FIELD: id}))
    }

    /// Only a client that declared form elicitation is asked; the protocol
    /// revision picks the channel.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn only_a_client_that_can_ask_is_asked() {
        let none = ClientCapabilities::default();
        let mut legacy_form = ClientCapabilities::default();
        legacy_form.elicitation = Some(ElicitationCapability::default());
        let url_only: ClientCapabilities = serde_json::from_value(json!({
            "elicitation": {"url": {}}
        }))
        .unwrap();
        let both: ClientCapabilities = serde_json::from_value(json!({
            "elicitation": {"form": {}, "url": {}}
        }))
        .unwrap();

        assert_eq!(choice_channel(None, false), ChoiceChannel::None);
        assert_eq!(choice_channel(Some(&none), true), ChoiceChannel::None);
        assert_eq!(choice_channel(Some(&url_only), false), ChoiceChannel::None);
        assert_eq!(
            choice_channel(Some(&legacy_form), false),
            ChoiceChannel::Request
        );
        assert_eq!(
            choice_channel(Some(&both), true),
            ChoiceChannel::InputRequired
        );
    }

    /// The form offers each option with its title and the cancel answer.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn the_form_offers_every_option_and_cancel() {
        let form = serde_json::to_value(elicitation_params(&choice())).unwrap();

        assert_eq!(form["mode"], "form");
        assert_eq!(
            form["message"],
            "push of source set `main` loaded nothing

Overwrite the infobase: infobase changes are lost.
Take the infobase into the set: unsaved file edits are lost."
        );
        let one_of = &form["requestedSchema"]["properties"][CHOICE_FIELD]["oneOf"];
        let ids: Vec<&str> = one_of
            .as_array()
            .unwrap()
            .iter()
            .map(|option| option["const"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["overwrite", "pull", CANCEL]);
        assert_eq!(one_of[0]["title"], "Overwrite the infobase");
        assert_eq!(one_of[2]["title"], CANCEL_TITLE);
        assert_eq!(form["requestedSchema"]["required"], json!([CHOICE_FIELD]));
    }

    /// The agreed text of the base divergence question: the heading, a line
    /// per option with what it loses, short labels and «Отменить». Labels
    /// change only together with this test.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn the_base_divergence_question_reads_as_agreed() {
        let mut agreed = choice();
        agreed.question = "База «origin» и исходники набора «main» разошлись.".into();
        agreed.options[0].title = "Перезаписать базу".into();
        agreed.options[0].description = "изменения, сделанные в базе, пропадут".into();
        agreed.options[1].title = "Перезаписать исходники".into();
        agreed.options[1].description = "несохранённые правки файлов пропадут".into();

        let form = serde_json::to_value(elicitation_params(&agreed)).unwrap();

        assert_eq!(
            form["message"],
            "База «origin» и исходники набора «main» разошлись.

             Перезаписать базу: изменения, сделанные в базе, пропадут.
             Перезаписать исходники: несохранённые правки файлов пропадут."
        );
        let titles: Vec<&str> = form["requestedSchema"]["properties"][CHOICE_FIELD]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|option| option["title"].as_str().unwrap())
            .collect();
        assert_eq!(
            titles,
            ["Перезаписать базу", "Перезаписать исходники", "Отменить"]
        );
    }

    /// Only an accepted offered option is a choice; everything else keeps
    /// the refusal, and Unica adds no `force` on its own.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn only_an_accepted_offered_option_is_a_choice() {
        let choice = choice();

        assert_eq!(
            chosen(&choice, &accepted("overwrite")).map(|option| &option.id),
            Some(&"overwrite".to_string())
        );
        assert_eq!(
            chosen(&choice, &accepted("pull")).map(|option| &option.id),
            Some(&"pull".to_string())
        );
        for refused in [
            accepted(CANCEL),
            accepted("force"),
            ElicitResult::new(ElicitationAction::Accept),
            ElicitResult::new(ElicitationAction::Decline),
            ElicitResult::new(ElicitationAction::Cancel)
                .with_content(json!({CHOICE_FIELD: "overwrite"})),
        ] {
            assert!(chosen(&choice, &refused).is_none(), "{refused:?}");
        }
    }

    /// The `input_required` result carries the question under its key and a
    /// state that binds the retry to this question; an answer to another
    /// question, or without the state, is no answer.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn a_retry_answers_only_the_question_it_was_asked() {
        let choice = choice();
        let asked = serde_json::to_value(input_required(&choice)).unwrap();

        assert_eq!(asked["resultType"], "input_required");
        assert_eq!(
            asked["inputRequests"][CHOICE_INPUT_KEY]["method"],
            "elicitation/create"
        );
        let state = asked["requestState"].as_str().unwrap();
        assert_eq!(state, choice_state(&choice));

        let mut responses = InputResponses::new();
        responses.insert(
            CHOICE_INPUT_KEY.to_string(),
            serde_json::to_value(accepted("pull")).unwrap(),
        );
        let answer = retried_answer(&choice, Some(&responses), Some(state)).unwrap();
        assert_eq!(chosen(&choice, &answer).unwrap().id, "pull");

        assert!(retried_answer(&choice, Some(&responses), None).is_none());
        let mut other = choice.clone();
        other.question.push_str(" again");
        assert!(retried_answer(&other, Some(&responses), Some(state)).is_none());
        assert!(retried_answer(&choice, None, Some(state)).is_none());

        responses.insert(CHOICE_INPUT_KEY.to_string(), json!("garbage"));
        let answer = retried_answer(&choice, Some(&responses), Some(state)).unwrap();
        assert!(chosen(&choice, &answer).is_none());
    }

    /// The response of the chosen call names the choice first.
    /// rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
    #[test]
    fn the_chosen_call_names_the_choice() {
        let choice = choice();
        let mut result = CallToolResult::default();
        result.content.push(ContentBlock::text("loaded"));

        let named = name_the_choice(result, &choice.options[0]);

        let first = serde_json::to_value(&named.content[0]).unwrap();
        let text = first["text"].as_str().unwrap();
        assert!(text.contains("Overwrite the infobase"), "{text}");
        assert!(text.contains("\"force\":true"), "{text}");
        assert_eq!(serde_json::to_value(&named.content[1]).unwrap()["text"], "loaded");
    }
}
