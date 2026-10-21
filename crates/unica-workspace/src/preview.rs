//! Rendering is supplied by trusted composition; workspace keeps and validates
//! the original retained images and the once-per-plan renderer instance.
use serde_json::Value;
use unica_source_contract::read::SourceImage;

pub trait SavedSourceDiffRenderer: Send + Sync {
    fn page(
        &self,
        images: &[SourceImage<'_>],
        key: &str,
        limit: usize,
        cursor: Option<&str>,
        checkpoint: &dyn Fn() -> Result<(), String>,
    ) -> Result<Value, String>;
}

pub type SavedSourceDiffFactory = fn(
    &[SourceImage<'_>],
    &dyn Fn() -> Result<(), String>,
) -> Result<Box<dyn SavedSourceDiffRenderer>, String>;
