use super::*;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

/// The descriptor with every MDClasses element under `md:` and no default
/// namespace; other namespaces keep their prefixes.
fn prefixed(source: &str) -> String {
    let body = source.trim_start_matches('\u{feff}');
    let offset = source.len() - body.len();
    let doc = roxmltree::Document::parse(body).unwrap();
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

/// Relations and member updates write the same change into a prefixed
/// descriptor as into its default-namespace form: existing elements are
/// found and replaced in place, inserted ones join MDClasses, and a repeat
/// is a no-op.
#[test]
fn relation_and_member_writers_keep_the_owner_prefix() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"A"}}},
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"B"}}},
            {"op":"object.create","args":{"values":{"kind":"Document","name":"D"}}}
        ]}),
    );
    for at in ["main:Catalog.A", "main:Document.D"] {
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"attribute.add","args":{"items":[{"name":"Text","type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]}}]}}]}),
        );
    }
    let files = [
        PathBuf::from("Catalogs/A.xml"),
        PathBuf::from("Documents/D.xml"),
    ];
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let edits = [
        json!({"at":"main:Catalog.A","ops":[
            {"op":"relation.add","args":{"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.B"}]}}},
            {"op":"relation.add","args":{"values":{"relation":"inputByString","targets":[{"fieldPath":"Catalog.A.Attribute.Text"}]}}}
        ]}),
        json!({"at":"main:Document.D","ops":[
            {"op":"relation.add","args":{"values":{"relation":"basedOn","targets":[{"metadataPath":"Catalog.A"}]}}}
        ]}),
        json!({"at":"main:Catalog.A.Attribute.Text","ops":[
            {"op":"attribute.set","args":{"values":{"synonym":"Подпись","comment":"Note","newName":"Label"}}}
        ]}),
        json!({"at":"main:Document.D.Attribute.Text","ops":[
            {"op":"attribute.set","args":{"values":{"comment":"Note"}}}
        ]}),
    ];
    let repeat = [
        json!({"at":"main:Catalog.A","ops":[
            {"op":"relation.replace","args":{"values":{"relation":"owners","targets":[{"metadataPath":"Catalog.B"}]}}}
        ]}),
        json!({"at":"main:Document.D","ops":[
            {"op":"relation.replace","args":{"values":{"relation":"basedOn","targets":[{"metadataPath":"Catalog.A"}]}}}
        ]}),
    ];
    let run = |prefix: bool| {
        for file in &files {
            let mut text = String::from_utf8(original[file].clone()).unwrap();
            // The member's Comment is absent, so attribute.set inserts it
            // before the closing Properties of the member.
            let members = text.find("<ChildObjects>").expect("members");
            let comment = members + text[members..].find("<Comment/>").expect("member comment");
            let line_start = text[..comment].rfind('\n').unwrap();
            text.replace_range(line_start..comment + "<Comment/>".len(), "");
            let text = if prefix { prefixed(&text) } else { text };
            std::fs::write(fixture.source.join(file), text).unwrap();
        }
        for edit in &edits {
            publish(&runtime, &fixture, edit.clone());
        }
        let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
        for edit in &repeat {
            publish(&runtime, &fixture, edit.clone());
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            after,
            "a repeated relation changed the descriptor (prefixed: {prefix})"
        );
        let result = files
            .iter()
            .map(|file| String::from_utf8(after[file].clone()).unwrap())
            .collect::<Vec<_>>();
        // Restore the whole tree, including renamed members, for the next run.
        for (path, bytes) in &original {
            std::fs::write(fixture.source.join(path), bytes).unwrap();
        }
        result
    };
    let plain = run(false);
    let with_prefix = run(true);
    for ((file, plain), prefixed) in files.iter().zip(&plain).zip(&with_prefix) {
        assert!(
            plain.contains("Catalog.B") || file.ends_with("D.xml"),
            "{file:?}: the plain run did not write the relation"
        );
        assert_eq!(
            element_names(prefixed),
            element_names(plain),
            "{file:?}: element namespaces differ"
        );
        assert_eq!(&unprefixed(prefixed), plain, "{file:?}");
    }
}
