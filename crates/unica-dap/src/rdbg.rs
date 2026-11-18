use std::time::Duration;

use roxmltree::{Document, Node};

use unica_source_contract::debug_source::DebugModuleId;

use crate::{Breakpoint, Result};

const REQUEST: &str = "http://v8.1c.ru/8.3/debugger/debugRDBGRequestResponse";
const BASE: &str = "http://v8.1c.ru/8.3/debugger/debugBaseData";
const BREAKPOINTS: &str = "http://v8.1c.ru/8.3/debugger/debugBreakpoints";
const CALCULATIONS: &str = "http://v8.1c.ru/8.3/debugger/debugCalculations";
const AUTO_ATTACH: &str = "http://v8.1c.ru/8.3/debugger/debugAutoAttach";
// resource-limit: dap.rdbg-request-timeout
// class: deadline
// unit: ms
// scope: request
// value: 3000
// resource: Срок HTTP-запроса к dbgs вне опроса событий.
// status: unproven
// origin: #3194
// problem: неизвестно
// on-reach: запрос DAP завершается ошибкой транспорта
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
// dbgs holds an idle event poll for about five seconds. Allow extra time
// under load so that we do not abandon the response carrying a stop.
// resource-limit: dap.rdbg-event-poll-timeout
// class: deadline
// unit: ms
// scope: request
// value: 30000
// resource: Срок одного фонового опроса событий dbgs.
// status: unproven
// origin: #3203
// problem: штатный пустой ответ dbgs приходит примерно через 5 секунд, а прежний срок 3 секунды мог прервать получение остановки
// on-reach: опрос возвращает пустой результат и повторяется на следующем проходе
const EVENT_POLL_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Rdbg {
    http: reqwest::blocking::Client,
    url: String,
    alias: String,
    id: uuid::Uuid,
}

pub(crate) enum TargetAttachment {
    Confirmed,
    Refused,
    NoResult,
}

impl Rdbg {
    pub fn attach(port: u16, alias: &str) -> Result<Self> {
        let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
        let client = Self {
            http: reqwest::blocking::Client::builder()
                .tls_backend_preconfigured(tls)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            url: format!("http://127.0.0.1:{port}/e1crdbg/rdbg"),
            alias: escape(alias),
            id: uuid::Uuid::new_v4(),
        };
        if let Err(error) = client.register() {
            client.detach();
            return Err(error);
        }
        Ok(client)
    }

    fn register(&self) -> Result<()> {
        let xml = self.call(
            "attachDebugUI",
            "<options><foregroundAbility>false</foregroundAbility></options>",
        )?;
        let doc = Document::parse(&xml)?;
        let root = doc.root_element();
        let status = field(root, "result").unwrap_or("missing result");
        if status != "registered" {
            return Err(format!("rdbg attach rejected: {status}").into());
        }
        Ok(())
    }

    pub fn set_auto_attach_types(&self, types: &[&str]) -> Result<()> {
        let target_types = types
            .iter()
            .map(|kind| format!("<targetType xmlns=\"{AUTO_ATTACH}\">{kind}</targetType>"))
            .collect::<String>();
        self.call(
            "setAutoAttachSettings",
            &format!("<autoAttachSettings>{target_types}</autoAttachSettings>"),
        )?;
        Ok(())
    }

    pub fn targets(&self) -> Result<String> {
        self.call("getDbgTargets", "")
    }

    pub fn attach_target(&self, target: uuid::Uuid) -> Result<TargetAttachment> {
        let xml = self.call(
            "attachDetachDbgTargets",
            &format!("<attach>true</attach><id><id xmlns=\"{BASE}\">{target}</id></id>"),
        )?;
        if xml.trim().is_empty() {
            return Ok(TargetAttachment::NoResult);
        }
        match field(Document::parse(&xml)?.root_element(), "result") {
            Some("true" | "1") => Ok(TargetAttachment::Confirmed),
            Some("false" | "0") => Ok(TargetAttachment::Refused),
            _ => Err("invalid rdbg target attachment response".into()),
        }
    }

    pub fn poll(&self) -> Result<String> {
        self.send(
            "pingDebugUIParams",
            &[("dbgui", &self.id.to_string())],
            None,
            EVENT_POLL_TIMEOUT,
        )
    }

