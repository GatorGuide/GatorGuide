use super::requirement_adapters::{
    RequirementSourceAdapter, RequirementSourceParserKind, select_requirement_source_adapter,
};
use super::requirement_course_codes::CourseCodeParser;
use super::requirement_document_facts::{
    RequirementSourceTextFacts, collect_docx_source_facts_for_entry_with_primary_titles,
    collect_html_source_facts_for_entry, collect_html_source_facts_for_entry_with_context,
    collect_html_source_facts_for_entry_with_primary_titles,
    collect_pdf_source_facts_with_primary_titles,
    collect_snapshot_source_facts_with_primary_titles,
};
use super::requirement_docx::{
    DocxSourceParser, get_linked_document_parser_type, is_docx_source_url, is_pdf_source_url,
};
use super::requirement_html_source::HtmlSourceParser;
use super::requirement_manifest_context::RequirementSourceManifestContext;
use super::requirement_pdf_extraction::PdfSourceParser;
use super::requirement_pdf_layout::PdfPageBlock;
use super::requirement_recovery_links::extract_supplemental_html_link_candidates;
use super::requirement_role_classifier::classify_requirement_source_role;
use super::requirement_section_roles::schedulable_inline_requirement_line_indices;
use super::requirement_snapshots::RequirementSnapshotStore;
use super::requirement_source_fetching::SourceDownloader;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

