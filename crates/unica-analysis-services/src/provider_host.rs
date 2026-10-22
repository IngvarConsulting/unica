//! Exact provider-host identity and producer-owned shared lifetime.
use std::collections::BTreeSet;
use unica_work_control::shared_work::{
    SharedWork, SharedWorkLease, SharedWorkLifetime, SharedWorkProducer,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedWorkIdentityError {
    Provider,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProviderHostKey {
    engine: String,
    target: String,
    capabilities: BTreeSet<String>,
}

impl ProviderHostKey {
    #[allow(dead_code)] // Canonical provider handler is installed only at Task 22 cutover.
    pub fn new(
        engine: impl Into<String>,
        target: impl Into<String>,
        capabilities: BTreeSet<String>,
    ) -> Result<Self, SharedWorkIdentityError> {
        let engine = engine.into();
        let target = target.into();
        if !closed_nonempty(&engine)
            || !closed_nonempty(&target)
            || capabilities.is_empty()
            || capabilities.iter().any(|value| !closed_nonempty(value))
        {
            return Err(SharedWorkIdentityError::Provider);
        }
        Ok(Self {
            engine,
            target,
            capabilities,
        })
    }
}

fn closed_nonempty(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.bytes().any(|byte| byte == 0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LongWorkFailure {
    Invalidated,
    Unavailable,
}

pub struct ProviderHostOwner {
    shared: SharedWork<ProviderHostKey, (), LongWorkFailure>,
}
impl Default for ProviderHostOwner {
    fn default() -> Self {
        Self {
            shared: SharedWork::new(SharedWorkLifetime::ProducerBound),
        }
    }
}
impl ProviderHostOwner {
    pub fn join_or_start<W>(
        &self,
        key: ProviderHostKey,
        work: W,
    ) -> SharedWorkLease<ProviderHostKey, (), LongWorkFailure>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        self.shared.join_or_start(key, work)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_provider_key_rejects_weak_identity_and_remains_exact() {
        assert!(
            ProviderHostKey::new("bsl-analyzer", "aarch64-apple-darwin", BTreeSet::new()).is_err()
        );
        assert_ne!(
            ProviderHostKey::new(
                "bsl-analyzer",
                "aarch64-apple-darwin",
                BTreeSet::from(["search".to_string()])
            )
            .unwrap(),
            ProviderHostKey::new(
                "bsl-analyzer",
                "aarch64-apple-darwin",
                BTreeSet::from(["diagnostics".to_string()])
            )
            .unwrap()
        );
    }
}
