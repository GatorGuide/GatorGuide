use super::requirement_course_codes::CourseCodeParser;
use super::requirement_manifest::normalize_manifest_url_without_hash_for_dedupe;
use super::requirement_role_classifier::{
    classify_requirement_source_role, has_conflicting_supplemental_degree_route,
};
use super::requirement_scope::PARSEABLE_SUPPORT_ROLES;
use super::transfer_plan_title_matching::{
    get_specialized_plan_title_tokens, label_mentions_different_transfer_planner_major,
    normalize_specialized_plan_tokens, normalize_transfer_planner_text,
    specialized_plan_tokens_match_candidate,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub fn build_primary_major_titles_by_plan_id(
    manifest_entries: &[Value],
) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    for entry in manifest_entries {
        if entry.get("ownerType").and_then(Value::as_str) != Some("major")
            || !js_truthy(entry.get("planId"))
            || !js_truthy(entry.get("ownerTitle"))
        {
            continue;
        }
        let plan_id = js_string(entry.get("planId").expect("truthy planId"));
        let owner_title = js_string(entry.get("ownerTitle").expect("truthy ownerTitle"));
        titles.insert(plan_id, normalize_transfer_planner_text(&owner_title));
    }
    titles
}

pub fn filter_manifest_entries_for_specialized_plan_title(
    entries: &[Value],
    bootstrap_plan_titles: &HashMap<String, String>,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    entries
        .iter()
        .filter(|entry| {
            manifest_entry_matches_specialized_plan_title(
                entry,
                bootstrap_plan_titles,
                primary_major_titles,
                entries,
                course_parser,
            )
        })
        .cloned()
        .collect()
}

pub fn manifest_entry_matches_specialized_plan_title(
    entry: &Value,
    bootstrap_plan_titles: &HashMap<String, String>,
    primary_major_titles: &HashMap<String, String>,
    manifest_entries: &[Value],
    course_parser: &CourseCodeParser,
) -> bool {
    if manifest_entry_mentions_different_major(entry, primary_major_titles, course_parser) {
        return false;
    }
    let label = string_field(entry, "label");
    let url = string_field(entry, "url");
    if has_conflicting_supplemental_degree_route(entry, &label, &url) {
        return false;
    }

    let plan_id = string_field(entry, "planId");
    let plan_title = bootstrap_plan_titles
        .get(&plan_id)
        .map(String::as_str)
        .unwrap_or_default();
    let plan_tokens = get_specialized_plan_title_tokens(plan_title);
    let pathway_id = string_field(entry, "pathwayId");
    if pathway_id.is_empty() {
        let plan_degree_kind = get_ba_bs_degree_kind(plan_title);
        let candidate_text = format!("{label} {url}");
        let candidate_degree_kind = get_source_degree_kind(&candidate_text);
        if plan_degree_kind.is_some()
            && candidate_degree_kind.is_some()
            && plan_degree_kind != candidate_degree_kind
            && !manifest_url_is_shared_across_ba_bs_variants(
                entry,
                bootstrap_plan_titles,
                manifest_entries,
            )
        {
            return false;
        }
        if plan_tokens.is_empty() {
            return true;
        }
        let candidate_tokens = normalize_specialized_plan_tokens(&candidate_text, 4)
            .into_iter()
            .collect::<HashSet<_>>();
        let acronym_tokens = normalize_specialized_plan_tokens(&candidate_text, 2)
            .into_iter()
            .collect::<HashSet<_>>();
        return specialized_plan_tokens_match_candidate(
            &plan_tokens,
            &candidate_tokens,
            &acronym_tokens,
        );
    }

    let plan_degree_kind = get_ba_bs_degree_kind(plan_title);
    let pathway_text = format!(
        "{} {} {}",
        pathway_id,
        string_field(entry, "ownerTitle"),
        string_field(entry, "ownerId")
    );
    let source_degree_text = format!("{label} {url}");
    let source_degree_kind = get_ba_bs_degree_kind(&source_degree_text);
    let pathway_degree_kind = get_ba_bs_degree_kind(&pathway_text);
    if pathway_degree_kind.is_some()
        && source_degree_kind.is_some()
        && pathway_degree_kind != source_degree_kind
        && !manifest_url_is_shared_across_ba_bs_variants(
            entry,
            bootstrap_plan_titles,
            manifest_entries,
        )
    {
        return false;
    }
    if plan_degree_kind.is_some()
        && pathway_degree_kind.is_some()
        && plan_degree_kind != pathway_degree_kind
    {
        return false;
    }
    if plan_tokens.is_empty() {
        return true;
    }
    let candidate_text = format!("{pathway_id} {label} {url}");
    let candidate_tokens = normalize_specialized_plan_tokens(&candidate_text, 4)
        .into_iter()
        .collect::<HashSet<_>>();
    let acronym_tokens = normalize_specialized_plan_tokens(&candidate_text, 2)
        .into_iter()
        .collect::<HashSet<_>>();
    specialized_plan_tokens_match_candidate(&plan_tokens, &candidate_tokens, &acronym_tokens)
}

