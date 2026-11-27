use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{fs, thread};

use serde_json::{json, Value};

#[path = "dap/steps.rs"]
mod steps;

const EMPTY_CONFIGURATION: &str = include_str!(
    "../../../tests/fixtures/formats/platform_xml/2.20/empty_configuration/src/Configuration.xml"
);

/// An empty connection wakes the listener after the adapter has exited.
fn serve_rdbg(
    listener: TcpListener,
    expected_host: &str,
    mut respond: impl FnMut(&str, &str) -> String,
) -> Vec<String> {
    serve_dbgs_http(listener, expected_host, |cmd, body| {
        http_ok(&respond(cmd, body))
    })
}

fn http_ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

fn serve_dbgs_http(
    listener: TcpListener,
    expected_host: &str,
    mut respond: impl FnMut(&str, &str) -> String,
) -> Vec<String> {
    let authority = format!("{expected_host}:{}", listener.local_addr().unwrap().port());
    let mut commands = Vec::new();
    for request in listener.incoming() {
        let mut socket = request.unwrap();
        let mut reader = BufReader::new(&socket);
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let target = line.split_whitespace().nth(1).unwrap();
        let url = url::Url::parse("http://localhost")
            .unwrap()
            .join(target)
            .unwrap();
        assert_eq!(url.path(), "/e1crdbg/rdbg");
        let cmd = url
            .query_pairs()
            .find(|(key, _)| key == "cmd")
            .unwrap()
            .1
            .into_owned();
        let mut length = 0;
        let mut host = None;
        loop {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            } else if name.eq_ignore_ascii_case("host") {
                host = Some(value.trim().to_owned());
            }
        }
        assert_eq!(host.as_deref(), Some(authority.as_str()));
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let response = respond(&cmd, std::str::from_utf8(&body).unwrap());
        commands.push(cmd);
        socket.write_all(response.as_bytes()).unwrap();
    }
    commands
}

fn registration_response(command: &str) -> String {
    if command == "attachDebugUI" {
        "<response><result>registered</result></response>".into()
    } else {
        "<response/>".into()
    }
}

fn attach(root: &Path, set: &str) -> (Vec<Value>, Vec<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        serve_rdbg(listener, "127.0.0.1", |cmd, _| registration_response(cmd))
    });
    let frames = dap_attach(
        &[],
        json!({"rootProject": root, "sourceSet": set, "debugServerPort": address.port()}),
    );
    drop(TcpStream::connect(address).unwrap());
    (frames, server.join().unwrap())
}

fn dap_attach(cli_args: &[&str], arguments: Value) -> Vec<Value> {
    let mut dap = Command::new(env!("CARGO_BIN_EXE_unica"))
        .arg("dap")
        .args(cli_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = dap.stdin.take().unwrap();
    for (i, (command, arguments)) in [
        ("initialize", json!({})),
        ("attach", arguments),
        ("disconnect", json!({})),
    ]
    .into_iter()
    .enumerate()
    {
        let message =
            json!({"type":"request", "seq":i + 1, "command":command, "arguments":arguments})
                .to_string();
        write!(stdin, "Content-Length: {}\r\n\r\n{message}", message.len()).unwrap();
    }
    drop(stdin);
    let output = dap.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut frames = Vec::new();
    let mut remaining = output.stdout.as_slice();
    while !remaining.is_empty() {
        let mut length = String::new();
        remaining.read_line(&mut length).unwrap();
        let n = length
            .trim()
            .strip_prefix("Content-Length: ")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        remaining = remaining.strip_prefix(b"\r\n").unwrap();
        let (body, rest) = remaining.split_at(n);
        frames.push(serde_json::from_slice(body).unwrap());
        remaining = rest;
    }
    frames
}

fn workspace(manifest: &str, configuration: &str) -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("v8project.yaml"), manifest).unwrap();
    fs::write(
        workspace.path().join("src/Configuration.xml"),
        configuration,
    )
    .unwrap();
    workspace
}

