#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum TaskPhase {
    Created,
    PreflightPassed,
    BaselineReady,
    Developing,
    LocalVerified,
    SynchronizationPrepared,
    SynchronizationConflicts,
    Synchronized,
    IntegrationPlanned,
    AcquiringLocks,
    Locked,
    MainMerged,
    MainValidated,
    Committing,
    CommittedAndUnlocked,
    ArchivedSuccess,
    CleanedSuccess,
    BlockedByForeignLock,
    StaleRelevantBaseline,
    LockPlanExpansionRequired,
    StaleSupportPreflight,
    UnexpectedDelta,
    ValidationFailed,
    CommitBlocked,
    RecoveryRequired,
    CommittedUnverified,
    AbandonmentReady,
    ArchivedAbandoned,
    CleanedAbandoned,
}

impl TaskPhase {
    pub const ALL: &[Self] = &[
        Self::Created,
        Self::PreflightPassed,
        Self::BaselineReady,
        Self::Developing,
        Self::LocalVerified,
        Self::SynchronizationPrepared,
        Self::SynchronizationConflicts,
        Self::Synchronized,
        Self::IntegrationPlanned,
        Self::AcquiringLocks,
        Self::Locked,
        Self::MainMerged,
        Self::MainValidated,
        Self::Committing,
        Self::CommittedAndUnlocked,
        Self::ArchivedSuccess,
        Self::CleanedSuccess,
        Self::BlockedByForeignLock,
        Self::StaleRelevantBaseline,
        Self::LockPlanExpansionRequired,
        Self::StaleSupportPreflight,
        Self::UnexpectedDelta,
        Self::ValidationFailed,
        Self::CommitBlocked,
        Self::RecoveryRequired,
        Self::CommittedUnverified,
        Self::AbandonmentReady,
        Self::ArchivedAbandoned,
        Self::CleanedAbandoned,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::PreflightPassed => "preflightPassed",
            Self::BaselineReady => "baselineReady",
            Self::Developing => "developing",
            Self::LocalVerified => "localVerified",
            Self::SynchronizationPrepared => "synchronizationPrepared",
            Self::SynchronizationConflicts => "synchronizationConflicts",
            Self::Synchronized => "synchronized",
            Self::IntegrationPlanned => "integrationPlanned",
            Self::AcquiringLocks => "acquiringLocks",
            Self::Locked => "locked",
            Self::MainMerged => "mainMerged",
            Self::MainValidated => "mainValidated",
            Self::Committing => "committing",
            Self::CommittedAndUnlocked => "committedAndUnlocked",
            Self::ArchivedSuccess => "archivedSuccess",
            Self::CleanedSuccess => "cleanedSuccess",
            Self::BlockedByForeignLock => "blockedByForeignLock",
            Self::StaleRelevantBaseline => "staleRelevantBaseline",
            Self::LockPlanExpansionRequired => "lockPlanExpansionRequired",
            Self::StaleSupportPreflight => "staleSupportPreflight",
            Self::UnexpectedDelta => "unexpectedDelta",
            Self::ValidationFailed => "validationFailed",
            Self::CommitBlocked => "commitBlocked",
            Self::RecoveryRequired => "recoveryRequired",
            Self::CommittedUnverified => "committedUnverified",
            Self::AbandonmentReady => "abandonmentReady",
            Self::ArchivedAbandoned => "archivedAbandoned",
            Self::CleanedAbandoned => "cleanedAbandoned",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionPolicy {
    ReadOnly,
    LocalJournaled,
    Contained,
    PreparedJournaledEffect,
    JournaledEffect,
    PreviewedJournaledEffect,
}

impl ExecutionPolicy {
    pub const ALL: &[Self] = &[
        Self::ReadOnly,
        Self::LocalJournaled,
        Self::Contained,
        Self::PreparedJournaledEffect,
        Self::JournaledEffect,
        Self::PreviewedJournaledEffect,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "readOnly",
            Self::LocalJournaled => "localJournaled",
            Self::Contained => "contained",
            Self::PreparedJournaledEffect => "preparedJournaledEffect",
            Self::JournaledEffect => "journaledEffect",
            Self::PreviewedJournaledEffect => "previewedJournaledEffect",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum BranchedLifecycleToolName {
    #[serde(rename = "unica.branched.start")]
    BranchedStart,
    #[serde(rename = "unica.branched.status")]
    BranchedStatus,
    #[serde(rename = "unica.branched.archive")]
    BranchedArchive,
    #[serde(rename = "unica.branched.cleanup")]
    BranchedCleanup,
    #[serde(rename = "unica.delivery.inspect")]
    DeliveryInspect,
    #[serde(rename = "unica.delivery.create")]
    DeliveryCreate,
    #[serde(rename = "unica.delivery.verify")]
    DeliveryVerify,
    #[serde(rename = "unica.delivery.deploy")]
    DeliveryDeploy,
    #[serde(rename = "unica.merge.compare")]
    MergeCompare,
    #[serde(rename = "unica.merge.prepare")]
    MergePrepare,
    #[serde(rename = "unica.merge.conflicts")]
    MergeConflicts,
    #[serde(rename = "unica.merge.resolve")]
    MergeResolve,
    #[serde(rename = "unica.merge.apply")]
    MergeApply,
    #[serde(rename = "unica.merge.verify")]
    MergeVerify,
    #[serde(rename = "unica.repository.status")]
    RepositoryStatus,
    #[serde(rename = "unica.repository.update")]
    RepositoryUpdate,
    #[serde(rename = "unica.repository.planLocks")]
    RepositoryPlanLocks,
    #[serde(rename = "unica.repository.lock")]
    RepositoryLock,
    #[serde(rename = "unica.repository.unlock")]
    RepositoryUnlock,
    #[serde(rename = "unica.repository.commit")]
    RepositoryCommit,
    #[serde(rename = "unica.repository.recover")]
    RepositoryRecover,
}

impl BranchedLifecycleToolName {
    pub const ALL: &[Self] = &[
        Self::BranchedStart,
        Self::BranchedStatus,
        Self::BranchedArchive,
        Self::BranchedCleanup,
        Self::DeliveryInspect,
        Self::DeliveryCreate,
        Self::DeliveryVerify,
        Self::DeliveryDeploy,
        Self::MergeCompare,
        Self::MergePrepare,
        Self::MergeConflicts,
        Self::MergeResolve,
        Self::MergeApply,
        Self::MergeVerify,
        Self::RepositoryStatus,
        Self::RepositoryUpdate,
        Self::RepositoryPlanLocks,
        Self::RepositoryLock,
        Self::RepositoryUnlock,
        Self::RepositoryCommit,
        Self::RepositoryRecover,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BranchedStart => "unica.branched.start",
            Self::BranchedStatus => "unica.branched.status",
            Self::BranchedArchive => "unica.branched.archive",
            Self::BranchedCleanup => "unica.branched.cleanup",
            Self::DeliveryInspect => "unica.delivery.inspect",
            Self::DeliveryCreate => "unica.delivery.create",
            Self::DeliveryVerify => "unica.delivery.verify",
            Self::DeliveryDeploy => "unica.delivery.deploy",
            Self::MergeCompare => "unica.merge.compare",
            Self::MergePrepare => "unica.merge.prepare",
            Self::MergeConflicts => "unica.merge.conflicts",
            Self::MergeResolve => "unica.merge.resolve",
            Self::MergeApply => "unica.merge.apply",
            Self::MergeVerify => "unica.merge.verify",
            Self::RepositoryStatus => "unica.repository.status",
            Self::RepositoryUpdate => "unica.repository.update",
            Self::RepositoryPlanLocks => "unica.repository.planLocks",
            Self::RepositoryLock => "unica.repository.lock",
            Self::RepositoryUnlock => "unica.repository.unlock",
            Self::RepositoryCommit => "unica.repository.commit",
            Self::RepositoryRecover => "unica.repository.recover",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_PHASES: &[&str] = &[
        "created",
        "preflightPassed",
        "baselineReady",
        "developing",
        "localVerified",
        "synchronizationPrepared",
        "synchronizationConflicts",
        "synchronized",
        "integrationPlanned",
        "acquiringLocks",
        "locked",
        "mainMerged",
        "mainValidated",
        "committing",
        "committedAndUnlocked",
        "archivedSuccess",
        "cleanedSuccess",
        "blockedByForeignLock",
        "staleRelevantBaseline",
        "lockPlanExpansionRequired",
        "staleSupportPreflight",
        "unexpectedDelta",
        "validationFailed",
        "commitBlocked",
        "recoveryRequired",
        "committedUnverified",
        "abandonmentReady",
        "archivedAbandoned",
        "cleanedAbandoned",
    ];

    const EXPECTED_POLICIES: &[&str] = &[
        "readOnly",
        "localJournaled",
        "contained",
        "preparedJournaledEffect",
        "journaledEffect",
        "previewedJournaledEffect",
    ];

    const EXPECTED_TOOLS: &[&str] = &[
        "unica.branched.start",
        "unica.branched.status",
        "unica.branched.archive",
        "unica.branched.cleanup",
        "unica.delivery.inspect",
        "unica.delivery.create",
        "unica.delivery.verify",
        "unica.delivery.deploy",
        "unica.merge.compare",
        "unica.merge.prepare",
        "unica.merge.conflicts",
        "unica.merge.resolve",
        "unica.merge.apply",
        "unica.merge.verify",
        "unica.repository.status",
        "unica.repository.update",
        "unica.repository.planLocks",
        "unica.repository.lock",
        "unica.repository.unlock",
        "unica.repository.commit",
        "unica.repository.recover",
    ];

    #[test]
    fn task_phase_has_the_exact_closed_json_vocabulary() {
        let actual = TaskPhase::ALL
            .iter()
            .map(TaskPhase::as_str)
            .collect::<Vec<_>>();
        assert_eq!(actual, EXPECTED_PHASES);
        for value in EXPECTED_PHASES {
            let encoded = format!("\"{value}\"");
            let parsed: TaskPhase = serde_json::from_str(&encoded).unwrap();
            assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
        }
        assert!(serde_json::from_str::<TaskPhase>("\"unknown\"").is_err());
    }

    #[test]
    fn execution_policy_has_the_exact_closed_json_vocabulary() {
        let actual = ExecutionPolicy::ALL
            .iter()
            .map(ExecutionPolicy::as_str)
            .collect::<Vec<_>>();
        assert_eq!(actual, EXPECTED_POLICIES);
        for value in EXPECTED_POLICIES {
            let encoded = format!("\"{value}\"");
            let parsed: ExecutionPolicy = serde_json::from_str(&encoded).unwrap();
            assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
        }
        assert!(serde_json::from_str::<ExecutionPolicy>("\"unknown\"").is_err());
    }

    #[test]
    fn tool_name_has_the_exact_closed_json_vocabulary() {
        let actual = BranchedLifecycleToolName::ALL
            .iter()
            .map(BranchedLifecycleToolName::as_str)
            .collect::<Vec<_>>();
        assert_eq!(actual, EXPECTED_TOOLS);
        for value in EXPECTED_TOOLS {
            let encoded = format!("\"{value}\"");
            let parsed: BranchedLifecycleToolName = serde_json::from_str(&encoded).unwrap();
            assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
        }
        assert!(serde_json::from_str::<BranchedLifecycleToolName>("\"unknown\"").is_err());
    }
}
