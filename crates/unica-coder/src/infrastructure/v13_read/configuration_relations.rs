use super::*;
use crate::infrastructure::v13_read_projection::project_configuration_with_counts;

impl LogicalViewReadAuthority<'_> {
    pub(super) fn configuration_with_relations(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let at = route.at();
        let (payload, counts) = if self.read.configuration_xml_requires_streaming() {
            let streamed = self
                .read
                .streamed_configuration_root(&|| self.read_checkpoint())?;
            (Arc::new(streamed.payload), Some(streamed.counts))
        } else {
            (self.configuration_payload(admitted)?, None)
        };
        let mut limits = payload
            .pointer("/properties/referenceLimits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        for (relation, value) in payload
            .pointer("/properties/referenceTargets")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let logical = value.as_str().ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "configuration reference has no address",
                )
            })?;
            let target = QualifiedAddress::parse(&format!("{}:{logical}", at.source_set()))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
            let segment = &target.segments()[0];
            let proof = if segment.kind() == NodeKind::OrdinaryInterface {
                self.ordinary_interface_view(&target, segment.name(), admitted)
                    .map(|_| ())
            } else {
                let owner = MetadataAddress::parse_logical(logical).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                self.verify_registered_owner(&owner, admitted)
            };
            match proof {
                Ok(()) => rows.push(json!({"relation":relation,"at":target.to_string(),"kind":segment.kind().as_str()})),
                Err(error) if error.code() == RefusalCode::NotFound || error.detail() == Some(RefusalDetail::SourceUnreadable) => {
                    limits.push(format!("{relation}: {target}: {error}"));
                }
                Err(error) => return Err(error),
            }
        }
        if at.is_configuration_relation() {
            let node = NodeView::new(
                at.to_string(),
                NodeKind::Relation.as_str(),
                "Relations",
                Map::new(),
            );
            return Ok(NodeViewData::Collection(CollectionView::new(
                node.with_limits(limits),
                rows,
            )));
        }
        let node = if let Some(counts) = counts {
            project_configuration_with_counts(at, &payload, &[], counts)?
        } else {
            project_typed_payload(route, payload.as_ref().clone())?
        };
        Ok(node
            .with_branch(BranchRef::new(format!("{at}.Relation"), rows.len()))
            .with_additional_limits(limits))
    }
}
