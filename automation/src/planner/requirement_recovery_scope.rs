use super::requirement_course_codes::CourseCodeParser;
use super::requirement_shape::{strip_leading_requirement_glyphs, strip_snapshot_page_prefix};
use super::requirement_snapshots::{RequirementSourceSnapshot, source_url_without_hash};
use super::transfer_plan_title_matching::{
    normalize_transfer_planner_semantic_pathway_label, normalize_transfer_planner_text,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;
use url::Url;

pub fn plan_has_pathway_manifest_entries(plan_id: Option<&Value>, manifest_entries: &[Value]) -> bool {
    let Some(plan_id) = plan_id.filter(|value| js_truthy(Some(*value))) else {
        return false;
    };
    manifest_entries.iter().any(|candidate| {
        candidate.get("planId") == Some(plan_id) && js_truthy(candidate.get("pathwayId"))
    })
}

pub fn parser_recovery_snapshot_conflicts_with_base_owner_scope(
    entry: &Value,
    snapshot_lines: &[String],
    manifest_entries: &[Value],
) -> bool {
    if js_truthy(entry.get("pathwayId"))
        || !plan_has_pathway_manifest_entries(entry.get("planId"), manifest_entries)
    {
        return false;
    }
    let supplemental_pathway_line_count = snapshot_lines
        .iter()
        .map(|line| normalize_whitespace(line))
        .filter(|line| {
            supplemental_official_source_pattern().is_match(line)
                && pathway_label_cue_pattern().is_match(line)
        })
        .count();
    supplemental_pathway_line_count >= 2
}

pub fn score_owner_snapshot_for_parser_recovery(
    entry: &Value,
    snapshot: &RequirementSourceSnapshot,
    course_parser: &CourseCodeParser,
) -> i32 {
    if snapshot.snapshot_lines.is_empty() {
        return 0;
    }
    let mut text_parts = Vec::with_capacity(2 + snapshot.snapshot_lines.len().min(140));
    if let Some(title) = snapshot.title.as_deref().filter(|value| !value.is_empty()) {
        text_parts.push(title);
    }
    if let Some(source_url) = snapshot.source_url.as_deref().filter(|value| !value.is_empty()) {
        text_parts.push(source_url);
    }
    text_parts.extend(snapshot.snapshot_lines.iter().take(140).map(String::as_str));
    let snapshot_text = normalize_matcher_text(&text_parts.join(" "));
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let title_tokens = parser_recovery_title_scope_tokens(entry);
    let course_code_count = snapshot
        .snapshot_lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .len() as i32;
    let cue_line_count = snapshot
        .snapshot_lines
        .iter()
        .filter(|line| requirement_cue_pattern().is_match(line))
        .count() as i32;
    let exact_title_score = if !exact_title.is_empty() && snapshot_text.contains(&exact_title) {
        80
    } else {
        0
    };
    let matched_title_token_count = title_tokens
        .iter()
        .filter(|token| snapshot_text.contains(token.as_str()))
        .count() as i32;
    let token_score = matched_title_token_count * 16;
    let source_url_score = match (
        snapshot.source_url.as_deref().and_then(source_url_without_hash),
        source_url_without_hash(&string_field(entry, "url")),
    ) {
        (Some(snapshot_url), Some(entry_url)) if snapshot_url == entry_url => 20,
        _ => 0,
    };
    let noisy_text = format!("{} {}", snapshot.title.as_deref().unwrap_or_default(), snapshot.source_url.as_deref().unwrap_or_default());
    let noisy_penalty = if graduate_supplemental_pattern().is_match(&noisy_text) {
        120
    } else {
        0
    };
    if source_url_score == 0
        && exact_title_score == 0
        && title_tokens.len() >= 2
        && matched_title_token_count < 2
    {
        return 0;
    }
    exact_title_score + token_score + course_code_count * 8 + cue_line_count * 4
        + source_url_score - noisy_penalty
}

pub fn parser_recovery_candidate_contains_selected_pathway_phrase(
    entry: &Value,
    label: &str,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !js_truthy(entry.get("pathwayId")) {
        return true;
    }
    let pathway_id = string_field(entry, "pathwayId");
    let pathway_phrase_tokens = normalize_matcher_text(&replace_path_separators(&pathway_id))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if pathway_phrase_tokens.is_empty() || !pathway_phrase_tokens.iter().any(|token| token.len() > 4) {
        return true;
    }
    let pathway_phrase = pathway_phrase_tokens.join(" ");
    let link_text = normalize_matcher_text(&format!("{label} {url}"));
    let compact_pathway_id = normalize_matcher_text(&pathway_id).replace(' ', "");
    let compact_link_text = link_text.replace(' ', "");
    let owner_acronym_match = parser_recovery_owner_program_acronym_tokens(
        entry,
        primary_major_titles,
    )
    .into_iter()
    .any(|token| {
        token.len() >= 3
            && link_text
                .split_whitespace()
                .any(|word| word == token.as_str())
    });
    link_text.contains(&pathway_phrase)
        || (!compact_pathway_id.is_empty() && compact_link_text.contains(&compact_pathway_id))
        || owner_acronym_match
}

pub fn scope_curriculum_era_lines(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let Some(legacy_start_index) = find_prior_admit_curriculum_start_index(lines, course_parser) else {
        return lines.to_vec();
    };
    if !is_prior_admit_curriculum_owner(entry) {
        let preceding_text = normalize_matcher_text(&lines[..legacy_start_index].join(" "));
        let has_current_curriculum_before_legacy =
            formal_options_pattern().is_match(&preceding_text)
                && (shared_curriculum_pattern().is_match(&preceding_text)
                    || beginning_spring_2026_pattern().is_match(&preceding_text)
                    || spring_2026_pattern().is_match(&preceding_text))
                && (foundation_courses_pattern().is_match(&preceding_text)
                    || methods_courses_pattern().is_match(&preceding_text)
                    || introductory_courses_pattern().is_match(&preceding_text)
                    || beginning_spring_2026_pattern().is_match(&preceding_text));
        return if has_current_curriculum_before_legacy {
            lines[..legacy_start_index].to_vec()
        } else {
            lines.to_vec()
        };
    }

    let mut legacy_end_index = lines.len().saturating_sub(1);
    for (index, line) in lines.iter().enumerate().skip(legacy_start_index + 1) {
        if graduation_or_contact_heading_pattern().is_match(&normalize_whitespace(line)) {
            legacy_end_index = index.saturating_sub(1).max(legacy_start_index);
            break;
        }
    }
    lines[legacy_start_index..=legacy_end_index].to_vec()
}

pub fn remove_following_different_owner_acronym_departmental_sections(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let owner_acronyms =
        parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles);
    if owner_acronyms.is_empty() {
        return lines.to_vec();
    }
    let mut found_owner_boundary = false;
    for (index, line) in lines.iter().enumerate() {
        let normalized_line = normalize_whitespace(line);
        let Some(captures) = departmental_degree_boundary_pattern().captures(&normalized_line) else {
            continue;
        };
        let Some(acronym) = captures.get(1) else {
            continue;
        };
        let boundary_acronym = normalize_matcher_text(acronym.as_str()).replace(' ', "");
        if boundary_acronym.is_empty() {
            continue;
        }
        if owner_acronyms.contains(&boundary_acronym) {
            found_owner_boundary = true;
        } else if found_owner_boundary {
            return lines[..index].to_vec();
        }
    }
    lines.to_vec()
}

pub fn has_owner_acronym_departmental_boundary(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let owner_acronyms =
        parser_recovery_owner_program_acronym_tokens(entry, primary_major_titles);
    !owner_acronyms.is_empty()
        && lines.iter().any(|line| {
            departmental_degree_boundary_pattern()
                .captures(&normalize_whitespace(line))
                .and_then(|captures| captures.get(1))
                .is_some_and(|acronym| {
                    owner_acronyms
                        .contains(&normalize_matcher_text(acronym.as_str()).replace(' ', ""))
                })
        })
}

pub fn is_html_section_boundary_line(line: &str) -> bool {
    html_section_boundary_pattern().is_match(line)
}

pub fn is_parser_recovery_pathway_section_boundary_line(line: &str) -> bool {
    pathway_section_boundary_pattern().is_match(&normalize_whitespace(line))
}

pub fn is_parser_recovery_section_boundary_line(line: &str) -> bool {
    html_section_boundary_pattern().is_match(line)
        || generic_section_boundary_pattern().is_match(line)
        || is_parser_recovery_pathway_section_boundary_line(line)
}

pub fn has_parser_recovery_requirement_cue(text: &str) -> bool {
    requirement_cue_pattern().is_match(text)
}

pub fn find_preceding_inline_requirement_start_index(
    entry: &Value,
    lines: &[String],
    scoped_start_index: usize,
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    if entry.is_null() || scoped_start_index == 0 || scoped_start_index >= lines.len() {
        return None;
    }
    let min_index = scoped_start_index.saturating_sub(8);
    let mut start_index = None;
    for index in (min_index..scoped_start_index).rev() {
        let line = strip_choice_list_line(&lines[index]);
        if looks_like_preceding_inline_requirement_line(&line, course_parser) {
            start_index = Some(index);
            continue;
        }
        if start_index.is_some() {
            break;
        }
        if line.is_empty() {
            continue;
        }
        if html_section_boundary_pattern().is_match(&line)
            || primary_requirement_line_pattern().is_match(&line)
            || pathway_section_boundary_pattern_source().is_match(&line)
            || preceding_inline_hard_boundary_pattern().is_match(&line)
        {
            break;
        }
    }
    start_index
}

pub fn has_upcoming_sectioned_course_rows(
    lines: &[String],
    start_index: usize,
    minimum_count: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let mut course_row_count = 0;
    let scan_end = lines.len().min(start_index.saturating_add(95));
    for index in start_index.saturating_add(1)..scan_end {
        let line = strip_choice_list_line(&lines[index]);
        if line.is_empty() {
            continue;
        }
        if is_sectioned_option_course_boundary(&line, course_parser) {
            break;
        }
        if sectioned_course_column_header_pattern().is_match(&line) {
            continue;
        }
        let course_line = sectioned_option_course_line(&line);
        if course_line.is_none()
            && course_row_count == 0
            && index > start_index.saturating_add(1)
            && (has_primary_requirement_section_cue(&line)
                || looks_like_sectioned_course_heading_line(&line, course_parser)
                || looks_like_short_named_course_section_heading(&line, course_parser))
        {
            break;
        }
        if course_line.is_some() {
            course_row_count += 1;
            if course_row_count >= minimum_count {
                return true;
            }
        }
    }
    false
}