const MAIN: &str =
    "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n";

fn assert_registered(frames: &[Value], commands: &[String]) {
    let reply = frames
        .iter()
        .find(|frame| frame["request_seq"] == 2)
        .unwrap();
    assert_eq!(reply["success"], true, "{reply}");
    assert!(frames.iter().any(|frame| frame["event"] == "initialized"));
    assert_eq!(
        commands,
        ["attachDebugUI", "setAutoAttachSettings", "detachDebugUI"]
    );
}

#[test]
fn attach_registers_with_dbgs_for_a_configuration_without_bsl() {
    let root = workspace(MAIN, EMPTY_CONFIGURATION);
    let (frames, commands) = attach(root.path(), "main");
    assert_registered(&frames, &commands);
}

#[test]
fn attach_defers_module_validation_until_a_source_is_requested() {
    let root = workspace(
        MAIN,
        include_str!(
            "../../../tests/fixtures/acceptance/workspace-resolve-resources/src/Configuration.xml"
        ),
    );
    let src = root.path().join("src");
    fs::create_dir_all(src.join("CommonModules/Probe/Ext")).unwrap();
    fs::write(src.join("CommonModules/Probe.xml"), "<broken").unwrap();
    fs::write(src.join("CommonModules/Probe/Ext/Module.bsl"), include_str!(
        "../../../tests/fixtures/acceptance/workspace-resolve-resources/src/CommonModules/Probe/Ext/Module.bsl"
    )).unwrap();
    let (frames, commands) = attach(root.path(), "main");
    assert_registered(&frames, &commands);
}

#[test]
fn unknown_source_set_fails_before_dbgs_registration() {
    let root = workspace(MAIN, EMPTY_CONFIGURATION);
    let (frames, commands) = attach(root.path(), "missing");
    let reply = frames
        .iter()
        .find(|frame| frame["request_seq"] == 2)
        .unwrap();
    assert_eq!(reply["success"], false, "{reply}");
    assert!(
        reply["message"].as_str().unwrap().contains("missing"),
        "{reply}"
    );
    assert!(!frames.iter().any(|frame| frame["event"] == "initialized"));
    assert!(commands.is_empty(), "{commands:?}");
}

#[test]
fn attach_registers_with_dbgs_for_a_saved_extension() {
    let root = workspace(
        include_str!("../../../tests/fixtures/acceptance/workspace-extension-create-language/v8project.yaml"),
        include_str!("../../../tests/fixtures/acceptance/workspace-extension-create-language/src/Configuration.xml"),
    );
    let (frames, commands) = attach(root.path(), "YAXUNIT");
    assert_registered(&frames, &commands);
}

fn request_ui_id(body: &str, expected_alias: &str) -> uuid::Uuid {
    let xml = roxmltree::Document::parse(body).unwrap();
    let field = |name: &str| {
        xml.descendants()
            .find(|node| node.has_tag_name(name))
            .and_then(|node| node.text())
            .unwrap()
    };
    assert_eq!(field("infoBaseAlias"), expected_alias);
    uuid::Uuid::parse_str(field("idOfDebuggerUI")).unwrap()
}

