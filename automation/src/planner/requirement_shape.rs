use regex::Regex;
use std::sync::OnceLock;

pub const CHOICE_COUNT_WORDS_PATTERN: &str = "one|two|three|four|five|six|seven|eight|nine|ten";

pub const CHOICE_REQUIREMENT_CONTEXT_PATTERN_SOURCE: &str = r"\b(?:choose(?:\s+(?:from|one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+))?|select(?:ed)?(?:\s+(?:one|from|one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+))?|electives?|options?|approved\s+(?:courses?|electives?|list)|elective\s+list|course\s+list|one\s+(?:course\s+)?from|one\s+of(?:\s+the\s+following)?|either)\b";

pub const STRONG_CHOICE_REQUIREMENT_CONTEXT_PATTERN_SOURCE: &str = r"\b(?:choose(?:\s+(?:from|one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+))?|select(?:ed)?(?:\s+(?:one|from|one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+))?|approved\s+(?:courses?|electives?|list)|elective\s+list|course\s+list|one\s+(?:course\s+)?from|one\s+of(?:\s+the\s+following)?|either|electives?)\b";

pub const CHOICE_REQUIREMENT_LABEL_PATTERN_SOURCE: &str = r"\b(?:programming|computing|statistics|thermodynamics|matrix|linear algebra|differential equations|biology|chemistry|physics|economics|communication|composition|science|engineering|fundamentals|mechanics|calculus|electives?|options?|approved|course\s+list|list)\b";
pub const PROGRAMMING_CHOICE_CONTEXT_PATTERN_SOURCE: &str =
    r"^(?:\[[^\]]+\]\s*)?(?:computer\s+)?(?:programming|computing)\b";
pub const CHOICE_LIST_START_PATTERN_SOURCE: &str = r"^\(?\s*(?:choose(?:\s+(?:from|one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+))?|select(?:ed)?(?:\s+(?:one|from|one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+))?|one\s+(?:course\s+)?from|one\s+of(?:\s+the\s+following)?|either|approved\s+(?:courses?|electives?|list)|elective\s+list)\b";
pub const NON_CHOICE_COURSE_LIST_CONTEXT_PATTERN_SOURCE: &str = r"\b(?:all\s+count|will\s+all\s+count|count\s+toward|counts?\s+toward|may\s+count\s+toward|may\s+be\s+(?:applied|used)\s+toward|up\s+to\s+[0-9]+(?:\.[0-9]+)?\s+credits?\b.{0,120}\b(?:applied|count|counts?|used)\b|does\s+not\s+count|not\s+count\s+as|should\s+not\s+take\s+both|may\s+request\b.{0,60}\bsubstitut|also\s+fulfills?|may\s+overlap|overlap\s+with|as\s+well\s+as|must\s+(?:include|complete)|required\s+to\s+complete|suggested\s+first|suggested\s+first-\s*and\s+second-year|sample\s+schedule|sequence|series)\b";
pub const SEQUENCE_ALTERNATIVE_CHOICE_LINE_PATTERN_SOURCE: &str = r"\b[A-Z&]+(?:\s+[A-Z&]+)?\s+[0-9]{3}[A-Z]?\s*,\s*[0-9]{3}[A-Z]?(?:\s*,\s*[0-9]{3}[A-Z]?){0,2}\s+or\s+(?:[A-Z&]+(?:\s+[A-Z&]+)?\s+)?[0-9]{3}[A-Z]?\s*,\s*[0-9]{3}[A-Z]?(?:\s*,\s*[0-9]{3}[A-Z]?){0,2}\b";
pub const SEQUENCE_CHOICE_CONTEXT_PATTERN_SOURCE: &str = r"\b(?:one\s+of\s+the\s+following(?:\s+\w+){0,4}\s+sequences?|(?:two|three)\s+quarters?\s+of\s+(?:physics|chemistry|mathematics|math|biology)\b.{0,80}\bone\s+of\s+the\s+following|choose\s+one\s+sequence|select\s+one\s+sequence|one\s+sequence|either\s+sequence|two-quarter\s+sequences?|three-quarter\s+sequences?|calculus-based\s+or\s+algebra-based|algebra-based\s+or\s+calculus-based|regular\s+or\s+accelerated\s+or\s+honors|standard\s+or\s+honors|standard\s+sequence|honors\s+sequence)\b";
pub const SEQUENCE_LABEL_ALTERNATIVE_PATTERN_SOURCE: &str = r"\b(?:calculus-based|algebra-based|regular|standard|accelerated|honors?)\b(?:\s+or\s+\b(?:calculus-based|algebra-based|regular|standard|accelerated|honors?)\b)+";
pub const SEQUENCE_HEADING_BOUNDARY_PATTERN_SOURCE: &str = r"\b(?:additional degree requirements?|minimum\s+[0-9]|gpa|option requirement|breadth|natural history|genetics|technical electives?|advanced|capstone|core courses?|required courses?|electives?|freshman year|sophomore year|junior and senior years)\b";

