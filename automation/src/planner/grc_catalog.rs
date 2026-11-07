use regex::Regex;
use serde::{Deserialize, Serialize};

const GRC_LEADING_STOPWORDS: [&str; 22] = [
    "AND", "ANY", "APPROVED", "AT", "BEFORE", "BETWEEN", "DIVISION", "FOR", "FROM", "IN", "INTO",
    "LEAST", "MINIMUM", "OF", "ONE", "OR", "PLUS", "THE", "THREE", "TO", "TWO", "WITH",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrcCourseDetailsInput {
    pub raw_bodies: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrcCourseCatalogInput {
    pub html: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GrcCourseCatalogSection {
    pub heading_text: String,
    pub course_code_candidate: Option<String>,
    pub course_title_candidate: Option<String>,
    pub course_code_syntax_valid: bool,
    pub details: GrcCourseDetails,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GrcCourseDetails {
    pub credit_label: Option<String>,
    pub credit_value: Option<f64>,
    pub catalog_description: Option<String>,
    pub enrollment_requirement_linked_anchor_texts: Vec<String>,
    pub enrollment_requirement_linked_course_code_candidates: Vec<String>,
    pub enrollment_requirement: Option<String>,
    pub enrollment_course_code_paths: Option<Vec<Vec<String>>>,
    pub enrollment_corequisite_course_code_paths: Option<Vec<Vec<String>>>,
    pub corequisite_requirement: Option<String>,
    pub corequisite_course_code_paths: Option<Vec<Vec<String>>>,
    pub satisfies_requirement: Option<String>,
    pub general_education_tags: Vec<String>,
}

pub fn parse_grc_course_catalog(html: &str) -> Vec<GrcCourseCatalogSection> {
    let heading_pattern =
        Regex::new(r"(?is)<h3[^>]*>(.*?)</h3>").expect("GRC course-heading pattern is valid");
    let headings = heading_pattern.captures_iter(html).collect::<Vec<_>>();
    let raw_bodies = headings
        .iter()
        .enumerate()
        .map(|(index, heading)| {
            let heading_end = heading
                .get(0)
                .expect("course heading has a full match")
                .end();
            let body_end = headings
                .get(index + 1)
                .and_then(|next| next.get(0))
                .map_or(html.len(), |next| next.start());
            html[heading_end..body_end].to_string()
        })
        .collect::<Vec<_>>();
    let details = parse_grc_course_details(&raw_bodies);
    let strip_html_pattern = Regex::new(r"<[^>]+>").expect("HTML tag pattern is valid");
    let entity_pattern = Regex::new(r"(?i)&(?:amp|apos|nbsp|quot|#39|#160|#8211|#8212|#8217);")
        .expect("HTML entity pattern is valid");
    let course_heading_parts_pattern =
        Regex::new(r"^(.+?)\s+-\s+(.+)$").expect("GRC course-heading parts pattern is valid");
    let course_code_syntax_pattern = Regex::new(r"^[A-Z0-9 &]+\s+\d{1,3}(?:\.\d+)?[A-Z]?$")
        .expect("GRC course-code syntax pattern is valid");

    headings
        .into_iter()
        .zip(details)
        .map(|(heading, details)| {
            let heading_text = strip_html(
                heading
                    .get(1)
                    .expect("course heading has inner HTML")
                    .as_str(),
                &strip_html_pattern,
                &entity_pattern,
            );
            let heading_parts = course_heading_parts_pattern.captures(&heading_text);
            let course_code_candidate =
                heading_parts.as_ref().map(|capture| capture[1].to_string());
            let course_title_candidate =
                heading_parts.as_ref().map(|capture| capture[2].to_string());
            let course_code_syntax_valid = course_code_candidate
                .as_deref()
                .map(|candidate| course_code_syntax_pattern.is_match(&candidate.to_uppercase()))
                .unwrap_or(false);

            GrcCourseCatalogSection {
                heading_text,
                course_code_candidate,
                course_title_candidate,
                course_code_syntax_valid,
                details,
            }
        })
        .collect()
}

pub fn parse_grc_course_details(raw_bodies: &[String]) -> Vec<GrcCourseDetails> {
    let credit_pattern =
        Regex::new(r"(?i)Credits:\s*([^\r\n<]+)").expect("GRC credit-label pattern is valid");
    let leading_credit_pattern =
        Regex::new(r"(?i)^\s*Credits:\s*[^<\r\n]+").expect("GRC leading-credit pattern is valid");
    let requirement_boundary_pattern = Regex::new(
        r"(?i)<br\s*/?>\s*<br\s*/?>\s*<strong>\s*(?:Enrollment Requirement|Enrollment Prerequisite|Satisfies Requirement|Course Fee|Course Outcomes|Program Outcomes|College-wide Outcomes)\s*:",
    )
    .expect("GRC requirement-boundary pattern is valid");
    let strip_html_pattern = Regex::new(r"<[^>]+>").expect("HTML tag pattern is valid");
    let anchor_pattern =
        Regex::new(r"(?is)<a\b([^>]*)>(.*?)</a>").expect("GRC course-link anchor pattern is valid");
    let href_attribute_pattern =
        Regex::new(r"(?i)(?:^|\s)href\s*=").expect("GRC href-attribute pattern is valid");
    let entity_pattern = Regex::new(r"(?i)&(?:amp|apos|nbsp|quot|#39|#160|#8211|#8212|#8217);")
        .expect("HTML entity pattern is valid");
    let numeric_credit_pattern =
        Regex::new(r"^\d+(?:\.\d+)?$").expect("numeric credit pattern is valid");
    let simple_requirement_course_pattern =
        Regex::new(r"\b[A-Z&]{1,8}(?:\s+[A-Z&]{1,8}){0,2}\s+\d{2,3}(?:\.\d+)?[A-Z]?\b")
            .expect("GRC simple-requirement course-code pattern is valid");
    let course_code_slash_shorthand_pattern = Regex::new(
        r"\b([A-Z&]{1,8}(?:\s+[A-Z&]{1,8}){0,2})\s+(\d{2,3}[A-Z]?)(\s*(?:/\s*\d{2,3}[A-Z]?)+)\b",
    )
    .expect("GRC course-code slash-shorthand pattern is valid");
    let ignored_linked_course_prefix_pattern =
        Regex::new(r"(?i)\b(?:eligible for|equivalent of|admission (?:into|to))\s*$")
            .expect("GRC linked-course ignored-prefix pattern is valid");
    let linked_high_school_requirement_pattern = Regex::new(r"(?i)\byears?\s+in high school\b")
        .expect("GRC linked-course high-school context pattern is valid");
    let recommended_requirement_pattern = Regex::new(r"(?i)\bRecommended\s*:")
        .expect("GRC recommended-requirement boundary pattern is valid");
    let alternative_boundary_exclusion_patterns = [
        Regex::new(r"\bor\s+(?:higher|lower|better|concurrent enrollment|instructor(?:'s)?|permission|consent|appropriate|equivalent)\b")
            .expect("GRC alternative-boundary context pattern is valid"),
        Regex::new(r"\bor\s+(?:in\s+)?(?:a\s+|an\s+)?high school\b[^,;)]*")
            .expect("GRC high-school alternative pattern is valid"),
        Regex::new(r"\bor\s+(?:one|two|three|1\s*1/2|1-1/2|\d+)\s+years?\s+of high school\b[^,;)]*")
            .expect("GRC high-school-years alternative pattern is valid"),
        Regex::new(r"\bor\s+(?:appropriate\s+)?(?:english|math|reading)\s+placement\b[^,;)]*")
            .expect("GRC placement alternative pattern is valid"),
        Regex::new(r"\bor\s+eligible for\b[^,;)]*")
            .expect("GRC eligibility alternative pattern is valid"),
    ];
    let alternative_word_pattern =
        Regex::new(r"\bor\b").expect("GRC alternative-word pattern is valid");
    let concurrent_enrollment_pattern = Regex::new(r"(?i)\bconcurrent enrollment\b")
        .expect("GRC concurrent-enrollment pattern is valid");
    let parenthetical_concurrent_enrollment_pattern =
        Regex::new(r"(?i)\(\s*([^()]+?)\s+or\s+concurrent enrollment\s*\)")
            .expect("GRC parenthetical concurrent-enrollment pattern is valid");
    let concurrent_completion_pattern =
        Regex::new(r"(?i)(?:at least\s+)?concurrent enrollment in or completion of\s+([^;.,]+)")
            .expect("GRC concurrent-completion pattern is valid");
    let concurrent_in_pattern = Regex::new(
        r"(?i)(?:at least\s+)?concurrent enrollment(?:\s+or\s+completion)? in\s+([^;.,]+)",
    )
    .expect("GRC concurrent-course pattern is valid");
    let requirement_text_parser = RequirementTextParser {
        course_code_pattern: &simple_requirement_course_pattern,
        slash_shorthand_pattern: &course_code_slash_shorthand_pattern,
        entity_pattern: &entity_pattern,
        ignored_course_prefix_pattern: &ignored_linked_course_prefix_pattern,
        high_school_context_pattern: &linked_high_school_requirement_pattern,
        recommended_pattern: &recommended_requirement_pattern,
        alternative_boundary_exclusion_patterns: &alternative_boundary_exclusion_patterns,
        alternative_word_pattern: &alternative_word_pattern,
        concurrent_enrollment_pattern: &concurrent_enrollment_pattern,
        parenthetical_concurrent_enrollment_pattern: &parenthetical_concurrent_enrollment_pattern,
        concurrent_completion_pattern: &concurrent_completion_pattern,
        concurrent_in_pattern: &concurrent_in_pattern,
    };
    let section_boundary_pattern = Regex::new(r"(?i)<br\s*/?>\s*<br\s*/?>\s*<strong>")
        .expect("GRC labeled-section boundary is valid");
    let heading_boundary_pattern = Regex::new(
        r"(?i)<strong>\s*(?:Course Fee|Course Outcomes|Program Outcomes|College-wide Outcomes)\s*:</strong>",
    )
    .expect("GRC labeled-heading boundary is valid");
    let enrollment_requirement_start_pattern =
        Regex::new(r"(?i)<strong>\s*Enrollment (?:Requirement|Prerequisite)\s*:</strong>")
            .expect("GRC enrollment-requirement start pattern is valid");
    let corequisite_start_pattern = Regex::new(r"(?i)<strong>\s*Corequisite\s*:</strong>")
        .expect("GRC corequisite start pattern is valid");
    let satisfies_requirement_start_pattern =
        Regex::new(r"(?i)<strong>\s*Satisfies Requirement\s*:</strong>")
            .expect("GRC satisfies-requirement start pattern is valid");
    let general_education_patterns = [
        Regex::new(r"(?i)\bbasic skills\s*/\s*communication\b|\bwritten communication\b")
            .expect("GRC communication-tag pattern is valid"),
        Regex::new(r"(?i)\bquantitative\b|\bsymbolic reasoning\b|\bquantitative skills\b|\bbasic skills requirement\b")
            .expect("GRC quantitative-tag pattern is valid"),
        Regex::new(r"(?i)\bhumanities\b|\bfine arts\b|\benglish\b")
            .expect("GRC arts-and-humanities-tag pattern is valid"),
        Regex::new(r"(?i)\bsocial science\b|\bsocial sciences\b")
            .expect("GRC social-science-tag pattern is valid"),
        Regex::new(r"(?i)\bnatural science\b|\bnatural sciences\b|\blab science\b")
            .expect("GRC natural-science-tag pattern is valid"),
        Regex::new(r"(?i)\bdiversity\b").expect("GRC diversity-tag pattern is valid"),
    ];

    raw_bodies
        .iter()
        .map(|raw_body| {
            let credit_label = credit_pattern
                .captures(raw_body)
                .map(|capture| normalize_whitespace(&capture[1], &entity_pattern));
            let credit_value = credit_label.as_deref().and_then(|label| {
                numeric_credit_pattern
                    .is_match(label)
                    .then(|| label.parse::<f64>().ok())
                    .flatten()
            });
            let without_credits = leading_credit_pattern.replace(raw_body, "");
            let before_requirement = requirement_boundary_pattern
                .find(&without_credits)
                .map(|boundary| &without_credits[..boundary.start()])
                .unwrap_or(&without_credits);
            let description = strip_html(before_requirement, &strip_html_pattern, &entity_pattern);
            let enrollment_requirement_html = extract_labeled_section(
                raw_body,
                &enrollment_requirement_start_pattern,
                &section_boundary_pattern,
                &heading_boundary_pattern,
            );
            let enrollment_requirement_linked_anchor_texts = enrollment_requirement_html
                .as_deref()
                .map(|section| {
                    extract_linked_anchor_texts(
                        section,
                        &anchor_pattern,
                        &href_attribute_pattern,
                        &strip_html_pattern,
                        &entity_pattern,
                    )
                })
                .unwrap_or_default();
            let enrollment_requirement_linked_course_code_candidates =
                extract_linked_course_code_candidates(
                    &enrollment_requirement_linked_anchor_texts,
                    &simple_requirement_course_pattern,
                    &course_code_slash_shorthand_pattern,
                    &entity_pattern,
                    &ignored_linked_course_prefix_pattern,
                    &linked_high_school_requirement_pattern,
                );
            let enrollment_requirement = enrollment_requirement_html
                .as_deref()
                .map(|section| strip_html(section, &strip_html_pattern, &entity_pattern));
            let (enrollment_course_code_paths, enrollment_corequisite_course_code_paths) =
                enrollment_requirement
                    .as_deref()
                    .map(|value| requirement_text_parser.parse_enrollment(value))
                    .unwrap_or_default();
            let corequisite_requirement = extract_labeled_section(
                raw_body,
                &corequisite_start_pattern,
                &section_boundary_pattern,
                &heading_boundary_pattern,
            )
            .as_deref()
            .map(|section| strip_html(section, &strip_html_pattern, &entity_pattern));
            let corequisite_course_code_paths = corequisite_requirement
                .as_deref()
                .and_then(|value| requirement_text_parser.parse_corequisite(value));
            let satisfies_requirement = extract_labeled_section(
                raw_body,
                &satisfies_requirement_start_pattern,
                &section_boundary_pattern,
                &heading_boundary_pattern,
            )
            .as_deref()
            .map(|section| strip_html(section, &strip_html_pattern, &entity_pattern));
            let general_education_tags = satisfies_requirement
                .as_deref()
                .map(|value| classify_general_education_tags(value, &general_education_patterns))
                .unwrap_or_default();

            GrcCourseDetails {
                credit_label,
                credit_value,
                catalog_description: (!description.is_empty()).then_some(description),
                enrollment_requirement_linked_anchor_texts,
                enrollment_requirement_linked_course_code_candidates,
                enrollment_requirement,
                enrollment_course_code_paths,
                enrollment_corequisite_course_code_paths,
                corequisite_requirement,
                corequisite_course_code_paths,
                satisfies_requirement,
                general_education_tags,
            }
        })
        .collect()
}

fn expand_course_code_slash_shorthand(
    value: &str,
    pattern: &Regex,
    entity_pattern: &Regex,
) -> String {
    let mut expanded = value.to_string();
    loop {
        let mut output = String::with_capacity(expanded.len());
        let mut previous_end = 0;
        let mut found_match = false;

        for captures in pattern.captures_iter(&expanded) {
            let full_match = captures.get(0).expect("slash pattern has a full match");
            output.push_str(&expanded[previous_end..full_match.start()]);
            let subject = normalize_whitespace(
                captures
                    .get(1)
                    .expect("slash pattern has a subject")
                    .as_str(),
                entity_pattern,
            );
            let first_number = captures
                .get(2)
                .expect("slash pattern has a first course number")
                .as_str();
            let trailing_numbers = captures
                .get(3)
                .expect("slash pattern has trailing course numbers")
                .as_str()
                .split('/')
                .map(|number| number.trim())
                .filter(|number| !number.is_empty());
            let codes = std::iter::once(first_number)
                .chain(trailing_numbers)
                .map(|number| format!("{subject} {number}"))
                .collect::<Vec<_>>();
            output.push_str(&codes.join(" or "));
            previous_end = full_match.end();
            found_match = true;
        }

        if !found_match {
            return expanded;
        }
        output.push_str(&expanded[previous_end..]);
        if output == expanded {
            return output;
        }
        expanded = output;
    }
}

fn extract_linked_course_code_candidates(
    anchor_texts: &[String],
    course_code_pattern: &Regex,
    slash_shorthand_pattern: &Regex,
    entity_pattern: &Regex,
    ignored_prefix_pattern: &Regex,
    high_school_context_pattern: &Regex,
) -> Vec<String> {
    let mut candidates = Vec::new();
    for anchor_text in anchor_texts {
        let normalized = normalize_whitespace(anchor_text, entity_pattern);
        let expanded = expand_course_code_slash_shorthand(
            &normalized,
            slash_shorthand_pattern,
            entity_pattern,
        );
        for course_match in course_code_pattern.find_iter(&expanded) {
            let course_code = course_match.as_str();
            let subject = course_code.rsplit_once(' ').map(|(subject, _)| subject);
            let leading_token = subject.and_then(|subject| subject.split_whitespace().next());
            let has_invalid_leading_token = leading_token
                .map(|token| GRC_LEADING_STOPWORDS.contains(&token.to_ascii_uppercase().as_str()))
                .unwrap_or(true);
            if has_invalid_leading_token {
                continue;
            }

            let boundary_start = expanded[..course_match.start()]
                .rfind(|character| matches!(character, ';' | '.' | ':' | '('))
                .map_or(0, |index| index + 1);
            let before = &expanded[boundary_start..course_match.start()];
            let after_end = expanded.len().min(course_match.end().saturating_add(80));
            let after = &expanded[course_match.end()..after_end];
            if ignored_prefix_pattern.is_match(before)
                || high_school_context_pattern.is_match(after)
            {
                continue;
            }
            candidates.push(course_code.to_string());
        }
    }
    candidates
}

fn strip_balanced_enclosing_parentheses(mut value: &str) -> &str {
    loop {
        let trimmed = value.trim();
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            return trimmed;
        }

        let mut depth = 0usize;
        let mut encloses_entire_value = true;
        for (index, character) in trimmed.char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        return trimmed;
                    }
                    depth -= 1;
                    if depth == 0 && index + character.len_utf8() != trimmed.len() {
                        encloses_entire_value = false;
                        break;
                    }
                }
                _ => {}
            }
        }

        if !encloses_entire_value || depth != 0 {
            return trimmed;
        }
        value = &trimmed[1..trimmed.len() - 1];
    }
}

