use super::requirement_category_options::build_category_requirement_options_from_choice_line;
use super::requirement_choice_lists::build_forward_labeled_choice_requirement_lines;
use super::requirement_course_codes::{CourseCodeParser, normalize_course_code};
use super::requirement_ids::slugify_requirement_identifier;
use super::requirement_parse_helpers::{
    parse_requirement_credit_amount, parse_requirement_credit_range,
};
use super::requirement_shape::{
    CHOICE_COUNT_WORDS_PATTERN, CHOICE_LIST_START_PATTERN_SOURCE,
    CHOICE_REQUIREMENT_LABEL_PATTERN_SOURCE,
    detect_option_cue, parse_choice_required_count,
};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

pub fn is_numbered_option_row_under_choose_one_heading(
    snapshot_lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_snapshot_page_prefix(
            snapshot_lines.get(index).map(String::as_str).unwrap_or_default(),
        ),
    );
    if !numbered_inline_option_row_pattern().is_match(&normalized_line)
        || course_parser
            .extract_from_requirement_line(&normalized_line)
            .is_empty()
    {
        return false;
    }
    let start = index.saturating_sub(24);
    for candidate_index in (start..=index).rev() {
        let candidate_line = normalize_whitespace(
            &super::requirement_choice_lists::strip_snapshot_page_prefix(
                snapshot_lines
                    .get(candidate_index)
                    .map(String::as_str)
                    .unwrap_or_default(),
            ),
        );
        if choose_one_option_heading_pattern().is_match(&candidate_line) {
            return true;
        }
        if candidate_index < index
            && !numbered_inline_option_row_pattern().is_match(&candidate_line)
            && super::requirement_shape::looks_like_standalone_requirement_title_line(
                &candidate_line,
            )
        {
            return false;
        }
    }
    false
}

pub fn looks_like_degree_total_general_elective_prose(text: &str) -> bool {
    let normalized_text = normalize_whitespace(text);
    if degree_total_free_electives_pattern().is_match(&normalized_text) {
        return true;
    }
    if degree_total_music_program_pattern().is_match(&normalized_text) {
        return true;
    }
    degree_total_general_elective_cue_pattern().is_match(&normalized_text)
        && (degree_total_general_elective_pattern().is_match(&normalized_text)
            || degree_total_elective_total_pattern().is_match(&normalized_text)
            || degree_total_general_elective_depending_pattern().is_match(&normalized_text))
}

pub fn looks_like_credit_summary_for_named_requirement_section(text: &str) -> bool {
    named_requirement_credit_summary_pattern().is_match(&normalize_whitespace(text))
}

pub fn build_category_requirement_options_from_credit_bucket(
    owner: &Value,
    text: &str,
    credit_range: Option<(f64, Option<f64>)>,
    descriptor: Option<&Value>,
) -> Vec<Value> {
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let credit_range_value = credit_range
        .map(|(minimum, maximum)| json!({"minCredits": minimum, "maxCredits": maximum}))
        .unwrap_or(Value::Null);
    if let Some(descriptor) = descriptor {
        let id_source = descriptor
            .get("approvedListKey")
            .filter(|value| !value.is_null())
            .or_else(|| descriptor.get("sourceCategoryCode"))
            .map(js_string_value)
            .unwrap_or_else(|| "undefined".to_string());
        return super::requirement_category_options::build_category_requirement_option(&json!({
            "id": format!(
                "{owner_id}:requirement-option:{}",
                slugify_requirement_identifier(&id_source)
            ),
            "sourceText": text,
            "sourceHeading": text,
            "sourceCategory": "credit-bucket",
            "descriptor": descriptor,
            "creditRange": credit_range_value,
            "approvedListKey": descriptor.get("approvedListKey").cloned().unwrap_or(Value::Null),
            "programSpecific": true,
        }))
        .into_iter()
        .collect();
    }
    let positive_text = get_positive_credit_bucket_category_text(text);
    let mut seen = HashSet::new();
    super::requirement_category_options::get_requirement_category_option_descriptors(
        &positive_text,
    )
    .into_iter()
    .filter_map(|descriptor| {
        let source_category_code = descriptor
            .get("sourceCategoryCode")
            .map(js_string_value)
            .unwrap_or_else(|| "undefined".to_string());
        let id_suffix = slugify_requirement_identifier(&format!("{source_category_code}-{text}"));
        let option = super::requirement_category_options::build_category_requirement_option(&json!({
            "id": format!("{owner_id}:requirement-option:category-credit-{id_suffix}"),
            "sourceText": text,
            "sourceHeading": text,
            "sourceCategory": "credit-bucket",
            "descriptor": descriptor,
            "creditRange": credit_range_value,
        }))?;
        let key = format!(
            "{}:{}:{}",
            option.pointer("/categoryOption/category").map(js_string_value).unwrap_or_default(),
            option.pointer("/categoryOption/creditMin").map(js_string_value).unwrap_or_default(),
            option.pointer("/categoryOption/creditMax").map(js_string_value).unwrap_or_default(),
        );
        seen.insert(key).then_some(option)
    })
    .collect()
}

pub fn build_generic_credit_bucket_requirement_groups(
    owner: &Value,
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let mut seen_group_ids = HashSet::new();
    let mut groups = Vec::new();
    for (index, raw_line) in snapshot_lines.iter().enumerate() {
        let normalized_line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(raw_line),
        );
        if !has_credit_bucket_cue(&normalized_line, course_parser)
            || should_skip_credit_bucket_line(snapshot_lines, index, &normalized_line, course_parser)
        {
            continue;
        }
        if !is_approved_list_placeholder_without_immediate_course_list(
            snapshot_lines,
            index,
            &normalized_line,
            course_parser,
        ) && super::requirement_recovery_scope::looks_like_sectioned_course_heading_line(
            &normalized_line,
            course_parser,
        ) && super::requirement_recovery_scope::has_upcoming_sectioned_course_rows(
            snapshot_lines,
            index,
            1,
            course_parser,
        ) {
            continue;
        }
        let source_line = build_credit_bucket_source_line(
            owner,
            snapshot_lines,
            index,
            course_parser,
        );
        for source_line in split_credit_bucket_source_lines(&source_line) {
            let course_codes =
                extract_course_codes_from_credit_bucket_text(&source_line, course_parser);
            if credit_bucket_explicit_choice_count_pattern().is_match(&source_line)
                && course_codes.len() >= 2
            {
                continue;
            }
            if has_parenthetical_sequence_choice_candidate(&source_line, course_parser)
                || looks_like_admission_decision_narrative_credit_bucket(&source_line)
            {
                continue;
            }
            let Some((minimum_credits, maximum_credits)) =
                parse_requirement_credit_range(&source_line)
            else {
                continue;
            };
            let credit_range = Some((minimum_credits, maximum_credits));
            let program_descriptor = get_program_specific_credit_bucket_descriptor(
                owner,
                &source_line,
                course_parser,
            );
            let approved_list_key = get_approved_list_key_from_credit_bucket_text(
                owner,
                &source_line,
                program_descriptor.as_ref(),
            );
            let generic_descriptors = if program_descriptor.is_some() {
                Vec::new()
            } else {
                super::requirement_category_options::get_requirement_category_option_descriptors(
                    &get_positive_credit_bucket_category_text(&source_line),
                )
            };
            let has_approved_list = approved_list_key.is_some()
                || program_approved_word_pattern().is_match(&source_line);
            let has_course_list_bucket = credit_bucket_course_list_bucket_pattern()
                .is_match(&source_line)
                && course_codes.len() >= 2;
            let has_subject_elective_bucket =
                credit_bucket_subject_elective_pattern().is_match(&source_line);
            let has_language_level_bucket =
                credit_bucket_language_level_pattern().is_match(&source_line);
            if program_descriptor.is_none()
                && generic_descriptors.is_empty()
                && !has_approved_list
                && !has_course_list_bucket
                && !has_subject_elective_bucket
                && !has_language_level_bucket
            {
                continue;
            }
            let source_line_course_code_count = course_codes.len();
            if source_line_course_code_count > 0
                && program_descriptor.is_none()
                && !starts_with_category_credit_cue(&source_line)
                && !has_approved_list
                && !has_course_list_bucket
            {
                continue;
            }
            let options = if has_course_list_bucket {
                build_course_requirement_options_from_credit_bucket(
                    owner,
                    &source_line,
                    credit_range,
                    course_parser,
                )
            } else {
                build_category_requirement_options_from_credit_bucket(
                    owner,
                    &source_line,
                    credit_range,
                    program_descriptor.as_ref(),
                )
            };
            let label_descriptor = program_descriptor
                .as_ref()
                .or_else(|| generic_descriptors.first());
            let credit_text = maximum_credits
                .filter(|maximum| *maximum != minimum_credits)
                .map(|maximum| {
                    format!(
                        "{}-{}",
                        js_number_string(minimum_credits),
                        js_number_string(maximum)
                    )
                })
                .unwrap_or_else(|| js_number_string(minimum_credits));
            let label = label_descriptor
                .and_then(|descriptor| descriptor.get("preferredLabel"))
                .filter(|value| !value.is_null())
                .map(js_string_value)
                .unwrap_or_else(|| {
                    if let Some(descriptor) = label_descriptor {
                        let long_label = descriptor
                            .get("longLabel")
                            .map(js_string_value)
                            .unwrap_or_else(|| "undefined".to_string());
                        format!("{credit_text} credits of {long_label}")
                    } else {
                        credit_bucket_leading_non_alphanumeric_pattern()
                            .replace(&source_line, "")
                            .to_string()
                    }
                });
            let group_id = format!(
                "{owner_id}:requirement-group:credit-bucket-{}",
                slugify_requirement_identifier(&format!(
                    "{label}-{}-{}",
                    js_number_string(minimum_credits),
                    maximum_credits
                        .map(js_number_string)
                        .unwrap_or_else(|| "open".to_string())
                ))
            );
            if seen_group_ids.contains(&group_id) {
                continue;
            }
            let category = if program_descriptor.is_some() {
                "program-approved-credit-bucket".to_string()
            } else {
                generic_descriptors
                    .first()
                    .and_then(|descriptor| descriptor.get("category"))
                    .map(js_string_value)
                    .map(|category| category.to_lowercase())
                    .unwrap_or_else(|| "approved-credit-bucket".to_string())
            };
            let notes = if program_descriptor.is_some() {
                vec!["Program-specific approved bucket; do not satisfy with generic Area of Inquiry category matching unless an approved filter also backs the course.".to_string()]
            } else {
                vec!["Credit bucket placeholder extracted from source text.".to_string()]
            };
            let group = build_parsed_requirement_group(&json!({
                "id": group_id,
                "label": label,
                "category": category,
                "subcategory": approved_list_key.clone(),
                "requirementType": "choose_credits",
                "minCredits": minimum_credits,
                "maxCredits": maximum_credits,
                "sourceHeading": source_line,
                "sourceRowText": source_line,
                "detectedOptionCue": "credit bucket",
                "approvedListKey": approved_list_key.clone(),
                "canCreatePlaceholder": true,
                "programSpecific": program_descriptor.is_some(),
                "notes": notes,
                "options": options,
            }));
            seen_group_ids.insert(group_id);
            groups.push(group);
        }
    }
    groups
}

pub fn filter_generic_bse_supplemental_groups_for_sbse(
    owner: &Value,
    groups: &[Value],
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    if owner.get("planId").and_then(Value::as_str)
        != Some("uw-seattle-sustainable-bioresource-systems-engineering")
    {
        return groups.to_vec();
    }
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    groups
        .iter()
        .filter(|group| {
            let line_index = find_snapshot_line_index_for_requirement_group(
                group,
                snapshot_lines,
                course_parser,
            );
            let marker_line = find_nearest_supplemental_source_marker(snapshot_lines, line_index);
            if !sbse_supplemental_source_marker_pattern()
                .is_match(marker_line.as_deref().unwrap_or_default())
            {
                return true;
            }
            group
                .get("id")
                .map(js_string_value)
                .is_some_and(|id| id.starts_with(&format!("{owner_id}:requirement-group:sbse-")))
        })
        .cloned()
        .collect()
}

pub fn filter_requirement_groups_by_supplemental_pathway_scope(
    owner: &Value,
    groups: &[Value],
    snapshot_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    groups
        .iter()
        .filter(|group| {
            let line_index = find_snapshot_line_index_for_requirement_group(
                group,
                snapshot_lines,
                course_parser,
            );
            let marker_line = find_nearest_supplemental_source_marker(snapshot_lines, line_index);
            if marker_line.is_none()
                && is_requirement_group_under_support_course_list(
                    snapshot_lines,
                    line_index,
                    course_parser,
                )
            {
                return false;
            }
            marker_line.map_or(true, |marker| {
                !is_pathway_supplemental_source_marker(&marker)
                    || (owner
                        .get("pathwayId")
                        .is_some_and(js_truthy)
                        && super::requirement_recovery_scope::line_matches_selected_pathway_identity(
                            owner,
                            &marker,
                            primary_major_titles,
                        ))
            })
        })
        .cloned()
        .collect()
}

fn is_pathway_supplemental_source_marker(line: &str) -> bool {
    supplemental_official_source_marker_pattern().is_match(line)
        && pathway_label_cue_pattern().is_match(line)
}

pub fn get_parallel_choose_one_heading_label(
    line: &str,
    course_parser: &CourseCodeParser,
) -> String {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if normalized_line.is_empty() || !course_parser.extract_from_line(&normalized_line).is_empty() {
        return String::new();
    }
    parallel_choose_one_heading_pattern()
        .captures(&normalized_line)
        .and_then(|captures| captures.get(1))
        .map(|capture| {
            parallel_choose_one_heading_terminal_punctuation_pattern()
                .replace_all(&normalize_whitespace(capture.as_str()), "")
                .to_string()
        })
        .map(|label| normalize_whitespace(&label))
        .unwrap_or_default()
}

pub fn get_parallel_choose_one_heading_subjects(label: &str) -> Vec<String> {
    let normalized_label = super::requirement_recovery_scope::normalize_matcher_text(label);
    if parallel_heading_biochemistry_pattern().is_match(&normalized_label) {
        return vec!["BIOC".to_string()];
    }
    if parallel_heading_mathematics_pattern().is_match(&normalized_label) {
        return vec!["MATH".to_string(), "Q SCI".to_string(), "QSCI".to_string()];
    }
    if parallel_heading_statistics_pattern().is_match(&normalized_label) {
        return vec!["STAT".to_string(), "BIOST".to_string(), "Q SCI".to_string(), "QSCI".to_string()];
    }
    if parallel_heading_physics_pattern().is_match(&normalized_label) {
        return vec!["PHYS".to_string()];
    }
    if parallel_heading_chemistry_pattern().is_match(&normalized_label) {
        return vec!["CHEM".to_string()];
    }
    if parallel_heading_biology_pattern().is_match(&normalized_label) {
        return vec!["BIOL".to_string()];
    }
    Vec::new()
}

pub fn line_starts_with_numbered_parallel_choice(line: &str) -> bool {
    numbered_inline_option_row_pattern().is_match(&normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    ))
}

pub fn heading_subjects_match_course_codes(subjects: &[String], course_codes: &[String]) -> bool {
    if subjects.is_empty() {
        return true;
    }
    let subject_set = subjects
        .iter()
        .map(|subject| normalize_whitespace(subject))
        .collect::<HashSet<_>>();
    course_codes.iter().any(|course_code| {
        let subject = normalize_course_code(course_code)
            .rsplit_once(' ')
            .map(|(subject, _)| normalize_whitespace(subject))
            .unwrap_or_default();
        subject_set.contains(&subject) || subject_set.contains(&subject.replace(' ', ""))
    })
}

pub fn build_parallel_choose_one_requirement_groups(
    owner: &Value,
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let mut groups = Vec::new();
    let mut seen_group_ids = HashSet::new();
    let mut index = 0usize;
    while index + 1 < snapshot_lines.len() {
        let first_heading = get_parallel_choose_one_heading_label(&snapshot_lines[index], course_parser);
        let second_heading = get_parallel_choose_one_heading_label(&snapshot_lines[index + 1], course_parser);
        if first_heading.is_empty() || second_heading.is_empty() {
            index += 1;
            continue;
        }
        let mut headings = vec![
            (first_heading.clone(), get_parallel_choose_one_heading_subjects(&first_heading), Vec::<(String, Vec<String>)>::new()),
            (second_heading.clone(), get_parallel_choose_one_heading_subjects(&second_heading), Vec::<(String, Vec<String>)>::new()),
        ];
        let scan_limit = snapshot_lines.len().min(index.saturating_add(36));
        for row_index in index.saturating_add(2)..scan_limit {
            if row_index > index.saturating_add(3)
                && !get_parallel_choose_one_heading_label(&snapshot_lines[row_index], course_parser).is_empty()
                && snapshot_lines
                    .get(row_index.saturating_add(1))
                    .is_some_and(|line| !get_parallel_choose_one_heading_label(line, course_parser).is_empty())
            {
                break;
            }
            let row = normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(
                    &snapshot_lines[row_index],
                ),
            );
            if row.is_empty() {
                continue;
            }
            let row_course_codes = course_parser.extract_from_line(&row);
            if headings.iter().any(|(_, _, rows)| !rows.is_empty())
                && !line_starts_with_numbered_parallel_choice(&row)
                && row_course_codes.is_empty()
                && parallel_choose_one_section_boundary_pattern().is_match(&row)
            {
                break;
            }
            if !line_starts_with_numbered_parallel_choice(&row) {
                continue;
            }
            let course_codes = course_parser.extract_from_requirement_line(&row);
            if course_codes.is_empty() {
                continue;
            }
            if let Some((_, _, rows)) = headings
                .iter_mut()
                .find(|(_, subjects, _)| heading_subjects_match_course_codes(subjects, &course_codes))
            {
                rows.push((row, course_codes));
            }
        }
        for (heading_label, _, rows) in &headings {
            if rows.len() < 2 {
                continue;
            }
            let group_id = format!(
                "{owner_id}:requirement-group:parallel-choose-one-{}",
                slugify_requirement_identifier(heading_label)
            );
            if seen_group_ids.contains(&group_id) {
                continue;
            }
            let source_heading = format!("{heading_label} - Choose One Option");
            let category = build_sectioned_course_category(heading_label);
            let options = rows
                .iter()
                .map(|(row, course_codes)| {
                    json!({
                        "id": format!(
                            "{owner_id}:requirement-option:{}",
                            slugify_requirement_identifier(&format!(
                                "{heading_label}-{}",
                                course_codes.join("-")
                            ))
                        ),
                        "uwCourses": course_codes,
                        "displayCourseCodes": course_codes,
                        "label": parallel_numbered_row_marker_pattern()
                            .replace(row, "")
                            .to_string(),
                        "sourceHeading": source_heading,
                        "sourceCategory": category,
                    })
                })
                .collect::<Vec<_>>();
            let group = build_parsed_requirement_group(&json!({
                "id": group_id,
                "label": source_heading,
                "category": category,
                "subcategory": category,
                "requirementType": "choose_one",
                "minCourses": 1,
                "maxCourses": 1,
                "selectionCount": 1,
                "requiredCount": 1,
                "sourceHeading": source_heading,
                "sourceRowText": source_heading,
                "detectedOptionCue": "parallel choose-one columns",
                "notes": ["Parsed from adjacent source columns headed Choose One Option."],
                "options": options,
            }));
            if array(group.get("options")).is_empty() {
                continue;
            }
            seen_group_ids.insert(group_id);
            groups.push(group);
        }
        index += 2;
    }
    groups
}

pub fn build_category_credit_bucket_option(group_id: &str, input: &Value) -> Value {
    let source_category_code = input
        .get("sourceCategoryCode")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let title = input
        .get("title")
        .cloned()
        .unwrap_or(Value::Null);
    let credits = input.get("credits").cloned().unwrap_or(Value::Null);
    let source_text = input
        .get("sourceText")
        .cloned()
        .unwrap_or(Value::Null);
    let approved_list_key = input
        .get("approvedListKey")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let program_specific = input
        .get("programSpecific")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| Value::Bool(false));
    let category = input.get("category").cloned().unwrap_or(Value::Null);
    json!({
        "id": format!(
            "{group_id}:option:{}",
            slugify_requirement_identifier(&source_category_code)
        ),
        "optionKind": "category-option",
        "label": title,
        "categoryOption": {
            "category": category,
            "sourceCategoryCode": source_category_code,
            "title": title,
            "credits": credits,
            "creditMin": credits,
            "creditMax": credits,
            "sourceText": source_text,
            "approvedListKey": approved_list_key,
            "programSpecific": program_specific,
        },
        "sourceHeading": source_text,
        "sourceCategory": category,
    })
}

pub fn build_sbse_comparison_table_requirement_groups(
    owner: &Value,
    snapshot_lines: &[String],
) -> Vec<Value> {
    if owner.get("planId").and_then(Value::as_str)
        != Some("uw-seattle-sustainable-bioresource-systems-engineering")
    {
        return Vec::new();
    }
    let snapshot_text = super::requirement_recovery_scope::normalize_matcher_text(
        &snapshot_lines.join(" "),
    );
    if !sbse_comparison_table_snapshot_pattern().is_match(&snapshot_text) {
        return Vec::new();
    }
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let plan_id = owner
        .get("planId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let mut groups = Vec::new();
    if let Some(line) = snapshot_lines.iter().find(|line| {
        sbse_engineering_elective_line_pattern().is_match(
            &normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            ),
        )
    }) {
        let source_heading = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        );
        let label = "Engineering Elective Credit (12 credits)";
        let group_id = format!("{owner_id}:requirement-group:sbse-engineering-elective-credit");
        let approved_list_key = format!("{plan_id}-engineering-elective-credit");
        let option = build_category_credit_bucket_option(
            &group_id,
            &json!({
                "category": "ENGINEERING_ELECTIVE_CREDIT",
                "sourceCategoryCode": "Engineering Elective Credit",
                "title": "Engineering Elective Credit",
                "credits": 12,
                "sourceText": source_heading,
                "approvedListKey": approved_list_key,
                "programSpecific": true,
            }),
        );
        groups.push(build_parsed_requirement_group(&json!({
            "id": group_id,
            "label": label,
            "category": "program-approved-credit-bucket",
            "subcategory": approved_list_key,
            "requirementType": "choose_credits",
            "minCredits": 12,
            "maxCredits": 12,
            "creditText": "12",
            "sourceHeading": source_heading,
            "sourceRowText": source_heading,
            "detectedOptionCue": "SBSE comparison-table engineering elective credit bucket",
            "approvedListKey": approved_list_key,
            "canCreatePlaceholder": true,
            "programSpecific": true,
            "notes": ["Parsed from the official BSE/SBSE comparison table."],
            "options": [option],
        })));
    }
    if let Some(line) = snapshot_lines.iter().find(|line| {
        sbse_diversity_line_pattern().is_match(&normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        ))
    }) {
        let source_heading = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        );
        let label = "5 credits of Diversity";
        let group_id = format!("{owner_id}:requirement-group:sbse-diversity");
        let option = build_category_credit_bucket_option(
            &group_id,
            &json!({
                "category": "diversity",
                "sourceCategoryCode": "DIV",
                "title": "Diversity",
                "credits": 5,
                "sourceText": source_heading,
                "programSpecific": true,
            }),
        );
        groups.push(build_parsed_requirement_group(&json!({
            "id": group_id,
            "label": label,
            "category": "diversity",
            "subcategory": "diversity",
            "requirementType": "choose_credits",
            "minCredits": 5,
            "maxCredits": 5,
            "creditText": "5",
            "sourceHeading": source_heading,
            "sourceRowText": source_heading,
            "detectedOptionCue": "SBSE comparison-table category credit bucket",
            "notes": ["Parsed from the official BSE/SBSE comparison table."],
            "options": [option],
        })));
    }
    if let Some(line) = snapshot_lines.iter().find(|line| {
        sbse_social_sciences_line_pattern().is_match(&normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        ))
    }) {
        let source_heading = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        );
        let label = "10 credits of Social Sciences";
        let group_id = format!("{owner_id}:requirement-group:sbse-social-sciences");
        let option = build_category_credit_bucket_option(
            &group_id,
            &json!({
                "category": "social_sciences",
                "sourceCategoryCode": "SSc",
                "title": "Social Sciences",
                "credits": 10,
                "sourceText": source_heading,
                "programSpecific": true,
            }),
        );
        groups.push(build_parsed_requirement_group(&json!({
            "id": group_id,
            "label": label,
            "category": "social_sciences",
            "subcategory": "social_sciences",
            "requirementType": "choose_credits",
            "minCredits": 10,
            "maxCredits": 10,
            "creditText": "10",
            "sourceHeading": source_heading,
            "sourceRowText": source_heading,
            "detectedOptionCue": "SBSE comparison-table category credit bucket",
            "notes": ["Parsed from the official BSE/SBSE comparison table."],
            "options": [option],
        })));
    }
    if let Some(line) = snapshot_lines.iter().find(|line| {
        sbse_statistics_line_pattern().is_match(&normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        ))
    }) {
        let source_heading = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        );
        let group_id = format!("{owner_id}:requirement-group:sbse-statistics");
        let options = ["QSCI 381", "INDE 315", "STAT 390"]
            .into_iter()
            .map(|course_code| json!({
                "id": format!(
                    "{owner_id}:requirement-option:{}",
                    slugify_requirement_identifier(course_code)
                ),
                "uwCourses": [course_code],
                "displayCourseCodes": [course_code],
                "label": course_code,
                "sourceHeading": source_heading,
                "sourceCategory": "statistics",
            }))
            .collect::<Vec<_>>();
        groups.push(build_parsed_requirement_group(&json!({
            "id": group_id,
            "label": "QSCI 381, INDE 315, or STAT 390",
            "category": "statistics",
            "subcategory": "statistics",
            "requirementType": "choose_one",
            "minCourses": 1,
            "maxCourses": 1,
            "selectionCount": 1,
            "requiredCount": 1,
            "sourceHeading": source_heading,
            "sourceRowText": source_heading,
            "detectedOptionCue": "SBSE comparison-table course choice",
            "notes": ["Parsed from the official BSE/SBSE comparison table."],
            "options": options,
        })));
    }
    if let Some(line) = snapshot_lines.iter().find(|line| {
        sbse_physics_line_pattern().is_match(&normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        ))
    }) {
        let source_heading = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        );
        let group_id = format!("{owner_id}:requirement-group:sbse-physics-sequence");
        let sequence_paths = json!([
            {
                "id": format!("{owner_id}:sequence-path:phys-121-122"),
                "label": "PHYS 121 and PHYS 122",
                "uwCourses": ["PHYS 121", "PHYS 122"],
                "displayCourseCodes": ["PHYS 121", "PHYS 122"],
                "conditionalLabCourses": [],
                "notes": [],
                "sourceText": source_heading,
            },
            {
                "id": format!("{owner_id}:sequence-path:phys-141-142"),
                "label": "PHYS 141 and PHYS 142",
                "uwCourses": ["PHYS 141", "PHYS 142"],
                "displayCourseCodes": ["PHYS 141", "PHYS 142"],
                "conditionalLabCourses": [],
                "notes": [],
                "sourceText": source_heading,
            },
        ]);
        let sequence_options = array(Some(&sequence_paths))
            .iter()
            .map(|path| json!({
                "id": format!(
                    "{owner_id}:requirement-option:{}",
                    slugify_requirement_identifier(&js_string_nullish(path.get("id")))
                ),
                "sequencePathId": path.get("id"),
                "pathLabel": path.get("label"),
                "uwCourses": path.get("uwCourses"),
                "displayCourseCodes": path.get("displayCourseCodes"),
                "conditionalLabCourses": path.get("conditionalLabCourses"),
                "notes": path.get("notes"),
                "label": path.get("label"),
                "sourceHeading": source_heading,
                "sourceCategory": "physics",
            }))
            .collect::<Vec<_>>();
        groups.push(build_parsed_requirement_group(&json!({
            "id": group_id,
            "label": "PHYS 121 and PHYS 122 or PHYS 141 and PHYS 142",
            "category": "physics",
            "subcategory": "physics",
            "requirementType": "sequence_choice",
            "sourceHeading": source_heading,
            "sourceRowText": source_heading,
            "detectedOptionCue": "SBSE comparison-table sequence choice",
            "notes": ["Parsed from the official BSE/SBSE comparison table."],
            "sequencePaths": sequence_paths,
            "options": sequence_options,
        })));
    }
    groups
        .into_iter()
        .filter(|group| {
            !array(group.get("options")).is_empty()
                || !array(group.get("sequencePaths")).is_empty()
        })
        .collect()
}

pub fn find_nearest_supplemental_source_marker(
    snapshot_lines: &[String],
    line_index: Option<usize>,
) -> Option<String> {
    let mut index = line_index?;
    loop {
        let line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(
                snapshot_lines.get(index).map(String::as_str).unwrap_or_default(),
            ),
        );
        if supplemental_official_source_marker_pattern().is_match(&line) {
            return Some(line);
        }
        if index == 0 {
            break;
        }
        index -= 1;
    }
    None
}

pub fn is_requirement_group_under_support_course_list(
    snapshot_lines: &[String],
    line_index: Option<usize>,
    course_parser: &CourseCodeParser,
) -> bool {
    let Some(line_index) = line_index else {
        return false;
    };
    let start_index = line_index.saturating_sub(28);
    for index in (start_index..=line_index).rev() {
        let line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(
                snapshot_lines.get(index).map(String::as_str).unwrap_or_default(),
            ),
        );
        if line.is_empty() {
            continue;
        }
        if supplemental_official_source_marker_pattern().is_match(&line) {
            return false;
        }
        if index == line_index && support_requirement_target_line_pattern().is_match(&line) {
            return false;
        }
        if index == line_index
            && (get_sequence_choice_cue(&line).is_some()
                || sequence_choice_context_pattern().is_match(&line))
            && course_parser.extract_from_line(&line).len() >= 4
            && (parse_requirement_credit_amount(&line).is_some()
                || parenthetical_number_pattern().is_match(&line))
        {
            return false;
        }
        if requirement_section_heading_pattern().is_match(&line)
            && !electives_list_heading_pattern().is_match(&line)
        {
            return false;
        }
        if electives_list_heading_pattern().is_match(&line)
            || approved_courses_list_heading_pattern().is_match(&line)
        {
            return true;
        }
    }
    false
}

