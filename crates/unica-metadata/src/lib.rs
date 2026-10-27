mod diagnostics;
mod event_subscription;
mod info;
mod info_properties;
mod integrity;
mod member_read_profile;
mod observed_types;
mod operations;
mod properties;
mod results;
mod schema;
mod types;

pub use diagnostics::*;
pub use event_subscription::*;
pub use info::*;
pub use info_properties::*;
pub use integrity::*;
pub use member_read_profile::*;
pub use observed_types::*;
pub use operations::*;
pub use properties::*;
pub use results::*;
pub use schema::*;
pub use types::*;

pub mod subsystem;
pub use subsystem::{
    EffectiveSubsystemRole, SubsystemAddress, SubsystemAddressError, SUBSYSTEM_ADDRESS_MAX_DEPTH,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataElement {
    pub collection: String,
    pub name: String,
}

pub mod applicability;
pub mod apply_operations;
pub mod contracts;
pub mod descriptor_operations;
pub mod event;
pub mod event_contract;
pub mod support;

pub mod subsystem_operations;

pub mod interface_operations;

pub mod site;

pub mod recovery;

pub mod support_state;

pub mod type_contract;

pub mod property_input;
pub mod request;

pub mod validation_subject;

pub mod identifier;

pub mod event_catalog;

pub mod interface;

pub mod support_edit;

pub mod subsystem_topology;

pub mod event_input;

pub mod configuration;
pub mod extension;

mod source_contracts;

pub mod language_code;
