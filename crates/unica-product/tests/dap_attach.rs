use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{fs, thread};

use serde_json::{json, Value};

const EMPTY_CONFIGURATION: &str = include_str!(
    "../../../tests/fixtures/formats/platform_xml/2.20/empty_configuration/src/Configuration.xml"
);

/// The server records commands actually received from the product. An empty connection
/// wakes the listener when the adapter has exited, including after a rejected attach.
fn attach(root: &Path, set: &str) -> (Vec<Value>, Vec<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
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
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let response = if cmd == "attachDebugUI" {
                "<response><result>registered</result></response>"
            } else {
                "<response/>"
            };
            commands.push(cmd);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
        }
        commands
    });

    let mut dap = Command::new(env!("CARGO_BIN_EXE_unica"))
        .arg("dap")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = dap.stdin.take().unwrap();
    for (i, (command, arguments)) in [
        ("initialize", json!({})),
        (
            "attach",
            json!({"rootProject": root, "sourceSet": set, "debugServerPort": address.port()}),
        ),
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
    drop(TcpStream::connect(address).unwrap());
    let commands = server.join().unwrap();
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
    (frames, commands)
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
