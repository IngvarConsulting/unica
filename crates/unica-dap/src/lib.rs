//! DAP attach sessions for an existing 1C debug server.

mod endpoint;
mod modules;
mod rdbg;

use std::{
    collections::{HashMap, HashSet},
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
use endpoint::{attach_server, server_from_cli, DebugServerError};
use modules::Modules;
use rdbg::{child, field, Rdbg, TargetAttachment};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_source_contract::debug_source::{
    external_processor_frame_name, DebugModuleId, DebugServerUrl, ResolveDebugModules,
    OBJECT_MODULE_PROPERTY_ID,
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

type NewResolver = fn(&Path, Option<&str>) -> Result<ResolveDebugModules>;

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

pub fn run_stdio(resolve_modules: NewResolver, args: &[String]) -> i32 {
    let selected_server = match server_from_cli(args) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("unica dap: {error}");
            return 2;
        }
    };
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
        selected_server,
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
    resolve_modules: NewResolver,
    selected_server: Option<DebugServerUrl>,
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
                    if let Some(error) = error.downcast_ref::<DebugServerError>() {
                        response["body"] =
                            json!({"error":{"id":error.id(),"format":error.to_string()}});
                    }
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
                    "supportsUnicaDebugServerUrl": true,
                    "supportsConditionalBreakpoints": true,
                    "supportsSetExpression": true,
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
                self.session = Some(Session::attach(
                    args,
                    self.resolve_modules,
                    self.selected_server.as_ref(),
                )?);
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

type Breakpoints = HashMap<String, (DebugModuleId, Vec<Breakpoint>)>;

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
    module: Option<DebugModuleId>,
    body: Value,
    variable_paths: HashMap<u64, Vec<String>>,
    evaluated_children: HashMap<u64, Vec<ContextVariable>>,
    collection_paths: HashSet<u64>,
    snapshots: HashMap<u64, SnapshotNode>,
}

struct StackEntry {
    line: u32,
    name: String,
    workspace_module: Option<DebugModuleId>,
    external_source: Option<String>,
}

struct SnapshotNode {
    source: Arc<str>,
    suffix: String,
    indexed: bool,
    children: Option<Arc<[ContextVariable]>>,
    child_references: HashMap<String, u64>,
}

struct Session {
    rdbg: Arc<Rdbg>,
    modules: Modules,
    breakpoints: Breakpoints,
    targets: HashMap<u64, Target>,
    next_id: u64,
    event_reader: Option<Receiver<Result<String>>>,
    stop_reader: Arc<AtomicBool>,
    auto_attach_types: Vec<&'static str>,
}

