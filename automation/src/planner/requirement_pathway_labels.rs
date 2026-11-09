use super::requirement_course_codes::CourseCodeParser;
use super::requirement_parse_helpers::is_noisy_line;
use super::requirement_recovery_scope::primary_major_title;
use super::transfer_plan_title_matching::{
    normalize_transfer_planner_semantic_pathway_label, normalize_transfer_planner_text,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

pub fn extract_pathway_labels(
    entry: &Value,
    lines: &[String],
    headings: &[String],
    course_parser: &CourseCodeParser,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let mut labels =
        extract_declared_pathway_section_labels(entry, lines, course_parser, primary_major_titles);
    labels.extend(headings.iter().chain(lines.iter()).filter_map(|line| {
        pathway_label_candidate(entry, line, course_parser, primary_major_titles)
    }));
    let mut labels = labels
        .into_iter()
        .filter(|label| !label.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    labels.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| right.cmp(left))
    });
    labels.into_iter().take(40).collect()
}

fn pathway_label_candidate(
    entry: &Value,
    raw_line: &str,
    course_parser: &CourseCodeParser,
    primary_major_titles: &HashMap<String, String>,
) -> Option<String> {
    let normalized = canonical_pathway_text(entry, raw_line, primary_major_titles);
    let is_degree_title = degree_title_pattern().is_match(&normalized);
    if normalized.is_empty()
        || is_noisy_line(&normalized)
        || (!is_degree_title
            && (normalized.encode_utf16().count() > 120
                || normalized.split_whitespace().count() > 14))
        || super::requirement_plan_matching::pathway_label_mentions_different_major(
            entry,
            &normalized,
            primary_major_titles,
            course_parser,
        )
        || excluded_pathway_label_pattern().is_match(&normalized)
        || additional_excluded_pathway_label_pattern().is_match(&normalized)
        || is_suspicious_pathway_label(&normalized)
    {
        return None;
    }
    let candidate = if let Some(captures) = inline_label_pattern().captures(&normalized) {
        captures
            .get(1)
            .map(|capture| capture.as_str())
            .unwrap_or(&normalized)
    } else if let Some(captures) = apply_label_pattern().captures(&normalized) {
        let subject = captures
            .get(1)
            .map(|capture| capture.as_str())
            .unwrap_or_default();
        let kind = captures
            .get(2)
            .map(|capture| capture.as_str())
            .unwrap_or_default();
        return normalize_candidate(
            entry,
            &format!("{subject} {kind}"),
            course_parser,
            primary_major_titles,
        );
    } else if let Some(captures) = degree_title_pattern().captures(&normalized) {
        captures
            .get(1)
            .map(|capture| capture.as_str())
            .unwrap_or(&normalized)
    } else {
        &normalized
    };
    if !pathway_cue_pattern().is_match(candidate) && !is_degree_title {
        return None;
    }
    normalize_candidate(entry, candidate, course_parser, primary_major_titles)
}

fn normalize_candidate(
    entry: &Value,
    raw: &str,
    course_parser: &CourseCodeParser,
    primary_major_titles: &HashMap<String, String>,
) -> Option<String> {
    let normalized = canonical_pathway_text(entry, raw, primary_major_titles);
    if normalized.is_empty()
        || super::requirement_plan_matching::pathway_label_mentions_different_major(
            entry,
            &normalized,
            primary_major_titles,
            course_parser,
        )
    {
        return None;
    }
    let label = normalized;
    if label.is_empty() || is_suspicious_pathway_label(&label) {
        None
    } else {
        Some(label)
    }
}

fn canonical_pathway_text(
    entry: &Value,
    raw: &str,
    primary_major_titles: &HashMap<String, String>,
) -> String {
    let normalized = normalize_transfer_planner_text(raw);
    if normalized.is_empty() {
        return String::new();
    }
    let major_title = primary_major_title(entry, primary_major_titles);
    let canonical = select_pathway_kind_segment(&normalized);
    let label = normalize_transfer_planner_semantic_pathway_label(&major_title, &canonical);
    normalize_transfer_planner_text(if label.is_empty() {
        &normalized
    } else {
        &label
    })
}

fn select_pathway_kind_segment(value: &str) -> String {
    let normalized = normalize_transfer_planner_text(value);
    let segments = segment_pattern()
        .split(&normalized)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    segments
        .iter()
        .rev()
        .find(|segment| {
            pathway_cue_pattern().is_match(segment)
                && !older_requirement_pattern().is_match(segment)
        })
        .map(|segment| (*segment).to_string())
        .unwrap_or(normalized)
}

