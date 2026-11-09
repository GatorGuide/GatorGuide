use super::requirement_course_codes::CourseCodeParser;
use super::requirement_shape::{strip_leading_requirement_glyphs, strip_snapshot_page_prefix};
use super::requirement_snapshots::{RequirementSourceSnapshot, source_url_without_hash};
use super::transfer_plan_title_matching::{
    normalize_transfer_planner_semantic_pathway_label, normalize_transfer_planner_text,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;
use url::Url;

pub fn plan_has_pathway_manifest_entries(
    plan_id: Option<&Value>,
    manifest_entries: &[Value],
) -> bool {
    let Some(plan_id) = plan_id.filter(|value| js_truthy(Some(*value))) else {
        return false;
    };
    manifest_entries.iter().any(|candidate| {
        candidate.get("planId") == Some(plan_id) && js_truthy(candidate.get("pathwayId"))
    })
}

pub fn parser_recovery_snapshot_conflicts_with_base_owner_scope(
    entry: &Value,
    snapshot_lines: &[String],
    manifest_entries: &[Value],
) -> bool {
    if js_truthy(entry.get("pathwayId"))
        || !plan_has_pathway_manifest_entries(entry.get("planId"), manifest_entries)
    {
        return false;
    }
    let supplemental_pathway_line_count = snapshot_lines
        .iter()
        .map(|line| normalize_whitespace(line))
        .filter(|line| {
            supplemental_official_source_pattern().is_match(line)
                && pathway_label_cue_pattern().is_match(line)
        })
        .count();
    supplemental_pathway_line_count >= 2
}

pub fn score_owner_snapshot_for_parser_recovery(
    entry: &Value,
    snapshot: &RequirementSourceSnapshot,
    course_parser: &CourseCodeParser,
) -> i32 {
    if snapshot.snapshot_lines.is_empty() {
        return 0;
    }
    let mut text_parts = Vec::with_capacity(2 + snapshot.snapshot_lines.len().min(140));
    if let Some(title) = snapshot.title.as_deref().filter(|value| !value.is_empty()) {
        text_parts.push(title);
    }
    if let Some(source_url) = snapshot
        .source_url
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        text_parts.push(source_url);
    }
    text_parts.extend(snapshot.snapshot_lines.iter().take(140).map(String::as_str));
    let snapshot_text = normalize_matcher_text(&text_parts.join(" "));
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let title_tokens = parser_recovery_title_scope_tokens(entry);
    let course_code_count = snapshot
        .snapshot_lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .len() as i32;
    let cue_line_count = snapshot
        .snapshot_lines
        .iter()
        .filter(|line| requirement_cue_pattern().is_match(line))
        .count() as i32;
    let exact_title_score = if !exact_title.is_empty() && snapshot_text.contains(&exact_title) {
        80
    } else {
        0
    };
    let matched_title_token_count = title_tokens
        .iter()
        .filter(|token| snapshot_text.contains(token.as_str()))
        .count() as i32;
    let token_score = matched_title_token_count * 16;
    let source_url_score = match (
        snapshot
            .source_url
            .as_deref()
            .and_then(source_url_without_hash),
        source_url_without_hash(&string_field(entry, "url")),
    ) {
        (Some(snapshot_url), Some(entry_url)) if snapshot_url == entry_url => 20,
        _ => 0,
    };
    let noisy_text = format!(
        "{} {}",
        snapshot.title.as_deref().unwrap_or_default(),
        snapshot.source_url.as_deref().unwrap_or_default()
    );
    let noisy_penalty = if graduate_supplemental_pattern().is_match(&noisy_text) {
        120
    } else {
        0
    };
    if source_url_score == 0
        && exact_title_score == 0
        && title_tokens.len() >= 2
        && matched_title_token_count < 2
    {
        return 0;
    }
    exact_title_score + token_score + course_code_count * 8 + cue_line_count * 4 + source_url_score
        - noisy_penalty
}

pub fn parser_recovery_candidate_contains_selected_pathway_phrase(
    entry: &Value,
    label: &str,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !js_truthy(entry.get("pathwayId")) {
        return true;
    }
    let pathway_id = string_field(entry, "pathwayId");
    let pathway_phrase_tokens = normalize_matcher_text(&replace_path_separators(&pathway_id))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if pathway_phrase_tokens.is_empty()
        || !pathway_phrase_tokens.iter().any(|token| token.len() > 4)
    {
        return true;
    }
    let pathway_phrase = pathway_phrase_tokens.join(" ");
    let link_text = normalize_matcher_text(&format!("{label} {url}"));
    let compact_pathway_id = normalize_matcher_text(&pathway_id).replace(' ', "");
    let compact_link_text = link_text.replace(' ', "");
    let owner_acronym_match =
        parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles)
            .into_iter()
            .any(|token| {
                token.len() >= 3
                    && link_text
                        .split_whitespace()
                        .any(|word| word == token.as_str())
            });
    link_text.contains(&pathway_phrase)
        || (!compact_pathway_id.is_empty() && compact_link_text.contains(&compact_pathway_id))
        || owner_acronym_match
}

pub fn uses_authoritative_bothell_chemistry_curriculum_scope(entry: &Value) -> bool {
    if entry.get("campusId").and_then(Value::as_str) != Some("uw-bothell")
        || !bothell_chemistry_plan_id_pattern().is_match(&string_field(entry, "planId"))
        || !bothell_chemistry_curriculum_url_pattern()
            .is_match(&bothell_chemistry_source_url(entry))
    {
        return false;
    }
    let plan_id = string_field(entry, "planId");
    let pathway_id = string_field(entry, "pathwayId");
    (plan_id == "uw-bothell-chemistry-ba" && !js_truthy(entry.get("pathwayId")))
        || (plan_id == "uw-bothell-chemistry-bs"
            && matches!(
                pathway_id.as_str(),
                "b-s-in-chemistry-general-option" | "biochemistry-option"
            ))
}

pub fn scope_bothell_chemistry_curriculum_lines(
    entry: &Value,
    lines: &[String],
) -> Option<Vec<String>> {
    if !uses_authoritative_bothell_chemistry_curriculum_scope(entry) {
        return None;
    }
    let plan_id = string_field(entry, "planId");
    let pathway_id = string_field(entry, "pathwayId");
    let (start_pattern, end_pattern) = match (plan_id.as_str(), pathway_id.as_str()) {
        ("uw-bothell-chemistry-ba", _) => (
            bothell_chemistry_ba_heading_pattern(),
            bothell_chemistry_general_bs_heading_pattern(),
        ),
        ("uw-bothell-chemistry-bs", "b-s-in-chemistry-general-option") => (
            bothell_chemistry_general_bs_heading_pattern(),
            bothell_chemistry_biochemistry_heading_pattern(),
        ),
        ("uw-bothell-chemistry-bs", "biochemistry-option") => (
            bothell_chemistry_biochemistry_heading_pattern(),
            bothell_chemistry_petitions_heading_pattern(),
        ),
        _ => return None,
    };
    find_exact_heading_section_lines(lines, start_pattern, end_pattern)
}

fn find_exact_heading_section_lines(
    lines: &[String],
    start_pattern: &Regex,
    end_pattern: &Regex,
) -> Option<Vec<String>> {
    let normalized_headings = lines
        .iter()
        .map(|line| {
            markdown_heading_prefix_pattern()
                .replace(&normalize_whitespace(line), "")
                .trim_matches(super::requirement_shape::is_js_whitespace)
                .to_string()
        })
        .collect::<Vec<_>>();
    let start_index = normalized_headings
        .iter()
        .position(|line| start_pattern.is_match(line))?;
    let end_index = normalized_headings
        .iter()
        .enumerate()
        .skip(start_index + 1)
        .find(|(_, line)| !line.is_empty() && end_pattern.is_match(line))
        .map(|(index, _)| index.saturating_sub(1))
        .unwrap_or_else(|| lines.len().saturating_sub(1));
    Some(lines[start_index..=end_index].to_vec())
}

fn bothell_chemistry_source_url(entry: &Value) -> String {
    entry
        .get("url")
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_else(|| string_field(entry, "sourceUrl"))
}

fn bothell_chemistry_plan_id_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^uw-bothell-chemistry-(?:ba|bs)$",
        ))
        .expect("Bothell chemistry plan id pattern")
    })
}

fn bothell_chemistry_curriculum_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)/stem/undergraduate/majors/chemistry/curriculum(?:$|[#?])",
        ))
        .expect("Bothell chemistry curriculum URL pattern")
    })
}

fn markdown_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"^#{1,6}\s+",
        ))
        .expect("markdown heading prefix pattern")
    })
}

fn bothell_chemistry_ba_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^B\.A\. in Chemistry$",
        ))
        .expect("Bothell BA heading")
    })
}

fn bothell_chemistry_general_bs_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^B\.S\. in Chemistry \(general option\)$",
        ))
        .expect("Bothell general BS heading")
    })
}

fn bothell_chemistry_biochemistry_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^B\.S\. in Chemistry \(biochemistry option\)$",
        ))
        .expect("Bothell biochemistry heading")
    })
}

fn bothell_chemistry_petitions_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Petitions$",
        ))
        .expect("Bothell petitions heading")
    })
}

pub fn should_use_full_focused_degree_html_scope(
    entry: &Value,
    title: Option<&str>,
    headings: &[String],
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    plan_has_pathways: bool,
    course_parser: &CourseCodeParser,
) -> bool {
    if !matches!(
        entry.get("parserType").and_then(Value::as_str),
        Some("html-degree-page" | "html-curriculum-page" | "html-overview-page")
    ) {
        return false;
    }
    let legacy_catalog_source = is_legacy_student_catalog_source(entry);
    let specialized_credential = catalog_owner_title_has_specialized_credential(entry);
    let should_stop_before_credentials = entry.get("ownerType").and_then(Value::as_str)
        == Some("major")
        && !js_truthy(entry.get("pathwayId"))
        && !specialized_credential
        && plan_has_pathways;
    if (legacy_catalog_source && (specialized_credential || should_stop_before_credentials))
        || js_truthy(entry.get("pathwayId"))
    {
        return false;
    }
    if has_owner_acronym_departmental_boundary(entry, lines, primary_major_titles)
        && find_different_owner_acronym_departmental_boundary_index(
            entry,
            lines,
            primary_major_titles,
        )
        .is_some_and(|index| index > 0)
    {
        return false;
    }

    let sampled_text = normalize_matcher_text(
        &std::iter::once(title.unwrap_or_default())
            .chain(headings.iter().take(30).map(String::as_str))
            .chain(lines.iter().take(320).map(String::as_str))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let owner_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let has_focused_owner_cue = if owner_title.is_empty() {
        parser_recovery_title_scope_tokens(entry)
            .iter()
            .any(|token| sampled_text.contains(token.as_str()))
    } else {
        sampled_text.contains(&owner_title)
    };
    let has_degree_requirement_cue = full_degree_requirement_cue_pattern().is_match(&sampled_text);
    let multiple_requirement_sections = headings
        .iter()
        .filter(|heading| focused_degree_requirement_heading_pattern().is_match(heading))
        .count()
        >= 3;
    let full_page_course_count = lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .len();
    let has_compact_full_degree_course_list = full_page_course_count >= 20
        && lines.len() <= 420
        && compact_full_degree_course_list_pattern().is_match(&sampled_text);
    has_focused_owner_cue
        && has_degree_requirement_cue
        && (multiple_requirement_sections || has_compact_full_degree_course_list)
}

pub fn should_use_full_focused_pathway_degree_html_scope(
    entry: &Value,
    title: Option<&str>,
    headings: &[String],
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> bool {
    if !js_truthy(entry.get("pathwayId"))
        || !matches!(
            entry.get("parserType").and_then(Value::as_str),
            Some("html-degree-page" | "html-curriculum-page" | "html-overview-page")
        )
    {
        return false;
    }
    let source_identity_text = normalize_matcher_text(
        &[
            string_field(entry, "label"),
            string_field(entry, "sourceLabel"),
            nullish_string_field(entry, "url", ""),
            title.unwrap_or_default().to_string(),
        ]
        .into_iter()
        .chain(headings.iter().take(12).cloned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" "),
    );
    let sampled_text = normalize_matcher_text(
        &std::iter::once(title.unwrap_or_default())
            .chain(headings.iter().take(30).map(String::as_str))
            .chain(lines.iter().take(360).map(String::as_str))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let course_count = lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .len();
    let primary_major_titles = HashMap::new();
    source_url_path_mentions_selected_pathway_identity(entry)
        && line_matches_strict_selected_pathway_identity(
            entry,
            &source_identity_text,
            &primary_major_titles,
        )
        && focused_pathway_degree_requirement_pattern().is_match(&sampled_text)
        && course_count >= 8
        && lines.len() <= 520
}

pub fn should_use_full_admission_prerequisite_html_scope(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> bool {
    let entry_text = normalize_matcher_text(
        &[
            string_field(entry, "parserType"),
            string_field(entry, "role"),
            string_field(entry, "label"),
            nullish_string_field(entry, "url", ""),
        ]
        .join(" "),
    );
    if !admission_prerequisite_pattern().is_match(&entry_text) {
        return false;
    }
    let course_count = lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .len();
    if course_count < 3 {
        return false;
    }
    let source_text = normalize_matcher_text(&lines.join(" "));
    full_admission_requirement_pattern().is_match(&source_text)
}

pub fn is_bothell_ias_undergraduate_major_source(entry: &Value) -> bool {
    let plan_id = nullish_string_field(entry, "planId", "");
    let pathway_id = nullish_string_field(entry, "pathwayId", "");
    let source_url = nullish_string_field(entry, "url", "");
    plan_id.starts_with("uw-bothell-")
        && (!js_truthy(entry.get("pathwayId"))
            || default_degree_route_pathway_pattern().is_match(&pathway_id))
        && bothell_ias_undergraduate_major_url_pattern().is_match(&source_url)
}

fn default_degree_route_pathway_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:b-?a|bachelor-of-arts|b-?s|bachelor-of-science)-route$",
        ))
        .expect("default degree route pathway pattern")
    })
}

fn bothell_ias_undergraduate_major_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)/ias/undergraduate/majors/",
        ))
        .expect("Bothell IAS undergraduate major URL pattern")
    })
}

pub fn source_url_path_mentions_selected_pathway_identity(entry: &Value) -> bool {
    if source_url_path_mentions_pathway_id(entry) {
        return true;
    }
    let source_url = nullish_string_field(entry, "url", "sourceUrl");
    let source_url = source_url.trim_matches(super::requirement_shape::is_js_whitespace);
    if !js_truthy(entry.get("pathwayId")) || source_url.is_empty() {
        return false;
    }
    let primary_major_titles = HashMap::new();
    if let Ok(url) = Url::parse(source_url) {
        let Some(decoded_path) = decode_uri_component(url.path()) else {
            return line_matches_strict_selected_pathway_identity(
                entry,
                &source_url,
                &primary_major_titles,
            );
        };
        let pathname_suffix = decoded_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" ");
        line_matches_strict_selected_pathway_identity(
            entry,
            &pathname_suffix,
            &primary_major_titles,
        )
    } else {
        line_matches_strict_selected_pathway_identity(entry, &source_url, &primary_major_titles)
    }
}

fn source_url_path_mentions_pathway_id(entry: &Value) -> bool {
    let pathway_id = nullish_string_field(entry, "pathwayId", "")
        .trim_matches(super::requirement_shape::is_js_whitespace)
        .to_lowercase();
    let source_url = nullish_string_field(entry, "url", "sourceUrl");
    let source_url = source_url.trim_matches(super::requirement_shape::is_js_whitespace);
    if pathway_id.is_empty() || source_url.is_empty() {
        return false;
    }
    let suffix = format!("/{pathway_id}");
    if let Ok(url) = Url::parse(source_url) {
        let pathname = url.path().to_lowercase();
        pathname == suffix || pathname.ends_with(&suffix)
    } else {
        source_url.to_lowercase().contains(&suffix)
    }
}

fn nullish_string_field(value: &Value, key: &str, fallback_key: &str) -> String {
    value
        .get(key)
        .filter(|field| !field.is_null())
        .or_else(|| value.get(fallback_key).filter(|field| !field.is_null()))
        .map(js_string_value)
        .unwrap_or_default()
}

fn focused_pathway_degree_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:degree requirements?|major requirements?|curriculum)\b",
        );
        Regex::new(&pattern).expect("focused pathway degree requirement pattern")
    })
}

fn full_admission_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:admission requirements?|application eligibility|prereq(?:uisite)? courses?|business prereq(?:uisite)?s?)\b",
        );
        Regex::new(&pattern).expect("full admission prerequisite HTML scope")
    })
}

pub fn scope_curriculum_era_lines(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let Some(legacy_start_index) = find_prior_admit_curriculum_start_index(lines, course_parser)
    else {
        return lines.to_vec();
    };
    if !is_prior_admit_curriculum_owner(entry) {
        let preceding_text = normalize_matcher_text(&lines[..legacy_start_index].join(" "));
        let has_current_curriculum_before_legacy = formal_options_pattern()
            .is_match(&preceding_text)
            && (shared_curriculum_pattern().is_match(&preceding_text)
                || beginning_spring_2026_pattern().is_match(&preceding_text)
                || spring_2026_pattern().is_match(&preceding_text))
            && (foundation_courses_pattern().is_match(&preceding_text)
                || methods_courses_pattern().is_match(&preceding_text)
                || introductory_courses_pattern().is_match(&preceding_text)
                || beginning_spring_2026_pattern().is_match(&preceding_text));
        return if has_current_curriculum_before_legacy {
            lines[..legacy_start_index].to_vec()
        } else {
            lines.to_vec()
        };
    }

    let mut legacy_end_index = lines.len().saturating_sub(1);
    for (index, line) in lines.iter().enumerate().skip(legacy_start_index + 1) {
        if graduation_or_contact_heading_pattern().is_match(&normalize_whitespace(line)) {
            legacy_end_index = index.saturating_sub(1).max(legacy_start_index);
            break;
        }
    }
    lines[legacy_start_index..=legacy_end_index].to_vec()
}

pub fn remove_following_different_owner_acronym_departmental_sections(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let owner_acronyms = parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles);
    if owner_acronyms.is_empty() {
        return lines.to_vec();
    }
    let mut found_owner_boundary = false;
    for (index, line) in lines.iter().enumerate() {
        let normalized_line = normalize_whitespace(line);
        let Some(captures) = departmental_degree_boundary_pattern().captures(&normalized_line)
        else {
            continue;
        };
        let Some(acronym) = captures.get(1) else {
            continue;
        };
        let boundary_acronym = normalize_matcher_text(acronym.as_str()).replace(' ', "");
        if boundary_acronym.is_empty() {
            continue;
        }
        if owner_acronyms.contains(&boundary_acronym) {
            found_owner_boundary = true;
        } else if found_owner_boundary {
            return lines[..index].to_vec();
        }
    }
    lines.to_vec()
}

