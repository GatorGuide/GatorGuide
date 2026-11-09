use super::requirement_course_codes::CourseCodeParser;
use super::requirement_docx::DocxSourceArtifacts;
use super::requirement_html::RequirementHtmlArtifacts;
use super::requirement_parse_helpers::{
    build_parse_confidence, extract_choose_statements, extract_relevant_requirement_lines,
    extract_requirement_cue_lines,
};
use super::requirement_pathway_labels::extract_pathway_labels;
use super::requirement_pdf_layout::{PdfPageBlock, scope_pdf_page_blocks};
use super::requirement_snapshots::RequirementSourceSnapshot;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceTextFacts {
    pub title: Option<String>,
    pub headings: Vec<String>,
    pub requirement_cue_lines: Vec<String>,
    pub choose_statements: Vec<String>,
    pub pathway_labels: Vec<String>,
    pub candidate_course_codes: Vec<String>,
    pub snapshot_lines: Vec<String>,
    pub parse_confidence: String,
    pub source_section_audit: Option<Value>,
}

pub fn collect_html_source_facts(
    artifacts: &RequirementHtmlArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_html_source_facts_for_entry(&Value::Null, artifacts, parser_type, course_parser)
}

pub fn collect_html_source_facts_for_entry(
    entry: &Value,
    artifacts: &RequirementHtmlArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_html_source_facts_for_entry_with_context(
        entry,
        artifacts,
        parser_type,
        course_parser,
        &std::collections::HashMap::new(),
        false,
    )
}

pub fn collect_html_source_facts_for_entry_with_primary_titles(
    entry: &Value,
    artifacts: &RequirementHtmlArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
    primary_major_titles: &std::collections::HashMap<String, String>,
) -> RequirementSourceTextFacts {
    collect_requirement_source_text_facts(
        entry,
        artifacts.title.clone(),
        artifacts.headings.clone(),
        artifacts.lines.clone(),
        parser_type,
        course_parser,
        primary_major_titles,
    )
}

