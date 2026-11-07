use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementHtmlArtifacts {
    pub title: Option<String>,
    pub headings: Vec<String>,
    pub lines: Vec<String>,
    pub links: Vec<(String, String)>,
}

pub fn parse_html_artifacts(html: &str) -> RequirementHtmlArtifacts {
    let chrome_stripped = strip_html_document_chrome(html);
    RequirementHtmlArtifacts {
        title: extract_title(html),
        headings: extract_headings(html),
        lines: build_html_lines_from_stripped(&chrome_stripped),
        links: extract_html_links(html),
    }
}

pub fn extract_html_links(html: &str) -> Vec<(String, String)> {
    html_anchor_pattern()
        .captures_iter(html)
        .filter_map(|captures| {
            let href = captures
                .get(1)
                .or_else(|| captures.get(2))
                .map(|value| normalize_whitespace(value.as_str()))
                .unwrap_or_default();
            let label = captures
                .get(3)
                .map(|value| strip_html(value.as_str()))
                .unwrap_or_default();
            Some((href, label))
        })
        .collect()
}

pub fn build_html_lines(html: &str) -> Vec<String> {
    build_html_lines_from_stripped(&strip_html_document_chrome(html))
}

pub fn extract_title(html: &str) -> Option<String> {
    title_pattern()
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|value| strip_html(value.as_str()))
}

pub fn extract_headings(html: &str) -> Vec<String> {
    let html = strip_html_document_chrome(html);
    let mut headings = Vec::new();
    for pattern in heading_patterns() {
        for captures in pattern.captures_iter(&html) {
            let Some(heading) = captures.get(1).map(|value| strip_html(value.as_str())) else {
                continue;
            };
            if !heading.is_empty() {
                headings.push(heading);
            }
        }
    }
    headings.sort();
    headings.dedup();
    headings.truncate(20);
    headings
}

pub fn decode_transfer_planner_html_entities(value: &str) -> String {
    let repaired = repair_loose_html_entity_artifacts(value);
    let numeric_decoded = numeric_entity_pattern()
        .replace_all(&repaired, |captures: &regex::Captures<'_>| {
            let Some(entity) = captures.get(1).map(|value| value.as_str()) else {
                return captures.get(0).map_or_else(String::new, |value| value.as_str().to_string());
            };
            decode_numeric_entity(entity).unwrap_or_else(|| {
                captures.get(0).map_or_else(String::new, |value| value.as_str().to_string())
            })
        })
        .into_owned();
    named_entity_pattern()
        .replace_all(&numeric_decoded, |captures: &regex::Captures<'_>| {
            let Some(entity) = captures.get(1).map(|value| value.as_str().to_ascii_lowercase()) else {
                return captures.get(0).map_or_else(String::new, |value| value.as_str().to_string());
            };
            match entity.as_str() {
                "amp" => "&".to_string(),
                "apos" => "'".to_string(),
                "gt" => ">".to_string(),
                "lt" => "<".to_string(),
                "mdash" | "ndash" => "-".to_string(),
                "nbsp" => " ".to_string(),
                "quot" | "rdquo" | "ldquo" => "\"".to_string(),
                "rsquo" | "lsquo" => "'".to_string(),
                _ => captures.get(0).map_or_else(String::new, |value| value.as_str().to_string()),
            }
        })
        .replace('\u{00a0}', " ")
}

fn strip_html_document_chrome(html: &str) -> String {
    let mut output = html.to_string();
    for pattern in chrome_patterns() {
        output = pattern.replace_all(&output, " ").into_owned();
    }
    html_comment_pattern().replace_all(&output, " ").into_owned()
}

fn collapse_html_table_rows(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;
    for row in table_row_pattern().captures_iter(html) {
        let Some(whole_row) = row.get(0) else {
            continue;
        };
        let Some(row_body) = row.get(1) else {
            continue;
        };
        output.push_str(&html[cursor..whole_row.start()]);
        let mut cells = table_cell_texts(row_body.as_str());
        cells.retain(|cell| !cell.is_empty());
        let split_subject_and_number = cells.len() >= 2
            && split_subject_pattern().is_match(&cells[0])
            && leading_course_number_pattern().is_match(&cells[1]);
        if split_subject_and_number {
            output.push('\n');
            output.push_str(&cells.join(" "));
            output.push('\n');
        } else {
            output.push_str(whole_row.as_str());
        }
        cursor = whole_row.end();
    }
    output.push_str(&html[cursor..]);
    output
}

fn table_cell_texts(row_body: &str) -> Vec<String> {
    let mut cells = Vec::new();
    for pattern in table_cell_patterns() {
        for captures in pattern.captures_iter(row_body) {
            if let Some(body) = captures.get(1) {
                cells.push((captures.get(0).map_or(0, |value| value.start()), strip_html(body.as_str())));
            }
        }
    }
    cells.sort_by_key(|(position, _)| *position);
    cells.into_iter().map(|(_, text)| text).collect()
}

