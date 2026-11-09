use super::requirement_course_codes::CourseCodeParser;
use super::requirement_ids::slugify_requirement_identifier;
use super::requirement_parse_helpers::parse_requirement_credit_range;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
struct CategoryOptionDescriptor {
    category: &'static str,
    source_category_code: &'static str,
    long_label: &'static str,
}

const CATEGORY_DESCRIPTORS: [CategoryOptionDescriptor; 8] = [
    CategoryOptionDescriptor {
        category: "NSC",
        source_category_code: "NSc",
        long_label: "Natural Sciences",
    },
    CategoryOptionDescriptor {
        category: "AH",
        source_category_code: "A&H",
        long_label: "Arts and Humanities",
    },
    CategoryOptionDescriptor {
        category: "SSC",
        source_category_code: "SSc",
        long_label: "Social Sciences",
    },
    CategoryOptionDescriptor {
        category: "QSR",
        source_category_code: "QSR",
        long_label: "Quantitative and Symbolic Reasoning",
    },
    CategoryOptionDescriptor {
        category: "VLPA",
        source_category_code: "VLPA",
        long_label: "Visual, Literary, and Performing Arts",
    },
    CategoryOptionDescriptor {
        category: "DIV",
        source_category_code: "DIV",
        long_label: "Diversity",
    },
    CategoryOptionDescriptor {
        category: "NW",
        source_category_code: "NW",
        long_label: "Natural World",
    },
    CategoryOptionDescriptor {
        category: "IANDS",
        source_category_code: "I&S",
        long_label: "Individuals and Societies",
    },
];

pub fn count_requirement_category_option_descriptors(text: &str) -> usize {
    let normalized = normalize_whitespace(text);
    CATEGORY_DESCRIPTORS
        .iter()
        .filter(|descriptor| category_pattern(descriptor.category).is_match(&normalized))
        .count()
}

pub fn get_requirement_category_option_descriptors(text: &str) -> Vec<Value> {
    let normalized = normalize_whitespace(text);
    CATEGORY_DESCRIPTORS
        .iter()
        .filter(|descriptor| category_pattern(descriptor.category).is_match(&normalized))
        .map(|descriptor| {
            json!({
                "category": descriptor.category,
                "sourceCategoryCode": descriptor.source_category_code,
                "longLabel": descriptor.long_label,
            })
        })
        .collect()
}

pub fn get_requirement_category_option_descriptor(text: &str) -> Option<Value> {
    let normalized = normalize_whitespace(text);
    if normalized.is_empty() {
        return None;
    }
    CATEGORY_DESCRIPTORS
        .iter()
        .find(|descriptor| category_pattern(descriptor.category).is_match(&normalized))
        .map(|descriptor| {
            json!({
                "category": descriptor.category,
                "sourceCategoryCode": descriptor.source_category_code,
                "longLabel": descriptor.long_label,
            })
        })
}