fn parse_simple_course_code_paths(
    value: &str,
    course_code_pattern: &Regex,
) -> Option<Vec<Vec<String>>> {
    let value = strip_balanced_enclosing_parentheses(value);
    let matches = course_code_pattern.find_iter(value).collect::<Vec<_>>();
    let first = matches.first()?;
    let last = matches.last()?;
    let leading_text = value[..first.start()].trim();
    let trailing_text = value[last.end()..].trim();
    if !leading_text.is_empty() || !matches!(trailing_text, "" | "." | "," | ";") {
        return None;
    }

    let course_codes = matches
        .iter()
        .map(|course_match| course_match.as_str().to_string())
        .collect::<Vec<_>>();
    if course_codes.iter().any(|course_code| {
        course_code
            .rsplit_once(' ')
            .and_then(|(subject, _)| subject.split_whitespace().next())
            .map(|token| GRC_LEADING_STOPWORDS.contains(&token.to_ascii_uppercase().as_str()))
            .unwrap_or(true)
    }) {
        return None;
    }

    let joiners = matches
        .windows(2)
        .map(|pair| {
            let separator = value[pair[0].end()..pair[1].start()]
                .trim()
                .to_ascii_lowercase();
            match separator.as_str() {
                "and" | "or" => Some(separator),
                "," | ";" | "." | "&" | "+" => Some("and".to_string()),
                _ => None,
            }
        })
        .collect::<Option<Vec<_>>>()?;

    let mut paths = vec![vec![course_codes[0].clone()]];
    for (joiner, course_code) in joiners.iter().zip(course_codes.iter().skip(1)) {
        if joiner == "or" {
            paths.push(vec![course_code.clone()]);
        } else {
            paths
                .last_mut()
                .expect("a course path exists before each additional code")
                .push(course_code.clone());
        }
    }

    if paths.len() > 1 && paths[0].len() > 1 {
        let prefix = paths[0][..paths[0].len() - 1].to_vec();
        let anchor_subject = course_code_subject(paths[0].last().expect("path is non-empty"));
        let alternatives_share_subject = paths.iter().skip(1).all(|path| {
            path.len() == 1 && course_code_subject(&path[0]).eq_ignore_ascii_case(&anchor_subject)
        });
        if alternatives_share_subject {
            for path in paths.iter_mut().skip(1) {
                let mut expanded_path = prefix.clone();
                expanded_path.append(path);
                *path = expanded_path;
            }
        }
    }

    Some(paths)
}