pub fn js_regex_compatibility_pattern(pattern: &str) -> String {
    pattern
        .replace(r"\b", r"(?-u:\b)")
        .replace(r"\d", "[0-9]")
        .replace(r"\w", "[A-Za-z0-9_]")
}

pub fn strip_snapshot_page_prefix(line: &str) -> String {
    normalize_js_whitespace(&snapshot_page_prefix_pattern().replace(line, ""))
}

pub fn strip_leading_requirement_glyphs(line: &str) -> String {
    let without_numbered_label = numbered_requirement_prefix_pattern().replace(line, "");
    let without_glyph_prefix =
        requirement_glyph_prefix_pattern().replace(&without_numbered_label, "");
    normalize_js_whitespace(&leading_bullet_pattern().replace(&without_glyph_prefix, ""))
}

pub fn looks_like_standalone_requirement_label_line(
    line: &str,
    course_codes_from_line: impl Fn(&str) -> Vec<String>,
) -> bool {
    let normalized = strip_leading_requirement_glyphs(&strip_snapshot_page_prefix(line));
    if normalized.is_empty()
        || !course_codes_from_line(&normalized).is_empty()
        || excluded_standalone_label_pattern().is_match(&normalized)
        || !credit_cue_pattern().is_match(&normalized)
    {
        return false;
    }
    standalone_requirement_domain_pattern().is_match(&normalized)
}

pub fn looks_like_standalone_requirement_title_line(
    line: &str,
    course_codes_from_line: impl Fn(&str) -> Vec<String>,
) -> bool {
    let normalized = strip_leading_requirement_glyphs(&strip_snapshot_page_prefix(line));
    if normalized.is_empty()
        || !course_codes_from_line(&normalized).is_empty()
        || simple_credit_label_pattern().is_match(&normalized)
        || excluded_standalone_label_pattern().is_match(&normalized)
    {
        return false;
    }
    standalone_requirement_domain_pattern().is_match(&normalized)
}

pub fn detect_option_cue(value: &str) -> Option<&'static str> {
    let text = normalize_js_whitespace(value);
    detect_option_cue_patterns()
        .iter()
        .find_map(|(pattern, label)| pattern.is_match(&text).then_some(*label))
}

pub fn detect_option_cue_or_none(value: &str) -> &'static str {
    detect_option_cue(value).unwrap_or("none")
}

