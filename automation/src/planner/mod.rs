mod deadlines;
mod equivalency_catalog;
mod equivalency_derived;
mod equivalency_output;
mod equivalency_report;
mod equivalency_rows;
mod grc_catalog;
mod grc_public_materials;
mod owner_source_links;
mod planner_reporting;
mod publish_readiness;
mod refresh_orchestration;
mod requirement_adapters;
mod requirement_approved_filter_output;
mod requirement_atoms;
mod requirement_bootstrap_index;
mod requirement_category_options;
mod requirement_choice_lists;
mod requirement_course_codes;
mod requirement_course_scoping;
mod requirement_degree_map_blocks;
mod requirement_document_facts;
mod requirement_docx;
mod requirement_groups;
mod requirement_hint_safety;
mod requirement_html;
mod requirement_html_source;
mod requirement_ids;
mod requirement_manifest;
mod requirement_manifest_context;
mod requirement_manifest_fallback;
mod requirement_manifest_selection;
mod requirement_parse_helpers;
mod requirement_parse_markdown;
mod requirement_parse_reports;
mod requirement_pathway_labels;
mod requirement_pdf_extraction;
mod requirement_pdf_layout;
mod requirement_plan_matching;
mod requirement_recovery_links;
mod requirement_recovery_scope;
mod requirement_reports;
mod requirement_role_classifier;
mod requirement_rule_utils;
mod requirement_scope;
mod requirement_section_roles;
mod requirement_shape;
mod requirement_snapshots;
mod requirement_source_audits;
mod requirement_source_fetching;
mod requirement_source_output;
mod requirement_source_pipeline;
mod requirement_source_quality;
mod requirement_source_recovery;
mod requirement_source_scoring;
mod sequence_text;
mod source_artifact_cache;
mod source_backed_coverage;
mod source_backed_coverage_command;
mod source_fingerprints;
mod source_pipeline_structural_patterns;
mod source_pipeline_trigger_fixtures;
mod source_pipeline_validation;
mod spot_check_major;
mod transfer_plan_title_matching;
mod uw_catalog;