pub fn collect_html_source_facts_for_entry_with_context(
    entry: &Value,
    artifacts: &RequirementHtmlArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
    primary_major_titles_override: &std::collections::HashMap<String, String>,
    plan_has_pathways: bool,
) -> RequirementSourceTextFacts {
    let primary_major_titles = primary_major_titles_override.clone();
    let is_bothell_ias_source =
        super::requirement_recovery_scope::is_bothell_ias_undergraduate_major_source(entry);
    let catalog_scope = scope_legacy_catalog_html_by_anchor(
        entry,
        &artifacts.html,
        &primary_major_titles,
        plan_has_pathways,
    );
    let catalog_scoped_lines = if is_bothell_ias_source {
        None
    } else {
        catalog_scope
            .as_ref()
            .and_then(|scope| scope.lines.as_ref())
            .filter(|lines| !lines.is_empty())
            .cloned()
    };
    let catalog_anchor_found = catalog_scope
        .as_ref()
        .and_then(|scope| scope.audit.get("anchorFound"))
        .and_then(Value::as_bool)
        == Some(true);
    let mut full_facts = collect_requirement_source_text_facts(
        entry,
        artifacts.title.clone(),
        artifacts.headings.clone(),
        artifacts.lines.clone(),
        parser_type,
        course_parser,
        &primary_major_titles,
    );
    full_facts.source_section_audit = catalog_scope.as_ref().map(|scope| scope.audit.clone());
    let current_catalog_lines = super::requirement_recovery_scope::scope_curriculum_era_lines(
        entry,
        &artifacts.lines,
        course_parser,
    );
    let source_lines = super::requirement_recovery_scope::
        remove_following_different_owner_acronym_departmental_sections(
            entry,
            &current_catalog_lines,
            &primary_major_titles,
        );
    let bothell_scope = super::requirement_recovery_scope::scope_bothell_chemistry_curriculum_lines(
        entry,
        &source_lines,
    );
    let is_authoritative_bothell_scope = bothell_scope.is_some();
    let scoped_lines = if let Some(catalog_lines) = catalog_scoped_lines.clone() {
        Some(catalog_lines)
    } else if is_authoritative_bothell_scope {
        bothell_scope
    } else if is_bothell_ias_source
        || super::requirement_recovery_scope::should_use_full_focused_pathway_degree_html_scope(
            entry,
            artifacts.title.as_deref(),
            &artifacts.headings,
            &source_lines,
            course_parser,
        )
        || super::requirement_recovery_scope::should_use_full_admission_prerequisite_html_scope(
            entry,
            &source_lines,
            course_parser,
        )
    {
        Some(source_lines.clone())
    } else {
        super::requirement_recovery_scope::find_parallel_pathway_html_table_scope(
            entry,
            &source_lines,
            &primary_major_titles,
            course_parser,
        )
        .map(|(_, _, parallel_lines)| {
            super::requirement_recovery_scope::preserve_prior_admit_pathway_context(
                entry,
                &source_lines,
                &parallel_lines,
                course_parser,
            )
        })
        .or_else(|| {
            super::requirement_recovery_scope::find_pathway_html_curriculum_section_candidate(
                entry,
                &source_lines,
                &primary_major_titles,
                course_parser,
            )
            .or_else(|| {
                super::requirement_recovery_scope::find_pathway_html_selected_section_candidate(
                    entry,
                    &source_lines,
                    &primary_major_titles,
                    course_parser,
                )
            })
            .map(|(start_index, end_index)| {
                super::requirement_recovery_scope::pathway_html_section_scope_lines(
                    entry,
                    &source_lines,
                    start_index,
                    end_index,
                    course_parser,
                )
            })
        })
        .or_else(|| {
            super::requirement_recovery_scope::catalog_owner_title_has_specialized_credential(
                entry,
            )
            .then(|| {
                super::requirement_recovery_scope::find_legacy_catalog_program_section_range(
                    entry,
                    &source_lines,
                    &primary_major_titles,
                    plan_has_pathways,
                    course_parser,
                )
            })
            .flatten()
            .map(|(start_index, end_index)| source_lines[start_index..=end_index].to_vec())
        })
        .or_else(|| {
            (super::requirement_recovery_scope::is_approved_elective_html_source(
                entry,
                artifacts.title.as_deref(),
                &artifacts.headings,
                &source_lines,
            ) || super::requirement_recovery_scope::should_use_full_generic_concentration_degree_scope(
                entry,
                &artifacts.headings,
                &source_lines,
                course_parser,
            ))
            .then(|| source_lines.clone())
        })
        .or_else(|| {
            let base_lines = super::requirement_recovery_scope::
                remove_parallel_pathway_html_table_content_from_base_scope(
                    entry,
                    &source_lines,
                    course_parser,
                );
            (base_lines != source_lines).then_some(base_lines)
        })
        .or_else(|| {
            super::requirement_recovery_scope::should_use_full_focused_degree_html_scope(
                entry,
                artifacts.title.as_deref(),
                &artifacts.headings,
                &source_lines,
                &primary_major_titles,
                plan_has_pathways,
                course_parser,
            )
            .then(|| source_lines.clone())
        })
        .or_else(|| (source_lines != artifacts.lines).then(|| source_lines.clone()))
        .or_else(|| {
            super::requirement_recovery_scope::find_legacy_catalog_program_section_range(
                entry,
                &source_lines,
                &primary_major_titles,
                plan_has_pathways,
                course_parser,
            )
            .map(|(start_index, end_index)| source_lines[start_index..=end_index].to_vec())
        })
        .or_else(|| {
            Some(super::requirement_recovery_scope::scope_html_lines_by_title(
                entry,
                artifacts.title.as_deref(),
                &artifacts.headings,
                &source_lines,
                &primary_major_titles,
                course_parser,
            ))
        })
    };
    let Some(scoped_lines) = scoped_lines else {
        return full_facts;
    };
    if catalog_scoped_lines.is_none() && scoped_lines.as_slice() == artifacts.lines.as_slice() {
        return full_facts;
    }
    let scoped_lines = super::requirement_groups::append_sbse_comparison_table_scope_lines(
        entry,
        &source_lines,
        &scoped_lines,
        course_parser,
    );
    let scoped_headings = if catalog_scoped_lines.is_some() {
        catalog_scope
            .as_ref()
            .map(|scope| scope.headings.clone())
            .filter(|headings| !headings.is_empty())
            .unwrap_or_else(|| artifacts.headings.clone())
    } else {
        artifacts.headings.clone()
    };
    let mut scoped_facts = collect_requirement_source_text_facts(
        entry,
        artifacts.title.clone(),
        scoped_headings,
        scoped_lines,
        parser_type,
        course_parser,
        &primary_major_titles,
    );
    scoped_facts.source_section_audit = catalog_scope.as_ref().map(|scope| scope.audit.clone());
    let is_legacy_catalog_source =
        super::requirement_recovery_scope::is_legacy_student_catalog_source(entry);
    let scoped_mentions_selected_pathway =
        super::requirement_recovery_scope::parsed_mentions_selected_pathway_section(
            entry,
            &scoped_facts.snapshot_lines,
            &primary_major_titles,
            course_parser,
        );
    let entry_has_pathway = entry.get("pathwayId").is_some_and(is_truthy_json_value);
    let scoped_removes_parallel_pathway_table = !entry_has_pathway
        && super::requirement_recovery_scope::remove_parallel_pathway_html_table_content_from_base_scope(
            entry,
            &full_facts.snapshot_lines,
            course_parser,
        )
        .len()
            < full_facts.snapshot_lines.len();
    if !scoped_mentions_selected_pathway
        && !scoped_removes_parallel_pathway_table
        && !is_legacy_catalog_source
        && matches!(parser_type, "html-degree-page" | "html-curriculum-page")
    {
        append_focused_scoped_html_overflow(&full_facts, &mut scoped_facts, course_parser);
    }
    let scoped_course_count = scoped_facts.candidate_course_codes.len();
    let legacy_catalog_scope_has_course_evidence =
        is_legacy_catalog_source && scoped_course_count >= 2;
    let full_alignment =
        super::requirement_source_scoring::get_parsed_owner_alignment_score(entry, &full_facts);
    let scoped_alignment =
        super::requirement_source_scoring::get_parsed_owner_alignment_score(entry, &scoped_facts);
    let full_course_count = full_facts.candidate_course_codes.len();
    let scoped_matches_pathway_sibling_context = entry_has_pathway
        && scoped_course_count >= 1
        && super::requirement_recovery_scope::parsed_mentions_selected_pathway_identity(
            entry,
            &scoped_facts.snapshot_lines,
            &primary_major_titles,
        )
        && super::requirement_recovery_scope::parsed_mentions_sibling_pathway_section(
            entry,
            &full_facts.snapshot_lines,
            &primary_major_titles,
            course_parser,
        );
    let dedicated_selected_pathway_source =
        super::requirement_recovery_scope::is_dedicated_selected_pathway_html_source(
            entry,
            full_facts.title.as_deref(),
            &primary_major_titles,
        );
    let dedicated_selected_pathway_route_source =
        super::requirement_plan_matching::is_dedicated_selected_pathway_route_source(
            entry,
            full_facts.title.as_deref(),
        );
    if (is_authoritative_bothell_scope && scoped_course_count >= 1)
        || catalog_anchor_found
        || legacy_catalog_scope_has_course_evidence
        || (full_alignment < 2 && scoped_alignment >= 2)
    {
        return scoped_facts;
    }
    if dedicated_selected_pathway_source
        && (!scoped_matches_pathway_sibling_context || dedicated_selected_pathway_route_source)
        && full_course_count > scoped_course_count
        && full_course_count
            <= (scoped_course_count.saturating_add(80)).max(scoped_course_count.saturating_mul(4))
        && full_alignment >= 1.max(scoped_alignment.saturating_sub(2))
    {
        return full_facts;
    }
    if (scoped_mentions_selected_pathway && scoped_course_count >= 1)
        || scoped_matches_pathway_sibling_context
        || (scoped_course_count >= 4
            && ((super::requirement_recovery_scope::has_owner_acronym_departmental_boundary(
                entry,
                &full_facts.snapshot_lines,
                &primary_major_titles,
            ) && super::requirement_recovery_scope::find_different_owner_acronym_departmental_boundary_index(
                entry,
                &full_facts.snapshot_lines,
                &primary_major_titles,
            )
            .is_some())
                || (super::requirement_recovery_scope::has_owner_acronym_departmental_boundary(
                    entry,
                    &scoped_facts.snapshot_lines,
                    &primary_major_titles,
                ) && super::requirement_recovery_scope::find_different_owner_acronym_departmental_boundary_index(
                    entry,
                    &full_facts.snapshot_lines,
                    &primary_major_titles,
                )
                .is_some())))
        || (scoped_removes_parallel_pathway_table
            && scoped_course_count >= 1
            && !entry_has_pathway)
    {
        return scoped_facts;
    }

    let full_has_requirement_anchors = full_facts
        .snapshot_lines
        .iter()
        .any(|line| requirement_anchor_heading_pattern().is_match(&normalize_whitespace(line)));
    let full_lower_division_count =
        super::requirement_source_scoring::get_parsed_lower_division_course_count(&full_facts);
    let scoped_lower_division_count =
        super::requirement_source_scoring::get_parsed_lower_division_course_count(&scoped_facts);
    let full_subjects = super::requirement_source_scoring::get_parsed_course_subjects(&full_facts);
    let scoped_subjects =
        super::requirement_source_scoring::get_parsed_course_subjects(&scoped_facts);
    if full_has_requirement_anchors
        && full_alignment >= 2.max(scoped_alignment.saturating_sub(1))
        && full_lower_division_count >= 4.max(scoped_lower_division_count.saturating_add(4))
        && scoped_lower_division_count <= 1.max(scoped_course_count / 4)
    {
        return full_facts;
    }
    if !entry_has_pathway
        && full_has_requirement_anchors
        && full_facts.candidate_course_codes.len() >= scoped_course_count.saturating_add(12)
        && full_alignment >= 1.max(scoped_alignment.saturating_sub(2))
    {
        return full_facts;
    }
    if scoped_alignment >= full_alignment.saturating_add(1)
        && scoped_course_count >= 2.max(full_facts.candidate_course_codes.len() / 4)
    {
        return scoped_facts;
    }
    if full_facts.candidate_course_codes.len() >= 80
        && scoped_alignment >= full_alignment
        && scoped_course_count >= 4
    {
        return scoped_facts;
    }
    if scoped_alignment >= full_alignment
        && scoped_course_count >= 2
        && !scoped_subjects.is_empty()
        && full_subjects.len() >= scoped_subjects.len().saturating_add(2)
    {
        return scoped_facts;
    }
    full_facts
}

