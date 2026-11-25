use super::*;
use crate::infrastructure::v13_read_projection::project_configuration_with_counts;

impl LogicalViewReadAuthority<'_> {
    pub(super) fn configuration_with_relations(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let at = route.at();
        let streamed = if at.is_configuration_relation() {
            Some(
                self.read
                    .configuration_relations(&|| self.read_checkpoint())?,
            )
        } else if self.read.configuration_xml_requires_streaming() {
            Some(
                self.read
                    .streamed_configuration_root(&|| self.read_checkpoint())?,
            )
        } else {
            None
        };
        let (payload, counts, registrations) = match streamed {
            Some(root) => (
                Arc::new(root.payload),
                Some(root.counts),
                Some(root.registered_reference_targets),
            ),
            None => (self.configuration_payload(admitted)?, None, None),
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
            let kind = segment.kind();
            let name = segment.name().ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "configuration reference has no target name",
                )
            })?;
            let registered = match &registrations {
                Some(registered) => registered.contains(logical),
                None => payload["registeredObjects"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|row| {
                        row["kind"].as_str() == Some(kind.as_str())
                            && row["name"].as_str() == Some(name)
                    }),
            };
            if !registered {
                limits.push(format!("{relation}: {target} is not registered"));
                continue;
            }
            let proof = if kind == NodeKind::OrdinaryInterface {
                self.read
                    .ordinary_interface_synonym(name, &mut || self.read_checkpoint())
                    .map(|_| ())
            } else {
                let owner = MetadataAddress::parse_logical(logical).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                self.owner_evidence(&owner, admitted)
                    .and_then(|descriptor| {
                        if descriptor.artifact_kind() == kind.as_str()
                            && descriptor.artifact_name() == Some(name)
                        {
                            Ok(())
                        } else {
                            Err(ViewError::detailed(
                                RefusalDetail::SourceUnreadable,
                                format!("metadata descriptor identity does not match {target}"),
                            ))
                        }
                    })
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