fn extract_declared_pathway_section_labels(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let Some((declared_count, pathway_kind)) = declared_pathway_choice(lines) else {
        return Vec::new();
    };
    let mut labels = Vec::new();
    let mut active = false;
    for (index, line) in lines.iter().enumerate() {
        let normalized = normalize_transfer_planner_text(line);
        if normalized.is_empty() {
            continue;
        }
        if is_declared_pathway_section_context_line(&normalized, &pathway_kind) {
            active = true;
            continue;
        }
        if !active {
            continue;
        }
        if pathway_section_boundary_pattern().is_match(&normalized) {
            active = false;
            continue;
        }
        let inline_course_list = inline_course_list_pattern().captures(&normalized);
        let raw_label = if let Some(captures) = inline_course_list {
            let label = captures
                .get(1)
                .map(|capture| capture.as_str())
                .unwrap_or_default();
            let codes = captures
                .get(2)
                .map(|capture| capture.as_str())
                .unwrap_or_default();
            if course_parser.extract_from_line(codes).is_empty() {
                continue;
            }
            label.to_string()
        } else {
            let nearby_course_list = lines
                .iter()
                .skip(index + 1)
                .take(4)
                .any(|candidate| !course_parser.extract_from_line(candidate).is_empty());
            if !nearby_course_list {
                continue;
            }
            normalized.clone()
        };
        let raw_label = pathway_heading_prefix_pattern().replace(&raw_label, "");
        let raw_label = raw_label.as_ref();
        if raw_label.encode_utf16().count() > 90
            || super::requirement_shape::normalize_js_whitespace(raw_label)
                .split_whitespace()
                .count()
                > 9
            || !course_parser.extract_from_line(raw_label).is_empty()
            || section_label_exclusion_pattern().is_match(raw_label)
            || pathway_section_boundary_pattern().is_match(raw_label)
            || super::requirement_plan_matching::pathway_label_mentions_different_major(
                entry,
                raw_label,
                primary_major_titles,
                course_parser,
            )
        {
            continue;
        }
        let with_kind = if pathway_cue_pattern().is_match(raw_label) {
            raw_label.to_string()
        } else {
            format!("{raw_label} {pathway_kind}")
        };
        if let Some(label) =
            normalize_candidate(entry, &with_kind, course_parser, primary_major_titles)
        {
            labels.push(label);
            if declared_count.is_some_and(|count| labels.len() >= count) {
                break;
            }
        }
    }
    labels
}

fn is_declared_pathway_section_context_line(line: &str, pathway_kind: &str) -> bool {
    let normalized = normalize_transfer_planner_text(line);
    if normalized.is_empty() {
        return false;
    }
    if pathway_kind == "concentration" {
        return concentration_section_context_pattern().is_match(&normalized)
            || courses_applicable_concentration_context_pattern().is_match(&normalized);
    }
    let kind = regex::escape(pathway_kind);
    [
        format!(r"(?i)\b(?:choose|select)\s+(?:one\s+of\s+)?(?:(?:[0-9]+|[a-z]+)\s+)?(?:formal\s+)?{kind}s?\b"),
        format!(r"(?i)\b(?:[0-9]+|[a-z]+)\s+(?:formal\s+)?{kind}s?\b.*\bchoose\s+from\b"),
        format!(r"(?i)\b{kind}s?\b.*\bchoose\s+one\b"),
        format!(r"(?i)\b{kind}s?\b.*\b(?:courses?|requirements?|listed below)\b"),
    ]
    .iter()
    .filter_map(|pattern| {
        let compatible_pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
        Regex::new(&compatible_pattern).ok()
    })
    .any(|pattern| pattern.is_match(&normalized))
}

fn declared_pathway_choice(lines: &[String]) -> Option<(Option<usize>, String)> {
    for line in lines {
        let normalized = normalize_transfer_planner_text(line);
        if let Some(captures) = declared_choice_pattern().captures(&normalized) {
            let (numeric_group, word_group, kind_group) = if captures.get(3).is_some() {
                (1, 2, 3)
            } else if captures.get(6).is_some() {
                (4, 5, 6)
            } else {
                (7, 8, 9)
            };
            let count = captures
                .get(numeric_group)
                .and_then(|capture| capture.as_str().parse::<usize>().ok())
                .or_else(|| {
                    captures
                        .get(word_group)
                        .and_then(|capture| word_count(capture.as_str()))
                });
            if count.is_some_and(|value| !(2..=10).contains(&value)) {
                continue;
            }
            let kind = captures
                .get(kind_group)
                .map(|capture| capture.as_str())
                .unwrap_or("pathway")
                .trim_end_matches('s')
                .to_lowercase();
            return Some((count, kind));
        }
        if let Some(captures) = standalone_choice_pattern().captures(&normalized) {
            let kind = captures
                .get(1)?
                .as_str()
                .trim_end_matches('s')
                .to_lowercase();
            return Some((None, kind));
        }
    }
    None
}

fn word_count(value: &str) -> Option<usize> {
    match value.to_ascii_lowercase().as_str() {
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        _ => None,
    }
}

fn js_compatible_pathway_pattern(pattern: &str) -> Regex {
    let compatible_pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
    Regex::new(&compatible_pattern).expect("JavaScript-compatible pathway-label pattern")
}