#[test]
fn pinned_debug_server_registers_and_detaches_only_its_own_ui() {
    let root = workspace(MAIN, EMPTY_CONFIGURATION);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://localhost:{}", address.port());
    let alias = "База & QA";
    let server = thread::spawn(move || {
        let mut ids = Vec::new();
        let commands = serve_rdbg(listener, "localhost", |cmd, body| {
            ids.push(request_ui_id(body, alias));
            registration_response(cmd)
        });
        (commands, ids)
    });

    for _ in 0..2 {
        let frames = dap_attach(
            &["--debug-server-url", &url],
            json!({"rootProject":root.path(), "debugServerUrl":url, "infoBaseAlias":alias}),
        );
        assert_eq!(frames[0]["command"], "initialize");
        assert_eq!(frames[0]["body"]["supportsUnicaDebugServerUrl"], true);
        assert!(frames.iter().any(|frame| frame["event"] == "initialized"));
        for response in frames.iter().filter(|frame| frame["type"] == "response") {
            assert_eq!(response["success"], true, "{response}");
        }
    }
    drop(TcpStream::connect(address).unwrap());
    let (commands, ids) = server.join().unwrap();
    let expected = ["attachDebugUI", "setAutoAttachSettings", "detachDebugUI"];
    assert_eq!(commands, expected.repeat(2));
    assert_eq!(ids[..3], [ids[0]; 3]);
    assert_eq!(ids[3..], [ids[3]; 3]);
    assert_ne!(ids[0], ids[3]);
}

fn assert_attach_error(frames: &[Value], id: u32) {
    let reply = frames
        .iter()
        .find(|frame| frame["command"] == "attach")
        .unwrap();
    assert_eq!(reply["success"], false, "{reply}");
    assert_eq!(reply["body"]["error"]["id"], id, "{reply}");
    assert!(!frames.iter().any(|frame| frame["event"] == "initialized"));
}

#[test]
fn pinned_debug_server_rejects_conflicting_or_invalid_arguments_before_registration() {
    let root = workspace(MAIN, EMPTY_CONFIGURATION);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://localhost:{}", address.port());
    let alternate = format!("http://{address}");
    // localhost and the numeric address reach our listener, but denote different authorities.
    let server = thread::spawn(move || {
        serve_rdbg(listener, "localhost", |cmd, _| registration_response(cmd))
    });
    let cli_args = ["--debug-server-url", url.as_str()];
    for mut args in [
        json!({"debugServerUrl":alternate}),
        json!({"debugServerUrl":url,"debugServerPort":address.port()}),
        json!({"debugServerUrl":url,"debugServerHost":"localhost"}),
        json!({"debugServerUrl":format!("{url}/unrelated")}),
        json!({"debugServerPort":address.port()}),
    ] {
        args["rootProject"] = json!(root.path());
        assert_attach_error(&dap_attach(&cli_args, args), 32601);
    }
    let unpinned = dap_attach(
        &[],
        json!({"rootProject":root.path(), "debugServerUrl":url}),
    );
    assert_attach_error(&unpinned, 32601);
    drop(TcpStream::connect(address).unwrap());
    assert!(server.join().unwrap().is_empty());
}

#[test]
fn unavailable_pinned_debug_server_reports_a_transport_error() {
    let root = workspace(MAIN, EMPTY_CONFIGURATION);
    let truncated = "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 64\r\n\r\n<response>";
    let alias = "Transport probe";
    for (failing_command, wire) in [
        ("attachDebugUI", ""),
        ("attachDebugUI", truncated),
        ("setAutoAttachSettings", truncated),
    ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://localhost:{}", address.port());
        let server = thread::spawn(move || {
            let mut ids = Vec::new();
            let commands = serve_dbgs_http(listener, "localhost", |cmd, body| {
                ids.push(request_ui_id(body, alias));
                if cmd == failing_command {
                    wire.to_owned()
                } else {
                    http_ok(&registration_response(cmd))
                }
            });
            (commands, ids)
        });
        let frames = dap_attach(
            &["--debug-server-url", &url],
            json!({"rootProject":root.path(), "debugServerUrl":url, "infoBaseAlias":alias}),
        );
        drop(TcpStream::connect(address).unwrap());
        let (commands, ids) = server.join().unwrap();
        let expected: &[&str] = if failing_command == "setAutoAttachSettings" {
            &["attachDebugUI", "setAutoAttachSettings", "detachDebugUI"]
        } else {
            &["attachDebugUI", "detachDebugUI"]
        };
        assert_eq!(commands, expected);
        assert!(ids.iter().all(|id| *id == ids[0]));
        assert_attach_error(&frames, 32602);
    }
}