fn build_html_lines_from_stripped(html: &str) -> Vec<String> {
    let collapsed = collapse_html_table_rows(html);
    let line_broken = block_tag_pattern().replace_all(&collapsed, "\n");
    line_broken
        .split('\n')
        .map(strip_html)
        .filter(|line| {
            !line.is_empty()
                && !noisy_line_pattern().is_match(line)
                && !html_control_line_pattern().is_match(line)
        })
        .collect()
}

pub fn strip_html(value: &str) -> String {
    let without_comments = html_comment_pattern().replace_all(value, " ");
    let without_tags = html_tag_pattern().replace_all(&without_comments, " ");
    normalize_whitespace(&decode_transfer_planner_html_entities(&without_tags))
}

fn repair_loose_html_entity_artifacts(value: &str) -> String {
    let repaired_mojibake = value
        .replace("â€“", " - ")
        .replace("â€”", " - ")
        .replace("â€˜", "'")
        .replace("â€™", "'")
        .replace("â€œ", "\"")
        .replace("â€�", "\"");
    let mut output = String::with_capacity(repaired_mojibake.len());
    let mut cursor = 0;
    for captures in loose_entity_artifact_pattern().captures_iter(&repaired_mojibake) {
        let (Some(whole), Some(marker), Some(code)) =
            (captures.get(0), captures.get(1), captures.get(2))
        else {
            continue;
        };
        let before = repaired_mojibake[..whole.start()].chars().next_back();
        let after = repaired_mojibake[whole.end()..].chars().next();
        let boundary_before = before.map_or(true, |character| {
            character.is_whitespace() || "([{,;:".contains(character)
        });
        let boundary_after = after.map_or(true, |character| {
            character.is_whitespace() || ")]},;:!?".contains(character)
        });
        if !boundary_before
            || !boundary_after
            || (code.as_str() == "160" && marker.as_str().is_empty())
        {
            continue;
        }
        output.push_str(&repaired_mojibake[cursor..whole.start()]);
        let replacement = match code.as_str() {
            "160" => " ",
            "8211" | "8212" => " - ",
            "8216" | "8217" => "'",
            "8220" | "8221" => "\"",
            _ => code.as_str(),
        };
        output.push_str(replacement);
        cursor = whole.end();
    }
    output.push_str(&repaired_mojibake[cursor..]);
    output
}

fn decode_numeric_entity(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let (digits, radix) = if let Some(value) = normalized.strip_prefix("#x") {
        (value, 16)
    } else if let Some(value) = normalized.strip_prefix('#') {
        (value, 10)
    } else {
        return None;
    };
    let code_point = u32::from_str_radix(digits, radix).ok()?;
    char::from_u32(code_point).map(|value| value.to_string())
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn chrome_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        ["script", "style", "nav", "header", "footer", "aside"]
            .into_iter()
            .map(|tag| {
                Regex::new(&format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}\s*>"))
                    .expect("HTML chrome pattern")
            })
            .collect()
    })
}

fn table_row_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?is)<tr\b[^>]*>(.*?)</tr\s*>").expect("HTML table row pattern"))
}
fn table_cell_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        ["td", "th"]
            .into_iter()
            .map(|tag| Regex::new(&format!(r"(?is)<{tag}\b[^>]*>(.*?)</{tag}\s*>" )).expect("HTML table cell pattern"))
            .collect()
    })
}
fn split_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[A-Za-z&]{1,20}(?:\s+[A-Za-z&]{1,20})?$").expect("split course subject pattern"))
}
fn leading_course_number_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\d{3}[A-Za-z]?\b").expect("split course number pattern"))
}
fn block_tag_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)</?(?:p|div|section|article|main|aside|header|footer|details|summary|nav|li|ul|ol|dl|dt|dd|table|thead|tbody|tfoot|caption|tr|td|th|button|h1|h2|h3|h4|h5|h6|br)\b[^>]*>").expect("HTML block tag pattern"))
}
fn title_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?is)<title\b[^>]*>(.*?)</title\s*>").expect("HTML title pattern"))
}
fn heading_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        (1..=4)
            .map(|level| Regex::new(&format!(r"(?is)<h{level}\b[^>]*>(.*?)</h{level}\s*>" )).expect("HTML heading pattern"))
            .collect()
    })
}
fn html_anchor_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r#"(?is)<a\b[^>]*href=(?:"([^"]+)"|'([^']+)')[^>]*>(.*?)</a>"#)
            .expect("html anchor")
    })
}

fn html_comment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").expect("HTML comment pattern"))
}
fn html_tag_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"<[^>]+>").expect("HTML tag pattern"))
}
fn html_control_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:Expand\s+All\s*(?:\|\s*)?Collapse\s+All|Collapse\s+All\s*(?:\|\s*)?Expand\s+All|Expand\s+All|Collapse\s+All)$").expect("HTML control line pattern"))
}
fn noisy_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$))").expect("noisy source line pattern"))
}
fn numeric_entity_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)&(\#x[0-9a-f]+|\#\d+);").expect("numeric HTML entity pattern"))
}
fn named_entity_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)&([a-z]+);").expect("named HTML entity pattern"))
}
fn loose_entity_artifact_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)((?:and\s+|amp\s+|&\s*|#\s*)?)(160|8211|8212|8216|8217|8220|8221)").expect("loose HTML entity artifact pattern"))
}
