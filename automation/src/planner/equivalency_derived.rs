use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedRuleSynthesisInput {
    #[serde(default)]
    pub official_rules: Vec<Value>,
    #[serde(default)]
    pub structured_rules: Vec<Value>,
    pub items: Vec<DerivedRuleSynthesisItem>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedRuleSynthesisItem {
    pub spec: Value,
    #[serde(default)]
    pub support_rules: Vec<Value>,
    #[serde(default)]
    pub support_rule_selectors: Vec<SupportRuleSelector>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SupportRuleSelector {
    SingleCourse {
        source_course_code: String,
        #[serde(default)]
        rule_status: Option<String>,
        #[serde(default)]
        target_includes: Option<String>,
    },
    ExactSourceSet {
        source_course_set: Vec<String>,
        #[serde(default)]
        rule_status: Option<String>,
        #[serde(default)]
        target_includes: Option<String>,
    },
    StructuredRule {
        id: String,
    },
}

pub fn synthesize_equivalency_rules(
    input: &DerivedRuleSynthesisInput,
) -> Result<Vec<Value>, String> {
    input
        .items
        .iter()
        .map(|item| {
            let selected_support_rules = if item.support_rule_selectors.is_empty() {
                item.support_rules.clone()
            } else {
                item.support_rule_selectors
                    .iter()
                    .map(|selector| resolve_support_rule_selector(input, selector))
                    .collect()
            };
            synthesize_one(&item.spec, &selected_support_rules)
        })
        .collect()
}

fn synthesize_one(spec: &Value, selected_support_rules: &[Value]) -> Result<Value, String> {
    let support_rules: Vec<_> = selected_support_rules
        .iter()
        .filter(|rule| !rule.is_null())
        .collect();
    let id =
        string_field(spec, "id").ok_or_else(|| "derived rule spec is missing id".to_string())?;
    let title = string_field(spec, "title")
        .ok_or_else(|| format!("derived rule spec {id:?} is missing title"))?;
    if support_rules.is_empty() {
        return Err(format!(
            "Unable to synthesize {id}: no supporting official guide rules were found."
        ));
    }

    let source_course_sets = spec
        .get("sourceCourseSets")
        .filter(|value| !value.is_null())
        .map(unique_course_sets)
        .unwrap_or_default();
    let source_course_label = optional_string_field(spec, "sourceCourseLabel")
        .unwrap_or_else(|| build_source_course_label(&source_course_sets));
    let effective_year_ranges = spec
        .get("effectiveYearRanges")
        .filter(|value| !value.is_null())
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| build_continuous_effective_ranges(&support_rules));
    let target_course_codes = spec
        .get("targetCourseCodes")
        .filter(|value| !value.is_null())
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| merge_target_course_codes(&support_rules));
    let target_requirement_tags = spec
        .get("targetRequirementTags")
        .filter(|value| !value.is_null())
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| merge_target_requirement_tags(&support_rules));
    let rule_type = optional_string_field(spec, "type").unwrap_or_else(|| "chain-rule".to_string());
    let acceptance_category = string_field(spec, "acceptanceCategory")
        .ok_or_else(|| format!("derived rule spec {id:?} is missing acceptanceCategory"))?;
    let rule_status = optional_string_field(spec, "ruleStatus")
        .or_else(|| {
            support_rules
                .first()
                .and_then(|rule| optional_string_field(rule, "ruleStatus"))
        })
        .unwrap_or_else(|| "active".to_string());
    let target_outcome = spec.get("targetOutcome").cloned().unwrap_or(Value::Null);
    let weaker_than_rule_ids = spec
        .get("weakerThanRuleIds")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let effective_date_label = match spec.get("effectiveDateLabel") {
        Some(value) => value.clone(),
        None => build_effective_date_label(&effective_year_ranges),
    };
    let guide_department = optional_string_field(spec, "guideDepartment")
        .or_else(|| build_guide_department(&support_rules));
    let source_course_title = optional_string_field(spec, "sourceCourseTitle")
        .or_else(|| build_source_course_title(&support_rules));
    let planner_warnings = unique_planner_text(
        spec.get("plannerWarnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    );
    let notes = build_notes(spec, &support_rules, &title);
    let source_links = unique_source_links(&support_rules);
    let is_obsolete_source_course = support_rules
        .iter()
        .all(|rule| rule.get("isObsoleteSourceCourse") == Some(&Value::Bool(true)));

    let mut rule = Map::new();
    rule.insert("id".to_string(), Value::String(id.to_string()));
    rule.insert("type".to_string(), Value::String(rule_type));
    rule.insert("title".to_string(), Value::String(title.to_string()));
    rule.insert(
        "acceptanceCategory".to_string(),
        Value::String(acceptance_category),
    );
    rule.insert("ruleStatus".to_string(), Value::String(rule_status));
    rule.insert(
        "sourceKind".to_string(),
        Value::String("uw-green-river-equivalency-guide-derived".to_string()),
    );
    rule.insert(
        "sourceSchoolId".to_string(),
        Value::String("grc".to_string()),
    );
    rule.insert("targetSchoolIds".to_string(), json!(["uw-seattle"]));
    rule.insert("sourceCourseSets".to_string(), json!(source_course_sets));
    rule.insert(
        "targetCourseCodes".to_string(),
        Value::Array(target_course_codes),
    );
    rule.insert("targetOutcome".to_string(), target_outcome);
    rule.insert("weakerThanRuleIds".to_string(), weaker_than_rule_ids);
    rule.insert(
        "effectiveYearRanges".to_string(),
        Value::Array(effective_year_ranges),
    );
    rule.insert("effectiveDateLabel".to_string(), effective_date_label);
    rule.insert(
        "guideDepartment".to_string(),
        guide_department.map(Value::String).unwrap_or(Value::Null),
    );
    rule.insert(
        "sourceCourseLabel".to_string(),
        Value::String(source_course_label),
    );
    rule.insert(
        "sourceCourseTitle".to_string(),
        source_course_title
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    rule.insert(
        "targetRequirementTags".to_string(),
        Value::Array(target_requirement_tags),
    );
    rule.insert(
        "isObsoleteSourceCourse".to_string(),
        Value::Bool(is_obsolete_source_course),
    );
    rule.insert("parsedFromOfficialGuide".to_string(), Value::Bool(false));
    rule.insert(
        "plannerWarnings".to_string(),
        Value::Array(planner_warnings.into_iter().map(Value::String).collect()),
    );
    rule.insert("notes".to_string(), Value::Array(notes));
    rule.insert("sourceLinks".to_string(), Value::Array(source_links));
    Ok(Value::Object(rule))
}

fn resolve_support_rule_selector(
    input: &DerivedRuleSynthesisInput,
    selector: &SupportRuleSelector,
) -> Value {
    match selector {
        SupportRuleSelector::StructuredRule { id } => input
            .structured_rules
            .iter()
            .find(|rule| string_field(rule, "id").as_deref() == Some(id.as_str()))
            .cloned()
            .unwrap_or(Value::Null),
        SupportRuleSelector::SingleCourse {
            source_course_code,
            rule_status,
            target_includes,
        } => {
            let normalized_code = normalize_course_code(source_course_code);
            find_best_support_rule(
                input.official_rules.iter().filter(|rule| {
                    rule.get("sourceCourseSets")
                        .and_then(Value::as_array)
                        .and_then(|sets| sets.first())
                        .and_then(Value::as_array)
                        .is_some_and(|first_set| first_set.len() == 1)
                        && source_course_codes(rule).contains(&normalized_code)
                }),
                rule_status.as_deref(),
                target_includes.as_deref(),
            )
            .cloned()
            .unwrap_or(Value::Null)
        }
        SupportRuleSelector::ExactSourceSet {
            source_course_set,
            rule_status,
            target_includes,
        } => {
            let expected = sorted_unique_course_codes(source_course_set.iter().map(String::as_str));
            find_best_support_rule(
                input.official_rules.iter().filter(|rule| {
                    rule.get("sourceCourseSets")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|source_set| normalized_course_set(source_set) == expected)
                }),
                rule_status.as_deref(),
                target_includes.as_deref(),
            )
            .cloned()
            .unwrap_or(Value::Null)
        }
    }
}

fn find_best_support_rule<'a>(
    rules: impl Iterator<Item = &'a Value>,
    rule_status: Option<&str>,
    target_includes: Option<&str>,
) -> Option<&'a Value> {
    let required_status = rule_status.filter(|status| !status.is_empty());
    let required_target = target_includes
        .filter(|target| !target.is_empty())
        .map(normalize_course_code);
    rules
        .filter(|rule| {
            required_status
                .is_none_or(|status| string_field(rule, "ruleStatus").as_deref() == Some(status))
        })
        .filter(|rule| {
            required_target
                .as_ref()
                .is_none_or(|target| merged_target_codes_for_rule(rule).contains(target))
        })
        .min_by(|left, right| compare_support_rules(left, right))
}

