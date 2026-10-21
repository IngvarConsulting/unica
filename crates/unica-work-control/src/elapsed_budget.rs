//! Monotonic elapsed budgets without absolute-instant overflow.
use std::time::{Duration, Instant};

pub struct ElapsedBudgetDeadline {
    started: Instant,
    pub budget: Duration,
}

impl ElapsedBudgetDeadline {
    pub fn new(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
        }
    }
    pub fn remaining(&self) -> Option<Duration> {
        self.budget
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
    }
}