fn pathway_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_pathway_pattern(r"(?i)\b(option|track|route|pathway|concentration)\b")
    })
}
fn degree_title_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)^(?:(?:Bachelor|Master|Doctor|Minor|Associate)(?: of [^:]{1,120})?|(?:B\.?\s*A\.?|B\.?\s*S\.?|M\.?\s*A\.?|M\.?\s*S\.?)(?: degree)?(?: with a major in [^:]{1,120})?)\s*:\s+(.{2,120})$"))
}
fn inline_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)^([^:]{1,120}\b(?:track|option|route|pathway|concentration)\b(?:\s*\([^)]{1,40}\))?)\s*:"))
}
fn apply_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)\bstudents apply(?: directly)?(?:\s+for|\s+to)?(?: the)?\s+(.{2,100}?)\s+(option|track|route|pathway|concentration)\b"))
}
fn segment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"\s+(?:[-–—])\s+|\s*:\s+|,\s+"))
}
fn older_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_pathway_pattern(r"(?i)^(?:older|prior|current)\b|^requirements?\s+for\b")
    })
}
fn excluded_pathway_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)^(?:\[?\s*supplemental official source\b|learn more|about|apply|download|click here to join|joining the|explore|how do i|planning|what is the difference)\b|\b(?:admissions?\s+pathway|current uw student admissions pathway|please check out|which is detailed at|to be considered|applicants?|double major|double degree|informal|concentration areas?|elective courses?|course lists?|courses by track)\b|\bbeyond the \d+(?:\.\d+)? credits required in the concentration area|^b\.?\s*s\.?\s+with\b|\b(?:if|for)\b.*\bconcentration\b"))
}
fn additional_excluded_pathway_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_pathway_pattern(r"(?i)\bwhich is detailed at\b|^(?:[†*§◊]+\s*)?(?:if|for)\b|^concentration\s+[ivxlcdm]+\b.*\b(?:credits?|courses?)\b|^(?:[0-9]+(?:\.[0-9]+)?\s+credits?\b.*\b)?(?:student(?:'s|’s|s)?\s+)?concentration area courses?$")
    })
}

fn is_suspicious_pathway_label(value: &str) -> bool {
    if value.starts_with('>')
        || suspicious_pathway_label_pattern().is_match(value)
        || super::source_pipeline_structural_patterns::is_suspicious_structural_pathway_label(value)
    {
        return true;
    }
    if !value.ends_with(')') {
        return false;
    }
    let open_count = value.chars().filter(|character| *character == '(').count();
    let close_count = value.chars().filter(|character| *character == ')').count();
    close_count > open_count
}

fn suspicious_pathway_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)\bclasses?\s+in\s+this\s+(?:option|track|route|pathway|certificate|concentration)\b.*\boffered\b|^the\b.*\b(?:option|track|route|pathway|certificate|concentration)\b.*\bprepare\b|\bhonou?rs?\s+thesis\s+option\b|^\d+(?:\.\d+)?\s+credits?\b.*\bconcentration area courses?$"))
}
fn concentration_section_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)\bconcentration area courses?\b"))
}
fn courses_applicable_concentration_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_pathway_pattern(
            r"(?i)\bcourses? that can be applied\b.*\bconcentration\b.*\blisted below\b",
        )
    })
}
fn pathway_section_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)\b(?:concentration|track|option|route|pathway)s?\b.*\b(?:area courses?|courses?|listed below|choose|select|customize|approved area)\b|\b(?:choose|select)\s+one\s+of\b.*\b(?:concentration|track|option|route|pathway)s?\b"))
}
fn pathway_section_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)^(?:admissions?|advising|student resources?|degree requirements?|core courses?|honou?rs?\b|recommended courses?|language courses?|career options?|share|support us|contact us|privacy|terms|site map)\b"))
}
fn section_label_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)^\[[^\]]+\]|\b(?:credits?|courses?|requirements?|offered|minimum|area other than|outside of|approved area)\b|[.;:]$"))
}
fn pathway_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)^(?:[A-Z]\.\s+|[0-9]+\)\s+)"))
}
fn inline_course_list_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"^([^:]{2,90}):\s+(.+)$"))
}
fn declared_choice_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_pathway_pattern(r"(?i)\b(?:choose|select)\s+(?:(?:one\s+of|from)\s+)?(?:(\d+)|([a-z]+))\s+(?:formal\s+)?(concentration|track|option|route|pathway)s?\b|\b(?:have|has|offer|offers|include|includes)\s+(?:(\d+)|([a-z]+))\s+(?:formal\s+)?(concentration|track|option|route|pathway)s?\s+to\s+choose\s+from\b|\b(?:(\d+)|([a-z]+))\s+(?:formal\s+)?(concentration|track|option|route|pathway)s?\s*:"))
}
fn standalone_choice_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_pathway_pattern(
            r"(?i)^\s*(concentration|track|option|route|pathway)s?\b(?:\s*\([^)]*\))*\s*$",
        )
    })
}