    pub fn breakpoints<'a>(
        &self,
        modules: impl Iterator<Item = (&'a DebugModuleId, &'a Vec<Breakpoint>)>,
    ) -> Result<()> {
        let mut workspace = String::from("<bpWorkspace>");
        for (id, lines) in modules {
            let (module_type, extension) = match &id.extension {
                Some(name) => ("ExtensionModule", escape(name)),
                None => ("ConfigModule", String::new()),
            };
            workspace.push_str(&format!(
                "<moduleBPInfo xmlns=\"{BREAKPOINTS}\"><id><type xmlns=\"{BASE}\">{module_type}</type><URL xmlns=\"{BASE}\"/><extensionName xmlns=\"{BASE}\">{extension}</extensionName><objectID xmlns=\"{BASE}\">{}</objectID><propertyID xmlns=\"{BASE}\">{}</propertyID><extId xmlns=\"{BASE}\">0</extId></id>",
                id.object, id.property
            ));
            for breakpoint in lines {
                let condition = breakpoint.condition.as_deref().map(escape);
                let conditional = condition.is_some();
                let condition = condition.map_or_else(String::new, |value| {
                    format!("<condition>{value}</condition>")
                });
                let line = breakpoint.line;
                workspace.push_str(&format!(
                    "<bpInfo><line>{line}</line><isActive>true</isActive><breakOnCondition>{conditional}</breakOnCondition>{condition}<breakOnParentMethod>false</breakOnParentMethod><breakOnHitCount>false</breakOnHitCount><hitCountVariant>0</hitCountVariant><hitCount>1</hitCount><showOutputMessage>false</showOutputMessage><putStackTrace>false</putStackTrace><putHitCount>false</putHitCount><continueExecution>false</continueExecution><temp>false</temp><user>true</user></bpInfo>"
                ));
            }
            workspace.push_str("</moduleBPInfo>");
        }
        workspace.push_str("</bpWorkspace>");
        self.call("setBreakpoints", &workspace)?;
        Ok(())
    }

    pub fn set_exception_filters(&self, all: bool, text: &[&str]) -> Result<()> {
        const FILTER: &str = "http://v8.1c.ru/8.3/debugger/debugRTEFilter";
        let enabled = all || !text.is_empty();
        let analyze = !all && !text.is_empty();
        let mut state = format!(
            "<state><stopOnErrors xmlns=\"{FILTER}\">{enabled}</stopOnErrors><analyzeErrorStr xmlns=\"{FILTER}\">{analyze}</analyzeErrorStr>"
        );
        if analyze {
            for condition in text {
                state.push_str(&format!(
                    "<strTemplate xmlns=\"{FILTER}\"><include>true</include><str>{}</str></strTemplate>",
                    escape(condition)
                ));
            }
        }
        state.push_str("</state>");
        self.call("setBreakOnRTE", &state)?;
        Ok(())
    }

    pub fn step(&self, target: uuid::Uuid, action: &str) -> Result<()> {
        self.call("step", &format!(
            "<targetID><id xmlns=\"{BASE}\">{target}</id></targetID><action>{action}</action><simple>false</simple>"
        ))?;
        Ok(())
    }

    pub fn variables(
        &self,
        target: uuid::Uuid,
        level: usize,
        evaluation: uuid::Uuid,
        path: &[String],
    ) -> Result<String> {
        let command = if path.is_empty() {
            "evalLocalVariables"
        } else {
            "evalExpr"
        };
        let source = calculation_source(level, evaluation, path, "context");
        self.call(command, &format!(
            "<calcWaitingTime>100</calcWaitingTime><targetID><id xmlns=\"{BASE}\">{target}</id></targetID><expr>{source}</expr>"
        ))
    }

    pub fn collection_sizes(
        &self,
        target: uuid::Uuid,
        level: usize,
        names: &[(usize, String, uuid::Uuid)],
    ) -> Result<String> {
        let mut body = format!(
            "<calcWaitingTime>100</calcWaitingTime><targetID><id xmlns=\"{BASE}\">{target}</id></targetID>"
        );
        for (_, name, evaluation) in names {
            let source =
                calculation_source(level, *evaluation, std::slice::from_ref(name), "context");
            body.push_str(&format!("<expr>{source}</expr>"));
        }
        self.call("evalExpr", &body)
    }

    pub fn collection(
        &self,
        target: uuid::Uuid,
        level: usize,
        evaluation: uuid::Uuid,
        path: &[String],
    ) -> Result<String> {
        let source = calculation_source(level, evaluation, path, "collection");
        self.call("evalExpr", &format!(
            "<calcWaitingTime>100</calcWaitingTime><targetID><id xmlns=\"{BASE}\">{target}</id></targetID><expr>{source}</expr>"
        ))
    }

    pub fn modify_value(
        &self,
        target: uuid::Uuid,
        level: usize,
        evaluation: uuid::Uuid,
        path: &[String],
        value: &str,
    ) -> Result<String> {
        let source = calculation_source(level, evaluation, path, "context");
        let value = escape(value);
        self.call("modifyValue", &format!(
            "<targetID><id xmlns=\"{BASE}\">{target}</id></targetID><modifyDataPath>{source}</modifyDataPath><newValueInfo><variant xmlns=\"{CALCULATIONS}\">expr</variant><valueExpression xmlns=\"{CALCULATIONS}\">{value}</valueExpression></newValueInfo><timeout>3000</timeout>"
        ))
    }

    fn call(&self, command: &str, body: &str) -> Result<String> {
        let request = format!(
            "<request><infoBaseAlias xmlns=\"{REQUEST}\">{}</infoBaseAlias><idOfDebuggerUI xmlns=\"{REQUEST}\">{}</idOfDebuggerUI>{}</request>",
            self.alias, self.id, namespace_request_children(body)?
        );
        self.send(command, &[], Some(request), REQUEST_TIMEOUT)
    }

    fn send(
        &self,
        command: &str,
        query: &[(&str, &str)],
        body: Option<String>,
        timeout: Duration,
    ) -> Result<String> {
        let mut url = reqwest::Url::parse(&self.url)?;
        url.query_pairs_mut()
            .append_pair("cmd", command)
            .extend_pairs(query);
        let mut request = self
            .http
            .post(url)
            .header("User-Agent", "1CV8")
            .timeout(timeout);
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(body);
        } else {
            request = request.body("");
        }
        let response = match request.send() {
            Ok(response) => response,
            Err(error) if command == "pingDebugUIParams" && error.is_timeout() => {
                return Ok(String::new());
            }
            Err(error) => return Err(format!("rdbg {command}: {error:?}").into()),
        };
        let status = response.status();
        let xml = response.text()?;
        if !status.is_success() {
            let detail = Document::parse(&xml).ok().and_then(|doc| {
                doc.descendants()
                    .find(|node| node.has_tag_name("descr"))
                    .and_then(|node| node.text())
                    .map(str::to_owned)
            });
            return Err(format!("rdbg {command}: {status} {}", detail.unwrap_or_default()).into());
        }
        Ok(xml)
    }

    pub fn detach(&self) {
        let _ = self.call("detachDebugUI", "");
    }
}

