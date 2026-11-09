use regex::Regex;
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::source_pipeline_validation::SourcePipelineValidationCheck;

fn patterns() -> &'static HashMap<&'static str, Regex> {
    static PATTERNS: OnceLock<HashMap<&'static str, Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            (
                "requirement-cue",
                r"(?i)\b(degree requirements?|major requirements?|graduation requirements?|major admissions requirements?|program requirements?|curriculum|worksheet|checklist|prerequisites?|bachelor(?:\s+of)?|undergraduate major admission|undergraduate program)\b",
            ),
            (
                "primary-requirement-cue",
                r"(?i)\bdegree requirements?\b|\bmajor requirements?\b|\bgraduation requirements?\b|\bprogram requirements?\b|\bstudent requirements?\b|\brequired courses?\b|\bcourse overview\b|\bcredit structure\b|\bcredits?\s*&\s*grade requirements?\b|\bdegree structure\b|\brequirements packet\b|\bdegreq\b",
            ),
            (
                "dedicated-requirement-cue",
                r"(?i)\bdegree sheet\b|\brequirement sheet\b|\bchecklist\b|\brequirements packet\b|\bdegreq\b",
            ),
            (
                "weak-primary-url",
                r"(?i)(?:/|^)(timeline(?:-and-requirements)?|graduate(?:-program|-admissions)?|phd|research|faculty|news|about)(?:[-/?#]|$)",
            ),
            (
                "weak-primary-heading",
                r"(?i)\b(timeline(?:\s*&\s*requirements)?|graduate program|graduate admissions|ms requirements?|phd requirements?|research|faculty|news)\b",
            ),
            ("overview-only", r"(?i)\boverview\b"),
            (
                "year-specific-requirement-source",
                r"(?i)\b(worksheet|checklist|degree requirements?|major requirements?|graduation requirements?|program requirements?|curriculum|degree sheet|requirement sheet)\b",
            ),
            ("singular-route", r"(?i)\b(?:option|concentration|track)\b"),
            ("plural-route", r"(?i)\b(?:options|concentrations|tracks)\b"),
            (
                "shared-tacoma-catalog",
                r"(?i)//(?:www\.)?washington\.edu/students/gencat/program/",
            ),
            ("linked-document", r"(?i)\.(?:pdf|docx)(?:$|[?#])"),
            ("degree-bsce", r"(?i)\bbsce\b|\bb\.?\s*s\.?\s*c\.?\s*e\.?\b"),
            ("degree-bsn", r"(?i)\bbsn\b|\bb\.?\s*s\.?\s*n\.?\b"),
            ("degree-bm", r"(?i)\bbm\b|\bb\.?\s*m\.?\b|\bbachelor\s+of\s+music\b"),
            ("degree-bs", r"(?i)\bbs\b|\bb\.?\s*s\.?\b|\bbachelor\s+of\s+science\b"),
            ("degree-ba", r"(?i)\bba\b|\bb\.?\s*a\.?\b|\bbachelor\s+of\s+arts\b"),
            (
                "graduate-route",
                r"(?i)\b(?:graduate|masters?|master(?:'s)?|m\.?\s*s\.?|m\.?\s*a\.?|ph\.?\s*d\.?|doctoral|doctor(?:\s+of\s+philosophy)?)\b",
            ),
            (
                "graduate-path",
                r"(?i)/(?:student/)?(?:applied-masters|masters?|graduate|amp)(?:[-/?#]|$)",
            ),
            (
                "undergraduate-route",
                r"(?i)\b(?:undergrad(?:uate)?|bachelor|b\.?\s*s\.?|b\.?\s*a\.?)\b|/undergrad(?:uate)?(?:[-/?#]|$)",
            ),
            ("year", r"(?:^|[^0-9])(20[0-9]{2})(?:$|[^0-9])"),
        ]
        .into_iter()
        .map(|(key, pattern)| {
            (
                key,
                Regex::new(pattern).expect("weak-existing-primary regex must compile"),
            )
        })
        .collect()
    })
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn array<'a>(value: Option<&'a Value>) -> &'a [Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn js_string(value: &Value) -> String {
    super::publish_readiness::js_string(value)
}

fn texts(values: impl IntoIterator<Item = Value>) -> Vec<String> {
    values
        .into_iter()
        .filter(is_truthy)
        .map(|value| js_string(&value))
        .collect()
}

fn degree_tokens(text: &str) -> HashSet<&'static str> {
    [
        ("bsce", "degree-bsce"),
        ("bsn", "degree-bsn"),
        ("bm", "degree-bm"),
        ("bs", "degree-bs"),
        ("ba", "degree-ba"),
    ]
    .into_iter()
    .filter_map(|(token, pattern)| patterns()[pattern].is_match(text).then_some(token))
    .collect()
}

/// Mirrors discovery's guard against replacing a multi-pathway major with a single-degree page.
pub fn is_single_degree_route_candidate_for_multi_pathway_major(
    target: &Value,
    candidate: &Value,
    candidate_text: &str,
) -> bool {
    if target.get("ownerType").and_then(Value::as_str) != Some("major")
        || target
            .get("pathwayCount")
            .and_then(Value::as_f64)
            .unwrap_or_default()
            < 1.0
    {
        return false;
    }
    let focused_text = texts(
        ["url", "label", "anchorText", "pageTitle"]
            .into_iter()
            .filter_map(|key| candidate.get(key).cloned()),
    )
    .join(" ");
    if degree_tokens(&focused_text).len() == 1 {
        return true;
    }
    degree_tokens(candidate_text).len() == 1
}

/// Builds the guarded review outcome used when a higher-scoring single-route page cannot replace a major primary.
pub fn build_multi_pathway_single_route_review_decision(
    target: &Value,
    candidates: &[Value],
) -> Option<Value> {
    let existing_url = target.get("existingPrimaryUrl")?.as_str()?;
    let mut primary_candidates = candidates
        .iter()
        .filter(|candidate| is_primary_discovery_candidate_eligible_for_target(candidate, target))
        .collect::<Vec<_>>();
    primary_candidates.sort_by(|left, right| compare_scored_discovery_candidates(left, right));
    let current_primary = primary_candidates
        .iter()
        .find(|candidate| candidate.get("url").and_then(Value::as_str) == Some(existing_url))
        .copied();
    let best_alternative = primary_candidates
        .iter()
        .find(|candidate| candidate.get("url").and_then(Value::as_str) != Some(existing_url))
        .copied()?;
    let alternative_text = texts(
        ["url", "label", "anchorText", "pageTitle"]
            .into_iter()
            .filter_map(|key| best_alternative.get(key).cloned()),
    )
    .join(" ");
    if !is_single_degree_route_candidate_for_multi_pathway_major(
        target,
        best_alternative,
        &alternative_text,
    ) {
        return None;
    }
    let score_delta = match (
        best_alternative.get("score").and_then(Value::as_f64),
        current_primary.and_then(|candidate| candidate.get("score").and_then(Value::as_f64)),
    ) {
        (Some(alternative), Some(current)) => json!(alternative - current),
        _ => Value::Null,
    };
    let review_candidate = if best_alternative
        .get("score")
        .and_then(Value::as_f64)
        .is_some_and(|score| score >= 12.0)
    {
        best_alternative.clone()
    } else {
        Value::Null
    };
    Some(json!({
        "suggestedPrimary": null,
        "currentPrimary": current_primary.cloned().unwrap_or(Value::Null),
        "action": "keep-existing-primary",
        "scoreDelta": score_delta,
        "reviewCandidate": review_candidate
    }))
}

