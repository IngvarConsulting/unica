use super::{registration_response, serve_rdbg, workspace, MAIN};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);
const TARGET: &str = "00000000-0000-4000-8000-000000000042";
const PROBE: &str = "f3317000-0000-4000-8000-000000000002";
const PROCESSOR: &str = "f3317000-0000-4000-8000-000000000003";
const HIDDEN: &str = "f3317000-0000-4000-8000-000000000099";
const COMMON_ROLE: &str = "d5963243-262e-4398-b4d7-fb16d06484f6";
const OBJECT_ROLE: &str = "a637f77f-3840-441d-a1c3-699c8c5cb7e0";

fn step_workspace() -> tempfile::TempDir {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/acceptance/workspace-resolve-resources/src");
    let manifest = format!("{MAIN}  - {{name: ext, type: EXTENSION, path: ext}}\n");
    let configuration = fs::read_to_string(fixture.join("Configuration.xml")).unwrap();
    let configuration = configuration.replace(
        "</ChildObjects>",
        "<CommonModule>Hidden</CommonModule></ChildObjects>",
    );
    let root = workspace(&manifest, &configuration);
    let main = root.path().join("src");
    let extension = root.path().join("ext");
    fs::create_dir(&extension).unwrap();
    fs::write(
        extension.join("Configuration.xml"),
        configuration.replace("ResourceResolve", "ProbeExtension"),
    )
    .unwrap();
    for dir in [&main, &extension] {
        for path in [
            "CommonModules/Probe.xml",
            "CommonModules/Probe/Ext/Module.bsl",
        ] {
            let dest = dir.join(path);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::copy(fixture.join(path), dest).unwrap();
        }
    }
    for path in [
        "DataProcessors/Processor.xml",
        "DataProcessors/Processor/Ext/ObjectModule.bsl",
    ] {
        let dest = main.join(path);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(fixture.join(path), dest).unwrap();
    }
    // Registered metadata with no saved BSL models the source evidence available to Unica.
    let hidden = fs::read_to_string(fixture.join("CommonModules/Probe.xml"))
        .unwrap()
        .replace(PROBE, HIDDEN)
        .replace(">Probe<", ">Hidden<");
    fs::write(main.join("CommonModules/Hidden.xml"), hidden).unwrap();
    root
}

struct Peer {
    address: SocketAddr,
    events: Sender<String>,
    actions: Receiver<String>,
    server: Option<thread::JoinHandle<Vec<String>>>,
}

