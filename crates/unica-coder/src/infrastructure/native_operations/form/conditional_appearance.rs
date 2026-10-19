use roxmltree::Node;
use serde::Serialize;
use serde_json::{json, Map, Value};

const FORM: &str = "http://v8.1c.ru/8.3/xcf/logform";
const SETTINGS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
const CORE: &str = "http://v8.1c.ru/8.1/data-composition-system/core";
const V8: &str = "http://v8.1c.ru/8.1/data/core";
const UI: &str = "http://v8.1c.ru/8.1/data/ui";
const XS: &str = "http://www.w3.org/2001/XMLSchema";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

#[derive(Debug, Serialize)]
pub(crate) struct ConditionalAppearance {
    rules: Vec<Rule>,
    limits: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Rule {
    name: String,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    appearance: Option<Value>,
    limits: Vec<String>,
}

type Read<T> = Result<T, String>;

fn children<'a, 'i>(node: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(Node::is_element)
}

fn only(node: Node<'_, '_>, namespace: &str, names: &[&str], typed: bool) -> Read<()> {
    if node
        .attributes()
        .any(|a| !(typed && a.namespace() == Some(XSI) && a.name() == "type"))
    {
        return Err("неподдержанные атрибуты".into());
    }
    if children(node).any(|c| {
        c.tag_name().namespace() != Some(namespace) || !names.contains(&c.tag_name().name())
    }) {
        return Err("неподдержанный дочерний элемент".into());
    }
    if node
        .children()
        .any(|c| c.is_text() && !c.text().unwrap_or_default().trim().is_empty())
        && !names.is_empty()
    {
        return Err("неожиданный текст контейнера".into());
    }
    Ok(())
}

fn one<'a, 'i>(node: Node<'a, 'i>, namespace: &str, name: &str) -> Read<Option<Node<'a, 'i>>> {
    let mut found = children(node).filter(|c| c.has_tag_name((namespace, name)));
    let first = found.next();
    if found.next().is_some() {
        return Err(format!("повторяющийся {name}"));
    }
    Ok(first)
}

fn required<'a, 'i>(node: Node<'a, 'i>, namespace: &str, name: &str) -> Read<Node<'a, 'i>> {
    one(node, namespace, name)?.ok_or_else(|| format!("отсутствует {name}"))
}

fn text(node: Node<'_, '_>) -> Read<String> {
    only(node, "", &[], false)?;
    Ok(node
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect())
}

fn boolean(node: Node<'_, '_>) -> Read<bool> {
    match text(node)?.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err("повреждено логическое значение".into()),
    }
}

fn optional_use(node: Node<'_, '_>, namespace: &str, out: &mut Map<String, Value>) -> Read<()> {
    if let Some(value) = one(node, namespace, "use")? {
        out.insert("use".into(), json!(boolean(value)?));
    }
    Ok(())
}

fn type_name(node: Node<'_, '_>) -> Read<(String, String)> {
    let value = node
        .attribute((XSI, "type"))
        .ok_or("отсутствует xsi:type")?;
    let (prefix, local) = value
        .split_once(':')
        .map_or((None, value), |(p, n)| (Some(p), n));
    let namespace = node
        .lookup_namespace_uri(prefix)
        .ok_or("неизвестное пространство имён xsi:type")?;
    Ok((namespace.to_string(), local.to_string()))
}

