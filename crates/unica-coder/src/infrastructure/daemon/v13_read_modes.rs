use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::apply::{
    FormItemRole, OperationRegistry, OperationSite, SourceSetTraits, TemplateFormat,
    IMPLEMENTED_APPLY_OPERATIONS,
};
use crate::domain::refusal::RefusalCode;
use crate::infrastructure::metadata_kinds::metadata_kind;
use serde_json::{json, Map, Value};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadModeError {
    code: RefusalCode,
    message: String,
}

impl ReadModeError {
    fn bad_value(message: impl Into<String>) -> Self {
        Self {
            code: RefusalCode::BadValue,
            message: message.into(),
        }
    }

    fn unsupported_section(message: impl Into<String>) -> Self {
        Self {
            code: RefusalCode::UnsupportedSection,
            message: message.into(),
        }
    }

    fn unsupported_scope(message: impl Into<String>) -> Self {
        Self {
            code: RefusalCode::UnsupportedScope,
            message: message.into(),
        }
    }

    pub(super) const fn code(&self) -> RefusalCode {
        self.code
    }
}

impl fmt::Display for ReadModeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ReadModeError {}

/// How a canonical search matches one source line: a literal needle or a
/// bounded, pre-compiled regular expression. Both report byte offsets of match
/// starts so the caller projects the same logical location either way.
pub(crate) enum SearchMatcher {
    Literal(String),
    Regex(regex::Regex),
}

impl SearchMatcher {
    pub(super) fn match_starts<'a>(
        &'a self,
        line: &'a str,
    ) -> Box<dyn Iterator<Item = usize> + 'a> {
        match self {
            Self::Literal(needle) => {
                Box::new(line.match_indices(needle.as_str()).map(|(start, _)| start))
            }
            Self::Regex(pattern) => Box::new(
                pattern
                    .find_iter(line)
                    .filter(|found| !found.is_empty())
                    .map(|found| found.start()),
            ),
        }
    }
}

/// Where `can` of a read node is computed: the traits of its source set —
/// the one owner of whether the source is written at all.
pub(super) type CanContext = SourceSetTraits;

#[cfg(test)]
pub(super) fn configuration_context() -> CanContext {
    SourceSetTraits::read(
        crate::domain::project_sources::SourceSetKind::Configuration,
        crate::domain::platform_profile::PlatformProfile::v8_3_27(),
    )
}

/// The node's address, template format and form item role, read from the
/// projected node data. `None` means the dictionary is not computed for this
/// node: the kind or the address is unknown to the grammar.
fn node_site(
    data: &Map<String, Value>,
    context: CanContext,
) -> Option<(
    QualifiedAddress,
    Option<TemplateFormat>,
    Option<FormItemRole>,
)> {
    NodeKind::parse(data.get("kind")?.as_str()?).ok()?;
    let at = QualifiedAddress::parse(data.get("at")?.as_str()?).ok()?;
    OperationSite::new(&at, context)?;
    let prop = |key: &str| {
        data.get("props")
            .and_then(|props| props.get(key))
            .and_then(Value::as_str)
    };
    let format = prop("format").map(TemplateFormat::parse);
    let item = prop("tag").map(FormItemRole::from_tag);
    Some((at, format, item))
}

/// The borrowed node carries no companion file of its own: its content is the
/// parent's, and operations writing that content do not apply.
fn inherited_node(data: &Map<String, Value>) -> bool {
    data.get("props")
        .and_then(|props| props.get("content"))
        .and_then(Value::as_str)
        == Some(crate::infrastructure::v13_read::INHERITED_CONTENT)
}

