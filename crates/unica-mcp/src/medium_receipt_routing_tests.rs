use super::*;
use crate::test_support::{
    assert_v13_profile_names, canonical_snapshot, listed_tool_names, modern_meta,
    modern_tasks_meta, spawn_unica_server,
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use unica_execution_contract::invocation::{DomainResult, InvocationStatus, TaskId};

/// Each transport is a fresh frontend. Only the daemon's handlers and Task
/// identity outlive a connection. A result read must never submit the tool.
async fn check_receipt_after_reconnecting(
    tool: V5ToolIdentity,
    public_name: &str,
    arguments: Value,
    native: bool,
) {
    let id = TaskId::new();
    let submits = Arc::new(AtomicUsize::new(0));
    let observations = Arc::new(AtomicUsize::new(0));
    let submitted = Arc::clone(&submits);
    let expected = arguments.clone();
    let call: Arc<CanonicalCallHandler> = Arc::new(move |actual_tool, args, _, _, _| {
        assert_eq!(actual_tool, tool);
        assert_eq!(Value::Object(args.clone()), expected);
        submitted.fetch_add(1, Ordering::SeqCst);
        Ok(CanonicalCallOutcome::Task(canonical_snapshot(
            id,
            InvocationStatus::Working,
            None,
        )))
    });
    let observed = Arc::clone(&observations);
    let get: Arc<CanonicalTaskHandler> = Arc::new(move |requested, _| {
        assert_eq!(requested, id);
        observed.fetch_add(1, Ordering::SeqCst);
        Ok(canonical_snapshot(
            id,
            InvocationStatus::Completed,
            Some(DomainResult::success("the original operation finished")),
        ))
    });
    let read = Arc::clone(&get);
    let wait: Arc<CanonicalTaskWaitHandler> =
        Arc::new(move |requested, _, deadline, _| read(requested, deadline));
    let cancel: Arc<CanonicalTaskHandler> =
        Arc::new(|_, _| panic!("disconnecting after handoff must not cancel the Task"));
    let connect = || {
        let server = UnicaServer::with_canonical_v13_task_handlers(
            Arc::clone(&call),
            Arc::clone(&get),
            Arc::clone(&wait),
            Arc::clone(&cancel),
        );
        spawn_unica_server(server).0
    };
    let mut client = connect();
    let advertised = if native {
        modern_meta()
    } else {
        modern_tasks_meta()
    };
    let catalog = listed_tool_names(&mut client, 1, Some(advertised)).await;
    assert_v13_profile_names(&catalog, !native);

    let meta = if native {
        modern_tasks_meta()
    } else {
        modern_meta()
    };
    client
        .send(
            json!({"jsonrpc":"2.0","id":30,"method":"tools/call","params":{
                "name":public_name,"arguments":arguments,"_meta":meta
            }}),
        )
        .await;
    let answer = client.receive().await;
    assert_eq!(
        observations.load(Ordering::SeqCst),
        0,
        "initial handoff MUST NOT wait once more"
    );
    assert_eq!(submits.load(Ordering::SeqCst), 1);
    if native {
        assert_eq!(answer["result"]["resultType"], "task", "{answer}");
        assert_eq!(answer["result"]["taskId"], id.to_string());
    } else {
        let receipt = &answer["result"]["structuredContent"];
        assert_eq!(
            receipt["data"]["task"]["taskId"],
            id.to_string(),
            "{answer}"
        );
        assert_eq!(receipt["data"]["task"]["status"], "working");
        assert_eq!(receipt["next"][0]["tool"], "unica.task.result");
        assert_eq!(receipt["next"][0]["args"]["taskId"], id.to_string());
    }
    client.shutdown().await;

    let mut reader = connect();
    let catalog = listed_tool_names(&mut reader, 1, Some(meta.clone())).await;
    assert_v13_profile_names(&catalog, native);
    for request_id in [30, 31] {
        if native {
            reader.send(json!({"jsonrpc":"2.0","id":request_id,"method":"tasks/get","params":{"taskId":id.to_string(),"_meta":meta}})).await;
            let reply = reader.receive().await;
            assert_eq!(reply["result"]["status"], "completed", "{reply}");
            assert_eq!(reply["result"]["taskId"], id.to_string());
            assert_eq!(
                reply["result"]["result"]["structuredContent"]["summary"],
                "the original operation finished"
            );
        } else {
            let next = &answer["result"]["structuredContent"]["next"][0];
            assert!(catalog
                .iter()
                .any(|name| Some(name.as_str()) == next["tool"].as_str()));
            reader
                .send(
                    json!({"jsonrpc":"2.0","id":request_id,"method":"tools/call","params":{
                        "name":next["tool"],"arguments":next["args"],"_meta":meta
                    }}),
                )
                .await;
            let reply = reader.receive().await;
            assert_eq!(
                reply["result"]["structuredContent"]["summary"], "the original operation finished",
                "{reply}"
            );
        }
    }
    assert_eq!(submits.load(Ordering::SeqCst), 1);
    assert_eq!(observations.load(Ordering::SeqCst), 2);
    reader.shutdown().await;
}

/// rule: INV.WIRE.NATIVE-TASK-CAPABILITY
#[tokio::test]
async fn modern_without_tasks_hands_off_read_and_build_without_observing_or_resubmitting() {
    check_receipt_after_reconnecting(V5ToolIdentity::Check, "unica.check", json!({}), false).await;
    check_receipt_after_reconnecting(
        V5ToolIdentity::Run,
        "unica.run",
        json!({"op":"make","args":{"output":"dist/main.cf"},"dryRun":false}),
        false,
    )
    .await;
}

/// rule: CTR.WIRE.CANONICAL-TOOL-PROFILES
#[tokio::test]
async fn a_native_call_after_listing_compatibility_tools_still_returns_a_native_task() {
    check_receipt_after_reconnecting(V5ToolIdentity::Check, "unica.check", json!({}), true).await;
}

/// rule: INV.WIRE.NATIVE-TASK-CAPABILITY
#[tokio::test]
async fn modern_push_hands_off_to_a_listed_result_tool() {
    check_receipt_after_reconnecting(
        V5ToolIdentity::Run,
        "unica.run",
        json!({"op":"push","args":{},"dryRun":false}),
        false,
    )
    .await;
}

/// rule: INV.WIRE.NATIVE-TASK-CAPABILITY
#[tokio::test]
async fn modern_quick_push_returns_the_terminal_result_inline() {
    let call: Arc<CanonicalCallHandler> = Arc::new(|tool, args, _, _, _| {
        assert_eq!(tool, V5ToolIdentity::Run);
        assert_eq!(args["op"], "push");
        crate::test_support::direct_outcome(DomainResult::success("push applied"))
    });
    let get: Arc<CanonicalTaskHandler> =
        Arc::new(|_, _| panic!("a quick push already has its result"));
    let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
    let (mut client, _) = spawn_unica_server(server);
    client.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"unica.run","arguments":{"op":"push","args":{},"dryRun":false},"_meta":modern_meta()
    }})).await;
    let answer = client.receive().await;
    assert_eq!(
        answer["result"]["structuredContent"]["summary"], "push applied",
        "{answer}"
    );
    assert!(
        answer["result"]["structuredContent"]["data"]
            .get("task")
            .is_none(),
        "{answer}"
    );
    client.shutdown().await;
}