fn candidate_has_source_kind(candidate: &Value, kind: &str) -> bool {
    array(candidate.get("sourceKinds"))
        .iter()
        .any(|source_kind| source_kind.as_str() == Some(kind))
}

fn search_slug_tokens(value: &str) -> Vec<String> {
    let mut slug = String::new();
    let mut previous_space = true;
    for character in value.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_space = false;
        } else if !previous_space {
            slug.push(' ');
            previous_space = true;
        }
    }
    slug.split_whitespace().map(str::to_string).collect()
}

fn bachelor_route_tokens_from_value(value: &str) -> Vec<HashSet<String>> {
    const DESCRIPTORS: &[&str] = &["arts", "science", "sciences", "music"];
    const BOUNDARIES: &[&str] = &[
        "admission",
        "admissions",
        "application",
        "apply",
        "career",
        "careers",
        "catalog",
        "course",
        "courses",
        "curriculum",
        "degree",
        "degrees",
        "department",
        "faq",
        "home",
        "major",
        "majors",
        "minor",
        "minors",
        "option",
        "options",
        "overview",
        "pathway",
        "prerequisite",
        "program",
        "programs",
        "requirement",
        "requirements",
        "school",
        "students",
        "undergraduate",
        "worksheet",
    ];
    const IGNORED: &[&str] = &[
        "and",
        "the",
        "of",
        "in",
        "for",
        "major",
        "minor",
        "program",
        "degree",
        "degrees",
        "route",
        "option",
        "track",
        "path",
        "pathway",
        "concentration",
        "arts",
        "science",
        "sciences",
        "music",
        "bachelor",
        "bothell",
        "campus",
        "seattle",
        "tacoma",
        "uw",
        "uwb",
        "uwt",
        "washington",
    ];
    let words = search_slug_tokens(value);
    let mut route_sets = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if word.as_str() != "bachelor" {
            continue;
        }
        let mut cursor = index + 1;
        if words.get(cursor).is_some_and(|word| word == "of") {
            cursor += 1;
        }
        if words
            .get(cursor)
            .is_some_and(|word| DESCRIPTORS.contains(&word.as_str()))
        {
            cursor += 1;
            if words.get(cursor).is_some_and(|word| word == "in") {
                cursor += 1;
            }
        }
        for optional in ["degree", "with"] {
            if words.get(cursor).is_some_and(|word| word == optional) {
                cursor += 1;
            }
        }
        if words
            .get(cursor)
            .is_some_and(|word| word == "a" || word == "an")
        {
            cursor += 1;
        }
        for optional in ["major", "in"] {
            if words.get(cursor).is_some_and(|word| word == optional) {
                cursor += 1;
            }
        }
        let mut route_tokens = Vec::new();
        while cursor < words.len() && route_tokens.len() < 6 {
            let token = &words[cursor];
            if BOUNDARIES.contains(&token.as_str()) {
                break;
            }
            if !IGNORED.contains(&token.as_str()) {
                route_tokens.push(token.clone());
            }
            cursor += 1;
        }
        let route_tokens = route_tokens
            .into_iter()
            .filter(|token| token.len() >= 3)
            .collect::<HashSet<_>>();
        if !route_tokens.is_empty() {
            route_sets.push(route_tokens);
        }
    }
    route_sets
}

fn candidate_bachelor_route_token_sets(candidate: &Value) -> Vec<HashSet<String>> {
    let anchor = candidate
        .get("sectionAnchor")
        .filter(|value| !value.is_null())
        .or_else(|| candidate.get("url"))
        .map(js_string)
        .unwrap_or_default();
    let credential_anchor = anchor.to_lowercase().contains("#credential-");
    let mut values = ["url", "label", "anchorText", "linkText", "pageTitle"]
        .into_iter()
        .filter_map(|key| candidate.get(key).cloned())
        .collect::<Vec<_>>();
    if !credential_anchor {
        values.extend(array(candidate.get("pageHeadings")).iter().cloned());
    }
    let mut route_sets = Vec::new();
    let url = candidate
        .get("url")
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_default();
    if let Ok(parsed) = url::Url::parse(&url) {
        if let Some(segments) = parsed.path_segments() {
            for segment in segments {
                let normalized = segment.replace(['-', '_'], " ");
                if search_slug_tokens(&normalized)
                    .iter()
                    .any(|token| token == "bachelor")
                {
                    route_sets.extend(bachelor_route_tokens_from_value(&normalized));
                }
            }
        }
    }
    for value in texts(values) {
        route_sets.extend(bachelor_route_tokens_from_value(&value));
    }
    route_sets
}

/// Mirrors CJS's veto when a labeled bachelor's route names a different major identity.
pub fn has_conflicting_bachelor_program_route(target: &Value, candidate: &Value) -> bool {
    const IGNORED: &[&str] = &[
        "and",
        "the",
        "of",
        "in",
        "for",
        "major",
        "minor",
        "program",
        "degree",
        "degrees",
        "route",
        "option",
        "track",
        "path",
        "pathway",
        "concentration",
        "arts",
        "science",
        "sciences",
        "music",
        "bachelor",
        "bothell",
        "campus",
        "seattle",
        "tacoma",
        "uw",
        "uwb",
        "uwt",
        "washington",
    ];
    let target_tokens = texts(
        ["title", "label"]
            .into_iter()
            .filter_map(|key| target.get(key).cloned()),
    )
    .join(" ");
    let target_tokens = search_slug_tokens(&target_tokens)
        .into_iter()
        .filter(|token| token.len() >= 3 && !IGNORED.contains(&token.as_str()))
        .collect::<HashSet<_>>();
    if target_tokens.is_empty() {
        return false;
    }
    candidate_bachelor_route_token_sets(candidate)
        .iter()
        .any(|route| route.iter().all(|token| !target_tokens.contains(token)))
}

fn is_catalog_section_anchor_candidate(candidate: &Value) -> bool {
    static GENERAL_CATALOG_ANCHOR: OnceLock<Regex> = OnceLock::new();
    let anchor = candidate
        .get("sectionAnchor")
        .filter(|value| !value.is_null())
        .or_else(|| candidate.get("url"))
        .map(js_string)
        .unwrap_or_default();
    anchor.to_lowercase().contains("#credential-")
        || GENERAL_CATALOG_ANCHOR
            .get_or_init(|| {
                Regex::new(r"(?i)#(?:program|credential)-UG-[A-Z0-9-]+")
                    .expect("valid general catalog anchor pattern")
            })
            .is_match(&anchor)
}

