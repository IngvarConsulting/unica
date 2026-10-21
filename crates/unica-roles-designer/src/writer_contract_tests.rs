use crate::descriptor::validate_role_edit_descriptor;
use crate::test_support::{operation, rights_xml, validate_every_role_value};
use crate::writer::*;
use roxmltree::Document;
use unica_roles::*;
#[test]
fn rights_profile_requires_registered_root_and_unprefixed_rights_xsi_type() {
    let valid = rights_xml("\n", "<value>true</value>");
    validate_every_role_value(&valid).unwrap();

    let alias = valid
        .replace("xmlns:xsi=", "xmlns:schemaInstance=")
        .replace("xsi:type=", "schemaInstance:type=");
    validate_every_role_value(&alias).unwrap();

    for invalid in [
        valid.replace(" xsi:type=\"Rights\"", ""),
        valid.replace("xsi:type=\"Rights\"", "xsi:type=\"Role\""),
        valid.replace(
            "xsi:type=\"Rights\"",
            "xmlns:roles=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"roles:Rights\"",
        ),
        valid
            .replace(
                "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\"",
                "<PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\"",
            )
            .replace("</Rights>", "</PredefinedData>"),
    ] {
        assert!(
            validate_every_role_value(&invalid).is_err(),
            "accepted incompatible root contract: {invalid}"
        );
    }
}
#[test]
fn role_identity_matching_uses_complete_xml_string_values() {
    let body = rights_xml("\n", "<value>true</value>");
    let misleading_object = body.replacen(
        "<name>Catalog.Demo</name>",
        "<name>Catalog.Demo<!--future-->Suffix</name>",
        1,
    );
    let error = apply_role_edit_operation(
        &misleading_object,
        &operation("Catalog.Demo", "Delete", true),
        0,
    )
    .expect_err("a prefix text node must not identify the role object");
    assert!(error.contains("was not found"), "{error}");
    validate_every_role_value(&misleading_object).unwrap();

    let misleading_right = body.replacen(
        "\t\t\t<name>Delete</name>",
        "\t\t\t<name>Delete<!--future-->Suffix</name>",
        1,
    );
    let (updated, effect) = apply_role_edit_operation(
        &misleading_right,
        &operation("Catalog.Demo", "Delete", true),
        0,
    )
    .unwrap();
    assert_eq!(effect.before, None);
    assert!(effect.changed);
    assert!(updated.contains("<name>Delete<!--future-->Suffix</name>"));
    let document = Document::parse(&updated).unwrap();
    let object = direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)[0];
    let right_names = direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
        .into_iter()
        .map(|right| {
            role_text_content(direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)[0])
        })
        .collect::<Vec<_>>();
    assert!(right_names.iter().any(|name| name == "DeleteSuffix"));
    assert!(right_names.iter().any(|name| name == "Delete"));
    validate_every_role_value(&updated).unwrap();

    let descriptor = format!(
        r#"<MetaDataObject xmlns="{ROLE_METADATA_NAMESPACE}" version="2.20"><Role><Properties><Name>Demo<!--future-->Suffix</Name></Properties></Role></MetaDataObject>"#
    );
    assert!(validate_role_edit_descriptor(descriptor.as_bytes(), "Demo").is_err());
    validate_role_edit_descriptor(descriptor.as_bytes(), "DemoSuffix").unwrap();
    let encoded_version = descriptor.replace("version=\"2.20\"", "version=\"2.2&#48;\"");
    assert!(validate_role_edit_descriptor(encoded_version.as_bytes(), "DemoSuffix").is_err());

    let split_boolean = body.replacen("<value>true</value>", "<value>tr<!--future-->ue</value>", 1);
    validate_role_rights_document(&split_boolean, RoleValueScope::SourceImage).unwrap();
    assert!(validate_every_role_value(&split_boolean).is_err());
    let error = apply_role_edit_operation(
        &split_boolean,
        &operation("Catalog.Demo", "Delete", true),
        0,
    )
    .expect_err("a split boolean must not have a distinct no-op interpretation");
    assert!(error.contains("one direct boolean text value"), "{error}");
}
#[test]
fn right_insertion_rejects_mixed_eol_and_preserves_uniform_lone_cr() {
    let mixed = rights_xml("\n", "<value>true</value>").replacen('\n', "\r\n", 1);
    let mixed_document = Document::parse(&mixed).unwrap();
    let mixed_object = direct_role_children(
        mixed_document.root_element(),
        "object",
        ROLE_RIGHTS_NAMESPACE,
    )[0];
    let error = right_insertion(&mixed, mixed_object, "Edit", true)
        .expect_err("mixed EOL must not choose a global fallback");
    assert!(error.contains("ambiguous"), "{error}");

    let lone_cr = rights_xml("\r", "<value>true</value>");
    let lone_cr_document = Document::parse(&lone_cr).unwrap();
    let lone_cr_object = direct_role_children(
        lone_cr_document.root_element(),
        "object",
        ROLE_RIGHTS_NAMESPACE,
    )[0];
    let (_, insertion) = right_insertion(&lone_cr, lone_cr_object, "Edit", true).unwrap();
    assert!(insertion.contains('\r'));
    assert!(!insertion.contains('\n'));
}
#[test]
fn range_writer_uses_direct_children_and_preserves_unknown_xml_bom_and_eol() {
    for (self_closing, expected_value) in [
        ("<value/>", "<value>true</value>"),
        ("<value />", "<value>true</value>"),
        ("<value></value>", "<value>true</value>"),
        (
            "<r:value future=\"x\"/>",
            "<r:value future=\"x\">true</r:value>",
        ),
    ] {
        let body = rights_xml("\r\n", self_closing);
        let (updated, effect) =
            apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", true), 0)
                .unwrap();
        assert_eq!(effect.before, None);
        assert!(effect.changed);
        assert!(updated.contains(expected_value), "{updated}");
        assert!(updated.contains("<future><name>Delete</name><value>false</value></future>"));
        assert!(updated.contains("restrictionByCondition"));
        assert!(updated.contains("FuturePlatformRight"));
        assert!(updated.contains("restrictionTemplate"));
        assert!(!updated.replace("\r\n", "").contains('\n'));
        validate_every_role_value(&updated).unwrap();

        let encoded = encode_role_xml(true, &updated);
        assert!(encoded.starts_with(&[0xEF, 0xBB, 0xBF]));
    }
}
#[test]
fn range_writer_replaces_only_boolean_text_and_preserves_value_markup() {
    let body = rights_xml(
        "\n",
        r#"<r:value future=">">false<!--future-value-marker--></r:value>"#,
    );

    let (updated, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", true), 0).unwrap();

    assert!(effect.changed);
    assert!(updated.contains(r#"<r:value future=">">true<!--future-value-marker--></r:value>"#));
    validate_every_role_value(&updated).unwrap();
}
#[test]
fn writer_ignores_and_preserves_unnamed_forward_compatible_blocks() {
    let unnamed_right = "<right future=\"unnamed-right\"><value>true</value><future/></right>";
    let unnamed_object =
        "<object future=\"unnamed-object\"><right><value>false</value></right></object>";
    let body = rights_xml("\n", "<value>true</value>")
        .replacen(
            "\t\t<right>",
            &format!("\t\t{unnamed_right}\n\t\t<right>"),
            1,
        )
        .replace("\n</Rights>", &format!("\n\t{unnamed_object}\n</Rights>"));

    let (updated, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0).unwrap();

    assert!(effect.changed);
    assert!(updated.contains(unnamed_right));
    assert!(updated.contains(unnamed_object));
    validate_every_role_value(&updated).unwrap();
}
#[test]
fn writer_inserts_with_existing_eol_and_data_processor_false_removes_whole_object() {
    let body = rights_xml("\r\n", "<value>true</value>");
    let (inserted, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "View", true), 0).unwrap();
    assert!(effect.changed);
    assert_eq!(effect.before, None);
    assert!(inserted.contains("<name>View</name>\r\n"));
    assert!(!inserted.replace("\r\n", "").contains('\n'));

    let (removed, effect) = apply_role_edit_operation(
        &inserted,
        &operation("DataProcessor.Worker", "Use", false),
        1,
    )
    .unwrap();
    assert_eq!(effect.action, RoleEditEffectAction::RemoveObject);
    assert_eq!(effect.before, Some(true));
    assert!(!removed.contains("DataProcessor.Worker"));
    assert_eq!(removed.matches("<name>View</name>").count(), 1);
    assert!(removed.contains("Catalog.Demo"));
    assert!(removed.contains("restrictionTemplate"));
}
#[test]
fn data_processor_removal_preserves_eol_when_object_follows_an_inline_sibling() {
    let body = rights_xml("\n", "<value>true</value>").replace(
        "\t</object>\n\t<object>\n\t\t<name>DataProcessor.Worker</name>",
        "\t</object><object>\n\t\t<name>DataProcessor.Worker</name>",
    );
    assert!(body.contains("\t</object><object>\n"), "{body}");

    let (removed, effect) =
        apply_role_edit_operation(&body, &operation("DataProcessor.Worker", "Use", false), 0)
            .unwrap();

    assert_eq!(effect.action, RoleEditEffectAction::RemoveObject);
    assert_eq!(effect.before, Some(true));
    assert!(!removed.contains("DataProcessor.Worker"));
    assert!(
        removed.contains("\t</object>\n\t<restrictionTemplate>"),
        "{removed}"
    );
    validate_every_role_value(&removed).unwrap();
}
#[test]
fn missing_right_inherits_prefix_only_roles_qnames() {
    let body = concat!(
        "<r:Rights xmlns:r=\"http://v8.1c.ru/8.2/roles\" ",
        "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
        "xsi:type=\"Rights\" version=\"2.20\">\n",
        "\t<r:setForNewObjects>false</r:setForNewObjects>\n",
        "\t<r:setForAttributesByDefault>true</r:setForAttributesByDefault>\n",
        "\t<r:independentRightsOfChildObjects>false</r:independentRightsOfChildObjects>\n",
        "\t<r:object>\n",
        "\t\t<r:name>Catalog.Demo</r:name>\n",
        "\t\t<r:right><r:name>Delete</r:name><r:value>true</r:value></r:right>\n",
        "\t</r:object>\n",
        "</r:Rights>"
    );
    let (updated, effect) =
        apply_role_edit_operation(body, &operation("Catalog.Demo", "View", true), 0).unwrap();
    assert!(effect.changed);
    assert!(updated.contains("<r:right>"), "{updated}");
    assert!(updated.contains("<r:name>View</r:name>"), "{updated}");
    assert!(updated.contains("<r:value>true</r:value>"), "{updated}");
    assert!(!updated.contains("<right>"), "{updated}");
    validate_every_role_value(&updated).unwrap();
}
#[test]
fn missing_right_does_not_copy_a_sibling_local_namespace_prefix() {
    let local_right = concat!(
        "<x:right xmlns:x=\"http://v8.1c.ru/8.2/roles\" future=\"keep\">",
        "<x:name>Delete</x:name><x:value>true</x:value><x:future/>",
        "</x:right>"
    );
    let body = concat!(
        "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" ",
        "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
        "xsi:type=\"Rights\" version=\"2.20\">",
        "<setForNewObjects>false</setForNewObjects>",
        "<setForAttributesByDefault>true</setForAttributesByDefault>",
        "<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
        "<object><name>Catalog.Demo</name>",
        "<x:right xmlns:x=\"http://v8.1c.ru/8.2/roles\" future=\"keep\">",
        "<x:name>Delete</x:name><x:value>true</x:value><x:future/>",
        "</x:right></object></Rights>"
    );

    let (updated, effect) =
        apply_role_edit_operation(body, &operation("Catalog.Demo", "View", true), 0).unwrap();

    assert!(effect.changed);
    assert!(updated.contains(local_right));
    assert!(updated.contains("<right><name>View</name><value>true</value></right>"));
    assert_eq!(updated.matches("xmlns:x=").count(), 1);
    validate_every_role_value(&updated).unwrap();
}
#[test]
fn duplicate_objects_and_rights_are_refused_before_any_mutation() {
    // Шесть веток отказа структурной проверки: неоднозначный документ
    // нельзя ни читать как истину, ни править «первым попавшимся».
    let base = rights_xml("\n", "<value>true</value>");
    for (label, body, expected) in [
        (
            "duplicate object",
            base.replacen(
                "\t<object>\n\t\t<name>DataProcessor.Worker</name>",
                "\t<object>\n\t\t<name>Catalog.Demo</name>\n\t\t<right><name>Read</name><value>true</value></right>\n\t</object>\n\t<object>\n\t\t<name>DataProcessor.Worker</name>",
                1,
            ),
            "duplicate role object",
        ),
        (
            "duplicate object name",
            base.replacen(
                "\t\t<name>Catalog.Demo</name>",
                "\t\t<name>Catalog.Demo</name>\n\t\t<name>Catalog.Other</name>",
                1,
            ),
            "duplicate direct names",
        ),
        (
            "duplicate right",
            base.replacen(
                "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
                "\t\t<right><name>Delete</name><value>false</value></right>",
                1,
            ),
            "duplicate right `Delete`",
        ),
        (
            "duplicate right name",
            base.replacen(
                "\t\t\t<name>Delete</name>",
                "\t\t\t<name>Delete</name>\n\t\t\t<name>Insert</name>",
                1,
            ),
            "must not have duplicate direct names",
        ),
        (
            "two direct values",
            base.replacen(
                "\t\t\t<value>true</value>",
                "\t\t\t<value>true</value>\n\t\t\t<value>false</value>",
                1,
            ),
            "must have one direct value",
        ),
    ] {
        let error = validate_role_rights_document(&body, RoleValueScope::SourceImage)
            .expect_err(label);
        assert!(error.contains(expected), "{label}: {error}");
    }

    // Неоднозначность до мутации ловит и сам writer.
    let ambiguous_object = base.replacen(
        "\t<object>\n\t\t<name>DataProcessor.Worker</name>",
        "\t<object>\n\t\t<name>Catalog.Demo</name>\n\t\t<right><name>Read</name><value>true</value></right>\n\t</object>\n\t<object>\n\t\t<name>DataProcessor.Worker</name>",
        1,
    );
    let error = apply_role_edit_operation(
        &ambiguous_object,
        &operation("Catalog.Demo", "Read", false),
        0,
    )
    .expect_err("ambiguous object must not be resolved by picking the first");
    assert!(error.contains("is ambiguous"), "{error}");

    let ambiguous_right = base.replacen(
        "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
        "\t\t<right><name>Delete</name><value>false</value></right>",
        1,
    );
    let error = apply_role_edit_operation(
        &ambiguous_right,
        &operation("Catalog.Demo", "Delete", false),
        0,
    )
    .expect_err("ambiguous right must not be resolved by picking the first");
    assert!(error.contains("is ambiguous"), "{error}");
}
#[test]
fn cdata_carries_identity_and_value_like_ordinary_text() {
    // CDATA — та же строка инфосета: адресация и чтение значения не должны
    // от неё зависеть.
    let body = rights_xml("\n", "<value><![CDATA[true]]></value>").replacen(
        "<name>Catalog.Demo</name>",
        "<name><![CDATA[Catalog.Demo]]></name>",
        1,
    );

    let (updated, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0).unwrap();

    assert_eq!(effect.before, Some(true), "CDATA-значение прочитано");
    assert!(effect.changed);
    assert!(
        updated.contains("<![CDATA[Catalog.Demo]]>"),
        "имя объекта сохранено как есть: {updated}"
    );
    validate_every_role_value(&updated).unwrap();
}
#[test]
fn an_untouched_non_boolean_neighbour_does_not_block_the_requested_edit() {
    // Небулевое значение соседнего права принадлежит исходнику, а не
    // писателю: раньше строгая пост-проверка отвергала весь вызов чужим
    // правом и без `operationIndex`.
    let body = rights_xml("\n", "<value>true</value>").replacen(
        "<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
        "<right><name>FuturePlatformRight</name><value/><future/></right>",
        1,
    );
    assert!(body.contains("<value/>"), "{body}");

    let (updated, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0).unwrap();

    assert!(effect.changed);
    assert!(updated.contains("<value/>"), "сосед сохранён как есть");
    let written = [("Catalog.Demo".to_string(), "Delete".to_string())];
    validate_role_rights_document(&updated, RoleValueScope::WrittenRights(&written)).unwrap();
    // А если бы писатель тронул именно это право, строгость осталась бы.
    let neighbour = [(
        "Catalog.Demo".to_string(),
        "FuturePlatformRight".to_string(),
    )];
    assert!(
        validate_role_rights_document(&updated, RoleValueScope::WrittenRights(&neighbour)).is_err()
    );
}
#[test]
fn default_valued_right_is_stored_as_absence_like_the_platform_export() {
    // ibcmd 8.3.27.2074 хранит объектное право только при значении,
    // отличном от `setForNewObjects`; переход к умолчанию — удаление
    // элемента вместе с его RLS, а не явный `false`.
    let body = rights_xml("\n", "<value>true</value>");
    let (updated, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 0).unwrap();
    assert!(effect.changed);
    assert_eq!(effect.before, Some(true));
    assert!(!effect.after);
    assert_eq!(effect.action, RoleEditEffectAction::SetRight);
    let document = Document::parse(&updated).unwrap();
    let object = direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)[0];
    assert!(
        !direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
            .into_iter()
            .any(|right| {
                role_text_content(direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)[0])
                    == "Delete"
            }),
        "{updated}"
    );
    drop(document);
    assert!(!updated.contains("restrictionByCondition"), "{updated}");
    // Блок не опустел (остался FuturePlatformRight и неизвестный child) —
    // он сохраняется вместе с ними.
    assert!(updated.contains("<name>Catalog.Demo</name>"), "{updated}");
    assert!(updated.contains("FuturePlatformRight"), "{updated}");
    assert!(
        updated.contains("<future><name>Delete</name><value>false</value></future>"),
        "{updated}"
    );
    assert!(updated.contains("restrictionTemplate"), "{updated}");
    validate_every_role_value(&updated).unwrap();

    // Повтор — no-op: право уже несёт умолчание отсутствием.
    let (repeated, second) =
        apply_role_edit_operation(&updated, &operation("Catalog.Demo", "Delete", false), 1)
            .unwrap();
    assert_eq!(repeated, updated);
    assert!(!second.changed);
    assert_eq!(second.before, None);
}
#[test]
fn removing_the_last_stored_right_drops_the_emptied_object_block() {
    let body = [
        "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Rights\" version=\"2.20\">",
        "\t<setForNewObjects>false</setForNewObjects>",
        "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
        "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
        "\t<object>",
        "\t\t<name>Catalog.Solo</name>",
        "\t\t<right>",
        "\t\t\t<name>Read</name>",
        "\t\t\t<value>true</value>",
        "\t\t</right>",
        "\t</object>",
        "\t<object>",
        "\t\t<name>Catalog.Kept</name>",
        "\t\t<right><name>Read</name><value>true</value></right>",
        "\t</object>",
        "</Rights>",
    ]
    .join("\n");

    let (updated, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Solo", "Read", false), 0).unwrap();
    assert!(effect.changed);
    assert_eq!(effect.action, RoleEditEffectAction::SetRight);
    assert!(!updated.contains("Catalog.Solo"), "{updated}");
    assert!(updated.contains("Catalog.Kept"), "{updated}");
    validate_every_role_value(&updated).unwrap();
}
#[test]
fn emptied_block_with_unknown_children_keeps_the_block_and_sheds_the_right() {
    let body = [
        "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Rights\" version=\"2.20\">",
        "\t<setForNewObjects>false</setForNewObjects>",
        "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
        "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
        "\t<object>",
        "\t\t<name>Catalog.Solo</name>",
        "\t\t<future keep=\"true\"/>",
        "\t\t<right><name>Read</name><value>true</value></right>",
        "\t</object>",
        "</Rights>",
    ]
    .join("\n");

    let (updated, effect) =
        apply_role_edit_operation(&body, &operation("Catalog.Solo", "Read", false), 0).unwrap();
    assert!(effect.changed);
    assert_eq!(effect.action, RoleEditEffectAction::SetRight);
    assert!(updated.contains("<name>Catalog.Solo</name>"), "{updated}");
    assert!(updated.contains("<future keep=\"true\"/>"), "{updated}");
    assert!(!updated.contains("<name>Read</name>"), "{updated}");
    validate_every_role_value(&updated).unwrap();
}
#[test]
fn set_for_new_objects_true_mirrors_the_stored_value_rule() {
    let body = rights_xml("\n", "<value>true</value>").replacen(
        "<setForNewObjects>false</setForNewObjects>",
        "<setForNewObjects>true</setForNewObjects>",
        1,
    );

    // Явный true при умолчании true уже совпадает с запросом — no-op:
    // нетронутая избыточная форма не нормализуется.
    let (same, noop) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", true), 0).unwrap();
    assert_eq!(same, body);
    assert!(!noop.changed);

    // false — значимое отклонение от умолчания: обычная замена значения.
    let (with_false, replaced) =
        apply_role_edit_operation(&body, &operation("Catalog.Demo", "Delete", false), 1).unwrap();
    assert!(replaced.changed);
    assert!(with_false.contains("<value>false</value>"), "{with_false}");
    validate_every_role_value(&with_false).unwrap();

    // Возврат к true — переход к умолчанию: право уходит из хранения.
    let (canonical, removed) =
        apply_role_edit_operation(&with_false, &operation("Catalog.Demo", "Delete", true), 2)
            .unwrap();
    assert!(removed.changed);
    assert_eq!(removed.before, Some(false));
    let document = Document::parse(&canonical).unwrap();
    let object = direct_role_children(document.root_element(), "object", ROLE_RIGHTS_NAMESPACE)[0];
    assert!(
        !direct_role_children(object, "right", ROLE_RIGHTS_NAMESPACE)
            .into_iter()
            .any(|right| {
                role_text_content(direct_role_children(right, "name", ROLE_RIGHTS_NAMESPACE)[0])
                    == "Delete"
            }),
        "{canonical}"
    );
    drop(document);
    validate_every_role_value(&canonical).unwrap();
}
#[test]
fn missing_object_is_error_except_absent_data_processor_use_false_noop() {
    let body = rights_xml("\n", "<value>true</value>");

    let missing =
        apply_role_edit_operation(&body, &operation("Catalog.Missing", "Delete", true), 0)
            .unwrap_err();
    assert!(missing.contains("was not found"), "{missing}");

    // Умолчание роли на неперечисленном объекте уже действует: запись
    // того же значения — законный no-op, а не отказ.
    let (noop_text, noop) =
        apply_role_edit_operation(&body, &operation("Catalog.Missing", "Delete", false), 0)
            .unwrap();
    assert_eq!(noop_text, body);
    assert!(!noop.changed);
    assert_eq!(noop.before, None);
    assert!(!noop.after);
    assert_eq!(noop.action, RoleEditEffectAction::SetRight);

    let (without_processor, first) =
        apply_role_edit_operation(&body, &operation("DataProcessor.Worker", "Use", false), 1)
            .unwrap();
    assert!(first.changed);
    assert_eq!(first.action, RoleEditEffectAction::RemoveObject);
    assert!(!without_processor.contains("DataProcessor.Worker"));

    let (repeated, second) = apply_role_edit_operation(
        &without_processor,
        &operation("DataProcessor.Worker", "Use", false),
        2,
    )
    .unwrap();
    assert_eq!(repeated, without_processor);
    assert!(!second.changed);
    assert_eq!(second.before, None);
    assert_eq!(second.action, RoleEditEffectAction::RemoveObject);
}
#[test]
fn role_metadata_emitter_defensively_escapes_name() {
    let xml = crate::descriptor::role_metadata_xml(
        "Bad<&\"'Name",
        "Synonym",
        "",
        "2.20",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        "ru",
    );

    let doc = Document::parse(&xml).unwrap();
    let name = doc
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Name")
        .and_then(|node| node.text());
    assert_eq!(name, Some("Bad<&\"'Name"));
}
