use super::*;

impl LogicalViewReadAuthority<'_> {
    pub(super) fn checked_configuration_relations(
        &self,
        collection: CollectionView,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let (node, rows) = collection.into_parts();
        let mut verified = Vec::new();
        let mut limits = Vec::new();
        for row in rows {
            let raw = row["at"].as_str().ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "configuration reference has no address",
                )
            })?;
            let at = QualifiedAddress::parse(raw).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
            let segment = &at.segments()[0];
            let proof = if segment.kind() == NodeKind::OrdinaryInterface {
                self.ordinary_interface_view(&at, segment.name(), admitted)
                    .map(|_| ())
            } else {
                let target =
                    MetadataAddress::parse_logical(&at.logical_path()).map_err(|error| {
                        ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                    })?;
                self.verify_registered_owner(&target, admitted)
            };
            match proof {
                Ok(()) => verified.push(row),
                Err(error)
                    if error.code() == RefusalCode::NotFound
                        || error.detail() == Some(RefusalDetail::SourceUnreadable) =>
                {
                    limits.push(format!(
                        "{}: {raw}: {error}",
                        row["relation"].as_str().unwrap_or_default()
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(NodeViewData::Collection(CollectionView::new(
            node.with_additional_limits(limits),
            verified,
        )))
    }
}