/// Checks the CJS candidate/target degree-token conflict rule, separate from bachelor-program slug matching.
pub fn has_conflicting_degree_route_tokens(target: &Value, candidate: &Value) -> bool {
    let target_text = texts(
        ["title", "label"]
            .into_iter()
            .filter_map(|key| target.get(key).cloned()),
    )
    .join(" ");
    let catalog_section_anchor = is_catalog_section_anchor_candidate(candidate);
    let mut candidate_values = ["url", "label", "anchorText", "linkText", "pageTitle"]
        .into_iter()
        .filter_map(|key| candidate.get(key).cloned())
        .collect::<Vec<_>>();
    if !catalog_section_anchor {
        candidate_values.extend(array(candidate.get("pageHeadings")).iter().cloned());
    }
    let candidate_text = texts(candidate_values).join(" ");
    let target_degrees = degree_tokens(&target_text);
    let candidate_degrees = degree_tokens(&candidate_text);
    !target_degrees.is_empty()
        && !candidate_degrees.is_empty()
        && candidate_degrees
            .iter()
            .any(|degree| !target_degrees.contains(degree))
}

/// Mirrors the base eligibility and campus-index identity gates used before primary selection.
pub fn is_primary_discovery_candidate_shape_eligible(candidate: &Value, target: &Value) -> bool {
    if candidate.is_null()
        || is_skip_navigation_discovery_candidate(candidate)
        || (candidate.get("requiresVerification") == Some(&Value::Bool(true))
            && candidate.get("verified") != Some(&Value::Bool(true)))
        || candidate.get("canCreateSchedulableRows") == Some(&Value::Bool(false))
        || candidate.get("canBePrimary") == Some(&Value::Bool(false))
        || candidate.get("sourceRoleStatus").and_then(Value::as_str) != Some("primary")
    {
        return false;
    }
    if !candidate_has_source_kind(candidate, "campus-major-index")
        || candidate_has_source_kind(candidate, "official-link")
    {
        return true;
    }
    let matches_major = candidate_number_or_zero(candidate.get("sameMajorIdentityScore")) > 0.0;
    if target.get("ownerType").and_then(Value::as_str) != Some("pathway") {
        return matches_major;
    }
    matches_major && candidate_number_or_zero(candidate.get("pathwayIdentityScore")) > 0.0
}

/// Mirrors primary eligibility with the target-specific degree and bachelor's-major conflict vetoes.
pub fn is_primary_discovery_candidate_eligible_for_target(
    candidate: &Value,
    target: &Value,
) -> bool {
    is_primary_discovery_candidate_shape_eligible(candidate, target)
        && !has_conflicting_degree_route_tokens(target, candidate)
        && !has_conflicting_bachelor_program_route(target, candidate)
}

/// CJS ordering: score, official/targeted provenance, detected year, then locale-like URL.
pub fn compare_scored_discovery_candidates(left: &Value, right: &Value) -> Ordering {
    let left_score = left.get("score").and_then(Value::as_f64);
    let right_score = right.get("score").and_then(Value::as_f64);
    if let (Some(left_score), Some(right_score)) = (left_score, right_score) {
        let score_delta = right_score - left_score;
        if score_delta != 0.0 {
            return score_delta.partial_cmp(&0.0).unwrap_or(Ordering::Equal);
        }
    }
    let provenance_count = |candidate: &Value| {
        usize::from(candidate_has_source_kind(candidate, "official-link"))
            + usize::from(candidate_has_source_kind(
                candidate,
                "targeted-official-candidate",
            ))
    };
    let provenance_order = provenance_count(right).cmp(&provenance_count(left));
    if provenance_order != Ordering::Equal {
        return provenance_order;
    }
    let year = |candidate: &Value| {
        candidate
            .get("latestDetectedYear")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MIN)
    };
    let year_order = year(right).cmp(&year(left));
    if year_order != Ordering::Equal {
        return year_order;
    }
    let left_url = left.get("url").and_then(Value::as_str).unwrap_or_default();
    let right_url = right.get("url").and_then(Value::as_str).unwrap_or_default();
    left_url
        .to_lowercase()
        .cmp(&right_url.to_lowercase())
        .then_with(|| left_url.cmp(right_url))
}

fn first_truthy_candidate_value(values: &[Option<&Value>]) -> Value {
    values
        .iter()
        .flatten()
        .find(|value| is_truthy(value))
        .map(|value| (**value).clone())
        .unwrap_or(Value::Null)
}

fn first_non_nullish_candidate_value(values: &[Option<&Value>]) -> Option<Value> {
    values
        .iter()
        .flatten()
        .find(|value| !value.is_null())
        .map(|value| (**value).clone())
}

fn candidate_string_list(values: &[Value], max_items: Option<usize>) -> Value {
    let mut unique = Vec::<String>::new();
    for value in values {
        if let Some(value) = value.as_array() {
            for item in value {
                if is_truthy(item) {
                    let item = js_string(item);
                    if !unique.contains(&item) {
                        unique.push(item);
                    }
                }
            }
        }
    }
    unique.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    if let Some(max_items) = max_items {
        unique.truncate(max_items);
    }
    Value::Array(unique.into_iter().map(Value::String).collect())
}

fn candidate_number_or_zero(value: Option<&Value>) -> f64 {
    match value {
        None | Some(Value::Null) => 0.0,
        Some(Value::Number(value)) => value.as_f64().unwrap_or(f64::NAN),
        Some(Value::Bool(value)) => f64::from(u8::from(*value)),
        Some(Value::String(value)) if value.trim().is_empty() => 0.0,
        Some(Value::String(value)) => value.parse().unwrap_or(f64::NAN),
        Some(Value::Array(values)) if values.is_empty() => 0.0,
        Some(Value::Array(values)) if values.len() == 1 => candidate_number_or_zero(values.first()),
        Some(Value::Array(_)) | Some(Value::Object(_)) => f64::NAN,
    }
}

fn candidate_score_or_negative_infinity(value: Option<&Value>) -> f64 {
    match value {
        None | Some(Value::Null) => f64::NEG_INFINITY,
        Some(value) => candidate_number_or_zero(Some(value)),
    }
}

