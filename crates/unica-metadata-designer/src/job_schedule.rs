//! The job schedule companion `ScheduledJobs/<Name>/Ext/Schedule.xml` over
//! already retained bytes. Only the observed `JobSchedule` 2.20 profile with a
//! complete `Schedule` is accepted; every other present form is refused, never
//! reported as an absent or default schedule.
use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};

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

fn children<'a, 'input>(node: Node<'a, 'input>) -> Result<Vec<Node<'a, 'input>>, String> {
    if node
        .children()
        .any(|child| child.is_text() && !child.text().unwrap_or_default().trim().is_empty())
    {
        return Err("JobSchedule contains unsupported text".to_string());
    }
    Ok(node.children().filter(Node::is_element).collect())
}

/// Facts of a present schedule: `state: present`, the twelve `Schedule`
/// attributes under their source names, and `WeekDays`/`Months` as
/// space-separated integers in source order.
pub fn parse_job_schedule(text: &str) -> Result<Value, String> {
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((EXTERNAL, "JobSchedule"))
        || root.attribute("version") != Some("2.20")
        || root
            .attributes()
            .any(|attr| attr.namespace().is_some() || attr.name() != "version")
    {
        return Err("unsupported JobSchedule root or version".to_string());
    }
    let nodes = children(root)?;
    let [schedule] = nodes.as_slice() else {
        return Err("JobSchedule must contain one supported Schedule".to_string());
    };
    if !schedule.has_tag_name((EXTERNAL, "Schedule"))
        || schedule
            .attributes()
            .any(|attr| attr.namespace().is_some() || !ATTRIBUTES.contains(&attr.name()))
    {
        return Err("unsupported Schedule structure".to_string());
    }
    let mut props = Map::new();
    props.insert("state".into(), json!("present"));
    for &name in ATTRIBUTES {
        let value = schedule
            .attribute(name)
            .ok_or_else(|| format!("Schedule.{name} is absent"))?;
        let value = match name {
            "BeginDate" | "EndDate" => {
                chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .map_err(|_| format!("invalid Schedule.{name}"))?;
                json!(value)
            }
            "BeginTime" | "EndTime" | "CompletionTime" => {
                chrono::NaiveTime::parse_from_str(value, "%H:%M:%S")
                    .map_err(|_| format!("invalid Schedule.{name}"))?;
                json!(value)
            }
            _ => json!(value
                .parse::<u32>()
                .map_err(|_| format!("invalid Schedule.{name}"))?),
        };
        props.insert(name.into(), value);
    }
    let nodes = children(*schedule)?;
    if nodes.len() != 2 {
        return Err("Schedule must contain WeekDays and Months".to_string());
    }
    for (name, maximum) in [("WeekDays", 7), ("Months", 12)] {
        let mut matching = nodes
            .iter()
            .filter(|node| node.has_tag_name((ENTERPRISE, name)));
        let node = matching
            .next()
            .ok_or_else(|| format!("Schedule.{name} is absent"))?;
        if matching.next().is_some()
            || node.attributes().len() != 0
            || node.children().any(|child| child.is_element())
        {
            return Err(format!("unsupported Schedule.{name}"));
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
                .map_err(|_| format!("invalid Schedule.{name}"))?;
            if !(1..=maximum).contains(&value) || values.contains(&value) {
                return Err(format!("invalid Schedule.{name}"));
            }
            values.push(value);
        }
        if values.is_empty() {
            return Err(format!("empty Schedule.{name} is unsupported"));
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
