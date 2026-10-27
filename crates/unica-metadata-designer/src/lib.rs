//! Designer codecs for metadata and its supplier-support state.
pub mod layout;
pub mod support;

pub mod xml_model;

pub mod info_projection;

pub mod integrity_check;

pub mod exchange_content;
pub mod form_metadata;

pub mod validation;

pub mod validation_context;

pub mod format_contract;

pub mod edit;

pub mod info;

pub mod job_schedule;

pub mod template_catalog;

pub mod member_properties;
pub mod predefined;

pub mod decode;

pub mod resource_plan;

pub mod remove;

pub mod help;

pub mod planner;

pub mod subsystem;

pub mod interface;

pub mod support_codec;

pub mod subsystem_topology;

pub mod resource_planner;

pub mod usage_scan;

pub mod child_resources;

pub mod borrowed_image;
pub mod code_support;
pub mod property_states;

pub mod template_owner;

pub mod image_plan;
pub mod relations;

pub mod client_interface;
pub mod configuration;
pub mod extension;

pub mod info_read;

pub mod style_item_read;

pub mod external;

pub mod borrowed_enum_value;
pub mod borrowed_form;
pub mod borrowed_object;

mod scheduled_job_key;