fn append_focused_scoped_html_overflow(
    full_facts: &RequirementSourceTextFacts,
    scoped_facts: &mut RequirementSourceTextFacts,
    course_parser: &CourseCodeParser,
) {
    let scoped_subjects =
        super::requirement_source_scoring::get_parsed_course_subjects(scoped_facts)
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
    if scoped_subjects.is_empty() {
        return;
    }
    let scoped_codes = scoped_facts
        .candidate_course_codes
        .iter()
        .map(|code| super::requirement_course_codes::normalize_course_code(code))
        .collect::<std::collections::HashSet<_>>();
    let mut safe_overflow_codes = full_facts
        .candidate_course_codes
        .iter()
        .map(|code| super::requirement_course_codes::normalize_course_code(code))
        .filter(|code| !scoped_codes.contains(code))
        .filter(|code| {
            super::requirement_source_scoring::get_course_code_numeric_level(code)
                .is_some_and(|level| level < 300)
        })
        .filter(|code| {
            super::requirement_source_scoring::get_course_code_subject(code)
                .is_some_and(|subject| scoped_subjects.contains(&subject))
        })
        .collect::<Vec<_>>();
    safe_overflow_codes.sort();
    safe_overflow_codes.dedup();
    if safe_overflow_codes.is_empty() {
        return;
    }
    let safe_overflow_code_set = safe_overflow_codes
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let safe_overflow_snapshot_lines = full_facts
        .snapshot_lines
        .iter()
        .filter(|line| {
            course_parser.extract_from_line(line).iter().any(|code| {
                safe_overflow_code_set.contains(
                    &super::requirement_course_codes::normalize_course_code(code),
                )
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    scoped_facts
        .candidate_course_codes
        .extend(safe_overflow_codes);
    scoped_facts.candidate_course_codes.sort();
    scoped_facts.candidate_course_codes.dedup();
    for line in safe_overflow_snapshot_lines {
        if !scoped_facts.snapshot_lines.contains(&line) {
            scoped_facts.snapshot_lines.push(line);
        }
    }
}

struct CatalogAnchorScopeResult {
    lines: Option<Vec<String>>,
    headings: Vec<String>,
    audit: Value,
}

fn scope_legacy_catalog_html_by_anchor(
    entry: &Value,
    html: &str,
    primary_major_titles: &std::collections::HashMap<String, String>,
    plan_has_pathways: bool,
) -> Option<CatalogAnchorScopeResult> {
    if !super::requirement_recovery_scope::is_legacy_student_catalog_source(entry) {
        return None;
    }
    let source_url = entry.get("url").and_then(Value::as_str)?;
    let source_role = super::requirement_role_classifier::classify_requirement_source_role(entry);
    let Some(requested_anchor) =
        super::requirement_source_recovery::get_requested_url_anchor(source_url)
    else {
        return scope_legacy_catalog_html_without_anchor(
            entry,
            html,
            source_role,
            primary_major_titles,
        );
    };
    let Some(anchor_match) =
        super::requirement_html::find_catalog_anchor_tag(html, &requested_anchor)
    else {
        return Some(CatalogAnchorScopeResult {
            lines: None,
            headings: Vec::new(),
            audit: build_catalog_source_section_audit(
                entry,
                source_role,
                Some(&requested_anchor),
                false,
                None,
                false,
                "fallback to scored HTML scope",
                Vec::new(),
            ),
        });
    };

    let peer_stop_boundary = super::requirement_html::find_next_catalog_peer_boundary(
        html,
        anchor_match.end_index,
        Some(&requested_anchor),
    );
    let should_stop_before_credentials = entry.get("ownerType").and_then(Value::as_str)
        == Some("major")
        && !entry.get("pathwayId").is_some_and(is_truthy_json_value)
        && requested_anchor
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("program-"))
        && plan_has_pathways;
    if should_stop_before_credentials {
        if let Some(base_section) =
            super::requirement_recovery_scope::find_base_catalog_credential_section(
                entry,
                html,
                anchor_match.end_index,
                peer_stop_boundary.index,
                primary_major_titles,
            )
            .filter(|section| !section.lines.is_empty())
        {
            let program_section_html = html.get(anchor_match.index..peer_stop_boundary.index)?;
            let program_section_lines =
                super::requirement_html::build_html_lines(program_section_html);
            let program_section_headings =
                super::requirement_html::extract_headings(program_section_html);
            let program_section_heading = program_section_headings
                .first()
                .filter(|heading| !heading.is_empty())
                .cloned()
                .or_else(|| {
                    (!anchor_match.heading.is_empty()).then(|| anchor_match.heading.clone())
                });
            let keep_lead_in =
                should_keep_legacy_catalog_program_lead_in(&program_section_lines, &base_section);
            let (section_lines, section_headings, section_heading) = if keep_lead_in {
                (
                    build_legacy_catalog_program_credential_scope_lines(
                        &program_section_lines,
                        &base_section,
                    ),
                    unique_strings_in_order(
                        program_section_headings
                            .into_iter()
                            .chain(base_section.headings.iter().cloned()),
                    ),
                    program_section_heading.unwrap_or_else(|| base_section.section_heading.clone()),
                )
            } else {
                (
                    base_section.lines.clone(),
                    base_section.headings.clone(),
                    base_section.section_heading.clone(),
                )
            };
            let section_matched =
                super::requirement_recovery_scope::catalog_section_matches_selected_major(
                    entry,
                    &section_heading,
                    &section_lines,
                    primary_major_titles,
                );
            let ignored = super::requirement_html::collect_ignored_catalog_neighboring_sections(
                html,
                peer_stop_boundary.index,
                if keep_lead_in {
                    Some(requested_anchor.as_str())
                } else {
                    Some("credential-derived")
                },
            );
            let audit = build_catalog_source_section_audit(
                entry,
                source_role,
                Some(&requested_anchor),
                true,
                Some(&section_heading),
                section_matched,
                &base_section.stop_boundary,
                ignored,
            );
            return Some(CatalogAnchorScopeResult {
                lines: Some(section_lines),
                headings: section_headings,
                audit,
            });
        }
    }

    let credential_stop_boundary = if should_stop_before_credentials {
        super::requirement_html::find_next_catalog_credential_boundary(
            html,
            anchor_match.end_index,
            peer_stop_boundary.index,
        )
    } else {
        None
    };
    let stop_boundary = credential_stop_boundary.unwrap_or(peer_stop_boundary);
    let section_html = html.get(anchor_match.index..stop_boundary.index)?;
    let section_lines = super::requirement_html::build_html_lines(section_html);
    let section_headings = super::requirement_html::extract_headings(section_html);
    let section_heading = section_headings
        .first()
        .cloned()
        .unwrap_or(anchor_match.heading);
    let section_matched = super::requirement_recovery_scope::catalog_section_matches_selected_major(
        entry,
        &section_heading,
        &section_lines,
        primary_major_titles,
    );
    let ignored = super::requirement_html::collect_ignored_catalog_neighboring_sections(
        html,
        stop_boundary.index,
        Some(&requested_anchor),
    );
    let audit = build_catalog_source_section_audit(
        entry,
        source_role,
        Some(&requested_anchor),
        true,
        Some(&section_heading),
        section_matched,
        &stop_boundary.label,
        ignored,
    );
    Some(CatalogAnchorScopeResult {
        lines: Some(section_lines),
        headings: section_headings,
        audit,
    })
}

fn scope_legacy_catalog_html_without_anchor(
    entry: &Value,
    html: &str,
    source_role: &str,
    primary_major_titles: &std::collections::HashMap<String, String>,
) -> Option<CatalogAnchorScopeResult> {
    if !is_legacy_catalog_html_parser_type(entry) {
        return None;
    }
    if entry.get("pathwayId").is_some_and(is_truthy_json_value) {
        for heading_match in super::requirement_html::find_catalog_credential_heading_matches(html)
        {
            let section_heading = heading_match.heading;
            if !catalog_credential_heading_matches_owner_major(
                entry,
                &section_heading,
                primary_major_titles,
            )
                || !super::requirement_recovery_scope::line_matches_strict_selected_pathway_identity(
                    entry,
                    &section_heading,
                    primary_major_titles,
                )
            {
                continue;
            }
            let stop_boundary = super::requirement_html::find_next_catalog_peer_boundary(
                html,
                heading_match.end_index,
                Some("credential-derived"),
            );
            let section_html = html.get(heading_match.index..stop_boundary.index)?;
            let lines = super::requirement_html::build_html_lines(section_html);
            let headings = super::requirement_html::extract_headings(section_html);
            let audit = build_catalog_source_section_audit(
                entry,
                source_role,
                None,
                false,
                Some(&section_heading),
                true,
                &stop_boundary.label,
                super::requirement_html::collect_ignored_catalog_neighboring_sections(
                    html,
                    stop_boundary.index,
                    Some("credential-derived"),
                ),
            );
            return Some(CatalogAnchorScopeResult {
                lines: Some(lines),
                headings,
                audit,
            });
        }
        return None;
    }

    for program_match in
        super::requirement_html::find_catalog_program_expandable_group_matches(html)
    {
        let stop_boundary = super::requirement_html::find_next_catalog_peer_boundary(
            html,
            program_match.stop_search_start,
            Some("program"),
        );
        let section_html = html.get(program_match.index..stop_boundary.index)?;
        let section_lines = super::requirement_html::build_html_lines(section_html);
        let section_headings = super::requirement_html::extract_headings(section_html);
        let catalog_program_heading = section_headings.iter().find(|heading| {
            legacy_catalog_program_heading_pattern().is_match(&normalize_whitespace(heading))
        });
        let section_heading = catalog_program_heading
            .cloned()
            .or_else(|| section_headings.first().cloned())
            .or_else(|| {
                let stripped = super::requirement_html::strip_html(
                    &html[program_match.index..program_match.end_index],
                );
                (!stripped.is_empty()).then_some(stripped)
            });
        let section_matched = if let Some(program_heading) = catalog_program_heading {
            super::requirement_recovery_scope::legacy_catalog_program_line_matches_owner(
                entry,
                program_heading,
                primary_major_titles,
            )
        } else {
            super::requirement_recovery_scope::catalog_section_matches_selected_major(
                entry,
                section_heading.as_deref().unwrap_or_default(),
                &section_lines,
                primary_major_titles,
            )
        };
        if !section_matched {
            continue;
        }

        let base_section = super::requirement_recovery_scope::find_base_catalog_credential_section(
            entry,
            html,
            program_match.stop_search_start,
            stop_boundary.index,
            primary_major_titles,
        )
        .filter(|section| !section.lines.is_empty());
        if let Some(base_section) = base_section {
            if should_keep_legacy_catalog_program_lead_in(&section_lines, &base_section) {
                let audit = build_catalog_source_section_audit(
                    entry,
                    source_role,
                    None,
                    false,
                    section_heading.as_deref(),
                    true,
                    &stop_boundary.label,
                    super::requirement_html::collect_ignored_catalog_neighboring_sections(
                        html,
                        stop_boundary.index,
                        Some("program"),
                    ),
                );
                return Some(CatalogAnchorScopeResult {
                    lines: Some(section_lines),
                    headings: section_headings,
                    audit,
                });
            }
            let audit = build_catalog_source_section_audit(
                entry,
                source_role,
                None,
                false,
                Some(&base_section.section_heading),
                true,
                &base_section.stop_boundary,
                super::requirement_html::collect_ignored_catalog_neighboring_sections(
                    html,
                    stop_boundary.index,
                    Some("credential-derived"),
                ),
            );
            return Some(CatalogAnchorScopeResult {
                lines: Some(base_section.lines),
                headings: base_section.headings,
                audit,
            });
        }

        let audit = build_catalog_source_section_audit(
            entry,
            source_role,
            None,
            false,
            section_heading.as_deref(),
            section_matched,
            &stop_boundary.label,
            super::requirement_html::collect_ignored_catalog_neighboring_sections(
                html,
                stop_boundary.index,
                Some("program"),
            ),
        );
        return Some(CatalogAnchorScopeResult {
            lines: Some(section_lines),
            headings: section_headings,
            audit,
        });
    }
    None
}

fn is_legacy_catalog_html_parser_type(entry: &Value) -> bool {
    matches!(
        entry.get("parserType").and_then(Value::as_str),
        Some("catalog-page" | "html-degree-page" | "html-curriculum-page" | "html-overview-page")
    )
}

fn catalog_credential_heading_matches_owner_major(
    entry: &Value,
    heading: &str,
    primary_major_titles: &std::collections::HashMap<String, String>,
) -> bool {
    let credential_title =
        super::requirement_html::normalize_catalog_base_credential_heading(heading);
    let major_title_part = super::transfer_plan_title_matching::normalize_transfer_planner_text(
        credential_title.split(':').next().unwrap_or_default(),
    );
    !major_title_part.is_empty()
        && (super::requirement_recovery_scope::text_matches_catalog_owner_scope_title(
            entry,
            &major_title_part,
            primary_major_titles,
        ) || super::requirement_recovery_scope::catalog_section_matches_selected_major(
            entry,
            &major_title_part,
            std::slice::from_ref(&major_title_part),
            primary_major_titles,
        ))
}

fn legacy_catalog_program_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Program of Study:\s*Major:",
        );
        Regex::new(&pattern).expect("legacy catalog program heading pattern")
    })
}

