//! Preserved pure interface oracles.
use super::*;
/// Писатель группы не знает о проверке состава: он же обслуживает
/// прежний `interface-edit`, где список может быть короче или длиннее.
/// В любом случае элементы второй группы остаются на своих местах.
#[test]
fn group_order_writer_rewrites_only_the_named_group_in_place() {
    let second = |text: &str| interface_text_group_command_order(text, "Second");
    for (requested, expected) in [
        (names(&["B", "A"]), names(&["B", "A"])),
        (names(&["B"]), names(&["B"])),
        (names(&["B", "A", "C"]), names(&["B", "A", "C"])),
    ] {
        let mut text = TWO_GROUP_ORDER.to_string();
        interface_text_replace_group_commands(&mut text, "First", &requested).unwrap();
        assert_eq!(interface_text_group_command_order(&text, "First"), expected);
        assert_eq!(second(&text), names(&["X", "Y"]), "{text}");
        assert!(text.contains("<Group>First</Group>"), "{text}");
        Document::parse(&text).unwrap();
    }
    // Перестановка переносит элементы платформы без переформатирования.
    let mut text = TWO_GROUP_ORDER.to_string();
    interface_text_replace_group_commands(&mut text, "First", &names(&["B", "A"])).unwrap();
    assert_eq!(
        text,
        TWO_GROUP_ORDER
            .replace("\"A\"", "\"@\"")
            .replace("\"B\"", "\"A\"")
            .replace("\"@\"", "\"B\"")
    );

    // У группы ещё нет порядка — он дописывается, чужие группы не трогаются.
    let mut text = TWO_GROUP_ORDER.to_string();
    interface_text_replace_group_commands(&mut text, "Third", &names(&["Z"])).unwrap();
    assert_eq!(
        interface_text_group_command_order(&text, "Third"),
        names(&["Z"])
    );
    assert_eq!(
        interface_text_group_command_order(&text, "First"),
        names(&["A", "B"])
    );
    assert_eq!(second(&text), names(&["X", "Y"]));
}

#[test]
fn order_membership_accepts_a_permutation_or_a_first_order_only() {
    let stored = names(&["A", "B", "C"]);
    interface_order_membership(&stored, &names(&["B", "A", "C"]), "groups").unwrap();
    interface_order_membership(&[], &names(&["B", "A"]), "groups").unwrap();
    for (requested, needle) in [
        (names(&["B", "A"]), "missing `C`"),
        (names(&["B", "A", "C", "D"]), "not stored `D`"),
        (names(&["B", "A", "D"]), "missing `C`; not stored `D`"),
        (names(&["B", "A", "A", "C"]), "`A` is listed more than once"),
        (names(&["B", "B", "C"]), "`B` is listed more than once"),
    ] {
        let error = interface_order_membership(&stored, &requested, "groups").unwrap_err();
        assert!(error.contains(needle), "{error}");
    }
    let error = interface_order_membership(&[], &names(&["A", "A"]), "groups").unwrap_err();
    assert!(error.contains("more than once"), "{error}");
}

#[test]
fn append_to_missing_commands_visibility_inserts_section_before_later_sections() {
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<CommandInterface xmlns=\"{INTERFACE_CI_NS}\" xmlns:xr=\"{INTERFACE_XR_NS}\" version=\"2.17\">\n\
\t<CommandsPlacement>\n\
\t</CommandsPlacement>\n\
</CommandInterface>"
    );

    interface_text_append_to_section(
            &mut text,
            "CommandsVisibility",
            "<Command name=\"Catalog.Products.StandardCommand.OpenList\"><Visibility><xr:Common>false</xr:Common></Visibility></Command>",
        )
        .unwrap();

    let visibility_index = text.find("<CommandsVisibility>").unwrap();
    let placement_index = text.find("<CommandsPlacement>").unwrap();
    assert!(visibility_index < placement_index, "{text}");
    assert!(text.contains("<xr:Common>false</xr:Common>"));
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

const TWO_GROUP_ORDER: &str = concat!(
        "<CommandInterface>\r\n\t<CommandsOrder>\r\n",
        "\t\t<Command name=\"A\">\r\n\t\t\t<CommandGroup>First</CommandGroup>\r\n\t\t</Command>\r\n",
        "\t\t<Command name=\"X\">\r\n\t\t\t<CommandGroup>Second</CommandGroup>\r\n\t\t</Command>\r\n",
        "\t\t<Command name=\"B\">\r\n\t\t\t<CommandGroup>First</CommandGroup>\r\n\t\t</Command>\r\n",
        "\t\t<Command name=\"Y\">\r\n\t\t\t<CommandGroup>Second</CommandGroup>\r\n\t\t</Command>\r\n",
        "\t</CommandsOrder>\r\n\t<GroupsOrder>\r\n\t\t<Group>First</Group>\r\n\t</GroupsOrder>\r\n</CommandInterface>",
    );