impl Session {
    fn attach(
        args: &Value,
        resolve_modules: NewResolver,
        selected_server: Option<&DebugServerUrl>,
    ) -> Result<Self> {
        let url = attach_server(args, selected_server)?;
        let root = Path::new(string(args, "rootProject")?);
        let alias = if args["infoBaseAlias"].is_null() {
            "DefAlias"
        } else {
            string(args, "infoBaseAlias")?
        };
        let source_set = if args["sourceSet"].is_null() {
            None
        } else {
            Some(string(args, "sourceSet")?)
        };
        let modules = Modules::new(resolve_modules(root, source_set)?);
        let rdbg = Arc::new(Rdbg::attach(&url, alias)?);
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
            "setExpression" => self.set_expression(args, events)?,
            "variables" => {
                let reference = number(args, "variablesReference")?;
                let variables = if self
                    .targets
                    .values()
                    .flat_map(|target| &target.frames)
                    .any(|frame| frame.snapshots.contains_key(&reference))
                {
                    self.snapshot_variables(reference, args, events)?
                } else {
                    self.variables(reference, events)?
                };
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
        let updated = self
            .modules
            .replace_breakpoints(&self.breakpoints, address, &breakpoints)?;
        self.rdbg
            .breakpoints(updated.values().map(|(id, points)| (id, points)))?;
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
        let (target, frame, level, path, evaluated_children, collection) = self
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
                        frame.collection_paths.contains(&reference),
                    ))
                })
            })
            .ok_or("unknown or expired variable reference")?;
        let mut values = if collection {
            let evaluation = Uuid::new_v4();
            let reply = self.rdbg.collection(target, level, evaluation, &path)?;
            self.wait_for_calculation(reply, evaluation, events, read_collection)?
        } else if let Some(children) = evaluated_children {
            children
        } else {
            let evaluation = Uuid::new_v4();
            let reply = self.rdbg.variables(target, level, evaluation, &path)?;
            self.wait_for_calculation(reply, evaluation, events, read_context)?
        };
        let addressable = path.first().is_none_or(|root| is_variable_address(root));
        if addressable {
            let parent = path.join(".");
            self.fill_collection_sizes(
                target,
                level,
                &mut values,
                |value| {
                    if collection {
                        Some(format!("{parent}{}", value.name))
                    } else if !is_identifier(&value.name) {
                        None
                    } else if parent.is_empty() {
                        Some(value.name.clone())
                    } else {
                        Some(format!("{parent}.{}", value.name))
                    }
                },
                events,
            );
        }
        let frame = self
            .targets
            .values_mut()
            .flat_map(|target| &mut target.frames)
            .find(|current| current.id == frame)
            .ok_or("variable reference expired during evaluation")?;
        let mut result = Vec::with_capacity(values.len());
        for value in values {
            let mut reference = 0;
            if value.has_children() && addressable {
                // RDBG property steps only accept named properties. An indexed
                // child needs a BSL expression and is read on its own expansion.
                let child_path = if collection {
                    vec![format!("{}{}", path.join("."), value.name)]
                } else if is_identifier(&value.name) {
                    let mut child_path = path.clone();
                    child_path.push(value.name.clone());
                    child_path
                } else {
                    Vec::new()
                };
                if !child_path.is_empty() {
                    reference = frame.reference_for(child_path, &mut self.next_id);
                    if value.indexed {
                        frame.collection_paths.insert(reference);
                    } else {
                        frame.collection_paths.remove(&reference);
                    }
                }
            }
            let mut variable = json!({"name":value.name,"value":value.summary,
                "type":value.type_name,"variablesReference":reference});
            if let Some(count) = value.indexed_count() {
                variable["indexedVariables"] = json!(count);
            }
            result.push(variable);
        }
        Ok(result)
    }

    fn fill_collection_sizes(
        &mut self,
        target: Uuid,
        level: usize,
        values: &mut [ContextVariable],
        address: impl Fn(&ContextVariable) -> Option<String>,
        events: &mut Vec<Value>,
    ) {
        let pending: Vec<_> = values
            .iter()
            .enumerate()
            .filter(|(_, value)| value.indexed && value.indexed_count.is_none())
            .filter_map(|(index, value)| Some((index, address(value)?, Uuid::new_v4())))
            .collect();
        if let Some(reply) = (!pending.is_empty())
            .then(|| self.rdbg.collection_sizes(target, level, &pending).ok())
            .flatten()
        {
            let mut awaiting: HashMap<_, _> = pending
                .into_iter()
                .map(|(index, _, evaluation)| (evaluation, index))
                .collect();
            // Count metadata is optional; keep locals and any sizes received if this wait fails.
            let _ = self.wait_for_reply(reply, events, |xml| {
                if xml.trim().is_empty() {
                    return Ok(None);
                }
                let doc = Document::parse(xml)?;
                for node in doc.descendants() {
                    let Some(evaluation) =
                        field(node, "expressionResultID").and_then(|id| id.parse::<Uuid>().ok())
                    else {
                        continue;
                    };
                    if let Some(index) = awaiting.remove(&evaluation) {
                        values[index].indexed_count = child(node, "resultValueInfo")
                            .and_then(|value| field(value, "collectionSize"))
                            .and_then(|count| count.parse().ok());
                    }
                }
                Ok(awaiting.is_empty().then_some(()))
            });
        }
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
        if args["unicaSnapshot"] == true {
            return self.evaluate_snapshot(frame_id, target, level, expression, events);
        }
        let path = vec![expression.to_owned()];
        let evaluation = Uuid::new_v4();
        let reply = self.rdbg.variables(target, level, evaluation, &path)?;
        let (value, children) =
            self.wait_for_calculation(reply, evaluation, events, read_expression)?;
        self.calculation_response(frame_id, path, value, children, "result")
    }

    fn evaluate_snapshot(
        &mut self,
        frame_id: u64,
        target: Uuid,
        level: usize,
        expression: &str,
        events: &mut Vec<Value>,
    ) -> Result<Value> {
        let evaluation = Uuid::new_v4();
        let expression = format!("ЗначениеВСтрокуВнутр({expression})");
        let reply = self
            .rdbg
            .variables(target, level, evaluation, &[expression])?;
        let source: Arc<str> = self
            .wait_for_calculation(reply, evaluation, events, read_snapshot)?
            .into();
        self.frame(frame_id)?;
        let evaluation = Uuid::new_v4();
        let path = [snapshot_expression(&source, "")];
        let reply = self.rdbg.variables(target, level, evaluation, &path)?;
        let (value, children) =
            self.wait_for_calculation(reply, evaluation, events, read_expression)?;
        self.frame(frame_id)?;
        let mut body =
            json!({"result":value.summary,"type":value.type_name,"variablesReference":0});
        if value.has_children() {
            self.next_id += 1;
            let id = self.next_id;
            let frame = self
                .targets
                .values_mut()
                .flat_map(|t| &mut t.frames)
                .find(|frame| frame.id == frame_id)
                .ok_or("unknown or expired frame")?;
            frame.snapshots.insert(
                id,
                SnapshotNode {
                    source,
                    suffix: String::new(),
                    indexed: value.indexed,
                    children: (!value.indexed).then(|| children.into()),
                    child_references: HashMap::new(),
                },
            );
            body["variablesReference"] = json!(id);
        }
        if let Some(count) = value.indexed_count() {
            body["indexedVariables"] = json!(count);
        }
        Ok(body)
    }

    fn snapshot_variables(
        &mut self,
        reference: u64,
        args: &Value,
        events: &mut Vec<Value>,
    ) -> Result<Vec<Value>> {
        let (target, frame_id, level, source, suffix, indexed, cached) = self
            .targets
            .values()
            .find_map(|target| {
                target.frames.iter().find_map(|frame| {
                    frame.snapshots.get(&reference).map(|node| {
                        (
                            target.id,
                            frame.id,
                            frame.level,
                            Arc::clone(&node.source),
                            node.suffix.clone(),
                            node.indexed,
                            node.children.clone(),
                        )
                    })
                })
            })
            .ok_or("unknown or expired variable reference")?;
        if (indexed && args["filter"] == "named") || (!indexed && args["filter"] == "indexed") {
            return Ok(Vec::new());
        }
        let start = args["start"].as_u64().unwrap_or(0) as usize;
        let count = args["count"]
            .as_u64()
            .filter(|count| *count > 0)
            .map_or(usize::MAX, |count| count as usize);
        let values: Arc<[ContextVariable]> = if let Some(cached) = cached {
            cached
        } else {
            let evaluation = Uuid::new_v4();
            let path = [snapshot_expression(&source, &suffix)];
            let rows = if indexed {
                let reply = self.rdbg.collection(target, level, evaluation, &path)?;
                self.wait_for_calculation(reply, evaluation, events, read_collection)?
            } else {
                let reply = self.rdbg.variables(target, level, evaluation, &path)?;
                self.wait_for_calculation(reply, evaluation, events, read_context)?
            };
            rows.into()
        };
        let mut page: Vec<_> = values.iter().skip(start).take(count).cloned().collect();
        self.fill_collection_sizes(
            target,
            level,
            &mut page,
            |value| {
                if indexed {
                    Some(snapshot_expression(
                        &source,
                        &format!("{suffix}{}", value.name),
                    ))
                } else if is_identifier(&value.name) {
                    Some(snapshot_expression(
                        &source,
                        &format!("{suffix}.{}", value.name),
                    ))
                } else {
                    None
                }
            },
            events,
        );
        let frame = self
            .targets
            .values_mut()
            .flat_map(|t| &mut t.frames)
            .find(|frame| frame.id == frame_id)
            .ok_or("unknown or expired variable reference")?;
        frame
            .snapshots
            .get_mut(&reference)
            .ok_or("unknown or expired variable reference")?
            .children = Some(Arc::clone(&values));
        let mut rows = Vec::new();
        for value in page {
            let mut row = json!({"name":value.name,"value":value.summary,"type":value.type_name,"variablesReference":0});
            if value.has_children() && (indexed || is_identifier(&value.name)) {
                let existing = frame.snapshots[&reference]
                    .child_references
                    .get(&value.name)
                    .copied();
                let id = existing.unwrap_or_else(|| {
                    let suffix = if indexed {
                        format!("{suffix}{}", value.name)
                    } else {
                        format!("{suffix}.{}", value.name)
                    };
                    self.next_id += 1;
                    frame.snapshots.insert(
                        self.next_id,
                        SnapshotNode {
                            source: Arc::clone(&source),
                            suffix,
                            indexed: value.indexed,
                            children: None,
                            child_references: HashMap::new(),
                        },
                    );
                    frame
                        .snapshots
                        .get_mut(&reference)
                        .expect("snapshot exists")
                        .child_references
                        .insert(value.name.clone(), self.next_id);
                    self.next_id
                });
                row["variablesReference"] = json!(id);
            }
            if let Some(count) = value.indexed_count() {
                row["indexedVariables"] = json!(count);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn set_expression(&mut self, args: &Value, events: &mut Vec<Value>) -> Result<Value> {
        let frame_id = number(args, "frameId")?;
        let (target, level) = self.frame(frame_id)?;
        let name = string(args, "expression")?.trim();
        if !is_identifier(name) {
            return Err("Укажите имя переменной выбранного кадра".into());
        }
        let value = string(args, "value")?;
        if value.trim().is_empty() {
            return Err("Введите выражение нового значения".into());
        }
        let lookup = Uuid::new_v4();
        let reply = self.rdbg.variables(target, level, lookup, &[])?;
        let locals = self.wait_for_calculation(reply, lookup, events, read_context)?;
        self.frame(frame_id)?;
        let name_key = name.to_lowercase();
        let local = locals
            .into_iter()
            .find(|local| local.name.to_lowercase() == name_key)
            .ok_or("Переменная не найдена в выбранном кадре")?;
        let path = vec![local.name];
        let evaluation = Uuid::new_v4();
        // A timed-out write may still have reached the platform. Retire the
        // snapshots before sending it, and never retry a write automatically.
        if let Some(thread) = self.thread_of(target) {
            for frame in &mut self
                .targets
                .get_mut(&thread)
                .expect("thread was found")
                .frames
            {
                frame
                    .variable_paths
                    .retain(|id, _| !frame.evaluated_children.contains_key(id));
                frame
                    .collection_paths
                    .retain(|id| frame.variable_paths.contains_key(id));
                frame.evaluated_children.clear();
                frame.snapshots.clear();
            }
        }
        let reply = self
            .rdbg
            .modify_value(target, level, evaluation, &path, value)?;
        if !reply.trim().is_empty() {
            let doc = Document::parse(&reply)?;
            if let Some(error) = field(doc.root_element(), "errorDescr") {
                let description = decode(Some(error))?;
                if !description.is_empty() {
                    return Err(description.into());
                }
            }
        }
        let (value, children) =
            self.wait_for_calculation(reply, evaluation, events, read_modified_value)?;
        self.calculation_response(frame_id, path, value, children, "value")
    }

    fn calculation_response(
        &mut self,
        frame_id: u64,
        path: Vec<String>,
        value: ContextVariable,
        children: Vec<ContextVariable>,
        key: &str,
    ) -> Result<Value> {
        let frame = self
            .targets
            .values_mut()
            .flat_map(|target| &mut target.frames)
            .find(|frame| frame.id == frame_id)
            .ok_or("unknown or expired frame")?;
        let reference = if value.has_children() {
            self.next_id += 1;
            let reference = self.next_id;
            // A variables request may read a live address again. Keep the
            // cached context for arbitrary expressions, which can have effects.
            if value.indexed && path.first().is_some_and(|root| is_variable_address(root)) {
                frame.collection_paths.insert(reference);
            }
            frame.variable_paths.insert(reference, path);
            frame.evaluated_children.insert(reference, children);
            reference
        } else {
            0
        };
        let mut response = json!({(key): value.summary, "variablesReference": reference});
        if let Some(count) = value.indexed_count() {
            response["indexedVariables"] = json!(count);
        }
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
        self.wait_for_reply(initial, events, |xml| {
            read_calculation(xml, evaluation, parse)
        })
    }

    fn wait_for_reply<T>(
        &mut self,
        initial: String,
        events: &mut Vec<Value>,
        mut parse: impl FnMut(&str) -> Result<Option<T>>,
    ) -> Result<T> {
        if let Some(value) = parse(&initial)? {
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
            if let Some(value) = parse(&reply)? {
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
                    let frames =
                        self.read_frames(result, field(result, "stopByBP") == Some("true"))?;
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
                        self.breakpoints.iter().any(|(at, (module, lines))| {
                            frame.module.as_ref() == Some(module)
                                && frame.body["source"]["path"]
                                    .as_str()
                                    .is_none_or(|path| path == at)
                                && lines.iter().any(|bp| {
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
                    let frames = self.read_frames(stack, false)?;
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

    fn read_frames(&mut self, result: Node<'_, '_>, breakpoint_stop: bool) -> Result<Vec<Frame>> {
        let stack = read_stack_entries(result, &mut self.modules)?;
        // Only the innermost frame can report a hit on one of the installed lines.
        let mut hit = stack.last().filter(|_| breakpoint_stop).and_then(|entry| {
            let module = entry.workspace_module.as_ref()?;
            self.breakpoints
                .iter()
                .find_map(|(at, (installed, points))| {
                    (installed == module && points.iter().any(|point| point.line == entry.line))
                        .then_some(at.as_str())
                })
        });
        let sources = self
            .modules
            .frame_addresses(stack.iter().rev().map(|entry| {
                (
                    entry.workspace_module.as_ref(),
                    entry.name.as_str(),
                    hit.take(),
                )
            }));
        Ok(stack
            .into_iter()
            .rev()
            .zip(sources)
            .enumerate()
            .map(|(level, (entry, source))| {
                self.next_id += 1;
                let id = self.next_id;
                let mut body = json!({
                    "id":id,"name":entry.name,
                    "line":entry.line,"column":1
                });
                if let Some(address) = source.or(entry.external_source) {
                    body["source"] = json!({"name":address,"path":address});
                }
                Frame {
                    id,
                    level,
                    module: entry.workspace_module,
                    body,
                    variable_paths: HashMap::new(),
                    evaluated_children: HashMap::new(),
                    collection_paths: HashSet::new(),
                    snapshots: HashMap::new(),
                }
            })
            .collect())
    }
}

fn read_stack_entries(result: Node<'_, '_>, modules: &mut Modules) -> Result<Vec<StackEntry>> {
    let mut stack = Vec::new();
    let mut external_sources = HashMap::new();
    for entry in result
        .children()
        .filter(|node| node.has_tag_name("callStack"))
    {
        let name = decode(field(entry, "presentation"))?;
        let (workspace_module, external_source) =
            match child(entry, "moduleID").and_then(frame_module) {
                Some(FrameModule::Workspace(id)) => (Some(id), None),
                Some(FrameModule::ExternalProcessor(id)) => {
                    let source = external_processor_frame_name(&name, &id).and_then(|owner| {
                        external_sources
                            .entry((id, owner.to_owned()))
                            .or_insert_with_key(|(id, owner)| {
                                modules.external_processor_address(id, owner)
                            })
                            .clone()
                    });
                    (None, source)
                }
                None => (None, None),
            };
        let line = field(entry, "lineNo")
            .and_then(|line| line.parse::<u32>().ok())
            .unwrap_or(0);
        stack.push(StackEntry {
            line,
            name,
            workspace_module,
            external_source,
        });
    }
    Ok(stack)
}

enum FrameModule {
    Workspace(DebugModuleId),
    ExternalProcessor(DebugModuleId),
}

/// External and internal modules can share both UUIDs. Only the wire type selects their source.
fn frame_module(module: Node<'_, '_>) -> Option<FrameModule> {
    let extension = field(module, "extensionName").filter(|name| !name.is_empty());
    let id = DebugModuleId {
        extension: extension.map(str::to_owned),
        object: field(module, "objectID")?.parse().ok()?,
        property: field(module, "propertyID")?.parse().ok()?,
    };
    match field(module, "type") {
        Some("ConfigModule") if extension.is_none() => Some(FrameModule::Workspace(id)),
        Some("ExtensionModule") if extension.is_some() => Some(FrameModule::Workspace(id)),
        Some("ExtMDModule") if extension.is_none() && id.property == OBJECT_MODULE_PROPERTY_ID => {
            Some(FrameModule::ExternalProcessor(id))
        }
        None if field(module, "URL").is_none()
            && field(module, "extId").is_none_or(|id| id == "0") =>
        {
            Some(FrameModule::Workspace(id))
        }
        _ => None,
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
    indexed: bool,
    indexed_count: Option<u64>,
}

fn is_variable_address(address: &str) -> bool {
    address.split('.').all(|segment| {
        let mut parts = segment.split('[');
        parts.next().is_some_and(is_identifier)
            && parts.all(|index| {
                index.strip_suffix(']').is_some_and(|index| {
                    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
                })
            })
    })
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

impl ContextVariable {
    fn indexed_count(&self) -> Option<u64> {
        self.indexed_count.filter(|_| self.indexed)
    }

    fn has_children(&self) -> bool {
        (self.expandable || self.indexed) && self.indexed_count() != Some(0)
    }
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

fn snapshot_expression(source: &str, suffix: &str) -> String {
    format!("ЗначениеИзСтрокиВнутр(ПолучитьСтрокуИзДвоичныхДанных(Base64Значение(\"{source}\"), КодировкаТекста.UTF8)){suffix}")
}

fn read_snapshot(result: Node<'_, '_>) -> Result<String> {
    // Preserve the raw string bytes returned by RDBG, independent of display
    // quoting, newline handling and localization.
    let (value, _) = read_expression(result)?;
    if value.type_name.is_empty() {
        return Err("1С не вернула снимок выражения".into());
    }
    let raw = child(result, "resultValueInfo")
        .and_then(|value| field(value, "valueString"))
        .ok_or("1С не вернула строку снимка целиком")?;
    let raw: String = raw.chars().filter(|ch| !ch.is_ascii_whitespace()).collect();
    if raw.is_empty() || STANDARD.decode(&raw).is_err() {
        return Err("1С вернула повреждённую строку снимка".into());
    }
    Ok(raw)
}

fn read_modified_value(result: Node<'_, '_>) -> Result<(ContextVariable, Vec<ContextVariable>)> {
    if let Some(error) = result.parent().and_then(|parent| field(parent, "errorStr")) {
        let description = decode(Some(error))?;
        if !description.is_empty() {
            return Err(description.into());
        }
    }
    read_expression(result)
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
        indexed: field(value, "isIIndexedCollectionRO") == Some("true"),
        indexed_count: field(value, "collectionSize").and_then(|size| size.parse().ok()),
    })
}

fn read_collection(result: Node<'_, '_>) -> Result<Vec<ContextVariable>> {
    if field(result, "evalResultState") != Some("correctly") {
        return Err("rdbg could not read collection elements".into());
    }
    if child(result, "resultValueInfo").and_then(|value| field(value, "collectionSize"))
        == Some("0")
    {
        return Ok(Vec::new());
    }
    let collection = child(result, "calculationResult")
        .filter(|value| field(*value, "viewInterface") == Some("collection"))
        .ok_or("rdbg did not return collection elements")?;
    collection
        .children()
        .filter(|item| item.has_tag_name("valueOfCollectionInfo"))
        .enumerate()
        .map(|(index, item)| {
            let value = child(item, "valueInfo").ok_or("missing collection element")?;
            read_value(format!("[{index}]"), value)
        })
        .collect()
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
                    indexed: false,
                    indexed_count: None,
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
    fn each_stop_looks_up_an_external_owner_once_across_recursive_frames_and_methods() {
        use std::{cell::RefCell, rc::Rc};
        use unica_address::address::QualifiedAddress;
        use unica_source_contract::debug_source::{
            DebugModuleMatches, DebugModuleQuery, DebugModuleSource,
        };

        fn frame(owner: &str, object: Uuid, method: &str) -> String {
            let name =
                STANDARD.encode(format!("ВнешняяОбработка.{owner}.МодульОбъекта.{method}()"));
            format!("<callStack><moduleID><type>ExtMDModule</type><objectID>{object}</objectID><propertyID>{OBJECT_MODULE_PROPERTY_ID}</propertyID></moduleID><presentation>{name}</presentation></callStack>")
        }
        let object = Uuid::from_u128(17);
        let other_object = Uuid::from_u128(18);
        let expected = "epf:ExternalDataProcessor.Probe.Module.Object";
        let calls = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&calls);
        let mut modules = Modules::new(Box::new(move |query| {
            let DebugModuleQuery::ExternalProcessorFrame { name, id } = query else {
                panic!("external module reached internal lookup")
            };
            recorded.borrow_mut().push((name.to_owned(), id.object));
            let modules = if name == "Probe" && id.object == object {
                vec![DebugModuleSource {
                    address: QualifiedAddress::parse(expected).unwrap(),
                    runtime_id: id.clone(),
                }]
            } else {
                vec![]
            };
            Ok(DebugModuleMatches {
                modules,
                source_sets_revision: 1,
            })
        }));
        let mut stack = String::from("<result>");
        for i in 0..20 {
            stack.push_str(&frame(
                "Probe",
                object,
                if i % 2 == 0 { "Enter" } else { "Recurse" },
            ));
        }
        for method in ["Enter", "Recurse"] {
            stack.push_str(&frame("Missing", object, method));
            stack.push_str(&frame("Probe", other_object, method));
        }
        stack.push_str("</result>");
        let doc = Document::parse(&stack).unwrap();
        for _ in 0..2 {
            let frames = read_stack_entries(doc.root_element(), &mut modules).unwrap();
            assert_eq!(frames.len(), 24);
            assert!(frames[..20]
                .iter()
                .all(|frame| frame.external_source.as_deref() == Some(expected)));
            assert!(frames[20..]
                .iter()
                .all(|frame| frame.external_source.is_none()));
            assert_eq!(
                calls.take(),
                [
                    ("Probe".to_owned(), object),
                    ("Missing".to_owned(), object),
                    ("Probe".to_owned(), other_object)
                ]
            );
        }
    }

    #[test]
    fn external_object_frames_keep_their_wire_type_even_when_internal_uuids_match() {
        fn parse(kind: &str, extension: &str, property: Uuid) -> Option<FrameModule> {
            let object = Uuid::from_u128(17);
            let xml = format!(
                "<moduleID><type>{kind}</type><extensionName>{extension}</extensionName>
                <objectID>{object}</objectID><propertyID>{property}</propertyID>
                <URL>e1cib/tempstorage/test-handle</URL></moduleID>"
            );
            frame_module(Document::parse(&xml).unwrap().root_element())
        }
        let Some(FrameModule::ExternalProcessor(external)) =
            parse("ExtMDModule", "", OBJECT_MODULE_PROPERTY_ID)
        else {
            panic!("external object must keep its external identity");
        };
        let Some(FrameModule::Workspace(internal)) =
            parse("ConfigModule", "", OBJECT_MODULE_PROPERTY_ID)
        else {
            panic!("expected configuration identity");
        };
        assert_eq!(external, internal);
        assert!(parse("SystemModule", "", OBJECT_MODULE_PROPERTY_ID).is_none());
        assert!(parse("ExtMDModule", "Extension", OBJECT_MODULE_PROPERTY_ID).is_none());
        assert!(parse("ExtMDModule", "", Uuid::from_u128(1)).is_none());
    }

    #[test]
    fn indexed_values_keep_platform_count_and_order() {
        let evaluation = Uuid::new_v4();
        let xml = format!(
            "<result><expressionResultID>{evaluation}</expressionResultID>\
             <evalResultState>correctly</evalResultState><calculationResult><viewInterface>collection</viewInterface>\
             <valueOfCollectionInfo><valueInfo><pres>MQ==</pres></valueInfo></valueOfCollectionInfo>\
             <valueOfCollectionInfo><valueInfo><pres>Mg==</pres></valueInfo></valueOfCollectionInfo>\
             </calculationResult></result>"
        );
        let items = read_calculation(&xml, evaluation, read_collection)
            .unwrap()
            .unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| (item.name.as_str(), item.summary.as_str()))
                .collect::<Vec<_>>(),
            [("[0]", "1"), ("[1]", "2")]
        );
        let xml = Document::parse("<value><isIIndexedCollectionRO>true</isIIndexedCollectionRO><collectionSize>2</collectionSize></value>").unwrap();
        let value = read_value("Массив".into(), xml.root_element()).unwrap();
        assert!(value.indexed);
        assert_eq!(value.indexed_count, Some(2));
    }

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
