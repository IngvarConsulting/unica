use super::*;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

/// The descriptor with every MDClasses element under `md:` and no default
/// namespace; other namespaces keep their prefixes.
fn prefixed(source: &str) -> String {
    let body = source.trim_start_matches('\u{feff}');
    let offset = source.len() - body.len();
    let Ok(doc) = roxmltree::Document::parse(body) else {
        return source.to_string();
    };
    if doc.root_element().lookup_namespace_uri(None) != Some(MD) {
        return source.to_string();
    }
    let mut inserts = Vec::new();
    for node in doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD))
    {
        let range = node.range();
        inserts.push(offset + range.start + 1);
        if let Some(close) = body[range.clone()].rfind("</") {
            inserts.push(offset + range.start + close + 2);
        }
    }
    inserts.sort_unstable_by(|a, b| b.cmp(a));
    let mut text = source.to_string();
    for at in inserts {
        text.insert_str(at, "md:");
    }
    text.replacen(&format!("xmlns=\"{MD}\""), &format!("xmlns:md=\"{MD}\""), 1)
}

fn unprefixed(source: &str) -> String {
    source.replace("<md:", "<").replace("</md:", "</").replacen(
        &format!("xmlns:md=\"{MD}\""),
        &format!("xmlns=\"{MD}\""),
        1,
    )
}

/// New elements get fresh UUIDs in each run; UUIDs of the original tree are
/// compared as they are.
fn masked_uuids(source: &str, known: &BTreeSet<String>) -> String {
    let bytes = source.as_bytes();
    let is_hex = |byte: &u8| byte.is_ascii_hexdigit();
    let mut text = String::with_capacity(source.len());
    let mut at = 0;
    while at < source.len() {
        let candidate = &bytes[at..(at + 36).min(bytes.len())];
        let uuid = candidate.len() == 36
            && candidate
                .iter()
                .enumerate()
                .all(|(index, byte)| match index {
                    8 | 13 | 18 | 23 => *byte == b'-',
                    _ => is_hex(byte),
                });
        if uuid && !known.contains(&source[at..at + 36]) {
            text.push_str("UUID");
            at += 36;
        } else {
            let ch = source[at..].chars().next().unwrap();
            text.push(ch);
            at += ch.len_utf8();
        }
    }
    text
}

fn element_names(source: &str) -> Vec<(Option<String>, String)> {
    let doc = roxmltree::Document::parse(source.trim_start_matches('\u{feff}')).unwrap();
    doc.descendants()
        .filter(|node| node.is_element())
        .map(|node| {
            (
                node.tag_name().namespace().map(str::to_string),
                node.tag_name().name().to_string(),
            )
        })
        .collect()
}

/// Operations whose writes are not checked here, with the reason.
const NOT_IN_MATRIX: &[(&str, &str)] = &[
    (
        "object.borrow",
        "borrows into an extension, which needs its own source set",
    ),
    (
        "help.create",
        "writes the help facet, not a descriptor element",
    ),
];

