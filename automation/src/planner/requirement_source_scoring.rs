use super::requirement_course_codes::{CourseCodeParser, normalize_course_code};
use super::requirement_document_facts::RequirementSourceTextFacts;
use super::requirement_manifest_fallback::get_source_role_score;
use super::requirement_recovery_links::{
    normalize_requirement_matcher_text, parser_recovery_title_scope_tokens,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;
use url::Url;

pub fn has_meaningful_parsed_content(facts: &RequirementSourceTextFacts) -> bool {
    !facts.candidate_course_codes.is_empty()
        || !facts.requirement_cue_lines.is_empty()
        || !facts.choose_statements.is_empty()
        || !facts.pathway_labels.is_empty()
}

pub fn get_parsed_owner_alignment_score(
    entry: &Value,
    facts: &RequirementSourceTextFacts,
) -> usize {
    let exact_title = entry
        .get("ownerTitle")
        .and_then(Value::as_str)
        .map(normalize_requirement_matcher_text)
        .unwrap_or_default();
    let sampled_text = normalize_requirement_matcher_text(
        &std::iter::once(facts.title.as_deref().unwrap_or_default())
            .chain(facts.headings.iter().take(8).map(String::as_str))
            .chain(facts.snapshot_lines.iter().take(12).map(String::as_str))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    );
    if !exact_title.is_empty() && sampled_text.contains(&exact_title) {
        return 10;
    }
    let tokens = parser_recovery_title_scope_tokens(entry);
    let token_matches = tokens
        .iter()
        .filter(|token| sampled_text.contains(token.as_str()))
        .count();
    let owner_searchable = ["ownerTitle", "label", "sourceLabel"]
        .into_iter()
        .filter_map(|field| entry.get(field).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let owner_searchable = normalize_requirement_matcher_text(&owner_searchable);
    let is_base_route = base_route_pattern().is_match(&owner_searchable);
    let base_route_alias_tokens = tokens
        .iter()
        .filter(|token| {
            !matches!(
                token.as_str(),
                "base" | "default" | "option" | "pathway" | "route" | "standard"
            )
        })
        .collect::<Vec<_>>();
    if token_matches < 2
        && is_base_route
        && !base_route_alias_tokens.is_empty()
        && base_route_alias_tokens
            .iter()
            .all(|token| sampled_text.contains(token.as_str()))
        && bachelor_major_pattern().is_match(&sampled_text)
    {
        return 2;
    }
    token_matches
}

pub fn get_parsed_degree_sheet_signal_score(facts: &RequirementSourceTextFacts) -> usize {
    let sampled_text = normalize_requirement_matcher_text(
        &facts
            .snapshot_lines
            .iter()
            .take(8)
            .chain(facts.requirement_cue_lines.iter().take(12))
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    );
    [
        (degree_sheet_before_applying_pattern(), 4),
        (degree_sheet_prerequisite_key_pattern(), 4),
        (degree_sheet_areas_of_inquiry_pattern(), 2),
        (degree_sheet_math_science_pattern(), 2),
        (degree_sheet_fundamentals_pattern(), 2),
        (degree_sheet_core_electives_pattern(), 2),
        (degree_sheet_additional_requirements_pattern(), 2),
    ]
    .into_iter()
    .filter_map(|(pattern, score)| pattern.is_match(&sampled_text).then_some(score))
    .sum()
}

pub fn should_keep_linked_supplemental_html_source(
    entry: &Value,
    base_facts: &RequirementSourceTextFacts,
    supplemental_entry: &Value,
    candidate: &Value,
    parsed_facts: &RequirementSourceTextFacts,
    course_parser: &CourseCodeParser,
) -> bool {
    if json_truthy(candidate.get("historical")) {
        return false;
    }
    if json_truthy(entry.get("pathwayId")) {
        return candidate
            .get("sameProgramRequirementLink")
            .and_then(Value::as_bool)
            == Some(true)
            && supplemental_html_candidate_has_selected_pathway_phrase(
                entry,
                candidate,
                parsed_facts,
            );
    }
    let base_snapshot_text = normalize_requirement_matcher_text(
        &base_facts
            .snapshot_lines
            .iter()
            .take(90)
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    );
    let base_snapshot_course_count = base_facts
        .snapshot_lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .len();
    let has_primary_requirement_cue = primary_required_courses_pattern()
        .is_match(&base_snapshot_text)
        || must_complete_courses_pattern().is_match(&base_snapshot_text)
        || curriculum_cue_pattern().is_match(&base_snapshot_text);
    let has_strong_primary_requirement_section = has_primary_requirement_cue
        && lower_upper_core_course_pattern().is_match(&base_snapshot_text)
        && base_facts
            .candidate_course_codes
            .len()
            .max(base_snapshot_course_count)
            >= 6;
    if !has_strong_primary_requirement_section {
        return true;
    }
    let link_text = normalize_requirement_matcher_text(&format!(
        "{} {}",
        string_field(supplemental_entry, "label"),
        string_field(supplemental_entry, "url")
    ));
    if supplemental_irrelevant_link_pattern().is_match(&link_text)
        || curriculum_variant_link_pattern().is_match(&link_text)
    {
        return false;
    }
    candidate
        .get("sameProgramRequirementLink")
        .and_then(Value::as_bool)
        == Some(true)
}

pub fn should_prefer_supplemental_html_source(
    entry: &Value,
    base_facts: &RequirementSourceTextFacts,
    candidate: &Value,
    parsed_facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !is_pathway_owner_entry(entry) || !has_meaningful_parsed_content(parsed_facts) {
        return false;
    }
    let mut candidate_entry = entry.as_object().cloned().unwrap_or_default();
    candidate_entry.remove("role");
    for field in ["url", "label"] {
        if let Some(value) = candidate.get(field) {
            candidate_entry.insert(field.to_string(), value.clone());
        }
    }
    let candidate_entry = Value::Object(candidate_entry);
    let candidate_role =
        super::requirement_role_classifier::classify_requirement_source_role(&candidate_entry);
    if !super::requirement_scope::build_requirement_source_scope(Some(candidate_role))
        .can_create_schedule_rows
    {
        return false;
    }
    let candidate_link_text = format!(
        "{} {}",
        string_field(candidate, "label"),
        string_field(candidate, "url")
    );
    if graduate_supplemental_pattern().is_match(&candidate_link_text)
        && !undergraduate_supplemental_pattern().is_match(&candidate_link_text)
    {
        return false;
    }
    let course_count = parsed_facts.candidate_course_codes.len();
    if course_count < 3 {
        return false;
    }
    let (identity_score, pathway_tokens, matched_tokens) = supplemental_html_pathway_identity_score(
        entry,
        candidate,
        parsed_facts,
        primary_major_titles,
    );
    if identity_score < 12 || matched_tokens.is_empty() {
        return false;
    }
    if pathway_tokens.iter().any(|token| token.len() > 4)
        && !matched_tokens.iter().any(|token| token.len() > 4)
    {
        return false;
    }
    if !supplemental_html_candidate_has_selected_pathway_phrase(entry, candidate, parsed_facts) {
        return false;
    }
    if get_parsed_owner_alignment_score(&candidate_entry, parsed_facts) < 2
        || !has_focused_pathway_requirement_signal(&candidate_entry, candidate, parsed_facts)
    {
        return false;
    }
    let base_course_count = base_facts.candidate_course_codes.len();
    if base_course_count == 0
        || string_field(candidate, "type") == "specialized"
        || json_truthy(candidate.get("sameProgramRequirementLink"))
    {
        return true;
    }
    if course_count <= 80.max((base_course_count as f64 * 0.8).floor() as usize) {
        return true;
    }
    let child_signal_score = get_parsed_degree_sheet_signal_score(parsed_facts);
    let base_signal_score = get_parsed_degree_sheet_signal_score(base_facts);
    child_signal_score >= base_signal_score + 2 && course_count <= 120.max(base_course_count)
}

pub fn should_prefer_supplemental_document_source(
    entry: &Value,
    base_facts: &RequirementSourceTextFacts,
    candidate: &Value,
    document_facts: &RequirementSourceTextFacts,
) -> bool {
    let has_strong_identity = [
        "exactTitleMatch",
        "titleAcronymMatch",
        "sameProgramRequirementLink",
    ]
    .into_iter()
    .any(|field| json_truthy(candidate.get(field)));
    if !has_strong_identity
        || json_truthy(candidate.get("historical"))
        || !has_meaningful_parsed_content(document_facts)
    {
        return false;
    }
    let mut document_entry = entry.as_object().cloned().unwrap_or_default();
    for (field, candidate_field) in [
        ("url", "url"),
        ("label", "label"),
        ("parserType", "parserType"),
    ] {
        if let Some(value) = candidate.get(candidate_field) {
            document_entry.insert(field.to_string(), value.clone());
        }
    }
    let document_entry = Value::Object(document_entry);
    if get_parsed_owner_alignment_score(&document_entry, document_facts) < 2 {
        return false;
    }
    let document_course_count = document_facts.candidate_course_codes.len();
    if document_course_count < 4 {
        return false;
    }
    let document_signal_score = get_parsed_degree_sheet_signal_score(document_facts);
    let base_signal_score = get_parsed_degree_sheet_signal_score(base_facts);
    let document_source_role =
        super::requirement_role_classifier::classify_requirement_source_role(&document_entry);
    let base_course_count = base_facts.candidate_course_codes.len();
    let base_text = normalize_requirement_matcher_text(
        &std::iter::once(base_facts.title.as_deref().unwrap_or_default())
            .chain(base_facts.headings.iter().map(String::as_str))
            .chain(
                base_facts
                    .requirement_cue_lines
                    .iter()
                    .take(20)
                    .map(String::as_str),
            )
            .chain(
                base_facts
                    .snapshot_lines
                    .iter()
                    .take(80)
                    .map(String::as_str),
            )
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let base_has_focused_curriculum_sections = focused_curriculum_requirements_pattern()
        .is_match(&base_text)
        && focused_curriculum_core_electives_pattern().is_match(&base_text);
    let base_is_focused_html_curriculum = matches!(
        string_field(entry, "parserType").as_str(),
        "html-curriculum-page" | "html-degree-page" | "catalog-page"
    ) && get_source_role_score(entry) >= 3
        && base_has_focused_curriculum_sections;
    let document_codes = document_facts
        .candidate_course_codes
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let base_only_course_count = base_facts
        .candidate_course_codes
        .iter()
        .filter(|course_code| !document_codes.contains(*course_code))
        .count();
    if base_is_focused_html_curriculum
        && base_course_count >= document_course_count + 12
        && base_only_course_count >= 8.max((base_course_count as f64 * 0.25).floor() as usize)
    {
        return false;
    }
    if has_strong_identity
        && matches!(
            document_source_role,
            "pathway-degree-sheet" | "primary-degree-requirements"
        )
        && document_signal_score >= 4
        && base_course_count >= document_course_count * 2
    {
        return true;
    }
    if document_signal_score >= 4.max(base_signal_score + 2) {
        return true;
    }
    document_course_count >= 6.max((base_course_count as f64 * 0.35).floor() as usize)
}

pub fn get_parsed_result_score(entry: &Value, facts: &RequirementSourceTextFacts) -> i64 {
    get_source_role_score(entry) as i64 * 60
        + get_parsed_owner_alignment_score(entry, facts) as i64 * 100
        + facts.candidate_course_codes.len() as i64 * 12
        + facts.requirement_cue_lines.len() as i64 * 3
        + facts.choose_statements.len() as i64 * 2
        + facts.pathway_labels.len() as i64
}

pub fn is_weak_parsed_result(
    entry: &Value,
    facts: &RequirementSourceTextFacts,
    used_snapshot_fallback: bool,
) -> bool {
    facts.candidate_course_codes.is_empty()
        || facts.parse_confidence != "high"
        || get_parsed_owner_alignment_score(entry, facts) < 2
        || used_snapshot_fallback
}

pub fn get_course_code_subject(course_code: &str) -> Option<String> {
    let normalized = normalize_course_code(course_code);
    course_code_subject_pattern()
        .captures(&normalized)
        .and_then(|captures| captures.get(1))
        .map(|subject| subject.as_str().to_string())
}

pub fn get_course_code_numeric_level(course_code: &str) -> Option<u16> {
    let normalized = normalize_course_code(course_code);
    course_code_numeric_level_pattern()
        .captures(&normalized)
        .and_then(|captures| captures.get(1))
        .and_then(|level| level.as_str().parse::<u16>().ok())
}

pub fn get_parsed_course_subjects(facts: &RequirementSourceTextFacts) -> Vec<String> {
    facts
        .candidate_course_codes
        .iter()
        .filter_map(|course_code| get_course_code_subject(course_code))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn get_parsed_lower_division_course_count(facts: &RequirementSourceTextFacts) -> usize {
    facts
        .candidate_course_codes
        .iter()
        .filter(|course_code| {
            get_course_code_numeric_level(course_code).is_some_and(|level| level < 300)
        })
        .count()
}

fn should_allow_alternate_to_replace_best_source_internal(
    best_entry: &Value,
    best_facts: &RequirementSourceTextFacts,
    best_used_snapshot_fallback: bool,
    alternate_entry: &Value,
    alternate_facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
    owner_identity_context: Option<(&HashMap<String, String>, &[Value], &CourseCodeParser)>,
) -> bool {
    let alternate_label = string_field(alternate_entry, "label");
    let alternate_url = string_field(alternate_entry, "url");
    if !has_meaningful_parsed_content(alternate_facts) {
        return false;
    }
    if let Some((bootstrap_plan_titles, manifest_entries, course_parser)) = owner_identity_context {
        if super::requirement_plan_matching::parser_recovery_candidate_conflicts_with_owner_identity(
            best_entry,
            &alternate_label,
            &alternate_url,
            bootstrap_plan_titles,
            primary_major_titles,
            manifest_entries,
            course_parser,
        ) {
            return false;
        }
    }
    if super::requirement_recovery_scope::parser_recovery_candidate_conflicts_with_pathway_scope(
        best_entry,
        &alternate_label,
        &alternate_url,
        primary_major_titles,
    ) {
        return false;
    }
    let best_scope = super::requirement_scope::build_requirement_source_scope(Some(
        super::requirement_role_classifier::classify_requirement_source_role(best_entry),
    ));
    let alternate_scope = super::requirement_scope::build_requirement_source_scope(Some(
        super::requirement_role_classifier::classify_requirement_source_role(alternate_entry),
    ));
    if best_scope.can_create_schedule_rows && !alternate_scope.can_create_schedule_rows {
        return false;
    }
    if !best_facts.candidate_course_codes.is_empty()
        && alternate_facts.candidate_course_codes.is_empty()
        && is_focused_degree_source(best_entry)
        && is_broad_supplemental_source(alternate_entry)
    {
        return false;
    }
    if !is_focused_degree_source(best_entry) {
        return true;
    }
    if is_legacy_student_catalog_source(alternate_entry) {
        return false;
    }
    let best_is_weak = is_weak_parsed_result(best_entry, best_facts, best_used_snapshot_fallback);
    let best_alignment = get_parsed_owner_alignment_score(best_entry, best_facts);
    let alternate_alignment = get_parsed_owner_alignment_score(alternate_entry, alternate_facts);
    if !best_is_weak && alternate_alignment <= best_alignment {
        return false;
    }
    if !is_broad_supplemental_source(alternate_entry) {
        return true;
    }
    best_is_weak
}

pub fn should_allow_alternate_to_replace_best_source(
    best_entry: &Value,
    best_facts: &RequirementSourceTextFacts,
    best_used_snapshot_fallback: bool,
    alternate_entry: &Value,
    alternate_facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    should_allow_alternate_to_replace_best_source_internal(
        best_entry,
        best_facts,
        best_used_snapshot_fallback,
        alternate_entry,
        alternate_facts,
        primary_major_titles,
        None,
    )
}

pub(super) fn should_allow_alternate_to_replace_best_source_with_context(
    best_entry: &Value,
    best_facts: &RequirementSourceTextFacts,
    best_used_snapshot_fallback: bool,
    alternate_entry: &Value,
    alternate_facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
    bootstrap_plan_titles: &HashMap<String, String>,
    manifest_entries: &[Value],
    course_parser: &CourseCodeParser,
) -> bool {
    should_allow_alternate_to_replace_best_source_internal(
        best_entry,
        best_facts,
        best_used_snapshot_fallback,
        alternate_entry,
        alternate_facts,
        primary_major_titles,
        Some((bootstrap_plan_titles, manifest_entries, course_parser)),
    )
}

fn should_merge_supplemental_alternate_source_internal(
    base_entry: &Value,
    base_facts: &RequirementSourceTextFacts,
    base_used_snapshot_fallback: bool,
    alternate_entry: &Value,
    alternate_facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
    owner_identity_context: Option<(&HashMap<String, String>, &[Value], &CourseCodeParser)>,
) -> bool {
    let alternate_label = string_field(alternate_entry, "label");
    let alternate_url = string_field(alternate_entry, "url");
    if !has_meaningful_parsed_content(alternate_facts) {
        return false;
    }
    if let Some((bootstrap_plan_titles, manifest_entries, course_parser)) = owner_identity_context {
        if super::requirement_plan_matching::parser_recovery_candidate_conflicts_with_owner_identity(
            base_entry,
            &alternate_label,
            &alternate_url,
            bootstrap_plan_titles,
            primary_major_titles,
            manifest_entries,
            course_parser,
        ) {
            return false;
        }
    }
    if super::requirement_recovery_scope::parser_recovery_candidate_conflicts_with_pathway_scope(
        base_entry,
        &alternate_label,
        &alternate_url,
        primary_major_titles,
    ) || get_parsed_owner_alignment_score(alternate_entry, alternate_facts) < 2
    {
        return false;
    }
    let base_scope = super::requirement_scope::build_requirement_source_scope(Some(
        super::requirement_role_classifier::classify_requirement_source_role(base_entry),
    ));
    let alternate_scope = super::requirement_scope::build_requirement_source_scope(Some(
        super::requirement_role_classifier::classify_requirement_source_role(alternate_entry),
    ));
    if (!base_scope.can_create_schedule_rows && alternate_scope.can_create_schedule_rows)
        || !alternate_scope.can_create_schedule_rows
    {
        return false;
    }
    let legacy_alternate = is_legacy_student_catalog_source(alternate_entry);
    if is_focused_degree_source(base_entry)
        && legacy_alternate
        && is_shared_language_department_source(base_entry)
    {
        return false;
    }
    let base_is_weak = is_weak_parsed_result(base_entry, base_facts, base_used_snapshot_fallback);
    if !base_is_weak && legacy_alternate {
        return false;
    }
    if !base_is_weak
        && is_focused_degree_source(base_entry)
        && is_focused_degree_source(alternate_entry)
    {
        return false;
    }
    if is_focused_degree_source(base_entry) && is_broad_supplemental_source(alternate_entry) {
        return false;
    }
    let adds_course_codes = alternate_facts
        .candidate_course_codes
        .iter()
        .any(|code| !base_facts.candidate_course_codes.contains(code));
    let adds_requirement_cues = alternate_facts
        .requirement_cue_lines
        .iter()
        .any(|line| !base_facts.requirement_cue_lines.contains(line));
    let adds_choose_statements = alternate_facts
        .choose_statements
        .iter()
        .any(|line| !base_facts.choose_statements.contains(line));
    adds_course_codes || adds_requirement_cues || adds_choose_statements
}

pub fn should_merge_supplemental_alternate_source(
    base_entry: &Value,
    base_facts: &RequirementSourceTextFacts,
    base_used_snapshot_fallback: bool,
    alternate_entry: &Value,
    alternate_facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    should_merge_supplemental_alternate_source_internal(
        base_entry,
        base_facts,
        base_used_snapshot_fallback,
        alternate_entry,
        alternate_facts,
        primary_major_titles,
        None,
    )
}

pub(super) fn should_merge_supplemental_alternate_source_with_context(
    base_entry: &Value,
    base_facts: &RequirementSourceTextFacts,
    base_used_snapshot_fallback: bool,
    alternate_entry: &Value,
    alternate_facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
    bootstrap_plan_titles: &HashMap<String, String>,
    manifest_entries: &[Value],
    course_parser: &CourseCodeParser,
) -> bool {
    should_merge_supplemental_alternate_source_internal(
        base_entry,
        base_facts,
        base_used_snapshot_fallback,
        alternate_entry,
        alternate_facts,
        primary_major_titles,
        Some((bootstrap_plan_titles, manifest_entries, course_parser)),
    )
}

fn is_shared_language_department_source(entry: &Value) -> bool {
    let source_text = ["url", "label", "sourceLabel", "ownerTitle"]
        .into_iter()
        .filter_map(|field| entry.get(field).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let source_text = normalize_requirement_matcher_text(&source_text);
    source_text.contains("frenchitalian")
        || (source_text.contains("french") && source_text.contains("italian"))
}

pub fn is_focused_degree_source(entry: &Value) -> bool {
    get_source_role_score(entry) >= 4
        || matches!(
            string_field(entry, "parserType").as_str(),
            "html-degree-page"
                | "html-curriculum-page"
                | "catalog-page"
                | "pdf-degree-sheet"
                | "pdf-worksheet"
        )
}

fn is_pathway_owner_entry(entry: &Value) -> bool {
    json_truthy(entry.get("pathwayId"))
        || entry
            .get("ownerId")
            .and_then(Value::as_str)
            .is_some_and(|owner_id| owner_id.contains(":pathway:"))
}

fn supplemental_html_pathway_identity_score(
    entry: &Value,
    candidate: &Value,
    facts: &RequirementSourceTextFacts,
    primary_major_titles: &HashMap<String, String>,
) -> (usize, Vec<String>, Vec<String>) {
    let raw_text = [
        string_field(entry, "pathwayId"),
        supplemental_pathway_owner_descriptor_pattern()
            .split(&string_field(entry, "ownerTitle"))
            .skip(1)
            .collect::<Vec<_>>()
            .join(" "),
        string_field(entry, "sourceLabel"),
        string_field(entry, "label"),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let normalized_base_title = normalize_requirement_matcher_text(
        &super::requirement_recovery_scope::primary_major_title(entry, primary_major_titles),
    );
    let base_title_tokens = normalized_base_title
        .split_whitespace()
        .map(str::to_string)
        .collect::<std::collections::HashSet<_>>();
    let mut pathway_tokens = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for captures in pathway_acronym_token_pattern().captures_iter(&raw_text) {
        if let Some(token) = captures.get(0) {
            let token = token.as_str().to_ascii_lowercase();
            if seen.insert(token.clone()) {
                pathway_tokens.push(token);
            }
        }
    }
    let normalized_raw_text = normalize_requirement_matcher_text(&raw_text);
    for token in normalized_raw_text.split_whitespace() {
        if token.len() >= 3
            && !pathway_identity_generic_token_pattern().is_match(token)
            && !base_title_tokens.contains(token)
            && seen.insert(token.to_string())
        {
            pathway_tokens.push(token.to_string());
        }
    }
    let candidate_text = normalize_requirement_matcher_text(
        &[
            string_field(candidate, "label"),
            string_field(candidate, "url"),
            facts.title.clone().unwrap_or_default(),
            facts
                .headings
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            facts
                .pathway_labels
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            facts
                .snapshot_lines
                .iter()
                .take(24)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        ]
        .join(" "),
    );
    let matched_tokens = pathway_tokens
        .iter()
        .filter(|token| normalized_matcher_text_contains_token(&candidate_text, token))
        .cloned()
        .collect::<Vec<_>>();
    let pathway_id = normalize_requirement_matcher_text(&string_field(entry, "pathwayId"));
    let compact_pathway_id = pathway_id.split_whitespace().collect::<String>();
    let compact_candidate_text = candidate_text.split_whitespace().collect::<String>();
    let mut score = matched_tokens.len() * 6;
    if !compact_pathway_id.is_empty() && compact_candidate_text.contains(&compact_pathway_id) {
        score += 10;
    }
    if matched_tokens.iter().any(|token| token.len() <= 4) {
        score += 4;
    }
    if matched_tokens.len() >= 2.min(pathway_tokens.len())
        && pathway_identity_requirement_cue_pattern().is_match(&candidate_text)
    {
        score += 6;
    }
    (score, pathway_tokens, matched_tokens)
}

fn normalized_matcher_text_contains_token(text: &str, token: &str) -> bool {
    let source = format!(r"\b{}\b", regex::escape(token));
    let pattern = super::requirement_shape::js_regex_compatibility_pattern(&source);
    Regex::new(&pattern).is_ok_and(|pattern| pattern.is_match(text))
}

fn supplemental_html_candidate_has_selected_pathway_phrase(
    entry: &Value,
    candidate: &Value,
    facts: &RequirementSourceTextFacts,
) -> bool {
    let pathway_id = string_field(entry, "pathwayId")
        .replace('-', " ")
        .replace('_', " ");
    let normalized_pathway_id = normalize_requirement_matcher_text(&pathway_id);
    let phrase = normalized_pathway_id
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_identity_stopword_pattern().is_match(token))
        .collect::<Vec<_>>()
        .join(" ");
    if phrase.is_empty() || !phrase.split_whitespace().any(|token| token.len() > 4) {
        return true;
    }
    let candidate_text = normalize_requirement_matcher_text(
        &[
            string_field(candidate, "label"),
            string_field(candidate, "url"),
            facts.title.clone().unwrap_or_default(),
            facts
                .headings
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            facts
                .pathway_labels
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            facts
                .snapshot_lines
                .iter()
                .take(24)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        ]
        .join(" "),
    );
    let compact_pathway_id = normalize_requirement_matcher_text(&string_field(entry, "pathwayId"))
        .split_whitespace()
        .collect::<String>();
    let compact_candidate_text = candidate_text.split_whitespace().collect::<String>();
    candidate_text.contains(&phrase)
        || (!compact_pathway_id.is_empty() && compact_candidate_text.contains(&compact_pathway_id))
}

fn has_focused_pathway_requirement_signal(
    entry: &Value,
    candidate: &Value,
    facts: &RequirementSourceTextFacts,
) -> bool {
    let sampled_text = normalize_requirement_matcher_text(
        &[
            string_field(candidate, "label"),
            facts.title.clone().unwrap_or_default(),
            facts
                .headings
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            facts
                .requirement_cue_lines
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            facts
                .snapshot_lines
                .iter()
                .take(32)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        ]
        .join(" "),
    );
    if sampled_text.is_empty() {
        return false;
    }
    if super::requirement_rule_utils::detect_option_replacement(
        entry,
        &facts.snapshot_lines,
        candidate.get("url"),
    )
    .is_some()
    {
        return true;
    }
    focused_pathway_degree_cue_pattern().is_match(&sampled_text)
        && focused_pathway_option_cue_pattern().is_match(&sampled_text)
        && (facts.candidate_course_codes.len() >= 3 || facts.requirement_cue_lines.len() >= 2)
}

fn is_broad_supplemental_source(entry: &Value) -> bool {
    get_source_role_score(entry) <= 2
        || matches!(
            string_field(entry, "parserType").as_str(),
            "html-overview-page" | "generic-html"
        )
}

fn json_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value
            .as_f64()
            .is_some_and(|number| number != 0.0 && !number.is_nan()),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

fn is_legacy_student_catalog_source(entry: &Value) -> bool {
    let Some(url) = entry.get("url").and_then(Value::as_str) else {
        return false;
    };
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    matches!(
        url.host_str(),
        Some("washington.edu" | "www.washington.edu")
    ) && url.path().starts_with("/students/gencat/program/")
}

fn string_field(entry: &Value, field: &str) -> String {
    entry
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn degree_sheet_before_applying_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\bbefore applying\b");
        Regex::new(&pattern).expect("degree sheet before applying")
    })
}

fn degree_sheet_prerequisite_key_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\bdenotes prerequisites\b");
        Regex::new(&pattern).expect("degree sheet prerequisite key")
    })
}

