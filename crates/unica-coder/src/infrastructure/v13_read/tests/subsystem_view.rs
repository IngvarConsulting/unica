use super::*;

/// Видимость команды в интерфейсе — это не одно значение. Замер на боевой
/// конфигурации: 99 блоков видимости из 1050 несут переопределения по ролям,
/// то есть каждая одиннадцатая команда. Отдать `visible` за всю правду
/// значило бы читать девять процентов команд неверно.
/// rule: INV.SOURCE.COMMAND-VISIBILITY-ROLE-SIGNAL
#[test]
fn command_visibility_says_when_roles_override_it() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<CommandsVisibility>
		<Command name="Catalog.Items.Command.Refresh">
			<Visibility>
				<xr:Common>true</xr:Common>
				<xr:Value name="Role.Кладовщик">false</xr:Value>
				<xr:Value name="Role.ПолныеПрава">true</xr:Value>
			</Visibility>
		</Command>
		<Command name="CommonCommand.ОткрытьНастройки">
			<Visibility>
				<xr:Common>false</xr:Common>
			</Visibility>
		</Command>
	</CommandsVisibility>
</CommandInterface>
"#,
    );
    let service = fixture.view_service();

    let commands =
        service.view(ViewRequest::new("main:Subsystem.Sales.Interface.Command").unwrap());

    assert!(commands.ok, "{:?}", refusal_codes(&commands));
    let items = commands.data.as_ref().unwrap()["items"].as_array().unwrap();
    let props = |title: &str| {
        items
            .iter()
            .find(|item| item["title"] == title)
            .unwrap_or_else(|| panic!("{title} among {items:?}"))["props"]
            .clone()
    };
    let overridden = props("Catalog.Items.Command.Refresh");
    assert_eq!(overridden["visible"], true);
    assert_eq!(overridden["roleOverrides"], 2);
    // Ноль говорит «переопределений нет», а не «не смотрели»: отсутствие поля
    // читатель не отличил бы от несчитанного.
    let plain = props("CommonCommand.ОткрытьНастройки");
    assert_eq!(plain["visible"], false);
    assert_eq!(plain["roleOverrides"], 0);
}

/// Порядок команд, групп и дочерних подсистем — три из пяти секций
/// командного интерфейса. Две из них наружу не выходили вовсе, а писать то,
/// чего инструмент не показывает, нельзя: предпросмотр правки нечем
/// подтвердить, а забор ревизии по прочитанному над невидимым фактом
/// несостоятелен.
/// rule: INV.SOURCE.COMMAND-INTERFACE-ORDERS
#[test]
fn the_command_interface_shows_every_order_it_holds() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<CommandsOrder>
		<Command name="Catalog.Items.StandardCommand.Zeta">
			<CommandGroup>NavigationPanelImportant</CommandGroup>
		</Command>
		<Command name="Catalog.Items.StandardCommand.OpenList">
			<CommandGroup>NavigationPanelImportant</CommandGroup>
		</Command>
	</CommandsOrder>
	<SubsystemsOrder>
		<Subsystem>Subsystem.Zeta</Subsystem>
		<Subsystem>Subsystem.Sales</Subsystem>
	</SubsystemsOrder>
	<GroupsOrder>
		<Group>NavigationPanelSeeAlso</Group>
		<Group>NavigationPanelImportant</Group>
	</GroupsOrder>
