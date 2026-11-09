use super::requirement_pdf_layout::get_pdf_title_scope_tokens;
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;

pub fn source_content_indicates_inactive_major(
    entry: &Value,
    parsed_title: Option<&str>,
    headings: &[String],
    requirement_cue_lines: &[String],
    choose_statements: &[String],
    snapshot_lines: &[String],
) -> bool {
    let mut searchable = ["ownerTitle", "label", "sourceLabel"]
        .into_iter()
        .filter_map(|field| entry.get(field))
        .filter(|value| is_js_truthy(value))
        .map(js_string)
        .collect::<Vec<_>>();
    searchable.extend(
        parsed_title
            .filter(|title| !title.is_empty())
            .map(str::to_string),
    );
    searchable.extend(headings.iter().filter(|line| !line.is_empty()).cloned());
    searchable.extend(
        requirement_cue_lines
            .iter()
            .filter(|line| !line.is_empty())
            .cloned(),
    );
    searchable.extend(
        choose_statements
            .iter()
            .filter(|line| !line.is_empty())
            .cloned(),
    );
    searchable.extend(
        snapshot_lines
            .iter()
            .filter(|line| !line.is_empty())
            .cloned(),
    );
    inactive_major_pattern().is_match(&normalize_matcher_text(&searchable.join(" ")))
}

pub fn should_promote_primary_overview_from_parsed_evidence(
    entry: &Value,
    parsed_source_role: Option<&str>,
    metadata_source_role: &str,
    parsed_course_codes: &[String],
    headings: &[String],
    requirement_cue_lines: &[String],
    choose_statements: &[String],
) -> bool {
    let resolved_source_role = parsed_source_role.unwrap_or(metadata_source_role);
    if entry
        .get("isPrimaryDegreeRequirementsLink")
        .and_then(Value::as_bool)
        != Some(true)
        || resolved_source_role != "overview"
        || parsed_course_codes.is_empty()
    {
        return false;
    }
    headings
        .iter()
        .chain(requirement_cue_lines.iter())
        .chain(choose_statements.iter())
        .any(|line| parsed_primary_requirement_heading_pattern().is_match(line.trim()))
}

