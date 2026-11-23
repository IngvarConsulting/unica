use super::{BootstrapDaemonCommit, BootstrapDaemonStatus, RuntimeHandoff};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};
use unica_artifact_delivery::{BootstrapError, Result};

pub fn run_daemon_owner(
    _plugin: &Path,
    _runtime: &unica_artifact_delivery::RuntimeManifest,
    _entrypoint: &Path,
    _handoff: &RuntimeHandoff<'_>,
) -> Result<i32> {
    Err(BootstrapError::new(
        "daemon-owner is unsupported on this platform",
    ))
}

pub fn await_bootstrap_daemon_publication() -> io::Result<()> {
    Ok(())
}

pub enum BootstrapDaemonChild {}

impl BootstrapDaemonChild {
    pub fn spawn_if_supervised(
        _state_root: &Path,
        _identity: &str,
        _idle_grace: Duration,
        _readiness_deadline: Instant,
        _deadline: Instant,
    ) -> io::Result<Option<Self>> {
        Ok(None)
    }

    pub fn id(&self) -> u32 {
        match *self {}
    }

    pub fn daemon_is_alive(&self) -> bool {
        match *self {}
    }

    pub fn status(&mut self, _deadline: Instant) -> io::Result<BootstrapDaemonStatus> {
        match *self {}
    }

    pub fn commit(&mut self, _deadline: Instant) -> io::Result<BootstrapDaemonCommit> {
        match *self {}
    }

    pub fn abort(&mut self, _deadline: Instant) -> io::Result<()> {
        match *self {}
    }
}