</CommandInterface>
"#,
    );
    let service = fixture.view_service();
    let at = "main:Subsystem.Sales.Interface";

    let node = service.view(ViewRequest::new(at).unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    let branches = node.data.as_ref().unwrap()["branches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let branch = |kind: &str| {
        branches
            .iter()
            .find(|branch| branch["at"] == format!("{at}.{kind}"))
            .cloned()
    };
    assert_eq!(branch("Group").unwrap()["count"], 2, "{branches:?}");
    assert_eq!(branch("Subsystem").unwrap()["count"], 2, "{branches:?}");

    // Группы идут в объявленном порядке, и пустая группа остаётся видимой:
    // порядок объявляется отдельно от наполнения.
    let groups = service.view(ViewRequest::new(&format!("{at}.Group")).unwrap());
    assert!(groups.ok, "{:?}", refusal_codes(&groups));
    let items = groups.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(items[0]["title"], "NavigationPanelSeeAlso");
    assert_eq!(items[0]["commands"], 0);
    assert_eq!(items[1]["title"], "NavigationPanelImportant");
    assert_eq!(items[1]["commands"], 2);

    // Внутри группы — её команды в порядке `CommandsOrder`.
    let one =
        service.view(ViewRequest::new(&format!("{at}.Group.NavigationPanelImportant")).unwrap());
    assert!(one.ok, "{:?}", refusal_codes(&one));
    let ordered = one.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(
        ordered,
        &vec![
            json!({"order": 1, "command": "Catalog.Items.StandardCommand.Zeta"}),
            json!({"order": 2, "command": "Catalog.Items.StandardCommand.OpenList"}),
        ]
    );

    // Ссылка платформы дополняется до адреса, по которому можно спуститься.
    let children = service.view(ViewRequest::new(&format!("{at}.Subsystem")).unwrap());
    assert!(children.ok, "{:?}", refusal_codes(&children));
    assert_eq!(
        children.data.as_ref().unwrap()["items"],
        json!([
            {"order": 1, "at": "main:Subsystem.Zeta"},
            {"order": 2, "at": "main:Subsystem.Sales"},
        ])
    );

    // Группы, которой в порядке нет, не существует и для чтения.
    let missing = service.view(ViewRequest::new(&format!("{at}.Group.Нет")).unwrap());
    assert!(!missing.ok);
    assert_eq!(missing.diagnostics[0]["code"], "not_found");
}

/// rule: INV.APP.SUPPORT-STATE
#[test]
fn canonical_subsystem_view_preserves_support_for_own_and_nested_descriptors() {
    let parent_uuid = "11111111-1111-4111-8111-111111111111";
    let child_uuid = "22222222-2222-4222-8222-222222222222";
    let support =
        |state, direct_edit_safe| json!({"support": state, "directEditSafe": direct_edit_safe});
    for (case, marker, parent, child) in [
        (
            "different object rules",
            Some(format!("{{6,0,1,0,0,{parent_uuid},1,0,{child_uuid}}}")),
            support("locked", Some(false)),
            support("editableWithSupport", Some(true)),
        ),
        (
            "removed object",
            Some(format!("{{6,0,1,2,0,{parent_uuid},0,0,{child_uuid}}}")),
            support("removedFromSupport", Some(true)),
            support("locked", Some(false)),
        ),
        (
            "configuration read only",
            Some(format!("{{6,1,1,0,0,{parent_uuid},1,0,{child_uuid}}}")),
            support("configurationReadOnly", Some(false)),
            support("configurationReadOnly", Some(false)),
        ),
        (
            "configuration removed from support",
            Some(String::new()),
            support("removedFromSupport", Some(true)),
            support("removedFromSupport", Some(true)),
        ),
        (
            "marker absent",
            None,
            support("notSupported", None),
            support("notSupported", None),
        ),
        (
            "UUID absent from marker",
            Some("{6,0,1,0,0,33333333-3333-4333-8333-333333333333}".to_string()),
            support("notSupported", None),
            support("notSupported", None),
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install_supported_subsystem_descriptors(&fixture);
        if let Some(marker) = marker {
            write(
                &fixture.source.join("Ext/ParentConfigurations.bin"),
                &marker,
            );
        }
        let service = fixture.view_service();
        for (at, expected) in [
            ("main:Subsystem.Sales", &parent),
            ("main:Subsystem.Sales.Subsystem.Orders", &child),
            ("main:Subsystem.Sales", &parent),
        ] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "{case} {at}: view must succeed");
            let props = &result.data.as_ref().unwrap()["props"];
            assert_eq!(props["support"], expected["support"], "{case} {at}");
            assert_eq!(
                props["directEditSafe"], expected["directEditSafe"],
                "{case} {at}"
            );
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains(fixture.source.to_str().unwrap()));
        }
    }
}

