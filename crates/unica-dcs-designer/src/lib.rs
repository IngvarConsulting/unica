//! Designer-кодек СКД: реализует семейный порт [`unica_dcs::codec::DcsCodec`]
//! без полномочий публикации workspace.
pub mod codec;
mod planner;
mod primitives;
mod recovery;
pub mod schema;
mod xml;

pub use codec::DesignerDcsCodec;