fn degree_sheet_areas_of_inquiry_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\bareas of inquiry\b");
        Regex::new(&pattern).expect("degree sheet areas of inquiry")
    })
}

fn degree_sheet_math_science_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\bmathematics natural sciences\b",
        );
        Regex::new(&pattern).expect("degree sheet math sciences")
    })
}

fn degree_sheet_fundamentals_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(r"\bfundamentals\b");
        Regex::new(&pattern).expect("degree sheet fundamentals")
    })
}

fn degree_sheet_core_electives_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\bcore and electives\b");
        Regex::new(&pattern).expect("degree sheet core and electives")
    })
}

fn degree_sheet_additional_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\badditional requirements\b",
        );
        Regex::new(&pattern).expect("degree sheet additional requirements")
    })
}

fn focused_curriculum_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:major|degree|program)?\s*requirements?\b",
        );
        Regex::new(&pattern).expect("focused curriculum requirements")
    })
}

fn focused_curriculum_core_electives_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:core|foundational|electives?|choose)\b",
        );
        Regex::new(&pattern).expect("focused curriculum core or electives")
    })
}

fn supplemental_pathway_owner_descriptor_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\s+[-–—:]\s+",
        ))
        .expect("supplemental owner descriptor separator")
    })
}

fn pathway_acronym_token_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\b[A-Z0-9&]{2,8}\b");
        Regex::new(&pattern).expect("supplemental pathway acronym token")
    })
}

