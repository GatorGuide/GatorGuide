use super::requirement_course_codes::CourseCodeParser;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PdfTextContent {
    #[serde(default)]
    pub items: Vec<PdfTextItem>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PdfTextItem {
    #[serde(rename = "str", default)]
    pub text: String,
    #[serde(default)]
    pub transform: Vec<f64>,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPageBlock {
    pub page_number: usize,
    pub page_text: String,
    pub line_texts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfLineSegment {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub end_x: f64,
}

#[derive(Debug, Clone)]
struct PositionedItem {
    text: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone)]
struct SegmentBuilder {
    items: Vec<PositionedItem>,
    end_x: f64,
}

#[derive(Debug, Clone)]
struct LineGroup {
    y: f64,
    max_height: f64,
    items: Vec<PositionedItem>,
}

pub fn build_pdf_page_line_texts(
    text_content: &PdfTextContent,
    course_codes: &CourseCodeParser,
) -> Vec<String> {
    let mut positioned_items = text_content
        .items
        .iter()
        .filter_map(|item| {
            let text = normalize_whitespace(&item.text);
            if text.is_empty() {
                return None;
            }
            Some(PositionedItem {
                text,
                x: item.transform.get(4).copied().unwrap_or(0.0),
                y: item.transform.get(5).copied().unwrap_or(0.0),
                width: item.width,
                height: item.height,
            })
        })
        .collect::<Vec<_>>();
    positioned_items.sort_by(compare_reading_order);
    if positioned_items.is_empty() {
        return Vec::new();
    }

    let mut groups: Vec<LineGroup> = Vec::new();
    for item in positioned_items {
        let line_tolerance = groups
            .last()
            .map(|group| 2.5_f64.max(6.0_f64.min(group.max_height.max(item.height) * 0.45)))
            .unwrap_or(0.0);
        let joins_current = groups
            .last()
            .is_some_and(|group| (group.y - item.y).abs() <= line_tolerance);
        if !joins_current {
            groups.push(LineGroup {
                y: item.y,
                max_height: item.height,
                items: vec![item],
            });
        } else if let Some(group) = groups.last_mut() {
            group.max_height = group.max_height.max(item.height);
            group.items.push(item);
        }
    }

    let mut segments = Vec::new();
    for group in groups {
        let mut items = group.items;
        items.sort_by(|left, right| left.x.partial_cmp(&right.x).unwrap_or(Ordering::Equal));
        let raw_gap = group.max_height * 5.5;
        let segment_gap_threshold = 36.0_f64.max(if raw_gap == 0.0 { 48.0 } else { raw_gap.min(96.0) });
        let mut line_segments: Vec<SegmentBuilder> = Vec::new();
        for item in items {
            let estimated_width = estimate_item_width(&item, group.max_height);
            let starts_new_segment = match line_segments.last() {
                None => true,
                Some(current) => should_start_new_pdf_line_segment(
                    current,
                    &item,
                    segment_gap_threshold,
                    course_codes,
                ),
            };
            if starts_new_segment {
                line_segments.push(SegmentBuilder {
                    end_x: item.x + estimated_width,
                    items: vec![item],
                });
            } else if let Some(current) = line_segments.last_mut() {
                current.end_x = current.end_x.max(item.x + estimated_width);
                current.items.push(item);
            }
        }
        for segment in line_segments {
            let text = normalize_whitespace(
                &segment
                    .items
                    .iter()
                    .map(|item| item.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if text.is_empty() {
                continue;
            }
            let x = segment
                .items
                .iter()
                .map(|item| item.x)
                .fold(f64::INFINITY, f64::min);
            segments.push(PdfLineSegment {
                text,
                x,
                y: group.y,
                end_x: segment.end_x,
            });
        }
    }
    order_pdf_line_segments(&segments)
        .into_iter()
        .map(|segment| normalize_whitespace(&segment.text))
        .filter(|line| !line.is_empty())
        .collect()
}

pub fn scope_pdf_page_blocks(
    owner_title: Option<&str>,
    source_label: Option<&str>,
    page_blocks: &[PdfPageBlock],
) -> Vec<PdfPageBlock> {
    let title_tokens = get_pdf_title_scope_tokens(owner_title, source_label);
    if title_tokens.is_empty() || page_blocks.len() <= 2 {
        return page_blocks.to_vec();
    }
    let exact_title = normalize_matcher_text(owner_title.unwrap_or_default());
    let mut scored_pages = page_blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let normalized_line = normalize_matcher_text(&block.page_text);
            let token_score = title_tokens
                .iter()
                .filter(|token| normalized_line.contains(token.as_str()))
                .count() as i64;
            let exact_title_score = if !exact_title.is_empty() && normalized_line.contains(&exact_title) {
                10
            } else {
                0
            };
            let requirement_signal_score = requirement_signal_regex()
                .find_iter(&normalized_line)
                .count() as i64;
            let score = exact_title_score + token_score * 2 + requirement_signal_score;
            (score > 0).then_some((index, score))
        })
        .collect::<Vec<_>>();
    scored_pages.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let Some((best_page_index, best_score)) = scored_pages.first().copied() else {
        return page_blocks.to_vec();
    };
    let minimum_score = 4_i64.max(best_score - 2);
    let matching_indices = scored_pages
        .iter()
        .filter(|(index, score)| index.abs_diff(best_page_index) <= 1 && *score >= minimum_score)
        .map(|(index, _)| *index)
        .collect::<Vec<_>>();
    let Some(start) = matching_indices.iter().min().copied() else {
        return page_blocks.to_vec();
    };
    let end = matching_indices
        .iter()
        .max()
        .copied()
        .unwrap_or(start)
        .saturating_add(1)
        .min(page_blocks.len() - 1);
    let start = start.saturating_sub(1);
    page_blocks[start..=end].to_vec()
}

pub fn scope_pdf_page_lines(
    owner_title: Option<&str>,
    source_label: Option<&str>,
    page_blocks: &[PdfPageBlock],
) -> Vec<String> {
    scope_pdf_page_blocks(owner_title, source_label, page_blocks)
        .into_iter()
        .flat_map(|block| {
            let page_number = block.page_number;
            block
                .line_texts
                .into_iter()
                .map(move |line| format!("[Page {page_number}] {line}"))
        })
        .collect()
}

pub fn get_pdf_title_scope_tokens(owner_title: Option<&str>, source_label: Option<&str>) -> Vec<String> {
    let combined = format!("{} {}", owner_title.unwrap_or_default(), source_label.unwrap_or_default());
    let mut tokens = normalize_matcher_text(&combined)
        .split_whitespace()
        .filter(|token| token.len() >= 4 && !pdf_scope_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    tokens.truncate(8);
    tokens
}

pub fn order_pdf_line_segments(segments: &[PdfLineSegment]) -> Vec<PdfLineSegment> {
    let normalized = segments
        .iter()
        .filter(|segment| !normalize_whitespace(&segment.text).is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let Some(boundary) = should_use_column_major_pdf_ordering(&normalized) else {
        let mut ordered = normalized;
        ordered.sort_by(compare_line_segment_reading_order);
        return ordered;
    };
    let mut left = normalized
        .iter()
        .filter(|segment| segment.x < boundary)
        .cloned()
        .collect::<Vec<_>>();
    let mut right = normalized
        .into_iter()
        .filter(|segment| segment.x >= boundary)
        .collect::<Vec<_>>();
    left.sort_by(compare_line_segment_reading_order);
    right.sort_by(compare_line_segment_reading_order);
    left.extend(right);
    left
}

fn should_use_column_major_pdf_ordering(segments: &[PdfLineSegment]) -> Option<f64> {
    if segments.len() < 12 {
        return None;
    }
    let mut sorted_x = segments
        .iter()
        .filter(|segment| segment.x.is_finite())
        .map(|segment| segment.x.round() as i64)
        .collect::<Vec<_>>();
    sorted_x.sort_unstable();
    sorted_x.dedup();
    if sorted_x.len() < 4 {
        return None;
    }
    let mut split: Option<(i64, f64)> = None;
    for pair in sorted_x.windows(2) {
        let gap = pair[1] - pair[0];
        if gap < 72 {
            continue;
        }
        if split.is_none_or(|(best_gap, _)| gap > best_gap) {
            split = Some((gap, (pair[1] as f64 + pair[0] as f64) / 2.0));
        }
    }
    let (_, boundary) = split?;
    let left = segments.iter().filter(|segment| segment.x < boundary).collect::<Vec<_>>();
    let right = segments.iter().filter(|segment| segment.x >= boundary).collect::<Vec<_>>();
    if left.len() < 5 || right.len() < 5 {
        return None;
    }
    let left_heading_count = left
        .iter()
        .filter(|segment| numbered_pdf_heading_regex().is_match(&normalize_whitespace(&segment.text)))
        .count();
    let right_heading_count = right
        .iter()
        .filter(|segment| numbered_pdf_heading_regex().is_match(&normalize_whitespace(&segment.text)))
        .count();
    let checklist_count = segments
        .iter()
        .filter(|segment| bare_checklist_line_regex().is_match(&normalize_whitespace(&segment.text)))
        .count();
    if left_heading_count < 1
        || right_heading_count < 1
        || left_heading_count + right_heading_count < 3
        || checklist_count < 6
    {
        return None;
    }
    Some(boundary)
}

fn should_start_new_pdf_line_segment(
    current: &SegmentBuilder,
    item: &PositionedItem,
    gap_threshold: f64,
    course_codes: &CourseCodeParser,
) -> bool {
    let gap = item.x - current.end_x;
    if gap > gap_threshold {
        return true;
    }
    let current_text = normalize_whitespace(
        &current
            .items
            .iter()
            .map(|current_item| current_item.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let item_text = normalize_whitespace(&item.text);
    if gap > 8.0
        && numbered_item_start_regex().is_match(&item_text)
        && bare_checklist_line_regex().is_match(&current_text)
    {
        return true;
    }
    if gap > 8.0
        && numbered_pdf_heading_regex().is_match(&current_text)
        && (source_line_starts_with_course_code(&item_text)
            || !course_codes.extract_from_line(&item_text).is_empty()
            || course_codes.normalize_subject(&item_text).is_some()
            || bare_checklist_line_regex().is_match(&item_text))
    {
        return true;
    }
    current
        .items
        .last()
        .is_some_and(|previous| {
            is_pdf_credit_marker_text(&previous.text)
                && !is_pdf_credit_marker_text(&item.text)
                && gap > 18.0_f64.max(gap_threshold * 0.35)
        })
}

fn estimate_item_width(item: &PositionedItem, max_height: f64) -> f64 {
    item.width.max(item.text.encode_utf16().count() as f64 * max_height.max(8.0) * 0.45)
}

fn compare_reading_order(left: &PositionedItem, right: &PositionedItem) -> Ordering {
    if (right.y - left.y).abs() > 2.5 {
        right.y.partial_cmp(&left.y).unwrap_or(Ordering::Equal)
    } else {
        left.x.partial_cmp(&right.x).unwrap_or(Ordering::Equal)
    }
}

fn compare_line_segment_reading_order(left: &PdfLineSegment, right: &PdfLineSegment) -> Ordering {
    if (right.y - left.y).abs() > 2.5 {
        right.y.partial_cmp(&left.y).unwrap_or(Ordering::Equal)
    } else {
        left.x.partial_cmp(&right.x).unwrap_or(Ordering::Equal)
    }
}

fn source_line_starts_with_course_code(line: &str) -> bool {
    source_course_code_line_regex().is_match(&strip_snapshot_page_prefix(line))
}

fn strip_snapshot_page_prefix(line: &str) -> String {
    snapshot_page_prefix_regex().replace(line, "").into_owned()
}

fn is_pdf_credit_marker_text(value: &str) -> bool {
    pdf_credit_marker_regex().is_match(&normalize_whitespace(value))
}

fn normalize_matcher_text(value: &str) -> String {
    matcher_non_alphanumeric_regex()
        .replace_all(&normalize_whitespace(value).to_lowercase(), " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pdf_scope_stopwords() -> &'static std::collections::HashSet<&'static str> {
    static VALUE: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
    VALUE.get_or_init(|| {
        [
            "and", "arts", "bachelor", "bothell", "campus", "catalog", "course", "courses",
            "curriculum", "degree", "major", "of", "official", "overview", "page", "program",
            "requirements", "science", "seattle", "studies", "tacoma", "the", "university", "uw",
            "washington",
        ]
        .into_iter()
        .collect()
    })
}
fn matcher_non_alphanumeric_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[^a-z0-9]+").expect("matcher punctuation pattern"))
}
fn requirement_signal_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"\b(requirements?|credits?|courses?|major|minor|curriculum|prerequisite)\b")
            .expect("PDF requirement-signal pattern")
    })
}
fn numbered_pdf_heading_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"^\s*\d+\)\s+[A-Za-z][^()]{2,80}\([A-Z& ]{2,10}\)")
            .expect("PDF heading pattern")
    })
}
fn bare_checklist_line_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?iu)^\s*(?:[^\w\d]{1,10}\s*)?\d{3}[A-Za-z]?\s*\(\d+(?:\.\d+)?\)")
            .expect("PDF checklist pattern")
    })
}
fn numbered_item_start_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\d+\)(?:\s+|$)").expect("PDF numbered-item pattern"))
}
fn source_course_code_line_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^[-*•\s]*(?:\(\s*\d+(?:\.\d+)?\s*\)\s*)?(?:[A-Z&]+(?:\s+[A-Z&]+)?(?:\s*/\s*[A-Z&]+(?:\s+[A-Z&]+)?)?\s+\d{3}[A-Za-z]?\*?|\d{3}[A-Za-z]?\b)")
            .expect("PDF course-code prefix pattern")
    })
}
fn snapshot_page_prefix_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\[Page\s+\d+\]\s*").expect("PDF page prefix pattern"))
}
fn pdf_credit_marker_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\d+(?:-\d+)?\s*cr$").expect("PDF credit marker pattern"))
}