pub fn remove_parallel_pathway_html_table_content_from_base_scope(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if js_truthy(entry.get("pathwayId")) || lines.len() < 8 {
        return lines.to_vec();
    }

    let mut remove_ranges = find_base_formal_options_removal_ranges(entry, lines, course_parser);
    for index in 0..lines.len() {
        if !course_parser.extract_from_line(&lines[index]).is_empty()
            || !is_likely_base_parallel_pathway_html_section_start_line(lines, index, course_parser)
        {
            continue;
        }
        let mut run_end = index;
        while run_end + 1 < lines.len()
            && course_parser.extract_from_line(&lines[run_end + 1]).is_empty()
            && is_likely_base_parallel_pathway_html_section_start_line(
                lines,
                run_end + 1,
                course_parser,
            )
        {
            run_end += 1;
        }
        if run_end == index {
            continue;
        }

        let content_start = run_end + 1;
        let mut content_end = lines.len().saturating_sub(1).min(content_start + 220);
        for scan_index in content_start.saturating_add(1)..=content_end {
            let line = normalize_whitespace(&lines[scan_index]);
            if scan_index > content_start + 2
                && !line.is_empty()
                && course_parser.extract_from_line(&line).is_empty()
                && pathway_html_table_content_end_boundary_pattern().is_match(&line)
            {
                content_end = scan_index - 1;
                break;
            }
        }
        let removed_course_count = lines[index..=content_end]
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if removed_course_count > 0 {
            remove_ranges.push((index, content_end));
        }
    }

    for index in 0..lines.len() {
        let line = normalize_whitespace(&lines[index]);
        if !course_parser.extract_from_line(&line).is_empty()
            || !is_likely_base_parallel_pathway_html_section_start_line(lines, index, course_parser)
        {
            continue;
        }
        let mut content_end = lines.len().saturating_sub(1).min(index + 220);
        for scan_index in index.saturating_add(1)..=content_end {
            let scan_line = normalize_whitespace(&lines[scan_index]);
            if scan_line.is_empty() {
                continue;
            }
            if scan_index > index + 2
                && is_likely_base_parallel_pathway_html_section_start_line(
                    lines,
                    scan_index,
                    course_parser,
                )
            {
                content_end = scan_index - 1;
                break;
            }
            if scan_index > index + 2
                && course_parser.extract_from_line(&scan_line).is_empty()
                && base_scope_pathway_section_end_boundary_pattern().is_match(&scan_line)
            {
                content_end = scan_index - 1;
                break;
            }
        }
        let removed_course_count = lines[index..=content_end]
            .iter()
            .map(|removed_line| course_parser.extract_from_line(removed_line).len())
            .sum::<usize>();
        if removed_course_count > 0 {
            remove_ranges.push((index, content_end));
        }
    }

    if remove_ranges.is_empty() {
        return lines.to_vec();
    }
    lines.iter()
        .enumerate()
        .filter(|(line_index, _)| {
            remove_ranges.iter().all(|(start_index, end_index)| {
                *line_index < *start_index || *line_index > *end_index
            })
        })
        .map(|(_, line)| line.clone())
        .collect()
}

fn find_base_formal_options_removal_ranges(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<(usize, usize)> {
    if js_truthy(entry.get("pathwayId")) || lines.len() < 8 {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    for index in 0..lines.len() {
        if !is_formal_options_base_section_start_line(&lines[index], course_parser) {
            continue;
        }
        let intro_text = normalize_matcher_text(
            &lines[index..lines.len().min(index + 18)].join(" "),
        );
        if !choose_one_formal_options_pattern().is_match(&intro_text) {
            continue;
        }
        let option_end = lines.len().min(index + 40);
        let option_heading_indexes = (index + 1..option_end)
            .filter(|line_index| is_formal_option_heading_line(&lines[*line_index], course_parser))
            .collect::<Vec<_>>();
        if option_heading_indexes.len() < 2
            || option_heading_indexes.first().is_some_and(|first| *first > index + 12)
        {
            continue;
        }

        let mut end_index = lines.len().saturating_sub(1).min(index + 220);
        for scan_index in index + 3..=end_index {
            let scan_line = strip_choice_list_line(&lines[scan_index]);
            if course_parser.extract_from_line(&scan_line).is_empty()
                && formal_options_end_boundary_pattern().is_match(&scan_line)
            {
                end_index = scan_index - 1;
                break;
            }
        }
        let removed_course_count = lines[index..=end_index]
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if removed_course_count > 0 {
            ranges.push((index, end_index));
        }
    }
    ranges
}

fn is_formal_options_base_section_start_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    formal_options_heading_pattern().is_match(&strip_choice_list_line(line))
        && course_parser.extract_from_line(line).is_empty()
}

fn is_formal_option_heading_line(line: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty() || !course_parser.extract_from_line(&normalized_line).is_empty() {
        return false;
    }
    formal_option_enumerated_heading_pattern().is_match(&normalized_line)
        || (normalized_line.split_whitespace().count() <= 8
            && formal_option_cue_pattern().is_match(&normalized_line)
            && !formal_option_action_pattern().is_match(&normalized_line))
}

fn is_likely_base_parallel_pathway_html_section_start_line(
    lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    is_likely_base_peer_pathway_html_section_start_line(&lines[index], course_parser)
        || is_likely_formal_pathway_table_heading_line(lines, index, course_parser)
}

fn is_likely_base_peer_pathway_html_section_start_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized = strip_pathway_heading_prefix(line);
    is_likely_peer_pathway_html_section_start_line(&normalized, course_parser)
        && pathway_label_cue_pattern().is_match(&normalized)
}

pub fn is_likely_peer_pathway_html_section_start_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized = strip_pathway_heading_prefix(line);
    let matcher_text = normalize_matcher_text(&normalized);
    if matcher_text.is_empty()
        || course_list_section_heading_pattern().is_match(&matcher_text)
        || noisy_inline_line_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&normalized)
    {
        return false;
    }
    let word_count = matcher_text.split_whitespace().count();
    if word_count > 12
        || peer_pathway_prose_rejection_pattern().is_match(&normalized)
    {
        return false;
    }
    let starts_with_enumerator = pathway_heading_enumerator_pattern()
        .is_match(&normalize_transfer_planner_text(line));
    let has_pathway_cue = pathway_label_cue_pattern().is_match(&normalized);
    let is_uppercase_heading = normalized.chars().any(|character| character.is_ascii_uppercase())
        && normalized == normalized.to_uppercase();
    has_pathway_cue || starts_with_enumerator || (is_uppercase_heading && word_count <= 8)
}

fn has_recent_formal_pathway_choice_context(lines: &[String], index: usize) -> bool {
    let start = index.saturating_sub(8);
    formal_options_context_pattern().is_match(&normalize_matcher_text(&lines[start..index].join(" ")))
}

fn is_likely_formal_pathway_table_heading_line(
    lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let raw_line = normalize_whitespace(&lines[index]);
    let normalized = strip_pathway_heading_prefix(&raw_line);
    let matcher_text = normalize_matcher_text(&normalized);
    if raw_line.is_empty()
        || matcher_text.is_empty()
        || !has_recent_formal_pathway_choice_context(lines, index)
        || noisy_inline_line_pattern().is_match(&raw_line)
        || !course_parser.extract_from_line(&raw_line).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&raw_line)
    {
        return false;
    }
    let word_count = matcher_text.split_whitespace().count();
    word_count > 0
        && word_count <= 8
        && pathway_heading_enumerator_pattern().is_match(&raw_line)
}

