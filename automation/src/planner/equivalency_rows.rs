use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EquivalencyGuideRow {
    pub department: String,
    pub obsolete: bool,
    pub row_number: usize,
    pub cccourse: String,
    pub uwequiv: String,
    pub source_course_set: Vec<String>,
    pub target_course_codes: Vec<String>,
    pub target_requirement_tags: Vec<String>,
    pub effective_year_ranges: Vec<EffectiveYearRange>,
    pub rule_type: String,
    pub acceptance_category: String,
    pub rule_status: String,
    pub planner_warnings: Vec<String>,
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_rule: Option<GeneratedEquivalencyRule>,
    pub uwreqs: String,
    pub effdate: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveYearRange {
    pub start_label: String,
    pub end_label: Option<String>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedEquivalencyRule {
    pub id: String,
    #[serde(rename = "type")]
    pub rule_type: String,
    pub title: String,
    pub acceptance_category: String,
    pub rule_status: String,
    pub source_kind: String,
    pub source_school_id: String,
    pub target_school_ids: Vec<String>,
    pub source_course_sets: Vec<Vec<String>>,
    pub target_course_codes: Vec<String>,
    pub target_outcome: String,
    pub weaker_than_rule_ids: Vec<String>,
    pub effective_year_ranges: Vec<EffectiveYearRange>,
    pub effective_date_label: Option<String>,
    pub guide_department: String,
    pub source_course_label: String,
    pub source_course_title: Option<String>,
    pub target_requirement_tags: Vec<String>,
    pub is_obsolete_source_course: bool,
    pub parsed_from_official_guide: bool,
    pub planner_warnings: Vec<String>,
    pub notes: Vec<String>,
    pub source_links: Vec<EquivalencySourceLink>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EquivalencySourceLink {
    pub label: String,
    pub url: String,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EquivalencyGuideParseInput {
    pub html: String,
    pub grc_course_titles: HashMap<String, String>,
}

pub fn parse_equivalency_guide_rows(html: &str) -> Vec<EquivalencyGuideRow> {
    parse_equivalency_guide_rows_with_titles(html, &HashMap::new())
}

pub fn parse_equivalency_guide_rows_with_titles(
    html: &str,
    course_titles: &HashMap<String, String>,
) -> Vec<EquivalencyGuideRow> {
    let heading_pattern = Regex::new(r"(?is)<h3(?:\s+[^>]*)?>(.*?)</h3>").unwrap();
    let headings: Vec<_> = heading_pattern
        .captures_iter(html)
        .filter_map(|captures| {
            let whole = captures.get(0)?;
            let body = captures.get(1)?;
            Some((whole.start(), whole.end(), strip_html(body.as_str())))
        })
        .collect();
    let table_pattern = Regex::new(
        r#"(?is)<table\b[^>]*class="[^"]*table[^"]*"[\s\S]*?<tbody>([\s\S]*?)</tbody>\s*</table>"#,
    )
    .unwrap();
    let row_pattern = Regex::new(r"(?is)<tr(?:\s+([^>]*))?>(.*?)</tr>").unwrap();
    let cell_pattern =
        Regex::new(r#"(?is)<td\b[^>]*class="(cccourse|uwequiv|uwreqs|effdate)"[^>]*>(.*?)</td>"#)
            .unwrap();
    let mut rows = Vec::new();

    for (index, (_, section_start, department)) in headings.iter().enumerate() {
        let section_end = headings
            .get(index + 1)
            .map(|next| next.0)
            .unwrap_or(html.len());
        let section_html = &html[*section_start..section_end];
        let Some(table) = table_pattern.captures(section_html) else {
            continue;
        };
        let Some(table_body) = table.get(1) else {
            continue;
        };
        for row_match in row_pattern.captures_iter(table_body.as_str()) {
            let row_attributes = row_match
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let row_html = row_match
                .get(2)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let mut values = std::collections::HashMap::<&str, String>::new();
            let mut matched_cell_count = 0;
            for cell in cell_pattern.captures_iter(row_html) {
                matched_cell_count += 1;
                let key = cell.get(1).map(|value| value.as_str()).unwrap_or_default();
                let value = cell.get(2).map(|value| value.as_str()).unwrap_or_default();
                values.insert(key, strip_html(value));
            }
            if matched_cell_count < 4 {
                continue;
            }
            let cccourse =
                canonicalize_legacy_codes_in_text(&values.remove("cccourse").unwrap_or_default());
            let uwequiv =
                canonicalize_legacy_codes_in_text(&values.remove("uwequiv").unwrap_or_default());
            let uwreqs =
                canonicalize_legacy_codes_in_text(&values.remove("uwreqs").unwrap_or_default());
            let effdate =
                canonicalize_legacy_codes_in_text(&values.remove("effdate").unwrap_or_default());
            let mut row = EquivalencyGuideRow {
                department: department.clone(),
                obsolete: row_attributes.contains("obsRow"),
                row_number: rows.len() + 1,
                source_course_set: parse_source_course_set(&cccourse),
                target_course_codes: parse_target_course_codes(&uwequiv),
                target_requirement_tags: parse_requirement_tags(&uwreqs),
                effective_year_ranges: parse_effective_year_ranges(&effdate),
                rule_type: String::new(),
                acceptance_category: String::new(),
                rule_status: String::new(),
                planner_warnings: Vec::new(),
                notes: Vec::new(),
                generated_rule: None,
                cccourse,
                uwequiv,
                uwreqs,
                effdate,
            };
            row.rule_type = classify_rule(&row);
            row.acceptance_category = build_acceptance_category(&row);
            row.rule_status = build_rule_status(&row);
            row.planner_warnings = build_warnings(&row);
            row.notes = build_notes(&row);
            row.generated_rule = Some(build_generated_rule(&row, course_titles));
            rows.push(row);
        }
    }
    rows
}

fn canonicalize_legacy_codes_in_text(value: &str) -> String {
    Regex::new(r"(?i)\bMATH&\s+254\b")
        .unwrap()
        .replace_all(&normalize_whitespace(value), "MATH& 264")
        .into_owned()
}

fn classify_rule(row: &EquivalencyGuideRow) -> String {
    if Regex::new(r"(?i)^No credit")
        .unwrap()
        .is_match(&row.uwequiv)
    {
        return "no-credit".to_string();
    }
    if Regex::new(r"(?i)\(\s*LC\s*\)|\bLC\b|\[\s*\d+\s+credits?\s+allowed\s*\]")
        .unwrap()
        .is_match(&format!("{} {} {}", row.cccourse, row.uwequiv, row.uwreqs))
    {
        return "limited-credit".to_string();
    }
    if row.source_course_set.len() > 1
        || Regex::new(r"(?i)\bif\s+(?:all|both)\b")
            .unwrap()
            .is_match(&row.uwequiv)
        || Regex::new(r"(?i)\bcombined entr(?:y|ies)\b")
            .unwrap()
            .is_match(&row.cccourse)
        || Regex::new(r"(?i)\botherwise\b")
            .unwrap()
            .is_match(&row.uwequiv)
    {
        return "sequence".to_string();
    }
    if !Regex::new(r"(?i)^No credit")
        .unwrap()
        .is_match(&row.uwequiv)
    {
        if row.target_course_codes.is_empty() {
            if Regex::new(r"(?i)\b(?:UW|[A-Z][A-Z &-]{0,14})\s+[1-4]XX\b")
                .unwrap()
                .is_match(&row.uwequiv)
            {
                return "elective-credit".to_string();
            }
        } else if row
            .target_course_codes
            .iter()
            .all(|code| Regex::new(r"\b[1-4]XX$").unwrap().is_match(code))
        {
            return "elective-credit".to_string();
        }
    }
    "direct-course".to_string()
}

fn build_acceptance_category(row: &EquivalencyGuideRow) -> String {
    if row.rule_type == "no-credit" {
        return "no-credit".to_string();
    }
    if row.obsolete {
        return "legacy-accepted".to_string();
    }
    if row.rule_type == "limited-credit"
        || row.rule_type == "sequence"
        || Regex::new(
            r"(?i)\b(?:otherwise|if course content|see individual|see .*combined entr(?:y|ies))\b",
        )
        .unwrap()
        .is_match(&format!("{} {}", row.cccourse, row.uwequiv))
    {
        return "accepted-with-warning".to_string();
    }
    "accepted".to_string()
}

fn build_rule_status(row: &EquivalencyGuideRow) -> String {
    if row.obsolete
        || Regex::new(r"(?i)\b(?:Prior to|thru)\b")
            .unwrap()
            .is_match(&row.effdate)
    {
        "legacy".to_string()
    } else {
        "active".to_string()
    }
}

fn build_warnings(row: &EquivalencyGuideRow) -> Vec<String> {
    let mut warnings = Vec::new();
    if row.obsolete {
        warnings.push(
            "The official guide marks this Green River course row as no longer offered."
                .to_string(),
        );
    }
    if row.rule_type == "sequence" {
        warnings.push(if row.source_course_set.len() > 1 {
            "The official guide describes this as a sequence or combined-course equivalency; do not award the strongest target outcome from a partial sequence.".to_string()
        } else {
            "The official guide references a combined-entry or conditional equivalency; preserve the row text until the sequence resolver can evaluate the full student history.".to_string()
        });
    }
    if row.rule_type == "limited-credit" {
        warnings.push("The official guide flags this row as limited credit; apply the published cap/discipline note before using it in planning.".to_string());
    }
    if row.rule_type == "no-credit" {
        warnings.push(
            "The official guide explicitly assigns no UW transfer credit for this row.".to_string(),
        );
    }
    if Regex::new(r"(?i)\botherwise\b")
        .unwrap()
        .is_match(&row.uwequiv)
    {
        warnings.push(format!("Official fallback text: {}", row.uwequiv));
    }
    warnings
}

fn build_notes(row: &EquivalencyGuideRow) -> Vec<String> {
    let mut notes = vec![
        format!("UW guide department: {}.", row.department),
        format!("Official Green River row: {}.", row.cccourse),
        format!(
            "Official UW equivalency row: {}.",
            if row.uwequiv.is_empty() {
                "(blank; see row wording)"
            } else {
                &row.uwequiv
            }
        ),
    ];
    if !row.uwreqs.is_empty() {
        notes.push(format!("Official UW requirement tags: {}.", row.uwreqs));
    }
    if !row.effdate.is_empty() {
        notes.push(format!("Official effective date: {}.", row.effdate));
    }
    if !row.target_course_codes.is_empty() {
        notes.push(format!(
            "Parsed primary UW target course codes: {}.",
            row.target_course_codes.join(", ")
        ));
    }
    notes
}

fn build_generated_rule(
    row: &EquivalencyGuideRow,
    course_titles: &HashMap<String, String>,
) -> GeneratedEquivalencyRule {
    let department_slug = slugify(&row.department);
    let course_slug = slugify(&row.cccourse);
    let mut source_titles = Vec::new();
    for course_code in &row.source_course_set {
        if let Some(title) = course_titles.get(course_code) {
            if !title.is_empty() && !source_titles.contains(title) {
                source_titles.push(title.clone());
            }
        }
    }
    let source_course_title = if source_titles.is_empty() {
        None
    } else {
        Some(source_titles.join(" / "))
    };
    GeneratedEquivalencyRule {
        id: format!("uw-grc-guide:{:04}:{}:{}", row.row_number, department_slug, course_slug),
        rule_type: row.rule_type.clone(),
        title: format!("{} -> {}", row.cccourse, if row.uwequiv.is_empty() { "see combined-entry row" } else { &row.uwequiv }),
        acceptance_category: row.acceptance_category.clone(),
        rule_status: row.rule_status.clone(),
        source_kind: "uw-green-river-equivalency-guide".to_string(),
        source_school_id: "grc".to_string(),
        target_school_ids: vec!["uw-seattle".to_string()],
        source_course_sets: if row.source_course_set.is_empty() { Vec::new() } else { vec![row.source_course_set.clone()] },
        target_course_codes: row.target_course_codes.clone(),
        target_outcome: if row.uwequiv.is_empty() {
            "See the official combined-entry wording on the Green River equivalency guide.".to_string()
        } else {
            row.uwequiv.clone()
        },
        weaker_than_rule_ids: Vec::new(),
        effective_year_ranges: row.effective_year_ranges.clone(),
        effective_date_label: if row.effdate.is_empty() { None } else { Some(row.effdate.clone()) },
        guide_department: row.department.clone(),
        source_course_label: row.cccourse.clone(),
        source_course_title,
        target_requirement_tags: row.target_requirement_tags.clone(),
        is_obsolete_source_course: row.obsolete,
        parsed_from_official_guide: true,
        planner_warnings: row.planner_warnings.clone(),
        notes: row.notes.clone(),
        source_links: vec![EquivalencySourceLink {
            label: "UW Green River transfer equivalency guide".to_string(),
            url: "https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/".to_string(),
            note: "Equivalency row parsed from the official UW Office of Admissions Green River transfer equivalency guide.".to_string(),
        }],
    }
}

fn slugify(value: &str) -> String {
    let normalized = normalize_whitespace(value)
        .to_lowercase()
        .replace('&', "and");
    let non_ascii_alphanumeric = Regex::new(r"[^a-z0-9]+").unwrap();
    let slug = non_ascii_alphanumeric
        .replace_all(&normalized, "-")
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "unknown".to_string()
    } else {
        slug
    }
}

fn parse_requirement_tags(value: &str) -> Vec<String> {
    let normalized = normalize_whitespace(value);
    if normalized.is_empty() {
        return Vec::new();
    }
    let brackets = Regex::new(r"\[[^\]]+\]").unwrap();
    let credits = Regex::new(r"\(\s*\d+(?:\.\d+)?\s*\)").unwrap();
    let cleaned =
        normalize_whitespace(&credits.replace_all(&brackets.replace_all(&normalized, " "), " "));
    let checks = [
        (r"(?i)\bA\s*&\s*H\b", "A&H"),
        (r"(?i)\bS\s*Sc\b", "SSc"),
        (r"(?i)\bN\s*Sc\b", "NSc"),
        (r"(?i)\bQSR\b", "QSR"),
        (r"(?i)\bVLPA\b", "VLPA"),
        (r"(?i)\bI\s*&\s*S\b", "I&S"),
        (r"(?i)\bDIV\b", "DIV"),
        (r"(?i)\bNW\b", "NW"),
    ];
    checks
        .into_iter()
        .filter_map(|(pattern, tag)| {
            Regex::new(pattern)
                .unwrap()
                .is_match(&cleaned)
                .then(|| tag.to_string())
        })
        .collect()
}

fn parse_effective_year_ranges(value: &str) -> Vec<EffectiveYearRange> {
    let label = normalize_whitespace(value);
    if label.is_empty() {
        return Vec::new();
    }
    if let Some(captures) = Regex::new(r"(?i)^(.+?)\s+thru\s+(.+)$")
        .unwrap()
        .captures(&label)
    {
        return vec![EffectiveYearRange {
            start_label: normalize_whitespace(captures.get(1).unwrap().as_str()),
            end_label: Some(normalize_whitespace(captures.get(2).unwrap().as_str())),
            note: "Effective-date range parsed from the official UW Green River equivalency guide."
                .to_string(),
        }];
    }
    if let Some(captures) = Regex::new(r"(?i)^Prior to\s+(.+)$")
        .unwrap()
        .captures(&label)
    {
        return vec![EffectiveYearRange {
            start_label: "prior-to-guide-cutoff".to_string(),
            end_label: Some(normalize_whitespace(captures.get(1).unwrap().as_str())),
            note: "Legacy effective-date cutoff parsed from the official UW Green River equivalency guide.".to_string(),
        }];
    }
    vec![EffectiveYearRange {
        start_label: label,
        end_label: None,
        note: "Effective-date start parsed from the official UW Green River equivalency guide."
            .to_string(),
    }]
}

fn parse_source_course_set(value: &str) -> Vec<String> {
    let mut text = normalize_whitespace(value);
    let leading_marker = Regex::new(r"^[§*]\s*").unwrap();
    text = leading_marker.replace(&text, "").into_owned();
    let note =
        Regex::new(r"(?i)\s+(?:formerly|see(?:\s+also)?|same as|was\b|combined entr(?:y|ies))\b")
            .unwrap();
    if let Some(found) = note.find(&text) {
        text.truncate(found.start());
    }
    let parenthetical = Regex::new(r"\([^)]*\)").unwrap();
    text = parenthetical.replace_all(&text, " ").into_owned();
    parse_course_codes_with_subject_carry(&text)
}

fn parse_target_course_codes(value: &str) -> Vec<String> {
    let normalized = normalize_whitespace(value);
    let stop = Regex::new(r"(?i)\bif\b|\botherwise\b|;").unwrap();
    let mut primary = stop
        .split(&normalized)
        .next()
        .unwrap_or_default()
        .to_string();
    let alternative = Regex::new(r"(?i)\bor\b").unwrap();
    primary = alternative.replace_all(&primary, ",").into_owned();
    parse_course_codes_with_subject_carry(&primary)
}

fn parse_course_codes_with_subject_carry(value: &str) -> Vec<String> {
    let marker = Regex::new(r"^[§*]\s*").unwrap();
    let parentheses = Regex::new(r"\([^)]*\)").unwrap();
    let full_course =
        Regex::new(r"(?i)^([A-Z][A-Z& -]{0,14}&?)\s+(\d{3}(?:\.\d+)?[A-Z]?|[1-4]XX)\b").unwrap();
    let carried_number = Regex::new(r"(?i)^(\d{3}(?:\.\d+)?[A-Z]?|[1-4]XX)\b").unwrap();
    let mut current_subject = None::<String>;
    let mut codes = Vec::new();
    for raw_part in parentheses.replace_all(value, " ").split(',') {
        let part = normalize_whitespace(&marker.replace(raw_part, ""));
        if part.is_empty() {
            continue;
        }
        if let Some(captures) = full_course.captures(&part) {
            let subject = normalize_course_subject(captures.get(1).unwrap().as_str());
            current_subject = Some(subject.clone());
            push_unique_code(
                &mut codes,
                normalize_course_code_from_parts(&subject, captures.get(2).unwrap().as_str()),
            );
            continue;
        }
        if let (Some(subject), Some(captures)) =
            (current_subject.as_ref(), carried_number.captures(&part))
        {
            push_unique_code(
                &mut codes,
                normalize_course_code_from_parts(subject, captures.get(1).unwrap().as_str()),
            );
        }
    }
    codes
}

fn normalize_course_subject(value: &str) -> String {
    let normalized = normalize_whitespace(value).to_ascii_uppercase();
    let hyphens = Regex::new(r"\s*-\s*").unwrap();
    hyphens
        .replace_all(&normalized, "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_course_code_from_parts(subject: &str, number: &str) -> String {
    let code = format!(
        "{} {}",
        normalize_course_subject(subject),
        normalize_whitespace(number).to_ascii_uppercase()
    );
    if code == "MATH& 254" {
        "MATH& 264".to_string()
    } else {
        code
    }
}

fn push_unique_code(codes: &mut Vec<String>, code: String) {
    if !codes.contains(&code) {
        codes.push(code);
    }
}

fn strip_html(value: &str) -> String {
    let tags = Regex::new(r"(?s)<[^>]+>").unwrap();
    normalize_whitespace(&tags.replace_all(value, " "))
}

fn normalize_whitespace(value: &str) -> String {
    decode_html_entities(value)
        .replace("Â§", "§")
        .replace("Â ", " ")
        .replace('\u{00a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_html_entities(value: &str) -> String {
    let entity_pattern =
        Regex::new(r"(?i)&(amp|apos|nbsp|quot|#038|#39|#160|#8211|#8212|#8217);").unwrap();
    entity_pattern
        .replace_all(value, |captures: &regex::Captures<'_>| {
            match captures
                .get(1)
                .map(|value| value.as_str().to_ascii_lowercase())
                .as_deref()
            {
                Some("amp" | "#038") => "&",
                Some("apos" | "#39" | "#8217") => "'",
                Some("nbsp" | "#160") => " ",
                Some("quot") => "\"",
                Some("#8211" | "#8212") => "-",
                _ => captures
                    .get(0)
                    .map(|value| value.as_str())
                    .unwrap_or_default(),
            }
            .to_string()
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rows_by_department_and_preserves_obsolete_and_entity_semantics() {
        let html = r#"
          <h3 class="department">Biology &amp; Health</h3>
          <table class="table table-striped"><tbody>
            <tr class="obsRow"><td class="cccourse">BIOL&#160;100</td><td class="uwequiv">BIOL&#8211;180</td><td class="uwreqs">Natural &amp; Life Sciences</td><td class="effdate">Before 2018</td></tr>
            <tr><td class="cccourse">BIOL 101</td><td class="uwequiv">BIOL 181</td><td class="uwreqs">Biology</td><td class="effdate">2019</td></tr>
            <tr><td class="cccourse">ignored row</td></tr>
          </tbody></table>
          <h3>Computer Science</h3>
          <table class="table"><tbody>
            <tr><td class="cccourse">CS 121</td><td class="uwequiv">CSE 121</td><td class="uwreqs">QSR</td><td class="effdate">2020</td></tr>
          </tbody></table>
        "#;
        let rows = parse_equivalency_guide_rows(html);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].department, "Biology & Health");
        assert_eq!(rows[0].row_number, 1);
        assert!(rows[0].obsolete);
        assert_eq!(rows[0].cccourse, "BIOL 100");
        assert_eq!(rows[0].uwequiv, "BIOL-180");
        assert_eq!(rows[0].uwreqs, "Natural & Life Sciences");
        assert_eq!(rows[0].source_course_set, vec!["BIOL 100"]);
        assert!(rows[0].target_course_codes.is_empty());
        assert_eq!(rows[0].rule_type, "direct-course");
        assert_eq!(rows[0].acceptance_category, "legacy-accepted");
        assert_eq!(rows[0].rule_status, "legacy");
        assert_eq!(rows[0].planner_warnings.len(), 1);
        assert_eq!(rows[0].notes[0], "UW guide department: Biology & Health.");
        let generated = rows[0].generated_rule.as_ref().unwrap();
        assert_eq!(
            generated.id,
            "uw-grc-guide:0001:biology-and-health:biol-100"
        );
        assert_eq!(generated.rule_type, "direct-course");
        assert_eq!(generated.target_school_ids, vec!["uw-seattle"]);
        assert_eq!(
            generated.effective_date_label.as_deref(),
            Some("Before 2018")
        );
        assert_eq!(
            generated.source_links[0].label,
            "UW Green River transfer equivalency guide"
        );
        assert_eq!(rows[1].row_number, 2);
        assert_eq!(rows[1].target_course_codes, vec!["BIOL 181"]);
        assert!(!rows[1].obsolete);
        assert_eq!(rows[2].department, "Computer Science");
        assert_eq!(rows[2].row_number, 3);
        assert_eq!(rows[2].target_requirement_tags, vec!["QSR"]);
        assert_eq!(rows[2].effective_year_ranges[0].start_label, "2020");
        assert_eq!(rows[2].effective_year_ranges[0].end_label, None);
    }

    #[test]
    fn generated_rule_enriches_source_titles_in_course_order_without_duplicates() {
        let html = r#"<h3>Math</h3><table class="table"><tbody>
          <tr><td class="cccourse">MATH& 151, 152, 163</td><td class="uwequiv">MATH 124, 125, 126</td><td class="uwreqs">QSR</td><td class="effdate">2020</td></tr>
        </tbody></table>"#;
        let titles = HashMap::from([
            ("MATH& 151".to_string(), "Calculus I".to_string()),
            ("MATH& 152".to_string(), "Calculus II".to_string()),
            ("MATH& 163".to_string(), "Calculus II".to_string()),
        ]);
        let rows = parse_equivalency_guide_rows_with_titles(html, &titles);
        assert_eq!(
            rows[0]
                .generated_rule
                .as_ref()
                .unwrap()
                .source_course_title
                .as_deref(),
            Some("Calculus I / Calculus II")
        );
    }

    #[test]
    fn parses_carried_subjects_alternatives_and_legacy_course_aliases() {
        assert_eq!(
            parse_source_course_set("MATH& 151, 152, 163; see also the older path"),
            vec!["MATH& 151", "MATH& 152", "MATH& 163"]
        );
        assert_eq!(
            parse_source_course_set("MATH& 254 formerly MATH& 163"),
            vec!["MATH& 264"]
        );
        assert_eq!(
            parse_target_course_codes("MATH 124 or MATH 125 if the student completes the sequence"),
            vec!["MATH 124", "MATH 125"]
        );
        assert_eq!(
            parse_target_course_codes("BIOL 180; otherwise BIOL 181"),
            vec!["BIOL 180"]
        );
    }

    #[test]
    fn parses_requirement_tags_and_effective_date_ranges_in_catalog_order() {
        assert_eq!(
            parse_requirement_tags("A & H (5) [campus note] S Sc N Sc QSR VLPA I & S DIV NW"),
            vec!["A&H", "SSc", "NSc", "QSR", "VLPA", "I&S", "DIV", "NW"]
        );
        let thru = parse_effective_year_ranges("Autumn 2018 thru Spring 2021");
        assert_eq!(thru[0].start_label, "Autumn 2018");
        assert_eq!(thru[0].end_label.as_deref(), Some("Spring 2021"));
        let prior = parse_effective_year_ranges("Prior to 2019");
        assert_eq!(prior[0].start_label, "prior-to-guide-cutoff");
        assert_eq!(prior[0].end_label.as_deref(), Some("2019"));
    }

    #[test]
    fn classifies_rows_and_builds_warnings_and_notes_deterministically() {
        let html = r#"<h3>Department</h3><table class="table"><tbody>
          <tr><td class="cccourse">BIOL 100</td><td class="uwequiv">No credit</td><td class="uwreqs">NW</td><td class="effdate">2020</td></tr>
          <tr><td class="cccourse">MATH 100 (LC)</td><td class="uwequiv">MATH 100 [5 credits allowed]</td><td class="uwreqs">QSR</td><td class="effdate">2020</td></tr>
          <tr><td class="cccourse">MATH 151, 152</td><td class="uwequiv">MATH 124 if all courses are completed; otherwise MATH 125</td><td class="uwreqs">QSR</td><td class="effdate">Prior to 2019</td></tr>
          <tr><td class="cccourse">ART 100</td><td class="uwequiv">UW 1XX</td><td class="uwreqs"></td><td class="effdate">2020</td></tr>
          <tr><td class="cccourse">ART 101</td><td class="uwequiv">ART 101</td><td class="uwreqs"></td><td class="effdate">2020</td></tr>
        </tbody></table>"#;
        let rows = parse_equivalency_guide_rows(html);
        assert_eq!(
            rows.iter()
                .map(|row| row.rule_type.as_str())
                .collect::<Vec<_>>(),
            vec![
                "no-credit",
                "limited-credit",
                "sequence",
                "elective-credit",
                "direct-course"
            ]
        );
        assert_eq!(rows[0].acceptance_category, "no-credit");
        assert_eq!(
            rows[0].planner_warnings[0],
            "The official guide explicitly assigns no UW transfer credit for this row."
        );
        assert_eq!(rows[1].acceptance_category, "accepted-with-warning");
        assert_eq!(rows[2].rule_status, "legacy");
        assert_eq!(rows[2].planner_warnings.len(), 2);
        assert_eq!(rows[3].acceptance_category, "accepted");
        assert_eq!(
            rows[4].notes.last().unwrap(),
            "Parsed primary UW target course codes: ART 101."
        );
    }

    #[test]
    fn ignores_tables_without_the_official_table_class_or_required_cells() {
        let html = r#"<h3>Department</h3><table><tbody><tr>
            <td class="cccourse">X 100</td><td class="uwequiv">Y 100</td>
            <td class="uwreqs">Tag</td><td class="effdate">2020</td>
          </tr></tbody></table>"#;
        assert!(parse_equivalency_guide_rows(html).is_empty());
    }
}
