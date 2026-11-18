//! DAP attach sessions for the local 1C debug server.

mod modules;
mod rdbg;

use std::{
    collections::HashMap,
    io::{self, BufRead, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use crossbeam_channel::{bounded, never, select, Receiver, RecvTimeoutError};
use modules::Modules;
use rdbg::{child, field, Rdbg, TargetAttachment};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_source_contract::debug_source::{DebugModuleId, DebugModuleSource};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

type ResolveModules = fn(&Path) -> Result<Vec<DebugModuleSource>>;

const DEFAULT_AUTO_ATTACH_TYPES: [&str; 3] = ["Client", "ManagedClient", "ServerEmulation"];
const DEBUG_TARGET_TYPES: &[&str] = &[
    "Client",
    "ManagedClient",
    "WEBClient",
    "COMConnector",
    "Server",
    "ServerEmulation",
    "WEBService",
    "HTTPService",
    "OData",
    "JOB",
    "JobFileMode",
    "MobileClient",
    "MobileServer",
    "MobileJobFileMode",
    "MobileManagedClient",
    "MobileManagedServer",
];

pub fn run_stdio(resolve_modules: ResolveModules) -> i32 {
    let (tx, rx) = bounded(8);
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            match read_frame(&mut input) {
                Ok(Some(message)) => {
                    if tx.send(Ok(message)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });

    let mut adapter = Adapter {
        output: io::stdout().lock(),
        resolve_modules,
        seq: 0,
        initialized: false,
        session: None,
    };
    if let Err(error) = adapter.run(rx) {
        eprintln!("unica dap: {error}");
        return 1;
    }
    0
}

fn read_frame(input: &mut impl BufRead) -> Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if Read::take(&mut *input, 8193).read_line(&mut line)? == 0 {
            return if length.is_none() {
                Ok(None)
            } else {
                Err("truncated DAP header".into())
            };
        }
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').ok_or("invalid DAP frame header")?;
        if name.eq_ignore_ascii_case("Content-Length")
            && length.replace(value.trim().parse::<usize>()?).is_some()
        {
            return Err("duplicate DAP Content-Length".into());
        }
        if !line.ends_with("\r\n") {
            return Err("invalid DAP frame header terminator".into());
        }
    }
    let length = length
        .filter(|n| *n <= 1024 * 1024)
        .ok_or("invalid DAP Content-Length")?;
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

struct Adapter<W> {
    output: W,
    resolve_modules: ResolveModules,
    seq: u64,
    initialized: bool,
    session: Option<Session>,
}

impl<W: Write> Adapter<W> {
    fn run(&mut self, requests: Receiver<Result<Value>>) -> Result<()> {
        let no_events = never();
        loop {
            let reader = self
                .session
                .as_ref()
                .and_then(|session| session.event_reader.as_ref())
                .unwrap_or(&no_events);
            let command = select! {
                recv(reader) -> reply => {
                    let reply = reply.unwrap_or_else(|_| Err("dbgs event reader stopped".into()));
                    self.handle_events(reply)?;
                    continue;
                }
                recv(requests) -> request => match request {
                    Ok(request) => request?,
                    Err(_) => return Ok(()),
                },
            };
            let name = string(&command, "command")?;
            let mut response = json!({
                "type":"response","request_seq":command["seq"],"command":name,"success":true
            });
            let mut events = Vec::new();
            let reply = self.handle(name, &command["arguments"], &mut events);
            match reply {
                Ok(body) => response["body"] = body,
                Err(error) => {
                    response["success"] = json!(false);
                    response["message"] = json!(error.to_string());
                }
            }
            self.send(response)?;
            self.emit(events)?;
            if name == "disconnect" {
                return Ok(());
            }
        }
    }

    fn handle_events(&mut self, reply: Result<String>) -> Result<()> {
        let mut events = Vec::new();
        let session = self
            .session
            .as_mut()
            .expect("only an active session reads events");
        if let Err(error) = reply.and_then(|xml| session.consume(&xml, &mut events)) {
            self.session = None;
            events = vec![
                event(
                    "output",
                    json!({"category":"stderr","output":format!("{error}\n")}),
                ),
                event("terminated", json!({})),
            ];
        }
        self.emit(events)
    }

    fn handle(&mut self, name: &str, args: &Value, events: &mut Vec<Value>) -> Result<Value> {
        match name {
            "initialize" if !self.initialized => {
                if args["linesStartAt1"] == false || args["columnsStartAt1"] == false {
                    return Err("unica dap uses one-based line and column numbers".into());
                }
                if args["pathFormat"]
                    .as_str()
                    .is_some_and(|format| format != "path")
                {
                    return Err("unica dap expects logical addresses in Source.path".into());
                }
                self.initialized = true;
                Ok(json!({
                    "supportsConfigurationDoneRequest": true,
                    "supportsConditionalBreakpoints": true,
                    "supportsExceptionFilterOptions": true,
                    "supportsExceptionInfoRequest": true,
                    "exceptionBreakpointFilters": [
                        {"filter":"all","label":"Все ошибки 1С","default":false},
                        {"filter":"text","label":"Ошибка 1С по тексту","default":false,
                         "supportsCondition":true,"conditionDescription":"Подстрока в описании ошибки"}
                    ]
                }))
            }
            "attach" if self.initialized && self.session.is_none() => {
                self.session = Some(Session::attach(args, self.resolve_modules)?);
                events.push(event("initialized", json!({})));
                Ok(json!({}))
            }
            "disconnect" => {
                self.session = None;
                Ok(json!({}))
            }
            _ => self
                .session
                .as_mut()
                .ok_or("no active debug session")?
                .request(name, args, events),
        }
    }

    fn emit(&mut self, events: Vec<Value>) -> Result<()> {
        for message in events {
            self.send(message)?;
        }
        Ok(())
    }

    fn send(&mut self, mut message: Value) -> Result<()> {
        self.seq += 1;
        message["seq"] = json!(self.seq);
        let body = serde_json::to_vec(&message)?;
        write!(self.output, "Content-Length: {}\r\n\r\n", body.len())?;
        self.output.write_all(&body)?;
        self.output.flush()?;
        Ok(())
    }
}

#[derive(Clone)]
struct Breakpoint {
    line: u32,
    condition: Option<String>,
}

struct Target {
    id: Uuid,
    frames: Vec<Frame>,
    stepping: bool,
    exception: Option<String>,
}

struct AvailableTarget {
    id: Uuid,
    kind: String,
    name: Option<String>,
    session_number: Option<u64>,
}

struct Frame {
    id: u64,
    level: usize,
    body: Value,
    variable_paths: HashMap<u64, Vec<String>>,
    evaluated_children: HashMap<u64, Vec<ContextVariable>>,
}

struct Session {
    rdbg: Arc<Rdbg>,
    modules: Modules,
    breakpoints: HashMap<DebugModuleId, Vec<Breakpoint>>,
    targets: HashMap<u64, Target>,
    next_id: u64,
    event_reader: Option<Receiver<Result<String>>>,
    stop_reader: Arc<AtomicBool>,
    auto_attach_types: Vec<&'static str>,
}

impl Session {
    fn attach(args: &Value, resolve_modules: ResolveModules) -> Result<Self> {
        let root = Path::new(string(args, "rootProject")?);
        if args["debugServerHost"]
            .as_str()
            .is_some_and(|host| host != "127.0.0.1" && host != "localhost")
        {
            return Err("the debug server must be local".into());
        }
        let port = args["debugServerPort"].as_u64().unwrap_or(1550);
        let alias = args["infoBaseAlias"].as_str().unwrap_or("DefAlias");
        let sources = resolve_modules(root)?;
        if !args["sourceSet"].is_null() {
            let set = string(args, "sourceSet")?;
            if !sources
                .iter()
                .any(|source| source.address.source_set() == set)
            {
                return Err(format!(
                    "debug source set {set} is not available; use the names resolved by workspace"
                )
                .into());
            }
        }
        let modules = Modules::new(sources)?;
        let rdbg = Arc::new(Rdbg::attach(port.try_into()?, alias)?);
        let session = Self {
            modules,
            rdbg,
            breakpoints: HashMap::new(),
            targets: HashMap::new(),
            next_id: 0,
            event_reader: None,
            stop_reader: Arc::new(AtomicBool::new(false)),
            auto_attach_types: DEFAULT_AUTO_ATTACH_TYPES.to_vec(),
        };
        session
            .rdbg
            .set_auto_attach_types(&DEFAULT_AUTO_ATTACH_TYPES)?;
        Ok(session)
    }

    fn request(&mut self, name: &str, args: &Value, events: &mut Vec<Value>) -> Result<Value> {
        let body = match name {
            "configurationDone" => {
                for target in available_targets(&self.rdbg.targets()?)? {
                    if self.auto_attach_types.contains(&target.kind.as_str()) {
                        self.add_target(target.id, events)?;
                    }
                }
                if self.event_reader.is_none() {
                    self.start_event_reader()?;
                }
                json!({})
            }
            "unica/debugTargets" => self.debug_targets()?,
            "unica/attachDebugTarget" => {
                let id = string(args, "id")?.parse()?;
                self.require_available_target(id)?;
                let thread = if let Some(thread) = self.thread_of(id) {
                    thread
                } else {
                    match self.rdbg.attach_target(id)? {
                        TargetAttachment::Confirmed => {}
                        TargetAttachment::NoResult => self.require_available_target(id)?,
                        TargetAttachment::Refused => {
                            return Err("dbgs refused to attach the debug target".into());
                        }
                    }
                    self.register_target(id, events)
                };
                json!({"threadId":thread})
            }
            "unica/setAutoAttachTypes" => {
                let types = parse_auto_attach_types(&args["types"])?;
                self.rdbg.set_auto_attach_types(&types)?;
                self.auto_attach_types = types;
                json!({"autoAttachTypes":self.auto_attach_types})
            }
            "setBreakpoints" => self.set_breakpoints(args)?,
            "setExceptionBreakpoints" => {
                let (all, text) = exception_filters(args)?;
                self.rdbg.set_exception_filters(all, &text)?;
                json!({})
            }
            "exceptionInfo" => {
                let target = self
                    .targets
                    .get(&number(args, "threadId")?)
                    .ok_or("unknown thread")?;
                let description = target
                    .exception
                    .as_deref()
                    .ok_or("thread is not stopped on an exception")?;
                json!({"exceptionId":"1C runtime error","description":description,"breakMode":"always",
                       "details":{"message":description}})
            }
            "threads" => json!({"threads": self.targets.keys().map(|id| {
                json!({"id":id,"name":format!("1C target {id}")})
            }).collect::<Vec<_>>()}),
            "stackTrace" => {
                let target = self
                    .targets
                    .get(&number(args, "threadId")?)
                    .ok_or("unknown thread")?;
                if target.frames.is_empty() && target.exception.is_none() {
                    return Err("thread is running".into());
                }
                json!({"stackFrames":target.frames.iter().map(|frame| &frame.body).collect::<Vec<_>>(),
                    "totalFrames":target.frames.len()})
            }
            "scopes" => {
                let id = number(args, "frameId")?;
                self.frame(id)?;
                json!({"scopes":[{"name":"Locals","presentationHint":"locals","variablesReference":id,"expensive":false}]})
            }
            "evaluate" => self.evaluate(args, events)?,
            "variables" => {
                let variables = self.variables(number(args, "variablesReference")?, events)?;
                json!({"variables":variables})
            }
            "next" | "stepIn" | "stepOut" | "continue" => {
                let id = number(args, "threadId")?;
                let target = self.targets.get_mut(&id).ok_or("unknown thread")?;
                if target.frames.is_empty() && target.exception.is_none() {
                    return Err("thread is running".into());
                }
                let action = match name {
                    "next" => "Step",
                    "stepIn" => "StepIn",
                    "stepOut" => "StepOut",
                    _ => "Continue",
                };
                let stepping = action != "Continue";
                self.rdbg.step(target.id, action)?;
                target.frames.clear();
                target.exception = None;
                target.stepping = stepping;
                events.push(event(
                    "continued",
                    json!({"threadId":id,"allThreadsContinued":false}),
                ));
                if stepping {
                    json!({})
                } else {
                    json!({"allThreadsContinued":false})
                }
            }
            _ => return Err(format!("unsupported DAP command {name}").into()),
        };
        Ok(body)
    }

    fn set_breakpoints(&mut self, args: &Value) -> Result<Value> {
        let source = &args["source"];
        let address = string(source, "path")?;
        let module = self.modules.id(address)?;
        let requested = args["breakpoints"]
            .as_array()
            .ok_or("expected breakpoints array")?;
        let breakpoints = requested
            .iter()
            .map(|bp| {
                if ["hitCondition", "logMessage"]
                    .iter()
                    .any(|key| !bp[*key].is_null())
                {
                    return Err("hit counts and logpoints are not supported".into());
                }
                let line: u32 = number(bp, "line")?.try_into()?;
                if line == 0 {
                    return Err("line numbers start at one".into());
                }
                let condition = if bp["condition"].is_null() {
                    None
                } else {
                    let text = string(bp, "condition")?;
                    (!text.trim().is_empty()).then(|| text.to_owned())
                };
                Ok(Breakpoint { line, condition })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut updated = self.breakpoints.clone();
        updated.insert(module, breakpoints.clone());
        self.rdbg.breakpoints(updated.iter())?;
        self.breakpoints = updated;
        Ok(json!({"breakpoints":breakpoints.iter().map(|bp| {
            json!({"verified":true,"line":bp.line,"source":source})
        }).collect::<Vec<_>>()}))
    }

    fn frame(&self, id: u64) -> Result<(Uuid, usize)> {
        self.targets
            .values()
            .find_map(|target| {
                target
                    .frames
                    .iter()
                    .find(|frame| frame.id == id)
                    .map(|frame| (target.id, frame.level))
            })
            .ok_or_else(|| "unknown or expired frame".into())
    }

    fn variables(&mut self, reference: u64, events: &mut Vec<Value>) -> Result<Vec<Value>> {
        let (target, frame, level, path, evaluated_children) = self
            .targets
            .values()
            .find_map(|target| {
                target.frames.iter().find_map(|frame| {
                    let path = if frame.id == reference {
                        Vec::new()
                    } else {
                        frame.variable_paths.get(&reference)?.clone()
                    };
                    Some((
                        target.id,
                        frame.id,
                        frame.level,
                        path,
                        frame.evaluated_children.get(&reference).cloned(),
                    ))
                })
            })
            .ok_or("unknown or expired variable reference")?;
        let values = if let Some(children) = evaluated_children {
            children
        } else {
            let evaluation = Uuid::new_v4();
            let reply = self.rdbg.variables(target, level, evaluation, &path)?;
            self.wait_for_calculation(reply, evaluation, events, read_context)?
        };
        let frame = self
            .targets
            .values_mut()
            .flat_map(|target| &mut target.frames)
            .find(|current| current.id == frame)
            .ok_or("variable reference expired during evaluation")?;
        let mut result = Vec::with_capacity(values.len());
        for value in values {
            let mut reference = 0;
            if value.expandable
                && path.first().map_or_else(
                    || is_identifier(&value.name),
                    |root| root.split('.').all(is_identifier),
                )
            {
                let mut child_path = path.clone();
                child_path.push(value.name.clone());
                reference = frame.reference_for(child_path, &mut self.next_id);
            }
            result.push(json!({"name":value.name,"value":value.summary,
                "type":value.type_name,"variablesReference":reference}));
        }
        Ok(result)
    }

    fn evaluate(&mut self, args: &Value, events: &mut Vec<Value>) -> Result<Value> {
        let frame_id = number(args, "frameId")?;
        let (target, level) = self.frame(frame_id)?;
        let expression = string(args, "expression")?;
        if expression.trim().is_empty() {
            return Err("Введите выражение для вычисления".into());
        }
        if expression.chars().any(|ch| {
            !matches!(ch, '\u{9}' | '\u{a}' | '\u{d}' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
        }) {
            return Err("Выражение содержит недопустимый символ".into());
        }
        let path = vec![expression.to_owned()];
        let evaluation = Uuid::new_v4();
        let reply = self.rdbg.variables(target, level, evaluation, &path)?;
        let (value, children) =
            self.wait_for_calculation(reply, evaluation, events, read_expression)?;
        let frame = self
            .targets
            .values_mut()
            .flat_map(|target| &mut target.frames)
            .find(|frame| frame.id == frame_id)
            .ok_or("unknown or expired frame")?;
        let reference = if value.expandable {
            self.next_id += 1;
            let reference = self.next_id;
            frame.variable_paths.insert(reference, path);
            frame.evaluated_children.insert(reference, children);
            reference
        } else {
            0
        };
        let mut response = json!({"result": value.summary, "variablesReference": reference});
        if !value.type_name.is_empty() {
            response["type"] = json!(value.type_name);
        }
        Ok(response)
    }

    fn wait_for_calculation<T>(
        &mut self,
        initial: String,
        evaluation: Uuid,
        events: &mut Vec<Value>,
        parse: fn(Node<'_, '_>) -> Result<T>,
    ) -> Result<T> {
        if let Some(value) = read_calculation(&initial, evaluation, parse)? {
            return Ok(value);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Some(reader) = &self.event_reader else {
                break;
            };
            let reply = match reader.recv_timeout(remaining) {
                Ok(reply) => reply?,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("dbgs event reader stopped".into())
                }
            };
            self.consume(&reply, events)?;
            if let Some(value) = read_calculation(&reply, evaluation, parse)? {
                return Ok(value);
            }
        }
        Err("rdbg did not return the calculation result".into())
    }

    fn start_event_reader(&mut self) -> Result<()> {
        let (tx, rx) = bounded(1);
        let rdbg = Arc::clone(&self.rdbg);
        let stop = Arc::clone(&self.stop_reader);
        std::thread::Builder::new()
            .name("unica-dap-events".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match rdbg.poll() {
                        Ok(xml) if xml.trim().is_empty() => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Ok(xml) => {
                            if tx.send(Ok(xml)).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = tx.send(Err(error));
                            break;
                        }
                    }
                }
            })?;
        self.event_reader = Some(rx);
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_reader.store(true, Ordering::Release);
        self.event_reader = None;
        self.rdbg.detach();
    }
}

impl Session {
    fn debug_targets(&self) -> Result<Value> {
        let targets: Vec<_> = available_targets(&self.rdbg.targets()?)?
            .into_iter()
            .map(|target| {
                let thread = self.thread_of(target.id);
                let mut item = json!({"id":target.id.to_string(),"type":target.kind,"connected":thread.is_some()});
                if let Some(thread) = thread {
                    item["threadId"] = json!(thread);
                }
                if let Some(name) = target.name {
                    item["name"] = json!(name);
                }
                if let Some(number) = target.session_number {
                    item["sessionNumber"] = json!(number);
                }
                item
            })
            .collect();
        Ok(
            json!({"targets":targets,"types":DEBUG_TARGET_TYPES,"autoAttachTypes":self.auto_attach_types}),
        )
    }

    fn require_available_target(&self, id: Uuid) -> Result<()> {
        if available_targets(&self.rdbg.targets()?)?
            .iter()
            .any(|target| target.id == id)
        {
            return Ok(());
        }
        Err("debug target is no longer available".into())
    }

    fn thread_of(&self, id: Uuid) -> Option<u64> {
        self.targets
            .iter()
            .find_map(|(key, target)| (target.id == id).then_some(*key))
    }

    fn add_target(&mut self, id: Uuid, events: &mut Vec<Value>) -> Result<()> {
        if self.thread_of(id).is_some() {
            return Ok(());
        }
        if matches!(self.rdbg.attach_target(id)?, TargetAttachment::Refused) {
            return Ok(());
        }
        self.register_target(id, events);
        Ok(())
    }

    fn register_target(&mut self, id: Uuid, events: &mut Vec<Value>) -> u64 {
        self.next_id += 1;
        let thread = self.next_id;
        self.targets.insert(
            thread,
            Target {
                id,
                frames: Vec::new(),
                stepping: false,
                exception: None,
            },
        );
        events.push(event(
            "thread",
            json!({"reason":"started","threadId":thread}),
        ));
        thread
    }

    fn consume(&mut self, xml: &str, events: &mut Vec<Value>) -> Result<()> {
        if xml.trim().is_empty() {
            return Ok(());
        }
        let doc = Document::parse(xml)?;
        let results = || {
            doc.root_element()
                .children()
                .filter(|node| node.has_tag_name("result"))
        };
        // dbgs reports the stack and its diagnostic as separate results in the
        // same poll. Prefer the exception result, which also includes its stack.
        let stops: HashMap<_, _> = results()
            .filter_map(|result| {
                let command = field(result, "cmdID")?;
                if !matches!(
                    command,
                    "callStackFormed" | "rteProcessing" | "rteOnBPConditionProcessing"
                ) {
                    return None;
                }
                let target = field(child(result, "targetID")?, "id")?;
                Some(((target, command), result))
            })
            .collect();
        for result in results() {
            let Some(target) = child(result, "targetID") else {
                continue;
            };
            let target_id = field(target, "id").ok_or("missing rdbg target ID")?;
            let id: Uuid = target_id.parse()?;
            let command = field(result, "cmdID");
            let auto_attach = field(target, "targetType")
                .is_some_and(|kind| self.auto_attach_types.contains(&kind));
            if command == Some("targetStarted") {
                if auto_attach {
                    self.add_target(id, events)?;
                }
                continue;
            }
            // An auto-attached server emulation can report its first stop
            // without a separate targetStarted notification.
            if matches!(command, Some("callStackFormed" | "rteProcessing")) && auto_attach {
                self.add_target(id, events)?;
            }
            let thread = self.thread_of(id);
            match (command, thread) {
                (Some("targetQuit"), Some(thread)) => {
                    self.targets.remove(&thread);
                    events.push(event(
                        "thread",
                        json!({"reason":"exited","threadId":thread}),
                    ));
                }
                (Some("callStackFormed"), Some(thread))
                    if !stops.contains_key(&(target_id, "rteProcessing")) =>
                {
                    let frames = self.read_frames(result)?;
                    let target = self.targets.get_mut(&thread).expect("thread was found");
                    if target.exception.is_some() {
                        if target.frames.is_empty() {
                            target.frames = frames;
                        }
                        continue;
                    }
                    // 1C also marks a normal step as stopByBP. Prefer a step
                    // reason unless the new location has a user breakpoint.
                    let user_breakpoint = frames.first().is_some_and(|frame| {
                        let address = frame.body["source"]["path"].as_str().unwrap_or_default();
                        self.modules
                            .id(address)
                            .ok()
                            .and_then(|module| self.breakpoints.get(&module))
                            .is_some_and(|lines| {
                                lines.iter().any(|bp| {
                                    bp.condition.is_none() && frame.body["line"] == bp.line
                                })
                            })
                    });
                    let reason = if target.stepping && !user_breakpoint {
                        "step"
                    } else if field(result, "stopByBP") == Some("true") {
                        "breakpoint"
                    } else {
                        "pause"
                    };
                    target.stepping = false;
                    target.frames = frames;
                    target.exception = None;
                    let mut body =
                        json!({"reason":reason,"threadId":thread,"allThreadsStopped":false});
                    if let Some(condition) = stops.get(&(target_id, "rteOnBPConditionProcessing")) {
                        body["reason"] = json!("breakpoint");
                        body["text"] = json!(condition_message(*condition));
                    }
                    events.push(event("stopped", body));
                }
                (Some("rteProcessing"), Some(thread)) => {
                    let message = child(result, "exception")
                        .and_then(|error| field(error, "descr"))
                        .unwrap_or("Ошибка выполнения 1С")
                        .to_owned();
                    let stack = if child(result, "callStack").is_some() {
                        result
                    } else {
                        stops
                            .get(&(target_id, "callStackFormed"))
                            .copied()
                            .unwrap_or(result)
                    };
                    let frames = self.read_frames(stack)?;
                    let target = self.targets.get_mut(&thread).expect("thread was found");
                    target.frames = frames;
                    target.stepping = false;
                    target.exception = Some(message.clone());
                    events.push(event("stopped", json!({
                        "reason":"exception","threadId":thread,"allThreadsStopped":false,"text":message
                    })));
                }
                (Some("rteOnBPConditionProcessing"), Some(_))
                    if !stops.contains_key(&(target_id, "callStackFormed")) =>
                {
                    events.push(event(
                        "output",
                        json!({
                            "category":"stderr","output":format!("{}\n", condition_message(result))
                        }),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read_frames(&mut self, result: Node<'_, '_>) -> Result<Vec<Frame>> {
        let stack: Vec<_> = result
            .children()
            .filter(|node| node.has_tag_name("callStack"))
            .collect();
        stack.into_iter().rev().enumerate().map(|(level, entry)| {
            self.next_id += 1;
            let id = self.next_id;
            let mut body = json!({
                "id":id,"name":decode(field(entry,"presentation"))?,
                "line":field(entry,"lineNo").ok_or("missing rdbg stack line")?.parse::<u32>()?,"column":1
            });
            if let Some(module) = child(entry, "moduleID") {
                let runtime_id = DebugModuleId {
                    extension: field(module, "extensionName").filter(|name| !name.is_empty()).map(str::to_owned),
                    object: field(module, "objectID").ok_or("missing rdbg object ID")?.parse()?,
                    property: field(module, "propertyID").ok_or("missing rdbg property ID")?.parse()?,
                };
                if let Some(address) = self.modules.address(&runtime_id) {
                    body["source"] = json!({"name":address,"path":address});
                }
            }
            Ok(Frame { id, level, body, variable_paths: HashMap::new(), evaluated_children: HashMap::new() })
        }).collect()
    }
}

impl Frame {
    fn reference_for(&mut self, path: Vec<String>, next_id: &mut u64) -> u64 {
        if let Some((&id, _)) = self
            .variable_paths
            .iter()
            .find(|(id, existing)| **existing == path && !self.evaluated_children.contains_key(id))
        {
            return id;
        }
        *next_id += 1;
        self.variable_paths.insert(*next_id, path);
        *next_id
    }
}

#[derive(Clone)]
struct ContextVariable {
    name: String,
    summary: String,
    type_name: String,
    expandable: bool,
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

fn read_calculation<T>(
    xml: &str,
    evaluation: Uuid,
    parse: fn(Node<'_, '_>) -> Result<T>,
) -> Result<Option<T>> {
    if xml.trim().is_empty() {
        return Ok(None);
    }
    let doc = Document::parse(xml)?;
    let Some(result) = doc.descendants().find(|node| {
        field(*node, "expressionResultID").and_then(|id| id.parse::<Uuid>().ok())
            == Some(evaluation)
    }) else {
        return Ok(None);
    };
    parse(result).map(Some)
}

fn read_expression(result: Node<'_, '_>) -> Result<(ContextVariable, Vec<ContextVariable>)> {
    if field(result, "evalResultState") != Some("correctly")
        || field(result, "errorOccurred") == Some("true")
    {
        let detail = decode(field(result, "exceptionStr"))?;
        return if detail.is_empty() {
            Err("Не удалось вычислить выражение в выбранном кадре".into())
        } else {
            Err(detail.into())
        };
    }
    let value = child(result, "resultValueInfo").ok_or("rdbg returned no expression value")?;
    let value = read_value(String::new(), value)?;
    let children = child(result, "calculationResult")
        .map(read_context_children)
        .transpose()?
        .unwrap_or_default();
    Ok((value, children))
}

fn read_value(name: String, value: Node<'_, '_>) -> Result<ContextVariable> {
    Ok(ContextVariable {
        name,
        summary: decode(field(value, "pres"))?,
        type_name: field(value, "typeName").unwrap_or_default().to_owned(),
        expandable: field(value, "isExpandable") == Some("true"),
    })
}

fn read_context(result: Node<'_, '_>) -> Result<Vec<ContextVariable>> {
    if field(result, "evalResultState") != Some("correctly") {
        return Err("rdbg could not read variables".into());
    }
    let context =
        child(result, "calculationResult").ok_or("rdbg did not return a local context")?;
    read_context_children(context)
}

fn read_context_children(context: Node<'_, '_>) -> Result<Vec<ContextVariable>> {
    context
        .children()
        .filter(|node| node.has_tag_name("valueOfContextPropInfo"))
        .map(|item| {
            let property = child(item, "propInfo").ok_or("missing variable property")?;
            let name = field(property, "propName")
                .ok_or("missing variable name")?
                .to_owned();
            if field(property, "isReaded") == Some("false") {
                return Ok(ContextVariable {
                    name,
                    summary: decode(field(property, "errorStr"))?,
                    type_name: String::new(),
                    expandable: false,
                });
            }
            let value = child(item, "valueInfo").ok_or("missing variable value")?;
            read_value(name, value)
        })
        .collect()
}

fn decode(text: Option<&str>) -> Result<String> {
    let compact: String = text
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    Ok(String::from_utf8(STANDARD.decode(compact)?)?)
}

fn available_targets(xml: &str) -> Result<Vec<AvailableTarget>> {
    if xml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc = Document::parse(xml)?;
    doc.root_element()
        .children()
        .filter(|node| node.has_tag_name("id"))
        .map(|node| {
            Ok(AvailableTarget {
                id: field(node, "id").ok_or("missing rdbg target ID")?.parse()?,
                kind: field(node, "targetType")
                    .ok_or("missing rdbg target type")?
                    .to_owned(),
                name: field(node, "userName")
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                session_number: field(node, "seanceNo").and_then(|number| number.parse().ok()),
            })
        })
        .collect()
}

fn condition_message(result: Node<'_, '_>) -> String {
    let error = child(result, "errorInfo").and_then(|info| child(info, "exception"));
    let detail = error
        .and_then(|error| field(error, "descr"))
        .unwrap_or("1С не вернула описание");
    format!("Ошибка в условии точки останова: {detail}")
}

fn exception_filters(args: &Value) -> Result<(bool, Vec<&str>)> {
    let mut all = false;
    let mut wants_text = false;
    let mut text = Vec::new();
    if let Some(filters) = args.get("filters") {
        for filter in filters
            .as_array()
            .ok_or("expected exception filters array")?
        {
            match filter.as_str() {
                Some("all") => all = true,
                Some("text") => wants_text = true,
                _ => return Err("unknown exception filter".into()),
            }
        }
    }
    if let Some(options) = args.get("filterOptions") {
        for option in options
            .as_array()
            .ok_or("expected exception filter options array")?
        {
            match string(option, "filterId")? {
                "all" => all = true,
                "text" => {
                    let condition = string(option, "condition")?;
                    if condition.trim().is_empty() {
                        return Err("Введите текст для фильтра ошибок".into());
                    }
                    text.push(condition);
                }
                _ => return Err("unknown exception filter".into()),
            }
        }
    }
    if wants_text && text.is_empty() {
        return Err("Введите текст для фильтра ошибок".into());
    }
    Ok((all, text))
}

fn parse_auto_attach_types(value: &Value) -> Result<Vec<&'static str>> {
    let requested = value.as_array().ok_or("expected types array")?;
    let mut types = Vec::new();
    for kind in requested {
        let kind = kind.as_str().ok_or("expected debug target type string")?;
        let kind = DEBUG_TARGET_TYPES
            .iter()
            .copied()
            .find(|known| *known == kind)
            .ok_or_else(|| format!("unsupported debug target type {kind}"))?;
        if !types.contains(&kind) {
            types.push(kind);
        }
    }
    Ok(types)
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("expected string {key}").into())
}

fn number(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("expected integer {key}").into())
}

fn event(name: &str, body: Value) -> Value {
    json!({"type":"event","event":name,"body":body})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_filter_options_distinguish_off_all_and_text() {
        assert_eq!(
            exception_filters(&json!({"filters":[]})).unwrap(),
            (false, vec![])
        );
        assert_eq!(
            exception_filters(&json!({"filters":["all"]})).unwrap(),
            (true, vec![])
        );
        assert_eq!(
            exception_filters(&json!({
                "filters":[],
                "filterOptions":[{"filterId":"text","condition":"Деление на ноль"}]
            }))
            .unwrap(),
            (false, vec!["Деление на ноль"])
        );
        assert!(exception_filters(&json!({"filters":["text"]})).is_err());
        assert!(exception_filters(&json!({"filters":["unknown"]})).is_err());
    }

    #[test]
    fn debug_target_snapshot_preserves_platform_identity() {
        let id = Uuid::new_v4();
        let xml = format!(
            "<response><id><id>{id}</id><targetType>HTTPService</targetType>\
             <userName>Тест</userName><seanceNo>42</seanceNo></id></response>"
        );
        let targets = available_targets(&xml).unwrap();
        assert_eq!(targets.len(), 1);
        let target = &targets[0];
        assert_eq!(target.id, id);
        assert_eq!(target.kind, "HTTPService");
        assert_eq!(target.name.as_deref(), Some("Тест"));
        assert_eq!(target.session_number, Some(42));
    }

    #[test]
    fn auto_attach_accepts_empty_and_rejects_unknown_types() {
        assert!(parse_auto_attach_types(&json!([])).unwrap().is_empty());
        assert_eq!(
            parse_auto_attach_types(&json!(["HTTPService", "HTTPService", "JOB"])).unwrap(),
            ["HTTPService", "JOB"]
        );
        assert!(parse_auto_attach_types(&json!(["unsupported"])).is_err());
    }

    #[test]
    fn evaluate_reads_value_and_reports_the_platform_error() {
        let evaluation = Uuid::new_v4();
        let reply = format!(
            "<result><expressionResultID>{evaluation}</expressionResultID>\
             <evalResultState>correctly</evalResultState><resultValueInfo>\
             <pres>ODI=</pres><typeName>Число</typeName><isExpandable>false</isExpandable>\
             </resultValueInfo></result>"
        );
        let (value, children) = read_calculation(&reply, evaluation, read_expression)
            .unwrap()
            .unwrap();
        assert!(children.is_empty());
        assert_eq!(value.summary, "82");
        assert_eq!(value.type_name, "Число");
        assert!(!value.expandable);

        let exception = STANDARD.encode("Переменная не определена");
        let reply = format!(
            "<result><expressionResultID>{evaluation}</expressionResultID>\
             <errorOccurred>true</errorOccurred><exceptionStr>{exception}</exceptionStr></result>"
        );
        let Err(error) = read_calculation(&reply, evaluation, read_expression) else {
            panic!("expression error must fail the request");
        };
        assert_eq!(error.to_string(), "Переменная не определена");
    }

    #[test]
    fn context_read_keeps_expandable_and_unreadable_properties_distinct() {
        let evaluation = Uuid::new_v4();
        let xml = format!(
            r#"<response xmlns="http://v8.1c.ru/8.3/debugger/debugBaseData">
                <result xmlns="http://v8.1c.ru/8.3/debugger/debugRDBGRequestResponse">
                    <evalResultState>correctly</evalResultState>
                    <expressionResultID>{evaluation}</expressionResultID>
                    <calculationResult xmlns="http://v8.1c.ru/8.3/debugger/debugCalculations">
                        <viewInterface>context</viewInterface>
                        <valueOfContextPropInfo>
                            <propInfo><propName>Вложенная</propName><isReaded>true</isReaded></propInfo>
                            <valueInfo><isExpandable>true</isExpandable><typeName>Структура</typeName></valueInfo>
                        </valueOfContextPropInfo>
                        <valueOfContextPropInfo>
                            <propInfo><propName>Ошибка</propName><isReaded>false</isReaded><errorStr>ZXJyb3I=</errorStr></propInfo>
                        </valueOfContextPropInfo>
                    </calculationResult>
                </result>
            </response>"#
        );
        let properties = read_calculation(&xml, evaluation, read_context)
            .unwrap()
            .unwrap();
        assert_eq!(properties.len(), 2);
        assert_eq!(properties[0].name, "Вложенная");
        assert_eq!(properties[0].type_name, "Структура");
        assert!(properties[0].expandable);
        assert_eq!(properties[1].summary, "error");
        assert!(!properties[1].expandable);
        assert!(read_calculation(&xml, Uuid::new_v4(), read_context)
            .unwrap()
            .is_none());
    }
}
