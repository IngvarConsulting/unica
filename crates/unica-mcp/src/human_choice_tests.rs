//! The door asks a person the choice a refusal offers, through `tools/call`
//! on the wire: a client without form elicitation gets the refusal, a
//! session before `2026-07-28` gets an `elicitation/create` request and a
//! `2026-07-28` client an `input_required` result.
//!
//! rule: INV.RUNTIME.RUNNER-BASE-REFUSALS

use super::*;
use crate::test_support::*;
use serde_json::json;
use unica_execution_contract::invocation::{DomainResult, HumanChoice, HumanChoiceOption};

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
        question: "push of source set `main` loaded nothing".into(),
        options: vec![
            HumanChoiceOption {
                id: "overwrite".into(),
                title: "Overwrite the infobase".into(),
                description: "infobase changes are lost".into(),
                tool: "unica.run".into(),
                arguments: push_call(true),
            },
            HumanChoiceOption {
                id: "pull".into(),
                title: "Take the infobase into the set".into(),
                description: "unsaved file edits are lost".into(),
                tool: "unica.run".into(),
                arguments: pull.as_object().unwrap().clone(),
            },
        ],
    }
}

/// A daemon fake: a push without `force` is refused with the choice, any
/// other call succeeds. Every call's arguments are recorded.
fn choosing_server() -> (UnicaServer, Arc<Mutex<Vec<Map<String, Value>>>>) {
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
    assert_eq!(asked["params"]["message"], human_choice::question_text(&choice()));
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
    let calls = calls.lock().unwrap();
    assert_eq!(calls.as_slice(), [push_call(false), push_call(true)]);
    drop(calls);
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
    assert_eq!(request["params"]["message"], human_choice::question_text(&choice()));
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
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0], push_call(false));
    assert_eq!(calls[1], push_call(false));
    assert_eq!(calls[2]["op"], "pull");
    assert_eq!(calls[2]["args"]["force"], true);
    drop(calls);
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