pub fn find_different_owner_acronym_departmental_boundary_index(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
) -> Option<usize> {
    let owner_acronyms = parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles);
    if owner_acronyms.is_empty() {
        return None;
    }
    for (index, line) in lines.iter().enumerate().skip(1) {
        let normalized_line = normalize_whitespace(line);
        let Some(captures) =
            different_owner_acronym_departmental_boundary_pattern().captures(&normalized_line)
        else {
            continue;
        };
        let boundary_acronym = captures
            .get(1)
            .map(|matched| normalize_matcher_text(matched.as_str()).replace(' ', ""))
            .unwrap_or_default();
        if !boundary_acronym.is_empty() && !owner_acronyms.contains(&boundary_acronym) {
            return Some(index);
        }
    }
    None
}

pub fn has_owner_acronym_departmental_boundary(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let owner_acronyms = parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles);
    !owner_acronyms.is_empty()
        && lines.iter().any(|line| {
            departmental_degree_boundary_pattern()
                .captures(&normalize_whitespace(line))
                .and_then(|captures| captures.get(1))
                .is_some_and(|acronym| {
                    owner_acronyms
                        .contains(&normalize_matcher_text(acronym.as_str()).replace(' ', ""))
                })
        })
}

pub fn is_html_section_boundary_line(line: &str) -> bool {
    html_section_boundary_pattern().is_match(line)
}

pub fn is_back_to_top_heading_line(line: &str) -> bool {
    back_to_top_heading_pattern().is_match(&normalize_whitespace(line))
}

pub fn is_parser_recovery_pathway_section_boundary_line(line: &str) -> bool {
    pathway_section_boundary_pattern().is_match(&normalize_whitespace(line))
}

pub fn is_parser_recovery_section_boundary_line(line: &str) -> bool {
    html_section_boundary_pattern().is_match(line)
        || generic_section_boundary_pattern().is_match(line)
        || is_parser_recovery_pathway_section_boundary_line(line)
}

pub fn has_parser_recovery_requirement_cue(text: &str) -> bool {
    requirement_cue_pattern().is_match(text)
}

pub fn find_preceding_inline_requirement_start_index(
    entry: &Value,
    lines: &[String],
    scoped_start_index: usize,
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    if entry.is_null() || scoped_start_index == 0 || scoped_start_index >= lines.len() {
        return None;
    }
    let min_index = scoped_start_index.saturating_sub(8);
    let mut start_index = None;
    for index in (min_index..scoped_start_index).rev() {
        let line = strip_choice_list_line(&lines[index]);
        if looks_like_preceding_inline_requirement_line(&line, course_parser) {
            start_index = Some(index);
            continue;
        }
        if start_index.is_some() {
            break;
        }
        if line.is_empty() {
            continue;
        }
        if html_section_boundary_pattern().is_match(&line)
            || primary_requirement_line_pattern().is_match(&line)
            || pathway_section_boundary_pattern_source().is_match(&line)
            || preceding_inline_hard_boundary_pattern().is_match(&line)
        {
            break;
        }
    }
    start_index
}

pub fn has_upcoming_sectioned_course_rows(
    lines: &[String],
    start_index: usize,
    minimum_count: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let mut course_row_count = 0;
    let scan_end = lines.len().min(start_index.saturating_add(95));
    for index in start_index.saturating_add(1)..scan_end {
        let line = strip_choice_list_line(&lines[index]);
        if line.is_empty() {
            continue;
        }
        if is_sectioned_option_course_boundary(&line, course_parser) {
            break;
        }
        if sectioned_course_column_header_pattern().is_match(&line) {
            continue;
        }
        let course_line = sectioned_option_course_line(&line);
        if course_line.is_none()
            && course_row_count == 0
            && index > start_index.saturating_add(1)
            && (has_primary_requirement_section_cue(&line)
                || looks_like_sectioned_course_heading_line(&line, course_parser)
                || looks_like_short_named_course_section_heading(&line, course_parser))
        {
            break;
        }
        if course_line.is_some() {
            course_row_count += 1;
            if course_row_count >= minimum_count {
                return true;
            }
        }
    }
    false
}

pub fn remove_parallel_pathway_html_table_content_from_base_scope(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if js_truthy(entry.get("pathwayId")) || lines.len() < 8 {
        return lines.to_vec();
    }

    let mut remove_ranges = find_base_formal_options_removal_ranges(entry, lines, course_parser);
    for index in 0..lines.len() {
        if !course_parser.extract_from_line(&lines[index]).is_empty()
            || !is_likely_base_parallel_pathway_html_section_start_line(lines, index, course_parser)
        {
            continue;
        }
        let mut run_end = index;
        while run_end + 1 < lines.len()
            && course_parser
                .extract_from_line(&lines[run_end + 1])
                .is_empty()
            && is_likely_base_parallel_pathway_html_section_start_line(
                lines,
                run_end + 1,
                course_parser,
            )
        {
            run_end += 1;
        }
        if run_end == index {
            continue;
        }

        let content_start = run_end + 1;
        let mut content_end = lines.len().saturating_sub(1).min(content_start + 220);
        for scan_index in content_start.saturating_add(1)..=content_end {
            let line = normalize_whitespace(&lines[scan_index]);
            if scan_index > content_start + 2
                && !line.is_empty()
                && course_parser.extract_from_line(&line).is_empty()
                && pathway_html_table_content_end_boundary_pattern().is_match(&line)
            {
                content_end = scan_index - 1;
                break;
            }
        }
        let removed_course_count = lines[index..=content_end]
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if removed_course_count > 0 {
            remove_ranges.push((index, content_end));
        }
    }

    for index in 0..lines.len() {
        let line = normalize_whitespace(&lines[index]);
        if !course_parser.extract_from_line(&line).is_empty()
            || !is_likely_base_parallel_pathway_html_section_start_line(lines, index, course_parser)
        {
            continue;
        }
        let mut content_end = lines.len().saturating_sub(1).min(index + 220);
        for scan_index in index.saturating_add(1)..=content_end {
            let scan_line = normalize_whitespace(&lines[scan_index]);
            if scan_line.is_empty() {
                continue;
            }
            if scan_index > index + 2
                && is_likely_base_parallel_pathway_html_section_start_line(
                    lines,
                    scan_index,
                    course_parser,
                )
            {
                content_end = scan_index - 1;
                break;
            }
            if scan_index > index + 2
                && course_parser.extract_from_line(&scan_line).is_empty()
                && base_scope_pathway_section_end_boundary_pattern().is_match(&scan_line)
            {
                content_end = scan_index - 1;
                break;
            }
        }
        let removed_course_count = lines[index..=content_end]
            .iter()
            .map(|removed_line| course_parser.extract_from_line(removed_line).len())
            .sum::<usize>();
        if removed_course_count > 0 {
            remove_ranges.push((index, content_end));
        }
    }

    if remove_ranges.is_empty() {
        return lines.to_vec();
    }
    lines
        .iter()
        .enumerate()
        .filter(|(line_index, _)| {
            remove_ranges.iter().all(|(start_index, end_index)| {
                *line_index < *start_index || *line_index > *end_index
            })
        })
        .map(|(_, line)| line.clone())
        .collect()
}

fn find_base_formal_options_removal_ranges(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<(usize, usize)> {
    if js_truthy(entry.get("pathwayId")) || lines.len() < 8 {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    for index in 0..lines.len() {
        if !is_formal_options_base_section_start_line(&lines[index], course_parser) {
            continue;
        }
        let intro_text =
            normalize_matcher_text(&lines[index..lines.len().min(index + 18)].join(" "));
        if !choose_one_formal_options_pattern().is_match(&intro_text) {
            continue;
        }
        let option_end = lines.len().min(index + 40);
        let option_heading_indexes = (index + 1..option_end)
            .filter(|line_index| is_formal_option_heading_line(&lines[*line_index], course_parser))
            .collect::<Vec<_>>();
        if option_heading_indexes.len() < 2
            || option_heading_indexes
                .first()
                .is_some_and(|first| *first > index + 12)
        {
            continue;
        }

        let mut end_index = lines.len().saturating_sub(1).min(index + 220);
        for scan_index in index + 3..=end_index {
            let scan_line = strip_choice_list_line(&lines[scan_index]);
            if course_parser.extract_from_line(&scan_line).is_empty()
                && formal_options_end_boundary_pattern().is_match(&scan_line)
            {
                end_index = scan_index - 1;
                break;
            }
        }
        let removed_course_count = lines[index..=end_index]
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if removed_course_count > 0 {
            ranges.push((index, end_index));
        }
    }
    ranges
}

fn is_formal_options_base_section_start_line(line: &str, course_parser: &CourseCodeParser) -> bool {
    formal_options_heading_pattern().is_match(&strip_choice_list_line(line))
        && course_parser.extract_from_line(line).is_empty()
}

fn is_formal_option_heading_line(line: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty() || !course_parser.extract_from_line(&normalized_line).is_empty() {
        return false;
    }
    formal_option_enumerated_heading_pattern().is_match(&normalized_line)
        || (normalized_line.split_whitespace().count() <= 8
            && formal_option_cue_pattern().is_match(&normalized_line)
            && !formal_option_action_pattern().is_match(&normalized_line))
}

fn is_likely_base_parallel_pathway_html_section_start_line(
    lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    is_likely_base_peer_pathway_html_section_start_line(&lines[index], course_parser)
        || is_likely_formal_pathway_table_heading_line(lines, index, course_parser)
}

fn is_likely_base_peer_pathway_html_section_start_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized = strip_pathway_heading_prefix(line);
    is_likely_peer_pathway_html_section_start_line(&normalized, course_parser)
        && pathway_label_cue_pattern().is_match(&normalized)
}

pub fn is_likely_peer_pathway_html_section_start_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized = strip_pathway_heading_prefix(line);
    let matcher_text = normalize_matcher_text(&normalized);
    if matcher_text.is_empty()
        || course_list_section_heading_pattern().is_match(&matcher_text)
        || noisy_inline_line_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&normalized)
    {
        return false;
    }
    let word_count = matcher_text.split_whitespace().count();
    if word_count > 12 || peer_pathway_prose_rejection_pattern().is_match(&normalized) {
        return false;
    }
    let starts_with_enumerator =
        pathway_heading_enumerator_pattern().is_match(&normalize_transfer_planner_text(line));
    let has_pathway_cue = pathway_label_cue_pattern().is_match(&normalized);
    let is_uppercase_heading = normalized
        .chars()
        .any(|character| character.is_ascii_uppercase())
        && normalized == normalized.to_uppercase();
    has_pathway_cue || starts_with_enumerator || (is_uppercase_heading && word_count <= 8)
}

fn has_recent_formal_pathway_choice_context(lines: &[String], index: usize) -> bool {
    let start = index.saturating_sub(8);
    formal_options_context_pattern()
        .is_match(&normalize_matcher_text(&lines[start..index].join(" ")))
}

fn is_likely_formal_pathway_table_heading_line(
    lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let raw_line = normalize_whitespace(&lines[index]);
    let normalized = strip_pathway_heading_prefix(&raw_line);
    let matcher_text = normalize_matcher_text(&normalized);
    if raw_line.is_empty()
        || matcher_text.is_empty()
        || !has_recent_formal_pathway_choice_context(lines, index)
        || noisy_inline_line_pattern().is_match(&raw_line)
        || !course_parser.extract_from_line(&raw_line).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&raw_line)
    {
        return false;
    }
    let word_count = matcher_text.split_whitespace().count();
    word_count > 0 && word_count <= 8 && pathway_heading_enumerator_pattern().is_match(&raw_line)
}

pub fn find_pathway_html_curriculum_section_candidate(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let mut candidates = Vec::new();
    let mut selected_start_indexes = Vec::new();
    let mut has_sibling_pathway_section = false;
    for (index, line) in lines.iter().enumerate() {
        let selected = is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        let peer =
            is_pathway_html_section_sibling_start(entry, line, primary_major_titles, course_parser);
        if selected {
            selected_start_indexes.push(index);
        } else if peer {
            has_sibling_pathway_section = true;
        }
    }
    if selected_start_indexes.is_empty() || !has_sibling_pathway_section {
        return None;
    }

    for selected_start_index in selected_start_indexes.iter().copied().filter(|index| {
        pathway_curriculum_or_requirements_pattern()
            .is_match(&strip_pathway_heading_prefix(&lines[*index]))
    }) {
        let end_index = find_pathway_html_section_end_index(
            entry,
            lines,
            selected_start_index,
            primary_major_titles,
            course_parser,
        );
        let section_lines = &lines[selected_start_index..=end_index];
        let course_count = section_lines
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if course_count == 0 {
            continue;
        }
        let start_index = find_pathway_html_section_start_index(
            entry,
            lines,
            selected_start_index,
            course_parser,
        );
        candidates.push((course_count, selected_start_index, start_index, end_index));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    candidates
        .first()
        .map(|candidate| (candidate.2, candidate.3))
}

pub fn find_pathway_html_selected_section_candidate(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let mut selected_start_indexes = Vec::new();
    let mut has_sibling_pathway_section = false;
    for (index, line) in lines.iter().enumerate() {
        let selected = is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        let peer =
            is_pathway_html_section_sibling_start(entry, line, primary_major_titles, course_parser);
        if selected {
            selected_start_indexes.push(index);
        } else if peer {
            has_sibling_pathway_section = true;
        }
    }
    if selected_start_indexes.is_empty() || !has_sibling_pathway_section {
        return None;
    }
    let mut candidates = Vec::<(usize, usize, usize, usize)>::new();
    for selected_start_index in selected_start_indexes {
        let end_index = find_pathway_html_section_end_index(
            entry,
            lines,
            selected_start_index,
            primary_major_titles,
            course_parser,
        );
        let course_count = lines[selected_start_index..=end_index]
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if course_count == 0 {
            continue;
        }
        let start_index = find_pathway_html_section_start_index(
            entry,
            lines,
            selected_start_index,
            course_parser,
        );
        candidates.push((course_count, selected_start_index, start_index, end_index));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    candidates
        .first()
        .map(|candidate| (candidate.2, candidate.3))
}

pub fn find_parallel_pathway_html_table_scope(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize, Vec<String>)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let mut peer_indexes = Vec::<(usize, bool)>::new();
    let mut peer_index_set = HashSet::new();
    for (index, line) in lines.iter().enumerate() {
        let selected = is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        let peer =
            is_pathway_html_section_sibling_start(entry, line, primary_major_titles, course_parser);
        if selected || peer {
            peer_indexes.push((index, selected));
            if course_parser.extract_from_line(line).is_empty() {
                peer_index_set.insert(index);
            }
        }
    }
    if !peer_indexes.iter().any(|(_, selected)| !selected) {
        return None;
    }

    for selected_start_index in peer_indexes
        .iter()
        .filter_map(|(index, selected)| (*selected).then_some(*index))
    {
        if !peer_index_set.contains(&selected_start_index) {
            continue;
        }
        let mut run_start = selected_start_index;
        while run_start > 0
            && (peer_index_set.contains(&(run_start - 1))
                || is_parallel_pathway_heading_bridge_line(
                    lines.get(run_start - 1).map(String::as_str),
                    course_parser,
                ))
        {
            run_start -= 1;
        }
        let mut run_end = selected_start_index;
        while run_end + 1 < lines.len()
            && (peer_index_set.contains(&(run_end + 1))
                || is_parallel_pathway_heading_bridge_line(
                    lines.get(run_end + 1).map(String::as_str),
                    course_parser,
                ))
        {
            run_end += 1;
        }
        let run_indexes = (run_start..=run_end)
            .filter(|index| peer_index_set.contains(index))
            .collect::<Vec<_>>();
        if run_indexes.len() < 2 {
            continue;
        }
        let Some(selected_position) = run_indexes
            .iter()
            .position(|index| *index == selected_start_index)
        else {
            continue;
        };
        let content_start = run_end + 1;
        let selected_heading = &lines[selected_start_index];
        let later_sibling_headings = run_indexes
            .iter()
            .skip(selected_position + 1)
            .map(|index| lines[*index].clone())
            .collect::<Vec<_>>();
        let mut start_index = content_start;
        if selected_position > 0 {
            let Some(repeated_selected_heading_index) = find_pathway_html_table_content_boundary(
                lines,
                content_start,
                std::slice::from_ref(selected_heading),
                course_parser,
            ) else {
                continue;
            };
            start_index = repeated_selected_heading_index;
        }
        let next_sibling_content_index = if later_sibling_headings.is_empty() {
            None
        } else {
            find_pathway_html_table_content_boundary(
                lines,
                content_start,
                &later_sibling_headings,
                course_parser,
            )
        };
        if start_index >= lines.len() {
            continue;
        }
        let default_end_index = find_pathway_html_table_content_end_index(
            entry,
            lines,
            start_index,
            primary_major_titles,
            course_parser,
        );
        let end_index = next_sibling_content_index
            .filter(|next_index| *next_index > start_index)
            .map(|next_index| default_end_index.min(next_index - 1))
            .unwrap_or(default_end_index);
        let selected_heading_bridge_lines = if selected_position < run_indexes.len() - 1 {
            lines[selected_start_index + 1..run_indexes[selected_position + 1]]
                .iter()
                .filter(|line| {
                    is_parallel_pathway_heading_bridge_line(Some(line.as_str()), course_parser)
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut seen = HashSet::new();
        let section_lines = std::iter::once(selected_heading.to_string())
            .chain(selected_heading_bridge_lines)
            .chain(lines[start_index..=end_index].iter().cloned())
            .filter(|line| seen.insert(line.clone()))
            .collect::<Vec<_>>();
        if section_lines
            .iter()
            .any(|line| !course_parser.extract_from_line(line).is_empty())
        {
            return Some((start_index, end_index, section_lines));
        }
    }
    None
}

fn find_pathway_html_table_content_boundary(
    lines: &[String],
    start_index: usize,
    heading_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    if lines.is_empty() || start_index >= lines.len() {
        return None;
    }
    let forward_limit = lines.len().saturating_sub(1).min(start_index + 220);
    for index in start_index..=forward_limit {
        let line = normalize_whitespace(&lines[index]);
        if line.is_empty()
            || !course_parser.extract_from_line(&line).is_empty()
            || pathway_section_boundary_pattern_source().is_match(&line)
        {
            continue;
        }
        let likely_heading = is_likely_peer_pathway_html_section_start_line(&line, course_parser)
            || is_likely_bare_pathway_html_section_heading(&line, course_parser);
        if !likely_heading {
            let normalized_line = normalize_transfer_planner_text(&line);
            let word_count = normalize_matcher_text(&line).split_whitespace().count();
            if word_count > 0
                && word_count <= 14
                && inline_repeated_pathway_heading_pattern().is_match(&normalized_line)
                && heading_lines
                    .iter()
                    .any(|heading_line| line_matches_pathway_heading_identity(heading_line, &line))
            {
                return Some(index);
            }
            continue;
        }
        if heading_lines
            .iter()
            .any(|heading_line| line_matches_pathway_heading_identity(heading_line, &line))
        {
            return Some(index);
        }
    }
    None
}

fn line_matches_pathway_heading_identity(heading_line: &str, candidate_line: &str) -> bool {
    let tokens = normalize_matcher_text(&strip_pathway_heading_prefix(heading_line))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let normalized_candidate = normalize_matcher_text(candidate_line);
    if tokens.is_empty() || normalized_candidate.is_empty() {
        return false;
    }
    let token_matches = tokens
        .iter()
        .filter(|token| normalized_line_contains_pathway_token(&normalized_candidate, token))
        .count();
    let required_matches = if tokens.len() <= 2 { tokens.len() } else { 2 };
    if token_matches >= required_matches.max(1) {
        return true;
    }
    let heading_acronyms = pathway_heading_acronym_pattern()
        .find_iter(&strip_pathway_heading_prefix(heading_line))
        .map(|matched| matched.as_str().to_string())
        .filter(|token| !pathway_heading_excluded_acronym_pattern().is_match(token))
        .collect::<HashSet<_>>();
    pathway_heading_acronym_pattern()
        .find_iter(&strip_pathway_heading_prefix(candidate_line))
        .any(|matched| heading_acronyms.contains(matched.as_str()))
}

fn is_parallel_pathway_heading_bridge_line(
    line: Option<&str>,
    course_parser: &CourseCodeParser,
) -> bool {
    let Some(line) = line else {
        return false;
    };
    let normalized_line = strip_choice_list_line(line);
    !normalized_line.is_empty()
        && course_parser.extract_from_line(&normalized_line).is_empty()
        && normalized_line.split_whitespace().count() <= 14
        && parallel_pathway_heading_bridge_pattern().is_match(&normalized_line)
}

fn find_pathway_html_table_content_end_index(
    entry: &Value,
    lines: &[String],
    start_index: usize,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> usize {
    let default_end_index = find_pathway_html_section_end_index(
        entry,
        lines,
        start_index,
        primary_major_titles,
        course_parser,
    );
    for index in start_index + 1..=default_end_index {
        let line = normalize_whitespace(&lines[index]);
        if index > start_index + 2
            && !line.is_empty()
            && course_parser.extract_from_line(&line).is_empty()
            && pathway_html_table_content_end_boundary_pattern().is_match(&line)
        {
            return index.saturating_sub(1).max(start_index);
        }
    }
    default_end_index
}

fn inline_repeated_pathway_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i):\s*\b(?:complete|choose|select|required|courses?\s+listed|all\s+\d+\s+courses?)\b")).expect("inline repeated pathway heading"))
}

fn pathway_heading_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\b[A-Z]{2,6}\b",
        ))
        .expect("pathway heading acronym")
    })
}