pub fn find_snapshot_line_index_for_requirement_group(
    group: &Value,
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    let source_value = group
        .get("sourceRowText")
        .filter(|value| !value.is_null())
        .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()))
        .or_else(|| group.get("label").filter(|value| !value.is_null()));
    let source_text = normalize_whitespace(&js_string_nullish(source_value));
    let normalized_source_text =
        super::requirement_recovery_scope::normalize_matcher_text(&source_text);
    if !normalized_source_text.is_empty() {
        if let Some((index, _)) = snapshot_lines.iter().enumerate().find(|(_, line)| {
            let normalized_line = super::requirement_recovery_scope::normalize_matcher_text(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            );
            normalized_line == normalized_source_text
                || (normalized_line.len() >= 40
                    && normalized_source_text.starts_with(&normalized_line))
                || (normalized_source_text.len() >= 40
                    && normalized_line.starts_with(&normalized_source_text))
        }) {
            return Some(index);
        }
    }
    let option_course_codes = unique_in_order(
        array(group.get("options"))
            .iter()
            .flat_map(|option| {
                ["uwCourses", "equivalentUwCourseCodes", "displayCourseCodes"]
                    .into_iter()
                    .flat_map(|key| array(option.get(key)).iter().cloned())
            })
            .map(|course_code| normalize_course_code(&js_string_value(&course_code)))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    if option_course_codes.len() < 2 {
        return None;
    }
    let course_code_search_keys = |course_code: &str| {
        let normalized = normalize_course_code(course_code);
        let compact = normalized.split_whitespace().collect::<String>();
        unique_in_order(
            [normalized, compact]
                .into_iter()
                .filter(|key| !key.is_empty())
                .collect(),
        )
    };
    let first_option_course_code_keys = course_code_search_keys(&option_course_codes[0]);
    let remaining_option_course_code_keys = option_course_codes
        .iter()
        .skip(1)
        .map(|course_code| course_code_search_keys(course_code))
        .collect::<Vec<_>>();
    let includes_any_key = |course_codes: &[String], search_keys: &[String]| {
        let available_keys = course_codes
            .iter()
            .flat_map(|course_code| course_code_search_keys(course_code))
            .collect::<HashSet<_>>();
        search_keys.iter().any(|key| available_keys.contains(key))
    };
    let source_looks_like_split_or_choice = {
        let source_heading = js_string_nullish(group.get("sourceHeading"));
        let source_row = js_string_nullish(group.get("sourceRowText"));
        let option_cue = js_string_nullish(group.get("detectedOptionCue"));
        split_choice_source_pattern().is_match(&format!("{source_heading} {source_row} {option_cue}"))
    };
    if source_looks_like_split_or_choice {
        if let Some((index, _)) = snapshot_lines.iter().enumerate().find(|(index, line)| {
            let normalized_line = normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            );
            if !split_choice_trailing_or_pattern().is_match(&normalized_line) {
                return false;
            }
            let current_codes = course_parser
                .extract_from_line(&normalized_line)
                .into_iter()
                .map(|course_code| normalize_course_code(&course_code))
                .collect::<Vec<_>>();
            if !includes_any_key(&current_codes, &first_option_course_code_keys) {
                return false;
            }
            let nearby_codes = snapshot_lines
                .iter()
                .skip(index.saturating_add(1))
                .take(3)
                .flat_map(|nearby_line| {
                    course_parser.extract_from_line(
                        &super::requirement_choice_lists::strip_choice_list_line(nearby_line),
                    )
                })
                .map(|course_code| normalize_course_code(&course_code))
                .collect::<Vec<_>>();
            remaining_option_course_code_keys
                .iter()
                .any(|search_keys| includes_any_key(&nearby_codes, search_keys))
        }) {
            return Some(index);
        }
    }
    snapshot_lines.iter().enumerate().find_map(|(index, line)| {
        let current_codes = course_parser
            .extract_from_line(&super::requirement_choice_lists::strip_choice_list_line(line))
            .into_iter()
            .map(|course_code| normalize_course_code(&course_code))
            .collect::<Vec<_>>();
        if !includes_any_key(&current_codes, &first_option_course_code_keys) {
            return None;
        }
        let nearby_codes = snapshot_lines
            .iter()
            .skip(index)
            .take(4)
            .flat_map(|nearby_line| {
                course_parser.extract_from_line(
                    &super::requirement_choice_lists::strip_choice_list_line(nearby_line),
                )
            })
            .map(|course_code| normalize_course_code(&course_code))
            .collect::<Vec<_>>();
        remaining_option_course_code_keys
            .iter()
            .any(|search_keys| includes_any_key(&nearby_codes, search_keys))
            .then_some(index)
    })
}

pub fn get_requirement_option_course_code_set(option: &Value) -> HashSet<String> {
    array(option.get("uwCourses"))
        .iter()
        .chain(array(option.get("equivalentUwCourseCodes")).iter())
        .map(|course_code| normalize_course_code(&js_string_value(course_code)))
        .filter(|course_code| !course_code.is_empty())
        .collect()
}

pub fn suppress_choice_groups_covered_by_credit_buckets(groups: &[Value]) -> Vec<Value> {
    let credit_bucket_groups = groups
        .iter()
        .filter(|group| group.get("requirementType").and_then(Value::as_str) == Some("choose_credits"))
        .collect::<Vec<_>>();
    let credit_bucket_groups_by_source = credit_bucket_groups
        .iter()
        .filter_map(|group| {
            let source_key = requirement_group_source_matcher_key(group);
            let code_set = array(group.get("options"))
                .iter()
                .flat_map(get_requirement_option_course_code_set)
                .collect::<HashSet<_>>();
            (!source_key.is_empty() && !code_set.is_empty()).then_some((source_key, code_set))
        })
        .collect::<Vec<_>>();
    let credit_bucket_option_code_sets = credit_bucket_groups
        .iter()
        .flat_map(|group| array(group.get("options")))
        .map(get_requirement_option_course_code_set)
        .filter(|code_set| code_set.len() > 1)
        .collect::<Vec<_>>();
    let credit_bucket_group_code_sets = credit_bucket_groups
        .iter()
        .map(|group| {
            array(group.get("options"))
                .iter()
                .flat_map(get_requirement_option_course_code_set)
                .collect::<HashSet<_>>()
        })
        .filter(|code_set| code_set.len() > 1)
        .collect::<Vec<_>>();
    let credit_bucket_category_sets_by_source = credit_bucket_groups
        .iter()
        .filter_map(|group| {
            let source_key = requirement_group_source_matcher_key(group);
            let category_set = array(group.get("options"))
                .iter()
                .filter_map(requirement_option_category_matcher_key)
                .collect::<HashSet<_>>();
            (!source_key.is_empty() && !category_set.is_empty())
                .then_some((source_key, category_set))
        })
        .collect::<Vec<_>>();
    if credit_bucket_option_code_sets.is_empty()
        && credit_bucket_group_code_sets.is_empty()
        && credit_bucket_category_sets_by_source.is_empty()
    {
        return groups.to_vec();
    }
    groups
        .iter()
        .filter(|group| {
            let requirement_type = group
                .get("requirementType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if requirement_type == "all_required"
                && group
                    .get("detectedOptionCue")
                    .map(js_string_value)
                    .is_some_and(|cue| embedded_alternative_required_list_pattern().is_match(&cue))
            {
                let source_key = requirement_group_source_matcher_key(group);
                let code_set = array(group.get("options"))
                    .iter()
                    .flat_map(get_requirement_option_course_code_set)
                    .collect::<HashSet<_>>();
                if !source_key.is_empty()
                    && !code_set.is_empty()
                    && credit_bucket_groups_by_source.iter().any(|(bucket_source, bucket_codes)| {
                        bucket_source == &source_key
                            && code_set.iter().all(|course_code| bucket_codes.contains(course_code))
                    })
                {
                    return false;
                }
            }
            if !matches!(requirement_type, "choose_one" | "choose_n") {
                return true;
            }
            let source_key = requirement_group_source_matcher_key(group);
            let category_set = array(group.get("options"))
                .iter()
                .filter_map(requirement_option_category_matcher_key)
                .collect::<HashSet<_>>();
            if !source_key.is_empty()
                && !category_set.is_empty()
                && credit_bucket_category_sets_by_source.iter().any(|(bucket_source, bucket_categories)| {
                    bucket_source == &source_key
                        && category_set.iter().all(|category| bucket_categories.contains(category))
                })
            {
                return false;
            }
            let code_set = array(group.get("options"))
                .iter()
                .flat_map(get_requirement_option_course_code_set)
                .collect::<HashSet<_>>();
            if code_set.is_empty() {
                return true;
            }
            !credit_bucket_option_code_sets.iter().any(|bucket_codes| {
                bucket_codes.len() == code_set.len()
                    && code_set.iter().all(|course_code| bucket_codes.contains(course_code))
            }) && !credit_bucket_group_code_sets.iter().any(|bucket_codes| {
                bucket_codes.len() > code_set.len()
                    && code_set.iter().all(|course_code| bucket_codes.contains(course_code))
            })
        })
        .cloned()
        .collect()
}

fn requirement_group_source_matcher_key(group: &Value) -> String {
    let source_text = group
        .get("sourceRowText")
        .filter(|value| !value.is_null())
        .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()))
        .or_else(|| group.get("label").filter(|value| !value.is_null()));
    super::requirement_recovery_scope::normalize_matcher_text(&js_string_nullish(source_text))
}

fn requirement_option_category_matcher_key(option: &Value) -> Option<String> {
    let category_option = option.get("categoryOption")?;
    let source = category_option
        .get("sourceCategoryCode")
        .filter(|value| !value.is_null())
        .or_else(|| category_option.get("category").filter(|value| !value.is_null()))?;
    let category = normalize_whitespace(&js_string_value(source)).to_uppercase();
    (!category.is_empty()).then_some(category)
}

pub fn get_requirement_group_option_course_set_key(option: &Value) -> String {
    let course_codes = unique_sorted(
        array(option.get("uwCourses"))
            .iter()
            .chain(array(option.get("displayCourseCodes")).iter())
            .map(|course_code| normalize_course_code(&js_string_value(course_code)))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    course_codes.join("|")
}

pub fn get_all_required_requirement_group_course_set_key(group: &Value) -> String {
    let course_codes = unique_sorted(
        array(group.get("options"))
            .iter()
            .flat_map(|option| {
                let codes = option
                    .get("uwCourses")
                    .filter(|value| !value.is_null())
                    .map(|value| array(Some(value)))
                    .unwrap_or_else(|| array(option.get("displayCourseCodes")));
                codes.iter().map(js_string_value).collect::<Vec<_>>()
            })
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    course_codes.join("|")
}

pub fn suppress_inline_required_groups_covered_by_choice_options(groups: &[Value]) -> Vec<Value> {
    let mut choice_option_course_set_keys = HashSet::new();
    for group in groups {
        if !matches!(
            group.get("requirementType").and_then(Value::as_str),
            Some("choose_one" | "sequence_choice")
        ) {
            continue;
        }
        for option in array(group.get("options")) {
            let key = get_requirement_group_option_course_set_key(option);
            if key.split('|').count() >= 2 {
                choice_option_course_set_keys.insert(key);
            }
        }
    }
    groups
        .iter()
        .filter(|group| {
            if group.get("requirementType").and_then(Value::as_str) != Some("all_required")
                || group.get("category").and_then(Value::as_str)
                    != Some("inline-numbered-requirement")
            {
                return true;
            }
            let key = get_all_required_requirement_group_course_set_key(group);
            key.is_empty() || !choice_option_course_set_keys.contains(&key)
        })
        .cloned()
        .collect()
}

pub fn suppress_repeated_required_alternative_groups_covered_by_sequence_choices(
    groups: &[Value],
) -> Vec<Value> {
    let sequence_path_source_texts = groups
        .iter()
        .filter(|group| group.get("requirementType").and_then(Value::as_str) == Some("sequence_choice"))
        .flat_map(|group| array(group.get("sequencePaths")))
        .map(|path| {
            super::requirement_recovery_scope::normalize_matcher_text(
                &js_string_nullish(path.get("sourceText")),
            )
        })
        .filter(|source| !source.is_empty())
        .collect::<HashSet<_>>();
    if sequence_path_source_texts.is_empty() {
        return groups.to_vec();
    }
    groups
        .iter()
        .filter(|group| {
            if group.get("subcategory").and_then(Value::as_str)
                != Some("required-alternative-slot")
            {
                return true;
            }
            let source_text = group
                .get("sourceRowText")
                .filter(|value| !value.is_null())
                .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()));
            let source = super::requirement_recovery_scope::normalize_matcher_text(
                &js_string_nullish(source_text),
            );
            source.is_empty() || !sequence_path_source_texts.contains(&source)
        })
        .cloned()
        .collect()
}

pub fn get_choice_requirement_group_course_set_key(group: &Value) -> String {
    let course_codes = unique_sorted(
        array(group.get("options"))
            .iter()
            .flat_map(|option| {
                ["uwCourses", "equivalentUwCourseCodes", "displayCourseCodes"]
                    .into_iter()
                    .flat_map(|key| array(option.get(key)).iter().map(js_string_value).collect::<Vec<_>>())
            })
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    course_codes.join("|")
}

pub fn get_choice_requirement_group_course_set(group: &Value) -> HashSet<String> {
    get_choice_requirement_group_course_set_key(group)
        .split('|')
        .filter(|course_code| !course_code.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn is_source_choice_requirement_group(group: &Value) -> bool {
    group.get("category").and_then(Value::as_str) == Some("source-choice")
        && matches!(
            group.get("requirementType").and_then(Value::as_str),
            Some("choose_one" | "choose_n")
        )
}

pub fn get_source_choice_requirement_group_scope_key(group: &Value) -> String {
    if !is_source_choice_requirement_group(group) {
        return String::new();
    }
    let label_key = super::requirement_recovery_scope::normalize_matcher_text(
        &js_string_nullish(group.get("label")),
    );
    let source_value = group
        .get("sourceRowText")
        .filter(|value| !value.is_null())
        .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()));
    let source_key = super::requirement_recovery_scope::normalize_matcher_text(
        &js_string_nullish(source_value),
    );
    if label_key.is_empty() || source_key.is_empty() {
        return String::new();
    }
    let required_count = group
        .get("requiredCount")
        .filter(|value| !value.is_null())
        .or_else(|| group.get("selectionCount").filter(|value| !value.is_null()))
        .or_else(|| {
            let minimum = group.get("minCourses").filter(|value| !value.is_null())?;
            let maximum = group.get("maxCourses").filter(|value| !value.is_null())?;
            js_strict_equal(minimum, maximum).then_some(minimum)
        });
    format!(
        "{}:{}:{}:{}",
        group.get("requirementType").and_then(Value::as_str).unwrap_or_default(),
        js_string_nullish(required_count),
        label_key,
        source_key
    )
}

pub fn is_proper_course_set_subset(
    candidate_subset: &HashSet<String>,
    candidate_superset: &HashSet<String>,
) -> bool {
    !candidate_subset.is_empty()
        && candidate_superset.len() > candidate_subset.len()
        && candidate_subset
            .iter()
            .all(|course_code| candidate_superset.contains(course_code))
}

pub fn suppress_subset_source_choice_groups(groups: &[Value]) -> Vec<Value> {
    let source_choice_groups = groups
        .iter()
        .enumerate()
        .map(|(index, group)| {
            (
                index,
                get_source_choice_requirement_group_scope_key(group),
                get_choice_requirement_group_course_set(group),
            )
        })
        .filter(|(_, scope_key, course_set)| !scope_key.is_empty() && !course_set.is_empty())
        .collect::<Vec<_>>();
    if source_choice_groups.len() < 2 {
        return groups.to_vec();
    }
    let suppressed_indexes = source_choice_groups
        .iter()
        .filter_map(|(index, scope_key, course_set)| {
            source_choice_groups
                .iter()
                .any(|(candidate_index, candidate_scope_key, candidate_set)| {
                    candidate_index != index
                        && candidate_scope_key == scope_key
                        && is_proper_course_set_subset(course_set, candidate_set)
                })
                .then_some(*index)
        })
        .collect::<HashSet<_>>();
    groups
        .iter()
        .enumerate()
        .filter(|(index, _)| !suppressed_indexes.contains(index))
        .map(|(_, group)| group.clone())
        .collect()
}

pub fn looks_like_merged_pdf_choice_fragment_group(group: &Value) -> bool {
    let source_value = group
        .get("sourceRowText")
        .filter(|value| !value.is_null())
        .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()))
        .or_else(|| group.get("label").filter(|value| !value.is_null()));
    let source_text = normalize_whitespace(&js_string_nullish(source_value));
    if source_text.is_empty() {
        return false;
    }
    merged_pdf_labeled_choice_pattern().find_iter(&source_text).count() >= 2
        && merged_pdf_choice_fragment_cue_pattern().is_match(&source_text)
}

pub fn suppress_generic_choice_groups_covered_by_inline_choices(groups: &[Value]) -> Vec<Value> {
    let mut inline_choice_keys = HashSet::new();
    for group in groups {
        if !matches!(
            group.get("requirementType").and_then(Value::as_str),
            Some("choose_one" | "choose_n")
        ) {
            continue;
        }
        if !js_string_nullish(group.get("id")).contains(":requirement-group:inline-choice-") {
            continue;
        }
        let key = get_choice_requirement_group_course_set_key(group);
        if !key.is_empty() {
            inline_choice_keys.insert(key);
        }
    }
    if inline_choice_keys.is_empty() {
        return groups.to_vec();
    }
    groups
        .iter()
        .filter(|group| {
            if group.get("category").and_then(Value::as_str) != Some("source-choice")
                || !matches!(
                    group.get("requirementType").and_then(Value::as_str),
                    Some("choose_one" | "choose_n")
                )
            {
                return true;
            }
            let group_id = js_string_nullish(group.get("id"));
            if group_id.contains(":requirement-group:inline-choice-") {
                return true;
            }
            let key = get_choice_requirement_group_course_set_key(group);
            !(!key.is_empty() && inline_choice_keys.contains(&key))
                && !looks_like_merged_pdf_choice_fragment_group(group)
        })
        .cloned()
        .collect()
}

pub fn suppress_owner_specific_requirement_group_leaks(owner: &Value, groups: &[Value]) -> Vec<Value> {
    if owner.get("planId").and_then(Value::as_str) != Some("uw-seattle-public-health-global-health") {
        return groups.to_vec();
    }
    let has_natural_science_subject_choices = groups
        .iter()
        .any(|group| public_health_biology_choice_pattern().is_match(&js_string_nullish(group.get("label"))))
        && groups
            .iter()
            .any(|group| public_health_chemistry_choice_pattern().is_match(&js_string_nullish(group.get("label"))));
    groups
        .iter()
        .filter(|group| {
            let label = normalize_whitespace(&js_string_nullish(group.get("label")));
            let source_value = group
                .get("sourceRowText")
                .filter(|value| !value.is_null())
                .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()));
            let source_text = normalize_whitespace(&js_string_nullish(source_value));
            if public_health_leaked_label_pattern().is_match(&label) {
                return false;
            }
            if has_natural_science_subject_choices
                && group.get("requirementType").and_then(Value::as_str) == Some("choose_credits")
                && public_health_natural_science_pattern()
                    .is_match(&format!("{label} {source_text}"))
            {
                return false;
            }
            group.get("category").and_then(Value::as_str) != Some("source-choice")
                || !public_health_service_learning_pattern()
                    .is_match(&format!("{label} {source_text}"))
        })
        .cloned()
        .collect()
}

pub fn coalesce_sectioned_credit_buckets_with_inline_choices(
    owner: &Value,
    groups: &[Value],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let mut replacements_by_index = HashMap::<usize, Value>::new();
    let mut consumed_indexes = HashSet::<usize>::new();
    for (bucket_index, bucket) in groups.iter().enumerate() {
        if bucket.get("requirementType").and_then(Value::as_str) != Some("choose_credits")
            || bucket.get("detectedOptionCue").and_then(Value::as_str)
                != Some("sectioned credit list")
        {
            continue;
        }
        let bucket_course_codes = unique_in_order(
            array(bucket.get("options"))
                .iter()
                .flat_map(|option| array(option.get("uwCourses")).iter().map(js_string_value))
                .map(|course_code| normalize_course_code(&course_code))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        if bucket_course_codes.len() < 2 {
            continue;
        }
        let choice_index = groups.iter().enumerate().position(|(candidate_index, candidate)| {
            if candidate_index == bucket_index
                || consumed_indexes.contains(&candidate_index)
                || candidate.get("requirementType").and_then(Value::as_str) != Some("choose_one")
                || candidate.get("category").and_then(Value::as_str) != Some("source-choice")
            {
                return false;
            }
            let source_text = candidate
                .get("sourceRowText")
                .filter(|value| !value.is_null())
                .or_else(|| candidate.get("sourceHeading").filter(|value| !value.is_null()));
            if !inline_or_cue_pattern().is_match(&js_string_nullish(source_text)) {
                return false;
            }
            let choice_course_codes = unique_in_order(
                array(candidate.get("options"))
                    .iter()
                    .flat_map(|option| array(option.get("uwCourses")).iter().map(js_string_value))
                    .map(|course_code| normalize_course_code(&course_code))
                    .filter(|course_code| !course_code.is_empty())
                    .collect(),
            );
            choice_course_codes.len() >= 2
                && choice_course_codes
                    .iter()
                    .any(|course_code| bucket_course_codes.contains(course_code))
        });
        let Some(choice_index) = choice_index else {
            continue;
        };
        let choice = &groups[choice_index];
        let choice_course_codes = unique_in_order(
            array(choice.get("options"))
                .iter()
                .flat_map(|option| array(option.get("uwCourses")).iter().map(js_string_value))
                .map(|course_code| normalize_course_code(&course_code))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        let required_course_codes = bucket_course_codes
            .iter()
            .filter(|course_code| !choice_course_codes.contains(course_code))
            .cloned()
            .collect::<Vec<_>>();
        if required_course_codes.is_empty() {
            continue;
        }
        let raw_text = bucket
            .get("sourceHeading")
            .filter(|value| !value.is_null())
            .or_else(|| bucket.get("label"));
        let choice_source = choice
            .get("sourceRowText")
            .filter(|value| !value.is_null())
            .or_else(|| choice.get("sourceHeading"));
        let paths = choice_course_codes
            .iter()
            .map(|choice_course_code| {
                let mut path_courses = required_course_codes.clone();
                path_courses.push(choice_course_code.clone());
                json!({
                    "label": path_courses.join(" + "),
                    "uwCourses": path_courses,
                    "sourceText": normalize_whitespace(&js_string_nullish(choice_source)),
                })
            })
            .collect::<Vec<_>>();
        let sequence_candidate = json!({
            "rawText": normalize_whitespace(&js_string_nullish(raw_text)),
            "sourceSection": bucket.get("sourceSection"),
            "detectedOptionCue": "sectioned credit list with inline course alternative",
            "paths": paths,
        });
        let Some(sequence_group) =
            build_sequence_choice_requirement_group(owner, &sequence_candidate, course_parser)
        else {
            continue;
        };
        let mut replacement = sequence_group;
        if let Some(object) = replacement.as_object_mut() {
            for key in [
                "label",
                "category",
                "subcategory",
                "minCredits",
                "maxCredits",
                "creditText",
                "sourceHeading",
                "sourceRowText",
            ] {
                if let Some(value) = bucket.get(key) {
                    object.insert(key.to_string(), value.clone());
                } else {
                    object.remove(key);
                }
            }
            let mut notes = array(bucket.get("notes"))
                .iter()
                .map(js_string_value)
                .collect::<Vec<_>>();
            notes.push(
                "Combined required section courses with the source's inline course alternative."
                    .to_string(),
            );
            object.insert("notes".into(), json!(unique_sorted(notes)));
        }
        replacements_by_index.insert(bucket_index, replacement);
        consumed_indexes.insert(choice_index);
    }
    groups
        .iter()
        .enumerate()
        .filter(|(index, _)| !consumed_indexes.contains(index))
        .map(|(index, group)| {
            replacements_by_index
                .remove(&index)
                .unwrap_or_else(|| group.clone())
        })
        .collect()
}

pub fn finalize_parsed_requirement_groups(
    owner: &Value,
    groups: &[Value],
    snapshot_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let groups = filter_requirement_groups_by_supplemental_pathway_scope(
        owner,
        groups,
        snapshot_lines,
        primary_major_titles,
        course_parser,
    );
    let groups = coalesce_sectioned_credit_buckets_with_inline_choices(owner, &groups, course_parser);
    let groups = suppress_inline_required_groups_covered_by_choice_options(&groups);
    let groups = suppress_repeated_required_alternative_groups_covered_by_sequence_choices(&groups);
    let groups = suppress_generic_choice_groups_covered_by_inline_choices(&groups);
    let groups = suppress_subset_source_choice_groups(&groups);
    let groups = suppress_choice_groups_covered_by_credit_buckets(&groups);
    let groups = dedupe_credit_bucket_requirement_groups(&groups);
    let groups = suppress_owner_specific_requirement_group_leaks(owner, &groups);
    let mut seen_ids = HashSet::new();
    groups
        .into_iter()
        .filter(|group| {
            let id_key = group
                .get("id")
                .map(|id| serde_json::to_string(id).unwrap_or_default())
                .unwrap_or_else(|| "undefined".to_string());
            seen_ids.insert(id_key)
        })
        .collect()
}

pub fn get_credit_bucket_dedupe_key(group: &Value) -> Option<String> {
    if group.get("requirementType").and_then(Value::as_str) != Some("choose_credits") {
        return None;
    }
    let source_value = group
        .get("sourceRowText")
        .filter(|value| !value.is_null())
        .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()))
        .or_else(|| group.get("label").filter(|value| !value.is_null()));
    let normalized_text = super::requirement_recovery_scope::normalize_matcher_text(
        &js_string_nullish(source_value),
    )
    .chars()
    .take(100)
    .collect::<String>();
    if normalized_text.is_empty() {
        return None;
    }
    let min_credits = group
        .get("minCredits")
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_default();
    let max_credits = group
        .get("maxCredits")
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_default();
    Some(format!("{normalized_text}::{min_credits}::{max_credits}"))
}

pub fn requirement_group_has_program_specific_category_option(group: &Value) -> bool {
    (group.get("programSpecific").is_some_and(js_truthy)
        && group.get("approvedListKey").is_some_and(js_truthy))
        || array(group.get("options")).iter().any(|option| {
            option
                .pointer("/categoryOption/programSpecific")
                .is_some_and(js_truthy)
        })
}

pub fn count_concrete_course_options(group: &Value) -> usize {
    array(group.get("options"))
        .iter()
        .filter(|option| !array(option.get("uwCourses")).is_empty())
        .count()
}

pub fn choose_preferred_requirement_group(existing: &Value, candidate: &Value) -> Value {
    let existing_concrete_count = count_concrete_course_options(existing);
    let candidate_concrete_count = count_concrete_course_options(candidate);
    if existing_concrete_count != candidate_concrete_count {
        return if candidate_concrete_count > existing_concrete_count {
            candidate.clone()
        } else {
            existing.clone()
        };
    }
    let existing_program_specific = requirement_group_has_program_specific_category_option(existing);
    let candidate_program_specific = requirement_group_has_program_specific_category_option(candidate);
    if existing_program_specific != candidate_program_specific {
        return if candidate_program_specific { candidate.clone() } else { existing.clone() };
    }
    let existing_category_option = array(existing.get("options")).iter().any(|option| {
        option.get("categoryOption").is_some_and(js_truthy)
    });
    let candidate_category_option = array(candidate.get("options")).iter().any(|option| {
        option.get("categoryOption").is_some_and(js_truthy)
    });
    if existing_category_option != candidate_category_option
        && (candidate_program_specific || existing_program_specific)
    {
        return if candidate_category_option { candidate.clone() } else { existing.clone() };
    }
    let existing_approved = existing.get("approvedListKey").is_some_and(js_truthy);
    let candidate_approved = candidate.get("approvedListKey").is_some_and(js_truthy);
    if existing_approved != candidate_approved
        && (candidate_program_specific || existing_program_specific)
    {
        return if candidate_approved { candidate.clone() } else { existing.clone() };
    }
    let existing_placeholder = existing.get("canCreatePlaceholder").is_some_and(js_truthy);
    let candidate_placeholder = candidate.get("canCreatePlaceholder").is_some_and(js_truthy);
    if existing_placeholder != candidate_placeholder
        && (candidate_program_specific || existing_program_specific)
    {
        return if candidate_placeholder { candidate.clone() } else { existing.clone() };
    }
    let existing_option_count = array(existing.get("options")).len();
    let candidate_option_count = array(candidate.get("options")).len();
    if existing_option_count != candidate_option_count {
        return if candidate_option_count > existing_option_count {
            candidate.clone()
        } else {
            existing.clone()
        };
    }
    existing.clone()
}

pub fn dedupe_credit_bucket_requirement_groups(groups: &[Value]) -> Vec<Value> {
    let mut key_indexes = HashMap::new();
    let mut result = Vec::new();
    for group in groups {
        let Some(key) = get_credit_bucket_dedupe_key(group) else {
            result.push(group.clone());
            continue;
        };
        if let Some(index) = key_indexes.get(&key).copied() {
            result[index] = choose_preferred_requirement_group(&result[index], group);
        } else {
            key_indexes.insert(key, result.len());
            result.push(group.clone());
        }
    }
    result
}

fn has_parenthetical_sequence_choice_candidate(
    source_line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = super::requirement_choice_lists::normalize_sequence_source_line(source_line);
    if !parenthetical_sequence_or_marker_pattern().is_match(&normalized_line) {
        return false;
    }
    for captures in parenthetical_sequence_choice_pattern().captures_iter(&normalized_line) {
        let (Some(left_text), Some(right_text)) = (captures.get(1), captures.get(2)) else {
            continue;
        };
        let left_courses = course_parser.extract_from_line(left_text.as_str());
        if left_courses.len() < 2 {
            continue;
        }
        let Some(subject) = left_courses[0].rsplit_once(' ').map(|(subject, _)| subject.to_string()) else {
            continue;
        };
        let right_explicit_courses = course_parser.extract_from_line(right_text.as_str());
        let rows = [super::sequence_text::SequenceCourseCodeRow {
            text: right_text.as_str().to_string(),
            fallback_subject: Some(subject.clone()),
            explicit_course_codes: right_explicit_courses,
        }];
        let right_courses = super::sequence_text::expand_sequence_course_codes(&rows)
            .into_iter()
            .next()
            .unwrap_or_default();
        if right_courses.len() >= 2
            && right_courses[0]
                .rsplit_once(' ')
                .is_some_and(|(right_subject, _)| right_subject == subject.as_str())
        {
            return true;
        }
    }
    false
}

pub fn extract_course_codes_from_credit_bucket_text(
    text: &str,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut course_codes = Vec::new();
    for captures in credit_bucket_explicit_course_pattern().captures_iter(text) {
        if let (Some(subject), Some(number)) = (captures.get(1), captures.get(2)) {
            if let Some(course_code) = course_parser.normalize_code(subject.as_str(), number.as_str()) {
                course_codes.push(course_code);
            }
        }
    }
    course_codes.extend(course_parser.extract_from_line(text));
    unique_in_order(
        course_codes
            .into_iter()
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    )
}

pub fn build_course_requirement_options_from_credit_bucket(
    owner: &Value,
    text: &str,
    credit_range: Option<(f64, Option<f64>)>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    extract_course_codes_from_credit_bucket_text(text, course_parser)
        .into_iter()
        .map(|course_code| {
            let metadata = course_parser.metadata_for(&course_code);
            let course_credit_range = course_parser.credit_range_for(&course_code);
            let department = course_code
                .rsplit_once(' ')
                .map(|(subject, _)| subject.to_string())
                .unwrap_or_default();
            let notes = credit_range
                .map(|(minimum, _)| vec![format!("Option parsed from a {}-credit source bucket.", js_number_string(minimum))])
                .unwrap_or_default();
            json!({
                "id": format!(
                    "{owner_id}:requirement-option:{}",
                    slugify_requirement_identifier(&format!("credit-bucket-{course_code}"))
                ),
                "displayCourseCodes": [course_code],
                "uwCourses": [course_code],
                "credits": course_credit_range.credits,
                "creditMin": course_credit_range.credit_min,
                "creditMax": course_credit_range.credit_max,
                "creditText": course_credit_range.credit_text,
                "title": metadata.and_then(|value| value.get("title")).cloned().unwrap_or(Value::Null),
                "department": department,
                "category": "approved-credit-bucket",
                "label": course_code,
                "sourceHeading": text,
                "sourceCategory": "credit-bucket",
                "notes": notes,
            })
        })
        .collect()
}

