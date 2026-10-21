//! Verified delivery and installation of exact Unica artifacts.

mod archive;
mod attempt;
mod cache;
mod download;
mod error;
mod manifest;
mod platform;
mod target;

pub use archive::{
    extract_verified_tar_gz, extract_verified_zip, sha256_file, verify_runtime_files,
};
pub use attempt::{diagnose, AttemptLog, AttemptSubject, OpenAttempt, Stage, UnfinishedAttempt};
pub use cache::{Prefetched, RuntimeInstallation, RuntimeInstaller};
pub use download::{DownloadObserver, Downloader, HttpDownloader, SilentDownload};
pub use error::{BootstrapError, Failure, Result};
pub use manifest::{
    Artifact, ArtifactRole, DeliveryForm, ReleaseIdentity, RuntimeAsset, RuntimeFile,
    RuntimeManifest, SourceIdentity, TargetRuntime, CORE_ARTIFACT,
};
pub use target::HostTarget;

pub mod engine;

pub mod shared;

pub mod desk;