fn should_keep_legacy_catalog_program_lead_in(
    section_lines: &[String],
    base_section: &super::requirement_recovery_scope::CatalogCredentialSection,
) -> bool {
    if base_section.lines.is_empty() || section_lines.is_empty() {
        return false;
    }
    let Some(credential_index) =
        get_legacy_catalog_program_lead_in_end_index(section_lines, base_section)
    else {
        return false;
    };
    if credential_index == 0 {
        return false;
    }
    let lead_in_text = super::requirement_recovery_scope::normalize_matcher_text(
        &section_lines[..credential_index].join(" "),
    );
    legacy_catalog_program_lead_in_cue_pattern().is_match(&lead_in_text)
}

fn get_legacy_catalog_program_lead_in_end_index(
    section_lines: &[String],
    base_section: &super::requirement_recovery_scope::CatalogCredentialSection,
) -> Option<usize> {
    let completion_index = section_lines.iter().position(|line| {
        legacy_catalog_completion_requirements_pattern().is_match(&normalize_whitespace(line))
    });
    let credential_heading =
        super::requirement_recovery_scope::normalize_matcher_text(&base_section.section_heading);
    let heading_index = if credential_heading.is_empty() {
        None
    } else {
        section_lines.iter().position(|line| {
            super::requirement_recovery_scope::normalize_matcher_text(line)
                .contains(&credential_heading)
        })
    };
    match (completion_index, heading_index) {
        (None, heading) => heading,
        (Some(completion), Some(heading)) if heading > completion => Some(heading),
        (Some(completion), _) => Some(completion),
    }
}