pub fn should_use_tacoma_metadata_primary_role(
    entry: &Value,
    parsed_source_role: Option<&str>,
    metadata_source_role: &str,
    parsed_course_codes: &[String],
    requirement_cue_lines: &[String],
    choose_statements: &[String],
    pathway_labels: &[String],
) -> bool {
    if entry.get("campusId").and_then(Value::as_str) != Some("uw-tacoma")
        || metadata_source_role != "primary-degree-requirements"
        || super::requirement_scope::get_requirement_source_role_status(
            parsed_source_role.or(Some(metadata_source_role)),
        )
        .as_str()
            == "primary"
    {
        return false;
    }
    if !parsed_course_codes.is_empty() || !pathway_labels.is_empty() {
        return true;
    }
    let evidence = requirement_cue_lines
        .iter()
        .chain(choose_statements.iter())
        .chain(pathway_labels.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    primary_requirement_cue_pattern().is_match(&evidence)
        || tacoma_parsed_primary_requirement_pattern().is_match(&evidence)
}

pub fn get_inactive_major_evidence_lines(
    requirement_cue_lines: &[String],
    choose_statements: &[String],
    snapshot_lines: &[String],
) -> Vec<String> {
    let mut seen = HashSet::new();
    requirement_cue_lines
        .iter()
        .chain(choose_statements.iter())
        .chain(snapshot_lines.iter())
        .map(|line| super::requirement_shape::normalize_js_whitespace(line))
        .filter(|line| !line.is_empty() && inactive_major_evidence_line_pattern().is_match(line))
        .filter(|line| seen.insert(line.clone()))
        .take(4)
        .collect()
}

pub fn classify_requirement_source_role(entry: &Value) -> &'static str {
    let source_identity = normalize_matcher_text(&joined_truthy_fields(
        entry,
        &["url", "label", "sourceLabel"],
    ));
    let source_searchable = normalize_matcher_text(&joined_truthy_fields(
        entry,
        &["url", "label", "sourceLabel", "role", "parserType"],
    ));
    let searchable = normalize_matcher_text(&format!(
        "{} {}",
        source_searchable,
        string_field(entry, "ownerTitle")
    ));
    if searchable.is_empty() {
        return "ignored";
    }

    if is_unscoped_acs_supplemental_source(entry) {
        return "non-schedulable-course-list";
    }
    if approved_course_list_pattern().is_match(&source_identity)
        && !primary_requirement_cue_pattern().is_match(&source_identity)
    {
        return "approved-course-list";
    }
    if has_standalone_elective_list_source_cue(&source_identity)
        && !primary_requirement_cue_pattern().is_match(&source_identity)
    {
        return "elective-list";
    }
    if is_historical_requirement_source_identity(&source_identity) {
        return "old-archival";
    }
    if entry
        .get("isPrimaryDegreeRequirementsLink")
        .is_some_and(is_js_truthy)
        && is_materialized_primary_requirement_link(entry, &source_searchable, &searchable)
    {
        return "primary-degree-requirements";
    }

    let manifest_role = string_field(entry, "role");
    let entry_url = string_field(entry, "url");
    if manifest_role == "overview" {
        return "overview";
    }
    let uwb_major_admissions = uwb_major_admissions_url_pattern().is_match(&entry_url);
    if uwb_major_admissions {
        return "admissions-preparation";
    }
    let is_document_worksheet = string_field(entry, "parserType") == "pdf-worksheet"
        || document_url_pattern().is_match(&entry_url);
    let major_planning_worksheet = major_planning_worksheet_pattern().is_match(&searchable);
    if major_planning_worksheet
        && is_document_worksheet
        && !uwb_mpw_registration_url_pattern().is_match(&entry_url)
        && !admission_prerequisite_pattern().is_match(&searchable)
    {
        return "primary-degree-requirements";
    }
    if uwb_mpw_registration_url_pattern().is_match(&entry_url) || major_planning_worksheet {
        return "admission-prerequisite-source";
    }
    match manifest_role.as_str() {
        "approved-course-list" => return "approved-course-list",
        "elective-list" => return "elective-list",
        "upper-division-prerequisite-table" => return "upper-division-prerequisite-table",
        "non-schedulable-course-list" => {
            if is_materialized_primary_requirement_link(entry, &source_searchable, &searchable) {
                return "primary-degree-requirements";
            }
            return "non-schedulable-course-list";
        }
        "admission-prerequisite-source" => return "admission-prerequisite-source",
        "admissions" => return "admissions-preparation",
        "degree-requirements" | "curriculum" | "worksheet"
            if !entry
                .get("isPrimaryDegreeRequirementsLink")
                .is_some_and(is_js_truthy) =>
        {
            return "support-source";
        }
        "degree-requirements" | "curriculum" | "worksheet" => {
            return "primary-degree-requirements";
        }
        "catalog" => return "official-catalog",
        "pathway-degree-sheet" => return "pathway-degree-sheet",
        _ => {}
    }

    if uw_general_catalog_program_url_pattern().is_match(&entry_url) {
        return "official-catalog";
    }
    if is_tacoma_set_undergrad_program_url(&entry_url) {
        if primary_requirement_cue_pattern().is_match(&searchable)
            || admission_prerequisite_pattern().is_match(&searchable)
            || tacoma_set_requirement_terms_pattern().is_match(&searchable)
        {
            return "primary-degree-requirements";
        }
        return "department-requirements";
    }
    if historical_fallback_pattern().is_match(&searchable) {
        return "old-archival";
    }
    if non_major_options_pattern().is_match(&source_searchable) {
        return "non-schedulable-course-list";
    }
    if equivalency_pattern().is_match(&searchable) {
        return "transfer-equivalency";
    }
    if green_river_pattern().is_match(&searchable) {
        return "matched-grc-track";
    }
    if sample_schedule_pattern().is_match(&searchable) {
        return "sample-schedule";
    }
    if degree_map_pattern().is_match(&searchable) {
        return "curriculum-map";
    }
    if pathway_degree_sheet_pattern().is_match(&source_searchable)
        && pathway_cue_pattern().is_match(&source_searchable)
    {
        return "pathway-degree-sheet";
    }
    if approved_course_list_pattern().is_match(&searchable) {
        return "approved-course-list";
    }
    if elective_list_pattern().is_match(&searchable) {
        return "elective-list";
    }
    if upper_division_prerequisite_pattern().is_match(&searchable) {
        return "upper-division-prerequisite-table";
    }
    if entry
        .get("isPrimaryDegreeRequirementsLink")
        .is_some_and(is_js_truthy)
        && is_materialized_primary_requirement_link(entry, &source_searchable, &searchable)
    {
        return "primary-degree-requirements";
    }
    if non_schedulable_course_list_pattern().is_match(&searchable) {
        return "non-schedulable-course-list";
    }
    if admission_prerequisite_pattern().is_match(&searchable) {
        return "admission-prerequisite-source";
    }
    if support_cue_pattern().is_match(&searchable) {
        return "support-source";
    }
    if primary_requirement_cue_pattern().is_match(&searchable)
        || pathway_degree_sheet_pattern().is_match(&searchable)
        || generic_degree_sheet_pattern().is_match(&searchable)
    {
        return "primary-degree-requirements";
    }
    if admissions_preparation_pattern().is_match(&searchable) {
        return "admissions-preparation";
    }
    if curriculum_map_fallback_pattern().is_match(&searchable) {
        return "curriculum-map";
    }
    if department_requirements_fallback_pattern().is_match(&searchable) {
        return "department-requirements";
    }
    "ignored"
}