pub fn build_credit_bucket_source_line(
    owner: &Value,
    snapshot_lines: &[String],
    start_index: usize,
    course_parser: &CourseCodeParser,
) -> String {
    let initial_line = snapshot_lines
        .get(start_index)
        .map(|line| super::requirement_choice_lists::strip_choice_list_line(line))
        .unwrap_or_default();
    let mut parts = if initial_line.is_empty() {
        Vec::new()
    } else {
        vec![initial_line.clone()]
    };
    let first_line = normalize_pdf_requirement_heading_line(&parts.join(" "));
    let first_line_was_trimmed =
        first_line != normalize_whitespace(&initial_line);
    if !super::requirement_rule_utils::should_join_credit_bucket_continuation(&first_line)
        || first_line_was_trimmed
    {
        return normalize_whitespace(&first_line);
    }
    let end = snapshot_lines.len().min(start_index.saturating_add(5));
    for index in start_index.saturating_add(1)..end {
        let continuation =
            super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[index]);
        let normalized_continuation = normalize_whitespace(&continuation);
        if additional_requirements_heading_pattern().is_match(&normalized_continuation) {
            continue;
        }
        if looks_like_sibling_credit_bucket_boundary(&continuation, course_parser) {
            break;
        }
        if !super::requirement_rule_utils::is_credit_bucket_continuation_line(&continuation) {
            break;
        }
        parts.push(continuation);
        let joined = parts.join(" ");
        if get_program_specific_credit_bucket_descriptor(owner, &joined, course_parser).is_some()
            && program_credit_bucket_continuation_stop_pattern().is_match(&joined)
        {
            break;
        }
    }
    normalize_whitespace(&parts.join(" "))
}

pub fn split_credit_bucket_source_lines(source_line: &str) -> Vec<String> {
    let normalized_line = normalize_whitespace(source_line);
    let mut parts = Vec::<String>::new();
    for part in credit_bucket_semicolon_split_pattern()
        .split(&normalized_line)
        .map(normalize_whitespace)
        .filter(|part| !part.is_empty())
    {
        if !parts.is_empty()
            && parse_requirement_credit_range(&part).is_none()
            && credit_bucket_and_course_prefix_pattern().is_match(&part)
        {
            let previous = parts.pop().unwrap_or_default();
            let continuation = credit_bucket_leading_and_pattern().replace(&part, "");
            parts.push(format!("{previous}, {}", normalize_whitespace(&continuation)));
        } else {
            parts.push(part);
        }
    }
    let credit_part_count = parts
        .iter()
        .filter(|part| {
            parse_requirement_credit_range(part).is_some()
                && !source_line_starts_with_course_code(part)
        })
        .count();
    if parts.len() < 2 || credit_part_count < 2 {
        return vec![normalized_line];
    }
    let prefix = parts
        .first()
        .and_then(|part| credit_bucket_heading_prefix_pattern().captures(part))
        .and_then(|captures| captures.get(1))
        .map(|capture| normalize_whitespace(capture.as_str()));
    parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            if index > 0 && !part.contains(':') {
                if let Some(prefix) = prefix.as_deref() {
                    return format!("{prefix}: {part}");
                }
            }
            part
        })
        .collect()
}

fn looks_like_sibling_credit_bucket_boundary(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    parse_requirement_credit_range(&normalized_line).is_some()
        && sibling_credit_bucket_heading_pattern().is_match(&normalized_line)
        && has_credit_bucket_cue(&normalized_line, course_parser)
}

pub fn get_program_elective_bucket_label(text: &str) -> String {
    let normalized_text = normalize_whitespace(text);
    let raw_label = program_elective_bucket_label_pattern()
        .captures(&normalized_text)
        .and_then(|captures| captures.get(1))
        .map(|capture| normalize_whitespace(capture.as_str()))
        .unwrap_or_default();
    if raw_label.is_empty() || program_elective_credit_only_pattern().is_match(&raw_label) {
        return "Program Elective Credit".to_string();
    }
    if program_electives_credit_pattern().is_match(&raw_label) {
        return program_electives_credit_pattern()
            .replace(&raw_label, "Elective Credit")
            .to_string();
    }
    program_elective_word_pattern()
        .replace(&raw_label, "Elective Credit")
        .to_string()
}

pub fn detect_program_elective_credit_bucket(owner: &Value, text: &str) -> Option<Value> {
    let normalized_text = normalize_whitespace(text);
    let (min_credits, max_credits) = parse_requirement_credit_range(&normalized_text)?;
    if program_free_electives_pattern().is_match(&normalized_text)
        || program_approved_cue_pattern().is_match(&normalized_text)
        || !program_elective_credit_cue_pattern().is_match(&normalized_text)
    {
        return None;
    }
    let label = get_program_elective_bucket_label(&normalized_text);
    let owner_value = owner
        .get("planId")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("ownerId").filter(|value| !value.is_null()));
    let owner_part = slugify_requirement_identifier(
        &owner_value
            .map(js_string_value)
            .unwrap_or_else(|| "program".to_string()),
    );
    let label_part = slugify_requirement_identifier(&label);
    let category = program_elective_category(&label);
    let credit_text = max_credits
        .filter(|max_credits| *max_credits != min_credits)
        .map(|max_credits| {
            format!(
                "{}-{}",
                js_number_string(min_credits),
                js_number_string(max_credits)
            )
        })
        .unwrap_or_else(|| js_number_string(min_credits));
    Some(json!({
        "category": category,
        "sourceCategoryCode": label,
        "longLabel": label,
        "preferredLabel": format!("{label} ({credit_text} credits)"),
        "approvedListKey": format!("{owner_part}-{label_part}"),
        "programSpecific": true,
    }))
}

pub fn extract_subject_prefix_from_credit_bucket_text(
    text: &str,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    let normalized_text = normalize_whitespace(text);
    for pattern in subject_prefix_credit_bucket_patterns() {
        let Some(subject) = pattern
            .captures(&normalized_text)
            .and_then(|captures| captures.get(1))
        else {
            continue;
        };
        if let Some(subject) = course_parser.normalize_subject(subject.as_str()) {
            return Some(subject);
        }
    }
    None
}

pub fn get_program_specific_credit_bucket_descriptor(
    owner: &Value,
    text: &str,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let normalized_text = normalize_whitespace(text);
    super::requirement_rule_utils::detect_program_approved_credit_bucket(
        &normalized_text,
        owner,
    )
    .or_else(|| detect_subject_prefix_credit_bucket(owner, &normalized_text, course_parser))
    .or_else(|| detect_program_elective_credit_bucket(owner, &normalized_text))
}

pub fn detect_subject_prefix_credit_bucket(
    owner: &Value,
    text: &str,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let normalized_text = normalize_whitespace(text);
    if !program_courses_or_electives_pattern().is_match(&normalized_text) {
        return None;
    }
    let subject_prefix =
        extract_subject_prefix_from_credit_bucket_text(&normalized_text, course_parser)?;
    let campus_id = js_string_nullish(owner.get("campusId"));
    let approved_courses = course_parser
        .metadata_course_codes_for_subject_prefix(&campus_id, &subject_prefix);
    if approved_courses.is_empty() {
        return None;
    }
    let owner_value = owner
        .get("planId")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("ownerId").filter(|value| !value.is_null()))
        .map(js_string_value)
        .unwrap_or_else(|| "program".to_string());
    let owner_part = slugify_requirement_identifier(&owner_value);
    let subject_part = slugify_requirement_identifier(&subject_prefix);
    let category = program_elective_category(&format!("{subject_prefix}_PREFIX"));
    Some(json!({
        "category": category,
        "sourceCategoryCode": format!("{subject_prefix} prefix"),
        "longLabel": format!("{subject_prefix} prefix courses"),
        "approvedListKey": format!("{owner_part}-{subject_part}-prefix-courses"),
        "programSpecific": true,
        "subjectPrefix": subject_prefix,
        "approvedUwCourseCodes": approved_courses,
    }))
}

pub fn get_approved_list_key_from_credit_bucket_text(
    owner: &Value,
    text: &str,
    descriptor: Option<&Value>,
) -> Option<String> {
    if let Some(key) = descriptor
        .and_then(|value| value.get("approvedListKey"))
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .filter(|value| !value.is_empty())
    {
        return Some(key);
    }
    let normalized_text = normalize_whitespace(text);
    if !program_approved_cue_pattern().is_match(&normalized_text) {
        return None;
    }
    let owner_value = owner
        .get("planId")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("ownerId").filter(|value| !value.is_null()))
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let owner_part = slugify_requirement_identifier(&owner_value);
    if program_elective_word_pattern().is_match(&normalized_text) {
        return Some(format!("{owner_part}-approved-electives"));
    }
    if program_natural_science_pattern().is_match(&normalized_text) {
        return Some(format!("{owner_part}-approved-natural-science"));
    }
    if program_math_science_pattern().is_match(&normalized_text) {
        return Some(format!("{owner_part}-approved-math-science"));
    }
    None
}

pub fn get_positive_credit_bucket_category_text(text: &str) -> String {
    let normalized_text = normalize_whitespace(text);
    let without_overlap_note = credit_bucket_overlap_note_pattern()
        .replace_all(&normalized_text, "");
    let positive_text = normalize_whitespace(
        &credit_bucket_overlap_tail_pattern().replace(&without_overlap_note, ""),
    );
    if positive_text.is_empty() {
        normalized_text
    } else {
        positive_text
    }
}

pub fn next_line_looks_like_course_list_for_heading(
    snapshot_lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let next_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(
            snapshot_lines.get(index.saturating_add(1)).map(String::as_str).unwrap_or_default(),
        ),
    );
    course_parser.extract_from_line(&next_line).len() >= 2
        && next_line_course_list_delimiter_pattern().is_match(&next_line)
}

pub fn is_approved_list_placeholder_without_immediate_course_list(
    snapshot_lines: &[String],
    index: usize,
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = normalize_whitespace(line);
    if !approved_list_and_pattern().is_match(&normalized_line)
        || !approved_list_context_pattern().is_match(&normalized_line)
        || next_line_looks_like_course_list_for_heading(snapshot_lines, index, course_parser)
    {
        return false;
    }
    let next_meaningful_line = snapshot_lines
        .iter()
        .skip(index.saturating_add(1))
        .take(3)
        .map(|candidate| {
            normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(candidate),
            )
        })
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_default();
    approved_list_from_placeholder_pattern().is_match(&normalized_line)
        || approved_list_adviser_pattern().is_match(&normalized_line)
        || approved_department_list_pattern()
            .is_match(&format!("{normalized_line} {next_meaningful_line}"))
}

pub fn looks_like_aggregate_general_education_credit_summary(text: &str) -> bool {
    let normalized_text = normalize_whitespace(text);
    aggregate_general_education_credit_mentions_pattern()
        .find_iter(&normalized_text)
        .count()
        >= 2
        && super::requirement_category_options::count_requirement_category_option_descriptors(
            &normalized_text,
        ) >= 2
        && aggregate_general_education_heading_pattern().is_match(&normalized_text)
        && aggregate_general_education_includes_pattern().is_match(&normalized_text)
}

pub fn looks_like_shared_humanities_social_credit_heading(text: &str) -> bool {
    let normalized_text = normalize_whitespace(text);
    parse_requirement_credit_range(&normalized_text).is_some()
        && (shared_humanities_social_credit_pattern().is_match(&normalized_text)
            || shared_vlpa_is_credit_pattern().is_match(&normalized_text)
            || shared_long_category_credit_pattern().is_match(&normalized_text))
}

pub fn has_following_specific_humanities_social_minimums(
    snapshot_lines: &[String],
    index: usize,
) -> bool {
    let following_text = normalize_whitespace(
        &snapshot_lines
            .iter()
            .skip(index.saturating_add(1))
            .take(4)
            .map(|line| super::requirement_choice_lists::strip_choice_list_line(line))
            .collect::<Vec<_>>()
            .join(" "),
    );
    following_humanities_minimum_pattern().is_match(&following_text)
        && following_social_sciences_minimum_pattern().is_match(&following_text)
}

pub fn should_skip_credit_bucket_line(
    snapshot_lines: &[String],
    index: usize,
    text: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_text = normalize_whitespace(text);
    if normalized_text.is_empty()
        || (looks_like_composite_credit_heading(&normalized_text)
            && !starts_with_category_credit_cue(&normalized_text))
    {
        return true;
    }
    if looks_like_degree_total_general_elective_prose(&normalized_text)
        || looks_like_aggregate_general_education_credit_summary(&normalized_text)
        || looks_like_credit_summary_for_named_requirement_section(&normalized_text)
        || looks_like_elective_timing_or_sample_plan_note(&normalized_text)
        || credit_bucket_exclusion_pattern().is_match(&normalized_text)
        || students_earn_remaining_credit_pattern().is_match(&normalized_text)
    {
        return true;
    }
    if is_generic_category_credit_bucket_heading(&normalized_text, course_parser)
        && has_immediate_sectioned_course_rows_after_heading(
            snapshot_lines,
            index,
            1,
            course_parser,
        )
    {
        return true;
    }
    if looks_like_shared_humanities_social_credit_heading(&normalized_text)
        && has_following_specific_humanities_social_minimums(snapshot_lines, index)
    {
        return true;
    }
    credit_bucket_elective_single_heading_pattern().is_match(&normalized_text)
        && !credit_bucket_elective_disqualifier_pattern().is_match(&normalized_text)
        && next_line_looks_like_course_list_for_heading(snapshot_lines, index, course_parser)
}

pub fn looks_like_composite_credit_heading(text: &str) -> bool {
    composite_credit_heading_pattern().is_match(&normalize_whitespace(text))
}

pub fn starts_with_category_credit_cue(text: &str) -> bool {
    let source_line = normalize_whitespace(text);
    category_credit_heading_prefix_pattern().is_match(&source_line)
        || category_credit_count_prefix_pattern().is_match(&source_line)
}

pub fn has_non_requirement_course_exclusion_cue(
    text: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_text = normalize_whitespace(text);
    !normalized_text.is_empty()
        && !course_parser.extract_from_line(&normalized_text).is_empty()
        && non_requirement_course_exclusion_pattern().is_match(&normalized_text)
}

pub fn has_course_question_support_cue(text: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized_text = normalize_whitespace(text);
    normalized_text.contains('?')
        && !source_line_starts_with_course_code(&normalized_text)
        && !course_parser.extract_from_line(&normalized_text).is_empty()
}

pub fn has_credit_bucket_cue(text: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized_text = normalize_whitespace(text);
    if parse_requirement_credit_range(&normalized_text).is_none() {
        return false;
    }
    credit_bucket_category_cue_pattern().is_match(&normalized_text)
        || credit_bucket_elective_from_pattern().is_match(&normalized_text)
        || credit_bucket_prefixed_course_pattern().is_match(&normalized_text)
        || credit_bucket_credits_course_list_pattern().is_match(&normalized_text)
        || credit_bucket_introductory_pattern().is_match(&normalized_text)
        || credit_bucket_language_pattern().is_match(&normalized_text)
        || credit_bucket_year_level_pattern().is_match(&normalized_text)
        || (credit_bucket_credit_from_pattern().is_match(&normalized_text)
            && course_parser.extract_from_line(&normalized_text).len() >= 2)
}

pub fn looks_like_admission_decision_narrative_credit_bucket(text: &str) -> bool {
    let normalized_text = normalize_whitespace(text);
    admission_decision_credit_cue_pattern().is_match(&normalized_text)
        || admission_decision_quality_pattern().is_match(&normalized_text)
}

pub fn looks_like_elective_timing_or_sample_plan_note(text: &str) -> bool {
    let normalized_text = normalize_whitespace(text);
    elective_timing_pattern().is_match(&normalized_text)
        && elective_taken_pattern().is_match(&normalized_text)
        && elective_sample_plan_pattern().is_match(&normalized_text)
}

pub fn looks_like_support_only_research_opportunity_line(source_line: &str, label: &str) -> bool {
    let normalized_line = normalize_whitespace(source_line);
    let normalized_label = normalize_whitespace(label);
    research_internship_service_label_pattern().is_match(&normalized_label)
        && research_opportunity_context_pattern().is_match(&normalized_line)
}

pub fn get_sequence_choice_cue(value: &str) -> Option<&'static str> {
    let text = normalize_whitespace(value);
    sequence_choice_cue_patterns()
        .iter()
        .find(|(pattern, _)| pattern.is_match(&text))
        .map(|(_, cue)| *cue)
}

pub fn get_sequence_path_label_from_text(
    text: &str,
    course_codes: &[String],
    fallback_label: Option<&str>,
) -> String {
    for (pattern, label) in sequence_path_label_patterns() {
        if pattern.is_match(text) {
            return (*label).to_string();
        }
    }
    if let Some(label) = fallback_label
        .map(normalize_whitespace)
        .filter(|label| !label.is_empty())
    {
        return label;
    }
    course_codes
        .first()
        .map(|course_code| format!("{course_code} sequence"))
        .unwrap_or_else(|| "sequence option".to_string())
}

pub fn is_equivalent_number_alias_line(line: &str, course_codes: &[String]) -> bool {
    if course_codes.len() != 2 {
        return false;
    }
    let normalized_line = normalize_whitespace(line);
    if !equivalent_number_alias_cue_pattern().is_match(&normalized_line) {
        return parenthetical_or_course_alias_pattern().is_match(&normalized_line)
            || slash_course_number_alias_pattern().is_match(&normalized_line);
    }
    true
}

pub fn should_skip_raw_partial_choice_line(
    is_combined_choice_line: bool,
    has_explicit_or: bool,
    has_own_choice_context: bool,
    normalized_line: &str,
) -> bool {
    if is_combined_choice_line || has_explicit_or {
        return false;
    }
    if !has_own_choice_context {
        return true;
    }
    choice_list_start_pattern().is_match(
        &super::requirement_choice_lists::strip_choice_list_line(normalized_line),
    ) && !choice_line_trailing_parenthesis_pattern().is_match(normalized_line)
}

pub fn get_choice_requirement_group_label(
    normalized_line: &str,
    course_codes: &[String],
    course_parser: &CourseCodeParser,
) -> String {
    let line_without_page = super::requirement_choice_lists::strip_choice_list_line(normalized_line);
    if let Some(match_start) = parenthetical_choice_group_label_pattern()
        .find(&line_without_page)
        .map(|matched| matched.start())
        .filter(|start| *start > 0)
    {
        let prefix = normalize_whitespace(&line_without_page[..match_start]);
        if !prefix.is_empty()
            && course_parser.extract_from_line(&prefix).is_empty()
            && choice_requirement_label_pattern().is_match(&prefix)
        {
            return prefix;
        }
    }
    if let Some(match_start) = choose_list_group_label_pattern()
        .find(&line_without_page)
        .map(|matched| matched.start())
        .filter(|start| *start > 0)
    {
        let prefix = normalize_whitespace(&line_without_page[..match_start]);
        if !prefix.is_empty()
            && course_parser.extract_from_line(&prefix).is_empty()
            && choice_requirement_label_pattern().is_match(&prefix)
        {
            return normalize_whitespace(&trailing_group_label_punctuation_pattern().replace_all(&prefix, ""));
        }
    }
    if normalized_line.encode_utf16().count() <= 120 {
        normalized_line.to_string()
    } else {
        course_codes.join(" or ")
    }
}

pub fn build_forward_labeled_choice_requirement_groups(
    owner: &Value,
    parsed_course_codes: &[String],
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let parsed_course_code_set = parsed_course_codes
        .iter()
        .map(|course_code| normalize_course_code(course_code))
        .collect::<HashSet<_>>();
    let mut groups = Vec::new();
    let mut seen_group_ids = HashSet::new();
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    for source_line in build_forward_labeled_choice_requirement_lines(snapshot_lines, course_parser) {
        let normalized_line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(&source_line),
        );
        let option_extraction_line =
            super::requirement_choice_lists::strip_non_option_course_evidence_from_choice_line(
                &normalized_line,
                course_parser,
            );
        let course_codes = unique_in_order(
            course_parser
                .extract_from_requirement_line(&option_extraction_line)
                .into_iter()
                .map(|course_code| normalize_course_code(&course_code))
                .filter(|course_code| parsed_course_code_set.contains(course_code))
                .collect(),
        );
        let fallback_credits = parse_requirement_credit_amount(&normalized_line);
        let category_options = build_category_requirement_options_from_choice_line(
            owner,
            &option_extraction_line,
            fallback_credits,
            course_parser,
        );
        if course_codes.len() + category_options.len() < 2 {
            continue;
        }
        let required_count = parse_choice_required_count(&option_extraction_line);
        let group_id = format!(
            "{owner_id}:requirement-group:{}",
            slugify_requirement_identifier(&course_codes.join("-or-"))
        );
        if seen_group_ids.contains(&group_id) {
            continue;
        }
        let mut options = course_codes
            .iter()
            .map(|course_code| {
                json!({
                    "id": format!("{owner_id}:requirement-option:{}", slugify_requirement_identifier(course_code)),
                    "uwCourses": [course_code],
                    "credits": fallback_credits,
                    "label": course_code,
                })
            })
            .collect::<Vec<_>>();
        options.extend(category_options);
        let mut group_input = Map::new();
        group_input.insert("id".into(), json!(group_id));
        group_input.insert(
            "label".into(),
            json!(get_choice_requirement_group_label(
                &option_extraction_line,
                &course_codes,
                course_parser,
            )),
        );
        group_input.insert("sourceHeading".into(), json!(normalized_line));
        group_input.insert("sourceRowText".into(), json!(normalized_line));
        group_input.insert("sourceSection".into(), Value::Null);
        group_input.insert(
            "detectedOptionCue".into(),
            detect_option_cue(&option_extraction_line)
                .map(|cue| json!(cue))
                .unwrap_or(Value::Null),
        );
        group_input.insert("category".into(), json!("source-choice"));
        group_input.insert(
            "requirementType".into(),
            json!(if required_count > 1 { "choose_n" } else { "choose_one" }),
        );
        group_input.insert("minCourses".into(), json!(required_count));
        group_input.insert("maxCourses".into(), json!(required_count));
        group_input.insert("selectionCount".into(), json!(required_count));
        group_input.insert("requiredCount".into(), json!(required_count));
        group_input.insert("options".into(), json!(options));
        let group = build_parsed_requirement_group(&Value::Object(group_input));
        if array(group.get("options")).is_empty() {
            continue;
        }
        let group_id = group.get("id").map(js_string_value).unwrap_or_default();
        seen_group_ids.insert(group_id);
        groups.push(group);
    }
    groups
}

pub fn extract_option_course_codes_from_sectioned_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let normalized_line = normalize_whitespace(&line.replace('*', ""));
    if normalized_line.is_empty() {
        return Vec::new();
    }
    let without_prerequisite_parentheticals = normalize_whitespace(
        &prerequisite_parenthetical_pattern().replace_all(&normalized_line, ""),
    );
    let leading_course_text = leading_course_text_pattern()
        .captures(&without_prerequisite_parentheticals)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str())
        .unwrap_or(&without_prerequisite_parentheticals);
    let leading_codes = extract_course_codes_from_joint_course_line(leading_course_text, course_parser);
    if !leading_codes.is_empty() {
        return leading_codes;
    }
    extract_course_codes_from_joint_course_line(
        &without_prerequisite_parentheticals,
        course_parser,
    )
}

pub fn build_sectioned_option_course_option(
    owner: &Value,
    input: &Value,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let course_line = js_string_nullish(input.get("courseLine"));
    let course_codes = extract_option_course_codes_from_sectioned_line(&course_line, course_parser);
    let primary_course_code = course_codes.first()?.clone();
    let metadata = course_parser.metadata_for(&primary_course_code);
    let title = get_inline_section_course_title(&course_line, &primary_course_code)
        .or_else(|| {
            metadata
                .and_then(|metadata| metadata.get("title"))
                .filter(|value| !value.is_null())
                .map(js_string_value)
                .map(|title| normalize_whitespace(&title))
                .filter(|title| !title.is_empty())
        })
        .or_else(|| {
            let title = normalize_whitespace(&js_string_nullish(input.get("title")));
            (!title.is_empty()).then_some(title)
        });
    let credit_range = input
        .get("creditRange")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| {
            let range = course_parser.credit_range_for(&primary_course_code);
            json!({
                "credits": range.credits,
                "creditMin": range.credit_min,
                "creditMax": range.credit_max,
                "creditText": range.credit_text,
            })
        });
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let normalized_course_line = normalize_whitespace(&trailing_asterisks_pattern().replace(&course_line, ""));
    let section_slug = super::requirement_rule_utils::slugify_parser_rule_value(
        &js_string_nullish(input.get("sectionHeading")),
    );
    let option_slug = super::requirement_rule_utils::slugify_parser_rule_value(&format!(
        "{section_slug}-{}",
        course_codes.join("-")
    ));
    let department = primary_course_code
        .rsplit_once(' ')
        .map(|(subject, _)| subject.to_string());
    let mut option = Map::new();
    option.insert(
        "id".into(),
        json!(format!("{owner_id}:requirement-option:{option_slug}")),
    );
    option.insert("displayCourseCodes".into(), json!([normalized_course_line]));
    option.insert("uwCourses".into(), json!([primary_course_code]));
    option.insert(
        "equivalentUwCourseCodes".into(),
        json!(course_codes.into_iter().skip(1).collect::<Vec<_>>()),
    );
    for key in ["credits", "creditMin", "creditMax", "creditText"] {
        option.insert(
            key.into(),
            credit_range.get(key).cloned().unwrap_or(Value::Null),
        );
    }
    option.insert("title".into(), title.map(|title| json!(title)).unwrap_or(Value::Null));
    option.insert("department".into(), department.map(|department| json!(department)).unwrap_or(Value::Null));
    option.insert("category".into(), nullish_value(input.get("category")));
    option.insert(
        "label".into(),
        json!(if normalized_course_line.is_empty() {
            primary_course_code
        } else {
            normalized_course_line
        }),
    );
    option.insert("sourceHeading".into(), nullish_value(input.get("sourceHeading")));
    option.insert("sourceCategory".into(), nullish_value(input.get("sectionHeading")));
    option.insert(
        "notes".into(),
        input.get("notes").filter(|value| !value.is_null()).cloned().unwrap_or_else(|| json!([])),
    );
    option.insert(
        "constraints".into(),
        input.get("constraints").filter(|value| !value.is_null()).cloned().unwrap_or_else(|| json!([])),
    );
    Some(build_parsed_requirement_option(&Value::Object(option)))
}

pub fn parse_sectioned_course_credit_list_line(line: &str) -> Option<Value> {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if normalized_line.is_empty() {
        return None;
    }
    let parts = normalized_line
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    let mut ranges = Vec::new();
    for part in &parts {
        let captures = sectioned_credit_range_part_pattern().captures(part)?;
        let minimum = captures.get(1)?.as_str().parse::<f64>().ok()?;
        let maximum = captures
            .get(2)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .filter(|maximum| *maximum >= minimum)
            .unwrap_or(minimum);
        if !minimum.is_finite()
            || minimum <= 0.0
            || minimum > 30.0
            || maximum > 30.0
        {
            return None;
        }
        ranges.push((minimum, maximum));
    }
    let (credits, credit_max) = ranges[0];
    Some(json!({
        "credits": credits,
        "creditMin": credits,
        "creditMax": credit_max,
        "creditText": parts.join(", "),
    }))
}

pub fn parse_section_heading_credit_range(line: &str) -> Option<Value> {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if let Some(captures) = leading_named_elective_credit_pattern().captures(&normalized_line) {
        let credits = captures.get(1)?.as_str().parse::<f64>().ok()?;
        if credits.is_finite() && credits > 0.0 {
            return Some(json!({"minCredits": credits, "maxCredits": credits}));
        }
    }
    if let Some((minimum, maximum)) = parse_requirement_credit_range(line) {
        return Some(json!({"minCredits": minimum, "maxCredits": maximum}));
    }
    parse_requirement_credit_amount(line)
        .filter(|credits| credits.is_finite())
        .map(|credits| json!({"minCredits": credits, "maxCredits": credits}))
}

pub fn parse_leading_section_heading_credit_range(line: &str) -> Option<Value> {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if normalized_line.is_empty() || source_line_starts_with_course_code(&normalized_line) {
        return None;
    }
    if let Some(range) = parse_section_heading_credit_range(&normalized_line) {
        return Some(range);
    }
    let captures = simple_leading_parenthetical_credit_range_pattern()
        .captures(&normalized_line)
        .or_else(|| simple_leading_credit_range_pattern().captures(&normalized_line))
        .or_else(|| leading_parenthetical_credit_range_pattern().captures(&normalized_line))?;
    let minimum = captures.get(1)?.as_str().parse::<f64>().ok()?;
    let maximum = captures
        .get(2)
        .and_then(|value| value.as_str().parse::<f64>().ok())
        .filter(|maximum| *maximum >= minimum)
        .unwrap_or(minimum);
    if !minimum.is_finite() || minimum <= 0.0 {
        return None;
    }
    Some(json!({"minCredits": minimum, "maxCredits": maximum}))
}

