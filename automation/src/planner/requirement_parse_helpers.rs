use super::requirement_course_codes::CourseCodeParser;
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;

pub fn extract_relevant_requirement_lines(lines: &[String], headings: &[String]) -> Vec<String> {
    let normalized_headings = headings
        .iter()
        .map(|heading| normalize_whitespace(heading))
        .collect::<HashSet<_>>();
    let mut relevant_line_indexes = BTreeSet::new();
    for (index, raw_line) in lines.iter().enumerate() {
        let line = normalize_whitespace(raw_line);
        if line.is_empty() {
            continue;
        }
        let has_sectioned_course_list_cue =
            sectioned_course_list_requirement_pattern().is_match(&line);
        let is_transfer_noise = is_transfer_credit_noise_only_line(&line);
        if is_transfer_noise
            || normalized_headings.contains(&line)
            || requirement_cue_pattern().is_match(&line)
            || requirement_friendly_hint_pattern().is_match(&line)
            || structural_requirement_pattern().is_match(&line)
            || has_sectioned_course_list_cue
        {
            if is_transfer_noise {
                continue;
            }
            let forward_window = if has_sectioned_course_list_cue {
                90
            } else if structural_requirement_pattern().is_match(&line)
                || requirement_friendly_hint_pattern().is_match(&line)
            {
                14
            } else if requirement_credit_from_pattern().is_match(&line) {
                10
            } else {
                5
            };
            let start_index = index.saturating_sub(1);
            let end_index = index.saturating_add(forward_window).min(lines.len() - 1);
            relevant_line_indexes.extend(start_index..=end_index);
        }
    }
    if relevant_line_indexes.is_empty() {
        return lines.iter().take(240).cloned().collect();
    }
    relevant_line_indexes
        .into_iter()
        .filter_map(|index| lines.get(index))
        .filter(|line| {
            !is_noisy_line(line) && !is_transfer_credit_noise_only_line(&normalize_whitespace(line))
        })
        .cloned()
        .collect()
}

pub fn extract_requirement_cue_lines(
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut cues = Vec::new();
    let mut seen = HashSet::new();
    for (index, raw_line) in lines.iter().enumerate() {
        let line = normalize_whitespace(raw_line);
        if line.is_empty()
            || is_noisy_line(&line)
            || transfer_credit_noise_pattern().is_match(&line)
        {
            continue;
        }
        let direct_cue = requirement_cue_pattern().is_match(&line)
            || general_education_cue_pattern().is_match(&line);
        let contextual_course_list_cue = !course_parser.extract_from_line(&line).is_empty()
            && has_requirement_context_near_line(lines, index, 8);
        if (direct_cue || contextual_course_list_cue) && seen.insert(line.clone()) {
            cues.push(line);
            if cues.len() >= 80 {
                break;
            }
        }
    }
    cues
}

pub fn extract_choose_statements(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| {
            choose_statement_pattern().is_match(line) || choose_count_pattern().is_match(line)
        })
        .take(20)
        .cloned()
        .collect()
}

pub fn parse_requirement_credit_amount(value: &str) -> Option<f64> {
    let text = normalize_whitespace(value);
    if let Some(captures) = credit_word_parenthetical_pattern().captures(&text) {
        return captures
            .get(2)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .filter(|credits| credits.is_finite())
            .or_else(|| {
                captures
                    .get(1)
                    .and_then(|word| requirement_word_number(word.as_str()))
            });
    }
    if let Some(captures) = credit_numeric_pattern().captures(&text) {
        return captures
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .filter(|credits| credits.is_finite());
    }
    credit_word_pattern()
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .and_then(|word| requirement_word_number(word.as_str()))
}

