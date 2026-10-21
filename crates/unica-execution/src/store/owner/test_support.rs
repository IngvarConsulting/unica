//! Delay a real worker completion in acceptance tests without exposing the
//! controller's private publication split to production callers.
use super::*;

pub(crate) fn read_loaded(
    owner: &mut PrototypeStoreOwner,
    request: ReadRequest,
) -> Result<ReadCompletion, OwnerError> {
    owner.read_loaded(request)
}

pub(crate) fn publish_loaded(
    owner: &mut PrototypeStoreOwner,
    completion: ReadCompletion,
) -> Result<ReadPublication, OwnerError> {
    owner.publish_loaded(completion)
}