pub fn collect_sectioned_option_course_options(
    owner: &Value,
    snapshot_lines: &[String],
    start_index: usize,
    end_index: usize,
    input: &Value,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let mut options: Vec<Value> = Vec::new();
    for index in start_index + 1..end_index.min(snapshot_lines.len()) {
        let line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[index]),
        );
        if line.is_empty() {
            continue;
        }
        let Some(course_line) = get_sectioned_option_course_line(&line) else {
            if is_course_continuation_line(&line, course_parser) && !options.is_empty() {
                let continuation_codes = course_parser.extract_from_line(&line);
                if let Some(previous) = options.last_mut().and_then(Value::as_object_mut) {
                    let uw_courses = string_values(previous.get("uwCourses"));
                    let mut equivalents = string_values(previous.get("equivalentUwCourseCodes"));
                    equivalents.extend(
                        continuation_codes
                            .into_iter()
                            .filter(|course_code| !uw_courses.contains(course_code)),
                    );
                    previous.insert(
                        "equivalentUwCourseCodes".into(),
                        json!(unique_in_order(equivalents)),
                    );
                    let mut display_codes = string_values(previous.get("displayCourseCodes"));
                    display_codes.push(normalize_whitespace(&line));
                    previous.insert(
                        "displayCourseCodes".into(),
                        json!(unique_in_order(display_codes)),
                    );
                    let prior_label = previous
                        .get("label")
                        .filter(|value| !value.is_null())
                        .map(js_string_value)
                        .filter(|label| !label.is_empty())
                        .or_else(|| uw_courses.first().cloned())
                        .unwrap_or_default();
                    previous.insert(
                        "label".into(),
                        json!(normalize_whitespace(&format!("{prior_label} {line}"))),
                    );
                }
            }
            continue;
        };
        if line.starts_with('*') && starred_course_note_pattern().is_match(&line) {
            continue;
        }
        let course_codes = extract_option_course_codes_from_sectioned_line(&course_line, course_parser);
        if course_codes.is_empty() {
            continue;
        }
        let title = get_nearby_section_course_title(snapshot_lines, index, course_parser);
        let credit_range = parse_requirement_credit_range(&line)
            .map(|(minimum, maximum)| credit_range_value(minimum, maximum, None))
            .unwrap_or_else(|| {
                get_nearby_section_course_credits(
                    snapshot_lines,
                    index,
                    &course_codes[0],
                    course_parser,
                )
            });
        let nearby_detail_lines = snapshot_lines
            .iter()
            .skip(index + 1)
            .take(3)
            .map(|candidate| {
                normalize_whitespace(
                    &super::requirement_choice_lists::strip_choice_list_line(candidate),
                )
            })
            .filter(|candidate| {
                !candidate.is_empty()
                    && !source_line_starts_with_course_code(candidate)
                    && !is_sectioned_course_credit_list_line(candidate)
            })
            .collect::<Vec<_>>();
        let max_credit_match = std::iter::once(title.as_deref().unwrap_or_default())
            .chain(nearby_detail_lines.iter().map(String::as_str))
            .find_map(|candidate| {
                max_degree_credit_constraint_pattern()
                    .captures(candidate)
                    .and_then(|captures| captures.get(1))
                    .and_then(|credits| credits.as_str().parse::<f64>().ok())
            });
        let mut notes = Vec::<String>::new();
        let mut constraints = Vec::<String>::new();
        if let Some(max_credits) = max_credit_match.filter(|credits| credits.is_finite()) {
            constraints.push(format!("max_degree_counting_credits:{}", js_number_string(max_credits)));
            notes.push(format!(
                "{} can count a maximum of {} credits toward the degree.",
                course_codes[0],
                js_number_string(max_credits)
            ));
        }
        if title
            .as_deref()
            .is_some_and(|title| prerequisite_title_pattern().is_match(title))
        {
            if let Some(title) = title.as_ref() {
                notes.push(title.clone());
            }
        }
        if let Some(must_be_taken_line) = snapshot_lines.get(index + 2) {
            if must_be_taken_pattern().is_match(must_be_taken_line) {
                notes.push(normalize_whitespace(must_be_taken_line));
            }
        }
        let option_course_lines = if should_split_sectioned_course_line_into_separate_options(
            &course_line,
            &course_codes,
        ) {
            course_codes.clone()
        } else {
            vec![course_line]
        };
        for option_course_line in option_course_lines {
            let mut option_input = Map::new();
            option_input.insert("courseLine".into(), json!(option_course_line));
            option_input.insert(
                "title".into(),
                title.as_ref().map(|title| json!(title)).unwrap_or(Value::Null),
            );
            option_input.insert("creditRange".into(), credit_range.clone());
            option_input.insert("category".into(), nullish_value(input.get("category")));
            option_input.insert("sectionHeading".into(), nullish_value(input.get("sectionHeading")));
            option_input.insert("sourceHeading".into(), nullish_value(input.get("sourceHeading")));
            option_input.insert("notes".into(), json!(notes));
            option_input.insert("constraints".into(), json!(constraints));
            if let Some(option) = build_sectioned_option_course_option(
                owner,
                &Value::Object(option_input),
                course_parser,
            ) {
                options.push(option);
            }
        }
    }
    options
}

pub fn parse_sectioned_course_required_count(line: &str) -> Option<usize> {
    let normalized_line = normalize_whitespace(line).to_lowercase();
    if sectioned_explicit_course_count_pattern().is_match(&normalized_line) {
        let explicit_choice_count = parse_choice_required_count(&normalized_line);
        if explicit_choice_count > 0 {
            return Some(explicit_choice_count);
        }
    }
    if let Some(captures) = sectioned_leading_word_count_pattern().captures(&normalized_line) {
        if let Some(parenthetical_count) = captures
            .get(2)
            .and_then(|count| count.as_str().parse::<usize>().ok())
            .filter(|count| *count > 0)
        {
            return Some(parenthetical_count);
        }
        return captures.get(1).and_then(|word| {
            match word.as_str() {
                "one" => Some(1),
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
        });
    }
    if !sectioned_credit_word_pattern().is_match(&normalized_line) {
        if let Some(count) = sectioned_leading_numeric_electives_pattern()
            .captures(&normalized_line)
            .and_then(|captures| captures.get(1))
            .and_then(|count| count.as_str().parse::<usize>().ok())
            .filter(|count| *count > 0)
        {
            return Some(count);
        }
    }
    None
}

pub fn has_immediate_sectioned_course_rows_after_heading(
    snapshot_lines: &[String],
    start_index: usize,
    minimum_count: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let mut course_row_count = 0usize;
    for index in start_index + 1..snapshot_lines.len().min(start_index + 14) {
        let line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[index]),
        );
        if line.is_empty() || immediate_section_table_header_pattern().is_match(&line) {
            continue;
        }
        if is_sectioned_option_course_boundary(&line, course_parser)
            || looks_like_sectioned_course_heading_line(&line, course_parser)
        {
            break;
        }
        if get_sectioned_option_course_line(&line).is_some() {
            course_row_count += 1;
            if course_row_count >= minimum_count {
                return true;
            }
            continue;
        }
        if course_row_count == 0 {
            break;
        }
    }
    false
}

pub fn get_next_meaningful_section_line(snapshot_lines: &[String], index: usize) -> String {
    snapshot_lines
        .iter()
        .skip(index.saturating_add(1))
        .map(|candidate| {
            normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(candidate),
            )
        })
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_default()
}

pub fn build_split_either_or_requirement_groups(
    owner: &Value,
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let mut groups = Vec::new();
    let mut seen_group_ids = HashSet::new();
    for (index, raw_line) in snapshot_lines.iter().enumerate() {
        let normalized_line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(raw_line),
        );
        let line_course_codes = course_parser.extract_from_line(&normalized_line);
        if split_either_or_trailing_or_pattern().is_match(&normalized_line)
            && line_course_codes.len() == 1
            && source_line_starts_with_course_code(&normalized_line)
        {
            if let Some(next_course_line) =
                get_split_either_or_course_line(snapshot_lines, index, true, course_parser)
            {
                let first_line = split_either_or_trailing_or_pattern()
                    .replace(&normalized_line, "")
                    .trim()
                    .to_string();
                if let Some(group) = build_split_either_or_group(
                    &owner_id,
                    vec![(first_line, line_course_codes[0].clone()), next_course_line],
                    course_parser,
                    &mut seen_group_ids,
                ) {
                    groups.push(group);
                }
            }
            continue;
        }
        if !standalone_or_line_pattern().is_match(&normalized_line) {
            continue;
        }
        let previous_course_line =
            get_split_either_or_course_line(snapshot_lines, index, false, course_parser);
        let next_course_line =
            get_split_either_or_course_line(snapshot_lines, index, true, course_parser);
        if let (Some(previous), Some(next)) = (previous_course_line, next_course_line) {
            if let Some(group) = build_split_either_or_group(
                &owner_id,
                vec![previous, next],
                course_parser,
                &mut seen_group_ids,
            ) {
                groups.push(group);
            }
        }
    }
    groups
}

fn get_split_either_or_course_line(
    snapshot_lines: &[String],
    start_index: usize,
    next: bool,
    course_parser: &CourseCodeParser,
) -> Option<(String, String)> {
    for distance in 1..=3usize {
        let index = if next {
            start_index.checked_add(distance)?
        } else {
            start_index.checked_sub(distance)?
        };
        let Some(raw_line) = snapshot_lines.get(index) else {
            break;
        };
        let normalized_line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(raw_line),
        );
        if normalized_line.is_empty() {
            continue;
        }
        let course_codes = course_parser.extract_from_line(&normalized_line);
        if course_codes.len() == 1 && source_line_starts_with_course_code(&normalized_line) {
            return Some((normalized_line, course_codes[0].clone()));
        }
        if !course_codes.is_empty()
            || super::requirement_recovery_scope::has_primary_requirement_section_cue(
                &normalized_line,
            )
        {
            break;
        }
    }
    None
}

fn build_split_either_or_group(
    owner_id: &str,
    course_lines: Vec<(String, String)>,
    course_parser: &CourseCodeParser,
    seen_group_ids: &mut HashSet<String>,
) -> Option<Value> {
    let course_codes = unique_in_order(
        course_lines
            .iter()
            .map(|(_, course_code)| normalize_course_code(course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    if course_codes.len() != 2
        || course_codes.iter().any(|course_code| {
            super::requirement_course_codes::get_parsed_course_level(course_code)
                .is_none_or(|level| level >= 300)
        })
    {
        return None;
    }
    let source_heading = normalize_whitespace(
        &course_lines
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>()
            .join(" OR "),
    );
    let source_row_text = course_lines
        .first()
        .map(|(line, _)| normalize_whitespace(line))
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| source_heading.clone());
    let group_id = format!(
        "{owner_id}:requirement-group:{}",
        slugify_requirement_identifier(&course_codes.join("-or-"))
    );
    if !seen_group_ids.insert(group_id.clone()) {
        return None;
    }
    let options = course_codes
        .iter()
        .map(|course_code| {
            let metadata = course_parser.metadata_for(course_code);
            let credit_range = course_parser.credit_range_for(course_code);
            json!({
                "id": format!(
                    "{owner_id}:requirement-option:{}",
                    slugify_requirement_identifier(course_code)
                ),
                "uwCourses": [course_code],
                "label": course_code,
                "title": metadata.and_then(|value| value.get("title")).cloned().unwrap_or(Value::Null),
                "credits": credit_range.credits,
                "creditMin": credit_range.credit_min,
                "creditMax": credit_range.credit_max,
                "creditText": credit_range.credit_text,
            })
        })
        .collect::<Vec<_>>();
    let group = build_parsed_requirement_group(&json!({
        "id": group_id,
        "label": course_codes.join(" or "),
        "sourceHeading": source_heading,
        "sourceRowText": source_row_text,
        "detectedOptionCue": "or",
        "category": "source-choice",
        "requirementType": "choose_one",
        "minCourses": 1,
        "maxCourses": 1,
        "selectionCount": 1,
        "requiredCount": 1,
        "options": options,
    }));
    if array(group.get("options")).is_empty() {
        None
    } else {
        Some(group)
    }
}

pub fn build_parser_rule_requirement_replacements(
    owner: &Value,
    snapshot_lines: &[String],
) -> Vec<Value> {
    let source_url = owner
        .get("sourceUrl")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("primarySourceUrl").filter(|value| !value.is_null()));
    let Some(replacement) = super::requirement_rule_utils::detect_option_replacement(
        owner,
        snapshot_lines,
        source_url,
    ) else {
        return Vec::new();
    };
    vec![json!({
        "baseRequirementId": replacement.get("baseRequirementId").cloned().unwrap_or(Value::Null),
        "replacedByRequirementId": replacement.get("replacedByRequirementId").cloned().unwrap_or(Value::Null),
        "appliesWhen": replacement.get("appliesWhen").cloned().unwrap_or(Value::Null),
        "replacementReason": replacement.get("replacementReason").cloned().unwrap_or(Value::Null),
        "sourceUrl": replacement.get("sourceUrl").cloned().unwrap_or(Value::Null),
        "sourceHeading": replacement.get("sourceHeading").cloned().unwrap_or(Value::Null),
    })]
}

pub fn build_sectioned_option_requirement_groups(
    owner: &Value,
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let source_url = owner
        .get("sourceUrl")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("primarySourceUrl").filter(|value| !value.is_null()));
    let Some(replacement) = super::requirement_rule_utils::detect_option_replacement(
        owner,
        snapshot_lines,
        source_url,
    ) else {
        return Vec::new();
    };
    let Some(option_acronym) = replacement.get("optionAcronym").and_then(Value::as_str) else {
        return Vec::new();
    };
    let option_pattern = regex::Regex::new(&format!(r"(?i)\b{}\b", regex::escape(option_acronym)))
        .expect("escaped option acronym pattern");
    let core_heading_pattern = core_heading_pattern();
    let elective_heading_pattern = elective_heading_pattern();
    let credit_heading_pattern = credit_heading_pattern();
    let core_heading_index = snapshot_lines.iter().position(|line| {
        let normalized = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(line),
        );
        option_pattern.is_match(&normalized)
            && core_heading_pattern.is_match(&normalized)
            && credit_heading_pattern.is_match(&normalized)
    });
    let Some(core_heading_index) = core_heading_index else {
        return Vec::new();
    };
    let elective_heading_index = snapshot_lines
        .iter()
        .enumerate()
        .skip(core_heading_index + 1)
        .find_map(|(index, line)| {
            let normalized = normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            );
            (option_pattern.is_match(&normalized)
                && elective_heading_pattern.is_match(&normalized)
                && credit_heading_pattern.is_match(&normalized))
            .then_some(index)
        });
    let Some(elective_heading_index) = elective_heading_index else {
        return Vec::new();
    };
    let option_section_end_index = snapshot_lines
        .iter()
        .enumerate()
        .skip(elective_heading_index + 1)
        .find_map(|(index, line)| {
            is_sectioned_option_course_boundary(line, course_parser).then_some(index)
        })
        .unwrap_or(snapshot_lines.len());
    let replacement_credits = replacement
        .get("replacementCredits")
        .cloned()
        .unwrap_or(Value::Null);
    let replacement_credit_text = js_string_value(&replacement_credits);
    let source_heading = format!(
        "{option_acronym} Option Core/Elective Requirement: {replacement_credit_text} credits"
    );
    let core_heading = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[core_heading_index]),
    );
    let elective_heading = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[elective_heading_index]),
    );
    let core_credits = parse_section_heading_credit_range(&core_heading).unwrap_or(Value::Null);
    let elective_credits =
        parse_section_heading_credit_range(&elective_heading).unwrap_or(Value::Null);
    let option_prefix = option_acronym.to_lowercase();
    let mut options = collect_sectioned_option_course_options(
        owner,
        snapshot_lines,
        core_heading_index,
        elective_heading_index,
        &json!({
            "category": format!("{option_prefix}_core_required"),
            "sectionHeading": core_heading,
            "sourceHeading": source_heading,
            "creditRange": core_credits,
        }),
        course_parser,
    );
    options.extend(collect_sectioned_option_course_options(
        owner,
        snapshot_lines,
        elective_heading_index,
        option_section_end_index,
        &json!({
            "category": format!("{option_prefix}_elective_option"),
            "sectionHeading": elective_heading,
            "sourceHeading": source_heading,
            "creditRange": elective_credits,
        }),
        course_parser,
    ));
    let evidence = string_values(replacement.get("rule").and_then(|rule| rule.get("evidence")));
    let source_row_text = if evidence.is_empty() {
        source_heading.clone()
    } else {
        evidence.join(" ")
    };
    let mut notes = vec![
        replacement
            .get("replacementReason")
            .map(js_string_value)
            .unwrap_or_default(),
        "Replacement and option-course choices parsed from source headings and course rows.".into(),
    ];
    notes.extend(evidence);
    let category = format!("{option_prefix}_core_elective");
    let group = build_parsed_requirement_group(&json!({
        "id": replacement.get("replacedByRequirementId").cloned().unwrap_or(Value::Null),
        "label": source_heading,
        "category": category,
        "subcategory": format!("{option_prefix}_core_elective_{}_credits", replacement_credit_text),
        "requirementType": "choose_credits",
        "minCredits": replacement_credits,
        "sourceHeading": source_heading,
        "sourceRowText": source_row_text,
        "detectedOptionCue": "option replacement",
        "notes": unique_sorted(notes),
        "options": options,
    }));
    if array(group.get("options")).is_empty() {
        Vec::new()
    } else {
        vec![group]
    }
}

pub fn build_internal_choice_split_sectioned_course_requirement_groups(
    owner: &Value,
    snapshot_lines: &[String],
    start_index: usize,
    end_index: usize,
    descriptor: &Value,
    source_heading: &str,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let mut choice_cue_index = None;
    let mut course_rows_before_choice = 0usize;
    for index in start_index + 1..end_index.min(snapshot_lines.len()) {
        let line = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[index]),
        );
        if line.is_empty() || parse_leading_section_heading_credit_range(&line).is_some() {
            continue;
        }
        if is_standalone_section_choice_cue_line(&line, course_parser) {
            if course_rows_before_choice > 0 {
                choice_cue_index = Some(index);
            }
            break;
        }
        if get_sectioned_option_course_line(&line).is_some() {
            course_rows_before_choice += 1;
        }
    }
    let Some(choice_cue_index) = choice_cue_index else {
        return Vec::new();
    };
    let category = nullish_value(descriptor.get("category"));
    let category_text = js_string_value(&category);
    let descriptor_label = descriptor
        .get("label")
        .map(js_string_value)
        .unwrap_or_default();
    let required_options = collect_sectioned_option_course_options(
        owner,
        snapshot_lines,
        start_index,
        choice_cue_index,
        &json!({
            "category": category,
            "sectionHeading": format!("{descriptor_label} required courses"),
            "sourceHeading": source_heading,
        }),
        course_parser,
    );
    let choice_options = collect_sectioned_option_course_options(
        owner,
        snapshot_lines,
        choice_cue_index,
        end_index,
        &json!({
            "category": category_text,
            "sectionHeading": format!("{descriptor_label} choice"),
            "sourceHeading": source_heading,
        }),
        course_parser,
    );
    if required_options.is_empty() || choice_options.len() < 2 {
        return Vec::new();
    }
    let required_credits = sum_sectioned_option_credits(&required_options);
    let descriptor_min_credits = descriptor.get("minCredits").and_then(Value::as_f64);
    let choice_credits = descriptor_min_credits
        .zip(required_credits)
        .map(|(minimum, required)| minimum - required)
        .filter(|credits| credits.is_finite());
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let required_course_codes = flatten_option_course_codes(&required_options, "uwCourses");
    let choice_course_codes = flatten_option_course_codes(&choice_options, "uwCourses");
    let required_group = build_parsed_requirement_group(&json!({
        "id": format!(
            "{owner_id}:requirement-group:{}",
            slugify_requirement_identifier(&format!(
                "{descriptor_label}-required-courses-{}",
                required_course_codes.join("-")
            ))
        ),
        "label": format!("{descriptor_label} required courses"),
        "category": category_text,
        "subcategory": category_text,
        "requirementType": "all_required",
        "minCourses": required_options.len(),
        "maxCourses": required_options.len(),
        "minCredits": required_credits,
        "maxCredits": required_credits,
        "creditText": required_credits.map(js_number_string),
        "sourceHeading": source_heading,
        "sourceRowText": source_heading,
        "detectedOptionCue": "sectioned required rows before explicit choice cue",
        "notes": ["Parsed from course rows before an explicit source choice cue."],
        "options": required_options,
    }));
    let choice_requirement_type = if choice_credits.is_some_and(|credits| credits > 0.0) {
        "choose_credits"
    } else {
        "choose_one"
    };
    let choice_credit_value = choice_credits
        .filter(|credits| *credits > 0.0)
        .map(|credits| json!(credits))
        .unwrap_or(Value::Null);
    let choice_group = build_parsed_requirement_group(&json!({
        "id": format!(
            "{owner_id}:requirement-group:{}",
            slugify_requirement_identifier(&format!(
                "{descriptor_label}-choice-{}",
                choice_course_codes.join("-or-")
            ))
        ),
        "label": format!("{descriptor_label} choice"),
        "category": category_text,
        "subcategory": category_text,
        "requirementType": choice_requirement_type,
        "minCourses": 1,
        "maxCourses": 1,
        "selectionCount": 1,
        "requiredCount": 1,
        "minCredits": choice_credit_value,
        "maxCredits": choice_credit_value,
        "creditText": choice_credits
            .filter(|credits| *credits > 0.0)
            .map(js_number_string),
        "sourceHeading": source_heading,
        "sourceRowText": normalize_whitespace(&snapshot_lines[choice_cue_index]),
        "detectedOptionCue": "sectioned explicit choice cue",
        "notes": ["Parsed from an explicit source choice cue inside a sectioned requirement."],
        "options": choice_options,
    }));
    vec![required_group, choice_group]
}

pub fn build_pathway_section_fallback_concentration_group(
    owner: &Value,
    snapshot_lines: &[String],
    existing_groups: &[Value],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    if !js_truthy(owner.get("pathwayId")) {
        return None;
    }
    let label = get_selected_pathway_display_label(owner, primary_major_titles)?;
    let normalized_label = super::requirement_recovery_scope::normalize_matcher_text(&label);
    if existing_groups.iter().any(|group| {
        let group_label = group
            .get("label")
            .filter(|value| !value.is_null())
            .map(js_string_value)
            .unwrap_or_default();
        super::requirement_recovery_scope::normalize_matcher_text(&group_label)
            == normalized_label
    }) {
        return None;
    }
    let selected_course_lines = collect_selected_pathway_section_course_lines(
        owner,
        snapshot_lines,
        primary_major_titles,
        course_parser,
    );
    if selected_course_lines.is_empty() {
        return None;
    }
    let concentration_credit_line = snapshot_lines.iter().find(|line| {
        concentration_credit_line_pattern().is_match(
            &normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            ),
        )
    })?;
    let concentration_credits = parse_requirement_credit_amount(concentration_credit_line)?;
    if !concentration_credits.is_finite() || concentration_credits <= 0.0 {
        return None;
    }
    let concentration_course_count =
        parse_requirement_course_count(concentration_credit_line).filter(|count| *count > 0);
    let category = build_sectioned_course_category(&label);
    let mut options = Vec::new();
    for selected_line in selected_course_lines {
        let line = selected_line
            .get("line")
            .map(js_string_value)
            .unwrap_or_default();
        let course_line = selected_line
            .get("courseLine")
            .map(js_string_value)
            .unwrap_or_default();
        let index = selected_line
            .get("index")
            .and_then(Value::as_u64)
            .map(|index| index as usize)
            .unwrap_or_default();
        let course_codes =
            extract_option_course_codes_from_sectioned_line(&course_line, course_parser);
        if course_codes.is_empty() {
            continue;
        }
        let option_course_lines = if should_split_sectioned_course_line_into_separate_options(
            &course_line,
            &course_codes,
        ) {
            course_codes
        } else {
            vec![course_line]
        };
        for option_course_line in option_course_lines {
            let mut option_input = Map::new();
            option_input.insert("courseLine".into(), json!(option_course_line));
            option_input.insert(
                "title".into(),
                get_nearby_section_course_title(snapshot_lines, index, course_parser)
                    .map(|title| json!(title))
                    .unwrap_or(Value::Null),
            );
            option_input.insert(
                "creditRange".into(),
                parse_requirement_credit_range(&line)
                    .map(|(minimum, maximum)| json!({
                        "minCredits": minimum,
                        "maxCredits": maximum,
                    }))
                    .unwrap_or(Value::Null),
            );
            option_input.insert("category".into(), json!(category));
            option_input.insert("sectionHeading".into(), json!(label));
            option_input.insert("sourceHeading".into(), json!(label));
            if let Some(option) = build_sectioned_option_course_option(
                owner,
                &Value::Object(option_input),
                course_parser,
            ) {
                options.push(option);
            }
        }
    }
    if options.is_empty() {
        return None;
    }
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let credits = js_number_string(concentration_credits);
    let group_id = format!(
        "{owner_id}:requirement-group:{}",
        slugify_requirement_identifier(&format!(
            "{label}-choose_credits-{credits}-{credits}"
        ))
    );
    let course_count = concentration_course_count
        .map(|count| json!(count))
        .unwrap_or(Value::Null);
    Some(build_parsed_requirement_group(&json!({
        "id": group_id,
        "label": label,
        "category": category,
        "subcategory": category,
        "requirementType": "choose_credits",
        "minCourses": course_count,
        "maxCourses": course_count,
        "minCredits": concentration_credits,
        "maxCredits": concentration_credits,
        "creditText": credits,
        "sourceHeading": label,
        "sourceRowText": label,
        "detectedOptionCue": "pathway-scoped subject concentration course list",
        "notes": ["Parsed from the selected pathway subject rows in the official concentration list."],
        "options": options,
    })))
}

pub fn build_standalone_course_choice_requirement_groups(
    owner: &Value,
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let mut groups = Vec::new();
    let mut seen_group_ids = HashSet::new();
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    for (index, snapshot_line) in snapshot_lines.iter().enumerate() {
        let source_row_text = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(snapshot_line),
        );
        if source_row_text.is_empty()
            || !course_parser.extract_from_line(&source_row_text).is_empty()
            || !standalone_course_count_choice_pattern().is_match(&source_row_text)
        {
            continue;
        }
        let required_count = parse_choice_required_count(&source_row_text);
        if required_count < 1 {
            continue;
        }
        let category = build_sectioned_course_category(&source_row_text);
        let mut options = Vec::new();
        for option_index in index + 1..snapshot_lines.len().min(index + 42) {
            let option_line = normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(
                    &snapshot_lines[option_index],
                ),
            );
            if option_line.is_empty() {
                continue;
            }
            let mut option_input = Map::new();
            option_input.insert("courseLine".into(), json!(option_line));
            option_input.insert(
                "title".into(),
                get_nearby_section_course_title(snapshot_lines, option_index, course_parser)
                    .map(|title| json!(title))
                    .unwrap_or(Value::Null),
            );
            let credit_range = parse_requirement_credit_range(&option_line).map(
                |(minimum, maximum)| {
                    json!({
                        "minCredits": minimum,
                        "maxCredits": maximum,
                    })
                },
            );
            option_input.insert(
                "creditRange".into(),
                credit_range.unwrap_or(Value::Null),
            );
            option_input.insert("category".into(), json!(category));
            option_input.insert("sectionHeading".into(), json!(source_row_text));
            option_input.insert("sourceHeading".into(), json!(source_row_text));
            if let Some(option) =
                build_sectioned_option_course_option(owner, &Value::Object(option_input), course_parser)
            {
                options.push(option);
                continue;
            }
            if !options.is_empty()
                && (is_standalone_section_choice_cue_line(&option_line, course_parser)
                    || is_sectioned_option_course_boundary(&option_line, course_parser)
                    || looks_like_sectioned_course_heading_line(&option_line, course_parser)
                    || super::requirement_recovery_scope::is_likely_peer_pathway_html_section_start_line(
                        &option_line,
                        course_parser,
                    ))
            {
                break;
            }
        }
        let mut seen_option_ids = HashSet::new();
        options.retain(|option| {
            let id = option.get("id").map(js_string_value).unwrap_or_default();
            seen_option_ids.insert(id)
        });
        if options.len() <= required_count {
            continue;
        }
        let course_codes = options
            .iter()
            .flat_map(|option| array(option.get("uwCourses")).iter().map(js_string_value))
            .collect::<Vec<_>>();
        let group_id = format!(
            "{owner_id}:requirement-group:{}",
            super::requirement_rule_utils::slugify_parser_rule_value(&format!(
                "{source_row_text}-{required_count}-{}",
                course_codes.join("-")
            ))
        );
        if !seen_group_ids.insert(group_id.clone()) {
            continue;
        }
        let mut group_input = Map::new();
        group_input.insert("id".into(), json!(group_id));
        group_input.insert("label".into(), json!(source_row_text));
        group_input.insert("category".into(), json!(category));
        group_input.insert("subcategory".into(), json!(category));
        group_input.insert(
            "requirementType".into(),
            json!(if required_count == 1 { "choose_one" } else { "choose_n" }),
        );
        group_input.insert("minCourses".into(), json!(required_count));
        group_input.insert("maxCourses".into(), json!(required_count));
        group_input.insert("selectionCount".into(), json!(required_count));
        group_input.insert("requiredCount".into(), json!(required_count));
        group_input.insert("sourceHeading".into(), json!(source_row_text));
        group_input.insert("sourceRowText".into(), json!(source_row_text));
        group_input.insert(
            "detectedOptionCue".into(),
            json!("standalone course-count choice list"),
        );
        group_input.insert(
            "notes".into(),
            json!(["Parsed from an explicit course-count choice cue and its following course rows."]),
        );
        group_input.insert("options".into(), json!(options));
        let group = build_parsed_requirement_group(&Value::Object(group_input));
        if !array(group.get("options")).is_empty() {
            groups.push(group);
        }
    }
    groups
}

pub fn split_sequence_alternative_labels(value: &str) -> Vec<String> {
    let normalized = normalize_whitespace(value);
    let labels = inline_or_cue_pattern()
        .split(&normalized)
        .map(|label| {
            let label = sequence_alternative_leading_marker_pattern().replace(label, "");
            let label = sequence_words_pattern().replace_all(&label, "");
            let label = sequence_subject_suffix_pattern().replace(&label, "");
            let label = sequence_label_trailing_noise_pattern().replace(&label, "");
            normalize_whitespace(&sequence_label_terminal_punctuation_pattern().replace(&label, ""))
        })
        .filter(|label| !label.is_empty())
        .filter(|label| sequence_allowed_alternative_label_pattern().is_match(label))
        .map(|label| get_sequence_path_label_from_text(&label, &[], None))
        .collect::<Vec<_>>();
    unique_in_order(labels)
}

pub fn get_simple_sequence_fallback_subject_from_context(
    snapshot_lines: &[String],
    index: usize,
) -> Option<String> {
    for row_index in (index.saturating_sub(8)..=index).rev() {
        let line = normalize_whitespace(
            snapshot_lines.get(row_index).map(String::as_str).unwrap_or_default(),
        );
        if let Some(captures) = trailing_parenthetical_subject_pattern().captures(&line) {
            if let Some(subject) = captures.get(1) {
                return Some(subject.as_str().to_ascii_uppercase());
            }
        }
        if row_index == index {
            continue;
        }
        if fallback_physics_context_pattern().is_match(&line) {
            return Some("PHYS".to_string());
        }
        if fallback_chemistry_context_pattern().is_match(&line) {
            return Some("CHEM".to_string());
        }
        if fallback_math_context_pattern().is_match(&line) {
            return Some("MATH".to_string());
        }
        if fallback_biology_context_pattern().is_match(&line) {
            return Some("BIOL".to_string());
        }
    }
    None
}

pub fn get_expected_sequence_subjects_from_cue(
    value: &str,
    fallback_subject: Option<&str>,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let text = normalize_whitespace(value);
    let mut subjects = Vec::new();
    if sequence_physics_cue_pattern().is_match(&text) {
        subjects.push("PHYS".to_string());
    }
    if sequence_chemistry_cue_pattern().is_match(&text) {
        subjects.push("CHEM".to_string());
    }
    if sequence_math_cue_pattern().is_match(&text) {
        subjects.push("MATH".to_string());
        subjects.push("QSCI".to_string());
    }
    if sequence_biology_cue_pattern().is_match(&text) {
        subjects.push("BIOL".to_string());
    }
    if let Some(fallback_subject) = fallback_subject
        .and_then(|subject| course_parser.normalize_subject(subject))
    {
        subjects.push(fallback_subject);
    }
    unique_in_order(
        subjects
            .into_iter()
            .filter_map(|subject| course_parser.normalize_subject(&subject))
            .collect(),
    )
}

pub fn sequence_row_subjects_fit_cue(
    row_course_codes: &[String],
    expected_subjects: &[String],
) -> bool {
    if expected_subjects.is_empty() {
        return true;
    }
    let row_subjects = unique_in_order(
        row_course_codes
            .iter()
            .filter_map(|course_code| course_code_subject(course_code))
            .collect(),
    );
    !row_subjects.is_empty()
        && row_subjects
            .iter()
            .any(|subject| expected_subjects.contains(subject))
}

pub fn sequence_row_looks_like_single_path(
    row_line: &str,
    row_course_codes: &[String],
    expected_subjects: &[String],
) -> bool {
    if row_course_codes.len() < 2 {
        return false;
    }
    let row_subjects = unique_in_order(
        row_course_codes
            .iter()
            .filter_map(|course_code| course_code_subject(course_code))
            .collect(),
    );
    if row_subjects.len() > 1 && !sequence_choice_context_pattern().is_match(row_line) {
        return false;
    }
    sequence_row_subjects_fit_cue(row_course_codes, expected_subjects)
}