fn pathway_heading_excluded_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:BA|BS|BABA)$",
        ))
        .expect("pathway heading excluded acronym")
    })
}

fn parallel_pathway_heading_bridge_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:choose|complete|select)\b.*\b(?:courses?|credits?|following|below)\b",
        ))
        .expect("parallel pathway heading bridge")
    })
}

pub fn find_nested_pathway_html_section_scope(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize, Vec<String>)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let peer_indexes = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let selected = is_likely_selected_pathway_html_section_start_line(
                entry,
                line,
                primary_major_titles,
                course_parser,
            );
            let peer = is_pathway_html_section_sibling_start(
                entry,
                line,
                primary_major_titles,
                course_parser,
            );
            (selected || peer).then_some((index, selected))
        })
        .collect::<Vec<_>>();
    if !peer_indexes.iter().any(|(_, selected)| !selected) {
        return None;
    }
    let strict_selected_start_indexes = peer_indexes
        .iter()
        .filter_map(|candidate| {
            let index = candidate.0;
            (candidate.1
                && line_matches_strict_selected_pathway_identity(
                    entry,
                    &lines[index],
                    primary_major_titles,
                ))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    for selected_start_index in strict_selected_start_indexes {
        let selected_end_index = find_pathway_html_section_end_index(
            entry,
            lines,
            selected_start_index,
            primary_major_titles,
            course_parser,
        );
        let selected_lines = lines[selected_start_index..=selected_end_index].to_vec();
        if !selected_lines
            .iter()
            .any(|line| !course_parser.extract_from_line(line).is_empty())
        {
            continue;
        }
        for parent_start_index in
            (selected_start_index.saturating_sub(220)..selected_start_index).rev()
        {
            let parent_line = &lines[parent_start_index];
            let is_loose_selected_parent = (is_likely_selected_pathway_html_section_start_line(
                entry,
                parent_line,
                primary_major_titles,
                course_parser,
            ) || line_matches_pathway_parent_context(
                entry,
                parent_line,
            )) && !is_pathway_html_section_sibling_start(
                entry,
                parent_line,
                primary_major_titles,
                course_parser,
            ) && !line_matches_strict_selected_pathway_identity(
                entry,
                parent_line,
                primary_major_titles,
            );
            if !is_loose_selected_parent {
                continue;
            }
            let first_nested_peer = peer_indexes
                .iter()
                .map(|(index, _)| *index)
                .filter(|index| *index > parent_start_index)
                .min();
            let parent_end_index = first_nested_peer
                .filter(|index| *index <= selected_start_index)
                .map(|index| index - 1)
                .unwrap_or(selected_start_index - 1);
            let parent_lines = &lines[parent_start_index..=parent_end_index];
            if !parent_lines
                .iter()
                .any(|line| !course_parser.extract_from_line(line).is_empty())
            {
                continue;
            }
            let mut seen = HashSet::new();
            let scoped_lines = parent_lines
                .iter()
                .chain(selected_lines.iter())
                .filter_map(|line| seen.insert(line.clone()).then_some(line.clone()))
                .collect();
            return Some((parent_start_index, selected_end_index, scoped_lines));
        }
    }
    None
}

pub fn line_matches_strict_selected_pathway_identity(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty() {
        return false;
    }
    let discriminator_tokens = selected_pathway_discriminator_tokens(entry, primary_major_titles);
    if !discriminator_tokens.is_empty()
        && !discriminator_tokens
            .iter()
            .any(|token| normalized_line_contains_pathway_token(&normalized_line, token))
    {
        return false;
    }
    strict_selected_pathway_scope_token_groups(entry, primary_major_titles)
        .iter()
        .any(|tokens| {
            let matches = tokens
                .iter()
                .filter(|token| normalized_line_contains_pathway_token(&normalized_line, token))
                .count();
            matches >= (if tokens.len() <= 2 { tokens.len() } else { 2 }).max(1)
        })
}

pub fn strict_selected_pathway_scope_token_groups(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<Vec<String>> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let major_title = primary_major_title(entry, primary_major_titles);
    let raw_labels = [
        replace_path_separators(&string_field(entry, "pathwayId")),
        normalize_transfer_planner_semantic_pathway_label(
            &major_title,
            &string_field(entry, "ownerTitle"),
        ),
    ];
    let mut groups = raw_labels
        .iter()
        .map(|label| strip_pathway_heading_prefix(label))
        .map(|label| pathway_scope_identity_tokens(&label))
        .filter(|tokens| !tokens.is_empty())
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| right.len().cmp(&left.len()));
    groups.dedup();
    groups
}

fn line_matches_pathway_parent_context(entry: &Value, line: &str) -> bool {
    if !js_truthy(entry.get("pathwayId")) {
        return false;
    }
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty()
        || !pathway_label_cue_pattern().is_match(&normalize_transfer_planner_text(line))
    {
        return false;
    }
    let owner_title = string_field(entry, "ownerTitle");
    let owner_title_prefix = owner_title_prefix_pattern()
        .split(&owner_title)
        .next()
        .unwrap_or(&owner_title)
        .to_string();
    let parent_labels = [
        string_field(entry, "sourceLabel"),
        string_field(entry, "label"),
        owner_title_prefix,
    ];
    parent_labels.iter().any(|label| {
        let tokens = normalize_matcher_text(&strip_pathway_heading_prefix(label))
            .split_whitespace()
            .filter(|token| {
                token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token)
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        !tokens.is_empty()
            && tokens
                .iter()
                .filter(|token| normalized_line_contains_pathway_token(&normalized_line, token))
                .count()
                >= (if tokens.len() <= 2 { tokens.len() } else { 2 }).max(1)
    })
}

pub(crate) fn is_likely_selected_pathway_html_section_start_line(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    if !line_matches_selected_pathway_identity(entry, line, primary_major_titles)
        || !line_contains_selected_pathway_discriminator(entry, line, primary_major_titles)
    {
        return false;
    }
    let normalized = strip_pathway_heading_prefix(line);
    let matcher_text = normalize_matcher_text(&normalized);
    if matcher_text.is_empty()
        || noisy_inline_line_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&normalized)
    {
        return false;
    }
    let word_count = matcher_text.split_whitespace().count();
    if word_count > 12 {
        return false;
    }
    let starts_with_enumerator =
        pathway_heading_enumerator_pattern().is_match(&normalize_transfer_planner_text(line));
    let has_pathway_cue = pathway_label_cue_pattern().is_match(&normalized);
    let is_uppercase_heading = normalized
        .chars()
        .any(|character| character.is_ascii_uppercase())
        && normalized == normalized.to_uppercase();
    has_pathway_cue
        || starts_with_enumerator
        || is_uppercase_heading
        || word_count <= 6
        || is_likely_bare_pathway_html_section_heading(&normalized, course_parser)
}

pub fn is_pathway_html_section_sibling_start(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    (is_likely_peer_pathway_html_section_start_line(line, course_parser)
        || (is_likely_bare_pathway_html_section_heading(line, course_parser)
            && !line_matches_selected_pathway_identity(entry, line, primary_major_titles)))
        && !is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        )
}

fn is_likely_bare_pathway_html_section_heading(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized = strip_pathway_heading_prefix(line);
    let matcher_text = normalize_matcher_text(&normalized);
    if matcher_text.is_empty()
        || course_list_section_heading_pattern().is_match(&matcher_text)
        || noisy_inline_line_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&normalized)
        || pathway_label_cue_pattern().is_match(&normalized)
        || bare_pathway_subheading_pattern().is_match(&normalized)
        || normalized.ends_with('.')
        || normalized.ends_with(';')
        || normalized.ends_with(':')
    {
        return false;
    }
    let words = matcher_text.split_whitespace().count();
    if words == 0 || words > 5 {
        return false;
    }
    let meaningful_words = normalized
        .split_whitespace()
        .filter(|word| !bare_pathway_filler_word_pattern().is_match(word))
        .collect::<Vec<_>>();
    !meaningful_words.is_empty()
        && meaningful_words
            .iter()
            .all(|word| bare_pathway_visible_word_pattern().is_match(word))
}

pub fn line_matches_selected_pathway_identity(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty() {
        return false;
    }
    let discriminator_tokens = selected_pathway_discriminator_tokens(entry, primary_major_titles);
    if !discriminator_tokens.is_empty()
        && !discriminator_tokens
            .iter()
            .any(|token| normalized_line_contains_pathway_token(&normalized_line, token))
    {
        return false;
    }
    selected_pathway_scope_token_groups(entry, primary_major_titles)
        .iter()
        .any(|tokens| {
            let matches = tokens
                .iter()
                .filter(|token| normalized_line_contains_pathway_token(&normalized_line, token))
                .count();
            let required = if tokens.len() <= 2 { tokens.len() } else { 2 };
            matches >= required.max(1)
        })
}

pub fn is_dedicated_selected_pathway_html_source(
    entry: &Value,
    full_title: Option<&str>,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !js_truthy(entry.get("pathwayId")) || is_legacy_student_catalog_source(entry) {
        return false;
    }
    let source_identity = normalize_matcher_text(
        &[
            string_field(entry, "url"),
            string_field(entry, "label"),
            string_field(entry, "sourceLabel"),
            full_title.unwrap_or_default().to_string(),
        ]
        .join(" "),
    );
    !source_identity.is_empty()
        && strict_selected_pathway_scope_token_groups(entry, primary_major_titles)
            .iter()
            .any(|tokens| {
                !tokens.is_empty()
                    && tokens
                        .iter()
                        .all(|token| source_identity.contains(token.as_str()))
            })
}

pub fn parsed_mentions_selected_pathway_section(
    entry: &Value,
    snapshot_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    snapshot_lines.iter().any(|line| {
        is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        )
    })
}

pub fn parsed_mentions_selected_pathway_identity(
    entry: &Value,
    snapshot_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    snapshot_lines
        .iter()
        .any(|line| line_matches_selected_pathway_identity(entry, line, primary_major_titles))
}

pub fn parsed_mentions_sibling_pathway_section(
    entry: &Value,
    snapshot_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    snapshot_lines.iter().any(|line| {
        is_pathway_html_section_sibling_start(entry, line, primary_major_titles, course_parser)
    })
}

fn line_contains_selected_pathway_discriminator(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let discriminator_tokens = selected_pathway_discriminator_tokens(entry, primary_major_titles);
    discriminator_tokens.is_empty() || {
        let normalized_line = normalize_matcher_text(line);
        discriminator_tokens
            .iter()
            .any(|token| normalized_line_contains_pathway_token(&normalized_line, token))
    }
}

fn normalized_line_contains_pathway_token(normalized_line: &str, token: &str) -> bool {
    Regex::new(&format!(r"(?i)\b{}\b", regex::escape(token)))
        .is_ok_and(|pattern| pattern.is_match(normalized_line))
}

fn selected_pathway_discriminator_tokens(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let major_title = primary_major_title(entry, primary_major_titles);
    let major_tokens = normalize_matcher_text(&major_title)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let pathway_id_label = replace_path_separators(&string_field(entry, "pathwayId"));
    let candidate_labels = [
        pathway_id_label,
        normalize_transfer_planner_semantic_pathway_label(
            &major_title,
            &string_field(entry, "ownerTitle"),
        ),
    ];
    let mut tokens = Vec::new();
    for label in candidate_labels {
        for token in normalize_matcher_text(&label).split_whitespace() {
            if token.len() >= 3
                && !pathway_scope_identity_stopwords().contains(token)
                && !major_tokens.contains(token)
                && !tokens
                    .iter()
                    .any(|existing: &String| existing.as_str() == token)
            {
                tokens.push(token.to_string());
            }
        }
    }
    tokens
}

fn find_pathway_html_section_end_index(
    entry: &Value,
    lines: &[String],
    selected_start_index: usize,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> usize {
    let mut end_index = lines
        .len()
        .saturating_sub(1)
        .min(selected_start_index + 220);
    let forward_limit = end_index;
    for index in selected_start_index + 1..=forward_limit {
        let line = normalize_whitespace(&lines[index]);
        if line.is_empty() {
            continue;
        }
        if is_pathway_html_section_sibling_start(entry, &line, primary_major_titles, course_parser)
        {
            end_index = index.saturating_sub(1).max(selected_start_index);
            break;
        }
        if index > selected_start_index + 2
            && (pathway_section_boundary_pattern_source().is_match(&line)
                || back_to_top_boundary_pattern().is_match(&line)
                || (html_section_boundary_pattern().is_match(&line)
                    && !is_likely_selected_pathway_html_section_start_line(
                        entry,
                        &line,
                        primary_major_titles,
                        course_parser,
                    )))
        {
            end_index = index.saturating_sub(1).max(selected_start_index);
            break;
        }
    }
    end_index
}

fn find_pathway_html_section_start_index(
    entry: &Value,
    lines: &[String],
    selected_start_index: usize,
    course_parser: &CourseCodeParser,
) -> usize {
    let mut start_index = selected_start_index;
    if concentration_pathway_id_pattern().is_match(&string_field(entry, "pathwayId")) {
        for index in (selected_start_index.saturating_sub(80)..selected_start_index).rev() {
            if concentration_area_heading_pattern().is_match(&normalize_whitespace(&lines[index])) {
                start_index = index;
                break;
            }
        }
    }
    for index in (selected_start_index.saturating_sub(3)..selected_start_index).rev() {
        if !course_parser.extract_from_line(&lines[index]).is_empty()
            || is_likely_peer_pathway_html_section_start_line(&lines[index], course_parser)
        {
            break;
        }
        if is_likely_pathway_choice_context_line(&lines[index], course_parser) {
            start_index = index;
            break;
        }
    }
    if !is_legacy_student_catalog_source(entry) {
        if let Some(prerequisite_start_index) = find_preceding_admission_requirement_start_index(
            entry,
            lines,
            start_index,
            course_parser,
        ) {
            start_index = prerequisite_start_index;
        }
    }
    start_index
}

pub fn find_preceding_admission_requirement_start_index(
    entry: &Value,
    lines: &[String],
    scoped_start_index: usize,
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    if scoped_start_index == 0 || scoped_start_index > lines.len() {
        return None;
    }
    let owner_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let prefers_science = admission_science_owner_pattern().is_match(&owner_title);
    let prefers_arts = admission_arts_owner_pattern().is_match(&owner_title);
    let minimum_index = scoped_start_index.saturating_sub(90);
    let mut candidates = Vec::<(usize, usize, usize)>::new();
    for index in (minimum_index..scoped_start_index).rev() {
        let line = normalize_whitespace(&lines[index]);
        let searchable_line = normalize_matcher_text(&line);
        if !admission_prerequisite_pattern().is_match(&searchable_line) {
            continue;
        }
        let between_course_count = lines[index..scoped_start_index]
            .iter()
            .flat_map(|between_line| course_parser.extract_from_line(between_line))
            .collect::<BTreeSet<_>>()
            .len();
        if between_course_count == 0 {
            continue;
        }
        let option_score = if (prefers_science
            && admission_science_option_pattern().is_match(&searchable_line))
            || (prefers_arts && admission_arts_option_pattern().is_match(&searchable_line))
        {
            2
        } else {
            0
        };
        let generic_score = admission_generic_prerequisite_pattern().is_match(&line) as usize;
        candidates.push((
            option_score + generic_score,
            scoped_start_index - index,
            index,
        ));
    }
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| right.2.cmp(&left.2))
    });
    candidates.first().map(|candidate| candidate.2)
}

pub fn pathway_html_section_scope_lines(
    entry: &Value,
    lines: &[String],
    start_index: usize,
    end_index: usize,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let selected_lines = lines[start_index..=end_index].to_vec();
    preserve_prior_admit_pathway_context(entry, lines, &selected_lines, course_parser)
}

pub fn preserve_prior_admit_pathway_context(
    entry: &Value,
    lines: &[String],
    selected_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if !is_prior_admit_curriculum_owner(entry) {
        return selected_lines.to_vec();
    }
    let Some(formal_options_index) = lines
        .iter()
        .position(|line| formal_options_heading_pattern().is_match(&strip_choice_list_line(line)))
    else {
        return selected_lines.to_vec();
    };
    let Some(first_pathway_heading_index) = lines.iter().enumerate().find_map(|(index, line)| {
        (index > formal_options_index
            && is_likely_peer_pathway_html_section_start_line(line, course_parser))
        .then_some(index)
    }) else {
        return selected_lines.to_vec();
    };
    let mut seen = HashSet::new();
    lines[..first_pathway_heading_index]
        .iter()
        .chain(selected_lines.iter())
        .filter_map(|line| seen.insert(line.clone()).then_some(line.clone()))
        .collect()
}