fn candidate_json_number(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Rust port of discovery's candidate merge, including verification precedence and metadata unions.
pub fn merge_scored_discovery_candidates(existing: &Value, incoming: &Value) -> Value {
    let incoming_verifies_existing = existing.get("requiresVerification")
        == Some(&Value::Bool(true))
        && incoming.get("verified") == Some(&Value::Bool(true));
    let existing_verifies_incoming = incoming.get("requiresVerification")
        == Some(&Value::Bool(true))
        && existing.get("verified") == Some(&Value::Bool(true));
    let existing_score = candidate_score_or_negative_infinity(existing.get("score"));
    let incoming_score = candidate_score_or_negative_infinity(incoming.get("score"));
    let use_incoming = if incoming_verifies_existing {
        true
    } else if existing_verifies_incoming {
        false
    } else {
        incoming_score > existing_score
    };
    let winning = if use_incoming { incoming } else { existing };
    let losing = if use_incoming { existing } else { incoming };
    let mut merged = losing.as_object().cloned().unwrap_or_default();
    if let Some(winner_fields) = winning.as_object() {
        merged.extend(winner_fields.clone());
    }

    for field in [
        "label",
        "anchorText",
        "linkText",
        "pageTitle",
        "sourcePageUrl",
        "discoveredFromUrl",
        "sectionAnchor",
    ] {
        let value = first_truthy_candidate_value(&[
            winning.get(field),
            existing.get(field),
            incoming.get(field),
        ]);
        merged.insert(field.to_string(), value);
    }
    for field in ["sourceRole", "sourceRoleStatus", "parserType"] {
        let value = first_truthy_candidate_value(&[
            winning.get(field),
            existing.get(field),
            incoming.get(field),
        ]);
        merged.insert(
            field.to_string(),
            if value.is_null() {
                Value::String(
                    if field == "parserType" {
                        "unknown"
                    } else {
                        "ignored"
                    }
                    .to_string(),
                )
            } else {
                value
            },
        );
    }
    merged.insert(
        "pageHeadings".to_string(),
        candidate_string_list(
            &[
                existing
                    .get("pageHeadings")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                incoming
                    .get("pageHeadings")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            ],
            None,
        ),
    );
    merged.insert(
        "contentSnippets".to_string(),
        candidate_string_list(
            &[
                existing
                    .get("contentSnippets")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                incoming
                    .get("contentSnippets")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            ],
            Some(24),
        ),
    );
    merged.insert(
        "sourceKinds".to_string(),
        candidate_string_list(
            &[
                existing
                    .get("sourceKinds")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                incoming
                    .get("sourceKinds")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            ],
            None,
        ),
    );
    for field in [
        "supportOnly",
        "canBePrimary",
        "canCreateSchedulableRows",
        "requiresVerification",
    ] {
        let value = first_non_nullish_candidate_value(&[
            winning.get(field),
            existing.get(field),
            incoming.get(field),
        ]);
        merged.insert(
            field.to_string(),
            Value::Bool(value.as_ref().is_some_and(is_truthy)),
        );
    }
    merged.insert(
        "verified".to_string(),
        Value::Bool(
            [
                winning.get("verified"),
                existing.get("verified"),
                incoming.get("verified"),
            ]
            .iter()
            .any(|value| value.is_some_and(|value| is_truthy(value))),
        ),
    );
    merged.insert(
        "sameDepartment".to_string(),
        Value::Bool(
            [
                winning.get("sameDepartment"),
                existing.get("sameDepartment"),
                incoming.get("sameDepartment"),
            ]
            .iter()
            .any(|value| value.is_some_and(|value| is_truthy(value))),
        ),
    );
    for field in ["sameMajorIdentityScore", "pathwayIdentityScore"] {
        let existing_value = candidate_number_or_zero(existing.get(field));
        let incoming_value = candidate_number_or_zero(incoming.get(field));
        let maximum = if existing_value.is_nan() || incoming_value.is_nan() {
            f64::NAN
        } else {
            existing_value.max(incoming_value)
        };
        merged.insert(field.to_string(), candidate_json_number(maximum));
    }
    let existing_depth = match existing.get("discoveryDepth") {
        None | Some(Value::Null) => f64::INFINITY,
        Some(value) => candidate_number_or_zero(Some(value)),
    };
    let incoming_depth = match incoming.get("discoveryDepth") {
        None | Some(Value::Null) => f64::INFINITY,
        Some(value) => candidate_number_or_zero(Some(value)),
    };
    let minimum_depth = if existing_depth.is_nan() || incoming_depth.is_nan() {
        f64::NAN
    } else {
        existing_depth.min(incoming_depth)
    };
    merged.insert(
        "discoveryDepth".to_string(),
        candidate_json_number(minimum_depth),
    );
    let years = collect_years([
        existing
            .get("detectedYears")
            .cloned()
            .unwrap_or_else(|| json!([])),
        incoming
            .get("detectedYears")
            .cloned()
            .unwrap_or_else(|| json!([])),
    ]);
    let latest_year = years.last().copied();
    merged.insert(
        "detectedYears".to_string(),
        Value::Array(years.iter().map(|year| json!(year)).collect()),
    );
    merged.insert(
        "latestDetectedYear".to_string(),
        latest_year.map_or(Value::Null, |year| json!(year)),
    );
    merged.insert(
        "reasons".to_string(),
        first_non_nullish_candidate_value(&[winning.get("reasons")]).unwrap_or_else(|| json!([])),
    );
    for field in ["score", "confidence"] {
        if let Some(value) = winning.get(field) {
            merged.insert(field.to_string(), value.clone());
        } else {
            merged.remove(field);
        }
    }
    Value::Object(merged)
}

fn candidate_link_text(candidate: &Value) -> String {
    let value = ["linkText", "anchorText", "label", "pageTitle"]
        .into_iter()
        .filter_map(|key| candidate.get(key))
        .find(|value| !value.is_null());
    let text = value.map(js_string).unwrap_or_default();
    super::requirement_shape::normalize_js_whitespace(&text)
}

fn candidate_section_anchor(url: &str) -> Option<String> {
    if let Ok(parsed) = url::Url::parse(url) {
        return parsed.fragment().map(|fragment| format!("#{fragment}"));
    }
    static FALLBACK: OnceLock<Regex> = OnceLock::new();
    FALLBACK
        .get_or_init(|| Regex::new(r"#.+$").expect("valid section-anchor fallback regex"))
        .find(url)
        .map(|matched| matched.as_str().to_string())
}

/// Mirrors discovery's exact skip-navigation candidate exclusion set.
pub fn is_skip_navigation_discovery_candidate(candidate: &Value) -> bool {
    const SKIP_LINK_TEXTS: &[&str] = &[
        "close menu",
        "contact us",
        "search",
        "site map",
        "skip to content",
        "skip to main content",
        "user account menu",
    ];
    let link_text = candidate_link_text(candidate).to_lowercase();
    let section_anchor = candidate
        .get("url")
        .filter(|url| !url.is_null())
        .map(js_string)
        .and_then(|url| candidate_section_anchor(&url))
        .unwrap_or_default()
        .to_lowercase();
    SKIP_LINK_TEXTS.contains(&link_text.as_str()) || section_anchor == "#content"
}

/// Mirrors discovery's graduate-only veto for an undergraduate owner.
pub fn is_graduate_only_candidate_for_undergraduate_target(
    target: &Value,
    combined_text: &str,
) -> bool {
    let target_text = texts(
        ["title", "label"]
            .into_iter()
            .filter_map(|key| target.get(key).cloned()),
    )
    .join(" ")
    .to_lowercase();
    if patterns()["graduate-route"].is_match(&target_text) {
        return false;
    }
    let candidate_is_graduate = patterns()["graduate-route"].is_match(combined_text)
        || patterns()["graduate-path"].is_match(combined_text);
    candidate_is_graduate && !patterns()["undergraduate-route"].is_match(combined_text)
}

fn collect_years(values: impl IntoIterator<Item = Value>) -> Vec<i32> {
    let mut years = HashSet::new();
    let matcher = &patterns()["year"];
    for value in values {
        let text = js_string(&value);
        for captures in matcher.captures_iter(&text) {
            if let Some(year) = captures
                .get(1)
                .and_then(|matched| matched.as_str().parse().ok())
            {
                years.insert(year);
            }
        }
    }
    let mut years = years.into_iter().collect::<Vec<_>>();
    years.sort_unstable();
    years
}

/// Rust port of the discovery module's weak-existing-primary trigger conditions used by the
/// validator's fixed year-specific and weak-source fixtures.
pub fn build_weak_existing_primary_signal_codes(input: &Value) -> (bool, Vec<String>) {
    let primary = input.get("primarySource").unwrap_or(&Value::Null);
    let block = input.get("parsedBlock").unwrap_or(&Value::Null);
    let headings = array(block.get("extractedHeadings"));
    let cues = array(block.get("requirementCueLines"));
    let choices = array(block.get("chooseStatements"));
    let mut source_text = texts(
        [
            primary.get("label"),
            primary.get("url"),
            block.get("primarySourceLabel"),
            block.get("sourceLabel"),
        ]
        .into_iter()
        .flatten()
        .cloned(),
    )
    .join(" \n");
    let heading_parts = headings
        .iter()
        .filter(|heading| is_truthy(heading))
        .map(js_string)
        .collect::<Vec<_>>();
    if !heading_parts.is_empty() {
        if !source_text.is_empty() {
            source_text.push_str(" ");
        }
        source_text.push_str(&heading_parts.join(" "));
    }
    let heading_text = headings
        .iter()
        .map(js_string)
        .collect::<Vec<_>>()
        .join(" \n");
    let primary_url = match primary.get("url") {
        Some(url) if !url.is_null() => js_string(url),
        _ => String::new(),
    };
    let has_strong_requirement_cue = patterns()["requirement-cue"].is_match(&source_text);
    let has_dedicated_requirement_cue = patterns()["primary-requirement-cue"]
        .is_match(&source_text)
        || patterns()["dedicated-requirement-cue"].is_match(&source_text);
    let weak_url = patterns()["weak-primary-url"].is_match(&primary_url);
    let weak_headings = patterns()["weak-primary-heading"].is_match(&heading_text);
    let overview_only =
        patterns()["overview-only"].is_match(&source_text) && !has_dedicated_requirement_cue;
    let quality_warnings = array(block.get("qualitySignals"))
        .iter()
        .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("warning"))
        .filter(|signal| signal.get("code").is_some_and(is_truthy))
        .collect::<Vec<_>>();
    let runtime_course_count = input
        .get("runtimeGrcCourseCount")
        .and_then(Value::as_f64)
        .unwrap_or_default();
    let no_tracks = !input.get("bestTrackId").is_some_and(is_truthy)
        && !input.get("trackRecommendationId").is_some_and(is_truthy);
    let safe_intentional_empty =
        block.get("ok").is_some_and(is_truthy) && runtime_course_count == 0.0 && no_tracks;
    let course_code_count = array(block.get("parsedUwCourseCodes")).len();
    let identity_years = collect_years(
        [
            primary.get("url"),
            primary.get("label"),
            block.get("primarySourceLabel"),
            block.get("sourceLabel"),
            block.get("extractedTitle"),
        ]
        .into_iter()
        .flatten()
        .cloned(),
    );
    let mut document_year_values = vec![json!(identity_years)];
    document_year_values.extend(headings.iter().cloned());
    document_year_values.extend(cues.iter().take(16).cloned());
    document_year_values.extend(choices.iter().take(8).cloned());
    let document_years = collect_years(document_year_values);
    let current_years = if patterns()["linked-document"].is_match(&primary_url) {
        document_years
    } else {
        identity_years
    };
    let year_specific = current_years.last().is_some()
        && (patterns()["linked-document"].is_match(&primary_url)
            || patterns()["year-specific-requirement-source"].is_match(&source_text));
    let identity_text = super::requirement_shape::normalize_js_whitespace(
        &texts(
            [
                primary.get("label"),
                block.get("primarySourceLabel"),
                block.get("sourceLabel"),
                block.get("extractedTitle"),
            ]
            .into_iter()
            .flatten()
            .cloned(),
        )
        .join(" "),
    );
    let major_pathway_scoped = input.get("ownerType").and_then(Value::as_str) == Some("major")
        && patterns()["singular-route"].is_match(&identity_text)
        && !patterns()["plural-route"].is_match(&identity_text);
    let shared_tacoma_catalog = input.get("campusId").and_then(Value::as_str) == Some("uw-tacoma")
        && patterns()["shared-tacoma-catalog"].is_match(&primary_url)
        && runtime_course_count == 0.0;

    let mut codes = Vec::new();
    if safe_intentional_empty {
        codes.push("safe-intentional-empty-state".to_string());
    }
    if !quality_warnings.is_empty() {
        codes.push("parser-quality-warning".to_string());
    }
    if course_code_count == 0 {
        codes.push("no-parsed-uw-course-codes".to_string());
    }
    if weak_url {
        codes.push("primary-url-looks-graduate-or-timeline".to_string());
    }
    if weak_headings && !has_strong_requirement_cue {
        codes.push("page-headings-look-graduate-or-timeline-heavy".to_string());
    }
    if overview_only {
        codes.push("primary-looks-overview-only".to_string());
    }
    if year_specific {
        codes.push("primary-source-appears-year-specific".to_string());
    }
    if major_pathway_scoped {
        codes.push("major-primary-looks-pathway-scoped".to_string());
    }
    if shared_tacoma_catalog {
        codes.push("shared-uw-tacoma-catalog-primary-with-empty-grc-runtime".to_string());
    }

    let downstream_weak =
        safe_intentional_empty || !quality_warnings.is_empty() || course_code_count == 0;
    let source_weak = weak_url || (weak_headings && !has_strong_requirement_cue) || overview_only;
    let triggered = (downstream_weak && source_weak)
        || (course_code_count == 0 && has_strong_requirement_cue)
        || year_specific
        || major_pathway_scoped
        || shared_tacoma_catalog;
    (triggered, codes)
}

