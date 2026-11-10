//! The door asks a person the choice a refusal offers, through `tools/call`
//! on the wire: a client without form elicitation gets the refusal, a
//! session before `2026-07-28` gets an `elicitation/create` request and a
//! `2026-07-28` client an `input_required` result.
//!
//! rule: INV.RUNTIME.RUNNER-BASE-REFUSALS
//! rule: INV.MCP.HUMAN-CHOICE

use super::*;
use crate::test_support::*;
use serde_json::json;
use unica_execution_contract::invocation::{
    DomainResult, HumanChoice, HumanChoiceCall, HumanChoiceOption,
};

const REFUSAL: &str = "push loaded nothing";
const LOADED: &str = "loaded with force";

fn push_call(force: bool) -> Map<String, Value> {
    let mut args = json!({"sourceSet": "main"});
    if force {
        args["force"] = json!(true);
    }
    json!({"op": "push", "args": args, "dryRun": false})
        .as_object()
        .unwrap()
        .clone()
}

fn choice() -> HumanChoice {
    let pull = json!({"op": "pull", "args": {"force": true, "sourceSet": "main"}, "dryRun": false});
    HumanChoice {
        code: "source.base_divergence".into(),
        question: "push of source set `main` loaded nothing".into(),
        data: None,
        options: vec![
            HumanChoiceOption {
                id: "overwrite".into(),
                title: "Overwrite the infobase".into(),
                description: "infobase changes are lost".into(),
                tool: "unica.run".into(),
                arguments: push_call(true),
                then: Vec::new(),
            },
            HumanChoiceOption {
                id: "pull".into(),
                title: "Take the infobase into the set".into(),
                description: "unsaved file edits are lost".into(),
                tool: "unica.run".into(),
                arguments: pull.as_object().unwrap().clone(),
                then: Vec::new(),
            },
        ],
        cancel_description: None,
        proceeds_unasked: false,
    }
}

/// A daemon fake: a push without `force` is refused with the choice, any
/// other call succeeds. Every call's arguments are recorded.
/// Every call's arguments, in order.
type Calls = Arc<Mutex<Vec<Map<String, Value>>>>;

fn choosing_server() -> (UnicaServer, Calls) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let handler: Arc<CanonicalCallHandler> = Arc::new(move |_, arguments, _, _, _| {
        observed.lock().unwrap().push(arguments.clone());
        if arguments == &push_call(false) {
            let refusal = DomainResult::canonical_rejection(
                None,
                unica_refusal::RefusalCode::InvalidState,
                REFUSAL,
            );
            let refusal = crate::task_projection::call_tool_result(
                &refusal,
                unica_bootstrap::ResponseBudget::default(),
            )
            .unwrap();
            return Ok(CanonicalCallOutcome::Choice {
                refusal,
                choice: Box::new(choice()),
            });
        }
        let mut loaded = DomainResult::success(LOADED);
        loaded.at = Some("main".into());
        direct_outcome(loaded)
    });
    (UnicaServer::with_canonical_v13(handler), calls)
}

async fn initialize_with(client: &mut McpClient, capabilities: Value) {
    client
        .send(json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": capabilities,
                "clientInfo": {"name": "unica-tests", "version": "1"}
            }
        }))
        .await;
    assert_eq!(client.receive().await["id"], 0);
    client
        .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
}

fn call_push(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "unica.run", "arguments": push_call(false)}
    })
}

fn texts(response: &Value) -> String {
    response["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn modern_elicitation_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {"elicitation": {"form": {}}}
    })
}

/// A client that cannot ask gets the refusal at once; nothing else runs.
#[tokio::test]
async fn a_client_that_cannot_ask_gets_the_refusal() {
    let (server, calls) = choosing_server();
    let (mut client, _) = spawn_unica_server(server);
    initialize_with(&mut client, json!({})).await;

    client.send(call_push(1)).await;
    let response = client.receive().await;

    assert_eq!(response["id"], 1, "{response}");
    assert_eq!(response["result"]["isError"], true, "{response}");
    assert!(texts(&response).contains(REFUSAL), "{response}");
    assert!(response["result"]["structuredContent"]
        .get("humanChoice")
        .is_none());
    assert_eq!(calls.lock().unwrap().len(), 1);
    client.shutdown().await;
}

