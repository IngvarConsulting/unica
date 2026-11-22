use super::*;

pub(super) fn bound_command_events(
    native: &mut NativeProjections<'_, '_>,
    form_at: &str,
    form: roxmltree::Node<'_, '_>,
    origin: &str,
) {
    let commands = native_form_children(form, "Commands")
        .flat_map(|container| native_form_children(container, "Command"));
    for command in commands {
        let Some(handler) = native_form_children(command, "Action")
            .next()
            .and_then(|action| action.text())
            .filter(|handler| !handler.is_empty())
        else {
            continue;
        };
        let name = command.attribute("name").expect("native form command name");
        let owner = format!("{form_at}.Command.{name}");
        let execute = format!("{owner}.Event.Execute");
        native.collection(&owner, "Event", origin, BTreeSet::from([execute.clone()]));
        let view = read(native.authority, &execute);
        assert_eq!(view["props"]["handler"], handler, "{execute}");
        assert_eq!(view["props"]["state"], "implemented", "{execute}");
        let implementation = format!("{form_at}.Module.Form.Method.{handler}");
        assert_eq!(view["props"]["implementationAt"], implementation);
        assert_eq!(read(native.authority, &implementation)["kind"], "Method");
        check_native_projection_refusal(
            native.authority,
            &format!("{owner}.Event.NoSuchEvent"),
            "form Event `NoSuchEvent` was not found",
        );
    }
}