/// rule: INV.APP.SUPPORT-STATE
#[test]
fn canonical_view_distinguishes_absent_support_from_unreadable_and_invalid_markers() {
    for marker_kind in ["absent", "invalid", "directory"] {
        let fixture = RealReaderFixture::new();
        install_supported_subsystem_descriptors(&fixture);
        let marker = fixture.source.join("Ext/ParentConfigurations.bin");
        match marker_kind {
            "invalid" => write(&marker, "{9,0,1}, invalid support header"),
            "directory" => fs::create_dir_all(&marker).unwrap(),
            _ => {}
        }
        let service = fixture.view_service();
        for at in [
            "main:Catalog.Items",
            "main:Subsystem.Sales",
            "main:Subsystem.Sales.Subsystem.Orders",
        ] {
            let result = service.view(ViewRequest::new(at).unwrap());
            if marker_kind == "absent" {
                assert!(
                    result.ok,
                    "{at}: view must succeed without a support marker"
                );
                // Без маркера поддержки у любого объекта одно состояние.
                let expected = json!("notSupported");
                assert_eq!(
                    result.data.as_ref().unwrap()["props"]["support"],
                    expected,
                    "{at}"
                );
                let props = &result.data.as_ref().unwrap()["props"];
                if at == "main:Catalog.Items" {
                    assert!(props.get("directEditSafe").is_none());
                } else {
                    assert_eq!(props["directEditSafe"], serde_json::Value::Null);
                    assert!(props.as_object().unwrap().contains_key("directEditSafe"));
                }
            } else {
                assert!(!result.ok, "{marker_kind} {at}: view must refuse");
                assert!(
                    result.data.is_none(),
                    "{marker_kind} {at}: data must be absent"
                );
                assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
                assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
                assert_eq!(result.diagnostics[0]["outcome"], "fixSource");
            }
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains(fixture.source.to_str().unwrap()));
        }
        // Коллекция обычной конфигурации читает корень вместе с его
        // поддержкой: без маркера её члены не несут состояния, а нечитаемый
        // маркер не выдаёт их свободными и отказывает, как карточка.
        let collection = service.view(ViewRequest::new("main:Catalog").unwrap());
        if marker_kind == "absent" {
            let items = view_items(&collection);
            assert!(!items.is_empty(), "collection lists its members");
            for item in &items {
                assert!(
                    item["props"].get("support").is_none(),
                    "{}: notSupported is not published",
                    item["at"]
                );
            }
            assert!(collection.data.as_ref().unwrap().get("limits").is_none());
        } else {
            assert!(!collection.ok, "{marker_kind}: collection must refuse");
            assert!(
                collection.data.is_none(),
                "{marker_kind}: data must be absent"
            );
            assert_eq!(collection.diagnostics[0]["detailCode"], "source_unreadable");
        }
        assert!(!serde_json::to_string(&collection)
            .unwrap()
            .contains(fixture.source.to_str().unwrap()));
    }
}