// The request wrapper has no namespace. Each top-level argument is in the
// request namespace, with nested protocol types declaring their own namespace.
fn namespace_request_children(body: &str) -> Result<String> {
    let wrapped = format!("<args>{body}</args>");
    let parsed = Document::parse(&wrapped).map_err(|_| "invalid text in RDBG request")?;
    let mut result = String::new();
    for child in parsed.root_element().children().filter(Node::is_element) {
        let text = &wrapped[child.range()];
        let split = 1 + child.tag_name().name().len();
        result.push_str(&text[..split]);
        result.push_str(&format!(" xmlns=\"{REQUEST}\""));
        result.push_str(&text[split..]);
    }
    Ok(result)
}

fn calculation_source(
    level: usize,
    evaluation: uuid::Uuid,
    path: &[String],
    interface: &str,
) -> String {
    let mut items = String::new();
    for (index, name) in path.iter().enumerate() {
        let name = escape(name);
        let (kind, expression, property) = if index == 0 {
            ("expression", name.as_str(), "")
        } else {
            ("property", "", name.as_str())
        };
        items.push_str(&format!(
                "<calcItem><itemType>{kind}</itemType><expression>{expression}</expression><property>{property}</property></calcItem>"
            ));
    }
    format!(
        "<stackLevel xmlns=\"{CALCULATIONS}\">{level}</stackLevel><srcCalcInfo xmlns=\"{CALCULATIONS}\"><expressionResultID>{evaluation}</expressionResultID>{items}<interfaces>{interface}</interfaces></srcCalcInfo><presOptions xmlns=\"{CALCULATIONS}\"><maxTextSize>307200</maxTextSize><stopOnFirstEOL>false</stopOnFirstEOL></presOptions>"
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name(name))
}

pub(crate) fn field<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(node, name).and_then(|child| child.text())
}
