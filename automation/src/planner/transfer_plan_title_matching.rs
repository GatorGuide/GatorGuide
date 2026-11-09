use super::requirement_html::decode_transfer_planner_html_entities;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const TITLE_SIGNATURE_STOPWORDS: &[&str] = &[
    "and",
    "arts",
    "baseline",
    "culture",
    "degree",
    "finish",
    "major",
    "media",
    "overall",
    "requirement",
    "requirements",
    "shared",
    "studies",
    "study",
    "the",
];

pub fn normalize_transfer_planner_text(value: &str) -> String {
    let decoded = decode_transfer_planner_html_entities(value);
    let mut separated = String::with_capacity(decoded.len());
    let mut in_dash_run = false;
    for character in decoded.chars() {
        if ('\u{2010}'..='\u{2015}').contains(&character) {
            if !in_dash_run {
                separated.push_str(" - ");
                in_dash_run = true;
            }
        } else {
            in_dash_run = false;
            separated.push(character);
        }
    }
    super::requirement_shape::normalize_js_whitespace(&separated)
}

pub fn strip_transfer_planner_plan_title_prefix_repeatedly(
    plan_title: &str,
    value: &str,
) -> String {
    let mut previous = normalize_transfer_planner_text(value);
    if previous.is_empty() {
        return String::new();
    }
    loop {
        let normalized_value = previous.to_lowercase();
        let mut next = None;
        for title_variant in normalized_plan_title_variants(plan_title) {
            if normalized_value == title_variant {
                next = Some(String::new());
                break;
            }
            for separator in [" - ", " – ", " — ", ": "] {
                let prefix = format!("{title_variant}{separator}");
                if normalized_value.starts_with(&prefix) {
                    if let Some(remainder) = previous.get(prefix.len()..) {
                        next = Some(remainder.trim().to_string());
                        break;
                    }
                }
            }
            if next.is_some() {
                break;
            }
        }
        let Some(next) = next else {
            return previous;
        };
        if next.is_empty() || next == previous {
            return if next.is_empty() { previous } else { next };
        }
        previous = next;
    }
}

pub fn normalize_transfer_planner_semantic_pathway_label(plan_title: &str, value: &str) -> String {
    let normalized = normalize_transfer_planner_text(value);
    if normalized.is_empty() {
        return String::new();
    }
    let mut cleaned = semantic_pathway_with_kind_in_pattern()
        .replace(&normalized, "$2 $1")
        .into_owned();
    cleaned = semantic_pathway_trailing_site_suffix_pattern()
        .replace(&cleaned, "")
        .into_owned();
    cleaned = semantic_pathway_document_title_suffix_pattern()
        .replace(&cleaned, "")
        .into_owned();
    cleaned = semantic_pathway_document_suffix_pattern()
        .replace(&cleaned, "")
        .into_owned();
    cleaned = semantic_pathway_requirements_suffix_pattern()
        .replace(&cleaned, "$1")
        .into_owned();
    cleaned = semantic_pathway_date_suffix_pattern()
        .replace(&cleaned, "$1")
        .into_owned();
    cleaned = semantic_pathway_requirements_for_prefix_pattern()
        .replace(&cleaned, "")
        .into_owned();
    cleaned = semantic_pathway_trailing_document_variant_pattern()
        .replace(&cleaned, "$1")
        .into_owned();
    cleaned = semantic_pathway_trailing_credits_pattern()
        .replace(&cleaned, "")
        .into_owned();
    cleaned = semantic_pathway_trailing_punctuation_pattern()
        .replace(&cleaned, "")
        .trim()
        .to_string();
    strip_transfer_planner_plan_title_prefix_repeatedly(plan_title, &cleaned)
}

pub fn build_transfer_planner_title_signature_tokens(value: &str) -> Vec<String> {
    tokenize_transfer_planner_text(value)
        .into_iter()
        .filter(|token| {
            token.chars().count() >= 4 && !TITLE_SIGNATURE_STOPWORDS.contains(&token.as_str())
        })
        .collect()
}

pub fn get_specialized_plan_title_tokens(title: &str) -> Vec<String> {
    specialized_plan_title_pattern()
        .captures(title)
        .and_then(|captures| captures.get(1))
        .map(|value| normalize_specialized_plan_tokens(value.as_str(), 4))
        .unwrap_or_default()
}

