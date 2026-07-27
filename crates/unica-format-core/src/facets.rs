//! Core-owned semantic groupings. Facets contain IDs, never copied values.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::semantic_ids::{SemanticFacetId, SemanticPropertyId, SemanticRelationId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SemanticFacetMember {
    Property(SemanticPropertyId),
    Relation(SemanticRelationId),
}

#[derive(Debug, Clone, Copy)]
pub struct SemanticFacetDefinition {
    pub id: SemanticFacetId,
    pub members: &'static [SemanticFacetMember],
}

const IDENTITY: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::METADATA_KIND),
    SemanticFacetMember::Property(SemanticPropertyId::METADATA_NAME),
    SemanticFacetMember::Property(SemanticPropertyId::METADATA_UUID),
    SemanticFacetMember::Property(SemanticPropertyId::METADATA_SYNONYM),
    SemanticFacetMember::Property(SemanticPropertyId::METADATA_COMMENT),
    SemanticFacetMember::Property(SemanticPropertyId::METADATA_CODE),
    SemanticFacetMember::Property(SemanticPropertyId::METADATA_DESCRIPTION),
];
const PRESENTATION: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::PRESENTATION_OBJECT),
    SemanticFacetMember::Property(SemanticPropertyId::PRESENTATION_EXTENDED_OBJECT),
    SemanticFacetMember::Property(SemanticPropertyId::PRESENTATION_LIST),
    SemanticFacetMember::Property(SemanticPropertyId::PRESENTATION_EXTENDED_LIST),
];
const SUPPORT: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::SUPPORT_STATE),
    SemanticFacetMember::Property(SemanticPropertyId::SUPPORT_AUTHORABILITY),
    SemanticFacetMember::Property(SemanticPropertyId::SUPPORT_EDIT_CAPABILITY),
];
const NUMBERING: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::DOCUMENT_NUMBER_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::DOCUMENT_NUMBER_LENGTH),
    SemanticFacetMember::Property(SemanticPropertyId::DOCUMENT_NUMBER_PERIODICITY),
    SemanticFacetMember::Property(SemanticPropertyId::DOCUMENT_NUMBER_AUTO),
];
const POSTING: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::DOCUMENT_POSTING_MODE),
    SemanticFacetMember::Property(SemanticPropertyId::DOCUMENT_REAL_TIME_POSTING_MODE),
    SemanticFacetMember::Property(SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_DELETION_MODE),
    SemanticFacetMember::Property(
        SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_WRITING_ON_POST_MODE,
    ),
    SemanticFacetMember::Relation(SemanticRelationId::REGISTER_RECORDS),
    SemanticFacetMember::Relation(SemanticRelationId::BASED_ON),
];
const HIERARCHY: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::CATALOG_HIERARCHICAL),
    SemanticFacetMember::Property(SemanticPropertyId::CATALOG_HIERARCHY_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMITED),
    SemanticFacetMember::Property(SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_COUNT),
    SemanticFacetMember::Property(SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT),
    SemanticFacetMember::Property(SemanticPropertyId::CATALOG_CODE_LENGTH),
    SemanticFacetMember::Property(SemanticPropertyId::CATALOG_DESCRIPTION_LENGTH),
];
const TYPING: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::CONSTANT_VALUE_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::DEFINED_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_PARAMETER_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_OPERATION_RETURN_TYPE),
];
const FIELDS: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_REQUIRED),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_FILL_CHECKING),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_INDEXING),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_MULTI_LINE),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_USE),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_FILL_VALUE),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_MASTER),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_MAIN_FILTER),
    SemanticFacetMember::Property(SemanticPropertyId::FIELD_DENY_INCOMPLETE_VALUES),
    SemanticFacetMember::Relation(SemanticRelationId::ATTRIBUTES),
    SemanticFacetMember::Relation(SemanticRelationId::DIMENSIONS),
    SemanticFacetMember::Relation(SemanticRelationId::RESOURCES),
    SemanticFacetMember::Relation(SemanticRelationId::COLUMNS),
];
const MODULE_EXECUTION: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_GLOBAL),
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_CLIENT_MANAGED_APPLICATION),
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_SERVER),
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_EXTERNAL_CONNECTION),
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_CLIENT_ORDINARY_APPLICATION),
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_SERVER_CALL),
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_PRIVILEGED),
    SemanticFacetMember::Property(SemanticPropertyId::MODULE_RETURN_VALUES_REUSE),
];
const SCHEDULING: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::JOB_METHOD),
    SemanticFacetMember::Property(SemanticPropertyId::JOB_USE),
    SemanticFacetMember::Property(SemanticPropertyId::JOB_PREDEFINED),
    SemanticFacetMember::Property(SemanticPropertyId::JOB_RESTART_COUNT),
    SemanticFacetMember::Property(SemanticPropertyId::JOB_RESTART_INTERVAL),
    SemanticFacetMember::Property(SemanticPropertyId::JOB_KEY),
];
const SUBSCRIPTION: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::SUBSCRIPTION_EVENT),
    SemanticFacetMember::Property(SemanticPropertyId::SUBSCRIPTION_HANDLER),
    SemanticFacetMember::Property(SemanticPropertyId::SUBSCRIPTION_SOURCE_TYPE),
];
const SERVICE: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::HTTP_SERVICE_ROOT_URL),
    SemanticFacetMember::Property(SemanticPropertyId::HTTP_SERVICE_REUSE_SESSIONS),
    SemanticFacetMember::Property(SemanticPropertyId::HTTP_SERVICE_SESSION_MAX_AGE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_NAMESPACE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_XDTO_PACKAGES),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_DESCRIPTOR_FILE_NAME),
    SemanticFacetMember::Relation(SemanticRelationId::URL_TEMPLATES),
    SemanticFacetMember::Relation(SemanticRelationId::METHODS),
    SemanticFacetMember::Relation(SemanticRelationId::OPERATIONS),
];
const OPERATION: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::HTTP_SERVICE_URL_TEMPLATE),
    SemanticFacetMember::Property(SemanticPropertyId::HTTP_SERVICE_METHOD),
    SemanticFacetMember::Property(SemanticPropertyId::HTTP_SERVICE_HANDLER),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_OPERATION_RETURN_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_OPERATION_NILLABLE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_OPERATION_TRANSACTIONED),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_OPERATION_PROCEDURE_NAME),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_PARAMETER_TYPE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_PARAMETER_NILLABLE),
    SemanticFacetMember::Property(SemanticPropertyId::WEB_SERVICE_PARAMETER_DIRECTION),
    SemanticFacetMember::Relation(SemanticRelationId::PARAMETERS),
];
const STRUCTURE: &[SemanticFacetMember] = &[
    SemanticFacetMember::Relation(SemanticRelationId::CHILDREN),
    SemanticFacetMember::Relation(SemanticRelationId::TABULAR_SECTIONS),
    SemanticFacetMember::Relation(SemanticRelationId::FORMS),
    SemanticFacetMember::Relation(SemanticRelationId::COMMANDS),
    SemanticFacetMember::Relation(SemanticRelationId::TEMPLATES),
    SemanticFacetMember::Relation(SemanticRelationId::ENUM_VALUES),
];
const ACCESS: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::ACCESS_NEW_OBJECTS_DEFAULT),
    SemanticFacetMember::Property(SemanticPropertyId::ACCESS_ATTRIBUTES_DEFAULT),
    SemanticFacetMember::Property(SemanticPropertyId::ACCESS_CHILD_OBJECTS_INDEPENDENT),
    SemanticFacetMember::Property(SemanticPropertyId::ACCESS_PERMISSION_NAME),
    SemanticFacetMember::Property(SemanticPropertyId::ACCESS_PERMISSION_ALLOWED),
    SemanticFacetMember::Property(SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS),
    SemanticFacetMember::Relation(SemanticRelationId::ACCESS_PERMISSIONS),
    SemanticFacetMember::Relation(SemanticRelationId::ACCESS_TARGET),
    SemanticFacetMember::Relation(SemanticRelationId::RESTRICTION_TEMPLATES),
];
const BACKING: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::BACKING_DESCRIPTOR_AVAILABLE),
    SemanticFacetMember::Property(SemanticPropertyId::BACKING_DESCRIPTOR_UUID),
    SemanticFacetMember::Property(SemanticPropertyId::BACKING_CONTENT_AVAILABLE),
    SemanticFacetMember::Property(SemanticPropertyId::BACKING_CONTENT_OPAQUE),
];
const UNKNOWN: &[SemanticFacetMember] = &[
    SemanticFacetMember::Property(SemanticPropertyId::UNKNOWN_FACTS),
    SemanticFacetMember::Relation(SemanticRelationId::UNKNOWN),
];