fn pathway_identity_generic_token_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"^(?:base|concentration|default|major|option|pathway|route|specialization|standard|track)$"))
            .expect("generic pathway identity token")
    })
}

fn pathway_identity_stopword_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"^(?:and|ba|bachelor|bs|certificate|concentration|degree|major|of|option|pathway|program|credit|credits|route|track|with)$"))
            .expect("pathway identity stopword")
    })
}

fn pathway_identity_requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:option|track|route|pathway|concentration|specialization|requirements?|curriculum)\b",
        );
        Regex::new(&pattern).expect("pathway identity requirement cue")
    })
}

fn primary_required_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\brequired courses?\b");
        Regex::new(&pattern).expect("primary required courses cue")
    })
}

fn must_complete_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\bmust complete\b.{0,120}\bcourses?\b",
        );
        Regex::new(&pattern).expect("must complete courses cue")
    })
}

fn curriculum_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(r"\bcurriculum\b");
        Regex::new(&pattern).expect("curriculum cue")
    })
}

fn lower_upper_core_course_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:lower division|upper division|core courses?)\b",
        );
        Regex::new(&pattern).expect("primary core course section cue")
    })
}

fn supplemental_irrelevant_link_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:minors?|certificates?|special programs?|student experience|scholarships?)\b",
        );
        Regex::new(&pattern).expect("irrelevant supplemental link")
    })
}