pub fn find_pathway_html_curriculum_section_candidate(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let mut candidates = Vec::new();
    let mut selected_start_indexes = Vec::new();
    let mut has_sibling_pathway_section = false;
    for (index, line) in lines.iter().enumerate() {
        let selected = is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        let peer = is_pathway_html_section_sibling_start(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        if selected {
            selected_start_indexes.push(index);
        } else if peer {
            has_sibling_pathway_section = true;
        }
    }
    if selected_start_indexes.is_empty() || !has_sibling_pathway_section {
        return None;
    }

    for selected_start_index in selected_start_indexes.iter().copied().filter(|index| {
        pathway_curriculum_or_requirements_pattern()
            .is_match(&strip_pathway_heading_prefix(&lines[*index]))
    }) {
        let end_index = find_pathway_html_section_end_index(
            entry,
            lines,
            selected_start_index,
            primary_major_titles,
            course_parser,
        );
        let section_lines = &lines[selected_start_index..=end_index];
        let course_count = section_lines
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if course_count == 0 {
            continue;
        }
        let start_index = find_pathway_html_section_start_index(
            entry,
            lines,
            selected_start_index,
            course_parser,
        );
        candidates.push((course_count, selected_start_index, start_index, end_index));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    candidates.first().map(|candidate| (candidate.2, candidate.3))
}

pub fn find_pathway_html_selected_section_candidate(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let mut selected_start_indexes = Vec::new();
    let mut has_sibling_pathway_section = false;
    for (index, line) in lines.iter().enumerate() {
        let selected = is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        let peer = is_pathway_html_section_sibling_start(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        if selected {
            selected_start_indexes.push(index);
        } else if peer {
            has_sibling_pathway_section = true;
        }
    }
    if selected_start_indexes.is_empty() || !has_sibling_pathway_section {
        return None;
    }
    let mut candidates = Vec::<(usize, usize, usize, usize)>::new();
    for selected_start_index in selected_start_indexes {
        let end_index = find_pathway_html_section_end_index(
            entry,
            lines,
            selected_start_index,
            primary_major_titles,
            course_parser,
        );
        let course_count = lines[selected_start_index..=end_index]
            .iter()
            .map(|line| course_parser.extract_from_line(line).len())
            .sum::<usize>();
        if course_count == 0 {
            continue;
        }
        let start_index = find_pathway_html_section_start_index(
            entry,
            lines,
            selected_start_index,
            course_parser,
        );
        candidates.push((course_count, selected_start_index, start_index, end_index));
    }
    candidates.sort_by(|left, right| {
        right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1))
    });
    candidates.first().map(|candidate| (candidate.2, candidate.3))
}

pub fn find_parallel_pathway_html_table_scope(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize, Vec<String>)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let mut peer_indexes = Vec::<(usize, bool)>::new();
    let mut peer_index_set = HashSet::new();
    for (index, line) in lines.iter().enumerate() {
        let selected = is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        let peer = is_pathway_html_section_sibling_start(
            entry,
            line,
            primary_major_titles,
            course_parser,
        );
        if selected || peer {
            peer_indexes.push((index, selected));
            if course_parser.extract_from_line(line).is_empty() {
                peer_index_set.insert(index);
            }
        }
    }
    if !peer_indexes.iter().any(|(_, selected)| !selected) {
        return None;
    }

    for selected_start_index in peer_indexes
        .iter()
        .filter_map(|(index, selected)| (*selected).then_some(*index))
    {
        if !peer_index_set.contains(&selected_start_index) {
            continue;
        }
        let mut run_start = selected_start_index;
        while run_start > 0
            && (peer_index_set.contains(&(run_start - 1))
                || is_parallel_pathway_heading_bridge_line(
                    lines.get(run_start - 1).map(String::as_str),
                    course_parser,
                ))
        {
            run_start -= 1;
        }
        let mut run_end = selected_start_index;
        while run_end + 1 < lines.len()
            && (peer_index_set.contains(&(run_end + 1))
                || is_parallel_pathway_heading_bridge_line(
                    lines.get(run_end + 1).map(String::as_str),
                    course_parser,
                ))
        {
            run_end += 1;
        }
        let run_indexes = (run_start..=run_end)
            .filter(|index| peer_index_set.contains(index))
            .collect::<Vec<_>>();
        if run_indexes.len() < 2 {
            continue;
        }
        let Some(selected_position) = run_indexes
            .iter()
            .position(|index| *index == selected_start_index)
        else {
            continue;
        };
        let content_start = run_end + 1;
        let selected_heading = &lines[selected_start_index];
        let later_sibling_headings = run_indexes
            .iter()
            .skip(selected_position + 1)
            .map(|index| lines[*index].clone())
            .collect::<Vec<_>>();
        let mut start_index = content_start;
        if selected_position > 0 {
            let Some(repeated_selected_heading_index) = find_pathway_html_table_content_boundary(
                lines,
                content_start,
                std::slice::from_ref(selected_heading),
                course_parser,
            ) else {
                continue;
            };
            start_index = repeated_selected_heading_index;
        }
        let next_sibling_content_index = if later_sibling_headings.is_empty() {
            None
        } else {
            find_pathway_html_table_content_boundary(
                lines,
                content_start,
                &later_sibling_headings,
                course_parser,
            )
        };
        if start_index >= lines.len() {
            continue;
        }
        let default_end_index = find_pathway_html_table_content_end_index(
            entry,
            lines,
            start_index,
            primary_major_titles,
            course_parser,
        );
        let end_index = next_sibling_content_index
            .filter(|next_index| *next_index > start_index)
            .map(|next_index| default_end_index.min(next_index - 1))
            .unwrap_or(default_end_index);
        let selected_heading_bridge_lines = if selected_position < run_indexes.len() - 1 {
            lines[selected_start_index + 1..run_indexes[selected_position + 1]]
                .iter()
                .filter(|line| {
                    is_parallel_pathway_heading_bridge_line(Some(line.as_str()), course_parser)
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut seen = HashSet::new();
        let section_lines = std::iter::once(selected_heading.to_string())
            .chain(selected_heading_bridge_lines)
            .chain(lines[start_index..=end_index].iter().cloned())
            .filter(|line| seen.insert(line.clone()))
            .collect::<Vec<_>>();
        if section_lines
            .iter()
            .any(|line| !course_parser.extract_from_line(line).is_empty())
        {
            return Some((start_index, end_index, section_lines));
        }
    }
    None
}

fn find_pathway_html_table_content_boundary(
    lines: &[String],
    start_index: usize,
    heading_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    if lines.is_empty() || start_index >= lines.len() {
        return None;
    }
    let forward_limit = lines.len().saturating_sub(1).min(start_index + 220);
    for index in start_index..=forward_limit {
        let line = normalize_whitespace(&lines[index]);
        if line.is_empty()
            || !course_parser.extract_from_line(&line).is_empty()
            || pathway_section_boundary_pattern_source().is_match(&line)
        {
            continue;
        }
        let likely_heading = is_likely_peer_pathway_html_section_start_line(&line, course_parser)
            || is_likely_bare_pathway_html_section_heading(&line, course_parser);
        if !likely_heading {
            let normalized_line = normalize_transfer_planner_text(&line);
            let word_count = normalize_matcher_text(&line).split_whitespace().count();
            if word_count > 0
                && word_count <= 14
                && inline_repeated_pathway_heading_pattern().is_match(&normalized_line)
                && heading_lines.iter().any(|heading_line| {
                    line_matches_pathway_heading_identity(heading_line, &line)
                })
            {
                return Some(index);
            }
            continue;
        }
        if heading_lines
            .iter()
            .any(|heading_line| line_matches_pathway_heading_identity(heading_line, &line))
        {
            return Some(index);
        }
    }
    None
}

fn line_matches_pathway_heading_identity(heading_line: &str, candidate_line: &str) -> bool {
    let tokens = normalize_matcher_text(&strip_pathway_heading_prefix(heading_line))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let normalized_candidate = normalize_matcher_text(candidate_line);
    if tokens.is_empty() || normalized_candidate.is_empty() {
        return false;
    }
    let token_matches = tokens
        .iter()
        .filter(|token| normalized_line_contains_pathway_token(&normalized_candidate, token))
        .count();
    let required_matches = if tokens.len() <= 2 { tokens.len() } else { 2 };
    if token_matches >= required_matches.max(1) {
        return true;
    }
    let heading_acronyms = pathway_heading_acronym_pattern()
        .find_iter(&strip_pathway_heading_prefix(heading_line))
        .map(|matched| matched.as_str().to_string())
        .filter(|token| !pathway_heading_excluded_acronym_pattern().is_match(token))
        .collect::<HashSet<_>>();
    pathway_heading_acronym_pattern()
        .find_iter(&strip_pathway_heading_prefix(candidate_line))
        .any(|matched| heading_acronyms.contains(matched.as_str()))
}

fn is_parallel_pathway_heading_bridge_line(
    line: Option<&str>,
    course_parser: &CourseCodeParser,
) -> bool {
    let Some(line) = line else {
        return false;
    };
    let normalized_line = strip_choice_list_line(line);
    !normalized_line.is_empty()
        && course_parser.extract_from_line(&normalized_line).is_empty()
        && normalized_line.split_whitespace().count() <= 14
        && parallel_pathway_heading_bridge_pattern().is_match(&normalized_line)
}

fn find_pathway_html_table_content_end_index(
    entry: &Value,
    lines: &[String],
    start_index: usize,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> usize {
    let default_end_index = find_pathway_html_section_end_index(
        entry,
        lines,
        start_index,
        primary_major_titles,
        course_parser,
    );
    for index in start_index + 1..=default_end_index {
        let line = normalize_whitespace(&lines[index]);
        if index > start_index + 2
            && !line.is_empty()
            && course_parser.extract_from_line(&line).is_empty()
            && pathway_html_table_content_end_boundary_pattern().is_match(&line)
        {
            return index.saturating_sub(1).max(start_index);
        }
    }
    default_end_index
}

fn inline_repeated_pathway_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i):\s*\b(?:complete|choose|select|required|courses?\s+listed|all\s+\d+\s+courses?)\b").expect("inline repeated pathway heading"))
}

fn pathway_heading_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b[A-Z]{2,6}\b").expect("pathway heading acronym"))
}

fn pathway_heading_excluded_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:BA|BS|BABA)$").expect("pathway heading excluded acronym"))
}

fn parallel_pathway_heading_bridge_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:choose|complete|select)\b.*\b(?:courses?|credits?|following|below)\b").expect("parallel pathway heading bridge"))
}

pub fn find_nested_pathway_html_section_scope(
    entry: &Value,
    lines: &[String],
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize, Vec<String>)> {
    if !js_truthy(entry.get("pathwayId")) || lines.len() < 4 {
        return None;
    }
    let peer_indexes = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let selected = is_likely_selected_pathway_html_section_start_line(
                entry,
                line,
                primary_major_titles,
                course_parser,
            );
            let peer = is_pathway_html_section_sibling_start(
                entry,
                line,
                primary_major_titles,
                course_parser,
            );
            (selected || peer).then_some((index, selected))
        })
        .collect::<Vec<_>>();
    if !peer_indexes.iter().any(|(_, selected)| !selected) {
        return None;
    }
    let strict_selected_start_indexes = peer_indexes
        .iter()
        .filter_map(|candidate| {
            let index = candidate.0;
            (candidate.1
                && line_matches_strict_selected_pathway_identity(
                    entry,
                    &lines[index],
                    primary_major_titles,
                ))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    for selected_start_index in strict_selected_start_indexes {
        let selected_end_index = find_pathway_html_section_end_index(
            entry,
            lines,
            selected_start_index,
            primary_major_titles,
            course_parser,
        );
        let selected_lines = lines[selected_start_index..=selected_end_index].to_vec();
        if !selected_lines
            .iter()
            .any(|line| !course_parser.extract_from_line(line).is_empty())
        {
            continue;
        }
        for parent_start_index in
            (selected_start_index.saturating_sub(220)..selected_start_index).rev()
        {
            let parent_line = &lines[parent_start_index];
            let is_loose_selected_parent = (is_likely_selected_pathway_html_section_start_line(
                entry,
                parent_line,
                primary_major_titles,
                course_parser,
            ) || line_matches_pathway_parent_context(entry, parent_line)) && !is_pathway_html_section_sibling_start(
                entry,
                parent_line,
                primary_major_titles,
                course_parser,
            ) && !line_matches_strict_selected_pathway_identity(
                entry,
                parent_line,
                primary_major_titles,
            );
            if !is_loose_selected_parent {
                continue;
            }
            let first_nested_peer = peer_indexes
                .iter()
                .map(|(index, _)| *index)
                .filter(|index| *index > parent_start_index)
                .min();
            let parent_end_index = first_nested_peer
                .filter(|index| *index <= selected_start_index)
                .map(|index| index - 1)
                .unwrap_or(selected_start_index - 1);
            let parent_lines = &lines[parent_start_index..=parent_end_index];
            if !parent_lines
                .iter()
                .any(|line| !course_parser.extract_from_line(line).is_empty())
            {
                continue;
            }
            let mut seen = HashSet::new();
            let scoped_lines = parent_lines
                .iter()
                .chain(selected_lines.iter())
                .filter_map(|line| seen.insert(line.clone()).then_some(line.clone()))
                .collect();
            return Some((parent_start_index, selected_end_index, scoped_lines));
        }
    }
    None
}

fn line_matches_strict_selected_pathway_identity(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty() {
        return false;
    }
    let discriminator_tokens = selected_pathway_discriminator_tokens(entry, primary_major_titles);
    if !discriminator_tokens.is_empty()
        && !discriminator_tokens
            .iter()
            .any(|token| normalized_line_contains_pathway_token(&normalized_line, token))
    {
        return false;
    }
    strict_selected_pathway_scope_token_groups(entry, primary_major_titles)
        .iter()
        .any(|tokens| {
            let matches = tokens
                .iter()
                .filter(|token| normalized_line_contains_pathway_token(&normalized_line, token))
                .count();
            matches >= (if tokens.len() <= 2 { tokens.len() } else { 2 }).max(1)
        })
}

fn strict_selected_pathway_scope_token_groups(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<Vec<String>> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let major_title = primary_major_title(entry, primary_major_titles);
    let raw_labels = [
        replace_path_separators(&string_field(entry, "pathwayId")),
        normalize_transfer_planner_semantic_pathway_label(
            &major_title,
            &string_field(entry, "ownerTitle"),
        ),
    ];
    let mut groups = raw_labels
        .iter()
        .map(|label| strip_pathway_heading_prefix(label))
        .map(|label| {
            normalize_matcher_text(&label)
                .split_whitespace()
                .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|tokens| !tokens.is_empty())
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| right.len().cmp(&left.len()));
    groups.dedup();
    groups
}

fn line_matches_pathway_parent_context(entry: &Value, line: &str) -> bool {
    if !js_truthy(entry.get("pathwayId")) {
        return false;
    }
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty() || !pathway_label_cue_pattern().is_match(&normalize_transfer_planner_text(line)) {
        return false;
    }
    let owner_title = string_field(entry, "ownerTitle");
    let owner_title_prefix = owner_title_prefix_pattern()
        .split(&owner_title)
        .next()
        .unwrap_or(&owner_title)
        .to_string();
    let parent_labels = [
        string_field(entry, "sourceLabel"),
        string_field(entry, "label"),
        owner_title_prefix,
    ];
    parent_labels.iter().any(|label| {
        let tokens = normalize_matcher_text(&strip_pathway_heading_prefix(label))
            .split_whitespace()
            .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
            .map(str::to_string)
            .collect::<Vec<_>>();
        !tokens.is_empty()
            && tokens
                .iter()
                .filter(|token| normalized_line_contains_pathway_token(&normalized_line, token))
                .count()
                >= (if tokens.len() <= 2 { tokens.len() } else { 2 }).max(1)
    })
}

fn is_likely_selected_pathway_html_section_start_line(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    if !line_matches_selected_pathway_identity(entry, line, primary_major_titles)
        || !line_contains_selected_pathway_discriminator(entry, line, primary_major_titles)
    {
        return false;
    }
    let normalized = strip_pathway_heading_prefix(line);
    let matcher_text = normalize_matcher_text(&normalized);
    if matcher_text.is_empty()
        || noisy_inline_line_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&normalized)
    {
        return false;
    }
    let word_count = matcher_text.split_whitespace().count();
    if word_count > 12 {
        return false;
    }
    let starts_with_enumerator = pathway_heading_enumerator_pattern()
        .is_match(&normalize_transfer_planner_text(line));
    let has_pathway_cue = pathway_label_cue_pattern().is_match(&normalized);
    let is_uppercase_heading = normalized.chars().any(|character| character.is_ascii_uppercase())
        && normalized == normalized.to_uppercase();
    has_pathway_cue
        || starts_with_enumerator
        || is_uppercase_heading
        || word_count <= 6
        || is_likely_bare_pathway_html_section_heading(&normalized, course_parser)
}