pub fn is_legacy_student_catalog_source(entry: &Value) -> bool {
    let source_url = entry
        .get("url")
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_default();
    legacy_student_catalog_source_pattern().is_match(&source_url)
}

fn legacy_student_catalog_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)//(?:www\.)?washington\.edu/students/gencat/program/",
        ))
        .expect("legacy student catalog source pattern")
    })
}

fn admission_prerequisite_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\bprereq(?:uisites?)?\b");
        Regex::new(&pattern).expect("admission prerequisite pattern")
    })
}

fn admission_science_owner_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:bachelor of science|bs)\b",
        );
        Regex::new(&pattern).expect("admission science owner pattern")
    })
}

fn admission_arts_owner_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:bachelor of arts|ba)\b",
        );
        Regex::new(&pattern).expect("admission arts owner pattern")
    })
}

fn admission_science_option_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:bachelor of science|science|bs)\b",
        );
        Regex::new(&pattern).expect("admission science option pattern")
    })
}

fn admission_arts_option_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:bachelor of arts|arts|ba)\b",
        );
        Regex::new(&pattern).expect("admission arts option pattern")
    })
}

fn admission_generic_prerequisite_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^prereq(?:uisites?)?$");
        Regex::new(&pattern).expect("generic prerequisite pattern")
    })
}

fn is_likely_pathway_choice_context_line(line: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized = normalize_transfer_planner_text(line);
    !normalized.is_empty()
        && course_parser.extract_from_line(&normalized).is_empty()
        && normalized.split_whitespace().count() <= 24
        && pathway_choice_context_pattern().is_match(&normalized)
}

pub fn should_use_full_generic_concentration_degree_scope(
    entry: &Value,
    headings: &[String],
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> bool {
    if js_truthy(entry.get("pathwayId"))
        || entry.get("parserType").and_then(Value::as_str) != Some("generic-html")
        || entry
            .get("isPrimaryDegreeRequirementsLink")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return false;
    }
    let mut searchable_parts = vec![
        string_field(entry, "ownerTitle"),
        string_field(entry, "label"),
        string_field(entry, "sourceLabel"),
    ];
    searchable_parts.extend(headings.iter().cloned());
    searchable_parts.extend(lines.iter().take(160).cloned());
    let searchable_text = normalize_matcher_text(&searchable_parts.join(" "));
    if !generic_degree_requirements_phrase_pattern().is_match(&searchable_text)
        || !concentration_courses_phrase_pattern().is_match(&searchable_text)
    {
        return false;
    }
    lines.iter().enumerate().any(|(index, line)| {
        concentration_courses_heading_pattern().is_match(&strip_choice_list_line(line))
            && has_upcoming_sectioned_course_rows(lines, index, 1, course_parser)
    })
}

fn looks_like_preceding_inline_requirement_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty()
        || noisy_inline_line_pattern().is_match(&normalized_line)
        || transfer_credit_noise_pattern().is_match(&normalized_line)
        || source_line_starts_with_course_code(&normalized_line)
    {
        return false;
    }
    let Some((label, body)) = inline_labeled_requirement_parts(&normalized_line, course_parser)
    else {
        return false;
    };
    if course_parser.extract_from_line(&body).is_empty() {
        return false;
    }
    !non_requirement_inline_label_pattern().is_match(&label)
}

fn inline_labeled_requirement_parts(
    raw_line: &str,
    course_parser: &CourseCodeParser,
) -> Option<(String, String)> {
    let line_without_choice_markers = strip_choice_list_line(raw_line);
    let normalized_line = normalize_whitespace(
        &inline_labeled_requirement_marker_pattern().replace(&line_without_choice_markers, ""),
    );
    let colon_index = normalized_line.find(':')?;
    if colon_index == 0 {
        return None;
    }
    let label = trailing_label_punctuation_pattern()
        .replace_all(&normalized_line[..colon_index], "")
        .to_string();
    let label = normalize_whitespace(&label);
    let body = normalize_whitespace(&normalized_line[colon_index + 1..]);
    if label.is_empty()
        || body.is_empty()
        || label.len() > 140
        || !course_parser.extract_from_line(&label).is_empty()
        || course_parser.extract_from_line(&body).is_empty()
    {
        return None;
    }
    Some((label, body))
}

fn strip_choice_list_line(line: &str) -> String {
    let without_page_prefix = strip_snapshot_page_prefix(line);
    normalize_whitespace(&strip_leading_requirement_glyphs(&without_page_prefix))
}

fn source_line_starts_with_course_code(line: &str) -> bool {
    source_line_course_prefix_pattern().is_match(line)
}

fn is_sectioned_option_course_boundary(line: &str, course_parser: &CourseCodeParser) -> bool {
    sectioned_option_course_supplemental_boundary_pattern().is_match(line)
        || sectioned_option_course_service_boundary_pattern().is_match(line)
        || (sectioned_option_course_requirement_boundary_pattern().is_match(line)
            && !sectioned_option_course_core_exception_pattern().is_match(line)
            && course_parser.extract_from_line(line).is_empty())
}

fn sectioned_option_course_line(line: &str) -> Option<String> {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty() {
        return None;
    }
    let mut candidates = vec![normalized_line.clone()];
    let without_credit_prefix = sectioned_course_credit_prefix_pattern()
        .replace(&normalized_line, "")
        .to_string();
    candidates.push(without_credit_prefix.clone());
    let without_bracket_prefix = sectioned_bracket_prefix_pattern()
        .replace(&normalized_line, "")
        .to_string();
    candidates.push(without_bracket_prefix.clone());
    candidates.push(
        sectioned_bracket_prefix_pattern()
            .replace(&without_credit_prefix, "")
            .to_string(),
    );
    candidates
        .into_iter()
        .map(|candidate| normalize_whitespace(&candidate))
        .find(|candidate| !candidate.is_empty() && source_line_starts_with_course_code(candidate))
}

pub fn has_sectioned_course_list_requirement_cue(line: &str) -> bool {
    sectioned_course_list_requirement_pattern().is_match(line)
}

pub fn get_pathway_heading_identity_token_count(line: &str) -> usize {
    normalize_matcher_text(&strip_pathway_heading_prefix(line))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .count()
}

pub fn has_primary_requirement_section_cue(line: &str) -> bool {
    primary_requirement_section_cue_pattern().is_match(line)
}

pub fn looks_like_sectioned_course_heading_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty()
        || sectioned_course_season_heading_pattern().is_match(&normalized_line)
        || source_line_starts_with_course_code(&normalized_line)
        || (!course_parser.extract_from_line(&normalized_line).is_empty()
            && !course_level_heading_pattern().is_match(&normalized_line))
    {
        return false;
    }
    sectioned_course_required_count_pattern().is_match(&normalized_line)
        || (sectioned_course_list_requirement_pattern().is_match(&normalized_line)
            && sectioned_course_heading_qualifier_pattern().is_match(&normalized_line))
}

pub fn looks_like_short_named_course_section_heading(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = strip_choice_list_line(line);
    let length = normalized_line.chars().count();
    length >= 3
        && length <= 90
        && normalized_line
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
        && !normalized_line.ends_with('.')
        && !normalized_line.ends_with(';')
        && !sectioned_course_credit_list_line(&normalized_line)
        && !source_line_starts_with_course_code(&normalized_line)
        && course_parser.extract_from_line(&normalized_line).is_empty()
        && !short_named_course_heading_exclusion_pattern().is_match(&normalized_line)
}

fn sectioned_course_credit_list_line(line: &str) -> bool {
    let normalized_line = strip_choice_list_line(line);
    let parts = normalized_line
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    !parts.is_empty()
        && parts.iter().all(|part| {
            sectioned_course_credit_range_pattern()
                .captures(part)
                .and_then(|captures| {
                    let minimum = captures.get(1)?.as_str().parse::<f64>().ok()?;
                    let maximum = captures
                        .get(2)
                        .and_then(|value| value.as_str().parse::<f64>().ok())
                        .unwrap_or(minimum);
                    (minimum > 0.0 && minimum <= 30.0 && maximum >= minimum && maximum <= 30.0)
                        .then_some(())
                })
                .is_some()
        })
}

pub fn is_approved_elective_html_source(
    entry: &Value,
    title: Option<&str>,
    headings: &[String],
    lines: &[String],
) -> bool {
    let source_identity_text = normalize_matcher_text(
        &[
            string_field(entry, "label"),
            string_field(entry, "sourceLabel"),
            string_field(entry, "url"),
            title.unwrap_or_default().to_string(),
            headings.join(" "),
            lines.iter().take(40).cloned().collect::<Vec<_>>().join(" "),
        ]
        .join(" "),
    );
    approved_electives_pattern().is_match(&source_identity_text)
        || elective_list_webpage_courses_pattern().is_match(&source_identity_text)
}

pub fn scope_html_lines_by_title(
    entry: &Value,
    title: Option<&str>,
    headings: &[String],
    source_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let title_tokens = parser_recovery_title_scope_tokens(entry);
    if title_tokens.is_empty() || source_lines.len() <= 20 {
        return source_lines.to_vec();
    }
    let use_owner_section_scoping = is_legacy_student_catalog_source(entry);
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let normalized_headings = headings
        .iter()
        .map(|heading| normalize_whitespace(heading))
        .collect::<HashSet<_>>();
    let owner_acronyms = parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles);
    let mut scored_lines = source_lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let normalized_line = normalize_matcher_text(line);
            let token_score = title_tokens
                .iter()
                .filter(|token| normalized_line.contains(token.as_str()))
                .count() as i32;
            let exact_title_score =
                !exact_title.is_empty() && normalized_line.contains(&exact_title);
            let heading_score = normalized_headings.contains(line) as i32 * 2;
            let line_tokens = normalized_line.split_whitespace().collect::<HashSet<_>>();
            let acronym_matches = owner_acronyms
                .iter()
                .filter(|token| line_tokens.contains(token.as_str()))
                .collect::<Vec<_>>();
            let heading_like_acronym_requirement_line = html_section_boundary_pattern()
                .is_match(&normalize_whitespace(line))
                || normalize_whitespace(line).split_whitespace().count() <= 8;
            let acronym_requirement_score = if !acronym_matches.is_empty()
                && heading_like_acronym_requirement_line
                && (structural_requirement_pattern().is_match(line)
                    || primary_requirement_line_pattern().is_match(line))
            {
                if acronym_matches.iter().any(|token| token.len() >= 4) {
                    28
                } else {
                    24
                }
            } else {
                0
            };
            let owner_section_start_bonus = if use_owner_section_scoping
                && is_likely_owner_html_section_start_line(entry, line, primary_major_titles)
            {
                10
            } else {
                0
            };
            let other_section_boundary_penalty = if use_owner_section_scoping
                && html_section_boundary_pattern().is_match(&normalize_whitespace(line))
                && !is_likely_owner_html_section_start_line(entry, line, primary_major_titles)
            {
                4
            } else {
                0
            };
            let requirement_signal_score = generic_html_requirement_signal_pattern()
                .find_iter(&normalized_line)
                .count() as i32;
            let transfer_noise_penalty = if transfer_credit_noise_pattern().is_match(line) {
                8
            } else {
                0
            };
            let score = (if exact_title_score { 10 } else { 0 })
                + acronym_requirement_score
                + token_score * 2
                + heading_score
                + owner_section_start_bonus
                + requirement_signal_score
                - other_section_boundary_penalty
                - transfer_noise_penalty;
            (score > 0).then_some((index, score))
        })
        .collect::<Vec<_>>();
    scored_lines.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let Some((best_line_index, best_score)) = scored_lines.first().copied() else {
        return source_lines.to_vec();
    };
    let matching_indexes = scored_lines
        .iter()
        .filter(|(index, score)| {
            index.abs_diff(best_line_index) <= 40 && *score >= 3.max(best_score - 3)
        })
        .map(|(index, _)| *index)
        .collect::<Vec<_>>();
    if matching_indexes.is_empty() {
        return source_lines.to_vec();
    }

    let owner_section_range = if use_owner_section_scoping {
        find_owner_html_section_range(entry, source_lines, best_line_index, primary_major_titles)
    } else {
        None
    };
    let mut start_index = owner_section_range.map(|range| range.0).unwrap_or_else(|| {
        matching_indexes
            .iter()
            .min()
            .copied()
            .unwrap_or(0)
            .saturating_sub(6)
    });
    let initial_end_index = owner_section_range.map(|range| range.1).unwrap_or_else(|| {
        source_lines.len().saturating_sub(1).min(
            matching_indexes
                .iter()
                .max()
                .copied()
                .unwrap_or(0)
                .saturating_add(24),
        )
    });
    let scoped_table_tail_start = start_index.max(initial_end_index.saturating_sub(16));
    let scoped_table_tail_text = normalize_matcher_text(
        &source_lines[scoped_table_tail_start..=initial_end_index].join(" "),
    );
    let approved_electives_scope_text = [
        string_field(entry, "label"),
        string_field(entry, "url"),
        source_lines[start_index..=initial_end_index].join(" "),
    ]
    .join(" ");
    let is_approved_electives_scope = approved_electives_pattern()
        .is_match(&normalize_matcher_text(&approved_electives_scope_text));
    let mut end_index = initial_end_index;
    if owner_section_range.is_none()
        && (generic_html_table_signal_pattern().is_match(&scoped_table_tail_text)
            || sectioned_course_list_requirement_pattern().is_match(&scoped_table_tail_text))
    {
        let continuation_end = source_lines
            .len()
            .min(initial_end_index.saturating_add(501));
        let continuation_tail =
            &source_lines[initial_end_index.saturating_add(1)..continuation_end];
        let mut continuation_course_count = 0;
        let mut quiet_line_count = 0;
        for (offset, continuation_line) in continuation_tail.iter().enumerate() {
            let line = normalize_whitespace(continuation_line);
            if line.is_empty() {
                continue;
            }
            let line_course_codes = course_parser.extract_from_line(&line);
            let has_table_signal = generic_html_table_signal_pattern().is_match(&line)
                || sectioned_course_list_requirement_pattern().is_match(&line);
            if continuation_course_count >= 4
                && html_section_boundary_pattern().is_match(&line)
                && !has_table_signal
            {
                let upcoming_end = continuation_tail.len().min(offset.saturating_add(13));
                let upcoming_course_count = continuation_tail[offset + 1..upcoming_end]
                    .iter()
                    .map(|nearby_line| course_parser.extract_from_line(nearby_line).len())
                    .sum::<usize>();
                if is_approved_electives_scope && upcoming_course_count > 0 {
                    quiet_line_count = 0;
                    end_index = initial_end_index.saturating_add(offset).saturating_add(1);
                    continue;
                }
                break;
            }
            if !line_course_codes.is_empty() || has_table_signal {
                continuation_course_count += line_course_codes.len();
                quiet_line_count = 0;
                end_index = initial_end_index.saturating_add(offset).saturating_add(1);
                continue;
            }
            if continuation_course_count == 0 {
                if offset >= 2 {
                    break;
                }
                continue;
            }
            quiet_line_count += 1;
            end_index = initial_end_index.saturating_add(offset).saturating_add(1);
            if quiet_line_count >= 16 {
                break;
            }
        }
        if continuation_course_count < 4 {
            end_index = initial_end_index;
        }
    }

    if owner_section_range.is_none() {
        if let Some(preceding_admission_start) = find_preceding_admission_requirement_start_index(
            entry,
            source_lines,
            start_index,
            course_parser,
        ) {
            start_index = preceding_admission_start;
        }
        if let Some(inline_requirement_start) = find_preceding_inline_requirement_start_index(
            entry,
            source_lines,
            start_index,
            course_parser,
        ) {
            start_index = inline_requirement_start;
        }
    }
    let mut scoped_lines = source_lines[start_index..=end_index].to_vec();
    if let Some(different_owner_boundary_index) =
        find_different_owner_acronym_departmental_boundary_index(
            entry,
            &scoped_lines,
            primary_major_titles,
        )
        .filter(|index| *index > 0)
    {
        scoped_lines.truncate(different_owner_boundary_index);
    }
    if scoped_lines.len() < 12 {
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            let mut with_title = Vec::with_capacity(scoped_lines.len() + 1);
            with_title.push(title.to_string());
            with_title.extend(scoped_lines);
            return with_title;
        }
    }
    scoped_lines
}

fn is_likely_owner_html_section_start_line(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty()
        || !html_section_boundary_pattern().is_match(&normalize_whitespace(line))
        || line_introduces_different_owner_descriptor(entry, &normalized_line)
    {
        return false;
    }
    let title_tokens = parser_recovery_title_scope_tokens(entry);
    if title_tokens.is_empty() {
        return false;
    }
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let token_matches = title_tokens
        .iter()
        .filter(|token| normalized_line.contains(token.as_str()))
        .count();
    let minimum_matches = 1.max(2.min(title_tokens.len()));
    (!exact_title.is_empty() && normalized_line.contains(&exact_title))
        || token_matches >= minimum_matches
}

fn line_introduces_different_owner_descriptor(entry: &Value, normalized_line: &str) -> bool {
    let owner_searchable_text = normalize_matcher_text(&format!(
        "{} {} {}",
        string_field(entry, "ownerTitle"),
        string_field(entry, "label"),
        string_field(entry, "sourceLabel")
    ));
    if owner_searchable_text.is_empty() {
        return false;
    }
    if matches!(
        entry.get("ownerType").and_then(Value::as_str),
        Some("major" | "pathway")
    ) && different_owner_degree_level_descriptor_pattern().is_match(normalized_line)
    {
        return true;
    }
    if data_science_descriptor_pattern().is_match(normalized_line)
        && !owner_searchable_text.contains("data science")
    {
        return true;
    }
    let descriptor = owner_descriptor_suffix_pattern()
        .captures(normalized_line)
        .and_then(|captures| captures.get(1))
        .map(|matched| normalize_matcher_text(matched.as_str()))
        .unwrap_or_default();
    !descriptor.is_empty()
        && owner_descriptor_kind_pattern().is_match(&descriptor)
        && !owner_searchable_text.contains(&descriptor)
}

fn find_owner_html_section_range(
    entry: &Value,
    lines: &[String],
    anchor_index: usize,
    primary_major_titles: &HashMap<String, String>,
) -> Option<(usize, usize)> {
    let backward_limit = anchor_index.saturating_sub(80);
    let mut section_start = None;
    let mut closest_start = None;
    for index in (backward_limit..=anchor_index.min(lines.len().saturating_sub(1))).rev() {
        let line = normalize_whitespace(&lines[index]);
        if line.is_empty() {
            continue;
        }
        if is_likely_owner_html_section_start_line(entry, &line, primary_major_titles) {
            if closest_start.is_none() {
                closest_start = Some(index);
            }
            if owner_program_of_study_heading_pattern().is_match(&line) {
                section_start = Some(index);
                break;
            }
        }
        if back_to_top_heading_pattern().is_match(&line) {
            break;
        }
    }
    let section_start = section_start.or(closest_start)?;
    let mut section_end = lines
        .len()
        .saturating_sub(1)
        .min(section_start.saturating_add(80));
    let forward_limit = lines
        .len()
        .saturating_sub(1)
        .min(section_start.saturating_add(220));
    for index in section_start.saturating_add(1)..=forward_limit {
        let line = normalize_whitespace(&lines[index]);
        if line.is_empty() {
            continue;
        }
        if back_to_top_heading_pattern().is_match(&line) {
            section_end = index.saturating_sub(1).max(section_start);
            break;
        }
        if html_section_boundary_pattern().is_match(&line)
            && !is_likely_owner_html_section_start_line(entry, &line, primary_major_titles)
        {
            section_end = index.saturating_sub(1).max(section_start);
            break;
        }
    }
    Some((
        section_start.saturating_sub(2),
        section_end.max(section_start),
    ))
}