fn is_materialized_primary_requirement_link(
    entry: &Value,
    source_searchable: &str,
    searchable: &str,
) -> bool {
    let manifest_role = string_field(entry, "role");
    if matches!(
        manifest_role.as_str(),
        "approved-course-list"
            | "elective-list"
            | "upper-division-prerequisite-table"
            | "admission-prerequisite-source"
            | "admissions"
            | "catalog"
            | "pathway-degree-sheet"
    ) {
        return false;
    }
    let url = string_field(entry, "url");
    let owner_id = string_field(entry, "ownerId");
    let owner_is_pathway =
        !string_field(entry, "pathwayId").is_empty() || owner_id.contains(":pathway:");
    if admission_terms_pattern().is_match(source_searchable)
        || support_cue_pattern().is_match(source_searchable)
    {
        return false;
    }
    let source_looks_like_pathway =
        pathway_cue_pattern().is_match(source_searchable) || pathway_url_pattern().is_match(&url);
    let source_looks_like_degree_program = degree_program_pattern().is_match(source_searchable)
        || bachelors_program_url_pattern().is_match(&url);
    let source_looks_like_requirements = primary_requirement_cue_pattern().is_match(searchable)
        || requirements_program_pattern().is_match(searchable);
    let owner_title = string_field(entry, "ownerTitle");
    let source_label = string_field(entry, "sourceLabel");
    let owner_title_tokens =
        get_pdf_title_scope_tokens(Some(owner_title.as_str()), Some(source_label.as_str()));
    let overlap = owner_title_tokens
        .iter()
        .filter(|token| source_searchable.contains(token.as_str()))
        .count();
    let source_looks_like_owner_degree_program = !source_looks_like_pathway
        && source_looks_like_degree_program
        && overlap >= 1usize.max(2.min(owner_title_tokens.len()));
    (owner_is_pathway && (source_looks_like_pathway || source_looks_like_degree_program))
        || source_looks_like_owner_degree_program
        || (source_looks_like_requirements
            && primary_requirement_parser_type_pattern()
                .is_match(&string_field(entry, "parserType")))
        || is_tacoma_primary_program_page(entry, source_searchable, searchable)
}

