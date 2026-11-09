use super::requirement_course_codes::normalize_course_code;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const PRIMARY_SOURCE_ROLES: &[&str] = &[
    "degree-requirements",
    "catalog",
    "curriculum",
    "worksheet",
    "official-catalog",
    "primary-degree-requirements",
    "department-requirements",
    "pathway-degree-sheet",
];

pub const SUPPORT_SOURCE_ROLES: &[&str] = &[
    "admissions",
    "overview",
    "equivalency",
    "availability",
    "other",
    "approved-course-list",
    "elective-list",
    "support-source",
    "admission-prerequisite-source",
    "admissions-preparation",
    "sample-schedule",
    "curriculum-map",
    "transfer-equivalency",
    "matched-grc-track",
];

pub const NON_SCHEDULABLE_SOURCE_ROLES: &[&str] = &[
    "upper-division-prerequisite-table",
    "non-schedulable-course-list",
];

pub const IGNORED_SOURCE_ROLES: &[&str] = &["old-archival", "ignored"];

pub const PARSEABLE_SUPPORT_ROLES: &[&str] = &[
    "approved-course-list",
    "elective-list",
    "upper-division-prerequisite-table",
    "non-schedulable-course-list",
    "sample-schedule",
    "admission-prerequisite-source",
    "admissions-preparation",
    "transfer-equivalency",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementSourceRoleStatus {
    Primary,
    Support,
    NonSchedulable,
    Ignored,
}

impl RequirementSourceRoleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Support => "support",
            Self::NonSchedulable => "non-schedulable",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceScope {
    pub can_create_required_rows: bool,
    pub can_create_option_groups: bool,
    pub can_create_credit_buckets: bool,
    pub can_create_category_options: bool,
    pub can_create_approved_filters: bool,
    pub can_create_elective_lists: bool,
    pub can_create_sequencing_hints: bool,
    pub can_create_admission_prep_rows: bool,
    pub can_create_schedule_rows: bool,
    pub support_only: bool,
    pub non_schedulable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlattenedRequirementSourceScope {
    pub source_scope: RequirementSourceScope,
    pub can_create_required_rows: bool,
    pub can_create_option_groups: bool,
    pub can_create_credit_buckets: bool,
    pub can_create_category_options: bool,
    pub can_create_approved_filters: bool,
    pub can_create_elective_lists: bool,
    pub can_create_sequencing_hints: bool,
    pub can_create_admission_prep_rows: bool,
    pub can_create_schedule_rows: bool,
    pub support_only: bool,
    pub non_schedulable: bool,
}

pub fn get_requirement_source_role_status(
    source_role: Option<&str>,
) -> RequirementSourceRoleStatus {
    match source_role.unwrap_or("ignored") {
        role if PRIMARY_SOURCE_ROLES.contains(&role) => RequirementSourceRoleStatus::Primary,
        role if SUPPORT_SOURCE_ROLES.contains(&role) => RequirementSourceRoleStatus::Support,
        role if NON_SCHEDULABLE_SOURCE_ROLES.contains(&role) => {
            RequirementSourceRoleStatus::NonSchedulable
        }
        role if IGNORED_SOURCE_ROLES.contains(&role) => RequirementSourceRoleStatus::Ignored,
        _ => RequirementSourceRoleStatus::Ignored,
    }
}

pub fn build_requirement_source_scope(source_role: Option<&str>) -> RequirementSourceScope {
    let role = source_role.unwrap_or("ignored");
    let status = get_requirement_source_role_status(Some(role));
    let primary = status == RequirementSourceRoleStatus::Primary;
    let support_only = status == RequirementSourceRoleStatus::Support;
    let non_schedulable = status == RequirementSourceRoleStatus::NonSchedulable;
    RequirementSourceScope {
        can_create_required_rows: primary,
        can_create_option_groups: primary,
        can_create_credit_buckets: primary,
        can_create_category_options: primary,
        can_create_approved_filters: role == "approved-course-list",
        can_create_elective_lists: role == "elective-list",
        can_create_sequencing_hints: role == "sample-schedule" || role == "curriculum-map",
        can_create_admission_prep_rows: matches!(
            role,
            "admission-prerequisite-source" | "admissions-preparation"
        ),
        can_create_schedule_rows: primary,
        support_only,
        non_schedulable: non_schedulable || status == RequirementSourceRoleStatus::Ignored,
    }
}

pub fn get_schedulable_source_section_lines(
    snapshot_lines: &[String],
    audit_rows: &[Value],
) -> Vec<String> {
    let mut decision_by_source_line_index = BTreeMap::<i64, bool>::new();
    for row in audit_rows {
        let Some(index) = row.get("sourceLineIndex").and_then(js_integer) else {
            continue;
        };
        decision_by_source_line_index.insert(
            index,
            row.get("schedulable").and_then(Value::as_bool) == Some(true),
        );
    }
    snapshot_lines
        .iter()
        .enumerate()
        .filter(|(index, _)| decision_by_source_line_index.get(&(*index as i64)) != Some(&false))
        .map(|(_, line)| line.clone())
        .collect()
}

pub fn filter_parsed_course_codes_by_source_sections(
    course_codes: &[String],
    audit_rows: &[Value],
) -> Vec<String> {
    let mut rows_by_course_code = HashMap::<String, Vec<bool>>::new();
    for row in audit_rows {
        for course_code in row
            .get("courseCodesExtracted")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let normalized_course_code = normalize_course_code(&js_string(course_code));
            if normalized_course_code.is_empty() {
                continue;
            }
            rows_by_course_code
                .entry(normalized_course_code)
                .or_default()
                .push(row.get("schedulable").is_some_and(js_truthy));
        }
    }
    course_codes
        .iter()
        .filter(|course_code| {
            rows_by_course_code
                .get(&normalize_course_code(course_code))
                .is_none_or(|rows| rows.iter().any(|schedulable| *schedulable))
        })
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn get_blocked_source_section_course_codes(audit_rows: &[Value]) -> Vec<String> {
    audit_rows
        .iter()
        .filter(|row| !row.get("schedulable").is_some_and(js_truthy))
        .flat_map(|row| {
            row.get("courseCodesExtracted")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|course_code| normalize_course_code(&js_string(course_code)))
                .filter(|course_code| !course_code.is_empty())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn flatten_requirement_source_scope(
    source_scope: RequirementSourceScope,
) -> FlattenedRequirementSourceScope {
    FlattenedRequirementSourceScope {
        source_scope,
        can_create_required_rows: source_scope.can_create_required_rows,
        can_create_option_groups: source_scope.can_create_option_groups,
        can_create_credit_buckets: source_scope.can_create_credit_buckets,
        can_create_category_options: source_scope.can_create_category_options,
        can_create_approved_filters: source_scope.can_create_approved_filters,
        can_create_elective_lists: source_scope.can_create_elective_lists,
        can_create_sequencing_hints: source_scope.can_create_sequencing_hints,
        can_create_admission_prep_rows: source_scope.can_create_admission_prep_rows,
        can_create_schedule_rows: source_scope.can_create_schedule_rows,
        support_only: source_scope.support_only,
        non_schedulable: source_scope.non_schedulable,
    }
}

pub fn can_requirement_source_role_create_schedulable_rows(source_role: Option<&str>) -> bool {
    build_requirement_source_scope(source_role).can_create_schedule_rows
}

pub fn get_block_scope_boolean(block: &Value, key: &str, fallback: bool) -> bool {
    block
        .get(key)
        .and_then(Value::as_bool)
        .or_else(|| {
            block
                .get("sourceScope")
                .and_then(|scope| scope.get(key))
                .and_then(Value::as_bool)
        })
        .unwrap_or(fallback)
}

fn source_role_status_for_block(block: &Value) -> RequirementSourceRoleStatus {
    let role = block
        .get("sourceRole")
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_else(|| "ignored".to_string());
    get_requirement_source_role_status(Some(&role))
}

pub fn is_block_support_only(block: &Value) -> bool {
    block.get("supportOnly").and_then(Value::as_bool) == Some(true)
        || block.get("sourceRoleStatus").and_then(Value::as_str) == Some("support")
        || source_role_status_for_block(block) == RequirementSourceRoleStatus::Support
}

pub fn is_block_non_schedulable(block: &Value) -> bool {
    block.get("nonSchedulable").and_then(Value::as_bool) == Some(true)
        || block.get("sourceRoleStatus").and_then(Value::as_str) == Some("non-schedulable")
        || matches!(
            source_role_status_for_block(block),
            RequirementSourceRoleStatus::NonSchedulable | RequirementSourceRoleStatus::Ignored
        )
}

pub fn can_parsed_block_create_schedulable_rows(block: &Value) -> bool {
    if block
        .get("canCreateSchedulableRows")
        .and_then(Value::as_bool)
        == Some(false)
        || block.get("canCreateScheduleRows").and_then(Value::as_bool) == Some(false)
        || block.get("canCreateRequiredRows").and_then(Value::as_bool) == Some(false)
        || block.get("supportOnly").and_then(Value::as_bool) == Some(true)
        || block.get("nonSchedulable").and_then(Value::as_bool) == Some(true)
    {
        return false;
    }

    block
        .get("sourceRole")
        .filter(|value| js_truthy(value))
        .map(|value| {
            let role = js_string(value);
            can_requirement_source_role_create_schedulable_rows(Some(&role))
        })
        .unwrap_or(true)
}

fn js_integer(value: &Value) -> Option<i64> {
    let number = value.as_f64()?;
    (number.is_finite()
        && number.fract() == 0.0
        && number >= i64::MIN as f64
        && number <= i64::MAX as f64)
        .then_some(number as i64)
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_string(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}
