use serde_json::{Map, Value};
use unica_address::address::{escape_name, QualifiedAddress};
use unica_operations::recovery::{MemberPathLookup, PreparedRedirect, RetiredArgument};
pub const RETIRED_ARGUMENTS: &[RetiredArgument] = &[RetiredArgument {
    op: "right.set",
    key: "values.object",
    address: "{at}.Right.{right}",
    retired: true,
}];
pub fn prepare(
    template: &str,
    at: &QualifiedAddress,
    args: &Map<String, Value>,
    value: &str,
    members: MemberPathLookup<'_>,
) -> Option<PreparedRedirect> {
    let _ = (at, args, members);
    Some(PreparedRedirect::literal(&template.replace(
        "{right}",
        &escape_name(&value.replacen('.', "_", 1)),
    )))
}