/// The operation dictionary of a node, computed from the one closed registry
/// that also validates `apply` calls: registry × applicability to the node's
/// site (source-set kind, route class and address shape) and, for a
/// template, the format filter. `None` means the dictionary is not computed
/// for this node: the answer then carries no `can`. A computed dictionary
/// may be empty (`[]`): nothing applies at this node, which is an answer.
fn computed_can_entries(data: &Map<String, Value>, context: CanContext) -> Option<Vec<Value>> {
    let (at, format, item) = node_site(data, context)?;
    let site = OperationSite::new(&at, context)?;
    // Содержимое заимствованного узла без своего файла принадлежит
    // родителю: операции, которые пишут это содержимое, `apply` отвергнет,
    // и словарь их не предлагает.
    let inherited = inherited_node(data);
    Some(
        OperationRegistry::closed()
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.applies_to_node(&site, format, item))
            .filter(|descriptor| !inherited || descriptor.content_resource().is_none())
            .map(|descriptor| {
                json!({
                    "op": descriptor.name(),
                    "args": descriptor.skeleton_key(),
                    "implemented": IMPLEMENTED_APPLY_OPERATIONS.contains(&descriptor.name()),
                })
            })
            .collect(),
    )
}

/// Every `view` answer of a node with a computed dictionary carries `can`:
/// what `apply` accepts here is part of the answer, not a separate question.
pub(super) fn attach_can(data: &mut Value, context: CanContext) {
    let Some(node) = data.as_object_mut() else {
        return;
    };
    if let Some(entries) = computed_can_entries(node, context) {
        node.insert("can".to_string(), Value::Array(entries));
    }
}

/// The answer to `<node>.Can.<op>`: one operation of the node's dictionary
/// with its argument contract narrowed to this node. `data` is the owning
/// node as `view` reads it; `at` is the canonical `Can` address.
pub(super) fn operation_node(
    data: &Value,
    operation: &str,
    at: &str,
    context: CanContext,
) -> Result<Value, ReadModeError> {
    let data = data
        .as_object()
        .ok_or_else(|| ReadModeError::bad_value("operation details require object node data"))?;
    let (owner, format, item) = node_site(data, context).ok_or_else(|| {
        let kind = data.get("kind").and_then(Value::as_str).unwrap_or_default();
        ReadModeError::unsupported_section(format!(
            "the operation dictionary is not computed for kind `{kind}` yet"
        ))
    })?;
    let descriptor = OperationRegistry::closed()
        .lookup(operation)
        .ok_or_else(|| {
            ReadModeError::bad_value(format!(
                "unknown operation `{operation}`; inspect the node can dictionary"
            ))
        })?;
    let site = OperationSite::new(&owner, context).expect("node_site proved the operation site");
    if inherited_node(data) && descriptor.content_resource().is_some() {
        return Err(ReadModeError::bad_value(format!(
            "operation `{operation}` writes content this borrowed node inherits from the parent; read the parent named by props.extends"
        )));
    }
    if !descriptor.applies_to_node(&site, format, item) {
        let kind = owner
            .segments()
            .last()
            .map_or("", |segment| segment.kind().as_str());
        return Err(ReadModeError::bad_value(format!(
            "operation `{operation}` does not apply to {kind}"
        )));
    }
    let implemented = IMPLEMENTED_APPLY_OPERATIONS.contains(&operation);
    let mut contract = descriptor.argument_contract_at(&site);
    // Командная панель принимает только кнопки: её контракт `element.add`
    // публикует один вид, как и отвечает писатель (#1447).
    let command_bar = data
        .get("props")
        .and_then(|props| props.get("tag"))
        .and_then(Value::as_str)
        .is_some_and(|tag| tag.starts_with("[AutoCommandBar") || tag.starts_with("[CmdBar"));
    if operation == "element.add" && command_bar {
        contract = Some(
            crate::domain::operation_contract::OperationContract::form_element_add_in_command_bar(),
        );
    }
    if implemented && contract.is_none() {
        return Err(ReadModeError::unsupported_section(format!(
            "detailed argument contract for `{operation}` is not available yet"
        )));
    }
    Ok(json!({
        "at": at,
        "kind": NodeKind::Can.as_str(),
        "title": operation,
        "props": {
            "op": operation,
            "args": descriptor.skeleton_key(),
            "implemented": implemented,
            "contract": contract.map(|contract| contract.details()),
        },
    }))
}