pub fn is_pathway_html_section_sibling_start(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    (is_likely_peer_pathway_html_section_start_line(line, course_parser)
        || (is_likely_bare_pathway_html_section_heading(line, course_parser)
            && !line_matches_selected_pathway_identity(entry, line, primary_major_titles)))
        && !is_likely_selected_pathway_html_section_start_line(
            entry,
            line,
            primary_major_titles,
            course_parser,
        )
}

fn is_likely_bare_pathway_html_section_heading(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized = strip_pathway_heading_prefix(line);
    let matcher_text = normalize_matcher_text(&normalized);
    if matcher_text.is_empty()
        || course_list_section_heading_pattern().is_match(&matcher_text)
        || noisy_inline_line_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
        || pathway_section_boundary_pattern_source().is_match(&normalized)
        || pathway_label_cue_pattern().is_match(&normalized)
        || bare_pathway_subheading_pattern().is_match(&normalized)
        || normalized.ends_with('.')
        || normalized.ends_with(';')
        || normalized.ends_with(':')
    {
        return false;
    }
    let words = matcher_text.split_whitespace().count();
    if words == 0 || words > 5 {
        return false;
    }
    let meaningful_words = normalized
        .split_whitespace()
        .filter(|word| !bare_pathway_filler_word_pattern().is_match(word))
        .collect::<Vec<_>>();
    !meaningful_words.is_empty()
        && meaningful_words
            .iter()
            .all(|word| bare_pathway_visible_word_pattern().is_match(word))
}

pub fn line_matches_selected_pathway_identity(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty() {
        return false;
    }
    let discriminator_tokens = selected_pathway_discriminator_tokens(entry, primary_major_titles);
    if !discriminator_tokens.is_empty()
        && !discriminator_tokens
            .iter()
            .any(|token| normalized_line_contains_pathway_token(&normalized_line, token))
    {
        return false;
    }
    selected_pathway_scope_token_groups(entry, primary_major_titles)
        .iter()
        .any(|tokens| {
            let matches = tokens
                .iter()
                .filter(|token| normalized_line_contains_pathway_token(&normalized_line, token))
                .count();
            let required = if tokens.len() <= 2 { tokens.len() } else { 2 };
            matches >= required.max(1)
        })
}

fn line_contains_selected_pathway_discriminator(
    entry: &Value,
    line: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    let discriminator_tokens = selected_pathway_discriminator_tokens(entry, primary_major_titles);
    discriminator_tokens.is_empty()
        || {
            let normalized_line = normalize_matcher_text(line);
            discriminator_tokens
                .iter()
                .any(|token| normalized_line_contains_pathway_token(&normalized_line, token))
        }
}

fn normalized_line_contains_pathway_token(normalized_line: &str, token: &str) -> bool {
    Regex::new(&format!(r"(?i)\b{}\b", regex::escape(token)))
        .is_ok_and(|pattern| pattern.is_match(normalized_line))
}

fn selected_pathway_discriminator_tokens(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let major_title = primary_major_title(entry, primary_major_titles);
    let major_tokens = normalize_matcher_text(&major_title)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let pathway_id_label = replace_path_separators(&string_field(entry, "pathwayId"));
    let candidate_labels = [
        pathway_id_label,
        normalize_transfer_planner_semantic_pathway_label(
            &major_title,
            &string_field(entry, "ownerTitle"),
        ),
    ];
    let mut tokens = Vec::new();
    for label in candidate_labels {
        for token in normalize_matcher_text(&label).split_whitespace() {
            if token.len() >= 3
                && !pathway_scope_identity_stopwords().contains(token)
                && !major_tokens.contains(token)
                && !tokens.iter().any(|existing| existing.as_str() == token)
            {
                tokens.push(token.to_string());
            }
        }
    }
    tokens
}

