mod deadlines;
mod equivalency_catalog;
mod equivalency_derived;
mod equivalency_output;
mod equivalency_report;
mod equivalency_rows;
mod grc_catalog;
mod grc_public_materials;
mod planner_reporting;
mod requirement_adapters;
mod requirement_atoms;
mod requirement_bootstrap_index;
mod requirement_course_codes;
mod requirement_category_options;
mod requirement_choice_lists;
mod requirement_groups;
mod requirement_course_scoping;
mod requirement_degree_map_blocks;
mod requirement_document_facts;
mod requirement_docx;
mod requirement_html_source;
mod requirement_html;
mod requirement_ids;
mod requirement_manifest;
mod requirement_manifest_selection;
mod requirement_manifest_fallback;
mod requirement_manifest_context;
mod requirement_plan_matching;
mod requirement_parse_helpers;
mod requirement_reports;
mod requirement_recovery_scope;
mod requirement_recovery_links;
mod requirement_role_classifier;
mod requirement_pdf_layout;
mod requirement_scope;
mod requirement_shape;
mod requirement_rule_utils;
mod requirement_source_fetching;
mod requirement_source_audits;
mod requirement_source_pipeline;
mod requirement_snapshots;
mod source_artifact_cache;
mod sequence_text;
mod uw_catalog;
mod transfer_plan_title_matching;

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
pub use planner_reporting::{
    normalize_markdown_report, serialize_json_report, serialize_markdown_lines,
    write_json_report, write_markdown_lines_report, write_markdown_report, write_report_pair,
};
pub use requirement_adapters::{
    RequirementSourceAdapter, RequirementSourceParserKind, select_requirement_source_adapter,
};
pub use requirement_atoms::{
    ParsedRequirementPhase, build_parsed_requirement_atom_candidates, build_source_line_hint,
    get_source_line_hints, infer_parsed_requirement_phase_from_hints,
};
pub use requirement_bootstrap_index::RequirementBootstrapPlanIndex;
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
    get_structured_uw_course_codes, structured_degree_map_block_matches_manifest_entry,
};
pub use requirement_document_facts::{
    RequirementSourceTextFacts, collect_docx_source_facts, collect_html_source_facts,
    collect_pdf_source_facts,
};
pub use requirement_docx::{
    DocxSourceArtifacts, DocxSourceParser, build_docx_lines, extract_docx_document_xml,
    extract_docx_text, get_linked_document_parser_type, is_docx_source_url,
    is_linked_document_source_url, is_pdf_source_url,
};
pub use requirement_html_source::HtmlSourceParser;
pub use requirement_manifest::{
    dedupe_parseable_primary_entries, exclude_unhashed_primary_manifest_duplicates,
    normalize_manifest_url_for_dedupe, normalize_manifest_url_without_hash_for_dedupe,
    parseable_manifest_entry_dedupe_key, preselect_parseable_manifest_entries,
};
pub use requirement_manifest_selection::select_parseable_manifest_entries;
pub use requirement_manifest_fallback::{
    build_parser_recovery_candidate_entry, compare_manifest_fallback_candidates,
    get_parser_recovery_candidate_parser_type, select_alternate_parseable_manifest_entries,
    should_allow_alternate_manifest_entry,
};
pub use requirement_manifest_context::RequirementSourceManifestContext;
pub use requirement_ids::{
    build_requirement_source_block_id, build_unique_derived_id, slugify_requirement_identifier,
    stable_source_id_hash,
};
pub use requirement_html::{
    RequirementHtmlArtifacts, build_html_lines as build_requirement_html_lines,
    decode_transfer_planner_html_entities, extract_headings as extract_requirement_html_headings,
    extract_title as extract_requirement_html_title,
    parse_html_artifacts as parse_requirement_html_artifacts,
};
pub use requirement_snapshots::{
    RequirementSourceSnapshot, RequirementSnapshotStore, SnapshotRecoveryArtifacts,
    normalize_snapshot_source_url, parse_heading_metadata as parse_requirement_snapshot_headings,
    source_url_has_hash as requirement_snapshot_source_has_hash,
    source_url_without_hash as requirement_snapshot_source_without_hash,
};
pub use requirement_source_audits::{
    build_course_emission_kind, build_parser_sequence_choice_audit_rows,
    build_requirement_support_list_from_metadata, build_source_scope_audit_lines,
    build_support_lists_from_source_section_audit_rows,
    filter_parsed_requirement_group_options_by_source_sections,
    filter_parsed_requirement_groups_by_course_code_predicate,
    filter_parsed_requirement_option_codes_by_source_sections,
    find_source_section_audit_row_for_text, get_source_section_decision_for_course,
    parsed_requirement_group_has_requirement_surface, source_section_decision_allows_course_option,
    source_section_rows_allow_course_code,
};
pub use requirement_source_pipeline::{
    RequirementManifestEntrySourceFacts, RequirementSourcePipeline,
    RequirementSourcePipelineResult, RequirementSourceRecoveryCandidates,
};
pub use requirement_source_fetching::{
    DEFAULT_CURL_ACCEPT_HEADER, DEFAULT_CURL_COMMAND, DEFAULT_HOST_COOLDOWN_MS,
    DEFAULT_MAX_RETRY_DELAY_MS, DEFAULT_RETRY_ATTEMPTS, DEFAULT_USER_AGENT, DownloadedSource,
    SOURCE_DOWNLOAD_MAX_BUFFER_BYTES, SourceDownloader, SourceDownloaderOptions,
    SourceFetchMode, build_curl_error_message, get_host_key, get_rate_limit_scope_key,
    is_retryable_http_status, parse_retry_after_to_ms, sanitize_source_url,
};
pub use requirement_pdf_layout::{
    PdfLineSegment, PdfPageBlock, PdfTextContent, PdfTextItem, build_pdf_page_line_texts,
    get_pdf_title_scope_tokens, order_pdf_line_segments, scope_pdf_page_blocks,
    scope_pdf_page_lines,
};
pub use requirement_category_options::{
    build_category_requirement_option, build_category_requirement_options_from_choice_line,
    count_requirement_category_option_descriptors, get_requirement_category_option_descriptor,
    get_requirement_category_option_descriptors,
};
pub use requirement_choice_lists::{
    build_choice_list_source_line, build_forward_labeled_choice_requirement_lines,
    build_multi_line_choice_requirement_lines, can_continue_wrapped_course_number,
    choice_list_continuation_looks_like_adjacent_requirement,
    choice_list_looks_closed, find_nearby_requirement_choice_label,
    find_nearest_lettered_choose_one_heading,
    has_choice_requirement_context, has_strong_choice_requirement_context,
    is_combined_choose_one_heading_with_lettered_option, is_lettered_option_line,
    is_lettered_option_line_under_choose_one_heading, is_numbered_choose_one_option_heading,
    normalize_sequence_source_line, strip_leading_lettered_option_marker,
    get_inline_labeled_requirement_parts, is_standalone_credit_amount_line,
    line_starts_parenthetical_choice_list, looks_like_choice_list_continuation,
    strip_inline_labeled_requirement_marker, InlineLabeledRequirementParts,
    strip_choice_list_line, strip_non_option_course_evidence_from_choice_line,
    trim_choice_list_continuation_noise,
};
pub use requirement_groups::{
    build_forward_labeled_choice_requirement_groups,
    build_inline_course_requirement_options, build_mixed_inline_required_course_list_groups,
    build_numbered_inline_all_required_course_group,
    build_parser_rule_requirement_replacements,
    build_parsed_requirement_course,
    build_parsed_requirement_courses_from_groups, build_parsed_requirement_group,
    build_source_derived_requirement_courses,
    build_internal_choice_split_sectioned_course_requirement_groups,
    build_parsed_requirement_option, build_pathway_section_fallback_concentration_group,
    build_sectioned_option_course_option, build_sectioned_option_requirement_groups,
    build_repeated_required_alternative_groups, build_split_either_or_requirement_groups,
    build_standalone_course_choice_requirement_groups,
    collect_sectioned_option_course_options, extract_inline_required_list_alternative_sets,
    get_mixed_required_course_list_parts, get_next_meaningful_section_line,
    detect_program_elective_credit_bucket, detect_subject_prefix_credit_bucket,
    get_program_specific_credit_bucket_descriptor,
    extract_subject_prefix_from_credit_bucket_text,
    get_approved_list_key_from_credit_bucket_text, get_positive_credit_bucket_category_text,
    get_program_elective_bucket_label, has_credit_bucket_cue,
    has_course_question_support_cue, has_non_requirement_course_exclusion_cue,
    build_category_requirement_options_from_credit_bucket,
    build_credit_bucket_source_line, build_course_requirement_options_from_credit_bucket,
    build_generic_credit_bucket_requirement_groups,
    build_parallel_choose_one_requirement_groups,
    build_category_credit_bucket_option, build_sbse_comparison_table_requirement_groups,
    finalize_parsed_requirement_groups,
    get_all_required_requirement_group_course_set_key, get_choice_requirement_group_course_set,
    get_choice_requirement_group_course_set_key, get_requirement_group_option_course_set_key,
    get_source_choice_requirement_group_scope_key, is_proper_course_set_subset,
    is_source_choice_requirement_group, looks_like_merged_pdf_choice_fragment_group,
    suppress_generic_choice_groups_covered_by_inline_choices,
    suppress_inline_required_groups_covered_by_choice_options,
    suppress_owner_specific_requirement_group_leaks,
    suppress_repeated_required_alternative_groups_covered_by_sequence_choices,
    suppress_subset_source_choice_groups,
    choose_preferred_requirement_group, count_concrete_course_options,
    dedupe_credit_bucket_requirement_groups, get_credit_bucket_dedupe_key,
    filter_generic_bse_supplemental_groups_for_sbse,
    filter_requirement_groups_by_supplemental_pathway_scope,
    find_nearest_supplemental_source_marker, find_snapshot_line_index_for_requirement_group,
    get_requirement_option_course_code_set, get_parallel_choose_one_heading_label,
    get_parallel_choose_one_heading_subjects, heading_subjects_match_course_codes,
    is_requirement_group_under_support_course_list, line_starts_with_numbered_parallel_choice,
    suppress_choice_groups_covered_by_credit_buckets,
    requirement_group_has_program_specific_category_option,
    extract_course_codes_from_credit_bucket_text, split_credit_bucket_source_lines,
    is_approved_list_placeholder_without_immediate_course_list,
    next_line_looks_like_course_list_for_heading,
    get_repeated_required_alternative_sets,
    has_immediate_sectioned_course_rows_after_heading,
    build_alternating_number_sequence_choice_candidate,
    build_ampersand_or_single_sequence_choice_candidate,
    build_ampersand_or_single_sequence_choice_requirement_groups,
    build_parenthetical_sequence_choice_candidates,
    build_parenthetical_sequence_choice_requirement_groups,
    build_sequence_choice_requirement_group, apply_sequence_lab_notes,
    collect_sequence_lab_notes, get_sequence_fallback_subject_from_context,
    get_simple_sequence_fallback_subject_from_context,
    get_expected_sequence_subjects_from_cue, is_sequence_choice_boundary_line,
    sequence_row_looks_like_single_path, sequence_row_subjects_fit_cue,
    split_sequence_alternative_labels, coalesce_sectioned_credit_buckets_with_inline_choices,
    extract_option_course_codes_from_sectioned_line,
    extract_parsed_requirement_group_course_codes, get_choice_requirement_group_label,
    get_trimmed_sectioned_course_group_heading, is_generic_category_credit_bucket_heading,
    is_generic_university_category_distribution_heading, normalize_pdf_requirement_heading_line,
    normalize_sectioned_course_group_label, parse_leading_section_heading_credit_range,
    parse_section_heading_credit_range, parse_sectioned_course_credit_list_line,
    parse_sectioned_course_required_count,
    get_sequence_choice_cue, get_sequence_path_label_from_text,
    is_equivalent_number_alias_line, is_numbered_option_row_under_choose_one_heading,
    looks_like_admission_decision_narrative_credit_bucket,
    looks_like_aggregate_general_education_credit_summary,
    looks_like_composite_credit_heading,
    looks_like_credit_summary_for_named_requirement_section,
    looks_like_degree_total_general_elective_prose,
    looks_like_elective_timing_or_sample_plan_note,
    looks_like_shared_humanities_social_credit_heading,
    looks_like_support_only_research_opportunity_line,
    has_following_specific_humanities_social_minimums, should_skip_credit_bucket_line,
    starts_with_category_credit_cue, should_skip_raw_partial_choice_line,
};
pub use requirement_parse_helpers::{
    build_parse_confidence, extract_choose_statements, extract_relevant_requirement_lines,
    extract_requirement_cue_lines, get_inline_minimum_course_list_text,
    parse_inline_minimum_course_count, parse_requirement_course_count,
    parse_requirement_credit_amount, parse_requirement_credit_range,
};
pub use requirement_role_classifier::{
    classify_requirement_source_role, has_conflicting_supplemental_degree_route,
};
pub use requirement_rule_utils::{
    build_rule_evidence, detect_option_replacement, detect_program_approved_credit_bucket,
    filter_course_codes_by_support_only_evidence, get_parser_rule, get_parser_rule_registry,
    has_option_replacement_requirement_cue, has_support_only_line_cue,
    is_credit_bucket_continuation_line, should_join_credit_bucket_continuation,
    slugify_parser_rule_value,
};
pub use requirement_plan_matching::{
    build_primary_major_titles_by_plan_id,
    filter_manifest_entries_for_specialized_plan_title,
    manifest_entry_matches_specialized_plan_title, manifest_entry_mentions_different_major,
    parser_recovery_candidate_conflicts_with_owner_identity,
};
pub use transfer_plan_title_matching::{
    build_transfer_planner_title_signature_tokens, get_specialized_plan_title_tokens,
    label_mentions_different_transfer_planner_major, normalize_specialized_plan_tokens,
    normalize_transfer_planner_semantic_pathway_label, normalize_transfer_planner_text,
    specialized_plan_tokens_match_candidate, strip_transfer_planner_plan_title_prefix_repeatedly,
};
pub use requirement_reports::build_parser_recovery_markdown_report;
pub use requirement_recovery_links::{
    ParserRecoveryLinkSignals, SupplementalDocumentLinkSignals,
    extract_html_anchor_candidates, extract_supplemental_html_link_candidates,
    has_primary_requirement_link_cue,
    has_recovery_link_signal, is_graduate_supplemental_source,
    is_ignored_parser_recovery_href, is_noisy_parser_recovery_link,
    is_official_linked_asset_url,
    normalize_requirement_matcher_text, parser_recovery_title_scope_tokens,
    parser_recovery_title_token_count, score_parser_recovery_link_candidate, score_supplemental_document_link_candidate,
};
pub use requirement_recovery_scope::{
    has_upcoming_sectioned_course_rows,
    parser_recovery_candidate_contains_selected_pathway_phrase,
    parser_recovery_candidate_conflicts_with_base_owner_scope,
    parser_recovery_candidate_conflicts_with_pathway_scope,
    parser_recovery_candidate_conflicts_with_program_sibling,
    parser_recovery_snapshot_conflicts_with_base_owner_scope,
    find_nested_pathway_html_section_scope,
    find_parallel_pathway_html_table_scope,
    find_pathway_html_curriculum_section_candidate,
    find_pathway_html_selected_section_candidate,
    find_preceding_inline_requirement_start_index,
    has_owner_acronym_departmental_boundary, has_parser_recovery_requirement_cue,
    has_primary_requirement_section_cue, is_different_program_sibling_page,
    is_html_section_boundary_line, is_legacy_catalog_credential_heading_line,
    is_parser_recovery_pathway_section_boundary_line,
    is_parser_recovery_section_boundary_line,
    line_looks_like_legacy_catalog_credential_section_start,
    looks_like_sectioned_course_heading_line,
    looks_like_short_named_course_section_heading,
    parser_recovery_owner_program_acronym_tokens, parser_recovery_section_line_score,
    pathway_html_section_scope_lines, preserve_prior_admit_pathway_context,
    is_graduate_career_planning_continuation_line, is_graduate_career_planning_note,
    is_pathway_html_section_sibling_start, is_pathway_section_boundary_line,
    line_matches_selected_pathway_identity, normalize_matcher_text, primary_major_title,
    plan_has_pathway_manifest_entries,
    remove_following_different_owner_acronym_departmental_sections,
    remove_parallel_pathway_html_table_content_from_base_scope,
    scope_curriculum_era_lines, score_owner_snapshot_for_parser_recovery,
    should_use_full_generic_concentration_degree_scope,
};
pub use requirement_scope::{
    FlattenedRequirementSourceScope, IGNORED_SOURCE_ROLES, NON_SCHEDULABLE_SOURCE_ROLES,
    PARSEABLE_SUPPORT_ROLES, PRIMARY_SOURCE_ROLES, RequirementSourceRoleStatus,
    RequirementSourceScope, SUPPORT_SOURCE_ROLES, build_requirement_source_scope,
    can_parsed_block_create_schedulable_rows,
    can_requirement_source_role_create_schedulable_rows, flatten_requirement_source_scope,
    filter_parsed_course_codes_by_source_sections, get_blocked_source_section_course_codes,
    get_schedulable_source_section_lines, get_block_scope_boolean,
    get_requirement_source_role_status, is_block_non_schedulable, is_block_support_only,
};
pub use requirement_shape::{
    CHOICE_COUNT_WORDS_PATTERN, CHOICE_LIST_START_PATTERN_SOURCE,
    CHOICE_REQUIREMENT_CONTEXT_PATTERN_SOURCE, CHOICE_REQUIREMENT_LABEL_PATTERN_SOURCE,
    NON_CHOICE_COURSE_LIST_CONTEXT_PATTERN_SOURCE,
    PROGRAMMING_CHOICE_CONTEXT_PATTERN_SOURCE,
    SEQUENCE_ALTERNATIVE_CHOICE_LINE_PATTERN_SOURCE,
    SEQUENCE_CHOICE_CONTEXT_PATTERN_SOURCE, SEQUENCE_HEADING_BOUNDARY_PATTERN_SOURCE,
    SEQUENCE_LABEL_ALTERNATIVE_PATTERN_SOURCE,
    STRONG_CHOICE_REQUIREMENT_CONTEXT_PATTERN_SOURCE, detect_option_cue,
    detect_option_cue_or_none, looks_like_standalone_requirement_label_line,
    looks_like_standalone_requirement_title_line, parse_choice_required_count,
    strip_leading_requirement_glyphs, strip_snapshot_page_prefix,
};
pub use sequence_text::{
    SequenceCourseCodeInput, SequenceCourseCodeRow, SequenceInlineSegmentsInput,
    expand_sequence_course_codes, split_inline_sequence_segments,
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
