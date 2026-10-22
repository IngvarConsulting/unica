//! Shared cancellation, explicit deadlines and progress observations.
//!
//! Execution and workspace owners retain their lifecycle decisions; these
//! signals neither create an attempt nor publish its durable outcome.

pub mod cancellation;
pub mod operation_deadline;
pub mod progress;
pub mod provider_deadline;

pub mod clock;

pub mod shared_work;

pub mod long_work;

pub mod elapsed_budget;