fn find_pathway_html_section_end_index(
    entry: &Value,
    lines: &[String],
    selected_start_index: usize,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> usize {
    let mut end_index = lines.len().saturating_sub(1).min(selected_start_index + 220);
    let forward_limit = end_index;
    for index in selected_start_index + 1..=forward_limit {
        let line = normalize_whitespace(&lines[index]);
        if line.is_empty() {
            continue;
        }
        if is_pathway_html_section_sibling_start(
            entry,
            &line,
            primary_major_titles,
            course_parser,
        ) {
            end_index = index.saturating_sub(1).max(selected_start_index);
            break;
        }
        if index > selected_start_index + 2
            && (pathway_section_boundary_pattern_source().is_match(&line)
                || back_to_top_boundary_pattern().is_match(&line)
                || (html_section_boundary_pattern().is_match(&line)
                    && !is_likely_selected_pathway_html_section_start_line(
                        entry,
                        &line,
                        primary_major_titles,
                        course_parser,
                    )))
        {
            end_index = index.saturating_sub(1).max(selected_start_index);
            break;
        }
    }
    end_index
}

fn find_pathway_html_section_start_index(
    entry: &Value,
    lines: &[String],
    selected_start_index: usize,
    course_parser: &CourseCodeParser,
) -> usize {
    let mut start_index = selected_start_index;
    if concentration_pathway_id_pattern().is_match(&string_field(entry, "pathwayId")) {
        for index in (selected_start_index.saturating_sub(80)..selected_start_index).rev() {
            if concentration_area_heading_pattern().is_match(&normalize_whitespace(&lines[index])) {
                start_index = index;
                break;
            }
        }
    }
    for index in (selected_start_index.saturating_sub(3)..selected_start_index).rev() {
        if !course_parser.extract_from_line(&lines[index]).is_empty()
            || is_likely_peer_pathway_html_section_start_line(&lines[index], course_parser)
        {
            break;
        }
        if is_likely_pathway_choice_context_line(&lines[index], course_parser) {
            start_index = index;
            break;
        }
    }
    start_index
}

pub fn pathway_html_section_scope_lines(
    entry: &Value,
    lines: &[String],
    start_index: usize,
    end_index: usize,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let selected_lines = lines[start_index..=end_index].to_vec();
    preserve_prior_admit_pathway_context(entry, lines, &selected_lines, course_parser)
}

pub fn preserve_prior_admit_pathway_context(
    entry: &Value,
    lines: &[String],
    selected_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if !is_prior_admit_curriculum_owner(entry) {
        return selected_lines.to_vec();
    }
    let Some(formal_options_index) = lines.iter().position(|line| {
        formal_options_heading_pattern().is_match(&strip_choice_list_line(line))
    }) else {
        return selected_lines.to_vec();
    };
    let Some(first_pathway_heading_index) = lines.iter().enumerate().find_map(|(index, line)| {
        (index > formal_options_index
            && is_likely_peer_pathway_html_section_start_line(line, course_parser))
        .then_some(index)
    }) else {
        return selected_lines.to_vec();
    };
    let mut seen = HashSet::new();
    lines[..first_pathway_heading_index]
        .iter()
        .chain(selected_lines.iter())
        .filter_map(|line| seen.insert(line.clone()).then_some(line.clone()))
        .collect()
}

fn is_likely_pathway_choice_context_line(line: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized = normalize_transfer_planner_text(line);
    !normalized.is_empty()
        && course_parser.extract_from_line(&normalized).is_empty()
        && normalized.split_whitespace().count() <= 24
        && pathway_choice_context_pattern().is_match(&normalized)
}

pub fn should_use_full_generic_concentration_degree_scope(
    entry: &Value,
    headings: &[String],
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> bool {
    if js_truthy(entry.get("pathwayId"))
        || entry.get("parserType").and_then(Value::as_str) != Some("generic-html")
        || entry.get("isPrimaryDegreeRequirementsLink").and_then(Value::as_bool) != Some(true)
    {
        return false;
    }
    let mut searchable_parts = vec![
        string_field(entry, "ownerTitle"),
        string_field(entry, "label"),
        string_field(entry, "sourceLabel"),
    ];
    searchable_parts.extend(headings.iter().cloned());
    searchable_parts.extend(lines.iter().take(160).cloned());
    let searchable_text = normalize_matcher_text(&searchable_parts.join(" "));
    if !generic_degree_requirements_phrase_pattern().is_match(&searchable_text)
        || !concentration_courses_phrase_pattern().is_match(&searchable_text)
    {
        return false;
    }
    lines.iter().enumerate().any(|(index, line)| {
        concentration_courses_heading_pattern().is_match(&strip_choice_list_line(line))
            && has_upcoming_sectioned_course_rows(lines, index, 1, course_parser)
    })
}

fn looks_like_preceding_inline_requirement_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty()
        || noisy_inline_line_pattern().is_match(&normalized_line)
        || transfer_credit_noise_pattern().is_match(&normalized_line)
        || source_line_starts_with_course_code(&normalized_line)
    {
        return false;
    }
    let Some((label, body)) = inline_labeled_requirement_parts(&normalized_line, course_parser) else {
        return false;
    };
    if course_parser.extract_from_line(&body).is_empty() {
        return false;
    }
    !non_requirement_inline_label_pattern().is_match(&label)
}

fn inline_labeled_requirement_parts(
    raw_line: &str,
    course_parser: &CourseCodeParser,
) -> Option<(String, String)> {
    let line_without_choice_markers = strip_choice_list_line(raw_line);
    let normalized_line = normalize_whitespace(
        &inline_labeled_requirement_marker_pattern()
            .replace(&line_without_choice_markers, ""),
    );
    let colon_index = normalized_line.find(':')?;
    if colon_index == 0 {
        return None;
    }
    let label = trailing_label_punctuation_pattern()
        .replace_all(&normalized_line[..colon_index], "")
        .to_string();
    let label = normalize_whitespace(&label);
    let body = normalize_whitespace(&normalized_line[colon_index + 1..]);
    if label.is_empty()
        || body.is_empty()
        || label.len() > 140
        || !course_parser.extract_from_line(&label).is_empty()
        || course_parser.extract_from_line(&body).is_empty()
    {
        return None;
    }
    Some((label, body))
}

fn strip_choice_list_line(line: &str) -> String {
    let without_page_prefix = strip_snapshot_page_prefix(line);
    normalize_whitespace(&strip_leading_requirement_glyphs(&without_page_prefix))
}

fn source_line_starts_with_course_code(line: &str) -> bool {
    source_line_course_prefix_pattern().is_match(line)
}

fn is_sectioned_option_course_boundary(line: &str, course_parser: &CourseCodeParser) -> bool {
    sectioned_option_course_supplemental_boundary_pattern().is_match(line)
        || sectioned_option_course_service_boundary_pattern().is_match(line)
        || (sectioned_option_course_requirement_boundary_pattern().is_match(line)
            && !sectioned_option_course_core_exception_pattern().is_match(line)
            && course_parser.extract_from_line(line).is_empty())
}

fn sectioned_option_course_line(line: &str) -> Option<String> {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty() {
        return None;
    }
    let mut candidates = vec![normalized_line.clone()];
    let without_credit_prefix = sectioned_course_credit_prefix_pattern()
        .replace(&normalized_line, "")
        .to_string();
    candidates.push(without_credit_prefix.clone());
    let without_bracket_prefix = sectioned_bracket_prefix_pattern()
        .replace(&normalized_line, "")
        .to_string();
    candidates.push(without_bracket_prefix.clone());
    candidates.push(
        sectioned_bracket_prefix_pattern()
            .replace(&without_credit_prefix, "")
            .to_string(),
    );
    candidates
        .into_iter()
        .map(|candidate| normalize_whitespace(&candidate))
        .find(|candidate| {
            !candidate.is_empty() && source_line_starts_with_course_code(candidate)
        })
}

pub fn has_primary_requirement_section_cue(line: &str) -> bool {
    primary_requirement_section_cue_pattern().is_match(line)
}

pub fn looks_like_sectioned_course_heading_line(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = strip_choice_list_line(line);
    if normalized_line.is_empty()
        || sectioned_course_season_heading_pattern().is_match(&normalized_line)
        || source_line_starts_with_course_code(&normalized_line)
        || (!course_parser.extract_from_line(&normalized_line).is_empty()
            && !course_level_heading_pattern().is_match(&normalized_line))
    {
        return false;
    }
    sectioned_course_required_count_pattern().is_match(&normalized_line)
        || (sectioned_course_list_requirement_pattern().is_match(&normalized_line)
            && sectioned_course_heading_qualifier_pattern().is_match(&normalized_line))
}

pub fn looks_like_short_named_course_section_heading(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = strip_choice_list_line(line);
    let length = normalized_line.chars().count();
    length >= 3
        && length <= 90
        && normalized_line
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
        && !normalized_line.ends_with('.')
        && !normalized_line.ends_with(';')
        && !sectioned_course_credit_list_line(&normalized_line)
        && !source_line_starts_with_course_code(&normalized_line)
        && course_parser.extract_from_line(&normalized_line).is_empty()
        && !short_named_course_heading_exclusion_pattern().is_match(&normalized_line)
}

fn sectioned_course_credit_list_line(line: &str) -> bool {
    let normalized_line = strip_choice_list_line(line);
    let parts = normalized_line.split(',').map(str::trim).collect::<Vec<_>>();
    !parts.is_empty()
        && parts.iter().all(|part| {
            sectioned_course_credit_range_pattern()
                .captures(part)
                .and_then(|captures| {
                    let minimum = captures.get(1)?.as_str().parse::<f64>().ok()?;
                    let maximum = captures
                        .get(2)
                        .and_then(|value| value.as_str().parse::<f64>().ok())
                        .unwrap_or(minimum);
                    (minimum > 0.0 && minimum <= 30.0 && maximum >= minimum && maximum <= 30.0)
                        .then_some(())
                })
                .is_some()
        })
}

pub fn parser_recovery_section_line_score(entry: &Value, line: &str) -> i32 {
    let normalized_line = normalize_matcher_text(line);
    if normalized_line.is_empty() || graduate_supplemental_pattern().is_match(line) {
        return 0;
    }
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    let mut score = 0;
    if !exact_title.is_empty() && normalized_line.contains(&exact_title) {
        score += 24;
    }
    score += parser_recovery_title_scope_tokens(entry)
        .iter()
        .filter(|token| normalized_line.contains(token.as_str()))
        .count() as i32
        * 5;
    if structural_requirement_pattern().is_match(line)
        || primary_requirement_line_pattern().is_match(line)
    {
        score += 12;
    }
    if requirement_cue_pattern().is_match(line) {
        score += 6;
    }
    if pathway_cue_pattern().is_match(line) {
        score += 5;
    }
    if section_noise_penalty_pattern().is_match(line) {
        score -= 12;
    }
    score
}

pub fn is_legacy_catalog_credential_heading_line(line: &str) -> bool {
    legacy_catalog_credential_heading_pattern().is_match(&normalize_whitespace(line))
}

pub fn line_looks_like_legacy_catalog_credential_section_start(
    lines: &[String],
    index: usize,
) -> bool {
    if index >= lines.len() {
        return false;
    }
    let following_text = lines[index + 1..lines.len().min(index + 7)]
        .iter()
        .map(|line| normalize_whitespace(line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    legacy_catalog_section_start_pattern().is_match(&following_text)
}

pub fn parser_recovery_candidate_conflicts_with_program_sibling(
    entry: &Value,
    label: &str,
    url: &str,
) -> bool {
    if !is_different_program_sibling_page(&string_field(entry, "url"), url) {
        return false;
    }
    let link_text = normalize_matcher_text(&format!("{label} {url}"));
    let exact_title = normalize_matcher_text(&string_field(entry, "ownerTitle"));
    if !exact_title.is_empty() && link_text.contains(&exact_title) {
        return false;
    }
    let title_tokens = parser_recovery_title_scope_tokens(entry);
    let title_token_overlap_count = title_tokens
        .iter()
        .filter(|token| link_text.contains(token.as_str()))
        .count();
    title_tokens.len() >= 2 && title_token_overlap_count < 2.min(title_tokens.len())
}

pub fn parser_recovery_candidate_conflicts_with_pathway_scope(
    entry: &Value,
    label: &str,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !js_truthy(entry.get("pathwayId")) {
        return parser_recovery_candidate_conflicts_with_base_owner_scope(
            entry,
            label,
            url,
            primary_major_titles,
        );
    }
    let link_text = format!("{label} {url}");
    let has_pathway_cue = selected_pathway_label_cue_pattern().is_match(&link_text);
    let is_support_or_admission_link = is_support_or_admission_link_pattern().is_match(&link_text);
    let owner_identity_text = normalize_matcher_text(&format!(
        "{} {} {} {}",
        string_field(entry, "pathwayId"),
        string_field(entry, "ownerTitle"),
        string_field(entry, "sourceLabel"),
        string_field(entry, "label")
    ));
    if has_pathway_cue {
        if let Some(acronym) = candidate_option_acronym_pattern()
            .captures(&link_text)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_lowercase())
        {
            if !owner_identity_text
                .split_whitespace()
                .any(|token| token == acronym.as_str())
            {
                return true;
            }
        }
    }

    let pathway_id = string_field(entry, "pathwayId");
    let pathway_phrase_tokens = normalize_matcher_text(&replace_path_separators(&pathway_id))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let pathway_phrase = pathway_phrase_tokens.join(" ");
    let normalized_link_text = normalize_matcher_text(&link_text);
    let compact_pathway_id = normalize_matcher_text(&pathway_id).replace(' ', "");
    let compact_link_text = normalized_link_text.replace(' ', "");
    if has_pathway_cue
        && pathway_phrase_tokens.iter().any(|token| token.len() > 4)
        && !normalized_link_text.contains(&pathway_phrase)
        && !( !compact_pathway_id.is_empty() && compact_link_text.contains(&compact_pathway_id) )
    {
        return true;
    }

    let owner_tokens = parser_recovery_selected_pathway_identity_tokens(
        entry,
        primary_major_titles,
    );
    let owner_token_groups = selected_pathway_scope_token_groups(entry, primary_major_titles);
    let candidate_tokens = parser_recovery_pathway_identity_tokens(
        entry,
        &link_text,
        primary_major_titles,
    );
    let looks_like_sibling_pathway_identity = !is_support_or_admission_link
        && is_same_origin_child_or_sibling_page(&string_field(entry, "url"), url)
        && candidate_tokens.len() >= 2;
    if !has_pathway_cue && !looks_like_sibling_pathway_identity {
        return false;
    }

    if !owner_token_groups.is_empty() && !candidate_tokens.is_empty() {
        if parser_recovery_candidate_has_sibling_pathway_qualifier(
            entry,
            &owner_token_groups,
            &candidate_tokens,
            url,
            primary_major_titles,
        ) {
            return true;
        }
        let candidate_token_set = candidate_tokens.iter().cloned().collect::<HashSet<_>>();
        let has_complete_owner_group = owner_token_groups.iter().any(|group| {
            group.iter().all(|token| candidate_token_set.contains(token))
        });
        if !has_complete_owner_group {
            return true;
        }
    }

    !owner_tokens.is_empty()
        && !candidate_tokens.is_empty()
        && !candidate_tokens.iter().any(|token| owner_tokens.contains(token))
}

pub fn parser_recovery_candidate_conflicts_with_base_owner_scope(
    entry: &Value,
    label: &str,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if js_truthy(entry.get("pathwayId")) {
        return false;
    }
    let Some(normalized_candidate_url) = normalize_url_for_comparison(url) else {
        return false;
    };
    let normalized_entry_url = normalize_url_for_comparison(&string_field(entry, "url"));
    let hash_fragment = url_hash_fragment(url);
    if (normalized_entry_url.as_deref() == Some(normalized_candidate_url.as_str())
        && hash_fragment.is_empty())
        || !is_same_origin_child_or_sibling_page(&string_field(entry, "url"), url)
    {
        return false;
    }

    let link_text = format!("{label} {url}");
    if is_support_or_admission_link_pattern().is_match(&link_text) {
        return false;
    }
    let last_segment = last_url_path_segment(url);
    let normalized_hash = normalize_matcher_text(&replace_path_separators(&hash_fragment));
    let generic_pathway_hub_segment = generic_pathway_hub_segment_pattern().is_match(&last_segment);
    let generic_hash_fragment = normalized_hash.is_empty()
        || generic_pathway_hash_pattern().is_match(&normalized_hash);
    if generic_pathway_hub_segment && generic_hash_fragment {
        return false;
    }

    let has_specific_pathway_cue = pathway_cue_pattern().is_match(&link_text)
        || specific_pathway_segment_pattern().is_match(&last_segment)
        || !generic_hash_fragment;
    if !has_specific_pathway_cue {
        return false;
    }

    let candidate_identity = format!(
        "{label} {} {}",
        replace_path_separators(&last_segment),
        replace_path_separators(&hash_fragment)
    );
    !parser_recovery_pathway_identity_tokens(
        entry,
        &candidate_identity,
        primary_major_titles,
    )
    .is_empty()
}

fn parser_recovery_pathway_identity_tokens(
    entry: &Value,
    value: &str,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let plan_id = string_field(entry, "planId");
    let primary_title = primary_major_titles
        .get(&plan_id)
        .cloned()
        .unwrap_or_else(|| {
            let owner_title = normalize_transfer_planner_text(&string_field(entry, "ownerTitle"));
            owner_title_prefix_pattern()
                .split(&owner_title)
                .next()
                .unwrap_or(&owner_title)
                .to_string()
        });
    let primary_title_tokens = normalize_matcher_text(&primary_title)
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut primary_tokens = primary_title_tokens.iter().cloned().collect::<BTreeSet<_>>();
    let acronym = primary_title_tokens
        .iter()
        .filter_map(|token| token.chars().next())
        .collect::<String>();
    if acronym.len() >= 2 {
        primary_tokens.insert(acronym);
    }
    let generic_tokens = parser_recovery_generic_identity_tokens();
    normalize_matcher_text(value)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !primary_tokens.contains(*token) && !generic_tokens.contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn parser_recovery_owner_program_acronym_tokens(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> BTreeSet<String> {
    let primary_title = primary_major_title(entry, primary_major_titles);
    let mut acronyms = program_acronym_tokens_from_title(&primary_title, true);
    acronyms.extend(program_acronym_tokens_from_title(&string_field(entry, "ownerTitle"), false));
    acronyms.extend(program_acronym_tokens_from_title(&string_field(entry, "sourceLabel"), false));
    let route_text = normalize_matcher_text(&format!(
        "{} {} {} {} {}",
        string_field(entry, "ownerId"),
        string_field(entry, "pathwayId"),
        string_field(entry, "ownerTitle"),
        string_field(entry, "sourceLabel"),
        string_field(entry, "label")
    ));
    if route_ell_acronym_pattern().is_match(&route_text)
        || teaching_ell_acronym_pattern().is_match(&route_text)
    {
        acronyms.extend(["ell".to_string(), "tell".to_string()]);
    }
    if route_sped_acronym_pattern().is_match(&route_text) {
        acronyms.insert("sped".to_string());
    }
    acronyms
}

fn program_acronym_tokens_from_title(value: &str, include_title_acronym: bool) -> BTreeSet<String> {
    let raw_title = normalize_whitespace(value);
    if raw_title.is_empty() {
        return BTreeSet::new();
    }
    let mut tokens = explicit_program_acronym_pattern()
        .captures_iter(&raw_title)
        .filter_map(|captures| captures.get(1))
        .map(|value| normalize_matcher_text(value.as_str()).replace(' ', ""))
        .filter(|token| valid_program_acronym(token))
        .collect::<BTreeSet<_>>();
    if include_title_acronym {
        let normalized_title = normalize_matcher_text(&raw_title);
        let title_words = normalized_title
            .split_whitespace()
            .filter(|token| {
                token.len() >= 2
                    && !program_acronym_stopwords().contains(*token)
                    && !excluded_program_acronym_tokens().contains(*token)
            })
            .collect::<Vec<_>>();
        let title_acronym = title_words
            .iter()
            .filter_map(|token| token.chars().next())
            .collect::<String>();
        if valid_program_acronym(&title_acronym) {
            tokens.insert(title_acronym);
        }
    }
    tokens
}

fn valid_program_acronym(value: &str) -> bool {
    (2..=8).contains(&value.len()) && !excluded_program_acronym_tokens().contains(value)
}

fn program_acronym_stopwords() -> BTreeSet<&'static str> {
    [
        "a", "and", "b", "bachelor", "bachelors", "ba", "bothell", "bs", "campus",
        "curriculum", "degree", "for", "in", "major", "of", "official", "overview", "page",
        "program", "requirements", "s", "seattle", "tacoma", "the", "to", "undergraduate",
        "university", "uw", "washington", "with",
    ]
    .into_iter()
    .collect()
}

fn excluded_program_acronym_tokens() -> BTreeSet<&'static str> {
    ["aa", "aab", "aas", "ba", "bas", "bba", "bfa", "bs", "bse", "bsn", "ma", "mba", "mfa", "ms"]
        .into_iter()
        .collect()
}

pub fn primary_major_title(entry: &Value, primary_major_titles: &HashMap<String, String>) -> String {
    primary_major_titles
        .get(&string_field(entry, "planId"))
        .cloned()
        .unwrap_or_else(|| {
            let owner_title = normalize_transfer_planner_text(&string_field(entry, "ownerTitle"));
            owner_title_prefix_pattern()
                .split(&owner_title)
                .next()
                .unwrap_or(&owner_title)
                .to_string()
        })
}

fn selected_pathway_scope_labels(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let major_title = primary_major_title(entry, primary_major_titles);
    let major_tokens = normalize_matcher_text(&major_title)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let pathway_id_label = replace_path_separators(&string_field(entry, "pathwayId"));
    let raw_labels = [
        pathway_id_label.clone(),
        normalize_transfer_planner_semantic_pathway_label(&major_title, &string_field(entry, "ownerTitle")),
        normalize_transfer_planner_semantic_pathway_label(&major_title, &string_field(entry, "sourceLabel")),
        normalize_transfer_planner_semantic_pathway_label(&major_title, &string_field(entry, "label")),
    ];
    let pathway_id_tokens = normalize_matcher_text(&pathway_id_label)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let mut labels = Vec::new();
    for raw_label in raw_labels {
        let label = strip_pathway_heading_prefix(&raw_label);
        let normalized_label = normalize_matcher_text(&label);
        let tokens = normalized_label
            .split_whitespace()
            .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
            .map(str::to_string)
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            continue;
        }
        let has_pathway_cue = selected_pathway_label_cue_pattern().is_match(&label);
        if !has_pathway_cue
            && tokens.len() > 1
            && tokens.iter().all(|token| major_tokens.contains(token))
        {
            continue;
        }
        if (has_pathway_cue || tokens.iter().any(|token| pathway_id_tokens.contains(token)))
            && !labels.contains(&label)
        {
            labels.push(label);
        }
    }
    labels
}

fn selected_pathway_scope_token_groups(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<Vec<String>> {
    let mut groups = selected_pathway_scope_labels(entry, primary_major_titles)
        .iter()
        .map(|label| {
            normalize_matcher_text(label)
                .split_whitespace()
                .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(*token))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|tokens| !tokens.is_empty())
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| right.len().cmp(&left.len()));
    groups.dedup();
    groups
}

fn parser_recovery_selected_pathway_identity_tokens(
    entry: &Value,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    if !js_truthy(entry.get("pathwayId")) {
        return Vec::new();
    }
    let mut labels = selected_pathway_scope_labels(entry, primary_major_titles);
    labels.push(replace_path_separators(&string_field(entry, "pathwayId")));
    labels
        .iter()
        .flat_map(|label| normalize_matcher_text(label).split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .filter(|token| token.len() >= 3 && !pathway_scope_identity_stopwords().contains(token.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parser_recovery_candidate_url_pathway_tokens(
    entry: &Value,
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> Vec<String> {
    let Ok(parsed) = Url::parse(url) else {
        return Vec::new();
    };
    let segments = parsed.path().split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>();
    let mut decoded_segments = Vec::new();
    for segment in segments.into_iter().rev().take(2).collect::<Vec<_>>().into_iter().rev() {
        let Some(decoded) = decode_uri_component(segment) else {
            return Vec::new();
        };
        decoded_segments.push(decoded);
    }
    let primary_tokens = normalize_matcher_text(&primary_major_title(entry, primary_major_titles))
        .split_whitespace()
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let generic_tokens = pathway_scope_identity_stopwords()
        .into_iter()
        .chain(["admission", "admissions", "curriculum", "prereq", "prerequisite", "prerequisites", "requirement", "requirements"])
        .collect::<HashSet<_>>();
    normalize_matcher_text(&decoded_segments.join(" "))
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !generic_tokens.contains(*token) && !primary_tokens.contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parser_recovery_candidate_has_sibling_pathway_qualifier(
    entry: &Value,
    owner_token_groups: &[Vec<String>],
    candidate_tokens: &[String],
    url: &str,
    primary_major_titles: &HashMap<String, String>,
) -> bool {
    if !js_truthy(entry.get("pathwayId"))
        || candidate_tokens.is_empty()
        || owner_token_groups.is_empty()
        || owner_token_groups.iter().any(|group| group.len() != 1)
    {
        return false;
    }
    let owner_tokens = owner_token_groups
        .iter()
        .filter_map(|group| group.first().cloned())
        .collect::<HashSet<_>>();
    let url_tokens = parser_recovery_candidate_url_pathway_tokens(
        entry,
        url,
        primary_major_titles,
    );
    !url_tokens.is_empty() && url_tokens.iter().any(|token| !owner_tokens.contains(token))
}

fn strip_pathway_heading_prefix(value: &str) -> String {
    let mut label = normalize_transfer_planner_text(value);
    label = pathway_letter_heading_prefix_pattern().replace(&label, "").into_owned();
    label = pathway_numeric_heading_prefix_pattern().replace(&label, "").into_owned();
    pathway_roman_heading_prefix_pattern().replace(&label, "").trim().to_string()
}

fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            decoded.push((hex_value(high)? << 4) | hex_value(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn pathway_scope_identity_stopwords() -> HashSet<&'static str> {
    [
        "and", "ba", "bachelor", "bs", "certificate", "concentration", "degree", "major",
        "of", "option", "pathway", "program", "credit", "credits", "route", "track", "with",
    ]
    .into_iter()
    .collect()
}

fn is_same_origin_child_or_sibling_page(base_url: &str, resolved_url: &str) -> bool {
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    if base.origin() != resolved.origin() {
        return false;
    }
    let base_segments = base.path().split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>();
    let resolved_segments = resolved.path().split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>();
    if base_segments.is_empty() || resolved_segments.is_empty() {
        return false;
    }
    let shared_prefix_length = base_segments
        .iter()
        .zip(resolved_segments.iter())
        .position(|(base_segment, resolved_segment)| base_segment != resolved_segment)
        .unwrap_or(base_segments.len().min(resolved_segments.len()));
    shared_prefix_length >= 1.max(3.min(base_segments.len().saturating_sub(1)))
}

pub fn is_different_program_sibling_page(base_url: &str, resolved_url: &str) -> bool {
    let (Ok(base), Ok(resolved)) = (Url::parse(base_url), Url::parse(resolved_url)) else {
        return false;
    };
    if base.origin() != resolved.origin() {
        return false;
    }
    let base_segments = base.path().split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>();
    let resolved_segments = resolved.path().split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>();
    let divergence_index = base_segments
        .iter()
        .zip(resolved_segments.iter())
        .position(|(base_segment, resolved_segment)| base_segment != resolved_segment)
        .unwrap_or(base_segments.len().min(resolved_segments.len()));
    let Some(parent_segment) = divergence_index
        .checked_sub(1)
        .and_then(|index| base_segments.get(index))
    else {
        return false;
    };
    let Some(base_divergent_segment) = base_segments.get(divergence_index) else {
        return false;
    };
    let Some(resolved_divergent_segment) = resolved_segments.get(divergence_index) else {
        return false;
    };
    let parent_is_program_collection = matches!(*parent_segment, "majors" | "programs" | "degrees");
    let both_are_credential_programs = credential_program_path_segment_pattern()
        .is_match(base_divergent_segment)
        && credential_program_path_segment_pattern().is_match(resolved_divergent_segment);
    divergence_index > 0 && (parent_is_program_collection || both_are_credential_programs)
}

fn normalize_url_for_comparison(value: &str) -> Option<String> {
    let mut parsed = Url::parse(value).ok()?;
    parsed.set_fragment(None);
    let path = parsed.path().trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let query = parsed.query().filter(|query| !query.is_empty())
        .map(|query| format!("?{query}")).unwrap_or_default();
    Some(format!("{}{path}{query}", parsed.origin().ascii_serialization()))
}

fn url_hash_fragment(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|parsed| parsed.fragment().map(str::to_string))
        .unwrap_or_default()
}

fn last_url_path_segment(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|parsed| parsed.path().split('/').filter(|segment| !segment.is_empty()).next_back().map(str::to_string))
        .unwrap_or_default()
}

fn replace_path_separators(value: &str) -> String {
    path_separator_pattern().replace_all(value, " ").into_owned()
}

pub fn is_graduate_career_planning_note(line: &str) -> bool {
    let normalized_line = normalize_whitespace(
        &super::requirement_choice_lists::strip_choice_list_line(line),
    );
    if graduate_career_pursuit_pattern().is_match(&normalized_line)
        && career_context_pattern().is_match(&normalized_line)
        && !graduate_program_requirement_exclusion_pattern().is_match(&normalized_line)
    {
        return true;
    }
    if graduate_school_career_pattern().is_match(&normalized_line)
        && !graduate_program_requirement_exclusion_pattern().is_match(&normalized_line)
    {
        return true;
    }
    graduate_work_career_pattern().is_match(&normalized_line)
        && !graduate_degree_requirement_exclusion_pattern().is_match(&normalized_line)
}

pub fn is_graduate_career_planning_continuation_line(
    snapshot_lines: &[String],
    index: usize,
    course_parser: &CourseCodeParser,
) -> bool {
    let raw_line = snapshot_lines
        .get(index)
        .map(|line| {
            normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            )
        })
        .unwrap_or_default();
    if raw_line.is_empty() {
        return false;
    }
    if is_graduate_career_planning_note(&raw_line) {
        return true;
    }
    let previous_context_line = snapshot_lines
        .iter()
        .take(index)
        .skip(index.saturating_sub(3))
        .rev()
        .map(|line| {
            normalize_whitespace(
                &super::requirement_choice_lists::strip_choice_list_line(line),
            )
        })
        .find(|line| {
            !line.is_empty() && !note_prefix_pattern().is_match(line)
        });
    let Some(previous_context_line) = previous_context_line else {
        return false;
    };
    if !is_graduate_career_planning_note(&previous_context_line) {
        return false;
    }
    !course_parser.extract_from_line(&raw_line).is_empty()
        && (graduate_course_continuation_prefix_pattern().is_match(&raw_line)
            || graduate_course_or_continuation_pattern().is_match(&raw_line))
        && !graduate_subject_heading_exclusion_pattern().is_match(&raw_line)
}

fn graduate_career_pursuit_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:may|can|could|often|typically)?\s*(?:pursu(?:e|ing)|seek|continue|go\s+on)\b.{0,180}\b(?:graduate|master(?:'s|s)?|m\.?\s*a\.?|m\.?\s*s\.?|ph\.?\s*d\.?)\b").expect("graduate career pursuit pattern"))
}
fn career_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:careers?|employment|jobs?|positions?|professional|work)\b").expect("career context pattern"))
}
fn graduate_program_requirement_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:graduate\s+(?:programs?|admissions?|requirements?|curriculum|coursework)|master(?:'s|s)?\s+program|degree\s+requirements?)\b").expect("graduate program requirement exclusion pattern"))
}
fn graduate_school_career_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:plan(?:s)?\s+to\s+pursue|pursu(?:e|ing)|prepar(?:e|es|ing)?\s+for|good\s+choice\s+for|careers?)\b.{0,160}\b(?:graduate\s+degree|graduate\s+school|professional\s+school|post[-\s]?graduate\s+work)\b").expect("graduate school career pattern"))
}
fn graduate_work_career_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:graduate work|graduate school|professional school|post[-\s]?graduate work)\b").expect("graduate work career pattern"))
}
fn graduate_degree_requirement_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:graduate\s+(?:program|degree|admission|requirements?)|master|m\.?\s*s\.?|m\.?\s*a\.?|ph\.?\s*d\.?|doctor)\b").expect("graduate degree requirement exclusion pattern"))
}
fn note_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:please\s+note|note)[:\s]").expect("note prefix pattern"))
}
fn graduate_course_continuation_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:\(?\s*\d+(?:\.\d+)?\s*\)|(?:or|-OR-)\b)").expect("graduate course continuation prefix pattern"))
}
fn graduate_course_or_continuation_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:or|-OR-)\b").expect("graduate course or continuation pattern"))
}
fn graduate_subject_heading_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:statistics|physics|biology|chemistry|mathematics|supporting requirements)\b").expect("graduate subject heading exclusion pattern"))
}

