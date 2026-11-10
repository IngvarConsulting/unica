//! The private runner project of a working copy: its projected configuration
//! and the ownership lock serializing runner invocations.
#[cfg(any(test, feature = "test-support"))]
pub use crate::platform::full_dump::test_support;
pub use crate::platform::full_dump::{
    current_host_platform, HostPlatform, RunnerProjectLockFailure, RunnerProjectStore,
    SerializedDumpProjection,
};
