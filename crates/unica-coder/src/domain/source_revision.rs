use serde::{Deserialize, Serialize};

pub const SOURCE_REVISION_ALGORITHM: &str = "unica-source-sha256-v1";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRevision {
    pub generation: u64,
    pub digest: String,
    pub algorithm: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRevisionTrustLoss {
    Startup,
    WatcherGap,
    Overflow,
    RootChanged,
    UnsupportedFence,
    ReconcileFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRevisionState {
    Reconciling {
        generation: u64,
    },
    Trusted(SourceRevision),
    Untrusted {
        generation: u64,
        reason: SourceRevisionTrustLoss,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRevisionMachine {
    state: SourceRevisionState,
    last_trusted: Option<SourceRevision>,
    trust_loss_epoch: u64,
}

impl SourceRevisionMachine {
    pub fn new() -> Self {
        Self {
            state: SourceRevisionState::Untrusted {
                generation: 0,
                reason: SourceRevisionTrustLoss::Startup,
            },
            last_trusted: None,
            trust_loss_epoch: 1,
        }
    }

    pub fn from_revision(revision: SourceRevision) -> Result<Self, String> {
        if revision.algorithm != SOURCE_REVISION_ALGORITHM
            || revision.generation == 0
            || revision.digest.len() != 64
            || !revision.digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("stored source revision is not compatible".to_string());
        }
        Ok(Self {
            state: SourceRevisionState::Untrusted {
                generation: revision.generation,
                reason: SourceRevisionTrustLoss::Startup,
            },
            last_trusted: Some(revision),
            trust_loss_epoch: 1,
        })
    }

    pub fn state(&self) -> &SourceRevisionState {
        &self.state
    }

    /// Derives the revision that a successful reconciliation would publish
    /// without changing trust, generation or the last trusted snapshot.
    pub(crate) fn candidate_for_digest(&self, digest: String) -> Result<SourceRevision, String> {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("source revision digest must be 64 hexadecimal characters".to_string());
        }
        let generation = match self.last_trusted.as_ref() {
            Some(previous) if previous.digest == digest => previous.generation,
            Some(previous) => previous.generation.saturating_add(1),
            None => 1,
        };
        Ok(SourceRevision {
            generation,
            digest,
            algorithm: SOURCE_REVISION_ALGORITHM.to_string(),
        })
    }

    pub fn begin_reconcile(&mut self) {
        self.state = SourceRevisionState::Reconciling {
            generation: self.generation(),
        };
    }

    pub fn finish_reconcile(&mut self, digest: String) -> Result<SourceRevision, String> {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            return Err("source revision digest must be 64 hexadecimal characters".to_string());
        }
        let revision = self.candidate_for_digest(digest)?;
        self.last_trusted = Some(revision.clone());
        self.state = SourceRevisionState::Trusted(revision.clone());
        Ok(revision)
    }

    pub fn finish_reconcile_if_trust_unchanged(
        &mut self,
        digest: String,
        expected_trust_loss_epoch: u64,
    ) -> Result<Option<SourceRevision>, String> {
        if self.trust_loss_epoch != expected_trust_loss_epoch {
            return Ok(None);
        }
        self.finish_reconcile(digest).map(Some)
    }

    pub fn trust_loss_epoch(&self) -> u64 {
        self.trust_loss_epoch
    }

    /// Installs a previously validated candidate only while the entire
    /// admitted machine state (including the trust-loss epoch) is unchanged.
    /// The candidate was already validated when it was prepared, so success is
    /// infallible and may be the final observable step of a retained commit.
    pub(crate) fn install_candidate_if_unchanged(
        &mut self,
        expected: &Self,
        candidate: SourceRevision,
    ) -> bool {
        if self != expected {
            return false;
        }
        self.last_trusted = Some(candidate.clone());
        self.state = SourceRevisionState::Trusted(candidate);
        true
    }

    pub fn lose_trust(&mut self, reason: SourceRevisionTrustLoss) {
        self.trust_loss_epoch = self.trust_loss_epoch.wrapping_add(1);
        self.state = SourceRevisionState::Untrusted {
            generation: self.generation(),
            reason,
        };
    }

    fn generation(&self) -> u64 {
        match &self.state {
            SourceRevisionState::Reconciling { generation }
            | SourceRevisionState::Untrusted { generation, .. } => *generation,
            SourceRevisionState::Trusted(revision) => revision.generation,
        }
    }
}

impl Default for SourceRevisionMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_machine_only_publishes_monotonic_trusted_generations() {
        let mut machine = SourceRevisionMachine::new();
        assert!(matches!(
            machine.state(),
            SourceRevisionState::Untrusted {
                generation: 0,
                reason: SourceRevisionTrustLoss::Startup
            }
        ));

        let first = machine.finish_reconcile("a".repeat(64)).unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(first.algorithm, SOURCE_REVISION_ALGORITHM);
        let unchanged = machine.finish_reconcile("a".repeat(64)).unwrap();
        assert_eq!(unchanged.generation, 1);
        let changed = machine.finish_reconcile("b".repeat(64)).unwrap();
        assert_eq!(changed.generation, 2);

        machine.lose_trust(SourceRevisionTrustLoss::WatcherGap);
        assert!(matches!(
            machine.state(),
            SourceRevisionState::Untrusted {
                generation: 2,
                reason: SourceRevisionTrustLoss::WatcherGap
            }
        ));
    }

    #[test]
    fn revision_machine_does_not_publish_across_a_trust_loss_epoch() {
        let mut machine = SourceRevisionMachine::new();
        machine.finish_reconcile("a".repeat(64)).unwrap();
        let admitted_epoch = machine.trust_loss_epoch();
        machine.begin_reconcile();
        machine.lose_trust(SourceRevisionTrustLoss::WatcherGap);

        assert_eq!(
            machine
                .finish_reconcile_if_trust_unchanged("b".repeat(64), admitted_epoch)
                .unwrap(),
            None
        );
        assert!(matches!(
            machine.state(),
            SourceRevisionState::Untrusted {
                reason: SourceRevisionTrustLoss::WatcherGap,
                ..
            }
        ));
    }
}