/// Состав подсистемы и её дочерние подсистемы — то, ради чего подсистему
/// читают. Состав — ссылки на чужие объекты, поэтому он живёт в ветви
/// `Relation`; дочерняя подсистема — потомок с собственным адресом, и
/// читается по нему на любой глубине регистрации.
/// rule: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
#[test]
fn a_subsystem_shows_its_content_and_reads_registered_children_at_depth() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &subsystem_descriptor("Sales", &["Catalog.Items", "Document.Order"], &["Orders"]),
    );
    // Ссылка по идентификатору — так платформа пишет указатель на
    // исчезнувший объект. Она не роняет чтение и не прячется: её называет
    // слот `limits`, а счёт ветви равен тому, что покажет страница.
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
        &subsystem_descriptor(
            "Orders",
            &["Document.Order", "0:78b4152e-0000-0000-0000-000000000000"],
            &["Returns"],
        ),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders/Subsystems/Returns.xml"),
        &subsystem_descriptor("Returns", &["Catalog.Items"], &[]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<SubsystemsOrder>
		<Subsystem>Subsystem.Sales.Subsystem.Orders.Subsystem.Returns</Subsystem>
	</SubsystemsOrder>
</CommandInterface>
"#,
    );
    // Файл на диске, который родитель не регистрирует, подсистемой не является.
    write(
        &fixture.source.join("Subsystems/Sales/Subsystems/Ghost.xml"),
        &subsystem_descriptor("Ghost", &[], &[]),
    );
    let service = fixture.view_service();

    let root = service.view(ViewRequest::new("main:Subsystem.Sales").unwrap());
    assert_eq!(
        view_branch_count(&root, "main:Subsystem.Sales.Relation"),
        Some(2)
    );
    assert_eq!(
        view_branch_count(&root, "main:Subsystem.Sales.Subsystem"),
        Some(1)
    );
    assert_eq!(
        view_items(&service.view(ViewRequest::new("main:Subsystem.Sales.Relation").unwrap())),
        vec![
            json!({"relation": "content", "at": "main:Catalog.Items", "kind": "Catalog"}),
            json!({"relation": "content", "at": "main:Document.Order", "kind": "Document"}),
        ]
    );
    let children =
        view_items(&service.view(ViewRequest::new("main:Subsystem.Sales.Subsystem").unwrap()));
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["at"], "main:Subsystem.Sales.Subsystem.Orders");
    assert_eq!(children[0]["kind"], "Subsystem");

    let nested_at = "main:Subsystem.Sales.Subsystem.Orders";
    let nested = service.view(ViewRequest::new(nested_at).unwrap());
    assert!(nested.ok, "{:?}", refusal_codes(&nested));
    let data = nested.data.as_ref().unwrap();
    assert_eq!(data["at"], nested_at);
    assert_eq!(data["kind"], "Subsystem");
    assert_eq!(data["title"], "Orders");
    assert_eq!(
        view_branch_count(&nested, &format!("{nested_at}.Relation")),
        Some(1)
    );
    assert_eq!(
        view_branch_count(&nested, &format!("{nested_at}.Subsystem")),
        Some(1)
    );
    assert_eq!(
        view_branch_count(&nested, &format!("{nested_at}.Interface")),
        Some(1)
    );
    let content = service.view(ViewRequest::new(&format!("{nested_at}.Relation")).unwrap());
    assert_eq!(
        view_items(&content),
        vec![json!({"relation": "content", "at": "main:Document.Order", "kind": "Document"})]
    );
    let limits = content.data.as_ref().unwrap()["limits"].to_string();
    assert!(
        limits.contains("0:78b4152e-0000-0000-0000-000000000000"),
        "content limits must name the dangling reference"
    );

    // Интерфейс вложенной подсистемы читается из её собственного файла, а
    // не из файла корня.
    let order = view_items(
        &service.view(ViewRequest::new(&format!("{nested_at}.Interface.Subsystem")).unwrap()),
    );
    assert_eq!(
        order,
        vec![json!({"order": 1, "at": "main:Subsystem.Sales.Subsystem.Orders.Subsystem.Returns"})]
    );
    let deepest = service.view(ViewRequest::new(order[0]["at"].as_str().unwrap()).unwrap());
    assert!(deepest.ok, "{:?}", refusal_codes(&deepest));
    assert_eq!(deepest.data.as_ref().unwrap()["title"], "Returns");

    // Родитель, прочитанный после потомка, не получает его нагрузку из кэша.
    let again = service.view(ViewRequest::new("main:Subsystem.Sales").unwrap());
    assert_eq!(
        again.data.as_ref().unwrap()["title"],
        root.data.as_ref().unwrap()["title"]
    );
    assert_eq!(
        view_branch_count(&again, "main:Subsystem.Sales.Relation"),
        Some(2)
    );

    // Страница состава режется общим курсором.
    let first = service.view(
        ViewRequest::new("main:Subsystem.Sales.Relation")
            .unwrap()
            .with_limit(1)
            .unwrap(),
    );
    assert_eq!(view_items(&first).len(), 1);
    assert!(
        first.cursor.is_some(),
        "a one-item page must leave a cursor"
    );

    for missing in [
        "main:Subsystem.Sales.Subsystem.Ghost",
        "main:Subsystem.Sales.Subsystem.Нет",
        "main:Subsystem.Sales.Relation.Catalog",
    ] {
        let result = service.view(ViewRequest::new(missing).unwrap());
        assert!(!result.ok, "{missing} must be refused");
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{missing}");
    }
}