pub fn normalize_specialized_plan_tokens(value: &str, minimum_length: usize) -> Vec<String> {
    let normalized = value.to_lowercase().replace('&', " and ");
    let excluded = [
        "bachelor",
        "business",
        "administration",
        "option",
        "track",
        "route",
        "pathway",
        "certificate",
        "concentration",
    ];
    let mut tokens = Vec::new();
    let mut token = String::new();
    for character in normalized.chars() {
        if character.is_ascii_alphanumeric() {
            token.push(character);
        } else if !token.is_empty() {
            if token.len() >= minimum_length && !excluded.contains(&token.as_str()) {
                tokens.push(std::mem::take(&mut token));
            } else {
                token.clear();
            }
        }
    }
    if token.len() >= minimum_length && !excluded.contains(&token.as_str()) {
        tokens.push(token);
    }
    tokens
}

pub fn specialized_plan_tokens_match_candidate(
    plan_tokens: &[String],
    candidate_tokens: &HashSet<String>,
    candidate_acronym_tokens: &HashSet<String>,
) -> bool {
    if plan_tokens
        .iter()
        .all(|token| candidate_tokens.contains(token))
    {
        return true;
    }
    let acronym = if plan_tokens.len() >= 2 {
        plan_tokens
            .iter()
            .filter_map(|token| token.chars().next())
            .collect::<String>()
    } else {
        String::new()
    };
    !acronym.is_empty() && candidate_acronym_tokens.contains(&acronym)
}

pub fn label_mentions_different_transfer_planner_major(
    current_plan_id: &str,
    label: &str,
    titles_by_plan_id: &HashMap<String, String>,
) -> bool {
    let normalized_label = normalize_transfer_planner_text(label).to_lowercase();
    if normalized_label.is_empty() {
        return false;
    }
    let label_tokens = tokenize_transfer_planner_text(&normalized_label)
        .into_iter()
        .collect::<HashSet<_>>();
    let current_title = titles_by_plan_id
        .get(current_plan_id)
        .map(String::as_str)
        .unwrap_or_default();
    let current_plan_tokens = build_transfer_planner_title_signature_tokens(current_title)
        .into_iter()
        .collect::<HashSet<_>>();
    let current_title_variants = normalized_plan_title_variants(current_title);

    for (plan_id, title) in titles_by_plan_id {
        if plan_id.as_str() == current_plan_id {
            continue;
        }
        for title_variant in normalized_plan_title_variants(title) {
            let title_word_count = title_variant.split_whitespace().count();
            if title_word_count < 2 {
                continue;
            }
            let shares_current_family = current_title_variants.iter().any(|current_variant| {
                title_variant.as_str() == current_variant.as_str()
                    || title_variant.starts_with(&format!("{current_variant} "))
                    || current_variant.starts_with(&format!("{title_variant} "))
            });
            if shares_current_family {
                continue;
            }
            let other_tokens = build_transfer_planner_title_signature_tokens(title);
            if other_tokens
                .iter()
                .all(|token| current_plan_tokens.contains(token))
                || !other_tokens
                    .iter()
                    .any(|token| !current_plan_tokens.contains(token))
            {
                continue;
            }
            if contains_normalized_plan_title_phrase(&normalized_label, &title_variant) {
                return true;
            }
        }
    }

    if current_plan_tokens
        .iter()
        .any(|token| label_tokens.contains(token))
    {
        return false;
    }

    for (plan_id, title) in titles_by_plan_id {
        if plan_id.as_str() == current_plan_id {
            continue;
        }
        if normalize_transfer_planner_text(title).is_empty() {
            continue;
        }
        let other_tokens = build_transfer_planner_title_signature_tokens(title);
        for title_variant in normalized_plan_title_variants(title) {
            if normalized_label == title_variant
                || normalized_label.starts_with(&format!("{title_variant} - "))
                || normalized_label.starts_with(&format!("{title_variant}: "))
            {
                return true;
            }
            if normalized_label
                .strip_prefix(&title_variant)
                .is_some_and(|suffix| pathway_prefix_pattern().is_match(suffix))
            {
                return true;
            }
        }

        if other_tokens.is_empty() {
            continue;
        }
        let looks_like_standalone_pathway_label = standalone_pathway_label_pattern()
            .is_match(&normalized_label)
            && !standalone_pathway_exclusion_pattern().is_match(&normalized_label);
        let minimum_matches = if looks_like_standalone_pathway_label && other_tokens.len() == 1 {
            2
        } else {
            2.min(other_tokens.len())
        };
        let overlap_count = other_tokens
            .iter()
            .filter(|token| label_tokens.contains(*token))
            .count();
        if overlap_count >= minimum_matches {
            return true;
        }
    }
    false
}

