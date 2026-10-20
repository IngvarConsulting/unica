//! Designer role codecs without source publication authority.
pub mod descriptor;
pub mod writer;

pub mod planner;

pub mod info;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod writer_contract_tests;