fn unique_locale_sorted(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut values = values
        .into_iter()
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| right.cmp(left))
    });
    values
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourcePipelineResult {
    pub adapter: RequirementSourceAdapter,
    pub source_role: &'static str,
    pub facts: RequirementSourceTextFacts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceResolution {
    pub parsed: RequirementSourcePipelineResult,
    pub resolution_strategy: &'static str,
    pub ok: bool,
    pub snapshot_path: Option<PathBuf>,
    pub snapshot_has_heading_metadata: bool,
    pub used_snapshot_fallback: bool,
    pub snapshot_fallback_reason: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplementalRequirementSourceResolution {
    pub entry: Value,
    pub resolution: RequirementSourceResolution,
    pub kind: String,
}

#[derive(Clone)]
struct ParsedLinkedHtmlSource {
    candidate: Value,
    entry: Value,
    resolution: RequirementSourceResolution,
}

#[derive(Clone)]
struct ParsedLinkedDocumentSource {
    candidate: Value,
    entry: Value,
    resolution: RequirementSourceResolution,
}

impl RequirementSourceResolution {
    pub fn parsed_owner_alignment_score(&self, entry: &Value) -> usize {
        super::requirement_source_scoring::get_parsed_owner_alignment_score(
            entry,
            &self.parsed.facts,
        )
    }

    pub fn parsed_result_score(&self, entry: &Value) -> i64 {
        super::requirement_source_scoring::get_parsed_result_score(entry, &self.parsed.facts)
    }

    pub fn has_meaningful_parsed_content(&self) -> bool {
        super::requirement_source_scoring::has_meaningful_parsed_content(&self.parsed.facts)
    }

    pub fn should_evaluate_alternate_sources(&self, entry: &Value) -> bool {
        super::requirement_source_scoring::is_weak_parsed_result(
            entry,
            &self.parsed.facts,
            self.used_snapshot_fallback,
        )
    }

    pub fn should_allow_alternate_to_replace(
        &self,
        best_entry: &Value,
        alternate_entry: &Value,
        alternate: &RequirementSourceResolution,
        primary_major_titles: &std::collections::HashMap<String, String>,
    ) -> bool {
        super::requirement_source_scoring::should_allow_alternate_to_replace_best_source(
            best_entry,
            &self.parsed.facts,
            self.used_snapshot_fallback,
            alternate_entry,
            &alternate.parsed.facts,
            primary_major_titles,
        )
    }

    fn should_allow_alternate_to_replace_with_context(
        &self,
        best_entry: &Value,
        alternate_entry: &Value,
        alternate: &RequirementSourceResolution,
        primary_major_titles: &std::collections::HashMap<String, String>,
        context: &RequirementSourceManifestContext,
        course_parser: &CourseCodeParser,
    ) -> bool {
        super::requirement_source_scoring::should_allow_alternate_to_replace_best_source_with_context(
            best_entry,
            &self.parsed.facts,
            self.used_snapshot_fallback,
            alternate_entry,
            &alternate.parsed.facts,
            primary_major_titles,
            context.bootstrap_plan_index().plan_titles_by_id(),
            context.manifest_entries(),
            course_parser,
        )
    }

    pub fn best_alternate_source_index(
        &self,
        primary_entry: &Value,
        alternates: &[SupplementalRequirementSourceResolution],
        primary_major_titles: &std::collections::HashMap<String, String>,
    ) -> Option<usize> {
        let mut best_entry = primary_entry.clone();
        let mut best_resolution = self.clone();
        let mut best_score = self.parsed_result_score(primary_entry);
        let mut best_index = None;
        for (index, alternate) in alternates.iter().enumerate() {
            if best_resolution.should_allow_alternate_to_replace(
                &best_entry,
                &alternate.entry,
                &alternate.resolution,
                primary_major_titles,
            ) {
                let score = alternate.resolution.parsed_result_score(&alternate.entry);
                if score > best_score {
                    best_entry = alternate.entry.clone();
                    best_resolution = alternate.resolution.clone();
                    best_score = score;
                    best_index = Some(index);
                }
            }
        }
        best_index
    }

    fn best_alternate_source_index_with_context(
        &self,
        primary_entry: &Value,
        alternates: &[SupplementalRequirementSourceResolution],
        primary_major_titles: &std::collections::HashMap<String, String>,
        context: &RequirementSourceManifestContext,
        course_parser: &CourseCodeParser,
    ) -> Option<usize> {
        let mut best_entry = primary_entry.clone();
        let mut best_resolution = self.clone();
        let mut best_score = self.parsed_result_score(primary_entry);
        let mut best_index = None;
        for (index, alternate) in alternates.iter().enumerate() {
            if best_resolution.should_allow_alternate_to_replace_with_context(
                &best_entry,
                &alternate.entry,
                &alternate.resolution,
                primary_major_titles,
                context,
                course_parser,
            ) {
                let score = alternate.resolution.parsed_result_score(&alternate.entry);
                if score > best_score {
                    best_entry = alternate.entry.clone();
                    best_resolution = alternate.resolution.clone();
                    best_score = score;
                    best_index = Some(index);
                }
            }
        }
        best_index
    }

    pub fn select_and_merge_alternate_sources(
        &self,
        primary_entry: &Value,
        alternates: &[SupplementalRequirementSourceResolution],
        primary_major_titles: &std::collections::HashMap<String, String>,
    ) -> (Value, Self) {
        let selected_index =
            self.best_alternate_source_index(primary_entry, alternates, primary_major_titles);
        let (best_entry, best_resolution) = selected_index
            .map(|index| {
                let selected = &alternates[index];
                (selected.entry.clone(), selected.resolution.clone())
            })
            .unwrap_or_else(|| (primary_entry.clone(), self.clone()));
        let mut supplemental_sources = Vec::new();
        if selected_index.is_some() {
            supplemental_sources.push(SupplementalRequirementSourceResolution {
                entry: primary_entry.clone(),
                resolution: self.clone(),
                kind: "alternate".to_string(),
            });
        }
        supplemental_sources.extend(
            alternates
                .iter()
                .enumerate()
                .filter(|(index, alternate)| {
                    Some(*index) != selected_index
                        && best_resolution.should_merge_supplemental_source(
                            &best_entry,
                            alternate,
                            primary_major_titles,
                        )
                })
                .map(|(_, alternate)| alternate.clone()),
        );
        let parser_type = best_entry
            .get("parserType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        (
            best_entry,
            best_resolution.merge_supplemental_sources(&supplemental_sources, &parser_type),
        )
    }

    fn select_and_merge_alternate_sources_with_context(
        &self,
        primary_entry: &Value,
        alternates: &[SupplementalRequirementSourceResolution],
        primary_major_titles: &std::collections::HashMap<String, String>,
        context: &RequirementSourceManifestContext,
        course_parser: &CourseCodeParser,
    ) -> (Value, Self) {
        let selected_index = self.best_alternate_source_index_with_context(
            primary_entry,
            alternates,
            primary_major_titles,
            context,
            course_parser,
        );
        let (best_entry, best_resolution) = selected_index
            .map(|index| {
                let selected = &alternates[index];
                (selected.entry.clone(), selected.resolution.clone())
            })
            .unwrap_or_else(|| (primary_entry.clone(), self.clone()));
        let mut supplemental_sources = Vec::new();
        if selected_index.is_some() {
            supplemental_sources.push(SupplementalRequirementSourceResolution {
                entry: primary_entry.clone(),
                resolution: self.clone(),
                kind: "alternate".to_string(),
            });
        }
        supplemental_sources.extend(
            alternates
                .iter()
                .enumerate()
                .filter(|(index, alternate)| {
                    Some(*index) != selected_index
                        && best_resolution.should_merge_supplemental_source_with_context(
                            &best_entry,
                            alternate,
                            primary_major_titles,
                            context,
                            course_parser,
                        )
                })
                .map(|(_, alternate)| alternate.clone()),
        );
        let parser_type = best_entry
            .get("parserType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        (
            best_entry,
            best_resolution.merge_supplemental_sources(&supplemental_sources, &parser_type),
        )
    }

    pub fn should_merge_supplemental_source(
        &self,
        base_entry: &Value,
        alternate: &SupplementalRequirementSourceResolution,
        primary_major_titles: &std::collections::HashMap<String, String>,
    ) -> bool {
        super::requirement_source_scoring::should_merge_supplemental_alternate_source(
            base_entry,
            &self.parsed.facts,
            self.used_snapshot_fallback,
            &alternate.entry,
            &alternate.resolution.parsed.facts,
            primary_major_titles,
        )
    }

    fn should_merge_supplemental_source_with_context(
        &self,
        base_entry: &Value,
        alternate: &SupplementalRequirementSourceResolution,
        primary_major_titles: &std::collections::HashMap<String, String>,
        context: &RequirementSourceManifestContext,
        course_parser: &CourseCodeParser,
    ) -> bool {
        super::requirement_source_scoring::should_merge_supplemental_alternate_source_with_context(
            base_entry,
            &self.parsed.facts,
            self.used_snapshot_fallback,
            &alternate.entry,
            &alternate.resolution.parsed.facts,
            primary_major_titles,
            context.bootstrap_plan_index().plan_titles_by_id(),
            context.manifest_entries(),
            course_parser,
        )
    }

    pub fn merge_eligible_supplemental_sources(
        &self,
        base_entry: &Value,
        supplemental_sources: &[SupplementalRequirementSourceResolution],
        primary_major_titles: &std::collections::HashMap<String, String>,
        parser_type: &str,
    ) -> Self {
        let eligible_sources = supplemental_sources
            .iter()
            .filter(|source| {
                self.should_merge_supplemental_source(base_entry, source, primary_major_titles)
            })
            .cloned()
            .collect::<Vec<_>>();
        self.merge_supplemental_sources(&eligible_sources, parser_type)
    }

    pub fn merge_supplemental_sources(
        &self,
        supplemental_sources: &[SupplementalRequirementSourceResolution],
        parser_type: &str,
    ) -> Self {
        if supplemental_sources.is_empty() {
            return self.clone();
        }
        let mut merged = self.clone();
        let mut course_codes = self.parsed.facts.candidate_course_codes.clone();
        let mut requirement_cue_lines = self.parsed.facts.requirement_cue_lines.clone();
        let mut headings = self.parsed.facts.headings.clone();
        let mut choose_statements = self.parsed.facts.choose_statements.clone();
        let mut pathway_labels = self.parsed.facts.pathway_labels.clone();
        let mut snapshot_lines = self.parsed.facts.snapshot_lines.clone();
        let mut used_snapshot_fallback = self.used_snapshot_fallback;
        for supplemental in supplemental_sources {
            let facts = &supplemental.resolution.parsed.facts;
            course_codes.extend(facts.candidate_course_codes.iter().cloned());
            requirement_cue_lines.extend(facts.requirement_cue_lines.iter().cloned());
            headings.extend(facts.headings.iter().cloned());
            choose_statements.extend(facts.choose_statements.iter().cloned());
            if supplemental.kind != "general" {
                pathway_labels.extend(facts.pathway_labels.iter().cloned());
            }
            let label = supplemental
                .entry
                .get("label")
                .filter(|value| !value.is_null())
                .or_else(|| {
                    supplemental
                        .entry
                        .get("url")
                        .filter(|value| !value.is_null())
                })
                .map(js_string_value)
                .unwrap_or_else(|| "Supplemental official source".to_string());
            let label = super::requirement_shape::normalize_js_whitespace(&label);
            if !label.is_empty() {
                snapshot_lines.push(format!("[Supplemental official source] {label}"));
            }
            snapshot_lines.extend(facts.snapshot_lines.iter().cloned());
            used_snapshot_fallback |= supplemental.resolution.used_snapshot_fallback;
        }
        course_codes = unique_locale_sorted(course_codes);
        let mut seen_cues = std::collections::HashSet::new();
        requirement_cue_lines.retain(|line| !line.is_empty() && seen_cues.insert(line.clone()));
        requirement_cue_lines.truncate(80);
        headings = unique_locale_sorted(headings)
            .into_iter()
            .take(24)
            .collect();
        choose_statements = unique_locale_sorted(choose_statements)
            .into_iter()
            .take(24)
            .collect();
        pathway_labels = unique_locale_sorted(pathway_labels)
            .into_iter()
            .take(24)
            .collect();
        snapshot_lines.truncate(1_200);
        merged.parsed.facts.candidate_course_codes = course_codes;
        merged.parsed.facts.requirement_cue_lines = requirement_cue_lines;
        merged.parsed.facts.headings = headings;
        merged.parsed.facts.choose_statements = choose_statements;
        merged.parsed.facts.pathway_labels = pathway_labels;
        merged.parsed.facts.snapshot_lines = snapshot_lines;
        merged.parsed.facts.parse_confidence =
            super::requirement_parse_helpers::build_parse_confidence(
                merged.parsed.facts.candidate_course_codes.len(),
                merged.parsed.facts.requirement_cue_lines.len(),
                parser_type,
            )
            .to_string();
        merged.used_snapshot_fallback = used_snapshot_fallback;
        merged
    }

    pub fn build_manifest_parse_failure(
        &self,
        base_result: &Value,
        entry: &Value,
        structured_course_codes: &[String],
    ) -> Value {
        let adapter = select_requirement_source_adapter(entry);
        let source_role = classify_requirement_source_role(entry);
        let source_scope =
            super::requirement_scope::build_requirement_source_scope(Some(source_role));
        let source_role_status =
            super::requirement_scope::get_requirement_source_role_status(Some(source_role));
        let flattened_scope = serde_json::to_value(
            super::requirement_scope::flatten_requirement_source_scope(source_scope),
        )
        .unwrap_or_else(|_| json!({}));
        let mut result = base_result.as_object().cloned().unwrap_or_default();
        if let Some(fields) = flattened_scope.as_object() {
            for (key, value) in fields {
                result.insert(key.clone(), value.clone());
            }
        }
        for (key, value) in [
            (
                "parserType",
                entry.get("parserType").cloned().unwrap_or(Value::Null),
            ),
            ("adapterId", json!(adapter.id)),
            ("adapterFamily", json!(adapter.family)),
            (
                "sourceUrl",
                entry.get("url").cloned().unwrap_or(Value::Null),
            ),
            (
                "sourceLabel",
                entry.get("label").cloned().unwrap_or(Value::Null),
            ),
            ("sourceRole", json!(source_role)),
            ("sourceRoleStatus", json!(source_role_status.as_str())),
            (
                "canCreateSchedulableRows",
                json!(source_scope.can_create_schedule_rows),
            ),
            ("sourceSectionAudit", Value::Null),
            ("sourceSectionFilterAuditRows", json!([])),
            ("sourceSectionFilterAuditLines", json!([])),
            ("parserSequenceChoiceAuditRows", json!([])),
            ("parserSequenceChoiceAuditLines", json!([])),
            ("sourceScopeAuditLines", json!([])),
            ("resolutionStrategy", json!(self.resolution_strategy)),
            ("ok", json!(false)),
            ("extractedTitle", Value::Null),
            ("extractedHeadings", json!([])),
            ("requirementCueLines", json!([])),
            ("chooseStatements", json!([])),
            ("pathwayLabels", json!([])),
            ("parsedUwCourseCodes", json!([])),
            ("approvedFilterUwCourseCodes", json!([])),
            ("electiveListUwCourseCodes", json!([])),
            ("supportOnlyUwCourseCodes", json!([])),
            ("supportLists", json!([])),
            ("sourceOnlyUwCourseCodes", json!([])),
            (
                "structuredOnlyUwCourseCodes",
                json!(structured_course_codes),
            ),
            ("parsedRequirementAtomCandidates", json!([])),
            ("parsedDegreeMapBlockCandidates", json!([])),
            ("parsedRequirementGroups", json!([])),
            ("parsedRequirementCourses", json!([])),
            ("parsedRequirementReplacements", json!([])),
            ("parseConfidence", json!("low")),
            (
                "snapshotPath",
                self.snapshot_path
                    .as_ref()
                    .map(|path| json!(path.to_string_lossy().to_string()))
                    .unwrap_or(Value::Null),
            ),
            ("usedSnapshotFallback", json!(self.used_snapshot_fallback)),
            (
                "snapshotFallbackReason",
                json!(self.snapshot_fallback_reason),
            ),
            (
                "error",
                self.error.clone().map(Value::String).unwrap_or(Value::Null),
            ),
        ] {
            result.insert(key.to_string(), value);
        }
        Value::Object(result)
    }

    pub fn build_manifest_parse_success(
        &self,
        base_result: &Value,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        structured_degree_map_blocks: &[Value],
        source_section_audit: Option<&Value>,
        snapshot_store: Option<&RequirementSnapshotStore>,
        course_parser: &CourseCodeParser,
    ) -> Value {
        let mut effective_entry_fields = entry.as_object().cloned().unwrap_or_default();
        if let Some(base_fields) = base_result.as_object() {
            for field in [
                "ownerId",
                "ownerTitle",
                "planId",
                "pathwayId",
                "campusId",
                "isPrimaryDegreeRequirementsLink",
            ] {
                if let Some(value) = base_fields.get(field) {
                    effective_entry_fields.insert(field.to_string(), value.clone());
                }
            }
        }
        let effective_entry = Value::Object(effective_entry_fields);
        let entry = &effective_entry;
        let parsed = &self.parsed;
        let effective_source_role = parsed.effective_source_role(entry);
        let source_scope =
            super::requirement_scope::build_requirement_source_scope(Some(effective_source_role));
        let groups = parsed.parsed_requirement_groups_with_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        let initial_course_codes = parsed.initial_parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            course_parser,
        );
        let parsed_course_codes = parsed.parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            &groups,
            course_parser,
        );
        let parsed_requirement_courses = if source_scope.can_create_required_rows {
            super::requirement_groups::build_source_derived_requirement_courses(&groups)
        } else {
            Vec::new()
        };
        let parsed_requirement_replacements =
            parsed.parsed_requirement_replacements_with_source_scope(entry, course_parser);
        let parsed_requirement_atom_candidates = parsed.parsed_requirement_atom_candidates(
            entry,
            &parsed_course_codes,
            &groups,
            course_parser,
        );
        let parsed_degree_map_block_candidates = parsed.parsed_degree_map_block_candidates(
            entry,
            &parsed_course_codes,
            &parsed.facts.requirement_cue_lines,
            &parsed.facts.choose_statements,
            &parsed.facts.pathway_labels,
            &parsed.facts.headings,
            structured_degree_map_blocks,
        );
        let audit_rows = parsed.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let blocked_course_codes =
            parsed.blocked_source_section_course_codes(entry, &initial_course_codes, course_parser);
        let sequence_choice_audit_rows = parsed.parser_sequence_choice_audit_rows(
            entry,
            &groups,
            &parsed_requirement_atom_candidates,
        );
        let source_scope_audit_lines = parsed.source_scope_audit_lines(
            entry,
            &parsed_course_codes,
            &blocked_course_codes,
            &groups,
            &parsed_requirement_courses,
            &parsed_requirement_atom_candidates,
        );
        let mut requirement_cue_lines = parsed.facts.requirement_cue_lines.clone();
        if parsed.source_inactive_major(entry) {
            for evidence_line in parsed.inactive_major_evidence_lines() {
                if !requirement_cue_lines.contains(&evidence_line) {
                    requirement_cue_lines.push(evidence_line);
                }
            }
        }
        let snapshot_path = self.snapshot_path.clone().or_else(|| {
            if !self.ok {
                return None;
            }
            let store = snapshot_store?;
            let owner_key = base_result
                .get("ownerId")
                .map(js_string_value)
                .unwrap_or_default();
            let source_url = entry.get("url").and_then(Value::as_str)?;
            store
                .write_snapshot(
                    &owner_key,
                    source_url,
                    parsed.facts.title.as_deref(),
                    &parsed.facts.snapshot_lines,
                    &parsed.facts.headings,
                )
                .ok()
        });
        let mut support_owner = base_result.as_object().cloned().unwrap_or_default();
        support_owner.insert(
            "sourceUrl".to_string(),
            entry.get("url").cloned().unwrap_or(Value::Null),
        );
        support_owner.insert(
            "sourceLabel".to_string(),
            entry.get("label").cloned().unwrap_or(Value::Null),
        );
        support_owner.insert("sourceRole".to_string(), json!(effective_source_role));
        let support_owner = Value::Object(support_owner);
        let approved_filter_key =
            super::requirement_source_audits::infer_approved_list_key_from_support_metadata(
                &support_owner,
            )
            .unwrap_or_default();
        let approved_filter_course_codes = if source_scope.can_create_approved_filters {
            super::requirement_source_audits::get_approved_filter_codes_for_support_source(
                &json!({
                    "filterKey": approved_filter_key,
                    "owner": support_owner,
                    "lines": parsed.facts.snapshot_lines,
                    "fallbackCourseCodes": parsed_course_codes,
                }),
                course_parser,
            )
        } else {
            Vec::new()
        };
        let elective_list_course_codes = if source_scope.can_create_elective_lists {
            parsed_course_codes.clone()
        } else {
            Vec::new()
        };
        let support_only_course_codes = if source_scope.support_only || source_scope.non_schedulable
        {
            parsed_course_codes.clone()
        } else {
            Vec::new()
        };
        let support_lists = parsed.parsed_requirement_support_lists(
            entry,
            &groups,
            &parsed_course_codes,
            &approved_filter_course_codes,
            &elective_list_course_codes,
            &support_only_course_codes,
            course_parser,
        );
        let structured_codes = structured_course_codes
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let parsed_codes = parsed_course_codes
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let source_only_course_codes = parsed_course_codes
            .iter()
            .filter(|course_code| !structured_codes.contains(*course_code))
            .cloned()
            .collect::<Vec<_>>();
        let structured_only_course_codes = structured_course_codes
            .iter()
            .filter(|course_code| !parsed_codes.contains(*course_code))
            .cloned()
            .collect::<Vec<_>>();
        let role_status = super::requirement_scope::get_requirement_source_role_status(Some(
            effective_source_role,
        ));
        let flattened_scope = serde_json::to_value(
            super::requirement_scope::flatten_requirement_source_scope(source_scope),
        )
        .unwrap_or_else(|_| json!({}));
        let mut result = base_result.as_object().cloned().unwrap_or_default();
        if let Some(fields) = flattened_scope.as_object() {
            for (key, value) in fields {
                result.insert(key.clone(), value.clone());
            }
        }
        let source_section_filter_audit_lines = audit_rows
            .iter()
            .filter_map(|row| row.get("copyOnlyDebugText").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let sequence_choice_audit_lines = sequence_choice_audit_rows
            .iter()
            .filter_map(|row| row.get("copyOnlyDebugText").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let effective_source_section_audit =
            source_section_audit.or(parsed.facts.source_section_audit.as_ref());
        for (key, value) in [
            (
                "parserType",
                entry.get("parserType").cloned().unwrap_or(Value::Null),
            ),
            ("adapterId", json!(parsed.adapter.id)),
            ("adapterFamily", json!(parsed.adapter.family)),
            (
                "sourceUrl",
                entry.get("url").cloned().unwrap_or(Value::Null),
            ),
            (
                "sourceLabel",
                entry.get("label").cloned().unwrap_or(Value::Null),
            ),
            (
                "sourceInactiveMajor",
                json!(parsed.source_inactive_major(entry)),
            ),
            ("sourceRole", json!(effective_source_role)),
            ("sourceRoleStatus", json!(role_status.as_str())),
            (
                "canCreateSchedulableRows",
                json!(source_scope.can_create_schedule_rows),
            ),
            (
                "sourceSectionAudit",
                effective_source_section_audit
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("sourceSectionFilterAuditRows", json!(audit_rows)),
            (
                "sourceSectionFilterAuditLines",
                json!(source_section_filter_audit_lines),
            ),
            (
                "parserSequenceChoiceAuditRows",
                json!(sequence_choice_audit_rows),
            ),
            (
                "parserSequenceChoiceAuditLines",
                json!(sequence_choice_audit_lines),
            ),
            ("sourceScopeAuditLines", json!(source_scope_audit_lines)),
            ("resolutionStrategy", json!(self.resolution_strategy)),
            ("ok", json!(self.ok)),
            ("extractedTitle", json!(parsed.facts.title)),
            ("extractedHeadings", json!(parsed.facts.headings)),
            ("requirementCueLines", json!(requirement_cue_lines)),
            ("chooseStatements", json!(parsed.facts.choose_statements)),
            ("pathwayLabels", json!(parsed.facts.pathway_labels)),
            ("structuredUwCourseCodes", json!(structured_course_codes)),
            ("parsedUwCourseCodes", json!(parsed_course_codes)),
            (
                "approvedFilterUwCourseCodes",
                json!(approved_filter_course_codes),
            ),
            (
                "electiveListUwCourseCodes",
                json!(elective_list_course_codes),
            ),
            ("supportOnlyUwCourseCodes", json!(support_only_course_codes)),
            ("supportLists", json!(support_lists)),
            ("sourceOnlyUwCourseCodes", json!(source_only_course_codes)),
            (
                "structuredOnlyUwCourseCodes",
                json!(structured_only_course_codes),
            ),
            (
                "parsedRequirementAtomCandidates",
                json!(parsed_requirement_atom_candidates),
            ),
            (
                "parsedDegreeMapBlockCandidates",
                json!(parsed_degree_map_block_candidates),
            ),
            ("parsedRequirementGroups", json!(groups)),
            (
                "parsedRequirementCourses",
                json!(parsed_requirement_courses),
            ),
            (
                "parsedRequirementReplacements",
                json!(parsed_requirement_replacements),
            ),
            ("parseConfidence", json!(parsed.facts.parse_confidence)),
            (
                "snapshotPath",
                snapshot_path
                    .as_ref()
                    .map(|path| json!(path.to_string_lossy().to_string()))
                    .unwrap_or(Value::Null),
            ),
            (
                "snapshotHasHeadingMetadata",
                json!(self.snapshot_has_heading_metadata),
            ),
            ("usedSnapshotFallback", json!(self.used_snapshot_fallback)),
            (
                "snapshotFallbackReason",
                json!(self.snapshot_fallback_reason),
            ),
            (
                "error",
                self.error.clone().map(Value::String).unwrap_or(Value::Null),
            ),
        ] {
            result.insert(key.to_string(), value);
        }
        let mut result = Value::Object(result);
        let quality_signals =
            super::requirement_source_quality::build_parse_quality_signals(&result);
        if let Some(result_fields) = result.as_object_mut() {
            result_fields.insert("qualitySignals".to_string(), json!(quality_signals));
        }
        result
    }
}

impl RequirementSourcePipelineResult {
    pub fn source_inactive_major(&self, entry: &Value) -> bool {
        super::requirement_role_classifier::source_content_indicates_inactive_major(
            entry,
            self.facts.title.as_deref(),
            &self.facts.headings,
            &self.facts.requirement_cue_lines,
            &self.facts.choose_statements,
            &self.facts.snapshot_lines,
        )
    }

    pub fn effective_source_role(&self, entry: &Value) -> &'static str {
        if self.source_inactive_major(entry) {
            return "non-schedulable-course-list";
        }
        let metadata_source_role =
            super::requirement_role_classifier::classify_requirement_source_role(entry);
        if super::requirement_role_classifier::should_promote_primary_overview_from_parsed_evidence(
            entry,
            Some(self.source_role),
            metadata_source_role,
            &self.facts.candidate_course_codes,
            &self.facts.headings,
            &self.facts.requirement_cue_lines,
            &self.facts.choose_statements,
        ) {
            return "primary-degree-requirements";
        }
        if super::requirement_role_classifier::should_use_tacoma_metadata_primary_role(
            entry,
            Some(self.source_role),
            metadata_source_role,
            &self.facts.candidate_course_codes,
            &self.facts.requirement_cue_lines,
            &self.facts.choose_statements,
            &self.facts.pathway_labels,
        ) {
            return metadata_source_role;
        }
        if self.source_role.is_empty() {
            metadata_source_role
        } else {
            self.source_role
        }
    }

    pub fn inactive_major_evidence_lines(&self) -> Vec<String> {
        super::requirement_role_classifier::get_inactive_major_evidence_lines(
            &self.facts.requirement_cue_lines,
            &self.facts.choose_statements,
            &self.facts.snapshot_lines,
        )
    }

    pub fn requirement_cue_lines_with_inactive_major_evidence(&self, entry: &Value) -> Vec<String> {
        let mut lines = self.facts.requirement_cue_lines.clone();
        if self.source_inactive_major(entry) {
            lines.extend(self.inactive_major_evidence_lines());
        }
        let mut seen = std::collections::HashSet::new();
        lines.retain(|line| seen.insert(line.clone()));
        lines
    }

    pub fn parser_prerequisite_filter_audit_rows(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        super::requirement_section_roles::build_parser_prerequisite_filter_audit_rows(
            entry.get("ownerId"),
            entry.get("url"),
            Some(self.effective_source_role(entry)),
            &self.facts.headings,
            &self.facts.snapshot_lines,
            course_parser,
        )
    }

    pub fn safe_candidate_course_codes(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        let safe_course_codes =
            super::requirement_hint_safety::filter_parsed_course_codes_by_hints_with_source_recovery(
                entry,
                &self.facts.snapshot_lines,
                &self.facts.candidate_course_codes,
                course_parser,
            );
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        if source_scope.can_create_schedule_rows {
            super::requirement_scope::filter_parsed_course_codes_by_source_sections(
                &safe_course_codes,
                &self.parser_prerequisite_filter_audit_rows(entry, course_parser),
            )
        } else {
            safe_course_codes
        }
    }

    pub fn parsed_generic_choice_requirement_groups(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let safe_course_codes = self.safe_candidate_course_codes(entry, course_parser);
        super::requirement_groups::build_generic_choice_requirement_groups(
            entry,
            &safe_course_codes,
            &self.facts.snapshot_lines,
            course_parser,
        )
    }

    pub fn parsed_lettered_choose_one_requirement_groups(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        super::requirement_groups::build_lettered_choose_one_requirement_groups(
            entry,
            &self.facts.snapshot_lines,
            course_parser,
        )
    }

    pub fn parsed_sequence_choice_requirement_groups(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let mut groups = self.parsed_lettered_choose_one_requirement_groups(entry, course_parser);
        let safe_course_codes = self.safe_candidate_course_codes(entry, course_parser);
        groups.extend(
            super::requirement_groups::build_generic_sequence_choice_requirement_groups(
                entry,
                &safe_course_codes,
                &self.facts.snapshot_lines,
                course_parser,
            ),
        );
        groups
    }

    pub fn parsed_source_derived_sectioned_course_requirement_groups(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        super::requirement_groups::build_source_derived_sectioned_course_requirement_groups(
            entry,
            &self.facts.snapshot_lines,
            primary_major_titles,
            course_parser,
        )
    }

    pub fn parsed_sectioned_course_requirement_groups(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let mut groups = self.parsed_source_derived_sectioned_course_requirement_groups(
            entry,
            primary_major_titles,
            course_parser,
        );
        groups.extend(
            super::requirement_groups::build_sectioned_option_requirement_groups(
                entry,
                &self.facts.snapshot_lines,
                course_parser,
            ),
        );
        groups
    }

    pub fn parsed_pre_inline_requirement_group_candidates(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let mut groups = self.parsed_sectioned_course_requirement_groups(
            entry,
            primary_major_titles,
            course_parser,
        );
        groups.extend(
            super::requirement_groups::build_standalone_course_choice_requirement_groups(
                entry,
                &self.facts.snapshot_lines,
                course_parser,
            ),
        );
        groups.extend(
            super::requirement_groups::build_split_either_or_requirement_groups(
                entry,
                &self.facts.snapshot_lines,
                course_parser,
            ),
        );
        groups
    }

    pub fn parsed_inline_labeled_requirement_groups(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let snapshot_lines = &self.facts.snapshot_lines;
        let schedulable_source_line_indices =
            schedulable_inline_requirement_line_indices(entry, snapshot_lines, course_parser);
        let mut groups = Vec::new();
        let mut seen_group_ids = std::collections::HashSet::new();
        for (index, snapshot_line) in snapshot_lines.iter().enumerate() {
            if !schedulable_source_line_indices.contains(&index)
                || super::requirement_recovery_scope::is_graduate_or_applied_masters_requirement_context(
                    entry,
                    snapshot_lines,
                    index,
                )
                || super::requirement_recovery_scope::is_graduate_career_planning_continuation_line(
                    snapshot_lines,
                    index,
                    course_parser,
                )
                || super::requirement_choice_lists::is_lettered_option_line_under_choose_one_heading(
                    snapshot_lines,
                    index,
                )
                || super::requirement_groups::is_numbered_option_row_under_choose_one_heading(
                    snapshot_lines,
                    index,
                    course_parser,
                )
            {
                continue;
            }
            let raw_line = super::requirement_choice_lists::strip_choice_list_line(snapshot_line);
            let raw_line = super::requirement_shape::normalize_js_whitespace(&raw_line);
            if raw_line.is_empty()
                || super::requirement_hint_safety::source_line_starts_with_course_code(&raw_line)
                || super::requirement_groups::is_non_requirement_sectioned_course_heading(&raw_line)
                || super::requirement_groups::looks_like_degree_total_general_elective_prose(
                    &raw_line,
                )
                || super::requirement_groups::looks_like_elective_timing_or_sample_plan_note(
                    &raw_line,
                )
            {
                continue;
            }
            let Some(parts) = super::requirement_choice_lists::get_inline_labeled_requirement_parts(
                &raw_line,
                course_parser,
            ) else {
                if let Some(group) =
                    super::requirement_groups::build_numbered_inline_all_required_course_group(
                        entry,
                        &raw_line,
                        course_parser,
                    )
                {
                    let group_id = group
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if seen_group_ids.insert(group_id) {
                        groups.push(group);
                    }
                }
                continue;
            };
            for group in
                super::requirement_groups::build_inline_labeled_requirement_groups_for_parts(
                    entry,
                    snapshot_lines,
                    index,
                    &parts,
                    course_parser,
                )
            {
                let group_id = group
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if seen_group_ids.insert(group_id) {
                    groups.push(group);
                }
            }
        }
        if let Some(group) =
            super::requirement_groups::build_pathway_section_fallback_concentration_group(
                entry,
                snapshot_lines,
                &groups,
                primary_major_titles,
                course_parser,
            )
        {
            let group_id = group
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if seen_group_ids.insert(group_id) {
                groups.push(group);
            }
        }
        groups
    }

    pub fn parsed_pre_credit_bucket_requirement_group_candidates(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let mut groups = self.parsed_pre_inline_requirement_group_candidates(
            entry,
            primary_major_titles,
            course_parser,
        );
        groups.extend(self.parsed_inline_labeled_requirement_groups(
            entry,
            primary_major_titles,
            course_parser,
        ));
        groups
    }

    pub fn parsed_requirement_groups_with_inline_scope(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        self.parsed_requirement_groups_with_inline_scope_from_source_evidence(
            entry,
            &[],
            primary_major_titles,
            course_parser,
        )
    }

    pub fn parsed_requirement_groups_with_inline_scope_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let effective_source_role = self.effective_source_role(entry);
        let source_scope =
            super::requirement_scope::build_requirement_source_scope(Some(effective_source_role));
        if !source_scope.can_create_option_groups
            && !source_scope.can_create_credit_buckets
            && !source_scope.can_create_category_options
        {
            return Vec::new();
        }
        let snapshot_lines = &self.facts.snapshot_lines;
        let source_section_audit_rows =
            self.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let excluded_course_codes =
            super::requirement_source_audits::get_globally_explicitly_excluded_course_codes(
                snapshot_lines,
                &source_section_audit_rows,
                course_parser,
            );
        let mut safe_course_codes = self.safe_candidate_course_codes(entry, course_parser);
        let recovered_structured_codes =
            super::requirement_hint_safety::recover_structured_course_codes_from_source_evidence(
                entry,
                structured_course_codes,
                &self.facts.requirement_cue_lines,
                &self.facts.choose_statements,
                snapshot_lines,
                course_parser,
            );
        let recovered_structured_codes = if source_scope.can_create_schedule_rows {
            super::requirement_scope::filter_parsed_course_codes_by_source_sections(
                &recovered_structured_codes,
                &source_section_audit_rows,
            )
        } else {
            recovered_structured_codes
        };
        safe_course_codes.extend(recovered_structured_codes);
        let safe_course_codes =
            super::requirement_source_audits::filter_explicitly_excluded_course_codes(
                &safe_course_codes,
                &excluded_course_codes,
            );
        let safe_course_codes =
            super::requirement_course_scoping::filter_graduate_cross_listed_companions_for_undergraduate_source(
                entry,
                snapshot_lines,
                &safe_course_codes,
                course_parser,
            );
        let mut groups = self.parsed_pre_credit_bucket_requirement_group_candidates(
            entry,
            primary_major_titles,
            course_parser,
        );
        groups.extend(
            super::requirement_groups::build_generic_credit_bucket_requirement_groups(
                entry,
                snapshot_lines,
                course_parser,
            ),
        );
        groups.extend(
            super::requirement_groups::build_parallel_choose_one_requirement_groups(
                entry,
                snapshot_lines,
                course_parser,
            ),
        );
        groups.extend(
            super::requirement_groups::build_sbse_comparison_table_requirement_groups(
                entry,
                snapshot_lines,
            ),
        );
        groups.extend(self.parsed_lettered_choose_one_requirement_groups(entry, course_parser));
        groups.extend(
            super::requirement_groups::build_generic_sequence_choice_requirement_groups(
                entry,
                &safe_course_codes,
                snapshot_lines,
                course_parser,
            ),
        );
        groups.extend(
            super::requirement_groups::build_forward_labeled_choice_requirement_groups(
                entry,
                &safe_course_codes,
                snapshot_lines,
                course_parser,
            ),
        );
        groups.extend(
            super::requirement_groups::build_generic_choice_requirement_groups(
                entry,
                &safe_course_codes,
                snapshot_lines,
                course_parser,
            ),
        );
        let source_section_filtered_groups =
            super::requirement_source_audits::filter_parsed_requirement_groups_by_source_sections(
                &groups,
                &source_section_audit_rows,
                course_parser,
            );
        let finalized_groups = super::requirement_groups::finalize_parsed_requirement_groups(
            entry,
            &source_section_filtered_groups,
            snapshot_lines,
            primary_major_titles,
            course_parser,
        );
        super::requirement_source_audits::filter_parsed_requirement_groups_by_course_code_predicate(
            &finalized_groups,
            |course_code| {
                !excluded_course_codes
                    .iter()
                    .any(|excluded| {
                        super::requirement_course_codes::normalize_course_code(excluded)
                            == super::requirement_course_codes::normalize_course_code(
                                course_code.as_str().unwrap_or_default(),
                            )
                    })
                    && !super::requirement_course_scoping::is_graduate_cross_listed_companion_for_undergraduate_source(
                        entry,
                        snapshot_lines,
                        course_code.as_str().unwrap_or_default(),
                        course_parser,
                    )
            },
        )
    }

    pub fn parsed_requirement_groups_with_source_metadata(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        self.parsed_requirement_groups_with_source_evidence(
            entry,
            &[],
            primary_major_titles,
            course_parser,
        )
    }

    pub fn parsed_requirement_groups_with_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let groups = self.parsed_requirement_groups_with_inline_scope_from_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        let audit_rows = self.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let effective_source_role = self.effective_source_role(entry);
        let source_scope =
            super::requirement_scope::build_requirement_source_scope(Some(effective_source_role));
        groups
            .iter()
            .map(|group| {
                let decision = super::requirement_source_audits::get_group_source_section_decision(
                    group,
                    &audit_rows,
                    course_parser,
                );
                let mut enriched = group.as_object().cloned().unwrap_or_default();
                let source_row_text = group
                    .get("sourceRowText")
                    .filter(|value| !value.is_null())
                    .or_else(|| group.get("sourceHeading").filter(|value| !value.is_null()))
                    .cloned()
                    .unwrap_or(Value::Null);
                let source_section = group
                    .get("sourceSection")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or(Value::Null);
                let detected_cue = group
                    .get("detectedOptionCue")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or_else(|| {
                        let cue_source = group
                            .get("sourceHeading")
                            .filter(|value| !value.is_null())
                            .or_else(|| group.get("label"));
                        super::requirement_shape::detect_option_cue(
                            cue_source.and_then(Value::as_str).unwrap_or_default(),
                        )
                        .map(|cue| Value::String(cue.to_string()))
                        .unwrap_or(Value::Null)
                    });
                let requirement_type = group
                    .get("requirementType")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let min_courses = group.get("minCourses").filter(|value| !value.is_null());
                let max_courses = group.get("maxCourses").filter(|value| !value.is_null());
                let count_fallback = (requirement_type != "choose_credits"
                    && min_courses.is_some()
                    && min_courses == max_courses)
                    .then(|| min_courses.cloned().unwrap_or(Value::Null))
                    .unwrap_or(Value::Null);
                enriched.insert("sourceRole".to_string(), json!(effective_source_role));
                enriched.insert("supportOnly".to_string(), json!(source_scope.support_only));
                enriched.insert(
                    "sourceUrl".to_string(),
                    entry.get("url").cloned().unwrap_or(Value::Null),
                );
                enriched.insert("sourceRowText".to_string(), source_row_text);
                enriched.insert("sourceSection".to_string(), source_section);
                enriched.insert(
                    "sourceSectionRole".to_string(),
                    decision
                        .get("detectedSectionRole")
                        .filter(|value| !value.is_null())
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                enriched.insert(
                    "sourceSectionSchedulable".to_string(),
                    decision
                        .get("schedulable")
                        .filter(|value| !value.is_null())
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                enriched.insert("detectedOptionCue".to_string(), detected_cue);
                enriched.insert(
                    "approvedListKey".to_string(),
                    group
                        .get("approvedListKey")
                        .filter(|value| !value.is_null())
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                let can_create_placeholder = group
                    .get("canCreatePlaceholder")
                    .and_then(Value::as_bool)
                    .unwrap_or(requirement_type == "choose_credits");
                enriched.insert(
                    "canCreatePlaceholder".to_string(),
                    json!(can_create_placeholder),
                );
                enriched.insert(
                    "programSpecific".to_string(),
                    group
                        .get("programSpecific")
                        .and_then(Value::as_bool)
                        .map(Value::Bool)
                        .unwrap_or(Value::Null),
                );
                for (field, value) in [
                    ("selectionCount", count_fallback.clone()),
                    ("requiredCount", count_fallback),
                ] {
                    let value = group
                        .get(field)
                        .filter(|value| !value.is_null())
                        .cloned()
                        .unwrap_or(value);
                    enriched.insert(field.to_string(), value);
                }
                Value::Object(enriched)
            })
            .collect()
    }

    pub fn parsed_source_derived_requirement_courses(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        self.parsed_source_derived_requirement_courses_from_source_evidence(
            entry,
            &[],
            primary_major_titles,
            course_parser,
        )
    }

    pub fn parsed_source_derived_requirement_courses_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        if !source_scope.can_create_required_rows {
            return Vec::new();
        }
        let groups = self.parsed_requirement_groups_with_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        super::requirement_groups::build_source_derived_requirement_courses(&groups)
    }

    pub fn parsed_requirement_replacements(&self, entry: &Value) -> Vec<Value> {
        super::requirement_groups::build_source_derived_requirement_replacements(
            entry,
            &self.facts.snapshot_lines,
        )
    }

    pub fn parsed_requirement_replacements_with_source_scope(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        if !source_scope.can_create_required_rows {
            return Vec::new();
        }
        let audit_rows = self.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let schedulable_lines = super::requirement_scope::get_schedulable_source_section_lines(
            &self.facts.snapshot_lines,
            &audit_rows,
        );
        super::requirement_groups::build_source_derived_requirement_replacements(
            entry,
            &schedulable_lines,
        )
    }

    pub fn parsed_requirement_support_lists(
        &self,
        entry: &Value,
        parsed_requirement_groups: &[Value],
        parsed_course_codes: &[String],
        approved_filter_course_codes: &[String],
        elective_list_course_codes: &[String],
        support_only_course_codes: &[String],
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let effective_source_role = self.effective_source_role(entry);
        let source_scope =
            super::requirement_scope::build_requirement_source_scope(Some(effective_source_role));
        let role_status = super::requirement_scope::get_requirement_source_role_status(Some(
            effective_source_role,
        ));
        let source_url = entry.get("url").cloned().unwrap_or(Value::Null);
        let mut id_input = entry.as_object().cloned().unwrap_or_default();
        id_input.insert("adapterId".to_string(), json!(self.adapter.id));
        id_input.insert("sourceRole".to_string(), json!(effective_source_role));
        id_input.insert("sourceRoleStatus".to_string(), json!(role_status.as_str()));
        id_input.insert("sourceUrl".to_string(), source_url.clone());
        id_input.insert("supportOnly".to_string(), json!(source_scope.support_only));
        id_input.insert(
            "nonSchedulable".to_string(),
            json!(source_scope.non_schedulable),
        );
        id_input.insert(
            "canCreateSchedulableRows".to_string(),
            json!(source_scope.can_create_schedule_rows),
        );
        id_input.insert(
            "canCreateScheduleRows".to_string(),
            json!(source_scope.can_create_schedule_rows),
        );
        let source_block_id =
            super::requirement_ids::build_requirement_source_block_id(&Value::Object(id_input));
        let audit_rows = self.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let empty_codes = Vec::<String>::new();
        let approved_filter_course_codes = if source_scope.can_create_approved_filters {
            approved_filter_course_codes
        } else {
            &empty_codes
        };
        let elective_list_course_codes = if source_scope.can_create_elective_lists {
            elective_list_course_codes
        } else {
            &empty_codes
        };
        let support_only_course_codes = if source_scope.support_only || source_scope.non_schedulable
        {
            support_only_course_codes
        } else {
            &empty_codes
        };
        let mut support_input = entry.as_object().cloned().unwrap_or_default();
        support_input.insert("id".to_string(), json!(source_block_id));
        support_input.insert("sourceUrl".to_string(), source_url);
        support_input.insert(
            "sourceLabel".to_string(),
            entry.get("label").cloned().unwrap_or(Value::Null),
        );
        support_input.insert("sourceRole".to_string(), json!(effective_source_role));
        support_input.insert("sourceRoleStatus".to_string(), json!(role_status.as_str()));
        support_input.insert(
            "parserType".to_string(),
            entry.get("parserType").cloned().unwrap_or(Value::Null),
        );
        support_input.insert("supportOnly".to_string(), json!(source_scope.support_only));
        support_input.insert(
            "nonSchedulable".to_string(),
            json!(source_scope.non_schedulable),
        );
        support_input.insert(
            "parsedRequirementGroups".to_string(),
            json!(parsed_requirement_groups),
        );
        support_input.insert(
            "parsedUwCourseCodes".to_string(),
            json!(parsed_course_codes),
        );
        support_input.insert(
            "approvedFilterUwCourseCodes".to_string(),
            json!(approved_filter_course_codes),
        );
        support_input.insert(
            "electiveListUwCourseCodes".to_string(),
            json!(elective_list_course_codes),
        );
        support_input.insert(
            "supportOnlyUwCourseCodes".to_string(),
            json!(support_only_course_codes),
        );
        support_input.insert(
            "sourceSectionFilterAuditRows".to_string(),
            json!(audit_rows),
        );
        let support_input = Value::Object(support_input);
        let mut support_lists =
            super::requirement_source_audits::build_requirement_support_lists_from_metadata(
                &support_input,
            );
        support_lists.extend(
            super::requirement_source_audits::build_support_lists_from_parsed_requirement_groups(
                &support_input,
                course_parser,
            ),
        );
        support_lists.extend(
            super::requirement_source_audits::build_support_lists_from_source_section_audit_rows(
                &support_input,
            ),
        );
        super::requirement_source_audits::dedupe_requirement_support_lists(support_lists)
    }

    pub fn parsed_course_codes_from_source_facts(
        &self,
        entry: &Value,
        parsed_requirement_groups: &[Value],
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        self.parsed_course_codes_from_source_evidence(
            entry,
            &[],
            parsed_requirement_groups,
            course_parser,
        )
    }

    pub fn initial_parsed_course_codes_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        let source_section_audit_rows =
            self.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        let mut parsed_course_codes = self.safe_candidate_course_codes(entry, course_parser);
        let recovered_structured_codes =
            super::requirement_hint_safety::recover_structured_course_codes_from_source_evidence(
                entry,
                structured_course_codes,
                &self.facts.requirement_cue_lines,
                &self.facts.choose_statements,
                &self.facts.snapshot_lines,
                course_parser,
            );
        if source_scope.can_create_schedule_rows {
            parsed_course_codes.extend(
                super::requirement_scope::filter_parsed_course_codes_by_source_sections(
                    &recovered_structured_codes,
                    &source_section_audit_rows,
                ),
            );
        } else {
            parsed_course_codes.extend(recovered_structured_codes);
        }
        let parsed_course_codes = parsed_course_codes
            .iter()
            .map(|course_code| super::requirement_course_codes::normalize_course_code(course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let excluded_course_codes =
            super::requirement_source_audits::get_globally_explicitly_excluded_course_codes(
                &self.facts.snapshot_lines,
                &source_section_audit_rows,
                course_parser,
            );
        let parsed_course_codes =
            super::requirement_source_audits::filter_explicitly_excluded_course_codes(
                &parsed_course_codes,
                &excluded_course_codes,
            );
        super::requirement_course_scoping::filter_graduate_cross_listed_companions_for_undergraduate_source(
            entry,
            &self.facts.snapshot_lines,
            &parsed_course_codes,
            course_parser,
        )
    }

    pub fn parsed_course_codes_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        parsed_requirement_groups: &[Value],
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        let source_section_audit_rows =
            self.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        let mut parsed_course_codes = self.initial_parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            course_parser,
        );
        parsed_course_codes.extend(
            super::requirement_groups::extract_parsed_requirement_group_course_codes(
                parsed_requirement_groups,
            ),
        );
        let derived_courses = if source_scope.can_create_required_rows {
            super::requirement_groups::build_source_derived_requirement_courses(
                parsed_requirement_groups,
            )
        } else {
            Vec::new()
        };
        parsed_course_codes.extend(derived_courses.iter().filter_map(|course| {
            course
                .get("normalizedCourseCode")
                .and_then(Value::as_str)
                .map(str::to_string)
        }));
        parsed_course_codes = parsed_course_codes
            .into_iter()
            .map(|course_code| super::requirement_course_codes::normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect();
        parsed_course_codes.sort();
        parsed_course_codes.dedup();
        let excluded_course_codes =
            super::requirement_source_audits::get_globally_explicitly_excluded_course_codes(
                &self.facts.snapshot_lines,
                &source_section_audit_rows,
                course_parser,
            );
        let parsed_course_codes =
            super::requirement_source_audits::filter_explicitly_excluded_course_codes(
                &parsed_course_codes,
                &excluded_course_codes,
            );
        super::requirement_course_scoping::filter_graduate_cross_listed_companions_for_undergraduate_source(
            entry,
            &self.facts.snapshot_lines,
            &parsed_course_codes,
            course_parser,
        )
    }

    pub fn parsed_course_code_provenance_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Value {
        let groups = self.parsed_requirement_groups_with_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        let parsed_course_codes = self.parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            &groups,
            course_parser,
        );
        let source_only_course_codes = parsed_course_codes
            .iter()
            .filter(|course_code| !structured_course_codes.contains(course_code))
            .cloned()
            .collect::<Vec<_>>();
        let structured_only_course_codes = structured_course_codes
            .iter()
            .filter(|course_code| !parsed_course_codes.contains(course_code))
            .cloned()
            .collect::<Vec<_>>();
        json!({
            "structuredUwCourseCodes": structured_course_codes,
            "parsedUwCourseCodes": parsed_course_codes,
            "sourceOnlyUwCourseCodes": source_only_course_codes,
            "structuredOnlyUwCourseCodes": structured_only_course_codes,
        })
    }

    pub fn parsed_requirement_support_lists_from_source_facts(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        self.parsed_requirement_support_lists_from_source_evidence(
            entry,
            &[],
            primary_major_titles,
            course_parser,
        )
    }

    pub fn parsed_requirement_support_lists_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let groups = self.parsed_requirement_groups_with_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        let parsed_course_codes = self.parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            &groups,
            course_parser,
        );

        let effective_source_role = self.effective_source_role(entry);
        let source_scope =
            super::requirement_scope::build_requirement_source_scope(Some(effective_source_role));
        let mut owner = entry.as_object().cloned().unwrap_or_default();
        owner.insert("sourceRole".to_string(), json!(effective_source_role));
        owner.insert(
            "sourceLabel".to_string(),
            entry.get("label").cloned().unwrap_or(Value::Null),
        );
        owner.insert(
            "sourceUrl".to_string(),
            entry.get("url").cloned().unwrap_or(Value::Null),
        );
        let owner = Value::Object(owner);
        let filter_key =
            super::requirement_source_audits::infer_approved_list_key_from_support_metadata(&owner)
                .unwrap_or_default();
        let support_list = json!({
            "listTitle": entry.get("label").cloned().unwrap_or(Value::Null),
        });
        let evidence_input = json!({
            "filterKey": filter_key,
            "owner": owner,
            "supportList": support_list,
            "lines": self.facts.snapshot_lines,
            "fallbackCourseCodes": parsed_course_codes,
        });
        let approved_filter_course_codes = if source_scope.can_create_approved_filters {
            super::requirement_source_audits::get_approved_filter_codes_for_support_source(
                &evidence_input,
                course_parser,
            )
        } else {
            Vec::new()
        };
        let elective_list_course_codes = if source_scope.can_create_elective_lists {
            parsed_course_codes.clone()
        } else {
            Vec::new()
        };
        let support_only_course_codes = if source_scope.support_only || source_scope.non_schedulable
        {
            parsed_course_codes.clone()
        } else {
            Vec::new()
        };
        self.parsed_requirement_support_lists(
            entry,
            &groups,
            &parsed_course_codes,
            &approved_filter_course_codes,
            &elective_list_course_codes,
            &support_only_course_codes,
            course_parser,
        )
    }

    pub fn parsed_requirement_atom_candidates(
        &self,
        entry: &Value,
        parsed_course_codes: &[String],
        parsed_requirement_groups: &[Value],
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        if !source_scope.can_create_required_rows {
            return Vec::new();
        }
        let grouped_course_codes = parsed_requirement_groups
            .iter()
            .filter(|group| {
                group.get("requirementType").and_then(Value::as_str) != Some("all_required")
            })
            .flat_map(|group| {
                super::requirement_groups::extract_parsed_requirement_group_course_codes(
                    std::slice::from_ref(group),
                )
            })
            .map(|course_code| super::requirement_course_codes::normalize_course_code(&course_code))
            .collect::<std::collections::HashSet<_>>();
        let standalone_course_codes = parsed_course_codes
            .iter()
            .map(|course_code| super::requirement_course_codes::normalize_course_code(course_code))
            .filter(|course_code| {
                !course_code.is_empty() && !grouped_course_codes.contains(course_code)
            })
            .collect::<Vec<_>>();
        super::requirement_atoms::build_parsed_requirement_atom_candidates(
            entry,
            &standalone_course_codes,
            &self.facts.snapshot_lines,
            &self.parser_prerequisite_filter_audit_rows(entry, course_parser),
            course_parser,
        )
    }

    pub fn parsed_requirement_atom_candidates_from_source_facts(
        &self,
        entry: &Value,
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        self.parsed_requirement_atom_candidates_from_source_evidence(
            entry,
            &[],
            primary_major_titles,
            course_parser,
        )
    }

    pub fn parsed_requirement_atom_candidates_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let groups = self.parsed_requirement_groups_with_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        let course_codes = self.parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            &groups,
            course_parser,
        );
        self.parsed_requirement_atom_candidates(entry, &course_codes, &groups, course_parser)
    }

    pub fn parser_sequence_choice_audit_rows(
        &self,
        entry: &Value,
        parsed_requirement_groups: &[Value],
        parsed_requirement_atom_candidates: &[Value],
    ) -> Vec<Value> {
        super::requirement_source_audits::build_parser_sequence_choice_audit_rows(
            entry.get("ownerId"),
            entry.get("url"),
            parsed_requirement_groups,
            parsed_requirement_atom_candidates,
        )
    }

    pub fn blocked_source_section_course_codes(
        &self,
        entry: &Value,
        initial_parsed_course_codes: &[String],
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        if !source_scope.can_create_schedule_rows {
            return Vec::new();
        }
        let audit_rows = self.parser_prerequisite_filter_audit_rows(entry, course_parser);
        let blocked_codes =
            super::requirement_scope::get_blocked_source_section_course_codes(&audit_rows);
        let blocked_codes =
            super::requirement_course_scoping::filter_graduate_cross_listed_companions_for_undergraduate_source(
                entry,
                &self.facts.snapshot_lines,
                &blocked_codes,
                course_parser,
            );
        let initial_codes = initial_parsed_course_codes
            .iter()
            .map(|course_code| super::requirement_course_codes::normalize_course_code(course_code))
            .collect::<std::collections::HashSet<_>>();
        blocked_codes
            .into_iter()
            .filter(|course_code| {
                !initial_codes.contains(&super::requirement_course_codes::normalize_course_code(
                    course_code,
                ))
            })
            .collect()
    }

    pub fn source_scope_audit_lines_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        course_parser: &CourseCodeParser,
    ) -> Vec<String> {
        let groups = self.parsed_requirement_groups_with_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        let initial_course_codes = self.initial_parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            course_parser,
        );
        let parsed_course_codes = self.parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            &groups,
            course_parser,
        );
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        let parsed_requirement_courses = if source_scope.can_create_required_rows {
            super::requirement_groups::build_source_derived_requirement_courses(&groups)
        } else {
            Vec::new()
        };
        let parsed_requirement_atom_candidates = if source_scope.can_create_required_rows {
            self.parsed_requirement_atom_candidates(
                entry,
                &parsed_course_codes,
                &groups,
                course_parser,
            )
        } else {
            Vec::new()
        };
        let blocked_course_codes =
            self.blocked_source_section_course_codes(entry, &initial_course_codes, course_parser);
        self.source_scope_audit_lines(
            entry,
            &parsed_course_codes,
            &blocked_course_codes,
            &groups,
            &parsed_requirement_courses,
            &parsed_requirement_atom_candidates,
        )
    }

    pub fn source_scope_audit_lines(
        &self,
        entry: &Value,
        parsed_course_codes: &[String],
        blocked_source_section_course_codes: &[String],
        parsed_requirement_groups: &[Value],
        parsed_requirement_courses: &[Value],
        parsed_requirement_atom_candidates: &[Value],
    ) -> Vec<String> {
        let effective_source_role = self.effective_source_role(entry);
        let source_scope =
            super::requirement_scope::build_requirement_source_scope(Some(effective_source_role));
        let course_codes = parsed_course_codes
            .iter()
            .chain(blocked_source_section_course_codes.iter())
            .map(|course_code| super::requirement_course_codes::normalize_course_code(course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect::<Vec<_>>();
        super::requirement_source_audits::build_source_scope_audit_lines(
            entry.get("ownerId"),
            entry.get("url"),
            Some(effective_source_role),
            source_scope,
            &course_codes,
            parsed_requirement_groups,
            parsed_requirement_courses,
            parsed_requirement_atom_candidates,
        )
    }

    pub fn parsed_degree_map_block_candidates_from_source_evidence(
        &self,
        entry: &Value,
        structured_course_codes: &[String],
        primary_major_titles: &std::collections::HashMap<String, String>,
        structured_blocks: &[Value],
        course_parser: &CourseCodeParser,
    ) -> Vec<Value> {
        let groups = self.parsed_requirement_groups_with_source_evidence(
            entry,
            structured_course_codes,
            primary_major_titles,
            course_parser,
        );
        let parsed_course_codes = self.parsed_course_codes_from_source_evidence(
            entry,
            structured_course_codes,
            &groups,
            course_parser,
        );
        let requirement_cue_lines = self.requirement_cue_lines_with_inactive_major_evidence(entry);
        self.parsed_degree_map_block_candidates(
            entry,
            &parsed_course_codes,
            &requirement_cue_lines,
            &self.facts.choose_statements,
            &self.facts.pathway_labels,
            &self.facts.headings,
            structured_blocks,
        )
    }

    pub fn parsed_degree_map_block_candidates(
        &self,
        entry: &Value,
        parsed_course_codes: &[String],
        requirement_cue_lines: &[String],
        choose_statements: &[String],
        pathway_labels: &[String],
        headings: &[String],
        structured_blocks: &[Value],
    ) -> Vec<Value> {
        let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
            self.effective_source_role(entry),
        ));
        if !source_scope.can_create_schedule_rows {
            return Vec::new();
        }
        let mut cue_lines = requirement_cue_lines.to_vec();
        if self.source_inactive_major(entry) {
            cue_lines.extend(self.inactive_major_evidence_lines());
        }
        let mut seen_cue_lines = std::collections::HashSet::new();
        cue_lines.retain(|line| seen_cue_lines.insert(line.clone()));
        super::requirement_degree_map_blocks::build_parsed_degree_map_block_candidates(
            entry,
            parsed_course_codes,
            &cue_lines,
            choose_statements,
            pathway_labels,
            headings,
            structured_blocks,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceRecoveryCandidates {
    pub generic_section_candidates: Vec<Value>,
    pub parser_recovery_links: Vec<Value>,
    pub supplemental_html_links: Vec<Value>,
    pub supplemental_document_links: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserRecoveryCandidateParseResult {
    pub owner: Option<Value>,
    pub attempt: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementManifestEntrySourceFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_title: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<Value>,
    pub pathway_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campus_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_parser_type: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_source_url: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_source_label: Option<Value>,
    pub is_primary_degree_requirements_link: bool,
    pub structured_uw_course_codes: Vec<String>,
    #[serde(flatten)]
    pub parsed: RequirementSourcePipelineResult,
}

#[derive(Clone)]
pub struct RequirementSourcePipeline {
    html_parser: HtmlSourceParser,
    docx_parser: DocxSourceParser,
    pdf_parser: Option<PdfSourceParser>,
}

impl RequirementSourcePipeline {
    pub fn new(
        downloader: SourceDownloader,
        temporary_root: impl Into<PathBuf>,
        cache_capacity: usize,
    ) -> Self {
        Self {
            html_parser: HtmlSourceParser::with_cache_capacity(downloader.clone(), cache_capacity),
            docx_parser: DocxSourceParser::with_cache_capacity(
                downloader,
                temporary_root,
                cache_capacity,
            ),
            pdf_parser: None,
        }
    }

    pub fn with_pdf_source_parser(mut self, pdf_parser: PdfSourceParser) -> Self {
        self.pdf_parser = Some(pdf_parser);
        self
    }

    pub fn new_with_pdfjs_extractor(
        downloader: SourceDownloader,
        temporary_root: impl Into<PathBuf>,
        cache_capacity: usize,
        node_command: impl Into<String>,
        helper_script: impl Into<PathBuf>,
    ) -> Self {
        let temporary_root = temporary_root.into();
        let pdf_parser = PdfSourceParser::with_cache_capacity(
            downloader.clone(),
            node_command,
            helper_script,
            temporary_root.clone(),
            cache_capacity,
        );
        Self::new(downloader, temporary_root, cache_capacity).with_pdf_source_parser(pdf_parser)
    }

    pub fn collect_manifest_entry_source_facts(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        pdf_page_blocks: Option<&[PdfPageBlock]>,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementManifestEntrySourceFacts, String> {
        let mut parsed = self.collect_source_facts_with_context(
            context,
            entry,
            timeout,
            pdf_page_blocks,
            course_parser,
        )?;
        parsed.source_role = parsed.effective_source_role(entry);
        Ok(RequirementManifestEntrySourceFacts {
            owner_id: entry.get("ownerId").cloned(),
            owner_title: entry.get("ownerTitle").cloned(),
            plan_id: entry.get("planId").cloned(),
            pathway_id: context.materializable_pathway_id(entry),
            campus_id: entry.get("campusId").cloned(),
            primary_parser_type: entry.get("parserType").cloned(),
            primary_source_url: entry.get("url").cloned(),
            primary_source_label: entry.get("label").cloned(),
            is_primary_degree_requirements_link: js_truthy(
                entry.get("isPrimaryDegreeRequirementsLink"),
            ),
            structured_uw_course_codes: context.structured_uw_course_codes(entry, course_parser),
            parsed,
        })
    }

    pub fn parse_parser_recovery_candidate<F>(
        &self,
        context: &RequirementSourceManifestContext,
        base_result: &Value,
        structured_course_codes: &[String],
        entry: &Value,
        candidate: &Value,
        timeout: Duration,
        snapshot_store: &RequirementSnapshotStore,
        course_parser: &CourseCodeParser,
        mut pdf_page_blocks_for_entry: F,
    ) -> ParserRecoveryCandidateParseResult
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
    {
        let candidate_entry =
            super::requirement_manifest_fallback::build_parser_recovery_candidate_entry(
                entry, candidate,
            );
        let parsed_result = if let Some(section_lines_value) = candidate
            .get("sectionLines")
            .filter(|value| !value.is_null())
        {
            let section_lines = section_lines_value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            let headings = candidate
                .get("headings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            let source_url_value = candidate
                .get("url")
                .filter(|value| !value.is_null())
                .or_else(|| entry.get("url"));
            let source_url = source_url_value.cloned().unwrap_or(Value::Null);
            let owner_id_value = entry.get("ownerId");
            let owner_label = js_interpolate_option(owner_id_value);
            let line_start = js_interpolate_option(candidate.get("sectionLineStart"));
            let line_end = js_interpolate_option(candidate.get("sectionLineEnd"));
            let source_url_text = js_interpolate_option(source_url_value);
            let mut section_audit_fields = serde_json::Map::new();
            section_audit_fields.insert("scoped".to_string(), json!(true));
            section_audit_fields.insert(
                "line".to_string(),
                json!(format!("Parser recovery scoped {owner_label} to lines {line_start}-{line_end} from {source_url_text}.")),
            );
            if let Some(source_url) = source_url_value {
                section_audit_fields.insert("sourceUrl".to_string(), source_url.clone());
            }
            if let Some(owner_id) = owner_id_value {
                section_audit_fields.insert("ownerId".to_string(), owner_id.clone());
            }
            section_audit_fields.insert(
                "reason".to_string(),
                json!("parser-warning-section-scoping"),
            );
            let section_audit = Value::Object(section_audit_fields);
            let artifacts = super::requirement_html::RequirementHtmlArtifacts {
                html: String::new(),
                title: candidate
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                headings,
                lines: section_lines,
                links: Vec::new(),
            };
            let parser_type = candidate_entry
                .get("parserType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut parsed = RequirementSourcePipelineResult {
                adapter: select_requirement_source_adapter(&candidate_entry),
                source_role: classify_requirement_source_role(&candidate_entry),
                facts: collect_html_source_facts_for_entry_with_primary_titles(
                    &candidate_entry,
                    &artifacts,
                    parser_type,
                    course_parser,
                    context.primary_major_titles(),
                ),
            };
            parsed.source_role = parsed.effective_source_role(&candidate_entry);
            Ok((parsed, Some(section_audit)))
        } else {
            let page_blocks = pdf_page_blocks_for_entry(&candidate_entry);
            self.collect_source_facts_with_context(
                context,
                &candidate_entry,
                timeout,
                page_blocks.as_deref(),
                course_parser,
            )
            .map(|mut parsed| {
                parsed.source_role = parsed.effective_source_role(&candidate_entry);
                (parsed, None)
            })
        };
        let (parsed, source_section_audit) = match parsed_result {
            Ok(parsed) => parsed,
            Err(error) => {
                return ParserRecoveryCandidateParseResult {
                    owner: None,
                    attempt:
                        super::requirement_source_recovery::build_parser_recovery_attempt_record(
                            candidate,
                            "parse-failed",
                            &json!({
                                "reason": "candidate failed during targeted parser recovery",
                                "error": error,
                            }),
                        ),
                };
            }
        };
        if !super::requirement_source_scoring::has_meaningful_parsed_content(&parsed.facts) {
            return ParserRecoveryCandidateParseResult {
                owner: None,
                attempt: super::requirement_source_recovery::build_parser_recovery_attempt_record(
                    candidate,
                    "no-meaningful-content",
                    &json!({
                        "reason": "candidate parsed but did not expose requirement cues, choices, pathways, or UW course codes",
                        "snapshotLines": parsed.facts.snapshot_lines,
                    }),
                ),
            };
        }
        let resolution_strategy = if super::requirement_source_recovery::same_source_url(
            candidate_entry.get("url"),
            entry.get("url"),
        ) {
            "primary-source"
        } else {
            "alternate-official-source"
        };
        let resolution = RequirementSourceResolution {
            parsed: parsed.clone(),
            resolution_strategy,
            ok: true,
            snapshot_path: None,
            snapshot_has_heading_metadata: false,
            used_snapshot_fallback: false,
            snapshot_fallback_reason: None,
            error: None,
        };
        let owner = resolution.build_manifest_parse_success(
            base_result,
            &candidate_entry,
            structured_course_codes,
            context.primary_major_titles(),
            context.degree_map_blocks(),
            source_section_audit.as_ref(),
            Some(snapshot_store),
            course_parser,
        );
        let quality_signal_codes = owner
            .get("qualitySignals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|signal| signal.get("code").cloned())
            .collect::<Vec<_>>();
        let quality_warning_codes = owner
            .get("qualitySignals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("warning"))
            .filter_map(|signal| signal.get("code").cloned())
            .collect::<Vec<_>>();
        let attempt = super::requirement_source_recovery::build_parser_recovery_attempt_record(
            candidate,
            "parsed",
            &json!({
                "sourceRole": owner.get("sourceRole").cloned().unwrap_or(Value::Null),
                "sourceRoleStatus": owner.get("sourceRoleStatus").cloned().unwrap_or(Value::Null),
                "parsedUwCourseCodes": owner.get("parsedUwCourseCodes").cloned().unwrap_or_else(|| json!([])),
                "snapshotLines": parsed.facts.snapshot_lines,
                "qualitySignalCodes": quality_signal_codes,
                "qualityWarningCodes": quality_warning_codes,
            }),
        );
        ParserRecoveryCandidateParseResult {
            owner: Some(owner),
            attempt,
        }
    }

    pub fn recover_manifest_result_from_candidates<F>(
        &self,
        context: &RequirementSourceManifestContext,
        base_result: &Value,
        structured_course_codes: &[String],
        entry: &Value,
        owner: Value,
        candidates: &[Value],
        initial_attempts: &[Value],
        source_unavailable_error: Option<String>,
        timeout: Duration,
        snapshot_store: &RequirementSnapshotStore,
        course_parser: &CourseCodeParser,
        mut pdf_page_blocks_for_entry: F,
    ) -> Value
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
    {
        let mut before_owner = owner;
        let before_quality_signals =
            super::requirement_source_quality::build_parse_quality_signals(&before_owner);
        if let Some(fields) = before_owner.as_object_mut() {
            fields.insert("qualitySignals".to_string(), json!(before_quality_signals));
        }
        let trigger_codes =
            super::requirement_source_quality::parser_recovery_trigger_codes(&before_owner);
        if trigger_codes.is_empty() {
            return before_owner;
        }
        let before_snapshot =
            super::requirement_source_recovery::build_parser_recovery_owner_snapshot(&before_owner);
        let original_source_url = before_owner
            .get("sourceUrl")
            .filter(|value| !value.is_null())
            .or_else(|| entry.get("url"))
            .cloned()
            .unwrap_or(Value::Null);
        let original_source_label = before_owner
            .get("sourceLabel")
            .filter(|value| !value.is_null())
            .or_else(|| entry.get("label"))
            .cloned()
            .unwrap_or(Value::Null);
        let source_unavailable = source_unavailable_error.is_some();
        let mut attempts = initial_attempts.to_vec();
        if let Some(error) = source_unavailable_error {
            let unavailable_candidate = json!({
                "strategy": "source-artifact-recovery",
                "url": entry.get("url").cloned().unwrap_or(Value::Null),
                "label": entry.get("label").cloned().unwrap_or(Value::Null),
                "parserType": entry.get("parserType").cloned().unwrap_or(Value::Null),
                "sourceRole": before_owner.get("sourceRole").cloned().unwrap_or(Value::Null),
                "sourceRoleStatus": before_owner.get("sourceRoleStatus").cloned().unwrap_or(Value::Null),
                "score": Value::Null,
                "reason": "refetch source HTML for parser recovery",
            });
            attempts.push(
                super::requirement_source_recovery::build_parser_recovery_attempt_record(
                    &unavailable_candidate,
                    "source-unavailable",
                    &json!({
                        "error": error,
                        "reason": "refetch source HTML for parser recovery",
                    }),
                ),
            );
        }
        let mut attempted_strategies = Vec::new();
        for candidate in candidates {
            if let Some(strategy) = candidate.get("strategy").and_then(Value::as_str) {
                if !attempted_strategies
                    .iter()
                    .any(|existing: &String| existing.as_str() == strategy)
                {
                    attempted_strategies.push(strategy.to_string());
                }
            }
        }
        let mut best_owner = before_owner.clone();
        let mut best_candidate = None::<Value>;
        let mut best_score =
            super::requirement_source_recovery::score_parser_recovery_owner(&before_owner);
        let mut support_owners = Vec::new();
        let mut support_sources = Vec::new();
        for candidate in candidates {
            let parsed_candidate = self.parse_parser_recovery_candidate(
                context,
                base_result,
                structured_course_codes,
                entry,
                candidate,
                timeout,
                snapshot_store,
                course_parser,
                &mut pdf_page_blocks_for_entry,
            );
            attempts.push(parsed_candidate.attempt.clone());
            let Some(candidate_owner) = parsed_candidate.owner else {
                continue;
            };
            let is_support = candidate_owner.get("supportOnly").and_then(Value::as_bool)
                == Some(true)
                || candidate_owner
                    .get("sourceRoleStatus")
                    .and_then(Value::as_str)
                    == Some("support");
            if is_support {
                let support_lists = candidate_owner
                    .get("supportLists")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if !support_lists.is_empty() {
                    let accepted_codes = support_lists
                        .iter()
                        .flat_map(|list| {
                            list.get("acceptedUwCourseCodes")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                        })
                        .map(str::to_string)
                        .collect::<std::collections::BTreeSet<_>>();
                    support_sources.push(json!({
                        "strategy": candidate.get("strategy").cloned().unwrap_or(Value::Null),
                        "sourceUrl": candidate_owner.get("sourceUrl").cloned().unwrap_or(Value::Null),
                        "sourceLabel": candidate_owner.get("sourceLabel").cloned().unwrap_or(Value::Null),
                        "sourceRole": candidate_owner.get("sourceRole").cloned().unwrap_or(Value::Null),
                        "acceptedUwCourseCodeCount": accepted_codes.len(),
                        "sourceEvidenceFingerprint": parsed_candidate.attempt.get("sourceEvidenceFingerprint").cloned().unwrap_or(Value::Null),
                    }));
                    support_owners.push(candidate_owner);
                }
                continue;
            }
            if !super::requirement_source_recovery::should_accept_parser_recovery_owner(
                &before_owner,
                &candidate_owner,
                candidate,
                context.primary_major_titles(),
                course_parser,
            ) {
                continue;
            }
            let candidate_score =
                super::requirement_source_recovery::score_parser_recovery_owner(&candidate_owner);
            if best_candidate.is_none() || candidate_score > best_score {
                best_score = candidate_score;
                best_owner = candidate_owner;
                best_candidate = Some(candidate.clone());
            }
        }
        let mut final_owner = super::requirement_source_recovery::merge_recovered_support_sources(
            &best_owner,
            &support_owners,
        );
        let final_quality_signals =
            super::requirement_source_quality::build_parse_quality_signals(&final_owner);
        if let Some(fields) = final_owner.as_object_mut() {
            fields.insert("qualitySignals".to_string(), json!(final_quality_signals));
        }
        let after_snapshot =
            super::requirement_source_recovery::build_parser_recovery_owner_snapshot(&final_owner);
        let mut recovered_sources = Vec::new();
        let mut selected_strategy = Value::Null;
        let mut selected_source_url = Value::Null;
        if let Some(candidate) = best_candidate {
            selected_strategy = candidate.get("strategy").cloned().unwrap_or(Value::Null);
            selected_source_url = candidate.get("url").cloned().unwrap_or(Value::Null);
            recovered_sources.push(json!({
                "strategy": selected_strategy,
                "sourceUrl": final_owner.get("sourceUrl").cloned().unwrap_or(Value::Null),
                "sourceLabel": final_owner.get("sourceLabel").cloned().unwrap_or(Value::Null),
                "sourceRole": final_owner.get("sourceRole").cloned().unwrap_or(Value::Null),
                "parsedUwCourseCodeCount": final_owner.get("parsedUwCourseCodes").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
                "sourceEvidenceFingerprint": after_snapshot.get("sourceFingerprint").cloned().unwrap_or(Value::Null),
            }));
        }
        let before_support_count = before_owner
            .get("supportLists")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let after_support_count = final_owner
            .get("supportLists")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let succeeded = !recovered_sources.is_empty()
            || (!support_owners.is_empty() && after_support_count > before_support_count);
        let blocker_type = if succeeded {
            Value::Null
        } else {
            json!(
                super::requirement_source_recovery::classify_parser_recovery_blocker(
                    source_unavailable,
                    candidates.len(),
                    &attempts,
                )
            )
        };
        let recovery = json!({
            "triggered": true,
            "triggerCodes": trigger_codes,
            "originalSourceUrl": original_source_url,
            "originalSourceLabel": original_source_label,
            "originalSourceFingerprint": before_snapshot.get("sourceFingerprint").cloned().unwrap_or(Value::Null),
            "selectedStrategy": selected_strategy,
            "selectedSourceUrl": selected_source_url,
            "attemptedStrategies": attempted_strategies,
            "attempts": attempts,
            "recoveredSources": recovered_sources,
            "supportSources": support_sources,
            "before": before_snapshot,
            "after": after_snapshot,
            "blockerType": blocker_type,
            "sourceUnavailable": source_unavailable,
            "candidateCount": candidates.len(),
            "succeeded": succeeded,
        });
        if let Some(fields) = final_owner.as_object_mut() {
            fields.insert("parserRecovery".to_string(), recovery);
        }
        final_owner
    }

    pub fn collect_parser_recovery_section_lines(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<String>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.scope_parser_recovery_section_lines(entry, &artifacts.lines, course_parser))
    }

    pub fn collect_generic_parser_recovery_section_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.generic_parser_recovery_section_candidates(
            entry,
            &artifacts.lines,
            artifacts.title.as_deref(),
            &artifacts.headings,
            course_parser,
        ))
    }

    pub fn collect_parser_recovery_link_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.extract_parser_recovery_link_candidates(entry, &artifacts.links, course_parser))
    }

    pub fn collect_supplemental_html_link_candidates(
        &self,
        entry: &Value,
        timeout: Duration,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(extract_supplemental_html_link_candidates(
            entry,
            &artifacts.links,
        ))
    }

    pub fn collect_supplemental_document_link_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.extract_supplemental_document_link_candidates(
            entry,
            &artifacts.links,
            course_parser,
        ))
    }

    pub fn collect_html_recovery_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementSourceRecoveryCandidates, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(RequirementSourceRecoveryCandidates {
                generic_section_candidates: Vec::new(),
                parser_recovery_links: Vec::new(),
                supplemental_html_links: Vec::new(),
                supplemental_document_links: Vec::new(),
            });
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(RequirementSourceRecoveryCandidates {
            generic_section_candidates: context.generic_parser_recovery_section_candidates(
                entry,
                &artifacts.lines,
                artifacts.title.as_deref(),
                &artifacts.headings,
                course_parser,
            ),
            parser_recovery_links: context.extract_parser_recovery_link_candidates(
                entry,
                &artifacts.links,
                course_parser,
            ),
            supplemental_html_links: extract_supplemental_html_link_candidates(
                entry,
                &artifacts.links,
            ),
            supplemental_document_links: context.extract_supplemental_document_link_candidates(
                entry,
                &artifacts.links,
                course_parser,
            ),
        })
    }

    pub fn parse_selected_manifest_entries_and_build_output<F, S>(
        &self,
        context: &RequirementSourceManifestContext,
        target_plan_ids: Option<&[String]>,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        parse_options: &Value,
        markdown_options: &Value,
        repository_root: &std::path::Path,
        mut source_section_audit_for_entry: S,
        pdf_page_blocks_for_entry: F,
    ) -> Result<super::requirement_source_output::RequirementSourcePipelineOutput, String>
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
        S: FnMut(&Value) -> Option<Value>,
    {
        let snapshot_only =
            parse_options.get("snapshotOnly").and_then(Value::as_bool) == Some(true);
        let owners = if snapshot_only {
            let selected = context.select_parseable_entries(target_plan_ids, course_parser);
            let deduplicated =
                super::requirement_manifest::dedupe_parseable_primary_entries(&selected);
            deduplicated
                .iter()
                .map(|entry| {
                    let base_result =
                        build_manifest_parse_base_result(context, entry, course_parser);
                    let source_section_audit =
                        source_section_audit_for_entry(entry).or_else(|| {
                            entry
                                .get("sourceSectionAudit")
                                .filter(|value| !value.is_null())
                                .cloned()
                        });
                    self.parse_snapshot_only_manifest_entry_result(
                        context,
                        &base_result,
                        entry,
                        course_parser,
                        snapshot_store,
                        source_section_audit.as_ref(),
                    )
                })
                .collect()
        } else {
            self.parse_selected_entries(
                context,
                target_plan_ids,
                timeout,
                course_parser,
                snapshot_store,
                source_section_audit_for_entry,
                pdf_page_blocks_for_entry,
            )
        };
        super::requirement_source_output::build_requirement_source_pipeline_output(
            &owners,
            parse_options,
            markdown_options,
            repository_root,
            snapshot_store,
            course_parser,
        )
    }

    pub fn parse_selected_manifest_entries_and_write_output<F, S>(
        &self,
        context: &RequirementSourceManifestContext,
        target_plan_ids: Option<&[String]>,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        parse_options: &Value,
        markdown_options: &Value,
        repository_root: &std::path::Path,
        artifact_paths: &super::requirement_source_output::RequirementSourcePipelineArtifactPaths,
        source_section_audit_for_entry: S,
        pdf_page_blocks_for_entry: F,
    ) -> Result<super::requirement_source_output::RequirementSourcePipelineOutput, String>
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
        S: FnMut(&Value) -> Option<Value>,
    {
        let output = self.parse_selected_manifest_entries_and_build_output(
            context,
            target_plan_ids,
            timeout,
            course_parser,
            snapshot_store,
            parse_options,
            markdown_options,
            repository_root,
            source_section_audit_for_entry,
            pdf_page_blocks_for_entry,
        )?;
        super::requirement_source_output::write_requirement_source_pipeline_artifacts(
            &output.artifacts,
            artifact_paths,
        )?;
        Ok(output)
    }

    fn parse_snapshot_only_manifest_entry_result(
        &self,
        context: &RequirementSourceManifestContext,
        base_result: &Value,
        entry: &Value,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        source_section_audit: Option<&Value>,
    ) -> Value {
        let structured_course_codes = context.structured_uw_course_codes(entry, course_parser);
        let resolution = self.collect_snapshot_only_source_facts_with_primary_titles(
            entry,
            course_parser,
            snapshot_store,
            context.primary_major_titles(),
        );
        if !resolution.ok {
            return resolution.build_manifest_parse_failure(
                base_result,
                entry,
                &structured_course_codes,
            );
        }
        resolution.build_manifest_parse_success(
            base_result,
            entry,
            &structured_course_codes,
            context.primary_major_titles(),
            context.degree_map_blocks(),
            source_section_audit,
            Some(snapshot_store),
            course_parser,
        )
    }

    pub fn parse_selected_entries<F, S>(
        &self,
        context: &RequirementSourceManifestContext,
        target_plan_ids: Option<&[String]>,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        source_section_audit_for_entry: S,
        pdf_page_blocks_for_entry: F,
    ) -> Vec<Value>
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
        S: FnMut(&Value) -> Option<Value>,
    {
        self.parse_selected_manifest_entries(
            context,
            target_plan_ids,
            timeout,
            course_parser,
            snapshot_store,
            |entry| build_manifest_parse_base_result(context, entry, course_parser),
            source_section_audit_for_entry,
            pdf_page_blocks_for_entry,
        )
    }

    pub fn parse_selected_manifest_entries<F, B, S>(
        &self,
        context: &RequirementSourceManifestContext,
        target_plan_ids: Option<&[String]>,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        base_result_for_entry: B,
        source_section_audit_for_entry: S,
        pdf_page_blocks_for_entry: F,
    ) -> Vec<Value>
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
        B: FnMut(&Value) -> Value,
        S: FnMut(&Value) -> Option<Value>,
    {
        let selected = context.select_parseable_entries(target_plan_ids, course_parser);
        let deduplicated = super::requirement_manifest::dedupe_parseable_primary_entries(&selected);
        self.parse_manifest_entries(
            context,
            &deduplicated,
            timeout,
            course_parser,
            snapshot_store,
            base_result_for_entry,
            source_section_audit_for_entry,
            pdf_page_blocks_for_entry,
        )
    }

    pub fn parse_manifest_entries<F, B, S>(
        &self,
        context: &RequirementSourceManifestContext,
        entries: &[Value],
        timeout: Duration,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        mut base_result_for_entry: B,
        mut source_section_audit_for_entry: S,
        mut pdf_page_blocks_for_entry: F,
    ) -> Vec<Value>
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
        B: FnMut(&Value) -> Value,
        S: FnMut(&Value) -> Option<Value>,
    {
        entries
            .iter()
            .map(|entry| {
                let base_result = base_result_for_entry(entry);
                let source_section_audit = source_section_audit_for_entry(entry).or_else(|| {
                    entry
                        .get("sourceSectionAudit")
                        .filter(|value| !value.is_null())
                        .cloned()
                });
                self.parse_manifest_entry_result(
                    context,
                    &base_result,
                    entry,
                    timeout,
                    course_parser,
                    snapshot_store,
                    source_section_audit.as_ref(),
                    |candidate| pdf_page_blocks_for_entry(candidate),
                )
            })
            .collect()
    }

    pub fn parse_manifest_entry_result<F>(
        &self,
        context: &RequirementSourceManifestContext,
        base_result: &Value,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        source_section_audit: Option<&Value>,
        mut pdf_page_blocks_for_entry: F,
    ) -> Value
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
    {
        let structured_course_codes = context.structured_uw_course_codes(entry, course_parser);
        let (effective_entry, resolution) = self.collect_manifest_entry_resolution(
            context,
            entry,
            timeout,
            course_parser,
            snapshot_store,
            &mut pdf_page_blocks_for_entry,
        );
        if !resolution.ok {
            return resolution.build_manifest_parse_failure(
                base_result,
                &effective_entry,
                &structured_course_codes,
            );
        }
        let mut owner = resolution.build_manifest_parse_success(
            base_result,
            &effective_entry,
            &structured_course_codes,
            context.primary_major_titles(),
            context.degree_map_blocks(),
            source_section_audit,
            Some(snapshot_store),
            course_parser,
        );
        if super::requirement_source_quality::parser_recovery_trigger_codes(&owner).is_empty() {
            return owner;
        }
        let (skip_html_candidates, skip_snapshot_candidates) =
            context.parser_recovery_catalog_scope_flags(entry, &owner);
        let source_url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let html_recovery_allowed =
            !is_pdf_source_url(source_url) && !is_docx_source_url(source_url);
        let mut candidates = Vec::new();
        let mut initial_attempts = Vec::new();
        let mut html_source_error = None;
        if html_recovery_allowed {
            match self.collect_html_recovery_candidates(context, entry, timeout, course_parser) {
                Ok(recovery_candidates) => {
                    if !skip_html_candidates {
                        candidates.extend(recovery_candidates.generic_section_candidates);
                        candidates.extend(recovery_candidates.parser_recovery_links);
                    }
                }
                Err(error) => {
                    let snapshot_source_url = owner
                        .get("sourceUrl")
                        .and_then(Value::as_str)
                        .unwrap_or(source_url);
                    let snapshot_path = owner
                        .get("snapshotPath")
                        .and_then(Value::as_str)
                        .map(PathBuf::from);
                    let cached_snapshot = snapshot_path
                        .as_deref()
                        .and_then(|path| {
                            snapshot_store.read_snapshot_file(path, Some(snapshot_source_url))
                        })
                        .or_else(|| {
                            snapshot_store.read_snapshot(
                                entry
                                    .get("ownerId")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                                Some(snapshot_source_url),
                            )
                        });
                    if let Some(snapshot) =
                        cached_snapshot.filter(|snapshot| !snapshot.snapshot_lines.is_empty())
                    {
                        let snapshot_candidate = json!({
                            "strategy": "source-artifact-recovery",
                            "url": entry.get("url").cloned().unwrap_or(Value::Null),
                            "label": entry.get("label").cloned().unwrap_or(Value::Null),
                            "parserType": entry.get("parserType").cloned().unwrap_or(Value::Null),
                            "sourceRole": owner.get("sourceRole").cloned().unwrap_or(Value::Null),
                            "sourceRoleStatus": owner.get("sourceRoleStatus").cloned().unwrap_or(Value::Null),
                            "score": Value::Null,
                            "reason": "reuse cached source snapshot for parser recovery",
                        });
                        initial_attempts.push(
                            super::requirement_source_recovery::build_parser_recovery_attempt_record(
                                &snapshot_candidate,
                                "snapshot-artifacts-used",
                                &json!({
                                    "error": error,
                                    "snapshotLines": snapshot.snapshot_lines.clone(),
                                }),
                            ),
                        );
                        if !skip_html_candidates {
                            let fallback_title = owner
                                .get("extractedTitle")
                                .filter(|value| !value.is_null())
                                .and_then(Value::as_str)
                                .or(snapshot.title.as_deref())
                                .or_else(|| owner.get("sourceLabel").and_then(Value::as_str))
                                .or_else(|| entry.get("label").and_then(Value::as_str));
                            let fallback_headings = owner
                                .get("extractedHeadings")
                                .and_then(Value::as_array)
                                .filter(|headings| !headings.is_empty())
                                .map(|headings| {
                                    headings
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .map(str::to_string)
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_else(|| snapshot.headings.clone());
                            candidates.extend(context.generic_parser_recovery_section_candidates(
                                entry,
                                &snapshot.snapshot_lines,
                                fallback_title,
                                &fallback_headings,
                                course_parser,
                            ));
                            let snapshot_links =
                                super::requirement_source_recovery::build_snapshot_parser_recovery_link_pairs(
                                    &snapshot.snapshot_lines,
                                );
                            candidates.extend(context.extract_parser_recovery_link_candidates(
                                entry,
                                &snapshot_links,
                                course_parser,
                            ));
                        }
                    } else {
                        html_source_error = Some(error);
                    }
                }
            }
        }
        if !skip_snapshot_candidates {
            let current_snapshot_path = owner
                .get("snapshotPath")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            candidates.extend(context.cached_snapshot_recovery_candidates(
                entry,
                snapshot_store,
                current_snapshot_path.as_deref(),
                course_parser,
            ));
        }
        let mut seen_candidates = std::collections::HashSet::new();
        candidates.retain(|candidate| {
            let key = [
                candidate
                    .get("strategy")
                    .map(js_string_value)
                    .unwrap_or_default(),
                candidate
                    .get("url")
                    .map(js_string_value)
                    .unwrap_or_default(),
                candidate
                    .get("sectionLineStart")
                    .map(js_string_value)
                    .unwrap_or_default(),
                candidate
                    .get("sectionLineEnd")
                    .map(js_string_value)
                    .unwrap_or_default(),
            ]
            .join(":");
            seen_candidates.insert(key)
        });
        let source_unavailable_error = html_source_error;
        owner = self.recover_manifest_result_from_candidates(
            context,
            base_result,
            &structured_course_codes,
            entry,
            owner,
            &candidates,
            &initial_attempts,
            source_unavailable_error,
            timeout,
            snapshot_store,
            course_parser,
            &mut pdf_page_blocks_for_entry,
        );
        owner
    }

    fn merge_linked_html_sources(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        base_resolution: &RequirementSourceResolution,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        preferred_source_entry: &mut Option<Value>,
    ) -> (RequirementSourceResolution, bool) {
        if select_requirement_source_adapter(entry).parser != RequirementSourceParserKind::Html {
            return (base_resolution.clone(), false);
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        if is_pdf_source_url(url) || is_docx_source_url(url) {
            return (base_resolution.clone(), false);
        }
        let Ok(artifacts) = self.html_parser.get_html_source_artifacts(url, timeout) else {
            return (base_resolution.clone(), false);
        };
        let candidates = extract_supplemental_html_link_candidates(entry, &artifacts.links);
        let mut parsed_sources = Vec::new();
        for candidate in candidates {
            let candidate_url = candidate
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if candidate_url.is_empty() || candidate_url == url {
                continue;
            }
            let label = candidate
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut candidate_fields = entry.as_object().cloned().unwrap_or_default();
            candidate_fields.remove("role");
            candidate_fields.insert("url".to_string(), json!(candidate_url));
            candidate_fields.insert("label".to_string(), json!(label));
            let candidate_entry = Value::Object(candidate_fields);
            let candidate_role = classify_requirement_source_role(&candidate_entry);
            if !super::requirement_scope::build_requirement_source_scope(Some(candidate_role))
                .can_create_schedule_rows
            {
                continue;
            }
            let Ok(mut parsed) = self.collect_source_facts_with_context(
                context,
                &candidate_entry,
                timeout,
                None,
                course_parser,
            ) else {
                continue;
            };
            parsed.source_role = parsed.effective_source_role(&candidate_entry);
            if is_track_catalog_supplemental_candidate(&candidate) {
                parsed.facts.candidate_course_codes.retain(|course_code| {
                    super::requirement_source_scoring::get_course_code_numeric_level(course_code)
                        .is_some_and(|level| level < 500)
                });
            }
            if !super::requirement_source_scoring::has_meaningful_parsed_content(&parsed.facts) {
                continue;
            }
            let alignment = super::requirement_source_scoring::get_parsed_owner_alignment_score(
                &candidate_entry,
                &parsed.facts,
            );
            let same_program_link = candidate
                .get("sameProgramRequirementLink")
                .and_then(Value::as_bool)
                == Some(true);
            let candidate_type = candidate
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let minimum_alignment = if same_program_link {
                0
            } else if candidate_type == "general" {
                3
            } else {
                2
            };
            if alignment < minimum_alignment
                || (candidate_type == "general"
                    && !same_program_link
                    && alignment < 10
                    && parsed.facts.candidate_course_codes.len() > 50)
            {
                continue;
            }
            let supplemental_entry = json!({"label": label, "url": candidate_url});
            parsed_sources.push(ParsedLinkedHtmlSource {
                candidate,
                entry: supplemental_entry,
                resolution: RequirementSourceResolution {
                    parsed,
                    resolution_strategy: "supplemental-html-source",
                    ok: true,
                    snapshot_path: None,
                    snapshot_has_heading_metadata: false,
                    used_snapshot_fallback: false,
                    snapshot_fallback_reason: None,
                    error: None,
                },
            });
        }
        let pathway_owner = js_truthy(entry.get("pathwayId"))
            || entry
                .get("ownerId")
                .and_then(Value::as_str)
                .is_some_and(|owner_id| owner_id.contains(":pathway:"));
        let mut retained_sources = parsed_sources
            .iter()
            .filter(|source| {
                source.candidate.get("type").and_then(Value::as_str) == Some("general")
            })
            .filter(|source| {
                super::requirement_source_scoring::should_keep_linked_supplemental_html_source(
                    entry,
                    &base_resolution.parsed.facts,
                    &source.entry,
                    &source.candidate,
                    &source.resolution.parsed.facts,
                    course_parser,
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let specialized_sources = parsed_sources
            .iter()
            .filter(|source| {
                source.candidate.get("type").and_then(Value::as_str) == Some("specialized")
            })
            .collect::<Vec<_>>();
        if specialized_sources.len() >= 2 {
            let minimum_support =
                2usize.max((specialized_sources.len() as f64 * 0.4).ceil() as usize);
            let mut course_support = std::collections::BTreeMap::<String, usize>::new();
            for source in &specialized_sources {
                let unique_codes = source
                    .resolution
                    .parsed
                    .facts
                    .candidate_course_codes
                    .iter()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                for course_code in unique_codes {
                    *course_support.entry(course_code).or_default() += 1;
                }
            }
            let mut shared_course_codes = unique_locale_sorted(
                course_support
                    .into_iter()
                    .filter(|(_, support)| *support >= minimum_support)
                    .map(|(course_code, _)| course_code)
                    .collect(),
            );
            if pathway_owner {
                shared_course_codes.retain(|course_code| {
                    super::requirement_source_scoring::get_course_code_numeric_level(course_code)
                        .is_some_and(|level| level < 300)
                });
            }
            if !shared_course_codes.is_empty() {
                let mut shared_facts = specialized_sources[0].resolution.parsed.facts.clone();
                shared_facts.title = None;
                shared_facts.headings.clear();
                shared_facts.candidate_course_codes = shared_course_codes;
                shared_facts.requirement_cue_lines = unique_locale_sorted(
                    specialized_sources
                        .iter()
                        .flat_map(|source| {
                            source
                                .resolution
                                .parsed
                                .facts
                                .requirement_cue_lines
                                .iter()
                                .cloned()
                        })
                        .collect(),
                )
                .into_iter()
                .take(40)
                .collect();
                shared_facts.choose_statements = unique_locale_sorted(
                    specialized_sources
                        .iter()
                        .flat_map(|source| {
                            source
                                .resolution
                                .parsed
                                .facts
                                .choose_statements
                                .iter()
                                .cloned()
                        })
                        .collect(),
                )
                .into_iter()
                .take(24)
                .collect();
                shared_facts.pathway_labels = unique_locale_sorted(
                    specialized_sources
                        .iter()
                        .flat_map(|source| {
                            source
                                .resolution
                                .parsed
                                .facts
                                .pathway_labels
                                .iter()
                                .cloned()
                        })
                        .collect(),
                )
                .into_iter()
                .take(24)
                .collect();
                let mut shared_snapshot_lines = Vec::new();
                for source in &specialized_sources {
                    let label = source
                        .entry
                        .get("label")
                        .filter(|value| !value.is_null())
                        .or_else(|| source.entry.get("url").filter(|value| !value.is_null()))
                        .map(js_string_value)
                        .unwrap_or_else(|| "Supplemental official source".to_string());
                    let label = super::requirement_shape::normalize_js_whitespace(&label);
                    if !label.is_empty() {
                        shared_snapshot_lines
                            .push(format!("[Supplemental official source] {label}"));
                    }
                    shared_snapshot_lines.extend(
                        source
                            .resolution
                            .parsed
                            .facts
                            .snapshot_lines
                            .iter()
                            .cloned(),
                    );
                }
                shared_snapshot_lines.truncate(1_200);
                shared_facts.snapshot_lines = shared_snapshot_lines;
                let parser_type = entry
                    .get("parserType")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if pathway_owner {
                    shared_facts.requirement_cue_lines.clear();
                    shared_facts.choose_statements.clear();
                    shared_facts.pathway_labels.clear();
                    shared_facts.snapshot_lines.clear();
                }
                shared_facts.parse_confidence =
                    super::requirement_parse_helpers::build_parse_confidence(
                        shared_facts.candidate_course_codes.len(),
                        shared_facts.requirement_cue_lines.len(),
                        parser_type,
                    )
                    .to_string();
                let mut shared_resolution = specialized_sources[0].resolution.clone();
                shared_resolution.parsed.facts = shared_facts;
                shared_resolution.resolution_strategy = base_resolution.resolution_strategy;
                let shared_source = ParsedLinkedHtmlSource {
                    candidate: Value::Null,
                    entry: json!({"label": "Supplemental official track and option pages"}),
                    resolution: shared_resolution,
                };
                if super::requirement_source_scoring::should_keep_linked_supplemental_html_source(
                    entry,
                    &base_resolution.parsed.facts,
                    &shared_source.entry,
                    &shared_source.candidate,
                    &shared_source.resolution.parsed.facts,
                    course_parser,
                ) {
                    retained_sources.push(shared_source);
                }
            }
        }
        if let Some(preferred) = retained_sources.iter().find(|source| {
            !source.candidate.is_null()
                && super::requirement_source_scoring::should_prefer_supplemental_html_source(
                    entry,
                    &base_resolution.parsed.facts,
                    &source.candidate,
                    &source.resolution.parsed.facts,
                    context.primary_major_titles(),
                )
        }) {
            let mut preferred_resolution = preferred.resolution.clone();
            preferred_resolution.resolution_strategy = base_resolution.resolution_strategy;
            let mut preferred_entry = entry.as_object().cloned().unwrap_or_default();
            for field in ["url", "label"] {
                if let Some(value) = preferred.entry.get(field) {
                    preferred_entry.insert(field.to_string(), value.clone());
                }
            }
            if let Some(parser_type) = preferred_entry
                .get("parserType")
                .filter(|value| !value.is_null())
            {
                let normalized_parser_type = js_string_value(parser_type)
                    .trim_matches(super::requirement_shape::is_js_whitespace)
                    .to_string();
                if !normalized_parser_type.is_empty() {
                    preferred_entry.insert(
                        "parserType".to_string(),
                        Value::String(normalized_parser_type),
                    );
                }
            }
            *preferred_source_entry = Some(Value::Object(preferred_entry));
            return (preferred_resolution, true);
        }
        let supplemental_sources = retained_sources
            .iter()
            .map(|source| SupplementalRequirementSourceResolution {
                entry: source.entry.clone(),
                resolution: source.resolution.clone(),
                kind: if source.candidate.is_null() {
                    "specialized-shared".to_string()
                } else {
                    source
                        .candidate
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("general")
                        .to_string()
                },
            })
            .collect::<Vec<_>>();
        if supplemental_sources.is_empty() {
            return (base_resolution.clone(), false);
        }
        let parser_type = entry
            .get("parserType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        (
            base_resolution.merge_supplemental_sources(&supplemental_sources, parser_type),
            false,
        )
    }

    fn collect_linked_document_sources<F>(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        base_facts: &RequirementSourceTextFacts,
        original_html_facts: &RequirementSourceTextFacts,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        mut pdf_page_blocks_for_entry: F,
    ) -> Vec<ParsedLinkedDocumentSource>
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
    {
        if select_requirement_source_adapter(entry).parser != RequirementSourceParserKind::Html {
            return Vec::new();
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        if is_pdf_source_url(url) || is_docx_source_url(url) {
            return Vec::new();
        }
        let Ok(artifacts) = self.html_parser.get_html_source_artifacts(url, timeout) else {
            return Vec::new();
        };
        let mut candidates = context.extract_supplemental_document_link_candidates(
            entry,
            &artifacts.links,
            course_parser,
        );
        if should_follow_linked_document(entry, base_facts, original_html_facts) {
            let mut seen_urls = candidates
                .iter()
                .filter_map(|candidate| candidate.get("url").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<std::collections::HashSet<_>>();
            if let Ok(base_url) = url::Url::parse(url) {
                let mut followed_count = 0usize;
                for (href, label) in &artifacts.links {
                    if !super::requirement_docx::is_linked_document_source_url(href) {
                        continue;
                    }
                    let Ok(mut resolved_url) = base_url.join(href) else {
                        continue;
                    };
                    resolved_url.set_fragment(None);
                    let resolved_url = resolved_url.to_string();
                    if resolved_url == url || !seen_urls.insert(resolved_url.clone()) {
                        continue;
                    }
                    let parser_type = get_linked_document_parser_type(&resolved_url, label);
                    candidates.push(json!({
                        "url": resolved_url,
                        "label": label,
                        "parserType": parser_type,
                        "followLinkedDocument": true,
                        "exactTitleMatch": false,
                        "titleAcronymMatch": false,
                        "sameProgramRequirementLink": false,
                        "historical": false,
                    }));
                    followed_count += 1;
                    if followed_count >= 2 {
                        break;
                    }
                }
            }
        }
        let mut parsed_sources = Vec::new();
        for candidate in candidates {
            let Some(candidate_url) = candidate.get("url").and_then(Value::as_str) else {
                continue;
            };
            if candidate_url == url {
                continue;
            }
            let label = candidate
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let parser_type = candidate
                .get("parserType")
                .and_then(Value::as_str)
                .unwrap_or_else(|| get_linked_document_parser_type(candidate_url, label));
            if !is_pdf_source_url(candidate_url) && !is_docx_source_url(candidate_url) {
                continue;
            }
            let mut candidate_fields = entry.as_object().cloned().unwrap_or_default();
            candidate_fields.remove("role");
            candidate_fields.insert("url".to_string(), json!(candidate_url));
            candidate_fields.insert("label".to_string(), json!(label));
            candidate_fields.insert("parserType".to_string(), json!(parser_type));
            let candidate_entry = Value::Object(candidate_fields);
            let page_blocks = if is_pdf_source_url(candidate_url) {
                pdf_page_blocks_for_entry(&candidate_entry)
            } else {
                None
            };
            let Ok(mut parsed) = self.collect_source_facts_with_context(
                context,
                &candidate_entry,
                timeout,
                page_blocks.as_deref(),
                course_parser,
            ) else {
                continue;
            };
            if !super::requirement_source_scoring::has_meaningful_parsed_content(&parsed.facts) {
                continue;
            }
            parsed.source_role = parsed.effective_source_role(&candidate_entry);
            let can_use_title_only_backfill = js_truthy(candidate.get("titleAcronymMatch"))
                && parsed_document_has_title_only_backfill_evidence(&parsed.facts, course_parser);
            if super::requirement_source_scoring::get_parsed_owner_alignment_score(
                &candidate_entry,
                &parsed.facts,
            ) < 2
                && !can_use_title_only_backfill
                && !js_truthy(candidate.get("followLinkedDocument"))
            {
                continue;
            }
            parsed_sources.push(ParsedLinkedDocumentSource {
                candidate,
                entry: candidate_entry,
                resolution: RequirementSourceResolution {
                    parsed,
                    resolution_strategy: "supplemental-document-source",
                    ok: true,
                    snapshot_path: None,
                    snapshot_has_heading_metadata: false,
                    used_snapshot_fallback: false,
                    snapshot_fallback_reason: None,
                    error: None,
                },
            });
        }
        parsed_sources
    }

    fn merge_linked_document_sources<F>(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        base_resolution: &RequirementSourceResolution,
        original_html_facts: &RequirementSourceTextFacts,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        followed_document_entry: &mut Option<Value>,
        pdf_page_blocks_for_entry: F,
    ) -> RequirementSourceResolution
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
    {
        let linked_sources = self.collect_linked_document_sources(
            context,
            entry,
            &base_resolution.parsed.facts,
            original_html_facts,
            timeout,
            course_parser,
            pdf_page_blocks_for_entry,
        );
        if let Some(preferred) = linked_sources.iter().find(|source| {
            super::requirement_source_scoring::should_prefer_supplemental_document_source(
                entry,
                &base_resolution.parsed.facts,
                &source.candidate,
                &source.resolution.parsed.facts,
            )
        }) {
            let supplemental = SupplementalRequirementSourceResolution {
                entry: preferred.entry.clone(),
                resolution: preferred.resolution.clone(),
                kind: "document".to_string(),
            };
            let parser_type = preferred
                .candidate
                .get("parserType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return base_resolution.merge_supplemental_sources(&[supplemental], parser_type);
        }
        let title_backfill_sources = build_title_only_course_backfill_sources(
            &base_resolution.parsed.facts,
            &linked_sources,
            course_parser,
        );
        if !title_backfill_sources.is_empty() {
            return base_resolution.merge_supplemental_sources(
                &title_backfill_sources,
                entry
                    .get("parserType")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        }
        let base_score = linked_document_follow_score(&base_resolution.parsed.facts);
        let Some(followed_document) = linked_sources.iter().find(|source| {
            js_truthy(source.candidate.get("followLinkedDocument"))
                && linked_document_follow_score(&source.resolution.parsed.facts) > base_score
        }) else {
            return base_resolution.clone();
        };
        let supplemental = SupplementalRequirementSourceResolution {
            entry: followed_document.entry.clone(),
            resolution: followed_document.resolution.clone(),
            kind: "document".to_string(),
        };
        let parser_type = followed_document
            .candidate
            .get("parserType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut merged = base_resolution.merge_supplemental_sources(&[supplemental], parser_type);
        merged.parsed.facts.headings = unique_locale_sorted(
            base_resolution
                .parsed
                .facts
                .headings
                .iter()
                .chain(followed_document.resolution.parsed.facts.headings.iter())
                .cloned()
                .collect(),
        )
        .into_iter()
        .take(20)
        .collect();
        merged.parsed.facts.requirement_cue_lines = unique_locale_sorted(
            base_resolution
                .parsed
                .facts
                .requirement_cue_lines
                .iter()
                .chain(
                    followed_document
                        .resolution
                        .parsed
                        .facts
                        .requirement_cue_lines
                        .iter(),
                )
                .cloned()
                .collect(),
        )
        .into_iter()
        .take(30)
        .collect();
        merged.parsed.facts.choose_statements = unique_locale_sorted(
            base_resolution
                .parsed
                .facts
                .choose_statements
                .iter()
                .chain(
                    followed_document
                        .resolution
                        .parsed
                        .facts
                        .choose_statements
                        .iter(),
                )
                .cloned()
                .collect(),
        )
        .into_iter()
        .take(20)
        .collect();
        merged.parsed.facts.pathway_labels = unique_locale_sorted(
            base_resolution
                .parsed
                .facts
                .pathway_labels
                .iter()
                .chain(
                    followed_document
                        .resolution
                        .parsed
                        .facts
                        .pathway_labels
                        .iter(),
                )
                .cloned()
                .collect(),
        )
        .into_iter()
        .take(20)
        .collect();
        let mut effective_entry = entry.as_object().cloned().unwrap_or_default();
        for field in ["url", "label", "parserType"] {
            if let Some(value) = followed_document.entry.get(field) {
                effective_entry.insert(field.to_string(), value.clone());
            }
        }
        let effective_entry = Value::Object(effective_entry);
        merged.parsed.adapter = select_requirement_source_adapter(&effective_entry);
        *followed_document_entry = Some(effective_entry);
        merged.parsed.facts.title = followed_document.resolution.parsed.facts.title.clone();
        merged.parsed.facts.snapshot_lines = followed_document
            .resolution
            .parsed
            .facts
            .snapshot_lines
            .clone();
        merged.parsed.facts.parse_confidence = followed_document
            .resolution
            .parsed
            .facts
            .parse_confidence
            .clone();
        merged.parsed.facts.source_section_audit = followed_document
            .resolution
            .parsed
            .facts
            .source_section_audit
            .clone();
        merged.parsed.source_role = followed_document.resolution.parsed.source_role;
        merged.used_snapshot_fallback = followed_document.resolution.used_snapshot_fallback;
        merged
    }

    pub fn collect_manifest_entry_resolution<F>(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        mut pdf_page_blocks_for_entry: F,
    ) -> (Value, RequirementSourceResolution)
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
    {
        let primary_page_blocks = pdf_page_blocks_for_entry(entry);
        match self.collect_source_facts_with_context(
            context,
            entry,
            timeout,
            primary_page_blocks.as_deref(),
            course_parser,
        ) {
            Ok(mut parsed) => {
                parsed.source_role = parsed.effective_source_role(entry);
                let primary_resolution = RequirementSourceResolution {
                    parsed,
                    resolution_strategy: "primary-source",
                    ok: true,
                    snapshot_path: None,
                    snapshot_has_heading_metadata: false,
                    used_snapshot_fallback: false,
                    snapshot_fallback_reason: None,
                    error: None,
                };
                let original_html_facts = primary_resolution.parsed.facts.clone();
                let mut preferred_html_entry = None;
                let mut followed_document_entry = None;
                let (primary_resolution, preferred_html_source) = self.merge_linked_html_sources(
                    context,
                    entry,
                    &primary_resolution,
                    timeout,
                    course_parser,
                    &mut preferred_html_entry,
                );
                let primary_resolution = if preferred_html_source {
                    primary_resolution
                } else {
                    self.merge_linked_document_sources(
                        context,
                        entry,
                        &primary_resolution,
                        &original_html_facts,
                        timeout,
                        course_parser,
                        &mut followed_document_entry,
                        &mut pdf_page_blocks_for_entry,
                    )
                };
                let alternates = self.collect_alternate_source_resolutions(
                    context,
                    entry,
                    &primary_resolution,
                    timeout,
                    course_parser,
                    &mut pdf_page_blocks_for_entry,
                );
                let (best_entry, best_resolution) = primary_resolution
                    .select_and_merge_alternate_sources_with_context(
                        entry,
                        &alternates,
                        context.primary_major_titles(),
                        context,
                        course_parser,
                    );
                let effective_entry =
                    if best_resolution.resolution_strategy != "alternate-official-source" {
                        preferred_html_entry
                            .or(followed_document_entry)
                            .unwrap_or(best_entry)
                    } else {
                        best_entry
                    };
                (effective_entry, best_resolution)
            }
            Err(primary_error) => {
                let source_scope = super::requirement_scope::build_requirement_source_scope(Some(
                    classify_requirement_source_role(entry),
                ));
                for alternate_entry in context.select_alternate_entries(entry, course_parser) {
                    let alternate_scope = super::requirement_scope::build_requirement_source_scope(
                        Some(classify_requirement_source_role(&alternate_entry)),
                    );
                    if source_scope.can_create_schedule_rows
                        && !alternate_scope.can_create_schedule_rows
                    {
                        continue;
                    }
                    let page_blocks = pdf_page_blocks_for_entry(&alternate_entry);
                    let Ok(mut parsed) = self.collect_source_facts_with_context(
                        context,
                        &alternate_entry,
                        timeout,
                        page_blocks.as_deref(),
                        course_parser,
                    ) else {
                        continue;
                    };
                    if !super::requirement_source_scoring::has_meaningful_parsed_content(
                        &parsed.facts,
                    ) {
                        continue;
                    }
                    parsed.source_role = parsed.effective_source_role(&alternate_entry);
                    return (
                        alternate_entry,
                        RequirementSourceResolution {
                            parsed,
                            resolution_strategy: "alternate-official-source",
                            ok: true,
                            snapshot_path: None,
                            snapshot_has_heading_metadata: false,
                            used_snapshot_fallback: false,
                            snapshot_fallback_reason: None,
                            error: None,
                        },
                    );
                }
                let mut snapshot_resolution = self
                    .collect_snapshot_only_source_facts_with_primary_titles(
                        entry,
                        course_parser,
                        snapshot_store,
                        context.primary_major_titles(),
                    );
                if snapshot_resolution.ok {
                    snapshot_resolution.snapshot_fallback_reason = Some(primary_error);
                } else {
                    snapshot_resolution.snapshot_fallback_reason = None;
                    snapshot_resolution.error = Some(primary_error);
                    snapshot_resolution.resolution_strategy = "primary-source";
                }
                (entry.clone(), snapshot_resolution)
            }
        }
    }

    pub fn collect_alternate_source_resolutions<F>(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        primary_resolution: &RequirementSourceResolution,
        timeout: Duration,
        course_parser: &CourseCodeParser,
        mut pdf_page_blocks_for_entry: F,
    ) -> Vec<SupplementalRequirementSourceResolution>
    where
        F: FnMut(&Value) -> Option<Vec<PdfPageBlock>>,
    {
        let alternate_entries = context.select_alternate_entries(entry, course_parser);
        let structured_course_codes = context.structured_uw_course_codes(entry, course_parser);
        let should_inspect = primary_resolution.should_evaluate_alternate_sources(entry)
            || !structured_course_codes.is_empty()
            || (!super::requirement_source_scoring::is_focused_degree_source(entry)
                && alternate_entries
                    .iter()
                    .any(super::requirement_source_scoring::is_focused_degree_source));
        if !should_inspect {
            return Vec::new();
        }
        let mut parsed_alternates = Vec::new();
        for alternate_entry in alternate_entries {
            let page_blocks = pdf_page_blocks_for_entry(&alternate_entry);
            let Ok(mut parsed) = self.collect_source_facts_with_context(
                context,
                &alternate_entry,
                timeout,
                page_blocks.as_deref(),
                course_parser,
            ) else {
                continue;
            };
            if !super::requirement_source_scoring::has_meaningful_parsed_content(&parsed.facts) {
                continue;
            }
            parsed.source_role = parsed.effective_source_role(&alternate_entry);
            parsed_alternates.push(SupplementalRequirementSourceResolution {
                entry: alternate_entry,
                resolution: RequirementSourceResolution {
                    parsed,
                    resolution_strategy: "alternate-official-source",
                    ok: true,
                    snapshot_path: None,
                    snapshot_has_heading_metadata: false,
                    used_snapshot_fallback: false,
                    snapshot_fallback_reason: None,
                    error: None,
                },
                kind: "alternate".to_string(),
            });
        }
        parsed_alternates
    }

    pub fn collect_snapshot_only_source_facts(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
    ) -> RequirementSourceResolution {
        self.collect_snapshot_only_source_facts_with_primary_titles(
            entry,
            course_parser,
            snapshot_store,
            &std::collections::HashMap::new(),
        )
    }

    pub fn collect_snapshot_only_source_facts_with_primary_titles(
        &self,
        entry: &Value,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
        primary_major_titles: &std::collections::HashMap<String, String>,
    ) -> RequirementSourceResolution {
        let owner_key = entry
            .get("ownerId")
            .map(js_string_value)
            .unwrap_or_default();
        let source_url = entry.get("url").and_then(Value::as_str);
        let adapter = select_requirement_source_adapter(entry);
        if let Some(snapshot) = snapshot_store.read_snapshot(&owner_key, source_url) {
            let mut parsed = RequirementSourcePipelineResult {
                adapter,
                source_role: classify_requirement_source_role(entry),
                facts: collect_snapshot_source_facts_with_primary_titles(
                    entry,
                    &snapshot,
                    course_parser,
                    primary_major_titles,
                ),
            };
            parsed.source_role = parsed.effective_source_role(entry);
            RequirementSourceResolution {
                parsed,
                resolution_strategy: "cached-snapshot",
                ok: true,
                snapshot_path: Some(snapshot.snapshot_path),
                snapshot_has_heading_metadata: snapshot.has_heading_metadata,
                used_snapshot_fallback: true,
                snapshot_fallback_reason: Some("snapshot-only mode".to_string()),
                error: None,
            }
        } else {
            let mut facts = RequirementSourceTextFacts::default();
            facts.parse_confidence = "low".to_string();
            let mut parsed = RequirementSourcePipelineResult {
                adapter,
                source_role: classify_requirement_source_role(entry),
                facts,
            };
            parsed.source_role = parsed.effective_source_role(entry);
            RequirementSourceResolution {
                parsed,
                resolution_strategy: "cached-snapshot",
                ok: false,
                snapshot_path: None,
                snapshot_has_heading_metadata: false,
                used_snapshot_fallback: false,
                snapshot_fallback_reason: Some(
                    "snapshot-only mode but no cached snapshot was available".to_string(),
                ),
                error: Some(
                    "Snapshot-only mode could not find a cached source snapshot.".to_string(),
                ),
            }
        }
    }

    pub fn collect_source_facts_with_snapshot(
        &self,
        entry: &Value,
        timeout: Duration,
        pdf_page_blocks: Option<&[PdfPageBlock]>,
        course_parser: &CourseCodeParser,
        snapshot_store: &RequirementSnapshotStore,
    ) -> RequirementSourceResolution {
        match self.collect_source_facts(entry, timeout, pdf_page_blocks, course_parser) {
            Ok(mut parsed) => {
                parsed.source_role = parsed.effective_source_role(entry);
                RequirementSourceResolution {
                    parsed,
                    resolution_strategy: "primary-source",
                    ok: true,
                    snapshot_path: None,
                    snapshot_has_heading_metadata: false,
                    used_snapshot_fallback: false,
                    snapshot_fallback_reason: None,
                    error: None,
                }
            }
            Err(primary_error) => {
                let owner_key = entry
                    .get("ownerId")
                    .map(js_string_value)
                    .unwrap_or_default();
                let source_url = entry.get("url").and_then(Value::as_str);
                let snapshot = snapshot_store.read_snapshot(&owner_key, source_url);
                if let Some(snapshot) = snapshot {
                    let mut parsed = RequirementSourcePipelineResult {
                        adapter: select_requirement_source_adapter(entry),
                        source_role: classify_requirement_source_role(entry),
                        facts: collect_snapshot_source_facts_with_primary_titles(
                            entry,
                            &snapshot,
                            course_parser,
                            &std::collections::HashMap::new(),
                        ),
                    };
                    parsed.source_role = parsed.effective_source_role(entry);
                    RequirementSourceResolution {
                        parsed,
                        resolution_strategy: "cached-snapshot",
                        ok: true,
                        snapshot_path: Some(snapshot.snapshot_path),
                        snapshot_has_heading_metadata: snapshot.has_heading_metadata,
                        used_snapshot_fallback: true,
                        snapshot_fallback_reason: Some(primary_error),
                        error: None,
                    }
                } else {
                    let mut facts = RequirementSourceTextFacts::default();
                    facts.parse_confidence = "low".to_string();
                    let mut parsed = RequirementSourcePipelineResult {
                        adapter: select_requirement_source_adapter(entry),
                        source_role: classify_requirement_source_role(entry),
                        facts,
                    };
                    parsed.source_role = parsed.effective_source_role(entry);
                    RequirementSourceResolution {
                        parsed,
                        resolution_strategy: "primary-source",
                        ok: false,
                        snapshot_path: None,
                        snapshot_has_heading_metadata: false,
                        used_snapshot_fallback: false,
                        snapshot_fallback_reason: None,
                        error: Some(primary_error),
                    }
                }
            }
        }
    }

    pub fn collect_source_facts(
        &self,
        entry: &Value,
        timeout: Duration,
        pdf_page_blocks: Option<&[PdfPageBlock]>,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementSourcePipelineResult, String> {
        self.collect_source_facts_internal(None, entry, timeout, pdf_page_blocks, course_parser)
    }

    pub fn collect_source_facts_with_context(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        pdf_page_blocks: Option<&[PdfPageBlock]>,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementSourcePipelineResult, String> {
        self.collect_source_facts_internal(
            Some(context),
            entry,
            timeout,
            pdf_page_blocks,
            course_parser,
        )
    }

    fn collect_source_facts_internal(
        &self,
        context: Option<&RequirementSourceManifestContext>,
        entry: &Value,
        timeout: Duration,
        pdf_page_blocks: Option<&[PdfPageBlock]>,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementSourcePipelineResult, String> {
        let adapter = select_requirement_source_adapter(entry);
        let mut source_role = classify_requirement_source_role(entry);
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let parser_type = entry.get("parserType").and_then(Value::as_str);
        let empty_primary_major_titles = std::collections::HashMap::new();
        let primary_major_titles = context
            .map(|context| context.primary_major_titles())
            .unwrap_or(&empty_primary_major_titles);
        let facts = if is_pdf_source_url(url) || adapter.parser == RequirementSourceParserKind::Pdf
        {
            let extracted_page_blocks = if pdf_page_blocks.is_none() {
                Some(
                    self.pdf_parser
                        .as_ref()
                        .ok_or_else(|| {
                            format!("PDF source facts require extracted page blocks for {}", url)
                        })?
                        .get_pdf_page_blocks(url, timeout, course_parser)?,
                )
            } else {
                None
            };
            let page_blocks = pdf_page_blocks
                .or_else(|| extracted_page_blocks.as_deref())
                .ok_or_else(|| {
                    format!("PDF source facts require extracted page blocks for {}", url)
                })?;
            collect_pdf_source_facts_with_primary_titles(
                entry,
                page_blocks,
                course_parser,
                primary_major_titles,
            )
        } else if is_docx_source_url(url) || adapter.parser == RequirementSourceParserKind::Docx {
            let artifacts = self.docx_parser.get_docx_source_artifacts(url, timeout)?;
            let label = entry
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let docx_parser_type = get_linked_document_parser_type(url, label);
            let mut docx_entry = entry.clone();
            if let Some(fields) = docx_entry.as_object_mut() {
                fields.insert("parserType".to_string(), json!(docx_parser_type));
            }
            source_role = classify_requirement_source_role(&docx_entry);
            collect_docx_source_facts_for_entry_with_primary_titles(
                &docx_entry,
                &artifacts,
                docx_parser_type,
                course_parser,
                primary_major_titles,
            )
        } else {
            let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
            if let Some(context) = context {
                let plan_id = entry
                    .get("planId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                collect_html_source_facts_for_entry_with_context(
                    entry,
                    &artifacts,
                    parser_type.unwrap_or_default(),
                    course_parser,
                    context.primary_major_titles(),
                    context.bootstrap_plan_index().pathway_count(plan_id) > 0,
                )
            } else {
                collect_html_source_facts_for_entry(
                    entry,
                    &artifacts,
                    parser_type.unwrap_or_default(),
                    course_parser,
                )
            }
        };
        Ok(RequirementSourcePipelineResult {
            adapter,
            source_role,
            facts,
        })
    }
}

fn build_manifest_parse_base_result(
    context: &RequirementSourceManifestContext,
    entry: &Value,
    course_parser: &CourseCodeParser,
) -> Value {
    let mut base = serde_json::Map::new();
    for key in ["ownerId", "ownerTitle", "planId", "campusId"] {
        if let Some(value) = entry.get(key) {
            base.insert(key.to_string(), value.clone());
        }
    }
    if let Some(pathway_id) = context.materializable_pathway_id(entry) {
        base.insert("pathwayId".to_string(), json!(pathway_id));
    }
    for (output_key, entry_key) in [
        ("primaryParserType", "parserType"),
        ("primarySourceUrl", "url"),
        ("primarySourceLabel", "label"),
    ] {
        if let Some(value) = entry.get(entry_key) {
            base.insert(output_key.to_string(), value.clone());
        }
    }
    base.insert(
        "isPrimaryDegreeRequirementsLink".to_string(),
        json!(js_truthy(entry.get("isPrimaryDegreeRequirementsLink"))),
    );
    base.insert(
        "structuredUwCourseCodes".to_string(),
        json!(context.structured_uw_course_codes(entry, course_parser)),
    );
    Value::Object(base)
}

fn is_track_catalog_supplemental_candidate(candidate: &Value) -> bool {
    let label = candidate
        .get("label")
        .map(js_string_value)
        .unwrap_or_default();
    let url = candidate
        .get("url")
        .map(js_string_value)
        .unwrap_or_default();
    track_catalog_supplemental_pattern().is_match(&format!("{label} {url}"))
}

fn track_catalog_supplemental_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:courses? by track|courses-track|list of geography courses by track|suggested course pathways?)\b",
        );
        regex::Regex::new(&pattern).expect("track catalog supplemental candidate")
    })
}

fn should_follow_linked_document(
    entry: &Value,
    merged_facts: &RequirementSourceTextFacts,
    original_html_facts: &RequirementSourceTextFacts,
) -> bool {
    if !merged_facts.candidate_course_codes.is_empty() {
        return false;
    }
    let snapshot_text = super::requirement_shape::normalize_js_whitespace(
        &original_html_facts.snapshot_lines.join(" "),
    )
    .to_lowercase();
    let source_url = entry
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    linked_document_follow_cue_pattern().is_match(&snapshot_text) || source_url.contains("/file/")
}

fn linked_document_follow_score(facts: &RequirementSourceTextFacts) -> i64 {
    facts.candidate_course_codes.len() as i64 * 100
        + facts.requirement_cue_lines.len() as i64 * 10
        + facts.choose_statements.len() as i64 * 5
}

fn linked_document_follow_cue_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b(?:download|checklist|worksheet|type:\s*(?:pdf|docx?))\b",
        );
        regex::Regex::new(&pattern).expect("linked document follow cue")
    })
}

fn calculus_title_ordinals() -> [(u8, &'static str); 4] {
    [(1, "i"), (2, "ii"), (3, "iii"), (4, "iv")]
}

fn calculus_ordinal_pattern(value: u8, roman: &str) -> regex::Regex {
    let source = format!(r"(?i)\bcalculus\s*(?:{value}|{roman})\b");
    regex::Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
        &source,
    ))
    .expect("calculus ordinal pattern")
}

fn base_facts_mention_title_only_calculus_ordinal(
    facts: &RequirementSourceTextFacts,
    value: u8,
    roman: &str,
) -> bool {
    let text = super::requirement_shape::normalize_js_whitespace(
        &facts
            .snapshot_lines
            .iter()
            .chain(facts.requirement_cue_lines.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    );
    !text.is_empty() && calculus_ordinal_pattern(value, roman).is_match(&text)
}

fn document_line_contains_course_code(
    line: &str,
    course_code: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized = super::requirement_course_codes::normalize_course_code(course_code);
    !normalized.is_empty()
        && course_parser.extract_from_line(line).iter().any(|found| {
            super::requirement_course_codes::normalize_course_code(found) == normalized
        })
}

fn line_has_course_code_with_calculus_ordinal(
    line: &str,
    course_code: &str,
    value: u8,
    roman: &str,
) -> bool {
    let normalized_line = super::requirement_shape::normalize_js_whitespace(line);
    let normalized_course_code =
        super::requirement_course_codes::normalize_course_code(course_code);
    if normalized_line.is_empty() || normalized_course_code.is_empty() {
        return false;
    }
    let escaped_code = regex::escape(&normalized_course_code)
        .replace(r"\ ", r"\s+")
        .replace(' ', r"\s+");
    let course_pattern = format!(r"(?i)\b{escaped_code}\b");
    let Ok(course_pattern) = regex::Regex::new(
        &super::requirement_shape::js_regex_compatibility_pattern(&course_pattern),
    ) else {
        return false;
    };
    let Some(match_result) = course_pattern.find(&normalized_line) else {
        return false;
    };
    let mut after_course_code_utf16_units = 0usize;
    let after_course_code = normalized_line
        .get(match_result.end()..)
        .unwrap_or_default()
        .chars()
        .take_while(|character| {
            let character_units = character.len_utf16();
            if after_course_code_utf16_units + character_units > 80 {
                return false;
            }
            after_course_code_utf16_units += character_units;
            true
        })
        .collect::<String>();
    let lowered = after_course_code.trim_start().to_ascii_lowercase();
    if lowered.starts_with("precalculus")
        || lowered.starts_with("pre-calculus")
        || lowered.starts_with("pre calculus")
    {
        return false;
    }
    let pattern = format!(r"(?i)^\s*calculus\s*(?:{value}|{roman})\b");
    regex::Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
        &pattern,
    ))
    .is_ok_and(|pattern| pattern.is_match(&after_course_code))
}

fn line_mentions_calculus_ordinal_title(line: &str, value: u8, roman: &str) -> bool {
    let normalized = super::requirement_shape::normalize_js_whitespace(line);
    let precalculus_pattern =
        regex::Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bpre[-\s]?calculus\s*(?:[1-4]|i|ii|iii|iv)\b",
        ))
        .expect("precalculus ordinal title");
    let without_precalculus = precalculus_pattern.replace_all(&normalized, " ");
    calculus_ordinal_pattern(value, roman).is_match(&without_precalculus)
}

