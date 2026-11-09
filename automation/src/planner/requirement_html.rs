use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementHtmlArtifacts {
    pub html: String,
    pub title: Option<String>,
    pub headings: Vec<String>,
    pub lines: Vec<String>,
    pub links: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogAnchorMatch {
    pub index: usize,
    pub end_index: usize,
    pub tag_html: String,
    pub heading: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPeerBoundary {
    pub index: usize,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProgramExpandableGroupMatch {
    pub index: usize,
    pub end_index: usize,
    pub data_expand: String,
    pub stop_search_start: usize,
}

pub fn find_next_catalog_peer_boundary(
    html: &str,
    search_start_index: usize,
    anchor: Option<&str>,
) -> CatalogPeerBoundary {
    let mut search_start_index = search_start_index.min(html.len());
    while !html.is_char_boundary(search_start_index) {
        search_start_index -= 1;
    }
    let pattern = if anchor.is_some_and(|anchor| {
        anchor
            .get(..11)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("credential-"))
    }) {
        credential_catalog_peer_boundary_pattern()
    } else {
        program_catalog_peer_boundary_pattern()
    };
    let Some(found) = pattern.find_at(html, search_start_index) else {
        return CatalogPeerBoundary {
            index: html.len(),
            label: "end of document".to_string(),
        };
    };
    let matched = found.as_str();
    let stripped = strip_html(matched);
    let label = if stripped.is_empty() {
        super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(matched), 120)
    } else {
        stripped
    };
    CatalogPeerBoundary {
        index: found.start(),
        label,
    }
}

pub fn find_next_catalog_credential_boundary(
    html: &str,
    search_start_index: usize,
    search_end_index: usize,
) -> Option<CatalogPeerBoundary> {
    let mut search_start_index = search_start_index.min(html.len());
    while !html.is_char_boundary(search_start_index) {
        search_start_index -= 1;
    }
    let found = catalog_credential_boundary_pattern().find_at(html, search_start_index)?;
    if found.start() >= search_end_index {
        return None;
    }
    let matched = found.as_str();
    let stripped = strip_html(matched);
    let label = if stripped.is_empty() {
        super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(matched), 120)
    } else {
        stripped
    };
    Some(CatalogPeerBoundary {
        index: found.start(),
        label,
    })
}

pub fn collect_ignored_catalog_neighboring_sections(
    html: &str,
    search_start_index: usize,
    anchor: Option<&str>,
) -> Vec<String> {
    let mut search_start_index = search_start_index.min(html.len());
    while !html.is_char_boundary(search_start_index) {
        search_start_index -= 1;
    }
    let pattern = if anchor.is_some_and(|anchor| {
        anchor
            .get(..11)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("credential-"))
    }) {
        credential_catalog_heading_pattern()
    } else {
        program_catalog_heading_pattern()
    };
    let mut headings = Vec::new();
    for captures in pattern.captures_iter(html) {
        let Some(whole) = captures.get(0) else {
            continue;
        };
        if whole.start() < search_start_index {
            continue;
        }
        let Some(heading) = captures.get(1).map(|value| strip_html(value.as_str())) else {
            continue;
        };
        if !heading.is_empty() {
            headings.push(heading);
        }
        if headings.len() >= 6 {
            break;
        }
    }
    headings
}

fn credential_catalog_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = r#"(?is)<h[3-5]\b[^>]*\bid=(?:"credential-[^"']+"|'credential-[^"']+')[^>]*>(.*?)</h[3-5]>"#;
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
        Regex::new(&pattern).expect("catalog credential heading pattern")
    })
}

fn program_catalog_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            r#"(?is)<h[23]\b[^>]*\bid=(?:"program-[^"']+"|'program-[^"']+')[^>]*>(.*?)</h[23]>"#;
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
        Regex::new(&pattern).expect("catalog program heading pattern")
    })
}

