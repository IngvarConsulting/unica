use unica_operations::recovery::RetiredArgument;
pub const RETIRED_ARGUMENTS: &[RetiredArgument] = &[
    RetiredArgument {
        op: "property.add",
        key: "values.propertyPath",
        address: "{at}.Property.{path}",
        retired: true,
    },
    RetiredArgument {
        op: "property.remove",
        key: "values.propertyPath",
        address: "{at}.Property.{path}.Property.{name}",
        retired: true,
    },
    RetiredArgument {
        op: "property.remove",
        key: "values.name",
        address: "{at}.Property.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "property.add",
        key: "propertyPath",
        address: "{at}.Property.{path}",
        retired: true,
    },
    RetiredArgument {
        op: "property.remove",
        key: "propertyPath",
        address: "{at}.Property.{path}.Property.{name}",
        retired: true,
    },
    RetiredArgument {
        op: "property.remove",
        key: "name",
        address: "{at}.Property.{value}",
        retired: true,
    },
];