pub fn build_category_requirement_option(input: &Value) -> Option<Value> {
    let source_text = normalize_whitespace(&js_string(
        input.get("sourceText").filter(|value| !value.is_null()),
    ));
    let descriptor_value = input
        .get("descriptor")
        .filter(|descriptor| !descriptor.is_null())
        .cloned()
        .or_else(|| get_requirement_category_option_descriptor(&source_text))?;
    if !js_truthy(&descriptor_value) {
        return None;
    }
    let category = descriptor_value.get("category").cloned();
    let source_category_code = descriptor_value.get("sourceCategoryCode").cloned();
    let long_label = descriptor_value
        .get("longLabel")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let credit_range = input.get("creditRange").filter(|range| !range.is_null());
    let parsed_range = if credit_range.is_none() {
        parse_requirement_credit_range(&source_text)
    } else {
        None
    };
    let explicit_credit_min = credit_range
        .and_then(|range| range.get("minCredits"))
        .filter(|value| !value.is_null());
    let credit_min = if let Some(value) = explicit_credit_min {
        value.as_f64()?
    } else {
        parsed_range.map(|range| range.0).or_else(|| {
            parse_category_option_credit_amount(
                &source_text,
                input.get("fallbackCredits").and_then(Value::as_f64),
            )
        })?
    };
    if !credit_min.is_finite() || credit_min <= 0.0 {
        return None;
    }
    let credit_max = credit_range
        .and_then(|range| range.get("maxCredits"))
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            if credit_range.is_none() {
                parsed_range
                    .and_then(|range| range.1)
                    .map(|credit_max| json!(credit_max))
            } else {
                None
            }
        })
        .unwrap_or_else(|| json!(credit_min));
    let credit_max_is_same_number = credit_max.as_f64().is_some_and(|value| value == credit_min);
    let credit_text = if js_truthy(&credit_max) && !credit_max_is_same_number {
        format!(
            "{}-{}",
            js_number_string(credit_min),
            js_string_value(&credit_max)
        )
    } else {
        js_number_string(credit_min)
    };
    let source_category_code_text = source_category_code
        .as_ref()
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let title = format!("{credit_text} credits of {long_label} ({source_category_code_text})");
    let program_specific_value = input
        .get("programSpecific")
        .filter(|value| !value.is_null())
        .or_else(|| {
            descriptor_value
                .get("programSpecific")
                .filter(|value| !value.is_null())
        })
        .cloned()
        .unwrap_or_else(|| json!(false));
    let program_specific = input.get("programSpecific").is_some_and(js_truthy)
        || descriptor_value
            .get("programSpecific")
            .is_some_and(js_truthy);
    let approved_list_key = input
        .get("approvedListKey")
        .filter(|value| !value.is_null())
        .or_else(|| {
            descriptor_value
                .get("approvedListKey")
                .filter(|value| !value.is_null())
        })
        .cloned()
        .unwrap_or(Value::Null);
    let mut category_option = Map::new();
    if let Some(category) = &category {
        category_option.insert("category".into(), category.clone());
    }
    if let Some(source_category_code) = &source_category_code {
        category_option.insert("sourceCategoryCode".into(), source_category_code.clone());
    }
    category_option.insert("title".into(), json!(title));
    category_option.insert("credits".into(), json!(credit_min));
    category_option.insert("creditMin".into(), json!(credit_min));
    category_option.insert("creditMax".into(), credit_max.clone());
    category_option.insert("sourceText".into(), json!(source_text));
    category_option.insert("approvedListKey".into(), approved_list_key);
    category_option.insert("programSpecific".into(), program_specific_value);

    let mut option = Map::new();
    if let Some(id) = input.get("id") {
        option.insert("id".into(), id.clone());
    }
    option.insert("optionKind".into(), json!("category-option"));
    option.insert("displayCourseCodes".into(), json!([]));
    option.insert("uwCourses".into(), json!([]));
    option.insert("equivalentUwCourseCodes".into(), json!([]));
    option.insert("credits".into(), json!(credit_min));
    option.insert("creditMin".into(), json!(credit_min));
    option.insert("creditMax".into(), credit_max);
    option.insert("creditText".into(), json!(credit_text));
    option.insert("maxCredits".into(), Value::Null);
    option.insert("title".into(), json!(title));
    option.insert("department".into(), Value::Null);
    if let Some(category) = category {
        option.insert("category".into(), category);
    }
    if let Some(source_heading) = input.get("sourceHeading") {
        option.insert("sourceHeading".into(), source_heading.clone());
    }
    if let Some(source_category) = input.get("sourceCategory") {
        option.insert("sourceCategory".into(), source_category.clone());
    }
    option.insert("grcMatches".into(), json!([]));
    option.insert("categoryOption".into(), Value::Object(category_option));
    option.insert("constraints".into(), json!([]));
    option.insert(
        "notes".into(),
        json!([if program_specific {
            "Program-specific category option; use the approved-list filter rather than generic Area of Inquiry matching."
        } else {
            "Category option; no specific Green River course is invented."
        }]),
    );
    option.insert("label".into(), json!(title));
    Some(Value::Object(option))
}

