use unica_operations::recovery::RetiredArgument;
pub const RETIRED_ARGUMENTS: &[RetiredArgument] = &[RetiredArgument {
    op: "mxl.set",
    key: "values.area",
    address: "{at}.Area.{value}",
    retired: true,
}];