pub fn is_sequence_choice_boundary_line(
    line: &str,
    paths_started: bool,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line =
        super::requirement_choice_lists::normalize_sequence_source_line(line);
    if normalized_line.is_empty() {
        return paths_started;
    }
    if sequence_or_only_line_pattern().is_match(&normalized_line) {
        return false;
    }
    if sequence_heading_boundary_pattern().is_match(&normalized_line)
        && !sequence_choice_context_pattern().is_match(&normalized_line)
    {
        return true;
    }
    paths_started
        && course_parser.extract_from_line(&normalized_line).is_empty()
        && super::requirement_recovery_scope::has_primary_requirement_section_cue(
            &normalized_line,
        )
}

fn get_lab_note_fallback_subject(
    text: &str,
    fallback_subject: Option<&str>,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    let normalized_text = normalize_whitespace(text);
    let explicit_subjects = unique_in_order(
        course_parser
            .extract_from_line(&normalized_text)
            .iter()
            .filter_map(|course_code| course_code_subject(course_code))
            .collect(),
    );
    if explicit_subjects.len() == 1 {
        return explicit_subjects.into_iter().next();
    }
    if lab_physics_cue_pattern().is_match(&normalized_text) {
        return Some("PHYS".to_string());
    }
    if lab_chemistry_cue_pattern().is_match(&normalized_text) {
        return Some("CHEM".to_string());
    }
    if lab_biology_cue_pattern().is_match(&normalized_text) {
        return Some("BIOL".to_string());
    }
    if lab_math_cue_pattern().is_match(&normalized_text) {
        return Some("MATH".to_string());
    }
    fallback_subject.map(str::to_string)
}

pub fn collect_sequence_lab_notes(
    snapshot_lines: &[String],
    start_index: usize,
    fallback_subject: Option<&str>,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let lines = snapshot_lines
        .iter()
        .skip(start_index)
        .take(24)
        .map(|line| normalize_whitespace(line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut lab_lines = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !sequence_lab_word_pattern().is_match(line) {
            continue;
        }
        lab_lines.push(line.clone());
        for continuation_index in index + 1..lines.len().min(index + 3) {
            let continuation_line = &lines[continuation_index];
            let combined = normalize_whitespace(
                &format!("{} {}", lab_lines.join(" "), continuation_line),
            );
            let continuation_fallback_subject = get_lab_note_fallback_subject(
                &combined,
                fallback_subject,
                course_parser,
            );
            let continuation_lab_courses = extract_course_codes_from_sequence_text(
                continuation_line,
                continuation_fallback_subject.as_deref(),
                course_parser,
            )
            .into_iter()
            .filter(|course_code| conditional_lab_course_pattern().is_match(course_code))
            .collect::<Vec<_>>();
            if continuation_lab_courses.is_empty() {
                break;
            }
            lab_lines.push(continuation_line.clone());
        }
    }
    let lab_lines = unique_in_order(lab_lines);
    if lab_lines.is_empty() {
        return None;
    }
    let lab_text = lab_lines.join(" ");
    let lab_fallback_subject =
        get_lab_note_fallback_subject(&lab_text, fallback_subject, course_parser);
    let conditional_lab_courses = extract_course_codes_from_sequence_text(
        &lab_text,
        lab_fallback_subject.as_deref(),
        course_parser,
    )
    .into_iter()
    .filter(|course_code| conditional_lab_course_pattern().is_match(course_code))
    .collect::<Vec<_>>();
    Some(json!({
        "text": normalize_whitespace(&lab_text),
        "conditionalLabCourses": conditional_lab_courses,
        "appliesToAlgebraPath": algebra_based_cue_pattern().is_match(&lines.join(" ")),
    }))
}

pub fn apply_sequence_lab_notes(paths: &[Value], lab_notes: Option<&Value>) -> Vec<Value> {
    let Some(lab_notes) = lab_notes.filter(|notes| {
        notes.get("text").is_some_and(|text| !js_string_nullish(Some(text)).is_empty())
    }) else {
        return paths.to_vec();
    };
    paths
        .iter()
        .map(|path| {
            let applies_to_path = lab_notes
                .get("appliesToAlgebraPath")
                .and_then(Value::as_bool)
                == Some(true)
                && algebra_based_cue_pattern().is_match(&js_string_nullish(path.get("label")));
            if !applies_to_path {
                return path.clone();
            }
            let mut updated = path.clone();
            if let Some(object) = updated.as_object_mut() {
                let mut conditional_lab_courses = array(path.get("conditionalLabCourses"))
                    .iter()
                    .map(js_string_value)
                    .collect::<Vec<_>>();
                conditional_lab_courses.extend(
                    array(lab_notes.get("conditionalLabCourses"))
                        .iter()
                        .map(js_string_value),
                );
                object.insert(
                    "conditionalLabCourses".into(),
                    json!(unique_in_order(conditional_lab_courses)),
                );
                let mut notes = array(path.get("notes"))
                    .iter()
                    .map(js_string_value)
                    .collect::<Vec<_>>();
                notes.push(format!(
                    "Conditional lab note: {}",
                    js_string_nullish(lab_notes.get("text"))
                ));
                object.insert("notes".into(), json!(unique_sorted(notes)));
            }
            updated
        })
        .collect()
}

pub fn get_sequence_fallback_subject_from_context(
    snapshot_lines: &[String],
    index: usize,
    current_line: &str,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    let mut context_lines = vec![(current_line, true)];
    let context_start = index.saturating_sub(6);
    for candidate_index in (context_start..index.min(snapshot_lines.len())).rev() {
        if let Some(line) = snapshot_lines.get(candidate_index) {
            context_lines.push((line.as_str(), false));
        }
    }
    for (raw_line, is_current) in context_lines {
        let line = normalize_whitespace(raw_line);
        let normalized_sequence_line =
            super::requirement_choice_lists::normalize_sequence_source_line(&line);
        if sequence_alternative_choice_pattern().is_match(&normalized_sequence_line) {
            let course_codes = course_parser.extract_from_line(&normalized_sequence_line);
            let mut subjects = Vec::new();
            for course_code in &course_codes {
                if let Some(subject) = course_code_subject(course_code) {
                    if !subjects.contains(&subject) {
                        subjects.push(subject);
                    }
                }
            }
            if subjects.len() == 1 {
                return Some(subjects[0].clone());
            }
            if sequence_bare_number_continuation_pattern().is_match(&normalized_sequence_line) {
                if let Some(first_subject) = course_codes
                    .first()
                    .and_then(|course_code| course_code_subject(course_code))
                {
                    return Some(first_subject);
                }
            }
        }
        if let Some(captures) = trailing_parenthetical_subject_pattern().captures(&line) {
            if let Some(subject) = captures
                .get(1)
                .and_then(|value| course_parser.normalize_subject(value.as_str()))
            {
                return Some(subject);
            }
        }
        if let Some(captures) = named_sequence_heading_subject_pattern().captures(&line) {
            if let Some(subject) = captures
                .get(1)
                .and_then(|value| course_parser.normalize_subject(value.as_str()))
            {
                return Some(subject);
            }
        }
        if !is_current {
            if fallback_physics_context_pattern().is_match(&line) {
                return Some("PHYS".to_string());
            }
            if fallback_chemistry_context_pattern().is_match(&line) {
                return Some("CHEM".to_string());
            }
            if fallback_math_context_pattern().is_match(&line) {
                return Some("MATH".to_string());
            }
            if fallback_biology_context_pattern().is_match(&line) {
                return Some("BIOL".to_string());
            }
        }
    }
    None
}

fn course_code_subject(course_code: &str) -> Option<String> {
    normalize_course_code(course_code)
        .rsplit_once(' ')
        .map(|(subject, _)| subject.to_string())
        .filter(|subject| !subject.is_empty())
}

pub fn build_parenthetical_sequence_choice_candidates(
    snapshot_lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let cue_line = super::requirement_choice_lists::normalize_sequence_source_line(
        snapshot_lines.get(index).map(String::as_str).unwrap_or_default(),
    );
    if !parenthetical_sequence_choice_cue_pattern().is_match(&cue_line) {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for captures in parenthetical_sequence_candidate_pattern().captures_iter(&cue_line) {
        let (Some(left_match), Some(right_match)) = (captures.get(1), captures.get(2)) else {
            continue;
        };
        let left_text = normalize_whitespace(left_match.as_str());
        let right_text = normalize_whitespace(right_match.as_str());
        let left_courses = extract_course_codes_from_sequence_text(&left_text, None, course_parser);
        let fallback_subject = left_courses.first().and_then(|course_code| course_code_subject(course_code));
        let right_courses = extract_course_codes_from_sequence_text(
            &right_text,
            fallback_subject.as_deref(),
            course_parser,
        );
        if left_courses.len() < 2
            || right_courses.len() < 2
            || course_code_subject(&left_courses[0]) != course_code_subject(&right_courses[0])
        {
            continue;
        }
        let raw_text = normalize_whitespace(&format!("{left_text} or {right_text}"));
        candidates.push(json!({
            "rawText": raw_text,
            "sourceSection": cue_line,
            "detectedOptionCue": "parenthetical sequence alternative",
            "paths": [
                {
                    "label": get_sequence_path_label_from_text(&left_text, &left_courses, None),
                    "uwCourses": left_courses,
                    "sourceText": left_text,
                },
                {
                    "label": get_sequence_path_label_from_text(&right_text, &right_courses, None),
                    "uwCourses": right_courses,
                    "sourceText": right_text,
                },
            ],
        }));
    }
    candidates
}

pub fn build_parenthetical_sequence_choice_requirement_groups(
    owner: &Value,
    parsed_course_codes: &[String],
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let parsed_course_code_set = parsed_course_codes
        .iter()
        .map(|course_code| normalize_course_code(course_code))
        .collect::<HashSet<_>>();
    let mut seen_group_ids = HashSet::new();
    let mut groups = Vec::new();
    for index in 0..snapshot_lines.len() {
        if super::requirement_choice_lists::is_lettered_option_line_under_choose_one_heading(
            snapshot_lines,
            index,
        ) {
            continue;
        }
        for candidate in
            build_parenthetical_sequence_choice_candidates(snapshot_lines, index, course_parser)
        {
            let raw_text = js_string_nullish(candidate.get("rawText"));
            if sequence_restriction_context_pattern().is_match(&raw_text) {
                continue;
            }
            let strong_sequence_cue = sequence_choice_context_pattern().is_match(&raw_text)
                || sequence_label_alternative_pattern().is_match(&raw_text)
                || sequence_alternative_choice_pattern().is_match(&raw_text);
            let mut filtered_candidate = candidate.clone();
            if let Some(paths) = filtered_candidate
                .get_mut("paths")
                .and_then(Value::as_array_mut)
            {
                paths.retain_mut(|path| {
                    if let Some(course_codes) = path
                        .get_mut("uwCourses")
                        .and_then(Value::as_array_mut)
                    {
                        course_codes.retain(|course_code| {
                            let normalized = normalize_course_code(&js_string_value(course_code));
                            strong_sequence_cue || parsed_course_code_set.contains(&normalized)
                        });
                        course_codes.len() >= 2
                    } else {
                        false
                    }
                });
            }
            let Some(group) = build_sequence_choice_requirement_group(
                owner,
                &filtered_candidate,
                course_parser,
            ) else {
                continue;
            };
            let group_id = group
                .get("id")
                .map(js_string_value)
                .unwrap_or_default();
            if seen_group_ids.insert(group_id) {
                groups.push(group);
            }
        }
    }
    groups
}

fn extract_course_codes_from_sequence_text(
    value: &str,
    fallback_subject: Option<&str>,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let text = super::requirement_choice_lists::normalize_sequence_source_line(value);
    let row = super::sequence_text::SequenceCourseCodeRow {
        text: text.clone(),
        fallback_subject: fallback_subject.map(str::to_string),
        explicit_course_codes: unique_in_order(
            course_parser
                .extract_from_line(&text)
                .into_iter()
                .map(|course_code| normalize_course_code(&course_code))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        ),
    };
    super::sequence_text::expand_sequence_course_codes(&[row])
        .into_iter()
        .next()
        .unwrap_or_default()
}

pub fn build_alternating_number_sequence_choice_candidate(
    snapshot_lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let cue_line = super::requirement_choice_lists::normalize_sequence_source_line(
        snapshot_lines.get(index).map(String::as_str).unwrap_or_default(),
    );
    if !sequence_label_alternative_pattern().is_match(&cue_line) {
        return None;
    }
    let labels = split_sequence_alternative_labels(&cue_line);
    let fallback_subject = get_simple_sequence_fallback_subject_from_context(snapshot_lines, index)?;
    if labels.len() != 2 {
        return None;
    }
    let expected_subjects = get_expected_sequence_subjects_from_cue(
        &cue_line,
        Some(&fallback_subject),
        course_parser,
    );
    let mut paths = labels
        .into_iter()
        .map(|label| json!({
            "label": label,
            "uwCourses": [],
            "sourceText": cue_line,
        }))
        .collect::<Vec<_>>();
    let mut source_lines = vec![cue_line.clone()];
    let mut alternating_index = 0usize;
    let end_index = snapshot_lines.len().min(index.saturating_add(16));
    for row_index in index.saturating_add(1)..end_index {
        let row_line = super::requirement_choice_lists::normalize_sequence_source_line(
            &snapshot_lines[row_index],
        );
        if is_sequence_choice_boundary_line(
            &row_line,
            paths
                .iter()
                .any(|path| !array(path.get("uwCourses")).is_empty()),
            course_parser,
        ) {
            break;
        }
        let row_course_codes = bare_sequence_course_number_pattern()
            .captures_iter(&row_line)
            .filter_map(|captures| captures.get(1))
            .filter_map(|number| course_parser.normalize_code(&fallback_subject, number.as_str()))
            .filter(|course_code| {
                super::requirement_course_codes::get_parsed_course_level(course_code)
                    .is_some_and(|level| level < 300)
            })
            .collect::<Vec<_>>();
        if row_course_codes.is_empty() {
            continue;
        }
        if row_course_codes.len() == paths.len() {
            for (path, course_code) in paths.iter_mut().zip(row_course_codes) {
                let mut courses = array(path.get("uwCourses"))
                    .iter()
                    .map(js_string_value)
                    .collect::<Vec<_>>();
                courses.push(course_code);
                let source_text = normalize_whitespace(&format!(
                    "{} {}",
                    js_string_nullish(path.get("sourceText")),
                    row_line
                ));
                if let Some(object) = path.as_object_mut() {
                    object.insert("uwCourses".into(), json!(courses));
                    object.insert("sourceText".into(), json!(source_text));
                }
            }
            source_lines.push(row_line);
        } else if row_course_codes.len() == 1 {
            let path_index = alternating_index % paths.len();
            alternating_index += 1;
            if let Some(path) = paths.get_mut(path_index) {
                let mut courses = array(path.get("uwCourses"))
                    .iter()
                    .map(js_string_value)
                    .collect::<Vec<_>>();
                courses.push(row_course_codes[0].clone());
                let source_text = normalize_whitespace(&format!(
                    "{} {}",
                    js_string_nullish(path.get("sourceText")),
                    row_line
                ));
                if let Some(object) = path.as_object_mut() {
                    object.insert("uwCourses".into(), json!(courses));
                    object.insert("sourceText".into(), json!(source_text));
                }
            }
            source_lines.push(row_line);
            if paths.iter().all(|path| array(path.get("uwCourses")).len() >= 3) {
                break;
            }
        }
    }
    let mut populated_paths = Vec::new();
    for mut path in paths {
        let courses = unique_in_order(
            array(path.get("uwCourses"))
                .iter()
                .map(js_string_value)
                .collect(),
        );
        if courses.len() < 2 {
            continue;
        }
        if let Some(object) = path.as_object_mut() {
            object.insert("uwCourses".into(), json!(courses));
        }
        populated_paths.push(path);
    }
    if populated_paths.len() < 2 {
        return None;
    }
    let lab_notes = collect_sequence_lab_notes(
        snapshot_lines,
        index.saturating_add(1),
        Some(&fallback_subject),
        course_parser,
    );
    let paths = apply_sequence_lab_notes(&populated_paths, lab_notes.as_ref());
    let source_section = super::requirement_choice_lists::find_nearby_requirement_choice_label(
        snapshot_lines,
        index,
        course_parser,
    );
    let detected_option_cue = get_sequence_choice_cue(&cue_line);
    Some(json!({
        "rawText": normalize_whitespace(&source_lines.join(" ")),
        "sourceSection": source_section,
        "detectedOptionCue": detected_option_cue,
        "paths": paths,
    }))
}

pub fn build_ampersand_or_single_sequence_choice_candidate(
    snapshot_lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(
            snapshot_lines.get(index).map(String::as_str).unwrap_or_default(),
        ),
    );
    if line.is_empty() || !ampersand_or_single_sequence_pattern().is_match(&line) {
        return None;
    }
    let mut parts = ampersand_or_separator_pattern().split(&line);
    let left_text = normalize_whitespace(parts.next().unwrap_or_default());
    let right_text = normalize_whitespace(parts.next().unwrap_or_default());
    if left_text.is_empty() || right_text.is_empty() || !left_text.contains('&') {
        return None;
    }
    let left_courses = unique_in_order(
        course_parser
            .extract_from_requirement_line(&left_text)
            .into_iter()
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    let right_courses = unique_in_order(
        course_parser
            .extract_from_requirement_line(&right_text)
            .into_iter()
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    if left_courses.len() < 2 || right_courses.len() != 1 {
        return None;
    }
    Some(json!({
        "rawText": line,
        "detectedOptionCue": "sequence or single course",
        "paths": [
            {
                "label": left_courses.join(" + "),
                "uwCourses": left_courses,
                "sourceText": left_text,
            },
            {
                "label": right_courses[0],
                "uwCourses": right_courses,
                "sourceText": right_text,
            },
        ],
    }))
}

pub fn build_ampersand_or_single_sequence_choice_requirement_groups(
    owner: &Value,
    parsed_course_codes: &[String],
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let parsed_course_code_set = parsed_course_codes
        .iter()
        .map(|course_code| normalize_course_code(course_code))
        .collect::<HashSet<_>>();
    let mut seen_group_ids = HashSet::new();
    let mut groups = Vec::new();
    for index in 0..snapshot_lines.len() {
        if super::requirement_choice_lists::is_lettered_option_line_under_choose_one_heading(
            snapshot_lines,
            index,
        ) {
            continue;
        }
        let Some(candidate) = build_ampersand_or_single_sequence_choice_candidate(
            snapshot_lines,
            index,
            course_parser,
        ) else {
            continue;
        };
        let raw_text = js_string_nullish(candidate.get("rawText"));
        if sequence_restriction_context_pattern().is_match(&raw_text) {
            continue;
        }
        let strong_sequence_cue = sequence_choice_context_pattern().is_match(&raw_text)
            || sequence_label_alternative_pattern().is_match(&raw_text)
            || sequence_alternative_choice_pattern().is_match(&raw_text);
        let minimum_courses_per_path = if candidate
            .get("detectedOptionCue")
            .and_then(Value::as_str)
            == Some("sequence or single course")
        {
            1
        } else {
            2
        };
        let mut filtered_candidate = candidate.clone();
        if let Some(paths) = filtered_candidate
            .get_mut("paths")
            .and_then(Value::as_array_mut)
        {
            paths.retain_mut(|path| {
                if let Some(course_codes) = path
                    .get_mut("uwCourses")
                    .and_then(Value::as_array_mut)
                {
                    course_codes.retain(|course_code| {
                        let normalized = normalize_course_code(&js_string_value(course_code));
                        strong_sequence_cue || parsed_course_code_set.contains(&normalized)
                    });
                    course_codes.len() >= minimum_courses_per_path
                } else {
                    false
                }
            });
        }
        let Some(group) =
            build_sequence_choice_requirement_group(owner, &filtered_candidate, course_parser)
        else {
            continue;
        };
        let group_id = group
            .get("id")
            .map(js_string_value)
            .unwrap_or_default();
        if seen_group_ids.insert(group_id) {
            groups.push(group);
        }
    }
    groups
}

pub fn build_sequence_choice_requirement_group(
    owner: &Value,
    candidate: &Value,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let detected_option_cue = candidate
        .get("detectedOptionCue")
        .filter(|value| !value.is_null())
        .cloned();
    let minimum_courses_per_path = if detected_option_cue.as_ref().and_then(Value::as_str)
        == Some("sequence or single course")
    {
        1
    } else {
        2
    };
    let raw_text = js_string_nullish(candidate.get("rawText"));
    let mut seen_course_paths = HashSet::new();
    let mut normalized_paths = Vec::new();
    for path in array(candidate.get("paths")) {
        let uw_courses = unique_in_order(
            array(path.get("uwCourses"))
                .iter()
                .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        let label = normalized_string_or_fallback(
            path.get("label"),
            &get_sequence_path_label_from_text(
                &js_string_nullish(path.get("sourceText")),
                &uw_courses,
                None,
            ),
        );
        let conditional_lab_courses = unique_in_order(
            array(path.get("conditionalLabCourses"))
                .iter()
                .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        let notes = unique_sorted(
            array(path.get("notes"))
                .iter()
                .map(|note| normalize_whitespace(&js_string_nullish(Some(note))))
                .filter(|note| !note.is_empty())
                .collect(),
        );
        let source_text = path
            .get("sourceText")
            .filter(|value| !value.is_null())
            .map(js_string_value)
            .unwrap_or_else(|| raw_text.clone());
        if uw_courses.len() < minimum_courses_per_path
            || !seen_course_paths.insert(uw_courses.join("|"))
        {
            continue;
        }
        normalized_paths.push(json!({
            "label": label,
            "uwCourses": uw_courses,
            "conditionalLabCourses": conditional_lab_courses,
            "notes": notes,
            "sourceText": normalize_whitespace(&source_text),
        }));
    }
    if normalized_paths.len() < 2 {
        return None;
    }

    let all_path_courses = unique_in_order(
        normalized_paths
            .iter()
            .flat_map(|path| array(path.get("uwCourses")).iter().map(js_string_value))
            .collect(),
    );
    let source_heading = {
        let normalized = normalize_whitespace(&raw_text);
        if normalized.is_empty() {
            all_path_courses.join(" or ")
        } else {
            normalized
        }
    };
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let group_id = format!(
        "{owner_id}:requirement-group:sequence-choice-{}",
        slugify_requirement_identifier(&all_path_courses.join("-or-"))
    );
    let sequence_paths = normalized_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let path_label = path
                .get("label")
                .map(js_string_value)
                .unwrap_or_default();
            let path_courses = array(path.get("uwCourses"))
                .iter()
                .map(js_string_value)
                .collect::<Vec<_>>();
            let fallback_label = format!("sequence-{}", index + 1);
            let path_id_suffix = slugify_requirement_identifier(&format!(
                "{}-{}",
                if path_label.is_empty() { &fallback_label } else { &path_label },
                path_courses.join("-")
            ));
            let path_id_suffix = if path_id_suffix.is_empty() {
                fallback_label.clone()
            } else {
                path_id_suffix
            };
            let path_id = format!("{group_id}:path:{path_id_suffix}");
            let label = if path_label.is_empty() {
                format!("sequence path {}", index + 1)
            } else {
                path_label
            };
            let uw_courses = path.get("uwCourses").cloned().unwrap_or_else(|| json!([]));
            let conditional_lab_courses = path
                .get("conditionalLabCourses")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let notes = path.get("notes").cloned().unwrap_or_else(|| json!([]));
            let source_text = normalized_string_or_fallback(path.get("sourceText"), &source_heading);
            json!({
                "id": path_id,
                "label": label,
                "uwCourses": uw_courses,
                "displayCourseCodes": uw_courses,
                "conditionalLabCourses": conditional_lab_courses,
                "notes": notes,
                "sourceText": source_text,
            })
        })
        .collect::<Vec<_>>();
    let options = sequence_paths
        .iter()
        .map(|path| {
            let path_id = path.get("id").map(js_string_value).unwrap_or_default();
            json!({
                "id": format!("{owner_id}:requirement-option:{}", slugify_requirement_identifier(&path_id)),
                "sequencePathId": path_id,
                "pathLabel": path.get("label"),
                "uwCourses": path.get("uwCourses"),
                "displayCourseCodes": path.get("displayCourseCodes"),
                "conditionalLabCourses": path.get("conditionalLabCourses"),
                "notes": path.get("notes"),
                "label": path.get("label"),
            })
        })
        .collect::<Vec<_>>();

    let mut group_input = Map::new();
    group_input.insert("id".into(), json!(group_id));
    group_input.insert(
        "label".into(),
        json!(get_choice_requirement_group_label(
            &source_heading,
            &all_path_courses,
            course_parser,
        )),
    );
    group_input.insert("sourceHeading".into(), json!(source_heading));
    group_input.insert("sourceRowText".into(), json!(source_heading));
    group_input.insert(
        "sourceSection".into(),
        nullish_value(candidate.get("sourceSection")),
    );
    group_input.insert(
        "detectedOptionCue".into(),
        detected_option_cue.unwrap_or_else(|| {
            get_sequence_choice_cue(&source_heading)
                .map(|cue| json!(cue))
                .unwrap_or(Value::Null)
        }),
    );
    group_input.insert("category".into(), json!("source-sequence-choice"));
    group_input.insert("requirementType".into(), json!("sequence_choice"));
    group_input.insert("minCourses".into(), json!(1));
    group_input.insert("maxCourses".into(), json!(1));
    group_input.insert("selectionCount".into(), json!(1));
    group_input.insert("requiredCount".into(), json!(1));
    group_input.insert("sequencePaths".into(), json!(sequence_paths));
    group_input.insert("options".into(), json!(options));
    Some(build_parsed_requirement_group(&Value::Object(group_input)))
}

pub fn extract_inline_required_list_alternative_sets(
    text: &str,
    course_parser: &CourseCodeParser,
) -> Vec<Vec<String>> {
    let normalized_text = normalize_whitespace(text);
    let mut alternative_sets = Vec::new();
    for captures in parenthetical_course_alternative_pattern().captures_iter(&normalized_text) {
        let (Some(subject), Some(number), Some(alternatives)) =
            (captures.get(1), captures.get(2), captures.get(3))
        else {
            continue;
        };
        let leading_course_code = course_parser.normalize_code(subject.as_str(), number.as_str());
        let mut course_codes = Vec::new();
        if let Some(leading_course_code) = leading_course_code {
            course_codes.push(leading_course_code);
        }
        let alternative_source = format!(
            "{} {} or {}",
            subject.as_str(),
            number.as_str(),
            alternatives.as_str()
        );
        course_codes.extend(course_parser.extract_from_line(&alternative_source));
        let course_codes = unique_in_order(
            course_codes
                .into_iter()
                .map(|course_code| normalize_course_code(&course_code))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        if course_codes.len() >= 2 {
            alternative_sets.push(course_codes);
        }
    }
    for captures in inline_course_or_alternative_pattern().captures_iter(&normalized_text) {
        let (Some(left_subject), Some(left_number), Some(right_number)) =
            (captures.get(1), captures.get(2), captures.get(4))
        else {
            continue;
        };
        let right_subject = captures.get(3).unwrap_or(left_subject);
        let course_codes = unique_in_order(
            [
                course_parser.normalize_code(left_subject.as_str(), left_number.as_str()),
                course_parser.normalize_code(right_subject.as_str(), right_number.as_str()),
            ]
            .into_iter()
            .flatten()
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
        );
        if course_codes.len() == 2 {
            alternative_sets.push(course_codes);
        }
    }
    let mut unique_sets = Vec::<Vec<String>>::new();
    let mut seen_sets = HashSet::new();
    for course_codes in alternative_sets {
        let key = course_codes.join("|");
        if seen_sets.insert(key) {
            unique_sets.push(course_codes);
        }
    }
    unique_sets
        .iter()
        .enumerate()
        .filter(|(index, course_codes)| {
            !unique_sets.iter().enumerate().any(|(candidate_index, candidate)| {
                candidate_index != *index
                    && candidate.len() > course_codes.len()
                    && course_codes
                        .iter()
                        .all(|course_code| candidate.contains(course_code))
            })
        })
        .map(|(_, course_codes)| course_codes.clone())
        .collect()
}

pub fn get_repeated_required_alternative_sets(
    text: &str,
    course_parser: &CourseCodeParser,
) -> Vec<Vec<String>> {
    let normalized_text = normalize_whitespace(text);
    if super::requirement_choice_lists::get_inline_labeled_requirement_parts(
        &normalized_text,
        course_parser,
    )
    .is_some()
        || repeated_required_alternative_exclusion_pattern().is_match(&normalized_text)
        || (!repeated_required_and_pattern().is_match(&normalized_text)
            && !repeated_required_comma_or_pattern().is_match(&normalized_text))
    {
        return Vec::new();
    }
    let alternative_sets = extract_inline_required_list_alternative_sets(&normalized_text, course_parser);
    if alternative_sets.len() < 2 {
        return Vec::new();
    }
    let flattened = alternative_sets.iter().flatten().collect::<Vec<_>>();
    if flattened.iter().collect::<HashSet<_>>().len() != flattened.len() {
        return Vec::new();
    }
    alternative_sets
}

pub fn build_repeated_required_alternative_groups(
    owner: &Value,
    source_line: &str,
    option_extraction_line: &str,
    source_section: &str,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    get_repeated_required_alternative_sets(option_extraction_line, course_parser)
        .into_iter()
        .enumerate()
        .map(|(index, alternative_set)| {
            let group_id = format!(
                "{owner_id}:requirement-group:required-alternative-slot-{}-{}",
                index + 1,
                slugify_requirement_identifier(&alternative_set.join("-or-"))
            );
            build_parsed_requirement_group(&json!({
                "id": group_id,
                "label": alternative_set.join(" / "),
                "category": "course-alias",
                "subcategory": "required-alternative-slot",
                "requirementType": "all_required",
                "minCourses": 1,
                "maxCourses": 1,
                "sourceHeading": source_line,
                "sourceRowText": source_line,
                "sourceSection": source_section,
                "detectedOptionCue": "repeated required alternative slots",
                "notes": ["Each repeated source slot is required; the courses inside this slot are alternatives."],
                "options": [{
                    "id": format!(
                        "{group_id}:option:{}",
                        slugify_requirement_identifier(&alternative_set.join("-or-"))
                    ),
                    "uwCourses": [alternative_set[0]],
                    "equivalentUwCourseCodes": alternative_set.iter().skip(1).collect::<Vec<_>>(),
                    "label": alternative_set.join(" / "),
                }],
            }))
        })
        .filter(|group| !array(group.get("options")).is_empty())
        .collect()
}

pub fn get_mixed_required_course_list_parts(
    parts: &super::requirement_choice_lists::InlineLabeledRequirementParts,
    course_parser: &CourseCodeParser,
) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let source_line = normalize_whitespace(&parts.source_line);
    let label = normalize_whitespace(&parts.label);
    let body = normalize_whitespace(&parts.body);
    if source_line.is_empty()
        || body.is_empty()
        || !inline_mixed_body_or_pattern().is_match(&body)
        || inline_mixed_source_exclusion_pattern().is_match(&source_line)
        || inline_mixed_minimum_course_pattern().is_match(&source_line)
        || !inline_mixed_required_label_pattern().is_match(&label)
        || !inline_mixed_body_delimiter_pattern().is_match(&body)
    {
        return None;
    }
    let all_course_codes = course_parser
        .extract_from_requirement_line(&body)
        .into_iter()
        .map(|course_code| normalize_course_code(&course_code))
        .collect::<Vec<_>>();
    let alternative_sets = extract_inline_required_list_alternative_sets(&body, course_parser);
    if all_course_codes.len() < 4 || alternative_sets.is_empty() {
        return None;
    }
    let alternative_course_codes = alternative_sets
        .iter()
        .flatten()
        .cloned()
        .collect::<HashSet<_>>();
    let required_course_codes = unique_in_order(
        all_course_codes
            .into_iter()
            .filter(|course_code| !alternative_course_codes.contains(course_code))
            .collect(),
    );
    if required_course_codes.len() < 2 {
        return None;
    }
    Some((required_course_codes, alternative_sets))
}

pub fn build_mixed_inline_required_course_list_groups(
    owner: &Value,
    parts: &super::requirement_choice_lists::InlineLabeledRequirementParts,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let Some((required_course_codes, alternative_sets)) =
        get_mixed_required_course_list_parts(parts, course_parser)
    else {
        return Vec::new();
    };
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let source_line = normalize_whitespace(&parts.source_line);
    let label = normalize_whitespace(&parts.label);
    let category = build_sectioned_course_category(&label);
    let required_group_id = format!(
        "{owner_id}:requirement-group:inline-required-{}",
        slugify_requirement_identifier(&format!(
            "{label}-{}",
            required_course_codes.join("-")
        ))
    );
    let required_group = build_parsed_requirement_group(&json!({
        "id": required_group_id,
        "label": label,
        "category": category,
        "subcategory": category,
        "requirementType": "all_required",
        "minCourses": required_course_codes.len(),
        "maxCourses": required_course_codes.len(),
        "sourceHeading": source_line,
        "sourceRowText": source_line,
        "detectedOptionCue": "inline required course list with embedded alternatives",
        "notes": ["Parsed from an inline required course list; embedded alternatives are separate choice groups."],
        "options": build_inline_course_requirement_options(
            &required_group_id,
            &required_course_codes,
            &source_line,
            false,
            course_parser,
        ),
    }));
    let mut groups = vec![required_group];
    for alternative_set in alternative_sets {
        let choice_group_id = format!(
            "{owner_id}:requirement-group:inline-choice-{}",
            slugify_requirement_identifier(&format!(
                "{label}-{}",
                alternative_set.join("-or-")
            ))
        );
        groups.push(build_parsed_requirement_group(&json!({
            "id": choice_group_id,
            "label": alternative_set.join(" or "),
            "category": "source-choice",
            "subcategory": category,
            "requirementType": "choose_one",
            "minCourses": 1,
            "maxCourses": 1,
            "selectionCount": 1,
            "requiredCount": 1,
            "sourceHeading": source_line,
            "sourceRowText": source_line,
            "detectedOptionCue": "or",
            "options": build_inline_course_requirement_options(
                &choice_group_id,
                &alternative_set,
                &source_line,
                false,
                course_parser,
            ),
        })));
    }
    groups
        .into_iter()
        .filter(|group| !array(group.get("options")).is_empty())
        .collect()
}

pub fn build_numbered_inline_all_required_course_group(
    owner: &Value,
    raw_line: &str,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let normalized_line = super::requirement_choice_lists::strip_inline_labeled_requirement_marker(
        raw_line,
    );
    let body = numbered_inline_required_body_pattern()
        .captures(&normalize_whitespace(raw_line))
        .and_then(|captures| captures.get(1))
        .map(|capture| normalize_whitespace(capture.as_str()))
        .unwrap_or_default();
    if body.is_empty()
        || !source_line_starts_with_course_code(&body)
        || super::requirement_choice_lists::has_choice_requirement_context(&body)
        || get_sequence_choice_cue(&body).is_some()
        || inline_numbered_or_pattern().is_match(&body)
        || inline_numbered_required_exclusion_pattern().is_match(&body)
        || inline_numbered_sequence_parenthetical_pattern().is_match(&body)
    {
        return None;
    }
    let course_codes = course_parser.extract_from_requirement_line(&body);
    if course_codes.len() < 2 {
        return None;
    }
    let label = normalize_whitespace(
        &inline_numbered_trailing_credit_pattern()
            .replace(&body, ""),
    );
    let credits = parse_requirement_credit_amount(&body);
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let group_id = format!(
        "{owner_id}:requirement-group:inline-numbered-required-{}",
        slugify_requirement_identifier(&format!("{label}-{}", course_codes.join("-")))
    );
    let options = build_inline_course_requirement_options(
        &group_id,
        &course_codes,
        &normalized_line,
        true,
        course_parser,
    );
    let group = build_parsed_requirement_group(&json!({
        "id": group_id,
        "label": label,
        "category": "inline-numbered-requirement",
        "subcategory": "inline-numbered-requirement",
        "requirementType": "all_required",
        "minCourses": course_codes.len(),
        "maxCourses": course_codes.len(),
        "minCredits": credits,
        "maxCredits": credits,
        "sourceHeading": normalized_line,
        "sourceRowText": normalized_line,
        "detectedOptionCue": "inline numbered required courses",
        "notes": ["Parsed from an inline numbered source requirement."],
        "options": options,
    }));
    (!array(group.get("options")).is_empty()).then_some(group)
}

pub fn build_inline_course_requirement_options(
    group_id: &str,
    course_codes: &[String],
    source_line: &str,
    _allow_total_credit_fallback: bool,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    course_codes
        .iter()
        .map(|course_code| {
            let normalized_code = normalize_course_code(course_code);
            let metadata = course_parser.metadata_for(&normalized_code);
            let credit_range = course_parser.credit_range_for(&normalized_code);
            let department = normalized_code
                .rsplit_once(' ')
                .map(|(subject, _)| subject.to_string())
                .unwrap_or_default();
            json!({
                "id": format!(
                    "{group_id}:option:{}",
                    slugify_requirement_identifier(&normalized_code)
                ),
                "uwCourses": [normalized_code],
                "displayCourseCodes": [normalized_code],
                "label": normalized_code,
                "title": metadata.and_then(|value| value.get("title")).cloned().unwrap_or(Value::Null),
                "department": department,
                "sourceHeading": source_line,
                "sourceCategory": "inline-labeled-requirement",
                "credits": credit_range.credits,
                "creditMin": credit_range.credit_min,
                "creditMax": credit_range.credit_max,
                "creditText": credit_range.credit_text,
            })
        })
        .collect()
}

pub fn build_parsed_requirement_group(input: &Value) -> Value {
    let label = normalize_whitespace(&js_string_nullish(input.get("label")));
    let category = normalize_whitespace(&js_string_nullish(input.get("category")));
    let source_heading = normalize_whitespace(&js_string_nullish(input.get("sourceHeading")));
    let source_heading = if source_heading.is_empty() {
        label.clone()
    } else {
        source_heading
    };
    let requirement_type = input.get("requirementType");
    let is_credit_bucket = requirement_type.and_then(Value::as_str) == Some("choose_credits");
    let is_sequence_choice = requirement_type.and_then(Value::as_str) == Some("sequence_choice");
    let min_courses = if is_credit_bucket {
        Value::Null
    } else {
        nullish_value(input.get("minCourses"))
    };
    let max_courses = if is_credit_bucket {
        Value::Null
    } else {
        nullish_value(input.get("maxCourses"))
    };
    let required_count = if is_credit_bucket {
        Value::Null
    } else if let Some(required_count) = input.get("requiredCount").filter(|value| !value.is_null()) {
        required_count.clone()
    } else if !min_courses.is_null()
        && !max_courses.is_null()
        && js_strict_equal(&min_courses, &max_courses)
    {
        min_courses.clone()
    } else {
        Value::Null
    };
    let min_credits = nullish_value(input.get("minCredits"));
    let max_credits = if input
        .as_object()
        .is_some_and(|object| object.contains_key("maxCredits"))
    {
        nullish_value(input.get("maxCredits"))
    } else {
        nullish_value(Some(&min_credits))
    };
    let credit_text = normalize_whitespace(&js_string_nullish(input.get("creditText")));
    let credit_text = if !credit_text.is_empty() {
        json!(credit_text)
    } else if is_credit_bucket
        && !min_credits.is_null()
        && !max_credits.is_null()
        && !js_strict_equal(&min_credits, &max_credits)
    {
        json!(format!(
            "{}-{}",
            js_string_value(&min_credits),
            js_string_value(&max_credits)
        ))
    } else if !min_credits.is_null() {
        json!(js_string_value(&min_credits))
    } else {
        Value::Null
    };
    let sequence_paths = build_sequence_paths(input, &source_heading);
    let options = array(input.get("options"))
        .iter()
        .map(|option| {
            let mut option_input = Map::new();
            option_input.insert("sourceHeading".into(), json!(source_heading.clone()));
            option_input.insert("sourceCategory".into(), json!(category.clone()));
            if let Some(option_fields) = option.as_object() {
                for (key, value) in option_fields {
                    option_input.insert(key.clone(), value.clone());
                }
            }
            build_parsed_requirement_option(&Value::Object(option_input))
        })
        .filter(|option| {
            !array(option.get("uwCourses")).is_empty()
                || !array(option.get("equivalentUwCourseCodes")).is_empty()
                || option.get("optionKind").and_then(Value::as_str) == Some("category-option")
        })
        .collect::<Vec<_>>();
    let mut output = Map::new();
    if let Some(id) = input.get("id") {
        output.insert("id".into(), id.clone());
    }
    output.insert("label".into(), json!(label));
    output.insert("category".into(), json!(category));
    output.insert("subcategory".into(), normalized_string_or_null(input.get("subcategory")));
    if let Some(requirement_type) = requirement_type {
        output.insert("requirementType".into(), requirement_type.clone());
    }
    let default_shape = if is_credit_bucket {
        Some("credit-bucket")
    } else if is_sequence_choice {
        Some("sequence-choice")
    } else {
        None
    };
    output.insert(
        "requirementShape".into(),
        input
            .get("requirementShape")
            .filter(|value| !value.is_null())
            .cloned()
            .or_else(|| default_shape.map(|shape| json!(shape)))
            .unwrap_or(Value::Null),
    );
    output.insert(
        "minCourses".into(),
        if is_sequence_choice { json!(1) } else { min_courses },
    );
    output.insert(
        "maxCourses".into(),
        if is_sequence_choice { json!(1) } else { max_courses },
    );
    let selection_count = if is_credit_bucket {
        Value::Null
    } else if is_sequence_choice {
        json!(1)
    } else {
        nullish_value(
            input
                .get("selectionCount")
                .filter(|value| !value.is_null())
                .or_else(|| (!required_count.is_null()).then_some(&required_count)),
        )
    };
    output.insert("selectionCount".into(), selection_count);
    output.insert(
        "requiredCount".into(),
        if is_sequence_choice { json!(1) } else { required_count },
    );
    output.insert("minCredits".into(), min_credits.clone());
    output.insert("maxCredits".into(), max_credits);
    output.insert("creditText".into(), credit_text);
    let satisfaction_mode = input
        .get("satisfactionMode")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| {
            json!(if is_credit_bucket { "credit-based" } else { "selection-count" })
        });
    output.insert("satisfactionMode".into(), satisfaction_mode);
    output.insert("sourceHeading".into(), json!(source_heading.clone()));
    output.insert(
        "sourceRowText".into(),
        normalized_string_or_fallback(input.get("sourceRowText"), &source_heading),
    );
    output.insert("sourceSection".into(), normalized_string_or_null(input.get("sourceSection")));
    output.insert(
        "sourceSectionRole".into(),
        normalized_string_or_null(input.get("sourceSectionRole")),
    );
    output.insert(
        "sourceSectionSchedulable".into(),
        boolean_or_null(input.get("sourceSectionSchedulable")),
    );
    let provided_option_cue =
        normalize_whitespace(&js_string_nullish(input.get("detectedOptionCue")));
    let detected_option_cue = if provided_option_cue.is_empty() {
        detect_option_cue(&source_heading)
            .map(|cue| json!(cue))
            .unwrap_or(Value::Null)
    } else {
        json!(provided_option_cue)
    };
    output.insert("detectedOptionCue".into(), detected_option_cue);
    output.insert("sourceRole".into(), normalized_string_or_null(input.get("sourceRole")));
    output.insert("sourceUrl".into(), normalized_string_or_null(input.get("sourceUrl")));
    output.insert("sourceScope".into(), normalized_string_or_null(input.get("sourceScope")));
    output.insert("pathwayId".into(), normalized_string_or_null(input.get("pathwayId")));
    output.insert("routeId".into(), normalized_string_or_null(input.get("routeId")));
    output.insert("canCreateScheduleRow".into(), boolean_or_null(input.get("canCreateScheduleRow")));
    output.insert("supportOnly".into(), boolean_or_null(input.get("supportOnly")));
    output.insert("approvedListKey".into(), normalized_string_or_null(input.get("approvedListKey")));
    output.insert("canCreatePlaceholder".into(), boolean_or_null(input.get("canCreatePlaceholder")));
    output.insert("programSpecific".into(), boolean_or_null(input.get("programSpecific")));
    output.insert("notes".into(), json!(normalize_text_array(input.get("notes"), true)));
    output.insert("sequencePaths".into(), json!(sequence_paths));
    output.insert("options".into(), json!(options));
    Value::Object(output)
}

pub fn extract_parsed_requirement_group_course_codes(groups: &[Value]) -> Vec<String> {
    let mut course_codes = Vec::new();
    for group in groups {
        for path in array(group.get("sequencePaths")) {
            course_codes.extend(
                array(path.get("uwCourses"))
                    .iter()
                    .chain(array(path.get("conditionalLabCourses")))
                    .map(|course_code| js_string_nullish(Some(course_code))),
            );
        }
        for option in array(group.get("options")) {
            course_codes.extend(
                array(option.get("uwCourses"))
                    .iter()
                    .chain(array(option.get("equivalentUwCourseCodes")))
                    .chain(array(option.get("conditionalLabCourses")))
                    .map(|course_code| js_string_nullish(Some(course_code))),
            );
        }
    }
    unique_sorted(course_codes)
}

pub fn build_parsed_requirement_course(input: &Value) -> Option<Value> {
    let course_code = normalize_whitespace(&js_string_nullish(input.get("courseCode")));
    let normalized_course_code = normalize_course_code(&js_string_nullish(
        input
            .get("normalizedCourseCode")
            .filter(|value| !value.is_null())
            .or_else(|| input.get("courseCode")),
    ));
    if course_code.is_empty() || normalized_course_code.is_empty() {
        return None;
    }
    let credits = nullish_value(input.get("credits"));
    let credit_min = first_non_nullish_value(input.get("creditMin"), input.get("credits"));
    let credit_max = first_non_nullish_value(input.get("creditMax"), input.get("credits"));
    let provided_credit_text = normalize_whitespace(&js_string_nullish(input.get("creditText")));
    let credit_text = if !provided_credit_text.is_empty() {
        json!(provided_credit_text)
    } else if let Some(credits) = input.get("credits").filter(|value| !value.is_null()) {
        json!(js_string_value(credits))
    } else {
        Value::Null
    };
    let category = normalized_string_or_default(input.get("category"), "unclassified");
    let requirement_group_id =
        normalized_string_or_default(input.get("requirementGroupId"), "unclassified");
    let requirement_type = input
        .get("requirementType")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!("all_required"));
    let option_role = input
        .get("optionRole")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!("required"));
    let source_heading = normalized_string_or_default(input.get("sourceHeading"), "Unknown source heading");
    let source_category = normalized_string_or_default(input.get("sourceCategory"), "Unknown source category");
    let notes = normalize_text_array(input.get("notes"), true);
    Some(json!({
        "courseCode": course_code,
        "normalizedCourseCode": normalized_course_code,
        "title": normalized_string_or_null(input.get("title")),
        "credits": credits,
        "creditMin": credit_min,
        "creditMax": credit_max,
        "creditText": credit_text,
        "category": category,
        "requirementGroupId": requirement_group_id,
        "requirementType": requirement_type,
        "optionRole": option_role,
        "sourceHeading": source_heading,
        "sourceCategory": source_category,
        "notes": notes,
    }))
}