pub fn build_source_pipeline_trigger_fixture_checks() -> Vec<SourcePipelineValidationCheck> {
    let stale = json!({
        "primarySource": {
            "label": "BA Biochemistry Checklist (PDF)",
            "url": "https://chem.washington.edu/sites/chem/files/documents/undergrad/babioccheck2017_001.pdf"
        },
        "parsedBlock": {
            "ok": true,
            "extractedTitle": "Bachelor of Arts in Biochemistry",
            "extractedHeadings": ["Degree Requirements", "Biochemistry"],
            "parsedUwCourseCodes": ["CHEM 237", "CHEM 242", "MATH 124", "PHYS 114"],
            "qualitySignals": [],
            "requirementCueLines": ["Bachelor of Arts in Biochemistry"],
            "chooseStatements": []
        },
        "runtimeGrcCourseCount": 10,
        "bestTrackId": "grc-associate-stem-chemistry-associate-in-science-transfer-track-1-chemistry",
        "trackRecommendationId": "grc-associate-stem-chemistry-associate-in-science-transfer-track-1-chemistry",
        "noPublicClassificationCount": 0
    });
    let (stale_triggered, stale_codes) = build_weak_existing_primary_signal_codes(&stale);
    let weak = json!({
        "primarySource": {
            "label": "Timeline and Requirements",
            "url": "https://astro.washington.edu/timeline-and-requirements"
        },
        "parsedBlock": {
            "ok": true,
            "extractedHeadings": ["Timeline & Requirements", "Overview", "Graduate Program"],
            "parsedUwCourseCodes": ["PHYS 121", "PHYS 122", "MATH 124"],
            "qualitySignals": []
        },
        "runtimeGrcCourseCount": 0,
        "bestTrackId": null,
        "trackRecommendationId": null,
        "noPublicClassificationCount": 0
    });
    let (weak_triggered, weak_codes) = build_weak_existing_primary_signal_codes(&weak);
    let stale_passed = stale_triggered
        && stale_codes
            .iter()
            .any(|code| code == "primary-source-appears-year-specific");
    let weak_passed = weak_triggered
        && weak_codes
            .iter()
            .any(|code| code == "safe-intentional-empty-state")
        && weak_codes
            .iter()
            .any(|code| code == "primary-url-looks-graduate-or-timeline");
    vec![
        SourcePipelineValidationCheck {
            id: "stale-year-replacement-trigger".to_string(),
            label: "Year-tied primary degree sheets can trigger re-evaluation even when parsing still succeeds".to_string(),
            status: if stale_passed { "passed" } else { "failed" }.to_string(),
            details: vec![stale_codes.join(", ")],
        },
        SourcePipelineValidationCheck {
            id: "weak-existing-primary-replacement-trigger".to_string(),
            label: "Weak-source replacement can trigger for an owner that already has an official primary source".to_string(),
            status: if weak_passed { "passed" } else { "failed" }.to_string(),
            details: vec![weak_codes.join(", ")],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        build_multi_pathway_single_route_review_decision,
        build_source_pipeline_trigger_fixture_checks, compare_scored_discovery_candidates,
        has_conflicting_bachelor_program_route, has_conflicting_degree_route_tokens,
        is_graduate_only_candidate_for_undergraduate_target,
        is_primary_discovery_candidate_eligible_for_target,
        is_primary_discovery_candidate_shape_eligible,
        is_single_degree_route_candidate_for_multi_pathway_major,
        is_skip_navigation_discovery_candidate, merge_scored_discovery_candidates,
    };

    #[test]
    fn replacement_trigger_fixture_checks_match_cjs_codes_and_order() {
        let checks = build_source_pipeline_trigger_fixture_checks();
        assert_eq!(
            checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            [
                "stale-year-replacement-trigger",
                "weak-existing-primary-replacement-trigger",
            ]
        );
        assert!(checks.iter().all(|check| check.status == "passed"));
        assert_eq!(checks[0].details, ["primary-source-appears-year-specific"]);
        assert_eq!(
            checks[1].details,
            [
                "safe-intentional-empty-state, primary-url-looks-graduate-or-timeline, page-headings-look-graduate-or-timeline-heavy, primary-looks-overview-only"
            ]
        );
    }

    #[test]
    fn pathway_signal_identity_uses_ecmascript_whitespace_normalization() {
        let input = serde_json::json!({
            "primarySource": {
                "label": "Option\u{FEFF}A",
                "url": "https://chem.washington.edu/requirements"
            },
            "parsedBlock": {
                "ok": true,
                "parsedUwCourseCodes": ["CHEM 142"],
                "qualitySignals": []
            },
            "runtimeGrcCourseCount": 1,
            "bestTrackId": "track",
            "trackRecommendationId": "track",
            "ownerType": "major"
        });
        let (triggered, codes) = super::build_weak_existing_primary_signal_codes(&input);
        assert!(triggered);
        assert!(
            codes
                .iter()
                .any(|code| code == "major-primary-looks-pathway-scoped")
        );
    }

    #[test]
    fn strong_existing_primary_has_no_replacement_trigger_signals() {
        let strong = serde_json::json!({
            "primarySource": {
                "label": "Degree requirements",
                "url": "https://astro.washington.edu/undergraduate-program"
            },
            "parsedBlock": {
                "ok": true,
                "extractedHeadings": [
                    "Undergraduate Program",
                    "Major Admissions Requirements",
                    "Degree Requirements"
                ],
                "parsedUwCourseCodes": ["PHYS 121", "PHYS 122", "MATH 124", "ASTR 300"],
                "qualitySignals": []
            },
            "runtimeGrcCourseCount": 8,
            "bestTrackId": "grc-associate-stem-physics-associate-in-science-transfer-track-2-physics",
            "trackRecommendationId": "grc-associate-stem-physics-associate-in-science-transfer-track-2-physics",
            "noPublicClassificationCount": 0
        });
        let (triggered, codes) = super::build_weak_existing_primary_signal_codes(&strong);
        assert!(!triggered);
        assert!(codes.is_empty());
    }

    #[test]
    fn bachelor_program_route_conflict_matches_major_identity_and_url_routes() {
        let chemistry = serde_json::json!({"title":"Chemistry"});
        let other_major = serde_json::json!({
            "label":"Bachelor of Science in Biology Degree Requirements"
        });
        let same_major = serde_json::json!({
            "label":"Bachelor of Science in Chemistry Degree Requirements"
        });
        assert!(has_conflicting_bachelor_program_route(
            &chemistry,
            &other_major
        ));
        assert!(!has_conflicting_bachelor_program_route(
            &chemistry,
            &same_major
        ));
        let route_in_path = serde_json::json!({
            "url":"https://example.test/bachelor-of-arts-in-biology"
        });
        assert!(has_conflicting_bachelor_program_route(
            &chemistry,
            &route_in_path
        ));
        let target_arts = serde_json::json!({"title":"American Indian Studies"});
        let named_route = serde_json::json!({
            "pageTitle":"Bachelor of Arts in American Indian Studies Degree Requirements"
        });
        assert!(!has_conflicting_bachelor_program_route(
            &target_arts,
            &named_route
        ));
    }

    #[test]
    fn candidate_degree_route_conflicts_match_target_tokens_and_page_headings() {
        let ba_target = serde_json::json!({"title":"BA Biochemistry"});
        let bs_candidate = serde_json::json!({
            "url":"https://chem.washington.edu/bs-biochemistry",
            "pageHeadings":["Bachelor of Science in Biochemistry"]
        });
        assert!(has_conflicting_degree_route_tokens(
            &ba_target,
            &bs_candidate
        ));
        let ba_candidate = serde_json::json!({
            "url":"https://chem.washington.edu/ba-biochemistry",
            "pageHeadings":["Bachelor of Arts in Biochemistry"]
        });
        assert!(!has_conflicting_degree_route_tokens(
            &ba_target,
            &ba_candidate
        ));
        assert!(!has_conflicting_degree_route_tokens(
            &serde_json::json!({"title":"Biochemistry"}),
            &bs_candidate
        ));
        let catalog_anchor = serde_json::json!({
            "url":"https://catalog.washington.edu/college/arts-sciences/chemistry/#credential-ba",
            "sectionAnchor":"#credential-ba",
            "label":"Chemistry program",
            "pageHeadings":["Bachelor of Science"]
        });
        assert!(!has_conflicting_degree_route_tokens(
            &ba_target,
            &catalog_anchor
        ));
        let general_catalog_anchor = serde_json::json!({
            "url":"https://www.washington.edu/students/gencat/program/C/Chemistry-1010.html#program-UG-CHEM-MAJOR",
            "sectionAnchor":"#program-UG-CHEM-MAJOR",
            "label":"Chemistry program",
            "pageHeadings":["Bachelor of Science"]
        });
        assert!(!has_conflicting_degree_route_tokens(
            &ba_target,
            &general_catalog_anchor
        ));
    }

    #[test]
    fn primary_candidate_target_eligibility_applies_degree_and_major_route_vetoes() {
        let chemistry = serde_json::json!({
            "ownerType":"major",
            "title":"Chemistry"
        });
        let primary = |url: &str, label: &str| {
            serde_json::json!({
                "url":url,
                "label":label,
                "sourceRoleStatus":"primary",
                "canCreateSchedulableRows":true,
                "canBePrimary":true
            })
        };
        assert!(!is_primary_discovery_candidate_eligible_for_target(
            &primary(
                "https://chem.washington.edu/biology",
                "Bachelor of Science in Biology Degree Requirements"
            ),
            &chemistry
        ));
        assert!(is_primary_discovery_candidate_eligible_for_target(
            &primary(
                "https://chem.washington.edu/chemistry",
                "Bachelor of Science in Chemistry Degree Requirements"
            ),
            &chemistry
        ));
        let ba_target = serde_json::json!({"ownerType":"pathway", "title":"BA Biochemistry"});
        assert!(!is_primary_discovery_candidate_eligible_for_target(
            &primary(
                "https://chem.washington.edu/bs-biochemistry",
                "Bachelor of Science in Biochemistry"
            ),
            &ba_target
        ));
    }

    #[test]
    fn primary_discovery_shape_eligibility_matches_verification_and_identity_gates() {
        let target = serde_json::json!({"ownerType":"major"});
        let primary = serde_json::json!({
            "sourceRoleStatus":"primary",
            "canCreateSchedulableRows":true,
            "canBePrimary":true
        });
        assert!(is_primary_discovery_candidate_shape_eligible(
            &primary, &target
        ));
        assert!(!is_primary_discovery_candidate_shape_eligible(
            &serde_json::json!({"sourceRoleStatus":"primary", "requiresVerification":true}),
            &target
        ));
        assert!(is_primary_discovery_candidate_shape_eligible(
            &serde_json::json!({"sourceRoleStatus":"primary", "requiresVerification":true, "verified":true}),
            &target
        ));
        assert!(!is_primary_discovery_candidate_shape_eligible(
            &serde_json::json!({"sourceRoleStatus":"primary", "canBePrimary":false}),
            &target
        ));
        assert!(!is_primary_discovery_candidate_shape_eligible(
            &serde_json::json!({"sourceRoleStatus":"primary", "label":"Search"}),
            &target
        ));
        let campus_index = serde_json::json!({
            "sourceRoleStatus":"primary",
            "sourceKinds":["campus-major-index"],
            "sameMajorIdentityScore":0
        });
        assert!(!is_primary_discovery_candidate_shape_eligible(
            &campus_index,
            &target
        ));
        let matching_index = serde_json::json!({
            "sourceRoleStatus":"primary",
            "sourceKinds":["campus-major-index"],
            "sameMajorIdentityScore":1
        });
        assert!(is_primary_discovery_candidate_shape_eligible(
            &matching_index,
            &target
        ));
        assert!(!is_primary_discovery_candidate_shape_eligible(
            &matching_index,
            &serde_json::json!({"ownerType":"pathway"})
        ));
        let matching_pathway_index = serde_json::json!({
            "sourceRoleStatus":"primary",
            "sourceKinds":["campus-major-index"],
            "sameMajorIdentityScore":1,
            "pathwayIdentityScore":1
        });
        assert!(is_primary_discovery_candidate_shape_eligible(
            &matching_pathway_index,
            &serde_json::json!({"ownerType":"pathway"})
        ));
    }

    #[test]
    fn skip_navigation_discovery_filter_matches_link_text_fallback_and_fragment_rules() {
        for label in [
            "Close menu",
            "Contact us",
            "Search",
            "Site map",
            "Skip to content",
            "Skip to main content",
            "User account menu",
        ] {
            assert!(is_skip_navigation_discovery_candidate(
                &serde_json::json!({"label": label})
            ));
        }
        assert!(!is_skip_navigation_discovery_candidate(
            &serde_json::json!({"label": "Skip to footer"})
        ));
        assert!(!is_skip_navigation_discovery_candidate(
            &serde_json::json!({"linkText":"","anchorText":"Skip to content"})
        ));
        assert!(is_skip_navigation_discovery_candidate(
            &serde_json::json!({"url":"https://example.test/page#content"})
        ));
        assert!(!is_skip_navigation_discovery_candidate(
            &serde_json::json!({"url":"https://example.test/page#section"})
        ));
    }

    #[test]
    fn candidate_merge_preserves_cjs_verification_winner_and_metadata_unions() {
        let existing = serde_json::json!({
            "url": "https://example.test/requirement",
            "score": 80,
            "requiresVerification": true,
            "verified": false,
            "label": "Existing degree page",
            "sourceKinds": ["official-link"],
            "pageHeadings": ["Zeta", "Alpha"],
            "contentSnippets": ["existing excerpt"],
            "detectedYears": [2017],
            "sameMajorIdentityScore": 1,
            "pathwayIdentityScore": 2,
            "discoveryDepth": 2,
            "canCreateSchedulableRows": true,
            "canBePrimary": true,
            "sourceRole": "primary-degree-requirements",
            "sourceRoleStatus": "primary",
            "parserType": "html-degree-page",
            "confidence": "medium",
            "reasons": ["existing reason"]
        });
        let incoming = serde_json::json!({
            "url": "https://example.test/requirement",
            "score": 10,
            "verified": true,
            "label": "",
            "anchorText": "New checklist anchor",
            "sourceKinds": ["discovered-anchor"],
            "pageHeadings": ["Alpha", "Beta"],
            "contentSnippets": ["incoming excerpt"],
            "detectedYears": [2018],
            "sameMajorIdentityScore": 3,
            "pathwayIdentityScore": 1,
            "discoveryDepth": 4,
            "confidence": "high",
            "reasons": ["incoming reason"]
        });
        let merged = merge_scored_discovery_candidates(&existing, &incoming);
        assert_eq!(merged["score"], 10);
        assert_eq!(merged["label"], "Existing degree page");
        assert_eq!(merged["anchorText"], "New checklist anchor");
        assert_eq!(
            merged["sourceKinds"],
            serde_json::json!(["discovered-anchor", "official-link"])
        );
        assert_eq!(
            merged["pageHeadings"],
            serde_json::json!(["Alpha", "Beta", "Zeta"])
        );
        assert_eq!(
            merged["contentSnippets"],
            serde_json::json!(["existing excerpt", "incoming excerpt"])
        );
        assert_eq!(merged["detectedYears"], serde_json::json!([2017, 2018]));
        assert_eq!(merged["latestDetectedYear"], 2018);
        assert_eq!(merged["sameMajorIdentityScore"], 3.0);
        assert_eq!(merged["pathwayIdentityScore"], 2.0);
        assert_eq!(merged["discoveryDepth"], 2.0);
        assert_eq!(merged["requiresVerification"], true);
        assert_eq!(merged["verified"], true);
        assert_eq!(merged["reasons"], serde_json::json!(["incoming reason"]));

        let verified_existing = serde_json::json!({"score": 40, "verified": true});
        let inferred_incoming = serde_json::json!({"score": 80, "requiresVerification": true});
        let merge = merge_scored_discovery_candidates(&verified_existing, &inferred_incoming);
        assert_eq!(merge["score"], 40);
        assert_eq!(merge["verified"], true);

        let lower_score = serde_json::json!({"score": 20, "label": "old"});
        let higher_score = serde_json::json!({"score": 30, "label": "new"});
        let score_winner = merge_scored_discovery_candidates(&lower_score, &higher_score);
        assert_eq!(score_winner["score"], 30);
        assert_eq!(score_winner["label"], "new");
    }

    #[test]
    fn scored_candidate_comparator_matches_cjs_provenance_year_and_url_ties() {
        let mut candidates = vec![
            serde_json::json!({"url":"https://example.test/z","score":64,"sourceKinds":["discovered-anchor"],"latestDetectedYear":2020}),
            serde_json::json!({"url":"https://example.test/b","score":64,"sourceKinds":["official-link"],"latestDetectedYear":2017}),
            serde_json::json!({"url":"https://example.test/a","score":64,"sourceKinds":["official-link"],"latestDetectedYear":2020}),
            serde_json::json!({"url":"https://example.test/c","score":70,"sourceKinds":[],"latestDetectedYear":null}),
        ];
        candidates.sort_by(compare_scored_discovery_candidates);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate["url"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "https://example.test/c",
                "https://example.test/a",
                "https://example.test/b",
                "https://example.test/z",
            ]
        );
    }

    #[test]
    fn multi_pathway_single_route_candidate_stays_review_only() {
        let target = serde_json::json!({
            "analysisMode": "weak-existing-primary",
            "ownerType": "major",
            "ownerKey": "uw-seattle-chemistry",
            "planId": "uw-seattle-chemistry",
            "pathwayId": null,
            "campusId": "uw-seattle",
            "title": "Chemistry",
            "label": "Chemistry",
            "existingPrimaryUrl": "https://chem.washington.edu/sites/chem/files/documents/undergrad/acs2018.pdf",
            "pathwayCount": 3,
            "reevaluationSignals": [{"code":"primary-source-appears-year-specific"}],
            "reevaluationContext": {"currentSourceLatestYear": 2018}
        });
        let candidates = vec![
            serde_json::json!({
                "url":"https://chem.washington.edu/ba-chemistry",
                "label":"UW BA in Chemistry requirements",
                "pageTitle":"BA Chemistry Degree Requirements",
                "pageHeadings":["Bachelor of Arts in Chemistry", "Degree Requirements"],
                "sourceRole":"primary-degree-requirements",
                "sourceRoleStatus":"primary",
                "supportOnly":false,
                "canBePrimary":true,
                "canCreateSchedulableRows":true,
                "score":64,
                "confidence":"high",
                "latestDetectedYear":null,
                "reasons":[
                    "explicit degree-requirements wording",
                    "explicitly names the selected major",
                    "matches major keyword \"chemistry\"",
                    "official source path matches the selected major",
                    "stays on the current official department host",
                    "stays on an official UW domain",
                    "specific bachelor route wording"
                ]
            }),
            serde_json::json!({
                "url":"https://chem.washington.edu/sites/chem/files/documents/undergrad/acs2018.pdf",
                "label":"BS Chemistry Checklist - ACS Certified (PDF)",
                "sourceRole":"primary-degree-requirements",
                "sourceRoleStatus":"primary",
                "supportOnly":false,
                "canBePrimary":true,
                "canCreateSchedulableRows":true,
                "score":28,
                "confidence":"high",
                "latestDetectedYear":2018,
                "reasons":[
                    "already stored as an official source",
                    "checklist-style wording",
                    "explicitly names the selected major",
                    "route-specific page may not cover every pathway in the selected major",
                    "stays on the current official department host"
                ]
            }),
        ];
        let decision = build_multi_pathway_single_route_review_decision(&target, &candidates)
            .expect("the single BA route should be retained for human review");
        assert_eq!(decision["action"], "keep-existing-primary");
        assert_eq!(
            decision["reviewCandidate"]["url"],
            "https://chem.washington.edu/ba-chemistry"
        );
        assert_eq!(
            decision["currentPrimary"]["url"],
            target["existingPrimaryUrl"]
        );
        assert_eq!(decision["scoreDelta"], 36.0);
    }

    #[test]
    fn biochemistry_mock_fixture_candidate_order_matches_cjs_scores() {
        let mut candidates = vec![
            serde_json::json!({"url":"https://chem.washington.edu/sites/chem/files/documents/undergrad/babioccheck2017_001.pdf","score":47,"sourceKinds":["discovered-anchor","official-link"],"latestDetectedYear":2017}),
            serde_json::json!({"url":"https://chem.washington.edu/sites/chem/files/documents/undergrad/biochem2018.pdf","score":83,"sourceKinds":["discovered-anchor"],"latestDetectedYear":2018}),
            serde_json::json!({"url":"https://chem.washington.edu/ba-biochemistry","score":66,"sourceKinds":["official-link"],"latestDetectedYear":null}),
            serde_json::json!({"url":"https://chem.washington.edu/bs-biochemistry","score":94,"sourceKinds":["discovered-anchor"],"latestDetectedYear":null}),
        ];
        candidates.sort_by(compare_scored_discovery_candidates);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate["url"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "https://chem.washington.edu/bs-biochemistry",
                "https://chem.washington.edu/sites/chem/files/documents/undergrad/biochem2018.pdf",
                "https://chem.washington.edu/ba-biochemistry",
                "https://chem.washington.edu/sites/chem/files/documents/undergrad/babioccheck2017_001.pdf",
            ]
        );
    }

    #[test]
    fn graduate_only_candidate_veto_preserves_undergraduate_exception_and_target_scope() {
        let undergraduate_target = serde_json::json!({
            "title": "Astronomy",
            "label": "Astronomy"
        });
        assert!(is_graduate_only_candidate_for_undergraduate_target(
            &undergraduate_target,
            "https://astro.washington.edu/graduate-program Graduate Program PhD Requirements"
        ));
        assert!(!is_graduate_only_candidate_for_undergraduate_target(
            &undergraduate_target,
            "https://astro.washington.edu/graduate-program Graduate Program Undergraduate Program"
        ));
        let graduate_target = serde_json::json!({
            "title": "Graduate Astronomy",
            "label": "Graduate Astronomy"
        });
        assert!(!is_graduate_only_candidate_for_undergraduate_target(
            &graduate_target,
            "https://astro.washington.edu/graduate-program Graduate Program"
        ));
    }

    #[test]
    fn multi_pathway_major_guard_detects_single_bachelor_route_candidates() {
        let target = serde_json::json!({
            "ownerType": "major",
            "pathwayCount": 3
        });
        let ba_candidate = serde_json::json!({
            "url": "https://chem.washington.edu/ba-chemistry",
            "label": "UW BA in Chemistry requirements",
            "pageTitle": "BA Chemistry Degree Requirements"
        });
        assert!(is_single_degree_route_candidate_for_multi_pathway_major(
            &target,
            &ba_candidate,
            "https://chem.washington.edu/ba-chemistry UW BA in Chemistry requirements BA Chemistry Degree Requirements"
        ));

        let two_route_candidate = serde_json::json!({
            "label": "BA and BS Chemistry Requirements",
            "pageTitle": "BA / BS Degree Requirements"
        });
        assert!(!is_single_degree_route_candidate_for_multi_pathway_major(
            &target,
            &two_route_candidate,
            "BA and BS Chemistry Requirements BA / BS Degree Requirements"
        ));
        let single_pathway_target = serde_json::json!({
            "ownerType": "major",
            "pathwayCount": 0
        });
        assert!(!is_single_degree_route_candidate_for_multi_pathway_major(
            &single_pathway_target,
            &ba_candidate,
            "BA Chemistry"
        ));
    }
}