pub fn parse_requirement_credit_range(value: &str) -> Option<(f64, Option<f64>)> {
    let text = normalize_whitespace(value);
    if text.is_empty() {
        return None;
    }
    if let Some(captures) = credit_word_parenthetical_pattern().captures(&text) {
        let credits = captures
            .get(2)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .filter(|credits| credits.is_finite() && *credits > 0.0)
            .or_else(|| {
                captures
                    .get(1)
                    .and_then(|word| requirement_word_number(word.as_str()))
                    .filter(|credits| *credits > 0.0)
            });
        if let Some(credits) = credits {
            return Some((credits, Some(credits)));
        }
    }
    for (pattern, requires_credit_cue) in [
        (credit_range_before_unit_pattern(), false),
        (credit_range_after_unit_pattern(), false),
        (parenthetical_credit_range_pattern(), true),
    ] {
        let Some(captures) = pattern.captures(&text) else {
            continue;
        };
        if requires_credit_cue && !credit_cue_pattern().is_match(&text) {
            continue;
        }
        let min = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        let max = captures
            .get(2)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        if let (Some(min), Some(max)) = (min, max) {
            if min.is_finite() && max.is_finite() && min > 0.0 && max >= min {
                return Some((min, Some(max)));
            }
        }
    }
    if let Some(captures) = minimum_credit_pattern().captures(&text) {
        if let Some(min) = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .filter(|min| min.is_finite() && *min > 0.0)
        {
            return Some((min, None));
        }
    }
    if category_option_pattern().is_match(&text) {
        if let Some(captures) = category_credit_pattern().captures(&text) {
            if let Some(credits) = captures
                .get(1)
                .and_then(|value| value.as_str().parse::<f64>().ok())
                .filter(|credits| credits.is_finite() && *credits > 0.0)
            {
                return Some((credits, Some(credits)));
            }
        }
    }
    if let Some(captures) = fixed_credit_pattern().captures(&text) {
        if let Some(credits) = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .filter(|credits| credits.is_finite() && *credits > 0.0)
        {
            return Some((credits, Some(credits)));
        }
    }
    if let Some(captures) = hyphen_credit_pattern().captures(&text) {
        if let Some(credits) = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .filter(|credits| credits.is_finite() && *credits > 0.0)
        {
            return Some((credits, Some(credits)));
        }
    }
    if category_option_pattern().is_match(&text) {
        if let Some(captures) = parenthetical_credit_amount_pattern().captures(&text) {
            if let Some(credits) = captures
                .get(1)
                .and_then(|value| value.as_str().parse::<f64>().ok())
                .filter(|credits| credits.is_finite() && *credits > 0.0)
            {
                return Some((credits, Some(credits)));
            }
        }
    }
    None
}

pub fn parse_inline_minimum_course_count(value: &str) -> Option<u64> {
    let text = normalize_whitespace(value).to_lowercase();
    if let Some(count) = inline_minimum_numeric_course_count_pattern()
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .and_then(|capture| capture.as_str().parse::<u64>().ok())
        .filter(|count| *count > 0)
    {
        return Some(count);
    }
    inline_minimum_word_course_count_pattern()
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .and_then(|word| requirement_word_number(word.as_str()))
        .map(|count| count as u64)
}

pub fn get_inline_minimum_course_list_text(value: &str) -> String {
    let text = normalize_whitespace(value);
    inline_minimum_course_list_pattern()
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .map(|list| normalize_whitespace(list.as_str()))
        .unwrap_or_default()
}

pub fn parse_requirement_course_count(value: &str) -> Option<u64> {
    let text = normalize_whitespace(value).to_lowercase();
    if let Some(captures) = course_count_numeric_pattern().captures(&text) {
        return captures
            .get(1)
            .and_then(|count| count.as_str().parse::<u64>().ok())
            .filter(|count| *count > 0);
    }
    course_count_word_pattern()
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .and_then(|word| requirement_word_number(word.as_str()))
        .map(|number| number as u64)
}

fn requirement_word_number(word: &str) -> Option<f64> {
    match word.to_ascii_lowercase().as_str() {
        "one" => Some(1.0),
        "two" => Some(2.0),
        "three" => Some(3.0),
        "four" => Some(4.0),
        "five" => Some(5.0),
        "six" => Some(6.0),
        "seven" => Some(7.0),
        "eight" => Some(8.0),
        "nine" => Some(9.0),
        "ten" => Some(10.0),
        _ => None,
    }
}