pub fn normalize_catalog_base_credential_heading(heading: &str) -> String {
    let mut normalized =
        super::transfer_plan_title_matching::normalize_transfer_planner_text(heading);
    for pattern in catalog_base_credential_prefix_patterns() {
        normalized = pattern.replace(&normalized, "").into_owned();
    }
    normalized
        .trim_matches(super::requirement_shape::is_js_whitespace)
        .to_string()
}

fn catalog_base_credential_prefix_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        [
            r"(?i)^Bachelor\s+of\s+(?:Arts|Science)\s+degree\s+with\s+a\s+major\s+in\s+",
            r"(?i)^Bachelor\s+of\s+[^:]+?\s+degree\s+with\s+a\s+major\s+in\s+",
            r"(?i)^Bachelor\s+of\s+(?:Arts|Science)\s+in\s+",
            r"(?i)^Bachelor\s+of\s+[^:]+?\s+in\s+",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("catalog base credential prefix pattern"))
        .collect()
    })
}

fn catalog_credential_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = r#"(?is)<div\b[^>]*\bclass=(?:"[^"']*\bexpandableGroup\b[^"']*"|'[^"']*\bexpandableGroup\b[^"']*')[^>]*\bdata-expand=(?:"[^"']*credential-[^"']+"|'[^"']*credential-[^"']+')[^>]*>.*?</div>"#;
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
        Regex::new(&pattern).expect("catalog credential boundary pattern")
    })
}

fn catalog_program_expandable_group_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = r#"(?i)<div\b[^>]*\bclass=(?:"[^"']*\bexpandableGroup\b[^"']*"|'[^"']*\bexpandableGroup\b[^"']*')[^>]*\bdata-expand=(?:"([^"']*program-[^"']+)"|'([^"']*program-[^"']+)')[^>]*>"#;
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
        Regex::new(&pattern).expect("catalog expandable program group pattern")
    })
}

fn catalog_undergraduate_program_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bprogram-UG-",
        ))
        .expect("catalog undergraduate program pattern")
    })
}

fn program_catalog_peer_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = r#"(?is)<(?:(?:h[23]\b[^>]*\bid=(?:"program-[^"']+"|'program-[^"']+')[^>]*>.*?</h[23]>)|(?:div\b[^>]*\bclass=(?:"[^"']*\bexpandableGroup\b[^"']*"|'[^"']*\bexpandableGroup\b[^"']*')[^>]*\bdata-expand=(?:"[^"']*program-[^"']+"|'[^"']*program-[^"']+')[^>]*>))"#;
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
        Regex::new(&pattern).expect("catalog program peer boundary pattern")
    })
}

fn credential_catalog_peer_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = r#"(?is)<(?:(?:h[23]\b[^>]*\bid=(?:"program-[^"']+"|'program-[^"']+')[^>]*>.*?</h[23]>)|(?:h[3-5]\b[^>]*\bid=(?:"credential-[^"']+"|'credential-[^"']+')[^>]*>.*?</h[3-5]>)|(?:div\b[^>]*\bclass=(?:"[^"']*\bexpandableGroup\b[^"']*"|'[^"']*\bexpandableGroup\b[^"']*')[^>]*\bdata-expand=(?:"[^"']*(?:program|credential)-[^"']+"|'[^"']*(?:program|credential)-[^"']+')[^>]*>))"#;
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
        Regex::new(&pattern).expect("catalog credential peer boundary pattern")
    })
}

pub fn find_catalog_credential_heading_matches(html: &str) -> Vec<CatalogAnchorMatch> {
    credential_catalog_heading_pattern()
        .captures_iter(html)
        .filter_map(|captures| {
            let whole = captures.get(0)?;
            let heading = captures
                .get(1)
                .map(|value| strip_html(value.as_str()))
                .unwrap_or_default();
            Some(CatalogAnchorMatch {
                index: whole.start(),
                end_index: whole.end(),
                tag_html: whole.as_str().to_string(),
                heading,
            })
        })
        .collect()
}

