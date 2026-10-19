use super::{ProviderReadAuthority, ViewError};
use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};
use unica_address::source_target::MetadataAddress;
use unica_refusal::RefusalDetail;

const EXTERNAL: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
const ENTERPRISE: &str = "http://v8.1c.ru/8.1/data/enterprise";
const ATTRIBUTES: &[&str] = &[
    "BeginDate",
    "EndDate",
    "BeginTime",
    "EndTime",
    "CompletionTime",
    "CompletionInterval",
    "RepeatPeriodInDay",
    "RepeatPause",
    "WeekDayInMonth",
    "DayInMonth",
    "WeeksPeriod",
    "DaysRepeatPeriod",
];

impl ProviderReadAuthority {
    pub(crate) fn job_schedule(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        checkpoint()?;
        let relative = self.attached_resource_relative(target, "Schedule.xml")?;
        let bytes =
            self.read_optional_relative_with_checkpoint(&relative, Some(256 * 1024), checkpoint)?;
        let Some(bytes) = bytes else {
            return Ok(json!({"state":"absent"}));
        };
        let text =
            std::str::from_utf8(&bytes).map_err(|_| unreadable("Schedule.xml is not UTF-8"))?;
        let result = parse(text);
        checkpoint()?;
        result
    }
}

fn unreadable(message: impl Into<String>) -> ViewError {
    ViewError::detailed(RefusalDetail::SourceUnreadable, message)
}

fn children<'a, 'input>(node: Node<'a, 'input>) -> Result<Vec<Node<'a, 'input>>, ViewError> {
    if node
        .children()
        .any(|child| child.is_text() && !child.text().unwrap_or_default().trim().is_empty())
    {
        return Err(unreadable("JobSchedule contains unsupported text"));
    }
    Ok(node.children().filter(Node::is_element).collect())
}

fn parse(text: &str) -> Result<Value, ViewError> {
    let document = Document::parse(text).map_err(|error| unreadable(error.to_string()))?;
    let root = document.root_element();
    if !root.has_tag_name((EXTERNAL, "JobSchedule"))
        || root.attribute("version") != Some("2.20")
        || root
            .attributes()
            .any(|attr| attr.namespace().is_some() || attr.name() != "version")
    {
        return Err(unreadable("unsupported JobSchedule root or version"));
    }
    let nodes = children(root)?;
    let [schedule] = nodes.as_slice() else {
        return Err(unreadable(
            "JobSchedule must contain one supported Schedule",
        ));
    };
    if !schedule.has_tag_name((EXTERNAL, "Schedule"))
        || schedule
            .attributes()
            .any(|attr| attr.namespace().is_some() || !ATTRIBUTES.contains(&attr.name()))
    {
        return Err(unreadable("unsupported Schedule structure"));
    }
    let mut props = Map::new();
    props.insert("state".into(), json!("present"));
    for &name in ATTRIBUTES {
        let value = schedule
            .attribute(name)
            .ok_or_else(|| unreadable(format!("Schedule.{name} is absent")))?;
        let value = match name {
            "BeginDate" | "EndDate" => {
                chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .map_err(|_| unreadable(format!("invalid Schedule.{name}")))?;
                json!(value)
            }
            "BeginTime" | "EndTime" | "CompletionTime" => {
                chrono::NaiveTime::parse_from_str(value, "%H:%M:%S")
                    .map_err(|_| unreadable(format!("invalid Schedule.{name}")))?;
                json!(value)
            }
            _ => json!(value
                .parse::<u32>()
                .map_err(|_| unreadable(format!("invalid Schedule.{name}")))?),
        };
        props.insert(name.into(), value);
    }
    let nodes = children(*schedule)?;
    if nodes.len() != 2 {
        return Err(unreadable("Schedule must contain WeekDays and Months"));
    }
    for (name, maximum) in [("WeekDays", 7), ("Months", 12)] {
        let mut matching = nodes
            .iter()
            .filter(|node| node.has_tag_name((ENTERPRISE, name)));
        let node = matching
            .next()
            .ok_or_else(|| unreadable(format!("Schedule.{name} is absent")))?;
        if matching.next().is_some()
            || node.attributes().len() != 0
            || node.children().any(|child| child.is_element())
        {
            return Err(unreadable(format!("unsupported Schedule.{name}")));
        }
        let text = node
            .children()
            .filter(Node::is_text)
            .filter_map(|node| node.text())
            .collect::<String>();
        let mut values = Vec::new();
        for token in text.split_whitespace() {
            let value = token
                .parse::<u32>()
                .map_err(|_| unreadable(format!("invalid Schedule.{name}")))?;
            if !(1..=maximum).contains(&value) || values.contains(&value) {
                return Err(unreadable(format!("invalid Schedule.{name}")));
            }
            values.push(value);
        }
        if values.is_empty() {
            return Err(unreadable(format!("empty Schedule.{name} is unsupported")));
        }
        props.insert(
            name.into(),
            json!(values
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" ")),
        );
    }
    Ok(Value::Object(props))
}