pub const SEMANTIC_FACET_REGISTRY: &[SemanticFacetDefinition] = &[
    SemanticFacetDefinition {
        id: SemanticFacetId::IDENTITY,
        members: IDENTITY,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::PRESENTATION,
        members: PRESENTATION,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::SUPPORT,
        members: SUPPORT,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::NUMBERING,
        members: NUMBERING,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::POSTING,
        members: POSTING,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::HIERARCHY,
        members: HIERARCHY,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::TYPING,
        members: TYPING,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::FIELDS,
        members: FIELDS,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::MODULE_EXECUTION,
        members: MODULE_EXECUTION,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::SCHEDULING,
        members: SCHEDULING,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::SUBSCRIPTION,
        members: SUBSCRIPTION,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::SERVICE,
        members: SERVICE,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::OPERATION,
        members: OPERATION,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::STRUCTURE,
        members: STRUCTURE,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::ACCESS,
        members: ACCESS,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::BACKING,
        members: BACKING,
    },
    SemanticFacetDefinition {
        id: SemanticFacetId::UNKNOWN,
        members: UNKNOWN,
    },
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SemanticFacets(BTreeMap<SemanticFacetId, Vec<SemanticFacetMember>>);

impl SemanticFacets {
    pub fn for_available(
        properties: impl IntoIterator<Item = SemanticPropertyId>,
        relations: impl IntoIterator<Item = SemanticRelationId>,
    ) -> Self {
        let properties = properties.into_iter().collect::<BTreeSet<_>>();
        let relations = relations.into_iter().collect::<BTreeSet<_>>();
        let entries = SEMANTIC_FACET_REGISTRY
            .iter()
            .filter_map(|definition| {
                let members = definition
                    .members
                    .iter()
                    .copied()
                    .filter(|member| match member {
                        SemanticFacetMember::Property(id) => properties.contains(id),
                        SemanticFacetMember::Relation(id) => relations.contains(id),
                    })
                    .collect::<Vec<_>>();
                (!members.is_empty()).then_some((definition.id, members))
            })
            .collect();
        Self(entries)
    }

    pub fn summary(&self) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(id, _)| {
                    matches!(
                        **id,
                        SemanticFacetId::IDENTITY
                            | SemanticFacetId::PRESENTATION
                            | SemanticFacetId::SUPPORT
                    )
                })
                .map(|(id, members)| (*id, members.clone()))
                .collect(),
        )
    }
}
