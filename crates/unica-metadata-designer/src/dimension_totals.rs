//! Local mutation and semantic evidence for accumulation dimension totals.
use roxmltree::{Document, Node};
use unica_metadata::{MetaDiagnostic, MetaDiagnosticCode};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
fn bad(message: &str) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message).with_field("useInTotals")
}
fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Node<'a, 'i>, MetaDiagnostic> {
    let mut found = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let value = found
        .next()
        .ok_or_else(|| bad(&format!("{name} is missing; no default is assumed")))?;
    if found.next().is_some() || !value.has_tag_name((MD, name)) || value.attributes().len() != 0 {
        return Err(bad(&format!("{name} must be one metadata property")));
    }
    Ok(value)
}
fn literal<'a>(
    xml: &'a str,
    node: Node<'_, '_>,
) -> Result<(&'a str, std::ops::Range<usize>), MetaDiagnostic> {
    let mut children = node.children();
    let text = children
        .next()
        .filter(|n| n.is_text())
        .ok_or_else(|| bad("expected a literal property value"))?;
    if children.next().is_some() {
        return Err(bad("expected a single literal property value"));
    }
    let range = text.range();
    let raw = &xml[range.clone()];
    let value = raw.trim();
    let start = range.start + raw.len() - raw.trim_start().len();
    Ok((value, start..start + value.len()))
}
fn flag(
    xml: &str,
    dimension: Node<'_, '_>,
) -> Result<(bool, std::ops::Range<usize>), MetaDiagnostic> {
    let property = child(child(dimension, "Properties")?, "UseInTotals")?;
    let (value, range) = literal(xml, property)?;
    match value {
        "true" => Ok((true, range)),
        "false" => Ok((false, range)),
        _ => Err(bad("UseInTotals must contain literal true or false")),
    }
}
fn valid_order(values: impl IntoIterator<Item = bool>) -> bool {
    let mut excluded = false;
    for included in values {
        if included && excluded {
            return false;
        }
        excluded |= !included;
    }
    true
}
/// Return the owned token after validating the pre-image. The caller validates
/// the complete updated image after applying all fields, including position.
pub(crate) fn edit(
    xml: &str,
    selected: std::ops::Range<usize>,
    value: bool,
) -> Result<Option<(std::ops::Range<usize>, &'static str)>, MetaDiagnostic> {
    validate_or_edit(xml, Some((selected, value)))
}
pub(crate) fn validate(xml: &str) -> Result<(), MetaDiagnostic> {
    validate_or_edit(xml, None).map(|_| ())
}
pub(crate) fn requested(operation: &unica_metadata::MetaEditOperation) -> bool {
    matches!(operation, unica_metadata::MetaEditOperation::Update { elements, .. }
        if elements.iter().any(|element| element.use_in_totals.is_some()))
}
fn validate_or_edit(
    xml: &str,
    change: Option<(std::ops::Range<usize>, bool)>,
) -> Result<Option<(std::ops::Range<usize>, &'static str)>, MetaDiagnostic> {
    let doc = Document::parse(xml).map_err(|_| bad("invalid metadata XML"))?;
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err(bad("expected metadata descriptor"));
    }
    let owners: Vec<_> = root.children().filter(Node::is_element).collect();
    let [owner] = owners.as_slice() else {
        return Err(bad("expected one accumulation register"));
    };
    if !owner.has_tag_name((MD, "AccumulationRegister")) {
        return Err(bad("useInTotals requires an accumulation register"));
    }
    let mode = child(child(*owner, "Properties")?, "RegisterType")?;
    let (mode, _) = literal(xml, mode)?;
    if !matches!(mode, "Balance" | "Turnovers") {
        return Err(bad("RegisterType must be Balance or Turnovers"));
    }
    let dimensions: Vec<_> = child(*owner, "ChildObjects")?
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Dimension")
        .collect();
    let mut flags = Vec::new();
    let mut target = None;
    for dimension in dimensions {
        if !dimension.has_tag_name((MD, "Dimension")) {
            return Err(bad("Dimension has a foreign namespace"));
        }
        let (current, range) = flag(xml, dimension)?;
        if change
            .as_ref()
            .is_some_and(|(selected, _)| dimension.range() == *selected)
        {
            target = Some((flags.len(), range));
        }
        flags.push(current);
    }
    let valid = |values: &[bool]| {
        if mode == "Balance" {
            values.iter().all(|v| *v)
        } else {
            valid_order(values.iter().copied())
        }
    };
    if !valid(&flags) {
        return Err(bad(
            "UseInTotals values violate register mode or dimension order",
        ));
    }
    let Some((_, value)) = change else {
        return Ok(None);
    };
    let (index, range) = target.ok_or_else(|| bad("selected dimension is not in the register"))?;
    let previous = flags[index];
    Ok((previous != value).then_some((range, if value { "true" } else { "false" })))
}
/// Effects of this writer retain the actual flag, independently of public read profiles.
pub(crate) fn effect(
    xml: &str,
    object: Node<'_, '_>,
    mut value: serde_json::Value,
) -> Result<serde_json::Value, MetaDiagnostic> {
    let parent = child(object, "ChildObjects")?;
    for item in value
        .as_array_mut()
        .ok_or_else(|| bad("dimension effect must be an array"))?
    {
        let name = item["name"]
            .as_str()
            .ok_or_else(|| bad("dimension effect has no name"))?;
        let dimension = parent
            .children()
            .filter(|n| n.has_tag_name((MD, "Dimension")))
            .find(|n| {
                child(*n, "Properties")
                    .and_then(|p| child(p, "Name"))
                    .is_ok_and(|n| {
                        n.text()
                            .is_some_and(|v| unica_metadata::metadata_name_eq(v, name))
                    })
            })
            .ok_or_else(|| bad("dimension effect target is unavailable"))?;
        item["UseInTotals"] = serde_json::Value::Bool(flag(xml, dimension)?.0);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use unica_metadata::{MetaCollection, MetaEditOperation, MetaElementUpdate};
    fn register() -> String {
        include_str!("../../../tests/fixtures/acceptance/workspace-register-dimensions/src/AccumulationRegisters/Остатки.xml")
            .trim_start_matches('\u{feff}').replace("<RegisterType>Balance</RegisterType>","<RegisterType>Turnovers</RegisterType>")
    }
    fn operation(name: &str, value: bool) -> MetaEditOperation {
        MetaEditOperation::Update {
            collection: MetaCollection::Dimensions,
            scope: None,
            elements: vec![MetaElementUpdate {
                name: name.into(),
                new_name: None,
                synonym: None,
                comment: None,
                r#type: None,
                required: None,
                fill_value: None,
                position: None,
                indexing: None,
                deny_incomplete_values: None,
                use_in_totals: Some(value),
            }],
        }
    }
    fn apply(
        xml: &mut String,
        ops: &[MetaEditOperation],
    ) -> Result<crate::edit::MetaEditCounts, unica_metadata::request::MetaFailure> {
        crate::edit::apply_typed_operations_with_uuid(xml, ops, "ru", &mut || {
            "11111111-1111-4111-8111-111111111111".into()
        })
    }
    #[test]
    fn totals_roundtrip_preserves_every_other_byte_and_semantic_effects() {
        for eol in ["\n", "\r\n"] {
            let before = register().replace("\r\n", "\n").replace('\n', eol);
            let mut after = before.clone();
            let result = apply(&mut after, &[operation("Склад", false)]).unwrap();
            assert_eq!(
                after,
                before
                    .rsplit_once("<UseInTotals>true</UseInTotals>")
                    .map(|(a, b)| format!("{a}<UseInTotals>false</UseInTotals>{b}"))
                    .unwrap()
            );
            assert_ne!(result.effects[0].before, result.effects[0].after);
            assert_eq!(
                result.effects[0].before.as_ref().unwrap()[0]["UseInTotals"],
                true
            );
            assert_eq!(
                result.effects[0].after.as_ref().unwrap()[0]["UseInTotals"],
                false
            );
            let saved = after.clone();
            let noop = apply(&mut after, &[operation("Склад", false)]).unwrap();
            assert_eq!(after, saved);
            assert_eq!(noop.effects[0].before, noop.effects[0].after);
            apply(&mut after, &[operation("Склад", true)]).unwrap();
            assert_eq!(after, before);
        }
    }
    #[test]
    fn totals_refuses_both_order_violations_and_balance_false() {
        let before = register();
        let mut after = before.clone();
        assert!(apply(&mut after, &[operation("Товар", false)]).is_err());
        assert_eq!(after, before);
        let all_false = before.replace(
            "<UseInTotals>true</UseInTotals>",
            "<UseInTotals>false</UseInTotals>",
        );
        let mut after = all_false.clone();
        assert!(apply(&mut after, &[operation("Склад", true)]).is_err());
        assert_eq!(after, all_false);
        let balance = before.replace("Turnovers", "Balance");
        let mut after = balance.clone();
        assert!(apply(&mut after, &[operation("Склад", false)]).is_err());
        assert_eq!(after, balance);
        apply(&mut after, &[operation("Склад", true)]).unwrap();
        assert_eq!(after, balance);
    }
    #[test]
    fn totals_does_not_repair_invalid_order_and_late_failure_is_atomic() {
        let before = register();
        let mut after = before.clone();
        assert!(apply(
            &mut after,
            &[operation("Склад", false), operation("Missing", false)]
        )
        .is_err());
        assert_eq!(after, before);
        let invalid = before.replacen(
            "<UseInTotals>true</UseInTotals>",
            "<UseInTotals>false</UseInTotals>",
            1,
        );
        let mut after = invalid.clone();
        assert!(apply(&mut after, &[operation("Товар", true)]).is_err());
        assert_eq!(after, invalid);
    }
    #[test]
    fn totals_rechecks_explicit_position_and_keeps_rename_effect_identity() {
        let before = register();
        let mut moved = operation("Склад", false);
        let MetaEditOperation::Update { elements, .. } = &mut moved else {
            unreachable!()
        };
        elements[0].position = Some(unica_metadata::MetaPosition {
            before: Some("Товар".into()),
            after: None,
        });
        let mut after = before.clone();
        assert!(apply(&mut after, &[moved]).is_err());
        assert_eq!(after, before);
        let mut renamed = operation("Склад", false);
        let MetaEditOperation::Update { elements, .. } = &mut renamed else {
            unreachable!()
        };
        elements[0].new_name = Some("Warehouse".into());
        let result = apply(&mut after, &[renamed]).unwrap();
        assert_eq!(
            result.effects[0].before.as_ref().unwrap()[0]["name"],
            "Склад"
        );
        assert_eq!(
            result.effects[0].after.as_ref().unwrap()[0]["name"],
            "Warehouse"
        );
        assert_eq!(
            result.effects[0].after.as_ref().unwrap()[0]["UseInTotals"],
            false
        );
    }
    #[test]
    fn totals_final_guard_catches_late_position_and_preserves_valid_batch() {
        let before = register();
        let mut moved = operation("Склад", false);
        let MetaEditOperation::Update { elements, .. } = &mut moved else {
            unreachable!()
        };
        elements[0].use_in_totals = None;
        elements[0].position = Some(unica_metadata::MetaPosition {
            before: Some("Товар".into()),
            after: None,
        });
        let mut after = before.clone();
        assert!(apply(&mut after, &[operation("Склад", false), moved.clone()]).is_err());
        assert_eq!(after, before);
        // An explicit no-op flag still requests final validation.
        let excluded = before
            .rsplit_once("<UseInTotals>true</UseInTotals>")
            .map(|(a, b)| format!("{a}<UseInTotals>false</UseInTotals>{b}"))
            .unwrap();
        let mut after = excluded.clone();
        assert!(apply(&mut after, &[operation("Склад", false), moved.clone()]).is_err());
        assert_eq!(after, excluded);
        // Moving two included dimensions is valid and still executes.
        let mut after = before.clone();
        apply(&mut after, &[operation("Склад", true), moved]).unwrap();
        assert_ne!(after, before);
    }
    #[test]
    fn totals_accepts_combined_flag_and_position_when_final_state_is_valid() {
        let mut xml = register().replace(
            "<UseInTotals>true</UseInTotals>",
            "<UseInTotals>false</UseInTotals>",
        );
        let mut moved = operation("Склад", true);
        let MetaEditOperation::Update { elements, .. } = &mut moved else {
            unreachable!()
        };
        elements[0].position = Some(unica_metadata::MetaPosition {
            before: Some("Товар".into()),
            after: None,
        });
        apply(&mut xml, &[moved]).unwrap();
        super::validate(&xml).unwrap();
        assert!(xml.find("<Name>Склад</Name>") < xml.find("<Name>Товар</Name>"));
    }
    #[test]
    fn totals_requires_proven_mode_and_unique_literal_flags() {
        for (old, new) in [
            ("<RegisterType>Turnovers</RegisterType>", ""),
            (
                "<RegisterType>Turnovers</RegisterType>",
                "<RegisterType>Future</RegisterType>",
            ),
            (
                "<RegisterType>Turnovers</RegisterType>",
                "<RegisterType>Turnovers</RegisterType><RegisterType>Turnovers</RegisterType>",
            ),
            (
                "<RegisterType>Turnovers</RegisterType>",
                "<RegisterType xmlns=\"urn:foreign\">Turnovers</RegisterType>",
            ),
            ("<UseInTotals>true</UseInTotals>", ""),
            ("<UseInTotals>true</UseInTotals>", "<UseInTotals/>"),
            (
                "<UseInTotals>true</UseInTotals>",
                "<UseInTotals>1</UseInTotals>",
            ),
            (
                "<UseInTotals>true</UseInTotals>",
                "<UseInTotals>true</UseInTotals><UseInTotals>false</UseInTotals>",
            ),
            (
                "<UseInTotals>true</UseInTotals>",
                "<UseInTotals xmlns=\"urn:foreign\">true</UseInTotals>",
            ),
            (
                "<UseInTotals>true</UseInTotals>",
                "<UseInTotals>true<!--x--></UseInTotals>",
            ),
            (
                "<UseInTotals>true</UseInTotals>",
                "<UseInTotals><![CDATA[true]]></UseInTotals>",
            ),
        ] {
            let before = register().replacen(old, new, 1);
            let mut after = before.clone();
            assert!(
                apply(&mut after, &[operation("Склад", false)]).is_err(),
                "{new}"
            );
            assert_eq!(after, before);
        }
    }
    #[test]
    fn totals_uses_namespace_uri_and_preserves_aliases() {
        let before = register().replace(
            "<UseInTotals>true</UseInTotals>",
            "<m:UseInTotals xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">true</m:UseInTotals>",
        );
        let mut after = before.clone();
        apply(&mut after, &[operation("Склад", false)]).unwrap();
        let (a, b) = before.rsplit_once("true</m:UseInTotals>").unwrap();
        assert_eq!(after, format!("{a}false</m:UseInTotals>{b}"));
    }
}