fn compare_support_rules(left: &Value, right: &Value) -> std::cmp::Ordering {
    support_status_priority(left)
        .cmp(&support_status_priority(right))
        .then_with(|| support_acceptance_priority(left).cmp(&support_acceptance_priority(right)))
        .then_with(|| {
            string_field(left, "id")
                .unwrap_or_default()
                .cmp(&string_field(right, "id").unwrap_or_default())
        })
}

fn support_status_priority(rule: &Value) -> u8 {
    match string_field(rule, "ruleStatus").as_deref() {
        Some("active") => 0,
        Some("legacy") => 1,
        Some("deprecated") => 2,
        _ => 1,
    }
}

fn support_acceptance_priority(rule: &Value) -> u8 {
    match string_field(rule, "acceptanceCategory").as_deref() {
        Some("preferred") => 0,
        Some("accepted") => 1,
        Some("accepted-with-warning") => 2,
        Some("legacy-accepted") => 3,
        Some("no-credit") => 4,
        _ => 3,
    }
}

fn source_course_codes(rule: &Value) -> Vec<String> {
    sorted_unique_course_codes(
        rule.get("sourceCourseSets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_str),
    )
}

fn normalized_course_set(value: &Value) -> Vec<String> {
    sorted_unique_course_codes(
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str),
    )
}

