//! Original pure Form codec oracles; assertions retained once.
use super::*;
use serde_json::{json, Map};

#[test]
fn form_item_binding_facts_preserve_both_targets_and_legacy_normalization() {
    let data = parse_form_event_evidence_xml(
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems>
<Button name="Both" id="1"><DataPath> Объект.<!-- split -->Ref </DataPath><CommandName> Form.Command.Заполнить </CommandName></Button>
<Button name="Command" id="2"><CommandName><![CDATA[ Form.StandardCommand.Close ]]></CommandName></Button>
</ChildItems></Form>"#,
    ).unwrap();
    let both = &data.elements[0];
    assert_eq!(both.data_path.as_deref(), Some(" Объект.Ref "));
    assert_eq!(
        both.command_name.as_deref(),
        Some(" Form.Command.Заполнить ")
    );
    assert!(both.binding_limits.is_empty());
    let legacy = both.binding.as_ref().unwrap();
    assert_eq!(legacy.kind, "dataPath");
    assert_eq!(legacy.target, "Объект.Ref");
    let command = &data.elements[1];
    assert_eq!(
        command.command_name.as_deref(),
        Some(" Form.StandardCommand.Close ")
    );
    let legacy = command.binding.as_ref().unwrap();
    assert_eq!(legacy.kind, "standardCommand");
    assert_eq!(legacy.target, "Close");
}

#[test]
fn form_item_binding_facts_keep_damage_local_to_each_property() {
    for property in ["DataPath", "CommandName"] {
        for raw in [
            format!("<{property}><Nested>bad</Nested></{property}>"),
            format!("<{property}>One</{property}><{property}>Two</{property}>"),
            format!("<{property} xmlns=\"urn:foreign\">Foreign</{property}>"),
            format!("<{property} unknown=\"value\">Bad</{property}>"),
        ] {
            let other = if property == "DataPath" {
                "<CommandName>CommonCommand.Probe</CommandName>"
            } else {
                "<DataPath>Объект.Code</DataPath>"
            };
            let data = parse_form_event_evidence_xml(&format!(
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\"><ChildItems><Button name=\"Probe\" id=\"1\">{raw}{other}</Button></ChildItems></Form>"
            )).unwrap();
            let element = &data.elements[0];
            assert_eq!(element.binding_limits.len(), 1);
            assert!(element.binding_limits[0].contains(property));
            if property == "DataPath" {
                assert!(element.data_path.is_none());
                assert_eq!(element.command_name.as_deref(), Some("CommonCommand.Probe"));
            } else {
                assert!(element.command_name.is_none());
                assert_eq!(element.data_path.as_deref(), Some("Объект.Code"));
            }
        }
    }
}
#[test]
fn generated_form_module_uses_the_8_3_27_crlf_serialization() {
    assert_platform_text_uses_crlf_without_bare_lf(form_add_module_bsl());
}

#[test]
fn form_info_projects_direct_command_action_as_execute_binding() {
    let data = parse_form_info_xml(
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems/>
  <Commands>
    <Command name="Refresh" id="1">
      <Action>RefreshAction</Action>
    </Command>
  </Commands>
</Form>"#,
        "Test".to_string(),
        String::new(),
        DomainObjectSupportData {
            state: unica_metadata::support_state::ObjectSupportState::NotSupported,
            direct_edit_safe: Some(true),
        },
    )
    .unwrap();

    assert_eq!(data.commands.len(), 1);
    assert_eq!(data.commands[0].actions.len(), 1);
    let action = &data.commands[0].actions[0];
    assert_eq!(action.name, "Execute");
    assert_eq!(action.handler, "RefreshAction");
    assert_eq!(action.call_type, None);
}

/// #1385: a form is registered in the platform slot of forms, which
/// depends on the owner kind: a catalog writes its forms after its tabular
/// sections, a document before them; templates and commands follow.
#[test]
fn form_registration_takes_the_platform_slot_of_its_owner_kind() {
    fn owner(kind: &str, children: &[&str]) -> String {
        let mut body = String::new();
        for child in children {
            let (tag, name) = child.split_once(':').unwrap();
            if matches!(tag, "Form" | "Template") {
                body.push_str(&format!("\t\t\t<{tag}>{name}</{tag}>\n"));
            } else {
                body.push_str(&format!(
                        "\t\t\t<{tag} uuid=\"00000000-0000-4000-8000-000000000301\">\n\t\t\t\t<Properties><Name>{name}</Name></Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t</ChildObjects>\n\t\t\t</{tag}>\n"
                    ));
            }
        }
        format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<{kind} uuid=\"00000000-0000-4000-8000-000000000300\">\n\t\t<Properties><Name>Owner</Name></Properties>\n\t\t<ChildObjects>\n{body}\t\t</ChildObjects>\n\t</{kind}>\n</MetaDataObject>\n"
            )
    }
    fn children(xml: &str) -> Vec<String> {
        let document = roxmltree::Document::parse(xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();
        object
            .children()
            .find(|node| node.has_tag_name("ChildObjects"))
            .unwrap()
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                let name = node
                    .descendants()
                    .find(|inner| inner.has_tag_name("Name"))
                    .and_then(|inner| inner.text())
                    .or_else(|| node.text())
                    .unwrap_or_default();
                format!("{}:{name}", node.tag_name().name())
            })
            .collect()
    }
    for (kind, existing, expected) in [
        (
            "Catalog",
            &["Attribute:A", "TabularSection:T"][..],
            &["Attribute:A", "TabularSection:T", "Form:New"][..],
        ),
        (
            "Catalog",
            &["Attribute:A", "TabularSection:T", "Form:F", "Command:C"][..],
            &[
                "Attribute:A",
                "TabularSection:T",
                "Form:F",
                "Form:New",
                "Command:C",
            ][..],
        ),
        (
            "Document",
            &["Attribute:A", "TabularSection:T", "Template:M"][..],
            &["Attribute:A", "Form:New", "TabularSection:T", "Template:M"][..],
        ),
        (
            "InformationRegister",
            &["Dimension:D", "Command:C"][..],
            &["Dimension:D", "Form:New", "Command:C"][..],
        ),
        // no settled platform order: forms stay before the first template
        (
            "SettingsStorage",
            &["Form:F", "Template:M"][..],
            &["Form:F", "Form:New", "Template:M"][..],
        ),
    ] {
        let updated = register_form_in_object_text(&owner(kind, existing), "New");
        assert_eq!(children(&updated), expected, "{kind} {existing:?}");
    }
}

#[test]
fn form_edit_rejects_malformed_or_unsupported_attribute_columns() {
    let cases = [
        (
            json!([{"name": "Data", "type": "ValueTable", "columns": {"name": "A", "type": "string"}}]),
            "columns must be an array",
        ),
        (
            json!([{"name": "Data", "type": "ValueTable", "columns": [null]}]),
            "column #1 must be an object",
        ),
        (
            json!([{"name": "Data", "type": "ValueTable", "columns": [{"type": "string"}]}]),
            "column #1 requires non-empty name",
        ),
        (
            json!([{"name": "Data", "type": "ValueTable", "columns": [{"name": "A"}]}]),
            "column 'A' requires non-empty type",
        ),
        (
            json!([{"name": "Data", "type": "string", "columns": [{"name": "A", "type": "string"}]}]),
            "columns are supported only for ValueTable or ValueTree",
        ),
    ];

    for (attrs, expected) in cases {
        let error = form_edit_validate_attribute_columns(attrs.as_array().unwrap()).unwrap_err();
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error:?}"
        );
    }
}