pub fn build_source_derived_requirement_courses(groups: &[Value]) -> Vec<Value> {
    let mut seen = HashSet::new();
    build_parsed_requirement_courses_from_groups(groups)
        .into_iter()
        .filter(|course| {
            let key = format!(
                "{}::{}::{}",
                js_string_nullish(course.get("requirementGroupId")),
                js_string_nullish(course.get("normalizedCourseCode")),
                js_string_nullish(course.get("optionRole")),
            );
            seen.insert(key)
        })
        .collect()
}

pub fn build_parsed_requirement_courses_from_groups(groups: &[Value]) -> Vec<Value> {
    let mut courses = Vec::new();
    for group in groups {
        let requirement_type = group.get("requirementType");
        let option_role = if matches!(requirement_type.and_then(Value::as_str), Some("all_required" | "sequence_choice")) {
            "required"
        } else {
            "option"
        };
        for option in array(group.get("options")) {
            let mut base = Map::new();
            base.insert(
                "title".into(),
                first_non_nullish_value(option.get("title"), option.get("label")),
            );
            base.insert("credits".into(), nullish_value(option.get("credits")));
            base.insert(
                "creditMin".into(),
                first_non_nullish_value(option.get("creditMin"), option.get("credits")),
            );
            base.insert(
                "creditMax".into(),
                first_non_nullish_value(option.get("creditMax"), option.get("credits")),
            );
            base.insert("creditText".into(), nullish_value(option.get("creditText")));
            base.insert(
                "category".into(),
                first_non_nullish_value(option.get("category"), group.get("category")),
            );
            base.insert("requirementGroupId".into(), nullish_value(group.get("id")));
            base.insert("requirementType".into(), nullish_value(requirement_type));
            base.insert(
                "sourceHeading".into(),
                first_non_nullish_value(
                    option.get("sourceHeading"),
                    group
                        .get("sourceHeading")
                        .filter(|value| !value.is_null())
                        .or_else(|| group.get("label")),
                ),
            );
            base.insert(
                "sourceCategory".into(),
                first_non_nullish_value(option.get("sourceCategory"), group.get("category")),
            );
            let mut notes = array(option.get("notes")).to_vec();
            notes.extend(array(option.get("constraints")).iter().cloned());
            base.insert("notes".into(), json!(notes));
            let display_course_codes = array(
                option
                    .get("displayCourseCodes")
                    .filter(|value| !value.is_null())
                    .or_else(|| option.get("uwCourses")),
            );
            for (index, course_code) in array(option.get("uwCourses")).iter().enumerate() {
                let display_course_code = display_course_codes
                    .get(index)
                    .filter(|value| !value.is_null())
                    .unwrap_or(course_code);
                let mut course_input = base.clone();
                course_input.insert("courseCode".into(), display_course_code.clone());
                course_input.insert("normalizedCourseCode".into(), course_code.clone());
                course_input.insert("optionRole".into(), json!(option_role));
                if let Some(course) = build_parsed_requirement_course(&Value::Object(course_input)) {
                    courses.push(course);
                }
            }
            for alias_code in array(option.get("equivalentUwCourseCodes")) {
                let mut course_input = base.clone();
                course_input.insert("courseCode".into(), alias_code.clone());
                course_input.insert("normalizedCourseCode".into(), alias_code.clone());
                course_input.insert("optionRole".into(), json!("alias"));
                if let Some(course) = build_parsed_requirement_course(&Value::Object(course_input)) {
                    courses.push(course);
                }
            }
        }
    }
    courses
}

pub fn build_parsed_requirement_option(input: &Value) -> Value {
    let uw_courses = unique_sorted(
        array(input.get("uwCourses"))
            .iter()
            .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    let display_course_codes = unique_sorted(
        array(
            input
                .get("displayCourseCodes")
                .filter(|value| !value.is_null())
                .or_else(|| input.get("uwCourses")),
        )
            .iter()
            .map(|course_code| normalize_whitespace(&js_string_nullish(Some(course_code))))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    let equivalent_uw_course_codes = unique_sorted(
        array(input.get("equivalentUwCourseCodes"))
            .iter()
            .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
            .filter(|course_code| !course_code.is_empty() && !uw_courses.contains(course_code))
            .collect(),
    );
    let label = normalize_whitespace(
        &input
            .get("label")
            .filter(|value| !value.is_null())
            .map(js_string_value)
            .unwrap_or_else(|| uw_courses.join(" / ")),
    );
    let raw_category_option = input
        .get("categoryOption")
        .filter(|category_option| js_truthy(category_option));
    let normalized_category_option = raw_category_option.map(normalize_category_option);
    let category_option_title = normalized_category_option
        .as_ref()
        .and_then(|category_option| category_option.get("title"))
        .filter(|title| !title.is_null())
        .cloned();
    let has_category_option = input.get("optionKind").and_then(Value::as_str)
        == Some("category-option")
        && normalized_category_option.as_ref().is_some_and(|category_option| {
            ["category", "sourceCategoryCode", "title"]
                .iter()
                .all(|key| category_option.get(*key).and_then(Value::as_str).is_some_and(|value| !value.is_empty()))
                && category_option
                    .get("credits")
                    .and_then(Value::as_f64)
                    .is_some_and(|credits| credits.is_finite() && credits > 0.0)
        });
    let category_option = if has_category_option {
        normalized_category_option.as_ref().map(|category_option| {
            let mut option = Map::new();
            for key in ["category", "sourceCategoryCode", "title", "credits", "creditMin", "creditMax", "sourceText", "approvedListKey"] {
                option.insert(key.to_string(), category_option.get(key).cloned().unwrap_or(Value::Null));
            }
            let program_specific = category_option
                .get("programSpecific")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            option.insert("programSpecific".into(), json!(program_specific));
            Value::Object(option)
        })
    } else {
        None
    };

    let conditional_lab_courses = unique_in_order(
        array(input.get("conditionalLabCourses"))
            .iter()
            .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    );
    let compound_components = array(input.get("compoundComponents"))
        .iter()
        .map(|component| {
            unique_in_order(
                array(Some(component))
                    .iter()
                    .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                    .filter(|course_code| !course_code.is_empty())
                    .collect(),
            )
        })
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let constraints = unique_sorted(
        array(input.get("constraints"))
            .iter()
            .map(|value| normalize_whitespace(&js_string_nullish(Some(value))))
            .filter(|value| !value.is_empty())
            .collect(),
    );
    let notes = unique_sorted(
        array(input.get("notes"))
            .iter()
            .map(|value| normalize_whitespace(&js_string_nullish(Some(value))))
            .filter(|value| !value.is_empty())
            .collect(),
    );
    let mut output = Map::new();
    if let Some(id) = input.get("id") {
        output.insert("id".into(), id.clone());
    }
    output.insert(
        "optionKind".into(),
        json!(if has_category_option { "category-option" } else { "course" }),
    );
    output.insert("sequencePathId".into(), normalized_string_or_null(input.get("sequencePathId")));
    output.insert("pathLabel".into(), normalized_string_or_null(input.get("pathLabel")));
    output.insert(
        "displayCourseCodes".into(),
        json!(if has_category_option { Vec::<String>::new() } else { display_course_codes }),
    );
    output.insert(
        "uwCourses".into(),
        json!(if has_category_option { Vec::<String>::new() } else { uw_courses }),
    );
    output.insert(
        "equivalentUwCourseCodes".into(),
        json!(if has_category_option { Vec::<String>::new() } else { equivalent_uw_course_codes }),
    );
    output.insert("conditionalLabCourses".into(), json!(conditional_lab_courses));
    output.insert("credits".into(), nullish_value(input.get("credits")));
    output.insert(
        "creditMin".into(),
        nullish_value(
            input
                .get("creditMin")
                .filter(|value| !value.is_null())
                .or_else(|| input.get("credits")),
        ),
    );
    output.insert(
        "creditMax".into(),
        nullish_value(
            input
                .get("creditMax")
                .filter(|value| !value.is_null())
                .or_else(|| input.get("credits")),
        ),
    );
    output.insert("creditText".into(), credit_text(input));
    output.insert("maxCredits".into(), nullish_value(input.get("maxCredits")));
    output.insert("title".into(), normalized_string_or_null(input.get("title")));
    output.insert("department".into(), normalized_string_or_null(input.get("department")));
    output.insert("category".into(), normalized_string_or_null(input.get("category")));
    output.insert("sourceHeading".into(), normalized_string_or_null(input.get("sourceHeading")));
    output.insert("sourceCategory".into(), normalized_string_or_null(input.get("sourceCategory")));
    output.insert(
        "grcMatches".into(),
        json!(if has_category_option {
            Vec::<String>::new()
        } else {
            unique_sorted(
                array(input.get("grcMatches"))
                    .iter()
                    .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                    .filter(|course_code| !course_code.is_empty())
                    .collect(),
            )
        }),
    );
    output.insert("compoundComponents".into(), json!(compound_components));
    output.insert("categoryOption".into(), category_option.unwrap_or(Value::Null));
    output.insert("constraints".into(), json!(constraints));
    output.insert("notes".into(), json!(notes));
    let label = if !label.is_empty() {
        label
    } else {
        category_option_title
            .map(|title| normalize_whitespace(&js_string_value(&title)))
            .unwrap_or_default()
    };
    output.insert("label".into(), json!(label));
    Value::Object(output)
}

fn build_sequence_paths(input: &Value, source_heading: &str) -> Vec<Value> {
    let mut seen_ids = HashSet::new();
    let mut paths = Vec::new();
    for (index, path) in array(input.get("sequencePaths")).iter().enumerate() {
        let uw_courses = unique_in_order(
            array(path.get("uwCourses"))
                .iter()
                .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        let conditional_lab_courses = unique_in_order(
            array(path.get("conditionalLabCourses"))
                .iter()
                .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        let provided_id = normalize_whitespace(&js_string_nullish(path.get("id")));
        let base_id = if provided_id.is_empty() {
            format!(
                "{}:sequence-path:{}",
                js_interpolate(input.get("id")),
                slugify_requirement_identifier(&uw_courses.join("-"))
            )
        } else {
            provided_id
        };
        let mut id = base_id.clone();
        if seen_ids.contains(&id) {
            let slug = slugify_requirement_identifier(&uw_courses.join("-"));
            let disambiguator = if slug.is_empty() {
                (index + 1).to_string()
            } else {
                slug
            };
            id = format!("{base_id}-{disambiguator}");
            let mut suffix = 2usize;
            while seen_ids.contains(&id) {
                id = format!("{base_id}-{disambiguator}-{suffix}");
                suffix += 1;
            }
        }
        seen_ids.insert(id.clone());
        let label = normalize_whitespace(&js_string_nullish(path.get("label")));
        let label = if label.is_empty() {
            uw_courses.join(" + ")
        } else {
            label
        };
        let display_course_codes = unique_in_order(
            array(
                path.get("displayCourseCodes")
                    .filter(|value| !value.is_null())
                    .or_else(|| path.get("uwCourses")),
            )
            .iter()
            .map(|course_code| normalize_whitespace(&js_string_nullish(Some(course_code))))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
        );
        let mapped_grc_course_codes = unique_in_order(
            array(path.get("mappedGrcCourseCodes"))
                .iter()
                .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                .filter(|course_code| !course_code.is_empty())
                .collect(),
        );
        let compound_components = array(path.get("compoundComponents"))
            .iter()
            .map(|component| {
                unique_in_order(
                    array(Some(component))
                        .iter()
                        .map(|course_code| normalize_course_code(&js_string_nullish(Some(course_code))))
                        .filter(|course_code| !course_code.is_empty())
                        .collect(),
                )
            })
            .filter(|component| !component.is_empty())
            .collect::<Vec<_>>();
        let source_text = normalized_string_or_fallback(path.get("sourceText"), source_heading);
        let mut normalized_path = Map::new();
        normalized_path.insert("id".into(), json!(id));
        normalized_path.insert("label".into(), json!(label));
        normalized_path.insert("uwCourses".into(), json!(uw_courses.clone()));
        normalized_path.insert("displayCourseCodes".into(), json!(display_course_codes));
        normalized_path.insert("mappedGrcCourseCodes".into(), json!(mapped_grc_course_codes));
        normalized_path.insert("compoundComponents".into(), json!(compound_components));
        normalized_path.insert("conditionalLabCourses".into(), json!(conditional_lab_courses));
        normalized_path.insert("notes".into(), json!(normalize_text_array(path.get("notes"), true)));
        normalized_path.insert("sourceText".into(), source_text);
        if !uw_courses.is_empty() {
            paths.push(Value::Object(normalized_path));
        }
    }
    paths
}

fn extract_course_codes_from_joint_course_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let normalized_line = normalize_whitespace(&line.replace('*', ""));
    let mut course_codes = Vec::new();
    for captures in shared_joint_course_pattern().captures_iter(&normalized_line) {
        if let (Some(left), Some(right), Some(number)) =
            (captures.get(1), captures.get(2), captures.get(3))
        {
            course_codes.extend(course_parser.normalize_code(left.as_str(), number.as_str()));
            course_codes.extend(course_parser.normalize_code(right.as_str(), number.as_str()));
        }
    }
    for captures in split_joint_course_pattern().captures_iter(&normalized_line) {
        if let (Some(left_subject), Some(left_number), Some(right_subject), Some(right_number)) =
            (captures.get(1), captures.get(2), captures.get(3), captures.get(4))
        {
            course_codes.extend(
                course_parser.normalize_code(left_subject.as_str(), left_number.as_str()),
            );
            course_codes.extend(
                course_parser.normalize_code(right_subject.as_str(), right_number.as_str()),
            );
        }
    }
    course_codes.extend(course_parser.extract_from_line(&normalized_line));
    unique_in_order(
        course_codes
            .into_iter()
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect(),
    )
}

fn get_inline_section_course_title(
    course_line: &str,
    primary_course_code: &str,
) -> Option<String> {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(course_line),
    );
    let course_number = normalize_course_code(primary_course_code)
        .rsplit(' ')
        .next()
        .unwrap_or_default()
        .to_string();
    if normalized_line.is_empty() || course_number.is_empty() {
        return None;
    }
    let course_number_pattern = regex::Regex::new(&format!(
        r"(?i)\b{}\b",
        regex::escape(&course_number)
    ))
    .ok()?;
    let number_match = course_number_pattern.find(&normalized_line)?;
    let after_course_number = &normalized_line[number_match.end()..];
    let before_alternative = alternative_course_title_boundary_pattern()
        .find(after_course_number)
        .map(|matched| &after_course_number[..matched.start()])
        .unwrap_or(after_course_number);
    let before_next_course = if let Some(next_course) = next_course_number_pattern().find(before_alternative) {
        trailing_course_subject_pattern()
            .replace(&before_alternative[..next_course.start()], "")
            .into_owned()
    } else {
        before_alternative.to_string()
    };
    let title = normalize_whitespace(
        &trailing_credit_parenthetical_pattern().replace(
            &leading_course_title_punctuation_pattern().replace(&before_next_course, ""),
            "",
        ),
    );
    (!title.is_empty() && title.chars().any(char::is_alphabetic)).then_some(title)
}

fn get_selected_pathway_display_label(
    owner: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Option<String> {
    let major_title = super::requirement_recovery_scope::primary_major_title(
        owner,
        primary_major_titles,
    );
    let owner_title = owner
        .get("ownerTitle")
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_default();
    let semantic_label = super::transfer_plan_title_matching::
        normalize_transfer_planner_semantic_pathway_label(&major_title, &owner_title);
    let fallback_label = owner
        .get("pathwayId")
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_default()
        .replace('-', " ")
        .replace('_', " ");
    let label = if semantic_label.is_empty() {
        fallback_label
    } else {
        semantic_label
    };
    let normalized = super::transfer_plan_title_matching::normalize_transfer_planner_text(&label);
    let normalized = selected_pathway_label_suffix_pattern()
        .replace(&normalized, "")
        .trim()
        .to_string();
    (!normalized.is_empty()).then_some(normalized)
}

fn collect_selected_pathway_section_course_lines(
    owner: &Value,
    snapshot_lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    if !js_truthy(owner.get("pathwayId")) {
        return Vec::new();
    }
    let selected_start_indexes = snapshot_lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let normalized = normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            );
            super::requirement_recovery_scope::line_matches_selected_pathway_identity(
                owner,
                &normalized,
                primary_major_titles,
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let mut selected_course_lines = Vec::new();
    let mut seen_course_lines = HashSet::new();
    for selected_start_index in selected_start_indexes {
        for index in selected_start_index..snapshot_lines.len().min(selected_start_index + 80) {
            let line = normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[index]),
            );
            if line.is_empty() {
                continue;
            }
            if index > selected_start_index
                && (super::requirement_recovery_scope::is_pathway_html_section_sibling_start(
                    owner,
                    &line,
                    primary_major_titles,
                    course_parser,
                )
                    || super::requirement_recovery_scope::is_pathway_section_boundary_line(&line)
                    || recommended_course_or_honours_thesis_pattern().is_match(&line))
            {
                break;
            }
            if let Some(course_line) = get_sectioned_option_course_line(&line) {
                if seen_course_lines.insert(course_line.clone()) {
                    selected_course_lines.push(json!({
                        "line": line,
                        "courseLine": course_line,
                        "index": index,
                    }));
                }
            }
        }
    }
    selected_course_lines
}

fn get_sectioned_option_course_line(line: &str) -> Option<String> {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if normalized_line.is_empty() {
        return None;
    }
    let candidates = unique_in_order(vec![
        normalized_line.clone(),
        normalize_whitespace(&leading_credit_label_pattern().replace(&normalized_line, "")),
        normalize_whitespace(&leading_bracket_label_pattern().replace(&normalized_line, "")),
        normalize_whitespace(&leading_bracket_label_pattern().replace(
            &leading_credit_label_pattern().replace(&normalized_line, ""),
            "",
        )),
    ]);
    candidates
        .into_iter()
        .find(|candidate| !candidate.is_empty() && source_line_starts_with_course_code(candidate))
}

fn should_split_sectioned_course_line_into_separate_options(
    line: &str,
    course_codes: &[String],
) -> bool {
    if course_codes.len() < 2 || line.contains('/') {
        return false;
    }
    let subjects = course_codes
        .iter()
        .filter_map(|course_code| course_code.rsplit_once(' ').map(|(subject, _)| subject))
        .collect::<HashSet<_>>();
    subjects.len() == 1 && comma_course_number_pattern().is_match(line)
}

fn get_nearby_section_course_credits(
    snapshot_lines: &[String],
    course_line_index: usize,
    course_code: &str,
    course_parser: &CourseCodeParser,
) -> Value {
    if let Some(credit_range) = parse_credit_bearing_course_row_credit_range(
        snapshot_lines.get(course_line_index).map(String::as_str).unwrap_or_default(),
    ) {
        return credit_range;
    }
    for index in course_line_index + 1..snapshot_lines.len().min(course_line_index + 5) {
        let candidate = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[index]),
        );
        if candidate.is_empty() || is_course_continuation_line(&candidate, course_parser) {
            continue;
        }
        if let Some(credit_range) = parse_sectioned_course_credit_list_line(&candidate) {
            return credit_range;
        }
        if plain_credit_number_range_pattern().is_match(&candidate) {
            if let Some((minimum, maximum)) =
                parse_requirement_credit_range(&format!("{candidate} credits"))
            {
                return credit_range_value(minimum, maximum, None);
            }
        }
        if looks_like_sectioned_course_heading_line(&candidate, course_parser)
            || source_line_starts_with_course_code(&candidate)
            || is_sectioned_option_course_boundary(&candidate, course_parser)
        {
            break;
        }
    }
    let range = course_parser.credit_range_for(course_code);
    json!({
        "credits": range.credits,
        "creditMin": range.credit_min,
        "creditMax": range.credit_max,
        "creditText": range.credit_text,
    })
}

