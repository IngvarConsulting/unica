//! Existing role read facts and capability projections.
use crate::known_role_rights;
use unica_metadata::support_state::ObjectSupportData as DomainObjectSupportData;
pub struct RoleInfoRightSummary {
    pub name: String,
    pub rls: bool,
}
pub struct RoleInfoObjectSummary {
    pub short_name: String,
    pub rights: Vec<RoleInfoRightSummary>,
}
pub struct RoleInfoGroup {
    pub type_prefix: String,
    pub objects: Vec<RoleInfoObjectSummary>,
}
/// Typed answer of `unica.role.info` (ADR-0023). Denied rights are always
/// present: hiding them behind a flag made "no denied rights" and "you did not
/// ask" the same observation.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleInfoData {
    pub name: String,
    pub synonym: Option<String>,
    pub support: DomainObjectSupportData,
    pub defaults: RoleDefaultsData,
    pub allowed: Vec<RoleGroupData>,
    pub denied: Vec<RoleGroupData>,
    pub totals: RoleTotalsData,
    pub restricted_objects: Vec<String>,
    pub templates: Vec<String>,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDefaultsData {
    pub set_for_new_objects: Option<String>,
    pub set_for_attributes_by_default: Option<String>,
    pub independent_rights_of_child_objects: Option<String>,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleGroupData {
    pub kind: String,
    pub objects: Vec<RoleObjectData>,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleObjectData {
    pub name: String,
    pub rights: Vec<RoleRightData>,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRightData {
    pub name: String,
    /// Row-level security restricts this right on this object.
    pub restricted: bool,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleTotalsData {
    pub allowed: usize,
    pub denied: usize,
}
pub fn role_validate_object_type(name: &str) -> &str {
    name.split_once('.')
        .map(|(prefix, _)| prefix)
        .unwrap_or(name)
}
pub fn role_validate_find_similar(needle: &str, haystack: &[&str]) -> Vec<String> {
    let needle_lower = needle.to_lowercase();
    let mut result = Vec::new();
    for item in haystack {
        let item_lower = item.to_lowercase();
        if needle_lower.contains(&item_lower) || item_lower.contains(&needle_lower) {
            result.push((*item).to_string());
        }
        if result.len() >= 3 {
            break;
        }
    }
    result
}
pub fn role_validate_known_rights(object_type: &str) -> &'static [&'static str] {
    known_role_rights(object_type)
}
pub fn add_role_info_right(
    groups: &mut Vec<RoleInfoGroup>,
    type_prefix: &str,
    short_name: &str,
    right: RoleInfoRightSummary,
) {
    let group_index = groups
        .iter()
        .position(|group| group.type_prefix == type_prefix)
        .unwrap_or_else(|| {
            groups.push(RoleInfoGroup {
                type_prefix: type_prefix.to_string(),
                objects: Vec::new(),
            });
            groups.len() - 1
        });

    let group = &mut groups[group_index];
    let object_index = group
        .objects
        .iter()
        .position(|object| object.short_name == short_name)
        .unwrap_or_else(|| {
            group.objects.push(RoleInfoObjectSummary {
                short_name: short_name.to_string(),
                rights: Vec::new(),
            });
            group.objects.len() - 1
        });
    group.objects[object_index].rights.push(right);
}
pub fn append_role_info_group(
    lines: &mut Vec<String>,
    objects: &[RoleInfoObjectSummary],
    is_denied: bool,
) {
    for object in objects {
        let rights = object
            .rights
            .iter()
            .map(|right| {
                if is_denied {
                    format!("-{}", right.name)
                } else if right.rls {
                    format!("{} [RLS]", right.name)
                } else {
                    right.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("    {}: {rights}", object.short_name));
    }
}