pub fn parser_recovery_section_line_score(entry: &Value, line: &str) -> i32 {
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty() || graduate_supplemental_pattern().is_match(line) {
        return 0;
    }
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let mut score = 0;
    if !exact_title.is_empty() && normalized_line.contains(&exact_title) {
        score += 24;
    }
    score += parser_recovery_title_scope_tokens(entry)
        .iter()
        .filter(|token| normalized_line.contains(token.as_str()))
        .count() as i32
        * 5;
    if structural_requirement_pattern().is_match(line)
        || primary_requirement_line_pattern().is_match(line)
    {
        score += 12;
    }
    if requirement_cue_pattern().is_match(line) {
        score += 6;
    }
    if pathway_cue_pattern().is_match(line) {
        score += 5;
    }
    if section_noise_penalty_pattern().is_match(line) {
        score -= 12;
    }
    score
}

pub fn is_legacy_catalog_credential_heading_line(line: &str) -> bool {
    legacy_catalog_credential_heading_pattern().is_match(&normalize_whitespace(line))
}

pub fn line_looks_like_legacy_catalog_credential_section_start(
    lines: &[String],
    index: usize,
) -> bool {
    if index >= lines.len() {
        return false;
    }
    let following_text = lines[index + 1..lines.len().min(index + 7)]
        .iter()
        .map(|line| normalize_whitespace(line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    legacy_catalog_section_start_pattern().is_match(&following_text)
}

pub fn parser_recovery_candidate_conflicts_with_program_sibling(
    entry: &Value,
    label: &str,
    url: &str,
) -> bool {
    if !is_different_program_sibling_page(&string_field(entry, "url"), url) {
        return false;
    }
    let link_text = normalize_matcher_text(&format!("{label} {url}"));
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    if !exact_title.is_empty() && link_text.contains(&exact_title) {
        return false;
    }
    let title_tokens = parser_recovery_title_scope_tokens(entry);
    let title_token_overlap_count = title_tokens
        .iter()
        .filter(|token| link_text.contains(token.as_str()))
        .count();
    title_tokens.len() >= 2 && title_token_overlap_count < 2.min(title_tokens.len())
}

pub fn parser_recovery_candidate_conflicts_with_pathway_scope(
    entry: &Value,
    label: &str,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !js_truthy(entry.get("pathwayId")) {
        return parser_recovery_candidate_conflicts_with_base_owner_scope(
            entry,
            label,
            url,
            primary_major_titles,
        );
    }
    let link_text = format!("{label} {url}");
    let has_pathway_cue = selected_pathway_label_cue_pattern().is_match(&link_text);
    let is_support_or_admission_link = is_support_or_admission_link_pattern().is_match(&link_text);
    let owner_identity_text = normalize_matcher_text(&format!(
        "{} {} {} {}",
        string_field(entry, "pathwayId"),
        string_field(entry, "ownerTitle"),
        string_field(entry, "sourceLabel"),
        string_field(entry, "label")
    ));
    if has_pathway_cue {
        if let Some(acronym) = candidate_option_acronym_pattern()
            .captures(&link_text)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_lowercase())
        {
            if !owner_identity_text
                .split_whitespace()
                .any(|token| token == acronym.as_str())
            {
                return true;
            }
        }
    }

    let pathway_id = string_field(entry, "pathwayId");
    let pathway_phrase_tokens = normalize_matcher_text(&replace_path_separators(&pathway_id))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let pathway_phrase = pathway_phrase_tokens.join(" ");
    let normalized_link_text = normalize_matcher_text(&link_text);
    let compact_pathway_id = normalize_matcher_text(&pathway_id).replace(' ', "");
    let compact_link_text = normalized_link_text.replace(' ', "");
    if has_pathway_cue
        && pathway_phrase_tokens.iter().any(|token| token.len() > 4)
        && !normalized_link_text.contains(&pathway_phrase)
        && !(!compact_pathway_id.is_empty() && compact_link_text.contains(&compact_pathway_id))
    {
        return true;
    }

    let owner_tokens =
        parser_recovery_selected_pathway_identity_tokens(entry, primary_major_titles);
    let owner_token_groups = selected_pathway_scope_token_groups(entry, primary_major_titles);
    let candidate_tokens =
        parser_recovery_pathway_identity_tokens(entry, &link_text, primary_major_titles);
    let looks_like_sibling_pathway_identity = !is_support_or_admission_link
        && is_same_origin_child_or_sibling_page(&string_field(entry, "url"), url)
        && candidate_tokens.len() >= 2;
    if !has_pathway_cue && !looks_like_sibling_pathway_identity {
        return false;
    }

    if !owner_token_groups.is_empty() && !candidate_tokens.is_empty() {
        if parser_recovery_candidate_has_sibling_pathway_qualifier(
            entry,
            &owner_token_groups,
            &candidate_tokens,
            url,
            primary_major_titles,
        ) {
            return true;
        }
        let candidate_token_set = candidate_tokens.iter().cloned().collect::<HashSet<_>>();
        let has_complete_owner_group = owner_token_groups.iter().any(|group| {
            group
                .iter()
                .all(|token| candidate_token_set.contains(token))
        });
        if !has_complete_owner_group {
            return true;
        }
    }

    !owner_tokens.is_empty()
        && !candidate_tokens.is_empty()
        && !candidate_tokens
            .iter()
            .any(|token| owner_tokens.contains(token))
}

pub fn parser_recovery_candidate_conflicts_with_base_owner_scope(
    entry: &Value,
    label: &str,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if js_truthy(entry.get("pathwayId")) {
        return false;
    }
    let Some(normalized_candidate_url) = normalize_url_for_comparison(url) else {
        return false;
    };
    let normalized_entry_url = normalize_url_for_comparison(&string_field(entry, "url"));
    let hash_fragment = url_hash_fragment(url);
    if (normalized_entry_url.as_deref() == Some(normalized_candidate_url.as_str())
        && hash_fragment.is_empty())
        || !is_same_origin_child_or_sibling_page(&string_field(entry, "url"), url)
    {
        return false;
    }

    let link_text = format!("{label} {url}");
    if is_support_or_admission_link_pattern().is_match(&link_text) {
        return false;
    }
    let last_segment = last_url_path_segment(url);
    let normalized_hash = normalize_matcher_text(&replace_path_separators(&hash_fragment));
    let generic_pathway_hub_segment = generic_pathway_hub_segment_pattern().is_match(&last_segment);
    let generic_hash_fragment =
        normalized_hash.is_empty() || generic_pathway_hash_pattern().is_match(&normalized_hash);
    if generic_pathway_hub_segment && generic_hash_fragment {
        return false;
    }

    let has_specific_pathway_cue = pathway_cue_pattern().is_match(&link_text)
        || specific_pathway_segment_pattern().is_match(&last_segment)
        || !generic_hash_fragment;
    if !has_specific_pathway_cue {
        return false;
    }

    let candidate_identity = format!(
        "{label} {} {}",
        replace_path_separators(&last_segment),
        replace_path_separators(&hash_fragment)
    );
    !parser_recovery_pathway_identity_tokens(entry, &candidate_identity, primary_major_titles)
        .is_empty()
}

fn parser_recovery_pathway_identity_tokens(
    entry: &Value,
    value: &str,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let plan_id = string_field(entry, "planId");
    let primary_title = primary_major_titles
        .get(&plan_id)
        .cloned()
        .unwrap_or_else(|| {
            let owner_title = normalize_transfer_planner_text(&string_field(entry, "ownerTitle"));
            owner_title_prefix_pattern()
                .split(&owner_title)
                .next()
                .unwrap_or(&owner_title)
                .to_string()
        });
    let primary_title_tokens = normalize_matcher_text(&primary_title)
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut primary_tokens = primary_title_tokens
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let acronym = primary_title_tokens
        .iter()
        .filter_map(|token| token.chars().next())
        .collect::<String>();
    if acronym.len() >= 2 {
        primary_tokens.insert(acronym);
    }
    let generic_tokens = parser_recovery_generic_identity_tokens();
    normalize_matcher_text(value)
        .split_whitespace()
        .filter(|token| {
            token.len() >= 3 && !primary_tokens.contains(*token) && !generic_tokens.contains(*token)
        })
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn parser_recovery_owner_program_acronym_tokens(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> BTreeSet<String> {
    let primary_title = primary_major_title(entry, primary_major_titles);
    let mut acronyms = program_acronym_tokens_from_title(&primary_title, true);
    acronyms.extend(program_acronym_tokens_from_title(
        &string_field(entry, "ownerTitle"),
        false,
    ));
    acronyms.extend(program_acronym_tokens_from_title(
        &string_field(entry, "sourceLabel"),
        false,
    ));
    let route_text = normalize_matcher_text(&format!(
        "{} {} {} {} {}",
        string_field(entry, "ownerId"),
        string_field(entry, "pathwayId"),
        string_field(entry, "ownerTitle"),
        string_field(entry, "sourceLabel"),
        string_field(entry, "label")
    ));
    if route_ell_acronym_pattern().is_match(&route_text)
        || teaching_ell_acronym_pattern().is_match(&route_text)
    {
        acronyms.extend(["ell".to_string(), "tell".to_string()]);
    }
    if route_sped_acronym_pattern().is_match(&route_text) {
        acronyms.insert("sped".to_string());
    }
    acronyms
}

fn program_acronym_tokens_from_title(value: &str, include_title_acronym: bool) -> BTreeSet<String> {
    let raw_title = normalize_whitespace(value);
    if raw_title.is_empty() {
        return BTreeSet::new();
    }
    let mut tokens = explicit_program_acronym_pattern()
        .captures_iter(&raw_title)
        .filter_map(|captures| captures.get(1))
        .map(|value| normalize_matcher_text(value.as_str()).replace(' ', ""))
        .filter(|token| valid_program_acronym(token))
        .collect::<BTreeSet<_>>();
    if include_title_acronym {
        let normalized_title = normalize_matcher_text(&raw_title);
        let title_words = normalized_title
            .split_whitespace()
            .filter(|token| {
                token.len() >= 2
                    && !program_acronym_stopwords().contains(*token)
                    && !excluded_program_acronym_tokens().contains(*token)
            })
            .collect::<Vec<_>>();
        let title_acronym = title_words
            .iter()
            .filter_map(|token| token.chars().next())
            .collect::<String>();
        if valid_program_acronym(&title_acronym) {
            tokens.insert(title_acronym);
        }
    }
    tokens
}

fn valid_program_acronym(value: &str) -> bool {
    (2..=8).contains(&value.len()) && !excluded_program_acronym_tokens().contains(value)
}

fn program_acronym_stopwords() -> BTreeSet<&'static str> {
    [
        "a",
        "and",
        "b",
        "bachelor",
        "bachelors",
        "ba",
        "bothell",
        "bs",
        "campus",
        "curriculum",
        "degree",
        "for",
        "in",
        "major",
        "of",
        "official",
        "overview",
        "page",
        "program",
        "requirements",
        "s",
        "seattle",
        "tacoma",
        "the",
        "to",
        "undergraduate",
        "university",
        "uw",
        "washington",
        "with",
    ]
    .into_iter()
    .collect()
}

fn excluded_program_acronym_tokens() -> BTreeSet<&'static str> {
    [
        "aa", "aab", "aas", "ba", "bas", "bba", "bfa", "bs", "bse", "bsn", "ma", "mba", "mfa", "ms",
    ]
    .into_iter()
    .collect()
}

pub fn primary_major_title(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> String {
    primary_major_titles
        .get(&string_field(entry, "planId"))
        .cloned()
        .unwrap_or_else(|| {
            let owner_title = normalize_transfer_planner_text(&string_field(entry, "ownerTitle"));
            owner_title_prefix_pattern()
                .split(&owner_title)
                .next()
                .unwrap_or(&owner_title)
                .to_string()
        })
}

fn selected_pathway_scope_labels(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let major_title = primary_major_title(entry, primary_major_titles);
    let major_tokens = normalize_matcher_text(&major_title)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let pathway_id_label = replace_path_separators(&string_field(entry, "pathwayId"));
    let raw_labels = [
        pathway_id_label.clone(),
        normalize_transfer_planner_semantic_pathway_label(
            &major_title,
            &string_field(entry, "ownerTitle"),
        ),
        normalize_transfer_planner_semantic_pathway_label(
            &major_title,
            &string_field(entry, "sourceLabel"),
        ),
        normalize_transfer_planner_semantic_pathway_label(
            &major_title,
            &string_field(entry, "label"),
        ),
    ];
    let pathway_id_tokens = normalize_matcher_text(&pathway_id_label)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let mut labels = Vec::new();
    for raw_label in raw_labels {
        let label = strip_pathway_heading_prefix(&raw_label);
        let normalized_label = normalize_matcher_text(&label);
        let tokens = normalized_label
            .split_whitespace()
            .filter(|token| {
                token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token)
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            continue;
        }
        let has_pathway_cue = selected_pathway_label_cue_pattern().is_match(&label);
        if !has_pathway_cue
            && tokens.len() > 1
            && tokens.iter().all(|token| major_tokens.contains(token))
        {
            continue;
        }
        if (has_pathway_cue || tokens.iter().any(|token| pathway_id_tokens.contains(token)))
            && !labels.contains(&label)
        {
            labels.push(label);
        }
    }
    labels
}

fn selected_pathway_scope_token_groups(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<Vec<String>> {
    let mut groups = selected_pathway_scope_labels(entry, primary_major_titles)
        .iter()
        .map(|label| pathway_scope_identity_tokens(label))
        .filter(|tokens| !tokens.is_empty())
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| right.len().cmp(&left.len()));
    groups.dedup();
    groups
}

fn pathway_scope_identity_tokens(label: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    normalize_matcher_text(label)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .filter_map(|token| {
            let token = token.to_string();
            seen.insert(token.clone()).then_some(token)
        })
        .collect()
}

fn parser_recovery_selected_pathway_identity_tokens(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let mut labels = selected_pathway_scope_labels(entry, primary_major_titles);
    labels.push(replace_path_separators(&string_field(entry, "pathwayId")));
    labels
        .iter()
        .flat_map(|label| {
            normalize_matcher_text(label)
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|token| {
            token.len() >= 3 && !pathway_scope_identity_stopwords().contains(token.as_str())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parser_recovery_candidate_url_pathway_tokens(
    entry: &Value,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let Ok(parsed) = Url::parse(url) else {
        return Vec::new();
    };
    let segments = parsed
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let mut decoded_segments = Vec::new();
    for segment in segments
        .into_iter()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let Some(decoded) = decode_uri_component(segment) else {
            return Vec::new();
        };
        decoded_segments.push(decoded);
    }
    let primary_tokens = normalize_matcher_text(&primary_major_title(entry, primary_major_titles))
        .split_whitespace()
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let generic_tokens = pathway_scope_identity_stopwords()
        .into_iter()
        .chain([
            "admission",
            "admissions",
            "curriculum",
            "prereq",
            "prerequisite",
            "prerequisites",
            "requirement",
            "requirements",
        ])
        .collect::<HashSet<_>>();
    normalize_matcher_text(&decoded_segments.join(" "))
        .split_whitespace()
        .filter(|token| {
            token.len() >= 3 && !generic_tokens.contains(*token) && !primary_tokens.contains(*token)
        })
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parser_recovery_candidate_has_sibling_pathway_qualifier(
    entry: &Value,
    owner_token_groups: &[Vec<String>],
    candidate_tokens: &[String],
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !js_truthy(entry.get("pathwayId"))
        || candidate_tokens.is_empty()
        || owner_token_groups.is_empty()
        || owner_token_groups.iter().any(|group| group.len() != 1)
    {
        return false;
    }
    let owner_tokens = owner_token_groups
        .iter()
        .filter_map(|group| group.first().cloned())
        .collect::<HashSet<_>>();
    let url_tokens = parser_recovery_candidate_url_pathway_tokens(entry, url, primary_major_titles);
    !url_tokens.is_empty() && url_tokens.iter().any(|token| !owner_tokens.contains(token))
}

fn strip_pathway_heading_prefix(value: &str) -> String {
    let mut label = normalize_transfer_planner_text(value);
    label = pathway_letter_heading_prefix_pattern()
        .replace(&label, "")
        .into_owned();
    label = pathway_numeric_heading_prefix_pattern()
        .replace(&label, "")
        .into_owned();
    pathway_roman_heading_prefix_pattern()
        .replace(&label, "")
        .trim()
        .to_string()
}

fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            decoded.push((hex_value(high)? << 4) | hex_value(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn pathway_scope_identity_stopwords() -> HashSet<&'static str> {
    [
        "and",
        "ba",
        "bachelor",
        "bs",
        "certificate",
        "concentration",
        "degree",
        "major",
        "of",
        "option",
        "pathway",
        "program",
        "credit",
        "credits",
        "route",
        "track",
        "with",
    ]
    .into_iter()
    .collect()
}

fn is_same_origin_child_or_sibling_page(base_url: &str, resolved_url: &str) -> bool {
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    if base.origin() != resolved.origin() {
        return false;
    }
    let base_segments = base
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let resolved_segments = resolved
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if base_segments.is_empty() || resolved_segments.is_empty() {
        return false;
    }
    let shared_prefix_length = base_segments
        .iter()
        .zip(resolved_segments.iter())
        .position(|(base_segment, resolved_segment)| base_segment != resolved_segment)
        .unwrap_or(base_segments.len().min(resolved_segments.len()));
    shared_prefix_length >= 1.max(3.min(base_segments.len().saturating_sub(1)))
}

pub fn is_different_program_sibling_page(base_url: &str, resolved_url: &str) -> bool {
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    if base.origin() != resolved.origin() {
        return false;
    }
    let base_segments = base
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let resolved_segments = resolved
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let divergence_index = base_segments
        .iter()
        .zip(resolved_segments.iter())
        .position(|(base_segment, resolved_segment)| base_segment != resolved_segment)
        .unwrap_or(base_segments.len().min(resolved_segments.len()));
    let Some(parent_segment) = divergence_index
        .checked_sub(1)
        .and_then(|index| base_segments.get(index))
    else {
        return false;
    };
    let Some(base_divergent_segment) = base_segments.get(divergence_index) else {
        return false;
    };
    let Some(resolved_divergent_segment) = resolved_segments.get(divergence_index) else {
        return false;
    };
    let parent_is_program_collection = matches!(*parent_segment, "majors" | "programs" | "degrees");
    let both_are_credential_programs = credential_program_path_segment_pattern()
        .is_match(base_divergent_segment)
        && credential_program_path_segment_pattern().is_match(resolved_divergent_segment);
    divergence_index > 0 && (parent_is_program_collection || both_are_credential_programs)
}

fn normalize_url_for_comparison(value: &str) -> Option<String> {
    let mut parsed = Url::parse(value).ok()?;
    parsed.set_fragment(None);
    let path = parsed.path().trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let query = parsed
        .query()
        .filter(|query| !query.is_empty())
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    Some(format!(
        "{}{path}{query}",
        parsed.origin().ascii_serialization()
    ))
}

fn url_hash_fragment(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|parsed| parsed.fragment().map(str::to_string))
        .unwrap_or_default()
}

fn last_url_path_segment(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|parsed| {
            parsed
                .path()
                .split('/')
                .filter(|segment| !segment.is_empty())
                .next_back()
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn replace_path_separators(value: &str) -> String {
    path_separator_pattern()
        .replace_all(value, " ")
        .into_owned()
}

pub fn line_references_different_major_scope(entry: &Value, line: &str) -> bool {
    let normalized_owner_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    if normalized_owner_title.is_empty() {
        return false;
    }
    let normalized_line = normalize_whitespace(line);
    let Some(mentioned_scope) = cross_major_scope_pattern()
        .captures(&normalized_line)
        .and_then(|captures| captures.get(1))
        .map(|scope| {
            let normalized_scope = normalize_matcher_text(scope.as_str());
            normalized_scope
                .strip_prefix("the ")
                .unwrap_or(&normalized_scope)
                .to_string()
        })
        .filter(|scope| !scope.is_empty())
    else {
        return false;
    };
    if course_code_in_scope_pattern().is_match(&mentioned_scope) {
        return false;
    }
    let owner_scope_acronym = normalized_owner_title
        .split_whitespace()
        .filter(|token| {
            token.len() > 1
                && !matches!(
                    *token,
                    "ba" | "bs" | "baba" | "degree" | "major" | "minor" | "program"
                )
                && !single_degree_letter_pattern().is_match(token)
        })
        .filter_map(|token| token.chars().next())
        .collect::<String>();
    if !owner_scope_acronym.is_empty() && mentioned_scope == owner_scope_acronym {
        return false;
    }
    !normalized_owner_title.contains(&mentioned_scope)
        && !mentioned_scope.contains(&normalized_owner_title)
}

fn cross_major_scope_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:if|for|required for)\s+([a-z][a-z&/,\- ]+?)\s+major\b",
        ))
        .expect("cross-major scope pattern")
    })
}

fn course_code_in_scope_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\b[a-z&]+(?:\s+[a-z&]+)*\s+\d{3}[a-z]?\b",
        ))
        .expect("course code in major scope pattern")
    })
}