fn build_legacy_catalog_program_credential_scope_lines(
    section_lines: &[String],
    base_section: &super::requirement_recovery_scope::CatalogCredentialSection,
) -> Vec<String> {
    let lead_in_lines = get_legacy_catalog_program_lead_in_end_index(section_lines, base_section)
        .filter(|index| *index > 0)
        .map(|index| section_lines[..index].to_vec())
        .unwrap_or_default();
    unique_strings_in_order(
        lead_in_lines
            .into_iter()
            .chain(base_section.lines.iter().cloned()),
    )
}

fn unique_strings_in_order(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn legacy_catalog_program_lead_in_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:admission requirements?|prerequisites?|pre[-\s]?requisite|how to apply to the major|transfer(?:ring)? from a community college|associate of science transfer|recommended preparation)\b",
        );
        Regex::new(&pattern).expect("legacy catalog program lead-in cue pattern")
    })
}

fn legacy_catalog_completion_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^Completion Requirements\b",
        );
        Regex::new(&pattern).expect("legacy catalog completion requirements heading")
    })
}

fn requirement_anchor_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:major admissions requirements|admission requirements|degree requirements|major requirements|curriculum)$",
        )
        .expect("requirement anchor heading")
    })
}

fn build_catalog_source_section_audit(
    entry: &Value,
    source_role: &str,
    requested_anchor: Option<&str>,
    anchor_found: bool,
    section_heading: Option<&str>,
    section_matched_selected_major: bool,
    stop_boundary: &str,
    ignored_neighboring_sections: Vec<String>,
) -> Value {
    let major_id = audit_value_text(entry.get("planId"), "unknown");
    let source_url = audit_value_text(entry.get("url"), "n/a");
    let section_heading_text = section_heading.unwrap_or("n/a");
    let ignored_text = if ignored_neighboring_sections.is_empty() {
        "none".to_string()
    } else {
        ignored_neighboring_sections.join(" | ")
    };
    let requested_anchor_text = requested_anchor.unwrap_or("none");
    let line = format!(
        "[source section audit] Major id: {major_id} Source URL: {source_url} Source role: {source_role} Requested anchor: {requested_anchor_text} Anchor found: {} Section heading: {section_heading_text} Section matched selected major: {} Stop boundary: {stop_boundary} Ignored neighboring sections: {ignored_text}",
        if anchor_found { "yes" } else { "no" },
        if section_matched_selected_major {
            "yes"
        } else {
            "no"
        },
    );
    let mut audit = serde_json::Map::new();
    audit.insert("line".to_string(), Value::String(line));
    if let Some(value) = entry.get("planId") {
        audit.insert("majorId".to_string(), value.clone());
    }
    if let Some(value) = entry.get("url") {
        audit.insert("sourceUrl".to_string(), value.clone());
    }
    audit.insert(
        "sourceRole".to_string(),
        Value::String(source_role.to_string()),
    );
    audit.insert(
        "requestedAnchor".to_string(),
        requested_anchor
            .map(|anchor| Value::String(anchor.to_string()))
            .unwrap_or(Value::Null),
    );
    audit.insert("anchorFound".to_string(), Value::Bool(anchor_found));
    audit.insert(
        "sectionHeading".to_string(),
        section_heading
            .map(|heading| Value::String(heading.to_string()))
            .unwrap_or(Value::Null),
    );
    audit.insert(
        "sectionMatchedSelectedMajor".to_string(),
        Value::Bool(section_matched_selected_major),
    );
    audit.insert(
        "stopBoundary".to_string(),
        Value::String(stop_boundary.to_string()),
    );
    audit.insert(
        "ignoredNeighboringSections".to_string(),
        serde_json::json!(ignored_neighboring_sections),
    );
    Value::Object(audit)
}