#[derive(Clone, Debug)]
struct RequirementCourseCodeMatch {
    course_code: String,
    start: usize,
    end: usize,
}

struct RequirementTextParser<'a> {
    course_code_pattern: &'a Regex,
    slash_shorthand_pattern: &'a Regex,
    entity_pattern: &'a Regex,
    ignored_course_prefix_pattern: &'a Regex,
    high_school_context_pattern: &'a Regex,
    recommended_pattern: &'a Regex,
    alternative_boundary_exclusion_patterns: &'a [Regex],
    alternative_word_pattern: &'a Regex,
    concurrent_enrollment_pattern: &'a Regex,
    parenthetical_concurrent_enrollment_pattern: &'a Regex,
    concurrent_completion_pattern: &'a Regex,
    concurrent_in_pattern: &'a Regex,
}

impl RequirementTextParser<'_> {
    fn parse_enrollment(
        &self,
        value: &str,
    ) -> (Option<Vec<Vec<String>>>, Option<Vec<Vec<String>>>) {
        let active_requirement = self
            .recommended_pattern
            .find(value)
            .map(|boundary| &value[..boundary.start()])
            .unwrap_or(value);
        let expanded_requirement = expand_course_code_slash_shorthand(
            active_requirement,
            self.slash_shorthand_pattern,
            self.entity_pattern,
        );
        let mut prerequisite_paths = Vec::new();
        let mut corequisite_paths = Vec::new();

        for raw_chunk in split_requirement_chunks(&expanded_requirement) {
            let raw_chunk = normalize_requirement_whitespace(raw_chunk);
            if raw_chunk.is_empty() {
                continue;
            }
            let (chunk, joiner) = strip_leading_requirement_joiner(&raw_chunk);
            if chunk.is_empty() {
                continue;
            }

            let (remainder, explicit_corequisite_paths) =
                self.extract_explicit_concurrent_enrollment_paths(chunk);
            let chunk_paths = self.parse_course_paths(&remainder);
            if chunk_paths.is_empty() {
                if !explicit_corequisite_paths.is_empty() {
                    if joiner == "or" && !prerequisite_paths.is_empty() {
                        prerequisite_paths = remove_overlapping_paths(
                            prerequisite_paths,
                            &explicit_corequisite_paths,
                        );
                    }
                    corequisite_paths = merge_requirement_paths(
                        corequisite_paths,
                        explicit_corequisite_paths,
                        joiner,
                    );
                }
                continue;
            }

            if !explicit_corequisite_paths.is_empty() {
                if joiner == "or" && !prerequisite_paths.is_empty() {
                    prerequisite_paths =
                        remove_overlapping_paths(prerequisite_paths, &explicit_corequisite_paths);
                }
                prerequisite_paths = merge_requirement_paths(
                    prerequisite_paths,
                    remove_overlapping_paths(chunk_paths, &explicit_corequisite_paths),
                    joiner,
                );
                corequisite_paths =
                    merge_requirement_paths(corequisite_paths, explicit_corequisite_paths, joiner);
                continue;
            }

            if self.concurrent_enrollment_pattern.is_match(chunk) {
                if joiner == "or" && !prerequisite_paths.is_empty() {
                    let overlapping_paths = chunk_paths
                        .iter()
                        .filter(|path| prerequisite_paths.contains(path))
                        .cloned()
                        .collect::<Vec<_>>();
                    if !overlapping_paths.is_empty() {
                        prerequisite_paths =
                            remove_overlapping_paths(prerequisite_paths, &overlapping_paths);
                        corequisite_paths =
                            merge_requirement_paths(corequisite_paths, overlapping_paths, "or");
                    }
                    continue;
                }
                corequisite_paths = merge_requirement_paths(corequisite_paths, chunk_paths, joiner);
                continue;
            }

            prerequisite_paths = merge_requirement_paths(prerequisite_paths, chunk_paths, joiner);
        }

        (
            (!prerequisite_paths.is_empty()).then_some(prerequisite_paths),
            (!corequisite_paths.is_empty()).then_some(corequisite_paths),
        )
    }

    fn parse_corequisite(&self, value: &str) -> Option<Vec<Vec<String>>> {
        let expanded_requirement = expand_course_code_slash_shorthand(
            value,
            self.slash_shorthand_pattern,
            self.entity_pattern,
        );
        let mut corequisite_paths = Vec::new();
        for raw_chunk in split_requirement_chunks(&expanded_requirement) {
            let raw_chunk = normalize_requirement_whitespace(raw_chunk);
            if raw_chunk.is_empty() {
                continue;
            }
            let (chunk, joiner) = strip_leading_requirement_joiner(&raw_chunk);
            if chunk.is_empty() {
                continue;
            }
            let chunk_paths = self.parse_course_paths(chunk);
            if !chunk_paths.is_empty() {
                corequisite_paths = merge_requirement_paths(corequisite_paths, chunk_paths, joiner);
            }
        }
        (!corequisite_paths.is_empty()).then_some(corequisite_paths)
    }

    fn parse_course_paths(&self, expression: &str) -> Vec<Vec<String>> {
        let expanded_expression = expand_course_code_slash_shorthand(
            expression,
            self.slash_shorthand_pattern,
            self.entity_pattern,
        );
        if let Some(paths) =
            parse_simple_course_code_paths(&expanded_expression, self.course_code_pattern)
        {
            return paths;
        }

        if let Some((start, end)) = find_simple_parenthetical(&expanded_expression) {
            let inner_paths =
                self.parse_simple_course_paths(&expanded_expression[start + 1..end - 1]);
            let outer_expression = format!(
                "{} {}",
                &expanded_expression[..start],
                &expanded_expression[end..]
            );
            let outer_paths = self.parse_simple_course_paths(&outer_expression);
            if !inner_paths.is_empty() && !outer_paths.is_empty() {
                return cross_combine_paths(inner_paths, outer_paths);
            }
            if !inner_paths.is_empty() {
                return inner_paths;
            }
            return outer_paths;
        }

        self.parse_simple_course_paths(&expanded_expression)
    }

    fn parse_simple_course_paths(&self, expression: &str) -> Vec<Vec<String>> {
        let expanded_expression = expand_course_code_slash_shorthand(
            expression,
            self.slash_shorthand_pattern,
            self.entity_pattern,
        );
        let matches = self.get_safe_course_code_matches(&expanded_expression);
        let Some(first_match) = matches.first() else {
            return Vec::new();
        };
        let mut paths = vec![vec![first_match.course_code.clone()]];

        for pair in matches.windows(2) {
            let between = &expanded_expression[pair[0].end..pair[1].start];
            if self.has_alternative_boundary(between) {
                paths.push(vec![pair[1].course_code.clone()]);
            } else {
                paths
                    .last_mut()
                    .expect("a path exists before each additional course")
                    .push(pair[1].course_code.clone());
            }
        }

        apply_shared_prefix_to_paths(paths)
    }

    fn get_safe_course_code_matches(&self, value: &str) -> Vec<RequirementCourseCodeMatch> {
        let mut matches = Vec::new();
        for course_match in self.course_code_pattern.find_iter(value) {
            let course_code = course_match.as_str();
            let invalid_leading_token = course_code
                .rsplit_once(' ')
                .and_then(|(subject, _)| subject.split_whitespace().next())
                .map(|token| GRC_LEADING_STOPWORDS.contains(&token.to_ascii_uppercase().as_str()))
                .unwrap_or(true);
            if invalid_leading_token {
                continue;
            }

            let boundary_start = value[..course_match.start()]
                .rfind(|character| matches!(character, ';' | '.' | ':' | '('))
                .map_or(0, |index| index + 1);
            let before = &value[boundary_start..course_match.start()];
            let after = value[course_match.end()..]
                .chars()
                .take(80)
                .collect::<String>();
            if self.ignored_course_prefix_pattern.is_match(before)
                || self.high_school_context_pattern.is_match(&after)
            {
                continue;
            }

            matches.push(RequirementCourseCodeMatch {
                course_code: course_code.to_string(),
                start: course_match.start(),
                end: course_match.end(),
            });
        }
        matches
    }

    fn has_alternative_boundary(&self, value: &str) -> bool {
        let mut cleaned = normalize_requirement_whitespace(value).to_ascii_lowercase();
        for pattern in self.alternative_boundary_exclusion_patterns {
            cleaned = pattern.replace_all(&cleaned, " ").into_owned();
        }
        self.alternative_word_pattern.is_match(&cleaned)
    }

    fn extract_explicit_concurrent_enrollment_paths(
        &self,
        value: &str,
    ) -> (String, Vec<Vec<String>>) {
        let mut remainder = normalize_requirement_whitespace(value);
        let mut explicit_paths = Vec::new();
        for pattern in [
            self.parenthetical_concurrent_enrollment_pattern,
            self.concurrent_completion_pattern,
            self.concurrent_in_pattern,
        ] {
            let (next_remainder, paths) =
                self.remove_concurrent_pattern_matches(&remainder, pattern);
            remainder = next_remainder;
            explicit_paths.extend(paths);
        }
        (remainder, unique_requirement_paths(explicit_paths))
    }

    fn remove_concurrent_pattern_matches(
        &self,
        value: &str,
        pattern: &Regex,
    ) -> (String, Vec<Vec<String>>) {
        let captures = pattern
            .captures_iter(value)
            .filter_map(|capture| {
                let full_match = capture.get(0)?;
                let expression = capture.get(1)?.as_str().to_string();
                Some((full_match.start(), full_match.end(), expression))
            })
            .collect::<Vec<_>>();
        if captures.is_empty() {
            return (value.to_string(), Vec::new());
        }

        let mut remainder = String::with_capacity(value.len());
        let mut previous_end = 0;
        let mut paths = Vec::new();
        for (start, end, expression) in captures {
            remainder.push_str(&value[previous_end..start]);
            remainder.push(' ');
            paths.extend(self.parse_course_paths(&expression));
            previous_end = end;
        }
        remainder.push_str(&value[previous_end..]);
        (normalize_requirement_whitespace(&remainder), paths)
    }
}