pub fn parser_recovery_candidate_conflicts_with_owner_identity(
    entry: &Value,
    label: &str,
    url: &str,
    bootstrap_plan_titles: &HashMap<String, String>,
    primary_major_titles: &HashMap<String, String>,
    manifest_entries: &[Value],
    course_parser: &CourseCodeParser,
) -> bool {
    if pathway_label_mentions_different_major(
        entry,
        label,
        primary_major_titles,
        course_parser,
    ) {
        return true;
    }

    let pathway_id = string_field(entry, "pathwayId");
    let source_degree_kind = get_ba_bs_degree_kind(&format!(
        "{pathway_id} {} {} {}",
        string_field(entry, "sourceLabel"),
        string_field(entry, "label"),
        string_field(entry, "url")
    ));
    let owner_degree_kind = if pathway_id.is_empty() {
        source_degree_kind.or_else(|| {
            let plan_id = string_field(entry, "planId");
            let title = bootstrap_plan_titles
                .get(&plan_id)
                .cloned()
                .unwrap_or_else(|| string_field(entry, "ownerTitle"));
            get_ba_bs_degree_kind(&title)
        })
    } else {
        source_degree_kind.or_else(|| {
            get_ba_bs_degree_kind(&format!(
                "{pathway_id} {} {}",
                string_field(entry, "ownerTitle"),
                string_field(entry, "ownerId")
            ))
        })
    };
    let candidate_degree_kind = get_ba_bs_degree_kind(&format!("{label} {url}"));
    if owner_degree_kind.is_none() || owner_degree_kind == candidate_degree_kind {
        return false;
    }
    if candidate_degree_kind.is_none() {
        return false;
    }

    let mut candidate_entry = entry.clone();
    if let Some(object) = candidate_entry.as_object_mut() {
        object.insert("label".to_string(), Value::String(label.to_string()));
        object.insert("url".to_string(), Value::String(url.to_string()));
    }
    !manifest_url_is_shared_across_ba_bs_variants(
        &candidate_entry,
        bootstrap_plan_titles,
        manifest_entries,
    )
}

pub fn manifest_entry_mentions_different_major(
    entry: &Value,
    primary_plan_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    let source_role = classify_requirement_source_role(entry);
    if !js_truthy(entry.get("isPrimaryDegreeRequirementsLink"))
        && PARSEABLE_SUPPORT_ROLES.contains(&source_role)
    {
        return false;
    }

    let mut candidates = Vec::new();
    for key in ["label", "sourceLabel", "ownerTitle"] {
        let value = string_field(entry, key);
        if !value.is_empty() && !candidates.contains(&value) {
            candidates.push(value);
        }
    }
    candidates.into_iter().any(|candidate| {
        let normalized_candidate = normalize_transfer_planner_text(&candidate).to_lowercase();
        let catalog_titles = get_catalog_owner_scope_titles(entry, primary_plan_titles);
        let matches_catalog_alias = catalog_titles.iter().any(|title| {
            let normalized_title = normalize_transfer_planner_text(title).to_lowercase();
            !normalized_title.is_empty() && normalized_candidate.contains(&normalized_title)
        });
        let matches_own_pathway = manifest_entry_candidate_matches_own_pathway_identity(entry, &candidate);
        !matches_catalog_alias
            && !matches_own_pathway
            && pathway_label_mentions_different_major(
                entry,
                &candidate,
                primary_plan_titles,
                course_parser,
            )
    })
}

fn pathway_label_mentions_different_major(
    entry: &Value,
    label: &str,
    primary_plan_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    if looks_like_standalone_pathway_label(label, course_parser) {
        return false;
    }
    let plan_id = string_field(entry, "planId");
    let mut titles = primary_plan_titles.clone();
    if !titles.contains_key(&plan_id) {
        let fallback = string_field(entry, "ownerTitle");
        let fallback = if fallback.is_empty() {
            string_field(entry, "planTitle")
        } else {
            fallback
        };
        titles.insert(plan_id.clone(), normalize_transfer_planner_text(&fallback));
    }
    label_mentions_different_transfer_planner_major(&plan_id, label, &titles)
}

fn looks_like_standalone_pathway_label(value: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized = normalize_transfer_planner_text(value);
    if normalized.is_empty()
        || !pathway_label_cue_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
        || normalized.encode_utf16().count() > 90
        || normalized.split_whitespace().count() > 8
    {
        return false;
    }
    !pathway_label_exclusion_pattern().is_match(&normalized)
}

fn get_catalog_owner_scope_titles(
    entry: &Value,
    primary_plan_titles: &HashMap<String, String>,
) -> Vec<String> {
    let plan_id = string_field(entry, "planId");
    let owner_title = string_field(entry, "ownerTitle");
    let normalized_owner_title = normalize_transfer_planner_text(&owner_title);
    let fallback_title = owner_title_prefix_pattern()
        .split(&normalized_owner_title)
        .next()
        .unwrap_or(&normalized_owner_title)
        .to_string();
    let primary_title = normalize_transfer_planner_text(
        primary_plan_titles
            .get(&plan_id)
            .unwrap_or(&fallback_title),
    );
    let mut titles = Vec::new();
    for title in [primary_title, normalize_transfer_planner_text(&owner_title)] {
        if !title.is_empty() && !titles.contains(&title) {
            titles.push(title);
        }
    }
    if plan_id == "uw-seattle-european-studies" {
        let alias = "International Studies: Europe".to_string();
        if !titles.contains(&alias) {
            titles.push(alias);
        }
    }
    titles
}

