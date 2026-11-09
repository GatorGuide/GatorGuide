use chrono::{SecondsFormat, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use url::Url;

use super::transfer_plan_title_matching::{
    label_mentions_different_transfer_planner_major, normalize_transfer_planner_text,
};

pub const DEFAULT_LINK_SNAPSHOT_FILENAME: &str = "transfer-planner-source-link-snapshot.json";
pub const DEFAULT_REQUIREMENT_PARSE_REPORT_FILENAME: &str =
    "transfer-planner-requirement-source-parse-report.json";
pub const DEFAULT_OUTPUT_JSON_FILENAME: &str = "transfer-planner-source-fingerprints.json";
pub const DEFAULT_OUTPUT_MD_FILENAME: &str = "transfer-planner-source-fingerprints.md";
pub const DEFAULT_CHANGE_CLASSIFICATION_JSON_FILENAME: &str =
    "transfer-planner-source-change-classification.json";
pub const DEFAULT_CHANGE_CLASSIFICATION_MD_FILENAME: &str =
    "transfer-planner-source-change-classification.md";
pub const DEFAULT_GENERATED_OUTPUT_RELATIVE_PATH: &str =
    "constants/transfer-planner-source/source-fingerprints.generated.ts";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintDiff {
    pub added: Vec<Value>,
    pub changed: Vec<Value>,
    pub unchanged: Vec<Value>,
    pub removed: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanChildPathwayCoverage {
    pub child_owner_ids: Vec<String>,
    pub parsed_uw_course_code_count: Value,
}

#[derive(Debug, Clone)]
enum JsPlusValue {
    Number(f64),
    String(String),
}

impl JsPlusValue {
    fn from_json(value: &Value) -> Self {
        match value {
            Value::String(value) => Self::String(value.clone()),
            Value::Array(_) | Value::Object(_) => Self::String(js_fingerprint_string(value)),
            _ => Self::Number(js_number_fingerprint(value).unwrap_or(f64::NAN)),
        }
    }

    fn to_js_string(&self) -> String {
        match self {
            Self::Number(value) if value.is_nan() => "NaN".to_string(),
            Self::Number(value) if *value == f64::INFINITY => "Infinity".to_string(),
            Self::Number(value) if *value == f64::NEG_INFINITY => "-Infinity".to_string(),
            Self::Number(value) => serde_json::Number::from_f64(*value)
                .map(|number| js_number_to_string(&number))
                .unwrap_or_else(|| "NaN".to_string()),
            Self::String(value) => value.clone(),
        }
    }

    fn add(&mut self, value: &Value) {
        let right = Self::from_json(value);
        *self = match (self.clone(), right) {
            (Self::String(left), right) => Self::String(left + &right.to_js_string()),
            (left, Self::String(right)) => Self::String(left.to_js_string() + &right),
            (Self::Number(left), Self::Number(right)) => Self::Number(left + right),
        };
    }

    fn into_json(self) -> Value {
        match self {
            Self::Number(value) => js_number_value(value),
            Self::String(value) => Value::String(value),
        }
    }
}

pub fn stable_for_hash(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(stable_for_hash).collect()),
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            let mut sorted = Map::with_capacity(map.len());
            for (key, val) in entries {
                sorted.insert(key.clone(), stable_for_hash(val));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

pub fn sha256_json(value: &Value) -> String {
    let stable = stable_for_hash(value);
    let mut serialized = String::new();
    write_javascript_json(&stable, &mut serialized);
    sha256_text(&serialized)
}

pub fn sha256_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_utf8_lossy(bytes: &[u8]) -> String {
    sha256_text(&String::from_utf8_lossy(bytes))
}

fn insert_defined_field(map: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value.clone());
    }
}

fn nullish_field(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| !value.is_null())
}

fn is_javascript_trim_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn js_trim(value: &str) -> &str {
    value.trim_matches(is_javascript_trim_whitespace)
}

fn nullish_array_value(value: Option<&Value>) -> Value {
    nullish_field(value)
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

fn js_length_property(value: &Value) -> Option<Value> {
    match value {
        Value::Array(items) => Some(json!(items.len())),
        Value::String(text) => Some(json!(text.encode_utf16().count())),
        Value::Object(fields) => fields.get("length").cloned(),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

fn insert_js_length_field(map: &mut Map<String, Value>, key: &str, value: &Value) {
    if let Some(length) = js_length_property(value) {
        map.insert(key.to_string(), length);
    }
}

fn js_display_nullish(value: Option<&Value>, fallback: &str) -> String {
    nullish_field(value)
        .map(js_fingerprint_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn js_display_direct(value: Option<&Value>) -> String {
    value
        .map(js_fingerprint_string)
        .unwrap_or_else(|| "undefined".to_string())
}

fn nullish_or_fingerprint<'a>(
    value: Option<&'a Value>,
    fallback: Option<&'a Value>,
) -> Option<&'a Value> {
    match value {
        Some(Value::Null) | None => fallback,
        Some(value) => Some(value),
    }
}

fn js_truthy_fingerprint(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn js_strict_equal(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(left)), Some(Value::Bool(right))) => left == right,
        (Some(Value::Number(left)), Some(Value::Number(right))) => left.as_f64() == right.as_f64(),
        (Some(Value::String(left)), Some(Value::String(right))) => left == right,
        (Some(Value::Array(_)), Some(Value::Array(_)))
        | (Some(Value::Object(_)), Some(Value::Object(_))) => {
            std::ptr::eq(left.unwrap(), right.unwrap())
        }
        _ => false,
    }
}

fn js_number_to_string(number: &serde_json::Number) -> String {
    let Some(value) = number.as_f64() else {
        return number.to_string();
    };
    if value == 0.0 {
        return "0".to_string();
    }

    let scientific = format!("{value:e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    let absolute = value.abs();
    let mut mantissa = mantissa.to_string();
    if mantissa.contains('.') {
        while mantissa.ends_with('0') {
            mantissa.pop();
        }
        if mantissa.ends_with('.') {
            mantissa.pop();
        }
    }

    if absolute >= 1e21 || absolute < 1e-6 {
        let exponent = if exponent >= 0 {
            format!("+{exponent}")
        } else {
            exponent.to_string()
        };
        return format!("{mantissa}e{exponent}");
    }

    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.strip_prefix('-').unwrap_or(&mantissa);
    let digits: String = unsigned.chars().filter(|ch| *ch != '.').collect();
    let decimal_index = 1 + exponent;
    let decimal = if decimal_index <= 0 {
        format!("0.{}{}", "0".repeat((-decimal_index) as usize), digits)
    } else if decimal_index as usize >= digits.len() {
        format!(
            "{}{}",
            digits,
            "0".repeat(decimal_index as usize - digits.len())
        )
    } else {
        let index = decimal_index as usize;
        format!("{}.{}", &digits[..index], &digits[index..])
    };
    if negative {
        format!("-{decimal}")
    } else {
        decimal
    }
}

fn write_javascript_json(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&js_number_to_string(value)),
        Value::String(value) => {
            output.push_str(&serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()));
        }
        Value::Array(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_javascript_json(item, output);
            }
            output.push(']');
        }
        Value::Object(fields) => {
            let entries = javascript_json_object_entries(fields);
            output.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()));
                output.push(':');
                write_javascript_json(item, output);
            }
            output.push('}');
        }
    }
}

fn javascript_json_object_entries(fields: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut entries: Vec<_> = fields.iter().collect();
    entries.sort_by(|(left, _), (right, _)| {
        match (javascript_array_index(left), javascript_array_index(right)) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    entries
}

fn javascript_array_index(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index < u32::MAX && index.to_string() == key).then_some(index)
}

fn write_javascript_json_pretty(value: &Value, output: &mut String, depth: usize) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            write_javascript_json(value, output);
        }
        Value::Array(items) if items.is_empty() => output.push_str("[]"),
        Value::Array(items) => {
            output.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                output.push_str(&" ".repeat((depth + 1) * 2));
                write_javascript_json_pretty(item, output, depth + 1);
                if index + 1 < items.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str(&" ".repeat(depth * 2));
            output.push(']');
        }
        Value::Object(fields) if fields.is_empty() => output.push_str("{}"),
        Value::Object(fields) => {
            let entries = javascript_json_object_entries(fields);
            output.push_str("{\n");
            for (index, (key, item)) in entries.iter().enumerate() {
                output.push_str(&" ".repeat((depth + 1) * 2));
                output.push_str(&serde_json::to_string(key).expect("valid UTF-8 JSON key"));
                output.push_str(": ");
                write_javascript_json_pretty(item, output, depth + 1);
                if index + 1 < entries.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str(&" ".repeat(depth * 2));
            output.push('}');
        }
    }
}

fn javascript_json_pretty(value: &Value) -> String {
    let mut output = String::new();
    write_javascript_json_pretty(value, &mut output, 0);
    output
}

fn js_fingerprint_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => js_number_to_string(value),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_fingerprint_string(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn js_join_array(value: Option<&Value>, separator: &str) -> Option<String> {
    let values = value?.as_array()?;
    Some(
        values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_fingerprint_string(value)
                }
            })
            .collect::<Vec<_>>()
            .join(separator),
    )
}

fn js_number_from_string(value: &str) -> Option<f64> {
    let value = js_trim(value);
    if value.is_empty() {
        return Some(0.0);
    }
    match value {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }

    if !value.starts_with('+') && !value.starts_with('-') {
        for (prefix, radix) in [
            ("0x", 16),
            ("0X", 16),
            ("0b", 2),
            ("0B", 2),
            ("0o", 8),
            ("0O", 8),
        ] {
            if let Some(digits) = value.strip_prefix(prefix) {
                if digits.is_empty() {
                    return None;
                }
                let mut number = 0.0;
                for digit in digits.chars() {
                    number = number * radix as f64 + digit.to_digit(radix)? as f64;
                }
                return Some(number);
            }
        }
    }

    let number = value.parse::<f64>().ok()?;
    if number.is_infinite() && !value.contains('e') && !value.contains('E') {
        return Some(f64::NAN);
    }
    Some(number)
}

fn js_number_fingerprint(value: &Value) -> Option<f64> {
    match value {
        Value::Null => Some(0.0),
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        Value::Number(value) => value.as_f64(),
        Value::String(value) => js_number_from_string(value),
        Value::Array(_) => js_number_from_string(&js_fingerprint_string(value)),
        Value::Object(_) => None,
    }
}

fn js_number_with_nullish_zero(value: Option<&Value>) -> Option<f64> {
    match nullish_field(value) {
        Some(value) => js_number_fingerprint(value),
        None => Some(0.0),
    }
}

fn js_number_value(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Null;
    }
    if value == 0.0 {
        return json!(0);
    }
    if value.fract() == 0.0 {
        if value >= i64::MIN as f64 && value < i64::MAX as f64 {
            return json!(value as i64);
        }
        if value >= 0.0 && value < u64::MAX as f64 {
            return json!(value as u64);
        }
    }
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn js_entry_key(entry: &Value, key: &str) -> String {
    js_trim(
        &entry
            .get(key)
            .filter(|value| !value.is_null())
            .map(js_fingerprint_string)
            .unwrap_or_default(),
    )
    .to_string()
}

fn locale_compare_fingerprint(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| right.cmp(left))
}

fn javascript_default_sort_strings(values: &mut [String]) {
    values.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
}

pub fn unique_sorted<I>(values: I) -> Vec<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut vec: Vec<String> = values
        .into_iter()
        .map(|v| v.as_ref().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    vec.sort_by(|left, right| locale_compare_fingerprint(left, right));
    vec.dedup();
    vec
}

pub fn unique_in_order(values: &[Value]) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let mut serialized = String::new();
        write_javascript_json(value, &mut serialized);
        if !serialized.is_empty() && seen.insert(serialized) {
            result.push(value.clone());
        }
    }
    result
}

pub fn merge_unique_strings(existing: &[Value], values: &[Value]) -> Vec<String> {
    let mut merged = HashSet::new();
    for val in existing {
        if let Some(s) = val.as_str() {
            merged.insert(s.to_string());
        }
    }
    for val in values {
        if js_truthy_fingerprint(val) {
            if let Some(s) = val.as_str() {
                merged.insert(s.to_string());
            }
        }
    }
    let mut res: Vec<String> = merged.into_iter().collect();
    javascript_default_sort_strings(&mut res);
    res
}

fn js_string_nullish_empty(value: Option<&Value>) -> String {
    value
        .filter(|value| !value.is_null())
        .map(js_fingerprint_string)
        .unwrap_or_default()
}

pub fn normalize_source_url_key(value: Option<&Value>) -> String {
    js_trim(&js_string_nullish_empty(value)).to_string()
}

pub fn normalize_transfer_planner_pathway_id_token(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let trimmed = js_trim(value).to_lowercase();
    let replaced_amp = trimmed.replace('&', " and ");
    let mut pending_dash = false;
    for ch in replaced_amp.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !normalized.is_empty() {
                normalized.push('-');
            }
            pending_dash = false;
            normalized.push(ch);
        } else {
            pending_dash = !normalized.is_empty();
        }
    }
    normalized
}

pub fn normalize_transfer_planner_pathway_id(
    plan_id: Option<&str>,
    pathway_id: Option<&str>,
) -> Option<String> {
    let trimmed_pathway = js_trim(pathway_id?);
    if trimmed_pathway.is_empty() {
        return None;
    }
    let normalized_plan_id = js_trim(plan_id.unwrap_or(""));
    let normalized_token = normalize_transfer_planner_pathway_id_token(trimmed_pathway);
    let alias = match (normalized_plan_id, normalized_token.as_str()) {
        ("uw-seattle-cinema-and-media-studies", "ba-option-family-cinema-studies")
        | ("uw-seattle-cinema-and-media-studies", "ba-option-family:cinema-studies") => {
            Some("cinema-studies")
        }
        ("uw-seattle-construction-management", "early-admission-option") => {
            Some("early-admission-pathway")
        }
        ("uw-seattle-construction-management", "freshmen-direct-option") => {
            Some("freshmen-direct-pathway")
        }
        ("uw-seattle-construction-management", "upper-division-admission-option") => {
            Some("upper-division-admission-pathway")
        }
        ("uw-seattle-economics", "strategy")
        | ("uw-seattle-economics", "bs-option-family-strategy") => {
            Some("bs-option-family:strategy")
        }
        ("uw-seattle-geography", "ba-option-family-in-geography-data-science")
        | ("uw-seattle-geography", "ba-option-family:in-geography-data-science")
        | ("uw-seattle-geography", "ba-option-family-in-geography-with-data-science")
        | ("uw-seattle-geography", "ba-option-family:in-geography-with-data-science")
        | ("uw-seattle-geography", "ccm-track")
        | ("uw-seattle-geography", "citizenship-and-migration-ccm-track")
        | ("uw-seattle-geography", "citizenship-and-migration-track") => {
            Some("cities-citizenship-and-migration-track")
        }
        ("uw-seattle-geography", "data-science-option")
        | (
            "uw-seattle-geography",
            "the-requirements-for-geography-majors-with-the-option-in-data-science-are-as-follows",
        ) => Some("geography-major-data-science-option"),
        ("uw-seattle-geography", "economy-and-sustainability-ees-track")
        | ("uw-seattle-geography", "economy-and-sustainability-track") => {
            Some("environment-economy-and-sustainability-track")
        }
        ("uw-seattle-geography", "health-and-development-track") => {
            Some("globalization-health-and-development-track")
        }
        ("uw-seattle-geography", "mapping-and-society-gms-track")
        | ("uw-seattle-geography", "mapping-and-society-track") => {
            Some("gis-mapping-and-society-track")
        }
        ("uw-seattle-biology", "bs-option-family-general")
        | ("uw-seattle-biology", "bs-option-family:general") => {
            Some("bs-option-family:general-biology")
        }
        ("uw-seattle-biology", "bs-option-family-plant")
        | ("uw-seattle-biology", "bs-option-family:plant") => {
            Some("bs-option-family:plant-biology")
        }
        ("uw-seattle-chemical-engineering", "nanoscience-and-molecular-engineering-nme-option")
        | ("uw-seattle-chemical-engineering", "nanoscience-and-molecular-engineering-option")
        | (
            "uw-seattle-materials-science-engineering",
            "nanoscience-and-molecular-engineering-nme-option",
        )
        | (
            "uw-seattle-materials-science-engineering",
            "nanoscience-and-molecular-engineering-option",
        ) => Some("nme-option"),
        ("uw-bothell-business-administration", "business-analytics-ai-concentration")
        | (
            "uw-bothell-business-administration",
            "business-analytics-and-artificial-intelligence-concentration",
        ) => Some("business-analytics-and-ai-concentration"),
        ("uw-bothell-business-administration", "management-information-systems-concentration")
        | (
            "uw-bothell-business-administration",
            "management-information-systems-mis-concentration",
        ) => Some("mis-concentration"),
        (
            "uw-bothell-business-administration",
            "technology-and-innovation-management-concentration",
        )
        | (
            "uw-bothell-business-administration",
            "technology-and-innovation-management-tim-concentration",
        ) => Some("tim-concentration"),
        ("uw-tacoma-history", "culture-and-society-option") => {
            Some("arts-culture-and-society-option")
        }
        ("uw-tacoma-history", "gender-and-identity-option") => {
            Some("power-gender-and-identity-option")
        }
        (
            "uw-tacoma-writing-studies",
            "grassroots-activism-and-community-organizing-this-track-centers-integrative-and-inclusive-pedagogy",
        )
        | ("uw-tacoma-writing-studies", "rhetoric-writing-and-social-change-track") => {
            Some("writing-and-social-change-track")
        }
        ("uw-tacoma-writing-studies", "to-complete-a-ba-in-the-technical-communication-track")
        | (
            "uw-tacoma-writing-studies",
            "to-complete-a-ba-in-the-technical-communication-track-of-writing-studies",
        ) => Some("technical-communication-track"),
        _ => None,
    };
    Some(alias.unwrap_or(trimmed_pathway).to_string())
}

pub fn build_transfer_planner_owner_id(plan_id: Option<&str>, pathway_id: Option<&str>) -> String {
    let normalized_plan_id = js_trim(plan_id.unwrap_or(""));
    let normalized_pathway_id =
        normalize_transfer_planner_pathway_id(Some(normalized_plan_id), pathway_id);
    match normalized_pathway_id {
        Some(pw) if !pw.is_empty() => format!("{normalized_plan_id}:pathway:{pw}"),
        _ => normalized_plan_id.to_string(),
    }
}

pub fn normalize_transfer_planner_owner_id(
    owner_id: &str,
    plan_id: Option<&str>,
    pathway_id: Option<&str>,
) -> String {
    let explicit_plan_id = js_trim(plan_id.unwrap_or(""));
    let normalized_owner_id = js_trim(owner_id);
    const PATHWAY_SEPARATOR: &str = ":pathway:";
    let pathway_index = normalized_owner_id.find(PATHWAY_SEPARATOR);

    if !explicit_plan_id.is_empty() && (pathway_id.is_some() || pathway_index.is_none()) {
        return build_transfer_planner_owner_id(Some(explicit_plan_id), pathway_id);
    }

    match pathway_index {
        None => {
            if !explicit_plan_id.is_empty() {
                explicit_plan_id.to_string()
            } else {
                normalized_owner_id.to_string()
            }
        }
        Some(idx) => {
            let pid = &normalized_owner_id[..idx];
            let pw = &normalized_owner_id[idx + PATHWAY_SEPARATOR.len()..];
            build_transfer_planner_owner_id(Some(pid), Some(pw))
        }
    }
}

pub fn normalize_source_owner_id(value: &str) -> String {
    let raw = js_trim(value);
    if raw.is_empty() {
        return String::new();
    }
    if let Some((plan_id, pathway_id)) = raw.split_once("::") {
        let has_line_terminator = |value: &str| {
            value
                .chars()
                .any(|ch| matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
        };
        if !plan_id.is_empty()
            && !pathway_id.is_empty()
            && !has_line_terminator(plan_id)
            && !has_line_terminator(pathway_id)
        {
            let normalized_pathway_id =
                normalize_transfer_planner_pathway_id(Some(plan_id), Some(pathway_id));
            return normalize_transfer_planner_owner_id(
                "",
                Some(plan_id),
                normalized_pathway_id.as_deref(),
            );
        }
    }
    normalize_transfer_planner_owner_id(raw, None, None)
}

pub fn normalize_source_owner_ids(values: Option<&Value>) -> Vec<String> {
    let list = match values {
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_fingerprint_string(value)
                }
            })
            .map(|value| normalize_source_owner_id(&value))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    unique_sorted(list)
}

pub fn get_plan_id_from_owner_id(owner_id: &str) -> String {
    let normalized = js_trim(owner_id);
    const PATHWAY_SEPARATOR: &str = ":pathway:";
    match normalized.find(PATHWAY_SEPARATOR) {
        Some(idx) => normalized[..idx].to_string(),
        None => normalized.to_string(),
    }
}

