use super::requirement_docx::is_docx_source_url;
use super::requirement_html::extract_html_links;
use super::requirement_snapshots::source_url_without_hash;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserRecoveryLinkSignals {
    pub exact_title_match: bool,
    pub title_token_overlap_count: usize,
    pub high_signal: bool,
    pub pathway_signal: bool,
    pub document_signal: bool,
    pub support_signal: bool,
    pub same_program_requirement_link: bool,
    pub same_program_specialization_link: bool,
    pub same_origin_child_or_sibling_page: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplementalDocumentLinkSignals {
    pub exact_title_match: bool,
    pub title_token_overlap_count: usize,
    pub has_document_requirement_signal: bool,
    pub title_acronym_match: bool,
    pub title_acronym_matches: Vec<String>,
    pub same_program_requirement_link: bool,
    pub historical: bool,
}

pub fn extract_supplemental_html_link_candidates(
    entry: &Value,
    links: &[(String, String)],
) -> Vec<Value> {
    let Some(entry_url) = entry.get("url").and_then(Value::as_str) else {
        return Vec::new();
    };
    let Some(base_url) = source_url_without_hash(entry_url) else {
        return Vec::new();
    };
    let (Ok(base), Ok(original_base)) = (Url::parse(&base_url), Url::parse(entry_url)) else {
        return Vec::new();
    };
    let entry_origin = base.origin().ascii_serialization();
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let title_tokens = title_scope_tokens(entry);
    let mut candidates = Vec::<(String, i32, Value)>::new();
    let mut candidate_indexes = HashMap::<String, usize>::new();

    for (href, label) in links.iter().cloned() {
        let source_link_text = format!("{label} {href}");
        if href.is_empty()
            || label.is_empty()
            || !supplemental_html_link_pattern().is_match(&source_link_text)
            || ignored_href_pattern().is_match(&href)
            || pdf_source_pattern().is_match(&href)
            || noisy_supplemental_html_label_pattern().is_match(&label)
        {
            continue;
        }
        let Ok(resolved) = original_base.join(&href) else {
            continue;
        };
        let Some(resolved_url) = source_url_without_hash(resolved.as_str()) else {
            continue;
        };
        if resolved_url == base_url || !resolved_url.starts_with(&entry_origin) {
            continue;
        }

        let link_text = format!("{label} {resolved_url}");
        let high_signal = high_signal_link_pattern().is_match(&link_text);
        let normalized_link_text = normalize_matcher_text(&link_text);
        let exact_title_match =
            !exact_title.is_empty() && normalized_link_text.contains(&exact_title);
        let title_token_overlap_count = title_tokens
            .iter()
            .filter(|token| normalized_link_text.contains(token.as_str()))
            .count();
        let same_origin_high_signal_document = is_same_origin_high_signal_requirement_document_link(
            &base_url,
            &resolved_url,
            &link_text,
        );
        let same_program_requirement_link =
            is_same_program_requirement_link(&base_url, &resolved_url, &link_text, high_signal)
                || same_origin_high_signal_document;
        let same_program_specialization_link =
            is_same_program_specialization_link(&base_url, &resolved_url, &link_text);
        if !title_tokens.is_empty()
            && !exact_title_match
            && title_token_overlap_count < 1
            && !same_program_requirement_link
            && !same_program_specialization_link
            && !same_origin_high_signal_document
        {
            continue;
        }
        if graduate_supplemental_pattern().is_match(&link_text) {
            continue;
        }
        let specialization_signal = specialization_link_pattern().is_match(&link_text);
        let degree_program_signal = low_signal_degree_program_pattern().is_match(&link_text);
        let candidate_type = if high_signal {
            Some("general")
        } else if specialization_signal || same_program_specialization_link {
            Some("specialized")
        } else if degree_program_signal {
            Some("general")
        } else {
            None
        };
        let Some(candidate_type) = candidate_type else {
            continue;
        };

        let mut score = 0;
        if exact_title_match {
            score += 20;
        }
        score += title_token_overlap_count as i32 * 6;
        if high_signal {
            score += 16;
        }
        if supplemental_prerequisite_pattern().is_match(&link_text) {
            score += 10;
        }
        if supplemental_curriculum_pattern().is_match(&link_text) {
            score += 10;
        }
        if supplemental_admission_degree_pattern().is_match(&link_text) {
            score += 10;
        }
        if specialization_signal {
            score += 7;
        }
        if degree_program_signal {
            score += 6;
        }
        if same_program_requirement_link {
            score += 12;
        }
        if same_program_specialization_link {
            score += 12;
        }
        if same_origin_high_signal_document {
            score += 12;
        }
        if supplemental_pathway_segment_pattern().is_match(&resolved_url) {
            score += 4;
        }
        let candidate = serde_json::json!({
            "url": resolved_url,
            "label": label,
            "score": score,
            "type": candidate_type,
            "sameProgramRequirementLink": same_program_requirement_link,
            "sameProgramSpecializationLink": same_program_specialization_link,
        });
        if let Some(index) = candidate_indexes.get(&resolved_url).copied() {
            if score > candidates[index].1 {
                candidates[index] = (resolved_url, score, candidate);
            }
        } else {
            candidate_indexes.insert(resolved_url.clone(), candidates.len());
            candidates.push((resolved_url, score, candidate));
        }
    }
    candidates.sort_by(|left, right| {
        right.1.cmp(&left.1).then_with(|| {
            let left_label = left
                .2
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let right_label = right
                .2
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_label
                .to_lowercase()
                .cmp(&right_label.to_lowercase())
                .then_with(|| left_label.cmp(right_label))
        })
    });
    candidates
        .into_iter()
        .take(8)
        .map(|(_, _, candidate)| candidate)
        .collect()
}

pub fn score_supplemental_document_link_candidate(
    entry: &Value,
    base_url: &str,
    resolved_url: &str,
    label: &str,
    href: &str,
    owner_acronym_tokens: &BTreeSet<String>,
) -> (i32, SupplementalDocumentLinkSignals) {
    let normalized_label = normalize_matcher_text(label);
    let normalized_link_text = normalize_matcher_text(&format!("{label} {resolved_url}"));
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let title_tokens = title_scope_tokens(entry);
    let title_token_overlap_count = title_tokens
        .iter()
        .filter(|token| normalized_link_text.contains(token.as_str()))
        .count();
    let link_text = format!("{label} {href}");
    let has_document_requirement_signal =
        document_requirement_signal_pattern().is_match(&link_text);
    let title_acronym_matches = if has_document_requirement_signal {
        let text_tokens = normalized_link_text
            .split_whitespace()
            .collect::<BTreeSet<_>>();
        owner_acronym_tokens
            .iter()
            .filter(|token| text_tokens.contains(token.as_str()))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let same_program_requirement_link = is_same_program_requirement_link(
        base_url,
        resolved_url,
        &format!("{label} {resolved_url}"),
        has_document_requirement_signal,
    );
    let historical = historical_document_label_pattern().is_match(label);
    let signals = SupplementalDocumentLinkSignals {
        exact_title_match: !exact_title.is_empty() && normalized_label.contains(&exact_title),
        title_token_overlap_count,
        has_document_requirement_signal,
        title_acronym_match: !title_acronym_matches.is_empty(),
        title_acronym_matches,
        same_program_requirement_link,
        historical,
    };
    let mut score = 0;
    if signals.exact_title_match {
        score += 30;
    }
    if signals.title_acronym_match {
        score += if signals
            .title_acronym_matches
            .iter()
            .any(|token| token.len() >= 4)
        {
            26
        } else {
            20
        };
    }
    score += signals.title_token_overlap_count as i32 * 8;
    if signals.has_document_requirement_signal {
        score += 12;
    }
    if wordpress_upload_path_pattern().is_match(resolved_url) {
        score += 6;
    }
    if signals.same_program_requirement_link {
        score += 12;
    }
    if signals.historical {
        score -= 30;
    }
    (score, signals)
}

pub fn extract_html_anchor_candidates(html: &str) -> Vec<(String, String)> {
    extract_html_links(html)
}

pub fn score_parser_recovery_link_candidate(
    entry: &Value,
    base_url: &str,
    resolved_url: &str,
    label: &str,
) -> (i32, ParserRecoveryLinkSignals) {
    let signals = parser_recovery_link_signals(entry, base_url, resolved_url, label);
    let link_text = format!("{label} {resolved_url}");
    let mut score = 0;
    if signals.exact_title_match {
        score += 30;
    }
    score += signals.title_token_overlap_count as i32 * 7;
    if signals.high_signal {
        score += 18;
    }
    if signals.document_signal {
        score += 16;
    }
    if signals.same_program_requirement_link {
        score += 16;
    }
    if signals.same_program_specialization_link {
        score += 14;
    }
    if signals.same_origin_child_or_sibling_page {
        score += 8;
    }
    if signals.pathway_signal {
        score += 8;
    }
    if signals.support_signal {
        score += 8;
    }
    if recovery_requirement_bonus_pattern().is_match(&link_text) {
        score += 8;
    }
    if recovery_requirement_path_bonus_pattern().is_match(resolved_url) {
        score += 6;
    }
    if graduate_supplemental_pattern().is_match(&link_text) {
        score -= 35;
    }
    (score, signals)
}

pub fn is_noisy_parser_recovery_link(label: &str, url: &str) -> bool {
    noisy_link_pattern().is_match(&format!("{label} {url}"))
}

pub fn is_graduate_supplemental_source(label: &str, url: &str) -> bool {
    graduate_supplemental_pattern().is_match(&format!("{label} {url}"))
}

pub fn is_ignored_parser_recovery_href(href: &str) -> bool {
    ignored_href_pattern().is_match(href)
}

pub fn parser_recovery_title_token_count(entry: &Value) -> usize {
    title_scope_tokens(entry).len()
}

pub fn parser_recovery_title_scope_tokens(entry: &Value) -> Vec<String> {
    title_scope_tokens(entry)
}

pub fn normalize_requirement_matcher_text(value: &str) -> String {
    normalize_matcher_text(value)
}

pub fn has_primary_requirement_link_cue(label: &str, url: &str) -> bool {
    primary_requirement_cue_pattern().is_match(&format!("{label} {url}"))
}

pub fn is_official_linked_asset_url(base_url: &str, resolved_url: &str) -> bool {
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    if base.origin() == resolved.origin() {
        return true;
    }
    let base_host = base.host_str().unwrap_or_default().to_ascii_lowercase();
    let resolved_host = resolved.host_str().unwrap_or_default().to_ascii_lowercase();
    let resolved_path = resolved.path().to_ascii_lowercase();
    cs_washington_host_pattern().is_match(&base_host)
        && resolved_host == "s3-us-west-2.amazonaws.com"
        && resolved_path.starts_with("/www-cse-public/")
}

pub fn has_recovery_link_signal(
    signals: &ParserRecoveryLinkSignals,
    owner_title_token_count: usize,
) -> bool {
    let has_title_token_identity =
        signals.title_token_overlap_count >= 1usize.max(2usize.min(owner_title_token_count));
    let has_owner_identity_signal = signals.exact_title_match
        || has_title_token_identity
        || signals.same_program_requirement_link
        || signals.same_program_specialization_link
        || (signals.same_origin_child_or_sibling_page && has_title_token_identity);
    has_owner_identity_signal
        || (signals.support_signal
            && (signals.same_program_requirement_link
                || signals.exact_title_match
                || signals.title_token_overlap_count > 0))
        || (signals.document_signal && has_owner_identity_signal)
}

fn parser_recovery_link_signals(
    entry: &Value,
    base_url: &str,
    resolved_url: &str,
    label: &str,
) -> ParserRecoveryLinkSignals {
    let link_text = format!("{label} {resolved_url}");
    let normalized_link_text = normalize_matcher_text(&link_text);
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let title_tokens = title_scope_tokens(entry);
    let title_token_overlap_count = title_tokens
        .iter()
        .filter(|token| normalized_link_text.contains(token.as_str()))
        .count();
    let high_signal = high_signal_link_pattern().is_match(&link_text);
    let pathway_signal = specialization_link_pattern().is_match(&link_text);
    let document_signal = pdf_source_pattern().is_match(resolved_url)
        || is_docx_source_url(resolved_url)
        || pathway_degree_sheet_pattern().is_match(&link_text);
    let support_signal = approved_course_list_pattern().is_match(&link_text)
        || elective_list_pattern().is_match(&link_text)
        || admission_prerequisite_pattern().is_match(&link_text)
        || support_link_pattern().is_match(&link_text);
    let same_program_requirement_link = is_same_program_requirement_link(
        base_url,
        resolved_url,
        &link_text,
        high_signal || document_signal,
    );
    let same_program_specialization_link =
        is_same_program_specialization_link(base_url, resolved_url, &link_text);
    let same_origin_child_or_sibling_page = same_origin_child_or_sibling(base_url, resolved_url);
    ParserRecoveryLinkSignals {
        exact_title_match: !exact_title.is_empty() && normalized_link_text.contains(&exact_title),
        title_token_overlap_count,
        high_signal,
        pathway_signal,
        document_signal,
        support_signal,
        same_program_requirement_link,
        same_program_specialization_link,
        same_origin_child_or_sibling_page,
    }
}

fn title_scope_tokens(entry: &Value) -> Vec<String> {
    let combined = format!(
        "{} {}",
        string_field(entry, "ownerTitle"),
        string_field(entry, "sourceLabel")
    );
    normalize_matcher_text(&combined)
        .split_whitespace()
        .filter(|token| token.len() >= 4 && !title_scope_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(8)
        .collect()
}

fn same_program_path_prefix(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let segments = parsed
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if let Some(index) = segments
        .iter()
        .position(|segment| credential_program_segment_pattern().is_match(segment))
    {
        return Some(format!("/{}", segments[..=index].join("/")));
    }
    let pathname = parsed.path().trim_end_matches('/');
    if same_program_terminal_segment_pattern().is_match(pathname) {
        return pathname
            .rsplit_once('/')
            .map(|(prefix, _)| prefix.to_string());
    }
    Some(pathname.to_string())
}

fn is_same_program_requirement_link(
    base_url: &str,
    resolved_url: &str,
    link_text: &str,
    high_signal: bool,
) -> bool {
    if !high_signal {
        return false;
    }
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    if base.origin() != resolved.origin() {
        return false;
    }
    let Some(base_prefix) = same_program_path_prefix(base_url) else {
        return false;
    };
    let base_prefix = base_prefix.trim_end_matches('/');
    let resolved_path = resolved.path().trim_end_matches('/');
    !base_prefix.is_empty()
        && base_prefix != "/"
        && resolved_path != base_prefix
        && resolved_path.starts_with(&format!("{base_prefix}/"))
        && high_signal_link_pattern().is_match(link_text)
}

fn is_same_origin_high_signal_requirement_document_link(
    base_url: &str,
    resolved_url: &str,
    link_text: &str,
) -> bool {
    if !is_docx_source_url(resolved_url) || !docx_study_plan_pattern().is_match(link_text) {
        return false;
    }
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    base.origin() == resolved.origin() && document_file_path_pattern().is_match(resolved.path())
}

fn is_same_program_specialization_link(
    base_url: &str,
    resolved_url: &str,
    link_text: &str,
) -> bool {
    if !specialization_link_pattern().is_match(link_text) {
        return false;
    }
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    if base.origin() != resolved.origin() {
        return false;
    }
    let Some(base_prefix) = same_program_path_prefix(base_url) else {
        return false;
    };
    let base_prefix = base_prefix.trim_end_matches('/');
    let resolved_path = resolved.path().trim_end_matches('/');
    !base_prefix.is_empty()
        && base_prefix != "/"
        && resolved_path != base_prefix
        && pathway_hub_pattern().is_match(base.path())
        && resolved_path.starts_with(&format!("{base_prefix}/"))
}

fn same_origin_child_or_sibling(base_url: &str, resolved_url: &str) -> bool {
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
    let shared_prefix = base_segments
        .iter()
        .zip(resolved_segments.iter())
        .position(|(left, right)| left != right)
        .unwrap_or(base_segments.len().min(resolved_segments.len()));
    shared_prefix >= 1usize.max(3usize.min(base_segments.len().saturating_sub(1)))
}

fn normalize_matcher_text(value: &str) -> String {
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

fn string_field(entry: &Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn title_scope_stopwords() -> BTreeSet<&'static str> {
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

macro_rules! regex {
    ($name:ident, $pattern:literal) => {
        fn $name() -> &'static Regex {
            static VALUE: OnceLock<Regex> = OnceLock::new();
            VALUE.get_or_init(|| {
                let pattern = super::requirement_shape::js_regex_compatibility_pattern($pattern);
                Regex::new(&pattern).expect(stringify!($name))
            })
        }
    };
}

regex!(
    high_signal_link_pattern,
    r"(?i)\b(curriculum|requirements?|degree requirements?|major requirements?|prerequisites?|prereq|approved electives?|elective lists?|elective webpage|course lists?|worksheet|checklist|degree map|curriculum map|schedule planning grid|schedule grid|planning grid|plan of study|program of study|study plan|sample plan)\b"
);
regex!(
    specialization_link_pattern,
    r"(?i)\b(track|option|concentration|specialization|route|pathway)\b"
);
regex!(
    pathway_degree_sheet_pattern,
    r"(?i)\b(degree sheet|requirement sheet|requirements packet|checklist|worksheet|plan of study|study plan)\b|degreq"
);
regex!(
    approved_course_list_pattern,
    r"(?i)\bapproved\b.{0,60}\b(courses?|course list|list)\b|\b(courses?|course list|list)\b.{0,60}\bapproved\b"
);
regex!(
    elective_list_pattern,
    r"(?i)\b(?:engineering|technical|departmental|major|science|natural science|approved)?\s*electives?\b.{0,50}\b(courses?|list|options?|page)\b|\b(courses?|list|options?)\b.{0,50}\belectives?\b|/electives?(?:[-/]|$)"
);
regex!(
    admission_prerequisite_pattern,
    r"(?i)\b(?:admissions?|admission|apply|application)\b.{0,80}\bprereq(?:uisites?|uisite courses?)\b|\bprereq(?:uisites?|uisite courses?)\b.{0,80}\b(?:admissions?|admission|apply|application)\b"
);
regex!(
    support_link_pattern,
    r"(?i)\b(advising|adviser|advisor|support sources?|student resources?|student support|forms?|petitions?|policies|policy[-\s]*(?:procedures?|resources?|forms?)|faq|frequently asked questions)\b"
);
regex!(
    pathway_hub_pattern,
    r"(?i)\b(curriculum|degree requirements?|major requirements?|program requirements?|tracks?|options?|routes?|pathways?|concentrations?|specializations?)\b|/(?:curriculum|requirements?|degree-requirements?|major-requirements?|tracks?|options?|routes?|pathways?|concentrations?|specializations?)(?:[/?#-]|$)"
);
regex!(
    credential_program_segment_pattern,
    r"(?i)^(?:bachelor|bachelors?|b-a|b-s|bba|basw|major)[-_]"
);
regex!(
    same_program_terminal_segment_pattern,
    r"(?i)(?:^|/)(?:admissions?|overview|curriculum|degree-requirements?|major-requirements?|requirements?|prerequisites?|checklist|worksheet|tracks?|options?|routes?|pathways?|concentrations?|specializations?)$"
);
regex!(
    recovery_requirement_bonus_pattern,
    r"(?i)\b(admissions?|degree requirements?|major requirements?|curriculum|prereq|worksheet|checklist|plan of study|program of study|study plan)\b"
);
regex!(
    recovery_requirement_path_bonus_pattern,
    r"(?i)/(?:requirements?|curriculum|degree|program|pathways?|tracks?|options?|concentrations?|worksheets?|checklists?|prereq|admissions?)(?:[-/?#]|$)"
);
regex!(
    graduate_supplemental_pattern,
    r"(?i)\b(graduate|ph\.?\s*d\.?|doctor(?:al|ate)?|m\.?\s*a\.?|masters?|master(?:'s|s)?)\b|/(?:ma|phd|graduate|masters?|amp)(?:[-/]|$)"
);
regex!(
    noisy_link_pattern,
    r"(?i)\b(apply|canvas|calendar|directory|map|myuw|library|tools|about|news|faculty|staff|contact|privacy|accessibility|terms|alumni|events|parking|transportation|research|housing|student life|jobs|visit|give|get involved|minors?|certificates?|graduate|scholarships?|course evaluations?|study abroad)\b"
);
regex!(pdf_source_pattern, r"(?i)\.pdf(?:$|[?#])");
regex!(
    cs_washington_host_pattern,
    r"(?i)(?:^|\.)cs\.washington\.edu$"
);
regex!(ignored_href_pattern, r"(?i)^(?:#|javascript:|mailto:|tel:)");
regex!(
    primary_requirement_cue_pattern,
    r"(?i)\bdegree requirements?\b|\bmajor requirements?\b|\bgraduation requirements?\b|\bprogram requirements?\b|\bdegree structure\b|\brequirements packet\b|\bdegreq\b"
);
regex!(
    docx_study_plan_pattern,
    r"(?i)\b(plan of study|program of study|study plan|sample plan)\b"
);
regex!(
    document_file_path_pattern,
    r"(?i)(?:^|/)(?:sites/default/files|files|docs?|documents?)(?:/|$)"
);
regex!(
    document_requirement_signal_pattern,
    r"(?i)\b(requirements?|degree|curriculum|worksheet|checklist|degree map|curriculum map|schedule planning grid|schedule grid|planning grid|plan of study|program of study|study plan|sample plan)\b"
);
regex!(
    historical_document_label_pattern,
    r"(?i)\b(?:prior years?|pre-|before|through|archived?|historical|old)\b|\b(?:autumn|winter|spring|summer)\s+(?:19|20)\d{2}\b"
);
regex!(wordpress_upload_path_pattern, r"(?i)/wp-content/uploads/");
regex!(
    supplemental_html_link_pattern,
    r"(?i)\b(curriculum|requirements?|degree requirements?|major requirements?|prerequisites?|prereq|approved electives?|elective lists?|elective webpage|course lists?|worksheet|checklist|degree map|curriculum map|schedule planning grid|schedule grid|planning grid|plan of study|program of study|study plan|sample plan|track|option|concentration|specialization|route|pathway|b\.?\s*a\.?|b\.?\s*s\.?|bachelor(?:'s)?|major(?: in)?|undergraduate studies?)\b"
);
regex!(
    noisy_supplemental_html_label_pattern,
    r"(?i)\b(apply|canvas|calendar|directory|map|myuw|library|tools|about|news|faculty|staff|contact|privacy|accessibility|terms|alumni|events|parking|transportation|research|housing|student life|jobs|visit|give|get involved|post graduation|minor|certificate|graduate|scholarships?|course lists?|course evaluations?|capstone courses?|study abroad|policy(?:\s*&\s*|\s+and\s+)procedures|suggested course pathways?)\b"
);
regex!(
    low_signal_degree_program_pattern,
    r"(?i)\b(?:b\.?\s*a\.?|b\.?\s*s\.?|bachelor(?:'s)?|major(?: in)?|undergraduate studies?)\b"
);
regex!(
    supplemental_prerequisite_pattern,
    r"(?i)\b(prereq|prerequisite)\b"
);
regex!(supplemental_curriculum_pattern, r"(?i)\bcurriculum\b");
regex!(
    supplemental_admission_degree_pattern,
    r"(?i)\b(admission requirements?|degree requirements?|major requirements?)\b"
);
regex!(
    supplemental_pathway_segment_pattern,
    r"(?i)(?:^|/)(?:track|tracks|option|options|concentration|specialization|pathway|route)(?:/|$|-)"
);
