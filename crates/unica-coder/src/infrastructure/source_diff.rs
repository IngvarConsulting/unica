//! Composition of guarded workspace images with the navigation renderer.
use unica_navigation::plan_diff::PlanDiffLayout;
use unica_source_contract::read::SourceImage;
use unica_workspace::preview::SavedSourceDiffRenderer;

struct NavigationSourceDiff(PlanDiffLayout);

impl SavedSourceDiffRenderer for NavigationSourceDiff {
    fn page(
        &self,
        images: &[SourceImage<'_>],
        key: &str,
        limit: usize,
        cursor: Option<&str>,
        checkpoint: &dyn Fn() -> Result<(), String>,
    ) -> Result<serde_json::Value, String> {
        self.0.page(images, key, limit, cursor, checkpoint)
    }
}

pub(crate) fn layout(
    images: &[SourceImage<'_>],
    checkpoint: &dyn Fn() -> Result<(), String>,
) -> Result<Box<dyn SavedSourceDiffRenderer>, String> {
    PlanDiffLayout::new(images, checkpoint)
        .map(|layout| Box::new(NavigationSourceDiff(layout)) as Box<dyn SavedSourceDiffRenderer>)
}