fn merged_target_codes_for_rule(rule: &Value) -> Vec<String> {
    let explicit = rule
        .get("targetCourseCodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    let extracted = string_field(rule, "targetOutcome")
        .map(|outcome| extract_course_codes_from_freeform_text(&outcome))
        .unwrap_or_default();
    sorted_unique_course_codes(explicit.chain(extracted.iter().map(String::as_str)))
}

fn unique_course_sets(value: &Value) -> Vec<Vec<String>> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|course_set| {
            sorted_unique_course_codes(
                course_set
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            )
        })
        .collect()
}

fn sorted_unique_course_codes<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut codes: Vec<_> = values
        .into_iter()
        .map(normalize_course_code)
        .filter(|value| !value.is_empty())
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

fn normalize_course_code(value: &str) -> String {
    let normalized = normalize_text(value).to_ascii_uppercase();
    if normalized == "MATH& 254" {
        "MATH& 264".to_string()
    } else {
        normalized
    }
}

fn build_source_course_label(source_course_sets: &[Vec<String>]) -> String {
    source_course_sets
        .iter()
        .map(|course_set| course_set.join(" + "))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn merge_target_course_codes(support_rules: &[&Value]) -> Vec<Value> {
    let mut codes = Vec::new();
    for rule in support_rules {
        for code in rule
            .get("targetCourseCodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            codes.push(code.to_string());
        }
        if let Some(outcome) = string_field(rule, "targetOutcome") {
            codes.extend(extract_course_codes_from_freeform_text(&outcome));
        }
    }
    sorted_unique_course_codes(codes.iter().map(String::as_str))
        .into_iter()
        .map(Value::String)
        .collect()
}

fn merge_target_requirement_tags(support_rules: &[&Value]) -> Vec<Value> {
    let tags = unique_planner_text(support_rules.iter().flat_map(|rule| {
        rule.get("targetRequirementTags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }));
    let mut tags = tags;
    tags.sort();
    tags.into_iter().map(Value::String).collect()
}

fn build_continuous_effective_ranges(support_rules: &[&Value]) -> Vec<Value> {
    let mut ranges = Vec::new();
    for range in support_rules.iter().flat_map(|rule| {
        rule.get("effectiveYearRanges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }) {
        if string_field(range, "startLabel")
            .as_deref()
            .is_some_and(|label| label.eq_ignore_ascii_case("legacy-planner-support"))
        {
            continue;
        }
        ranges.push(range);
    }
    if ranges.is_empty() {
        return Vec::new();
    }
    let starts: Vec<_> = ranges
        .iter()
        .filter_map(|range| string_field(range, "startLabel"))
        .filter(|label| !label.is_empty())
        .collect();
    let ends: Vec<_> = ranges
        .iter()
        .filter_map(|range| string_field(range, "endLabel"))
        .filter(|label| !label.is_empty())
        .collect();
    let Some(latest_start) = starts
        .iter()
        .max_by(|left, right| compare_term_labels(left, right))
    else {
        return Vec::new();
    };
    let earliest_end = ends
        .iter()
        .min_by(|left, right| compare_term_labels(left, right));
    let mut range = Map::new();
    range.insert(
        "startLabel".to_string(),
        Value::String(latest_start.clone()),
    );
    range.insert(
        "endLabel".to_string(),
        earliest_end
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    range.insert(
        "note".to_string(),
        Value::String("Derived from the overlapping effective dates of the supporting official UW equivalency-guide rows.".to_string()),
    );
    vec![Value::Object(range)]
}

fn compare_term_labels(left: &str, right: &str) -> std::cmp::Ordering {
    let left_key = guide_term_sort_key(left);
    let right_key = guide_term_sort_key(right);
    match (left_key, right_key) {
        (Some(left), Some(right)) if left != right => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => left.cmp(right),
    }
}

fn guide_term_sort_key(label: &str) -> Option<i64> {
    let captures = Regex::new(r"(?i)\b(WIN|SPR|SUM|AUT)\s+QTR\.\s+(\d{4})\b")
        .ok()?
        .captures(label)?;
    let order = match captures.get(1)?.as_str().to_ascii_uppercase().as_str() {
        "WIN" => 1,
        "SPR" => 2,
        "SUM" => 3,
        "AUT" => 4,
        _ => return None,
    };
    captures
        .get(2)?
        .as_str()
        .parse::<i64>()
        .ok()
        .map(|year| year * 10 + order)
}

fn build_effective_date_label(ranges: &[Value]) -> Value {
    let Some(range) = ranges.first() else {
        return Value::Null;
    };
    let Some(start) = string_field(range, "startLabel") else {
        return Value::Null;
    };
    match string_field(range, "endLabel").filter(|value| !value.is_empty()) {
        Some(end) => Value::String(format!("{start} thru {end}")),
        None => Value::String(start),
    }
}

fn build_guide_department(support_rules: &[&Value]) -> Option<String> {
    unique_planner_text(
        support_rules
            .iter()
            .filter_map(|rule| rule.get("guideDepartment")),
    )
    .first()
    .cloned()
}

fn build_source_course_title(support_rules: &[&Value]) -> Option<String> {
    let titles = unique_planner_text(
        support_rules
            .iter()
            .filter_map(|rule| rule.get("sourceCourseTitle")),
    );
    if titles.is_empty() {
        None
    } else {
        Some(titles.join(" / "))
    }
}

fn unique_planner_text<'a>(values: impl IntoIterator<Item = &'a Value>) -> Vec<String> {
    unique_strings(
        values
            .into_iter()
            .filter_map(Value::as_str)
            .map(str::to_string),
    )
}

fn unique_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut output = Vec::new();
    for value in values {
        let normalized = normalize_text(&value);
        if !normalized.is_empty() && !output.contains(&normalized) {
            output.push(normalized);
        }
    }
    output
}

fn build_notes(spec: &Value, support_rules: &[&Value], title: &str) -> Vec<Value> {
    let mut notes = vec![format!(
        "Synthesized from parsed official UW Green River equivalency-guide rows for {title}."
    )];
    let mut rule_ids: Vec<_> = support_rules
        .iter()
        .filter_map(|rule| string_field(rule, "id"))
        .collect();
    rule_ids.sort();
    rule_ids.dedup();
    notes.extend(
        rule_ids
            .iter()
            .map(|id| format!("Supporting official guide rule: {id}.")),
    );
    for rule in support_rules {
        let id = string_field(rule, "id").unwrap_or_default();
        let source_label = rule
            .get("sourceCourseLabel")
            .filter(|value| !value.is_null())
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| string_field(rule, "title"))
            .unwrap_or_default();
        notes.push(format!(
            "Derived from official guide rule {id}: {}.",
            normalize_text(&source_label)
        ));
    }
    notes.extend(
        spec.get("notes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|value| value.to_string()),
    );
    unique_strings(notes)
        .into_iter()
        .map(Value::String)
        .collect()
}

fn unique_source_links(support_rules: &[&Value]) -> Vec<Value> {
    let mut by_url = Vec::<(String, Value)>::new();
    let derived_link = json!({
        "label": "UW Green River derived equivalency synthesis",
        "url": "https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/",
        "note": "Structured transfer-planner rule derived from parsed official UW Green River equivalency-guide rows."
    });
    insert_link(&mut by_url, derived_link);
    for rule in support_rules {
        for link in rule
            .get("sourceLinks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            insert_link(&mut by_url, link.clone());
        }
    }
    by_url.sort_by(|left, right| {
        string_field(&left.1, "label")
            .unwrap_or_default()
            .cmp(&string_field(&right.1, "label").unwrap_or_default())
            .then_with(|| left.0.cmp(&right.0))
    });
    by_url.into_iter().map(|(_, value)| value).collect()
}

fn insert_link(links: &mut Vec<(String, Value)>, link: Value) {
    let Some(url) = string_field(&link, "url").filter(|url| !url.is_empty()) else {
        return;
    };
    if let Some((_, existing)) = links
        .iter_mut()
        .find(|(existing_url, _)| existing_url == &url)
    {
        *existing = link;
    } else {
        links.push((url, link));
    }
}

fn extract_course_codes_from_freeform_text(value: &str) -> Vec<String> {
    let mut normalized = normalize_text(value).replace("Â§", "§");
    let marker = Regex::new(r"^[§*]\s*").unwrap();
    normalized = marker.replace(&normalized, "").into_owned();
    let parentheses = Regex::new(r"\([^)]*\)").unwrap();
    normalized = parentheses.replace_all(&normalized, " ").into_owned();
    let separators = Regex::new(r"[;/]").unwrap();
    normalized = separators.replace_all(&normalized, " , ").into_owned();
    let alternatives = Regex::new(r"(?i)\b(?:or|and)\b").unwrap();
    normalized = alternatives.replace_all(&normalized, " , ").into_owned();
    let tokens = Regex::new(r"(?i)[A-Z&-]+|[1-4]XX|\d{3}(?:\.\d+)?[A-Z]?|,").unwrap();
    let subject = Regex::new(r"(?i)^[A-Z][A-Z&-]{0,14}&?$").unwrap();
    let number = Regex::new(r"(?i)^(?:\d{3}(?:\.\d+)?[A-Z]?|[1-4]XX)$").unwrap();
    let values: Vec<_> = tokens
        .find_iter(&normalized)
        .map(|capture| capture.as_str().to_ascii_uppercase())
        .collect();
    let mut current_subject = None::<String>;
    let mut codes = Vec::new();
    let mut index = 0;
    while index < values.len() {
        let token = &values[index];
        let next = values
            .get(index + 1)
            .map(String::as_str)
            .unwrap_or_default();
        if token == "," {
            index += 1;
            continue;
        }
        if subject.is_match(token) && !next.is_empty() && number.is_match(next) {
            let subject = normalize_text(token).to_ascii_uppercase();
            current_subject = Some(subject.clone());
            push_unique(
                &mut codes,
                normalize_course_code(&format!("{subject} {next}")),
            );
            index += 2;
            continue;
        }
        if let Some(subject) = current_subject.as_ref() {
            if number.is_match(token) {
                push_unique(
                    &mut codes,
                    normalize_course_code(&format!("{subject} {token}")),
                );
            }
        }
        index += 1;
    }
    codes
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn optional_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .filter(|field| !field.is_null())
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesizes_derived_rule_with_merged_courses_dates_notes_and_links() {
        let input: DerivedRuleSynthesisInput = serde_json::from_value(json!({
            "items": [{
                "spec": {
                    "id": "chain:chem",
                    "title": "Chemistry sequence",
                    "acceptanceCategory": "accepted-with-warning",
                    "targetOutcome": "UW CHEM 142 and CHEM 152",
                    "plannerWarnings": ["Keep the sequence whole.", " Keep the sequence whole. "],
                    "notes": ["Additional context."]
                },
                "supportRules": [
                    {
                        "id": "guide:chem-161",
                        "ruleStatus": "active",
                        "sourceCourseSets": [["CHEM& 161"]],
                        "targetCourseCodes": ["CHEM 142"],
                        "targetOutcome": "CHEM 142",
                        "sourceCourseLabel": "CHEM& 161",
                        "sourceCourseTitle": "General Chemistry I",
                        "guideDepartment": "Chemistry",
                        "effectiveYearRanges": [{"startLabel":"WIN QTR. 2024", "endLabel":"AUT QTR. 2026", "note":"row"}],
                        "targetRequirementTags": ["NW", "QSR"],
                        "isObsoleteSourceCourse": false,
                        "sourceLinks": [{"label":"Official guide", "url":"https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/", "note":"official"}]
                    },
                    {
                        "id": "guide:chem-162",
                        "ruleStatus": "active",
                        "sourceCourseSets": [["CHEM& 162"]],
                        "targetCourseCodes": ["CHEM 152"],
                        "targetOutcome": "CHEM 152",
                        "sourceCourseLabel": "CHEM& 162",
                        "sourceCourseTitle": "General Chemistry II",
                        "guideDepartment": "Chemistry",
                        "effectiveYearRanges": [{"startLabel":"SPR QTR. 2025", "endLabel":"SPR QTR. 2026", "note":"row"}],
                        "targetRequirementTags": ["QSR"],
                        "isObsoleteSourceCourse": false,
                        "sourceLinks": [{"label":"Official guide", "url":"https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/", "note":"official"}]
                    }
                ]
            }]
        })).unwrap();
        let rules = synthesize_equivalency_rules(&input).unwrap();
        let rule = &rules[0];
        assert_eq!(rule["id"], "chain:chem");
        assert_eq!(rule["type"], "chain-rule");
        assert_eq!(rule["sourceCourseSets"], json!([]));
        assert_eq!(rule["sourceCourseLabel"], "");
        assert_eq!(rule["targetCourseCodes"], json!(["CHEM 142", "CHEM 152"]));
        assert_eq!(rule["targetRequirementTags"], json!(["NW", "QSR"]));
        assert_eq!(rule["ruleStatus"], "active");
        assert_eq!(
            rule["effectiveDateLabel"],
            "SPR QTR. 2025 thru SPR QTR. 2026"
        );
        assert_eq!(rule["guideDepartment"], "Chemistry");
        assert_eq!(
            rule["sourceCourseTitle"],
            "General Chemistry I / General Chemistry II"
        );
        assert_eq!(rule["plannerWarnings"], json!(["Keep the sequence whole."]));
        assert_eq!(rule["notes"].as_array().unwrap().len(), 6);
        assert_eq!(
            rule["sourceLinks"],
            json!([{
                "label":"Official guide", "url":"https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/", "note":"official"
            }])
        );
    }

    #[test]
    fn extracts_course_codes_from_freeform_outcomes_with_carried_subjects() {
        assert_eq!(
            extract_course_codes_from_freeform_text("UW CHEM 142 and CHEM 152; see also CHEM& 161"),
            vec!["CHEM 142", "CHEM 152", "CHEM& 161"]
        );
        assert_eq!(
            extract_course_codes_from_freeform_text("§ MATH& 254 or MATH& 255"),
            vec!["MATH& 264", "MATH& 255"]
        );
    }

    #[test]
    fn fails_closed_when_a_derived_rule_has_no_support() {
        let input: DerivedRuleSynthesisInput = serde_json::from_value(json!({
            "items": [{"spec":{"id":"chain:missing","title":"Missing","acceptanceCategory":"warning","targetOutcome":"none"},"supportRules":[null]}]
        })).unwrap();
        assert!(
            synthesize_equivalency_rules(&input)
                .unwrap_err()
                .contains("no supporting official guide rules")
        );
    }
}