/// Delivery follows the daemon's disposition even when its Task has already completed.
#[tokio::test]
async fn a_completed_task_handoff_keeps_its_receipt_and_exact_result() {
    let id = TaskId::new();
    let completed = canonical_snapshot(
        id,
        InvocationStatus::Completed,
        Some(DomainResult::success("push applied")),
    );
    let submitted = completed.clone();
    let call: Arc<CanonicalCallHandler> =
        Arc::new(move |_, _, _, _, _| Ok(CanonicalCallOutcome::Task(submitted.clone())));
    let get: Arc<CanonicalTaskHandler> = Arc::new(move |requested, _| {
        assert_eq!(requested, id);
        Ok(completed.clone())
    });
    let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_v13_tasks(
        call,
        Arc::clone(&get),
        get,
    ));
    client
        .send(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
              "name":"unica.run","arguments":{"op":"push","dryRun":false},"_meta":modern_meta()
            }}),
        )
        .await;
    let receipt = client.receive().await;
    assert_eq!(
        receipt["result"]["structuredContent"]["data"]["task"]["taskId"],
        id.to_string(),
        "{receipt}"
    );
    client
        .send(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
              "name":"unica.task.result","arguments":{"taskId":id,"waitMs":0},"_meta":modern_meta()
            }}),
        )
        .await;
    let result = client.receive().await;
    assert_eq!(
        result["result"]["structuredContent"]["summary"], "push applied",
        "{result}"
    );
    client.shutdown().await;
}

/// Cancellation belongs to the observing request. Only the separate cancel
/// tool can request cancellation of the accepted work.
#[tokio::test]
async fn cancelling_a_modern_result_wait_preserves_the_working_task() {
    let id = TaskId::new();
    let entered = Arc::new(tokio::sync::Notify::new());
    let stopped = Arc::new(tokio::sync::Notify::new());
    let signal_entered = entered.clone();
    let signal_stopped = stopped.clone();
    let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |requested, _, _, control| {
        assert_eq!(requested, id);
        signal_entered.notify_one();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !control.token().is_cancelled() {
            assert!(
                Instant::now() < deadline,
                "observation cancellation was not delivered"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        signal_stopped.notify_one();
        Ok(canonical_snapshot(id, InvocationStatus::Working, None))
    });
    let get: Arc<CanonicalTaskHandler> = Arc::new(move |requested, _| {
        assert_eq!(requested, id);
        Ok(canonical_snapshot(id, InvocationStatus::Working, None))
    });
    let server = UnicaServer::with_canonical_v13_task_handlers(
        Arc::new(|_, _, _, _, _| panic!("a read cannot resubmit")),
        get,
        wait,
        Arc::new(|_, _| panic!("a cancelled observation cannot cancel work")),
    );
    let (mut client, _) = spawn_unica_server(server);
    client.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
      "name":"unica.task.result","arguments":{"taskId":id,"waitMs":7000},"_meta":modern_meta()
    }})).await;
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    client.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1,"reason":"stop waiting"}})).await;
    tokio::time::timeout(Duration::from_secs(5), stopped.notified())
        .await
        .unwrap();
    client
        .send(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
              "name":"unica.task.get","arguments":{"taskId":id},"_meta":modern_meta()
            }}),
        )
        .await;
    loop {
        let response = client.receive().await;
        if response["id"] == 2 {
            assert_eq!(
                response["result"]["structuredContent"]["data"]["task"]["status"], "working",
                "{response}"
            );
            break;
        }
    }
    client.shutdown().await;
}