pub fn build_category_requirement_options_from_choice_line(
    owner: &Value,
    normalized_line: &str,
    fallback_credits: Option<f64>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    if !category_choice_line_pattern().is_match(normalized_line) {
        return Vec::new();
    }
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let segments = category_choice_split_pattern()
        .split(normalized_line)
        .map(normalize_whitespace)
        .filter(|segment| !segment.is_empty())
        .filter(|segment| course_parser.extract_from_line(segment).is_empty())
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    let mut options = Vec::new();
    for (index, segment) in segments.into_iter().enumerate() {
        let slug = slugify_requirement_identifier(&format!("category-{segment}-{}", index + 1));
        let mut input = Map::new();
        input.insert(
            "id".into(),
            json!(format!("{owner_id}:requirement-option:{slug}")),
        );
        input.insert("sourceText".into(), json!(segment));
        input.insert("sourceHeading".into(), json!(normalized_line));
        input.insert("sourceCategory".into(), json!("source-choice"));
        if let Some(fallback_credits) = fallback_credits {
            input.insert("fallbackCredits".into(), json!(fallback_credits));
        }
        let Some(option) = build_category_requirement_option(&Value::Object(input)) else {
            continue;
        };
        let category = option
            .pointer("/categoryOption/category")
            .map(js_string_value)
            .unwrap_or_default();
        let credits = option
            .pointer("/categoryOption/credits")
            .map(js_string_value)
            .unwrap_or_default();
        let source_text = option
            .pointer("/categoryOption/sourceText")
            .map(js_string_value)
            .unwrap_or_default();
        if seen.insert((category, credits, source_text)) {
            options.push(option);
        }
    }
    options
}

fn parse_category_option_credit_amount(text: &str, fallback_credits: Option<f64>) -> Option<f64> {
    let normalized = normalize_whitespace(text);
    if let Some(captures) = parenthetical_credit_amount_pattern().captures(&normalized) {
        return captures
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok());
    }
    if let Some(captures) = explicit_credit_amount_pattern().captures(&normalized) {
        return captures
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok());
    }
    fallback_credits
}

fn category_choice_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\bor\b|choose|select|one of"))
}
fn category_choice_split_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\bor\b|;|•"))
}

fn category_pattern(category: &str) -> &'static Regex {
    match category {
        "NSC" => category_nsc_pattern(),
        "AH" => category_ah_pattern(),
        "SSC" => category_ssc_pattern(),
        "QSR" => category_qsr_pattern(),
        "VLPA" => category_vlpa_pattern(),
        "DIV" => category_div_pattern(),
        "NW" => category_nw_pattern(),
        "IANDS" => category_iands_pattern(),
        _ => category_never_pattern(),
    }
}

fn category_nsc_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\b(?:N\s*Sc|natural sciences?|natural science)\b"))
}
fn category_ah_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        category_js_regex(
            r"(?i)\b(?:A\s*&\s*H|arts?\s+(?:and|&)\s+humanities|humanities|fine arts?)\b",
        )
    })
}
fn category_ssc_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\b(?:S\s*Sc|social sciences?)\b"))
}
fn category_qsr_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\bQSR\b|\bquantitative and symbolic reasoning\b"))
}
fn category_vlpa_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        category_js_regex(r"(?i)\bVLPA\b|\bvisual,\s*literary,\s*and performing arts\b")
    })
}
fn category_div_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\bDIV\b|\bDiversity\s+Requirement\b|\b(?:\d+(?:\.\d+)?\s+(?:credits?\s+of\s+)?Diversity|Diversity\s*\([^)]*\)|Diversity\s*:\s*\d|Diversity\s+credits?)\b"))
}
fn category_nw_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\bNW\b|\bnatural world\b"))
}
fn category_iands_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\bI\s*&\s*S\b|\bindividuals\s+and\s+societies\b"))
}
fn category_never_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"\b\B"))
}
fn parenthetical_credit_amount_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"\(\s*(\d+(?:\.\d+)?)\s*\)"))
}
fn explicit_credit_amount_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| category_js_regex(r"(?i)\b(\d+(?:\.\d+)?)\s*(?:credits?|cr)\b"))
}

fn category_js_regex(pattern: &str) -> Regex {
    Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
        pattern,
    ))
    .expect("category option JavaScript regex")
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn js_string(value: Option<&Value>) -> String {
    value.map(js_string_value).unwrap_or_default()
}

fn js_string_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value
            .as_f64()
            .map(js_number_string)
            .unwrap_or_else(|| value.to_string()),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_string_value(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn js_number_string(value: f64) -> String {
    super::requirement_shape::js_number_f64_to_string(value)
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