/// Спуск идёт по регистрации, и каждый дескриптор цепочки должен называть
/// ту подсистему, по имени которой к нему пришли. Файл с чужим именем на её
/// месте не становится ни ею, ни дорогой к её детям.
#[test]
fn a_nested_subsystem_read_rejects_a_descriptor_that_names_another_subsystem() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &subsystem_descriptor("Sales", &[], &["Orders"]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
        &subsystem_descriptor("Impostor", &[], &["Returns"]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders/Subsystems/Returns.xml"),
        &subsystem_descriptor("Returns", &[], &[]),
    );
    let service = fixture.view_service();

    for at in [
        "main:Subsystem.Sales.Subsystem.Orders",
        "main:Subsystem.Sales.Subsystem.Orders.Subsystem.Returns",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "{at} must be refused");
        assert_ne!(result.diagnostics[0]["code"], "not_found", "{at}");
    }
}

#[test]
fn subsystem_text_properties_preserve_presence_and_damage_at_every_depth() {
    for (fragment, comment, explanation, damaged) in [
        ("<Comment/><Explanation/>", Some(""), Some(""), false),
        ("<Comment></Comment><Explanation></Explanation>", Some(""), Some(""), false),
        ("<Comment>  </Comment><Explanation>  </Explanation>", Some("  "), Some(""), false),
        ("<Comment>а<![CDATA[б]]>в</Comment><Explanation><v8:item><v8:lang>en</v8:lang><v8:content>English</v8:content></v8:item><v8:item><v8:lang>ru</v8:lang><v8:content>Пояснение</v8:content></v8:item></Explanation>", Some("абв"), Some("Пояснение"), false),
        ("", None, None, false),
        ("<Comment><Bad/></Comment><Explanation>garbage</Explanation>", None, None, true),
        ("<Comment attr=\"x\"/><Explanation attr=\"x\"/>", None, None, true),
        ("<Comment/><Comment/><Explanation/><Explanation/>", None, None, true),
        ("<Comment xmlns=\"urn:foreign\"/><Explanation xmlns=\"urn:foreign\"/>", None, None, true),
        ("<Comment><Bad/></Comment><Explanation><v8:item><v8:lang>ru</v8:lang></v8:item></Explanation>", None, None, true),
        ("<Comment/><Explanation><item xmlns=\"urn:foreign\"><lang>ru</lang><content>x</content></item></Explanation>", Some(""), None, true),
    ] {
        let fixture = RealReaderFixture::new();
        let paths = ["Subsystems/Sales.xml", "Subsystems/Sales/Subsystems/Orders.xml"];
        for (path, name, child) in [(paths[0], "Sales", "Orders"), (paths[1], "Orders", "Returns")] {
            let xml = subsystem_descriptor(name, &["Catalog.Items"], &[child])
                .replace("<Synonym/>", &format!("<Synonym/>{fragment}"));
            write(&fixture.source.join(path), &xml);
            let ci = fixture.source.join(path.trim_end_matches(".xml")).join("Ext/CommandInterface.xml");
            write(&ci, r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#);
        }
        let service = fixture.view_service();
        for (path, at) in paths.into_iter().zip(["main:Subsystem.Sales", "main:Subsystem.Sales.Subsystem.Orders"]) {
            let before = fs::read(fixture.source.join(path)).unwrap();
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "{fragment}: the call failed");
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["props"]["Comment"], json!(comment), "{at}: {fragment}");
            assert_eq!(data["props"]["Explanation"], json!(explanation), "{at}: {fragment}");
            let limits = data["limits"].to_string();
            assert_eq!(limits.contains("properties."), damaged, "{at}: {fragment}: unexpected answer");
            for branch in ["Relation", "Subsystem", "Interface"] {
                assert_eq!(view_branch_count(&result, &format!("{at}.{branch}")), Some(1), "{at}: {fragment}");
                assert!(service.view(ViewRequest::new(&format!("{at}.{branch}")).unwrap()).ok);
            }
            assert_eq!(fs::read(fixture.source.join(path)).unwrap(), before);
        }
    }
}