/// `diff` compares what the sources say. The dictionary is computed from the
/// registry and the node's place, so comparing it never observes a source
/// change: it leaves the compared data.
pub(super) fn source_backed(data: &mut Value) {
    if let Some(node) = data.as_object_mut() {
        node.remove("can");
        node.remove("limits");
    }
}

#[cfg(test)]
mod operation_dictionary_tests {
    use super::{attach_can, operation_node};
    use crate::domain::refusal::RefusalCode;
    use serde_json::{json, Value};

    fn can_of(mut data: Value) -> Value {
        attach_can(&mut data, super::configuration_context());
        data
    }

    #[test]
    fn detailed_dcs_can_publishes_structured_operands_and_refuses_retired_schema_input() {
        let data = json!({"at":"cf:Report.F05Report.Template.F05Schema.DataSet.F05Data.Query","kind":"Query"});
        let brief = can_of(data.clone());
        let query = brief["can"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["op"] == "query.set")
            .unwrap();
        assert_eq!(
            query,
            &json!({"op":"query.set","args":"values","implemented":true})
        );
        let at = "cf:Report.F05Report.Template.F05Schema.DataSet.F05Data.Query.Can.query.set";
        let detailed =
            operation_node(&data, "query.set", at, super::configuration_context()).unwrap();
        assert_eq!(detailed["at"], at);
        assert_eq!(detailed["kind"], "Can");
        assert_eq!(detailed["props"]["op"], "query.set");
        assert_eq!(detailed["props"]["implemented"], true);
        assert_eq!(
            detailed["props"]["contract"]["argsSchema"]["properties"]["values"]["properties"]
                ["query"]["type"],
            "string"
        );
        assert!(serde_json::to_vec(&detailed).unwrap().len() < 64 * 1024);
        for op in ["dcs.set", "query.unknown", "enumValue.add"] {
            assert_eq!(
                operation_node(&data, op, at, super::configuration_context())
                    .unwrap_err()
                    .code(),
                RefusalCode::BadValue,
                "{op}"
            );
        }
    }

    #[test]
    fn dcs_and_mxl_template_can_follow_the_actual_body_format() {
        for (format, accepted, rejected) in [
            ("DataCompositionSchema", "dataSet.add", "area.add"),
            ("SpreadsheetDocument", "area.add", "dataSet.add"),
        ] {
            let data = json!({"at":"cf:Report.F05Report.Template.Layout","kind":"Template","title":"Layout","props":{"format":format}});
            let listed = can_of(data.clone());
            let names = listed["can"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["op"].as_str().unwrap().to_string())
                .collect::<Vec<_>>();
            assert!(names.iter().any(|name| name == accepted), "{format}");
            assert!(!names.iter().any(|name| name == rejected), "{format}");
            let at = "cf:Report.F05Report.Template.Layout.Can.x";
            operation_node(&data, accepted, at, super::configuration_context()).unwrap();
            assert_eq!(
                operation_node(&data, rejected, at, super::configuration_context())
                    .unwrap_err()
                    .code(),
                RefusalCode::BadValue
            );
        }
    }

