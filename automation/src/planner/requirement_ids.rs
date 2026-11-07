use super::requirement_scope::get_requirement_source_role_status;
use serde_json::Value;
use std::collections::HashMap;

pub fn slugify_requirement_identifier(value: &str) -> String {
    let mut slug = String::new();
    let mut pending_separator = false;
    for character in normalize_whitespace(value).to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            pending_separator = false;
            slug.push(character);
        } else {
            pending_separator = !slug.is_empty();
        }
    }
    slug
}

pub fn stable_source_id_hash(value: &str) -> String {
    let hash = value.encode_utf16().fold(0i32, |hash, code_unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(code_unit))
    });
    to_base36(hash.unsigned_abs())
}

pub fn build_requirement_source_block_id(owner: &Value) -> String {
    let adapter_slug = slugify_requirement_identifier(&nullish_string(owner.get("adapterId")));
    let source_role = owner
        .get("sourceRole")
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_default();
    let role_status = owner
        .get("sourceRoleStatus")
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_else(|| {
            get_requirement_source_role_status(Some(&source_role))
                .as_str()
                .to_string()
        });
    let needs_disambiguator = role_status != "primary"
        || owner.get("supportOnly").and_then(Value::as_bool) == Some(true)
        || owner.get("nonSchedulable").and_then(Value::as_bool) == Some(true)
        || owner.get("canCreateSchedulableRows").and_then(Value::as_bool) == Some(false)
        || owner.get("canCreateScheduleRows").and_then(Value::as_bool) == Some(false);
    let suffix = if needs_disambiguator {
        let role_slug = slugify_requirement_identifier(if source_role.is_empty() {
            "support"
        } else {
            &source_role
        });
        let source_url = nullish_string(owner.get("sourceUrl"));
        format!(
            "{adapter_slug}:{role_slug}:{}",
            stable_source_id_hash(&source_url)
        )
    } else {
        adapter_slug
    };
    format!("{}:source-block:{suffix}", js_interpolate(owner.get("ownerId")))
}

pub fn build_unique_derived_id(base_id: &str, seen_counts: &mut HashMap<String, usize>) -> String {
    let next_count = seen_counts
        .get(base_id)
        .copied()
        .unwrap_or_default()
        .saturating_add(1);
    seen_counts.insert(base_id.to_string(), next_count);
    if next_count == 1 {
        base_id.to_string()
    } else {
        format!("{base_id}-{next_count}")
    }
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn nullish_string(value: Option<&Value>) -> String {
    value
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_default()
}

fn js_interpolate(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(value) => js_string(value),
    }
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| if value.is_null() { String::new() } else { js_string(value) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn to_base36(mut value: u32) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while value > 0 {
        let digit = (value % 36) as u8;
        digits.push(if digit < 10 {
            (b'0' + digit) as char
        } else {
            (b'a' + digit - 10) as char
        });
        value /= 36;
    }
    digits.iter().rev().collect()
}