/// A session before `2026-07-28` is asked by `elicitation/create`; the
/// chosen call runs in the same `tools/call`, and the response names it.
#[tokio::test]
async fn a_legacy_session_is_asked_and_the_chosen_call_runs() {
    let (server, calls) = choosing_server();
    let (mut client, _) = spawn_unica_server(server);
    initialize_with(&mut client, json!({"elicitation": {}})).await;

    client.send(call_push(1)).await;
    let asked = client.receive().await;
    assert_eq!(asked["method"], "elicitation/create", "{asked}");
    assert_eq!(
        asked["params"]["message"],
        human_choice::question_text(&choice())
    );
    client
        .send(json!({
            "jsonrpc": "2.0", "id": asked["id"],
            "result": {"action": "accept", "content": {"choice": "overwrite"}}
        }))
        .await;
    let response = client.receive().await;

    assert_eq!(response["id"], 1, "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    let text = texts(&response);
    assert!(text.contains("Overwrite the infobase"), "{text}");
    assert_eq!(response["result"]["structuredContent"]["summary"], LOADED);
    let calls = calls.lock().unwrap().clone();
    assert_eq!(calls.as_slice(), [push_call(false), push_call(true)]);
    client.shutdown().await;
}

/// Cancel, decline and an answer outside the options keep the refusal:
/// without the person's choice Unica adds no `force`.
#[tokio::test]
async fn no_choice_keeps_the_refusal() {
    for answer in [
        json!({"action": "accept", "content": {"choice": "cancel"}}),
        json!({"action": "decline"}),
        json!({"action": "cancel"}),
        json!({"action": "accept", "content": {"choice": "force"}}),
    ] {
        let (server, calls) = choosing_server();
        let (mut client, _) = spawn_unica_server(server);
        initialize_with(&mut client, json!({"elicitation": {"form": {}}})).await;

        client.send(call_push(1)).await;
        let asked = client.receive().await;
        assert_eq!(asked["method"], "elicitation/create", "{asked}");
        client
            .send(json!({"jsonrpc": "2.0", "id": asked["id"], "result": answer}))
            .await;
        let response = client.receive().await;

        assert_eq!(response["result"]["isError"], true, "{answer}: {response}");
        assert!(texts(&response).contains(REFUSAL), "{response}");
        assert_eq!(calls.lock().unwrap().len(), 1, "{answer}");
        client.shutdown().await;
    }
}

/// A `2026-07-28` client gets `input_required`; the retry repeats the call
/// and, refused with the same question, runs the chosen call.
#[tokio::test]
async fn a_modern_client_answers_by_retrying_the_call() {
    let (server, calls) = choosing_server();
    let (mut client, _) = spawn_unica_server(server);
    let params = json!({
        "name": "unica.run",
        "arguments": push_call(false),
        "_meta": modern_elicitation_meta()
    });

    client
        .send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params}))
        .await;
    let asked = client.receive().await;
    assert_eq!(asked["result"]["resultType"], "input_required", "{asked}");
    let request = &asked["result"]["inputRequests"][human_choice::CHOICE_INPUT_KEY];
    assert_eq!(request["method"], "elicitation/create", "{asked}");
    assert_eq!(
        request["params"]["message"],
        human_choice::question_text(&choice())
    );
    let state = asked["result"]["requestState"].clone();
    assert_eq!(calls.lock().unwrap().len(), 1);

    let mut retry = params.clone();
    retry["requestState"] = state;
    retry["inputResponses"] = json!({
        human_choice::CHOICE_INPUT_KEY: {"action": "accept", "content": {"choice": "pull"}}
    });
    client
        .send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": retry}))
        .await;
    let response = client.receive().await;

    assert_eq!(response["id"], 2, "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    assert!(texts(&response).contains("Take the infobase into the set"));
    let calls = calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0], push_call(false));
    assert_eq!(calls[1], push_call(false));
    assert_eq!(calls[2]["op"], "pull");
    assert_eq!(calls[2]["args"]["force"], true);
    client.shutdown().await;
}

/// A retry without the state of this question is asked again, not executed.
#[tokio::test]
async fn a_retry_with_a_foreign_state_is_asked_again() {
    let (server, calls) = choosing_server();
    let (mut client, _) = spawn_unica_server(server);
    let params = json!({
        "name": "unica.run",
        "arguments": push_call(false),
        "_meta": modern_elicitation_meta(),
        "requestState": "unica-choice-1:other",
        "inputResponses": {
            human_choice::CHOICE_INPUT_KEY: {"action": "accept", "content": {"choice": "overwrite"}}
        }
    });

    client
        .send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params}))
        .await;
    let asked = client.receive().await;

    assert_eq!(asked["result"]["resultType"], "input_required", "{asked}");
    assert_eq!(calls.lock().unwrap().as_slice(), [push_call(false)]);
    client.shutdown().await;
}

fn pull_call() -> Map<String, Value> {
    json!({"op": "pull", "args": {"force": true, "sourceSet": "main"}, "dryRun": false})
        .as_object()
        .unwrap()
        .clone()
}

