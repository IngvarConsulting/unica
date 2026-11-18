pub mod article_fields;
mod container;
pub mod corpus;
pub mod installation;
mod locator;
pub use locator::PlatformHelpLocator;
pub mod provider;

#[cfg(test)]
mod platform_types;
#[cfg(test)]
mod real_installation;
#[cfg(test)]
mod retrieval_gate;
