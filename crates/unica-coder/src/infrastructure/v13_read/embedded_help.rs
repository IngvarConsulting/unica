use super::LogicalViewReadAuthority;
use crate::application::v13::view::{ViewReadAuthority, ViewSourceSnapshot};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData, TextView};
use serde_json::{json, Map};
use std::path::{Path, PathBuf};
use unica_address::address::QualifiedAddress;
use unica_address::source_target::MetadataAddress;
use unica_execution_contract::view_error::ViewError;
use unica_metadata::embedded_help::{owner, page_file_name};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::{portable_path::PortablePathExt, SourceSetKind};

impl LogicalViewReadAuthority<'_> {
    pub(super) fn with_help_branch(
        &self,
        at: &QualifiedAddress,
        view: NodeViewData,
    ) -> Result<NodeViewData, ViewError> {
        let Some(target) = owner(at.segments()) else {
            return Ok(view);
        };
        if !self.supports_embedded_help() {
            return Ok(view);
        }
        match self
            .read
            .embedded_help_manifest(&target, &mut || self.read_checkpoint())
        {
            Ok(Some(_)) => Ok(view.with_branch(BranchRef::uncounted(format!("{at}.Help")))),
            Ok(None) => Ok(view),
            Err(error)
                if matches!(
                    error.code(),
                    RefusalCode::Cancelled | RefusalCode::DeadlineExceeded
                ) =>
            {
                Err(error)
            }
            Err(error) => {
                Ok(view.with_limit(format!("embedded help is unreadable at {at}.Help: {error}")))
            }
        }
    }

    pub(super) fn embedded_help_view(
        &self,
        at: &QualifiedAddress,
        target: &MetadataAddress,
        language: Option<&str>,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if let Some(code) = language {
            page_file_name(code).map_err(|error| ViewError::new(RefusalCode::BadValue, error))?;
        }
        let (manifest, pages) = self.local_help(target, admitted)?;
        let Some(code) = language else {
            let mut items = Vec::new();
            let mut limits = Vec::new();
            for code in pages {
                let page = at.member(&code).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                let relative = help_page_path(&manifest, &code)?;
                match self.read.embedded_help_exists(&relative, &mut || self.read_checkpoint()) {
                    Ok(true) => items.push(json!({"at":page.to_string(),"kind":"Help","title":code,"props":{"language":code}})),
                    Ok(false) => limits.push(format!("registered help HTML at {page} is missing")),
                    Err(error) => limits.push(format!("registered help HTML at {page} is unreadable: {error}")),
                }
            }
            return Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(at.to_string(), "Help", "Help", Map::new()).with_limits(limits),
                items,
            )));
        };
        let path = self.registered_help_page(&manifest, &pages, code)?;
        let text = self
            .read
            .embedded_help_text(&path, &mut || self.read_checkpoint())?;
        let mut props = Map::new();
        props.insert("language".into(), json!(code));
        Ok(NodeViewData::Text(
            TextView::new(NodeView::new(at.to_string(), "Help", code, props), text)
                .with_revision(Some(admitted.revision.clone())),
        ))
    }

    pub(super) fn embedded_help_export_path(
        &self,
        at: &QualifiedAddress,
        target: &MetadataAddress,
        language: Option<&str>,
    ) -> Result<String, ViewError> {
        let admitted = self.snapshot(at)?;
        let (manifest, pages) = self.local_help(target, &admitted)?;
        let relative = match language {
            Some(code) => self.registered_help_page(&manifest, &pages, code)?,
            None => manifest,
        };
        Ok(relative.portable().to_string())
    }

    fn supports_embedded_help(&self) -> bool {
        matches!(
            self.read.source_set_kind(),
            SourceSetKind::Configuration | SourceSetKind::Extension
        )
    }

    fn local_help(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(PathBuf, Vec<String>), ViewError> {
        if !self.supports_embedded_help() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "this source set has no embedded owner help route",
            ));
        }
        self.verify_registered_owner(target, admitted)?;
        self.read
            .embedded_help_manifest(target, &mut || self.read_checkpoint())?
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    "embedded help descriptor was not found",
                )
            })
    }

    fn registered_help_page(
        &self,
        manifest: &Path,
        pages: &[String],
        language: &str,
    ) -> Result<PathBuf, ViewError> {
        let path = help_page_path(manifest, language)?;
        if !pages.iter().any(|registered| registered == language) {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "help language is not registered in Help.xml",
            ));
        }
        if !self
            .read
            .embedded_help_exists(&path, &mut || self.read_checkpoint())?
        {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "registered help HTML was not found",
            ));
        }
        Ok(path)
    }
}

fn help_page_path(manifest: &Path, language: &str) -> Result<PathBuf, ViewError> {
    let name =
        page_file_name(language).map_err(|error| ViewError::new(RefusalCode::BadValue, error))?;
    Ok(manifest.with_file_name("Help").join(name))
}
