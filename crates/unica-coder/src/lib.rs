pub mod application;
mod composition;
pub mod domain;
pub(crate) mod infrastructure;
pub mod interfaces;

#[cfg(test)]
pub(crate) mod test_support;

pub use infrastructure::platform::run_platform_main;
