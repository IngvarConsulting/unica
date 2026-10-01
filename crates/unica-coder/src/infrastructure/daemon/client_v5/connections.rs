//! A frontend owns a renewable daemon lease, not a permanent endpoint address.
use super::*;
use std::sync::{Arc, Condvar, Mutex};

pub(crate) struct V5DaemonClient {
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
    pub(crate) fn connect(
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

    pub(crate) fn core_identity(&self) -> &CoreIdentity {
        &self.identity
    }

    pub(crate) fn connect_peer_before(
        &self,
        deadline: Instant,
    ) -> Result<V5DaemonProcessOwner, V5TransportError> {
        self.peer_before(deadline)
            .map_err(V5TransportError::RequestNotSent)
    }

    fn peer_before(&self, deadline: Instant) -> Result<V5DaemonProcessOwner, String> {
        loop {
            remaining(deadline, "connection acquisition")?;
            let mut state = self
                .state
                .lock()
                .map_err(|_| "daemon connection state is poisoned")?;
            while state.refreshing {
                let budget = remaining(deadline, "connection refresh")?;
                state = self
                    .refreshed
                    .wait_timeout(state, budget)
                    .map_err(|_| "daemon connection state is poisoned")?
                    .0;
            }
            let anchor = Arc::clone(&state.anchor);
            drop(state);
            let Some(configuration) = &self.configuration else {
                return V5DaemonProcessOwner::connect_before(anchor.record.clone(), deadline);
            };
            let directory = DaemonStateDirectory::open(&configuration.state_root, &self.identity)?;
            if directory.read_v5_endpoint_record()?.as_ref() == Some(&anchor.record) {
                match V5DaemonProcessOwner::connect_classified_before(
                    anchor.record.clone(),
                    existing_endpoint_probe_deadline(deadline)?,
                ) {
                    Ok(peer) => return Ok(peer),
                    Err(ConnectionFailure::Rejected(message)) => return Err(message),
                    Err(ConnectionFailure::Unavailable(_)) => {}
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
            return V5DaemonProcessOwner::connect_before(result?.record.clone(), deadline);
        }
    }
}

// Scripted wire fixtures have no launchable binary or published state root.
#[cfg(test)]
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
mod tests;