fn course_code_looks_like_calculus_ordinal(course_code: &str, value: u8) -> bool {
    let expected = match value {
        1 => Some("124"),
        2 => Some("125"),
        3 => Some("126"),
        _ => None,
    };
    let Some(expected) = expected else {
        return false;
    };
    let normalized = super::requirement_course_codes::normalize_course_code(course_code);
    calculus_course_number_pattern()
        .captures(&normalized)
        .and_then(|captures| captures.get(1))
        .is_some_and(|number| number.as_str() == expected)
}

fn calculus_course_number_pattern() -> &'static regex::Regex {
    static VALUE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        let pattern =
            super::requirement_shape::js_regex_compatibility_pattern(r"\b([0-9]{3})[A-Z]?$");
        regex::Regex::new(&pattern).expect("calculus course number")
    })
}

fn find_document_line_for_course_code_with_calculus_ordinal(
    snapshot_lines: &[String],
    course_code: &str,
    value: u8,
    roman: &str,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    for (index, line) in snapshot_lines.iter().enumerate() {
        if !document_line_contains_course_code(line, course_code, course_parser) {
            continue;
        }
        if line_has_course_code_with_calculus_ordinal(line, course_code, value, roman) {
            return Some(line.clone());
        }
        if !course_code_looks_like_calculus_ordinal(course_code, value) {
            continue;
        }
        let nearby_start = index.saturating_sub(6);
        let nearby_end = snapshot_lines.len().min(index.saturating_add(7));
        if snapshot_lines[nearby_start..nearby_end]
            .iter()
            .any(|candidate| line_mentions_calculus_ordinal_title(candidate, value, roman))
        {
            return Some(format!(
                "{} Calculus {}",
                super::requirement_course_codes::normalize_course_code(course_code),
                roman.to_ascii_uppercase()
            ));
        }
    }
    None
}

