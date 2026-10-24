//! Bounded observations of one Task read. No error prose or endpoint credentials.
use super::protocol_v5::V5EndpointRecord;
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Where one Task read failed. Peer acquisition joins connect and handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskObservationStage {
    AcquirePeer,
    BindControl,
    WriteRequest,
    ReadResponse,
    DecodeResponse,
    ProjectResult,
}

/// One failed Task read for the frontend log: when the stage timer started,
/// how long it ran, the cutoff it had and the hashed endpoint generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskObservation {
    pub stage: TaskObservationStage,
    pub started_at_epoch_ms: u64,
    pub elapsed_ms: u64,
    pub cutoff_after_ms: u64,
    pub cutoff_expired: bool,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_generation"
    )]
    pub endpoint_generation_hash: Option<[u8; 16]>,
}

/// Starts a stage clock against the cutoff of the read it observes.
#[derive(Clone, Copy)]
pub struct TaskObservationTimer {
    started: Instant,
    started_at_epoch_ms: u64,
    cutoff: Instant,
}
impl TaskObservationTimer {
    pub fn start(cutoff: Instant) -> Self {
        Self::since(Instant::now(), cutoff)
    }

    pub fn since(started: Instant, cutoff: Instant) -> Self {
        let epoch_ms = millis(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
        );
        Self {
            started,
            started_at_epoch_ms: epoch_ms.saturating_sub(millis(started.elapsed())),
            cutoff,
        }
    }

    pub fn failed(
        self,
        stage: TaskObservationStage,
        at: Instant,
        endpoint: Option<&V5EndpointRecord>,
    ) -> TaskObservation {
        TaskObservation {
            stage,
            started_at_epoch_ms: self.started_at_epoch_ms,
            elapsed_ms: millis(at.saturating_duration_since(self.started)),
            cutoff_after_ms: millis(self.cutoff.saturating_duration_since(self.started)),
            cutoff_expired: at >= self.cutoff,
            endpoint_generation_hash: endpoint.map(|record| {
                let mut hash = Sha256::new();
                hash.update(b"unica-task-endpoint-generation-v1\0");
                hash.update(record.instance_id().as_bytes());
                let digest = hash.finalize();
                let mut generation = [0; 16];
                generation.copy_from_slice(&digest[..16]);
                generation
            }),
        }
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
fn serialize_generation<S: Serializer>(
    generation: &Option<[u8; 16]>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match generation {
        Some(bytes) => serializer.serialize_str(&format!("{:032x}", u128::from_be_bytes(*bytes))),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::CoreIdentity;

    #[test]
    fn observation_exposes_only_bounded_timing_and_generation_digest() {
        let record = V5EndpointRecord::new(CoreIdentity::production_v5(), 41001).unwrap();
        let started = Instant::now();
        let timer = TaskObservationTimer::since(started, started + Duration::from_millis(225));
        let first = timer.failed(
            TaskObservationStage::ReadResponse,
            started + Duration::from_millis(226),
            Some(&record),
        );
        assert_eq!(first.elapsed_ms, 226);
        assert_eq!(first.cutoff_after_ms, 225);
        assert!(first.cutoff_expired);
        let value = serde_json::to_value(first).unwrap();
        let hash = value["endpointGenerationHash"].as_str().unwrap();
        assert_eq!(hash.len(), 32);
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        let encoded = value.to_string();
        assert!(!encoded.contains(record.token()));
        assert!(!encoded.contains(record.instance_id()));
        assert!(!encoded.contains("41001"));
        let same = timer.failed(TaskObservationStage::DecodeResponse, started, Some(&record));
        assert_eq!(
            same.endpoint_generation_hash,
            first.endpoint_generation_hash
        );
        let next = V5EndpointRecord::new(CoreIdentity::production_v5(), 41001).unwrap();
        assert_ne!(
            timer
                .failed(TaskObservationStage::ReadResponse, started, Some(&next))
                .endpoint_generation_hash,
            first.endpoint_generation_hash
        );
        let acquired = timer.failed(TaskObservationStage::AcquirePeer, started, None);
        assert!(serde_json::to_value(acquired)
            .unwrap()
            .get("endpointGenerationHash")
            .is_none());
    }
}