pub fn get_pathway_id_from_owner_id(owner_id: &str) -> Option<String> {
    let normalized = js_trim(owner_id);
    const PATHWAY_SEPARATOR: &str = ":pathway:";
    normalized
        .find(PATHWAY_SEPARATOR)
        .map(|idx| normalized[idx + PATHWAY_SEPARATOR.len()..].to_string())
}

pub fn get_ba_bs_degree_kind(value: &str) -> Option<&'static str> {
    let text = normalize_transfer_planner_text(value).to_lowercase();
    let mut cleaned = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            cleaned.push(ch);
        } else {
            cleaned.push(' ');
        }
    }
    static BA_PATTERN: OnceLock<Regex> = OnceLock::new();
    let ba = BA_PATTERN
        .get_or_init(|| Regex::new(r"(?-u:\b)(?:b\s*a|ba|bachelor\s+of\s+arts)(?-u:\b)").unwrap());
    if ba.is_match(&cleaned) {
        return Some("ba");
    }
    static BS_PATTERN: OnceLock<Regex> = OnceLock::new();
    let bs = BS_PATTERN.get_or_init(|| {
        Regex::new(r"(?-u:\b)(?:b\s*s|bs|bachelor\s+of\s+science)(?-u:\b)").unwrap()
    });
    if bs.is_match(&cleaned) {
        return Some("bs");
    }
    None
}