fn single_degree_letter_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"^[abms]$",
        ))
        .expect("single degree letter pattern")
    })
}

pub fn line_references_conflicting_degree_route(
    entry: &Value,
    lines: &[String],
    hint_line_index: isize,
) -> bool {
    if hint_line_index < 0 || hint_line_index as usize >= lines.len() {
        return false;
    }
    let Some(route) = owner_degree_route(entry) else {
        return false;
    };
    let index = hint_line_index as usize;
    let nearby_lines = lines[index.saturating_sub(4)..=index]
        .iter()
        .map(|line| normalize_whitespace(line))
        .collect::<Vec<_>>();
    if matching_degree_route_pattern(route)
        .is_match(nearby_lines.last().map(String::as_str).unwrap_or_default())
    {
        return false;
    }
    nearby_lines
        .iter()
        .any(|line| conflicting_degree_route_pattern(route).is_match(line))
}

pub fn line_belongs_to_degree_route_comparison_section(
    lines: &[String],
    hint_line_index: isize,
) -> bool {
    if hint_line_index < 0 || hint_line_index as usize >= lines.len() {
        return false;
    }
    let index = hint_line_index as usize;
    let previous_lines = &lines[index.saturating_sub(25)..=index];
    let Some(comparison_start_index) = previous_lines
        .iter()
        .rposition(|line| degree_route_comparison_heading_pattern().is_match(line))
    else {
        return false;
    };
    !previous_lines[comparison_start_index + 1..]
        .iter()
        .any(|line| {
            degree_route_comparison_terminator_pattern().is_match(&normalize_whitespace(line))
        })
}

fn owner_degree_route(entry: &Value) -> Option<&'static str> {
    let searchable = normalize_whitespace(
        &[
            string_field(entry, "ownerTitle"),
            string_field(entry, "title"),
            string_field(entry, "planId"),
            string_field(entry, "ownerId"),
        ]
        .join(" "),
    );
    if bachelor_arts_route_pattern().is_match(&searchable) {
        Some("ba")
    } else if bachelor_science_route_pattern().is_match(&searchable) {
        Some("bs")
    } else {
        None
    }
}

fn bachelor_arts_route_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:b\.?\s*a\.?|bachelor\s+of\s+arts)\b|\(ba\)|(?:^|[-_\s])ba(?:[-_\s]|$)",
        ))
        .expect("bachelor of arts route pattern")
    })
}

fn bachelor_science_route_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:b\.?\s*s\.?|bachelor\s+of\s+science)\b|\(bs\)|(?:^|[-_\s])bs(?:[-_\s]|$)",
        ))
        .expect("bachelor of science route pattern")
    })
}

fn conflicting_degree_route_pattern(route: &str) -> &'static Regex {
    static BA: OnceLock<Regex> = OnceLock::new();
    static BS: OnceLock<Regex> = OnceLock::new();
    if route == "ba" {
        BA.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:b\.?\s*s\.?|bachelor\s+of\s+science)\s+majors?\b|\b(?:b\.?\s*s\.?|bachelor\s+of\s+science)\b.{0,80}\badditional\s+(?:class\s+)?requirements?\b")).expect("conflicting bachelor of science route pattern"))
    } else {
        BS.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:b\.?\s*a\.?|bachelor\s+of\s+arts)\s+majors?\b|\b(?:b\.?\s*a\.?|bachelor\s+of\s+arts)\b.{0,80}\badditional\s+(?:class\s+)?requirements?\b")).expect("conflicting bachelor of arts route pattern"))
    }
}

fn matching_degree_route_pattern(route: &str) -> &'static Regex {
    static BA: OnceLock<Regex> = OnceLock::new();
    static BS: OnceLock<Regex> = OnceLock::new();
    if route == "ba" {
        BA.get_or_init(|| {
            Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
                r"(?i)\b(?:b\.?\s*a\.?|bachelor\s+of\s+arts)\b",
            ))
            .expect("matching bachelor of arts route pattern")
        })
    } else {
        BS.get_or_init(|| {
            Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
                r"(?i)\b(?:b\.?\s*s\.?|bachelor\s+of\s+science)\b",
            ))
            .expect("matching bachelor of science route pattern")
        })
    }
}

fn degree_route_comparison_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bbachelors?\s+of\s+science\s+or\s+arts?\b",
        ))
        .expect("degree route comparison heading pattern")
    })
}

fn degree_route_comparison_terminator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\blearn\s+more\s+about\s+b\.?\s*s\.?\s+in\b|^(?:b\.?\s*[as]\.?(?:\s+in\s+.{1,80})?\s+requirements?|(?:degree|major|completion)\s+requirements?)\b")).expect("degree route comparison terminator pattern"))
}

pub fn is_graduate_or_applied_masters_requirement_context(
    owner: &Value,
    snapshot_lines: &[String],
    index: usize,
) -> bool {
    let owner_source_url = owner
        .get("sourceUrl")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("primarySourceUrl"));
    let source_context = [
        "sourceUrl",
        "primarySourceUrl",
        "sourceLabel",
        "primarySourceLabel",
    ]
    .iter()
    .filter_map(|key| owner.get(*key).filter(|value| js_truthy(Some(*value))))
    .map(js_string_value)
    .collect::<Vec<_>>()
    .join(" ");
    if graduate_supplemental_pattern().is_match(&source_context)
        && !undergraduate_or_bachelor_context_pattern().is_match(&source_context)
    {
        return true;
    }
    if snapshot_lines.is_empty() {
        return false;
    }
    let context_index = index.min(snapshot_lines.len() - 1);
    let local_start = context_index.saturating_sub(24);
    let local_end = (context_index + 8).min(snapshot_lines.len());
    let local_lines = snapshot_lines[local_start..local_end]
        .iter()
        .map(|line| {
            normalize_whitespace(&super::requirement_choice_lists::strip_choice_list_line(
                line,
            ))
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    for line in local_lines.iter().rev() {
        if supplemental_official_source_heading_pattern().is_match(line)
            || title_snapshot_header_pattern().is_match(line)
        {
            return graduate_supplemental_pattern().is_match(line)
                && !undergraduate_bachelor_or_bs_context_pattern().is_match(line);
        }
    }
    let prior_context_lines = snapshot_lines[local_start..=context_index]
        .iter()
        .map(|line| {
            normalize_whitespace(&super::requirement_choice_lists::strip_choice_list_line(
                line,
            ))
        })
        .filter(|line| !line.is_empty() && !snapshot_header_metadata_pattern().is_match(line))
        .collect::<Vec<_>>();
    for line in prior_context_lines.iter().rev() {
        if is_graduate_career_planning_note(line) {
            continue;
        }
        if is_undergraduate_catalog_requirement_reset_line(line)
            || is_undergraduate_catalog_section_reset_line(line)
            || has_undergraduate_requirement_reset_cue(line)
            || is_tacoma_rn_bsn_sample_program_plan_reset_line(owner_source_url, line)
        {
            break;
        }
        if graduate_degree_context_pattern().is_match(line)
            && !undergraduate_bachelor_or_bs_context_pattern().is_match(line)
        {
            return true;
        }
        if undergraduate_bachelor_or_bs_context_pattern().is_match(line) {
            break;
        }
    }
    let local_content_lines = local_lines
        .iter()
        .filter(|line| !snapshot_header_metadata_pattern().is_match(line.as_str()))
        .collect::<Vec<_>>();
    let has_graduate_context = local_content_lines.iter().any(|line| {
        graduate_degree_context_pattern().is_match(line) && !is_graduate_career_planning_note(line)
    });
    has_graduate_context
        && !local_content_lines
            .iter()
            .any(|line| undergraduate_bachelor_or_bs_context_pattern().is_match(line))
        && !local_content_lines.iter().any(|line| {
            is_undergraduate_catalog_requirement_reset_line(line)
                || is_undergraduate_catalog_section_reset_line(line)
                || has_undergraduate_requirement_reset_cue(line)
                || is_tacoma_rn_bsn_sample_program_plan_reset_line(owner_source_url, line)
        })
}

pub fn is_graduate_career_planning_note(line: &str) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if graduate_career_pursuit_pattern().is_match(&normalized_line)
        && career_context_pattern().is_match(&normalized_line)
        && !graduate_program_requirement_exclusion_pattern().is_match(&normalized_line)
    {
        return true;
    }
    if graduate_school_career_pattern().is_match(&normalized_line)
        && !graduate_program_requirement_exclusion_pattern().is_match(&normalized_line)
    {
        return true;
    }
    graduate_work_career_pattern().is_match(&normalized_line)
        && !graduate_degree_requirement_exclusion_pattern().is_match(&normalized_line)
}

pub fn is_graduate_career_planning_continuation_line(
    snapshot_lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let raw_line = snapshot_lines
        .get(index)
        .map(|line| {
            normalize_whitespace(&super::requirement_choice_lists::strip_choice_list_line(
                line,
            ))
        })
        .unwrap_or_default();
    if raw_line.is_empty() {
        return false;
    }
    if is_graduate_career_planning_note(&raw_line) {
        return true;
    }
    let previous_context_line = snapshot_lines
        .iter()
        .take(index)
        .skip(index.saturating_sub(3))
        .rev()
        .map(|line| {
            normalize_whitespace(&super::requirement_choice_lists::strip_choice_list_line(
                line,
            ))
        })
        .find(|line| !line.is_empty() && !note_prefix_pattern().is_match(line));
    let Some(previous_context_line) = previous_context_line else {
        return false;
    };
    if !is_graduate_career_planning_note(&previous_context_line) {
        return false;
    }
    !course_parser.extract_from_line(&raw_line).is_empty()
        && (graduate_course_continuation_prefix_pattern().is_match(&raw_line)
            || graduate_course_or_continuation_pattern().is_match(&raw_line))
        && !graduate_subject_heading_exclusion_pattern().is_match(&raw_line)
}

fn graduate_career_pursuit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:may|can|could|often|typically)?\s*(?:pursu(?:e|ing)|seek|continue|go\s+on)\b.{0,180}\b(?:graduate|master(?:'s|s)?|m\.?\s*a\.?|m\.?\s*s\.?|ph\.?\s*d\.?)\b")).expect("graduate career pursuit pattern"))
}
fn career_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:careers?|employment|jobs?|positions?|professional|work)\b",
        ))
        .expect("career context pattern")
    })
}
fn graduate_program_requirement_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:graduate\s+(?:programs?|admissions?|requirements?|curriculum|coursework)|master(?:'s|s)?\s+program|degree\s+requirements?)\b")).expect("graduate program requirement exclusion pattern"))
}
fn graduate_school_career_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:plan(?:s)?\s+to\s+pursue|pursu(?:e|ing)|prepar(?:e|es|ing)?\s+for|good\s+choice\s+for|careers?)\b.{0,160}\b(?:graduate\s+degree|graduate\s+school|professional\s+school|post[-\s]?graduate\s+work)\b")).expect("graduate school career pattern"))
}
fn graduate_work_career_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:graduate work|graduate school|professional school|post[-\s]?graduate work)\b")).expect("graduate work career pattern"))
}
fn graduate_degree_requirement_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:graduate\s+(?:program|degree|admission|requirements?)|master|m\.?\s*s\.?|m\.?\s*a\.?|ph\.?\s*d\.?|doctor)\b")).expect("graduate degree requirement exclusion pattern"))
}
fn note_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:please\s+note|note)[:\s]",
        ))
        .expect("note prefix pattern")
    })
}
fn graduate_course_continuation_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:\(?\s*\d+(?:\.\d+)?\s*\)|(?:or|-OR-)\b)",
        ))
        .expect("graduate course continuation prefix pattern")
    })
}
fn graduate_course_or_continuation_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:or|-OR-)\b",
        ))
        .expect("graduate course or continuation pattern")
    })
}
fn graduate_subject_heading_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:statistics|physics|biology|chemistry|mathematics|supporting requirements)\b",
        ))
        .expect("graduate subject heading exclusion pattern")
    })
}

pub fn normalize_matcher_text(value: &str) -> String {
    let mut output = String::new();
    let mut pending_separator = false;
    for character in value.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !output.is_empty() {
                output.push(' ');
            }
            pending_separator = false;
            output.push(character);
        } else {
            pending_separator = !output.is_empty();
        }
    }
    output
}

pub fn normalize_catalog_credential_matcher_title(value: &str) -> String {
    let mut title = normalize_transfer_planner_text(value);
    title = catalog_credential_degree_parenthetical_pattern()
        .replace(&title, "")
        .into_owned();
    title = catalog_credential_degree_suffix_pattern()
        .replace(&title, "")
        .into_owned();
    normalize_matcher_text(&title)
}

pub fn catalog_owner_scope_matcher_titles(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for title in catalog_owner_scope_titles(entry, primary_major_titles) {
        for candidate in [
            normalize_matcher_text(&title),
            normalize_catalog_credential_matcher_title(&title),
        ] {
            if !candidate.is_empty() && seen.insert(candidate.clone()) {
                normalized.push(candidate);
            }
        }
    }
    normalized
}

fn catalog_owner_scope_titles(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut titles = Vec::new();
    for title in [
        primary_major_title(entry, primary_major_titles),
        string_field(entry, "ownerTitle"),
        if string_field(entry, "planId") == "uw-seattle-european-studies" {
            "International Studies: Europe".to_string()
        } else {
            String::new()
        },
    ] {
        let title = normalize_transfer_planner_text(&title);
        if !title.is_empty() && seen.insert(title.clone()) {
            titles.push(title);
        }
    }
    titles
}

pub fn catalog_owner_title_has_specialized_credential(entry: &Value) -> bool {
    let owner_title = normalize_transfer_planner_text(&string_field(entry, "ownerTitle"));
    catalog_specialized_credential_pattern().is_match(&owner_title)
}