fn normalize_requirement_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_requirement_chunks(value: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    for (index, character) in value.char_indices() {
        let is_boundary = character == ';'
            || (character == '.'
                && value[index + character.len_utf8()..]
                    .chars()
                    .next()
                    .map_or(true, char::is_whitespace));
        if is_boundary {
            chunks.push(&value[start..index]);
            start = index + character.len_utf8();
        }
    }
    chunks.push(&value[start..]);
    chunks
}

fn strip_leading_requirement_joiner(value: &str) -> (&str, &'static str) {
    let trimmed = value.trim();
    for joiner in ["or", "and"] {
        let Some(prefix) = trimmed.get(..joiner.len()) else {
            continue;
        };
        let is_word_boundary = trimmed[joiner.len()..]
            .chars()
            .next()
            .map_or(true, |character| {
                !character.is_ascii_alphanumeric() && character != '_'
            });
        if prefix.eq_ignore_ascii_case(joiner) && is_word_boundary {
            return (
                trimmed[joiner.len()..].trim_start(),
                if joiner == "or" { "or" } else { "and" },
            );
        }
    }
    (trimmed, "and")
}

fn find_simple_parenthetical(value: &str) -> Option<(usize, usize)> {
    for (start, character) in value.char_indices() {
        if character != '(' {
            continue;
        }
        for (relative_end, inner_character) in value[start + 1..].char_indices() {
            if inner_character == '(' {
                break;
            }
            if inner_character == ')' {
                let end = start + 1 + relative_end + inner_character.len_utf8();
                if end > start + 2 {
                    return Some((start, end));
                }
                break;
            }
        }
    }
    None
}

