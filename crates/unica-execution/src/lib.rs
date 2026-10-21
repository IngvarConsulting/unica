//! Execution lifecycle decisions and owner-issued transition permits.
pub mod clock;
#[allow(dead_code)] // Preserved module allowance from the prior application owner.
pub mod coordinator;
#[allow(dead_code)] // Preserved module allowance from the prior application owner.
pub mod invocation;
#[allow(dead_code)] // Preserved module allowance from the prior application owner.
pub mod invocation_store;
#[allow(dead_code)] // Preserved module allowance from the prior application owner.
pub mod invocation_store_v5;
#[allow(dead_code)] // Preserved module allowance from the prior application owner.
pub mod invocation_v5;
#[allow(dead_code)] // Preserved module allowance from the prior application owner.
pub mod receipt_ledger;
#[allow(dead_code)] // Preserved module allowance from the prior application owner.
pub mod receipt_ledger_actor;

pub(crate) mod store;

#[allow(dead_code)] // Existing runtime state helpers remain with their actual owner.
pub mod runtime_jobs;

#[allow(dead_code)]
pub mod lifecycle_link_store;
pub mod receipt_runtime;
#[allow(dead_code)]
pub mod receipt_store;
pub mod task_store;
mod terminal_codec;
