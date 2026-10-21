use super::*;
/// Uses the existing platform profile for the first descriptor. Refreshes are
/// surgical: extension-owned structure is never regenerated from the parent.
pub fn plan_borrow_object(
    parent: &[u8],
    existing: Option<&[u8]>,
    requested_overrides: Option<&[String]>,
    fresh_uuid: fn() -> String,
) -> Result<BorrowObjectPlan, BorrowObjectError> {
    plan_borrow_object_with_identity(
        parent,
        existing,
        requested_overrides,
        &CfeBorrowIdentity::default(),
        fresh_uuid,
    )
}