fn unique_requirement_paths(paths: Vec<Vec<String>>) -> Vec<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| !path.is_empty() && seen.insert(path.join("|")))
        .collect()
}

fn cross_combine_paths(
    left_paths: Vec<Vec<String>>,
    right_paths: Vec<Vec<String>>,
) -> Vec<Vec<String>> {
    if left_paths.is_empty() {
        return unique_requirement_paths(right_paths);
    }
    if right_paths.is_empty() {
        return unique_requirement_paths(left_paths);
    }
    unique_requirement_paths(
        left_paths
            .iter()
            .flat_map(|left| {
                right_paths.iter().map(move |right| {
                    let mut path = left.clone();
                    path.extend(right.iter().cloned());
                    path
                })
            })
            .collect(),
    )
}

fn merge_requirement_paths(
    existing_paths: Vec<Vec<String>>,
    incoming_paths: Vec<Vec<String>>,
    joiner: &str,
) -> Vec<Vec<String>> {
    if incoming_paths.is_empty() {
        return existing_paths;
    }
    if existing_paths.is_empty() {
        return unique_requirement_paths(incoming_paths);
    }
    if joiner == "or" {
        let mut paths = existing_paths;
        paths.extend(incoming_paths);
        unique_requirement_paths(paths)
    } else {
        cross_combine_paths(existing_paths, incoming_paths)
    }
}