pub fn normalize_matcher_text(value: &str) -> String {
    let mut output = String::new();
    let mut pending_separator = false;
    for character in value.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !output.is_empty() {
                output.push(' ');
            }
            pending_separator = false;
            output.push(character);
        } else {
            pending_separator = !output.is_empty();
        }
    }
    output
}

fn parser_recovery_generic_identity_tokens() -> BTreeSet<&'static str> {
    [
        "and", "bachelor", "business", "complete", "concentration", "courses", "current",
        "curriculum", "degree", "admission", "admissions", "major", "option", "pathway",
        "prerequisite", "prerequisites", "program", "requirements", "route", "school",
        "students", "track", "undergraduate",
    ]
    .into_iter()
    .collect()
}

fn string_field(value: &Value, key: &str) -> String {
    value.get(key)
        .filter(|value| !value.is_null())
        .map(|value| match value {
            Value::String(value) => value.clone(),
            Value::Number(value) => value.to_string(),
            Value::Bool(value) => value.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

fn owner_title_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\s+-\s+").expect("owner title prefix"))
}

fn is_support_or_admission_link_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\bapproved\b.{0,60}\b(courses?|course list|list)\b|\b(courses?|course list|list)\b.{0,60}\bapproved\b|\b(?:engineering|technical|departmental|major|science|natural science|approved)?\s*electives?\b.{0,50}\b(courses?|list|options?|page)\b|\b(courses?|list|options?)\b.{0,50}\belectives?\b|/electives?(?:[-/]|$)|\b(?:admissions?|admission|apply|application)\b.{0,80}\bprereq(?:uisites?|uisite courses?)\b|\bprereq(?:uisites?|uisite courses?)\b.{0,80}\b(?:admissions?|admission|apply|application)\b|\b(advising|adviser|advisor|support sources?|student resources?|student support|forms?|petitions?|policies|policy[-\s]*(?:procedures?|resources?|forms?)|faq|frequently asked questions)\b")
            .expect("support or admission link cue")
    })
}