fn parsed_document_has_title_only_backfill_evidence(
    facts: &RequirementSourceTextFacts,
    course_parser: &CourseCodeParser,
) -> bool {
    let snapshot_course_codes = unique_locale_sorted(
        facts
            .snapshot_lines
            .iter()
            .flat_map(|line| course_parser.extract_from_line(line))
            .collect(),
    );
    calculus_title_ordinals().into_iter().any(|(value, roman)| {
        snapshot_course_codes.iter().any(|course_code| {
            find_document_line_for_course_code_with_calculus_ordinal(
                &facts.snapshot_lines,
                course_code,
                value,
                roman,
                course_parser,
            )
            .is_some()
        })
    })
}

fn build_title_only_course_backfill_sources(
    base_facts: &RequirementSourceTextFacts,
    linked_sources: &[ParsedLinkedDocumentSource],
    course_parser: &CourseCodeParser,
) -> Vec<SupplementalRequirementSourceResolution> {
    let needed_ordinals = calculus_title_ordinals()
        .into_iter()
        .filter(|(value, roman)| {
            base_facts_mention_title_only_calculus_ordinal(base_facts, *value, roman)
        })
        .collect::<Vec<_>>();
    if needed_ordinals.is_empty() {
        return Vec::new();
    }
    let mut existing_course_codes = base_facts
        .candidate_course_codes
        .iter()
        .map(|code| super::requirement_course_codes::normalize_course_code(code))
        .filter(|code| !code.is_empty())
        .collect::<std::collections::HashSet<_>>();
    let mut recovered_sources = Vec::new();
    for supplemental in linked_sources {
        let candidate = &supplemental.candidate;
        if js_truthy(candidate.get("historical"))
            || ![
                "exactTitleMatch",
                "titleAcronymMatch",
                "sameProgramRequirementLink",
            ]
            .into_iter()
            .any(|field| js_truthy(candidate.get(field)))
        {
            continue;
        }
        let snapshot_lines = &supplemental.resolution.parsed.facts.snapshot_lines;
        let mut recovered_course_codes = Vec::new();
        let mut recovered_snapshot_lines = Vec::new();
        let candidate_course_codes = unique_locale_sorted(
            supplemental
                .resolution
                .parsed
                .facts
                .candidate_course_codes
                .iter()
                .cloned()
                .chain(
                    snapshot_lines
                        .iter()
                        .flat_map(|line| course_parser.extract_from_line(line)),
                )
                .collect(),
        );
        for course_code in candidate_course_codes {
            let normalized_course_code =
                super::requirement_course_codes::normalize_course_code(&course_code);
            if normalized_course_code.is_empty()
                || existing_course_codes.contains(&normalized_course_code)
            {
                continue;
            }
            let matching_line = needed_ordinals.iter().find_map(|(value, roman)| {
                find_document_line_for_course_code_with_calculus_ordinal(
                    snapshot_lines,
                    &normalized_course_code,
                    *value,
                    roman,
                    course_parser,
                )
            });
            if let Some(matching_line) = matching_line {
                recovered_course_codes.push(normalized_course_code.clone());
                recovered_snapshot_lines.push(matching_line);
                existing_course_codes.insert(normalized_course_code);
            }
        }
        if recovered_course_codes.is_empty() {
            continue;
        }
        let recovered_snapshot_line_count = recovered_snapshot_lines.len();
        let recovered_snapshot_lines = unique_locale_sorted(recovered_snapshot_lines);
        let recovered_course_codes = unique_locale_sorted(recovered_course_codes);
        let mut source = supplemental.clone();
        let facts = &mut source.resolution.parsed.facts;
        facts.headings.clear();
        facts.requirement_cue_lines = std::iter::once("Prerequisites".to_string())
            .chain(recovered_snapshot_lines.iter().cloned())
            .collect();
        facts.choose_statements.clear();
        facts.pathway_labels.clear();
        facts.candidate_course_codes = recovered_course_codes.clone();
        facts.snapshot_lines = std::iter::once("Prerequisites".to_string())
            .chain(recovered_snapshot_lines.iter().cloned())
            .collect();
        let parser_type = candidate
            .get("parserType")
            .and_then(Value::as_str)
            .or_else(|| supplemental.entry.get("parserType").and_then(Value::as_str))
            .unwrap_or("pdf-degree-sheet");
        facts.parse_confidence = super::requirement_parse_helpers::build_parse_confidence(
            recovered_course_codes.len(),
            recovered_snapshot_line_count,
            parser_type,
        )
        .to_string();
        recovered_sources.push(SupplementalRequirementSourceResolution {
            entry: source.entry,
            resolution: source.resolution,
            kind: "document".to_string(),
        });
    }
    recovered_sources
}

fn js_interpolate_option(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(value) => js_string_value(value),
    }
}

fn js_string_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(js_string_value)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
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