fn remove_overlapping_paths(
    paths: Vec<Vec<String>>,
    overlapping_paths: &[Vec<String>],
) -> Vec<Vec<String>> {
    if paths.is_empty() || overlapping_paths.is_empty() {
        return paths;
    }
    paths
        .into_iter()
        .filter(|path| !overlapping_paths.contains(path))
        .collect()
}

fn apply_shared_prefix_to_paths(mut paths: Vec<Vec<String>>) -> Vec<Vec<String>> {
    if paths.len() < 2 || paths[0].len() < 2 {
        return unique_requirement_paths(paths);
    }
    let prefix = paths[0][..paths[0].len() - 1].to_vec();
    let anchor_subject = course_code_subject(paths[0].last().expect("path is non-empty"));
    let alternatives_share_subject = paths.iter().skip(1).all(|path| {
        path.len() == 1 && course_code_subject(&path[0]).eq_ignore_ascii_case(&anchor_subject)
    });
    if alternatives_share_subject {
        for path in paths.iter_mut().skip(1) {
            let mut expanded_path = prefix.clone();
            expanded_path.append(path);
            *path = expanded_path;
        }
    }
    unique_requirement_paths(paths)
}

fn course_code_subject(course_code: &str) -> String {
    course_code
        .rsplit_once(' ')
        .map(|(subject, _)| subject.to_uppercase())
        .unwrap_or_default()
}

fn extract_labeled_section(
    raw_body: &str,
    start_pattern: &Regex,
    section_boundary_pattern: &Regex,
    heading_boundary_pattern: &Regex,
) -> Option<String> {
    let start = start_pattern.find(raw_body)?;
    let remainder = &raw_body[start.end()..];
    let end = [
        section_boundary_pattern.find(remainder),
        heading_boundary_pattern.find(remainder),
    ]
    .into_iter()
    .flatten()
    .map(|boundary| boundary.start())
    .min()
    .unwrap_or(remainder.len());
    Some(remainder[..end].to_string())
}

fn extract_linked_anchor_texts(
    section_html: &str,
    anchor_pattern: &Regex,
    href_attribute_pattern: &Regex,
    strip_html_pattern: &Regex,
    entity_pattern: &Regex,
) -> Vec<String> {
    anchor_pattern
        .captures_iter(section_html)
        .filter_map(|capture| {
            let attributes = capture.get(1)?.as_str();
            if !href_attribute_pattern.is_match(attributes) {
                return None;
            }
            let anchor_text = capture.get(2)?.as_str();
            Some(strip_html(anchor_text, strip_html_pattern, entity_pattern))
        })
        .collect()
}

fn classify_general_education_tags(value: &str, patterns: &[Regex]) -> Vec<String> {
    const TAGS: [&str; 6] = ["COMM", "QSR", "AH", "SSC", "NSC", "DIV"];
    TAGS.iter()
        .zip(patterns)
        .filter(|(_, pattern)| pattern.is_match(value))
        .map(|(tag, _)| (*tag).to_string())
        .collect()
}

fn strip_html(value: &str, strip_html_pattern: &Regex, entity_pattern: &Regex) -> String {
    let without_tags = strip_html_pattern.replace_all(value, " ");
    normalize_whitespace(&without_tags, entity_pattern)
}

