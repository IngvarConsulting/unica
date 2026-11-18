//! DAP attach sessions for the local 1C debug server.

mod modules;
mod rdbg;

use std::{
    collections::HashMap,
    io::{self, BufRead, Read, Write},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use modules::Modules;
use rdbg::{child, field, Rdbg};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_source_contract::debug_source::{DebugModuleId, DebugModuleSource};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

type ResolveModules = fn(&Path) -> Result<Vec<DebugModuleSource>>;

pub fn run_stdio(resolve_modules: ResolveModules) -> i32 {
    let (tx, rx) = mpsc::sync_channel(8);
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
    fn run(&mut self, requests: mpsc::Receiver<Result<Value>>) -> Result<()> {
        loop {
            let command = match requests.recv_timeout(Duration::from_millis(100)) {
                Ok(request) => request?,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(session) = &mut self.session {
                        match session.poll() {
                            Ok(events) => self.emit(events)?,
                            Err(error) => {
                                self.session = None;
                                self.emit(vec![
                                    event(
                                        "output",
                                        json!({"category":"stderr","output":format!("{error}\n")}),
                                    ),
                                    event("terminated", json!({})),
                                ])?;
                            }
                        }
                    }
                    continue;
                }
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
                Ok(json!({"supportsConfigurationDoneRequest":true}))
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

struct Target {
    id: Uuid,
    frames: Vec<Frame>,
    stepping: bool,
}

struct Frame {
    id: u64,
    level: usize,
    body: Value,
    variable_paths: HashMap<u64, Vec<String>>,
}

struct Session {
    rdbg: Rdbg,
    modules: Modules,
    breakpoints: HashMap<DebugModuleId, Vec<u32>>,
    targets: HashMap<u64, Target>,
    next_id: u64,
    configured: bool,
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
        Ok(Self {
            modules: Modules::new(sources)?,
            rdbg: Rdbg::attach(port.try_into()?, alias)?,
            breakpoints: HashMap::new(),
            targets: HashMap::new(),
            next_id: 0,
            configured: false,
        })
    }

    fn request(&mut self, name: &str, args: &Value, events: &mut Vec<Value>) -> Result<Value> {
        let body = match name {
            "configurationDone" => {
                let xml = self.rdbg.targets()?;
                if !xml.trim().is_empty() {
                    let doc = Document::parse(&xml)?;
                    for node in doc
                        .descendants()
                        .filter(|node| field(*node, "targetType") == Some("Client"))
                    {
                        if let Some(id) = field(node, "id") {
                            self.add_target(id.parse()?, events)?;
                        }
                    }
                }
                self.configured = true;
                json!({})
            }
            "setBreakpoints" => self.set_breakpoints(args)?,
            "threads" => json!({"threads": self.targets.keys().map(|id| {
                json!({"id":id,"name":format!("1C client {id}")})
            }).collect::<Vec<_>>()}),
            "stackTrace" => {
                let target = self
                    .targets
                    .get(&number(args, "threadId")?)
                    .ok_or("unknown thread")?;
                if target.frames.is_empty() {
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
            "variables" => {
                let variables = self.variables(number(args, "variablesReference")?, events)?;
                json!({"variables":variables})
            }
            "next" | "continue" => {
                let id = number(args, "threadId")?;
                let target = self.targets.get_mut(&id).ok_or("unknown thread")?;
                if target.frames.is_empty() {
                    return Err("thread is running".into());
                }
                let stepping = name == "next";
                self.rdbg
                    .step(target.id, if stepping { "Step" } else { "Continue" })?;
                target.frames.clear();
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
        let lines = requested
            .iter()
            .map(|bp| {
                if ["condition", "hitCondition", "logMessage"]
                    .iter()
                    .any(|key| !bp[*key].is_null())
                {
                    return Err("only unconditional breakpoints are supported".into());
                }
                let line: u32 = number(bp, "line")?.try_into()?;
                if line == 0 {
                    return Err("line numbers start at one".into());
                }
                Ok(line)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut updated = self.breakpoints.clone();
        updated.insert(module, lines.clone());
        self.rdbg.breakpoints(updated.iter())?;
        self.breakpoints = updated;
        Ok(json!({"breakpoints":lines.iter().map(|line| {
            json!({"verified":true,"line":line,"source":source})
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
        let (target, frame, level, path) = self
            .targets
            .values()
            .find_map(|target| {
                target.frames.iter().find_map(|frame| {
                    let path = if frame.id == reference {
                        Vec::new()
                    } else {
                        frame.variable_paths.get(&reference)?.clone()
                    };
                    Some((target.id, frame.id, frame.level, path))
                })
            })
            .ok_or("unknown or expired variable reference")?;
        let evaluation = Uuid::new_v4();
        let reply = self.rdbg.variables(target, level, evaluation, &path)?;
        let values = self.wait_for_variables(reply, evaluation, events)?;
        let frame = self
            .targets
            .values_mut()
            .flat_map(|target| &mut target.frames)
            .find(|current| current.id == frame)
            .ok_or("variable reference expired during evaluation")?;
        let mut result = Vec::with_capacity(values.len());
        for value in values {
            let mut reference = 0;
            if value.expandable && (!path.is_empty() || is_identifier(&value.name)) {
                let mut child_path = path.clone();
                child_path.push(value.name.clone());
                reference = match frame
                    .variable_paths
                    .iter()
                    .find(|(_, path)| **path == child_path)
                {
                    Some((id, _)) => *id,
                    None => {
                        self.next_id += 1;
                        frame.variable_paths.insert(self.next_id, child_path);
                        self.next_id
                    }
                };
            }
            result.push(json!({"name":value.name,"value":value.summary,
                "type":value.type_name,"variablesReference":reference}));
        }
        Ok(result)
    }

    fn wait_for_variables(
        &mut self,
        initial: String,
        evaluation: Uuid,
        events: &mut Vec<Value>,
    ) -> Result<Vec<ContextVariable>> {
        if let Some(variables) = read_context(&initial, evaluation)? {
            return Ok(variables);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let reply = self.rdbg.poll()?;
            self.consume(&reply, events)?;
            if let Some(variables) = read_context(&reply, evaluation)? {
                return Ok(variables);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("rdbg did not return variables".into())
    }

    fn poll(&mut self) -> Result<Vec<Value>> {
        if !self.configured {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        self.consume(&self.rdbg.poll()?, &mut events)?;
        Ok(events)
    }

    fn add_target(&mut self, id: Uuid, events: &mut Vec<Value>) -> Result<()> {
        if self.targets.values().any(|target| target.id == id) {
            return Ok(());
        }
        self.rdbg.attach_target(id)?;
        self.next_id += 1;
        let thread = self.next_id;
        self.targets.insert(
            thread,
            Target {
                id,
                frames: Vec::new(),
                stepping: false,
            },
        );
        events.push(event(
            "thread",
            json!({"reason":"started","threadId":thread}),
        ));
        Ok(())
    }

    fn consume(&mut self, xml: &str, events: &mut Vec<Value>) -> Result<()> {
        if xml.trim().is_empty() {
            return Ok(());
        }
        let doc = Document::parse(xml)?;
        for result in doc
            .root_element()
            .children()
            .filter(|node| node.has_tag_name("result"))
        {
            let Some(target) = child(result, "targetID") else {
                continue;
            };
            let id: Uuid = field(target, "id")
                .ok_or("missing rdbg target ID")?
                .parse()?;
            let thread = self
                .targets
                .iter()
                .find_map(|(key, target)| (target.id == id).then_some(*key));
            match (field(result, "cmdID"), thread) {
                (Some("targetStarted"), _) if field(target, "targetType") == Some("Client") => {
                    self.add_target(id, events)?;
                }
                (Some("targetQuit"), Some(thread)) => {
                    self.targets.remove(&thread);
                    events.push(event(
                        "thread",
                        json!({"reason":"exited","threadId":thread}),
                    ));
                }
                (Some("callStackFormed"), Some(thread)) => {
                    let frames = self.read_frames(result)?;
                    let target = self.targets.get_mut(&thread).expect("thread was found");
                    // 1C also marks a normal step as stopByBP. Prefer a step
                    // reason unless the new location has a user breakpoint.
                    let user_breakpoint = frames.first().is_some_and(|frame| {
                        let address = frame.body["source"]["path"].as_str().unwrap_or_default();
                        self.modules
                            .id(address)
                            .ok()
                            .and_then(|module| self.breakpoints.get(&module))
                            .is_some_and(|lines| {
                                lines.iter().any(|line| frame.body["line"] == *line)
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
                    events.push(event(
                        "stopped",
                        json!({"reason":reason,"threadId":thread,"allThreadsStopped":false}),
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
            Ok(Frame { id, level, body, variable_paths: HashMap::new() })
        }).collect()
    }
}

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

fn read_context(xml: &str, evaluation: Uuid) -> Result<Option<Vec<ContextVariable>>> {
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
    if field(result, "evalResultState") != Some("correctly") {
        return Err("rdbg could not read variables".into());
    }
    let context =
        child(result, "calculationResult").ok_or("rdbg did not return a local context")?;
    let values = context
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
            Ok(ContextVariable {
                name,
                summary: decode(field(value, "pres"))?,
                type_name: field(value, "typeName").unwrap_or_default().to_owned(),
                expandable: field(value, "isExpandable") == Some("true"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(values))
}

fn decode(text: Option<&str>) -> Result<String> {
    let compact: String = text
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    Ok(String::from_utf8(STANDARD.decode(compact)?)?)
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
        let properties = read_context(&xml, evaluation).unwrap().unwrap();
        assert_eq!(properties.len(), 2);
        assert_eq!(properties[0].name, "Вложенная");
        assert_eq!(properties[0].type_name, "Структура");
        assert!(properties[0].expandable);
        assert_eq!(properties[1].summary, "error");
        assert!(!properties[1].expandable);
        assert!(read_context(&xml, Uuid::new_v4()).unwrap().is_none());
    }
}