fn find_prior_admit_curriculum_start_index(
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Option<usize> {
    if lines.len() < 12 {
        return None;
    }
    for (index, line) in lines.iter().enumerate() {
        if !prior_admit_curriculum_start_pattern().is_match(&normalize_whitespace(line)) {
            continue;
        }
        let following_lines = &lines[index..lines.len().min(index + 36)];
        let following_text = normalize_matcher_text(&following_lines.join(" "));
        let following_course_count = following_lines
            .iter()
            .flat_map(|line| course_parser.extract_from_line(line))
            .collect::<BTreeSet<_>>()
            .len();
        if following_course_count >= 3
            && major_requirements_pattern().is_match(&following_text)
            && prior_admit_curriculum_cue_pattern().is_match(&following_text)
        {
            return Some(index);
        }
    }
    None
}

fn is_prior_admit_curriculum_owner(entry: &Value) -> bool {
    let owner_text = normalize_whitespace(&format!(
        "{} {} {} {}",
        string_field(entry, "ownerId"),
        string_field(entry, "ownerTitle"),
        string_field(entry, "pathwayId"),
        string_field(entry, "label")
    ))
    .replace('_', " ")
    .replace(':', " ")
    .replace('-', " ")
    .to_lowercase();
    prior_admit_owner_era_pattern().is_match(&owner_text)
}

fn prior_admit_curriculum_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:for students admitted\b.*\b(?:before|prior to)\b|students admitted\b.*\b(?:before|prior to)\b|degree requirements?\b.*\bfor students admitted\b.*\b(?:before|prior to)\b)")
            .expect("prior admit curriculum start")
    })
}

fn prior_admit_owner_era_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:pre|before|prior\s+to)\s+(?:spring|summer|autumn|fall|winter)\s+\d{4}\b")
            .expect("prior admit owner era")
    })
}

fn major_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bmajor requirements?\b").expect("major requirements"))
}

fn prior_admit_curriculum_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:introductory|methods?|formal options?)\b").expect("prior admit curriculum cue"))
}

fn formal_options_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bformal options?\b").expect("formal options"))
}

fn shared_curriculum_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bshared curriculum\b").expect("shared curriculum"))
}

fn beginning_spring_2026_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bbeginning in spring 2026\b").expect("beginning spring 2026"))
}

fn spring_2026_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bspring 2026\b").expect("spring 2026"))
}

fn foundation_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bfoundation courses?\b").expect("foundation courses"))
}

fn methods_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bmethods courses?\b").expect("methods courses"))
}

fn introductory_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bintroductory courses?\b").expect("introductory courses"))
}

fn graduation_or_contact_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:graduation requirements?|contact)$").expect("graduation or contact heading"))
}

fn departmental_degree_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^([A-Z][A-Z0-9&]{1,8})\s+Departmental\s+Degree\s+requirements?\b")
            .expect("departmental degree boundary")
    })
}

fn pathway_section_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:[A-Z][A-Za-z0-9_&'./-]+(?:\s+[A-Z][A-Za-z0-9_&'./-]+){0,8}\s+)?(?:Track|Option|Concentration|Pathway)(?:\s+(?:Degree|Major|Program|Curriculum|Requirements?|Courses?|Plan|Checklist))*$")
            .expect("parser recovery pathway section boundary")
    })
}

fn html_section_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:Program of Study:|[A-Z][A-Z0-9&]{1,8}\s+Departmental\s+Degree\s+requirements?\b|Bachelor\b|Minor\b|Master\b|Doctor\b|Undergraduate Programs\b|Graduate Programs\b|Back to Top\b)")
            .expect("parser recovery html section boundary")
    })
}

fn generic_section_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:Bachelor|Minor|Master|Doctor|Program of Study|Degree Requirements|Major Requirements|Admissions?|Application|Sample|Suggested Plan|Courses?)\b")
            .expect("parser recovery generic section boundary")
    })
}

fn credential_program_path_segment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:bachelor|bachelors?|ba|bs|bba|major|minor)[-_]")
            .expect("credential program path segment")
    })
}

fn pathway_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(track|option|route|pathway|concentration|specialization)\b")
            .expect("pathway cue")
    })
}

fn sectioned_course_column_header_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:Course\s*#|Course|Courses|Course\s+Name|Topic|Credits?|Department|Dept\.?|TOTAL)$").expect("sectioned course column header"))
}

fn sectioned_option_course_supplemental_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\[Supplemental official source\]").expect("sectioned course supplemental boundary"))
}

fn sectioned_option_course_service_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:B\.S\. degree requirements|Undergraduate advising|(?:[A-Z& ]+\s+)?Capstone|Tuition and scholarships|Contact\b|Log in|Privacy|Terms)$").expect("sectioned course service boundary"))
}

fn sectioned_option_course_requirement_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:admission|requirements?)\b").expect("sectioned course requirement boundary"))
}

