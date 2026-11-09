//! Designer managed-form codecs without source publication authority.
pub mod form_move;
pub mod form_root_properties;
pub mod root;

mod dialect;

pub mod event_context;

pub mod codec;
pub mod read;

pub mod planner;

pub mod event_patch;

#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/support/text.rs"]
pub mod test_support;

mod column_add;
mod column_type;
pub mod default_forms;
