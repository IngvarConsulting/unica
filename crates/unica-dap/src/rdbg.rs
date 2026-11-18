use std::time::Duration;

use roxmltree::{Document, Node};

use unica_source_contract::debug_source::DebugModuleId;

use crate::Result;

const REQUEST: &str = "http://v8.1c.ru/8.3/debugger/debugRDBGRequestResponse";
const BASE: &str = "http://v8.1c.ru/8.3/debugger/debugBaseData";
const BREAKPOINTS: &str = "http://v8.1c.ru/8.3/debugger/debugBreakpoints";
const CALCULATIONS: &str = "http://v8.1c.ru/8.3/debugger/debugCalculations";
const AUTO_ATTACH: &str = "http://v8.1c.ru/8.3/debugger/debugAutoAttach";

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
                .timeout(Duration::from_secs(3))
                .build()?,
            url: format!("http://127.0.0.1:{port}/e1crdbg/rdbg"),
            alias: escape(alias),
            id: uuid::Uuid::new_v4(),
        };
        let xml = client.call(
            "attachDebugUI",
            "<options><foregroundAbility>false</foregroundAbility></options>",
        )?;
        let doc = Document::parse(&xml)?;
        let root = doc.root_element();
        let status = field(root, "result").unwrap_or("missing result");
        if status != "registered" {
            return Err(format!("rdbg attach rejected: {status}").into());
        }
        Ok(client)
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
        )
    }

    pub fn breakpoints<'a>(
        &self,
        modules: impl Iterator<Item = (&'a DebugModuleId, &'a Vec<u32>)>,
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
            for line in lines {
                workspace.push_str(&format!(
                    "<bpInfo><line>{line}</line><isActive>true</isActive><breakOnCondition>false</breakOnCondition><breakOnParentMethod>false</breakOnParentMethod><breakOnHitCount>false</breakOnHitCount><hitCountVariant>0</hitCountVariant><hitCount>1</hitCount><showOutputMessage>false</showOutputMessage><putStackTrace>false</putStackTrace><putHitCount>false</putHitCount><continueExecution>false</continueExecution><temp>false</temp><user>true</user></bpInfo>"
                ));
            }
            workspace.push_str("</moduleBPInfo>");
        }
        workspace.push_str("</bpWorkspace>");
        self.call("setBreakpoints", &workspace)?;
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
        let command = if path.is_empty() {
            "evalLocalVariables"
        } else {
            "evalExpr"
        };
        self.call(command, &format!(
            "<calcWaitingTime>100</calcWaitingTime><targetID><id xmlns=\"{BASE}\">{target}</id></targetID><expr><stackLevel xmlns=\"{CALCULATIONS}\">{level}</stackLevel><srcCalcInfo xmlns=\"{CALCULATIONS}\"><expressionResultID>{evaluation}</expressionResultID>{items}<interfaces>context</interfaces></srcCalcInfo><presOptions xmlns=\"{CALCULATIONS}\"><maxTextSize>307200</maxTextSize><stopOnFirstEOL>false</stopOnFirstEOL></presOptions></expr>"
        ))
    }

    fn call(&self, command: &str, body: &str) -> Result<String> {
        let request = format!(
            "<request><infoBaseAlias xmlns=\"{REQUEST}\">{}</infoBaseAlias><idOfDebuggerUI xmlns=\"{REQUEST}\">{}</idOfDebuggerUI>{}</request>",
            self.alias, self.id, namespace_request_children(body)
        );
        self.send(command, &[], Some(request))
    }

    fn send(&self, command: &str, query: &[(&str, &str)], body: Option<String>) -> Result<String> {
        let mut url = reqwest::Url::parse(&self.url)?;
        url.query_pairs_mut()
            .append_pair("cmd", command)
            .extend_pairs(query);
        let mut request = self.http.post(url).header("User-Agent", "1CV8");
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
}

impl Drop for Rdbg {
    fn drop(&mut self) {
        let _ = self.call("detachDebugUI", "");
    }
}

// The request wrapper has no namespace. Each top-level argument is in the
// request namespace, with nested protocol types declaring their own namespace.
fn namespace_request_children(body: &str) -> String {
    let wrapped = format!("<args>{body}</args>");
    let parsed = Document::parse(&wrapped).expect("internally generated rdbg arguments");
    let mut result = String::new();
    for child in parsed.root_element().children().filter(Node::is_element) {
        let text = &wrapped[child.range()];
        let split = 1 + child.tag_name().name().len();
        result.push_str(&text[..split]);
        result.push_str(&format!(" xmlns=\"{REQUEST}\""));
        result.push_str(&text[split..]);
    }
    result
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