pub fn legacy_catalog_parent_program_matcher_titles(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    if !catalog_owner_title_has_specialized_credential(entry) {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut parents = Vec::new();
    for title in catalog_owner_scope_titles(entry, primary_major_titles) {
        let normalized_title = normalize_transfer_planner_text(&title);
        let parent_title = normalized_title
            .split_once(':')
            .map(|(parent, _)| parent)
            .unwrap_or(&normalized_title);
        let parent_title = specialized_credential_parenthetical_pattern()
            .replace(parent_title, "")
            .into_owned();
        let parent_title = parent_title.trim_matches(super::requirement_shape::is_js_whitespace);
        let normalized_parent = normalize_catalog_credential_matcher_title(parent_title);
        let normalized_exact = normalize_catalog_credential_matcher_title(&normalized_title);
        if !normalized_parent.is_empty()
            && normalized_parent != normalized_exact
            && seen.insert(normalized_parent.clone())
        {
            parents.push(normalized_parent);
        }
    }
    parents
}

pub fn text_matches_catalog_owner_scope_title(
    entry: &Value,
    text: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_text = normalize_matcher_text(text);
    !normalized_text.is_empty()
        && catalog_owner_scope_matcher_titles(entry, primary_major_titles)
            .iter()
            .any(|title| normalized_text.contains(title))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogCredentialSection {
    pub section_heading: String,
    pub lines: Vec<String>,
    pub headings: Vec<String>,
    pub stop_boundary: String,
}

pub fn should_prefer_base_catalog_credential_section(entry: &Value) -> bool {
    entry.get("ownerType").and_then(Value::as_str) == Some("major")
        && !js_truthy(entry.get("pathwayId"))
}

pub fn is_likely_base_catalog_credential_heading(
    entry: &Value,
    heading: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_heading = normalize_transfer_planner_text(heading);
    let normalized_credential_title = normalize_catalog_credential_matcher_title(
        &super::requirement_html::normalize_catalog_base_credential_heading(&normalized_heading),
    );
    let normalized_owner_titles = catalog_owner_scope_matcher_titles(entry, primary_major_titles);
    let credential_matches_known_owner_title = !normalized_credential_title.is_empty()
        && normalized_owner_titles
            .iter()
            .any(|title| title == &normalized_credential_title);
    if catalog_owner_title_has_specialized_credential(entry)
        && !credential_matches_known_owner_title
    {
        return false;
    }
    if normalized_heading.is_empty()
        || (normalized_heading.contains(':') && !credential_matches_known_owner_title)
        || non_base_catalog_credential_heading_pattern().is_match(&normalized_heading)
    {
        return false;
    }
    if credential_matches_known_owner_title {
        return true;
    }

    let primary_owner_credential_title = normalize_catalog_credential_matcher_title(
        &primary_major_title(entry, primary_major_titles),
    );
    if !catalog_owner_title_has_specialized_credential(entry)
        && !primary_owner_credential_title.is_empty()
        && normalized_credential_title.contains(&primary_owner_credential_title)
        && normalized_credential_title != primary_owner_credential_title
    {
        return false;
    }
    catalog_section_matches_selected_major(
        entry,
        &normalized_heading,
        &[normalized_heading.clone()],
        primary_major_titles,
    )
}

pub fn catalog_section_matches_selected_major(
    entry: &Value,
    section_heading: &str,
    section_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let mut scoped_lines = vec![section_heading.to_string()];
    scoped_lines.extend(section_lines.iter().take(20).cloned());
    let scoped_text = normalize_matcher_text(&scoped_lines.join(" "));
    let primary_owner_title = normalize_catalog_credential_matcher_title(&primary_major_title(
        entry,
        primary_major_titles,
    ));
    let normalized_section_heading_title = normalize_catalog_credential_matcher_title(
        &catalog_program_of_study_prefix_pattern()
            .replace(&normalize_transfer_planner_text(section_heading), "")
            .into_owned(),
    );
    if !catalog_owner_title_has_specialized_credential(entry)
        && !primary_owner_title.is_empty()
        && primary_owner_title.split_whitespace().count() == 1
        && normalized_section_heading_title.contains(&primary_owner_title)
        && normalized_section_heading_title != primary_owner_title
    {
        return false;
    }
    if text_matches_catalog_owner_scope_title(entry, &scoped_text, primary_major_titles) {
        return true;
    }
    let title_tokens = parser_recovery_title_scope_tokens(entry);
    let token_matches = title_tokens
        .iter()
        .filter(|token| scoped_text.contains(token.as_str()))
        .count();
    token_matches >= 2.min(title_tokens.len()).max(1) && !title_tokens.is_empty()
}

pub fn legacy_catalog_program_line_matches_owner(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_line = normalize_matcher_text(line);
    let normalized_whitespace_line = normalize_whitespace(line);
    if !legacy_catalog_program_major_prefix_pattern().is_match(&normalized_whitespace_line)
        || normalized_line.is_empty()
    {
        return false;
    }

    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let primary_owner_title = normalize_catalog_credential_matcher_title(&primary_major_title(
        entry,
        primary_major_titles,
    ));
    let program_major_title = normalize_catalog_credential_matcher_title(
        &catalog_program_of_study_prefix_pattern()
            .replace(
                &normalize_transfer_planner_text(&normalized_whitespace_line),
                "",
            )
            .into_owned(),
    );
    if !catalog_owner_title_has_specialized_credential(entry)
        && !primary_owner_title.is_empty()
        && primary_owner_title.split_whitespace().count() == 1
        && program_major_title.contains(&primary_owner_title)
        && program_major_title != primary_owner_title
    {
        return false;
    }
    if !exact_title.is_empty() && normalized_line.contains(&exact_title) {
        return true;
    }
    if text_matches_catalog_owner_scope_title(entry, &normalized_line, primary_major_titles) {
        return true;
    }
    if legacy_catalog_parent_program_matcher_titles(entry, primary_major_titles)
        .iter()
        .any(|title| normalized_line.contains(title))
    {
        return true;
    }

    let title_tokens = parser_recovery_title_scope_tokens(entry);
    if title_tokens.is_empty() {
        return false;
    }
    let owner_acronym_tokens =
        parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles);
    let comparable_title_tokens = title_tokens
        .iter()
        .filter(|token| !owner_acronym_tokens.contains(*token))
        .collect::<Vec<_>>();
    let required_title_tokens = if comparable_title_tokens.is_empty() {
        title_tokens.iter().collect::<Vec<_>>()
    } else {
        comparable_title_tokens
    };
    required_title_tokens
        .iter()
        .all(|token| normalized_line.contains(token.as_str()))
}

fn should_stop_major_legacy_catalog_scope_before_credentials(
    entry: &Value,
    plan_has_pathways: bool,
) -> bool {
    entry.get("ownerType").and_then(Value::as_str) == Some("major")
        && !js_truthy(entry.get("pathwayId"))
        && !catalog_owner_title_has_specialized_credential(entry)
        && plan_has_pathways
}

fn find_legacy_catalog_base_credential_line_range(
    entry: &Value,
    lines: &[String],
    start_index: usize,
    section_end_index: usize,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize)> {
    if !should_prefer_base_catalog_credential_section(entry) {
        return None;
    }
    let minimum_course_count = if catalog_owner_title_has_specialized_credential(entry) {
        1
    } else {
        2
    };
    for index in
        start_index.saturating_add(1)..=section_end_index.min(lines.len().saturating_sub(1))
    {
        let line = normalize_whitespace(&lines[index]);
        if !is_legacy_catalog_credential_heading_line(&line)
            || !line_looks_like_legacy_catalog_credential_section_start(lines, index)
            || !is_likely_base_catalog_credential_heading(entry, &line, primary_major_titles)
        {
            continue;
        }
        let mut end_index = section_end_index.min(lines.len().saturating_sub(1));
        for next_index in index + 1..=end_index {
            let next_line = normalize_whitespace(&lines[next_index]);
            if next_line.is_empty() {
                continue;
            }
            if back_to_top_heading_pattern().is_match(&next_line)
                || owner_program_of_study_heading_pattern().is_match(&next_line)
                || (is_legacy_catalog_credential_heading_line(&next_line)
                    && line_looks_like_legacy_catalog_credential_section_start(lines, next_index))
            {
                end_index = next_index.saturating_sub(1).max(index);
                break;
            }
        }
        let course_count = lines[index..=end_index]
            .iter()
            .flat_map(|section_line| course_parser.extract_from_line(section_line))
            .collect::<BTreeSet<_>>()
            .len();
        if course_count >= minimum_course_count {
            return Some((index, end_index));
        }
    }
    None
}

fn legacy_catalog_program_range_has_actionable_precredential_requirements(
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> bool {
    if lines.is_empty() {
        return false;
    }
    let course_count = lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .len();
    course_count >= 2
        && lines.iter().any(|line| {
            actionable_precredential_requirement_pattern().is_match(&normalize_whitespace(line))
        })
}

pub fn find_legacy_catalog_program_section_range(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    plan_has_pathways: bool,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize)> {
    if !is_legacy_student_catalog_source(entry) || lines.len() <= 20 {
        return None;
    }
    let specialized_credential = catalog_owner_title_has_specialized_credential(entry);
    let minimum_course_count = if specialized_credential { 1 } else { 2 };
    let requires_base_credential_range =
        should_stop_major_legacy_catalog_scope_before_credentials(entry, plan_has_pathways)
            && !specialized_credential;
    let mut candidate_ranges = Vec::new();
    for start_index in 0..lines.len() {
        let line = normalize_whitespace(&lines[start_index]);
        let program_line_matches_owner =
            legacy_catalog_program_line_matches_owner(entry, &line, primary_major_titles);
        if !program_line_matches_owner && !owner_program_of_study_heading_pattern().is_match(&line)
        {
            continue;
        }
        let mut end_index = lines.len().saturating_sub(1).min(start_index + 220);
        for index in start_index + 1..lines.len() {
            let next_line = normalize_whitespace(&lines[index]);
            if next_line.is_empty() {
                continue;
            }
            if back_to_top_heading_pattern().is_match(&next_line) && !specialized_credential {
                end_index = index.saturating_sub(1).max(start_index);
                break;
            }
            if requires_base_credential_range
                && is_legacy_catalog_credential_heading_line(&next_line)
                && line_looks_like_legacy_catalog_credential_section_start(lines, index)
            {
                if is_likely_base_catalog_credential_heading(
                    entry,
                    &next_line,
                    primary_major_titles,
                ) {
                    continue;
                }
                end_index = index.saturating_sub(1).max(start_index);
                break;
            }
            if owner_program_of_study_heading_pattern().is_match(&next_line) {
                end_index = index.saturating_sub(1).max(start_index);
                break;
            }
        }
        let credential_range = find_legacy_catalog_base_credential_line_range(
            entry,
            lines,
            start_index,
            end_index,
            primary_major_titles,
            course_parser,
        );
        let program_section_lines = &lines[start_index..=end_index];
        let has_actionable_precredential_requirements = requires_base_credential_range
            && credential_range.is_none()
            && program_line_matches_owner
            && legacy_catalog_program_range_has_actionable_precredential_requirements(
                program_section_lines,
                course_parser,
            );
        if (!program_line_matches_owner && credential_range.is_none())
            || (specialized_credential && credential_range.is_none())
            || (requires_base_credential_range
                && credential_range.is_none()
                && !has_actionable_precredential_requirements)
        {
            continue;
        }
        let (selected_start, selected_end) = credential_range.unwrap_or((start_index, end_index));
        let section_lines = &lines[selected_start..=selected_end];
        let course_count = section_lines
            .iter()
            .flat_map(|section_line| course_parser.extract_from_line(section_line))
            .collect::<BTreeSet<_>>()
            .len();
        let requirement_signal_count = section_lines
            .iter()
            .filter(|section_line| {
                legacy_catalog_program_requirement_signal_pattern().is_match(section_line)
            })
            .count();
        let score = course_count as f64 * 5.0
            + requirement_signal_count as f64 * 3.0
            + (section_lines.len() as f64 / 12.0).min(10.0);
        candidate_ranges.push((selected_start, selected_end, course_count, score));
    }
    candidate_ranges.retain(|candidate| candidate.2 >= minimum_course_count);
    candidate_ranges.sort_by(|left, right| {
        right
            .3
            .total_cmp(&left.3)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    candidate_ranges
        .first()
        .map(|candidate| (candidate.0, candidate.1))
}

pub fn find_base_catalog_credential_section(
    entry: &Value,
    html: &str,
    search_start_index: usize,
    search_end_index: usize,
    primary_major_titles: &HashMap<String, String>,
) -> Option<CatalogCredentialSection> {
    if !should_prefer_base_catalog_credential_section(entry) {
        return None;
    }
    for captures in catalog_credential_heading_pattern().captures_iter(html) {
        let Some(whole) = captures.get(0) else {
            continue;
        };
        if whole.start() < search_start_index {
            continue;
        }
        if whole.start() >= search_end_index {
            break;
        }
        let section_heading = captures
            .get(1)
            .map(|heading| super::requirement_html::strip_html(heading.as_str()))
            .unwrap_or_default();
        if !is_likely_base_catalog_credential_heading(entry, &section_heading, primary_major_titles)
        {
            continue;
        }
        let peer_boundary = super::requirement_html::find_next_catalog_peer_boundary(
            html,
            whole.end(),
            Some("credential-derived"),
        );
        let stop_index = peer_boundary.index.min(search_end_index);
        let section_html = html.get(whole.start()..stop_index)?;
        return Some(CatalogCredentialSection {
            section_heading,
            lines: super::requirement_html::build_html_lines(section_html),
            headings: super::requirement_html::extract_headings(section_html),
            stop_boundary: if peer_boundary.index <= search_end_index {
                peer_boundary.label
            } else {
                "end of matched major program".to_string()
            },
        });
    }
    None
}

fn catalog_credential_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r#"(?is)<h[3-5]\b[^>]*\bid=(?:"credential-[^"']+"|'credential-[^"']+')[^>]*>(.*?)</h[3-5]>"#,
        );
        Regex::new(&pattern).expect("catalog credential heading pattern")
    })
}

fn non_base_catalog_credential_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:minor|master|doctor|graduate|certificate)\b",
        );
        Regex::new(&pattern).expect("non-base catalog credential heading pattern")
    })
}

fn legacy_catalog_program_major_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Program of Study:\s*Major:",
        );
        Regex::new(&pattern).expect("legacy catalog program major prefix pattern")
    })
}

fn catalog_program_of_study_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Program\s+of\s+Study:\s*(?:Major|Minor):\s*",
        ))
        .expect("catalog program of study prefix pattern")
    })
}

fn catalog_credential_degree_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\s*\((?:B\.?\s*A\.?|B\.?\s*S\.?|BABA|BSN)\)\s*$",
        );
        Regex::new(&pattern).expect("catalog credential degree parenthetical pattern")
    })
}

fn catalog_credential_degree_suffix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\s+\b(?:B\.?\s*A\.?|B\.?\s*S\.?|BABA|BSN)\b\s*$",
        );
        Regex::new(&pattern).expect("catalog credential degree suffix pattern")
    })
}

fn specialized_credential_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\s*\([^)]*\b(?:designed|option|track|route|pathway|concentration)\b[^)]*\)\s*$",
        );
        Regex::new(&pattern).expect("specialized credential parenthetical pattern")
    })
}

fn catalog_specialized_credential_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i):\s*\S|\([^)]*\b(?:designed|option|track|route|pathway|concentration)\b[^)]*\)",
        );
        Regex::new(&pattern).expect("catalog specialized credential pattern")
    })
}

fn parser_recovery_generic_identity_tokens() -> BTreeSet<&'static str> {
    [
        "and",
        "bachelor",
        "business",
        "complete",
        "concentration",
        "courses",
        "current",
        "curriculum",
        "degree",
        "admission",
        "admissions",
        "major",
        "option",
        "pathway",
        "prerequisite",
        "prerequisites",
        "program",
        "requirements",
        "route",
        "school",
        "students",
        "track",
        "undergraduate",
    ]
    .into_iter()
    .collect()
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .filter(|value| !value.is_null())
        .map(|value| match value {
            Value::String(value) => value.clone(),
            Value::Number(value) => super::requirement_shape::js_number_to_string(value),
            Value::Bool(value) => value.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

fn owner_title_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\s+-\s+",
        ))
        .expect("owner title prefix")
    })
}

fn is_support_or_admission_link_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\bapproved\b.{0,60}\b(courses?|course list|list)\b|\b(courses?|course list|list)\b.{0,60}\bapproved\b|\b(?:engineering|technical|departmental|major|science|natural science|approved)?\s*electives?\b.{0,50}\b(courses?|list|options?|page)\b|\b(courses?|list|options?)\b.{0,50}\belectives?\b|/electives?(?:[-/]|$)|\b(?:admissions?|admission|apply|application)\b.{0,80}\bprereq(?:uisites?|uisite courses?)\b|\bprereq(?:uisites?|uisite courses?)\b.{0,80}\b(?:admissions?|admission|apply|application)\b|\b(advising|adviser|advisor|support sources?|student resources?|student support|forms?|petitions?|policies|policy[-\s]*(?:procedures?|resources?|forms?)|faq|frequently asked questions)\b"))
            .expect("support or admission link cue")
    })
}

fn find_prior_admit_curriculum_start_index(
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    if lines.len() < 12 {
        return None;
    }
    for (index, line) in lines.iter().enumerate() {
        if !prior_admit_curriculum_start_pattern().is_match(&normalize_whitespace(line)) {
            continue;
        }
        let following_lines = &lines[index..lines.len().min(index + 36)];
        let following_text = normalize_matcher_text(&following_lines.join(" "));
        let following_course_count = following_lines
            .iter()
            .flat_map(|line| course_parser.extract_from_line(line))
            .collect::<BTreeSet<_>>()
            .len();
        if following_course_count >= 3
            && major_requirements_pattern().is_match(&following_text)
            && prior_admit_curriculum_cue_pattern().is_match(&following_text)
        {
            return Some(index);
        }
    }
    None
}

fn is_prior_admit_curriculum_owner(entry: &Value) -> bool {
    let owner_text = normalize_whitespace(&format!(
        "{} {} {} {}",
        string_field(entry, "ownerId"),
        string_field(entry, "ownerTitle"),
        string_field(entry, "pathwayId"),
        string_field(entry, "label")
    ))
    .replace('_', " ")
    .replace(':', " ")
    .replace('-', " ")
    .to_lowercase();
    prior_admit_owner_era_pattern().is_match(&owner_text)
}

fn prior_admit_curriculum_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:for students admitted\b.*\b(?:before|prior to)\b|students admitted\b.*\b(?:before|prior to)\b|degree requirements?\b.*\bfor students admitted\b.*\b(?:before|prior to)\b)"))
            .expect("prior admit curriculum start")
    })
}

fn prior_admit_owner_era_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:pre|before|prior\s+to)\s+(?:spring|summer|autumn|fall|winter)\s+\d{4}\b",
        ))
        .expect("prior admit owner era")
    })
}

fn major_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bmajor requirements?\b",
        ))
        .expect("major requirements")
    })
}

fn prior_admit_curriculum_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:introductory|methods?|formal options?)\b",
        ))
        .expect("prior admit curriculum cue")
    })
}

fn formal_options_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bformal options?\b",
        ))
        .expect("formal options")
    })
}

fn shared_curriculum_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bshared curriculum\b",
        ))
        .expect("shared curriculum")
    })
}

fn beginning_spring_2026_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bbeginning in spring 2026\b",
        ))
        .expect("beginning spring 2026")
    })
}

fn spring_2026_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bspring 2026\b",
        ))
        .expect("spring 2026")
    })
}

fn foundation_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bfoundation courses?\b",
        ))
        .expect("foundation courses")
    })
}

fn methods_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bmethods courses?\b",
        ))
        .expect("methods courses")
    })
}

fn introductory_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bintroductory courses?\b",
        ))
        .expect("introductory courses")
    })
}

fn graduation_or_contact_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:graduation requirements?|contact)$",
        ))
        .expect("graduation or contact heading")
    })
}

fn departmental_degree_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^([A-Z][A-Z0-9&]{1,8})\s+Departmental\s+Degree\s+requirements?\b",
        ))
        .expect("departmental degree boundary")
    })
}

fn pathway_section_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:[A-Z][A-Za-z0-9_&'./-]+(?:\s+[A-Z][A-Za-z0-9_&'./-]+){0,8}\s+)?(?:Track|Option|Concentration|Pathway)(?:\s+(?:Degree|Major|Program|Curriculum|Requirements?|Courses?|Plan|Checklist))*$"))
            .expect("parser recovery pathway section boundary")
    })
}

fn html_section_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:Program of Study:|[A-Z][A-Z0-9&]{1,8}\s+Departmental\s+Degree\s+requirements?\b|Bachelor\b|Minor\b|Master\b|Doctor\b|Undergraduate Programs\b|Graduate Programs\b|Back to Top\b)"))
            .expect("parser recovery html section boundary")
    })
}

fn generic_section_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:Bachelor|Minor|Master|Doctor|Program of Study|Degree Requirements|Major Requirements|Admissions?|Application|Sample|Suggested Plan|Courses?)\b"))
            .expect("parser recovery generic section boundary")
    })
}

fn credential_program_path_segment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:bachelor|bachelors?|ba|bs|bba|major|minor)[-_]",
        ))
        .expect("credential program path segment")
    })
}

fn pathway_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(track|option|route|pathway|concentration|specialization)\b",
        ))
        .expect("pathway cue")
    })
}

fn sectioned_course_column_header_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:Course\s*#|Course|Courses|Course\s+Name|Topic|Credits?|Department|Dept\.?|TOTAL)$")).expect("sectioned course column header"))
}