fn tokenize_transfer_planner_text(value: &str) -> Vec<String> {
    let normalized = normalize_transfer_planner_text(value).to_lowercase();
    let mut tokens = Vec::new();
    let mut token = String::new();
    for character in normalized.chars() {
        if character.is_ascii_alphanumeric() {
            token.push(character);
        } else if !token.is_empty() {
            tokens.push(std::mem::take(&mut token));
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn normalized_plan_title_variants(title: &str) -> Vec<String> {
    let normalized = normalize_transfer_planner_text(title).to_lowercase();
    let without_trailing_parenthetical = trailing_parenthetical_pattern()
        .replace(&normalized, "")
        .into_owned();
    let mut variants = Vec::new();
    for base in [normalized, without_trailing_parenthetical] {
        if base.is_empty() {
            continue;
        }
        push_unique(&mut variants, base.clone());
        push_unique(
            &mut variants,
            communication_pattern()
                .replace_all(&base, "communications")
                .into_owned(),
        );
        push_unique(
            &mut variants,
            communications_pattern()
                .replace_all(&base, "communication")
                .into_owned(),
        );
    }
    variants
}

fn contains_normalized_plan_title_phrase(label: &str, title: &str) -> bool {
    if label.is_empty() || title.is_empty() {
        return false;
    }
    label.match_indices(title).any(|(start, matched)| {
        let end = start + matched.len();
        let before_is_boundary = label[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_ascii_alphanumeric());
        let after_is_boundary = label[end..]
            .chars()
            .next()
            .is_none_or(|character| !character.is_ascii_alphanumeric());
        before_is_boundary && after_is_boundary
    })
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

macro_rules! regex {
    ($name:ident, $pattern:literal) => {
        fn $name() -> &'static Regex {
            static VALUE: OnceLock<Regex> = OnceLock::new();
            VALUE.get_or_init(|| Regex::new($pattern).expect(stringify!($name)))
        }
    };
}

regex!(
    specialized_plan_title_pattern,
    r":\s*([^()]+?)(?:\s*\([^)]*\))?\s*$"
);
regex!(trailing_parenthetical_pattern, r"\s*\([^)]*\)\s*$");
regex!(communication_pattern, r"\bcommunication\b");
regex!(communications_pattern, r"\bcommunications\b");
regex!(
    pathway_prefix_pattern,
    r"(?i)^\s+(?:option|track|route|pathway|certificate|concentration)\b"
);
regex!(
    standalone_pathway_label_pattern,
    r"(?i)\b(?:option|track|route|pathway|certificate|concentration)\b"
);
regex!(
    standalone_pathway_exclusion_pattern,
    r"(?i)(?:\s[-–—:]\s|\|)|\bmajor\b"
);
regex!(
    semantic_pathway_document_suffix_pattern,
    r"(?i)\s*(?:\[(?:pdf|docx?|html?)\]|\((?:pdf|docx?|html?)\)|\b(?:pdf|docx?|html)\b)\s*$"
);
regex!(
    semantic_pathway_document_title_suffix_pattern,
    r"(?i)\s+(?:degree\s+program\s+sheet|program\s+sheet|degree\s+sheet|worksheet|check\s*list|checklist)\b.*$"
);
regex!(
    semantic_pathway_requirements_suffix_pattern,
    r"(?i)\b(option|track|route|pathway|certificate|concentration)\b(?:\s*[:-]\s*|\s+)(?:older\s+|prior\s+|current\s+|academic\s+|course\s+|program\s+|degree\s+|major\s+|graduation\s+)*requirements?\b.*$"
);
regex!(
    semantic_pathway_date_suffix_pattern,
    r"(?i)\b(option|track|route|pathway|certificate|concentration)\b(?:\s+(?:autumn|winter|spring|summer|fall)\s+\d{4})+(?:\s*[-–—]\s*(?:autumn|winter|spring|summer|fall)\s+\d{4})?\s*$"
);
regex!(
    semantic_pathway_trailing_site_suffix_pattern,
    r"(?i)\s+[-–—]\s+(?:UW|University of Washington)\b.*$"
);
regex!(
    semantic_pathway_with_kind_in_pattern,
    r"(?i)^(?:.*?\bwith\s+(option|track|route|pathway|certificate|concentration)\s+in\s+)(.{2,100})$"
);
regex!(
    semantic_pathway_requirements_for_prefix_pattern,
    r"(?i)^requirements?\s+for\s+(?:the\s+)?"
);
regex!(
    semantic_pathway_trailing_document_variant_pattern,
    r"(?i)\b(option|track|route|pathway|certificate|concentration)\b\s+[-â€“â€”–—]\s+.*$"
);
regex!(
    semantic_pathway_trailing_credits_pattern,
    r"(?i)\s+\((?:\d+(?:-\d+)?\s+credits?)\)\s*$"
);
regex!(
    semantic_pathway_trailing_punctuation_pattern,
    r"\s+[.;:]\s*$"
);