impl Peer {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (events, pending) = mpsc::channel();
        let (seen, actions) = mpsc::channel();
        let server = thread::spawn(move || {
            serve_rdbg(listener, |command, body| match command {
                "pingDebugUIParams" => pending.try_recv().unwrap_or_default(),
                "attachDetachDbgTargets" => "<response><result>true</result></response>".into(),
                "step" => {
                    let doc = roxmltree::Document::parse(body).unwrap();
                    let action = doc
                        .descendants()
                        .find(|node| node.has_tag_name("action"))
                        .unwrap();
                    assert!(doc
                        .descendants()
                        .any(|node| node.has_tag_name("id") && node.text() == Some(TARGET)));
                    seen.send(action.text().unwrap().to_owned()).unwrap();
                    "<response/>".into()
                }
                _ => registration_response(command),
            })
        });
        Self {
            address,
            events,
            actions,
            server: Some(server),
        }
    }

    fn stop(&self, stack: &[String]) {
        self.events.send(format!("<response><result><cmdID>callStackFormed</cmdID><targetID><id>{TARGET}</id><targetType>ManagedClient</targetType></targetID>{}</result></response>", stack.concat())).unwrap();
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.server.take() {
            let result = handle.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

struct Dap {
    child: Child,
    messages: Receiver<Value>,
    seq: u64,
}

impl Dap {
    fn open(root: &Path, port: u16) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unica"))
            .arg("dap")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (send, messages) = mpsc::channel();
        thread::spawn(move || {
            let mut header = String::new();
            while output.read_line(&mut header).unwrap() != 0 {
                let n: usize = header
                    .trim()
                    .strip_prefix("Content-Length: ")
                    .unwrap()
                    .parse()
                    .unwrap();
                header.clear();
                output.read_line(&mut header).unwrap();
                assert_eq!(header, "\r\n");
                header.clear();
                let mut body = vec![0; n];
                output.read_exact(&mut body).unwrap();
                if send.send(serde_json::from_slice(&body).unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut dap = Self {
            child,
            messages,
            seq: 0,
        };
        dap.request("initialize", json!({}));
        dap.request(
            "attach",
            json!({"rootProject": root, "debugServerPort": port}),
        );
        dap.request("configurationDone", json!({}));
        dap
    }

    fn receive(&self, matches: impl Fn(&Value) -> bool) -> Value {
        loop {
            let msg = self
                .messages
                .recv_timeout(TIMEOUT)
                .expect("timed out waiting for DAP");
            assert_ne!(msg["event"], "terminated", "{msg}");
            if msg["event"] == "output" {
                eprintln!("{}", msg["body"]);
            }
            if matches(&msg) {
                return msg;
            }
        }
    }

    fn request(&mut self, command: &str, arguments: Value) -> Value {
        self.seq += 1;
        let message =
            json!({"type": "request", "seq": self.seq, "command": command, "arguments": arguments})
                .to_string();
        let input = self.child.stdin.as_mut().unwrap();
        write!(input, "Content-Length: {}\r\n\r\n{message}", message.len()).unwrap();
        input.flush().unwrap();
        let reply = self.receive(|msg| msg["request_seq"] == self.seq);
        assert_eq!(reply["success"], true, "{reply}");
        reply["body"].clone()
    }

    fn stopped_stack(&mut self, reason: &str, expected: &[(Option<&str>, u32)]) -> u64 {
        let stopped = self.receive(|msg| msg["event"] == "stopped");
        assert_eq!(stopped["body"]["reason"], reason, "{stopped}");
        let id = stopped["body"]["threadId"].as_u64().unwrap();
        let body = self.request("stackTrace", json!({"threadId":id}));
        let frames = body["stackFrames"].as_array().unwrap();
        assert_eq!(frames.len(), expected.len(), "{body}");
        for (frame, &(path, line)) in frames.iter().zip(expected) {
            assert_eq!(frame["source"]["path"].as_str(), path, "{frame}");
            assert_eq!(frame["line"], line, "{frame}");
        }
        id
    }
}

impl Drop for Dap {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn module(kind_fields: &str, object: &str, property: &str, presentation: &str) -> String {
    format!("<moduleID>{kind_fields}<objectID>{object}</objectID><propertyID>{property}</propertyID></moduleID><presentation>{presentation}</presentation>")
}

fn frame(module: &str, line: u32) -> String {
    format!("<callStack>{module}<lineNo>{line}</lineNo></callStack>")
}

#[test]
fn step_in_and_out_preserve_sources_across_module_boundaries() {
    let root = step_workspace();
    let peer = Peer::start();
    let mut dap = Dap::open(root.path(), peer.address.port());
    // 1C 8.5.4.2028 sends a Base64-encoded presentation and omits type for the base configuration.
    // ОбщийМодуль.Probe.Модуль.Run()
    let cfg = module(
        "",
        PROBE,
        COMMON_ROLE,
        "0J7QsdGJ0LjQudCc0L7QtNGD0LvRjC5Qcm9iZS7QnNC+0LTRg9C70YwuUnVuKCk=",
    );
    // ProbeExtension ОбщийМодуль.Probe.Модуль.Entry()
    let ext = module(
        "<type>ExtensionModule</type><extensionName>ProbeExtension</extensionName>",
        PROBE,
        COMMON_ROLE,
        "UHJvYmVFeHRlbnNpb24g0J7QsdGJ0LjQudCc0L7QtNGD0LvRjC5Qcm9iZS7QnNC+0LTRg9C70YwuRW50cnkoKQ==",
    );
    // Обработка.Processor.МодульОбъекта.Entry()
    let processor = module(
        "",
        PROCESSOR,
        OBJECT_ROLE,
        "0J7QsdGA0LDQsdC+0YLQutCwLlByb2Nlc3Nvci7QnNC+0LTRg9C70YzQntCx0YrQtdC60YLQsC5FbnRyeSgp",
    );
    let hidden = module("", HIDDEN, COMMON_ROLE, "UGluZygp");
    // The external module deliberately collides with a saved configuration processor.
    let external = module(
        "<type>ExtMDModule</type><URL>file:///unsaved.epf</URL>",
        PROCESSOR,
        OBJECT_ROLE,
        "UGluZygp",
    );
    let external_url_only =
        "<moduleID><type>ExtMDModule</type><URL>file:///unsaved.epf</URL></moduleID><presentation>UGluZygp</presentation>";
    let incomplete = format!("<callStack><moduleID><type>ConfigModule</type><objectID>{PROBE}</objectID></moduleID><presentation>UGluZygp</presentation></callStack>");
    let base = Some("main:CommonModule.Probe");
    let overlay = Some("ext:CommonModule.Probe");
    let process = Some("main:DataProcessor.Processor.Module.Object");
    peer.stop(&[frame(&cfg, 4)]);
    let thread_id = dap.stopped_stack("pause", &[(base, 4)]);

    let cases = [
        (
            "stepIn",
            vec![frame(&cfg, 4), frame(&ext, 7)],
            vec![(overlay, 7), (base, 4)],
        ),
        ("stepOut", vec![frame(&cfg, 5)], vec![(base, 5)]),
        (
            "stepIn",
            vec![frame(&cfg, 5), frame(&processor, 8)],
            vec![(process, 8), (base, 5)],
        ),
        (
            "stepIn",
            vec![frame(&cfg, 5), frame(&processor, 8), frame(&cfg, 18)],
            vec![(base, 18), (process, 8), (base, 5)],
        ),
        (
            "stepOut",
            vec![frame(&cfg, 5), frame(&processor, 9)],
            vec![(process, 9), (base, 5)],
        ),
        ("stepOut", vec![frame(&cfg, 6)], vec![(base, 6)]),
        (
            "stepIn",
            vec![frame(&cfg, 6), frame(&external, 9)],
            vec![(None, 9), (base, 6)],
        ),
        ("stepOut", vec![frame(&cfg, 7)], vec![(base, 7)]),
        // A registered module with unavailable BSL can sit between two visible frames.
        (
            "stepIn",
            vec![frame(&cfg, 7), frame(&hidden, 10), frame(&processor, 11)],
            vec![(process, 11), (None, 10), (base, 7)],
        ),
        (
            "stepOut",
            vec![frame(&cfg, 7), frame(&hidden, 12)],
            vec![(None, 12), (base, 7)],
        ),
        ("stepOut", vec![frame(&cfg, 8)], vec![(base, 8)]),
        // Preserve a reported opaque frame, and a visible stack if the peer omits it entirely.
        (
            "stepIn",
            vec![frame(&cfg, 8), incomplete, frame(&processor, 13)],
            vec![(process, 13), (None, 0), (base, 8)],
        ),
        ("stepOut", vec![frame(&cfg, 9)], vec![(base, 9)]),
        (
            "stepIn",
            vec![frame(&cfg, 9), frame(external_url_only, 14)],
            vec![(None, 14), (base, 9)],
        ),
        ("stepOut", vec![frame(&cfg, 10)], vec![(base, 10)]),
    ];
    for (command, stack, expected) in cases {
        dap.request(command, json!({"threadId": thread_id}));
        let action = peer.actions.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            action,
            if command == "stepIn" {
                "StepIn"
            } else {
                "StepOut"
            }
        );
        peer.stop(&stack);
        assert_eq!(dap.stopped_stack("step", &expected), thread_id);
    }
    dap.request("disconnect", json!({}));
}