fn parse_credit_bearing_course_row_credit_range(course_line: &str) -> Option<Value> {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(course_line),
    );
    let captures = course_row_parenthetical_credit_pattern()
        .captures(&normalized_line)
        .or_else(|| course_row_parenthetical_fallback_pattern().captures(&normalized_line))?;
    let credits = captures.get(1)?.as_str().parse::<f64>().ok()?;
    let has_explicit_credit_unit = captures.get(2).is_some();
    if !credits.is_finite() || credits <= 0.0 || (!has_explicit_credit_unit && credits > 30.0) {
        return None;
    }
    Some(credit_range_value(credits, Some(credits), None))
}

fn credit_range_value(
    credits: f64,
    credit_max: Option<f64>,
    credit_text: Option<String>,
) -> Value {
    let credit_max = credit_max
        .filter(|maximum| maximum.is_finite() && *maximum >= credits)
        .unwrap_or(credits);
    let credit_text = credit_text.unwrap_or_else(|| {
        if credit_max != credits {
            format!("{}-{}", js_number_string(credits), js_number_string(credit_max))
        } else {
            js_number_string(credits)
        }
    });
    json!({
        "credits": credits,
        "creditMin": credits,
        "creditMax": credit_max,
        "creditText": credit_text,
    })
}

fn string_values(value: Option<&Value>) -> Vec<String> {
    array(value)
        .iter()
        .map(js_string_value)
        .collect()
}

fn sum_sectioned_option_credits(options: &[Value]) -> Option<f64> {
    let mut total = 0.0;
    for option in options {
        let credit = option
            .get("creditMin")
            .filter(|value| !value.is_null())
            .or_else(|| option.get("credits"));
        let number = match credit {
            None | Some(Value::Null) => Some(0.0),
            Some(Value::Bool(value)) => Some(if *value { 1.0 } else { 0.0 }),
            Some(Value::Number(value)) => value.as_f64(),
            Some(Value::String(value)) if value.trim().is_empty() => Some(0.0),
            Some(Value::String(value)) => value.trim().parse::<f64>().ok(),
            _ => None,
        }?;
        if !number.is_finite() {
            return None;
        }
        total += number;
    }
    Some(total)
}

fn flatten_option_course_codes(options: &[Value], field: &str) -> Vec<String> {
    options
        .iter()
        .flat_map(|option| array(option.get(field)).iter().map(js_string_value))
        .collect()
}

pub fn normalize_pdf_requirement_heading_line(line: &str) -> String {
    let mut normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if normalized_line.is_empty() || source_line_starts_with_course_code(&normalized_line) {
        return normalized_line;
    }
    let inline_credit_heading_end = pdf_credit_marker_pattern()
        .find_iter(&normalized_line)
        .map(|marker| marker.end())
        .find(|end| inline_course_after_pdf_marker_pattern().is_match(&normalized_line[*end..]));
    if let Some(end) = inline_credit_heading_end {
        normalized_line = normalize_whitespace(&normalized_line[..end]);
    }
    let credit_markers = pdf_credit_marker_pattern()
        .find_iter(&normalized_line)
        .collect::<Vec<_>>();
    if credit_markers.len() >= 2 && pdf_ellipsis_pattern().is_match(&normalized_line) {
        if let Some(first_marker) = credit_markers.first() {
            return normalize_whitespace(&normalized_line[..first_marker.end()]);
        }
    }
    normalized_line
}

pub fn normalize_sectioned_course_group_label(line: &str) -> String {
    let cleaned = trailing_category_punctuation_pattern()
        .replace(&normalize_pdf_requirement_heading_line(line), "");
    let cleaned = normalize_whitespace(&cleaned);
    if cleaned.is_empty() {
        "Sectioned course group".into()
    } else {
        cleaned
    }
}

pub fn is_generic_university_category_distribution_heading(line: &str) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    !normalized_line.is_empty()
        && !source_line_starts_with_course_code(&normalized_line)
        && line_has_generic_category_distribution_cue(&normalized_line)
        && generic_university_category_list_pattern().is_match(&normalized_line)
        && !generic_category_list_following_pattern().is_match(&normalized_line)
}

pub fn is_generic_category_credit_bucket_heading(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if normalized_line.is_empty()
        || source_line_starts_with_course_code(&normalized_line)
        || !line_has_generic_category_distribution_cue(&normalized_line)
        || !course_parser.extract_from_line(&normalized_line).is_empty()
        || parse_requirement_credit_range(&normalized_line).is_none()
    {
        return false;
    }
    generic_category_credit_bucket_heading_pattern().is_match(&normalized_line)
}

pub fn get_trimmed_sectioned_course_group_heading(
    line: &str,
    descriptor: Option<&Value>,
) -> String {
    let normalized_line = normalize_sectioned_course_group_label(line);
    if !line_has_generic_category_distribution_cue(&normalized_line) {
        return normalized_line;
    }
    let descriptor_min_credits = descriptor
        .and_then(|descriptor| descriptor.get("minCredits"))
        .filter(|value| !value.is_null())
        .and_then(Value::as_f64);
    let matching_part = normalized_line
        .split(';')
        .map(normalize_whitespace)
        .filter(|part| !part.is_empty())
        .find(|part| {
            if line_has_generic_category_distribution_cue(part)
                || !trimmed_section_heading_candidate_pattern().is_match(part)
            {
                return false;
            }
            let Some(descriptor_min_credits) = descriptor_min_credits else {
                return true;
            };
            parse_section_heading_credit_range(part)
                .and_then(|range| range.get("minCredits").and_then(Value::as_f64))
                .is_none_or(|minimum| minimum == descriptor_min_credits)
        });
    if let Some(matching_part) = matching_part {
        return matching_part;
    }
    if let Some(category_clause) = generic_category_clause_pattern().find(&normalized_line) {
        let prefix = normalized_line[..category_clause.start()]
            .trim_end_matches(|character: char| character == ';' || character == ',');
        let prefix = normalize_whitespace(prefix);
        if !prefix.is_empty() && trimmed_section_heading_candidate_pattern().is_match(&prefix) {
            return prefix;
        }
    }
    normalized_line
}

fn line_has_generic_category_distribution_cue(line: &str) -> bool {
    super::requirement_category_options::get_requirement_category_option_descriptor(line).is_some()
}

fn build_sectioned_course_category(line: &str) -> String {
    let normalized_line = normalize_pdf_requirement_heading_line(line);
    let cleaned = sectioned_category_min_max_pattern()
        .replace_all(&normalized_line, "");
    let cleaned = sectioned_category_credit_amount_pattern().replace_all(&cleaned, "");
    let cleaned = sectioned_category_cue_pattern().replace_all(&cleaned, "");
    let cleaned = trailing_category_punctuation_pattern().replace(&cleaned, "");
    let cleaned = normalize_whitespace(&cleaned);
    let value: &str = if cleaned.is_empty() { line } else { cleaned.as_ref() };
    let category = super::requirement_rule_utils::slugify_parser_rule_value(value).replace('-', "_");
    if category.is_empty() { "sectioned_course_group".to_string() } else { category }
}

fn get_nearby_section_course_title(
    snapshot_lines: &[String],
    course_line_index: usize,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    for index in course_line_index + 1..snapshot_lines.len().min(course_line_index + 4) {
        let candidate = normalize_whitespace(
            &super::requirement_choice_lists::strip_choice_list_line(&snapshot_lines[index]),
        );
        if candidate.is_empty() || is_sectioned_course_credit_list_line(&candidate) {
            continue;
        }
        if is_course_continuation_line(&candidate, course_parser) {
            continue;
        }
        if source_line_starts_with_course_code(&candidate)
            || is_sectioned_option_course_boundary(&candidate, course_parser)
            || looks_like_sectioned_course_heading_line(&candidate, course_parser)
            || formal_option_contact_advisor_boundary_pattern().is_match(&candidate)
            || academic_advising_progress_boundary_pattern().is_match(&candidate)
        {
            return None;
        }
        return looks_like_nearby_section_course_title(&candidate, course_parser)
            .then_some(candidate);
    }
    None
}

fn looks_like_nearby_section_course_title(candidate: &str, course_parser: &CourseCodeParser) -> bool {
    if candidate.is_empty() {
        return false;
    }
    if max_degree_credit_constraint_pattern().is_match(candidate) {
        return true;
    }
    if !course_parser.extract_from_line(candidate).is_empty()
        || candidate.chars().next().is_some_and(|character| character.is_ascii_lowercase())
    {
        return false;
    }
    !nearby_course_title_exclusion_pattern().is_match(candidate)
}

fn is_course_continuation_line(line: &str, course_parser: &CourseCodeParser) -> bool {
    starts_with_course_continuation_pattern().is_match(&normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    )) && !course_parser.extract_from_line(line).is_empty()
}

fn is_sectioned_course_credit_list_line(line: &str) -> bool {
    parse_sectioned_course_credit_list_line(line).is_some()
}

fn source_line_starts_with_course_code(line: &str) -> bool {
    let normalized_line = page_prefix_pattern().replace(line, "");
    direct_course_row_prefix_pattern().is_match(&normalize_whitespace(&normalized_line))
}

fn is_sectioned_option_course_boundary(line: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized_line = normalize_whitespace(line);
    if normalized_line.is_empty() {
        return false;
    }
    supplemental_source_boundary_pattern().is_match(&normalized_line)
        || standalone_section_boundary_pattern().is_match(&normalized_line)
        || (admission_or_requirement_heading_pattern().is_match(&normalized_line)
            && !core_or_elective_heading_pattern().is_match(&normalized_line)
            && course_parser.extract_from_line(&normalized_line).is_empty())
}

fn looks_like_sectioned_course_heading_line(line: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if normalized_line.is_empty()
        || season_heading_pattern().is_match(&normalized_line)
        || source_line_starts_with_course_code(&normalized_line)
        || (!course_parser.extract_from_line(&normalized_line).is_empty()
            && !course_level_heading_pattern().is_match(&normalized_line))
    {
        return false;
    }
    sectioned_required_count_pattern().is_match(&normalized_line)
        || (sectioned_course_list_cue_pattern().is_match(&normalized_line)
            && sectioned_course_list_topic_pattern().is_match(&normalized_line))
}

fn is_standalone_section_choice_cue_line(line: &str, _course_parser: &CourseCodeParser) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    !normalized_line.is_empty()
        && !source_line_starts_with_course_code(&normalized_line)
        && standalone_section_following_pattern().is_match(&normalized_line)
}

fn sequence_choice_cue_patterns() -> &'static [(regex::Regex, &'static str)] {
    static VALUE: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        [
            (r"(?i)\bone\s+of\s+the\s+following(?:\s+\w+){0,4}\s+sequences?\b", "one of the following sequences"),
            (r"(?i)\bchoose\s+one\s+sequence\b", "choose one sequence"),
            (r"(?i)\bselect\s+one\s+sequence\b", "select one sequence"),
            (r"(?i)\bone\s+sequence\b", "one sequence"),
            (r"(?i)\beither\s+sequence\b", "either sequence"),
            (r"(?i)\b(?:calculus-based|algebra-based)\b.{0,40}\bor\b.{0,40}\b(?:calculus-based|algebra-based)\b", "calculus-based vs algebra-based"),
            (r"(?i)\b(?:regular|standard|accelerated|honors?)\b.{0,40}\bor\b.{0,40}\b(?:regular|standard|accelerated|honors?)\b", "standard vs honors"),
            (r"(?i)\b[A-Z&]+(?:\s+[A-Z&]+)?\s+\d{3}[A-Z]?\s*,\s*\d{3}[A-Z]?(?:\s*,\s*\d{3}[A-Z]?){0,2}\s+or\s+(?:[A-Z&]+(?:\s+[A-Z&]+)?\s+)?\d{3}[A-Z]?\s*,\s*\d{3}[A-Z]?(?:\s*,\s*\d{3}[A-Z]?){0,2}\b", "or between grouped series"),
        ]
        .into_iter()
        .map(|(pattern, cue)| (regex::Regex::new(pattern).expect("sequence choice cue pattern"), cue))
        .collect()
    })
}

fn sequence_path_label_patterns() -> &'static [(regex::Regex, &'static str)] {
    static VALUE: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        [
            (r"(?i)\bcalculus-based\b", "calculus-based"),
            (r"(?i)\balgebra-based\b", "algebra-based"),
            (r"(?i)\bhonors?\b", "honors"),
            (r"(?i)\baccelerated\b", "accelerated"),
            (r"(?i)\b(?:regular|standard)\b", "standard"),
            (r"(?i)\bbiology\b", "biology sequence"),
            (r"(?i)\bchemistry\b", "chemistry sequence"),
            (r"(?i)\bphysics\b", "physics sequence"),
            (r"(?i)\bmathematics|math\b", "math sequence"),
        ]
        .into_iter()
        .map(|(pattern, label)| (regex::Regex::new(pattern).expect("sequence path label pattern"), label))
        .collect()
    })
}

