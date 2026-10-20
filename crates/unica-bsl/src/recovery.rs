use unica_operations::recovery::RetiredArgument;
pub const RETIRED_ARGUMENTS: &[RetiredArgument] = &[
    RetiredArgument {
        op: "code.replace",
        key: "selector.method",
        address: "{at^Module}.Method.{value}",
        retired: true,
    },
    RetiredArgument {
        op: "code.insert",
        key: "selector.method",
        address: "{at^Module}.Method.{value}",
        retired: true,
    },
];