#[test]
fn placement_after_column_group_preserves_the_whole_subtree() {
    let group = "<ColumnGroup name=\"Group\" id=\"2\"><ChildItems><InputField name=\"Nested\" id=\"3\"/></ChildItems></ColumnGroup>";
    let mut xml = format!(
        "<Form xmlns=\"{FORM_LOGFORM_NS}\"><ChildItems><Table name=\"Rows\" id=\"1\"><ChildItems>{group}<InputField name=\"Last\" id=\"4\"/></ChildItems></Table></ChildItems></Form>"
    );
    let target = form_edit_target_child_items_range(&xml, None, Some("Group")).unwrap();
    form_edit_insert_lines_into_target(
        &mut xml,
        target,
        &["<InputField name=\"Added\" id=\"5\"/>".to_string()],
    )
    .unwrap();
    assert!(
        xml.contains(group),
        "the existing group must remain byte-for-byte intact"
    );
    let doc = Document::parse(&xml).unwrap();
    let rows = doc
        .descendants()
        .find(|n| n.attribute("name") == Some("Rows"))
        .unwrap();
    let names = form_child(rows, "ChildItems")
        .unwrap()
        .children()
        .filter_map(|n| n.attribute("name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["Group", "Added", "Last"]);
    assert!(form_edit_target_child_items_range(&xml, None, Some("Missing")).is_err());
}

#[test]
fn column_group_placement_keeps_namespace_base_form_and_event_boundaries() {
    let own = "<ColumnGroup name=\"Group\" id=\"1\"/>";
    let foreign = "<ColumnGroup xmlns=\"urn:foreign\" name=\"Group\"/>";
    let base = "<BaseForm><ChildItems><ColumnGroup name=\"Group\"/></ChildItems></BaseForm>";
    let xml = format!(
        "<Form xmlns=\"{FORM_LOGFORM_NS}\"><ChildItems>{own}{foreign}</ChildItems>{base}</Form>"
    );
    let doc = Document::parse(&xml).unwrap();
    let target = form_edit_find_placement_element(doc.root_element(), "Group")
        .unwrap()
        .unwrap();
    assert_eq!(target.attribute("id"), Some("1"));
    assert!(form_edit_find_element_nodes(doc.root_element(), "Group").is_empty());

    let xml = xml.replace(foreign, &format!("{foreign}{own}"));
    let doc = Document::parse(&xml).unwrap();
    assert!(form_edit_find_placement_element(doc.root_element(), "Group").is_err());
}

#[test]
fn auto_command_bar_placement_covers_root_and_table_panels() {
    for self_closing in [true, false] {
        for populated in [true, false] {
            for target in ["RootBar", "RowsBar"] {
                let content = if populated {
                    "<ChildItems><Button name=\"Existing\" id=\"3\"/></ChildItems>"
                } else {
                    ""
                };
                let panel = |name: &str, id: &str| {
                    if self_closing && !populated {
                        format!("<AutoCommandBar name=\"{name}\" id=\"{id}\"/>")
                    } else {
                        format!("<AutoCommandBar name=\"{name}\" id=\"{id}\">{content}</AutoCommandBar>")
                    }
                };
                let mut xml = format!("<Form xmlns=\"{FORM_LOGFORM_NS}\">{}<ChildItems><Table name=\"Rows\" id=\"1\">{}</Table></ChildItems><BaseForm><AutoCommandBar name=\"BaselineBar\" id=\"-1\"/></BaseForm></Form>",panel("RootBar","-1"),panel("RowsBar","2"));
                let insert = form_edit_target_child_items_range(&xml, Some(target), None).unwrap();
                form_edit_insert_lines_into_target(
                    &mut xml,
                    insert,
                    &["<Button name=\"Added\" id=\"4\"/>".to_string()],
                )
                .unwrap();
                let doc = Document::parse(&xml).unwrap();
                let added = doc
                    .descendants()
                    .find(|n| n.attribute("name") == Some("Added"))
                    .unwrap();
                let items = added.parent_element().unwrap();
                assert_eq!(items.tag_name().name(), "ChildItems");
                assert_eq!(
                    items.parent_element().unwrap().attribute("name"),
                    Some(target)
                );
                if populated {
                    assert_eq!(items.children().filter(|n| n.is_element()).count(), 2);
                }
                assert!(
                    form_edit_target_child_items_range(&xml, Some("BaselineBar"), None).is_err()
                );
                assert!(form_edit_target_child_items_range(&xml, Some("Missing"), None).is_err());
            }
        }
    }
}

#[test]
fn auto_command_bar_targets_reject_ambiguous_names_and_support_empty_forms() {
    let xml=format!("<Form xmlns=\"{FORM_LOGFORM_NS}\"><AutoCommandBar name=\"Duplicate\" id=\"-1\"/><ChildItems><Table name=\"Rows\" id=\"1\"><AutoCommandBar name=\"Duplicate\" id=\"2\"/></Table></ChildItems></Form>");
    assert!(
        form_edit_target_child_items_range(&xml, Some("Duplicate"), None)
            .err()
            .expect("ambiguous placement must fail")
            .contains("ambiguous")
    );
    let mut xml = format!(
        "<Form xmlns=\"{FORM_LOGFORM_NS}\"><AutoCommandBar name=\"EmptyBar\" id=\"-1\"/></Form>"
    );
    let target = form_edit_target_child_items_range(&xml, Some("EmptyBar"), None).unwrap();
    form_edit_insert_lines_into_target(
        &mut xml,
        target,
        &["<Button name=\"Only\" id=\"1\"/>".to_string()],
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let only = doc
        .descendants()
        .find(|n| n.attribute("name") == Some("Only"))
        .unwrap();
    assert!(form_edit_elements_match_target(
        doc.root_element(),
        &[only],
        &["Only".to_string()],
        Some("EmptyBar"),
        None
    ));
    assert!(form_child(doc.root_element(), "ChildItems").is_none());
}

#[test]
fn auto_command_bar_after_requires_direct_child_items_parent() {
    let mut xml=format!("<Form xmlns=\"{FORM_LOGFORM_NS}\"><AutoCommandBar name=\"RootBar\" id=\"-1\"><ChildItems><Button name=\"First\" id=\"4\"/></ChildItems></AutoCommandBar><ChildItems><Table name=\"Rows\" id=\"1\"><AutoCommandBar name=\"RowsBar\" id=\"2\"/></Table></ChildItems></Form>");
    assert!(form_edit_target_child_items_range(&xml, None, Some("RootBar")).is_err());
    assert!(form_edit_target_child_items_range(&xml, None, Some("RowsBar")).is_err());
    assert!(form_edit_placement_is_command_bar(
        Document::parse(&xml).unwrap().root_element(),
        None,
        Some("First")
    )
    .unwrap());
    let target = form_edit_target_child_items_range(&xml, None, Some("First")).unwrap();
    form_edit_insert_lines_into_target(
        &mut xml,
        target,
        &["<Button name=\"Second\" id=\"5\"/>".to_string()],
    )
    .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let items = doc
        .descendants()
        .find(|n| n.attribute("name") == Some("First"))
        .unwrap()
        .parent_element()
        .unwrap();
    let names = items
        .children()
        .filter(|n| n.is_element())
        .filter_map(|n| n.attribute("name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["First", "Second"]);
}

#[test]
fn form_edit_namespace_repair_accepts_whitespace_around_equals() {
    let mut xml = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8 = "http://v8.1c.ru/8.1/data/core"><ChildItems/></Form>"#.to_string();
    let root_start = Document::parse(&xml).unwrap().root_element().range().start;

    form_edit_ensure_emitted_namespaces(&mut xml, root_start, "<v8:item/>").unwrap();

    assert_eq!(xml.matches("xmlns:v8").count(), 1, "{xml}");
    Document::parse(&xml).unwrap();
}

#[test]
fn form_edit_namespace_repair_uses_parsed_root_after_comment() {
    let mut xml = r#"<!-- misleading <Form marker --><Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><ChildItems/></Form>"#.to_string();
    let root_start = Document::parse(&xml).unwrap().root_element().range().start;

    form_edit_ensure_emitted_namespaces(&mut xml, root_start, "<v8:item/>").unwrap();

    assert!(xml.starts_with("<!-- misleading <Form marker -->"), "{xml}");
    assert!(xml[root_start..].starts_with("<Form "), "{xml}");
    Document::parse(&xml).unwrap();
}

#[test]
fn form_edit_namespace_repair_is_noop_without_emitted_prefixes() {
    let mut xml =
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><ChildItems/></Form>"#.to_string();
    let original = xml.clone();
    let root_start = Document::parse(&xml).unwrap().root_element().range().start;

    form_edit_ensure_emitted_namespaces(&mut xml, root_start, "<Table/>").unwrap();

    assert_eq!(xml, original);
}

#[test]
fn form_element_dispatcher_ignores_nested_table_and_page_properties() {
    let definition = json!({
        "elements": [
            {
                "table": "Rows",
                "commandBar": {"autofill": false},
                "columns": []
            },
            {
                "pages": "Tabs",
                "children": [
                    {"page": "Main", "group": "horizontal"}
                ]
            }
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(xml.contains("<Table name=\"Rows\""), "{xml}");
    assert!(xml.contains("<Page name=\"Main\""), "{xml}");
    assert!(xml.contains("<Group>Horizontal</Group>"), "{xml}");
}

#[test]
fn form_compile_writes_command_representation_from_the_command_set() {
    let definition = json!({
        "commands": [{
            "name": "Find",
            "action": "Find",
            "representation": "textPicture"
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let command = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "Command")))
        .unwrap();
    let representation = command
        .children()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "Representation")))
        .and_then(|node| node.text());
    assert_eq!(representation, Some("TextPicture"), "{xml}");
}

/// Command properties come out in the FormCommand XDTO order that
/// platform dumps use: Title, Shortcut, Picture, Action, Representation.
#[test]
fn form_compile_writes_command_properties_in_platform_order() {
    let definition = json!({
        "commands": [{
            "name": "Find",
            "title": "Find",
            "representation": "Picture",
            "action": "Find",
            "picture": "StdPicture.Find",
            "shortcut": "Ctrl+F"
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let command = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "Command")))
        .unwrap();
    let order = command
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        ["Title", "Shortcut", "Picture", "Action", "Representation"],
        "{xml}"
    );
}

/// Each command key `command.add` publishes is written exactly as an
/// 8.3.27 dump writes it (#1439). Every expected block repeats, with
/// neutral names and texts, the shape of a `<Command>` from a platform
/// dump that carries only the keys under test: tabs, element order,
/// multilingual ToolTip, Shortcut text, Picture with `xr:Ref` and
/// `xr:LoadTransparent`, Representation from the command set.
#[test]
fn form_command_keys_are_written_like_the_platform_dump() {
    let cases = [
        (
            json!({"action": "ЗаполнитьСведения",
                       "tooltip": "Заполнить сведения из документа"}),
            "ЗаполнитьСведения",
            36,
            concat!(
                "\t\t<Command name=\"ЗаполнитьСведения\" id=\"36\">\n",
                "\t\t\t<ToolTip>\n",
                "\t\t\t\t<v8:item>\n",
                "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
                "\t\t\t\t\t<v8:content>Заполнить сведения из документа</v8:content>\n",
                "\t\t\t\t</v8:item>\n",
                "\t\t\t</ToolTip>\n",
                "\t\t\t<Action>ЗаполнитьСведения</Action>\n",
                "\t\t</Command>",
            ),
        ),
        (
            json!({"title": "Добавить строку", "shortcut": "Cmd+Z",
                       "action": "ДобавитьСтроку"}),
            "ДобавитьСтроку",
            5,
            concat!(
                "\t\t<Command name=\"ДобавитьСтроку\" id=\"5\">\n",
                "\t\t\t<Title>\n",
                "\t\t\t\t<v8:item>\n",
                "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
                "\t\t\t\t\t<v8:content>Добавить строку</v8:content>\n",
                "\t\t\t\t</v8:item>\n",
                "\t\t\t</Title>\n",
                "\t\t\t<Shortcut>Cmd+Z</Shortcut>\n",
                "\t\t\t<Action>ДобавитьСтроку</Action>\n",
                "\t\t</Command>",
            ),
        ),
        (
            json!({"title": "Отметить все", "picture": "StdPicture.CheckAll",
                       "action": "ОтметитьВсе"}),
            "ОтметитьВсе",
            10,
            concat!(
                "\t\t<Command name=\"ОтметитьВсе\" id=\"10\">\n",
                "\t\t\t<Title>\n",
                "\t\t\t\t<v8:item>\n",
                "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
                "\t\t\t\t\t<v8:content>Отметить все</v8:content>\n",
                "\t\t\t\t</v8:item>\n",
                "\t\t\t</Title>\n",
                "\t\t\t<Picture>\n",
                "\t\t\t\t<xr:Ref>StdPicture.CheckAll</xr:Ref>\n",
                "\t\t\t\t<xr:LoadTransparent>true</xr:LoadTransparent>\n",
                "\t\t\t</Picture>\n",
                "\t\t\t<Action>ОтметитьВсе</Action>\n",
                "\t\t</Command>",
            ),
        ),
        (
            json!({"title": "Сканировать", "action": "Сканировать",
                       "representation": "Picture"}),
            "Сканировать",
            1,
            concat!(
                "\t\t<Command name=\"Сканировать\" id=\"1\">\n",
                "\t\t\t<Title>\n",
                "\t\t\t\t<v8:item>\n",
                "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
                "\t\t\t\t\t<v8:content>Сканировать</v8:content>\n",
                "\t\t\t\t</v8:item>\n",
                "\t\t\t</Title>\n",
                "\t\t\t<Action>Сканировать</Action>\n",
                "\t\t\t<Representation>Picture</Representation>\n",
                "\t\t</Command>",
            ),
        ),
        (
            // Keys in a different order than the platform writes them.
            json!({"representation": "Picture", "action": "СписокОбновить",
                       "picture": "StdPicture.Refresh", "shortcut": "F5",
                       "tooltip": "Обновить список", "title": "Обновить"}),
            "СписокОбновить",
            5,
            DUMP_SHAPED_REFRESH_COMMAND,
        ),
    ];
    for (definition, name, id, expected) in cases {
        let mut lines = Vec::new();
        emit_form_command(
            &mut lines,
            definition.as_object().unwrap(),
            name,
            id,
            "\t\t",
            Some("ru"),
        )
        .unwrap();
        assert_eq!(lines.join("\n"), expected, "{name}");
    }
}

/// The form compiler and `command.add` share one command writer:
/// a compiled form carries the same `<Command>` block, ToolTip included.
#[test]
fn form_compile_writes_the_same_command_block_as_form_command_add() {
    let commands = json!([{
        "name": "СписокОбновить",
        "title": "Обновить",
        "tooltip": "Обновить список",
        "shortcut": "F5",
        "picture": "StdPicture.Refresh",
        "action": "СписокОбновить",
        "representation": "Picture"
    }]);
    let mut ids = FormIdAllocator { next: 4 };
    let mut lines = Vec::new();
    emit_form_commands(&mut lines, Some(&commands), "\t", &mut ids, Some("ru")).unwrap();
    assert_eq!(
        lines.join("\n"),
        format!("\t<Commands>\n{DUMP_SHAPED_REFRESH_COMMAND}\n\t</Commands>")
    );
}

/// The button value set is not the command one: 8.3.27 does not load
/// `PictureAndText` on a form command (#1436), so compile writes nothing.
#[test]
fn form_compile_refuses_button_representation_on_command() {
    for invalid in ["PictureAndText", "None", "Compact"] {
        let definition = json!({
            "commands": [{
                "name": "Find",
                "action": "Find",
                "representation": invalid
            }]
        });

        let error = form_compile_xml(&definition, "2.20").unwrap_err();
        assert!(
            error.contains("Command 'Find'")
                && error.contains(invalid)
                && error.contains("TextPicture"),
            "{invalid}: {error}"
        );
    }
}

#[test]
fn form_compile_emits_supported_table_properties() {
    let definition = json!({
        "attributes": [{
            "name": "Rows",
            "type": "ValueTable"
        }],
        "elements": [{
            "table": "Rows",
            "path": "Rows",
            "representation": "List",
            "autoInsertNewRow": true,
            "columns": []
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let table = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "Table")))
        .unwrap();
    let table_property_order = table
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .filter(|name| ["Representation", "AutoInsertNewRow", "DataPath"].contains(name))
        .collect::<Vec<_>>();

    assert!(
        xml.contains("<Representation>List</Representation>"),
        "{xml}"
    );
    assert!(
        xml.contains("<AutoInsertNewRow>true</AutoInsertNewRow>"),
        "{xml}"
    );
    assert_eq!(
        table_property_order,
        ["Representation", "AutoInsertNewRow", "DataPath"],
        "{xml}"
    );
}

#[test]
fn form_compile_accepts_every_table_representation_case_insensitively() {
    for (input, expected) in [
        ("List", "List"),
        ("tree", "Tree"),
        ("HIERARCHICALLIST", "HierarchicalList"),
    ] {
        let definition = json!({
            "elements": [{
                "table": "Rows",
                "representation": input
            }]
        });

        let (xml, _) = form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{input}: {error}"));

        assert!(
            xml.contains(&format!("<Representation>{expected}</Representation>")),
            "{input}: {xml}"
        );
    }
}

#[test]
fn form_compile_omits_false_auto_insert_new_row_default() {
    let definition = json!({
        "elements": [{
            "table": "Rows",
            "autoInsertNewRow": false
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(!xml.contains("<AutoInsertNewRow>"), "{xml}");
}

#[test]
fn form_compile_emits_tooltip_and_button_appearance() {
    let definition = json!({
        "attributes": [{"name": "Status", "type": "String"}],
        "elements": [
            {
                "input": "Comment",
                "title": "Comment",
                "tooltip": "Enter <comment> & confirm",
                "tooltipRepresentation": "Button",
                "disabled": true
            },
            {
                "button": "Apply",
                "title": "Apply",
                "tooltip": "Apply <changes> & continue",
                "tooltipRepresentation": "Button",
                "backColor": "#FFE0A0",
                "font": {
                    "ref": "style:Button&Main",
                    "bold": true,
                    "italic": false,
                    "kind": "StyleItem"
                }
            },
            {
                "button": "Secondary",
                "font": "style:Secondary&<Main>"
            },
            {
                "check": "Ready",
                "title": "Ready",
                "tooltip": "Check <details> & continue",
                "tooltipRepresentation": "Balloon"
            },
            {
                "labelField": "Status",
                "path": "Status",
                "tooltip": "Status <value> & details",
                "tooltipRepresentation": "Button"
            }
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(
        xml.contains(concat!(
            "<Title>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>Comment</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</Title>\n",
            "\t\t\t<ToolTip>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>Enter &lt;comment&gt; &amp; confirm</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</ToolTip>\n",
            "\t\t\t<ToolTipRepresentation>Button</ToolTipRepresentation>\n",
            "\t\t\t<Enabled>false</Enabled>"
        )),
        "{xml}"
    );
    assert!(
            xml.contains(concat!(
                "<Title>\n",
                "\t\t\t\t<v8:item>\n",
                "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
                "\t\t\t\t\t<v8:content>Apply</v8:content>\n",
                "\t\t\t\t</v8:item>\n",
                "\t\t\t</Title>\n",
                "\t\t\t<ToolTip>\n",
                "\t\t\t\t<v8:item>\n",
                "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
                "\t\t\t\t\t<v8:content>Apply &lt;changes&gt; &amp; continue</v8:content>\n",
                "\t\t\t\t</v8:item>\n",
                "\t\t\t</ToolTip>\n",
                "\t\t\t<ToolTipRepresentation>Button</ToolTipRepresentation>\n",
                "\t\t\t<BackColor>#FFE0A0</BackColor>\n",
                "\t\t\t<Font ref=\"style:Button&amp;Main\" bold=\"true\" italic=\"false\" kind=\"StyleItem\"/>\n",
                "\t\t\t<ExtendedTooltip"
            )),
            "{xml}"
        );
    assert!(
        xml.contains("<Font ref=\"style:Secondary&amp;&lt;Main&gt;\" kind=\"StyleItem\"/>"),
        "{xml}"
    );
    assert!(
        xml.contains(concat!(
            "<Title>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>Ready</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</Title>\n",
            "\t\t\t<ToolTip>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>Check &lt;details&gt; &amp; continue</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</ToolTip>\n",
            "\t\t\t<ToolTipRepresentation>Balloon</ToolTipRepresentation>\n",
            "\t\t\t<TitleLocation>Right</TitleLocation>\n",
            "\t\t\t<CheckBoxType>Auto</CheckBoxType>"
        )),
        "{xml}"
    );
    assert!(
        xml.contains(concat!(
            "<DataPath>Status</DataPath>\n",
            "\t\t\t<ToolTip>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>Status &lt;value&gt; &amp; details</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</ToolTip>\n",
            "\t\t\t<ToolTipRepresentation>Button</ToolTipRepresentation>\n",
            "\t\t\t<ContextMenu"
        )),
        "{xml}"
    );
}

#[test]
fn form_compile_emits_group_show_left_margin() {
    let definition = json!({
        "elements": [
            {
                "group": "vertical",
                "name": "NoLeftMargin",
                "disabled": true,
                "showLeftMargin": false,
                "children": []
            },
            {
                "group": "vertical",
                "name": "WithLeftMargin",
                "showLeftMargin": true,
                "children": []
            }
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(
        xml.contains(concat!(
            "<Group>Vertical</Group>\n",
            "\t\t\t<Enabled>false</Enabled>\n",
            "\t\t\t<ShowLeftMargin>false</ShowLeftMargin>\n",
            "\t\t\t<ExtendedTooltip name=\"NoLeftMarginРасширеннаяПодсказка\""
        )),
        "{xml}"
    );
    let with_left_margin = &xml[xml
        .find("<UsualGroup name=\"WithLeftMargin\"")
        .expect("group should be emitted")..];
    assert!(
        with_left_margin.contains(concat!(
            "<Group>Vertical</Group>\n",
            "\t\t\t<ShowLeftMargin>true</ShowLeftMargin>\n",
            "\t\t\t<ExtendedTooltip name=\"WithLeftMarginРасширеннаяПодсказка\""
        )),
        "{with_left_margin}"
    );
}

#[test]
fn form_compile_emits_documented_input_column_properties() {
    let definition = json!({
        "attributes": [
            {"name": "ПутьКФайлу", "type": "String"},
            {"name": "Объект", "type": "CatalogObject.CorpusCatalog", "main": true}
        ],
        "elements": [
            {
                "input": "ПутьКФайлу",
                "path": "ПутьКФайлу",
                "choiceButton": true,
                "showInHeader": true
            },
            {
                "input": "Сумма",
                "path": "Объект.Товары.Сумма",
                "horizontalAlign": "Right",
                "headerHorizontalAlign": "Right"
            },
            {
                "input": "Заполнитель",
                "title": "",
                "multiLine": true,
                "passwordMode": true,
                "choiceButton": false,
                "clearButton": true,
                "spinButton": true,
                "dropListButton": true,
                "markIncomplete": true,
                "skipOnInput": true,
                "showInHeader": false,
                "headerHorizontalAlign": "Right",
                "autoMaxWidth": false,
                "width": 40,
                "height": 3,
                "horizontalStretch": true,
                "verticalStretch": true,
                "horizontalAlign": "Right<&",
                "inputHint": "Меньше < и & больше"
            }
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(
        xml.contains(concat!(
            "<InputField name=\"ПутьКФайлу\" id=\"1\">",
            "\n\t\t\t<DataPath>ПутьКФайлу</DataPath>\n",
            "\t\t\t<ChoiceButton>true</ChoiceButton>\n",
            "\t\t\t<ShowInHeader>true</ShowInHeader>\n",
            "\t\t\t<ContextMenu name=\"ПутьКФайлуКонтекстноеМеню\""
        )),
        "{xml}"
    );
    assert!(
        xml.contains(concat!(
            "<InputField name=\"Сумма\" id=\"4\">",
            "\n\t\t\t<DataPath>Объект.Товары.Сумма</DataPath>\n",
            "\t\t\t<HeaderHorizontalAlign>Right</HeaderHorizontalAlign>\n",
            "\t\t\t<HorizontalAlign>Right</HorizontalAlign>\n",
            "\t\t\t<ContextMenu name=\"СуммаКонтекстноеМеню\""
        )),
        "{xml}"
    );
    assert!(
        xml.contains(concat!(
            "<InputField name=\"Заполнитель\" id=\"7\">",
            "\n\t\t\t<Title/>\n",
            "\t\t\t<MultiLine>true</MultiLine>\n",
            "\t\t\t<PasswordMode>true</PasswordMode>\n",
            "\t\t\t<ChoiceButton>false</ChoiceButton>\n",
            "\t\t\t<ClearButton>true</ClearButton>\n",
            "\t\t\t<SpinButton>true</SpinButton>\n",
            "\t\t\t<DropListButton>true</DropListButton>\n",
            "\t\t\t<AutoMarkIncomplete>true</AutoMarkIncomplete>\n",
            "\t\t\t<SkipOnInput>true</SkipOnInput>\n",
            "\t\t\t<ShowInHeader>false</ShowInHeader>\n",
            "\t\t\t<HeaderHorizontalAlign>Right</HeaderHorizontalAlign>\n",
            "\t\t\t<AutoMaxWidth>false</AutoMaxWidth>\n",
            "\t\t\t<Width>40</Width>\n",
            "\t\t\t<Height>3</Height>\n",
            "\t\t\t<HorizontalStretch>true</HorizontalStretch>\n",
            "\t\t\t<VerticalStretch>true</VerticalStretch>\n",
            "\t\t\t<HorizontalAlign>Right&lt;&amp;</HorizontalAlign>\n",
            "\t\t\t<InputHint>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>Меньше &lt; и &amp; больше</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</InputHint>\n",
            "\t\t\t<ContextMenu name=\"ЗаполнительКонтекстноеМеню\""
        )),
        "{xml}"
    );
}

#[test]
fn form_compile_emits_documented_command_sources_and_global_buttons() {
    let definition = json!({
        "elements": [
            {
                "cmdBar": "FormCommands",
                "commandSource": "Form",
                "autofill": true,
                "children": [
                    {
                        "button": "Local",
                        "command": "Save&<",
                        "commandName": "CommonCommand.Ignored&<",
                        "stdCommand": "IgnoredLocalStandard"
                    },
                    {
                        "button": "Global",
                        "commandName": "CommonCommand.Open&<",
                        "stdCommand": "IgnoredGlobalStandard"
                    },
                    {
                        "button": "Fallback",
                        "command": "",
                        "commandName": "CommonCommand.Fallback",
                        "stdCommand": "IgnoredFallbackStandard"
                    },
                    {
                        "button": "Standard",
                        "stdCommand": "Table.Add"
                    },
                    {
                        "button": "Help",
                        "type": "hyperlink",
                        "commandName": "CommonCommand.Help"
                    }
                ]
            },
            {
                "cmdBar": "GlobalCommands",
                "commandSource": "FormCommandPanelGlobalCommands",
                "autofill": false
            }
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(
        xml.contains(concat!(
            "<CommandBar name=\"FormCommands\" id=\"1\">\n",
            "\t\t\t<CommandSource>Form</CommandSource>\n",
            "\t\t\t<Autofill>true</Autofill>\n",
            "\t\t\t<ChildItems>"
        )),
        "{xml}"
    );
    assert!(
        xml.contains("<CommandName>Form.Command.Save&amp;&lt;</CommandName>"),
        "{xml}"
    );
    assert!(!xml.contains("CommonCommand.Ignored"), "{xml}");
    assert!(!xml.contains("IgnoredLocalStandard"), "{xml}");
    assert!(
        xml.contains("<CommandName>CommonCommand.Open&amp;&lt;</CommandName>"),
        "{xml}"
    );
    assert!(!xml.contains("IgnoredGlobalStandard"), "{xml}");
    assert!(
        xml.contains("<CommandName>CommonCommand.Fallback</CommandName>"),
        "{xml}"
    );
    assert!(!xml.contains("IgnoredFallbackStandard"), "{xml}");
    assert!(
        xml.contains("<CommandName>Form.Item.Table.StandardCommand.Add</CommandName>"),
        "{xml}"
    );
    assert_eq!(
        xml.matches("<Type>CommandBarButton</Type>").count(),
        4,
        "{xml}"
    );
    assert!(xml.contains("<Type>CommandBarHyperlink</Type>"), "{xml}");
    assert!(!xml.contains("<Type>Hyperlink</Type>"), "{xml}");
    assert!(
        !xml.contains("<CommandName>Form.Command.</CommandName>"),
        "{xml}"
    );
    let global_commands = xml
        .split_once("<CommandBar name=\"GlobalCommands\"")
        .and_then(|(_, tail)| tail.split_once("</CommandBar>"))
        .map(|(command_bar, _)| command_bar)
        .expect("GlobalCommands command bar must be emitted");
    assert!(
        global_commands.contains("<CommandSource>FormCommandPanelGlobalCommands</CommandSource>"),
        "{global_commands}"
    );
}

#[test]
fn form_compile_emits_multilingual_tooltip_values() {
    let definition = json!({
        "elements": [
            {
                "input": "Input",
                "tooltip": {"ru": "Поле < &", "en": "Input > &"}
            },
            {
                "button": "Button",
                "tooltip": {"ru": "Кнопка < &", "en": "Button > &"}
            },
            {
                "check": "Check",
                "tooltip": {"ru": "Флажок < &", "en": "Check > &"}
            },
            {
                "labelField": "Label",
                "tooltip": {"ru": "Надпись < &", "en": "Label > &"}
            },
            {
                "input": "Empty",
                "tooltip": {"ru": "", "en": ""}
            },
            {
                "input": "FormattedWrapper",
                "tooltip": {"text": "Not a tooltip language", "formatted": true}
            }
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    for (ru, en) in [
        ("Поле &lt; &amp;", "Input &gt; &amp;"),
        ("Кнопка &lt; &amp;", "Button &gt; &amp;"),
        ("Флажок &lt; &amp;", "Check &gt; &amp;"),
        ("Надпись &lt; &amp;", "Label &gt; &amp;"),
    ] {
        assert!(
                xml.contains(&format!(
                    "<ToolTip>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>{ru}</v8:content>\n\t\t\t\t</v8:item>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>en</v8:lang>\n\t\t\t\t\t<v8:content>{en}</v8:content>"
                )),
                "{xml}"
            );
    }

    let empty_start = xml.find("<InputField name=\"Empty\"").unwrap();
    let empty_end = empty_start + xml[empty_start..].find("</InputField>").unwrap();
    assert!(
        !xml[empty_start..empty_end].contains("<ToolTip"),
        "{}",
        &xml[empty_start..empty_end]
    );
    let formatted_start = xml.find("<InputField name=\"FormattedWrapper\"").unwrap();
    let formatted_end = formatted_start + xml[formatted_start..].find("</InputField>").unwrap();
    assert!(
        !xml[formatted_start..formatted_end].contains("<ToolTip"),
        "{}",
        &xml[formatted_start..formatted_end]
    );
}

#[test]
fn form_add_emits_platform_8_3_27_root_defaults() {
    fn root_children(xml: &str) -> Vec<(String, String)> {
        let document = Document::parse(xml).unwrap();
        document
            .root_element()
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                (
                    node.tag_name().name().to_string(),
                    node.text().unwrap_or("").trim().to_string(),
                )
            })
            .collect()
    }

    for (object_type, object_name, purpose) in [
        ("Catalog", "Goods", "List"),
        ("Catalog", "Goods", "Choice"),
        ("InformationRegister", "Prices", "Record"),
        ("ExternalDataProcessor", "Processor", "Object"),
    ] {
        let xml = form_add_content_xml(object_type, object_name, purpose, "2.20").unwrap();
        let names = root_children(&xml)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["AutoCommandBar", "Attributes"], "{xml}");
        assert!(!xml.contains("<Autofill>true</Autofill>"), "{xml}");
        assert!(!xml.contains("<ChildItems"), "{xml}");
    }

    let catalog = form_add_content_xml("Catalog", "Goods", "Object", "2.20").unwrap();
    assert_eq!(
        root_children(&catalog),
        [
            ("UseForFoldersAndItems".to_string(), "Items".to_string()),
            ("AutoCommandBar".to_string(), String::new()),
            ("Attributes".to_string(), "".to_string()),
        ],
        "{catalog}"
    );

    let report_defaults = [
        ("ReportFormType", "Main"),
        ("AutoShowState", "Auto"),
        ("ReportResultViewMode", "Auto"),
        ("ViewModeApplicationOnSetReportResult", "Auto"),
        ("AutoCommandBar", ""),
        ("Attributes", ""),
    ];
    for object_type in ["Report", "ExternalReport"] {
        let xml = form_add_content_xml(object_type, "Sales", "Object", "2.20").unwrap();
        let actual = root_children(&xml);
        let expected = report_defaults
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{object_type}: {xml}");
    }
}

#[test]
fn form_compile_infers_catalog_object_scope_and_preserves_explicit_scope() {
    let base = json!({
        "attributes": [{
            "name": "Object",
            "type": "CatalogObject.CorpusCatalog",
            "main": true
        }],
        "elements": [{"input": "Description", "path": "Object.Description"}]
    });

    let (xml, _) = form_compile_xml(&base, "2.20").unwrap();
    assert!(
        xml.contains("\t<UseForFoldersAndItems>Items</UseForFoldersAndItems>\n\t<AutoCommandBar"),
        "{xml}"
    );

    let mut explicit = base;
    explicit["properties"] = json!({"useForFoldersAndItems": "FoldersAndItems"});
    let (xml, _) = form_compile_xml(&explicit, "2.20").unwrap();
    assert_eq!(xml.matches("<UseForFoldersAndItems>").count(), 1, "{xml}");
    assert!(
        xml.contains("<UseForFoldersAndItems>FoldersAndItems</UseForFoldersAndItems>"),
        "{xml}"
    );
}

#[test]
fn form_compile_omits_empty_tooltip_and_button_appearance_values() {
    let definition = json!({
        "elements": [{
            "button": "NoAppearance",
            "tooltip": "",
            "tooltipRepresentation": "",
            "backColor": "",
            "font": ""
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let button_start = xml.find("<Button name=\"NoAppearance\"").unwrap();
    let button_end = button_start + xml[button_start..].find("</Button>").unwrap();
    let button = &xml[button_start..button_end];

    assert!(!button.contains("<ToolTip>"), "{button}");
    assert!(!button.contains("<ToolTip/>"), "{button}");
    assert!(!button.contains("<ToolTipRepresentation>"), "{button}");
    assert!(!button.contains("<BackColor>"), "{button}");
    assert!(!button.contains("<Font"), "{button}");
}

#[test]
fn form_compile_infers_report_defaults_in_platform_order() {
    for main_type in [
        "ReportObject.CorpusReport",
        "ExternalReportObject.CorpusReport",
    ] {
        let definition = json!({
            "attributes": [{
                "name": "Object",
                "type": main_type,
                "main": true
            }]
        });

        let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
        let document = Document::parse(&xml).unwrap();
        let actual = document
            .root_element()
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                (
                    node.tag_name().name().to_string(),
                    node.text().unwrap_or("").trim().to_string(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            [
                ("ReportFormType".to_string(), "Main".to_string()),
                ("AutoShowState".to_string(), "Auto".to_string()),
                ("ReportResultViewMode".to_string(), "Auto".to_string()),
                (
                    "ViewModeApplicationOnSetReportResult".to_string(),
                    "Auto".to_string(),
                ),
                ("AutoCommandBar".to_string(), String::new()),
                ("Attributes".to_string(), String::new()),
            ],
            "{main_type}: {xml}"
        );
    }
}

#[test]
fn form_compile_places_explicit_auto_title_before_report_defaults() {
    let definition = json!({
        "properties": {"autoTitle": false},
        "attributes": [{
            "name": "Object",
            "type": "ReportObject.CorpusReport",
            "main": true
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let actual = Document::parse(&xml)
        .unwrap()
        .root_element()
        .children()
        .filter(|node| node.is_element())
        .take(5)
        .map(|node| node.tag_name().name().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        [
            "AutoTitle",
            "ReportFormType",
            "AutoShowState",
            "ReportResultViewMode",
            "ViewModeApplicationOnSetReportResult",
        ],
        "{xml}"
    );
}

#[test]
fn form_compile_emits_english_primitive_types_for_8_3_27() {
    let definition = json!({
        "attributes": [
            {"name": "Text", "type": "String"},
            {"name": "Amount", "type": "Number"},
            {"name": "Flag", "type": "Boolean"},
            {"name": "Day", "type": "Date"},
            {"name": "Moment", "type": "DateTime"}
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert_eq!(
        xml.matches("<v8:Type>xs:string</v8:Type>").count(),
        1,
        "{xml}"
    );
    assert_eq!(
        xml.matches("<v8:Type>xs:decimal</v8:Type>").count(),
        1,
        "{xml}"
    );
    assert_eq!(
        xml.matches("<v8:Type>xs:boolean</v8:Type>").count(),
        1,
        "{xml}"
    );
    assert_eq!(
        xml.matches("<v8:Type>xs:dateTime</v8:Type>").count(),
        2,
        "{xml}"
    );
    assert!(xml.contains("<v8:Digits>10</v8:Digits>"), "{xml}");
    assert!(
        xml.contains("<v8:FractionDigits>0</v8:FractionDigits>"),
        "{xml}"
    );
    assert!(
        xml.contains("<v8:DateFractions>Date</v8:DateFractions>"),
        "{xml}"
    );
    assert!(
        xml.contains("<v8:DateFractions>DateTime</v8:DateFractions>"),
        "{xml}"
    );
    assert!(!xml.contains("<v8:Type>String</v8:Type>"), "{xml}");
    assert!(!xml.contains("<v8:Type>decimal</v8:Type>"), "{xml}");
}

#[test]
fn form_compile_emits_the_documented_8_3_27_type_mappings() {
    let cases = [
        ("ValueTable", "<v8:Type>v8:ValueTable</v8:Type>"),
        ("ValueTree", "<v8:Type>v8:ValueTree</v8:Type>"),
        ("ValueList", "<v8:Type>v8:ValueListType</v8:Type>"),
        ("FormattedString", "<v8:Type>v8ui:FormattedString</v8:Type>"),
        ("Picture", "<v8:Type>v8ui:Picture</v8:Type>"),
        ("StandardPeriod", "<v8:Type>v8:StandardPeriod</v8:Type>"),
        (
            "StandardBeginningDate",
            "<v8:Type>v8:StandardBeginningDate</v8:Type>",
        ),
        ("UUID", "<v8:Type>v8:UUID</v8:Type>"),
        ("DynamicList", "<v8:Type>cfg:DynamicList</v8:Type>"),
        ("ConstantsSet", "<v8:Type>cfg:ConstantsSet</v8:Type>"),
        ("ReportObject", "<v8:Type>cfg:ReportObject</v8:Type>"),
        (
            "DataCompositionSettings",
            "<v8:Type>dcsset:DataCompositionSettings</v8:Type>",
        ),
        (
            "DataCompositionSchema",
            "<v8:Type>dcssch:DataCompositionSchema</v8:Type>",
        ),
        (
            "DataCompositionComparisonType",
            "<v8:Type>dcscor:DataCompositionComparisonType</v8:Type>",
        ),
        (
            "DefinedType.Money",
            "<v8:TypeSet>cfg:DefinedType.Money</v8:TypeSet>",
        ),
        (
            "Characteristic.Goods",
            "<v8:TypeSet>cfg:Characteristic.Goods</v8:TypeSet>",
        ),
        ("AnyRef", "<v8:TypeSet>cfg:AnyRef</v8:TypeSet>"),
        ("CatalogRef", "<v8:TypeSet>cfg:CatalogRef</v8:TypeSet>"),
    ];

    for (type_name, expected) in cases {
        let definition = json!({"attributes": [{"name": "Value", "type": type_name}]});
        let (xml, _) = form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{type_name}: {error}"));
        assert!(xml.contains(expected), "{type_name}: {xml}");
    }
}

#[test]
fn form_compile_normalizes_documented_russian_type_aliases() {
    let cases = [
        ("СписокЗначений", "<v8:Type>v8:ValueListType</v8:Type>"),
        ("СтандартныйПериод", "<v8:Type>v8:StandardPeriod</v8:Type>"),
        (
            "СтандартнаяДатаНачала",
            "<v8:Type>v8:StandardBeginningDate</v8:Type>",
        ),
        ("УникальныйИдентификатор", "<v8:Type>v8:UUID</v8:Type>"),
        (
            "Характеристика.Товар",
            "<v8:TypeSet>cfg:Characteristic.Товар</v8:TypeSet>",
        ),
        ("ЛюбаяСсылка", "<v8:TypeSet>cfg:AnyRef</v8:TypeSet>"),
        ("ЛюбаяСсылкаИБ", "<v8:TypeSet>cfg:AnyIBRef</v8:TypeSet>"),
    ];

    for (type_name, expected) in cases {
        let definition = json!({"attributes": [{"name": "Value", "type": type_name}]});
        let (xml, _) = form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{type_name}: {error}"));
        assert!(xml.contains(expected), "{type_name}: {xml}");
    }
}

#[test]
fn form_compile_supports_every_documented_bare_reference_type_set() {
    for type_name in form_type_set_names() {
        let definition = json!({"attributes": [{"name": "Value", "type": type_name}]});
        let (xml, _) = form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{type_name}: {error}"));
        assert!(
            xml.contains(&format!("<v8:TypeSet>cfg:{type_name}</v8:TypeSet>")),
            "{type_name}: {xml}"
        );
    }
}

#[test]
fn form_compile_emits_local_namespaces_for_special_8_3_27_types() {
    let cases = [
        (
            "mxl:SpreadsheetDocument",
            "http://v8.1c.ru/8.2/data/spreadsheet",
        ),
        (
            "fd:FormattedDocument",
            "http://v8.1c.ru/8.2/data/formatted-document",
        ),
        ("d5p1:TextDocument", "http://v8.1c.ru/8.1/data/txtedt"),
        ("d5p1:Chart", "http://v8.1c.ru/8.2/data/chart"),
        ("d5p1:GanttChart", "http://v8.1c.ru/8.2/data/chart"),
        ("d5p1:Dendrogram", "http://v8.1c.ru/8.2/data/chart"),
        (
            "d5p1:FlowchartContextType",
            "http://v8.1c.ru/8.2/data/graphscheme",
        ),
        ("d5p1:GeographicalSchema", "http://v8.1c.ru/8.2/data/geo"),
        (
            "d5p1:DataAnalysisTimeIntervalUnitType",
            "http://v8.1c.ru/8.2/data/data-analysis",
        ),
        ("pdfdoc:PDFDocument", "http://v8.1c.ru/8.3/data/pdf"),
        ("pl:Planner", "http://v8.1c.ru/8.3/data/planner"),
    ];

    for (type_name, namespace) in cases {
        let definition = json!({"attributes": [{"name": "Value", "type": type_name}]});
        let (xml, _) = form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{type_name}: {error}"));
        let prefix = type_name.split_once(':').unwrap().0;
        assert!(
            xml.contains(&format!(
                "<v8:Type xmlns:{prefix}=\"{namespace}\">{type_name}</v8:Type>"
            )),
            "{type_name}: {xml}"
        );
    }
}

#[test]
fn form_compile_groups_composite_type_description_children_in_xsd_order() {
    let type_id = "11111111-2222-4333-8444-555555555555";
    let definition = json!({
        "attributes": [{
            "name": "Value",
            "type": format!(
                "typeid:{type_id} | DefinedType.Money | string(12,fixed) | CatalogRef.Goods | decimal(15,2,nonneg) | time"
            )
        }]
    });
    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let wrapper = document
        .descendants()
        .find(|node| node.has_tag_name("Attribute"))
        .and_then(|node| form_child(node, "Type"))
        .unwrap();
    let names = wrapper
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "Type",
            "Type",
            "Type",
            "Type",
            "TypeSet",
            "TypeId",
            "NumberQualifiers",
            "StringQualifiers",
            "DateQualifiers",
        ],
        "{xml}"
    );
    assert!(xml.contains("<v8:AllowedLength>Fixed</v8:AllowedLength>"));
    assert!(xml.contains("<v8:AllowedSign>Nonnegative</v8:AllowedSign>"));
    assert!(xml.contains(&format!("<v8:TypeId>{type_id}</v8:TypeId>")));
}

#[test]
fn form_compile_rejects_types_outside_the_fixed_8_3_27_contract() {
    for type_name in [
        "string(foo)",
        "string(1025)",
        "string(0,fixed)",
        "string(12,other)",
        "decimal(10)",
        "decimal(39,0)",
        "decimal(10,11)",
        "decimal(10,2,positive)",
        "decimal(10.5,2)",
        "typeid:not-a-uuid",
        "Unknown",
        "Unknown.Name",
        "v8:Unknown",
        "CatalogRef.",
        "CatalogRef.Foo.Bar",
        "String | string(10)",
        "Date | DateTime",
        "String || Boolean",
    ] {
        let definition = json!({"attributes": [{"name": "Value", "type": type_name}]});
        let error = form_compile_xml(&definition, "2.20").unwrap_err();
        assert!(error.contains(type_name), "{type_name}: {error}");
        assert!(error.contains("8.3.27"), "{type_name}: {error}");
    }
}

#[test]
fn form_type_parameter_boundaries_match_8_3_27() {
    for type_name in [
        "string(0)",
        "string(1024)",
        "string(12,variable)",
        "string(12,fixed)",
        "decimal(0,0)",
        "decimal(38,0)",
        "decimal(38,38)",
        "decimal(38,38,nonneg)",
    ] {
        let definition = json!({"attributes": [{"name": "Value", "type": type_name}]});
        form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{type_name}: {error}"));
    }
}

#[test]
fn form_parameters_use_the_same_8_3_27_type_contract() {
    let type_id = "11111111-2222-4333-8444-555555555555";
    let definition = json!({
        "parameters": [{
            "name": "Choice",
            "type": format!("typeid:{type_id} | AnyRef | Boolean | string(12,fixed)")
        }]
    });
    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let wrapper = document
        .descendants()
        .find(|node| node.has_tag_name("Parameter"))
        .and_then(|node| form_child(node, "Type"))
        .unwrap();
    let names = wrapper
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        ["Type", "Type", "TypeSet", "TypeId", "StringQualifiers"],
        "{xml}"
    );

    let invalid = json!({"parameters": [{"name": "Broken", "type": "string(1025)"}]});
    let error = form_compile_xml(&invalid, "2.20").unwrap_err();
    assert!(error.contains("string(1025)"), "{error}");
    assert!(error.contains("8.3.27"), "{error}");
}

#[test]
fn form_format_type_preserves_the_full_roundtrip_contract() {
    let type_id = "11111111-2222-4333-8444-555555555555";
    let xml = format!(
        r#"<Type xmlns:v8="{FORM_V8_NS}" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:d5p1="http://v8.1c.ru/8.2/data/chart">
                <v8:Type>xs:string</v8:Type>
                <v8:Type>xs:decimal</v8:Type>
                <v8:Type>xs:binary</v8:Type>
                <v8:Type>d5p1:Chart</v8:Type>
                <v8:TypeSet>cfg:DefinedType.Money</v8:TypeSet>
                <v8:TypeSet>cfg:AnyRef</v8:TypeSet>
                <v8:TypeId>{type_id}</v8:TypeId>
                <v8:NumberQualifiers><v8:Digits>15</v8:Digits><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Nonnegative</v8:AllowedSign></v8:NumberQualifiers>
                <v8:StringQualifiers><v8:Length>12</v8:Length><v8:AllowedLength>Fixed</v8:AllowedLength></v8:StringQualifiers>
            </Type>"#
    );
    let document = Document::parse(&xml).unwrap();

    let formatted = form_format_type(document.root_element());
    assert_eq!(
            formatted,
            format!(
                "string(12,fixed) | decimal(15,2,nonneg) | binary | d5p1:Chart | DefinedType.Money | AnyRef | typeid:{type_id}"
            )
        );

    let mut emitted = Vec::new();
    emit_form_type(&mut emitted, &formatted, "").unwrap();
    assert!(
        emitted
            .iter()
            .any(|line| line == "\t<v8:Type>xs:base64Binary</v8:Type>"),
        "{emitted:?}"
    );
}

#[test]
fn form_binary_aliases_emit_the_native_base64_qname() {
    for alias in ["binary", "xs:binary", "xs:base64Binary"] {
        let mut emitted = Vec::new();
        emit_form_type(&mut emitted, alias, "").unwrap();
        assert!(
            emitted
                .iter()
                .any(|line| line == "\t<v8:Type>xs:base64Binary</v8:Type>"),
            "{alias}: {emitted:?}"
        );
        assert!(
            !emitted.iter().any(|line| line.contains("xs:binary")),
            "{emitted:?}"
        );
    }
    for aliases in [
        "binary | xs:binary",
        "binary | xs:base64Binary",
        "xs:binary | xs:base64Binary",
    ] {
        let mut emitted = Vec::new();
        assert!(
            emit_form_type(&mut emitted, aliases, "").is_err(),
            "{aliases}"
        );
        assert!(
            emitted.is_empty(),
            "invalid unions must not emit a partial type"
        );
    }
    let native = format!("<Type xmlns:v8=\"{FORM_V8_NS}\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"><v8:Type>xs:base64Binary</v8:Type></Type>");
    let document = Document::parse(&native).unwrap();
    assert_eq!(form_format_type(document.root_element()), "binary");
}

#[test]
fn form_compile_omits_auto_title_for_the_documented_empty_marker() {
    let definition = json!({
        "title": "Corpus form",
        "properties": {"autoTitle": ""}
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(xml.contains("<Title>"), "{xml}");
    assert!(!xml.contains("<AutoTitle"), "{xml}");
}

#[test]
fn form_compile_honors_pascal_case_auto_title_alias_during_title_injection() {
    for (value, expected) in [(json!(false), Some("false")), (json!(""), None)] {
        let definition = json!({
            "title": "Corpus form",
            "properties": {"AutoTitle": value}
        });

        let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
        let document = Document::parse(&xml).unwrap();
        let actual = document
            .root_element()
            .children()
            .find(|node| node.has_tag_name("AutoTitle"))
            .and_then(|node| node.text());

        assert_eq!(actual, expected, "{xml}");
        assert_eq!(
            xml.matches("<AutoTitle>").count(),
            usize::from(expected.is_some())
        );
    }
}

#[test]
fn form_compile_rejects_unknown_and_duplicate_root_properties() {
    for value in [json!(1), json!("")] {
        let unknown = json!({"properties": {"bogus": value}});
        let error = form_compile_xml(&unknown, "2.20").unwrap_err();
        assert!(error.contains("unsupported form root property"), "{error}");
    }

    let duplicate = json!({
        "properties": {"autoTitle": false, "AutoTitle": true}
    });
    let error = form_compile_xml(&duplicate, "2.20").unwrap_err();
    assert!(error.contains("duplicate form root property"), "{error}");
}

#[test]
fn form_compile_rejects_duplicate_report_property_aliases() {
    for (camel, pascal) in [
        ("reportFormType", "ReportFormType"),
        ("autoShowState", "AutoShowState"),
        ("reportResultViewMode", "ReportResultViewMode"),
        (
            "viewModeApplicationOnSetReportResult",
            "ViewModeApplicationOnSetReportResult",
        ),
    ] {
        let mut properties = Map::new();
        properties.insert(camel.to_string(), Value::String("Auto".to_string()));
        properties.insert(pascal.to_string(), Value::String("Auto".to_string()));
        if camel == "reportFormType" {
            properties.insert(camel.to_string(), Value::String("Main".to_string()));
            properties.insert(pascal.to_string(), Value::String("Main".to_string()));
        }
        let definition = json!({
            "properties": properties,
            "attributes": [{
                "name": "Object",
                "type": "ReportObject.CorpusReport",
                "main": true
            }]
        });

        let error = form_compile_xml(&definition, "2.20").unwrap_err();
        assert!(
            error.contains("duplicate form root property"),
            "{camel}: {error}"
        );
    }
}

#[test]
fn form_compile_rejects_empty_string_and_reference_root_properties() {
    for property in [
        "settingsStorage",
        "groupList",
        "reportResult",
        "detailsData",
        "variantAppearance",
        "customSettingsFolder",
    ] {
        let mut properties = Map::new();
        properties.insert(property.to_string(), Value::String(String::new()));
        let definition = json!({"properties": properties});

        let error = form_compile_xml(&definition, "2.20").unwrap_err();
        assert!(error.contains(property), "{property}: {error}");
        assert!(error.contains("non-empty string"), "{property}: {error}");
    }
}

#[test]
fn form_compile_rejects_root_property_values_outside_8_3_27_enums() {
    for (property, value) in [
        ("windowOpeningMode", "Modeless"),
        ("enterKeyBehavior", "NewLine"),
        ("saveDataInSettings", "Use"),
        ("autoTime", "Current"),
        ("usePostingMode", "Postings"),
        ("verticalScroll", "Auto"),
        ("group", "AlwaysVertical"),
    ] {
        let definition = json!({"properties": {(property): value}});
        let error = form_compile_xml(&definition, "2.20").unwrap_err();
        assert!(error.contains(property), "{property}: {error}");
        assert!(error.contains("8.3.27"), "{property}: {error}");
    }
}

#[test]
fn form_compile_accepts_every_8_3_27_element_enum_value() {
    type EnumDefinition = fn(&str) -> Value;
    type EnumCase<'a> = (&'a str, &'a [&'a str], EnumDefinition);

    let cases: &[EnumCase<'_>] = &[
        (
            "Group",
            &[
                "Horizontal",
                "Vertical",
                "HorizontalIfPossible",
                "AlwaysHorizontal",
            ],
            |value| json!({"elements": [{"name": "Group", "group": value}]}),
        ),
        (
            "Representation",
            &["List", "HierarchicalList", "Tree"],
            |value| json!({"elements": [{"table": "Rows", "representation": value}]}),
        ),
        (
            "Behavior",
            &["Usual", "Collapsible", "PopUp", "Auto"],
            |value| json!({"elements": [{"group": "Group", "behavior": value}]}),
        ),
        (
            "Representation",
            &[
                "None",
                "StrongSeparation",
                "WeakSeparation",
                "NormalSeparation",
                "GroupBox",
                "Line",
                "Margin",
            ],
            |value| json!({"elements": [{"group": "Group", "representation": value}]}),
        ),
        (
            "CurrentRowUse",
            &["Use", "DontUse", "Auto"],
            |value| json!({"elements": [{"group": "Group", "currentRowUse": value}]}),
        ),
        (
            "CheckBoxType",
            &["Auto", "CheckBox", "Tumbler", "Switcher"],
            |value| json!({"elements": [{"check": "Check", "checkBoxType": value}]}),
        ),
        (
            "TitleLocation",
            &["None", "Auto", "Left", "Top", "Right", "Bottom"],
            |value| json!({"elements": [{"input": "Input", "titleLocation": value}]}),
        ),
        (
            "Type",
            &[
                "CommandBarButton",
                "UsualButton",
                "Hyperlink",
                "CommandBarHyperlink",
            ],
            |value| json!({"elements": [{"button": "Button", "type": value}]}),
        ),
        (
            "Representation",
            &["Text", "Picture", "PictureAndText", "Auto"],
            |value| json!({"elements": [{"button": "Button", "representation": value}]}),
        ),
        (
            "LocationInCommandBar",
            &[
                "Auto",
                "InAdditionalSubmenu",
                "InCommandBar",
                "InCommandBarAndInAdditionalSubmenu",
            ],
            |value| json!({"elements": [{"button": "Button", "locationInCommandBar": value}]}),
        ),
    ];

    for (xml_name, values, definition) in cases {
        for value in *values {
            let (xml, _) = form_compile_xml(&definition(value), "2.20")
                .unwrap_or_else(|error| panic!("{xml_name}={value}: {error}"));
            assert!(
                xml.contains(&format!("<{xml_name}>{value}</{xml_name}>")),
                "{xml_name}={value}: {xml}"
            );
        }
    }
}

#[test]
fn form_element_emitters_escape_enum_text_when_preflight_is_bypassed() {
    let mut lines = vec!["<Form>".to_string()];
    let mut ids = FormIdAllocator::new();
    let check = json!({
        "checkBoxType": "</CheckBoxType><Injected/>",
        "titleLocation": "</TitleLocation><Injected/>"
    });
    emit_form_check(
        &mut lines,
        check.as_object().unwrap(),
        "Check",
        "\t",
        &mut ids,
        Some("ru"),
    )
    .unwrap();
    let input = json!({"titleLocation": "</TitleLocation><Injected/>"});
    emit_form_input(
        &mut lines,
        input.as_object().unwrap(),
        "Input",
        "\t",
        &mut ids,
        Some("ru"),
    )
    .unwrap();
    lines.push("</Form>".to_string());
    let xml = lines.join("\n");

    Document::parse(&xml).unwrap_or_else(|error| panic!("{error}: {xml}"));
    assert!(!xml.contains("<Injected/>"), "{xml}");
    assert!(xml.contains("&lt;Injected/&gt;"), "{xml}");
}

#[test]
fn form_compile_rejects_noncanonical_8_3_27_root_uint32_values() {
    for property in ["width", "height", "scale"] {
        for value in [json!(-1), json!(1.5), json!(4_294_967_296_u64)] {
            let definition = json!({"properties": {(property): value}});

            let error = form_compile_xml(&definition, "2.20").unwrap_err();

            assert!(error.contains(property), "{property}: {error}");
            assert!(error.contains("0..=4294967295"), "{property}: {error}");
        }
    }
}

#[test]
fn form_compile_accepts_8_3_27_root_uint32_boundaries() {
    let definition = json!({
        "properties": {
            "width": 0,
            "height": 4_294_967_295_u64,
            "scale": 4_294_967_295_u64
        }
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(xml.contains("<Width>0</Width>"), "{xml}");
    assert!(xml.contains("<Height>4294967295</Height>"), "{xml}");
    assert!(xml.contains("<Scale>4294967295</Scale>"), "{xml}");
}

#[test]
fn form_compile_preserves_explicit_report_defaults_without_duplicates() {
    let definition = json!({
        "properties": {
            "ReportFormType": "Settings",
            "autoShowState": "DontShow",
            "ReportResultViewMode": "Auto",
            "viewModeApplicationOnSetReportResult": "Auto"
        },
        "attributes": [{
            "name": "Object",
            "type": "ReportObject.CorpusReport",
            "main": true
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let actual = document
        .root_element()
        .children()
        .filter(|node| node.is_element())
        .take(4)
        .map(|node| {
            (
                node.tag_name().name().to_string(),
                node.text().unwrap_or("").trim().to_string(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        [
            ("ReportFormType".to_string(), "Settings".to_string()),
            ("AutoShowState".to_string(), "DontShow".to_string()),
            ("ReportResultViewMode".to_string(), "Auto".to_string()),
            (
                "ViewModeApplicationOnSetReportResult".to_string(),
                "Auto".to_string(),
            ),
        ],
        "{xml}"
    );
    for tag in [
        "ReportFormType",
        "AutoShowState",
        "ReportResultViewMode",
        "ViewModeApplicationOnSetReportResult",
    ] {
        assert_eq!(xml.matches(&format!("<{tag}>")).count(), 1, "{xml}");
    }
}

#[test]
fn form_compile_emits_every_supported_8_3_27_binding_path_property() {
    let definition = json!({
        "attributes": [
            {"name": "Value", "type": "String"},
            {
                "name": "Rows",
                "type": "ValueTable",
                "columns": [
                    {"name": "Value", "type": "String"},
                    {"name": "Presentation", "type": "String"},
                    {"name": "Picture", "type": "Number"}
                ]
            }
        ],
        "elements": [
            {
                "group": "Header",
                "titleDataPath": "Value"
            },
            {
                "input": "Editor",
                "path": "Rows",
                "footerDataPath": "Value",
                "multipleValueDataPath": "Rows.Value",
                "multipleValuePresentDataPath": "Rows.Presentation",
                "multipleValuePictureDataPath": "Rows.Picture"
            },
            {
                "table": "Rows",
                "path": "Rows",
                "rowPictureDataPath": "Rows.Picture"
            }
        ]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    for (tag, value) in [
        ("DataPath", "Rows"),
        ("TitleDataPath", "Value"),
        ("FooterDataPath", "Value"),
        ("MultipleValueDataPath", "Rows.Value"),
        ("MultipleValuePresentDataPath", "Rows.Presentation"),
        ("RowPictureDataPath", "Rows.Picture"),
        ("MultipleValuePictureDataPath", "Rows.Picture"),
    ] {
        assert!(
            xml.contains(&format!("<{tag}>{value}</{tag}>")),
            "{tag}: {xml}"
        );
    }
}

#[test]
fn form_compile_emits_input_dimensions_before_multiple_value_paths_in_8_3_27_order() {
    let definition = json!({
        "attributes": [{
            "name": "Rows",
            "type": "ValueTable",
            "columns": [
                {"name": "Value", "type": "String"},
                {"name": "Presentation", "type": "String"},
                {"name": "Picture", "type": "Number"}
            ]
        }],
        "elements": [{
            "input": "Editor",
            "path": "Rows",
            "multipleValueDataPath": "Rows.Value",
            "multipleValuePresentDataPath": "Rows.Presentation",
            "multipleValuePictureDataPath": "Rows.Picture",
            "width": 10,
            "height": 20
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let input = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "InputField")))
        .unwrap();
    let relevant = input
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .filter(|name| {
            [
                "DataPath",
                "Width",
                "Height",
                "MultipleValueDataPath",
                "MultipleValuePictureDataPath",
                "MultipleValuePresentDataPath",
            ]
            .contains(name)
        })
        .collect::<Vec<_>>();

    assert_eq!(
        relevant,
        [
            "DataPath",
            "Width",
            "Height",
            "MultipleValueDataPath",
            "MultipleValuePictureDataPath",
            "MultipleValuePresentDataPath",
        ],
        "{xml}"
    );
}

#[test]
fn form_compile_emits_valuetable_columns_used_by_multiple_value_paths() {
    let definition = json!({
        "attributes": [{
            "name": "Rows",
            "type": "ValueTable",
            "columns": [
                {"name": "Value", "type": "String"},
                {"name": "Presentation", "type": "String"},
                {"name": "Picture", "type": "Number"}
            ]
        }],
        "elements": [{
            "input": "Editor",
            "path": "Rows",
            "multipleValueDataPath": "Rows.Value",
            "multipleValuePresentDataPath": "Rows.Presentation",
            "multipleValuePictureDataPath": "Rows.Picture"
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    let document = Document::parse(&xml).unwrap();
    let attribute = document
        .descendants()
        .find(|node| {
            node.has_tag_name((FORM_LOGFORM_NS, "Attribute"))
                && node.attribute("name") == Some("Rows")
        })
        .unwrap();
    let columns = attribute
        .descendants()
        .filter(|node| node.has_tag_name((FORM_LOGFORM_NS, "Column")))
        .filter_map(|node| node.attribute("name"))
        .collect::<Vec<_>>();
    assert_eq!(columns, ["Value", "Presentation", "Picture"], "{xml}");
}

#[test]
fn form_compile_accepts_supported_collection_type_aliases_with_columns() {
    for type_name in [
        "valuetable",
        "ТаблицаЗначений",
        "valuetree",
        "ДеревоЗначений",
    ] {
        let definition = json!({
            "attributes": [{
                "name": "Rows",
                "type": type_name,
                "columns": [{"name": "Value", "type": "String"}]
            }]
        });

        let (xml, _) = form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{type_name}: {error}"));

        assert!(xml.contains("<Columns>"), "{type_name}: {xml}");
    }
}

#[test]
fn form_compile_rejects_multiple_value_paths_for_scalar_input() {
    let definition = json!({
        "attributes": [{"name": "Value", "type": "String"}],
        "elements": [{
            "input": "Editor",
            "path": "Value",
            "multipleValueDataPath": "Value.Item"
        }]
    });

    let error = form_compile_xml(&definition, "2.20").unwrap_err();

    assert!(error.contains("MultipleValueDataPath"), "{error}");
    assert!(error.contains("collection"), "{error}");
    assert!(error.contains("Value"), "{error}");
}

#[test]
fn form_compile_rejects_multiple_value_paths_outside_the_input_collection() {
    for invalid_path in ["Other.Value", "Rows"] {
        let definition = json!({
            "attributes": [
                {
                    "name": "Rows",
                    "type": "ValueTable",
                    "columns": [{"name": "Value", "type": "String"}]
                },
                {
                    "name": "Other",
                    "type": "ValueTable",
                    "columns": [{"name": "Value", "type": "String"}]
                }
            ],
            "elements": [{
                "input": "Editor",
                "path": "Rows",
                "multipleValueDataPath": invalid_path
            }]
        });

        let error = form_compile_xml(&definition, "2.20").unwrap_err();

        assert!(
            error.contains("MultipleValueDataPath"),
            "{invalid_path}: {error}"
        );
        assert!(error.contains("Rows"), "{invalid_path}: {error}");
        assert!(error.contains("subpath"), "{invalid_path}: {error}");
    }
}

#[test]
fn form_compile_rejects_unknown_multiple_value_collection_column() {
    let definition = json!({
        "attributes": [{
            "name": "Rows",
            "type": "ValueTable",
            "columns": [{"name": "Value", "type": "String"}]
        }],
        "elements": [{
            "input": "Editor",
            "path": "Rows",
            "multipleValueDataPath": "Rows.Missing"
        }]
    });

    let error = form_compile_xml(&definition, "2.20").unwrap_err();

    assert!(error.contains("MultipleValueDataPath"), "{error}");
    assert!(error.contains("Missing"), "{error}");
    assert!(error.contains("column"), "{error}");
}

#[test]
fn form_compile_emits_group_title_path_before_current_row_use() {
    let definition = json!({
        "attributes": [{"name": "Value", "type": "String"}],
        "elements": [{
            "group": "Header",
            "titleDataPath": "Value",
            "currentRowUse": "DontUse"
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let group = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "UsualGroup")))
        .unwrap();
    let relevant = group
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .filter(|name| ["TitleDataPath", "CurrentRowUse"].contains(name))
        .collect::<Vec<_>>();

    assert_eq!(relevant, ["TitleDataPath", "CurrentRowUse"], "{xml}");
}

#[test]
fn form_compile_rejects_header_data_path_for_usual_group() {
    let definition = json!({
        "attributes": [{"name": "Value", "type": "String"}],
        "elements": [{
            "group": "Header",
            "headerDataPath": "Value"
        }]
    });

    let error = form_compile_xml(&definition, "2.20").unwrap_err();

    assert!(error.contains("HeaderDataPath"), "{error}");
    assert!(error.contains("UsualGroup"), "{error}");
    assert!(error.contains("8.3.27"), "{error}");
}

#[test]
fn form_compile_rejects_popup_group_with_line_representation() {
    let definition = json!({
        "elements": [{
            "group": "Header",
            "behavior": "PopUp",
            "representation": "Line"
        }]
    });

    let error = form_compile_xml(&definition, "2.20").unwrap_err();

    assert!(error.contains("PopUp"), "{error}");
    assert!(error.contains("Line"), "{error}");
    assert!(error.contains("8.3.27"), "{error}");
}

#[test]
fn form_compile_emits_checkbox_title_location_before_checkbox_type() {
    let definition = json!({
        "elements": [{
            "check": "Enabled",
            "titleLocation": "Bottom",
            "checkBoxType": "Tumbler"
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let check = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "CheckBoxField")))
        .unwrap();
    let relevant = check
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .filter(|name| ["TitleLocation", "CheckBoxType"].contains(name))
        .collect::<Vec<_>>();

    assert_eq!(relevant, ["TitleLocation", "CheckBoxType"], "{xml}");
}

#[test]
fn form_compile_emits_button_representation_before_command_name() {
    let definition = json!({
        "elements": [{
            "button": "Run",
            "command": "Run",
            "representation": "Text"
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let button = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "Button")))
        .unwrap();
    let relevant = button
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .filter(|name| ["Representation", "CommandName"].contains(name))
        .collect::<Vec<_>>();

    assert_eq!(relevant, ["Representation", "CommandName"], "{xml}");
}

#[test]
fn form_compile_emits_related_row_picture_path_before_nil_row_filter() {
    let definition = json!({
        "attributes": [{
            "name": "Rows",
            "type": "ValueTable",
            "columns": [{"name": "Picture", "type": "Number"}]
        }],
        "elements": [{
            "table": "Rows",
            "path": "Rows",
            "rowPictureDataPath": "Rows.Picture"
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let table = document
        .descendants()
        .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "Table")))
        .unwrap();
    let relevant = table
        .children()
        .filter(|node| node.is_element())
        .filter(|node| {
            ["DataPath", "RowFilter", "RowPictureDataPath"].contains(&node.tag_name().name())
        })
        .collect::<Vec<_>>();

    assert_eq!(
        relevant
            .iter()
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>(),
        ["DataPath", "RowPictureDataPath", "RowFilter"],
        "{xml}"
    );
    assert_eq!(
        relevant[2].attribute(("http://www.w3.org/2001/XMLSchema-instance", "nil")),
        Some("true"),
        "{xml}"
    );
}

#[test]
fn form_compile_rejects_unrelated_row_picture_data_path() {
    let definition = json!({
        "attributes": [
            {"name": "Rows", "type": "ValueTable"},
            {"name": "Picture", "type": "Number"}
        ],
        "elements": [{
            "table": "Rows",
            "path": "Rows",
            "rowPictureDataPath": "Picture"
        }]
    });

    let error = form_compile_xml(&definition, "2.20").unwrap_err();

    assert!(error.contains("RowPictureDataPath"), "{error}");
    assert!(error.contains("Rows"), "{error}");
    assert!(error.contains("subpath"), "{error}");
}

#[test]
fn form_compile_rejects_noncanonical_input_uint32_values() {
    for property in ["width", "height"] {
        for value in [json!(-1), json!(1.5), json!(4_294_967_296_u64), json!("10")] {
            let definition = json!({
                "elements": [{"input": "Editor", (property): value}]
            });

            let error = form_compile_xml(&definition, "2.20").unwrap_err();

            assert!(error.contains(property), "{property}: {error}");
            assert!(error.contains("0..=4294967295"), "{property}: {error}");
        }
    }
}

#[test]
fn form_compile_accepts_input_uint32_boundaries() {
    let definition = json!({
        "elements": [{
            "input": "Editor",
            "width": 0,
            "height": 4_294_967_295_u64
        }]
    });

    let (xml, _) = form_compile_xml(&definition, "2.20").unwrap();

    assert!(xml.contains("<Width>0</Width>"), "{xml}");
    assert!(xml.contains("<Height>4294967295</Height>"), "{xml}");
}

#[test]
fn form_compile_rejects_missing_roots_for_all_binding_properties() {
    for (json_key, xml_tag) in [
        ("path", "DataPath"),
        ("titleDataPath", "TitleDataPath"),
        ("footerDataPath", "FooterDataPath"),
        ("multipleValueDataPath", "MultipleValueDataPath"),
        (
            "multipleValuePresentDataPath",
            "MultipleValuePresentDataPath",
        ),
        ("rowPictureDataPath", "RowPictureDataPath"),
        (
            "multipleValuePictureDataPath",
            "MultipleValuePictureDataPath",
        ),
    ] {
        let mut element = match json_key {
            "titleDataPath" => json!({"group": "Header"}),
            "rowPictureDataPath" => json!({"table": "Rows", "path": "Rows"}),
            _ => json!({"input": "Value"}),
        };
        element[json_key] = json!("Missing.Value");
        let definition = json!({
            "attributes": [{"name": "Rows", "type": "ValueTable"}],
            "elements": [element]
        });

        let Err(error) = form_compile_xml(&definition, "2.20") else {
            panic!("{json_key} unexpectedly passed validation");
        };
        assert!(error.contains(xml_tag), "{json_key}: {error}");
        assert!(error.contains("Missing"), "{json_key}: {error}");
    }
}

#[test]
fn form_compile_uses_shared_binding_path_semantics() {
    for path in ["Rows[0].Value", "~Rows.Value", "123", "1/2:dead-beef"] {
        let definition = json!({
            "attributes": [{"name": "Rows", "type": "ValueTable"}],
            "elements": [{"input": "Value", "path": path}]
        });
        form_compile_xml(&definition, "2.20").unwrap_or_else(|error| panic!("{path}: {error}"));
    }

    let items_path = json!({
        "attributes": [{"name": "Rows", "type": "ValueTable"}],
        "elements": [{
            "table": "Rows",
            "path": "Rows",
            "columns": [{
                "input": "Value",
                "path": "Items.Rows.CurrentData.Value"
            }]
        }]
    });
    form_compile_xml(&items_path, "2.20").unwrap();

    for opaque_table_path in ["123", "1/2:dead-beef"] {
        let opaque_items_path = json!({
            "attributes": [{"name": "Rows", "type": "ValueTable"}],
            "elements": [{
                "table": "Rows",
                "path": opaque_table_path,
                "columns": [{
                    "input": "Value",
                    "path": "Items.Rows.CurrentData.Value"
                }]
            }]
        });
        form_compile_xml(&opaque_items_path, "2.20")
            .unwrap_or_else(|error| panic!("{opaque_table_path}: {error}"));
    }

    let missing_table = json!({
        "attributes": [{"name": "Rows", "type": "ValueTable"}],
        "elements": [{
            "input": "Value",
            "path": "Items.Unknown.CurrentData.Value"
        }]
    });
    let Err(error) = form_compile_xml(&missing_table, "2.20") else {
        panic!("missing Items table unexpectedly passed validation");
    };
    assert!(
        error.contains("table element 'Unknown' not found"),
        "{error}"
    );
}

#[test]
fn form_compile_rejects_non_string_element_event_handlers() {
    for definition in [
        json!({
            "elements": [{
                "input": "Field",
                "on": [{"event": "OnChange", "handler": 123}]
            }]
        }),
        json!({
            "elements": [{
                "input": "Field",
                "handlers": {"OnChange": 123},
                "on": ["OnChange"]
            }]
        }),
    ] {
        let Err(error) = form_compile_xml(&definition, "2.20") else {
            panic!("non-string event handler must be rejected");
        };
        assert!(error.contains("FORM_EVENT_EMPTY_HANDLER"), "{error}");
    }
}

#[test]
fn form_compile_rejects_table_events_without_path() {
    let definition = json!({
        "elements": [{
            "table": "Rows",
            "on": ["Selection"],
            "columns": []
        }]
    });

    let Err(error) = form_compile_xml(&definition, "2.20") else {
        panic!("an unbound Table event must be rejected");
    };
    assert!(error.contains("FORM_EVENT_NOT_ALLOWED"), "{error}");
    assert!(error.contains("non-empty path/DataPath"), "{error}");
}

#[test]
fn text_document_binding_uses_the_qname_namespace_and_requires_a_path() {
    let xml = format!(
        r#"<Form xmlns="{FORM_LOGFORM_NS}" xmlns:v8="{FORM_V8_NS}"><ChildItems><TextDocumentField name="Field"><DataPath>Protocol</DataPath></TextDocumentField></ChildItems><Attributes><Attribute name="Protocol"><Type><v8:Type xmlns:txt="http://v8.1c.ru/8.1/data/txtedt">txt:TextDocument</v8:Type></Type></Attribute></Attributes></Form>"#
    );
    let document = Document::parse(&xml).unwrap();
    let definition = json!({"elements":[{"textDoc":"Field","path":"Protocol"}]});
    assert!(
        form_validate_requested_text_document_bindings(document.root_element(), &definition)
            .is_ok()
    );
    assert!(form_edit_validate_emitted_type_qnames(
        document.root_element(),
        &["txt:TextDocument".to_string()]
    )
    .is_ok());

    let without_path = json!({"elements":[{"textDoc":"Field"}]});
    let error =
        form_validate_requested_text_document_bindings(document.root_element(), &without_path)
            .unwrap_err();
    assert!(error.contains("requires non-empty path"), "{error}");

    let union = xml.replace(
        "</Type>",
        "<v8:Type xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">xs:string</v8:Type></Type>",
    );
    let union = Document::parse(&union).unwrap();
    let error = form_validate_requested_text_document_bindings(union.root_element(), &definition)
        .unwrap_err();
    assert!(
        error.contains("requires a TextDocument attribute"),
        "{error}"
    );

    let wrong_namespace = xml.replace(
        "http://v8.1c.ru/8.1/data/txtedt",
        "http://www.w3.org/2001/XMLSchema",
    );
    let wrong_namespace = Document::parse(&wrong_namespace).unwrap();
    let error =
        form_validate_requested_text_document_bindings(wrong_namespace.root_element(), &definition)
            .unwrap_err();
    assert!(
        error.contains("requires a TextDocument attribute"),
        "{error}"
    );
}

#[test]
fn retired_form_compiler_does_not_accept_text_document_field() {
    let error = form_compile_xml(
        &json!({
            "attributes":[{"name":"Protocol","type":"d5p1:TextDocument"}],
            "elements":[{"textDoc":"Field","path":"Protocol"}]
        }),
        "2.20",
    )
    .unwrap_err();
    assert!(error.contains("only by item.add"), "{error}");
}

#[test]
fn html_document_field_matches_platform_fixture_and_preserves_requested_properties() {
    let (xml, _) = form_compile_xml(
            &json!({
                "attributes": [{"name": "АдресHTML", "type": "string"}],
                "elements": [{"html": "ПолеHTML", "path": "АдресHTML",
                    "title": "Описание & детали", "titleLocation": "none",
                    "tooltip": "Подсказка", "tooltipRepresentation": "Button",
                    "skipOnInput": true, "width": 1, "height": 2,
                    "autoMaxWidth": false, "autoMaxHeight": false,
                    "visible": false, "enabled": false, "readOnly": true,
                    "on": ["OnClick", "DocumentComplete"],
                "handlers": {"OnClick": "ПолеHTMLПриНажатии", "DocumentComplete": "ПолеHTMLДокументСформирован"}}]
            }),
            "2.20",
        )
        .unwrap();
    let doc = Document::parse(&xml).unwrap();
    let field = doc
        .descendants()
        .find(|n| n.has_tag_name("HTMLDocumentField"))
        .unwrap();
    let fixture = include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/DataProcessors/ТестHTML/Forms/Форма/Ext/Form.xml"));
    let oracle = Document::parse(fixture.trim_start_matches('\u{feff}')).unwrap();
    let oracle_field = oracle
        .descendants()
        .find(|n| n.has_tag_name("HTMLDocumentField"))
        .unwrap();
    for tag in [
        "DataPath",
        "SkipOnInput",
        "TitleLocation",
        "ContextMenu",
        "ExtendedTooltip",
    ] {
        let actual = field.children().find(|n| n.has_tag_name(tag)).unwrap();
        let expected = oracle_field
            .children()
            .find(|n| n.has_tag_name(tag))
            .unwrap();
        assert_eq!(actual.text(), expected.text(), "{tag}");
        assert_eq!(
            actual.attribute("name"),
            expected.attribute("name"),
            "{tag}"
        );
    }
    let child_names = field
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .collect::<Vec<_>>();
    // Канон ревью #1389 задаёт общий порядок HTML- и TextDocumentField.
    // Положение SkipOnInput после ReadOnly подтверждено живым кругом
    // 8.3.27.2074 при ревью #1389; выгрузка HTML ниже сверяет значения
    // DataPath, SkipOnInput, TitleLocation и companions.
    assert_eq!(
        child_names,
        [
            "DataPath",
            "Visible",
            "Enabled",
            "ReadOnly",
            "SkipOnInput",
            "Title",
            "TitleLocation",
            "ToolTip",
            "ToolTipRepresentation",
            "Width",
            "AutoMaxWidth",
            "Height",
            "AutoMaxHeight",
            "ContextMenu",
            "ExtendedTooltip",
            "Events",
        ]
    );
    let binding_order = |node: roxmltree::Node<'_, '_>| {
        node.children()
            .filter(|child| child.is_element())
            .map(|child| child.tag_name().name())
            .filter(|name| ["DataPath", "SkipOnInput", "TitleLocation"].contains(name))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(binding_order(field), binding_order(oracle_field));
    // Two 8.3.27.2074 exports preserve this dimension/automatic-bound
    // order; matching child values alone misses platform normalization.
    assert_eq!(
        child_names
            .iter()
            .copied()
            .filter(|name| ["Width", "AutoMaxWidth", "Height", "AutoMaxHeight"].contains(name))
            .collect::<Vec<_>>(),
        ["Width", "AutoMaxWidth", "Height", "AutoMaxHeight"]
    );
    for (tag, value) in [
        ("Width", "1"),
        ("Height", "2"),
        ("AutoMaxWidth", "false"),
        ("AutoMaxHeight", "false"),
        ("Visible", "false"),
        ("Enabled", "false"),
        ("ReadOnly", "true"),
    ] {
        assert_eq!(
            field
                .children()
                .find(|n| n.has_tag_name(tag))
                .unwrap()
                .text(),
            Some(value)
        );
    }
    assert!(field
        .descendants()
        .any(|n| n.has_tag_name("content") && n.text() == Some("Описание & детали")));
    let events: Vec<_> = field
        .descendants()
        .filter(|n| n.has_tag_name("Event"))
        .map(|n| (n.attribute("name").unwrap(), n.text().unwrap()))
        .collect();
    assert_eq!(
        events,
        [
            ("OnClick", "ПолеHTMLПриНажатии"),
            ("DocumentComplete", "ПолеHTMLДокументСформирован")
        ]
    );
    let ids: Vec<_> = field
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        3
    );
}

#[test]
fn html_document_field_rejects_invalid_events_bindings_and_properties() {
    for (key, value, expected) in [
        ("on", json!(["OnChange"]), "FORM_EVENT_NOT_ALLOWED"),
        ("on", json!(["Click"]), "FORM_EVENT_NOT_ALLOWED"),
        ("path", json!("Missing"), "missing top-level form attribute"),
        ("titleLocation", json!("invalid"), "titleLocation"),
        ("width", json!(-1), "width"),
        ("height", json!(4294967296_u64), "height"),
        ("skipOnInput", json!("true"), "boolean"),
        ("autoMaxWidth", json!(0), "boolean"),
        ("readOnly", json!("false"), "boolean"),
    ] {
        let mut element = json!({"html": "Html", "path": "Source"});
        element[key] = value;
        let definition =
            json!({"attributes": [{"name": "Source", "type": "string"}], "elements": [element]});
        let error = form_compile_xml(&definition, "2.20").unwrap_err();
        assert!(error.contains(expected), "{key}: {error}");
    }
}

#[test]
fn form_compile_uses_shared_element_event_matrix() {
    let valid = json!({
        "attributes": [{"name": "Rows", "type": "ValueTable"}],
        "elements": [{
            "table": "Rows",
            "path": "Rows",
            "on": ["Selection"],
            "columns": []
        }]
    });
    let (xml, _) = form_compile_xml(&valid, "2.20").unwrap();
    assert!(
        xml.contains("<Event name=\"Selection\">RowsВыборСтроки</Event>"),
        "{xml}"
    );

    for (definition, expected_code) in [
        (
            json!({
                "attributes": [{"name": "Rows", "type": "ValueTable"}],
                "elements": [{
                    "table": "Rows",
                    "path": "Rows",
                    "on": ["OnCreateAtServer"],
                    "columns": []
                }]
            }),
            "FORM_EVENT_NOT_ALLOWED",
        ),
        (
            json!({"elements": [{"button": "Run", "on": ["Click"]}]}),
            "FORM_EVENT_NOT_ALLOWED",
        ),
        (
            json!({
                "elements": [{
                    "input": "Name",
                    "on": [{
                        "event": "OnChange",
                        "handler": "NameOnChange",
                        "callType": "After"
                    }]
                }]
            }),
            "FORM_EVENT_CALL_TYPE_NOT_ALLOWED",
        ),
    ] {
        let Err(error) = form_compile_xml(&definition, "2.20") else {
            panic!("invalid element event must be rejected: {definition}");
        };
        assert!(error.contains(expected_code), "{error}");
    }
}

#[test]
fn form_compile_accepts_persistent_object_and_record_event_families() {
    for main_type in [
        "ChartOfAccountsObject.Main",
        "ChartOfCalculationTypesObject.Payroll",
        "AccumulationRegisterRecordSet.Stock",
        "AccountingRegisterRecordSet.Accounting",
        "CalculationRegisterRecordSet.Payroll",
    ] {
        let definition = json!({
            "attributes": [{"name": "Object", "type": main_type, "main": true}],
            "events": {"OnReadAtServer": "ObjectOnReadAtServer"}
        });

        let (xml, _) = form_compile_xml(&definition, "2.20")
            .unwrap_or_else(|error| panic!("{main_type}: {error}"));

        assert!(
            xml.contains(&format!("<v8:Type>cfg:{main_type}</v8:Type>")),
            "{main_type}: {xml}"
        );
        assert!(
            xml.contains("<Event name=\"OnReadAtServer\">ObjectOnReadAtServer</Event>"),
            "{main_type}: {xml}"
        );
    }
}

#[test]
fn form_compile_skill_tables_document_only_supported_dsl_keys() {
    const SKILL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/unica/skills/form-compile/SKILL.md"
    ));
    const ELEMENTS_START: &str = "### Элементы (ключ определяет тип)\n\n";
    const START: &str = "<!-- form-event-registry:start -->";
    const END: &str = "<!-- form-event-registry:end -->";
    let skill = SKILL.replace("\r\n", "\n");

    let element_table = skill
        .split_once(ELEMENTS_START)
        .and_then(|(_, tail)| tail.split_once("\n\n### ").map(|(table, _)| table))
        .expect("form-compile element table must remain present for contract checks");
    let documented_element_keys = element_table.lines().filter_map(|line| {
        line.strip_prefix("| `\"")
            .and_then(|line| line.split_once("\"`"))
            .map(|(key, _)| key)
    });

    for key in documented_element_keys {
        let element = Map::from_iter([(key.to_string(), json!("DocumentedElement"))]);
        assert!(
            FormEditElementDefinitionKind::from_object(&element).is_ok(),
            "form-compile element table documents unsupported DSL key `{key}`"
        );
    }

    let section = skill
        .split_once(START)
        .and_then(|(_, tail)| tail.split_once(END).map(|(section, _)| section))
        .expect("form-compile event table must be delimited for contract checks");
    let documented_keys = section.lines().filter_map(|line| {
        line.strip_prefix("| `")
            .and_then(|line| line.split_once("` | "))
            .map(|(key, _)| key)
    });

    for key in documented_keys {
        let element = Map::from_iter([(key.to_string(), json!("DocumentedElement"))]);
        assert!(
            FormEditElementDefinitionKind::from_object(&element).is_ok(),
            "form-compile event table documents unsupported DSL key `{key}`"
        );
    }
}

const DUMP_SHAPED_REFRESH_COMMAND: &str = concat!(
    "\t\t<Command name=\"СписокОбновить\" id=\"5\">\n",
    "\t\t\t<Title>\n",
    "\t\t\t\t<v8:item>\n",
    "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
    "\t\t\t\t\t<v8:content>Обновить</v8:content>\n",
    "\t\t\t\t</v8:item>\n",
    "\t\t\t</Title>\n",
    "\t\t\t<ToolTip>\n",
    "\t\t\t\t<v8:item>\n",
    "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
    "\t\t\t\t\t<v8:content>Обновить список</v8:content>\n",
    "\t\t\t\t</v8:item>\n",
    "\t\t\t</ToolTip>\n",
    "\t\t\t<Shortcut>F5</Shortcut>\n",
    "\t\t\t<Picture>\n",
    "\t\t\t\t<xr:Ref>StdPicture.Refresh</xr:Ref>\n",
    "\t\t\t\t<xr:LoadTransparent>true</xr:LoadTransparent>\n",
    "\t\t\t</Picture>\n",
    "\t\t\t<Action>СписокОбновить</Action>\n",
    "\t\t\t<Representation>Picture</Representation>\n",
    "\t\t</Command>",
);

/// A form edit takes the line endings of the form: a CRLF export stays CRLF,
/// an LF or mixed form keeps them as they were (#1996).
#[test]
fn form_edit_inserts_lines_with_the_form_line_ending() {
    let form = |newline: &str| {
        [
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" version=\"2.20\">",
            "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>",
            "</Form>",
        ]
        .join(newline)
    };
    let defn = json!({"attributes": [{"name": "Строка", "type": "string"}]});
    let path = std::path::Path::new("Catalogs/X/Forms/F/Ext/Form.xml");
    let edit = |mut text: String| {
        let start = text.find("<Form").unwrap();
        form_edit_apply_definition(&mut text, &defn, path, start, Some("ru")).unwrap();
        text
    };

    let crlf = edit(form("\r\n"));
    assert!(crlf.contains("<Attribute name=\"Строка\""), "{crlf}");
    assert!(!has_bare_lf(&crlf), "{crlf:?}");

    let lf = edit(form("\n"));
    assert!(lf.contains("<Attribute name=\"Строка\""), "{lf}");
    assert!(!lf.contains('\r'), "{lf:?}");

    // A lone CR makes the endings mixed too: the inserted LF stays.
    let lone_cr = form("\r\n").replacen("\r\n", "\r", 1);
    let edited = edit(lone_cr);
    assert!(has_bare_lf(&edited), "{edited:?}");

    let mixed = form("\r\n").replacen("\r\n", "\n", 1);
    let edited = edit(mixed.clone());
    assert!(edited.starts_with(&mixed[..mixed.find("<Form").unwrap()]));
    assert!(
        has_bare_lf(&edited) && edited.contains("\r\n"),
        "{edited:?}"
    );
}