pub fn parse_choice_required_count(value: &str) -> usize {
    let text = normalize_js_whitespace(value).to_ascii_lowercase();
    if let Some(captures) = numeric_choice_count_pattern().captures(&text) {
        return captures
            .get(1)
            .and_then(|number| number.as_str().parse::<usize>().ok())
            .filter(|number| *number > 0)
            .unwrap_or(1);
    }
    if let Some(captures) = word_choice_count_pattern().captures(&text) {
        let word = captures
            .get(1)
            .or_else(|| captures.get(2))
            .or_else(|| captures.get(3))
            .map(|value| value.as_str());
        if let Some(count) = word.and_then(word_number) {
            return count;
        }
    }
    minimum_numeric_course_count_pattern()
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .and_then(|number| number.as_str().parse::<usize>().ok())
        .filter(|number| *number > 0)
        .unwrap_or(1)
}

fn word_number(word: &str) -> Option<usize> {
    Some(match word {
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        _ => return None,
    })
}

pub(super) fn normalize_js_whitespace(value: &str) -> String {
    value
        .split(is_js_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn js_slice_utf16_units(value: &str, end: usize) -> String {
    let mut result = String::new();
    let mut units = 0usize;
    for character in value.chars() {
        let character_units = character.len_utf16();
        if units + character_units > end {
            break;
        }
        result.push(character);
        units += character_units;
    }
    result
}

pub(super) fn js_number_to_string(value: &serde_json::Number) -> String {
    value
        .as_f64()
        .map(js_number_f64_to_string)
        .unwrap_or_else(|| value.to_string())
}

pub(super) fn js_number_f64_to_string(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let negative = value.is_sign_negative();
    let raw = value.abs().to_string();
    let exponent_split = raw.find('e').or_else(|| raw.find('E'));
    let (mantissa, explicit_exponent) = exponent_split
        .map(|index| {
            (
                &raw[..index],
                raw[index + 1..].parse::<i32>().unwrap_or_default(),
            )
        })
        .unwrap_or((raw.as_str(), 0));
    let decimal_index = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let mut digits = mantissa
        .bytes()
        .filter(|digit| digit.is_ascii_digit())
        .map(char::from)
        .collect::<String>();
    let leading_zero_count = digits.bytes().take_while(|digit| *digit == b'0').count();
    digits.drain(..leading_zero_count);
    let decimal_position = decimal_index + explicit_exponent - leading_zero_count as i32;
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        return "0".to_string();
    }

    let scientific_exponent = decimal_position - 1;
    let use_scientific = value.abs() >= 1e21 || value.abs() < 1e-6;
    let mut result = if use_scientific {
        let significand = if digits.len() == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        if scientific_exponent >= 0 {
            format!("{significand}e+{scientific_exponent}")
        } else {
            format!("{significand}e{scientific_exponent}")
        }
    } else if decimal_position <= 0 {
        format!("0.{}{}", "0".repeat((-decimal_position) as usize), digits)
    } else if decimal_position as usize >= digits.len() {
        format!(
            "{}{}",
            digits,
            "0".repeat(decimal_position as usize - digits.len())
        )
    } else {
        let split_at = decimal_position as usize;
        format!("{}.{}", &digits[..split_at], &digits[split_at..])
    };
    if negative {
        result.insert(0, '-');
    }
    result
}

pub(super) fn is_js_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'
            | '\u{000a}'
            | '\u{000b}'
            | '\u{000c}'
            | '\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn snapshot_page_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^\[Page\s+[0-9]+\]\s*").expect("snapshot page prefix pattern")
    })
}
fn numbered_requirement_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"^\(\s*[0-9]+(?:\.[0-9]+)?\s*\)\s*[^A-Za-z0-9(]{1,12}\s*")
            .expect("numbered requirement prefix pattern")
    })
}
fn requirement_glyph_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"^[^A-Za-z0-9(\[]{1,12}\s*").expect("requirement glyph prefix pattern")
    })
}
fn leading_bullet_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r"^(?:(?:[•●‣▸ᐅ-]|\x{00e2}\x{20ac}\x{00a2}|\x{00e2}\x{20ac}\x{201d}\x{008f})\s*)+",
        )
        .expect("leading bullet pattern")
    })
}
fn excluded_standalone_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\b(?:students?|see|details?|list|technical electives?|provide|coursework)\b",
        ))
        .expect("standalone label exclusion pattern")
    })
}
fn credit_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:credits?|cr)\b"))
            .expect("credit cue pattern")
    })
}
fn standalone_requirement_domain_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:programming|statistics|thermodynamics|matrix|linear algebra|differential equations|biology|chemistry|physics|economics|communication|composition|science|engineering|fundamentals|mechanics|calculus)\b")).expect("standalone requirement domain pattern"))
}
fn simple_credit_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^[0-9]+(?:-[0-9]+)?\s*cr$").expect("simple credit label pattern")
    })
}
fn detect_option_cue_patterns() -> &'static [(Regex, &'static str)] {
    static VALUE: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    VALUE.get_or_init(|| {
        [
            (r"(?i)\bone\s+of\s+the\s+following(?:\s+[A-Za-z0-9_]+){0,4}\s+sequences?\b", "one of the following sequences"),
            (r"(?i)\bchoose\s+one\s+sequence\b", "choose one sequence"),
            (r"(?i)\bselect\s+one\s+sequence\b", "select one sequence"),
            (r"(?i)\b(?:calculus-based|algebra-based)\b.{0,40}\bor\b.{0,40}\b(?:calculus-based|algebra-based)\b", "sequence labels"),
            (r"(?i)\b(?:regular|standard|accelerated|honors?)\b.{0,40}\bor\b.{0,40}\b(?:regular|standard|accelerated|honors?)\b", "sequence labels"),
            (r"(?i)\bchoose\s+one\b", "choose one"),
            (r"(?i)\bchoose\s+from\b", "choose from"),
            (r"(?i)\bchoose\s+(?:one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+)\b", "choose count"),
            (r"(?i)\bchoose\b", "choose"),
            (r"(?i)\bone\s+(?:course\s+)?from\b", "one from"),
            (r"(?i)\bone\s+of(?:\s+the\s+following)?\b", "one of the following"),
            (r"(?i)\bselect\s+one\b", "select one"),
            (r"(?i)\bselect\s+from\b", "select from"),
            (r"(?i)\bselect\s+(?:one|two|three|four|five|six|seven|eight|nine|ten|[0-9]+)\b", "select count"),
            (r"(?i)\bselect\b", "select"),
            (r"(?i)\beither\b", "either"),
            (r"(?i)\b(?:approved\s+)?(?:course\s+)?alternatives?\s+to\b", "alternative to"),
            (r"(?i)\bor\b", "or"),
            (r"(?i)\bapproved\s+electives?\b", "approved elective"),
            (r"(?i)\bapproved\s+list\b", "approved list"),
            (r"(?i)\belective\s+list\b", "elective list"),
            (r"(?i)\belectives?\b", "elective"),
        ]
        .into_iter()
        .map(|(pattern, label)| {
            (
                Regex::new(&js_regex_compatibility_pattern(pattern)).expect("option cue pattern"),
                label,
            )
        })
        .collect()
    })
}
fn numeric_choice_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?-u:\b)(?:choose|select)\s+([0-9]+)(?-u:\b)")
            .expect("numeric choice count pattern")
    })
}
fn word_choice_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?-u:\b)(?:choose|select)\s+(one|two|three|four|five|six|seven|eight|nine|ten)(?-u:\b)|(?-u:\b)(one|two|three|four|five|six|seven|eight|nine|ten)\s+of(?:\s+the\s+following)?(?-u:\b)|(?-u:\b)(?:minimum|at\s+least)\s+(?:of\s+)?(one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\s+from(?-u:\b)").expect("word choice count pattern"))
}
fn minimum_numeric_course_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?-u:\b)(?:minimum|at\s+least)\s+(?:of\s+)?([0-9]+)\s+courses?\s+from(?-u:\b)")
            .expect("minimum numeric course count pattern")
    })
}