fn is_tacoma_primary_program_page(
    entry: &Value,
    source_searchable: &str,
    searchable: &str,
) -> bool {
    let parser_type = string_field(entry, "parserType");
    let url = string_field(entry, "url");
    if string_field(entry, "campusId") != "uw-tacoma"
        || !entry
            .get("isPrimaryDegreeRequirementsLink")
            .is_some_and(is_js_truthy)
        || !tacoma_primary_parser_type_pattern().is_match(&parser_type)
        || !official_tacoma_url(&url)
    {
        return false;
    }
    let owner_title = string_field(entry, "ownerTitle");
    let source_label = string_field(entry, "sourceLabel");
    let mut identity_groups = Vec::new();
    let title_tokens =
        get_pdf_title_scope_tokens(Some(owner_title.as_str()), Some(source_label.as_str()));
    if !title_tokens.is_empty() {
        identity_groups.push(title_tokens);
    }
    let base_title = tacoma_base_title(&owner_title);
    if !base_title.is_empty() {
        let base_tokens = get_pdf_title_scope_tokens(Some(base_title.as_str()), Some(""));
        if !base_tokens.is_empty() {
            identity_groups.push(base_tokens);
        }
    }
    let acronym = tacoma_title_acronym(&base_title);
    if !acronym.is_empty() {
        identity_groups.push(vec![acronym]);
    }
    if identity_groups.is_empty() {
        return true;
    }
    identity_groups.iter().any(|tokens| {
        let overlap = tokens
            .iter()
            .filter(|token| source_searchable.contains(token.as_str()))
            .count();
        overlap >= 1usize.max(2.min(tokens.len()))
    })
}

fn tacoma_title_acronym(title: &str) -> String {
    let normalized = normalize_matcher_text(title);
    for (phrase, acronym) in [
        ("computer science and systems", "css"),
        ("ethnic gender and labor studies", "egls"),
        ("politics philosophy and economics", "ppe"),
        ("sustainable urban development", "sud"),
    ] {
        if phrase_pattern(phrase).is_match(&normalized) {
            return acronym.to_string();
        }
    }
    String::new()
}

fn tacoma_base_title(title: &str) -> String {
    let chars = title.char_indices().collect::<Vec<_>>();
    for (index, character) in chars.iter().enumerate() {
        if !matches!(character.1, '-' | '\u{2013}' | '\u{2014}') {
            continue;
        }
        let before_space = index > 0 && chars[index - 1].1.is_whitespace();
        let after_space = chars
            .get(index + 1)
            .is_some_and(|(_, next)| next.is_whitespace());
        if before_space && after_space {
            return title[..character.0].trim().to_string();
        }
    }
    title.trim().to_string()
}

pub fn has_conflicting_supplemental_degree_route(entry: &Value, label: &str, url: &str) -> bool {
    let entry_tokens = degree_route_tokens(&format!(
        "{} {} {} {}",
        string_field(entry, "ownerTitle"),
        string_field(entry, "sourceLabel"),
        string_field(entry, "label"),
        string_field(entry, "url")
    ));
    let candidate_tokens = degree_route_tokens(&format!("{label} {url}"));
    if entry_tokens.is_empty() || candidate_tokens.is_empty() {
        return false;
    }
    (entry_tokens.contains("ba") && candidate_tokens.contains("bs"))
        || (entry_tokens.contains("bs") && candidate_tokens.contains("ba"))
        || (entry_tokens.contains("ell") && candidate_tokens.contains("sped"))
        || (entry_tokens.contains("sped") && candidate_tokens.contains("ell"))
}

fn is_unscoped_acs_supplemental_source(entry: &Value) -> bool {
    let source = ["label", "sourceLabel", "url"]
        .into_iter()
        .map(|key| string_field(entry, key))
        .collect::<Vec<_>>()
        .join(" ");
    let source_tokens = degree_route_tokens(&source);
    if !source_tokens.contains("acs") {
        return false;
    }
    let owner_scope = ["ownerId", "planId", "pathwayId", "ownerTitle"]
        .into_iter()
        .map(|key| string_field(entry, key))
        .collect::<Vec<_>>()
        .join(" ");
    !degree_route_tokens(&owner_scope).contains("acs")
}

