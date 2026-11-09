use super::requirement_bootstrap_index::RequirementBootstrapPlanIndex;
use super::requirement_course_codes::CourseCodeParser;
use super::requirement_course_scoping::filter_graduate_cross_listed_companions_for_undergraduate_source;
use super::requirement_degree_map_blocks::get_structured_uw_course_codes;
use super::requirement_docx::{get_linked_document_parser_type, is_linked_document_source_url};
use super::requirement_manifest_fallback::{
    build_parser_recovery_candidate_entry, get_parser_recovery_candidate_parser_type,
    select_alternate_parseable_manifest_entries,
};
use super::requirement_manifest_selection::select_parseable_manifest_entries;
use super::requirement_plan_matching::{
    build_primary_major_titles_by_plan_id, manifest_entry_matches_specialized_plan_title,
    parser_recovery_candidate_conflicts_with_owner_identity,
};
use super::requirement_recovery_links::{
    has_primary_requirement_link_cue, has_recovery_link_signal, is_graduate_supplemental_source,
    is_ignored_parser_recovery_href, is_noisy_parser_recovery_link, is_official_linked_asset_url,
    parser_recovery_title_token_count, score_parser_recovery_link_candidate,
    score_supplemental_document_link_candidate,
};
use super::requirement_recovery_scope::{
    catalog_owner_title_has_specialized_credential, find_nested_pathway_html_section_scope,
    find_parallel_pathway_html_table_scope, find_pathway_html_curriculum_section_candidate,
    find_pathway_html_selected_section_candidate, find_preceding_inline_requirement_start_index,
    has_parser_recovery_requirement_cue, is_back_to_top_heading_line,
    is_different_program_sibling_page, is_html_section_boundary_line,
    is_legacy_catalog_credential_heading_line, is_parser_recovery_pathway_section_boundary_line,
    is_parser_recovery_section_boundary_line,
    line_looks_like_legacy_catalog_credential_section_start,
    parser_recovery_candidate_conflicts_with_base_owner_scope,
    parser_recovery_candidate_conflicts_with_pathway_scope,
    parser_recovery_candidate_conflicts_with_program_sibling,
    parser_recovery_candidate_contains_selected_pathway_phrase,
    parser_recovery_owner_program_acronym_tokens, parser_recovery_section_line_score,
    parser_recovery_snapshot_conflicts_with_base_owner_scope, pathway_html_section_scope_lines,
    preserve_prior_admit_pathway_context,
    remove_following_different_owner_acronym_departmental_sections,
    remove_parallel_pathway_html_table_content_from_base_scope, scope_curriculum_era_lines,
    score_owner_snapshot_for_parser_recovery, should_use_full_generic_concentration_degree_scope,
};
use super::requirement_role_classifier::{
    classify_requirement_source_role, has_conflicting_supplemental_degree_route,
};
use super::requirement_scope::get_requirement_source_role_status;
use super::requirement_snapshots::{
    RequirementSnapshotStore, RequirementSourceSnapshot, source_url_without_hash,
};
use super::requirement_source_recovery::get_requested_url_anchor;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use url::Url;

fn is_parseable_manifest_parser_type(value: Option<&Value>) -> bool {
    matches!(
        value.and_then(Value::as_str),
        Some(
            "html-degree-page"
                | "html-curriculum-page"
                | "html-admissions-page"
                | "html-overview-page"
                | "catalog-page"
                | "generic-html"
                | "pdf-degree-sheet"
                | "pdf-worksheet"
                | "generic-pdf"
        )
    )
}