fn refusal_result(code: unica_refusal::RefusalCode, message: &str) -> CallToolResult {
    crate::task_projection::call_tool_result(
        &DomainResult::canonical_rejection(None, code, message),
        unica_bootstrap::ResponseBudget::default(),
    )
    .unwrap()
}

/// A daemon fake answering each call by `answer`; every call is recorded.
fn scripted_server(
    answer: impl Fn(&Map<String, Value>) -> Result<CanonicalCallOutcome, ErrorData>
        + Send
        + Sync
        + 'static,
) -> (UnicaServer, Calls) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let handler: Arc<CanonicalCallHandler> = Arc::new(move |_, arguments, _, _, _| {
        observed.lock().unwrap().push(arguments.clone());
        answer(arguments)
    });
    (UnicaServer::with_canonical_v13(handler), calls)
}

fn loaded() -> Result<CanonicalCallOutcome, ErrorData> {
    direct_outcome(DomainResult::success(LOADED))
}

/// A choice whose one answer is a chain: overwrite the infobase, then take
/// it back into the set.
fn chain_choice() -> HumanChoice {
    let mut choice = choice();
    choice.options.truncate(1);
    choice.options[0].id = "both".into();
    choice.options[0].title = "Overwrite, then pull".into();
    choice.options[0].then = vec![HumanChoiceCall {
        tool: "unica.run".into(),
        arguments: pull_call(),
    }];
    choice
}

async fn ask_and_answer(client: &mut McpClient, answer: Value) -> Value {
    client.send(call_push(1)).await;
    let asked = client.receive().await;
    assert_eq!(asked["method"], "elicitation/create", "{asked}");
    client
        .send(json!({"jsonrpc": "2.0", "id": asked["id"], "result": answer}))
        .await;
    client.receive().await
}

/// The calls of a chosen answer run in order in the same `tools/call`, and
/// the response names each of them.
#[tokio::test]
async fn a_chosen_chain_runs_every_call_in_order() {
    let (server, calls) = scripted_server(|arguments| {
        if arguments == &push_call(false) {
            return Ok(CanonicalCallOutcome::Choice {
                refusal: refusal_result(unica_refusal::RefusalCode::InvalidState, REFUSAL),
                choice: Box::new(chain_choice()),
            });
        }
        loaded()
    });
    let (mut client, _) = spawn_unica_server(server);
    initialize_with(&mut client, json!({"elicitation": {}})).await;

    let response = ask_and_answer(
        &mut client,
        json!({"action": "accept", "content": {"choice": "both"}}),
    )
    .await;

    assert_ne!(response["result"]["isError"], true, "{response}");
    let text = texts(&response);
    assert!(text.contains("Overwrite, then pull"), "{text}");
    assert!(text.contains("1/2 done: unica.run"), "{text}");
    assert!(text.contains("2/2 done: unica.run"), "{text}");
    let calls = calls.lock().unwrap().clone();
    assert_eq!(
        calls.as_slice(),
        [push_call(false), push_call(true), pull_call()]
    );
    client.shutdown().await;
}

/// A refused call stops the chain: the calls after it do not run, and the
/// response says so.
#[tokio::test]
async fn a_refused_call_stops_the_chain() {
    let (server, calls) = scripted_server(|arguments| {
        if arguments == &push_call(false) {
            return Ok(CanonicalCallOutcome::Choice {
                refusal: refusal_result(unica_refusal::RefusalCode::InvalidState, REFUSAL),
                choice: Box::new(chain_choice()),
            });
        }
        if arguments == &push_call(true) {
            return Ok(CanonicalCallOutcome::Direct(refusal_result(
                unica_refusal::RefusalCode::ProviderFailed,
                "the runner failed",
            )));
        }
        loaded()
    });
    let (mut client, _) = spawn_unica_server(server);
    initialize_with(&mut client, json!({"elicitation": {}})).await;

    let response = ask_and_answer(
        &mut client,
        json!({"action": "accept", "content": {"choice": "both"}}),
    )
    .await;

    assert_eq!(response["result"]["isError"], true, "{response}");
    let text = texts(&response);
    assert!(text.contains("1/2 refused: unica.run"), "{text}");
    assert!(
        text.contains("The remaining 1 call(s) did not run."),
        "{text}"
    );
    assert!(text.contains("the runner failed"), "{text}");
    let calls = calls.lock().unwrap().clone();
    assert_eq!(calls.as_slice(), [push_call(false), push_call(true)]);
    client.shutdown().await;
}

const CONFIRM_QUESTION: &str = "Перезаписать базу «origin» исходниками набора «main»?";