/// Each metadata write operation, on a prefixed descriptor tree, writes the
/// same result as on its default-namespace form: every element stays in its
/// namespace and the bytes agree once the prefix is removed. An operation
/// without a sample here fails the inventory check below.
#[test]
fn medium_every_metadata_write_keeps_the_owner_prefix() {
    let steps: Vec<(&str, Value)> = vec![
        (
            "object.create",
            json!({"at":"main:Configuration","ops":[
                {"op":"object.create","args":{"values":{"kind":"Catalog","name":"Extra"}}}
            ]}),
        ),
        (
            "attribute.add",
            json!({"at":"main:Catalog.A","ops":[{"op":"attribute.add","args":{"items":[
                {"name":"Text","type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]}}
            ]}}]}),
        ),
        (
            "attribute.set",
            json!({"at":"main:Catalog.A.Attribute.Text","ops":[
                {"op":"attribute.set","args":{"values":{"synonym":"Подпись","comment":"Note","newName":"Label"}}}
            ]}),
        ),
        (
            "tabularSection.add",
            json!({"at":"main:Catalog.A","ops":[{"op":"tabularSection.add","args":{"items":[{"name":"Goods"}]}}]}),
        ),
        (
            "tabularSection.set",
            json!({"at":"main:Catalog.A.TabularSection.Goods","ops":[
                {"op":"tabularSection.set","args":{"values":{"synonym":"Товары","comment":"Rows"}}}
            ]}),
        ),
        (
            "attribute.add",
            json!({"at":"main:Catalog.A.TabularSection.Goods","ops":[{"op":"attribute.add","args":{"items":[
                {"name":"Qty","type":{"variants":[{"kind":"string","length":5,"allowedLength":"variable"}]}}
            ]}}]}),
        ),
        (
            "attribute.copy",
            json!({"at":"main:Catalog.A.TabularSection.Goods.Attribute.Qty","ops":[
                {"op":"attribute.copy","args":{"values":{"name":"QtyCopy"}}}
            ]}),
        ),
        (
            "attribute.remove",
            json!({"at":"main:Catalog.A.TabularSection.Goods.Attribute.QtyCopy","ops":[
                {"op":"attribute.remove","args":{}}
            ]}),
        ),
        (
            "tabularSection.remove",
            json!({"at":"main:Catalog.A.TabularSection.Goods","ops":[{"op":"tabularSection.remove","args":{}}]}),
        ),
        (
            "relation.add",
            json!({"at":"main:Catalog.A","ops":[
                {"op":"relation.add","args":{"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.B"}]}}}
            ]}),
        ),
        (
            "relation.replace",
            json!({"at":"main:Catalog.A","ops":[
                {"op":"relation.replace","args":{"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.Extra"}]}}}
            ]}),
        ),
        (
            "relation.remove",
            json!({"at":"main:Catalog.A","ops":[
                {"op":"relation.remove","args":{"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.Extra"}]}}}
            ]}),
        ),
        (
            "command.add",
            json!({"at":"main:Catalog.A","ops":[{"op":"command.add","args":{"items":[{"name":"Run"}]}}]}),
        ),
        (
            "command.set",
            json!({"at":"main:Catalog.A.Command.Run","ops":[
                {"op":"command.set","args":{"values":{"comment":"Runs"}}}
            ]}),
        ),
        (
            "command.remove",
            json!({"at":"main:Catalog.A.Command.Run","ops":[{"op":"command.remove","args":{}}]}),
        ),
        (
            "template.add",
            json!({"at":"main:Catalog.A","ops":[{"op":"template.add","args":{"items":[
                {"name":"Print","templateType":"SpreadsheetDocument"}
            ]}}]}),
        ),
        (
            "template.set",
            json!({"at":"main:Catalog.A.Template.Print","ops":[
                {"op":"template.set","args":{"values":{"comment":"Printed"}}}
            ]}),
        ),
        (
            "template.remove",
            json!({"at":"main:Catalog.A.Template.Print","ops":[{"op":"template.remove","args":{}}]}),
        ),
        (
            "predefinedItem.add",
            json!({"at":"main:Catalog.A","ops":[{"op":"predefinedItem.add","args":{"items":[
                {"id":"a1b2c3d4-1111-4111-8111-111111112489","name":"First","description":"Первый"}
            ]}}]}),
        ),
        (
            "predefinedItem.set",
            json!({"at":"main:Catalog.A.PredefinedItem.First","ops":[
                {"op":"predefinedItem.set","args":{"values":{"description":"Первый элемент"}}}
            ]}),
        ),
        (
            "predefinedItem.remove",
            json!({"at":"main:Catalog.A.PredefinedItem.First","ops":[{"op":"predefinedItem.remove","args":{}}]}),
        ),
        (
            "enumValue.add",
            json!({"at":"main:Enum.E","ops":[{"op":"enumValue.add","args":{"items":[
                {"name":"Red","synonym":"Красный","comment":"Value"}
            ]}}]}),
        ),
        (
            "enumValue.set",
            json!({"at":"main:Enum.E.EnumValue.Red","ops":[
                {"op":"enumValue.set","args":{"values":{"comment":"Changed","newName":"Crimson"}}}
            ]}),
        ),
        (
            "enumValue.remove",
            json!({"at":"main:Enum.E.EnumValue.Crimson","ops":[{"op":"enumValue.remove","args":{}}]}),
        ),
        (
            "dimension.add",
            json!({"at":"main:InformationRegister.R","ops":[{"op":"dimension.add","args":{"items":[{"name":"Extra"}]}}]}),
        ),
        (
            "dimension.set",
            json!({"at":"main:InformationRegister.R.Dimension.Extra","ops":[
                {"op":"dimension.set","args":{"values":{"comment":"Axis","master":false}}}
            ]}),
        ),
        (
            "resource.add",
            json!({"at":"main:InformationRegister.R","ops":[{"op":"resource.add","args":{"items":[{"name":"Value"}]}}]}),
        ),
        (
            "resource.set",
            json!({"at":"main:InformationRegister.R.Resource.Value","ops":[
                {"op":"resource.set","args":{"values":{"comment":"Amount"}}}
            ]}),
        ),
        (
            "resource.remove",
            json!({"at":"main:InformationRegister.R.Resource.Value","ops":[{"op":"resource.remove","args":{}}]}),
        ),
        (
            "dimension.remove",
            json!({"at":"main:InformationRegister.R.Dimension.Extra","ops":[{"op":"dimension.remove","args":{}}]}),
        ),
        (
            "column.add",
            json!({"at":"main:DocumentJournal.J","ops":[{"op":"column.add","args":{"items":[{"name":"Number"}]}}]}),
        ),
        (
            "column.set",
            json!({"at":"main:DocumentJournal.J.Column.Number","ops":[
                {"op":"column.set","args":{"values":{"comment":"Shown","newName":"Num"}}}
            ]}),
        ),
        (
            "column.remove",
            json!({"at":"main:DocumentJournal.J.Column.Num","ops":[{"op":"column.remove","args":{}}]}),
        ),
        (
            "content.set",
            json!({"at":"main:ExchangePlan.X","ops":[{"op":"content.set","args":{"items":[
                {"at":"main:Catalog.B","autoRecord":"Allow"}
            ]}}]}),
        ),
        (
            "object.remove",
            json!({"at":"main:Catalog.Extra","ops":[{"op":"object.remove","args":{}}]}),
        ),
        // Writer paths reached through other operations: a register adds a
        // record to its registrar document and a form registration
        // its owner.
        (
            "object.create",
            json!({"at":"main:Configuration","ops":[
                {"op":"object.create","args":{"values":{"kind":"AccumulationRegister","name":"Turnover"}}},
                {"op":"resource.add","args":{"at":"main:AccumulationRegister.Turnover","items":[{"name":"Amount"}]}}
            ]}),
        ),
        (
            "form.add",
            json!({"at":"main:Catalog.B","ops":[{"op":"form.add","args":{"items":[{"name":"Choose","type":"ChoiceForm"}]}}]}),
        ),
    ];

    let sampled = steps
        .iter()
        .map(|(operation, _)| *operation)
        .chain(NOT_IN_MATRIX.iter().map(|(operation, _)| *operation))
        .collect::<BTreeSet<_>>();
    let registry = unica_metadata::apply_operations::MetadataOperation::ALL
        .iter()
        .map(|operation| operation.name())
        .collect::<BTreeSet<_>>();
    let missing = registry.difference(&sampled).collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "every metadata write operation needs a sample in this matrix or a reason in NOT_IN_MATRIX: {missing:?}"
    );

    let fixture = Fixture::blank();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"A"}}},
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"B"}}},
            {"op":"object.create","args":{"values":{"kind":"Document","name":"D"}}},
            {"op":"object.create","args":{"values":{"kind":"Enum","name":"E"}}},
            {"op":"object.create","args":{"values":{"kind":"ExchangePlan","name":"X"}}},
            {"op":"object.create","args":{"values":{"kind":"InformationRegister","name":"R"}}},
            {"op":"dimension.add","args":{"at":"main:InformationRegister.R","items":[{"name":"Key"}]}}
        ]}),
    );
    // A journal requires an existing registered document.
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{"kind":"DocumentJournal","name":"J"}}}
        ]}),
    );
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let known = original
        .values()
        .filter_map(|bytes| std::str::from_utf8(bytes).ok())
        .flat_map(|text| {
            text.char_indices()
                .filter_map(|(at, _)| text.get(at..at + 36))
                .filter(|candidate| {
                    uuid::Uuid::parse_str(candidate).is_ok() && candidate.len() == 36
                })
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    let run = |prefix: bool| {
        for (path, bytes) in &original {
            let text = String::from_utf8(bytes.clone()).unwrap_or_default();
            let written = if prefix && path.extension().is_some_and(|ext| ext == "xml") {
                prefixed(&text).into_bytes()
            } else {
                bytes.clone()
            };
            std::fs::write(fixture.source.join(path), written).unwrap();
        }
        let mut failures = Vec::new();
        for (operation, edit) in &steps {
            let planned = call(&runtime, &fixture, ToolIdentity::Apply, edit.clone());
            let applied = planned.ok.then(|| {
                let token = planned.data.as_ref().unwrap()["executionToken"].clone();
                call(
                    &runtime,
                    &fixture,
                    ToolIdentity::Apply,
                    json!({"executionToken": token}),
                )
            });
            if !applied.as_ref().is_some_and(|result| result.ok) {
                failures.push(format!(
                    "{operation} (prefixed: {prefix}): {}",
                    applied
                        .as_ref()
                        .map_or(&planned.summary, |result| &result.summary)
                ));
            }
        }
        let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
        // Restore the whole tree for the next run.
        let created = after
            .keys()
            .filter(|path| !original.contains_key(*path))
            .cloned()
            .collect::<Vec<_>>();
        for path in created {
            let _ = std::fs::remove_file(fixture.source.join(path));
        }
        for (path, bytes) in &original {
            std::fs::write(fixture.source.join(path), bytes).unwrap();
        }
        (after, failures)
    };
    let (plain, plain_failures) = run(false);
    assert!(
        plain_failures.is_empty(),
        "the default-namespace run must apply every sample:\n{}",
        plain_failures.join("\n")
    );
    let (with_prefix, prefix_failures) = run(true);
    let mut problems = prefix_failures;
    for (path, bytes) in &plain {
        let Some(prefixed_bytes) = with_prefix.get(path) else {
            problems.push(format!("{path:?}: missing after the prefixed run"));
            continue;
        };
        if path.extension().is_none_or(|ext| ext != "xml") {
            if prefixed_bytes != bytes {
                problems.push(format!("{path:?}: non-XML bytes differ"));
            }
            continue;
        }
        let plain_text = String::from_utf8(bytes.clone()).unwrap();
        let prefixed_text = String::from_utf8(prefixed_bytes.clone()).unwrap();
        if roxmltree::Document::parse(plain_text.trim_start_matches('\u{feff}')).is_err() {
            continue;
        }
        let was_prefixed = original
            .get(path)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .is_some_and(|text| prefixed(text) != text);
        if was_prefixed && prefixed(&unprefixed(&prefixed_text)) != prefixed_text {
            problems.push(format!("{path:?}: the prefixed descriptor lost its prefix"));
        } else if element_names(&prefixed_text) != element_names(&plain_text) {
            problems.push(format!("{path:?}: element namespaces differ"));
        } else if masked_uuids(&unprefixed(&prefixed_text), &known)
            != masked_uuids(&plain_text, &known)
        {
            problems.push(format!("{path:?}: bytes differ once the prefix is removed"));
        }
    }
    for path in with_prefix.keys().filter(|path| !plain.contains_key(*path)) {
        problems.push(format!("{path:?}: only written by the prefixed run"));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn root_comment_edits_preserve_a_foreign_default_namespace_in_either_declaration_order() {
    for foreign_first in [false, true] {
        let fixture = Fixture::blank();
        let runtime = runtime();
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[
                {"op":"object.create","args":{"values":{"kind":"Catalog","name":"A"}}}
            ]}),
        );
        let path = fixture.source.join("Catalogs/A.xml");
        let original = std::fs::read_to_string(&path).unwrap();
        let declaration = format!("xmlns:md=\"{MD}\"");
        let with_default = if foreign_first {
            format!("xmlns=\"urn:unrelated\" {declaration}")
        } else {
            format!("{declaration} xmlns=\"urn:unrelated\"")
        };
        let before = prefixed(&original).replacen(&declaration, &with_default, 1);
        assert!(before.contains("<md:Comment/>"));
        std::fs::write(&path, &before).unwrap();
        let request = json!({"at":"main:Catalog.A","ops":[
            {"op":"props.set","args":{"values":{"Comment":"Note"}}}
        ]});
        publish(&runtime, &fixture, request.clone());
        let expected = before.replace("<md:Comment/>", "<md:Comment>Note</md:Comment>");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        publish(&runtime, &fixture, request);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }
}