fn pdf_credit_marker_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\[[^\]]*\b(?:minimum\s+)?\d+(?:\.\d+)?(?:\s*(?:-|to)\s*\d+(?:\.\d+)?)?\s*(?:credits?|cr\.?)[^\]]*\]").expect("PDF heading credit marker pattern"))
}
fn inline_course_after_pdf_marker_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\s+[A-Z&]{1,8}(?:\s+[A-Z&]{1,8}){0,2}\s+\d{3}[A-Za-z]?\b").expect("inline course after PDF marker pattern"))
}
fn pdf_ellipsis_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[.\u{2026}]{3,}").expect("PDF heading ellipsis pattern"))
}
fn generic_university_category_list_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:chosen|selected|taken)\s+from\b.{0,80}\b(?:university|uw)\b.{0,40}\blist\b").expect("generic university category list pattern"))
}
fn generic_category_list_following_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:following|below|listed)\b").expect("generic category list following pattern"))
}
fn generic_category_credit_bucket_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:Additional\s+)?(?:Diversity Requirement|Arts?\s+(?:and|&)\s+Humanities|Social Sciences?|Natural Sciences?|Natural Science|Natural World|Individuals\s+(?:and|&)\s+Societies|Quantitative\s+(?:and|&)\s+Symbolic\s+Reasoning|\b(?:A\s*&\s*H|S\s*Sc|N\s*Sc|I\s*&\s*S|VLPA|DIV|QSR|NW)\b)").expect("generic category credit bucket heading pattern"))
}
fn trimmed_section_heading_candidate_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:electives?|approved|selected|following|from)\b").expect("trimmed section heading candidate pattern"))
}
fn generic_category_clause_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\s*(?:;|,)?\s*(?:the\s+final\s+math\s+course,\s*)?(?:VLPA|I\s*&\s*S|A\s*&\s*H|S\s*Sc|N\s*Sc|Arts?\s+(?:and|&)\s+Humanities|Social Sciences?|Natural Sciences?|Natural World|Diversity)\b").expect("generic category clause pattern"))
}
fn selected_pathway_label_suffix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:option|track|route|pathway|concentration)\b$").expect("selected pathway label suffix pattern"))
}
fn concentration_credit_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\b.{0,140}\bconcentration\s+area(?:\s+courses?)?\b").expect("concentration credit line pattern"))
}
fn recommended_course_or_honours_thesis_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:recommended courses?|honou?rs?\s+thesis)\b").expect("recommended courses or honours thesis pattern"))
}
fn inline_mixed_body_or_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bor\b").expect("inline mixed body OR pattern"))
}
fn inline_mixed_source_exclusion_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:choose|select|one\s+of|any\s+of|electives?|approved|from\s+the\s+following|from\s+approved)\b").expect("inline mixed source exclusion pattern"))
}
fn inline_mixed_minimum_course_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:minimum|at\s+least)\s+(?:of\s+)?(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\b").expect("inline mixed minimum course pattern"))
}
fn inline_mixed_required_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:core courses?|required courses?|major requirements?|requirements?)\b").expect("inline mixed required label pattern"))
}
fn inline_mixed_body_delimiter_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[,;]").expect("inline mixed body delimiter pattern"))
}
fn repeated_required_alternative_exclusion_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:choose|select|one\s+of|any\s+of|either)\b").expect("repeated required alternative exclusion pattern"))
}
fn repeated_required_and_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\band\b").expect("repeated required and pattern"))
}
fn repeated_required_comma_or_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i),\s*[^,]{0,80}\bor\b").expect("repeated required comma OR pattern"))
}
fn parenthetical_course_alternative_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b([A-Za-z&]{1,20}(?:\s+[A-Za-z&]{1,20})?)\s*(\d{3}[A-Za-z]?)\s*\(\s*or\s+([^)]*)\)").expect("parenthetical course alternative pattern"))
}
fn inline_course_or_alternative_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b([A-Za-z&]{1,20}(?:\s+[A-Za-z&]{1,20})?)\s*(\d{3}[A-Za-z]?)(?:\s*\([^)]*\))?\s+or\s+(?:(?:([A-Za-z&]{1,20}(?:\s+[A-Za-z&]{1,20})?)\s*)?(\d{3}[A-Za-z]?))").expect("inline course OR alternative pattern"))
}
fn source_line_starts_with_course_code(line: &str) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_snapshot_page_prefix(line),
    );
    direct_course_code_line_pattern().is_match(&normalized_line)
}
fn direct_course_code_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^[-*\x{2022}\x{F071}\s]*(?:\(\s*\d+(?:\.\d+)?\s*\)\s*)?(?:[A-Z&]+(?:\s+[A-Z&]+)?(?:\s*/\s*[A-Z&]+(?:\s+[A-Z&]+)?)?\s+\d{3}[A-Za-z]?\*?|\d{3}[A-Za-z]?\b)").expect("direct course code line pattern"))
}
fn numbered_inline_option_row_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^\d+\s*[\).]\s+").expect("numbered inline option row pattern"))
}
fn numbered_inline_required_body_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^\s*\d+\s*[\).]\s+(.+)$").expect("numbered inline required body pattern"))
}
fn choose_one_option_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bchoose\s+one\s+option\b").expect("choose one option heading pattern"))
}
fn degree_total_free_electives_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\(?\s*includes?\s+\d+(?:\.\d+)?\s+credits?\s+free\s+electives?\s*\)?$").expect("degree total free electives pattern"))
}
fn degree_total_music_program_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:Admission to the Bachelor of Music|This degree is offered with a major in Music)\b.*\bminimum\s+of\s+1?80\s+credits?\b.*\bdepartments?\s+other\s+than\s+the\s+School\s+of\s+Music\b").expect("degree total music program pattern"))
}
fn degree_total_general_elective_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:additional\s+)?(?:general\s+)?electives?\b").expect("degree total general elective cue pattern"))
}
fn degree_total_general_elective_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:as needed to reach|totaling(?: a)?(?: minimum)?(?: of)?|minimum(?: of)? total(?: of)?|total(?:s|ed)?(?: the)?)\b.{0,80}\b1?80\s+credits?\b").expect("degree total general elective pattern"))
}
fn degree_total_elective_total_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bbring(?:s|ing)?(?: your)? total to\s+1?80\b").expect("degree total elective total pattern"))
}
fn degree_total_general_elective_depending_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bgeneral electives?\s+may also be required\b.{0,120}\bdepending\b").expect("degree total general elective depending pattern"))
}
fn sequence_alternative_leading_marker_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^[^A-Za-z]*(?:[a-z]\)|\d+\))?\s*").expect("sequence alternative leading marker pattern"))
}
fn sequence_words_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:sequence|series)\b").expect("sequence words pattern"))
}
fn sequence_subject_suffix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\s+\b(?:calculus|chemistry|physics|biology)\b$").expect("sequence subject suffix pattern"))
}
fn sequence_label_trailing_noise_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[^A-Za-z-]+$").expect("sequence label trailing noise pattern"))
}
fn sequence_label_terminal_punctuation_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[:;,.-]+$").expect("sequence label terminal punctuation pattern"))
}
fn sequence_allowed_alternative_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:calculus-based|algebra-based|regular|standard|accelerated|honors?)$").expect("allowed sequence alternative label pattern"))
}
fn parenthetical_sequence_choice_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\(\s*or\s+").expect("parenthetical sequence choice cue pattern"))
}
fn parenthetical_sequence_candidate_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)((?:[A-Z&]+(?:\s+[A-Z&]+)?\s+)?\d{3}[A-Za-z]?(?:\s*,\s*(?:[A-Z&]+(?:\s+[A-Z&]+)?\s+)?\d{3}[A-Za-z]?){1,5})\s*\(\s*or\s+([^)]+)\)").expect("parenthetical sequence candidate pattern"))
}
fn sequence_alternative_choice_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(super::requirement_shape::SEQUENCE_ALTERNATIVE_CHOICE_LINE_PATTERN_SOURCE).expect("sequence alternative choice pattern"))
}
fn sequence_bare_number_continuation_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?:,\s*\d{3}[A-Za-z]?|\bor\s+\d{3}[A-Za-z]?)").expect("sequence bare number continuation pattern"))
}
fn trailing_parenthetical_subject_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\(([A-Z&]{2,8})\)\s*$").expect("trailing parenthetical subject pattern"))
}
fn named_sequence_heading_subject_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:physics|chemistry|mathematics|math|biology|biochemistry)\b.*\(([A-Z&]{2,8})\)").expect("named sequence heading subject pattern"))
}
fn fallback_physics_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bphysics\b").expect("fallback physics context pattern"))
}
fn fallback_chemistry_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bchemistry\b").expect("fallback chemistry context pattern"))
}
fn fallback_math_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bmathematics\b|\bcalculus\b").expect("fallback math context pattern"))
}
fn fallback_biology_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bbiology\b").expect("fallback biology context pattern"))
}
fn ampersand_or_single_sequence_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b[A-Z]{2,8}\s+\d{3}[^:]*&[^:]*\bor\b[^:]*[A-Z]{2,8}\s+\d{3}").expect("ampersand-or-single sequence pattern"))
}
fn ampersand_or_separator_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bor\b").expect("ampersand-or separator pattern"))
}
fn inline_or_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bor\b").expect("inline or cue pattern"))
}
fn merged_pdf_labeled_choice_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bSelect\s+[A-Z][A-Za-z&/ -]{1,40}:").expect("merged PDF labeled choice pattern"))
}
fn merged_pdf_choice_fragment_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:competency|300-400 level|At least)\b").expect("merged PDF choice fragment cue pattern"))
}
fn public_health_biology_choice_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^Select BIOL$").expect("public health biology choice pattern"))
}
fn public_health_chemistry_choice_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^Select CHEM$").expect("public health chemistry choice pattern"))
}
fn public_health_leaked_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:ELECTIVES\s*\*?|following|cr)$").expect("public health leaked label pattern"))
}
fn public_health_natural_science_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bNatural Sciences?\b").expect("public health natural science pattern"))
}
fn public_health_service_learning_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bPUBLIC HEALTH SERVICE LEARNING\b").expect("public health service learning pattern"))
}
fn sbse_comparison_table_snapshot_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcomparison table\b.*\bbse\b.*\bsbse\b").expect("SBSE comparison table snapshot pattern"))
}
fn sbse_engineering_elective_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^Engineering Electives?:\s*12\s+credits\s+chosen\s+from\b").expect("SBSE engineering elective line pattern"))
}
fn sbse_diversity_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^Diversity\s*\(DIV\):\s*5\s+credits\b").expect("SBSE diversity line pattern"))
}
fn sbse_social_sciences_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^10\s+credits\s+chosen\s+from\s+the\s+University\s+SSc\s+list\b").expect("SBSE social sciences line pattern"))
}
fn sbse_statistics_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^QSCI\s+381\s+\(or\s+INDE\s+315\s+or\s+STAT\s+390\)").expect("SBSE statistics line pattern"))
}
fn sbse_physics_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^PHYS\s+121\s+and\s+PHYS\s+122\s+\(or\s+PHYS\s+141,\s*142\)").expect("SBSE physics line pattern"))
}
fn sbse_supplemental_source_marker_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\[Supplemental official source\].*\bBSE\b").expect("SBSE supplemental source marker pattern"))
}
fn pathway_label_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:option|track|route|pathway|concentration)\b").expect("pathway label cue pattern"))
}
fn parallel_choose_one_section_boundary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:freshman|sophomore|junior|senior)\s+year\b|^major requirements?\b").expect("parallel choose one section boundary pattern"))
}
fn parallel_numbered_row_marker_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^\s*\d+\s*[).]\s+").expect("parallel numbered row marker pattern"))
}
fn parallel_choose_one_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(.+?)\s+(?:[-\x{2013}\x{2014}]|\S{1,4})?\s*choose\s+one\s+option:?$").expect("parallel choose one heading pattern"))
}
fn parallel_choose_one_heading_terminal_punctuation_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[:;.\-]+$").expect("parallel choose one heading punctuation pattern"))
}
fn parallel_heading_biochemistry_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\bbiochemistry\b").expect("parallel biochemistry heading pattern"))
}
fn parallel_heading_mathematics_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\bmathematics?\b").expect("parallel mathematics heading pattern"))
}
fn parallel_heading_statistics_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\bstatistics?\b").expect("parallel statistics heading pattern"))
}
fn parallel_heading_physics_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\bphysics?\b").expect("parallel physics heading pattern"))
}
fn parallel_heading_chemistry_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\bchemistry\b").expect("parallel chemistry heading pattern"))
}
fn parallel_heading_biology_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\bbiology\b").expect("parallel biology heading pattern"))
}
fn supplemental_official_source_marker_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\[Supplemental official source\]").expect("supplemental official source marker pattern"))
}
fn support_requirement_target_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:requirements?|option courses?|credits?)\b").expect("support requirement target line pattern"))
}
fn bare_sequence_course_number_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\b(\d{3}[A-Za-z]?)\b").expect("bare sequence course number pattern"))
}
fn sequence_lab_word_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\blabs?|laboratory\b").expect("sequence lab word pattern"))
}
fn conditional_lab_course_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\b(?:117|118|119)\b").expect("conditional lab course pattern"))
}
fn algebra_based_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\balgebra-based\b").expect("algebra-based cue pattern"))
}
fn lab_physics_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:physics|phys)\b").expect("lab physics cue pattern"))
}
fn lab_chemistry_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:chemistry|chem)\b").expect("lab chemistry cue pattern"))
}
fn lab_biology_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:biology|biol)\b").expect("lab biology cue pattern"))
}
fn lab_math_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:mathematics|math|calculus)\b").expect("lab math cue pattern"))
}
fn sequence_physics_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bphysics\b").expect("sequence physics cue pattern"))
}
fn sequence_chemistry_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bchemistry\b|\bchem\b").expect("sequence chemistry cue pattern"))
}
fn sequence_math_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bmathematics\b|\bmath\b|\bcalculus\b|\bstatistics\b").expect("sequence math cue pattern"))
}
fn sequence_biology_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bbiology\b|\bbiol\b").expect("sequence biology cue pattern"))
}
fn sequence_or_only_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^OR$").expect("sequence OR-only line pattern"))
}
fn sequence_heading_boundary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(super::requirement_shape::SEQUENCE_HEADING_BOUNDARY_PATTERN_SOURCE).expect("sequence heading boundary pattern"))
}
fn sequence_label_alternative_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(super::requirement_shape::SEQUENCE_LABEL_ALTERNATIVE_PATTERN_SOURCE).expect("sequence label alternative pattern"))
}
fn sequence_restriction_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:course restrictions?|exclude|excludes|excluded|CR/NC|independent study|internships?)\b").expect("sequence restriction context pattern"))
}
fn sequence_choice_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        let source = format!(
            "(?i)(?:{})|(?:{})|(?:{})",
            super::requirement_shape::SEQUENCE_CHOICE_CONTEXT_PATTERN_SOURCE,
            super::requirement_shape::SEQUENCE_LABEL_ALTERNATIVE_PATTERN_SOURCE,
            super::requirement_shape::SEQUENCE_ALTERNATIVE_CHOICE_LINE_PATTERN_SOURCE,
        );
        regex::Regex::new(&source).expect("sequence choice context pattern")
    })
}
fn parenthetical_number_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\(\s*\d+(?:\.\d+)?\s*\)").expect("parenthetical number pattern"))
}
fn requirement_section_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:requirements?|coursework requirements?|core courses?|degree requirements?)\b").expect("requirement section heading pattern"))
}
fn electives_list_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\belectives?\s+list\b").expect("electives list heading pattern"))
}
fn approved_courses_list_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bapproved courses?\s+list\b").expect("approved courses list heading pattern"))
}
fn split_choice_source_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bor\b").expect("split choice source pattern"))
}
fn split_choice_trailing_or_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bor\s*$").expect("split choice trailing OR pattern"))
}
fn embedded_alternative_required_list_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)inline required course list with embedded alternatives").expect("embedded alternative required list pattern"))
}
fn credit_bucket_explicit_choice_count_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(&format!(r"(?i)\b(?:choose|select)\s+(?:{CHOICE_COUNT_WORDS_PATTERN}|\d+)\b")).expect("credit bucket explicit choice count pattern"))
}
fn parenthetical_sequence_or_marker_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\(\s*or\s+").expect("parenthetical sequence OR marker pattern"))
}
fn parenthetical_sequence_choice_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)((?:[A-Z&]+(?:\s+[A-Z&]+)?\s+)?\d{3}[A-Za-z]?(?:\s*,\s*(?:[A-Z&]+(?:\s+[A-Z&]+)?\s+)?\d{3}[A-Za-z]?){1,5})\s*\(\s*or\s+([^)]+)\)").expect("parenthetical sequence choice pattern"))
}
fn credit_bucket_course_list_bucket_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\s+from\b|\bcredits?\b.{0,120}:\s*[A-Z]{2,8}&?\s+\d{3}").expect("credit bucket course list bucket pattern"))
}
fn credit_bucket_subject_elective_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\s+from\b.{0,120}\belectives?\b|\bcourses?\s+with\s+an?\b.{0,60}\bprefix\b|\bfree electives?\b|\b(?:credits?|\d+(?:\.\d+)?\s*-\s*credits?)\b.{0,120}\bintroductory\s+[A-Z]{2,8}\s+classes?\b").expect("credit bucket subject elective pattern"))
}
fn credit_bucket_language_level_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\s+(?:primary|secondary)?\s*language\b|\bcomplete\b.{0,100}\b(?:second|third|fourth)[-\s]?year\s+level\b").expect("credit bucket language level pattern"))
}
fn program_approved_word_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bapproved\b").expect("program approved word pattern"))
}
fn credit_bucket_leading_non_alphanumeric_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^[^A-Za-z0-9]+").expect("credit bucket leading non-alphanumeric pattern"))
}
fn credit_bucket_explicit_course_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:from|following:?)\s+([A-Za-z&]{2,8}|[A-Za-z&]{1,4}\s+[A-Za-z&]{1,4})\s+(\d{3}[A-Za-z]?)\b").expect("credit bucket explicit course pattern"))
}
fn additional_requirements_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:\[Page\s+\d+\]\s*)?Additional Requirements$").expect("additional requirements heading pattern"))
}
fn program_credit_bucket_continuation_stop_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bnatural science courses for\b|website").expect("program credit bucket continuation stop pattern"))
}
fn sibling_credit_bucket_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^[A-Za-z][A-Za-z/&\s-]{2,80}:\s+").expect("sibling credit bucket heading pattern"))
}
fn credit_bucket_semicolon_split_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\s*;\s*").expect("credit bucket semicolon split pattern"))
}
fn credit_bucket_and_course_prefix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^and\s+[A-Z]{2,8}&?\s+\d{3}").expect("credit bucket and course prefix pattern"))
}
fn credit_bucket_leading_and_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^and\s+").expect("credit bucket leading and pattern"))
}
fn credit_bucket_heading_prefix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^([^:]{3,90}):\s*").expect("credit bucket heading prefix pattern"))
}
fn subject_prefix_credit_bucket_patterns() -> [&'static regex::Regex; 4] {
    [
        subject_prefixed_courses_pattern(),
        subject_prefix_courses_pattern(),
        courses_with_subject_prefix_pattern(),
        from_subject_electives_pattern(),
    ]
}
fn subject_prefixed_courses_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b([A-Za-z&]{1,8}(?:\s+[A-Za-z&]{1,8})?)\s*-\s*prefixed\s+courses?\b").expect("subject prefixed courses pattern"))
}
fn subject_prefix_courses_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b([A-Za-z&]{1,8}(?:\s+[A-Za-z&]{1,8})?)\s+prefix\s+courses?\b").expect("subject prefix courses pattern"))
}
fn courses_with_subject_prefix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcourses?\s+with\s+an?\s+([A-Za-z&]{1,8}(?:\s+[A-Za-z&]{1,8})?)\s+prefix\b").expect("courses with subject prefix pattern"))
}
fn from_subject_electives_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bfrom\s+([A-Za-z&]{1,8}(?:\s+[A-Za-z&]{1,8})?)\s+electives?\b").expect("from subject electives pattern"))
}
fn program_courses_or_electives_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:electives?|courses?)\b").expect("program courses or electives pattern"))
}
fn program_elective_bucket_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b((?:approved\s+)?(?:advanced\s+)?(?:engineering|technical|departmental|major|science|natural science|program|concentration|option|upper[-\s]?division)\s+electives?(?:\s+credit)?|elective\s+credit)\b").expect("program elective bucket label pattern"))
}
fn program_elective_credit_only_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^elective\s+credit$").expect("program elective credit only pattern"))
}
fn program_electives_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\belectives?\s+credit\b").expect("program electives credit pattern"))
}
fn program_elective_word_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\belectives?\b").expect("program elective word pattern"))
}
fn program_free_electives_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bfree electives?\b").expect("program free electives pattern"))
}
fn program_approved_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bapproved\b").expect("program approved cue pattern"))
}
fn program_elective_credit_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:engineering|technical|departmental|major|science|natural science|program|concentration|option|upper[-\s]?division)\s+electives?(?:\s+credit)?\b|\belective\s+credit\b|\belectives?\b.{0,100}\bcredits?\s+(?:chosen\s+)?from\b|\bcredits?\s+(?:chosen\s+)?from\b.{0,100}\belectives?\b").expect("program elective credit cue pattern"))
}
fn program_natural_science_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bnatural sciences?\b").expect("program natural science pattern"))
}
fn program_math_science_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bmath(?:ematics)?/science\b|\bmath(?:ematics)?\s*&\s*science\b").expect("program math science pattern"))
}
fn program_elective_category_normalization_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[^A-Z0-9]+").expect("program elective category normalization pattern"))
}
fn program_elective_category(label: &str) -> String {
    let uppercase = label.to_ascii_uppercase();
    let normalized = program_elective_category_normalization_pattern()
        .replace_all(&uppercase, "_");
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "PROGRAM_ELECTIVE_CREDIT".to_string()
    } else {
        normalized.to_string()
    }
}
fn non_requirement_course_exclusion_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:will\s+not|cannot|can't|does\s+not|do\s+not|may\s+not|not)\s+(?:substitut(?:e|es?|ed|ing)|count|receive\s+credit|be\s+(?:used|applied)|satisfy|fulfill|meet|apply)\b|\b(?:do|does|will|can|could|may)\b.{0,80}\b(?:not|n't)\b.{0,80}\b(?:count|substitut(?:e|es?|ed|ing)|receive\s+credit|satisfy|fulfill|meet|apply)\b").expect("non-requirement course exclusion pattern"))
}
fn shared_humanities_social_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bArts?\s+(?:and|&)\s+Humanities\s+(?:and|&)\s+Social Sciences?\b").expect("shared humanities social credit pattern"))
}
fn shared_vlpa_is_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bVLPA\s+(?:and|&)\s+I\s*&\s*S\b").expect("shared VLPA I&S credit pattern"))
}
fn shared_long_category_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bVisual,\s*Literary,\s*and\s*Performing\s*Arts\b.{0,80}\bIndividuals\s+(?:and|&)\s+Societies\b").expect("shared long category credit pattern"))
}
fn following_humanities_minimum_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:minimum|no fewer than)\b.{0,80}\b(?:Arts?\s+(?:and|&)\s+Humanities|A\s*&\s*H|VLPA|Visual,\s*Literary,\s*and\s*Performing\s*Arts)\b").expect("following humanities minimum pattern"))
}
fn following_social_sciences_minimum_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:minimum|no fewer than)\b.{0,80}\b(?:Social Sciences?|S\s*Sc|I\s*&\s*S|Individuals\s+(?:and|&)\s+Societies)\b").expect("following social sciences minimum pattern"))
}
fn credit_bucket_exclusion_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:course restrictions?|exclude|excludes|excluded|CR/NC|independent study|internships?)\b").expect("credit bucket exclusion pattern"))
}
fn students_earn_remaining_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:students?\s+earn|remaining)\b.{0,120}\b(?:major\s+elective|elective)\s+credits?\b.{0,120}\bin\s+addition\s+to\b").expect("students earn remaining credit pattern"))
}
fn credit_bucket_elective_single_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\belective\b").expect("credit bucket elective single heading pattern"))
}
fn credit_bucket_elective_disqualifier_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:approved|from|of|list)\b").expect("credit bucket elective disqualifier pattern"))
}
fn next_line_course_list_delimiter_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[,;]").expect("next line course list delimiter pattern"))
}
fn approved_list_and_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bapproved\b.*\blist\b").expect("approved list and pattern"))
}
fn approved_list_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:from|approved|department[-\s]approved|see adviser|accordion|website)\b").expect("approved list context pattern"))
}
fn approved_list_from_placeholder_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bfrom\s+(?:the\s+)?approved\s+list\b").expect("approved list placeholder pattern"))
}
fn approved_list_adviser_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bsee adviser\b").expect("approved list adviser pattern"))
}
fn approved_department_list_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bdepartment[-\s]approved\s+lists?\b").expect("approved department list pattern"))
}
fn credit_bucket_overlap_note_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\([^)]*\b(?:cannot\s+overlap|also\s+fulfills?|fulfills?)\b[^)]*\)?").expect("credit bucket overlap note pattern"))
}
fn credit_bucket_overlap_tail_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:cannot\s+overlap\s+with|almost\s+always\s+also\s+fulfills?|also\s+fulfills?|fulfills?)\b.*$").expect("credit bucket overlap tail pattern"))
}
fn aggregate_general_education_credit_mentions_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b\d+(?:\.\d+)?\s+credits?\b").expect("aggregate general education credit mentions pattern"))
}
fn aggregate_general_education_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bgeneral education requirements?\b").expect("aggregate general education heading pattern"))
}
fn aggregate_general_education_includes_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:which includes|including|includes)\b").expect("aggregate general education includes pattern"))
}
fn composite_credit_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:Areas of Inquiry|Mathematics\s*&\s*Natural Sciences|Mathematics and Natural Sciences|General Education Component|Computer Engineering Component|Core and Electives|Fundamentals)\b").expect("composite credit heading pattern"))
}
fn category_credit_heading_prefix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:Diversity Requirement|Arts?\s+(?:and|&)\s+Humanities|Social Sciences?|Natural Sciences?|Natural World|Individuals\s+and\s+Societies|Quantitative and Symbolic Reasoning|\b(?:A\s*&\s*H|S\s*Sc|N\s*Sc|I\s*&\s*S|VLPA|DIV|QSR|NW)\b)").expect("category credit heading prefix pattern"))
}
fn category_credit_count_prefix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\d+(?:\.\d+)?\s+(?:additional\s+)?(?:diversity|arts?\s+(?:and|&)\s+humanities|social sciences?|natural sciences?|natural world|individuals\s+and\s+societies|quantitative and symbolic reasoning)\s+credits?\b").expect("category credit count prefix pattern"))
}
fn credit_bucket_category_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:approved|electives?|natural sciences?|social sciences?|arts?\s+(?:and|&)\s+humanities|humanities|diversity|quantitative and symbolic reasoning|natural world|individuals\s+and\s+societies|A\s*&\s*H|S\s*Sc|N\s*Sc|I\s*&\s*S|VLPA|DIV|QSR|NW|Math/Science|Math\s*&\s*Science)\b").expect("credit bucket category cue pattern"))
}
fn credit_bucket_elective_from_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\s+from\b.{0,120}\belectives?\b").expect("credit bucket elective from pattern"))
}
fn credit_bucket_prefixed_course_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\b.{0,160}\b(?:[A-Za-z&]{1,8}(?:\s+[A-Za-z&]{1,8})?\s*-\s*prefixed\s+courses?|[A-Za-z&]{1,8}(?:\s+[A-Za-z&]{1,8})?\s+prefix\s+courses?|courses?\s+with\s+an?\s+[A-Za-z&]{1,8}(?:\s+[A-Za-z&]{1,8})?\s+prefix)\b").expect("credit bucket prefixed course pattern"))
}
fn credit_bucket_credits_course_list_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\b.{0,120}:\s*[A-Z]{2,8}&?\s+\d{3}").expect("credit bucket credits course list pattern"))
}
fn credit_bucket_introductory_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:credits?|\d+(?:\.\d+)?\s*-\s*credits?)\b.{0,120}\bintroductory\s+[A-Z]{2,8}\s+classes?\b").expect("credit bucket introductory pattern"))
}
fn credit_bucket_language_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\s+(?:primary|secondary)?\s*language\b").expect("credit bucket language pattern"))
}
fn credit_bucket_year_level_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcomplete\b.{0,100}\b(?:second|third|fourth)[-\s]?year\s+level\b").expect("credit bucket year level pattern"))
}
fn credit_bucket_credit_from_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\s+from\b").expect("credit bucket credit from pattern"))
}
fn named_requirement_credit_summary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b\d+(?:\.\d+)?\s+credits?\s*\(\s*\d+\s+courses?(?:\s+total)?\s*\)\s+from\s+(?:core courses?|(?:the\s+)?(?:student.{0,12}|selected|chosen|their)\s+concentration\s+area\s+courses?)\b").expect("named requirement credit summary pattern"))
}
fn admission_decision_credit_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:admission decisions?|completion of .{0,80}requirements? does not guarantee admission|does not guarantee admission)\b").expect("admission decision credit cue pattern"))
}
fn admission_decision_quality_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:performance and potential|personal motivation|quality of relevant experience)\b").expect("admission decision quality pattern"))
}
fn elective_timing_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\belectives?\b").expect("elective timing pattern"))
}
fn elective_taken_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bmay be taken\b").expect("elective taken pattern"))
}
fn elective_sample_plan_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:designated|schedule permitting|sample plan|suggested)\b").expect("elective sample plan pattern"))
}
fn research_internship_service_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bresearch,\s*internships?,\s*and\s*service learning\b").expect("research internship service label pattern"))
}
fn research_opportunity_context_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:opportunities?|welcome|available|no formal internship|see adviser)\b").expect("research opportunity context pattern"))
}
fn inline_numbered_or_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bor\b").expect("inline numbered OR pattern"))
}
fn inline_numbered_required_exclusion_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:from|electives?|approved|minimum|maximum)\b").expect("inline numbered required exclusion pattern"))
}
fn inline_numbered_sequence_parenthetical_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\([^)]*\b(?:sequence|calculus-based|algebra-based|honors?|accelerated)\b[^)]*\)").expect("inline numbered sequence parenthetical pattern"))
}
fn inline_numbered_trailing_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\s*\(\s*\d+(?:\.\d+)?\s*credits?\s*\)\s*$").expect("inline numbered trailing credit pattern"))
}
fn choice_list_start_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(CHOICE_LIST_START_PATTERN_SOURCE).expect("choice list start pattern"))
}
fn choice_line_trailing_parenthesis_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\)\s*$").expect("choice line trailing parenthesis pattern"))
}
fn equivalent_number_alias_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:formerly|renumbered|equivalent|students can apply either)\b").expect("equivalent number alias cue pattern"))
}
fn parenthetical_or_course_alias_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\(\s*or\s+[A-Z]{2,8}\s+\d{3}").expect("parenthetical OR course alias pattern"))
}
fn slash_course_number_alias_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)[A-Z]{2,8}\s+\d{3}\s*/\s*\d{3}").expect("slash course number alias pattern"))
}
fn split_either_or_trailing_or_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bor\s*$").expect("split either-or trailing OR pattern"))
}
fn standalone_or_line_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^OR$").expect("standalone OR line pattern"))
}
fn starred_course_note_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:must|prereq|note|only)\b").expect("starred course note pattern"))
}
fn leading_credit_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\d+(?:\.\d+)?\s*credits?\s*:\s*").expect("leading course credit label pattern"))
}
fn leading_bracket_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\[[^\]]+\]\s*").expect("leading course bracket label pattern"))
}
fn comma_course_number_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r",\s*\d{3}[A-Za-z]?\b").expect("comma course number pattern"))
}
fn plain_credit_number_range_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^\d+(?:\s*(?:-|to)\s*\d+)?$").expect("plain credit number range pattern"))
}
fn course_row_parenthetical_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\(\s*(\d+(?:\.\d+)?)(?:\s*(credits?|cr\.?))?(?:\s*,\s*(?:A\s*&\s*H|S\s*Sc|N\s*Sc|DIV|W|C|RSN|QSR|VLPA|I\s*&\s*S|NW))*\s*\)(?!.*\))").expect("course-row parenthetical credit pattern"))
}
fn course_row_parenthetical_fallback_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\(\s*(\d+(?:\.\d+)?)(?:\s*(credits?|cr\.?))?(?:\s*;[^)]*)?\)").expect("course-row fallback parenthetical credit pattern"))
}
fn standalone_course_count_choice_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:choose|select)\s+(?:one|two|three|four|five|six|\d+)(?:\s+courses?)?\s+(?:of|from)\s+(?:the\s+)?(?:following|list\s+below)\b").expect("standalone course-count choice pattern"))
}
fn sectioned_category_min_max_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:a\s+)?(?:minimum|maximum)\s+of\s+\d+(?:\.\d+)?\s+credits?\b").expect("section category min/max pattern"))
}
fn sectioned_category_credit_amount_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b\d+(?:\.\d+)?\s+credits?(?:\s+total|\s+required)?\b").expect("section category credit pattern"))
}
fn sectioned_category_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:selected from the following list|courses? listed below|listed below are required|the following departments|will satisfy|requirement|requirements|required)\b").expect("section category cue pattern"))
}
fn trailing_category_punctuation_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[:.]+$").expect("section category punctuation pattern"))
}
fn sectioned_credit_range_part_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(\d+(?:\.\d+)?)(?:\s*(?:-|to)\s*(\d+(?:\.\d+)?))?$").expect("sectioned credit range part pattern"))
}
fn leading_named_elective_credit_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(\d+(?:\.\d+)?)\s+(?:free\s+)?electives?\b").expect("leading named elective credit pattern"))
}
fn simple_leading_parenthetical_credit_range_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\(\s*(\d+(?:\.\d+)?)(?:\s*(?:-|to)\s*(\d+(?:\.\d+)?))?\s*(?:credits?|cr\.?)\s*\)").expect("simple parenthetical credit range pattern"))
}
fn simple_leading_credit_range_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(\d+(?:\.\d+)?)(?:\s*(?:-|to)\s*(\d+(?:\.\d+)?))?\s*(?:credits?|cr\.?)(?:\s|$)").expect("simple leading credit range pattern"))
}
fn leading_parenthetical_credit_range_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\(?\s*(\d+(?:\.\d+)?)(?:\s*(?:[-–—]|to)\s*(\d+(?:\.\d+)?))?\s*(?:credits?|cr\.?)\s*\)?(?:\s|$)").expect("leading parenthetical credit range pattern"))
}
fn page_prefix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\[Page\s+\d+\]\s*").expect("page prefix pattern"))
}
fn direct_course_row_prefix_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^[-*\u{2610}\u{2611}\u{2612}\u{2713}\u{2714}\u{F071}\u{25A1}\u{25A0}\u{25AA}\u{25CF}\u{2022}\s]*(?:\(\s*\d+(?:\.\d+)?\s*\)\s*)?(?:[A-Z&]+(?:\s+[A-Z&]+)?(?:\s*/\s*[A-Z&]+(?:\s+[A-Z&]+)?)?\s+\d{3}[A-Z]?\*?|\d{3}[A-Z]?\b)").expect("direct course row prefix pattern"))
}
fn supplemental_source_boundary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\[Supplemental official source\]").expect("supplemental source boundary pattern"))
}
fn standalone_section_boundary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:B\.S\. degree requirements|Undergraduate advising|(?:[A-Z& ]+\s+)?Capstone|Tuition and scholarships|Contact\b|Log in|Privacy|Terms)$").expect("standalone section boundary pattern"))
}
fn admission_or_requirement_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:admission|requirements?)\b").expect("admission/requirement heading pattern"))
}
fn core_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcore\b").expect("core heading pattern"))
}
fn elective_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\belectives?\b").expect("elective heading pattern"))
}
fn credit_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\b").expect("credit heading pattern"))
}
fn core_or_elective_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:core|electives?)\b").expect("core/elective heading pattern"))
}
fn season_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:Autumn|Fall|Winter|Spring|Summer)(?:\s*&\s*(?:Autumn|Fall|Winter|Spring|Summer))*$").expect("season heading pattern"))
}
fn course_level_heading_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b\d{3}[-\s]*level\b").expect("course level heading pattern"))
}
fn immediate_section_table_header_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:Course|Courses|Topic|Credits?|Department|Dept\.?|TOTAL)$").expect("immediate section table header pattern"))
}
fn sectioned_explicit_course_count_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:choose|select)\s+(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\b").expect("sectioned explicit course count pattern"))
}
fn sectioned_leading_word_count_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(one|two|three|four|five|six|seven|eight|nine|ten)(?:\s*\(\s*(\d+)\s*\))?\s+.+?\b(?:courses?|electives?)\b").expect("sectioned leading word count pattern"))
}
fn sectioned_leading_numeric_electives_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^(\d+)\s+.+?\belectives?\b").expect("sectioned leading numeric elective count pattern"))
}
fn sectioned_credit_word_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bcredits?\b").expect("sectioned credit word pattern"))
}
fn sectioned_required_count_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:choose|select)\s+(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\b|^(?:one|two|three|four|five|six|seven|eight|nine|ten)(?:\s*\(\s*\d+\s*\))?\s+.+?\b(?:courses?|electives?)\b|^\d+\s+.+?\belectives?\b").expect("sectioned required count pattern"))
}
fn sectioned_course_list_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:selected from the following list|courses? listed below|listed below are required|the following departments|will satisfy|choose|select|following list)\b").expect("sectioned course list cue pattern"))
}
fn sectioned_course_list_topic_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:credits?|courses?|electives?|minimum|maximum)\b").expect("sectioned course list topic pattern"))
}
fn standalone_section_following_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:one|choose|select)\s+(?:course\s+)?(?:of|from)\s+(?:the\s+)?following\b").expect("standalone section choice pattern"))
}
fn formal_option_contact_advisor_boundary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^(?:formal\s+options?|contact\b|advisor\b)").expect("formal option/contact boundary pattern"))
}
fn academic_advising_progress_boundary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:academic\s+advisor|track\s+your\s+progress|declare\s+(?:the|your)\s+major)\b").expect("academic advising boundary pattern"))
}
fn max_degree_credit_constraint_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bmaximum\s+of\s+(\d+(?:\.\d+)?)\s+(?:credits?|cr\.?)\b.{0,50}\b(?:allowed|towards?\s+degree|toward\s+degree|count)\b").expect("maximum degree credit constraint pattern"))
}
fn prerequisite_title_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bprereq").expect("prerequisite title pattern"))
}
fn must_be_taken_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\bmust\s+be\s+taken\b").expect("must be taken pattern"))
}
fn nearby_course_title_exclusion_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:credits?|free electives?|requirements?|degree|minimum|maximum|department'?s approval|other disciplines|can be counted as|diversity|natural world|qsr)\b").expect("nearby course title exclusion pattern"))
}
fn starts_with_course_continuation_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)^\s*(?:or|and)\s+").expect("course continuation prefix pattern"))
}

fn prerequisite_parenthetical_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\([^)]*\bprereq(?:uisites?)?\b[^)]*\)").expect("prerequisite parenthetical pattern"))
}
fn leading_course_text_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^(.+?)(?::|\s+\()").expect("leading course text pattern"))
}
fn shared_joint_course_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\b([A-Z&]+(?:\s+[A-Z&]+)*)\s*/\s*([A-Z&]+(?:\s+[A-Z&]+)*)\s+(\d{3}[A-Za-z]?)\b").expect("shared-number joint course pattern"))
}
fn split_joint_course_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\b([A-Z&]+(?:\s+[A-Z&]+)*)\s+(\d{3}[A-Za-z]?)\s*/\s*([A-Z&]+(?:\s+[A-Z&]+)*)\s+(\d{3}[A-Za-z]?)\b").expect("split-number joint course pattern"))
}
fn trailing_asterisks_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\*+$").expect("trailing course asterisk pattern"))
}
fn alternative_course_title_boundary_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\s+or\s+(?=[A-Z])").expect("course title alternative boundary"))
}
fn next_course_number_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"\b\d{3}[A-Za-z]?\b").expect("next course number pattern"))
}
fn trailing_course_subject_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?:\s+(?:[A-Z&]{1,12}\s*){1,3})$").expect("trailing course subject pattern"))
}
fn leading_course_title_punctuation_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"^\s*[:\-–—]\s*").expect("leading course title punctuation pattern"))
}
fn trailing_credit_parenthetical_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"(?i)\s*\(\s*\d+(?:\.\d+)?(?:\s*(?:-|to)\s*\d+(?:\.\d+)?)?\s*(?:credits?|cr\.?)?\s*\)\s*$").expect("trailing credit parenthetical pattern"))
}

fn parenthetical_choice_group_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        regex::Regex::new(r"(?i)\s*\([^)]*\b(?:choose|select|or|(?:one|two|three|four|five|six|seven|eight|nine|ten)\s+of|one\s+of|one\s+from|one\s+course\s+from)\b")
            .expect("parenthetical choice group label pattern")
    })
}
fn choose_list_group_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        regex::Regex::new(r"(?i)\s*\(?\s*(?:choose(?:\s+(?:from|one|two|three|four|five|six|seven|eight|nine|ten|\d+))?|select|one\s+(?:course\s+)?from|one\s+of)\b")
            .expect("choose-list group label pattern")
    })
}
fn choice_requirement_label_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        regex::Regex::new(&format!("(?i){CHOICE_REQUIREMENT_LABEL_PATTERN_SOURCE}"))
            .expect("choice requirement label pattern")
    })
}
fn trailing_group_label_punctuation_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| regex::Regex::new(r"[:;,-]+$").expect("trailing group label punctuation pattern"))
}

fn normalize_category_option(raw: &Value) -> Value {
    let credits = nullish_value(raw.get("credits"));
    let credit_min = nullish_value(
        raw.get("creditMin")
            .filter(|value| !value.is_null())
            .or_else(|| raw.get("credits")),
    );
    let credit_max = nullish_value(
        raw.get("creditMax")
            .filter(|value| !value.is_null())
            .or_else(|| raw.get("credits")),
    );
    let program_specific = raw
        .get("programSpecific")
        .and_then(Value::as_bool)
        .map(Value::from)
        .unwrap_or(Value::Null);
    json!({
        "category": normalized_string_or_null(raw.get("category")),
        "sourceCategoryCode": normalized_string_or_null(raw.get("sourceCategoryCode")),
        "title": normalized_string_or_null(raw.get("title")),
        "credits": credits,
        "creditMin": credit_min,
        "creditMax": credit_max,
        "sourceText": normalized_string_or_null(raw.get("sourceText")),
        "approvedListKey": normalized_string_or_null(raw.get("approvedListKey")),
        "programSpecific": program_specific,
    })
}

fn credit_text(input: &Value) -> Value {
    let provided_credit_text = normalize_whitespace(&js_string_nullish(input.get("creditText")));
    if !provided_credit_text.is_empty() {
        return json!(provided_credit_text);
    }
    input
        .get("credits")
        .filter(|credits| !credits.is_null())
        .map(|credits| json!(js_string_value(credits)))
        .unwrap_or(Value::Null)
}

fn normalized_string_or_fallback(value: Option<&Value>, fallback: &str) -> Value {
    let normalized = normalize_whitespace(&js_string_nullish(value));
    json!(if normalized.is_empty() { fallback.to_string() } else { normalized })
}

fn normalize_text_array(value: Option<&Value>, sort: bool) -> Vec<String> {
    let values = array(value)
        .iter()
        .map(|item| normalize_whitespace(&js_string_nullish(Some(item))))
        .filter(|item| !item.is_empty())
        .collect();
    if sort {
        unique_sorted(values)
    } else {
        unique_in_order(values)
    }
}

fn boolean_or_null(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_bool)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn js_strict_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            matches!((left.as_f64(), right.as_f64()), (Some(left), Some(right)) if left == right)
        }
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

fn js_interpolate(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(value) => js_string_value(value),
    }
}

fn first_non_nullish_value(left: Option<&Value>, right: Option<&Value>) -> Value {
    left.filter(|value| !value.is_null())
        .or_else(|| right.filter(|value| !value.is_null()))
        .cloned()
        .unwrap_or(Value::Null)
}

fn normalized_string_or_default(value: Option<&Value>, default: &str) -> String {
    let normalized = normalize_whitespace(&js_string_nullish(value));
    if normalized.is_empty() {
        default.to_string()
    } else {
        normalized
    }
}

fn normalized_string_or_null(value: Option<&Value>) -> Value {
    let normalized = normalize_whitespace(&js_string_nullish(value));
    if normalized.is_empty() {
        Value::Null
    } else {
        json!(normalized)
    }
}

fn nullish_value(value: Option<&Value>) -> Value {
    value.filter(|value| !value.is_null()).cloned().unwrap_or(Value::Null)
}

fn array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn unique_sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    values.dedup();
    values
}

fn unique_in_order(values: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    values
        .into_iter()
        .filter(|value| {
            if seen.contains(value) {
                false
            } else {
                seen.push(value.clone());
                true
            }
        })
        .collect()
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn js_string_nullish(value: Option<&Value>) -> String {
    value
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_default()
}

fn js_string_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value
            .as_f64()
            .map(js_number_string)
            .unwrap_or_else(|| value.to_string()),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| if value.is_null() { String::new() } else { js_string_value(value) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

fn js_number_string(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}