fn inline_minimum_numeric_course_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"(?i)\b(?:minimum|at\s+least)\s+(?:of\s+)?(\d+)\s+courses?\s+from\b",
        )
    })
}
fn inline_minimum_word_course_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_parse_pattern(r"(?i)\b(?:minimum|at\s+least)\s+(?:of\s+)?(one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\s+from\b"))
}
fn inline_minimum_course_list_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_parse_pattern(r"(?i)\b(?:minimum|at\s+least)\s+(?:of\s+)?(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\s+from\s+(.+)$"))
}
fn credit_word_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(one|two|three|four|five|six|seven|eight|nine|ten)\s*\(\s*(\d+(?:\.\d+)?)\s*\)\s*(?:credits?|cr)\b")
    })
}

fn credit_numeric_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_parse_pattern(r"(?i)\b(\d+(?:\.\d+)?)\s*(?:credits?|cr)\b"))
}

fn credit_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"(?i)\b(one|two|three|four|five|six|seven|eight|nine|ten)\s+(?:credits?|cr)\b",
        )
    })
}

fn credit_range_before_unit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(\d+(?:\.\d+)?)\s*(?:-|–|—|to)\s*(\d+(?:\.\d+)?)\s*(?:additional\s+)?(?:credits?|cr)\b")
    })
}

fn credit_range_after_unit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(\d+(?:\.\d+)?)\s*(?:additional\s+)?(?:credits?|cr)\s*(?:-|–|—|to)\s*(\d+(?:\.\d+)?)\s*(?:credits?|cr)?\b")
    })
}

fn parenthetical_credit_range_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"(?i)\(\s*(\d+(?:\.\d+)?)\s*(?:-|–|—|to)\s*(\d+(?:\.\d+)?)\s*\)",
        )
    })
}

fn credit_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_parse_pattern(r"(?i)\b(?:credits?|cr)\b"))
}

fn minimum_credit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"(?i)\b(?:minimum(?:\s+of)?|at\s+least|min\.?)\s+(\d+(?:\.\d+)?)\s*(?:credits?|cr)\b",
        )
    })
}

fn category_credit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"(?i)\b(\d+(?:\.\d+)?)\s+(?:additional\s+)?(?:[A-Za-z&]+\s+){0,5}credits?\b",
        )
    })
}

fn fixed_credit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(\d+(?:\.\d+)?)\s*(?:additional\s+)?(?:credits?|cr)\b")
    })
}

fn hyphen_credit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_parse_pattern(r"(?i)\b(\d+(?:\.\d+)?)\s*-\s*credits?\b"))
}

fn parenthetical_credit_amount_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_parse_pattern(r"\(\s*(\d+(?:\.\d+)?)\s*\)"))
}

fn category_option_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)(?:\b(?:N\s*Sc|natural sciences?|natural science)\b|\b(?:A\s*&\s*H|arts?\s+(?:and|&)\s+humanities|humanities|fine arts?)\b|\b(?:S\s*Sc|social sciences?)\b|\bQSR\b|\bquantitative and symbolic reasoning\b|\bVLPA\b|\bvisual,\s*literary,\s*and performing arts\b|\bDIV\b|\bDiversity\s+Requirement\b|\b(?:\d+(?:\.\d+)?\s+(?:credits?\s+of\s+)?Diversity|Diversity\s*:\s*\d|Diversity\s+credits?)\b|\bDiversity\s*\([^)]*\)|\bNW\b|\bnatural world\b|\bI\s*&\s*S\b|\bindividuals\s+and\s+societies\b)")
    })
}

fn course_count_numeric_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_parse_pattern(r"\b(\d+)\s+courses?\b"))
}

fn course_count_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"\b(one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\b",
        )
    })
}

pub fn build_parse_confidence(
    parsed_course_code_count: usize,
    requirement_cue_line_count: usize,
    parser_type: &str,
) -> &'static str {
    if parsed_course_code_count >= 8
        || (parsed_course_code_count >= 4 && requirement_cue_line_count >= 4)
    {
        "high"
    } else if parsed_course_code_count >= 2
        || requirement_cue_line_count >= 4
        || matches!(parser_type, "pdf-degree-sheet" | "pdf-worksheet")
    {
        "medium"
    } else {
        "low"
    }
}