    #[test]
    fn every_node_answer_carries_its_dictionary_and_keeps_its_own_slots() {
        let projected = can_of(json!({
            "at": "main:Catalog.Items",
            "kind": "Catalog",
            "title": "Items",
            "props": {"hierarchical": false},
            "branches": [{"at": "main:Catalog.Items.Attribute", "count": 2}]
        }));
        assert_eq!(projected["props"], json!({"hierarchical": false}));
        assert_eq!(projected["branches"][0]["count"], 2);
        let can = projected["can"].as_array().expect("computed can entries");
        let entry = |op: &str| {
            can.iter()
                .find(|entry| entry["op"] == op)
                .unwrap_or_else(|| panic!("missing `{op}`"))
                .clone()
        };
        // The dictionary comes from the one closed registry × applicability,
        // with the honesty flag mirroring the Run dictionary.
        assert_eq!(
            entry("props.set"),
            json!({"op": "props.set", "args": "values", "implemented": true})
        );
        assert_eq!(
            entry("attribute.add"),
            json!({"op": "attribute.add", "args": "items", "implemented": true})
        );
        // A new object is created by its parent, the configuration root.
        assert!(can.iter().all(|entry| entry["op"] != "object.create"));
        // A metadata object's own attribute operations live on the attribute.
        assert!(can.iter().all(|entry| entry["op"] != "attribute.set"));
        assert_eq!(
            entry("form.add"),
            json!({"op": "form.add", "args": "items", "implemented": true})
        );
        assert_eq!(
            entry("object.remove"),
            json!({"op": "object.remove", "args": "at", "implemented": true})
        );
        assert!(
            can.iter().all(|entry| entry["op"] != "enumValue.add"),
            "a Catalog node must not advertise Enum-only operations"
        );
        let data_set = can_of(
            json!({"at":"main:Report.Sales.Template.Layout.DataSet.Main","kind":"DataSet","title":"Main","props":{}}),
        );
        let data_set_can = data_set["can"].as_array().expect("computed can entries");
        assert!(data_set_can.iter().all(|entry| entry["op"] != "dcs.set"));
        assert!(data_set_can
            .iter()
            .any(|entry| entry == &json!({"op":"dataSet.set","args":"values","implemented":true})));
    }

    #[test]
    fn a_node_without_a_computed_dictionary_has_no_can_and_its_operation_is_refused() {
        let module = can_of(json!({
            "at": "main:CommonModule.Общий",
            "kind": "Module",
            "title": "Common module Общий"
        }));
        let can = module["can"].as_array().expect("module can entries");
        assert!(can.iter().any(|entry| entry["op"] == "code.insert"));
        assert!(can.iter().all(|entry| entry["op"] != "attribute.add"));

        let unknown = json!({"at": "main:Mystery.X", "kind": "Mystery", "title": "X"});
        assert!(can_of(unknown.clone()).get("can").is_none());
        // Вид без словаря отказывает по виду, даже если и операции нет.
        assert_eq!(
            operation_node(
                &unknown,
                "no.such",
                "main:Mystery.X.Can.no.such",
                super::configuration_context()
            )
            .unwrap_err()
            .code()
            .as_str(),
            "unsupported_section"
        );
        let refusal = operation_node(
            &unknown,
            "props.set",
            "main:Mystery.X.Can.props.set",
            super::configuration_context(),
        )
        .unwrap_err();
        assert_eq!(refusal.code().as_str(), "unsupported_section");

        let configuration = can_of(json!({"at": "main:Configuration", "kind": "Configuration"}));
        let can = configuration["can"].as_array().expect("root can entries");
        assert!(can.iter().any(|entry| entry["op"] == "subsystem.create"));
        assert!(can.iter().any(|entry| entry["op"] == "object.create"));
        // A configuration root is not an extension: nothing to borrow into.
        assert!(can.iter().all(|entry| entry["op"] != "object.borrow"));
    }

    /// `diff` сравнивает исходники: словарь вычисляется из реестра и места
    /// узла и из сравнения уходит, а данные узла остаются.
    #[test]
    fn diff_compares_source_backed_data_without_the_computed_dictionary() {
        let mut data = can_of(json!({
            "at": "main:Catalog.Items",
            "kind": "Catalog",
            "title": "Items",
            "props": {"synonym": "Items"}
        }));
        assert!(data.get("can").is_some());
        super::source_backed(&mut data);
        assert_eq!(
            data,
            json!({"at": "main:Catalog.Items", "kind": "Catalog", "title": "Items", "props": {"synonym": "Items"}})
        );
    }
}

