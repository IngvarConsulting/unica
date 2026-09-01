pub(crate) mod apply;
pub(crate) mod check;
pub(crate) mod diff;
pub(crate) mod find;
pub(crate) mod task_tools;
pub(crate) mod tool_catalog;
pub(crate) mod view;

use std::time::Duration;

/// One bounded provider budget for a canonical logical read. It outlives the
/// seven-second inline-to-Task handoff window: handoff changes delivery, not
/// the lifetime of the admitted read.
pub(crate) const LOGICAL_READ_OPERATION_BUDGET: Duration = Duration::from_secs(120);