fn is_legacy_student_catalog_source(entry: &Value) -> bool {
    let Some(url) = entry.get("url").and_then(Value::as_str) else {
        return false;
    };
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    matches!(
        parsed.host_str(),
        Some("washington.edu" | "www.washington.edu")
    ) && parsed.path().starts_with("/students/gencat/program/")
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

#[derive(Debug, Clone, Default)]
pub struct RequirementSourceManifestContext {
    manifest_entries: Vec<Value>,
    degree_map_blocks: Vec<Value>,
    bootstrap_plan_index: RequirementBootstrapPlanIndex,
    primary_major_titles: HashMap<String, String>,
}

impl RequirementSourceManifestContext {
    pub fn new(
        manifest_entries: Vec<Value>,
        bootstrap_plans: &[Value],
        degree_map_blocks: Vec<Value>,
    ) -> Self {
        let bootstrap_plan_index = RequirementBootstrapPlanIndex::from_plans(bootstrap_plans);
        let primary_major_titles = build_primary_major_titles_by_plan_id(&manifest_entries);
        Self {
            manifest_entries,
            degree_map_blocks,
            bootstrap_plan_index,
            primary_major_titles,
        }
    }

    pub fn manifest_entries(&self) -> &[Value] {
        &self.manifest_entries
    }

    pub fn bootstrap_plan_index(&self) -> &RequirementBootstrapPlanIndex {
        &self.bootstrap_plan_index
    }

    pub fn primary_major_titles(&self) -> &HashMap<String, String> {
        &self.primary_major_titles
    }

    pub fn degree_map_blocks(&self) -> &[Value] {
        &self.degree_map_blocks
    }

    pub fn parser_recovery_catalog_scope_flags(
        &self,
        entry: &Value,
        owner: &Value,
    ) -> (bool, bool) {
        if !is_legacy_student_catalog_source(entry) {
            return (false, false);
        }
        let anchored_section = entry
            .get("url")
            .and_then(Value::as_str)
            .and_then(get_requested_url_anchor)
            .is_some()
            && owner
                .get("sourceSectionAudit")
                .and_then(|audit| audit.get("anchorFound"))
                .and_then(Value::as_bool)
                == Some(true);
        let specialized_credential = catalog_owner_title_has_specialized_credential(entry);
        let owner_is_major = entry.get("ownerType").and_then(Value::as_str) == Some("major");
        let has_pathway = entry
            .get("pathwayId")
            .is_some_and(|value| !value.is_null() && value.as_str() != Some(""));
        let plan_id = entry
            .get("planId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let stop_before_child_credentials = owner_is_major
            && !has_pathway
            && !specialized_credential
            && self.bootstrap_plan_index.pathway_count(plan_id) > 0;
        (
            anchored_section || specialized_credential || stop_before_child_credentials,
            anchored_section || specialized_credential,
        )
    }

    pub fn select_parseable_entries(
        &self,
        target_plan_ids: Option<&[String]>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        select_parseable_manifest_entries(
            &self.manifest_entries,
            target_plan_ids,
            self.bootstrap_plan_index.plan_titles_by_id(),
            &self.primary_major_titles,
            course_parser,
        )
    }

    pub fn select_alternate_entries(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        select_alternate_parseable_manifest_entries(
            entry,
            &self.manifest_entries,
            self.bootstrap_plan_index.plan_titles_by_id(),
            &self.primary_major_titles,
            course_parser,
            |base_entry, label, url| {
                self.candidate_conflicts_with_pathway_scope(base_entry, label, url, course_parser)
            },
        )
    }

    pub fn structured_uw_course_codes(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        get_structured_uw_course_codes(entry, &self.degree_map_blocks, course_parser)
    }

    pub fn filter_undergraduate_course_codes(
        &self,
        entry: &Value,
        source_lines: &[String],
        course_codes: &[String],
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        filter_graduate_cross_listed_companions_for_undergraduate_source(
            entry,
            source_lines,
            course_codes,
            course_parser,
        )
    }

    pub fn materializable_pathway_id(&self, entry: &Value) -> Option<String> {
        self.bootstrap_plan_index
            .materializable_parser_pathway_id(entry)
    }

    pub fn snapshot_is_active_for_owner(
        &self,
        entry: &Value,
        snapshot: &RequirementSourceSnapshot,
        course_parser: &CourseCodeParser,
    ) -> bool {
        let Some(snapshot_url) = snapshot
            .source_url
            .as_deref()
            .and_then(source_url_without_hash)
        else {
            return true;
        };
        if has_conflicting_supplemental_degree_route(
            entry,
            snapshot.title.as_deref().unwrap_or_default(),
            &snapshot_url,
        ) {
            return false;
        }

        let mut active_urls = BTreeSet::new();
        if let Some(url) = entry
            .get("url")
            .and_then(Value::as_str)
            .and_then(source_url_without_hash)
        {
            active_urls.insert(url);
        }
        for candidate in &self.manifest_entries {
            if candidate.get("ownerId") != entry.get("ownerId")
                || candidate.get("campusId") != entry.get("campusId")
                || !is_parseable_manifest_parser_type(candidate.get("parserType"))
                || !manifest_entry_matches_specialized_plan_title(
                    candidate,
                    self.bootstrap_plan_index.plan_titles_by_id(),
                    &self.primary_major_titles,
                    &self.manifest_entries,
                    course_parser,
                )
            {
                continue;
            }
            let label = candidate
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = candidate
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if has_conflicting_supplemental_degree_route(entry, label, url)
                || self.candidate_conflicts_with_pathway_scope(entry, label, url, course_parser)
            {
                continue;
            }
            if let Some(url) = source_url_without_hash(url) {
                active_urls.insert(url);
            }
        }
        active_urls.contains(&snapshot_url)
    }

    pub fn cached_snapshot_recovery_candidates(
        &self,
        entry: &Value,
        snapshot_store: &RequirementSnapshotStore,
        current_snapshot_path: Option<&Path>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let owner_id = entry
            .get("ownerId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut scored_candidates = Vec::new();
        for snapshot in snapshot_store.read_owner_snapshots(owner_id) {
            if current_snapshot_path.is_some_and(|path| path == snapshot.snapshot_path.as_path())
                || !self.snapshot_is_active_for_owner(entry, &snapshot, course_parser)
            {
                continue;
            }
            let score = self.score_owner_snapshot(entry, &snapshot, course_parser);
            if score < 40 {
                continue;
            }
            let candidate_url = snapshot
                .source_url
                .as_deref()
                .filter(|url| !url.is_empty())
                .or_else(|| entry.get("url").and_then(Value::as_str))
                .unwrap_or_default();
            let candidate_label = snapshot
                .title
                .as_deref()
                .filter(|title| !title.is_empty())
                .or_else(|| entry.get("label").and_then(Value::as_str))
                .filter(|label| !label.is_empty())
                .unwrap_or("Cached owner source snapshot");
            if self.candidate_conflicts_with_pathway_scope(
                entry,
                candidate_label,
                candidate_url,
                course_parser,
            ) || has_conflicting_supplemental_degree_route(entry, candidate_label, candidate_url)
                || parser_recovery_candidate_conflicts_with_program_sibling(
                    entry,
                    candidate_label,
                    candidate_url,
                )
                || self.snapshot_conflicts_with_base_owner_scope(entry, &snapshot.snapshot_lines)
            {
                continue;
            }

            let candidate_entry = self.build_recovery_candidate_entry(
                entry,
                "cached-owner-snapshot-recovery",
                candidate_url,
                candidate_label,
            );
            let source_role = classify_requirement_source_role(&candidate_entry);
            let source_role_status = get_requirement_source_role_status(Some(source_role));
            let candidate_title = snapshot
                .title
                .clone()
                .unwrap_or_else(|| candidate_label.to_string());
            let mut section_candidates = self.generic_parser_recovery_section_candidates(
                &candidate_entry,
                &snapshot.snapshot_lines,
                Some(&candidate_title),
                &snapshot.headings,
                course_parser,
            );
            for section_candidate in &mut section_candidates {
                let section_score = section_candidate
                    .get("score")
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32;
                let mut section_candidate = section_candidate.clone();
                if let Some(fields) = section_candidate.as_object_mut() {
                    fields.insert(
                        "score".to_string(),
                        serde_json::json!(score + section_score),
                    );
                    let reason = fields
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    fields.insert(
                        "reason".to_string(),
                        serde_json::json!(format!("{reason}; cached same-owner source snapshot")),
                    );
                }
                scored_candidates.push((score + section_score, section_candidate));
            }
            let course_code_count = snapshot
                .snapshot_lines
                .iter()
                .flat_map(|line| course_parser.extract_from_line(line))
                .collect::<BTreeSet<_>>()
                .len();
            if course_code_count > 0 && !is_legacy_student_catalog_source(&candidate_entry) {
                let candidate = serde_json::json!({
                    "strategy": "cached-owner-snapshot-recovery",
                    "url": candidate_url,
                    "label": format!("Cached source: {}", super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(candidate_label), 120)),
                    "parserType": candidate_entry.get("parserType").and_then(Value::as_str).unwrap_or_default(),
                    "sourceRole": source_role,
                    "sourceRoleStatus": source_role_status.as_str(),
                    "score": score,
                    "reason": "cached same-owner official source snapshot",
                    "sectionLineStart": 0,
                    "sectionLineEnd": snapshot.snapshot_lines.len(),
                    "sectionLines": snapshot.snapshot_lines,
                    "title": candidate_title,
                    "headings": snapshot.headings,
                });
                scored_candidates.push((score, candidate));
            }
        }
        scored_candidates.sort_by(|left, right| right.0.cmp(&left.0));
        let mut seen = HashSet::new();
        scored_candidates
            .into_iter()
            .filter_map(|(_, candidate)| {
                let key = format!(
                    "{}:{}:{}:{}",
                    candidate
                        .get("strategy")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    candidate
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    candidate
                        .get("sectionLineStart")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                    candidate
                        .get("sectionLineEnd")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                );
                seen.insert(key).then_some(candidate)
            })
            .take(6)
            .collect()
    }

    pub fn score_owner_snapshot(
        &self,
        entry: &Value,
        snapshot: &RequirementSourceSnapshot,
        course_parser: &CourseCodeParser,
    ) -> i32 {
        score_owner_snapshot_for_parser_recovery(entry, snapshot, course_parser)
    }

    pub fn snapshot_conflicts_with_base_owner_scope(
        &self,
        entry: &Value,
        snapshot_lines: &[String],
    ) -> bool {
        parser_recovery_snapshot_conflicts_with_base_owner_scope(
            entry,
            snapshot_lines,
            &self.manifest_entries,
        )
    }

    pub fn scope_parser_recovery_section_lines(
        &self,
        entry: &Value,
        lines: &[String],
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        let curriculum_scoped = scope_curriculum_era_lines(entry, lines, course_parser);
        remove_following_different_owner_acronym_departmental_sections(
            entry,
            &curriculum_scoped,
            &self.primary_major_titles,
        )
    }

    pub fn generic_parser_recovery_section_candidates(
        &self,
        entry: &Value,
        raw_lines: &[String],
        title: Option<&str>,
        headings: &[String],
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let source_url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        if is_graduate_supplemental_source(
            entry
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            source_url,
        ) {
            return Vec::new();
        }
        let lines = self.scope_parser_recovery_section_lines(entry, raw_lines, course_parser);
        if lines.is_empty() {
            return Vec::new();
        }
        if should_use_full_generic_concentration_degree_scope(
            entry,
            headings,
            &lines,
            course_parser,
        ) {
            let first_line = lines.first().map(String::as_str).unwrap_or_default();
            return vec![serde_json::json!({
                "strategy": "section-scoping-recovery",
                "url": source_url,
                "label": format!(
                    "Scoped current requirements: {}",
                    super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(first_line), 120)
                ),
                "parserType": entry.get("parserType").and_then(Value::as_str).unwrap_or_default(),
                "sourceRole": classify_requirement_source_role(entry),
                "sourceRoleStatus": get_requirement_source_role_status(Some(classify_requirement_source_role(entry))).as_str(),
                "score": 130,
                "reason": "full concentration degree requirements page keeps sibling concentration course sections together",
                "sectionLineStart": 0,
                "sectionLineEnd": lines.len(),
                "sectionLines": lines,
                "title": title,
                "headings": headings,
            })];
        }
        if let Some((start_index, end_index)) = find_pathway_html_curriculum_section_candidate(
            entry,
            &lines,
            &self.primary_major_titles,
            course_parser,
        ) {
            let section_lines = pathway_html_section_scope_lines(
                entry,
                &lines,
                start_index,
                end_index,
                course_parser,
            );
            if section_lines
                .iter()
                .any(|line| !course_parser.extract_from_line(line).is_empty())
            {
                let first_line = section_lines
                    .first()
                    .map(String::as_str)
                    .unwrap_or_default();
                return vec![serde_json::json!({
                    "strategy": "section-scoping-recovery",
                    "url": source_url,
                    "label": format!(
                        "Scoped pathway section: {}",
                        super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(first_line), 120)
                    ),
                    "parserType": entry.get("parserType").and_then(Value::as_str).unwrap_or_default(),
                    "sourceRole": classify_requirement_source_role(entry),
                    "sourceRoleStatus": get_requirement_source_role_status(Some(classify_requirement_source_role(entry))).as_str(),
                    "score": 120,
                    "reason": "selected pathway heading matched within a broader official source",
                    "sectionLineStart": start_index,
                    "sectionLineEnd": end_index,
                    "sectionLines": section_lines,
                    "title": title,
                    "headings": headings,
                })];
            }
        }
        if let Some((start_index, end_index, nested_lines)) = find_nested_pathway_html_section_scope(
            entry,
            &lines,
            &self.primary_major_titles,
            course_parser,
        ) {
            let section_lines =
                preserve_prior_admit_pathway_context(entry, &lines, &nested_lines, course_parser);
            let first_line = section_lines
                .first()
                .map(String::as_str)
                .unwrap_or_default();
            return vec![serde_json::json!({
                "strategy": "section-scoping-recovery",
                "url": source_url,
                "label": format!(
                    "Scoped pathway section: {}",
                    super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(first_line), 120)
                ),
                "parserType": entry.get("parserType").and_then(Value::as_str).unwrap_or_default(),
                "sourceRole": classify_requirement_source_role(entry),
                "sourceRoleStatus": get_requirement_source_role_status(Some(classify_requirement_source_role(entry))).as_str(),
                "score": 120,
                "reason": "selected pathway heading matched within a broader official source",
                "sectionLineStart": start_index,
                "sectionLineEnd": end_index,
                "sectionLines": section_lines,
                "title": title,
                "headings": headings,
            })];
        }
        if let Some((start_index, end_index, table_lines)) = find_parallel_pathway_html_table_scope(
            entry,
            &lines,
            &self.primary_major_titles,
            course_parser,
        ) {
            let section_lines =
                preserve_prior_admit_pathway_context(entry, &lines, &table_lines, course_parser);
            let first_line = section_lines
                .first()
                .map(String::as_str)
                .unwrap_or_default();
            return vec![serde_json::json!({
                "strategy": "section-scoping-recovery",
                "url": source_url,
                "label": format!(
                    "Scoped pathway section: {}",
                    super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(first_line), 120)
                ),
                "parserType": entry.get("parserType").and_then(Value::as_str).unwrap_or_default(),
                "sourceRole": classify_requirement_source_role(entry),
                "sourceRoleStatus": get_requirement_source_role_status(Some(classify_requirement_source_role(entry))).as_str(),
                "score": 120,
                "reason": "selected pathway heading matched within a broader official source",
                "sectionLineStart": start_index,
                "sectionLineEnd": end_index,
                "sectionLines": section_lines,
                "title": title,
                "headings": headings,
            })];
        }
        if let Some((start_index, end_index)) = find_pathway_html_selected_section_candidate(
            entry,
            &lines,
            &self.primary_major_titles,
            course_parser,
        ) {
            let section_lines = pathway_html_section_scope_lines(
                entry,
                &lines,
                start_index,
                end_index,
                course_parser,
            );
            let first_line = section_lines
                .first()
                .map(String::as_str)
                .unwrap_or_default();
            return vec![serde_json::json!({
                "strategy": "section-scoping-recovery",
                "url": source_url,
                "label": format!(
                    "Scoped pathway section: {}",
                    super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(first_line), 120)
                ),
                "parserType": entry.get("parserType").and_then(Value::as_str).unwrap_or_default(),
                "sourceRole": classify_requirement_source_role(entry),
                "sourceRoleStatus": get_requirement_source_role_status(Some(classify_requirement_source_role(entry))).as_str(),
                "score": 120,
                "reason": "selected pathway heading matched within a broader official source",
                "sectionLineStart": start_index,
                "sectionLineEnd": end_index,
                "sectionLines": section_lines,
                "title": title,
                "headings": headings,
            })];
        }
        let base_scope_lines = remove_parallel_pathway_html_table_content_from_base_scope(
            entry,
            &lines,
            course_parser,
        );
        if base_scope_lines.as_slice() != lines.as_slice() {
            let first_line = base_scope_lines
                .first()
                .map(String::as_str)
                .unwrap_or_default();
            return vec![serde_json::json!({
                "strategy": "section-scoping-recovery",
                "url": source_url,
                "label": format!(
                    "Scoped current requirements: {}",
                    super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(first_line), 120)
                ),
                "parserType": entry.get("parserType").and_then(Value::as_str).unwrap_or_default(),
                "sourceRole": classify_requirement_source_role(entry),
                "sourceRoleStatus": get_requirement_source_role_status(Some(classify_requirement_source_role(entry))).as_str(),
                "score": 100,
                "reason": "base owner excluded sibling formal-option table content from a broader official source",
                "sectionLineStart": 0,
                "sectionLineEnd": base_scope_lines.len(),
                "sectionLines": base_scope_lines,
                "title": title,
                "headings": headings,
            })];
        }
        let has_pathway_id = entry
            .get("pathwayId")
            .and_then(Value::as_str)
            .is_some_and(|pathway_id| !pathway_id.is_empty());
        let parser_type = entry
            .get("parserType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let source_role = classify_requirement_source_role(entry);
        let source_role_status = get_requirement_source_role_status(Some(source_role));
        let mut scored_candidates = Vec::<(i32, usize, usize, Value)>::new();

        for (index, line) in lines.iter().enumerate() {
            let line_course_codes = course_parser.extract_from_line(line);
            if has_pathway_id
                && !line_course_codes.is_empty()
                && !is_html_section_boundary_line(line)
                && !is_parser_recovery_pathway_section_boundary_line(line)
            {
                continue;
            }
            let score = parser_recovery_section_line_score(entry, line);
            if score < 10
                || self.candidate_conflicts_with_pathway_scope(
                    entry,
                    line,
                    source_url,
                    course_parser,
                )
            {
                continue;
            }

            let mut end_index = lines.len().min(index + 220);
            for next_index in index + 1..end_index {
                let next_line = normalize_whitespace(&lines[next_index]);
                if is_back_to_top_heading_line(&next_line)
                    || next_line
                        .to_ascii_lowercase()
                        .starts_with("program of study:")
                {
                    end_index = next_index;
                    break;
                }
                if is_legacy_student_catalog_source(entry)
                    && is_legacy_catalog_credential_heading_line(&next_line)
                    && line_looks_like_legacy_catalog_credential_section_start(&lines, next_index)
                {
                    end_index = next_index;
                    break;
                }
                if (next_index > index + 12
                    || is_parser_recovery_pathway_section_boundary_line(&lines[next_index]))
                    && is_parser_recovery_section_boundary_line(&lines[next_index])
                {
                    end_index = next_index;
                    break;
                }
            }
            let start_index =
                find_preceding_inline_requirement_start_index(entry, &lines, index, course_parser)
                    .unwrap_or(index);
            let section_lines = lines[start_index..end_index].to_vec();
            let sampled_text = section_lines
                .iter()
                .take(60)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            let has_section_course_codes = section_lines
                .iter()
                .any(|section_line| !course_parser.extract_from_line(section_line).is_empty());
            if !has_parser_recovery_requirement_cue(&sampled_text) && !has_section_course_codes {
                continue;
            }
            let label = format!(
                "Scoped section: {}",
                super::requirement_shape::js_slice_utf16_units(&normalize_whitespace(line), 120)
            );
            let candidate = serde_json::json!({
                "strategy": "section-scoping-recovery",
                "url": source_url,
                "label": label,
                "parserType": parser_type,
                "sourceRole": source_role,
                "sourceRoleStatus": source_role_status.as_str(),
                "score": score,
                "reason": "owner/requirement heading matched within a broader official source",
                "sectionLineStart": start_index,
                "sectionLineEnd": end_index,
                "sectionLines": section_lines,
                "title": title,
                "headings": headings,
            });
            scored_candidates.push((score, start_index, end_index, candidate));
        }

        scored_candidates.sort_by(|left, right| right.0.cmp(&left.0));
        let mut seen = HashSet::new();
        scored_candidates
            .into_iter()
            .filter_map(|(_, start_index, end_index, candidate)| {
                seen.insert((start_index, end_index)).then_some(candidate)
            })
            .take(4)
            .collect()
    }

    pub fn candidate_conflicts_with_base_owner_scope(
        &self,
        entry: &Value,
        label: &str,
        url: &str,
    ) -> bool {
        parser_recovery_candidate_conflicts_with_base_owner_scope(
            entry,
            label,
            url,
            &self.primary_major_titles,
        )
    }

    pub fn build_recovery_candidate_entry(
        &self,
        entry: &Value,
        strategy: &str,
        candidate_url: &str,
        candidate_label: &str,
    ) -> Value {
        let candidate = serde_json::json!({
            "strategy": strategy,
            "url": candidate_url,
            "label": candidate_label,
            "parserType": get_parser_recovery_candidate_parser_type(
                entry,
                candidate_url,
                candidate_label,
            ),
        });
        build_parser_recovery_candidate_entry(entry, &candidate)
    }

    pub fn extract_parser_recovery_link_candidates(
        &self,
        entry: &Value,
        links: &[(String, String)],
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let Some(entry_url) = entry.get("url").and_then(Value::as_str) else {
            return Vec::new();
        };
        let Some(base_url) = source_url_without_hash(entry_url) else {
            return Vec::new();
        };
        let Ok(original_base) = Url::parse(entry_url) else {
            return Vec::new();
        };
        let has_pathway_id = entry
            .get("pathwayId")
            .and_then(Value::as_str)
            .is_some_and(|pathway_id| !pathway_id.is_empty());
        let mut candidates = Vec::<(String, i32, Value)>::new();
        let mut candidate_indexes = HashMap::<String, usize>::new();

        for (href, label) in links.iter().cloned() {
            if href.is_empty() || label.is_empty() || is_ignored_parser_recovery_href(&href) {
                continue;
            }
            let Ok(resolved) = original_base.join(&href) else {
                continue;
            };
            let Some(resolved_url) = source_url_without_hash(resolved.as_str()) else {
                continue;
            };
            if resolved_url == base_url
                || !is_official_linked_asset_url(entry_url, &resolved_url)
                || is_noisy_parser_recovery_link(&label, &resolved_url)
                || is_graduate_supplemental_source(&label, &resolved_url)
            {
                continue;
            }

            let (score, signals) =
                score_parser_recovery_link_candidate(entry, entry_url, &resolved_url, &label);
            let owner_title_token_count = parser_recovery_title_token_count(entry);
            let required_sibling_title_overlap = 2usize.min(owner_title_token_count);
            if is_different_program_sibling_page(entry_url, &resolved_url)
                && !signals.exact_title_match
                && signals.title_token_overlap_count < required_sibling_title_overlap
            {
                continue;
            }
            if self.candidate_conflicts_with_pathway_scope(
                entry,
                &label,
                &resolved_url,
                course_parser,
            ) {
                continue;
            }
            if signals.document_signal
                && has_conflicting_supplemental_degree_route(entry, &label, &resolved_url)
            {
                continue;
            }
            if has_pathway_id
                && signals.document_signal
                && !self.candidate_contains_selected_pathway_phrase(entry, &label, &resolved_url)
            {
                continue;
            }
            if is_different_program_sibling_page(entry_url, &resolved_url)
                && !signals.same_program_requirement_link
                && !signals.same_program_specialization_link
                && !signals.exact_title_match
            {
                continue;
            }
            if !has_pathway_id
                && signals.pathway_signal
                && !signals.exact_title_match
                && !has_primary_requirement_link_cue(&label, &resolved_url)
            {
                continue;
            }
            if !has_recovery_link_signal(&signals, owner_title_token_count) || score < 12 {
                continue;
            }

            let parser_type =
                get_parser_recovery_candidate_parser_type(entry, &resolved_url, &label);
            let mut candidate_entry = entry.clone();
            if let Some(object) = candidate_entry.as_object_mut() {
                object.remove("role");
                object.insert("url".to_string(), Value::String(resolved_url.clone()));
                object.insert("label".to_string(), Value::String(label.clone()));
                object.insert("parserType".to_string(), Value::String(parser_type.clone()));
            }
            let source_role = classify_requirement_source_role(&candidate_entry);
            let source_role_status = get_requirement_source_role_status(Some(source_role));
            let strategy = if source_role_status.as_str() == "support" {
                "support-source-recovery"
            } else if signals.document_signal {
                "linked-official-document-recovery"
            } else {
                "official-sibling-child-page-recovery"
            };
            let reason = match strategy {
                "linked-official-document-recovery" => {
                    "official linked requirement document candidate"
                }
                "support-source-recovery" => "official support source candidate",
                _ => "official child or sibling requirement page candidate",
            };
            let candidate = serde_json::json!({
                "strategy": strategy,
                "url": resolved_url,
                "label": label,
                "parserType": parser_type,
                "sourceRole": source_role,
                "sourceRoleStatus": source_role_status.as_str(),
                "score": score,
                "signals": signals,
                "reason": reason,
            });
            if let Some(index) = candidate_indexes.get(&resolved_url).copied() {
                if score > candidates[index].1 {
                    candidates[index] = (resolved_url, score, candidate);
                }
            } else {
                candidate_indexes.insert(resolved_url.clone(), candidates.len());
                candidates.push((resolved_url, score, candidate));
            }
        }

        candidates.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| {
                    let left_label = left
                        .2
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let right_label = right
                        .2
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    left_label.to_lowercase().cmp(&right_label.to_lowercase())
                })
                .then_with(|| {
                    right
                        .2
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .cmp(
                            left.2
                                .get("label")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        )
                })
        });
        candidates
            .into_iter()
            .take(6)
            .map(|(_, _, candidate)| candidate)
            .collect()
    }

    fn active_manifest_document_source_urls(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> BTreeSet<String> {
        let mut active_urls = BTreeSet::new();
        if let Some(url) = entry
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| is_linked_document_source_url(*url))
            .and_then(source_url_without_hash)
        {
            active_urls.insert(url);
        }
        for candidate in &self.manifest_entries {
            if candidate.get("ownerId") != entry.get("ownerId")
                || candidate.get("campusId") != entry.get("campusId")
                || !is_parseable_manifest_parser_type(candidate.get("parserType"))
                || !manifest_entry_matches_specialized_plan_title(
                    candidate,
                    self.bootstrap_plan_index.plan_titles_by_id(),
                    &self.primary_major_titles,
                    &self.manifest_entries,
                    course_parser,
                )
            {
                continue;
            }
            let label = candidate
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = candidate
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if has_conflicting_supplemental_degree_route(entry, label, url)
                || self.candidate_conflicts_with_pathway_scope(entry, label, url, course_parser)
                || !is_linked_document_source_url(url)
            {
                continue;
            }
            if let Some(url) = source_url_without_hash(url) {
                active_urls.insert(url);
            }
        }
        active_urls
    }

    pub fn extract_supplemental_document_link_candidates(
        &self,
        entry: &Value,
        links: &[(String, String)],
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let Some(entry_url) = entry.get("url").and_then(Value::as_str) else {
            return Vec::new();
        };
        let Some(base_url) = source_url_without_hash(entry_url) else {
            return Vec::new();
        };
        let Ok(original_base) = Url::parse(entry_url) else {
            return Vec::new();
        };
        let active_document_urls = self.active_manifest_document_source_urls(entry, course_parser);
        let restrict_to_active_documents = !active_document_urls.is_empty();
        let owner_acronym_tokens =
            parser_recovery_owner_program_acronym_tokens(entry, &self.primary_major_titles);
        let mut candidates = Vec::<(String, i32, usize, Value)>::new();
        let mut candidate_indexes = HashMap::<String, usize>::new();

        for (link_index, (href, label)) in links.iter().cloned().enumerate() {
            let link_index = link_index + 1;
            if href.is_empty()
                || label.is_empty()
                || !is_linked_document_source_url(&href)
                || is_ignored_parser_recovery_href(&href)
            {
                continue;
            }
            let Ok(resolved) = original_base.join(&href) else {
                continue;
            };
            let Some(resolved_url) = source_url_without_hash(resolved.as_str()) else {
                continue;
            };
            if !is_official_linked_asset_url(entry_url, &resolved_url)
                || (restrict_to_active_documents && !active_document_urls.contains(&resolved_url))
                || has_conflicting_supplemental_degree_route(entry, &label, &resolved_url)
            {
                continue;
            }
            let (score, signals) = score_supplemental_document_link_candidate(
                entry,
                &base_url,
                &resolved_url,
                &label,
                &href,
                &owner_acronym_tokens,
            );
            if !signals.exact_title_match
                && !signals.title_acronym_match
                && signals.title_token_overlap_count < 2
                && !signals.same_program_requirement_link
            {
                continue;
            }
            let parser_type = get_linked_document_parser_type(&resolved_url, &label);
            let candidate = serde_json::json!({
                "url": resolved_url,
                "label": label,
                "parserType": parser_type,
                "score": score,
                "exactTitleMatch": signals.exact_title_match,
                "titleAcronymMatch": signals.title_acronym_match,
                "titleAcronymMatches": signals.title_acronym_matches,
                "sameProgramRequirementLink": signals.same_program_requirement_link,
                "historical": signals.historical,
                "linkIndex": link_index,
            });
            if let Some(index) = candidate_indexes.get(&resolved_url).copied() {
                if score > candidates[index].1 {
                    candidates[index] = (resolved_url, score, link_index, candidate);
                }
            } else {
                candidate_indexes.insert(resolved_url.clone(), candidates.len());
                candidates.push((resolved_url, score, link_index, candidate));
            }
        }

        let mut candidates = candidates
            .into_iter()
            .filter(|(_, score, _, _)| *score >= 24)
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.2.cmp(&right.2)));
        candidates
            .into_iter()
            .take(2)
            .map(|(_, _, _, candidate)| candidate)
            .collect()
    }

    pub fn candidate_contains_selected_pathway_phrase(
        &self,
        entry: &Value,
        label: &str,
        url: &str,
    ) -> bool {
        parser_recovery_candidate_contains_selected_pathway_phrase(
            entry,
            label,
            url,
            &self.primary_major_titles,
        )
    }

    pub fn candidate_conflicts_with_pathway_scope(
        &self,
        entry: &Value,
        label: &str,
        url: &str,
        course_parser: &CourseCodeParser,
    ) -> bool {
        parser_recovery_candidate_conflicts_with_owner_identity(
            entry,
            label,
            url,
            self.bootstrap_plan_index.plan_titles_by_id(),
            &self.primary_major_titles,
            &self.manifest_entries,
            course_parser,
        ) || parser_recovery_candidate_conflicts_with_pathway_scope(
            entry,
            label,
            url,
            &self.primary_major_titles,
        )
    }
}