fn degree_route_tokens(value: &str) -> HashSet<&'static str> {
    let normalized = normalize_matcher_text(value);
    let mut tokens = HashSet::new();
    if degree_ba_pattern().is_match(&normalized) {
        tokens.insert("ba");
    }
    if degree_bs_pattern().is_match(&normalized) {
        tokens.insert("bs");
    }
    if acs_pattern().is_match(&normalized) {
        tokens.insert("acs");
        tokens.insert("bs");
    }
    if ell_pattern().is_match(&normalized) {
        tokens.insert("ell");
    }
    if sped_pattern().is_match(&normalized) {
        tokens.insert("sped");
    }
    tokens
}

fn has_standalone_elective_list_source_cue(source: &str) -> bool {
    elective_list_pattern().is_match(source)
        || engineering_electives_pattern().is_match(source)
        || electives_html_pattern().is_match(source)
}

fn is_historical_requirement_source_identity(source: &str) -> bool {
    historical_identity_pattern().is_match(source)
}

fn official_tacoma_url(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "tacoma.uw.edu" || host.ends_with(".tacoma.uw.edu"))
}

fn is_tacoma_set_undergrad_program_url(url: &str) -> bool {
    tacoma_set_undergrad_url_pattern().is_match(url)
}

fn joined_truthy_fields(entry: &Value, fields: &[&str]) -> String {
    fields
        .iter()
        .filter_map(|key| {
            let value = entry.get(*key)?;
            is_js_truthy(value).then(|| js_string(value))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .filter(|field| !field.is_null())
        .map(js_string)
        .unwrap_or_default()
}

fn is_js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
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

fn normalize_matcher_text(value: &str) -> String {
    let mut normalized = String::new();
    let mut separator_pending = false;
    for character in value.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if separator_pending && !normalized.is_empty() {
                normalized.push(' ');
            }
            separator_pending = false;
            normalized.push(character);
        } else {
            separator_pending = !normalized.is_empty();
        }
    }
    normalized
}

fn phrase_pattern(phrase: &str) -> Regex {
    let tokens = phrase.split_whitespace().collect::<Vec<_>>().join(r"\s+");
    let pattern = format!(r"(?i)\b{tokens}\b");
    Regex::new(&js_regex_compatibility_pattern(&pattern)).expect("title phrase pattern")
}

fn parsed_primary_requirement_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:(?:undergraduate|bachelor(?:'s)?)\s+)?(?:major|degree|program)\s+(?:and\s+minor\s+)?requirements?\b|\b(?:required|core)\s+(?:courses?|coursework)\b")).expect("parsed primary requirement heading"))
}

fn inactive_major_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:not able|unable|cannot|can not|not currently able)\b.{0,120}\baccept(?:ing)?\b.{0,80}\bstudents?\b.{0,80}\bmajor\b|\bstudents?\b.{0,60}\bmay not declare\b.{0,80}\bmajor\b|\bmajor\b.{0,80}\b(?:not\b.{0,30}|no\s+longer\s+)accepting\b.{0,60}\bstudents?\b")).expect("inactive major source pattern"))
}

fn inactive_major_evidence_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:not able|unable|cannot|can not|not currently able)\b.{0,120}\b(?:accept(?:ing)?\b.{0,80}\bstudents?|offer\b.{0,80}\bupper\s+level\s+courses?)\b.{0,120}\bmajor\b|\bstudents?\b.{0,60}\bmay not declare\b.{0,80}\bmajor\b|\bmajor\b.{0,80}\bno\s+longer\s+accepting\b.{0,60}\bstudents?\b")).expect("inactive major evidence line pattern"))
}

fn js_regex_compatibility_pattern(pattern: &str) -> String {
    pattern.replace(r"\b", r"(?-u:\b)").replace(r"\d", "[0-9]")
}