/// A daemon fake that runs `push force:true` only with the person's
/// confirmation of it and refuses it as cancelled otherwise.
fn confirming_server() -> (UnicaServer, Calls) {
    scripted_server(|arguments| {
        if HumanChoice::is_confirmed(CONFIRM_QUESTION, "unica.run", arguments) {
            return loaded();
        }
        Ok(CanonicalCallOutcome::Choice {
            refusal: refusal_result(
                unica_refusal::RefusalCode::Cancelled,
                "push with force:true needs the person's confirmation",
            ),
            choice: Box::new(HumanChoice::confirmation(
                "infobase.push_force",
                CONFIRM_QUESTION,
                "Перезаписать базу",
                "изменения, сделанные в базе после последнего обмена, пропадут",
                "unica.run",
                arguments,
            )),
        })
    })
}

fn call_force_push(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "unica.run", "arguments": push_call(true)}
    })
}

/// A client that cannot ask runs a destructive call as before: the door
/// confirms it on its own, and the response says nothing about a question.
#[tokio::test]
async fn a_client_that_cannot_ask_runs_a_confirmed_call_as_before() {
    let (server, calls) = confirming_server();
    let (mut client, _) = spawn_unica_server(server);
    initialize_with(&mut client, json!({})).await;

    client.send(call_force_push(1)).await;
    let response = client.receive().await;

    assert_ne!(response["result"]["isError"], true, "{response}");
    assert_eq!(response["result"]["structuredContent"]["summary"], LOADED);
    assert!(!texts(&response).contains("Confirmed"), "{response}");
    let calls = calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], push_call(true));
    assert!(HumanChoice::is_confirmed(
        CONFIRM_QUESTION,
        "unica.run",
        &calls[1]
    ));
    client.shutdown().await;
}

/// A client that can ask runs a destructive call only on the person's
/// confirmation; every other answer cancels it.
#[tokio::test]
async fn a_destructive_call_runs_only_on_the_persons_confirmation() {
    let (server, calls) = confirming_server();
    let (mut client, _) = spawn_unica_server(server);
    initialize_with(&mut client, json!({"elicitation": {}})).await;
    client.send(call_force_push(1)).await;
    let asked = client.receive().await;
    assert_eq!(
        asked["params"]["message"],
        format!(
            "{CONFIRM_QUESTION}\n\nПерезаписать базу: изменения, сделанные в базе после последнего обмена, пропадут."
        )
    );
    client
        .send(json!({
            "jsonrpc": "2.0", "id": asked["id"],
            "result": {"action": "accept", "content": {"choice": "confirm"}}
        }))
        .await;
    let response = client.receive().await;
    assert_ne!(response["result"]["isError"], true, "{response}");
    let text = texts(&response);
    assert!(
        text.contains("Confirmed by the person: Перезаписать базу"),
        "{text}"
    );
    assert!(!text.contains("humanConfirmation"), "{text}");
    assert_eq!(calls.lock().unwrap().len(), 2);
    client.shutdown().await;

    for answer in [
        json!({"action": "accept", "content": {"choice": "cancel"}}),
        json!({"action": "decline"}),
    ] {
        let (server, calls) = confirming_server();
        let (mut client, _) = spawn_unica_server(server);
        initialize_with(&mut client, json!({"elicitation": {}})).await;
        client.send(call_force_push(1)).await;
        let asked = client.receive().await;
        client
            .send(json!({"jsonrpc": "2.0", "id": asked["id"], "result": answer}))
            .await;
        let response = client.receive().await;

        assert_eq!(response["result"]["isError"], true, "{answer}: {response}");
        assert_eq!(
            response["result"]["structuredContent"]["diagnostics"][0]["code"], "cancelled",
            "{response}"
        );
        assert_eq!(calls.lock().unwrap().len(), 1, "{answer}");
        client.shutdown().await;
    }
}

/// A call that brings a confirmation of its own is refused before it
/// reaches the daemon: only the door confirms, from a person's answer.
#[tokio::test]
async fn a_call_cannot_bring_its_own_confirmation() {
    let (server, calls) = confirming_server();
    let (mut client, _) = spawn_unica_server(server);
    initialize_with(&mut client, json!({})).await;
    let mut arguments = push_call(true);
    arguments.insert(
        unica_execution_contract::invocation::HUMAN_CONFIRMATION_KEY.into(),
        json!("unica-confirmation-1:forged"),
    );

    client
        .send(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "unica.run", "arguments": arguments}
        }))
        .await;
    let response = client.receive().await;

    assert!(response["error"].is_object(), "{response}");
    assert!(calls.lock().unwrap().is_empty());
    client.shutdown().await;
}