pub fn find_catalog_program_expandable_group_matches(
    html: &str,
) -> Vec<CatalogProgramExpandableGroupMatch> {
    let mut matches = Vec::new();
    for captures in catalog_program_expandable_group_pattern().captures_iter(html) {
        let Some(whole) = captures.get(0) else {
            continue;
        };
        let data_expand = captures
            .get(1)
            .or_else(|| captures.get(2))
            .map(|value| value.as_str())
            .unwrap_or_default();
        if !catalog_undergraduate_program_pattern().is_match(data_expand) {
            continue;
        }
        let header_end = html
            .get(whole.end()..)
            .and_then(|remaining| remaining.find("</div>"))
            .map(|relative_end| whole.end() + relative_end);
        let stop_search_start = header_end
            .map(|end| end + "</div>".len())
            .unwrap_or(whole.end());
        matches.push(CatalogProgramExpandableGroupMatch {
            index: whole.start(),
            end_index: whole.end(),
            data_expand: data_expand.to_string(),
            stop_search_start,
        });
    }
    matches
}

pub fn find_catalog_anchor_tag(html: &str, anchor: &str) -> Option<CatalogAnchorMatch> {
    if anchor.is_empty() {
        return None;
    }
    let escaped_anchor = regex::escape(anchor);
    let exact_attribute_pattern =
        format!(r#"(?i)<[^>]+\b(?:id|name)=(?:"{escaped_anchor}"|'{escaped_anchor}')[^>]*>"#);
    let exact_attribute_pattern =
        super::requirement_shape::js_regex_compatibility_pattern(&exact_attribute_pattern);
    if let Ok(pattern) = Regex::new(&exact_attribute_pattern) {
        if let Some(found) = pattern.find(html) {
            return Some(CatalogAnchorMatch {
                index: found.start(),
                end_index: found.end(),
                tag_html: found.as_str().to_string(),
                heading: strip_html(found.as_str()),
            });
        }
    }

    let data_expand_pattern = format!(
        r#"(?i)<[^>]+\bdata-expand=(?:"[^"']*{escaped_anchor}[^"']*"|'[^"']*{escaped_anchor}[^"']*')[^>]*>"#
    );
    let data_expand_pattern =
        super::requirement_shape::js_regex_compatibility_pattern(&data_expand_pattern);
    let pattern = Regex::new(&data_expand_pattern).ok()?;
    let found = pattern.find(html)?;
    Some(CatalogAnchorMatch {
        index: found.start(),
        end_index: found.end(),
        tag_html: found.as_str().to_string(),
        heading: strip_html(found.as_str()),
    })
}

pub fn parse_html_artifacts(html: &str) -> RequirementHtmlArtifacts {
    let chrome_stripped = strip_html_document_chrome(html);
    RequirementHtmlArtifacts {
        html: html.to_string(),
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
    headings.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| right.cmp(left))
    });
    headings.dedup();
    headings.truncate(20);
    headings
}

pub fn decode_transfer_planner_html_entities(value: &str) -> String {
    let repaired = repair_loose_html_entity_artifacts(value);
    let numeric_decoded = numeric_entity_pattern()
        .replace_all(&repaired, |captures: &regex::Captures<'_>| {
            let Some(entity) = captures.get(1).map(|value| value.as_str()) else {
                return captures
                    .get(0)
                    .map_or_else(String::new, |value| value.as_str().to_string());
            };
            decode_numeric_entity(entity).unwrap_or_else(|| {
                captures
                    .get(0)
                    .map_or_else(String::new, |value| value.as_str().to_string())
            })
        })
        .into_owned();
    named_entity_pattern()
        .replace_all(&numeric_decoded, |captures: &regex::Captures<'_>| {
            let Some(entity) = captures
                .get(1)
                .map(|value| value.as_str().to_ascii_lowercase())
            else {
                return captures
                    .get(0)
                    .map_or_else(String::new, |value| value.as_str().to_string());
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
                _ => captures
                    .get(0)
                    .map_or_else(String::new, |value| value.as_str().to_string()),
            }
        })
        .replace('\u{00a0}', " ")
}