macro_rules! matcher {
    ($name:ident, $pattern:literal) => {
        fn $name() -> &'static Regex {
            static VALUE: OnceLock<Regex> = OnceLock::new();
            VALUE.get_or_init(|| {
                Regex::new(&js_regex_compatibility_pattern($pattern)).expect(stringify!($name))
            })
        }
    };
}

matcher!(
    approved_course_list_pattern,
    r"(?i)\bapproved\b.{0,60}\b(courses?|course list|list)\b|\b(courses?|course list|list)\b.{0,60}\bapproved\b"
);
matcher!(
    elective_list_pattern,
    r"(?i)\b(?:engineering|technical|departmental|major|science|natural science|approved)?\s*electives?\b.{0,50}\b(courses?|list|options?|page)\b|\b(courses?|list|options?)\b.{0,50}\belectives?\b|/electives?(?:[-/]|$)"
);
matcher!(
    engineering_electives_pattern,
    r"(?i)\b(?:engineering|technical|departmental|major|science|natural science|approved)\s+electives?\b"
);
matcher!(electives_html_pattern, r"(?i)\belectives?\s+html\b");
matcher!(
    primary_requirement_cue_pattern,
    r"(?i)\bdegree requirements?\b|\bmajor requirements?\b|\bgraduation requirements?\b|\bprogram requirements?\b|\bdegree structure\b|\brequirements packet\b|\bdegreq\b"
);
matcher!(
    tacoma_parsed_primary_requirement_pattern,
    r"(?i)\b(admission requirements?|degree requirements?|graduation requirements?|students? choose\b.{0,80}\b(?:options?|tracks?|routes?|pathways?)|core courses?|preparatory courses?)\b"
);
matcher!(
    admission_prerequisite_pattern,
    r"(?i)\b(?:admissions?|admission|apply|application)\b.{0,80}\bprereq(?:uisites?|uisite courses?)\b|\bprereq(?:uisites?|uisite courses?)\b.{0,80}\b(?:admissions?|admission|apply|application)\b"
);
matcher!(
    admission_terms_pattern,
    r"(?i)\b(?:admissions?|apply|application|preparation|prereq(?:uisites?)?)\b"
);
matcher!(
    support_cue_pattern,
    r"(?i)\b(advising|adviser|advisor|support sources?|student resources?|student support|forms?|petitions?|policies|policy[-\s]*(?:procedures?|resources?|forms?)|faq|frequently asked questions)\b"
);
matcher!(
    historical_identity_pattern,
    r"(?i)\b(?:archive|archived|retired|old requirements?|previous requirements?|pre[-\s]?(?:winter|spring|summer|autumn|fall)?[-\s]?20\d{2})\b"
);
matcher!(
    historical_fallback_pattern,
    r"\b(?:archive|archived|retired|old requirements?|prior to|pre-20\d{2})\b"
);
matcher!(
    pathway_cue_pattern,
    r"(?i)\b(track|option|route|pathway|concentration|specialization)\b"
);
matcher!(
    pathway_url_pattern,
    r"(?i)(?:^|[-/])(?:track|option|route|pathway|concentration|specialization)(?:[-/?#]|$)"
);
matcher!(
    degree_program_pattern,
    r"(?i)\b(?:bachelor|bachelors?|b\.?\s*a\.?|b\.?\s*s\.?|ba|bs|bba)\b"
);
matcher!(bachelors_program_url_pattern, r"(?i)(?:^|/)b(?:a|s|ba)[-/]");
matcher!(
    requirements_program_pattern,
    r"(?i)\b(?:curriculum|requirements?|degree|major|program)\b"
);
matcher!(
    primary_requirement_parser_type_pattern,
    r"^(?:html-degree-page|html-curriculum-page|pdf-degree-sheet|pdf-worksheet|generic-html)$"
);
matcher!(
    tacoma_primary_parser_type_pattern,
    r"^(?:generic-html|html-curriculum-page|html-degree-page|html-overview-page)$"
);
matcher!(
    uwb_major_admissions_url_pattern,
    r"(?i)uwb\.edu/[^?#]*/majors/[^/?#]+/admissions/?(?:[?#].*)?$"
);
matcher!(
    uwb_mpw_registration_url_pattern,
    r"(?i)admissions\.uwb\.edu/register/mpw-"
);
matcher!(document_url_pattern, r"(?i)\.(?:pdf|docx)(?:$|[?#])");
matcher!(
    major_planning_worksheet_pattern,
    r"(?i)\bmajor planning worksheet\b"
);
matcher!(
    uw_general_catalog_program_url_pattern,
    r"(?i)//(?:www\.)?washington\.edu/students/gencat/program/"
);
matcher!(
    tacoma_set_undergrad_url_pattern,
    r"(?i)//(?:www\.)?tacoma\.uw\.edu/set/programs/undergrad/[^/?#]+/?(?:[?#].*)?$"
);
matcher!(
    tacoma_set_requirement_terms_pattern,
    r"(?i)\b(?:degree requirements?|major requirements?|program requirements?|curriculum|courses?|schedule planning|prerequisites?|admission requirements?)\b"
);
matcher!(
    degree_map_pattern,
    r"(?i)\b(?:curriculum map|degree map|four[-\s]?year plan|schedule planning grid|schedule grid|planning grid)\b"
);
matcher!(
    pathway_degree_sheet_pattern,
    r"(?i)\b(degree sheet|requirement sheet|requirements packet|checklist|worksheet|plan of study|study plan)\b|degreq"
);
matcher!(
    non_major_options_pattern,
    r"(?i)\bnon[-\s]?major options?\b|/non-major-options(?:[/?#-]|$)"
);
matcher!(
    equivalency_pattern,
    r"(?i)\bequivalenc(?:y|ies)\b|equivalency guide"
);
matcher!(
    green_river_pattern,
    r"(?i)\bgreen river\b|\bgrc\b|\bassociate\b|\bast-?2\b|\bmrp\b"
);
matcher!(
    sample_schedule_pattern,
    r"(?i)\bsample (?:schedule|plan)\b|\bstudy plan\b|\bplan of study\b"
);
matcher!(
    upper_division_prerequisite_pattern,
    r"(?i)\b(?:upper[-\s]?division|[34]00[-\s]?level|[34]00\s+level)\b.{0,80}\bprereq(?:uisites?)?\b|\bprereq(?:uisites?)?\b.{0,80}\b(?:upper[-\s]?division|[34]00[-\s]?level|[34]00\s+level)\b"
);
matcher!(
    non_schedulable_course_list_pattern,
    r"(?i)\b(course lists?|list of courses|courses by track|course descriptions?|all courses|course catalog|print courses?|suggested course pathways?|computing specializations?|capstones?)\b|/(?:courses?|course-list|course-lists|print/courses|capstones?|computing-specializations)(?:[-/?#]|$)"
);
matcher!(
    generic_degree_sheet_pattern,
    r"(?i)\bdegree sheet\b|\brequirement sheet\b|\bchecklist\b"
);
matcher!(
    admissions_preparation_pattern,
    r"\badmissions?\b|\bapply\b|\bapplication\b|\bpreparation\b|\bprerequisites?\b"
);
matcher!(
    curriculum_map_fallback_pattern,
    r"\bcurriculum(?: map)?\b|\bdegree map\b|\bfour-year plan\b"
);
matcher!(
    department_requirements_fallback_pattern,
    r"\brequirements?\b|\bundergraduate\b|\bmajor\b|\bprogram\b"
);
matcher!(degree_ba_pattern, r"\b(?:ba|b a|bachelor of arts)\b");
matcher!(degree_bs_pattern, r"\b(?:bs|b s|bachelor of science)\b");
matcher!(
    acs_pattern,
    r"\bacs(?:[-_\s]?(?:certified|\d{2,4}))?\b|\bamerican chemical society\b"
);
matcher!(
    ell_pattern,
    r"\b(?:ell|tell)\b|\benglish language learners?\b|\bteaching english language learners?\b"
);
matcher!(sped_pattern, r"\b(?:sped|special education)\b");