fn is_transfer_credit_noise_only_line(line: &str) -> bool {
    if !transfer_credit_noise_pattern().is_match(line) {
        return false;
    }
    let Some(noise_match) = transfer_credit_noise_pattern().find(line) else {
        return true;
    };
    let before_noise = line[..noise_match.start()].trim_end_matches(|character: char| {
        character.is_whitespace() || ";,:()".contains(character)
    });
    !explicit_course_code_pattern().is_match(before_noise)
}

fn has_requirement_context_near_line(lines: &[String], index: usize, radius: usize) -> bool {
    let start = index.saturating_sub(radius);
    let end = index
        .saturating_add(radius)
        .min(lines.len().saturating_sub(1));
    if lines.is_empty() {
        return false;
    }
    (start..=end).any(|context_index| {
        let line = normalize_whitespace(&lines[context_index]);
        !line.is_empty()
            && (structural_requirement_pattern().is_match(&line)
                || requirement_cue_pattern().is_match(&line)
                || requirement_friendly_hint_pattern().is_match(&line)
                || course_cluster_context_pattern().is_match(&line))
    })
}

pub fn is_noisy_line(line: &str) -> bool {
    noisy_line_pattern().is_match(line)
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn js_compatible_parse_pattern(pattern: &str) -> Regex {
    let compatible_pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
    Regex::new(&compatible_pattern).expect("JavaScript-compatible requirement-parse pattern")
}

fn explicit_course_code_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"\b([A-Z&][A-Za-z&]{0,19}(?:\s+[A-Z&][A-Za-z&]{0,19})?)\s*(\d{3}[A-Za-z]?)\b",
        )
    })
}

fn sectioned_course_list_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(?:selected from|from the following list|both courses?|list of approved courses|approved list of courses|courses? listed below|listed below are required|following departments|technical electives?|engineering fundamentals electives?|science electives?|math elective|core and elective requirements?)\b")
    })
}

fn requirement_credit_from_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b\d+\s+(?:credits?|cr\.?)(?:\s*,?\s*from)?\b")
    })
}

fn noisy_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$)))")
    })
}

fn transfer_credit_noise_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(transfer credits?|transfer equivalenc(?:y|ies)|articulation agreement|community colleges?|community college)\b")
    })
}

fn requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(required|requirements|prereq|prerequisite|complete|credits|credit|elective|select|choose|one of the following|two of the following|option|track|route|pathway|concentration)\b")
    })
}

fn general_education_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(areas of inquiry|arts?\s+(?:and|&)\s+humanities|social sciences?|natural sciences?|a&h|ssc|nsc|diversity|additional a&h|additional areas? of inquiry|additional coursework)\b")
    })
}

fn structural_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(admission requirements|degree requirements|major requirements|completion requirements|required courses|elective courses|core courses|core requirements|curriculum|prerequisite courses|shared set of core courses|specialization|track|option|route|pathway|concentration|checklist|foundation|distribution requirement)\b")
    })
}

fn course_cluster_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(model program of study|program of study|curriculum overview|curriculum plan|core curriculum|foundation courses|practice courses|required curriculum|graduation requirements|social welfare electives|upper-division .*electives)\b")
    })
}

fn requirement_friendly_hint_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(required|requirements?|prereq|prerequisite|complete|completed|admission|degree requirements?|credits?|engineering fundamentals|mathematics|sciences|written\s*&\s*oral communication|english composition|areas of inquiry|choose from the following|select one sequence|prior to the start of|before the start of|continuation requirements?)\b")
    })
}

fn choose_statement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(
            r"(?i)\b(choose|select|one of the following|two of the following)\b",
        )
    })
}

fn choose_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_parse_pattern(r"(?i)\b(?:two|three|four|five|six|seven|eight|nine|ten|\d+)\s+(?:formal\s+)?(?:concentrations?|tracks?|options?|routes?|pathways?)\b\s*:")
    })
}