fn normalize_whitespace(value: &str, entity_pattern: &Regex) -> String {
    let decoded =
        entity_pattern.replace_all(value, |capture: &regex::Captures<'_>| {
            match capture[0].to_ascii_lowercase().as_str() {
                "&amp;" => "&",
                "&apos;" | "&#39;" => "'",
                "&nbsp;" | "&#160;" => " ",
                "&quot;" => "\"",
                "&#8211;" => "-",
                "&#8212;" => "-",
                "&#8217;" => "'",
                _ => "",
            }
        });
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{GrcCourseDetails, parse_grc_course_catalog, parse_grc_course_details};

    #[test]
    fn splits_catalog_html_into_ordered_heading_and_course_detail_sections() {
        let html = [
            "Catalog introduction",
            "<h3 class=\"course\">ENGL&amp; 101 - Composition</h3>",
            "Credits: 2.5<br><p>Writing &amp; research.</p>",
            "<h3>Table of contents</h3>",
            "Credits: Variable<br>Navigation body.",
        ]
        .join("");
        let sections = parse_grc_course_catalog(&html);

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading_text, "ENGL& 101 - Composition");
        assert_eq!(
            sections[0].course_code_candidate.as_deref(),
            Some("ENGL& 101")
        );
        assert_eq!(
            sections[0].course_title_candidate.as_deref(),
            Some("Composition")
        );
        assert!(sections[0].course_code_syntax_valid);
        assert_eq!(sections[0].details.credit_label.as_deref(), Some("2.5"));
        assert_eq!(
            sections[0].details.catalog_description.as_deref(),
            Some("Writing & research.")
        );
        assert_eq!(sections[1].heading_text, "Table of contents");
        assert_eq!(sections[1].course_code_candidate, None);
        assert_eq!(sections[1].course_title_candidate, None);
        assert!(!sections[1].course_code_syntax_valid);
        assert_eq!(
            sections[1].details.credit_label.as_deref(),
            Some("Variable")
        );
    }

    #[test]
    fn parses_credits_and_descriptions_without_crossing_requirement_sections() {
        let bodies = vec![
            "Credits: 5<br><br><strong>Enrollment Prerequisite:</strong> MATH& 151".to_string(),
            "<p>Course description &amp; overview</p><br><br><strong>Satisfies Requirement:</strong> Humanities".to_string(),
            "   Credits: 2.5<br><br><strong>Course Outcomes:</strong> Outcome text".to_string(),
            "Credits: Variable<br>Laboratory course description.".to_string(),
        ];
        assert_eq!(
            parse_grc_course_details(&bodies),
            vec![
                GrcCourseDetails {
                    credit_label: Some("5".to_string()),
                    credit_value: Some(5.0),
                    catalog_description: None,
                    enrollment_requirement_linked_anchor_texts: Vec::new(),
                    enrollment_requirement_linked_course_code_candidates: Vec::new(),
                    enrollment_requirement: Some("MATH& 151".to_string()),
                    enrollment_course_code_paths: Some(vec![vec!["MATH& 151".to_string()]]),
                    enrollment_corequisite_course_code_paths: None,
                    corequisite_requirement: None,
                    corequisite_course_code_paths: None,
                    satisfies_requirement: None,
                    general_education_tags: Vec::new(),
                },
                GrcCourseDetails {
                    credit_label: None,
                    credit_value: None,
                    catalog_description: Some("Course description & overview".to_string()),
                    satisfies_requirement: Some("Humanities".to_string()),
                    general_education_tags: vec!["AH".to_string()],
                    ..Default::default()
                },
                GrcCourseDetails {
                    credit_label: Some("2.5".to_string()),
                    credit_value: Some(2.5),
                    catalog_description: None,
                    satisfies_requirement: None,
                    general_education_tags: Vec::new(),
                    ..Default::default()
                },
                GrcCourseDetails {
                    credit_label: Some("Variable".to_string()),
                    credit_value: None,
                    catalog_description: Some("Laboratory course description.".to_string()),
                    satisfies_requirement: None,
                    general_education_tags: Vec::new(),
                    ..Default::default()
                },
            ]
        );
    }

    #[test]
    fn extracts_enrollment_and_corequisite_sections_without_heading_markup() {
        let body = "<strong>Enrollment Requirement:</strong> MATH& 151<br><br><strong>Corequisite:</strong> ENGL 101<br><br><strong>Course Outcomes:</strong> separate text".to_string();
        let details = parse_grc_course_details(&[body]);
        assert_eq!(
            details[0].enrollment_requirement.as_deref(),
            Some("MATH& 151")
        );
        assert_eq!(
            details[0].enrollment_course_code_paths,
            Some(vec![vec!["MATH& 151".to_string()]])
        );
        assert_eq!(
            details[0].corequisite_requirement.as_deref(),
            Some("ENGL 101")
        );
        assert_eq!(
            details[0].corequisite_course_code_paths,
            Some(vec![vec!["ENGL 101".to_string()]])
        );
    }

    #[test]
    fn parses_enrollment_course_paths_with_context_and_recommended_boundaries() {
        let bodies = vec![
            "<strong>Enrollment Requirement:</strong> MATH& 151 or ENGL& 101".to_string(),
            "<strong>Enrollment Requirement:</strong> MATH& 151 with grade of 2.0 or higher"
                .to_string(),
            "<strong>Enrollment Requirement:</strong> At least MATH& 151".to_string(),
            "<strong>Enrollment Requirement:</strong> MATH& 151 Recommended: MATH& 152".to_string(),
            "<strong>Enrollment Requirement:</strong> MATH& 151; ENGL& 101".to_string(),
            "<strong>Enrollment Requirement:</strong> MATH& 151 / 152".to_string(),
            "<strong>Enrollment Requirement:</strong> MATH& 151/152 with minimum grade of 2.0"
                .to_string(),
            "<strong>Enrollment Requirement:</strong> MATH 121 &amp; ENGL 101 + CHEM 111"
                .to_string(),
            "<strong>Enrollment Requirement:</strong> (MATH 121 or ENGL 101)".to_string(),
            "<strong>Enrollment Requirement:</strong> (MATH 121 or ENGL 101".to_string(),
            "<strong>Enrollment Requirement:</strong> MATH 121.".to_string(),
        ];
        let details = parse_grc_course_details(&bodies);

        assert_eq!(
            details[0].enrollment_course_code_paths,
            Some(vec![
                vec!["MATH& 151".to_string()],
                vec!["ENGL& 101".to_string()]
            ])
        );
        assert_eq!(
            details[1].enrollment_course_code_paths,
            Some(vec![vec!["MATH& 151".to_string()]])
        );
        assert_eq!(
            details[2].enrollment_course_code_paths,
            Some(vec![vec!["MATH& 151".to_string()]])
        );
        assert_eq!(
            details[3].enrollment_course_code_paths,
            Some(vec![vec!["MATH& 151".to_string()]])
        );
        assert_eq!(
            details[4].enrollment_course_code_paths,
            Some(vec![vec!["MATH& 151".to_string(), "ENGL& 101".to_string()]])
        );
        assert_eq!(
            details[5].enrollment_course_code_paths,
            Some(vec![
                vec!["MATH& 151".to_string()],
                vec!["MATH& 152".to_string()]
            ])
        );
        assert_eq!(
            details[6].enrollment_course_code_paths,
            Some(vec![
                vec!["MATH& 151".to_string()],
                vec!["MATH& 152".to_string()]
            ])
        );
        assert_eq!(
            details[7].enrollment_course_code_paths,
            Some(vec![vec![
                "MATH 121".to_string(),
                "ENGL 101".to_string(),
                "CHEM 111".to_string()
            ]])
        );
        assert_eq!(
            details[8].enrollment_course_code_paths,
            Some(vec![
                vec!["MATH 121".to_string()],
                vec!["ENGL 101".to_string()]
            ])
        );
        assert_eq!(
            details[9].enrollment_course_code_paths,
            Some(vec![
                vec!["MATH 121".to_string()],
                vec!["ENGL 101".to_string()]
            ])
        );
        assert_eq!(
            details[10].enrollment_course_code_paths,
            Some(vec![vec!["MATH 121".to_string()]])
        );
    }

    #[test]
    fn separates_explicit_concurrent_enrollment_from_prerequisite_paths() {
        let body =
            "<strong>Enrollment Requirement:</strong> MATH 110; concurrent enrollment in MATH 111"
                .to_string();
        let details = parse_grc_course_details(&[body]);

        assert_eq!(
            details[0].enrollment_course_code_paths,
            Some(vec![vec!["MATH 110".to_string()]])
        );
        assert_eq!(
            details[0].enrollment_corequisite_course_code_paths,
            Some(vec![vec!["MATH 111".to_string()]])
        );
    }

    #[test]
    fn nested_requirement_choices_preserve_shared_prerequisite_paths() {
        let body = "<strong>Enrollment Requirement:</strong> MATH 121 with a grade of 2.0 or higher and (ENGL 101 or CHEM 111)".to_string();
        let details = parse_grc_course_details(&[body]);

        assert_eq!(
            details[0].enrollment_course_code_paths,
            Some(vec![
                vec!["ENGL 101".to_string(), "MATH 121".to_string()],
                vec!["CHEM 111".to_string(), "MATH 121".to_string()]
            ])
        );
    }

    #[test]
    fn parses_plain_corequisite_and_or_paths_with_concurrent_enrollment_context() {
        let bodies = vec![
            "<strong>Corequisite:</strong> ENGL 101 or MATH 101".to_string(),
            "<strong>Corequisite:</strong> MATH 120 and MATH 121 or MATH 122".to_string(),
            "<strong>Corequisite:</strong> MATH 110 or concurrent enrollment in MATH 111"
                .to_string(),
            "<strong>Corequisite:</strong> MATH 120/121".to_string(),
        ];
        let details = parse_grc_course_details(&bodies);

        assert_eq!(
            details[0].corequisite_course_code_paths,
            Some(vec![
                vec!["ENGL 101".to_string()],
                vec!["MATH 101".to_string()]
            ])
        );
        assert_eq!(
            details[1].corequisite_course_code_paths,
            Some(vec![
                vec!["MATH 120".to_string(), "MATH 121".to_string()],
                vec!["MATH 120".to_string(), "MATH 122".to_string()]
            ])
        );
        assert_eq!(
            details[2].corequisite_course_code_paths,
            Some(vec![vec!["MATH 110".to_string(), "MATH 111".to_string()]])
        );
        assert_eq!(
            details[3].corequisite_course_code_paths,
            Some(vec![
                vec!["MATH 120".to_string()],
                vec!["MATH 121".to_string()]
            ])
        );
    }

    #[test]
    fn extracts_only_hyperlinked_course_anchor_texts_from_enrollment_sections() {
        let body = "<strong>Enrollment Requirement:</strong><a class=\"catalog-course\" href=\"course-151\">MATH&amp; 151</a><a data-href=\"course-199\">MATH&amp; 199</a><a HREF=\"course-152\">MATH&amp; <em>152</em></a><br><br><strong>Course Outcomes:</strong> separate text".to_string();
        let details = parse_grc_course_details(&[body]);
        assert_eq!(
            details[0].enrollment_requirement_linked_anchor_texts,
            vec!["MATH& 151", "MATH& 152"]
        );
        assert_eq!(
            details[0].enrollment_requirement_linked_course_code_candidates,
            vec!["MATH& 151", "MATH& 152"]
        );
    }

    #[test]
    fn linked_course_candidates_expand_slashes_and_ignore_non_course_context() {
        let body = concat!(
            "<strong>Enrollment Requirement:</strong>",
            "<a href=\"a\">eligible for MATH 121</a>",
            "<a href=\"b\">equivalent of MATH 122</a>",
            "<a href=\"c\">admission into MATH 123</a>",
            "<a href=\"d\">MATH 124 with two years in high school</a>",
            "<a href=\"e\">MATH 125/126</a>"
        )
        .to_string();
        let details = parse_grc_course_details(&[body]);
        assert_eq!(
            details[0].enrollment_requirement_linked_course_code_candidates,
            vec!["MATH 125", "MATH 126"]
        );
    }

    #[test]
    fn classifies_general_education_tags_in_the_catalog_order() {
        let body = "<strong>Satisfies Requirement:</strong> Written Communication, Quantitative Skills, Fine Arts, Social Sciences, Natural Science, and Diversity<br><br><strong>Course Outcomes:</strong> separate text".to_string();
        let details = parse_grc_course_details(&[body]);
        assert_eq!(
            details[0].satisfies_requirement.as_deref(),
            Some(
                "Written Communication, Quantitative Skills, Fine Arts, Social Sciences, Natural Science, and Diversity"
            )
        );
        assert_eq!(
            details[0].general_education_tags,
            vec!["COMM", "QSR", "AH", "SSC", "NSC", "DIV"]
        );
    }

    #[test]
    fn keeps_an_empty_matched_credit_label_distinct_from_a_missing_label() {
        let bodies = vec![
            "Credits:   <br>Description".to_string(),
            "Description only".to_string(),
        ];
        let details = parse_grc_course_details(&bodies);
        assert_eq!(details[0].credit_label.as_deref(), Some(""));
        assert_eq!(details[0].credit_value, None);
        assert_eq!(details[1].credit_label, None);
    }
}