pub(super) fn search_scope_prefix(
    scope: &QualifiedAddress,
) -> Result<Option<String>, ReadModeError> {
    if scope.segments().len() != 1 {
        return Err(ReadModeError::unsupported_scope(
            "literal search currently accepts only Configuration or one metadata-object subtree",
        ));
    }
    let Some(owner) = scope.segments().first() else {
        return Err(ReadModeError::bad_value(
            "search scope has no logical segments",
        ));
    };
    if owner.kind() == NodeKind::Configuration {
        return Ok(None);
    }
    let layout = metadata_kind(owner.kind().as_str()).ok_or_else(|| {
        ReadModeError::unsupported_scope(format!(
            "search scope kind `{}` has no Platform XML subtree",
            owner.kind().as_str()
        ))
    })?;
    let mut prefix = layout.directory.to_string();
    if let Some(name) = owner.name() {
        prefix.push('/');
        prefix.push_str(name);
    }
    Ok(Some(prefix))
}

#[cfg(test)]
mod tests {
    use super::search_scope_prefix;
    use crate::domain::address::QualifiedAddress;

    #[test]
    fn logical_scope_maps_to_a_retained_relative_prefix_without_accepting_paths() {
        let root = QualifiedAddress::parse("main:Configuration").unwrap();
        let catalog = QualifiedAddress::parse("main:Catalog.Items").unwrap();
        assert_eq!(search_scope_prefix(&root).unwrap(), None);
        assert_eq!(
            search_scope_prefix(&catalog).unwrap().as_deref(),
            Some("Catalogs/Items")
        );
        let attribute = QualifiedAddress::parse("main:Catalog.Items.Attribute.Code").unwrap();
        assert!(search_scope_prefix(&attribute).is_err());
    }
}

#[cfg(test)]
mod inherited_can_tests {
    use super::{attach_can, operation_node};
    use crate::domain::apply::SourceSetTraits;
    use serde_json::{json, Value};

    fn extension() -> super::CanContext {
        SourceSetTraits::read(
            crate::domain::project_sources::SourceSetKind::Extension,
            crate::domain::platform_profile::PlatformProfile::v8_3_27(),
        )
    }

    fn ops(data: &Value) -> Vec<String> {
        let mut data = data.clone();
        attach_can(&mut data, extension());
        data["can"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["op"].as_str().unwrap().to_string())
            .collect()
    }

    /// #1381: словарь заимствованного узла без своего файла — и в каждом
    /// ответе `view`, и по адресу `.Can.<op>` — не предлагает операций,
    /// которые пишут унаследованное содержимое; описание объекта пишется.
    #[test]
    fn inherited_node_can_hides_content_writers() {
        let own = json!({"at":"ext:Role.R","kind":"Role","title":"R","props":{}});
        let inherited =
            json!({"at":"ext:Role.R","kind":"Role","title":"R","props":{"content":"inherited"}});
        assert!(ops(&own).iter().any(|op| op == "right.add"));
        let inherited_ops = ops(&inherited);
        assert!(!inherited_ops.iter().any(|op| op == "right.add"));
        assert!(inherited_ops.iter().any(|op| op == "props.set"));

        let form = json!({"at":"ext:DataProcessor.D.Form.F","kind":"Form","title":"F","props":{"content":"inherited"}});
        let form_ops = ops(&form);
        assert!(!form_ops
            .iter()
            .any(|op| op == "element.add" || op == "event.bind"));
        assert!(form_ops.iter().any(|op| op == "form.remove"));

        let at = "ext:Role.R.Can.right.add";
        let refused = operation_node(&inherited, "right.add", at, extension())
            .expect_err("a content writer is not readable through Can");
        assert!(refused.to_string().contains("inherits from the parent"));
        if let Err(error) = operation_node(&own, "right.add", at, extension()) {
            assert!(!error.to_string().contains("inherits from the parent"));
        }
        if let Err(error) = operation_node(
            &inherited,
            "props.set",
            "ext:Role.R.Can.props.set",
            extension(),
        ) {
            assert!(!error.to_string().contains("inherits from the parent"));
        }
    }
}