fn sectioned_option_course_core_exception_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:core|electives?)\b").expect("sectioned course core exception"))
}

fn sectioned_course_credit_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\d+(?:\.\d+)?\s*credits?\s*:\s*").expect("sectioned course credit prefix"))
}

fn sectioned_bracket_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\[[^\]]+\]\s*").expect("sectioned course bracket prefix"))
}

fn primary_requirement_section_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:graduation requirements?|degree requirements?|major requirements?|completion requirements?|core requirements?|elective requirements?|general education requirements?|required courses?|fundamentals|mathematics|natural sciences?|physics|chemistry|biology|calculus|composition|data and society|core and electives|areas of inquiry|engineering fundamentals|mathematics\s*&\s*natural sciences|written\s*&\s*oral communication)\b|^requirements?$|^major in\b|^course #$|\b(?:concentration|option|track|pathway|route)\b$").expect("primary requirement section cue"))
}

fn sectioned_course_season_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:Autumn|Fall|Winter|Spring|Summer)(?:\s*&\s*(?:Autumn|Fall|Winter|Spring|Summer))*$").expect("sectioned course season heading"))
}

fn course_level_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b\d{3}[-\s]*level\b").expect("course level heading"))
}

fn sectioned_course_required_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:choose|select)\s+(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)\s+courses?\b|^(?:one|two|three|four|five|six|seven|eight|nine|ten)(?:\s*\(\s*\d+\s*\))?\s+.+?\b(?:courses?|electives?)\b|^\d+\s+.+?\belectives?\b").expect("sectioned course required count"))
}

fn sectioned_course_list_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:selected from|from the following list|both courses?|list of approved courses|approved list of courses|courses? listed below|listed below are required|following departments|technical electives?|engineering fundamentals electives?|science electives?|math elective|core and elective requirements?)\b").expect("sectioned course list requirement"))
}

fn sectioned_course_heading_qualifier_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:credits?|courses?|electives?|minimum|maximum)\b").expect("sectioned course heading qualifier"))
}

fn short_named_course_heading_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:advising|admission|apply|application|contact|scholarships?|tuition|sample|planning|overview|objectives?|outcomes?|learn by doing|recommended preparation)\b").expect("short named course heading exclusion"))
}

fn sectioned_course_credit_range_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(\d+(?:\.\d+)?)(?:\s*(?:-|to)\s*(\d+(?:\.\d+)?))?$").expect("sectioned course credit range"))
}

fn generic_degree_requirements_phrase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bdegree requirements\b").expect("generic degree requirements phrase"))
}

fn concentration_courses_phrase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bconcentration courses\b").expect("concentration courses phrase"))
}

fn concentration_courses_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bconcentration courses?:?$").expect("concentration courses heading"))
}

fn pathway_curriculum_or_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:curriculum|requirements?)\b").expect("pathway curriculum or requirements"))
}

fn back_to_top_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^Back to Top\b").expect("back to top boundary"))
}

fn concentration_pathway_id_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bconcentration\b").expect("concentration pathway id"))
}

fn concentration_area_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bconcentration\s+area(?:\s+courses?)?\b").expect("concentration area heading"))
}

fn pathway_choice_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:choose|select|formal|options?|tracks?|routes?|pathways?|concentrations?)\b").expect("pathway choice context"))
}

fn bare_pathway_subheading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:admission|admissions|application|apply|capstone|checklist|core|course|courses|coursework|credits?|curriculum|degree|electives?|internship|list|major|minor|overview|prereq|prerequisite|program|requirements?|sample|schedule|series|student|students|worksheet)\b").expect("bare pathway subheading"))
}

fn bare_pathway_filler_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:and|&|of|the)$").expect("bare pathway filler word"))
}

fn bare_pathway_visible_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^[A-Z0-9&][A-Za-z0-9&/()'-]*$").expect("bare pathway visible word"))
}

fn formal_options_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^formal options?\b").expect("formal options heading"))
}

fn formal_options_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bformal options?\b").expect("formal options context"))
}

fn choose_one_formal_options_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(?:choose|select|declare)\s+one\b").expect("choose one formal option"))
}

fn formal_option_enumerated_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^[A-Z]\.\s+\S").expect("formal option enumerated heading"))
}

fn formal_option_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:option|certificate|concentration|track|pathway)\b").expect("formal option cue"))
}

fn formal_option_action_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:choose|select|declare|complete|credits?)\b").expect("formal option action"))
}

fn formal_options_end_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:general electives?|graduation requirements?|admissions?|advising|contact\b|how to apply\b|degree requirements?|major requirements?)\b").expect("formal options end boundary"))
}

fn course_list_section_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^list\s+[a-z0-9]\b").expect("course list section heading"))
}

fn pathway_label_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(option|track|route|pathway|concentration)\b").expect("pathway label cue"))
}

fn peer_pathway_prose_rejection_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:choose|select|following|formal)\b").expect("peer pathway prose rejection"))
}

fn pathway_heading_enumerator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[A-Z]\.\s+|^\(?\d+[).]\s+").expect("pathway heading enumerator"))
}

fn pathway_html_table_content_end_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:general electives?|graduation requirements?|admission requirements?|curriculum|major requirements?|program overview|contact\b|for students admitted\b)").expect("pathway HTML table content end boundary"))
}

fn base_scope_pathway_section_end_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:advisors?|advisor|major coordinator|department|career opportunities|admission requirements?|degree requirements?|contact)\b").expect("base scope pathway section end boundary"))
}

fn noisy_inline_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$)))")
            .expect("noisy inline requirement line")
    })
}

fn transfer_credit_noise_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(transfer credits?|transfer equivalenc(?:y|ies)|articulation agreement|community colleges?|community college)\b").expect("transfer credit noise"))
}

fn source_line_course_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[-*\x{2022}\x{F071}\s]*(?:\(\s*\d+(?:\.\d+)?\s*\)\s*)?(?:[A-Z&]+(?:\s+[A-Z&]+)?(?:\s*/\s*[A-Z&]+(?:\s+[A-Z&]+)?)?\s+\d{3}[A-Za-z]?\*?|\d{3}[A-Za-z]?\b)").expect("source line course code prefix"))
}

fn inline_labeled_requirement_marker_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\d+\s*[).]\s*|^[A-Z]\s*[).]\s*").expect("inline requirement marker"))
}

fn trailing_label_punctuation_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[:.]+$").expect("trailing inline label punctuation"))
}

fn non_requirement_inline_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(admissions?|application|deadline|tuition|fees?|contact|career|graduate)\b").expect("non-requirement inline label"))
}

fn preceding_inline_hard_boundary_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:admissions?|application|how do you major|contact|career)\b").expect("preceding inline hard boundary"))
}

pub fn is_pathway_section_boundary_line(line: &str) -> bool {
    pathway_section_boundary_pattern_source().is_match(line)
}

fn pathway_section_boundary_pattern_source() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:admissions?|advising|student resources?|degree requirements?|core courses?|honou?rs?\b|recommended courses?|language courses?|career options?|share|support us|contact us|privacy|terms|site map)\b").expect("pathway section boundary"))
}

fn structural_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(admission requirements|degree requirements|major requirements|completion requirements|required courses|elective courses|core courses|core requirements|curriculum|prerequisite courses|shared set of core courses|specialization|track|option|route|pathway|concentration|checklist|foundation|distribution requirement)\b").expect("structural requirement"))
}

fn primary_requirement_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bdegree requirements?\b|\bmajor requirements?\b|\bgraduation requirements?\b|\bprogram requirements?\b|\bdegree structure\b|\brequirements packet\b|\bdegreq\b").expect("primary requirement line"))
}

fn section_noise_penalty_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(admission|apply|application|sample schedule|four[-\s]?year plan|graduate)\b").expect("section noise penalty"))
}

fn legacy_catalog_credential_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^Bachelor\b.+\bdegree\b.+\bmajor\b").expect("legacy catalog credential heading"))
}

fn legacy_catalog_section_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bCredential Overview\b|\bAdditional Admission Requirements\b|\bCompletion Requirements\b").expect("legacy catalog section start"))
}

fn generic_pathway_hub_segment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:curriculum|overview|requirements?|degree-requirements?|major-requirements?|program-requirements?|options?|tracks?|routes?|pathways?|concentrations?|specializations?)$")
            .expect("generic pathway hub segment")
    })
}

fn generic_pathway_hash_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:content|main|top|curriculum|overview|requirements?|degree requirements?|major requirements?|program requirements?|options?|tracks?|routes?|pathways?|concentrations?|specializations?)$")
            .expect("generic pathway hash fragment")
    })
}

fn specific_pathway_segment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[-_])(?:option|track|route|pathway|concentration|specialization)(?:[-_]|$)")
            .expect("specific pathway segment")
    })
}

fn path_separator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[-_]+").expect("path separators"))
}

fn parser_recovery_title_scope_tokens(entry: &Value) -> Vec<String> {
    let value = format!("{} {}", string_field(entry, "ownerTitle"), string_field(entry, "sourceLabel"));
    normalize_matcher_text(&value)
        .split_whitespace()
        .filter(|token| token.len() >= 4 && !parser_recovery_title_scope_stopwords().contains(*token))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(8)
        .collect()
}

fn parser_recovery_title_scope_stopwords() -> BTreeSet<&'static str> {
    [
        "and", "arts", "bachelor", "bothell", "campus", "catalog", "course", "courses",
        "curriculum", "degree", "major", "of", "official", "overview", "page", "program",
        "requirements", "science", "seattle", "studies", "tacoma", "the", "university", "uw",
        "washington",
    ]
    .into_iter()
    .collect()
}

fn requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(required|requirements|prereq|prerequisite|complete|credits|credit|elective|select|choose|one of the following|two of the following|option|track|route|pathway|concentration)\b")
            .expect("requirement cue")
    })
}

fn graduate_supplemental_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(graduate|ph\.?\s*d\.?|doctor(?:al|ate)?|m\.?\s*a\.?|masters?|master(?:'s|s)?)\b|/(?:ma|phd|graduate|masters?|amp)(?:[-/]|$)")
            .expect("graduate supplemental source")
    })
}

fn selected_pathway_label_cue_pattern() -> &'static Regex {
    pathway_cue_pattern()
}

fn explicit_program_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\(([A-Z][A-Z0-9&\s/-]{1,12})\)").expect("explicit program acronym"))
}

fn route_ell_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(?:ell|english language learners?)\b").expect("ELL route alias"))
}

fn teaching_ell_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\bteaching english language learners?\b").expect("TELL route alias"))
}

fn route_sped_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(?:sped|special education)\b").expect("SPED route alias"))
}

fn candidate_option_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"\b([A-Z][A-Z0-9&]{1,8})\s+Option\b")
            .expect("candidate pathway option acronym")
    })
}

fn pathway_letter_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[A-Z]\.\s+").expect("pathway letter prefix"))
}

fn pathway_numeric_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\(?\d+[).]\s+").expect("pathway numeric prefix"))
}

fn pathway_roman_heading_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:[ivxlcdm]+)[).]\s+").expect("pathway Roman numeral prefix"))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

fn supplemental_official_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^\[Supplemental official source\]")
            .expect("supplemental official source label")
    })
}

fn pathway_label_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(option|track|route|pathway|concentration)\b")
            .expect("pathway label cue")
    })
}
