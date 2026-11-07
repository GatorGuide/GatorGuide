use super::requirement_course_codes::CourseCodeParser;
use super::requirement_docx::DocxSourceArtifacts;
use super::requirement_html::RequirementHtmlArtifacts;
use super::requirement_parse_helpers::{
    build_parse_confidence, extract_choose_statements, extract_relevant_requirement_lines,
    extract_requirement_cue_lines,
};
use super::requirement_pdf_layout::{PdfPageBlock, scope_pdf_page_blocks};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;
use regex::Regex;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceTextFacts {
    pub title: Option<String>,
    pub headings: Vec<String>,
    pub requirement_cue_lines: Vec<String>,
    pub choose_statements: Vec<String>,
    pub candidate_course_codes: Vec<String>,
    pub snapshot_lines: Vec<String>,
    pub parse_confidence: String,
}

pub fn collect_html_source_facts(
    artifacts: &RequirementHtmlArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_requirement_source_text_facts(
        artifacts.title.clone(),
        artifacts.headings.clone(),
        artifacts.lines.clone(),
        parser_type,
        course_parser,
    )
}

pub fn collect_docx_source_facts(
    artifacts: &DocxSourceArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_requirement_source_text_facts(
        artifacts.title.clone(),
        Vec::new(),
        artifacts.lines.clone(),
        parser_type,
        course_parser,
    )
}

pub fn collect_pdf_source_facts(
    entry: &Value,
    page_blocks: &[PdfPageBlock],
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    let owner_title = entry.get("ownerTitle").and_then(Value::as_str);
    let source_label = entry.get("sourceLabel").and_then(Value::as_str);
    let scoped_lines = scope_pdf_page_blocks(owner_title, source_label, page_blocks)
        .into_iter()
        .flat_map(|block| {
            let page_number = block.page_number;
            block
                .line_texts
                .into_iter()
                .map(move |line| format!("[Page {page_number}] {line}"))
        })
        .collect::<Vec<_>>();
    let title = scoped_lines
        .first()
        .map(|line| page_prefix_pattern().replace(line, "").into_owned())
        .map(|line| normalize_whitespace(&line))
        .filter(|line| !line.is_empty());
    let parser_type = entry
        .get("parserType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    collect_requirement_source_text_facts(
        title,
        Vec::new(),
        scoped_lines,
        parser_type,
        course_parser,
    )
}

fn collect_requirement_source_text_facts(
    title: Option<String>,
    headings: Vec<String>,
    lines: Vec<String>,
    parser_type: &str,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    let requirement_cue_lines = extract_requirement_cue_lines(&lines, course_parser);
    let choose_statements = extract_choose_statements(&lines);
    let relevant_lines = extract_relevant_requirement_lines(&lines, &headings);
    let candidate_course_codes = relevant_lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let parse_confidence = build_parse_confidence(
        candidate_course_codes.len(),
        requirement_cue_lines.len(),
        parser_type,
    )
    .to_string();
    RequirementSourceTextFacts {
        title,
        headings,
        requirement_cue_lines,
        choose_statements,
        candidate_course_codes,
        snapshot_lines: lines.into_iter().take(1_200).collect(),
        parse_confidence,
    }
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn page_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\[Page \d+\]\s*").expect("PDF page label prefix"))
}