fn source_labels_as_spread_values(source: &Value) -> Vec<Value> {
    match nullish_field(source.get("labels")) {
        Some(Value::Array(labels)) => labels.clone(),
        Some(Value::String(labels)) => labels
            .chars()
            .map(|label| json!(label.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn build_source_owner_identity_text(source: &Value) -> String {
    let mut parts = Vec::new();
    for label in source_labels_as_spread_values(source) {
        if js_truthy_fingerprint(&label) {
            parts.push(js_fingerprint_string(&label));
        }
    }
    for field in ["title", "url", "finalUrl"] {
        if let Some(value) = source
            .get(field)
            .filter(|value| js_truthy_fingerprint(value))
        {
            parts.push(js_fingerprint_string(value));
        }
    }
    parts.join(" ")
}

pub fn source_owner_id_conflicts_with_source(
    owner_id: &str,
    source: &Value,
    titles_by_plan_id: &HashMap<String, String>,
) -> bool {
    let plan_id = get_plan_id_from_owner_id(owner_id);
    if plan_id.is_empty() {
        return false;
    }
    let mut identity_candidates = Vec::new();
    for label in source_labels_as_spread_values(source) {
        if js_truthy_fingerprint(&label) {
            identity_candidates.push(js_fingerprint_string(&label));
        }
    }
    if let Some(title) = source
        .get("title")
        .filter(|title| js_truthy_fingerprint(title))
    {
        identity_candidates.push(js_fingerprint_string(title));
    }
    if identity_candidates.iter().any(|candidate| {
        label_mentions_different_transfer_planner_major(&plan_id, candidate, titles_by_plan_id)
    }) {
        return true;
    }

    let pathway_id = get_pathway_id_from_owner_id(owner_id);
    let owner_identity = match pathway_id {
        Some(ref pw) => format!("{owner_id} {pw}"),
        None => format!("{owner_id} "),
    };
    let owner_degree_kind = get_ba_bs_degree_kind(&owner_identity);
    let source_degree_kind = get_ba_bs_degree_kind(&build_source_owner_identity_text(source));
    match (owner_degree_kind, source_degree_kind) {
        (Some(o), Some(s)) => o != s,
        _ => false,
    }
}

pub fn normalize_and_filter_source_owner_ids(
    values: Option<&Value>,
    source: &Value,
    titles_by_plan_id: &HashMap<String, String>,
) -> Vec<String> {
    let normalized = normalize_source_owner_ids(values);
    let filtered: Vec<String> = normalized
        .into_iter()
        .filter(|owner_id| {
            !source_owner_id_conflicts_with_source(owner_id, source, titles_by_plan_id)
        })
        .collect();
    unique_sorted(filtered)
}

pub fn is_transfer_planner_uw_owner_id(owner_id: &str) -> bool {
    static UW_OWNER_PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = UW_OWNER_PATTERN
        .get_or_init(|| Regex::new(r"(?i)^uw-(?:seattle|bothell|tacoma)-").unwrap());
    pattern.is_match(owner_id)
}

pub fn build_requirement_report_titles_by_plan_id(
    requirement_owners: &[Value],
) -> HashMap<String, String> {
    let mut titles_by_plan_id = HashMap::new();
    static TRAILING_TITLE_DASH: OnceLock<Regex> = OnceLock::new();
    let trailing_dash = TRAILING_TITLE_DASH.get_or_init(|| Regex::new(r"\s+-\s+.+$").unwrap());

    for owner in requirement_owners {
        let plan_id = if let Some(plan_id) = nullish_field(owner.get("planId")) {
            js_fingerprint_string(plan_id)
        } else {
            let owner_id = owner
                .get("ownerId")
                .filter(|owner_id| !owner_id.is_null())
                .map(js_fingerprint_string)
                .unwrap_or_default();
            get_plan_id_from_owner_id(&owner_id)
        };
        let plan_id = js_trim(&plan_id).to_string();
        let owner_title = js_trim(
            &owner
                .get("ownerTitle")
                .filter(|title| !title.is_null())
                .map(js_fingerprint_string)
                .unwrap_or_default(),
        )
        .to_string();
        if plan_id.is_empty() || owner_title.is_empty() {
            continue;
        }

        let existing = titles_by_plan_id.contains_key(&plan_id);
        let owner_has_pathway = owner
            .get("pathwayId")
            .filter(|pathway_id| !pathway_id.is_null())
            .map(js_fingerprint_string)
            .is_some_and(|pathway_id| !js_trim(&pathway_id).is_empty());
        if !existing || !owner_has_pathway {
            let title = if owner_has_pathway {
                trailing_dash.replace(&owner_title, "").into_owned()
            } else {
                owner_title
            };
            titles_by_plan_id.insert(plan_id, title);
        }
    }

    titles_by_plan_id
}

pub fn read_requirement_snapshot_hash(owner: &Value, repo_root: &Path) -> Option<String> {
    let snapshot_path_str = owner.get("snapshotPath").and_then(Value::as_str)?;
    let snapshot_path = Path::new(snapshot_path_str);
    let resolved_path = if snapshot_path.is_absolute() {
        snapshot_path.to_path_buf()
    } else {
        repo_root.join("source").join(snapshot_path)
    };
    if !resolved_path.is_file() {
        return None;
    }
    let content = fs::read(&resolved_path).ok()?;
    Some(sha256_utf8_lossy(&content))
}

pub fn build_source_fingerprint_entry(
    source: &Value,
    titles_by_plan_id: &HashMap<String, String>,
) -> Value {
    let mut fingerprint_input = Map::new();
    for key in [
        "ok",
        "status",
        "finalUrl",
        "contentType",
        "contentLength",
        "etag",
        "lastModified",
        "bodyHash",
        "title",
        "error",
        "fetchMode",
    ] {
        insert_defined_field(&mut fingerprint_input, key, source.get(key));
    }

    let owner_ids =
        normalize_and_filter_source_owner_ids(source.get("ownerIds"), source, titles_by_plan_id);
    let mut entry = Map::new();
    insert_defined_field(&mut entry, "url", source.get("url"));
    entry.insert(
        "finalUrl".to_string(),
        nullish_field(source.get("finalUrl"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    entry.insert(
        "labels".to_string(),
        nullish_field(source.get("labels"))
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    entry.insert("ownerIds".to_string(), json!(owner_ids));
    entry.insert(
        "kinds".to_string(),
        nullish_field(source.get("kinds"))
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    entry.insert(
        "ok".to_string(),
        json!(source.get("ok").is_some_and(js_truthy_fingerprint)),
    );
    for key in [
        "status",
        "contentType",
        "contentLength",
        "etag",
        "lastModified",
        "title",
    ] {
        entry.insert(
            key.to_string(),
            nullish_field(source.get(key))
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    entry.insert(
        "fetchMode".to_string(),
        nullish_field(source.get("fetchMode"))
            .cloned()
            .unwrap_or_else(|| json!("unknown")),
    );
    entry.insert(
        "resourceFingerprint".to_string(),
        json!(sha256_json(&Value::Object(fingerprint_input))),
    );
    Value::Object(entry)
}

pub fn build_requirement_backed_source_fingerprint_entry(
    owner: &Value,
    owner_ids: Option<Vec<String>>,
    repo_root: &Path,
) -> Value {
    let resolved_owner_ids = owner_ids.unwrap_or_else(|| {
        let oid = owner
            .get("ownerId")
            .filter(|owner_id| !owner_id.is_null())
            .map(js_fingerprint_string)
            .unwrap_or_default();
        normalize_source_owner_ids(Some(&json!([oid])))
    });
    let snapshot_hash = read_requirement_snapshot_hash(owner, repo_root);
    let mut fingerprint_input = Map::new();
    insert_defined_field(&mut fingerprint_input, "ok", owner.get("ok"));
    insert_defined_field(&mut fingerprint_input, "finalUrl", owner.get("sourceUrl"));
    fingerprint_input.insert(
        "contentType".to_string(),
        json!("requirement-source-snapshot"),
    );
    fingerprint_input.insert("contentLength".to_string(), Value::Null);
    fingerprint_input.insert("etag".to_string(), Value::Null);
    fingerprint_input.insert("lastModified".to_string(), Value::Null);
    fingerprint_input.insert(
        "bodyHash".to_string(),
        snapshot_hash
            .as_deref()
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    insert_defined_field(&mut fingerprint_input, "title", owner.get("extractedTitle"));
    insert_defined_field(&mut fingerprint_input, "error", owner.get("error"));
    fingerprint_input.insert("fetchMode".to_string(), json!("requirement-snapshot"));

    let mut labels = Vec::new();
    if let Some(label) = owner.get("sourceLabel") {
        if js_truthy_fingerprint(label) {
            labels.push(label.clone());
        }
    }
    let pathway_id_present = owner.get("pathwayId").is_some_and(js_truthy_fingerprint);
    let kinds = vec![json!(if pathway_id_present {
        "pathway"
    } else {
        "major"
    })];

    let mut entry = Map::new();
    insert_defined_field(&mut entry, "url", owner.get("sourceUrl"));
    insert_defined_field(&mut entry, "finalUrl", owner.get("sourceUrl"));
    entry.insert("labels".to_string(), json!(labels));
    entry.insert("ownerIds".to_string(), json!(resolved_owner_ids));
    entry.insert("kinds".to_string(), json!(kinds));
    entry.insert(
        "ok".to_string(),
        json!(owner.get("ok").is_some_and(js_truthy_fingerprint)),
    );
    entry.insert("status".to_string(), Value::Null);
    entry.insert(
        "contentType".to_string(),
        json!("requirement-source-snapshot"),
    );
    entry.insert("contentLength".to_string(), Value::Null);
    entry.insert("etag".to_string(), Value::Null);
    entry.insert("lastModified".to_string(), Value::Null);
    entry.insert(
        "title".to_string(),
        nullish_field(owner.get("extractedTitle"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    entry.insert("fetchMode".to_string(), json!("requirement-snapshot"));
    entry.insert(
        "resourceFingerprint".to_string(),
        json!(sha256_json(&Value::Object(fingerprint_input))),
    );
    Value::Object(entry)
}

pub fn build_requirement_fingerprint_entry(owner: &Value) -> Value {
    let field = |key: &str| nullish_array_value(owner.get(key));
    let parsed_uw_course_codes = field("parsedUwCourseCodes");
    let source_only_uw_course_codes = field("sourceOnlyUwCourseCodes");
    let structured_only_uw_course_codes = field("structuredOnlyUwCourseCodes");
    let approved_filter_uw_course_codes = field("approvedFilterUwCourseCodes");
    let elective_list_uw_course_codes = field("electiveListUwCourseCodes");
    let support_only_uw_course_codes = field("supportOnlyUwCourseCodes");
    let extracted_headings = field("extractedHeadings");
    let requirement_cue_lines = field("requirementCueLines");
    let choose_statements = field("chooseStatements");
    let quality_signals = field("qualitySignals")
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut coded_quality_signals = quality_signals
        .iter()
        .map(|signal| signal.get("code").cloned())
        .collect::<Vec<_>>();
    coded_quality_signals.sort_by(|left, right| match (left, right) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(left), Some(right)) => {
            let left = js_fingerprint_string(left);
            let right = js_fingerprint_string(right);
            left.encode_utf16().cmp(right.encode_utf16())
        }
    });
    let quality_signal_codes = coded_quality_signals
        .into_iter()
        .map(|code| code.unwrap_or(Value::Null))
        .collect::<Vec<_>>();
    let quality_warning_count = quality_signals
        .iter()
        .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("warning"))
        .count();
    let quality_note_count = quality_signals
        .iter()
        .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("note"))
        .count();

    let source_role = nullish_field(owner.get("sourceRole"))
        .cloned()
        .unwrap_or(Value::Null);
    let source_role_status = nullish_field(owner.get("sourceRoleStatus"))
        .cloned()
        .unwrap_or(Value::Null);
    let can_create_schedulable_rows = owner
        .get("canCreateSchedulableRows")
        .and_then(Value::as_bool)
        != Some(false);
    let support_only = owner.get("supportOnly").and_then(Value::as_bool) == Some(true);

    let mut fingerprint_input = Map::new();
    insert_defined_field(&mut fingerprint_input, "ok", owner.get("ok"));
    insert_defined_field(
        &mut fingerprint_input,
        "parseConfidence",
        owner.get("parseConfidence"),
    );
    fingerprint_input.insert("sourceRole".to_string(), source_role.clone());
    fingerprint_input.insert("sourceRoleStatus".to_string(), source_role_status.clone());
    fingerprint_input.insert(
        "canCreateSchedulableRows".to_string(),
        json!(can_create_schedulable_rows),
    );
    fingerprint_input.insert("supportOnly".to_string(), json!(support_only));
    fingerprint_input.insert(
        "parsedUwCourseCodes".to_string(),
        parsed_uw_course_codes.clone(),
    );
    fingerprint_input.insert(
        "sourceOnlyUwCourseCodes".to_string(),
        source_only_uw_course_codes.clone(),
    );
    fingerprint_input.insert(
        "structuredOnlyUwCourseCodes".to_string(),
        structured_only_uw_course_codes.clone(),
    );
    fingerprint_input.insert(
        "approvedFilterUwCourseCodes".to_string(),
        approved_filter_uw_course_codes.clone(),
    );
    fingerprint_input.insert(
        "electiveListUwCourseCodes".to_string(),
        elective_list_uw_course_codes.clone(),
    );
    fingerprint_input.insert(
        "supportOnlyUwCourseCodes".to_string(),
        support_only_uw_course_codes.clone(),
    );
    fingerprint_input.insert("extractedHeadings".to_string(), extracted_headings.clone());
    fingerprint_input.insert(
        "requirementCueLines".to_string(),
        requirement_cue_lines.clone(),
    );
    fingerprint_input.insert("chooseStatements".to_string(), choose_statements.clone());
    fingerprint_input.insert(
        "qualitySignalCodes".to_string(),
        json!(quality_signal_codes),
    );
    insert_defined_field(&mut fingerprint_input, "error", owner.get("error"));

    let primary_source_url =
        nullish_or_fingerprint(owner.get("primarySourceUrl"), owner.get("sourceUrl"));
    let primary_source_label =
        nullish_or_fingerprint(owner.get("primarySourceLabel"), owner.get("sourceLabel"));
    let parse_confidence = nullish_field(owner.get("parseConfidence"))
        .cloned()
        .unwrap_or_else(|| json!("low"));
    let support_lists = field("supportLists");

    let mut entry = Map::new();
    for key in ["ownerId", "ownerTitle", "planId"] {
        insert_defined_field(&mut entry, key, owner.get(key));
    }
    entry.insert(
        "pathwayId".to_string(),
        nullish_field(owner.get("pathwayId"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    for key in ["campusId", "parserType", "sourceUrl", "sourceLabel"] {
        insert_defined_field(&mut entry, key, owner.get(key));
    }
    insert_defined_field(&mut entry, "primarySourceUrl", primary_source_url);
    insert_defined_field(&mut entry, "primarySourceLabel", primary_source_label);
    entry.insert("sourceRole".to_string(), source_role);
    entry.insert("sourceRoleStatus".to_string(), source_role_status);
    entry.insert(
        "canCreateSchedulableRows".to_string(),
        json!(can_create_schedulable_rows),
    );
    entry.insert(
        "canCreateRequiredRows".to_string(),
        json!(owner.get("canCreateRequiredRows").and_then(Value::as_bool) == Some(true)),
    );
    entry.insert(
        "canCreateScheduleRows".to_string(),
        json!(owner.get("canCreateScheduleRows").and_then(Value::as_bool) == Some(true)),
    );
    entry.insert("supportOnly".to_string(), json!(support_only));
    entry.insert(
        "nonSchedulable".to_string(),
        json!(owner.get("nonSchedulable").and_then(Value::as_bool) == Some(true)),
    );
    entry.insert(
        "ok".to_string(),
        json!(owner.get("ok").is_some_and(js_truthy_fingerprint)),
    );
    entry.insert("parseConfidence".to_string(), parse_confidence);
    insert_js_length_field(
        &mut entry,
        "parsedUwCourseCodeCount",
        &parsed_uw_course_codes,
    );
    insert_js_length_field(
        &mut entry,
        "sourceOnlyUwCourseCodeCount",
        &source_only_uw_course_codes,
    );
    insert_js_length_field(
        &mut entry,
        "structuredOnlyUwCourseCodeCount",
        &structured_only_uw_course_codes,
    );
    insert_js_length_field(
        &mut entry,
        "approvedFilterUwCourseCodeCount",
        &approved_filter_uw_course_codes,
    );
    insert_js_length_field(
        &mut entry,
        "electiveListUwCourseCodeCount",
        &elective_list_uw_course_codes,
    );
    insert_js_length_field(
        &mut entry,
        "supportOnlyUwCourseCodeCount",
        &support_only_uw_course_codes,
    );
    insert_js_length_field(&mut entry, "supportListCount", &support_lists);
    insert_js_length_field(&mut entry, "extractedHeadingCount", &extracted_headings);
    insert_js_length_field(
        &mut entry,
        "requirementCueLineCount",
        &requirement_cue_lines,
    );
    insert_js_length_field(&mut entry, "chooseStatementCount", &choose_statements);
    entry.insert(
        "qualitySignalCodes".to_string(),
        json!(quality_signal_codes),
    );
    entry.insert(
        "qualityWarningCount".to_string(),
        json!(quality_warning_count),
    );
    entry.insert("qualityNoteCount".to_string(), json!(quality_note_count));
    entry.insert(
        "requirementFingerprint".to_string(),
        json!(sha256_json(&Value::Object(fingerprint_input))),
    );
    entry.insert("parsedUwCourseCodes".to_string(), parsed_uw_course_codes);
    entry.insert(
        "sourceOnlyUwCourseCodes".to_string(),
        source_only_uw_course_codes,
    );
    entry.insert(
        "structuredOnlyUwCourseCodes".to_string(),
        structured_only_uw_course_codes,
    );
    entry.insert(
        "approvedFilterUwCourseCodes".to_string(),
        approved_filter_uw_course_codes,
    );
    entry.insert(
        "electiveListUwCourseCodes".to_string(),
        elective_list_uw_course_codes,
    );
    entry.insert(
        "supportOnlyUwCourseCodes".to_string(),
        support_only_uw_course_codes,
    );
    Value::Object(entry)
}

pub fn get_requirement_fingerprint_compare_key(entry: &Value) -> String {
    let owner_id = js_entry_key(entry, "ownerId");
    let source_url = {
        let u1 = normalize_source_url_key(entry.get("sourceUrl"));
        if !u1.is_empty() {
            u1
        } else {
            let u2 = normalize_source_url_key(entry.get("primarySourceUrl"));
            if !u2.is_empty() {
                u2
            } else {
                "unknown-source".to_string()
            }
        }
    };
    let parser_type = match js_entry_key(entry, "parserType") {
        value if value.is_empty() => "unknown-parser".to_string(),
        value => value,
    };
    let source_role = match js_entry_key(entry, "sourceRole") {
        value if value.is_empty() => "unknown-role".to_string(),
        value => value,
    };
    if !owner_id.is_empty() {
        format!("{owner_id}\0{source_url}\0{parser_type}\0{source_role}")
    } else {
        String::new()
    }
}

pub fn compare_fingerprints<F>(
    previous_entries: &[Value],
    current_entries: &[Value],
    key_getter: F,
    hash_name: &str,
) -> FingerprintDiff
where
    F: Fn(&Value) -> String,
{
    let normalized_key = |entry: &Value| js_trim(&key_getter(entry)).to_string();
    let mut previous_by_key = HashMap::new();
    for entry in previous_entries {
        previous_by_key.insert(normalized_key(entry), entry);
    }
    let mut current_by_key = HashMap::new();
    for entry in current_entries {
        current_by_key.insert(normalized_key(entry), entry);
    }

    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();
    let mut removed = Vec::new();

    for current in current_entries {
        let key = normalized_key(current);
        if let Some(previous) = previous_by_key
            .get(&key)
            .filter(|previous| js_truthy_fingerprint(previous))
        {
            let prev_hash = previous.get(hash_name);
            let curr_hash = current.get(hash_name);
            if !js_strict_equal(prev_hash, curr_hash) {
                changed.push(current.clone());
            } else {
                unchanged.push(current.clone());
            }
        } else {
            added.push(current.clone());
        }
    }

    for previous in previous_entries {
        let key = normalized_key(previous);
        if !current_by_key.contains_key(&key) {
            removed.push(previous.clone());
        }
    }

    FingerprintDiff {
        added,
        changed,
        unchanged,
        removed,
    }
}

pub fn add_requirement_backed_source_fingerprints(
    source_fingerprints: &[Value],
    requirement_owners: &[Value],
    titles_by_plan_id: &HashMap<String, String>,
    current_generated_source_owner_ids_by_url: &HashMap<String, Vec<String>>,
    repo_root: &Path,
) -> Vec<Value> {
    let mut entries = source_fingerprints.to_vec();
    let mut entry_by_url: HashMap<String, usize> = HashMap::new();
    let mut current_owner_ids_by_url: HashMap<String, Vec<String>> = HashMap::new();

    for owner in requirement_owners {
        let normalized_source_url = normalize_source_url_key(owner.get("sourceUrl"));
        if normalized_source_url.is_empty() {
            continue;
        }

        let labels: Vec<Value> = owner
            .get("sourceLabel")
            .filter(|label| js_truthy_fingerprint(label))
            .cloned()
            .into_iter()
            .collect();
        let source_stub = json!({
            "url": owner.get("sourceUrl"),
            "finalUrl": owner.get("sourceUrl"),
            "labels": labels,
            "title": owner.get("extractedTitle"),
        });

        let oid = owner
            .get("ownerId")
            .filter(|owner_id| !owner_id.is_null())
            .map(js_fingerprint_string)
            .unwrap_or_default();
        let normalized_owner_ids = normalize_and_filter_source_owner_ids(
            Some(&json!([oid])),
            &source_stub,
            titles_by_plan_id,
        );
        if normalized_owner_ids.is_empty() {
            continue;
        }

        let existing = current_owner_ids_by_url
            .entry(normalized_source_url)
            .or_default();
        let existing_values: Vec<Value> = existing.iter().map(|s| json!(s)).collect();
        let new_values: Vec<Value> = normalized_owner_ids.iter().map(|s| json!(s)).collect();
        *existing = merge_unique_strings(&existing_values, &new_values);
    }

    let mut current_source_owner_ids_by_url = current_generated_source_owner_ids_by_url.clone();
    for (url, owner_ids) in &current_owner_ids_by_url {
        let existing = current_source_owner_ids_by_url
            .entry(url.clone())
            .or_default();
        let existing_values: Vec<Value> = existing.iter().map(|s| json!(s)).collect();
        let new_values: Vec<Value> = owner_ids.iter().map(|s| json!(s)).collect();
        *existing = merge_unique_strings(&existing_values, &new_values);
    }

    let mut add_lookup = |entry_idx: usize, entry: &Value, map: &mut HashMap<String, usize>| {
        for lookup_url in [entry.get("url"), entry.get("finalUrl")] {
            let normalized = normalize_source_url_key(lookup_url);
            if !normalized.is_empty() && !map.contains_key(&normalized) {
                map.insert(normalized, entry_idx);
            }
        }
    };

    for (idx, entry) in entries.iter().enumerate() {
        add_lookup(idx, entry, &mut entry_by_url);
    }

    for owner in requirement_owners {
        let normalized_source_url = normalize_source_url_key(owner.get("sourceUrl"));
        if normalized_source_url.is_empty() {
            continue;
        }

        let labels: Vec<Value> = owner
            .get("sourceLabel")
            .filter(|label| js_truthy_fingerprint(label))
            .cloned()
            .into_iter()
            .collect();
        let source_stub = json!({
            "url": owner.get("sourceUrl"),
            "finalUrl": owner.get("sourceUrl"),
            "labels": labels,
            "title": owner.get("extractedTitle"),
        });

        let oid = owner
            .get("ownerId")
            .filter(|owner_id| !owner_id.is_null())
            .map(js_fingerprint_string)
            .unwrap_or_default();
        let normalized_owner_ids = normalize_and_filter_source_owner_ids(
            Some(&json!([oid])),
            &source_stub,
            titles_by_plan_id,
        );
        if normalized_owner_ids.is_empty() {
            continue;
        }

        let entry_idx = if let Some(&idx) = entry_by_url.get(&normalized_source_url) {
            idx
        } else {
            let new_entry = build_requirement_backed_source_fingerprint_entry(
                owner,
                Some(normalized_owner_ids.clone()),
                repo_root,
            );
            entries.push(new_entry);
            let idx = entries.len() - 1;
            add_lookup(idx, &entries[idx], &mut entry_by_url);
            idx
        };

        let current_owner_ids = current_source_owner_ids_by_url
            .get(&normalized_source_url)
            .cloned()
            .unwrap_or_default();
        if !current_owner_ids.is_empty() {
            let current_owner_id_set: HashSet<&str> =
                current_owner_ids.iter().map(String::as_str).collect();
            let current_plan_ids: HashSet<String> = current_owner_ids
                .iter()
                .map(|id| get_plan_id_from_owner_id(id))
                .filter(|p| !p.is_empty())
                .collect();
            let existing_owners = entries[entry_idx]
                .get("ownerIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let filtered_owners: Vec<Value> = existing_owners
                .into_iter()
                .filter(|o| {
                    if let Some(id) = o.as_str() {
                        let plan_id = get_plan_id_from_owner_id(id);
                        !current_plan_ids.contains(&plan_id) || current_owner_id_set.contains(id)
                    } else {
                        false
                    }
                })
                .collect();
            let current_values: Vec<Value> =
                current_owner_ids.into_iter().map(|s| json!(s)).collect();
            let merged = merge_unique_strings(&filtered_owners, &current_values);
            if let Some(obj) = entries[entry_idx].as_object_mut() {
                obj.insert("ownerIds".to_string(), json!(merged));
            }
        }

        let entry = &mut entries[entry_idx];
        if let Some(obj) = entry.as_object_mut() {
            let prev_labels = obj
                .get("labels")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let new_label = owner.get("sourceLabel").cloned().unwrap_or(Value::Null);
            obj.insert(
                "labels".to_string(),
                json!(merge_unique_strings(&prev_labels, &[new_label])),
            );

            let prev_owners = obj
                .get("ownerIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let new_owners: Vec<Value> = normalized_owner_ids.iter().map(|s| json!(s)).collect();
            obj.insert(
                "ownerIds".to_string(),
                json!(merge_unique_strings(&prev_owners, &new_owners)),
            );

            let prev_kinds = obj
                .get("kinds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let pathway_id_present = !owner
                .get("pathwayId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty();
            let new_kind = json!(if pathway_id_present {
                "pathway"
            } else {
                "major"
            });
            obj.insert(
                "kinds".to_string(),
                json!(merge_unique_strings(&prev_kinds, &[new_kind])),
            );
        }
    }

    let mut filtered_entries = Vec::new();
    for mut entry in entries {
        let allowed_owner_ids = {
            let u1 = normalize_source_url_key(entry.get("url"));
            let u2 = normalize_source_url_key(entry.get("finalUrl"));
            let l1 = current_source_owner_ids_by_url
                .get(&u1)
                .cloned()
                .unwrap_or_default();
            let l2 = current_source_owner_ids_by_url
                .get(&u2)
                .cloned()
                .unwrap_or_default();
            let v1: Vec<Value> = l1.into_iter().map(|s| json!(s)).collect();
            let v2: Vec<Value> = l2.into_iter().map(|s| json!(s)).collect();
            merge_unique_strings(&v1, &v2)
        };
        let allowed_owner_id_set: HashSet<String> = allowed_owner_ids.into_iter().collect();

        if let Some(obj) = entry.as_object_mut() {
            let existing_owners = obj
                .get("ownerIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let new_owners: Vec<Value> = if !allowed_owner_id_set.is_empty() {
                existing_owners
                    .into_iter()
                    .filter(|o| {
                        if let Some(id) = o.as_str() {
                            !is_transfer_planner_uw_owner_id(id)
                                || allowed_owner_id_set.contains(id)
                        } else {
                            false
                        }
                    })
                    .collect()
            } else {
                existing_owners
                    .into_iter()
                    .filter(|o| {
                        if let Some(id) = o.as_str() {
                            !is_transfer_planner_uw_owner_id(id)
                        } else {
                            false
                        }
                    })
                    .collect()
            };
            obj.insert("ownerIds".to_string(), json!(new_owners));
        }

        let owner_count = entry
            .get("ownerIds")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        let has_major_or_pathway = entry
            .get("kinds")
            .and_then(Value::as_array)
            .map(|kinds| {
                kinds
                    .iter()
                    .any(|k| k.as_str() == Some("major") || k.as_str() == Some("pathway"))
            })
            .unwrap_or(false);

        if owner_count > 0 || !has_major_or_pathway {
            filtered_entries.push(entry);
        }
    }

    filtered_entries.sort_by(|left, right| {
        let l = left.get("url").and_then(Value::as_str).unwrap_or("");
        let r = right.get("url").and_then(Value::as_str).unwrap_or("");
        locale_compare_fingerprint(l, r)
    });

    filtered_entries
}

pub fn is_support_only_requirement_fingerprint(entry: &Value) -> bool {
    entry.get("supportOnly").and_then(Value::as_bool) == Some(true)
        || entry.get("sourceRoleStatus").and_then(Value::as_str) == Some("support")
        || entry
            .get("canCreateSchedulableRows")
            .and_then(Value::as_bool)
            == Some(false)
        || entry.get("canCreateScheduleRows").and_then(Value::as_bool) == Some(false)
}

pub fn has_no_usable_parsed_courses(entry: &Value) -> bool {
    match nullish_field(entry.get("parsedUwCourseCodeCount")) {
        None => true,
        Some(Value::Number(count)) => count.as_f64() == Some(0.0),
        Some(_) => false,
    }
}

pub fn is_parent_plan_requirement_fingerprint(entry: &Value) -> bool {
    let owner_id = js_entry_key(entry, "ownerId");
    let plan_id = js_entry_key(entry, "planId");
    let has_pathway_id = entry.get("pathwayId").is_some_and(js_truthy_fingerprint);
    !owner_id.is_empty() && !plan_id.is_empty() && owner_id == plan_id && !has_pathway_id
}

pub fn is_child_pathway_requirement_fingerprint(entry: &Value, target_plan_id: &str) -> bool {
    let owner_id = js_entry_key(entry, "ownerId");
    let plan_id = js_entry_key(entry, "planId");
    let has_pathway_id = entry.get("pathwayId").is_some_and(js_truthy_fingerprint);
    if owner_id.is_empty()
        || target_plan_id.is_empty()
        || plan_id != target_plan_id
        || owner_id == target_plan_id
    {
        return false;
    }
    has_pathway_id || owner_id.starts_with(&format!("{target_plan_id}:pathway:"))
}

pub fn build_plan_child_pathway_coverage(
    requirement_source_fingerprints: &[Value],
) -> HashMap<String, PlanChildPathwayCoverage> {
    let mut coverage_by_plan_id: HashMap<String, (HashSet<String>, JsPlusValue)> = HashMap::new();
    for entry in requirement_source_fingerprints {
        let plan_id = js_entry_key(entry, "planId");
        if !is_child_pathway_requirement_fingerprint(entry, &plan_id) {
            continue;
        }
        if is_support_only_requirement_fingerprint(entry) {
            continue;
        }
        let count = nullish_field(entry.get("parsedUwCourseCodeCount"))
            .cloned()
            .unwrap_or_else(|| json!(0));
        if js_number_fingerprint(&count).is_some_and(|count| count <= 0.0) {
            continue;
        }
        let owner_id = entry
            .get("ownerId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let entry_cov = coverage_by_plan_id
            .entry(plan_id.to_string())
            .or_insert_with(|| (HashSet::new(), JsPlusValue::Number(0.0)));
        entry_cov.0.insert(owner_id);
        entry_cov.1.add(&count);
    }

    let mut result = HashMap::new();
    for (plan_id, (owners, count)) in coverage_by_plan_id {
        let mut sorted_owners: Vec<String> = owners.into_iter().collect();
        sorted_owners.sort_by(|left, right| locale_compare_fingerprint(left, right));
        result.insert(
            plan_id,
            PlanChildPathwayCoverage {
                child_owner_ids: sorted_owners,
                parsed_uw_course_code_count: count.into_json(),
            },
        );
    }
    result
}

pub fn get_child_pathway_coverage_for_parent(
    entry: Option<&Value>,
    plan_child_pathway_coverage_by_plan_id: &HashMap<String, PlanChildPathwayCoverage>,
) -> Option<PlanChildPathwayCoverage> {
    let entry = entry?;
    if !is_parent_plan_requirement_fingerprint(entry) {
        return None;
    }
    if js_number_with_nullish_zero(entry.get("parsedUwCourseCodeCount"))
        .is_some_and(|count| count > 0.0)
    {
        return None;
    }
    let plan_id = js_entry_key(entry, "planId");
    let coverage = plan_child_pathway_coverage_by_plan_id.get(&plan_id)?;
    if !coverage.child_owner_ids.is_empty() {
        Some(coverage.clone())
    } else {
        None
    }
}

pub fn parse_confidence_rank(value: Option<&str>) -> usize {
    match value {
        Some("high") => 3,
        Some("medium") => 2,
        Some("low") => 1,
        _ => 0,
    }
}

pub fn extract_year_signals(value: &str) -> Vec<i64> {
    static YEAR_PATTERN: OnceLock<Regex> = OnceLock::new();
    let yp = YEAR_PATTERN.get_or_init(|| Regex::new(r"(?-u:\b)(20[0-9]{2})(?-u:\b)").unwrap());
    static AY_PATTERN: OnceLock<Regex> = OnceLock::new();
    let ap = AY_PATTERN.get_or_init(|| {
        Regex::new(
            r"(?-u:\b)(?:(?i:ay|academic[-\t\n\x0b\f\r \x{00a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]*year))?[\t\n\x0b\f\r \x{00a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]*([0-9]{2})[_/-]([0-9]{2})(?-u:\b)",
        )
        .unwrap()
    });
    let mut years = Vec::new();
    for cap in yp.captures_iter(value) {
        if let Some(m) = cap.get(1) {
            if let Ok(y) = m.as_str().parse::<i64>() {
                years.push(y);
            }
        }
    }
    for cap in ap.captures_iter(value) {
        if let (Some(m1), Some(m2)) = (cap.get(1), cap.get(2)) {
            if let Ok(s) = m1.as_str().parse::<i64>() {
                years.push(2000 + s);
            }
            if let Ok(e) = m2.as_str().parse::<i64>() {
                years.push(2000 + e);
            }
        }
    }
    years
}

pub fn get_max_year_signal(entry: &Value) -> Option<i64> {
    let fields = [
        "sourceUrl",
        "sourceLabel",
        "primarySourceUrl",
        "primarySourceLabel",
    ]
    .into_iter()
    .map(|key| entry.get(key).cloned().unwrap_or(Value::Null))
    .collect::<Vec<_>>();
    let combined = js_join_array(Some(&Value::Array(fields)), " ").unwrap_or_default();
    let years = extract_year_signals(&combined);
    years.into_iter().max()
}

pub fn source_urls_look_like_siblings(previous_url: &str, current_url: &str) -> bool {
    let Ok(prev_parsed) = Url::parse(previous_url) else {
        return false;
    };
    let Ok(curr_parsed) = Url::parse(current_url) else {
        return false;
    };
    if prev_parsed.host_str() != curr_parsed.host_str() {
        return false;
    }
    let prev_segments: Vec<&str> = prev_parsed
        .path()
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let curr_segments: Vec<&str> = curr_parsed
        .path()
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let shared_count = prev_segments
        .iter()
        .zip(curr_segments.iter())
        .filter(|(a, b)| a == b)
        .count();
    let threshold = 1.max(3.min(prev_segments.len().saturating_sub(1)));
    shared_count >= threshold
}

pub fn is_current_year_sibling_promotion(
    previous_entry: Option<&Value>,
    current_entry: Option<&Value>,
) -> bool {
    let (Some(prev), Some(curr)) = (previous_entry, current_entry) else {
        return false;
    };
    let prev_url = prev.get("sourceUrl").and_then(Value::as_str).unwrap_or("");
    let curr_url = curr.get("sourceUrl").and_then(Value::as_str).unwrap_or("");
    if prev_url.is_empty() || curr_url.is_empty() || prev_url == curr_url {
        return false;
    }
    let prev_year = get_max_year_signal(prev);
    let curr_year = get_max_year_signal(curr);
    let prev_course_count = js_number_with_nullish_zero(prev.get("parsedUwCourseCodeCount"));
    let curr_course_count = js_number_with_nullish_zero(curr.get("parsedUwCourseCodeCount"));
    let prev_warning_count = js_number_with_nullish_zero(prev.get("qualityWarningCount"));
    let curr_warning_count = js_number_with_nullish_zero(curr.get("qualityWarningCount"));

    let equal_or_better_coverage = match (
        curr_course_count,
        prev_course_count,
        curr_warning_count,
        prev_warning_count,
    ) {
        (Some(curr_courses), Some(prev_courses), Some(curr_warnings), Some(prev_warnings)) => {
            let warning_limit = if prev_warnings.is_nan() {
                f64::NAN
            } else {
                prev_warnings.max(1.0)
            };
            curr_courses >= prev_courses && curr_warnings <= warning_limit
        }
        _ => false,
    };

    equal_or_better_coverage
        && source_urls_look_like_siblings(prev_url, curr_url)
        && (prev_year.is_none() || curr_year.is_none() || curr_year >= prev_year)
}

pub fn classify_action_status(
    category: &str,
    previous_entry: Option<&Value>,
    current_entry: Option<&Value>,
    course_delta_added: usize,
    course_delta_removed: usize,
    child_pathway_coverage_present: bool,
) -> &'static str {
    if category == "support-only-source-changed" {
        return "generated-evidence-only";
    }
    if category == "source-became-weak-or-overview" {
        return "needs-discovery-rule";
    }
    if category == "source-structure-changed" {
        if child_pathway_coverage_present {
            return "generated-evidence-only";
        }
        let curr_warn = current_entry
            .and_then(|entry| entry.get("qualityWarningCount"))
            .and_then(js_number_fingerprint);
        let prev_warn = previous_entry
            .and_then(|entry| entry.get("qualityWarningCount"))
            .and_then(js_number_fingerprint);
        return if curr_warn
            .zip(prev_warn)
            .is_some_and(|(current, previous)| current > previous)
        {
            "needs-parser-rule"
        } else {
            "auto-applied"
        };
    }
    if category == "current-year-sibling-found" {
        return "auto-applied";
    }
    if category == "new-course-added" {
        return if course_delta_added > 0 {
            "auto-applied"
        } else {
            "no-op"
        };
    }
    if category == "course-removed" {
        return if course_delta_removed > 0 {
            "auto-applied"
        } else {
            "no-op"
        };
    }
    "auto-applied"
}

pub fn recommended_actions_for_change(
    category: &str,
    current_entry: Option<&Value>,
    child_pathway_coverage_present: bool,
) -> Vec<String> {
    match category {
        "new-course-added" => {
            let schedulable_false = current_entry
                .and_then(|e| e.get("canCreateSchedulableRows"))
                .and_then(Value::as_bool)
                == Some(false);
            vec![
                "targeted-requirement-parse".to_string(),
                "equivalency-mapping-refresh".to_string(),
                if schedulable_false {
                    "preserve-as-support-evidence".to_string()
                } else {
                    "student-runtime-regeneration".to_string()
                },
                "source-fingerprint-refresh".to_string(),
            ]
        }
        "course-removed" => vec![
            "targeted-requirement-parse".to_string(),
            "verify-alternate-official-support".to_string(),
            "student-runtime-regeneration".to_string(),
            "source-fingerprint-refresh".to_string(),
        ],
        "source-structure-changed" => {
            if child_pathway_coverage_present {
                vec![
                    "preserve-parent-overview-evidence".to_string(),
                    "targeted-requirement-parse".to_string(),
                    "source-fingerprint-refresh".to_string(),
                ]
            } else {
                vec![
                    "targeted-parser-recovery".to_string(),
                    "targeted-requirement-parse".to_string(),
                    "source-fingerprint-refresh".to_string(),
                ]
            }
        }
        "current-year-sibling-found" => vec![
            "high-confidence-source-promotion".to_string(),
            "targeted-requirement-parse".to_string(),
            "source-fingerprint-refresh".to_string(),
        ],
        "source-became-weak-or-overview" => vec![
            "targeted-source-discovery".to_string(),
            "primary-source-review-queue".to_string(),
            "targeted-parser-recovery".to_string(),
        ],
        "support-only-source-changed" => vec![
            "regenerate-support-filters".to_string(),
            "preserve-support-only-no-required-rows".to_string(),
            "source-fingerprint-refresh".to_string(),
        ],
        _ => vec!["source-fingerprint-refresh".to_string()],
    }
}

pub fn normalize_course_codes(values: Option<&Value>) -> Vec<String> {
    match values {
        Some(Value::Array(arr)) => {
            let list = arr
                .iter()
                .map(|value| {
                    if value.is_null() {
                        String::new()
                    } else {
                        js_fingerprint_string(value)
                    }
                })
                .map(|value| js_trim(&value).to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            unique_sorted(list)
        }
        _ => Vec::new(),
    }
}

pub fn diff_values(previous_values: &[Value], current_values: &[Value]) -> Value {
    let prev_set: HashSet<String> = previous_values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let curr_set: HashSet<String> = current_values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let mut added: Vec<String> = curr_set.difference(&prev_set).cloned().collect();
    added.sort_by(|left, right| locale_compare_fingerprint(left, right));
    let mut removed: Vec<String> = prev_set.difference(&curr_set).cloned().collect();
    removed.sort_by(|left, right| locale_compare_fingerprint(left, right));
    json!({
        "added": added,
        "removed": removed,
    })
}

pub fn get_quality_signal_delta(
    previous_entry: Option<&Value>,
    current_entry: Option<&Value>,
) -> Value {
    let prev = previous_entry
        .and_then(|e| e.get("qualitySignalCodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let curr = current_entry
        .and_then(|e| e.get("qualitySignalCodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    diff_values(&prev, &curr)
}

pub fn get_requirement_fingerprint_summary(entry: Option<&Value>) -> Value {
    let Some(entry) = entry.filter(|entry| js_truthy_fingerprint(entry)) else {
        return Value::Null;
    };
    json!({
        "sourceUrl": entry.get("sourceUrl").cloned().unwrap_or(Value::Null),
        "sourceRole": entry.get("sourceRole").cloned().unwrap_or(Value::Null),
        "sourceRoleStatus": entry.get("sourceRoleStatus").cloned().unwrap_or(Value::Null),
        "parserType": entry.get("parserType").cloned().unwrap_or(Value::Null),
        "ok": entry.get("ok").and_then(Value::as_bool) == Some(true),
        "parseConfidence": entry.get("parseConfidence").cloned().unwrap_or(Value::Null),
        "parsedUwCourseCodeCount": nullish_field(entry.get("parsedUwCourseCodeCount")).cloned().unwrap_or_else(|| json!(0)),
        "sourceOnlyUwCourseCodeCount": nullish_field(entry.get("sourceOnlyUwCourseCodeCount")).cloned().unwrap_or_else(|| json!(0)),
        "structuredOnlyUwCourseCodeCount": nullish_field(entry.get("structuredOnlyUwCourseCodeCount")).cloned().unwrap_or_else(|| json!(0)),
        "approvedFilterUwCourseCodeCount": nullish_field(entry.get("approvedFilterUwCourseCodeCount")).cloned().unwrap_or_else(|| json!(0)),
        "electiveListUwCourseCodeCount": nullish_field(entry.get("electiveListUwCourseCodeCount")).cloned().unwrap_or_else(|| json!(0)),
        "supportOnlyUwCourseCodeCount": nullish_field(entry.get("supportOnlyUwCourseCodeCount")).cloned().unwrap_or_else(|| json!(0)),
        "supportListCount": nullish_field(entry.get("supportListCount")).cloned().unwrap_or_else(|| json!(0)),
        "extractedHeadingCount": nullish_field(entry.get("extractedHeadingCount")).cloned().unwrap_or_else(|| json!(0)),
        "requirementCueLineCount": nullish_field(entry.get("requirementCueLineCount")).cloned().unwrap_or_else(|| json!(0)),
        "chooseStatementCount": nullish_field(entry.get("chooseStatementCount")).cloned().unwrap_or_else(|| json!(0)),
        "qualitySignalCodes": nullish_field(entry.get("qualitySignalCodes")).cloned().unwrap_or_else(|| json!([])),
        "qualityWarningCount": nullish_field(entry.get("qualityWarningCount")).cloned().unwrap_or_else(|| json!(0)),
        "qualityNoteCount": nullish_field(entry.get("qualityNoteCount")).cloned().unwrap_or_else(|| json!(0)),
        "requirementFingerprint": nullish_field(entry.get("requirementFingerprint")).cloned().unwrap_or(Value::Null),
    })
}

pub fn build_source_resource_summary(entry: Option<&Value>) -> Value {
    let Some(entry) = entry.filter(|entry| js_truthy_fingerprint(entry)) else {
        return Value::Null;
    };
    json!({
        "url": entry.get("url").cloned().unwrap_or(Value::Null),
        "finalUrl": entry.get("finalUrl").cloned().unwrap_or(Value::Null),
        "ok": entry.get("ok").and_then(Value::as_bool) == Some(true),
        "status": entry.get("status").cloned().unwrap_or(Value::Null),
        "contentType": entry.get("contentType").cloned().unwrap_or(Value::Null),
        "contentLength": entry.get("contentLength").cloned().unwrap_or(Value::Null),
        "etag": entry.get("etag").cloned().unwrap_or(Value::Null),
        "lastModified": entry.get("lastModified").cloned().unwrap_or(Value::Null),
        "title": entry.get("title").cloned().unwrap_or(Value::Null),
        "fetchMode": entry.get("fetchMode").cloned().unwrap_or(Value::Null),
        "resourceFingerprint": entry.get("resourceFingerprint").cloned().unwrap_or(Value::Null),
    })
}

pub fn get_source_resource_for_requirement<'a>(
    entry: Option<&'a Value>,
    source_by_url: &'a HashMap<String, Value>,
) -> Option<&'a Value> {
    let entry = entry?;
    let u1 = normalize_source_url_key(entry.get("sourceUrl"));
    if let Some(src) = source_by_url.get(&u1).filter(|source| !source.is_null()) {
        return Some(src);
    }
    let u2 = normalize_source_url_key(entry.get("primarySourceUrl"));
    source_by_url.get(&u2).filter(|source| !source.is_null())
}

pub fn build_source_change_record(
    category: &str,
    previous_entry: Option<&Value>,
    current_entry: Option<&Value>,
    previous_source_resource: Option<&Value>,
    current_source_resource: Option<&Value>,
    course_delta: Value,
    signal_delta: Value,
    reason: &str,
    child_pathway_coverage: Option<&PlanChildPathwayCoverage>,
) -> Value {
    let effective_entry = nullish_or_fingerprint(current_entry, previous_entry);
    let added_count = course_delta
        .get("added")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let removed_count = course_delta
        .get("removed")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let action_status = classify_action_status(
        category,
        previous_entry,
        current_entry,
        added_count,
        removed_count,
        child_pathway_coverage.is_some(),
    );

    let effective_support_only = effective_entry
        .map(is_support_only_requirement_fingerprint)
        .unwrap_or(false);

    let prev_headings = previous_entry
        .and_then(|e| e.get("extractedHeadingCount"))
        .cloned()
        .unwrap_or(Value::Null);
    let curr_headings = current_entry
        .and_then(|e| e.get("extractedHeadingCount"))
        .cloned()
        .unwrap_or(Value::Null);
    let heading_delta_num = match (
        current_entry.filter(|entry| js_truthy_fingerprint(entry)),
        previous_entry.filter(|entry| js_truthy_fingerprint(entry)),
    ) {
        (Some(c), Some(p)) => match (
            js_number_with_nullish_zero(c.get("extractedHeadingCount")),
            js_number_with_nullish_zero(p.get("extractedHeadingCount")),
        ) {
            (Some(current), Some(previous)) => js_number_value(current - previous),
            _ => Value::Null,
        },
        _ => Value::Null,
    };

    let prev_cues = previous_entry
        .and_then(|e| e.get("requirementCueLineCount"))
        .cloned()
        .unwrap_or(Value::Null);
    let curr_cues = current_entry
        .and_then(|e| e.get("requirementCueLineCount"))
        .cloned()
        .unwrap_or(Value::Null);
    let prev_chooses = previous_entry
        .and_then(|e| e.get("chooseStatementCount"))
        .cloned()
        .unwrap_or(Value::Null);
    let curr_chooses = current_entry
        .and_then(|e| e.get("chooseStatementCount"))
        .cloned()
        .unwrap_or(Value::Null);

    let coverage_val = child_pathway_coverage
        .map(|cov| {
            json!({
                "childOwnerIds": cov.child_owner_ids,
                "parsedUwCourseCodeCount": cov.parsed_uw_course_code_count,
            })
        })
        .unwrap_or(Value::Null);

    let recommended = recommended_actions_for_change(
        category,
        nullish_or_fingerprint(current_entry, previous_entry),
        child_pathway_coverage.is_some(),
    );

    let auto_applied =
        action_status == "auto-applied" || action_status == "generated-evidence-only";

    let src_url = nullish_or_fingerprint(
        current_entry.and_then(|entry| entry.get("sourceUrl")),
        previous_entry.and_then(|entry| entry.get("sourceUrl")),
    )
    .cloned()
    .unwrap_or(Value::Null);

    let src_role = nullish_or_fingerprint(
        current_entry.and_then(|entry| entry.get("sourceRole")),
        previous_entry.and_then(|entry| entry.get("sourceRole")),
    )
    .cloned()
    .unwrap_or(Value::Null);

    let src_role_status = nullish_or_fingerprint(
        current_entry.and_then(|entry| entry.get("sourceRoleStatus")),
        previous_entry.and_then(|entry| entry.get("sourceRoleStatus")),
    )
    .cloned()
    .unwrap_or(Value::Null);

    json!({
        "changeType": category,
        "ownerId": effective_entry.and_then(|e| e.get("ownerId")).cloned().unwrap_or(Value::Null),
        "ownerTitle": effective_entry.and_then(|e| e.get("ownerTitle")).cloned().unwrap_or(Value::Null),
        "planId": effective_entry.and_then(|e| e.get("planId")).cloned().unwrap_or(Value::Null),
        "pathwayId": effective_entry.and_then(|e| e.get("pathwayId")).cloned().unwrap_or(Value::Null),
        "campusId": effective_entry.and_then(|e| e.get("campusId")).cloned().unwrap_or(Value::Null),
        "sourceUrl": src_url,
        "previousSourceUrl": previous_entry.and_then(|e| e.get("sourceUrl")).cloned().unwrap_or(Value::Null),
        "currentSourceUrl": current_entry.and_then(|e| e.get("sourceUrl")).cloned().unwrap_or(Value::Null),
        "sourceRole": src_role,
        "sourceRoleStatus": src_role_status,
        "supportOnly": effective_support_only,
        "previousFingerprintSummary": get_requirement_fingerprint_summary(previous_entry),
        "currentFingerprintSummary": get_requirement_fingerprint_summary(current_entry),
        "previousSourceResource": build_source_resource_summary(previous_source_resource),
        "currentSourceResource": build_source_resource_summary(current_source_resource),
        "courseDelta": course_delta,
        "headingDelta": {
            "previousHeadingCount": prev_headings,
            "currentHeadingCount": curr_headings,
            "delta": heading_delta_num,
        },
        "sectionDelta": {
            "previousRequirementCueLineCount": prev_cues,
            "currentRequirementCueLineCount": curr_cues,
            "previousChooseStatementCount": prev_chooses,
            "currentChooseStatementCount": curr_chooses,
        },
        "qualitySignalDelta": signal_delta,
        "childPathwayCoverage": coverage_val,
        "recommendedAction": recommended,
        "actionStatus": action_status,
        "autoApplied": auto_applied,
        "reason": reason,
    })
}

pub fn build_requirement_change_records(
    previous_entry: Option<&Value>,
    current_entry: Option<&Value>,
    previous_source_resource: Option<&Value>,
    current_source_resource: Option<&Value>,
    plan_child_pathway_coverage_by_plan_id: &HashMap<String, PlanChildPathwayCoverage>,
) -> Vec<Value> {
    let mut records = Vec::new();
    let prev_codes =
        normalize_course_codes(previous_entry.and_then(|e| e.get("parsedUwCourseCodes")));
    let curr_codes =
        normalize_course_codes(current_entry.and_then(|e| e.get("parsedUwCourseCodes")));
    let prev_code_vals: Vec<Value> = prev_codes.iter().map(|s| json!(s)).collect();
    let curr_code_vals: Vec<Value> = curr_codes.iter().map(|s| json!(s)).collect();
    let course_delta = diff_values(&prev_code_vals, &curr_code_vals);
    let signal_delta = get_quality_signal_delta(previous_entry, current_entry);

    let current_is_support = current_entry
        .map(is_support_only_requirement_fingerprint)
        .unwrap_or(false);
    let previous_is_support = previous_entry
        .map(is_support_only_requirement_fingerprint)
        .unwrap_or(false);
    let current_child_coverage = get_child_pathway_coverage_for_parent(
        current_entry,
        plan_child_pathway_coverage_by_plan_id,
    );

    let added_count = course_delta
        .get("added")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let removed_count = course_delta
        .get("removed")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);

    if current_is_support || previous_is_support {
        let prev_hash = previous_entry.and_then(|e| e.get("requirementFingerprint"));
        let curr_hash = current_entry.and_then(|e| e.get("requirementFingerprint"));
        if added_count > 0 || removed_count > 0 || prev_hash != curr_hash {
            records.push(build_source_change_record(
                "support-only-source-changed",
                previous_entry,
                current_entry,
                previous_source_resource,
                current_source_resource,
                course_delta,
                signal_delta,
                "Support-only official source changed; regenerate filters/evidence without creating required schedule rows.",
                current_child_coverage.as_ref(),
            ));
        }
        return records;
    }

    if is_current_year_sibling_promotion(previous_entry, current_entry) {
        records.push(build_source_change_record(
            "current-year-sibling-found",
            previous_entry,
            current_entry,
            previous_source_resource,
            current_source_resource,
            course_delta.clone(),
            signal_delta.clone(),
            "Owner source URL changed to a sibling/current-year source with equal or better parsed coverage.",
            current_child_coverage.as_ref(),
        ));
    }

    if added_count > 0 {
        records.push(build_source_change_record(
            "new-course-added",
            previous_entry,
            current_entry,
            previous_source_resource,
            current_source_resource,
            course_delta.clone(),
            signal_delta.clone(),
            "New UW course code(s) appeared in the parsed official source.",
            current_child_coverage.as_ref(),
        ));
    }

    if removed_count > 0 {
        records.push(build_source_change_record(
            "course-removed",
            previous_entry,
            current_entry,
            previous_source_resource,
            current_source_resource,
            course_delta.clone(),
            signal_delta.clone(),
            "Previously UW course code(s) disappeared from the parsed official source.",
            current_child_coverage.as_ref(),
        ));
    }

    let prev_conf = parse_confidence_rank(
        previous_entry
            .and_then(|e| e.get("parseConfidence"))
            .and_then(Value::as_str),
    );
    let curr_conf = parse_confidence_rank(
        current_entry
            .and_then(|e| e.get("parseConfidence"))
            .and_then(Value::as_str),
    );
    let warning_increase = match (
        js_number_with_nullish_zero(current_entry.and_then(|e| e.get("qualityWarningCount"))),
        js_number_with_nullish_zero(previous_entry.and_then(|e| e.get("qualityWarningCount"))),
    ) {
        (Some(current), Some(previous)) => current > previous,
        _ => false,
    };

    let cue_delta = match (
        js_number_with_nullish_zero(current_entry.and_then(|e| e.get("requirementCueLineCount"))),
        js_number_with_nullish_zero(previous_entry.and_then(|e| e.get("requirementCueLineCount"))),
    ) {
        (Some(current), Some(previous)) => (current - previous).abs(),
        _ => f64::NAN,
    };

    let heading_delta = match (
        js_number_with_nullish_zero(current_entry.and_then(|e| e.get("extractedHeadingCount"))),
        js_number_with_nullish_zero(previous_entry.and_then(|e| e.get("extractedHeadingCount"))),
    ) {
        (Some(current), Some(previous)) => (current - previous).abs(),
        _ => f64::NAN,
    };

    let signals_added = signal_delta
        .get("added")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let changed_structure = warning_increase
        || signals_added > 0
        || cue_delta >= 5.0
        || heading_delta >= 3.0
        || prev_conf != curr_conf;

    if changed_structure {
        let reason = if current_child_coverage.is_some() {
            "Parent overview structure changed, but same-plan child pathway sources provide schedulable requirement coverage."
        } else {
            "Parsed structure, parser confidence, requirement cues, headings, or quality warnings changed materially."
        };
        records.push(build_source_change_record(
            "source-structure-changed",
            previous_entry,
            current_entry,
            previous_source_resource,
            current_source_resource,
            course_delta.clone(),
            signal_delta.clone(),
            reason,
            current_child_coverage.as_ref(),
        ));
    }

    let prev_has_no = previous_entry
        .map(has_no_usable_parsed_courses)
        .unwrap_or(true);
    let curr_has_no = current_entry
        .map(has_no_usable_parsed_courses)
        .unwrap_or(true);
    let signals_contain_weak = current_entry
        .and_then(|e| e.get("qualitySignalCodes"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter().any(|s| {
                s.as_str() == Some("no-parsed-uw-course-codes")
                    || s.as_str() == Some("low-confidence-parsed-source")
            })
        })
        .unwrap_or(false);

    let became_weak =
        (!prev_has_no && curr_has_no) || curr_conf < prev_conf || signals_contain_weak;

    if became_weak {
        records.push(build_source_change_record(
            "source-became-weak-or-overview",
            previous_entry,
            current_entry,
            previous_source_resource,
            current_source_resource,
            course_delta,
            signal_delta,
            "Official source became weak, overview-only, or stopped yielding schedulable requirement coverage.",
            current_child_coverage.as_ref(),
        ));
    }

    records
}

pub fn build_source_only_change_records(
    source_diff: &FingerprintDiff,
    requirement_diff: &FingerprintDiff,
    previous_source_by_url: &HashMap<String, Value>,
    current_source_by_url: &HashMap<String, Value>,
    previous_requirement_by_owner_id: &HashMap<String, Value>,
    current_requirement_by_owner_id: &HashMap<String, Value>,
    plan_child_pathway_coverage_by_plan_id: &HashMap<String, PlanChildPathwayCoverage>,
) -> Vec<Value> {
    let mut records = Vec::new();
    let mut changed_requirement_owner_ids: HashSet<String> = HashSet::new();
    for entry in requirement_diff
        .changed
        .iter()
        .chain(requirement_diff.added.iter())
        .chain(requirement_diff.removed.iter())
    {
        if let Some(oid) = entry.get("ownerId").and_then(Value::as_str) {
            changed_requirement_owner_ids.insert(oid.to_string());
        }
    }

    for source_entry in source_diff.changed.iter().chain(source_diff.added.iter()) {
        if let Some(owner_ids) = source_entry.get("ownerIds").and_then(Value::as_array) {
            for oid_val in owner_ids {
                let Some(owner_id) = oid_val.as_str() else {
                    continue;
                };
                if changed_requirement_owner_ids.contains(owner_id) {
                    continue;
                }

                let current_entry = current_requirement_by_owner_id.get(owner_id);
                let previous_entry = previous_requirement_by_owner_id
                    .get(owner_id)
                    .filter(|entry| !entry.is_null())
                    .or(current_entry);
                if !current_entry.is_some_and(|entry| js_truthy_fingerprint(entry))
                    && !previous_entry.is_some_and(|entry| js_truthy_fingerprint(entry))
                {
                    continue;
                }

                let src_url_key = normalize_source_url_key(source_entry.get("url"));
                let previous_source_resource = previous_source_by_url
                    .get(&src_url_key)
                    .filter(|source| !source.is_null())
                    .or_else(|| {
                        get_source_resource_for_requirement(previous_entry, previous_source_by_url)
                    });
                let current_source_resource = current_source_by_url
                    .get(&src_url_key)
                    .filter(|source| !source.is_null())
                    .or_else(|| {
                        get_source_resource_for_requirement(current_entry, current_source_by_url)
                    });

                let is_support = current_entry
                    .map(is_support_only_requirement_fingerprint)
                    .unwrap_or(false);
                let category = if is_support {
                    "support-only-source-changed"
                } else {
                    "source-structure-changed"
                };

                let empty_course_delta = json!({ "added": [], "removed": [] });
                let signal_delta = get_quality_signal_delta(previous_entry, current_entry);
                let child_coverage = get_child_pathway_coverage_for_parent(
                    current_entry,
                    plan_child_pathway_coverage_by_plan_id,
                );

                records.push(build_source_change_record(
                    category,
                    previous_entry,
                    current_entry,
                    previous_source_resource,
                    current_source_resource,
                    empty_course_delta,
                    signal_delta,
                    "Official source resource fingerprint changed while parsed requirement facts stayed stable.",
                    child_coverage.as_ref(),
                ));
            }
        }
    }

    records
}

pub fn count_by<F>(records: &[Value], get_key: F) -> Map<String, Value>
where
    F: Fn(&Value) -> String,
{
    let mut result = Map::new();
    for record in records {
        let key = get_key(record);
        let next_count = result.get(&key).and_then(Value::as_i64).unwrap_or(0) + 1;
        result.insert(key, json!(next_count));
    }
    result
}

pub fn build_source_change_classification_report(input: &Value) -> Value {
    let empty_arr = Vec::new();
    let empty_obj = Map::new();

    let previous_fingerprints = input.get("previousFingerprints");
    let prev_sources = previous_fingerprints
        .and_then(|f| f.get("sourceFingerprints"))
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);
    let prev_requirements = previous_fingerprints
        .and_then(|f| f.get("requirementSourceFingerprints"))
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);

    let curr_sources = input
        .get("sourceFingerprints")
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);
    let curr_requirements = input
        .get("requirementSourceFingerprints")
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);

    let source_diff = if let Some(diff) = nullish_field(input.get("sourceDiff")) {
        serde_json::from_value(diff.clone()).unwrap_or_default()
    } else {
        compare_fingerprints(
            prev_sources,
            curr_sources,
            |e| js_entry_key(e, "url"),
            "resourceFingerprint",
        )
    };

    let requirement_diff = if let Some(diff) = nullish_field(input.get("requirementDiff")) {
        serde_json::from_value(diff.clone()).unwrap_or_default()
    } else {
        compare_fingerprints(
            prev_requirements,
            curr_requirements,
            get_requirement_fingerprint_compare_key,
            "requirementFingerprint",
        )
    };

    let mut previous_requirement_by_owner_id = HashMap::new();
    for entry in prev_requirements {
        let owner_id = js_entry_key(entry, "ownerId");
        if !owner_id.is_empty() {
            previous_requirement_by_owner_id.insert(owner_id, entry.clone());
        }
    }
    let mut current_requirement_by_owner_id = HashMap::new();
    for entry in curr_requirements {
        let owner_id = js_entry_key(entry, "ownerId");
        if !owner_id.is_empty() {
            current_requirement_by_owner_id.insert(owner_id, entry.clone());
        }
    }

    let mut previous_source_by_url = HashMap::new();
    for entry in prev_sources {
        let u = normalize_source_url_key(entry.get("url"));
        if !u.is_empty() {
            previous_source_by_url.insert(u, entry.clone());
        }
    }
    let mut current_source_by_url = HashMap::new();
    for entry in curr_sources {
        let u = normalize_source_url_key(entry.get("url"));
        if !u.is_empty() {
            current_source_by_url.insert(u, entry.clone());
        }
    }

    let plan_child_pathway_coverage_by_plan_id =
        build_plan_child_pathway_coverage(curr_requirements);

    let mut records = Vec::new();

    for current_entry in requirement_diff
        .changed
        .iter()
        .chain(requirement_diff.added.iter())
    {
        let owner_id = js_entry_key(current_entry, "ownerId");
        let previous_entry = previous_requirement_by_owner_id.get(&owner_id);
        let previous_source_resource =
            get_source_resource_for_requirement(previous_entry, &previous_source_by_url);
        let current_source_resource =
            get_source_resource_for_requirement(Some(current_entry), &current_source_by_url);

        if previous_entry.is_none() {
            let added_codes = normalize_course_codes(current_entry.get("parsedUwCourseCodes"));
            let is_support = is_support_only_requirement_fingerprint(current_entry);
            let category = if is_support {
                "support-only-source-changed"
            } else if !added_codes.is_empty() {
                "new-course-added"
            } else {
                "source-structure-changed"
            };

            let added_vals: Vec<Value> = added_codes.into_iter().map(|s| json!(s)).collect();
            let course_delta = json!({
                "added": added_vals,
                "removed": [],
            });

            let sigs = current_entry
                .get("qualitySignalCodes")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let signal_delta = json!({
                "added": sigs,
                "removed": [],
            });

            let child_coverage = get_child_pathway_coverage_for_parent(
                Some(current_entry),
                &plan_child_pathway_coverage_by_plan_id,
            );

            records.push(build_source_change_record(
                category,
                None,
                Some(current_entry),
                None,
                current_source_resource,
                course_delta,
                signal_delta,
                "New parsed requirement-source fingerprint appeared for this owner.",
                child_coverage.as_ref(),
            ));
            continue;
        }

        records.extend(build_requirement_change_records(
            previous_entry,
            Some(current_entry),
            previous_source_resource,
            current_source_resource,
            &plan_child_pathway_coverage_by_plan_id,
        ));
    }

    for previous_entry in &requirement_diff.removed {
        let previous_source_resource =
            get_source_resource_for_requirement(Some(previous_entry), &previous_source_by_url);
        let removed_codes = normalize_course_codes(previous_entry.get("parsedUwCourseCodes"));
        let removed_vals: Vec<Value> = removed_codes.into_iter().map(|s| json!(s)).collect();
        let course_delta = json!({
            "added": [],
            "removed": removed_vals,
        });
        let sigs = previous_entry
            .get("qualitySignalCodes")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let signal_delta = json!({
            "added": [],
            "removed": sigs,
        });

        records.push(build_source_change_record(
            "course-removed",
            Some(previous_entry),
            None,
            previous_source_resource,
            None,
            course_delta,
            signal_delta,
            "Parsed requirement-source fingerprint disappeared for this owner.",
            None,
        ));
    }

    records.extend(build_source_only_change_records(
        &source_diff,
        &requirement_diff,
        &previous_source_by_url,
        &current_source_by_url,
        &previous_requirement_by_owner_id,
        &current_requirement_by_owner_id,
        &plan_child_pathway_coverage_by_plan_id,
    ));

    let mut unique_records = unique_in_order(&records);
    let counts_by_change_type = count_by(&unique_records, |r| {
        r.get("changeType")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    });
    let counts_by_action_status = count_by(&unique_records, |r| {
        r.get("actionStatus")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    });

    unique_records.sort_by(|left, right| {
        let l_plan = js_string_nullish_empty(left.get("planId"));
        let r_plan = js_string_nullish_empty(right.get("planId"));
        let l_owner = js_string_nullish_empty(left.get("ownerId"));
        let r_owner = js_string_nullish_empty(right.get("ownerId"));
        let l_type = js_string_nullish_empty(left.get("changeType"));
        let r_type = js_string_nullish_empty(right.get("changeType"));
        locale_compare_fingerprint(&l_plan, &r_plan)
            .then_with(|| locale_compare_fingerprint(&l_owner, &r_owner))
            .then_with(|| locale_compare_fingerprint(&l_type, &r_type))
    });

    let affected_owners: HashSet<&str> = unique_records
        .iter()
        .filter_map(|r| r.get("ownerId").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .collect();
    let affected_plans: HashSet<&str> = unique_records
        .iter()
        .filter_map(|r| r.get("planId").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .collect();

    json!({
        "generatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "totalChangeCount": unique_records.len(),
        "countsByChangeType": Value::Object(counts_by_change_type),
        "countsByActionStatus": Value::Object(counts_by_action_status),
        "affectedOwnerCount": affected_owners.len(),
        "affectedPlanCount": affected_plans.len(),
        "changes": unique_records,
    })
}

pub fn collect_source_diff_owner_ids(entries: &[Value]) -> Vec<String> {
    let mut all_ids = Vec::new();
    for entry in entries {
        if let Some(owner_ids) = entry.get("ownerIds") {
            all_ids.extend(normalize_source_owner_ids(Some(owner_ids)));
        }
    }
    unique_sorted(all_ids)
}

pub fn collect_requirement_diff_owner_ids(entries: &[Value]) -> Vec<String> {
    let list: Vec<String> = entries
        .iter()
        .map(|entry| js_entry_key(entry, "ownerId"))
        .filter(|owner_id| !owner_id.is_empty())
        .collect();
    unique_sorted(list)
}

pub fn build_current_generated_source_owner_ids_by_url(
    repo_root: &Path,
) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();
    let runtime_dir = repo_root
        .join("source")
        .join("constants")
        .join("transfer-planner-source")
        .join("student-runtime.generated");

    let plans_dir = runtime_dir.join("major-plans-by-plan-id");
    if let Ok(entries) = fs::read_dir(plans_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(bytes) = fs::read(&path) {
                    if let Ok(Value::Array(plans)) = serde_json::from_slice(&bytes) {
                        for plan in plans {
                            let plan_id = js_string_nullish_empty(plan.get("id"));
                            if let Some(links) = plan.get("officialLinks").and_then(Value::as_array)
                            {
                                for link in links {
                                    add_current_source_owner_url_entry(
                                        &mut index,
                                        link.get("url"),
                                        &plan_id,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let pathways_dir = runtime_dir.join("pathways-by-plan-id");
    if let Ok(entries) = fs::read_dir(pathways_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(bytes) = fs::read(&path) {
                    if let Ok(Value::Object(map)) = serde_json::from_slice(&bytes) {
                        for (plan_id, pathways) in map {
                            if let Value::Array(list) = pathways {
                                for pathway in list {
                                    let pathway_id = pathway
                                        .get("id")
                                        .map(js_fingerprint_string)
                                        .unwrap_or_else(|| "undefined".to_string());
                                    let owner_key = format!("{plan_id}::{pathway_id}");
                                    if let Some(links) =
                                        pathway.get("officialLinks").and_then(Value::as_array)
                                    {
                                        for link in links {
                                            add_current_source_owner_url_entry(
                                                &mut index,
                                                link.get("url"),
                                                &owner_key,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let tracks_path = runtime_dir.join("tracks.generated.json");
    if let Ok(bytes) = fs::read(&tracks_path) {
        if let Ok(Value::Array(tracks)) = serde_json::from_slice(&bytes) {
            for track in tracks {
                let track_id = js_string_nullish_empty(track.get("id"));
                if let Some(links) = track.get("officialLinks").and_then(Value::as_array) {
                    for link in links {
                        add_current_source_owner_url_entry(&mut index, link.get("url"), &track_id);
                    }
                }
            }
        }
    }

    let mut result = HashMap::with_capacity(index.len());
    for (url, owners) in index {
        let mut sorted: Vec<String> = owners.into_iter().collect();
        javascript_default_sort_strings(&mut sorted);
        result.insert(url, sorted);
    }
    result
}

fn add_current_source_owner_url_entry(
    index: &mut HashMap<String, HashSet<String>>,
    url: Option<&Value>,
    owner_id: &str,
) {
    let normalized_url = normalize_source_url_key(url);
    let normalized_owner_id = normalize_source_owner_id(owner_id);
    if normalized_url.is_empty() || normalized_owner_id.is_empty() {
        return;
    }
    index
        .entry(normalized_url)
        .or_default()
        .insert(normalized_owner_id);
}

pub fn build_report(
    source_snapshot: &Value,
    requirement_report: &Value,
    previous_fingerprints: &Value,
    current_generated_source_owner_ids_by_url: Option<&HashMap<String, Vec<String>>>,
    repo_root: &Path,
) -> Value {
    let empty_owners = Vec::new();
    let requirement_owners = requirement_report
        .get("owners")
        .and_then(Value::as_array)
        .unwrap_or(&empty_owners);

    let titles_by_plan_id = build_requirement_report_titles_by_plan_id(requirement_owners);

    let default_generated_owner_ids;
    let generated_owner_ids = match current_generated_source_owner_ids_by_url {
        Some(m) => m,
        None => {
            default_generated_owner_ids =
                build_current_generated_source_owner_ids_by_url(repo_root);
            &default_generated_owner_ids
        }
    };

    let empty_sources = Vec::new();
    let raw_sources = source_snapshot
        .get("sources")
        .and_then(Value::as_array)
        .unwrap_or(&empty_sources);

    let source_fingerprints_input: Vec<Value> = raw_sources
        .iter()
        .map(|src| build_source_fingerprint_entry(src, &titles_by_plan_id))
        .collect();

    let source_fingerprints = add_requirement_backed_source_fingerprints(
        &source_fingerprints_input,
        requirement_owners,
        &titles_by_plan_id,
        generated_owner_ids,
        repo_root,
    );

    let requirement_source_fingerprints: Vec<Value> = requirement_owners
        .iter()
        .map(build_requirement_fingerprint_entry)
        .collect();

    let empty_prev = Vec::new();
    let prev_source_fingerprints = previous_fingerprints
        .get("sourceFingerprints")
        .and_then(Value::as_array)
        .unwrap_or(&empty_prev);
    let prev_requirement_fingerprints = previous_fingerprints
        .get("requirementSourceFingerprints")
        .and_then(Value::as_array)
        .unwrap_or(&empty_prev);

    let source_diff = compare_fingerprints(
        prev_source_fingerprints,
        &source_fingerprints,
        |entry| js_entry_key(entry, "url"),
        "resourceFingerprint",
    );

    let requirement_diff = compare_fingerprints(
        prev_requirement_fingerprints,
        &requirement_source_fingerprints,
        get_requirement_fingerprint_compare_key,
        "requirementFingerprint",
    );

    let added_source_owner_ids = collect_source_diff_owner_ids(&source_diff.added);
    let changed_source_owner_ids = collect_source_diff_owner_ids(&source_diff.changed);
    let removed_source_owner_ids = collect_source_diff_owner_ids(&source_diff.removed);

    let added_requirement_owner_ids = collect_requirement_diff_owner_ids(&requirement_diff.added);
    let changed_requirement_owner_ids =
        collect_requirement_diff_owner_ids(&requirement_diff.changed);
    let removed_requirement_owner_ids =
        collect_requirement_diff_owner_ids(&requirement_diff.removed);

    let mut touched_source_owner_ids = Vec::new();
    touched_source_owner_ids.extend(added_source_owner_ids.clone());
    touched_source_owner_ids.extend(changed_source_owner_ids.clone());
    touched_source_owner_ids.extend(removed_source_owner_ids.clone());
    let touched_source_owner_ids = unique_sorted(touched_source_owner_ids);

    let mut touched_requirement_owner_ids = Vec::new();
    touched_requirement_owner_ids.extend(added_requirement_owner_ids.clone());
    touched_requirement_owner_ids.extend(changed_requirement_owner_ids.clone());
    touched_requirement_owner_ids.extend(removed_requirement_owner_ids.clone());
    let touched_requirement_owner_ids = unique_sorted(touched_requirement_owner_ids);

    let classification_input = json!({
        "previousFingerprints": previous_fingerprints,
        "sourceFingerprints": &source_fingerprints,
        "requirementSourceFingerprints": &requirement_source_fingerprints,
        "sourceDiff": &source_diff,
        "requirementDiff": &requirement_diff,
    });
    let source_change_classification =
        build_source_change_classification_report(&classification_input);

    let source_fingerprint_owner_ids = unique_sorted(
        source_fingerprints
            .iter()
            .flat_map(|e| normalize_source_owner_ids(e.get("ownerIds"))),
    );

    let requirement_fingerprint_owner_ids = unique_sorted(
        requirement_source_fingerprints
            .iter()
            .filter_map(|e| e.get("ownerId").and_then(Value::as_str)),
    );
    let change_classification_json_path = repo_root
        .join("source")
        .join(".tmp")
        .join("reports")
        .join(DEFAULT_CHANGE_CLASSIFICATION_JSON_FILENAME);
    let change_classification_markdown_path = repo_root
        .join("source")
        .join(".tmp")
        .join("reports")
        .join(DEFAULT_CHANGE_CLASSIFICATION_MD_FILENAME);

    let mut report = json!({
        "generatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "sourceSnapshotGeneratedAt": source_snapshot.get("generatedAt").cloned().unwrap_or(Value::Null),
        "requirementParseReportGeneratedAt": requirement_report.get("generatedAt").cloned().unwrap_or(Value::Null),
        "totalSourceFingerprints": source_fingerprints.len(),
        "totalRequirementSourceFingerprints": requirement_source_fingerprints.len(),
        "changedSourceFingerprintCount": source_diff.changed.len(),
        "changedRequirementFingerprintCount": requirement_diff.changed.len(),
        "addedSourceFingerprintCount": source_diff.added.len(),
        "addedRequirementFingerprintCount": requirement_diff.added.len(),
        "removedSourceFingerprintCount": source_diff.removed.len(),
        "removedRequirementFingerprintCount": requirement_diff.removed.len(),
        "sourceFingerprintOwnerIds": source_fingerprint_owner_ids,
        "requirementFingerprintOwnerIds": requirement_fingerprint_owner_ids,
        "addedSourceOwnerIds": added_source_owner_ids,
        "changedSourceOwnerIds": changed_source_owner_ids,
        "removedSourceOwnerIds": removed_source_owner_ids,
        "touchedSourceOwnerIds": touched_source_owner_ids,
        "addedRequirementOwnerIds": added_requirement_owner_ids,
        "changedRequirementOwnerIds": changed_requirement_owner_ids,
        "removedRequirementOwnerIds": removed_requirement_owner_ids,
        "touchedRequirementOwnerIds": touched_requirement_owner_ids,
        "sourceChangeClassificationSummary": {
            "reportJsonPath": change_classification_json_path.to_string_lossy().to_string(),
            "reportMarkdownPath": change_classification_markdown_path.to_string_lossy().to_string(),
            "totalChangeCount": source_change_classification.get("totalChangeCount").cloned().unwrap_or_else(|| json!(0)),
            "affectedOwnerCount": source_change_classification.get("affectedOwnerCount").cloned().unwrap_or_else(|| json!(0)),
            "affectedPlanCount": source_change_classification.get("affectedPlanCount").cloned().unwrap_or_else(|| json!(0)),
            "countsByChangeType": source_change_classification.get("countsByChangeType").cloned().unwrap_or_else(|| json!({})),
            "countsByActionStatus": source_change_classification.get("countsByActionStatus").cloned().unwrap_or_else(|| json!({})),
        },
        "sourceChangeClassification": source_change_classification,
        "sourceDiff": source_diff,
        "requirementDiff": requirement_diff,
        "sourceFingerprints": source_fingerprints,
        "requirementSourceFingerprints": requirement_source_fingerprints,
    });
    let report_object = report.as_object_mut().expect("report is an object");
    if source_snapshot.get("generatedAt").is_none() {
        report_object.remove("sourceSnapshotGeneratedAt");
    }
    if requirement_report.get("generatedAt").is_none() {
        report_object.remove("requirementParseReportGeneratedAt");
    }
    report
}

pub fn build_generated_file(report: &Value) -> String {
    let empty_array = Value::Array(Vec::new());
    let source_fingerprints_json =
        javascript_json_pretty(report.get("sourceFingerprints").unwrap_or(&empty_array));
    let requirement_fingerprints_json = javascript_json_pretty(
        report
            .get("requirementSourceFingerprints")
            .unwrap_or(&empty_array),
    );

    format!(
        "import type {{ TransferPlannerRequirementSourceFingerprintEntry, TransferPlannerSourceFingerprintEntry }} from \"./schema\";\n\n// Generated by scripts/planner/build-transfer-planner-source-fingerprints.cjs\nexport const TRANSFER_PLANNER_FINGERPRINTS: TransferPlannerSourceFingerprintEntry[] = {source_fingerprints_json};\n\nexport const TRANSFER_PLANNER_REQUIREMENT_FINGERPRINTS: TransferPlannerRequirementSourceFingerprintEntry[] = {requirement_fingerprints_json};\n"
    )
}

pub fn build_source_fingerprints_markdown(
    report: &Value,
    change_classification_md_path: &str,
) -> String {
    let mut lines = vec![
        "# Transfer Planner Source Fingerprints".to_string(),
        "".to_string(),
        format!("Generated: {}", js_display_direct(report.get("generatedAt"))),
        "".to_string(),
        format!("- Source snapshot generated: {}", js_display_direct(report.get("sourceSnapshotGeneratedAt"))),
        format!("- Requirement parse report generated: {}", js_display_direct(report.get("requirementParseReportGeneratedAt"))),
        format!("- Source fingerprints: {}", js_display_direct(report.get("totalSourceFingerprints"))),
        format!("- Requirement-source fingerprints: {}", js_display_direct(report.get("totalRequirementSourceFingerprints"))),
        format!("- Changed source resource fingerprints: {}", js_display_direct(report.get("changedSourceFingerprintCount"))),
        format!("- Changed parsed requirement fingerprints: {}", js_display_direct(report.get("changedRequirementFingerprintCount"))),
        format!("- Added source fingerprints: {}", js_display_direct(report.get("addedSourceFingerprintCount"))),
        format!("- Added requirement fingerprints: {}", js_display_direct(report.get("addedRequirementFingerprintCount"))),
        format!("- Removed source fingerprints: {}", js_display_direct(report.get("removedSourceFingerprintCount"))),
        format!("- Removed requirement fingerprints: {}", js_display_direct(report.get("removedRequirementFingerprintCount"))),
        format!("- Touched source owners: {}", report.get("touchedSourceOwnerIds").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0)),
        format!("- Touched requirement owners: {}", report.get("touchedRequirementOwnerIds").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0)),
        format!("- Classified source changes: {}", js_display_direct(report.get("sourceChangeClassificationSummary").and_then(|summary| summary.get("totalChangeCount")))),
        format!("- Source-change report: {change_classification_md_path}"),
        "".to_string(),
        "Source resource fingerprints track official URL metadata/body hashes.".to_string(),
        "Requirement-source fingerprints track parsed requirement facts separately, so cosmetic page changes do not automatically become planner requirement changes.".to_string(),
        "".to_string(),
    ];

    if let Some(changed) = report
        .get("sourceDiff")
        .and_then(|d| d.get("changed"))
        .and_then(Value::as_array)
    {
        if !changed.is_empty() {
            lines.push("## Changed Source Resources".to_string());
            lines.push("".to_string());
            for entry in changed.iter().take(40) {
                let url = js_display_direct(entry.get("url"));
                let owners = js_join_array(entry.get("ownerIds"), ", ").unwrap_or_default();
                let fp = js_display_direct(entry.get("resourceFingerprint"));
                lines.push(format!("- {url}"));
                lines.push(format!("  - owners: {owners}"));
                lines.push(format!("  - fingerprint: {fp}"));
            }
            lines.push("".to_string());
        }
    }

    if let Some(changed) = report
        .get("requirementDiff")
        .and_then(|d| d.get("changed"))
        .and_then(Value::as_array)
    {
        if !changed.is_empty() {
            lines.push("## Changed Parsed Requirement Facts".to_string());
            lines.push("".to_string());
            for entry in changed.iter().take(40) {
                let title = js_display_direct(entry.get("ownerTitle"));
                let oid = js_display_direct(entry.get("ownerId"));
                let src = js_display_direct(entry.get("sourceUrl"));
                let fp = js_display_direct(entry.get("requirementFingerprint"));
                let src_only = js_display_direct(entry.get("sourceOnlyUwCourseCodeCount"));
                let struct_only = js_display_direct(entry.get("structuredOnlyUwCourseCodeCount"));
                let signals =
                    js_join_array(entry.get("qualitySignalCodes"), ", ").unwrap_or_default();

                lines.push(format!("- {title} ({oid})"));
                lines.push(format!("  - source: {src}"));
                lines.push(format!("  - fingerprint: {fp}"));
                lines.push(format!("  - source-only UW course codes: {src_only}"));
                lines.push(format!(
                    "  - structured-only UW course codes: {struct_only}"
                ));
                if entry
                    .get("qualitySignalCodes")
                    .and_then(Value::as_array)
                    .is_some_and(|codes| !codes.is_empty())
                {
                    lines.push(format!("  - quality signals: {signals}"));
                }
            }
            lines.push("".to_string());
        }
    }

    lines.join("\n")
}

pub fn build_source_change_classification_markdown(change_report: &Value) -> String {
    let mut lines = vec![
        "# Transfer Planner Source Change Classification".to_string(),
        "".to_string(),
        format!(
            "Generated: {}",
            js_display_direct(change_report.get("generatedAt"))
        ),
        "".to_string(),
        format!(
            "- Classified changes: {}",
            js_display_direct(change_report.get("totalChangeCount"))
        ),
        format!(
            "- Affected owners: {}",
            js_display_direct(change_report.get("affectedOwnerCount"))
        ),
        format!(
            "- Affected plans: {}",
            js_display_direct(change_report.get("affectedPlanCount"))
        ),
        "".to_string(),
        "## Change Types".to_string(),
        "".to_string(),
    ];

    if let Some(types) = change_report
        .get("countsByChangeType")
        .and_then(Value::as_object)
    {
        let mut sorted: Vec<(&String, &Value)> = types.iter().collect();
        sorted.sort_by(|(a, _), (b, _)| locale_compare_fingerprint(a, b));
        for (k, v) in sorted {
            lines.push(format!("- {k}: {}", js_display_direct(Some(v))));
        }
    }

    lines.push("".to_string());
    lines.push("## Action Statuses".to_string());
    lines.push("".to_string());

    if let Some(statuses) = change_report
        .get("countsByActionStatus")
        .and_then(Value::as_object)
    {
        let mut sorted: Vec<(&String, &Value)> = statuses.iter().collect();
        sorted.sort_by(|(a, _), (b, _)| locale_compare_fingerprint(a, b));
        for (k, v) in sorted {
            lines.push(format!("- {k}: {}", js_display_direct(Some(v))));
        }
    }

    lines.push("".to_string());

    let changes = change_report.get("changes").and_then(Value::as_array);
    if let Some(changes) = changes {
        if !changes.is_empty() {
            lines.push("## Classified Changes".to_string());
            lines.push("".to_string());
            for change in changes.iter().take(120) {
                let heading_value = nullish_field(change.get("ownerTitle"))
                    .or_else(|| nullish_field(change.get("ownerId")));
                let heading = heading_value
                    .map(js_fingerprint_string)
                    .unwrap_or_else(|| "Unknown owner".to_string());
                lines.push(format!("### {heading}"));
                lines.push("".to_string());
                lines.push(format!(
                    "- Change type: {}",
                    js_display_direct(change.get("changeType"))
                ));
                lines.push(format!(
                    "- Owner: {}",
                    js_display_nullish(change.get("ownerId"), "n/a")
                ));
                lines.push(format!(
                    "- Plan: {}",
                    js_display_nullish(change.get("planId"), "n/a")
                ));
                lines.push(format!(
                    "- Source: {}",
                    js_display_nullish(change.get("sourceUrl"), "n/a")
                ));

                let role = js_display_nullish(change.get("sourceRole"), "n/a");
                let role_status = js_display_nullish(change.get("sourceRoleStatus"), "n/a");
                lines.push(format!("- Source role: {role} ({role_status})"));

                lines.push(format!(
                    "- Action status: {}",
                    js_display_direct(change.get("actionStatus"))
                ));

                let rec = js_join_array(change.get("recommendedAction"), ", ")
                    .unwrap_or_else(|| "none".to_string());
                let rec = if rec.is_empty() {
                    "none".to_string()
                } else {
                    rec
                };
                lines.push(format!("- Recommended action: {rec}"));

                lines.push(format!(
                    "- Reason: {}",
                    js_display_direct(change.get("reason"))
                ));

                let added_courses = js_join_array(
                    change
                        .get("courseDelta")
                        .and_then(|delta| delta.get("added")),
                    ", ",
                )
                .unwrap_or_else(|| "none".to_string());
                let added_courses = if added_courses.is_empty() {
                    "none".to_string()
                } else {
                    added_courses
                };

                let removed_courses = js_join_array(
                    change
                        .get("courseDelta")
                        .and_then(|delta| delta.get("removed")),
                    ", ",
                )
                .unwrap_or_else(|| "none".to_string());
                let removed_courses = if removed_courses.is_empty() {
                    "none".to_string()
                } else {
                    removed_courses
                };

                lines.push(format!(
                    "- Course delta: +{added_courses}; -{removed_courses}"
                ));

                let prev_h = js_display_nullish(
                    change
                        .get("headingDelta")
                        .and_then(|d| d.get("previousHeadingCount")),
                    "n/a",
                );
                let curr_h = js_display_nullish(
                    change
                        .get("headingDelta")
                        .and_then(|d| d.get("currentHeadingCount")),
                    "n/a",
                );
                lines.push(format!("- Heading count: {prev_h} -> {curr_h}"));

                let prev_c = js_display_nullish(
                    change
                        .get("sectionDelta")
                        .and_then(|d| d.get("previousRequirementCueLineCount")),
                    "n/a",
                );
                let curr_c = js_display_nullish(
                    change
                        .get("sectionDelta")
                        .and_then(|d| d.get("currentRequirementCueLineCount")),
                    "n/a",
                );
                lines.push(format!("- Requirement cue lines: {prev_c} -> {curr_c}"));

                let warn_before = js_join_array(
                    change
                        .get("previousFingerprintSummary")
                        .and_then(|summary| summary.get("qualitySignalCodes")),
                    ", ",
                )
                .unwrap_or_else(|| "none".to_string());
                let warn_before = if warn_before.is_empty() {
                    "none".to_string()
                } else {
                    warn_before
                };

                let warn_after = js_join_array(
                    change
                        .get("currentFingerprintSummary")
                        .and_then(|summary| summary.get("qualitySignalCodes")),
                    ", ",
                )
                .unwrap_or_else(|| "none".to_string());
                let warn_after = if warn_after.is_empty() {
                    "none".to_string()
                } else {
                    warn_after
                };

                lines.push(format!("- Quality signals: {warn_before} -> {warn_after}"));

                if let Some(child_cov) = change.get("childPathwayCoverage").filter(|v| !v.is_null())
                {
                    if let Some(owners) = child_cov
                        .get("childOwnerIds")
                        .and_then(Value::as_array)
                        .filter(|owners| !owners.is_empty())
                    {
                        let owners_count = owners.len();
                        let courses_count = child_cov
                            .get("parsedUwCourseCodeCount")
                            .map(js_fingerprint_string)
                            .unwrap_or_else(|| "undefined".to_string());
                        lines.push(format!("- Child pathway coverage: {owners_count} owner(s), {courses_count} parsed UW course code(s)"));
                    }
                }

                let fp_before = js_display_nullish(
                    change
                        .get("previousFingerprintSummary")
                        .and_then(|s| s.get("requirementFingerprint")),
                    "n/a",
                );
                let fp_after = js_display_nullish(
                    change
                        .get("currentFingerprintSummary")
                        .and_then(|s| s.get("requirementFingerprint")),
                    "n/a",
                );
                lines.push(format!("- Fingerprint: {fp_before} -> {fp_after}"));
                lines.push("".to_string());
            }

            if changes.len() > 120 {
                lines.push(format!(
                    "- ... {} additional classified changes omitted from markdown.",
                    changes.len() - 120
                ));
                lines.push("".to_string());
            }
        } else {
            lines.push("## Classified Changes".to_string());
            lines.push("".to_string());
            lines.push("- None.".to_string());
            lines.push("".to_string());
        }
    } else {
        lines.push("## Classified Changes".to_string());
        lines.push("".to_string());
        lines.push("- None.".to_string());
        lines.push("".to_string());
    }

    lines.join("\n")
}

pub fn load_previous_fingerprints(repo_root: &Path) -> Value {
    let ts_path = repo_root
        .join("source")
        .join("constants")
        .join("transfer-planner-source")
        .join("source-fingerprints.generated.ts");
    if !ts_path.is_file() {
        return json!({
            "sourceFingerprints": [],
            "requirementSourceFingerprints": []
        });
    }

    match fs::read_to_string(&ts_path) {
        Ok(content) => {
            let source_fingerprints = parse_exported_json_array_bounded(
                &content,
                "TRANSFER_PLANNER_FINGERPRINTS",
                Some("TRANSFER_PLANNER_REQUIREMENT_FINGERPRINTS"),
            );
            let requirement_fingerprints = parse_exported_json_array_bounded(
                &content,
                "TRANSFER_PLANNER_REQUIREMENT_FINGERPRINTS",
                None,
            );
            let source_parse_failed =
                content.contains("TRANSFER_PLANNER_FINGERPRINTS") && source_fingerprints.is_none();
            let requirement_parse_failed = content
                .contains("TRANSFER_PLANNER_REQUIREMENT_FINGERPRINTS")
                && requirement_fingerprints.is_none();
            if source_parse_failed || requirement_parse_failed {
                println!(
                    "Could not load previous source fingerprints: generated arrays could not be parsed"
                );
                return json!({
                    "sourceFingerprints": [],
                    "requirementSourceFingerprints": []
                });
            }
            json!({
                "sourceFingerprints": source_fingerprints.unwrap_or_default(),
                "requirementSourceFingerprints": requirement_fingerprints.unwrap_or_default(),
            })
        }
        Err(error) => {
            println!("Could not load previous source fingerprints: {error}");
            json!({
                "sourceFingerprints": [],
                "requirementSourceFingerprints": []
            })
        }
    }
}

fn parse_exported_json_array_bounded(
    source: &str,
    var_name: &str,
    _end_marker: Option<&str>,
) -> Option<Vec<Value>> {
    let declaration = format!("export const {var_name}");
    let var_idx = source.match_indices(&declaration).find_map(|(index, _)| {
        let declaration_end = index + declaration.len();
        let name_boundary_ok = !source
            .as_bytes()
            .get(declaration_end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'));
        let line_start = source[..index].rfind('\n').map_or(0, |newline| newline + 1);
        (name_boundary_ok && source[line_start..index].trim().is_empty()).then_some(index)
    })?;
    let search_slice = &source[var_idx..];
    let eq_idx = search_slice.find('=')?;
    let array_start = search_slice[eq_idx..].find('[')? + eq_idx;
    let array_end = find_json_array_end(search_slice.as_bytes(), array_start)?;
    let json_str = search_slice[array_start..array_end].trim();
    serde_json::from_str(json_str).ok()
}

fn find_json_array_end(source: &[u8], start: usize) -> Option<usize> {
    if source.get(start) != Some(&b'[') {
        return None;
    }

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in source.iter().copied().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn run_build_source_fingerprints(repo_root: &Path, args: &[String]) -> Result<Value, String> {
    let source_root = repo_root.join("source");
    let tmp_root = source_root.join(".tmp");
    for category in [
        "reports",
        "snapshots",
        "error_logs",
        "logs",
        "exports",
        "cache",
        "downloads",
        "screenshots",
        "scratch",
        "qa",
        "builds",
    ] {
        fs::create_dir_all(tmp_root.join(category)).map_err(|error| {
            format!(
                "could not create planner temporary directory {}: {error}",
                tmp_root.join(category).display()
            )
        })?;
    }

    let tmp_reports_dir = tmp_root.join("reports");
    let link_snapshot_path = tmp_reports_dir.join(DEFAULT_LINK_SNAPSHOT_FILENAME);
    let requirement_parse_report_path =
        tmp_reports_dir.join(DEFAULT_REQUIREMENT_PARSE_REPORT_FILENAME);
    if args.iter().any(|arg| arg == "--check-sources-first") || !link_snapshot_path.is_file() {
        run_node_prerequisite(
            &source_root,
            "scripts/planner/check-transfer-planner-sources.cjs",
        )?;
    }
    if args.iter().any(|arg| arg == "--parse-requirements-first")
        || !requirement_parse_report_path.is_file()
    {
        run_node_prerequisite(
            &source_root,
            "scripts/planner/parse-transfer-planner-requirement-sources.cjs",
        )?;
    }

    let source_snapshot = read_required_json(&link_snapshot_path, "source-link snapshot")?;
    let requirement_report =
        read_required_json(&requirement_parse_report_path, "requirement parse report")?;
    let previous_fingerprints = load_previous_fingerprints(repo_root);
    let mut report = build_report(
        &source_snapshot,
        &requirement_report,
        &previous_fingerprints,
        None,
        repo_root,
    );

    let output_json_path = tmp_reports_dir.join(DEFAULT_OUTPUT_JSON_FILENAME);
    let output_md_path = tmp_reports_dir.join(DEFAULT_OUTPUT_MD_FILENAME);
    let change_json_path = tmp_reports_dir.join(DEFAULT_CHANGE_CLASSIFICATION_JSON_FILENAME);
    let change_md_path = tmp_reports_dir.join(DEFAULT_CHANGE_CLASSIFICATION_MD_FILENAME);
    if let Some(summary) = report
        .get_mut("sourceChangeClassificationSummary")
        .and_then(Value::as_object_mut)
    {
        summary.insert(
            "reportJsonPath".to_string(),
            json!(change_json_path.to_string_lossy().to_string()),
        );
        summary.insert(
            "reportMarkdownPath".to_string(),
            json!(change_md_path.to_string_lossy().to_string()),
        );
    }

    write_json_with_newline(&output_json_path, &report, "source fingerprints report")?;
    let generated_path = source_root.join(DEFAULT_GENERATED_OUTPUT_RELATIVE_PATH);
    let generated_file = build_generated_file(&report);
    fs::write(&generated_path, generated_file).map_err(|error| {
        format!(
            "could not write generated fingerprint registry {}: {error}",
            generated_path.display()
        )
    })?;
    let markdown = build_source_fingerprints_markdown(&report, &change_md_path.to_string_lossy());
    write_text_with_newline(&output_md_path, &markdown, "source fingerprints Markdown")?;

    let change_report = report.get("sourceChangeClassification").ok_or_else(|| {
        "source fingerprints report omitted source-change classification".to_string()
    })?;
    write_json_with_newline(
        &change_json_path,
        change_report,
        "change classification report",
    )?;
    let change_markdown = build_source_change_classification_markdown(change_report);
    write_text_with_newline(
        &change_md_path,
        &change_markdown,
        "change classification Markdown",
    )?;

    println!("Source fingerprints: {}", report["totalSourceFingerprints"]);
    println!(
        "Requirement-source fingerprints: {}",
        report["totalRequirementSourceFingerprints"]
    );
    println!(
        "Changed source resource fingerprints: {}",
        report["changedSourceFingerprintCount"]
    );
    println!(
        "Changed parsed requirement fingerprints: {}",
        report["changedRequirementFingerprintCount"]
    );
    println!(
        "Classified source changes: {}",
        report["sourceChangeClassificationSummary"]["totalChangeCount"]
    );
    println!("JSON report: {}", output_json_path.display());
    println!("Markdown report: {}", output_md_path.display());
    println!("Source-change report: {}", change_md_path.display());
    println!(
        "Generated fingerprint registry: {}",
        generated_path.display()
    );

    Ok(report)
}

fn run_node_prerequisite(source_root: &Path, script: &str) -> Result<(), String> {
    let failure = format!("Command failed: node {script}");
    let status = Command::new("node")
        .arg(script)
        .current_dir(source_root)
        .status()
        .map_err(|_| failure.clone())?;
    if status.success() {
        Ok(())
    } else {
        Err(failure)
    }
}

fn read_required_json(path: &Path, label: &str) -> Result<Value, String> {
    if !path.is_file() {
        return Err(format!("Missing {label} at {}", path.display()));
    }
    let content = fs::read(path)
        .map_err(|error| format!("could not read {label} at {}: {error}", path.display()))?;
    parse_json_utf8_lossy(&content).map_err(|error| format!("invalid JSON in {label}: {error}"))
}

fn parse_json_utf8_lossy(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let content = String::from_utf8_lossy(bytes);
    serde_json::from_str(&content)
}

fn write_json_with_newline(path: &Path, value: &Value, label: &str) -> Result<(), String> {
    let mut content = javascript_json_pretty(value).into_bytes();
    content.push(b'\n');
    fs::write(path, content)
        .map_err(|error| format!("could not write {label} to {}: {error}", path.display()))
}

fn write_text_with_newline(path: &Path, text: &str, label: &str) -> Result<(), String> {
    let mut output = text.to_string();
    if !output.ends_with('\n') {
        output.push('\n');
    }
    fs::write(path, output)
        .map_err(|error| format!("could not write {label} to {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_text_hashing_uses_utf8_replacement_semantics() {
        assert_eq!(sha256_utf8_lossy(&[0xff]), sha256_text("\u{fffd}"));
    }

    #[test]
    fn report_json_decoding_replaces_invalid_utf8_like_node() {
        let parsed = parse_json_utf8_lossy(b"{\"title\":\"\xff\"}").unwrap();
        assert_eq!(parsed["title"], "\u{fffd}");
    }

    #[test]
    fn generated_registry_array_scan_ignores_brackets_and_terminators_in_strings() {
        let source = r#"// Misleading text: export const FIRST = [{"text": "not the array"}];
        export const FIRST_EXTRA = [{"text": "prefix declaration"}];
        export const FIRST: unknown[] = [
          {"text": "embedded SECOND ]; marker", "nested": [1, 2]}
        ] as const; // trailing ]; marker
        export const SECOND = [];"#;
        let parsed = parse_exported_json_array_bounded(source, "FIRST", Some("SECOND"));
        assert_eq!(
            parsed,
            Some(vec![
                json!({"text": "embedded SECOND ]; marker", "nested": [1, 2]})
            ])
        );
    }

    #[test]
    fn fingerprint_number_stringification_uses_ecmascript_notation_thresholds() {
        assert_eq!(js_fingerprint_string(&json!(1.0)), "1");
        assert_eq!(js_fingerprint_string(&json!(1e-6)), "0.000001");
        assert_eq!(js_fingerprint_string(&json!(1e-7)), "1e-7");
        assert_eq!(js_fingerprint_string(&json!(1e21)), "1e+21");
    }

    #[test]
    fn javascript_numeric_results_serialize_like_json_stringify() {
        assert_eq!(js_number_value(1.0), json!(1));
        assert_eq!(serde_json::to_string(&js_number_value(1.0)).unwrap(), "1");
        assert_eq!(js_number_value(f64::NAN), Value::Null);
    }

    #[test]
    fn javascript_number_coercion_accepts_radix_literals_and_rejects_nonstandard_infinity() {
        assert_eq!(js_number_fingerprint(&json!("0x10")), Some(16.0));
        assert_eq!(js_number_fingerprint(&json!("0b11")), Some(3.0));
        assert_eq!(js_number_fingerprint(&json!("0o10")), Some(8.0));
        assert!(js_number_fingerprint(&json!("inf")).unwrap().is_nan());
        assert_eq!(
            js_number_fingerprint(&json!("Infinity")),
            Some(f64::INFINITY)
        );
    }

    #[test]
    fn change_record_deduplication_uses_javascript_json_stringify_semantics() {
        let same_number = [json!({"count": 1.0}), json!({"count": 1})];
        assert_eq!(unique_in_order(&same_number).len(), 1);

        let different_property_order = [json!({"a": 1, "b": 2}), json!({"b": 2, "a": 1})];
        assert_eq!(unique_in_order(&different_property_order).len(), 2);
    }

    #[test]
    fn classification_counts_keep_record_creation_order_before_changes_are_sorted() {
        let report = build_source_change_classification_report(&json!({
            "previousFingerprints": {
                "sourceFingerprints": [],
                "requirementSourceFingerprints": []
            },
            "sourceFingerprints": [],
            "requirementSourceFingerprints": [],
            "sourceDiff": {"added": [], "changed": [], "unchanged": [], "removed": []},
            "requirementDiff": {
                "added": [
                    {
                        "ownerId": "z-owner",
                        "planId": "z-plan",
                        "supportOnly": true,
                        "parsedUwCourseCodes": []
                    },
                    {
                        "ownerId": "a-owner",
                        "planId": "a-plan",
                        "parsedUwCourseCodes": []
                    }
                ],
                "changed": [],
                "unchanged": [],
                "removed": []
            }
        }));

        let counts = report["countsByChangeType"].as_object().unwrap();
        assert_eq!(
            counts.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["support-only-source-changed", "source-structure-changed"]
        );
        assert_eq!(report["changes"][0]["planId"], "a-plan");
    }

    #[test]
    fn fingerprint_hash_serialization_matches_javascript_number_json() {
        let mut serialized = String::new();
        write_javascript_json(&stable_for_hash(&json!({"value": 1.0})), &mut serialized);
        assert_eq!(serialized, "{\"value\":1}");
        assert_eq!(sha256_json(&json!(1.0)), sha256_json(&json!(1)));
        assert_eq!(
            sha256_json(&json!({"b": 2, "a": 1})),
            sha256_json(&json!({"a": 1, "b": 2}))
        );

        let mut object = Map::new();
        object.insert("10".to_string(), json!(10));
        object.insert("2".to_string(), json!(2));
        object.insert("a".to_string(), json!(1));
        object.insert("\u{e000}".to_string(), json!(3));
        object.insert("\u{10000}".to_string(), json!(4));
        let mut serialized = String::new();
        write_javascript_json(&stable_for_hash(&Value::Object(object)), &mut serialized);
        assert_eq!(
            serialized,
            "{\"2\":2,\"10\":10,\"a\":1,\"\u{10000}\":4,\"\u{e000}\":3}"
        );
        assert_eq!(
            javascript_json_pretty(&json!({"value": 1.0, "items": [2.0]})),
            "{\n  \"value\": 1,\n  \"items\": [\n    2\n  ]\n}"
        );
    }

    #[test]
    fn source_url_trimming_matches_ecmascript_whitespace() {
        assert_eq!(
            normalize_source_url_key(Some(&json!(
                "\u{feff}\u{00a0} https://example.test \u{3000}"
            ))),
            "https://example.test"
        );
        assert_eq!(
            normalize_source_url_key(Some(&json!("\u{0085}https://example.test\u{0085}"))),
            "\u{0085}https://example.test\u{0085}"
        );
    }

    #[test]
    fn generated_source_owner_index_stringifies_non_string_urls_like_javascript() {
        assert_eq!(js_string_nullish_empty(Some(&json!(123))), "123");
        assert_eq!(js_string_nullish_empty(Some(&Value::Null)), "");
        assert_eq!(js_string_nullish_empty(None), "");

        let mut index = HashMap::new();
        let numeric_url = json!(123);
        add_current_source_owner_url_entry(
            &mut index,
            Some(&numeric_url),
            "uw-seattle-computer-science",
        );
        assert!(
            index
                .get("123")
                .is_some_and(|owners| owners.contains("uw-seattle-computer-science"))
        );
    }

    #[test]
    fn pathway_id_tokens_use_javascript_unicode_lowercasing_before_ascii_filtering() {
        assert_eq!(normalize_transfer_planner_pathway_id_token("İ"), "i");
        assert_eq!(normalize_transfer_planner_pathway_id_token("K"), "k");
    }

    #[test]
    fn merge_unique_strings_keeps_existing_empty_values_but_filters_new_falsey_values() {
        assert_eq!(
            merge_unique_strings(
                &[json!(""), json!("a")],
                &[json!(""), json!("b"), Value::Null, json!(false)],
            ),
            vec!["", "a", "b"]
        );
    }

    #[test]
    fn default_string_sort_uses_javascript_utf16_order() {
        assert_eq!(
            merge_unique_strings(&[], &[json!("\u{e000}"), json!("\u{10000}")]),
            vec!["\u{10000}", "\u{e000}"]
        );
    }

    #[test]
    fn unique_sorted_uses_locale_like_case_ordering() {
        assert_eq!(
            unique_sorted(["b", "A", "a", "B"]),
            vec!["a", "A", "b", "B"]
        );
    }

    #[test]
    fn diff_values_uses_locale_like_ordering_for_added_and_removed_codes() {
        let delta = diff_values(&[json!("a")], &[json!("A"), json!("b"), json!("B")]);
        assert_eq!(delta["added"], json!(["A", "b", "B"]));
        assert_eq!(delta["removed"], json!(["a"]));
    }

    #[test]
    fn source_identity_text_uses_javascript_truthiness_and_string_coercion() {
        let source = json!({
            "labels": ["BA", null, 0, 2, {"kind": "source"}],
            "title": "Title",
            "url": false,
            "finalUrl": ["final", "url"]
        });
        assert_eq!(
            build_source_owner_identity_text(&source),
            "BA 2 [object Object] Title final,url"
        );

        let spread_string_labels = json!({"labels": "BA"});
        let spread_identity = build_source_owner_identity_text(&spread_string_labels);
        assert_eq!(spread_identity, "B A");
        assert_eq!(get_ba_bs_degree_kind(&spread_identity), Some("ba"));
    }

    #[test]
    fn fingerprint_compare_treats_falsey_previous_map_values_as_absent() {
        let current = json!({"url": "", "resourceFingerprint": "current"});
        let diff = compare_fingerprints(
            &[Value::Null],
            std::slice::from_ref(&current),
            |entry| js_entry_key(entry, "url"),
            "resourceFingerprint",
        );
        assert_eq!(diff.added, vec![current]);
        assert!(diff.changed.is_empty());
        assert!(diff.unchanged.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn fingerprint_compare_uses_javascript_strict_value_equality() {
        let previous_number = json!({"id": "same", "hash": 1});
        let current_number = json!({"id": "same", "hash": 1.0});
        let numeric_diff = compare_fingerprints(
            std::slice::from_ref(&previous_number),
            std::slice::from_ref(&current_number),
            |entry| js_entry_key(entry, "id"),
            "hash",
        );
        assert_eq!(numeric_diff.unchanged, vec![current_number]);

        let previous_object = json!({"id": "same", "hash": {"value": 1}});
        let current_object = json!({"id": "same", "hash": {"value": 1}});
        let object_diff = compare_fingerprints(
            std::slice::from_ref(&previous_object),
            std::slice::from_ref(&current_object),
            |entry| js_entry_key(entry, "id"),
            "hash",
        );
        assert_eq!(object_diff.changed, vec![current_object]);
    }

    #[test]
    fn stable_hash_object_keys_follow_javascript_utf16_sort_order() {
        let mut object = Map::new();
        object.insert("\u{e000}".to_string(), json!(1));
        object.insert("\u{10000}".to_string(), json!(2));
        let stable = stable_for_hash(&Value::Object(object));
        let keys = stable
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["\u{10000}".to_string(), "\u{e000}".to_string()]);
    }

    fn req(overrides: Value) -> Value {
        let mut base = json!({
            "ownerId": "uw-seattle-computer-science",
            "ownerTitle": "Computer Science",
            "planId": "uw-seattle-computer-science",
            "pathwayId": Value::Null,
            "campusId": "uw-seattle",
            "parserType": "pdf-degree-sheet",
            "sourceUrl": "https://www.cs.washington.edu/academics/ugrad/curriculum/CS_2025.pdf",
            "sourceLabel": "CS degree requirements 2025",
            "sourceRole": "primary-degree-requirements",
            "sourceRoleStatus": "primary",
            "canCreateSchedulableRows": true,
            "canCreateRequiredRows": true,
            "canCreateScheduleRows": true,
            "supportOnly": false,
            "nonSchedulable": false,
            "ok": true,
            "parseConfidence": "high",
            "parsedUwCourseCodeCount": 1,
            "sourceOnlyUwCourseCodeCount": 0,
            "structuredOnlyUwCourseCodeCount": 0,
            "approvedFilterUwCourseCodeCount": 0,
            "electiveListUwCourseCodeCount": 0,
            "supportOnlyUwCourseCodeCount": 0,
            "supportListCount": 0,
            "extractedHeadingCount": 2,
            "requirementCueLineCount": 4,
            "chooseStatementCount": 1,
            "qualitySignalCodes": [],
            "qualityWarningCount": 0,
            "qualityNoteCount": 0,
            "requirementFingerprint": "previous",
            "parsedUwCourseCodes": ["CSE 121"],
            "sourceOnlyUwCourseCodes": [],
            "structuredOnlyUwCourseCodes": [],
            "approvedFilterUwCourseCodes": [],
            "electiveListUwCourseCodes": [],
            "supportOnlyUwCourseCodes": []
        });
        if let (Some(base_obj), Some(over_obj)) = (base.as_object_mut(), overrides.as_object()) {
            for (k, v) in over_obj {
                base_obj.insert(k.clone(), v.clone());
            }
        }
        base
    }

    fn classify_many(previous_entries: Vec<Value>, current_entries: Vec<Value>) -> Value {
        let previous_fingerprints = json!({
            "sourceFingerprints": [],
            "requirementSourceFingerprints": previous_entries,
        });
        let requirement_diff = compare_fingerprints(
            &previous_fingerprints["requirementSourceFingerprints"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            &current_entries,
            |e| {
                e.get("ownerId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            },
            "requirementFingerprint",
        );
        let input = json!({
            "previousFingerprints": previous_fingerprints,
            "sourceFingerprints": [],
            "requirementSourceFingerprints": current_entries,
            "sourceDiff": {
                "added": [],
                "changed": [],
                "unchanged": [],
                "removed": [],
            },
            "requirementDiff": requirement_diff,
        });
        build_source_change_classification_report(&input)
    }

    fn classify(previous_entry: Option<Value>, current_entry: Option<Value>) -> Value {
        let prev = previous_entry.into_iter().collect();
        let curr = current_entry.into_iter().collect();
        classify_many(prev, curr)
    }

    #[test]
    fn quality_signal_codes_use_javascript_default_sort_order() {
        let entry = build_requirement_fingerprint_entry(&json!({
            "qualitySignals": [
                {"code": "\u{e000}"},
                {"code": "\u{10000}"}
            ]
        }));
        assert_eq!(
            entry["qualitySignalCodes"],
            json!(["\u{10000}", "\u{e000}"])
        );
    }

    #[test]
    fn course_code_normalization_uses_nullish_string_coercion() {
        assert_eq!(
            normalize_course_codes(Some(&json!([null, 151, " MATH 151 ", "MATH 151"]))),
            vec!["151", "MATH 151"]
        );
    }

    #[test]
    fn source_owner_id_normalization_requires_nonempty_javascript_regex_groups() {
        assert_eq!(normalize_source_owner_id("plan::path"), "plan:pathway:path");
        assert_eq!(normalize_source_owner_id("plan::"), "plan::");
        assert_eq!(normalize_source_owner_id("::path"), "::path");
        assert_eq!(normalize_source_owner_id("::"), "::");
        assert!(normalize_source_owner_ids(Some(&json!("uw-seattle-computer-science"))).is_empty());
    }

    #[test]
    fn year_signal_extraction_uses_javascript_ascii_digit_semantics() {
        assert_eq!(
            extract_year_signals("2025 AY 24/25 ٢٠٢٥"),
            vec![2025, 2024, 2025]
        );
        assert!(extract_year_signals("academic\u{0085}year24/25").is_empty());
        assert_eq!(
            extract_year_signals("academic\u{feff}year24/25"),
            vec![2024, 2025]
        );
        assert_eq!(extract_year_signals("AY24/25"), vec![2024, 2025]);
        assert_eq!(extract_year_signals("AcademicYear24/25"), vec![2024, 2025]);
    }

    #[test]
    fn sibling_promotion_compares_string_counts_with_javascript_numeric_coercion() {
        let previous = json!({
            "sourceUrl": "https://example.test/catalog/2024.pdf",
            "parsedUwCourseCodeCount": "5",
            "qualityWarningCount": "3"
        });
        let lower_coverage = json!({
            "sourceUrl": "https://example.test/catalog/2025.pdf",
            "parsedUwCourseCodeCount": "4",
            "qualityWarningCount": "2"
        });
        assert!(!is_current_year_sibling_promotion(
            Some(&previous),
            Some(&lower_coverage)
        ));

        let worse_warnings = json!({
            "sourceUrl": "https://example.test/catalog/2025.pdf",
            "parsedUwCourseCodeCount": "5",
            "qualityWarningCount": "4"
        });
        assert!(!is_current_year_sibling_promotion(
            Some(&previous),
            Some(&worse_warnings)
        ));
    }

    #[test]
    fn sibling_promotion_preserves_javascript_math_max_nan_semantics() {
        let previous = json!({
            "sourceUrl": "https://example.test/catalog/2024.pdf",
            "parsedUwCourseCodeCount": 5,
            "qualityWarningCount": "NaN"
        });
        let current = json!({
            "sourceUrl": "https://example.test/catalog/2025.pdf",
            "parsedUwCourseCodeCount": 5,
            "qualityWarningCount": 0
        });
        assert!(!is_current_year_sibling_promotion(
            Some(&previous),
            Some(&current)
        ));
    }

    #[test]
    fn max_year_signal_uses_javascript_array_join_coercion() {
        assert_eq!(get_max_year_signal(&json!({"sourceUrl": 2025})), Some(2025));
        assert_eq!(
            get_max_year_signal(&json!({"sourceLabel": ["AY", 24, 25]})),
            None
        );
    }

    #[test]
    fn parent_coverage_uses_javascript_numeric_coercion_for_course_counts() {
        let mut coverage = HashMap::new();
        coverage.insert(
            "plan".to_string(),
            PlanChildPathwayCoverage {
                child_owner_ids: vec!["plan:pathway:child".to_string()],
                parsed_uw_course_code_count: json!(1),
            },
        );
        let parent_with_courses = json!({
            "ownerId": "plan",
            "planId": "plan",
            "parsedUwCourseCodeCount": "1"
        });
        assert!(
            get_child_pathway_coverage_for_parent(Some(&parent_with_courses), &coverage).is_none()
        );

        let parent_without_courses = json!({
            "ownerId": "plan",
            "planId": "plan",
            "parsedUwCourseCodeCount": "0"
        });
        assert!(
            get_child_pathway_coverage_for_parent(Some(&parent_without_courses), &coverage)
                .is_some()
        );
    }

    #[test]
    fn no_usable_course_count_uses_javascript_strict_zero_equality() {
        assert!(has_no_usable_parsed_courses(&json!({})));
        assert!(has_no_usable_parsed_courses(
            &json!({"parsedUwCourseCodeCount": null})
        ));
        assert!(has_no_usable_parsed_courses(
            &json!({"parsedUwCourseCodeCount": 0})
        ));
        assert!(!has_no_usable_parsed_courses(
            &json!({"parsedUwCourseCodeCount": "0"})
        ));
        assert!(!has_no_usable_parsed_courses(
            &json!({"parsedUwCourseCodeCount": false})
        ));
    }

    #[test]
    fn child_pathway_coverage_uses_javascript_addition_for_string_counts() {
        let entries = [
            json!({
                "ownerId": "plan:pathway:first",
                "planId": "plan",
                "pathwayId": "first",
                "parsedUwCourseCodeCount": "3"
            }),
            json!({
                "ownerId": "plan:pathway:second",
                "planId": "plan",
                "pathwayId": "second",
                "parsedUwCourseCodeCount": 2
            }),
        ];
        let coverage = build_plan_child_pathway_coverage(&entries);
        assert_eq!(coverage["plan"].parsed_uw_course_code_count, json!("032"));
    }

    #[test]
    fn count_by_preserves_first_seen_object_key_order() {
        let records = [
            json!({"kind": "zeta"}),
            json!({"kind": "alpha"}),
            json!({"kind": "zeta"}),
        ];
        let counts = count_by(&records, |record| {
            record["kind"].as_str().unwrap_or("unknown").to_string()
        });
        assert_eq!(
            counts.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["zeta", "alpha"]
        );
        assert_eq!(counts["zeta"], 2);
        assert_eq!(counts["alpha"], 1);
    }

    #[test]
    fn requirement_fingerprint_entry_matches_javascript_undefined_and_nullish_fields() {
        let entry = build_requirement_fingerprint_entry(&json!({
            "qualitySignals": [
                {"code": "zeta", "severity": "warning"},
                {"code": "alpha", "severity": "note"},
                {}
            ]
        }));
        let object = entry.as_object().unwrap();
        assert!(!object.contains_key("ownerId"));
        assert!(!object.contains_key("sourceUrl"));
        assert!(!object.contains_key("primarySourceUrl"));
        assert_eq!(entry["parseConfidence"], "low");
        assert_eq!(entry["qualitySignalCodes"], json!(["alpha", "zeta", null]));
        assert_eq!(entry["qualityWarningCount"], 1);
        assert_eq!(entry["qualityNoteCount"], 1);

        let null_fallback = build_requirement_fingerprint_entry(&json!({"sourceUrl": null}));
        assert_eq!(null_fallback["primarySourceUrl"], Value::Null);
    }

    #[test]
    fn requirement_fingerprint_preserves_non_nullish_array_fields_and_javascript_lengths() {
        let entry = build_requirement_fingerprint_entry(&json!({
            "parsedUwCourseCodes": "AB𝄞",
            "sourceOnlyUwCourseCodes": null,
            "supportLists": "🌱",
            "qualitySignals": []
        }));
        assert_eq!(entry["parsedUwCourseCodes"], "AB𝄞");
        assert_eq!(entry["parsedUwCourseCodeCount"], 4);
        assert_eq!(entry["sourceOnlyUwCourseCodes"], json!([]));
        assert_eq!(entry["sourceOnlyUwCourseCodeCount"], 0);
        assert_eq!(entry["supportListCount"], 2);
    }

    #[test]
    fn source_fingerprint_entry_omits_undefined_url_and_hash_fields() {
        let entry = build_source_fingerprint_entry(&json!({}), &HashMap::new());
        let object = entry.as_object().unwrap();
        assert!(!object.contains_key("url"));
        assert_eq!(entry["finalUrl"], Value::Null);
        assert_eq!(entry["resourceFingerprint"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn source_change_record_uses_javascript_nullish_fallbacks() {
        let previous = json!({
            "ownerId": "uw-seattle-example",
            "ownerTitle": "Example",
            "planId": "uw-seattle-example",
            "sourceUrl": "https://example.edu/previous",
            "sourceRole": "primary-degree-requirements",
            "sourceRoleStatus": "primary",
            "parsedUwCourseCodeCount": null,
            "qualitySignalCodes": null,
            "requirementFingerprint": null
        });
        let current = json!({
            "ownerId": "uw-seattle-example",
            "sourceUrl": null,
            "sourceRole": null,
            "sourceRoleStatus": null
        });
        let record = build_source_change_record(
            "source-structure-changed",
            Some(&previous),
            Some(&current),
            None,
            None,
            json!({"added": [], "removed": []}),
            json!({"added": [], "removed": []}),
            "test",
            None,
        );
        assert_eq!(record["sourceUrl"], "https://example.edu/previous");
        assert_eq!(record["sourceRole"], "primary-degree-requirements");
        assert_eq!(record["sourceRoleStatus"], "primary");
        assert_eq!(
            record["previousFingerprintSummary"]["parsedUwCourseCodeCount"],
            0
        );
        assert_eq!(
            record["previousFingerprintSummary"]["qualitySignalCodes"],
            json!([])
        );
        assert_eq!(
            record["previousFingerprintSummary"]["requirementFingerprint"],
            Value::Null
        );
    }

    #[test]
    fn source_change_classifier_compares_duplicate_owner_fingerprints_by_source_url() {
        let primary = req(json!({
            "ownerId": "uw-example-multi-source",
            "planId": "uw-example-multi-source",
            "sourceUrl": "https://example.edu/program/requirements",
            "requirementFingerprint": "primary-source-fingerprint",
            "parsedUwCourseCodes": ["EXAM 101"],
            "parsedUwCourseCodeCount": 1,
        }));
        let supplemental = req(json!({
            "ownerId": "uw-example-multi-source",
            "planId": "uw-example-multi-source",
            "sourceUrl": "https://example.edu/program/supplemental-list",
            "sourceRole": "approved-course-list",
            "sourceRoleStatus": "support",
            "canCreateSchedulableRows": false,
            "canCreateRequiredRows": false,
            "canCreateScheduleRows": false,
            "supportOnly": true,
            "requirementFingerprint": "supplemental-source-fingerprint",
            "parsedUwCourseCodes": ["EXAM 201"],
            "parsedUwCourseCodeCount": 1,
        }));
        let previous = vec![primary.clone(), supplemental.clone()];
        let current = vec![primary, supplemental];
        let requirement_diff = compare_fingerprints(
            &previous,
            &current,
            get_requirement_fingerprint_compare_key,
            "requirementFingerprint",
        );
        let input = json!({
            "previousFingerprints": {
                "sourceFingerprints": [],
                "requirementSourceFingerprints": previous,
            },
            "sourceFingerprints": [],
            "requirementSourceFingerprints": current,
            "requirementDiff": requirement_diff,
        });
        let report = build_source_change_classification_report(&input);

        assert_eq!(report["totalChangeCount"], 0);
    }

    #[test]
    fn source_fingerprints_drop_stale_owner_ids_from_mismatched_source_link_snapshots() {
        let requirement_owners = vec![
            req(json!({
                "ownerId": "uw-tacoma-computer-engineering",
                "ownerTitle": "Computer Engineering",
                "planId": "uw-tacoma-computer-engineering",
                "campusId": "uw-tacoma",
                "sourceUrl": "https://www.tacoma.uw.edu/set/programs/undergrad/cengr",
                "sourceLabel": "Computer Engineering",
            })),
            req(json!({
                "ownerId": "uw-tacoma-computer-science-and-systems:pathway:bachelor-of-arts",
                "ownerTitle": "Computer Science and Systems - Bachelor of Arts",
                "planId": "uw-tacoma-computer-science-and-systems",
                "pathwayId": "bachelor-of-arts",
                "campusId": "uw-tacoma",
                "sourceUrl": "https://www.tacoma.uw.edu/set/programs/undergrad/css/ba",
                "sourceLabel": "UW Tacoma Computer Science and Systems BA degree requirements",
            })),
            req(json!({
                "ownerId": "uw-tacoma-computer-science-and-systems:pathway:bachelor-of-science",
                "ownerTitle": "Computer Science and Systems - Bachelor of Science",
                "planId": "uw-tacoma-computer-science-and-systems",
                "pathwayId": "bachelor-of-science",
                "campusId": "uw-tacoma",
                "sourceUrl": "https://www.tacoma.uw.edu/set/programs/undergrad/css/bs",
                "sourceLabel": "UW Tacoma Computer Science and Systems BS degree requirements",
            })),
        ];

        let source_snapshot = json!({
            "generatedAt": "test",
            "sources": [
                {
                    "url": "https://www.tacoma.uw.edu/set/programs/undergrad/cengr",
                    "finalUrl": "https://www.tacoma.uw.edu/set/programs/undergrad/cengr",
                    "labels": ["Computer Engineering"],
                    "ownerIds": [
                        "uw-tacoma-computer-engineering",
                        "uw-tacoma-computer-science-and-systems",
                        "uw-tacoma-computer-science-and-systems::bachelor-of-science"
                    ],
                    "kinds": ["major", "pathway"],
                    "ok": true,
                },
                {
                    "url": "https://www.tacoma.uw.edu/set/programs/undergrad/css/ba",
                    "finalUrl": "https://www.tacoma.uw.edu/set/programs/undergrad/css/ba",
                    "labels": ["UW Tacoma Computer Science and Systems BA degree requirements"],
                    "ownerIds": [
                        "uw-tacoma-computer-science-and-systems:pathway:bachelor-of-arts",
                        "uw-tacoma-computer-science-and-systems:pathway:bachelor-of-science"
                    ],
                    "kinds": ["pathway"],
                    "ok": true,
                }
            ]
        });

        let requirement_report = json!({
            "generatedAt": "test",
            "owners": requirement_owners,
        });

        let empty_map = HashMap::new();
        let report = build_report(
            &source_snapshot,
            &requirement_report,
            &json!({"sourceFingerprints": [], "requirementSourceFingerprints": []}),
            Some(&empty_map),
            Path::new("."),
        );

        let source_fingerprints = report["sourceFingerprints"].as_array().unwrap();
        let cengr = source_fingerprints
            .iter()
            .find(|e| e["url"].as_str().unwrap_or("").contains("/undergrad/cengr"))
            .unwrap();
        let css_ba = source_fingerprints
            .iter()
            .find(|e| {
                e["url"]
                    .as_str()
                    .unwrap_or("")
                    .contains("/undergrad/css/ba")
            })
            .unwrap();

        assert_eq!(cengr["ownerIds"], json!(["uw-tacoma-computer-engineering"]));
        let css_ba_owners: Vec<&str> = css_ba["ownerIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            css_ba_owners
                .contains(&"uw-tacoma-computer-science-and-systems:pathway:bachelor-of-arts")
        );
        assert!(
            !css_ba_owners
                .contains(&"uw-tacoma-computer-science-and-systems:pathway:bachelor-of-science")
        );
    }

    #[test]
    fn source_change_classifier_detects_new_course_additions() {
        let report = classify(
            Some(req(json!({
                "requirementFingerprint": "a",
                "parsedUwCourseCodes": ["CSE 121"],
                "parsedUwCourseCodeCount": 1,
            }))),
            Some(req(json!({
                "requirementFingerprint": "b",
                "parsedUwCourseCodes": ["CSE 121", "CSE 122"],
                "parsedUwCourseCodeCount": 2,
            }))),
        );
        let changes = report["changes"].as_array().unwrap();
        let change = changes
            .iter()
            .find(|e| e["changeType"] == "new-course-added")
            .unwrap();
        assert_eq!(change["courseDelta"]["added"], json!(["CSE 122"]));
        let rec = change["recommendedAction"].as_array().unwrap();
        assert!(rec.iter().any(|a| a == "equivalency-mapping-refresh"));
    }

    #[test]
    fn source_change_classifier_detects_course_removals() {
        let report = classify(
            Some(req(json!({
                "requirementFingerprint": "a",
                "parsedUwCourseCodes": ["MSE 170", "MSE 220"],
                "parsedUwCourseCodeCount": 2,
                "planId": "uw-seattle-materials-science-engineering",
                "ownerId": "uw-seattle-materials-science-engineering",
            }))),
            Some(req(json!({
                "requirementFingerprint": "b",
                "parsedUwCourseCodes": ["MSE 170"],
                "parsedUwCourseCodeCount": 1,
                "planId": "uw-seattle-materials-science-engineering",
                "ownerId": "uw-seattle-materials-science-engineering",
            }))),
        );
        let changes = report["changes"].as_array().unwrap();
        let change = changes
            .iter()
            .find(|e| e["changeType"] == "course-removed")
            .unwrap();
        assert_eq!(change["courseDelta"]["removed"], json!(["MSE 220"]));
        let rec = change["recommendedAction"].as_array().unwrap();
        assert!(rec.iter().any(|a| a == "verify-alternate-official-support"));
    }

    #[test]
    fn fingerprint_markdown_preserves_undefined_timestamp_interpolation() {
        let report = json!({
            "sourceDiff": {"changed": []},
            "requirementDiff": {"changed": []},
            "sourceChangeClassificationSummary": {}
        });
        let markdown = build_source_fingerprints_markdown(&report, "classification.md");
        assert!(markdown.contains("- Source snapshot generated: undefined"));
        assert!(markdown.contains("- Requirement parse report generated: undefined"));
        assert!(markdown.contains("- Source fingerprints: undefined"));
    }

    #[test]
    fn fingerprint_markdown_checks_signal_array_length_before_joined_text() {
        let report = json!({
            "generatedAt": "test",
            "sourceDiff": {"changed": []},
            "requirementDiff": {"changed": [{
                "ownerTitle": "Owner",
                "ownerId": "owner-id",
                "sourceUrl": "https://example.test",
                "requirementFingerprint": "hash",
                "sourceOnlyUwCourseCodeCount": 0,
                "structuredOnlyUwCourseCodeCount": 0,
                "qualitySignalCodes": [null]
            }]}
        });
        let markdown = build_source_fingerprints_markdown(&report, "classification.md");
        assert!(markdown.contains("  - quality signals: \n"));
    }

    #[test]
    fn classification_markdown_uses_nullish_n_a_fallbacks_for_counts() {
        let report = json!({
            "generatedAt": "test",
            "totalChangeCount": 1,
            "affectedOwnerCount": 1,
            "affectedPlanCount": 1,
            "countsByChangeType": {},
            "countsByActionStatus": {},
            "changes": [{
                "ownerTitle": "Owner",
                "ownerId": "owner-id",
                "changeType": "source-structure-changed",
                "planId": "plan-id",
                "sourceUrl": "https://example.test",
                "sourceRole": "primary",
                "sourceRoleStatus": "primary",
                "actionStatus": "auto-applied",
                "recommendedAction": [null, "target", false],
                "reason": "Reason",
                "courseDelta": {"added": [null, "CSE 121"], "removed": []},
                "headingDelta": {"previousHeadingCount": null, "currentHeadingCount": 2},
                "sectionDelta": {"previousRequirementCueLineCount": null, "currentRequirementCueLineCount": 5},
                "childPathwayCoverage": {"childOwnerIds": [], "parsedUwCourseCodeCount": 0},
                "previousFingerprintSummary": null,
                "currentFingerprintSummary": null
            }]
        });
        let markdown = build_source_change_classification_markdown(&report);
        assert!(markdown.contains("- Heading count: n/a -> 2"));
        assert!(markdown.contains("- Requirement cue lines: n/a -> 5"));
        assert!(markdown.contains("- Recommended action: , target, false"));
        assert!(markdown.contains("- Course delta: +, CSE 121; -none"));
        assert!(!markdown.contains("- Child pathway coverage:"));
    }

    #[test]
    fn nullish_summary_entries_match_javascript_falsey_guards() {
        assert_eq!(
            get_requirement_fingerprint_summary(Some(&Value::Null)),
            Value::Null
        );
        assert_eq!(
            build_source_resource_summary(Some(&Value::Null)),
            Value::Null
        );
    }

    #[test]
    fn source_only_change_resource_fallback_ignores_null_map_values() {
        let source_diff = FingerprintDiff {
            changed: vec![json!({"url": "https://source", "ownerIds": ["owner"]})],
            ..FingerprintDiff::default()
        };
        let previous_entry = json!({"ownerId": "owner", "sourceUrl": "https://fallback"});
        let current_entry = previous_entry.clone();
        let previous_requirements = HashMap::from([("owner".to_string(), previous_entry)]);
        let current_requirements = HashMap::from([("owner".to_string(), current_entry)]);
        let fallback_resource = json!({"url": "https://fallback"});
        let previous_sources = HashMap::from([
            ("https://source".to_string(), Value::Null),
            ("https://fallback".to_string(), fallback_resource.clone()),
        ]);
        let current_sources = previous_sources.clone();

        let records = build_source_only_change_records(
            &source_diff,
            &FingerprintDiff::default(),
            &previous_sources,
            &current_sources,
            &previous_requirements,
            &current_requirements,
            &HashMap::new(),
        );

        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0]["previousSourceResource"]["url"],
            "https://fallback"
        );
        assert_eq!(
            records[0]["currentSourceResource"]["url"],
            "https://fallback"
        );
    }

    #[test]
    fn missing_warning_count_keeps_javascript_undefined_comparison_false() {
        let previous = json!({});
        let current = json!({"qualityWarningCount": 2});
        assert_eq!(
            classify_action_status(
                "source-structure-changed",
                Some(&previous),
                Some(&current),
                0,
                0,
                false,
            ),
            "auto-applied"
        );
    }

    #[test]
    fn source_change_classifier_detects_structural_parser_drift() {
        let report = classify(
            Some(req(json!({
                "requirementFingerprint": "a",
                "requirementCueLineCount": 4,
                "qualitySignalCodes": [],
            }))),
            Some(req(json!({
                "requirementFingerprint": "b",
                "requirementCueLineCount": 12,
                "qualitySignalCodes": ["material-source-structured-drift"],
                "qualityWarningCount": 1,
            }))),
        );
        let changes = report["changes"].as_array().unwrap();
        let change = changes
            .iter()
            .find(|e| e["changeType"] == "source-structure-changed")
            .unwrap();
        assert_eq!(change["actionStatus"], "needs-parser-rule");
        let rec = change["recommendedAction"].as_array().unwrap();
        assert!(rec.iter().any(|a| a == "targeted-parser-recovery"));
    }

    #[test]
    fn structural_change_detection_coerces_string_counts_like_javascript() {
        let report = classify(
            Some(req(json!({
                "requirementFingerprint": "before",
                "requirementCueLineCount": "0",
                "qualitySignalCodes": [],
            }))),
            Some(req(json!({
                "requirementFingerprint": "after",
                "requirementCueLineCount": "5",
                "qualitySignalCodes": [],
            }))),
        );
        assert!(
            report["changes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|change| change["changeType"] == "source-structure-changed")
        );
    }

    #[test]
    fn source_change_classifier_treats_child_covered_parent_overview_drift_as_generated_evidence() {
        let plan_id = "uw-tacoma-ethnic-gender-and-labor-studies";
        let previous_parent = req(json!({
            "ownerId": plan_id,
            "planId": plan_id,
            "campusId": "uw-tacoma",
            "sourceUrl": "https://www.tacoma.uw.edu/sias/socs/ethnic-gender-and-labor-studies",
            "parserType": "html-overview-page",
            "parsedUwCourseCodes": [],
            "parsedUwCourseCodeCount": 0,
            "requirementFingerprint": "parent-before",
            "qualitySignalCodes": ["no-parsed-uw-course-codes"],
            "qualityWarningCount": 1,
        }));
        let current_parent = req(json!({
            "ownerId": plan_id,
            "planId": plan_id,
            "campusId": "uw-tacoma",
            "sourceUrl": "https://www.tacoma.uw.edu/sias/socs/ethnic-gender-and-labor-studies",
            "parserType": "html-overview-page",
            "parsedUwCourseCodes": [],
            "parsedUwCourseCodeCount": 0,
            "requirementFingerprint": "parent-after",
            "qualitySignalCodes": ["large-structured-only-course-gap", "material-source-structured-drift"],
            "qualityWarningCount": 2,
        }));
        let child = req(json!({
            "ownerId": format!("{plan_id}:pathway:ethnic-studies-option"),
            "ownerTitle": "Ethnic, Gender and Labor Studies (BA) - Ethnic Studies Option",
            "planId": plan_id,
            "pathwayId": "ethnic-studies-option",
            "campusId": "uw-tacoma",
            "sourceUrl": "https://www.tacoma.uw.edu/sias/socs/ethnic-studies-option",
            "parsedUwCourseCodes": ["TEGL 101", "TSOC 439"],
            "parsedUwCourseCodeCount": 2,
            "requirementFingerprint": "child-stable",
        }));

        let report = classify_many(
            vec![previous_parent, child.clone()],
            vec![current_parent, child],
        );
        let changes = report["changes"].as_array().unwrap();
        let change = changes
            .iter()
            .find(|e| e["ownerId"] == plan_id && e["changeType"] == "source-structure-changed")
            .unwrap();

        assert_eq!(change["actionStatus"], "generated-evidence-only");
        assert_eq!(change["autoApplied"], true);
        let rec = change["recommendedAction"].as_array().unwrap();
        assert!(rec.iter().any(|a| a == "preserve-parent-overview-evidence"));
        assert_eq!(
            change["childPathwayCoverage"]["childOwnerIds"],
            json!([format!("{plan_id}:pathway:ethnic-studies-option")])
        );
    }

    #[test]
    fn source_change_classifier_detects_current_year_sibling_promotion() {
        let report = classify(
            Some(req(json!({
                "requirementFingerprint": "a",
                "sourceUrl": "https://www.cs.washington.edu/academics/ugrad/curriculum/CS_2025.pdf",
                "sourceLabel": "CS degree requirements 2025",
                "parsedUwCourseCodeCount": 2,
                "parsedUwCourseCodes": ["CSE 121", "CSE 122"],
            }))),
            Some(req(json!({
                "requirementFingerprint": "b",
                "sourceUrl": "https://www.cs.washington.edu/academics/ugrad/curriculum/CS_2026.pdf",
                "sourceLabel": "CS degree requirements 2026",
                "parsedUwCourseCodeCount": 2,
                "parsedUwCourseCodes": ["CSE 121", "CSE 122"],
            }))),
        );
        let changes = report["changes"].as_array().unwrap();
        let change = changes
            .iter()
            .find(|e| e["changeType"] == "current-year-sibling-found")
            .unwrap();
        assert_eq!(change["actionStatus"], "auto-applied");
        let rec = change["recommendedAction"].as_array().unwrap();
        assert!(rec.iter().any(|a| a == "high-confidence-source-promotion"));
    }

    #[test]
    fn source_change_classifier_detects_weak_overview_regression() {
        let report = classify(
            Some(req(json!({
                "requirementFingerprint": "a",
                "parseConfidence": "high",
                "parsedUwCourseCodeCount": 3,
                "parsedUwCourseCodes": ["TCOM 201", "TCOM 230", "TCOM 320"],
            }))),
            Some(req(json!({
                "requirementFingerprint": "b",
                "parseConfidence": "low",
                "parsedUwCourseCodeCount": 0,
                "parsedUwCourseCodes": [],
                "qualitySignalCodes": ["no-parsed-uw-course-codes", "low-confidence-parsed-source"],
                "qualityWarningCount": 2,
            }))),
        );
        let changes = report["changes"].as_array().unwrap();
        let change = changes
            .iter()
            .find(|e| e["changeType"] == "source-became-weak-or-overview")
            .unwrap();
        assert_eq!(change["actionStatus"], "needs-discovery-rule");
        let rec = change["recommendedAction"].as_array().unwrap();
        assert!(rec.iter().any(|a| a == "targeted-source-discovery"));
    }

    #[test]
    fn support_only_source_changes_regenerate_filters_without_required_row_scheduling() {
        let previous = req(json!({
            "requirementFingerprint": "a",
            "sourceRole": "approved-course-list",
            "sourceRoleStatus": "support",
            "canCreateSchedulableRows": false,
            "canCreateRequiredRows": false,
            "canCreateScheduleRows": false,
            "supportOnly": true,
            "parsedUwCourseCodes": ["PHYS 121"],
            "parsedUwCourseCodeCount": 1,
            "approvedFilterUwCourseCodes": ["PHYS 121"],
            "approvedFilterUwCourseCodeCount": 1,
        }));
        let current = req(json!({
            "requirementFingerprint": "b",
            "sourceRole": "approved-course-list",
            "sourceRoleStatus": "support",
            "canCreateSchedulableRows": false,
            "canCreateRequiredRows": false,
            "canCreateScheduleRows": false,
            "supportOnly": true,
            "parsedUwCourseCodes": ["PHYS 121", "CHEM 142"],
            "parsedUwCourseCodeCount": 2,
            "approvedFilterUwCourseCodes": ["PHYS 121", "CHEM 142"],
            "approvedFilterUwCourseCodeCount": 2,
        }));
        let report = classify(Some(previous), Some(current));
        let changes = report["changes"].as_array().unwrap();
        let change = changes
            .iter()
            .find(|e| e["changeType"] == "support-only-source-changed")
            .unwrap();

        assert_eq!(change["actionStatus"], "generated-evidence-only");
        let rec = change["recommendedAction"].as_array().unwrap();
        assert!(
            rec.iter()
                .any(|a| a == "preserve-support-only-no-required-rows")
        );
        assert!(!rec.iter().any(|a| a == "student-runtime-regeneration"));
    }
}