fn curriculum_variant_link_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\bcurriculum\s+(?:options?|tracks?|pathways?|routes?|concentrations?|specializations?)\b",
        );
        Regex::new(&pattern).expect("curriculum variant link")
    })
}

fn graduate_supplemental_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:graduate|ph\.?\s*d\.?|doctor(?:al|ate)?|m\.?\s*a\.?|masters?|master(?:'s)?)\b|/(?:ma|phd|graduate|masters?|amp)(?:[-/]|$)",
        );
        Regex::new(&pattern).expect("graduate supplemental source")
    })
}

fn undergraduate_supplemental_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\bundergrad(?:uate)?\b");
        Regex::new(&pattern).expect("undergraduate supplemental source")
    })
}

fn focused_pathway_degree_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:degree|major|program|curriculum|requirements?)\b",
        );
        Regex::new(&pattern).expect("focused pathway degree cue")
    })
}

fn focused_pathway_option_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:option|track|route|pathway|concentration|specialization|core|electives?)\b",
        );
        Regex::new(&pattern).expect("focused pathway option cue")
    })
}

fn course_code_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"^([A-Z& ]+)\s+[0-9]",
        ))
        .expect("course code subject pattern")
    })
}

fn course_code_numeric_level_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\b([0-9]{3})[A-Z]?$");
        Regex::new(&pattern).expect("course code numeric level pattern")
    })
}

fn base_route_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(r"(?i)\b(?:standard|default|base)\b.*\b(?:route|pathway|option)\b|\b(?:route|pathway|option)\b.*\b(?:standard|default|base)\b"))
            .expect("base route alignment pattern")
    })
}

fn bachelor_major_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:bachelor|major|b a)\b",
        ))
        .expect("parsed bachelor major alignment")
    })
}
