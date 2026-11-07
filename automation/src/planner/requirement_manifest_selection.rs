use super::requirement_course_codes::CourseCodeParser;
use super::requirement_manifest::{
    exclude_unhashed_primary_manifest_duplicates, preselect_parseable_manifest_entries,
};
use super::requirement_plan_matching::manifest_entry_matches_specialized_plan_title;
use serde_json::Value;
use std::collections::HashMap;

pub fn select_parseable_manifest_entries(
    manifest_entries: &[Value],
    target_plan_ids: Option<&[String]>,
    bootstrap_plan_titles: &HashMap<String, String>,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let preselected = preselect_parseable_manifest_entries(manifest_entries, target_plan_ids);
    let mut selected = preselected
        .iter()
        .filter(|entry| {
            manifest_entry_matches_specialized_plan_title(
                entry,
                bootstrap_plan_titles,
                primary_major_titles,
                manifest_entries,
                course_parser,
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    selected = exclude_unhashed_primary_manifest_duplicates(&selected);
    selected.sort_by(|left, right| {
        locale_like_compare(
            &string_field(left, "ownerTitle"),
            &string_field(right, "ownerTitle"),
        )
    });
    selected
}

fn locale_like_compare(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