fn strip_html_document_chrome(html: &str) -> String {
    let mut output = html.to_string();
    for pattern in chrome_patterns() {
        output = pattern.replace_all(&output, " ").into_owned();
    }
    html_comment_pattern()
        .replace_all(&output, " ")
        .into_owned()
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
                cells.push((
                    captures.get(0).map_or(0, |value| value.start()),
                    strip_html(body.as_str()),
                ));
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
            super::requirement_shape::is_js_whitespace(character) || "([{,;:".contains(character)
        });
        let boundary_after = after.map_or(true, |character| {
            super::requirement_shape::is_js_whitespace(character) || ")]},;:!?".contains(character)
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
    super::requirement_shape::normalize_js_whitespace(value)
}

fn compile_js_regex(pattern: &str, message: &'static str) -> Regex {
    let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
    Regex::new(&pattern).expect(message)
}

fn chrome_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        ["script", "style", "nav", "header", "footer", "aside"]
            .into_iter()
            .map(|tag| {
                compile_js_regex(
                    &format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}\s*>"),
                    "HTML chrome pattern",
                )
            })
            .collect()
    })
}

fn table_row_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        compile_js_regex(r"(?is)<tr\b[^>]*>(.*?)</tr\s*>", "HTML table row pattern")
    })
}
fn table_cell_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        ["td", "th"]
            .into_iter()
            .map(|tag| {
                compile_js_regex(
                    &format!(r"(?is)<{tag}\b[^>]*>(.*?)</{tag}\s*>"),
                    "HTML table cell pattern",
                )
            })
            .collect()
    })
}
fn split_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^[A-Za-z&]{1,20}(?:\s+[A-Za-z&]{1,20})?$")
            .expect("split course subject pattern")
    })
}
fn leading_course_number_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| compile_js_regex(r"^\d{3}[A-Za-z]?\b", "split course number pattern"))
}
fn block_tag_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| compile_js_regex(r"(?i)</?(?:p|div|section|article|main|aside|header|footer|details|summary|nav|li|ul|ol|dl|dt|dd|table|thead|tbody|tfoot|caption|tr|td|th|button|h1|h2|h3|h4|h5|h6|br)\b[^>]*>", "HTML block tag pattern"))
}
fn title_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        compile_js_regex(r"(?is)<title\b[^>]*>(.*?)</title\s*>", "HTML title pattern")
    })
}
fn heading_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        (1..=4)
            .map(|level| {
                compile_js_regex(
                    &format!(r"(?is)<h{level}\b[^>]*>(.*?)</h{level}\s*>"),
                    "HTML heading pattern",
                )
            })
            .collect()
    })
}
fn html_anchor_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        compile_js_regex(
            r#"(?is)<a\b[^>]*href=(?:"([^"]+)"|'([^']+)')[^>]*>(.*?)</a>"#,
            "html anchor",
        )
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
    VALUE.get_or_init(|| compile_js_regex(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$)))", "noisy source line pattern"))
}
fn numeric_entity_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        compile_js_regex(r"(?i)&(\#x[0-9a-f]+|\#\d+);", "numeric HTML entity pattern")
    })
}
fn named_entity_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)&([a-z]+);").expect("named HTML entity pattern"))
}
fn loose_entity_artifact_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let whitespace = r"[\t\n\x{000B}\f\r \x{00A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";
        let pattern = format!(
            r"(?i)((?:and{whitespace}+|amp{whitespace}+|&{whitespace}*|#{whitespace}*)?)(160|8211|8212|8216|8217|8220|8221)",
            whitespace = whitespace,
        );
        Regex::new(&pattern).expect("loose HTML entity artifact pattern")
    })
}