pub use deadlines::{DeadlineRefreshReport, refresh as refresh_deadlines};
pub use equivalency_catalog::render_transfer_equivalency_catalog_typescript;
pub use equivalency_derived::{DerivedRuleSynthesisInput, synthesize_equivalency_rules};
pub use equivalency_output::{EquivalencyRulesOutputInput, render_equivalency_guide_typescript};
pub use equivalency_report::{
    EquivalencyGuideReportInput, build_equivalency_guide_report,
    render_equivalency_guide_report_markdown,
};
pub use equivalency_rows::{
    EquivalencyGuideParseInput, EquivalencyGuideRow, parse_equivalency_guide_rows,
    parse_equivalency_guide_rows_with_titles,
};
pub use grc_catalog::{
    GrcCourseCatalogInput, GrcCourseCatalogSection, GrcCourseDetails, GrcCourseDetailsInput,
    parse_grc_course_catalog, parse_grc_course_details,
};
pub use owner_source_links::run_probe as run_owner_source_link_probe;
pub use planner_reporting::{
    normalize_markdown_report, serialize_json_report, serialize_markdown_lines, write_json_report,
    write_markdown_lines_report, write_markdown_report, write_report_pair,
};
pub use publish_readiness::{
    build_course_source_coverage, build_release_readiness, build_requirement_source_fidelity,
    build_student_visible_facts, collect_source_backed_blockers, is_student_visible,
    run_publish_readiness,
};
pub use refresh_orchestration::run_planner_refresh;
pub use requirement_adapters::{
    RequirementSourceAdapter, RequirementSourceParserKind, select_requirement_source_adapter,
};
pub use requirement_approved_filter_output::{
    build_generated_program_approved_course_filters,
    build_generated_program_approved_course_filters_typescript,
    render_generated_program_approved_course_filters,
};
pub use requirement_atoms::{
    ParsedRequirementPhase, build_parsed_requirement_atom_candidates, build_source_line_hint,
    get_source_line_hints, infer_parsed_requirement_phase_from_hints,
};
pub use requirement_bootstrap_index::RequirementBootstrapPlanIndex;
pub use requirement_category_options::{
    build_category_requirement_option, build_category_requirement_options_from_choice_line,
    count_requirement_category_option_descriptors, get_requirement_category_option_descriptor,
    get_requirement_category_option_descriptors,
};
pub use requirement_choice_lists::{
    InlineLabeledRequirementParts, build_choice_list_source_line,
    build_forward_labeled_choice_requirement_lines, build_multi_line_choice_requirement_lines,
    can_continue_wrapped_course_number, choice_list_continuation_looks_like_adjacent_requirement,
    choice_list_looks_closed, find_nearby_requirement_choice_label,
    find_nearest_lettered_choose_one_heading, get_inline_labeled_requirement_parts,
    has_choice_requirement_context, has_strong_choice_requirement_context,
    is_combined_choose_one_heading_with_lettered_option, is_lettered_option_line,
    is_lettered_option_line_under_choose_one_heading, is_numbered_choose_one_option_heading,
    is_standalone_credit_amount_line, line_starts_parenthetical_choice_list,
    looks_like_choice_list_continuation, normalize_sequence_source_line, strip_choice_list_line,
    strip_inline_labeled_requirement_marker, strip_leading_lettered_option_marker,
    strip_non_option_course_evidence_from_choice_line, trim_choice_list_continuation_noise,
};
pub use requirement_course_codes::{
    CourseCodeParser as RequirementCourseCodeParser,
    CourseCreditRange as RequirementCourseCreditRange, get_course_level_summary,
    get_explicitly_excluded_course_codes_from_lines, get_parsed_course_level,
};
pub use requirement_course_scoping::{
    filter_graduate_cross_listed_companions_for_undergraduate_source,
    is_graduate_cross_listed_companion_for_undergraduate_source,
    manifest_entry_can_use_graduate_course_codes,
};
pub use requirement_degree_map_blocks::{
    build_parsed_degree_map_block_candidates, get_structured_uw_course_codes,
    structured_degree_map_block_matches_manifest_entry,
};
pub use requirement_document_facts::{
    RequirementSourceTextFacts, collect_docx_source_facts, collect_html_source_facts,
    collect_html_source_facts_for_entry_with_context, collect_pdf_source_facts,
};
pub use requirement_docx::{
    DocxSourceArtifacts, DocxSourceParser, build_docx_lines, extract_docx_document_xml,
    extract_docx_text, get_linked_document_parser_type, is_docx_source_url,
    is_linked_document_source_url, is_pdf_source_url,
};
pub use requirement_groups::{
    append_sbse_comparison_table_scope_lines, apply_sequence_lab_notes,
    build_alternating_number_sequence_choice_candidate,
    build_alternating_number_sequence_choice_requirement_groups,
    build_ampersand_or_single_sequence_choice_candidate,
    build_ampersand_or_single_sequence_choice_requirement_groups,
    build_category_credit_bucket_option, build_category_requirement_options_from_credit_bucket,
    build_columnar_sequence_choice_candidate, build_columnar_sequence_choice_requirement_groups,
    build_course_requirement_options_from_credit_bucket, build_credit_bucket_source_line,
    build_forward_labeled_choice_requirement_groups, build_generic_choice_requirement_groups,
    build_generic_credit_bucket_requirement_groups,
    build_generic_sequence_choice_requirement_groups, build_inline_course_requirement_options,
    build_inline_labeled_requirement_groups_for_parts,
    build_inline_minimum_course_choice_requirement_group,
    build_internal_choice_split_sectioned_course_requirement_groups,
    build_lettered_choose_one_requirement_groups, build_mixed_inline_required_course_list_groups,
    build_multiline_sequence_choice_candidate, build_numbered_inline_all_required_course_group,
    build_parallel_choose_one_requirement_groups, build_parenthetical_sequence_choice_candidates,
    build_parenthetical_sequence_choice_requirement_groups, build_parsed_requirement_course,
    build_parsed_requirement_courses_from_groups, build_parsed_requirement_group,
    build_parsed_requirement_option, build_parser_rule_requirement_replacements,
    build_pathway_section_fallback_concentration_group, build_repeated_required_alternative_groups,
    build_sbse_comparison_table_requirement_groups, build_sectioned_option_course_option,
    build_sectioned_option_requirement_groups, build_sequence_choice_requirement_group,
    build_source_derived_requirement_courses, build_source_derived_requirement_replacements,
    build_source_derived_sectioned_course_requirement_groups,
    build_split_either_or_requirement_groups, build_standalone_course_choice_requirement_groups,
    choose_preferred_requirement_group, coalesce_sectioned_credit_buckets_with_inline_choices,
    collect_immediate_section_course_lines, collect_sectioned_option_course_options,
    collect_sequence_lab_notes, count_concrete_course_options,
    dedupe_credit_bucket_requirement_groups, detect_program_elective_credit_bucket,
    detect_subject_prefix_credit_bucket, extract_course_codes_from_credit_bucket_text,
    extract_inline_required_list_alternative_sets, extract_option_course_codes_from_sectioned_line,
    extract_parsed_requirement_group_course_codes, extract_subject_prefix_from_credit_bucket_text,
    filter_generic_bse_supplemental_groups_for_sbse,
    filter_requirement_groups_by_supplemental_pathway_scope, finalize_parsed_requirement_groups,
    find_nearest_supplemental_source_marker, find_snapshot_line_index_for_requirement_group,
    get_all_required_requirement_group_course_set_key,
    get_approved_list_key_from_credit_bucket_text, get_choice_requirement_group_course_set,
    get_choice_requirement_group_course_set_key, get_choice_requirement_group_label,
    get_credit_bucket_dedupe_key, get_expected_sequence_subjects_from_cue,
    get_fallback_sectioned_elective_heading_descriptor, get_mixed_required_course_list_parts,
    get_next_meaningful_section_line, get_parallel_choose_one_heading_label,
    get_parallel_choose_one_heading_subjects, get_positive_credit_bucket_category_text,
    get_program_elective_bucket_label, get_program_specific_credit_bucket_descriptor,
    get_repeated_required_alternative_sets, get_requirement_group_option_course_set_key,
    get_requirement_option_course_code_set, get_sbse_comparison_table_scope_lines,
    get_sectioned_course_heading_descriptor, get_sectioned_course_list_end_index,
    get_sequence_choice_cue, get_sequence_fallback_subject_from_context,
    get_sequence_path_label_from_text, get_simple_sequence_fallback_subject_from_context,
    get_source_choice_requirement_group_scope_key, get_trimmed_sectioned_course_group_heading,
    has_course_question_support_cue, has_credit_bucket_cue,
    has_following_specific_humanities_social_minimums,
    has_immediate_sectioned_course_rows_after_heading, has_non_requirement_course_exclusion_cue,
    heading_subjects_match_course_codes,
    is_approved_list_placeholder_without_immediate_course_list, is_equivalent_number_alias_line,
    is_generic_category_credit_bucket_heading, is_generic_university_category_distribution_heading,
    is_non_requirement_sectioned_course_heading, is_numbered_option_row_under_choose_one_heading,
    is_proper_course_set_subset, is_requirement_group_under_support_course_list,
    is_sectioned_course_list_boundary, is_sequence_choice_boundary_line,
    is_source_choice_requirement_group, line_starts_with_numbered_parallel_choice,
    looks_like_admission_decision_narrative_credit_bucket,
    looks_like_aggregate_general_education_credit_summary,
    looks_like_broad_requirement_container_heading, looks_like_composite_credit_heading,
    looks_like_credit_summary_for_named_requirement_section,
    looks_like_degree_total_general_elective_prose, looks_like_elective_timing_or_sample_plan_note,
    looks_like_enumerated_section_heading, looks_like_merged_pdf_choice_fragment_group,
    looks_like_shared_humanities_social_credit_heading,
    looks_like_support_only_research_opportunity_line,
    looks_like_title_and_credit_line_for_previous_course,
    next_line_looks_like_course_list_for_heading, normalize_pdf_requirement_heading_line,
    normalize_sectioned_course_group_label, parse_leading_section_heading_credit_range,
    parse_section_heading_credit_range, parse_sectioned_course_credit_list_line,
    parse_sectioned_course_required_count, requirement_group_has_program_specific_category_option,
    sequence_row_looks_like_single_path, sequence_row_subjects_fit_cue,
    should_skip_credit_bucket_line, should_skip_raw_partial_choice_line,
    split_credit_bucket_source_lines, split_sequence_alternative_labels,
    starts_with_category_credit_cue, suppress_choice_groups_covered_by_credit_buckets,
    suppress_generic_choice_groups_covered_by_inline_choices,
    suppress_inline_required_groups_covered_by_choice_options,
    suppress_owner_specific_requirement_group_leaks,
    suppress_repeated_required_alternative_groups_covered_by_sequence_choices,
    suppress_subset_source_choice_groups,
};
pub use requirement_hint_safety::{
    collect_requirement_source_course_code_candidates,
    extract_safe_direct_course_line_codes_from_lines,
    extract_safe_known_subject_course_cluster_codes_from_lines,
    extract_subject_scoped_bare_checklist_codes_from_lines, filter_parsed_course_codes_by_hints,
    filter_parsed_course_codes_by_hints_with_source_recovery, is_bare_course_code_source_hint,
    is_direct_course_line_recovery_source, is_unsafe_requirement_course_hint,
    recover_structured_course_codes_from_source_evidence,
};
pub use requirement_html::{
    CatalogAnchorMatch, CatalogPeerBoundary, CatalogProgramExpandableGroupMatch,
    RequirementHtmlArtifacts, build_html_lines as build_requirement_html_lines,
    collect_ignored_catalog_neighboring_sections as collect_requirement_ignored_catalog_sections,
    decode_transfer_planner_html_entities, extract_headings as extract_requirement_html_headings,
    extract_title as extract_requirement_html_title,
    find_catalog_anchor_tag as find_requirement_catalog_anchor_tag,
    find_catalog_credential_heading_matches as find_requirement_catalog_credential_heading_matches,
    find_catalog_program_expandable_group_matches as find_requirement_catalog_program_groups,
    find_next_catalog_credential_boundary as find_next_requirement_catalog_credential_boundary,
    find_next_catalog_peer_boundary as find_next_requirement_catalog_peer_boundary,
    normalize_catalog_base_credential_heading as normalize_requirement_catalog_base_credential_heading,
    parse_html_artifacts as parse_requirement_html_artifacts,
};
pub use requirement_html_source::HtmlSourceParser;
pub use requirement_ids::{
    build_requirement_source_block_id, build_unique_derived_id, slugify_requirement_identifier,
    stable_source_id_hash,
};
pub use requirement_manifest::{
    dedupe_parseable_primary_entries, exclude_unhashed_primary_manifest_duplicates,
    normalize_manifest_url_for_dedupe, normalize_manifest_url_without_hash_for_dedupe,
    parseable_manifest_entry_dedupe_key, preselect_parseable_manifest_entries,
    should_parse_requirement_source_entry,
};
pub use requirement_manifest_context::RequirementSourceManifestContext;
pub use requirement_manifest_fallback::{
    build_parser_recovery_candidate_entry, compare_manifest_fallback_candidates,
    get_parser_recovery_candidate_parser_type, select_alternate_parseable_manifest_entries,
    should_allow_alternate_manifest_entry,
};
pub use requirement_manifest_selection::select_parseable_manifest_entries;
pub use requirement_parse_helpers::{
    build_parse_confidence, extract_choose_statements, extract_relevant_requirement_lines,
    extract_requirement_cue_lines, get_inline_minimum_course_list_text, is_noisy_line,
    parse_inline_minimum_course_count, parse_requirement_course_count,
    parse_requirement_credit_amount, parse_requirement_credit_range,
};
pub use requirement_parse_markdown::{
    build_requirement_source_parse_markdown_lines, build_requirement_source_parse_markdown_report,
};
pub use requirement_parse_reports::build_parse_report;
pub use requirement_pathway_labels::extract_pathway_labels;
pub use requirement_pdf_extraction::PdfSourceParser;
pub use requirement_pdf_layout::{
    PdfLineSegment, PdfPageBlock, PdfTextContent, PdfTextItem, build_pdf_page_line_texts,
    get_pdf_title_scope_tokens, order_pdf_line_segments, scope_pdf_page_blocks,
    scope_pdf_page_lines,
};
pub use requirement_plan_matching::{
    build_primary_major_titles_by_plan_id, filter_manifest_entries_for_specialized_plan_title,
    manifest_entry_matches_specialized_plan_title, manifest_entry_mentions_different_major,
    parser_recovery_candidate_conflicts_with_owner_identity,
};
pub use requirement_recovery_links::{
    ParserRecoveryLinkSignals, SupplementalDocumentLinkSignals, extract_html_anchor_candidates,
    extract_supplemental_html_link_candidates, has_primary_requirement_link_cue,
    has_recovery_link_signal, is_graduate_supplemental_source, is_ignored_parser_recovery_href,
    is_noisy_parser_recovery_link, is_official_linked_asset_url,
    normalize_requirement_matcher_text, parser_recovery_title_scope_tokens,
    parser_recovery_title_token_count, score_parser_recovery_link_candidate,
    score_supplemental_document_link_candidate,
};
pub use requirement_recovery_scope::{
    CatalogCredentialSection, catalog_owner_scope_matcher_titles,
    catalog_owner_title_has_specialized_credential, find_base_catalog_credential_section,
    find_nested_pathway_html_section_scope, find_parallel_pathway_html_table_scope,
    find_pathway_html_curriculum_section_candidate, find_pathway_html_selected_section_candidate,
    find_preceding_admission_requirement_start_index,
    find_preceding_inline_requirement_start_index, get_pathway_heading_identity_token_count,
    has_owner_acronym_departmental_boundary, has_parser_recovery_requirement_cue,
    has_primary_requirement_section_cue, has_sectioned_course_list_requirement_cue,
    has_upcoming_sectioned_course_rows, is_bothell_ias_undergraduate_major_source,
    is_different_program_sibling_page, is_graduate_career_planning_continuation_line,
    is_graduate_career_planning_note, is_graduate_or_applied_masters_requirement_context,
    is_html_section_boundary_line, is_legacy_catalog_credential_heading_line,
    is_likely_base_catalog_credential_heading, is_parser_recovery_pathway_section_boundary_line,
    is_parser_recovery_section_boundary_line, is_pathway_html_section_sibling_start,
    is_pathway_section_boundary_line, legacy_catalog_parent_program_matcher_titles,
    line_belongs_to_degree_route_comparison_section,
    line_looks_like_legacy_catalog_credential_section_start,
    line_matches_selected_pathway_identity, line_references_conflicting_degree_route,
    line_references_different_major_scope, looks_like_sectioned_course_heading_line,
    looks_like_short_named_course_section_heading, normalize_catalog_credential_matcher_title,
    normalize_matcher_text, parsed_mentions_selected_pathway_identity,
    parsed_mentions_selected_pathway_section, parsed_mentions_sibling_pathway_section,
    parser_recovery_candidate_conflicts_with_base_owner_scope,
    parser_recovery_candidate_conflicts_with_pathway_scope,
    parser_recovery_candidate_conflicts_with_program_sibling,
    parser_recovery_candidate_contains_selected_pathway_phrase,
    parser_recovery_owner_program_acronym_tokens, parser_recovery_section_line_score,
    parser_recovery_snapshot_conflicts_with_base_owner_scope, pathway_html_section_scope_lines,
    plan_has_pathway_manifest_entries, preserve_prior_admit_pathway_context, primary_major_title,
    remove_following_different_owner_acronym_departmental_sections,
    remove_parallel_pathway_html_table_content_from_base_scope,
    scope_bothell_chemistry_curriculum_lines, scope_curriculum_era_lines,
    score_owner_snapshot_for_parser_recovery, should_prefer_base_catalog_credential_section,
    should_use_full_admission_prerequisite_html_scope,
    should_use_full_focused_pathway_degree_html_scope,
    should_use_full_generic_concentration_degree_scope,
    source_url_path_mentions_selected_pathway_identity, text_matches_catalog_owner_scope_title,
    uses_authoritative_bothell_chemistry_curriculum_scope,
};
pub use requirement_reports::{
    build_parser_recovery_markdown_report, build_parser_recovery_report,
    build_uw_mse_course_extraction_audit,
};
pub use requirement_role_classifier::{
    classify_requirement_source_role, get_inactive_major_evidence_lines,
    has_conflicting_supplemental_degree_route,
    should_promote_primary_overview_from_parsed_evidence, should_use_tacoma_metadata_primary_role,
    source_content_indicates_inactive_major,
};
pub use requirement_rule_utils::{
    build_rule_evidence, detect_option_replacement, detect_program_approved_credit_bucket,
    filter_course_codes_by_support_only_evidence, get_parser_rule, get_parser_rule_registry,
    has_option_replacement_requirement_cue, has_support_only_line_cue,
    is_credit_bucket_continuation_line, should_join_credit_bucket_continuation,
    slugify_parser_rule_value,
};
pub use requirement_scope::{
    FlattenedRequirementSourceScope, IGNORED_SOURCE_ROLES, NON_SCHEDULABLE_SOURCE_ROLES,
    PARSEABLE_SUPPORT_ROLES, PRIMARY_SOURCE_ROLES, RequirementSourceRoleStatus,
    RequirementSourceScope, SUPPORT_SOURCE_ROLES, build_requirement_source_scope,
    can_parsed_block_create_schedulable_rows, can_requirement_source_role_create_schedulable_rows,
    filter_parsed_course_codes_by_source_sections, flatten_requirement_source_scope,
    get_block_scope_boolean, get_blocked_source_section_course_codes,
    get_requirement_source_role_status, get_schedulable_source_section_lines,
    is_block_non_schedulable, is_block_support_only,
};
pub use requirement_section_roles::{
    build_parser_prerequisite_filter_audit_rows, classify_requirement_group_source_section,
    schedulable_inline_requirement_line_indices,
};
pub use requirement_shape::{
    CHOICE_COUNT_WORDS_PATTERN, CHOICE_LIST_START_PATTERN_SOURCE,
    CHOICE_REQUIREMENT_CONTEXT_PATTERN_SOURCE, CHOICE_REQUIREMENT_LABEL_PATTERN_SOURCE,
    NON_CHOICE_COURSE_LIST_CONTEXT_PATTERN_SOURCE, PROGRAMMING_CHOICE_CONTEXT_PATTERN_SOURCE,
    SEQUENCE_ALTERNATIVE_CHOICE_LINE_PATTERN_SOURCE, SEQUENCE_CHOICE_CONTEXT_PATTERN_SOURCE,
    SEQUENCE_HEADING_BOUNDARY_PATTERN_SOURCE, SEQUENCE_LABEL_ALTERNATIVE_PATTERN_SOURCE,
    STRONG_CHOICE_REQUIREMENT_CONTEXT_PATTERN_SOURCE, detect_option_cue, detect_option_cue_or_none,
    looks_like_standalone_requirement_label_line, looks_like_standalone_requirement_title_line,
    parse_choice_required_count, strip_leading_requirement_glyphs, strip_snapshot_page_prefix,
};
pub use requirement_snapshots::{
    RequirementSnapshotStore, RequirementSourceSnapshot, SnapshotRecoveryArtifacts,
    normalize_snapshot_source_url, parse_heading_metadata as parse_requirement_snapshot_headings,
    source_url_has_hash as requirement_snapshot_source_has_hash,
    source_url_without_hash as requirement_snapshot_source_without_hash,
};
pub use requirement_source_audits::{
    build_course_emission_kind, build_parser_sequence_choice_audit_rows,
    build_requirement_support_list_from_metadata, build_requirement_support_lists_from_metadata,
    build_source_scope_audit_lines, build_support_lists_from_parsed_requirement_groups,
    build_support_lists_from_source_section_audit_rows, dedupe_requirement_support_lists,
    extract_approved_filter_evidence_from_lines, filter_explicitly_excluded_course_codes,
    filter_parsed_requirement_group_options_by_source_sections,
    filter_parsed_requirement_groups_by_course_code_predicate,
    filter_parsed_requirement_groups_by_source_sections,
    filter_parsed_requirement_option_codes_by_source_sections,
    find_source_section_audit_row_for_text, get_approved_filter_codes_for_support_source,
    get_globally_explicitly_excluded_course_codes, get_group_source_section_decision,
    get_requirement_support_list_semantic_key, get_source_section_decision_for_course,
    infer_approved_list_key_from_support_metadata,
    parsed_requirement_group_has_requirement_surface, source_section_decision_allows_course_option,
    source_section_rows_allow_course_code,
};
pub use requirement_source_fetching::{
    DEFAULT_CURL_ACCEPT_HEADER, DEFAULT_CURL_COMMAND, DEFAULT_HOST_COOLDOWN_MS,
    DEFAULT_MAX_RETRY_DELAY_MS, DEFAULT_RETRY_ATTEMPTS, DEFAULT_USER_AGENT, DownloadedSource,
    SOURCE_DOWNLOAD_MAX_BUFFER_BYTES, SourceDownloader, SourceDownloaderOptions, SourceFetchMode,
    build_curl_error_message, get_host_key, get_rate_limit_scope_key, is_retryable_http_status,
    parse_retry_after_to_ms, sanitize_source_url,
};
pub use requirement_source_output::{
    RequirementSourceAdapterArtifacts, RequirementSourcePipelineArtifactPaths,
    RequirementSourcePipelineArtifacts, RequirementSourcePipelineOutput,
    build_requirement_source_adapter_artifacts, build_requirement_source_adapter_blocks,
    build_requirement_source_adapter_summary, build_requirement_source_pipeline_artifacts,
    build_requirement_source_pipeline_output, render_requirement_source_adapter_artifacts,
    write_requirement_source_pipeline_artifacts,
};
pub use requirement_source_pipeline::{
    ParserRecoveryCandidateParseResult, RequirementManifestEntrySourceFacts,
    RequirementSourcePipeline, RequirementSourcePipelineResult,
    RequirementSourceRecoveryCandidates, RequirementSourceResolution,
    SupplementalRequirementSourceResolution,
};
pub use requirement_source_quality::{
    build_parse_quality_signals, build_uw_mse_course_extraction_audit_for_owner,
    parser_recovery_trigger_codes,
};
pub use requirement_source_recovery::{
    build_parser_recovery_attempt_record, build_parser_recovery_owner_snapshot,
    build_parser_recovery_source_fingerprint, classify_parser_recovery_blocker,
    merge_recovered_support_sources, parser_recovery_owner_looks_too_broad, same_source_url,
    score_parser_recovery_owner, should_accept_parser_recovery_owner,
};
pub use requirement_source_scoring::{
    get_course_code_numeric_level, get_course_code_subject, get_parsed_course_subjects,
    get_parsed_lower_division_course_count, get_parsed_owner_alignment_score,
    get_parsed_result_score, has_meaningful_parsed_content, is_weak_parsed_result,
    should_allow_alternate_to_replace_best_source, should_merge_supplemental_alternate_source,
};
pub use sequence_text::{
    SequenceCourseCodeInput, SequenceCourseCodeRow, SequenceInlineSegmentsInput,
    expand_sequence_course_codes, split_inline_sequence_segments,
};
pub use source_backed_coverage::{
    AUDIT_ROW_COLLECTIONS as SOURCE_BACKED_AUDIT_ROW_COLLECTIONS, AccountingRegressionInput,
    AeronauticsCategoryOptionRegressionInput, Ast2CivilMechanicalRegressionInput,
    Ast2ComputerElectricalRegressionInput, BioengineeringRegressionInput,
    COVERAGE_GATE_DESCRIPTION, COVERAGE_GATE_LABEL, ChemicalEngineeringRegressionInput,
    CivilEngineeringRegressionInput, ComputerEngineeringRegressionInput,
    ComputerScienceDataScienceRegressionInput, EcePhotonicsRegressionInput,
    EnvironmentalEngineeringRegressionInput, GENERATED_REGISTRY_ISSUE_TYPES,
    GENERATED_REGISTRY_PROTECTED_PLAN_IDS, HIGH_RISK_AUDIT_CATEGORIES,
    ISSUE_TYPES as SOURCE_BACKED_ISSUE_TYPES, ME_ENGINEERING_FUNDAMENTALS_UW_CODES,
    MappingRegressionRowGroups, MatchedTrackSummaryAuditInput, MseNmeRegressionInput,
    ParsedMatchedTrackSummary, PrepCreditSeparationInput, ProtectedRequirementAuditInputs,
    SCOPE_ISSUE_TYPES as SOURCE_SCOPE_ISSUE_TYPES, SbseBusinessComputationOptionInput,
    SbseNoTranscriptClassificationInput, SourceBackedAuditBuildInput, SourceBackedAuditCliOptions,
    SourceBackedAuditExecutionArtifacts, SourceBackedAuditMode, SourceBackedAuditModeInputs,
    SourceBackedAuditRenderedArtifacts, SourceBackedAuditReportPaths,
    SourceBackedMappingAuditInput, SourceBackedOwnerInput, SourceBackedRegressionCheckGroups,
    append_regression_check, append_source_backed_coverage_markdown_gate_sections,
    assemble_mapping_regression_rows, assemble_source_backed_audit_execution_artifacts,
    assert_order_check, assert_visible_course_codes_in_quarter_plan_check,
    assert_visible_courses_check, audit_target_uw_course_codes,
    base_requirement_coverage_should_delegate_to_pathways, block_has_support_metadata,
    build_accounting_regression_checks, build_actionable_audit_issue_metadata,
    build_actionable_source_backed_coverage_issue_for_test,
    build_aeronautics_category_option_regression_checks,
    build_aeronautics_mixed_category_parser_extraction_regression_row,
    build_ast2_civil_mechanical_regression_checks,
    build_ast2_computer_electrical_regression_checks, build_bioengineering_regression_checks,
    build_category_mapping_regression_rows_from_audit_rows,
    build_chemical_engineering_elective_parser_extraction_regression_row,
    build_chemical_engineering_regression_checks, build_civil_engineering_regression_checks,
    build_compound_equivalency_mapping_regression_rows,
    build_computer_engineering_approved_filter_check,
    build_computer_engineering_approved_source_role_check,
    build_computer_engineering_completed_programming_check,
    build_computer_engineering_credit_bucket_check,
    build_computer_engineering_parser_extraction_regression_rows,
    build_computer_engineering_programming_option_check,
    build_computer_engineering_regression_checks,
    build_computer_engineering_required_physics_coverage_check,
    build_computer_engineering_selected_cse143_path_check,
    build_computer_science_approved_source_role_check,
    build_computer_science_data_science_regression_check,
    build_computer_science_prerequisite_filter_parser_extraction_regression_row,
    build_computer_science_regression_checks, build_coverage_rows_for_owner,
    build_coverage_rows_for_owners, build_credit_category_shape_audit_rows_for_owner,
    build_credit_category_shape_audit_rows_for_owners, build_credit_category_shape_rows_for_plan,
    build_ece_photonics_regression_checks, build_elective_approved_list_shape_audit_rows_for_owner,
    build_elective_approved_list_shape_audit_rows_for_owners,
    build_environmental_engineering_parser_extraction_regression_rows,
    build_environmental_engineering_regression_checks, build_generated_registry_audit_artifacts,
    build_generated_registry_audit_artifacts_with_context, build_generated_registry_audit_markdown,
    build_generated_registry_audit_owners, build_generated_registry_audit_report,
    build_generated_registry_audit_rows, build_generated_registry_audit_rows_with_context,
    build_generated_registry_issue_counts, build_generated_registry_protected_rows,
    build_generated_registry_regression_row, build_generated_registry_rows_from_seed_rows,
    build_generated_registry_rows_from_shape_rows, build_generated_shape_audit_group_row,
    build_generated_shape_audit_rows_for_owner, build_generated_shape_audit_rows_for_owners,
    build_generated_shape_audit_support_row, build_generated_source_seed_audit_rows_for_owner,
    build_generated_source_seed_audit_rows_for_owners,
    build_hidden_unmapped_mapping_regression_rows, build_informatics_scope_check,
    build_issue_counts, build_known_single_equivalency_audit_rows, build_major_gen_ed_scope_check,
    build_mapping_audit_artifacts, build_mapping_audit_markdown, build_mapping_audit_report,
    build_mapping_issue_counts, build_mapping_regression_row, build_matched_track_summary_check,
    build_matched_track_summary_checks, build_mechanical_engineering_regression_checks,
    build_mechanical_source_discovery_audit_lines, build_mechanical_source_section_audit_lines,
    build_mse_nme_regression_checks, build_parsed_requirement_rows_for_coverage,
    build_parser_audit_rows_for_owners, build_parser_category_option_audit_rows_for_owner,
    build_parser_credit_bucket_audit_rows_for_owner, build_parser_extraction_regression_row,
    build_parser_option_extraction_audit_rows_for_owner,
    build_parser_prerequisite_filter_audit_rows_for_owner,
    build_parser_sequence_choice_audit_rows_for_owner, build_prep_credit_separation_check,
    build_program_filter_mapping_regression_rows, build_protected_requirement_rows,
    build_protected_sequence_parser_extraction_regression_rows, build_requirement_shape_audit_row,
    build_requirement_shape_audit_rows_for_owner, build_requirement_shape_audit_rows_for_owners,
    build_requirement_shape_rows_for_plan, build_requirement_shape_rows_for_source_blocks,
    build_runtime_compound_mapping_regression_rows, build_sbse_business_computation_option_check,
    build_sbse_catalog_anchor_parser_extraction_regression_row,
    build_sbse_no_transcript_classification_check,
    build_sequence_pathway_shape_audit_rows_for_owner,
    build_sequence_pathway_shape_audit_rows_for_owners, build_sequence_pathway_shape_rows_for_plan,
    build_single_equivalency_audit_row, build_single_equivalency_audit_rows_for_owner,
    build_single_equivalency_audit_rows_for_owners, build_single_equivalency_audit_rows_for_plan,
    build_single_equivalency_mapping_regression_rows,
    build_source_backed_audit_execution_artifacts_for_args,
    build_source_backed_audit_execution_artifacts_for_flags,
    build_source_backed_audit_execution_artifacts_for_input,
    build_source_backed_audit_failure_diagnostic_lines,
    build_source_backed_audit_rendered_artifacts_for_args, build_source_backed_audit_summary_lines,
    build_source_backed_coverage_artifacts_with_context, build_source_backed_coverage_markdown,
    build_source_backed_coverage_markdown_summary, build_source_backed_coverage_report,
    build_source_backed_coverage_report_artifacts,
    build_source_backed_coverage_report_with_context,
    build_source_backed_coverage_report_with_owner_context,
    build_source_backed_mapping_audit_artifacts, build_source_backed_mapping_audit_rows,
    build_source_backed_owners, build_source_backed_protected_rows,
    build_source_backed_regression_checks, build_source_backed_regression_checks_from_context,
    build_source_role_coverage_rows, build_source_scope_audit_rows_for_owner,
    build_source_scope_audit_rows_for_owners, build_source_scope_emission_kind,
    build_source_scope_issue_counts, build_source_scope_regression_rows,
    checklist_item_creates_generated_schedule_row, checklist_item_grc_course_codes,
    checklist_item_uw_course_codes, classify_coverage_issue, classify_generated_seed_issue,
    classify_source_backed_coverage_issue_for_test, collect_actionable_issue_rows,
    collect_generated_requirement_groups_by_id, enrich_source_backed_coverage_report,
    enrich_source_backed_coverage_report_with_context, escape_audit_markdown,
    filter_parser_extraction_regression_rows_for_target, find_generated_group_for_parsed_group,
    find_programming_audit_row, generated_group_shape_issue, get_confidence_score,
    get_coverage_block_pathway_id, get_default_plan_context_for_audit_row, get_hidden_reason,
    get_official_compound_equivalency_rules, get_option_grc_course_codes_for_coverage,
    get_option_uw_course_codes_for_coverage, get_parsed_engineering_fundamentals_from_blocks,
    get_plan_context_for_audit_row, get_recommended_fix_for_layer,
    get_suspected_layer_for_actionable_issue, get_visible_course_codes_for_requirement,
    grc_equivalents_for_uw_course, grc_equivalents_for_uw_courses,
    has_complete_grc_equivalent_course_set_for_uw_course, has_no_audit_issue,
    hydrate_source_backed_audit_snapshot_text, include_parser_extraction_target,
    is_intentionally_superseded_generated_shape_audit_group,
    is_non_schedulable_contextual_source_row,
    is_parsed_choice_represented_by_selected_runtime_alternative,
    is_parsed_course_represented_by_unselected_runtime_option, join_audit_markdown_list,
    normalized_compound_course_set_key, parse_grouped_choice_max, parse_matched_track_summary,
    parse_source_backed_audit_cli_options, parsed_block_schedulable_evidence_includes_course,
    parsed_coverage_rows_match, parsed_group_has_generated_shape_audit_surface,
    parsed_requirement_blocks_for_scope, parsed_requirement_group_contains_programming_options,
    parsed_row_is_represented_by_sibling_pathway_source,
    protected_sequence_expectation_applies_for_test,
    render_source_backed_audit_execution_artifacts,
    render_source_backed_audit_execution_artifacts_for_source_root,
    runtime_checklist_items_for_coverage, runtime_generated_course_codes,
    runtime_generated_course_codes_for_source, runtime_grc_course_list,
    runtime_item_matches_parsed_requirement_context, runtime_requirement_group_represents_choice,
    schedulable_parsed_uw_course_codes, scheduled_course_codes_for_coverage,
    scoped_uw_courses_backing_grc_course, select_source_backed_audit_mode,
    serialize_source_backed_audit_artifacts, serialize_source_backed_coverage_artifacts,
    source_backed_audit_failure_exit_code, source_backed_audit_failure_exit_code_for_mode,
    source_backed_audit_report_paths, visible_course_codes_for_requirement,
    visible_planned_course_codes, visible_planned_course_labels, visible_planned_courses,
    visible_prompt_option_group_shows_option, visible_selection_prompt_option_group_ids,
    visible_selection_prompt_option_groups_for_coverage,
    write_source_backed_audit_rendered_artifacts,
    write_source_backed_audit_rendered_artifacts_for_args,
};
pub use source_backed_coverage_command::run_source_backed_coverage_audit;
pub use source_fingerprints::{
    build_report as build_source_fingerprints_report, build_requirement_fingerprint_entry,
    build_source_change_classification_report, compare_fingerprints,
    get_requirement_fingerprint_compare_key, run_build_source_fingerprints,
};
pub use source_pipeline_validation::{
    DerivedOptionAliasTitle, SourcePipelineArtifactAlignmentResult,
    SourcePipelineEligiblePromotionChecks, SourcePipelineJsonReports,
    SourcePipelineParserAlignmentResult, SourcePipelinePromotionContext,
    SourcePipelineRelationshipChecks, SourcePipelineReportArtifacts, SourcePipelineScopedOwnerSets,
    SourcePipelineValidationCheck, SourcePipelineValidationExecution,
    SourcePipelineValidationExecutionInput, SourcePipelineValidationInput,
    SourcePipelineValidationMetricInputs, SourcePipelineValidationMetrics,
    SourcePipelineValidationReport, assemble_source_pipeline_core_checks,
    build_artifact_alignment_checks, build_discovery_partition_check,
    build_eligible_promotion_checks, build_owner_key, build_parseable_primary_manifest_owner_ids,
    build_parseable_requirement_source_manifest_entries, build_parser_backed_review_owner_keys,
    build_primary_manifest_owner_ids, build_promotion_entry_candidate_from_owner,
    build_registry_parser_alignment_result, build_review_and_promotion_checks,
    build_review_owner_key_set, build_source_pipeline_core_execution,
    build_source_pipeline_core_execution_from_json, build_source_pipeline_scoped_owner_sets,
    build_source_pipeline_validation_execution, build_source_pipeline_validation_metrics,
    build_source_pipeline_validation_report, compare_string_sets,
    current_primary_cannot_create_schedulable_rows, find_failed_parseable_requirement_sources,
    get_blocked_missing_primary_auto_promotion_owners, get_eligible_auto_promotion_owners,
    get_eligible_missing_primary_auto_promotion_owners,
    get_eligible_weak_existing_replacement_owners, get_missing_primary_auto_promotion_block_reason,
    get_review_queue_entries, get_single_specialized_plan_official_url,
    get_validation_scoped_discovery_owner_count, get_validation_scoped_discovery_owners,
    get_validation_scoped_review_queue_entries, has_derived_parent_pathway_runtime_coverage,
    has_document_url_with_appended_path, has_student_runtime_alias_coverage,
    is_ba_bs_route_promotion_for_broad_major_owner, is_catalog_credential_promotion_entry,
    is_catalog_credential_promotion_for_different_major,
    is_catalog_program_page_promotion_for_different_major, is_catalog_program_promotion_entry,
    is_catalog_program_promotion_for_different_major, is_clearly_support_only_promotion_entry,
    is_inactive_pathway_review_owner, is_minor_credential_promotion_for_major_owner,
    is_pathway_scoped_catalog_credential_promotion_for_broad_major_owner,
    is_schedulable_primary_suggestion, is_seattle_jsis_asia_concentration_leaked_promotion_entry,
    is_skip_navigation_promotion_entry, is_structural_placeholder_review_owner,
    is_suspicious_structural_pathway_promotion_entry,
    is_tacoma_global_studies_leaked_promotion_entry, is_unsafe_automatic_promotion_entry,
    is_unscoped_acs_route_promotion_for_broad_major_owner, load_source_pipeline_json_reports,
    normalize_catalog_credential_major_name, normalize_promotion_url_for_comparison,
    normalize_requirement_source_url_for_coverage, parse_derived_option_alias_title,
    parse_target_plan_ids, render_markdown_report, render_source_pipeline_report_artifacts,
    run_async_source_pipeline_check, run_source_pipeline_check,
    should_skip_manual_override_promotion, suggested_primary_is_safe_for_automatic_promotion,
    suggested_primary_matches_specialized_plan_source, write_source_pipeline_report_artifacts,
};
pub use spot_check_major::run_spot_check_major;
pub use transfer_plan_title_matching::{
    build_transfer_planner_title_signature_tokens, get_specialized_plan_title_tokens,
    label_mentions_different_transfer_planner_major, normalize_specialized_plan_tokens,
    normalize_transfer_planner_semantic_pathway_label, normalize_transfer_planner_text,
    specialized_plan_tokens_match_candidate, strip_transfer_planner_plan_title_prefix_repeatedly,
};
pub use uw_catalog::{UwCatalogReport, refresh as refresh_uw_catalog};

use std::path::Path;

pub use grc_public_materials::{
    GrcAnnualScheduleCandidate, GrcAnnualScheduleCandidatesInput, GrcAnnualScheduleFilterInput,
    GrcCatalogArchiveEntriesInput, GrcCatalogEntry, GrcCurrentCatalog,
    GrcCurrentCatalogDetailsInput, GrcPublicMaterials, GrcSchedule,
    build_paged_course_descriptions_url, detect_catalog_page_numbers,
    filter_relevant_annual_schedule_values, parse_annual_schedule_candidates,
    parse_catalog_archive_entries, parse_current_catalog_details,
};

pub fn discover_grc_public_materials(
    repository_root: &Path,
    force_refresh: bool,
    allow_snapshot_fallback: bool,
    cache_only: bool,
) -> Result<GrcPublicMaterials, String> {
    grc_public_materials::discover(
        repository_root,
        force_refresh,
        allow_snapshot_fallback,
        cache_only,
    )
}
