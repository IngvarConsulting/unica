use super::*;

// resource-limit: mcp.eof-cancellation-grace
// class: deadline
// unit: ms
// scope: process
// value: 2000
// resource: Срок завершения отменённых вызовов и рабочих потоков поставщиков после закрытия stdio хостом.
// status: unproven
// origin: коммит 5f6649325 (fix: drain accepted mcp calls on eof)
// problem: неизвестно
// on-reach: frontend пишет «unica mcp shutdown grace expired while tool calls or provider workers were cleaning up» и останавливает рантайм
pub const EOF_CANCELLATION_GRACE: Duration = Duration::from_secs(2);

pub fn drain_stdio_frontend(in_flight: &InFlightRegistry, manual_calls: &ManualCalls) -> bool {
    manual_calls.wait_manual_settled();
    drain_mcp_shutdown(in_flight, EOF_CANCELLATION_GRACE)
}

pub fn drain_mcp_shutdown(in_flight: &InFlightRegistry, grace: Duration) -> bool {
    drain_mcp_shutdown_with(in_flight, grace, |remaining| {
        let deadline = Instant::now() + remaining;
        let code_search_idle =
            unica_code_intelligence::coordinator::drain_code_search_workers(remaining);
        let diagnostics_idle = unica_diagnostics::coordinator::drain_diagnostic_workers(
            deadline.saturating_duration_since(Instant::now()),
        );
        code_search_idle && diagnostics_idle
    })
}

pub fn drain_mcp_shutdown_with(
    in_flight: &InFlightRegistry,
    grace: Duration,
    drain_providers: impl FnOnce(Duration) -> bool,
) -> bool {
    let deadline = Instant::now() + grace;
    in_flight.cancel_all();
    let calls_idle = in_flight.wait_idle(deadline.saturating_duration_since(Instant::now()));
    let providers_idle = drain_providers(deadline.saturating_duration_since(Instant::now()));
    calls_idle && providers_idle
}

/// Retains each running call until its owner finishes, so shutdown can cancel
/// calls and wait without relying on SDK internals.
#[derive(Debug, Default)]
pub struct InFlightRegistry {
    pub state: Mutex<InFlightState>,
    pub changed: Condvar,
}

#[derive(Debug, Default)]
pub struct InFlightState {
    pub running: Vec<(u64, CancellationToken)>,
    pub next_id: u64,
}

impl InFlightRegistry {
    pub fn admit(self: &Arc<Self>) -> Result<InFlightGuard, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "in-flight registry lock poisoned".to_string())?;
        let id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| "in-flight call identifier overflow".to_string())?;
        state.next_id = id;
        let token = CancellationToken::new();
        state.running.push((id, token.clone()));
        Ok(InFlightGuard {
            registry: Arc::clone(self),
            id,
            token,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn running(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.running.len())
            .unwrap_or(0)
    }

    pub fn cancel_all(&self) {
        if let Ok(state) = self.state.lock() {
            for (_, token) in state.running.iter() {
                token.cancel();
            }
        }
    }

    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let deadline = Instant::now() + timeout;
        while !state.running.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok((next, _)) = self.changed.wait_timeout(state, remaining) else {
                return false;
            };
            state = next;
        }
        true
    }

    pub fn release(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.running.retain(|(entry, _)| *entry != id);
        }
        self.changed.notify_all();
    }
}

#[derive(Debug)]
pub struct InFlightGuard {
    pub registry: Arc<InFlightRegistry>,
    pub id: u64,
    pub token: CancellationToken,
}

impl InFlightGuard {
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.registry.release(self.id);
    }
}
