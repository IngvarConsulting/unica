//! A refusal that offers a person a choice (`HumanChoice`) instead of a dead
//! end, or that asks the person to confirm a destructive call. The door asks
//! the person in the course of the call when the client can ask: by an
//! `elicitation/create` request before protocol `2026-07-28` and by an
//! `input_required` result from `2026-07-28` on. The chosen answer runs as
//! its own call or chain of calls, and the response names it. A cancelled or
//! declined question, an answer outside the offered options and a question
//! nobody answered in time all keep the refusal. A client that cannot ask
//! keeps the refusal of a choice and runs a confirmed call as before.
//!
//! rule: INV.MCP.HUMAN-CHOICE

use rmcp::model::{
    CallToolResult, ClientCapabilities, ContentBlock, ElicitRequestParams, ElicitResult,
    ElicitationAction, InputRequest, InputRequests, InputRequiredResult, InputResponses,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use unica_execution_contract::invocation::{
    without_confirmation, HumanChoice, HumanChoiceOption, HUMAN_CONFIRMATION_KEY,
};

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
        "_meta": {QUESTION_META_KEY: question_meta(choice)},
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

/// Key of the question's structured description in the form's `_meta`.
pub(crate) const QUESTION_META_KEY: &str = "unica/question";

/// What a client needs to draw its own screen for the question: its stable
/// code, its structured facts and, per answer value, the label and what the
/// answer loses. The answer is still one value of the form's enumeration.
pub(crate) fn question_meta(choice: &HumanChoice) -> Value {
    let mut options: Vec<Value> = choice
        .options
        .iter()
        .map(|option| {
            json!({"value": option.id, "title": option.title, "description": option.description})
        })
        .collect();
    let mut cancel = json!({"value": CANCEL, "title": CANCEL_TITLE});
    if let Some(description) = &choice.cancel_description {
        cancel["description"] = json!(description);
    }
    options.push(cancel);
    let mut meta = json!({
        "code": choice.code,
        "kind": if choice.proceeds_unasked { "confirmation" } else { "choice" },
        "options": options,
    });
    if let Some(data) = &choice.data {
        meta["data"] = data.clone();
    }
    meta
}

/// The question with what each answer loses, one line per option: a form
/// option carries only its label.
pub(crate) fn question_text(choice: &HumanChoice) -> String {
    let mut text = choice.question.clone();
    text.push('\n');
    for option in &choice.options {
        text.push_str(&format!("\n{}: {}.", option.title, option.description));
    }
    if let Some(description) = &choice.cancel_description {
        text.push_str(&format!("\n{CANCEL_TITLE}: {description}."));
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

/// Questions one call asks at most: an answer that runs into a new question
/// about the same call asks it too, up to this many in all; past it the call
/// keeps its refusal (INV.MCP.HUMAN-CHOICE).
pub(crate) const MAX_QUESTIONS_PER_CALL: usize = 3;

const CHAIN_STATE_PREFIX: &str = "unica-choice-chain-1:";

/// The answers a protocol `2026-07-28` retry carries for the questions it
/// was asked before the current one, and the state of the current question.
/// Each answer applies only while its question comes back the same.
#[derive(Default)]
pub(crate) struct RetriedChain {
    answered: Vec<(String, Value)>,
    current: Option<String>,
}

impl RetriedChain {
    pub(crate) fn parse(state: Option<&str>) -> Self {
        let Some(state) = state else {
            return Self::default();
        };
        let Some(chain) = state.strip_prefix(CHAIN_STATE_PREFIX) else {
            return Self {
                answered: Vec::new(),
                current: Some(state.to_string()),
            };
        };
        let parsed: Option<Value> = serde_json::from_str(chain).ok();
        let answered = parsed
            .as_ref()
            .and_then(|chain| chain.get("answered")?.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| {
                Some((
                    entry.get("state")?.as_str()?.to_string(),
                    entry.get("answer")?.clone(),
                ))
            })
            .collect();
        let current = parsed
            .as_ref()
            .and_then(|chain| chain.get("state")?.as_str().map(str::to_string));
        Self { answered, current }
    }

    /// The answer to the `index`-th question of the call, `choice`, if the
    /// retry carries one for this very question.
    pub(crate) fn answer(
        &self,
        index: usize,
        choice: &HumanChoice,
        responses: Option<&InputResponses>,
    ) -> Option<ElicitResult> {
        let state = choice_state(choice);
        if let Some((answered_state, answer)) = self.answered.get(index) {
            return (*answered_state == state).then(|| answer_of(answer.clone()));
        }
        if index != self.answered.len() || self.current.as_deref() != Some(state.as_str()) {
            return None;
        }
        Some(answer_of(responses?.get(CHOICE_INPUT_KEY)?.clone()))
    }
}

/// A malformed answer is no answer from the person: it cancels.
fn answer_of(response: Value) -> ElicitResult {
    serde_json::from_value(response)
        .unwrap_or_else(|_| ElicitResult::new(ElicitationAction::Cancel))
}

/// The answer a client that cannot ask stands for: the first option of a
/// confirmation, as if the person had chosen it.
pub(crate) fn unasked_confirmation(choice: &HumanChoice) -> ElicitResult {
    ElicitResult::new(ElicitationAction::Accept)
        .with_content(json!({CHOICE_FIELD: choice.options[0].id}))
}

/// The response names the answers given before the last question first, in
/// the order they were asked.
pub(crate) fn name_earlier_choices(
    mut result: CallToolResult,
    earlier: &[(&HumanChoice, &HumanChoiceOption)],
) -> CallToolResult {
    if earlier.is_empty() {
        return result;
    }
    let text = earlier
        .iter()
        .map(|(choice, option)| {
            if choice.proceeds_unasked {
                format!("Confirmed by the person: {}.", option.title)
            } else {
                format!(
                    "Chosen by the person instead of the refusal: {}.",
                    option.title
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    result.content.insert(0, ContentBlock::text(text));
    result
}

/// The `input_required` result asking `choice` after the questions the call
/// was already answered: its `requestState` carries those answers, so the
/// retry repeats the call and every answer applies again in turn.
pub(crate) fn input_required_after(
    choice: &HumanChoice,
    answered: &[(&HumanChoice, &ElicitResult)],
) -> InputRequiredResult {
    let mut result = input_required(choice);
    if !answered.is_empty() {
        let chain = json!({
            "answered": answered
                .iter()
                .map(|(question, answer)| json!({"state": choice_state(question), "answer": answer}))
                .collect::<Vec<_>>(),
            "state": choice_state(choice),
        });
        result.request_state = Some(format!("{CHAIN_STATE_PREFIX}{chain}"));
    }
    result
}

/// The answer to the first `input_required` question carried by a retry, if
/// it answers this question.
#[cfg(test)]
pub(crate) fn retried_answer(
    choice: &HumanChoice,
    responses: Option<&InputResponses>,
    state: Option<&str>,
) -> Option<ElicitResult> {
    RetriedChain::parse(state).answer(0, choice, responses)
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

/// The calls of a chosen answer, in order: its own call and the ones that
/// follow it.
pub(crate) fn calls(option: &HumanChoiceOption) -> Vec<(&str, &Map<String, Value>)> {
    std::iter::once((option.tool.as_str(), &option.arguments))
        .chain(
            option
                .then
                .iter()
                .map(|call| (call.tool.as_str(), &call.arguments)),
        )
        .collect()
}

/// Whether a call names the confirmation only the door may set.
pub(crate) fn carries_confirmation(arguments: Option<&Map<String, Value>>) -> bool {
    arguments.is_some_and(|arguments| arguments.contains_key(HUMAN_CONFIRMATION_KEY))
}

/// How one call of a chosen answer ended.
pub(crate) struct StepReport {
    pub(crate) tool: String,
    pub(crate) arguments: Map<String, Value>,
    pub(crate) refused: bool,
}

impl StepReport {
    pub(crate) fn of(tool: &str, arguments: &Map<String, Value>, result: &CallToolResult) -> Self {
        Self {
            tool: tool.to_string(),
            arguments: without_confirmation(arguments),
            refused: result.is_error == Some(true),
        }
    }
}

/// The response of the chosen answer names the choice and, for a chain, how
/// each call ended before the content of the last call that ran.
pub(crate) fn name_the_choice(
    mut result: CallToolResult,
    choice: &HumanChoice,
    option: &HumanChoiceOption,
    steps: &[StepReport],
) -> CallToolResult {
    let call = |step: &StepReport| {
        format!(
            "{} {}",
            step.tool,
            serde_json::to_string(&step.arguments).expect("arguments serialize")
        )
    };
    let mut text = if choice.proceeds_unasked {
        format!("Confirmed by the person: {}", option.title)
    } else {
        format!(
            "Chosen by the person instead of the refusal: {}",
            option.title
        )
    };
    match steps {
        [only] if option.then.is_empty() => text.push_str(&format!(" ({}).", call(only))),
        _ => {
            text.push('.');
            let total = 1 + option.then.len();
            for (index, step) in steps.iter().enumerate() {
                let outcome = if step.refused { "refused" } else { "done" };
                text.push_str(&format!(
                    "\n{}/{total} {outcome}: {}",
                    index + 1,
                    call(step)
                ));
            }
            if steps.len() < total {
                text.push_str(&format!(
                    "\nThe remaining {} call(s) did not run.",
                    total - steps.len()
                ));
            }
        }
    }
    result.content.insert(0, ContentBlock::text(text));
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
            code: "source.base_divergence".into(),
            question: "push of source set `main` loaded nothing".into(),
            data: Some(json!({"sourceSet": "main"})),
            options: vec![
                HumanChoiceOption {
                    id: "overwrite".into(),
                    title: "Overwrite the infobase".into(),
                    description: "infobase changes are lost".into(),
                    tool: "unica.run".into(),
                    arguments: call("push", json!({"force": true, "sourceSet": "main"})),
                    then: Vec::new(),
                },
                HumanChoiceOption {
                    id: "pull".into(),
                    title: "Take the infobase into the set".into(),
                    description: "unsaved file edits are lost".into(),
                    tool: "unica.run".into(),
                    arguments: call("pull", json!({"force": true, "sourceSet": "main"})),
                    then: Vec::new(),
                },
            ],
            cancel_description: None,
            proceeds_unasked: false,
        }
    }

    fn accepted(id: &str) -> ElicitResult {
        ElicitResult::new(ElicitationAction::Accept).with_content(json!({CHOICE_FIELD: id}))
    }

    /// Only a client that declared form elicitation is asked; the protocol
    /// revision picks the channel.
    /// rule: INV.MCP.HUMAN-CHOICE
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
            "push of source set `main` loaded nothing\n\n\
             Overwrite the infobase: infobase changes are lost.\n\
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

    /// The form's `_meta` carries the stable code of the question, its
    /// structured facts and every answer value with its label and loss, so a
    /// client can draw its own screen; the answer stays an enumerated value.
    /// rule: INV.MCP.HUMAN-CHOICE
    #[test]
    fn the_form_names_the_question_code_and_its_facts() {
        let form = serde_json::to_value(elicitation_params(&choice())).unwrap();
        let meta = &form["_meta"][QUESTION_META_KEY];

        assert_eq!(meta["code"], "source.base_divergence");
        assert_eq!(meta["kind"], "choice");
        assert_eq!(meta["data"], json!({"sourceSet": "main"}));
        let values: Vec<&str> = meta["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|option| option["value"].as_str().unwrap())
            .collect();
        assert_eq!(values, ["overwrite", "pull", CANCEL]);
        assert_eq!(
            meta["options"][0]["description"],
            "infobase changes are lost"
        );

        let mut confirmation = choice();
        confirmation.proceeds_unasked = true;
        confirmation.data = None;
        let form = serde_json::to_value(elicitation_params(&confirmation)).unwrap();
        assert_eq!(form["_meta"][QUESTION_META_KEY]["kind"], "confirmation");
        assert!(form["_meta"][QUESTION_META_KEY].get("data").is_none());
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
            "База «origin» и исходники набора «main» разошлись.\n\n\
             Перезаписать базу: изменения, сделанные в базе, пропадут.\n\
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

    /// What the person can do instead of an answer goes with «Отменить»: a
    /// last line of the question and the description of the cancel answer.
    /// rule: INV.MCP.HUMAN-CHOICE
    #[test]
    fn the_cancel_answer_carries_its_own_line_when_the_question_has_one() {
        let mut asked = choice();
        asked.options.truncate(1);
        asked.cancel_description = Some("доработку можно внести в расширение".into());

        let form = serde_json::to_value(elicitation_params(&asked)).unwrap();

        assert_eq!(
            form["message"],
            "push of source set `main` loaded nothing\n\n\
             Overwrite the infobase: infobase changes are lost.\n\
             Отменить: доработку можно внести в расширение."
        );
        let options = form["_meta"][QUESTION_META_KEY]["options"]
            .as_array()
            .unwrap();
        assert_eq!(options[1]["value"], CANCEL);
        assert_eq!(
            options[1]["description"],
            "доработку можно внести в расширение"
        );
        let plain = serde_json::to_value(elicitation_params(&choice())).unwrap();
        let options = plain["_meta"][QUESTION_META_KEY]["options"]
            .as_array()
            .unwrap();
        assert!(options.last().unwrap().get("description").is_none());
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

        let step = StepReport::of("unica.run", &choice.options[0].arguments, &result);
        let named = name_the_choice(result, &choice, &choice.options[0], &[step]);

        let first = serde_json::to_value(&named.content[0]).unwrap();
        let text = first["text"].as_str().unwrap();
        assert!(text.contains("Overwrite the infobase"), "{text}");
        assert!(text.contains("\"force\":true"), "{text}");
        assert_eq!(
            serde_json::to_value(&named.content[1]).unwrap()["text"],
            "loaded"
        );
    }
}
