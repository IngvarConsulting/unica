//! Concrete entropy source supplied to source-image builders by composition.
pub(crate) fn fresh_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