fn manifest_entry_candidate_matches_own_pathway_identity(entry: &Value, candidate: &str) -> bool {
    let pathway_id = string_field(entry, "pathwayId");
    if pathway_id.is_empty() || candidate.is_empty() {
        return false;
    }
    let pathway_tokens = normalize_specialized_plan_tokens(&pathway_id, 4);
    if pathway_tokens.is_empty() {
        return false;
    }
    let candidate_tokens = normalize_specialized_plan_tokens(candidate, 2)
        .into_iter()
        .collect::<HashSet<_>>();
    pathway_tokens
        .iter()
        .all(|token| candidate_tokens.contains(token))
}

fn manifest_url_is_shared_across_ba_bs_variants(
    entry: &Value,
    bootstrap_plan_titles: &HashMap<String, String>,
    manifest_entries: &[Value],
) -> bool {
    let plan_title = bootstrap_plan_titles
        .get(&string_field(entry, "planId"))
        .map(String::as_str)
        .unwrap_or_default();
    let Some(plan_degree_kind) = get_ba_bs_degree_kind(plan_title) else {
        return false;
    };
    let plan_title_base = get_degree_variant_title_base(plan_title);
    if plan_title_base.is_empty() {
        return false;
    }
    let normalized_entry_url =
        normalize_manifest_url_without_hash_for_dedupe(&string_field(entry, "url"));
    manifest_entries.iter().any(|candidate| {
        if std::ptr::eq(candidate, entry)
            || string_field(candidate, "planId") == string_field(entry, "planId")
            || normalize_manifest_url_without_hash_for_dedupe(&string_field(candidate, "url"))
                != normalized_entry_url
        {
            return false;
        }
        let candidate_title = bootstrap_plan_titles
            .get(&string_field(candidate, "planId"))
            .map(String::as_str)
            .unwrap_or_default();
        get_ba_bs_degree_kind(candidate_title) != Some(plan_degree_kind)
            && get_degree_variant_title_base(candidate_title) == plan_title_base
    })
}

fn get_source_degree_kind(text: &str) -> Option<&'static str> {
    if ba_source_degree_pattern().is_match(text) {
        Some("ba")
    } else if bs_source_degree_pattern().is_match(text) {
        Some("bs")
    } else {
        None
    }
}

fn get_ba_bs_degree_kind(title: &str) -> Option<&'static str> {
    let normalized = normalize_matcher_text(title);
    if ba_degree_pattern().is_match(&normalized) {
        Some("ba")
    } else if bs_degree_pattern().is_match(&normalized) {
        Some("bs")
    } else {
        None
    }
}

fn get_degree_variant_title_base(title: &str) -> String {
    let without_parenthetical = degree_kind_parenthetical_pattern()
        .replace_all(title, " ")
        .into_owned();
    let without_suffix = degree_kind_suffix_pattern()
        .replace_all(&without_parenthetical, " ")
        .into_owned();
    normalize_specialized_plan_tokens(&without_suffix, 2).join(" ")
}

fn normalize_matcher_text(value: &str) -> String {
    let mut normalized = String::new();
    let mut pending_separator = false;
    for character in value.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !normalized.is_empty() {
                normalized.push(' ');
            }
            pending_separator = false;
            normalized.push(character);
        } else {
            pending_separator = !normalized.is_empty();
        }
    }
    normalized
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_default()
}

fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
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

macro_rules! matcher {
    ($name:ident, $pattern:literal) => {
        fn $name() -> &'static Regex {
            static VALUE: OnceLock<Regex> = OnceLock::new();
            VALUE.get_or_init(|| Regex::new($pattern).expect(stringify!($name)))
        }
    };
}

matcher!(owner_title_prefix_pattern, r"\s+-\s+");
matcher!(pathway_label_cue_pattern, r"(?i)\b(option|track|route|pathway|concentration)\b");
matcher!(pathway_label_exclusion_pattern, r"(?i)\b(?:major|degree|program|department|school|college|admissions?|application|requirements?|courses?|curriculum)\b");
matcher!(ba_source_degree_pattern, r"(?i)\bB\.?\s*A\.?\b|\bba[-\s]");
matcher!(bs_source_degree_pattern, r"(?i)\bB\.?\s*S\.?\b|\bbs[-\s]");
matcher!(ba_degree_pattern, r"\b(?:b\s*a|ba|bachelor\s+of\s+arts)\b");
matcher!(bs_degree_pattern, r"\b(?:b\s*s|bs|bachelor\s+of\s+science)\b");
matcher!(degree_kind_parenthetical_pattern, r"(?i)\([^)]*\bB[AS]\b[^)]*\)");
matcher!(degree_kind_suffix_pattern, r"(?i)\bB\.?\s*[AS]\.?\b");