fn typed_value(node: Node<'_, '_>) -> Read<Value> {
    let (namespace, kind) = type_name(node)?;
    if namespace == V8 && kind == "LocalStringType" {
        only(node, V8, &["item"], true)?;
        let values = children(node).map(|item| {
            only(item, V8, &["lang", "content"], false)?;
            Ok(json!({"language": text(required(item, V8, "lang")?)?, "content": text(required(item, V8, "content")?)?}))
        }).collect::<Read<Vec<_>>>()?;
        return Ok(json!({"type":kind,"values":values}));
    }
    only(node, "", &[], true)?;
    let raw: String = node
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect();
    let value = match (namespace.as_str(), kind.as_str()) {
        (CORE, "Field") | (XS, "string") => json!(raw),
        (UI, "Color") => {
            if let Some((prefix, name)) = raw.split_once(':') {
                let canonical = match node.lookup_namespace_uri(Some(prefix)) {
                    Some("http://v8.1c.ru/8.1/data/ui/colors/windows") => "win",
                    Some("http://v8.1c.ru/8.1/data/ui/colors/web") => "web",
                    Some("http://v8.1c.ru/8.1/data/ui/style") => "style",
                    _ => return Err("неподдержанное пространство имён цвета".into()),
                };
                if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    return Err("повреждено имя цвета".into());
                }
                json!(format!("{canonical}:{name}"))
            } else if raw.len() == 7
                && raw.starts_with('#')
                && raw[1..].chars().all(|c| c.is_ascii_hexdigit())
            {
                json!(raw)
            } else {
                return Err("неподдержанное значение цвета".into());
            }
        }
        (XS, "boolean") => match raw.trim() {
            "true" | "1" => json!(true),
            "false" | "0" => json!(false),
            _ => return Err("повреждено boolean".into()),
        },
        (XS, "decimal") => {
            let raw = raw.trim();
            let digits = raw.strip_prefix(['+', '-']).unwrap_or(raw);
            if !digits.chars().any(|c| c.is_ascii_digit())
                || digits.chars().any(|c| !c.is_ascii_digit() && c != '.')
                || digits.matches('.').count() > 1
            {
                return Err("повреждено decimal".into());
            }
            // Decimal stays lexical: converting to f64 would lose platform precision.
            json!(raw)
        }
        _ => return Err(format!("неподдержанный тип {{{namespace}}}{kind}")),
    };
    Ok(json!({"type":kind,"value":value}))
}

fn selection(node: Node<'_, '_>) -> Read<Value> {
    only(node, SETTINGS, &["item"], false)?;
    children(node)
        .map(|item| {
            only(item, SETTINGS, &["field", "use"], false)?;
            let mut value = Map::new();
            value.insert(
                "field".into(),
                json!(text(required(item, SETTINGS, "field")?)?),
            );
            optional_use(item, SETTINGS, &mut value)?;
            Ok(Value::Object(value))
        })
        .collect::<Read<Vec<_>>>()
        .map(Value::Array)
}

fn filter_item(node: Node<'_, '_>) -> Read<Value> {
    let (namespace, kind) = type_name(node)?;
    if namespace != SETTINGS {
        return Err("неподдержанное пространство имён условия".into());
    }
    let mut value = Map::new();
    value.insert("type".into(), json!(kind));
    match kind.as_str() {
        "FilterItemComparison" => {
            only(
                node,
                SETTINGS,
                &["left", "right", "comparisonType", "use"],
                true,
            )?;
            value.insert(
                "left".into(),
                typed_value(required(node, SETTINGS, "left")?)?,
            );
            let comparison = text(required(node, SETTINGS, "comparisonType")?)?;
            if ![
                "Equal",
                "NotEqual",
                "Less",
                "LessOrEqual",
                "Greater",
                "GreaterOrEqual",
                "InList",
                "NotInList",
                "InHierarchy",
                "NotInHierarchy",
                "InListByHierarchy",
                "NotInListByHierarchy",
                "Contains",
                "NotContains",
                "BeginsWith",
                "Like",
                "NotLike",
                "Filled",
                "NotFilled",
            ]
            .contains(&comparison.as_str())
            {
                return Err(format!("неподдержанный comparisonType {comparison}"));
            }
            value.insert("comparisonType".into(), json!(comparison));
            if let Some(right) = one(node, SETTINGS, "right")? {
                value.insert("right".into(), typed_value(right)?);
            } else if !["Filled", "NotFilled"].contains(&comparison.as_str()) {
                return Err("отсутствует right".into());
            }
        }
        "FilterItemGroup" => {
            only(node, SETTINGS, &["item", "groupType", "use"], true)?;
            let group = text(required(node, SETTINGS, "groupType")?)?;
            if !["AndGroup", "OrGroup", "NotGroup"].contains(&group.as_str()) {
                return Err(format!("неподдержанный groupType {group}"));
            }
            value.insert("groupType".into(), json!(group));
            let items = children(node)
                .filter(|n| n.has_tag_name((SETTINGS, "item")))
                .map(filter_item)
                .collect::<Read<Vec<_>>>()?;
            value.insert("items".into(), json!(items));
        }
        _ => return Err(format!("неподдержанный тип условия {kind}")),
    }
    optional_use(node, SETTINGS, &mut value)?;
    Ok(Value::Object(value))
}

