//! A frontend owns a renewable daemon lease, not a permanent endpoint address.
use super::*;
use std::sync::{Arc, Condvar, Mutex};

pub struct V5DaemonClient {
    identity: CoreIdentity,
    configuration: Option<Configuration>,
    state: Mutex<ConnectionState>,
    refreshed: Condvar,
}

struct Configuration {
    state_root: PathBuf,
    executable: PathBuf,
    idle_grace: Duration,
}

struct ConnectionState {
    anchor: Arc<V5DaemonProcessOwner>,
    refreshing: bool,
}

impl V5DaemonClient {
    pub fn connect(
        state_root: PathBuf,
        identity: CoreIdentity,
        executable: PathBuf,
        idle_grace: Duration,
    ) -> Result<Self, String> {
        let anchor = V5DaemonProcessOwner::connect_or_spawn(
            &state_root,
            identity.clone(),
            executable.clone(),
            idle_grace,
        )?;
        Ok(Self {
            identity,
            configuration: Some(Configuration {
                state_root,
                executable,
                idle_grace,
            }),
            state: Mutex::new(ConnectionState {
                anchor: Arc::new(anchor),
                refreshing: false,
            }),
            refreshed: Condvar::new(),
        })
    }

    pub fn core_identity(&self) -> &CoreIdentity {
        &self.identity
    }

    pub fn connect_peer_before(
        &self,
        deadline: Instant,
    ) -> Result<V5DaemonProcessOwner, V5TransportError> {
        self.peer_before(deadline).map_err(|failure| match failure {
            V5ConnectFailure::NotInTime { .. } => {
                V5TransportError::NotConnectedInTime(failure.to_string())
            }
            V5ConnectFailure::Refused(message) => V5TransportError::RequestNotSent(message),
        })
    }

    /// A peer of the anchor's daemon. The anchor's endpoint answers in the
    /// whole remaining time: a busy daemon is waited for, not replaced, and
    /// only an endpoint that refuses the connection sends discovery to start
    /// a daemon again.
    fn peer_before(&self, deadline: Instant) -> Result<V5DaemonProcessOwner, V5ConnectFailure> {
        loop {
            in_time(deadline, "connection acquisition")?;
            let mut state = self
                .state
                .lock()
                .map_err(|_| "daemon connection state is poisoned")?;
            while state.refreshing {
                let budget = in_time(deadline, "connection refresh")?;
                state = self
                    .refreshed
                    .wait_timeout(state, budget)
                    .map_err(|_| "daemon connection state is poisoned")?
                    .0;
            }
            let anchor = Arc::clone(&state.anchor);
            drop(state);
            let Some(configuration) = &self.configuration else {
                return V5DaemonProcessOwner::connect_classified_before(
                    anchor.record.clone(),
                    deadline,
                )
                .map_err(Into::into);
            };
            let directory = DaemonStateDirectory::open(&configuration.state_root, &self.identity)?;
            if directory.read_v5_endpoint_record()?.as_ref() == Some(&anchor.record) {
                match V5DaemonProcessOwner::connect_classified_before(
                    anchor.record.clone(),
                    deadline,
                ) {
                    Ok(peer) => return Ok(peer),
                    Err(ConnectionFailure::Unavailable(_)) => {}
                    Err(failure) => return Err(failure.into()),
                }
            }
            // Only discovery is serialized. A completed refresh by a competing
            // call invalidates our observation; never publish an older anchor.
            let mut state = self
                .state
                .lock()
                .map_err(|_| "daemon connection state is poisoned")?;
            if state.refreshing || !Arc::ptr_eq(&anchor, &state.anchor) {
                continue;
            }
            state.refreshing = true;
            drop(state);
            let refreshed = V5DaemonProcessOwner::connect_or_spawn_before(
                &configuration.state_root,
                self.identity.clone(),
                configuration.executable.clone(),
                configuration.idle_grace,
                deadline,
            );
            let mut state = self
                .state
                .lock()
                .map_err(|_| "daemon connection state is poisoned")?;
            state.refreshing = false;
            let result = refreshed.map(|owner| {
                let owner = Arc::new(owner);
                state.anchor = Arc::clone(&owner);
                owner
            });
            self.refreshed.notify_all();
            drop(state);
            // Peers never own discovery's mutex, including their handshake.
            return V5DaemonProcessOwner::connect_classified_before(
                result?.record.clone(),
                deadline,
            )
            .map_err(Into::into);
        }
    }
}

// Scripted wire fixtures have no launchable binary or published state root.
#[cfg(any(test, feature = "test-support"))]
impl From<V5DaemonProcessOwner> for V5DaemonClient {
    fn from(anchor: V5DaemonProcessOwner) -> Self {
        Self {
            identity: anchor.core_identity().clone(),
            configuration: None,
            state: Mutex::new(ConnectionState {
                anchor: Arc::new(anchor),
                refreshing: false,
            }),
            refreshed: Condvar::new(),
        }
    }
}

#[cfg(test)]
#[path = "connections/tests.rs"]
mod medium_tests;