fn audit_value_text(value: Option<&Value>, fallback: &str) -> String {
    match value {
        None | Some(Value::Null) => fallback.to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => super::requirement_shape::js_number_to_string(value),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                Value::Null => String::new(),
                _ => audit_value_text(Some(value), ""),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

fn is_truthy_json_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

pub fn collect_docx_source_facts(
    artifacts: &DocxSourceArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_docx_source_facts_for_entry(&Value::Null, artifacts, parser_type, course_parser)
}

pub fn collect_docx_source_facts_for_entry(
    entry: &Value,
    artifacts: &DocxSourceArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_docx_source_facts_for_entry_with_primary_titles(
        entry,
        artifacts,
        parser_type,
        course_parser,
        &std::collections::HashMap::new(),
    )
}

pub fn collect_docx_source_facts_for_entry_with_primary_titles(
    entry: &Value,
    artifacts: &DocxSourceArtifacts,
    parser_type: &str,
    course_parser: &CourseCodeParser,
    primary_major_titles: &std::collections::HashMap<String, String>,
) -> RequirementSourceTextFacts {
    collect_requirement_source_text_facts(
        entry,
        artifacts.title.clone(),
        Vec::new(),
        artifacts.lines.clone(),
        parser_type,
        course_parser,
        primary_major_titles,
    )
}

pub fn collect_snapshot_source_facts(
    entry: &Value,
    snapshot: &RequirementSourceSnapshot,
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_snapshot_source_facts_with_primary_titles(
        entry,
        snapshot,
        course_parser,
        &std::collections::HashMap::new(),
    )
}

pub fn collect_snapshot_source_facts_with_primary_titles(
    entry: &Value,
    snapshot: &RequirementSourceSnapshot,
    course_parser: &CourseCodeParser,
    primary_major_titles: &std::collections::HashMap<String, String>,
) -> RequirementSourceTextFacts {
    let parser_type = entry
        .get("parserType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    collect_requirement_source_text_facts(
        entry,
        snapshot.title.clone(),
        snapshot.headings.clone(),
        snapshot.snapshot_lines.clone(),
        parser_type,
        course_parser,
        primary_major_titles,
    )
}

pub fn collect_pdf_source_facts(
    entry: &Value,
    page_blocks: &[PdfPageBlock],
    course_parser: &CourseCodeParser,
) -> RequirementSourceTextFacts {
    collect_pdf_source_facts_with_primary_titles(
        entry,
        page_blocks,
        course_parser,
        &std::collections::HashMap::new(),
    )
}

pub fn collect_pdf_source_facts_with_primary_titles(
    entry: &Value,
    page_blocks: &[PdfPageBlock],
    course_parser: &CourseCodeParser,
    primary_major_titles: &std::collections::HashMap<String, String>,
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
        entry,
        title,
        Vec::new(),
        scoped_lines,
        parser_type,
        course_parser,
        primary_major_titles,
    )
}

fn collect_requirement_source_text_facts(
    entry: &Value,
    title: Option<String>,
    headings: Vec<String>,
    lines: Vec<String>,
    parser_type: &str,
    course_parser: &CourseCodeParser,
    primary_major_titles: &std::collections::HashMap<String, String>,
) -> RequirementSourceTextFacts {
    let requirement_cue_lines = extract_requirement_cue_lines(&lines, course_parser);
    let choose_statements = extract_choose_statements(&lines);
    let relevant_lines = extract_relevant_requirement_lines(&lines, &headings);
    let raw_candidate_course_codes = relevant_lines
        .iter()
        .flat_map(|line| course_parser.extract_from_line(line))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let candidate_course_codes =
        super::requirement_hint_safety::filter_parsed_course_codes_by_hints_with_source_recovery(
            entry,
            &lines,
            &raw_candidate_course_codes,
            course_parser,
        );
    let pathway_labels = extract_pathway_labels(
        entry,
        &lines,
        &headings,
        course_parser,
        primary_major_titles,
    );
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
        pathway_labels,
        candidate_course_codes,
        snapshot_lines: lines.into_iter().take(1_200).collect(),
        parse_confidence,
        source_section_audit: None,
    }
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn page_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(r"^\[Page \d+\]\s*");
        Regex::new(&pattern).expect("PDF page label prefix")
    })
}