fn sectioned_option_course_supplemental_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^\[Supplemental official source\]",
        ))
        .expect("sectioned course supplemental boundary")
    })
}

fn sectioned_option_course_service_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:B\.S\. degree requirements|Undergraduate advising|(?:[A-Z& ]+\s+)?Capstone|Tuition and scholarships|Contact\b|Log in|Privacy|Terms)$")).expect("sectioned course service boundary"))
}

fn sectioned_option_course_requirement_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:admission|requirements?)\b",
        ))
        .expect("sectioned course requirement boundary")
    })
}

fn sectioned_option_course_core_exception_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:core|electives?)\b",
        ))
        .expect("sectioned course core exception")
    })
}

fn sectioned_course_credit_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^\d+(?:\.\d+)?\s*credits?\s*:\s*",
        ))
        .expect("sectioned course credit prefix")
    })
}

fn sectioned_bracket_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^\[[^\]]+\]\s*",
        ))
        .expect("sectioned course bracket prefix")
    })
}

fn primary_requirement_section_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:graduation requirements?|degree requirements?|major requirements?|completion requirements?|core requirements?|elective requirements?|general education requirements?|required courses?|fundamentals|mathematics|natural sciences?|physics|chemistry|biology|calculus|composition|data and society|core and electives|areas of inquiry|engineering fundamentals|mathematics\s*&\s*natural sciences|written\s*&\s*oral communication)\b|^requirements?$|^major in\b|^course #$|\b(?:concentration|option|track|pathway|route)\b$")).expect("primary requirement section cue"))
}

fn sectioned_course_season_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:Autumn|Fall|Winter|Spring|Summer)(?:\s*&\s*(?:Autumn|Fall|Winter|Spring|Summer))*$")).expect("sectioned course season heading"))
}

fn course_level_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b\d{3}[-\s]*level\b",
        ))
        .expect("course level heading")
    })
}

fn sectioned_course_required_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:choose|select)\s+(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\b|^(?:one|two|three|four|five|six|seven|eight|nine|ten)(?:\s*\(\s*\d+\s*\))?\s+.+?\b(?:courses?|electives?)\b|^\d+\s+.+?\belectives?\b")).expect("sectioned course required count"))
}

fn sectioned_course_list_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:selected from|from the following list|both courses?|list of approved courses|approved list of courses|courses? listed below|listed below are required|following departments|technical electives?|engineering fundamentals electives?|science electives?|math elective|core and elective requirements?)\b")).expect("sectioned course list requirement"))
}

fn sectioned_course_heading_qualifier_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:credits?|courses?|electives?|minimum|maximum)\b",
        ))
        .expect("sectioned course heading qualifier")
    })
}

fn short_named_course_heading_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:advising|admission|apply|application|contact|scholarships?|tuition|sample|planning|overview|objectives?|outcomes?|learn by doing|recommended preparation)\b")).expect("short named course heading exclusion"))
}

fn sectioned_course_credit_range_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(\d+(?:\.\d+)?)(?:\s*(?:-|to)\s*(\d+(?:\.\d+)?))?$",
        ))
        .expect("sectioned course credit range")
    })
}

fn generic_degree_requirements_phrase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bdegree requirements\b",
        ))
        .expect("generic degree requirements phrase")
    })
}

fn concentration_courses_phrase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bconcentration courses\b",
        ))
        .expect("concentration courses phrase")
    })
}

fn concentration_courses_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bconcentration courses?:?$",
        ))
        .expect("concentration courses heading")
    })
}

fn pathway_curriculum_or_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:curriculum|requirements?)\b",
        ))
        .expect("pathway curriculum or requirements")
    })
}

fn back_to_top_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Back to Top\b",
        ))
        .expect("back to top boundary")
    })
}

fn concentration_pathway_id_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bconcentration\b",
        ))
        .expect("concentration pathway id")
    })
}

fn concentration_area_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bconcentration\s+area(?:\s+courses?)?\b",
        ))
        .expect("concentration area heading")
    })
}

fn pathway_choice_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:choose|select|formal|options?|tracks?|routes?|pathways?|concentrations?)\b",
        ))
        .expect("pathway choice context")
    })
}

fn bare_pathway_subheading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:admission|admissions|application|apply|capstone|checklist|core|course|courses|coursework|credits?|curriculum|degree|electives?|internship|list|major|minor|overview|prereq|prerequisite|program|requirements?|sample|schedule|series|student|students|worksheet)\b")).expect("bare pathway subheading"))
}

fn bare_pathway_filler_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:and|&|of|the)$",
        ))
        .expect("bare pathway filler word")
    })
}

fn bare_pathway_visible_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"^[A-Z0-9&][A-Za-z0-9&/()'-]*$",
        ))
        .expect("bare pathway visible word")
    })
}

fn formal_options_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^formal options?\b",
        ))
        .expect("formal options heading")
    })
}

fn formal_options_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bformal options?\b",
        ))
        .expect("formal options context")
    })
}

fn choose_one_formal_options_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:choose|select|declare)\s+one\b",
        ))
        .expect("choose one formal option")
    })
}

fn formal_option_enumerated_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"^[A-Z]\.\s+\S",
        ))
        .expect("formal option enumerated heading")
    })
}

fn formal_option_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:option|certificate|concentration|track|pathway)\b",
        ))
        .expect("formal option cue")
    })
}

fn formal_option_action_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:choose|select|declare|complete|credits?)\b",
        ))
        .expect("formal option action")
    })
}

fn formal_options_end_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:general electives?|graduation requirements?|admissions?|advising|contact\b|how to apply\b|degree requirements?|major requirements?)\b")).expect("formal options end boundary"))
}

fn course_list_section_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^list\s+[a-z0-9]\b",
        ))
        .expect("course list section heading")
    })
}

fn pathway_label_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(option|track|route|pathway|concentration)\b",
        ))
        .expect("pathway label cue")
    })
}

fn peer_pathway_prose_rejection_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:choose|select|following|formal)\b",
        ))
        .expect("peer pathway prose rejection")
    })
}

fn pathway_heading_enumerator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^[A-Z]\.\s+|^\(?\d+[).]\s+",
        ))
        .expect("pathway heading enumerator")
    })
}

fn pathway_html_table_content_end_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:general electives?|graduation requirements?|admission requirements?|curriculum|major requirements?|program overview|contact\b|for students admitted\b)")).expect("pathway HTML table content end boundary"))
}

fn base_scope_pathway_section_end_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:advisors?|advisor|major coordinator|department|career opportunities|admission requirements?|degree requirements?|contact)\b")).expect("base scope pathway section end boundary"))
}

fn noisy_inline_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$)))"))
            .expect("noisy inline requirement line")
    })
}

fn actionable_precredential_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:recommended preparation|admission requirements?|minimum course requirements?|prerequisites?)\b",
        );
        Regex::new(&pattern).expect("actionable pre-credential requirements")
    })
}

fn legacy_catalog_program_requirement_signal_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:admission requirements?|completion requirements?|graduation requirements?|required core|prerequisites?|electives?)\b",
        );
        Regex::new(&pattern).expect("legacy catalog program requirement signal")
    })
}

fn full_degree_requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:degree requirements?|total credits? for degree|major requirements?)\b",
        );
        Regex::new(&pattern).expect("focused full-degree requirement cue")
    })
}

fn focused_degree_requirement_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:core|fundamentals?|mathematics|natural science|programming|capstone|electives?|requirements?)\b",
        );
        Regex::new(&pattern).expect("focused full-degree section heading")
    })
}

fn compact_full_degree_course_list_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:degree requirements?|core courses?|core \([0-9]+\s*credits?|electives?|capstone)\b",
        );
        Regex::new(&pattern).expect("compact full-degree course list")
    })
}

fn generic_html_requirement_signal_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:requirements?|credits?|courses?|major|minor|curriculum|prerequisite|elective|option|track|pathway|concentration)\b",
        );
        Regex::new(&pattern).expect("generic HTML requirement signal")
    })
}

fn generic_html_table_signal_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:course #|course name|credits?|cr\.?|requirements?|electives?|concentrations?|offered|prereq|prerequisite)\b",
        );
        Regex::new(&pattern).expect("generic HTML table signal")
    })
}

fn elective_list_webpage_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\belective (?:lists?|webpage|courses?)\b",
        );
        Regex::new(&pattern).expect("elective list or webpage scope")
    })
}

fn approved_electives_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bapproved electives?\b",
        );
        Regex::new(&pattern).expect("approved electives scope")
    })
}

fn different_owner_degree_level_descriptor_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:minor|master|doctor|graduate)\b",
        );
        Regex::new(&pattern).expect("different owner degree descriptor")
    })
}

fn data_science_descriptor_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\bdata science\b");
        Regex::new(&pattern).expect("data science descriptor")
    })
}

fn owner_descriptor_suffix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i):\s*([a-z0-9][a-z0-9 -]{0,80})$",
        );
        Regex::new(&pattern).expect("owner descriptor suffix")
    })
}

fn owner_descriptor_kind_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:data science|track|option|route|pathway|concentration|specialization)\b",
        );
        Regex::new(&pattern).expect("owner descriptor kind")
    })
}

fn owner_program_of_study_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Program of Study:",
        ))
        .expect("owner program of study heading")
    })
}

fn back_to_top_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Back to Top\b",
        ))
        .expect("back to top heading")
    })
}

fn transfer_credit_noise_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(transfer credits?|transfer equivalenc(?:y|ies)|articulation agreement|community colleges?|community college)\b")).expect("transfer credit noise"))
}

fn source_line_course_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^[-*\x{2022}\x{F071}\s]*(?:\(\s*\d+(?:\.\d+)?\s*\)\s*)?(?:[A-Z&]+(?:\s+[A-Z&]+)?(?:\s*/\s*[A-Z&]+(?:\s+[A-Z&]+)?)?\s+\d{3}[A-Za-z]?\*?|\d{3}[A-Za-z]?\b)")).expect("source line course code prefix"))
}

fn inline_labeled_requirement_marker_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^\d+\s*[).]\s*|^[A-Z]\s*[).]\s*",
        ))
        .expect("inline requirement marker")
    })
}

fn trailing_label_punctuation_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"[:.]+$",
        ))
        .expect("trailing inline label punctuation")
    })
}

fn non_requirement_inline_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(admissions?|application|deadline|tuition|fees?|contact|career|graduate)\b",
        ))
        .expect("non-requirement inline label")
    })
}

fn preceding_inline_hard_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:admissions?|application|how do you major|contact|career)\b",
        ))
        .expect("preceding inline hard boundary")
    })
}

pub fn is_pathway_section_boundary_line(line: &str) -> bool {
    pathway_section_boundary_pattern_source().is_match(line)
}

fn pathway_section_boundary_pattern_source() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:admissions?|advising|student resources?|degree requirements?|core courses?|honou?rs?\b|recommended courses?|language courses?|career options?|share|support us|contact us|privacy|terms|site map)\b")).expect("pathway section boundary"))
}

fn structural_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(admission requirements|degree requirements|major requirements|completion requirements|required courses|elective courses|core courses|core requirements|curriculum|prerequisite courses|shared set of core courses|specialization|track|option|route|pathway|concentration|checklist|foundation|distribution requirement)\b")).expect("structural requirement"))
}

fn primary_requirement_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\bdegree requirements?\b|\bmajor requirements?\b|\bgraduation requirements?\b|\bprogram requirements?\b|\bdegree structure\b|\brequirements packet\b|\bdegreq\b")).expect("primary requirement line"))
}

fn section_noise_penalty_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(admission|apply|application|sample schedule|four[-\s]?year plan|graduate)\b",
        ))
        .expect("section noise penalty")
    })
}

fn legacy_catalog_credential_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Bachelor\b.+\bdegree\b.+\bmajor\b",
        ))
        .expect("legacy catalog credential heading")
    })
}

fn legacy_catalog_section_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\bCredential Overview\b|\bAdditional Admission Requirements\b|\bCompletion Requirements\b")).expect("legacy catalog section start"))
}

fn generic_pathway_hub_segment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:curriculum|overview|requirements?|degree-requirements?|major-requirements?|program-requirements?|options?|tracks?|routes?|pathways?|concentrations?|specializations?)$"))
            .expect("generic pathway hub segment")
    })
}

fn generic_pathway_hash_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:content|main|top|curriculum|overview|requirements?|degree requirements?|major requirements?|program requirements?|options?|tracks?|routes?|pathways?|concentrations?|specializations?)$"))
            .expect("generic pathway hash fragment")
    })
}

fn specific_pathway_segment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)(?:^|[-_])(?:option|track|route|pathway|concentration|specialization)(?:[-_]|$)",
        ))
        .expect("specific pathway segment")
    })
}

fn different_owner_acronym_departmental_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^([A-Z][A-Z0-9&]{1,8})\s+Departmental\s+Degree\s+requirements?\b",
        );
        Regex::new(&pattern).expect("different owner acronym departmental boundary")
    })
}

fn path_separator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"[-_]+",
        ))
        .expect("path separators")
    })
}

fn parser_recovery_title_scope_tokens(entry: &Value) -> Vec<String> {
    let value = format!(
        "{} {}",
        string_field(entry, "ownerTitle"),
        string_field(entry, "sourceLabel")
    );
    normalize_matcher_text(&value)
        .split_whitespace()
        .filter(|token| {
            token.len() >= 4 && !parser_recovery_title_scope_stopwords().contains(*token)
        })
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(8)
        .collect()
}

fn parser_recovery_title_scope_stopwords() -> BTreeSet<&'static str> {
    [
        "and",
        "arts",
        "bachelor",
        "bothell",
        "campus",
        "catalog",
        "course",
        "courses",
        "curriculum",
        "degree",
        "major",
        "of",
        "official",
        "overview",
        "page",
        "program",
        "requirements",
        "science",
        "seattle",
        "studies",
        "tacoma",
        "the",
        "university",
        "uw",
        "washington",
    ]
    .into_iter()
    .collect()
}

fn requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(required|requirements|prereq|prerequisite|complete|credits|credit|elective|select|choose|one of the following|two of the following|option|track|route|pathway|concentration)\b"))
            .expect("requirement cue")
    })
}

fn graduate_supplemental_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(graduate|ph\.?\s*d\.?|doctor(?:al|ate)?|m\.?\s*a\.?|masters?|master(?:'s|s)?)\b|/(?:ma|phd|graduate|masters?|amp)(?:[-/]|$)"))
            .expect("graduate supplemental source")
    })
}

fn graduate_degree_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:graduate\s+(?:programs?|admissions?|degree|student status|work)|master(?:'s|s)?\s+program|master\s+of|m\.?\s*s\.?|m\.?\s*a\.?|ph\.?\s*d\.?|doctor(?:al|ate)?|doctor\s+of|thesis/non-thesis|dissertation|general examination)\b"))
            .expect("graduate degree context")
    })
}

fn undergraduate_or_bachelor_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bundergrad(?:uate)?\b",
        ))
        .expect("undergraduate context")
    })
}

fn undergraduate_bachelor_or_bs_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bundergrad(?:uate)?|bachelor|b\.?\s*s\.?\b",
        ))
        .expect("undergraduate or bachelor context")
    })
}

fn supplemental_official_source_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^\[Supplemental official source\]",
        ))
        .expect("supplemental official source heading")
    })
}

fn title_snapshot_header_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Title:",
        ))
        .expect("title snapshot header")
    })
}

fn snapshot_header_metadata_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:owner|source|headings):\s*",
        ))
        .expect("snapshot metadata header")
    })
}

fn is_undergraduate_catalog_requirement_reset_line(line: &str) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    undergraduate_requirement_reset_heading_pattern().is_match(&normalized_line)
        && !graduate_degree_context_pattern().is_match(&normalized_line)
}

fn is_undergraduate_catalog_section_reset_line(line: &str) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    undergraduate_catalog_section_reset_pattern().is_match(&normalized_line)
        && !graduate_degree_context_pattern().is_match(&normalized_line)
}

fn has_undergraduate_requirement_reset_cue(line: &str) -> bool {
    undergraduate_requirement_reset_cue_pattern().is_match(&normalize_whitespace(line))
        && !graduate_degree_context_pattern().is_match(line)
}

fn is_tacoma_rn_bsn_sample_program_plan_reset_line(source_url: Option<&Value>, line: &str) -> bool {
    source_url.map_or(false, |source_url| {
        tacoma_rn_bsn_sample_program_plan_url_pattern().is_match(&js_string_value(source_url))
    }) && tacoma_rn_bsn_sample_program_plan_reset_pattern().is_match(&normalize_whitespace(line))
}

fn undergraduate_requirement_reset_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:additional\s+completion\s+requirements|option\s+specific\s+credits?)\b",
        ))
        .expect("undergraduate requirement reset heading")
    })
}

fn undergraduate_catalog_section_reset_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)^(?:undergraduate\s+programs?|program\s+of\s+study:\s+major\b|bachelor\s+of\b|bachelor\b.+\bmajor\b|b\.?\s*[as]\.?\s+in\b)")).expect("undergraduate catalog section reset"))
}

fn undergraduate_requirement_reset_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:curriculum|degree\s+options?|major\s+options?|courses?\s+for\s+graduation|core\s+courses?)\b")).expect("undergraduate requirement reset cue"))
}

fn tacoma_rn_bsn_sample_program_plan_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)//www\.tacoma\.uw\.edu/nursing/rn-bsn-sample-program-plans(?:$|[?#/])",
        ))
        .expect("Tacoma RN-BSN sample plan URL")
    })
}

fn tacoma_rn_bsn_sample_program_plan_reset_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:rn-bsn\s+sample\s+program\s+plans?|sample\s+program\s+plans?\s+for\s+(?:summer|autumn)\s+cohort)\b")).expect("Tacoma RN-BSN sample plan reset"))
}

fn selected_pathway_label_cue_pattern() -> &'static Regex {
    pathway_cue_pattern()
}

fn explicit_program_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\(([A-Z][A-Z0-9&\s/-]{1,12})\)",
        ))
        .expect("explicit program acronym")
    })
}

fn route_ell_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:ell|english language learners?)\b",
        ))
        .expect("ELL route alias")
    })
}

fn teaching_ell_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\bteaching english language learners?\b",
        ))
        .expect("TELL route alias")
    })
}

fn route_sped_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:sped|special education)\b",
        ))
        .expect("SPED route alias")
    })
}

fn candidate_option_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\b([A-Z][A-Z0-9&]{1,8})\s+Option\b",
        ))
        .expect("candidate pathway option acronym")
    })
}

fn pathway_letter_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^[A-Z]\.\s+",
        ))
        .expect("pathway letter prefix")
    })
}

fn pathway_numeric_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^\(?\d+[).]\s+",
        ))
        .expect("pathway numeric prefix")
    })
}

fn pathway_roman_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:[ivxlcdm]+)[).]\s+",
        ))
        .expect("pathway Roman numeral prefix")
    })
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn js_string_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
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
        Value::Object(_) => "[object Object]".into(),
    }
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

fn supplemental_official_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^\[Supplemental official source\]",
        ))
        .expect("supplemental official source label")
    })
}