fn filter(node: Node<'_, '_>) -> Read<Value> {
    only(node, SETTINGS, &["item"], false)?;
    children(node)
        .map(filter_item)
        .collect::<Read<Vec<_>>>()
        .map(Value::Array)
}

fn appearance(node: Node<'_, '_>) -> Read<Value> {
    only(node, CORE, &["item"], false)?;
    children(node)
        .map(|item| {
            if type_name(item)? != (SETTINGS.to_string(), "SettingsParameterValue".to_string()) {
                return Err("неподдержанный тип параметра оформления".into());
            }
            only(item, CORE, &["parameter", "value", "use"], true)?;
            let mut value = Map::new();
            value.insert("type".into(), json!("SettingsParameterValue"));
            value.insert(
                "parameter".into(),
                json!(text(required(item, CORE, "parameter")?)?),
            );
            value.insert("value".into(), typed_value(required(item, CORE, "value")?)?);
            optional_use(item, CORE, &mut value)?;
            Ok(Value::Object(value))
        })
        .collect::<Read<Vec<_>>>()
        .map(Value::Array)
}

fn bounded(node: Node<'_, '_>, parse: fn(Node<'_, '_>) -> Read<Value>) -> Read<Value> {
    // Bound the recursive, compact fact independently; never truncate valid JSON.
    if node.descendants().filter(Node::is_element).count() > 256
        || node
            .descendants()
            .any(|n| n.ancestors().take_while(|a| *a != node).count() > 32)
    {
        return Err(
            "структура превышает предел компактного чтения (256 элементов, глубина 32)".into(),
        );
    }
    let value = parse(node)?;
    if value.to_string().len() > 2048 {
        return Err("значение превышает 2048 байт UTF-8".into());
    }
    Ok(value)
}

fn fact(
    node: Node<'_, '_>,
    name: &str,
    parse: fn(Node<'_, '_>) -> Read<Value>,
    limits: &mut Vec<String>,
) -> Option<Value> {
    match one(node, SETTINGS, name).and_then(|n| n.map(|n| bounded(n, parse)).transpose()) {
        Ok(value) => value,
        Err(error) => {
            limits.push(format!("{name}: {error}"));
            None
        }
    }
}

pub(super) fn read(root: Node<'_, '_>) -> Option<ConditionalAppearance> {
    let attrs = children(root).find(|n| n.has_tag_name((FORM, "Attributes")))?;
    let mut containers = children(attrs).filter(|n| n.tag_name().name() == "ConditionalAppearance");
    let container = containers.next()?;
    let mut result = ConditionalAppearance {
        rules: Vec::new(),
        limits: Vec::new(),
    };
    if containers.next().is_some() || !container.has_tag_name((FORM, "ConditionalAppearance")) {
        result.limits.push(
            "ConditionalAppearance: повторяющийся контейнер или неподдержанное пространство имён"
                .into(),
        );
    }
    if let Err(error) = only(container, SETTINGS, &["item"], false) {
        result
            .limits
            .push(format!("ConditionalAppearance: {error}"));
    }
    for item in children(container).filter(|n| n.tag_name().name() == "item") {
        let name = format!("Rule{}", result.rules.len() + 1);
        let mut limits = Vec::new();
        if !item.has_tag_name((SETTINGS, "item")) {
            limits.push(format!("{name}: неподдержанное пространство имён правила"));
            result.rules.push(Rule {
                name,
                enabled: None,
                selection: None,
                filter: None,
                appearance: None,
                limits,
            });
            continue;
        }
        if let Err(error) = only(
            item,
            SETTINGS,
            &["selection", "filter", "appearance", "use"],
            false,
        ) {
            limits.push(format!("{name}: {error}"));
        }
        let enabled = match one(item, SETTINGS, "use").and_then(|n| n.map(boolean).transpose()) {
            Ok(value) => value,
            Err(error) => {
                limits.push(format!("use: {error}"));
                None
            }
        };
        let selection = fact(item, "selection", selection, &mut limits);
        let filter = fact(item, "filter", filter, &mut limits);
        let appearance = fact(item, "appearance", appearance, &mut limits);
        result.rules.push(Rule {
            name,
            enabled,
            selection,
            filter,
            appearance,
            limits,
        });
    }
    Some(result)
}
