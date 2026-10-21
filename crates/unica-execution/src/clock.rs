use std::time::Instant;
use unica_work_control::clock::Clock;

#[derive(Debug, Default, Clone, Copy)]
pub struct TokioClock;

impl Clock for TokioClock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }
}
